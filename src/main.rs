use std::fs::{self, File};
use std::io::{self, Cursor, IsTerminal, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use clap::Parser;
use crossbeam_channel::bounded;
use ignore::{WalkBuilder, WalkState};
use indicatif::{HumanBytes, HumanDuration, ProgressBar, ProgressDrawTarget, ProgressStyle};
use tempfile::NamedTempFile;
use zip::write::FileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

const MAX_PARALLEL_FILE_SIZE: u64 = 64 * 1024 * 1024;
const DEFAULT_MEMORY_LIMIT_MIB: u64 = 256;
const MIB: u64 = 1024 * 1024;

/// zwi — zip with ignore
///
/// Create a zip archive from a directory, automatically respecting .gitignore
/// rules. The .git directory is always excluded.
#[derive(Parser)]
#[command(name = "zwi", version, about)]
struct Cli {
    /// Directory to zip
    directory: PathBuf,

    /// Output zip file path [default: <directory-name>.zip]
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Path to a custom ignore file (instead of .gitignore)
    #[arg(short = 'i', long = "ignore-file")]
    ignore_file: Option<PathBuf>,

    /// Number of files to compress concurrently
    #[arg(short = 'j', long, value_parser = parse_threads, default_value_t = default_threads())]
    threads: usize,

    /// Deflate compression level: 1 is fastest, 9 is smallest
    #[arg(long, value_parser = clap::value_parser!(i64).range(1..=9), default_value_t = 6)]
    compression_level: i64,

    /// Maximum memory used for parallel file buffers, in MiB
    #[arg(long, value_parser = parse_memory_limit, default_value_t = DEFAULT_MEMORY_LIMIT_MIB)]
    memory_limit: u64,

    /// Suppress status and summary output
    #[arg(short, long)]
    quiet: bool,
}

#[derive(Clone)]
struct ArchiveEntry {
    path: PathBuf,
    name: String,
    size: u64,
    is_dir: bool,
}

struct CompressedFile {
    name: String,
    archive: Vec<u8>,
    _memory: MemoryPermit,
}

struct MemoryBudget {
    available: Mutex<u64>,
    changed: Condvar,
}

struct MemoryPermit {
    budget: Arc<MemoryBudget>,
    bytes: u64,
}

fn default_threads() -> usize {
    // Each worker can hold one compressed file in memory. A modest cap delivers
    // useful parallelism without multiplying peak memory on many-core machines.
    thread::available_parallelism()
        .map(usize::from)
        .unwrap_or(1)
        .clamp(1, 8)
}

fn parse_threads(value: &str) -> Result<usize, String> {
    value
        .parse::<usize>()
        .ok()
        .filter(|threads| *threads > 0)
        .ok_or_else(|| "thread count must be at least 1".to_owned())
}

fn parse_memory_limit(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .ok()
        .filter(|mib| *mib > 0 && *mib <= usize::MAX as u64 / MIB)
        .ok_or_else(|| "memory limit must be a positive number of MiB".to_owned())
}

impl MemoryBudget {
    fn new(bytes: u64) -> Arc<Self> {
        Arc::new(Self {
            available: Mutex::new(bytes),
            changed: Condvar::new(),
        })
    }

    fn acquire(self: &Arc<Self>, bytes: u64) -> MemoryPermit {
        let mut available = self.available.lock().unwrap();
        while *available < bytes {
            available = self.changed.wait(available).unwrap();
        }
        *available -= bytes;
        MemoryPermit {
            budget: Arc::clone(self),
            bytes,
        }
    }
}

impl Drop for MemoryPermit {
    fn drop(&mut self) {
        let mut available = self.budget.available.lock().unwrap();
        *available += self.bytes;
        self.budget.changed.notify_all();
    }
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let dir = &cli.directory;

    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", dir.display()).into());
    }

    let dir = dir.canonicalize()?;
    let ignore_file = cli.ignore_file.as_deref();
    let gitignore_path = dir.join(".gitignore");

    if ignore_file.is_none() && !gitignore_path.exists() {
        return Err(format!(
            "No .gitignore file found in '{}'. Use --ignore-file to specify a custom ignore file.",
            dir.display()
        )
        .into());
    }

    if let Some(path) = ignore_file
        && !path.exists()
    {
        return Err(format!("Ignore file '{}' does not exist", path.display()).into());
    }

    let output_path = match &cli.output {
        Some(p) => p.clone(),
        None => {
            let dir_name = dir
                .file_name()
                .ok_or("Cannot determine directory name")?
                .to_string_lossy();
            PathBuf::from(format!("{dir_name}.zip"))
        }
    };
    let output_absolute = absolute_path(&output_path)?;

    let scan = spinner("Scanning files", cli.quiet);
    let entries = discover_entries(&dir, ignore_file, &output_absolute, cli.threads)?;
    let file_count = entries.iter().filter(|entry| !entry.is_dir).count();
    let total_bytes: u64 = entries.iter().map(|entry| entry.size).sum();
    scan.finish_and_clear();

    if !cli.quiet {
        eprintln!(
            "  Found {} file(s), {} to process using {} thread(s)",
            file_count,
            HumanBytes(total_bytes),
            cli.threads.min(file_count.max(1))
        );
    }

    // Build beside the destination so the final rename stays on one filesystem.
    // A failure at any earlier point drops and removes the incomplete temp file.
    let output_parent = output_absolute
        .parent()
        .ok_or("Cannot determine output directory")?;
    let mut temporary_output = NamedTempFile::new_in(output_parent)?;
    let mut zip = ZipWriter::new(temporary_output.as_file_mut());
    let options = FileOptions::<()>::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(cli.compression_level));

    for entry in entries.iter().filter(|entry| entry.is_dir) {
        zip.add_directory(&entry.name, options)?;
    }

    let files: Vec<_> = entries.into_iter().filter(|entry| !entry.is_dir).collect();
    let progress = byte_progress(total_bytes, file_count, cli.quiet);
    compress_pipeline(
        files,
        cli.threads,
        cli.compression_level,
        cli.memory_limit * MIB,
        &progress,
        &mut zip,
    )?;
    zip.finish()?.flush()?;
    progress.finish_and_clear();
    temporary_output.persist(&output_path)?;

    if !cli.quiet {
        let archive_size = fs::metadata(&output_path)?.len();
        eprintln!(
            "✓ Created '{}' • {} file(s) • {} → {} • {}",
            output_path.display(),
            file_count,
            HumanBytes(total_bytes),
            HumanBytes(archive_size),
            HumanDuration(started.elapsed())
        );
    }

    Ok(())
}

