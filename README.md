# zwi

Zip with ignore — a cross-platform CLI tool that creates zip archives from a directory while respecting `.gitignore` rules. The `.git` directory is always excluded.

## Features

- Respects `.gitignore` rules automatically
- Always excludes `.git` directory
- Supports custom ignore files via `--ignore-file`
- Cross-platform (Linux, macOS, Windows)
- Installable via `uv` — no Rust toolchain required

## Installation

### With `uv` (recommended)

Install as a tool:

```bash
uv tool install zwi \
  --no-index \
  --find-links https://github.com/curtisalexander/zwi/releases/expanded_assets/v0.1.0
```

Or run directly without installing:

```bash
uvx --no-index --from zwi \
  --find-links https://github.com/curtisalexander/zwi/releases/expanded_assets/v0.1.0 \
  zwi .
```

Or install into a virtual environment:

```bash
uv pip install zwi \
  --no-index \
  --find-links https://github.com/curtisalexander/zwi/releases/expanded_assets/v0.1.0
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

## License

MIT
