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

<!-- gitnexus:start -->
# GitNexus — Code Intelligence

This project is indexed by GitNexus as **gac** (197 symbols, 389 relationships, 9 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

## Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

## When Debugging

1. `gitnexus_query({query: "<error or symptom>"})` — find execution flows related to the issue
2. `gitnexus_context({name: "<suspect function>"})` — see all callers, callees, and process participation
3. `READ gitnexus://repo/gac/process/{processName}` — trace the full execution flow step by step
4. For regressions: `gitnexus_detect_changes({scope: "compare", base_ref: "main"})` — see what your branch changed

## When Refactoring

- **Renaming**: MUST use `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` first. Review the preview — graph edits are safe, text_search edits need manual review. Then run with `dry_run: false`.
- **Extracting/Splitting**: MUST run `gitnexus_context({name: "target"})` to see all incoming/outgoing refs, then `gitnexus_impact({target: "target", direction: "upstream"})` to find all external callers before moving code.
- After any refactor: run `gitnexus_detect_changes({scope: "all"})` to verify only expected files changed.

## Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

## Tools Quick Reference

| Tool | When to use | Command |
|------|-------------|---------|
| `query` | Find code by concept | `gitnexus_query({query: "auth validation"})` |
| `context` | 360-degree view of one symbol | `gitnexus_context({name: "validateUser"})` |
| `impact` | Blast radius before editing | `gitnexus_impact({target: "X", direction: "upstream"})` |
| `detect_changes` | Pre-commit scope check | `gitnexus_detect_changes({scope: "staged"})` |
| `rename` | Safe multi-file rename | `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` |
| `cypher` | Custom graph queries | `gitnexus_cypher({query: "MATCH ..."})` |

## Impact Risk Levels

| Depth | Meaning | Action |
|-------|---------|--------|
| d=1 | WILL BREAK — direct callers/importers | MUST update these |
| d=2 | LIKELY AFFECTED — indirect deps | Should test |
| d=3 | MAY NEED TESTING — transitive | Test if critical path |

## Resources

| Resource | Use for |
|----------|---------|
| `gitnexus://repo/gac/context` | Codebase overview, check index freshness |
| `gitnexus://repo/gac/clusters` | All functional areas |
| `gitnexus://repo/gac/processes` | All execution flows |
| `gitnexus://repo/gac/process/{name}` | Step-by-step execution trace |

## Self-Check Before Finishing

Before completing any code modification task, verify:
1. `gitnexus_impact` was run for all modified symbols
2. No HIGH/CRITICAL risk warnings were ignored
3. `gitnexus_detect_changes()` confirms changes match expected scope
4. All d=1 (WILL BREAK) dependents were updated

## Keeping the Index Fresh

After committing code changes, the GitNexus index becomes stale. Re-run analyze to update it:

```bash
npx gitnexus analyze
```

If the index previously included embeddings, preserve them by adding `--embeddings`:

```bash
npx gitnexus analyze --embeddings
```

To check whether embeddings exist, inspect `.gitnexus/meta.json` — the `stats.embeddings` field shows the count (0 means no embeddings). **Running analyze without `--embeddings` will delete any previously generated embeddings.**

> Claude Code users: A PostToolUse hook handles this automatically after `git commit` and `git merge`.

## CLI

| Task | Read this skill file |
|------|---------------------|
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md` |
| Blast radius / "What breaks if I change X?" | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?" | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md` |
| Rename / extract / split / refactor | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md` |
| Tools, resources, schema reference | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md` |
| Index, status, clean, wiki CLI commands | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md` |

<!-- gitnexus:end -->
