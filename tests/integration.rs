use std::fs;
use std::io::Read;
use std::path::Path;

use assert_cmd::Command;
use assert_cmd::cargo::cargo_bin_cmd;
use predicates::prelude::*;
use tempfile::TempDir;
use zip::ZipArchive;

/// Helper: create a file with content inside a directory.
fn create_file(dir: &Path, name: &str, content: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(&path, content).unwrap();
}

/// Helper: list all file entries in a zip archive (sorted).
fn zip_entries(zip_path: &Path) -> Vec<String> {
    let file = fs::File::open(zip_path).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let mut entries: Vec<String> = (0..archive.len())
        .map(|i| archive.by_index(i).unwrap().name().to_string())
        .filter(|name| !name.ends_with('/')) // skip directory entries
        .collect();
    entries.sort();
    entries
}

/// Helper: read a file's content from inside a zip archive.
fn zip_file_content(zip_path: &Path, name: &str) -> String {
    let file = fs::File::open(zip_path).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    let mut entry = archive.by_name(name).unwrap();
    let mut content = String::new();
    entry.read_to_string(&mut content).unwrap();
    content
}

/// Read every byte so the independent ZIP reader verifies every entry's CRC.
fn validate_all_zip_entries(zip_path: &Path) {
    let file = fs::File::open(zip_path).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).unwrap();
        std::io::copy(&mut entry, &mut std::io::sink()).unwrap();
    }
}

/// Helper: create a Command for the zwi binary.
fn zwi_cmd() -> Command {
    cargo_bin_cmd!("zwi")
}

#[test]
fn test_basic_zip_with_gitignore() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();

    // Create files
    create_file(&dir, ".gitignore", "*.log\nbuild/\n");
    create_file(&dir, "src/main.rs", "fn main() {}");
    create_file(&dir, "README.md", "# Hello");
    create_file(&dir, "debug.log", "some log");
    create_file(&dir, "build/output.bin", "binary");

    // Create a .git directory that should be excluded
    create_file(&dir, ".git/config", "[core]");

    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success()
        .stderr(predicate::str::contains("Created"));

    let entries = zip_entries(&output_zip);

    // Should include these files
    assert!(entries.contains(&"src/main.rs".to_string()));
    assert!(entries.contains(&"README.md".to_string()));
    assert!(entries.contains(&".gitignore".to_string()));

    // Should NOT include ignored files
    assert!(!entries.contains(&"debug.log".to_string()));
    assert!(!entries.contains(&"build/output.bin".to_string()));

    // Should NOT include .git
    assert!(!entries.iter().any(|e| e.starts_with(".git/")));

    // Verify file content is preserved
    assert_eq!(zip_file_content(&output_zip, "src/main.rs"), "fn main() {}");
    assert_eq!(zip_file_content(&output_zip, "README.md"), "# Hello");
}

#[test]
fn test_nested_gitignore_files_are_respected_with_directory_scope() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();

    create_file(&dir, ".gitignore", "*.tmp\n");
    create_file(
        &dir,
        "src/.gitignore",
        "!keep.tmp\n/local-only.txt\n*.generated\n",
    );
    create_file(&dir, "root.tmp", "ignored by the root rules");
    create_file(&dir, "src/drop.tmp", "also ignored by the root rules");
    create_file(&dir, "src/keep.tmp", "restored by the nested rules");
    create_file(&dir, "src/local-only.txt", "ignored only below src");
    create_file(&dir, "src/nested/output.generated", "nested rule inherited");
    create_file(&dir, "docs/local-only.txt", "nested rule must not leak");
    create_file(&dir, "docs/output.generated", "nested rule must not leak");

    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();

    let entries = zip_entries(&output_zip);

    assert!(entries.contains(&"src/.gitignore".to_string()));
    assert!(entries.contains(&"src/keep.tmp".to_string()));
    assert!(entries.contains(&"docs/local-only.txt".to_string()));
    assert!(entries.contains(&"docs/output.generated".to_string()));
    assert!(!entries.contains(&"root.tmp".to_string()));
    assert!(!entries.contains(&"src/drop.tmp".to_string()));
    assert!(!entries.contains(&"src/local-only.txt".to_string()));
    assert!(!entries.contains(&"src/nested/output.generated".to_string()));
}

