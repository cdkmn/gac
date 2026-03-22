# gac — Agent Guidelines

This is a Rust CLI tool that generates AI-powered commit messages using Ollama.

## Build / Test / Lint Commands

```bash
# Build the project
cargo build

# Build release (optimized)
cargo build --release

# Run all tests
cargo test

# Run tests for a specific module
cargo test <module_name>

# Run a single test by name
cargo test test_name_here

# Run with output displayed
cargo test -- --nocapture

# Check formatting
cargo fmt --check

# Auto-fix formatting
cargo fmt

# Run clippy lints
cargo clippy --all-targets

# Run clippy with strict warnings
cargo clippy --all-targets -- -D warnings

# Build docs
cargo doc

# Run all checks (fmt + clippy + test)
cargo CI
```

Note: For Windows, commands are the same but run in PowerShell/CMD.

## Project Structure

```
gac/
├── src/
│   ├── main.rs      # Entry point, CLI parsing, orchestration
│   ├── config.rs    # Config loading (.gac.toml)
│   ├── diff.rs      # Git diff parsing and strategy selection
│   ├── git.rs       # Git operations (staged files, scopes)
│   ├── logging.rs   # Custom CLI logging formatter
│   ├── ollama.rs    # Ollama API client (streaming/non-streaming)
│   ├── prompt.rs    # Prompt building with Askama templates
│   └── stats.rs     # Generation stats display
├── templates/       # Askama templates for prompts
├── Cargo.toml
└── .gac.toml        # Project config (also in templates/)
```

## Code Style Guidelines

### Formatting
- Use `cargo fmt` with default rustfmt settings
- 4-space indentation, no tabs
- Maximum line length: 100 characters (soft limit, prefer readability)
- Use trailing commas in multi-line constructs

### Imports
- Group imports in order:
  1. Standard library (`std::`)
  2. Crate imports (alphabetical)
  3. `crate::` local imports
  4. `super::` parent imports
- Blank line between groups
- Use absolute paths within `use` statements

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
| Structs | PascalCase | `Config`, `FileDiff` |
| Enums | PascalCase | `Strategy`, `Approval` |
| Enum variants | PascalCase | `Strategy::Direct` |
| Functions | snake_case | `get_staged_files()` |
| Variables | snake_case | `max_diff_chars` |
| Constants | SCREAMING_SNAKE_CASE | `TOTAL_VRAM` |
| Type aliases | PascalCase | - |
| Traits | PascalCase | - |

### Types and Generics
- Prefer explicit type annotations for public API signatures
- Use idiomatic Rust collections: `Vec<T>`, `HashMap<K,V>`, `Option<T>`
- Use `&str` for string slices, `String` for owned strings
- Use `&[T]` for slice parameters, `&mut T` for mutation

### Error Handling
- Use `anyhow::Result<T>` for application errors with rich context
- Use `anyhow::bail!("message")` for early returns with errors
- Use `anyhow::Context` for adding context to `std::io` errors
- Never use `unwrap()` on user input or network responses
- Use `?` operator for error propagation

```rust
// Good
fn load_config() -> anyhow::Result<Config> {
    let raw = std::fs::read_to_string(path)?;
    let config: Config = toml::from_str(&raw)
        .context("Invalid config format")?;
    Ok(config)
}

// Good - early return
if config.is_empty() {
    anyhow::bail!("No staged changes found");
}

// Avoid unwrap on fallible operations
// let value = risky_function().unwrap(); // BAD
let value = risky_function()?; // GOOD
```

### Async Code
- Use `#[tokio::main]` for the main entry point
- Mark all async functions with `async fn`
- Use `futures_util::StreamExt` for stream processing
- Keep async boundaries minimal

### Logging and Debugging
- Use `tracing` for structured logging
- Log levels: `error!`, `warn!`, `info!`, `debug!`, `trace!`
- Use `debug!` for verbose internal details
- Use structured fields: `debug!(count = files.len(), "message")`
- Never log sensitive data (API keys, tokens, etc.)

### Documentation
- Document public API with doc comments (`///`)
- Use `//!` for module-level documentation
- Keep doc comments concise and useful
- No doc comments for private/internal functions

### Struct Definitions

```rust
// Use Derive macros for common traits
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub model: String,
    pub options: OllamaOptions,
}

// Implement Default for simple structs
impl Default for OllamaOptions {
    fn default() -> Self {
        Self {
            num_ctx: 2048,
            temperature: 0.2,
        }
    }
}
```

### Pattern Guidelines
- Match arms should be clearly separated
- Use `if let` for single-pattern matching
- Use early returns to reduce nesting
- Sort match arms alphabetically when order doesn't matter

## Template Guidelines (Askama)

Templates are in `templates/` with `.md` extension. Use Askama syntax:
- `{{ variable }}` - escaped output
- `{% if condition %}...{% endif %}` - conditionals
- `{% for item in items %}...{% endfor %}` - loops

Include templates with `include_str!("../templates/file.md")`.

## Context-Mode MCP Tools

This repository has context-mode MCP tools configured. Follow these routing rules:

### BLOCKED commands
- **curl/wget**: Use `context-mode_ctx_fetch_and_index()` or `context-mode_ctx_execute()`
- **Inline HTTP** (`fetch`, `requests`): Use sandbox equivalent
- **Direct web fetching**: Use `context-mode_ctx_fetch_and_index()`

### REDIRECTED tools
- **Shell (>20 lines)**: Use `context-mode_ctx_batch_execute()` or `context-mode_ctx_execute()`
- **File reading for analysis**: Use `context-mode_ctx_execute_file()`
- **grep/search**: Use `context-mode_ctx_execute()` in sandbox

### Tool selection hierarchy
1. `context-mode_ctx_batch_execute()` — Primary tool for commands + search
2. `context-mode_ctx_search()` — Query indexed content
3. `context-mode_ctx_execute()` — Sandbox execution
4. `context-mode_ctx_fetch_and_index()` — Web fetching

## Output Constraints
- Keep responses under 500 words
- Write code/configs to files, return only path + 1-line description
- Use descriptive source labels when indexing content