fn discover_entries(
    dir: &Path,
    custom_ignore_file: Option<&Path>,
    output_absolute: &Path,
    threads: usize,
) -> Result<Vec<ArchiveEntry>, Box<dyn std::error::Error>> {
    let mut builder = WalkBuilder::new(dir);
    builder
        .hidden(false)
        .parents(false)
        .git_global(false)
        .git_exclude(false)
        .threads(threads);

    if let Some(ignore_file) = custom_ignore_file {
        builder.git_ignore(false).add_ignore(ignore_file);
    } else {
        // Discover .gitignore files while descending so that each file's
        // patterns are scoped to its directory. zwi does not require the
        // input directory to be part of a Git repository.
        builder.git_ignore(true).require_git(false);
    }

    let entries = std::sync::Mutex::new(Vec::new());
    let errors = std::sync::Mutex::new(Vec::new());
    builder.build_parallel().run(|| {
        let entries = &entries;
        let errors = &errors;
        Box::new(move |result| {
            let entry = match result {
                Ok(entry) => entry,
                Err(error) => {
                    errors.lock().unwrap().push(error.to_string());
                    return WalkState::Continue;
                }
            };
            let path = entry.path();
            if path == dir || is_git_path(path, dir) {
                return WalkState::Continue;
            }
            if absolute_path(path).is_ok_and(|absolute| absolute == output_absolute) {
                return WalkState::Continue;
            }

            let relative = match path.strip_prefix(dir) {
                Ok(relative) => relative,
                Err(error) => {
                    errors.lock().unwrap().push(error.to_string());
                    return WalkState::Continue;
                }
            };
            let name = relative
                .to_string_lossy()
                .replace(['\\', std::path::MAIN_SEPARATOR], "/");
            let is_dir = entry.file_type().is_some_and(|kind| kind.is_dir());
            let size = if is_dir {
                0
            } else {
                match entry.metadata() {
                    Ok(metadata) => metadata.len(),
                    Err(error) => {
                        errors.lock().unwrap().push(error.to_string());
                        return WalkState::Continue;
                    }
                }
            };
            entries.lock().unwrap().push(ArchiveEntry {
                path: path.to_owned(),
                name,
                size,
                is_dir,
            });
            WalkState::Continue
        })
    });

    let errors = errors.into_inner().unwrap();
    if !errors.is_empty() {
        return Err(format!("Failed to scan input: {}", errors.join("; ")).into());
    }
    let mut entries = entries.into_inner().unwrap();
    entries.sort_unstable_by(|left, right| left.name.cmp(&right.name));
    Ok(entries)
}