#[test]
fn test_custom_ignore_file() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();

    // No .gitignore, but we'll use a custom ignore file
    let ignore_file = tmp.path().join("custom.ignore");
    fs::write(&ignore_file, "*.tmp\nsecret/\n").unwrap();

    create_file(&dir, "app.py", "print('hello')");
    create_file(&dir, "data.tmp", "temporary");
    create_file(&dir, "secret/key.pem", "-----BEGIN-----");
    create_file(&dir, "docs/guide.md", "# Guide");

    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .arg("--ignore-file")
        .arg(&ignore_file)
        .assert()
        .success();

    let entries = zip_entries(&output_zip);

    assert!(entries.contains(&"app.py".to_string()));
    assert!(entries.contains(&"docs/guide.md".to_string()));

    // Should be excluded by custom ignore
    assert!(!entries.contains(&"data.tmp".to_string()));
    assert!(!entries.contains(&"secret/key.pem".to_string()));
}

#[test]
fn test_no_gitignore_shows_error() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();
    create_file(&dir, "file.txt", "hello");

    zwi_cmd()
        .arg(&dir)
        .assert()
        .failure()
        .stderr(predicate::str::contains("No .gitignore file found"))
        .stderr(predicate::str::contains("--ignore-file"));
}

#[test]
fn test_invalid_directory() {
    zwi_cmd()
        .arg("/nonexistent/path")
        .assert()
        .failure()
        .stderr(predicate::str::contains("is not a directory"));
}

#[test]
fn test_default_output_name() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("my-project");
    fs::create_dir_all(&dir).unwrap();

    create_file(&dir, ".gitignore", "");
    create_file(&dir, "file.txt", "content");

    zwi_cmd()
        .arg(&dir)
        .assert()
        .success()
        .stderr(predicate::str::contains("my-project.zip"));

    // Clean up the zip file created in the current directory
    let _ = fs::remove_file("my-project.zip");
}

#[test]
fn test_help_flag() {
    zwi_cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("zip with ignore"))
        .stdout(predicate::str::contains("--ignore-file"))
        .stdout(predicate::str::contains("[default: 6]"))
        .stdout(predicate::str::contains("--output"));
}

#[test]
fn test_version_flag() {
    zwi_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("zwi 0.3.0"));
}

#[test]
fn test_gitignore_wildcard_patterns() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();

    create_file(&dir, ".gitignore", "*.o\n*.so\ntarget/\n");
    create_file(&dir, "main.c", "#include <stdio.h>");
    create_file(&dir, "main.o", "object file");
    create_file(&dir, "lib.so", "shared object");
    create_file(&dir, "target/debug/bin", "binary");
    create_file(&dir, "src/util.c", "void helper() {}");
    create_file(&dir, "src/util.o", "object file");

    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();

    let entries = zip_entries(&output_zip);

    // Source files included
    assert!(entries.contains(&"main.c".to_string()));
    assert!(entries.contains(&"src/util.c".to_string()));
    assert!(entries.contains(&".gitignore".to_string()));

    // Build artifacts excluded
    assert!(!entries.contains(&"main.o".to_string()));
    assert!(!entries.contains(&"lib.so".to_string()));
    assert!(!entries.contains(&"src/util.o".to_string()));
    assert!(!entries.iter().any(|e| e.starts_with("target/")));
}

#[test]
fn test_negation_pattern_in_gitignore() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();

    // Ignore all .log files except important.log
    create_file(&dir, ".gitignore", "*.log\n!important.log\n");
    create_file(&dir, "debug.log", "debug info");
    create_file(&dir, "important.log", "keep this");
    create_file(&dir, "app.py", "print('hello')");

    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();

    let entries = zip_entries(&output_zip);

    assert!(entries.contains(&"app.py".to_string()));
    assert!(entries.contains(&"important.log".to_string()));
    assert!(!entries.contains(&"debug.log".to_string()));
}

#[test]
fn test_hidden_files_included() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();

    create_file(&dir, ".gitignore", "");
    create_file(&dir, ".env", "SECRET=value");
    create_file(&dir, ".hidden_dir/config", "config data");
    create_file(&dir, "visible.txt", "hello");

    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();

    let entries = zip_entries(&output_zip);

    // Hidden files should be included (they're not in .gitignore)
    assert!(entries.contains(&".env".to_string()));
    assert!(entries.contains(&".hidden_dir/config".to_string()));
    assert!(entries.contains(&"visible.txt".to_string()));
}

#[test]
fn test_missing_ignore_file() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();
    create_file(&dir, "file.txt", "content");

    zwi_cmd()
        .arg(&dir)
        .arg("--ignore-file")
        .arg("/nonexistent/ignore")
        .assert()
        .failure()
        .stderr(predicate::str::contains("does not exist"));
}

