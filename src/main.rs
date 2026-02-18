use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use ignore::WalkBuilder;
use zip::write::FileOptions;
use zip::CompressionMethod;
use zip::ZipWriter;

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
}

fn run(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let dir = &cli.directory;

    if !dir.is_dir() {
        return Err(format!("'{}' is not a directory", dir.display()).into());
    }

    // Canonicalize the directory so we can compute relative paths reliably.
    let dir = dir.canonicalize()?;

    // Determine whether we have an ignore file.
    let ignore_file = cli.ignore_file.as_deref();
    let gitignore_path = dir.join(".gitignore");

    if ignore_file.is_none() && !gitignore_path.exists() {
        return Err(format!(
            "No .gitignore file found in '{}'. \
             Use --ignore-file to specify a custom ignore file.",
            dir.display()
        )
        .into());
    }

    // Validate the custom ignore file if provided.
    if let Some(path) = ignore_file
        && !path.exists()
    {
        return Err(format!("Ignore file '{}' does not exist", path.display()).into());
    }

    // Determine output path.
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

    // Canonicalize the output path's parent so we can detect it during the walk.
    let output_path_canonical = if output_path.exists() {
        Some(output_path.canonicalize()?)
    } else {
        None
    };

    let file = File::create(&output_path)?;
    let mut zip = ZipWriter::new(file);
    let options = FileOptions::<()>::default().compression_method(CompressionMethod::Deflated);

    // Build the walker.
    let mut builder = WalkBuilder::new(&dir);
    builder
        .hidden(false) // include hidden files (except .git)
        .git_global(false)
        .git_ignore(false) // we add ignore files explicitly below
        .git_exclude(false);

    if let Some(path) = ignore_file {
        builder.add_ignore(path);
    } else {
        builder.add_ignore(&gitignore_path);
    }

    let mut count: u64 = 0;

    for entry in builder.build() {
        let entry = entry?;
        let path = entry.path();

        // Skip the root directory itself.
        if path == dir {
            continue;
        }

        // Always skip .git directories.
        if is_git_path(path, &dir) {
            continue;
        }

        // Skip the output zip file if it is inside the directory being zipped.
        if let Some(ref canonical) = output_path_canonical
            && let Ok(entry_canonical) = path.canonicalize()
            && entry_canonical == *canonical
        {
            continue;
        }

        let relative = path.strip_prefix(&dir)?;
        // Normalize path separators to forward slashes for zip compatibility.
        let name = relative
            .to_string_lossy()
            .replace(std::path::MAIN_SEPARATOR, "/");

        if path.is_dir() {
            zip.add_directory(&name, options)?;
        } else {
            zip.start_file(&name, options)?;
            let mut f = File::open(path)?;
            io::copy(&mut f, &mut zip)?;
            count += 1;
        }
    }

    zip.finish()?;

    eprintln!(
        "Created '{}' with {count} file(s)",
        output_path.display()
    );

    Ok(())
}

/// Returns true if the path is inside a `.git` directory.
fn is_git_path(path: &Path, base: &Path) -> bool {
    if let Ok(relative) = path.strip_prefix(base) {
        relative
            .components()
            .any(|c| c.as_os_str() == ".git")
    } else {
        false
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    if let Err(e) = run(cli) {
        eprintln!("Error: {e}");
        return ExitCode::FAILURE;
    }

    ExitCode::SUCCESS
}
