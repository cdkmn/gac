# gac — AI-Powered Commit Message Generator

A CLI tool that generates meaningful commit messages using llama.cpp over llama-swap. It analyzes your staged git changes and produces conventional commit messages, handling large diffs intelligently through different strategies.

## Purpose

`gac` (Git AI Commit) automates commit message writing by:

- Analyzing staged changes with AI
- Generating conventional commit messages
- Supporting multiple diff strategies for various project sizes
- Auto-detecting project scopes from file paths
- Providing interactive approval before committing

## Installation

### Prerequisites

- Rust 1.70+ ([install via rustup](https://rustup.rs/))
- [llama-swap](https://github.com/mostlygeek/llama-swap) installed and running locally

### Build from Source

```bash
# Clone the repository
git clone https://github.com/cdkmn/gac.git
cd gac

# Build release version
cargo build --release

# The binary will be at target/release/gac (or target/release/gac.exe on Windows)
```

### Install Default Config

```bash
./target/release/gac init
```

## Usage

### Basic Workflow

```bash
# Stage your changes
git add .

# Run gac - it will analyze changes and prompt for confirmation
gac
```

### Command Line Options

```bash
gac [OPTIONS]

Options:
  -m, --model <MODEL>       Override the Ollama model name
  -c, --num-ctx <NUM_CTX>   Override context window size
  -v, --verbose             Show info-level messages
      --debug               Show debug-level messages
  -q, --quiet               Suppress all output except errors
  init                      Create a .gac.toml config file
  -h, --help                Show help information
  -V, --version             Show version
```

### Examples

```bash
# Use a different model
gac --model qwen2.5-coder:7b

# Increase context window
gac --num-ctx 4096

# Verbose output for debugging
gac --verbose

# Quiet mode (errors only)
gac --quiet
```

## Configuration

### Project Config (`.gac.toml`)

Create this file in your project root:

```toml
[model]
name = "cogito:8b"
endpoint = "http://localhost:11434"
think = false

[options]
num_ctx = 2048
num_predict = 256
temperature = 0.2
top_p = 0.9
num_gpu = 999

[diff]
max_chars = 6000
exclude_patterns = [
    "package-lock.json",
    "yarn.lock",
    "*.lock"
]

[scopes.config]
paths = ["src/config.rs"]
[scopes.ui]
paths = ["src/ui/**", "templates/**"]
```

### User Config

Place at `~/.config/gac/config.toml` (Linux) or equivalent config directory on macOS/Windows.

### Scope Detection

Define scopes in `.gac.toml` to auto-detect commit scope based on changed files:

```toml
[scopes.api]
paths = ["src/api/**", "src/routes/**"]

[scopes.auth]
paths = ["src/auth/**"]
description = "Authentication & authorization"
```

## Diff Strategies

gac automatically selects the best strategy based on diff size:

| Strategy      | When Used                | Description                         |
| ------------- | ------------------------ | ----------------------------------- |
| **Direct**    | Diff fits in context     | Sends full diff to model            |
| **Summarize** | Medium diffs (<20 files) | Summarizes each file individually   |
| **StatOnly**  | Large diffs (20+ files)  | Uses stat + top priority file diffs |

Priority scoring considers:

- Source files (`rs`, `go`, `py`, etc.) → High priority
- Config files (`json`, `yaml`, `toml`) → Low priority
- Files in `src/` or `lib/` → Higher priority boost

## Development

### Setup

```bash
# Clone and enter the repo
git clone https://github.com/cdkmn/gac.git
cd gac

# Install dependencies
cargo fetch
```

### Build Commands

```bash
# Debug build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Run tests with output
cargo test -- --nocapture

# Run a specific test
cargo test test_name_here

# Format code
cargo fmt

# Check formatting
cargo fmt --check

# Lint with clippy
cargo clippy --all-targets

# Lint with strict warnings
cargo clippy --all-targets -- -D warnings

# Build documentation
cargo doc

# Run all checks (CI)
cargo CI
```

### Project Structure

```text
gac/
├── src/
│   ├── main.rs      # Entry point, CLI parsing, orchestration
│   ├── config.rs    # Config loading (.gac.toml)
│   ├── diff.rs      # Git diff parsing and strategy selection
│   ├── git.rs       # Git operations (staged files, scopes)
│   ├── logging.rs   # Custom CLI logging formatter
│   ├── ollama.rs    # Ollama API client
│   ├── prompt.rs    # Prompt building with Askama templates
│   └── stats.rs     # Generation stats display
├── templates/       # Askama templates for prompts
├── Cargo.toml
└── .gac.toml       # Example config
```

### Key Dependencies

- **clap** — CLI argument parsing
- **anyhow** — Error handling
- **tokio** — Async runtime
- **reqwest** — HTTP client for Ollama API
- **tracing** — Structured logging
- **askama** — Template engine for prompts
- **dialoguer** — Interactive prompts

### Adding New Features

1. Fork and create a branch: `git checkout -b feature/your-feature`
2. Follow code style guidelines in `AGENTS.md`
3. Run `cargo fmt` before committing
4. Run `cargo clippy -- -D warnings` to catch issues
5. Submit a pull request

## Commit Message Format

Generated messages follow [Conventional Commits](https://www.conventionalcommits.org/):

```text
<type>(<scope>): <subject>

[optional body]

[optional footer]
```

Types: `feat`, `fix`, `docs`, `style`, `refactor`, `perf`, `test`, `build`, `ci`, `chore`, `revert`

## License

MIT License
