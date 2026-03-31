# gac — Agent Guidelines

Rust CLI tool that generates AI-powered commit messages using Ollama.

## Build / Test / Lint Commands

```bash
cargo build                          # Debug build
cargo build --release                # Optimized release build
cargo test                           # Run all tests
cargo test <module_name>             # Run tests for a specific module (e.g., cargo test diff)
cargo test <test_name>               # Run a single test by name (e.g., cargo test test_parse_diff_single_file)
cargo test -- --nocapture            # Run tests with stdout visible
cargo test <module> -- <test_name>   # Run one test in a module (e.g., cargo test diff::tests::test_parse_diff_single_file)
cargo fmt --check                    # Check formatting
cargo fmt                            # Auto-fix formatting
cargo clippy --all-targets           # Run clippy lints
cargo clippy --all-targets -- -D warnings  # Strict clippy (must pass)
cargo doc                            # Build docs
cargo CI                             # Run all checks (fmt + clippy + test)
```

Windows: same commands in PowerShell/CMD.

## Project Structure

```
gac/
├── src/
│   ├── main.rs       # Entry point, CLI parsing, orchestration
│   ├── config.rs     # Config loading (.gac.toml), scope definitions
│   ├── diff.rs       # Git diff parsing, priority scoring, strategy selection
│   ├── git.rs        # Git operations (staged files, scopes, excludes)
│   ├── logging.rs    # Custom CLI logging formatter
│   ├── ollama.rs     # Ollama API client (streaming/non-streaming, retry)
│   ├── prompt.rs     # Prompt building with Askama templates
│   ├── spinner.rs    # Progress spinners/bars (indicatif)
│   ├── stats.rs      # Generation stats display (tokens, VRAM, timing)
│   └── validate.rs   # Conventional commit validation + auto-fix
├── templates/        # Askama templates for system prompts
├── Cargo.toml
└── .gac.toml         # Project config template
```

## Code Style Guidelines

### Formatting
- Use `cargo fmt` — default rustfmt settings
- 4-space indentation, no tabs
- Max line length: 100 chars (soft limit, prefer readability)
- Trailing commas in multi-line constructs

### Imports
Group in order, blank line between groups:
1. Standard library (`std::`)
2. External crates (alphabetical)
3. `crate::` local imports

```rust
use std::{collections::HashMap, path::Path};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::{config::Config, ollama::generate_streaming};
```

### Naming Conventions

| Type | Convention | Example |
|------|------------|---------|
| Modules | snake_case | `git.rs`, `ollama.rs` |
| Structs/Enums | PascalCase | `Config`, `Strategy` |
| Enum variants | PascalCase | `Strategy::Direct` |
| Functions/Variables | snake_case | `get_staged_files()`, `max_diff_chars` |
| Constants | SCREAMING_SNAKE_CASE | `MAX_RETRIES` |

### Types
- `&str` for string slices, `String` for owned strings
- `&[T]` for slice parameters, `&mut T` for mutation
- `Vec<T>`, `HashMap<K,V>`, `Option<T>` for collections
- Derive common traits: `#[derive(Debug, Clone, Serialize, Deserialize)]`

### Error Handling
- Use `anyhow::Result<T>` for application errors
- Use `anyhow::bail!("message")` for early returns
- Use `?` operator for error propagation
- Never use `unwrap()` on user input or network responses
- Use `warn!` instead of `unwrap_or_default()` on fallible operations

```rust
fn load_config() -> anyhow::Result<Config> {
    let raw = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&raw)
        .context("Invalid config format")?;
    Ok(config)
}
```

### Async Code
- `#[tokio::main]` for entry point
- Reuse `reqwest::Client` across calls (connection pooling)
- Use `futures_util::StreamExt` for streams, `buffer_unordered(n)` for parallelism
- Keep async boundaries minimal

### Logging
- Use `tracing` — levels: `error!`, `warn!`, `info!`, `debug!`
- Structured fields: `debug!(count = files.len(), "message")`
- Never log sensitive data (API keys, tokens)

### Templates (Askama)
- Location: `templates/` with `.md` extension
- Syntax: `{{ variable }}`, `{% if %}`, `{% for %}`
- Include via `include_str!("../templates/file.md")`
- Render with `.unwrap()` only for compile-time-checked templates

## Key Architecture Patterns

- **Diff strategies**: Direct (fits context) → Summarize (per-file API calls) → StatOnly (top-N files)
- **Config cascade**: defaults → user config (`~/.config/gac/`) → project config (`.gac.toml`) → CLI overrides
- **Commit validation**: regex check → up to 2 retries with error context → auto-fix fallback → user approval
