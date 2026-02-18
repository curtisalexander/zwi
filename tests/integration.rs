use std::fs;
use std::io::Read;
use std::path::Path;

use assert_cmd::cargo::cargo_bin_cmd;
use assert_cmd::Command;
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
        .stdout(predicate::str::contains("--output"));
}

#[test]
fn test_version_flag() {
    zwi_cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("zwi"));
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