fn compress_pipeline<W: Write + Seek>(
    files: Vec<ArchiveEntry>,
    requested_threads: usize,
    compression_level: i64,
    memory_limit: u64,
    progress: &ProgressBar,
    output: &mut ZipWriter<W>,
) -> Result<(), Box<dyn std::error::Error>> {
    if files.is_empty() {
        return Ok(());
    }

    let total_file_count = files.len();
    let (buffered, streamed): (Vec<_>, Vec<_>) = files.into_iter().partition(|entry| {
        entry.size <= MAX_PARALLEL_FILE_SIZE && buffer_reservation(entry.size) <= memory_limit
    });
    let files = Arc::new(buffered);
    let next = Arc::new(AtomicUsize::new(0));
    let completed = Arc::new(AtomicUsize::new(0));
    let memory = MemoryBudget::new(memory_limit);
    let worker_count = requested_threads.min(files.len());
    let (sender, receiver) = bounded::<Result<CompressedFile, String>>(worker_count.max(1));

    thread::scope(|scope| -> Result<(), Box<dyn std::error::Error>> {
        for _ in 0..worker_count {
            let files = Arc::clone(&files);
            let next = Arc::clone(&next);
            let completed = Arc::clone(&completed);
            let memory = Arc::clone(&memory);
            let sender = sender.clone();
            let progress = progress.clone();
            scope.spawn(move || {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(entry) = files.get(index) else {
                        break;
                    };
                    let reserved = buffer_reservation(entry.size);
                    let permit = memory.acquire(reserved);
                    let result = compress_file(
                        entry,
                        compression_level,
                        reserved as usize,
                        permit,
                        &progress,
                    );
                    if result.is_ok() {
                        let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
                        progress
                            .set_message(format!("Compressing  {count}/{total_file_count} files"));
                    }
                    if sender.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(sender);

        for result in receiver {
            let compressed = result.map_err(io::Error::other)?;
            let reader = Cursor::new(compressed.archive);
            let mut archive = ZipArchive::new(reader)?;
            let file = archive.by_index(0)?;
            output.raw_copy_file_rename(file, &compressed.name)?;
        }

        for entry in streamed {
            let options = FileOptions::<()>::default()
                .compression_method(CompressionMethod::Deflated)
                .compression_level(Some(compression_level))
                .large_file(zip64_required(entry.size));
            output.start_file(&entry.name, options)?;
            let input = File::open(&entry.path)?;
            let mut reader = ProgressReader { input, progress };
            io::copy(&mut reader, output)?;
            let count = completed.fetch_add(1, Ordering::Relaxed) + 1;
            progress.set_message(format!("Compressing  {count}/{total_file_count} files"));
        }
        Ok(())
    })
}

fn compress_file(
    entry: &ArchiveEntry,
    compression_level: i64,
    buffer_capacity: usize,
    memory: MemoryPermit,
    progress: &ProgressBar,
) -> Result<CompressedFile, String> {
    let input = File::open(&entry.path)
        .map_err(|error| format!("Could not open '{}': {error}", entry.path.display()))?;
    let mut reader = ProgressReader { input, progress };
    let cursor = Cursor::new(Vec::with_capacity(buffer_capacity));
    let mut zip = ZipWriter::new(cursor);
    let options = FileOptions::<()>::default()
        .compression_method(CompressionMethod::Deflated)
        .compression_level(Some(compression_level));
    zip.start_file(&entry.name, options)
        .map_err(|error| format!("Could not archive '{}': {error}", entry.path.display()))?;
    io::copy(&mut reader, &mut zip)
        .map_err(|error| format!("Could not read '{}': {error}", entry.path.display()))?;
    let cursor = zip
        .finish()
        .map_err(|error| format!("Could not compress '{}': {error}", entry.path.display()))?;
    Ok(CompressedFile {
        name: entry.name.clone(),
        archive: cursor.into_inner(),
        _memory: memory,
    })
}

fn buffer_reservation(file_size: u64) -> u64 {
    // Raw deflate can be slightly larger than incompressible input. Reserve 1%
    // plus room for ZIP headers so Vec never needs to grow beyond its permit.
    file_size
        .saturating_add(file_size / 100)
        .saturating_add(64 * 1024)
}

fn zip64_required(file_size: u64) -> bool {
    // The compressed form can be slightly larger than incompressible input.
    // ZIP64 must be selected before start_file writes the local file header.
    buffer_reservation(file_size) > u32::MAX as u64
}

struct ProgressReader<'a> {
    input: File,
    progress: &'a ProgressBar,
}

impl Read for ProgressReader<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.input.read(buffer)?;
        self.progress.inc(read as u64);
        Ok(read)
    }
}

