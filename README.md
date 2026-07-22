# zwi

Zip with ignore — a cross-platform CLI tool that creates zip archives from a directory while respecting `.gitignore` rules. The `.git` directory is always excluded.

## Features

- Respects `.gitignore` rules automatically
- Always excludes `.git` directory
- Supports custom ignore files via `--ignore-file`
- Live file, byte, throughput, elapsed-time, and ETA status
- Parallel file discovery and compression
- Cross-platform (Linux, macOS, Windows)
- Installable via `uv` — no Rust toolchain required

## Installation

### With `uv` (recommended)

Install as a tool:

```bash
uv tool install zwi \
  --no-index \
  --find-links https://github.com/curtisalexander/zwi/releases/expanded_assets/v0.2.0
```

Or run directly without installing:

```bash
uvx --no-index --from zwi \
  --find-links https://github.com/curtisalexander/zwi/releases/expanded_assets/v0.2.0 \
  zwi .
```

Or install into a virtual environment:

```bash
uv pip install zwi \
  --no-index \
  --find-links https://github.com/curtisalexander/zwi/releases/expanded_assets/v0.2.0
```

### From source

```bash
cargo install --path .
```

## Usage

```
zwi [OPTIONS] <DIRECTORY>
```

### Arguments

- `<DIRECTORY>` — Directory to zip

### Options

- `-o, --output <PATH>` — Output zip file path (default: `<directory-name>.zip`)
- `-i, --ignore-file <PATH>` — Path to a custom ignore file (instead of `.gitignore`)
- `-j, --threads <COUNT>` — Files to compress concurrently (default: up to 8 logical CPUs)
- `--compression-level <1-9>` — Deflate level; 1 is fastest, 9 is smallest (default: 3)
- `--memory-limit <MIB>` — Maximum memory for parallel buffers (default: 256)
- `-q, --quiet` — Suppress status and summary output
- `-h, --help` — Print help
- `-V, --version` — Print version

### Examples

Zip the current directory:

```bash
zwi .
```

Zip a specific directory with a custom output name:

```bash
zwi my-project -o archive.zip
```

Use a custom ignore file:

```bash
zwi my-project --ignore-file .myignore
```

## Performance

`zwi` discovers files in parallel, then uses a bounded producer/consumer pipeline
to compress files concurrently. Files up to 64 MiB can enter the parallel lane,
but active and queued buffers share a global 256 MiB memory budget. Larger files,
or files that cannot fit a smaller configured budget, stream directly into the
archive with constant memory. Use `--memory-limit` and `--threads` to tune memory
and CPU usage, or `--compression-level 1` when throughput matters more than
archive size.

## Releasing

Releases are triggered by version tags. After the version is updated consistently
in `Cargo.toml`, `pyproject.toml`, `python/zwi/__init__.py`, and this README, commit
and push the change, then create and push the matching tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

GitHub Actions verifies that the tag matches `Cargo.toml`, builds all platform
wheels, and publishes the GitHub release.

## License

MIT