#[test]
fn test_git_directory_always_excluded() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();

    create_file(&dir, ".gitignore", "");
    create_file(&dir, ".git/HEAD", "ref: refs/heads/main");
    create_file(&dir, ".git/config", "[core]");
    create_file(&dir, ".git/objects/abc123", "blob");
    create_file(&dir, "src/main.rs", "fn main() {}");

    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();

    let entries = zip_entries(&output_zip);

    // No .git entries at all
    assert!(!entries.iter().any(|e| e.contains(".git/")));
    assert!(!entries.iter().any(|e| e == ".git"));

    // But source files are present
    assert!(entries.contains(&"src/main.rs".to_string()));
}

#[test]
fn test_parallel_pipeline_preserves_all_content() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();
    create_file(&dir, ".gitignore", "");
    for index in 0..40 {
        create_file(
            &dir,
            &format!("nested/file-{index}.txt"),
            &format!("content for file {index}"),
        );
    }
    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .arg("--threads")
        .arg("4")
        .assert()
        .success();

    let entries = zip_entries(&output_zip);
    assert_eq!(entries.len(), 41);
    for index in 0..40 {
        let name = format!("nested/file-{index}.txt");
        assert_eq!(
            zip_file_content(&output_zip, &name),
            format!("content for file {index}")
        );
    }
}

#[test]
fn test_output_inside_source_is_not_archived() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();
    create_file(&dir, ".gitignore", "");
    create_file(&dir, "file.txt", "content");
    let output_zip = dir.join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();

    assert!(!zip_entries(&output_zip).contains(&"output.zip".to_string()));
}

#[test]
fn test_quiet_suppresses_summary() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();
    create_file(&dir, ".gitignore", "");
    create_file(&dir, "file.txt", "content");
    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .arg("--quiet")
        .assert()
        .success()
        .stderr(predicate::str::is_empty());
}

#[test]
fn test_small_memory_limit_falls_back_to_streaming() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();
    create_file(&dir, ".gitignore", "");
    fs::write(dir.join("large.bin"), vec![b'x'; 2 * 1024 * 1024]).unwrap();
    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .arg("--memory-limit")
        .arg("1")
        .assert()
        .success();

    assert_eq!(
        zip_file_content(&output_zip, "large.bin").len(),
        2 * 1024 * 1024
    );
}

#[test]
fn test_empty_directories_zero_length_and_unicode_are_valid() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(dir.join("empty/nested")).unwrap();
    create_file(&dir, ".gitignore", "");
    create_file(&dir, "zero.bin", "");
    create_file(&dir, "café/東京-🚀.txt", "portable unicode");
    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .arg("--threads")
        .arg("4")
        .assert()
        .success();

    validate_all_zip_entries(&output_zip);
    assert_eq!(
        zip_file_content(&output_zip, "café/東京-🚀.txt"),
        "portable unicode"
    );
    let file = fs::File::open(&output_zip).unwrap();
    let mut archive = ZipArchive::new(file).unwrap();
    assert!(archive.by_name("empty/nested/").is_ok());
}

#[test]
fn test_existing_archive_is_replaced_only_after_success() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();
    create_file(&dir, ".gitignore", "");
    create_file(&dir, "first.txt", "first version");
    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();
    create_file(&dir, "second.txt", "second version");
    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();

    validate_all_zip_entries(&output_zip);
    assert_eq!(
        zip_file_content(&output_zip, "second.txt"),
        "second version"
    );
}

#[cfg(unix)]
#[test]
fn test_compression_failure_preserves_previous_valid_archive() {
    use std::os::unix::net::UnixListener;

    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();
    create_file(&dir, ".gitignore", "");
    create_file(&dir, "file.txt", "known good content");
    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();
    let original = fs::read(&output_zip).unwrap();
    let _socket = UnixListener::bind(dir.join("unreadable.socket")).unwrap();

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .failure();

    assert_eq!(fs::read(&output_zip).unwrap(), original);
    validate_all_zip_entries(&output_zip);
}

#[cfg(unix)]
#[test]
fn test_backslashes_are_normalized_for_windows_readers() {
    let tmp = TempDir::new().unwrap();
    let dir = tmp.path().join("project");
    fs::create_dir_all(&dir).unwrap();
    create_file(&dir, ".gitignore", "");
    create_file(&dir, "folder\\file.txt", "portable path");
    let output_zip = tmp.path().join("output.zip");

    zwi_cmd()
        .arg(&dir)
        .arg("-o")
        .arg(&output_zip)
        .assert()
        .success();

    validate_all_zip_entries(&output_zip);
    assert_eq!(
        zip_file_content(&output_zip, "folder/file.txt"),
        "portable path"
    );
}