fn spinner(message: &str, quiet: bool) -> ProgressBar {
    if quiet || !io::stderr().is_terminal() {
        return ProgressBar::hidden();
    }
    let progress = ProgressBar::new_spinner();
    progress.set_draw_target(ProgressDrawTarget::stderr_with_hz(10));
    progress.set_style(
        ProgressStyle::with_template("{spinner:.cyan} {msg}  {elapsed_precise}")
            .unwrap()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"]),
    );
    progress.set_message(message.to_owned());
    progress.enable_steady_tick(Duration::from_millis(100));
    progress
}

fn byte_progress(total_bytes: u64, file_count: usize, quiet: bool) -> ProgressBar {
    if quiet || !io::stderr().is_terminal() {
        return ProgressBar::hidden();
    }
    let progress = ProgressBar::new(total_bytes);
    progress.set_draw_target(ProgressDrawTarget::stderr_with_hz(10));
    progress.set_style(
        ProgressStyle::with_template(
            "{spinner:.cyan} {msg}\n  [{bar:36.cyan/blue}] {bytes}/{total_bytes}  {bytes_per_sec}  ETA {eta}",
        )
        .unwrap()
        .progress_chars("█▓░"),
    );
    progress.set_message(format!("Compressing  0/{file_count} files"));
    progress
}

fn absolute_path(path: &Path) -> io::Result<PathBuf> {
    if path.exists() {
        return path.canonicalize();
    }

    let absolute = if path.is_absolute() {
        path.to_owned()
    } else {
        std::env::current_dir()?.join(path)
    };
    let parent = absolute.parent().unwrap_or(Path::new(".")).canonicalize()?;
    let file_name = absolute
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    Ok(parent.join(file_name))
}

/// Returns true if the path is inside a `.git` directory.
fn is_git_path(path: &Path, base: &Path) -> bool {
    path.strip_prefix(base).is_ok_and(|relative| {
        relative
            .components()
            .any(|component| component.as_os_str() == ".git")
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Err(error) = run(cli) {
        eprintln!("Error: {error}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip64_is_enabled_before_a_file_can_cross_the_32_bit_limit() {
        assert!(!zip64_required(1024 * 1024));
        assert!(zip64_required(u32::MAX as u64));
    }
}
