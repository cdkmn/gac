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

```text
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

| Type                | Convention           | Example                                |
| ------------------- | -------------------- | -------------------------------------- |
| Modules             | snake_case           | `git.rs`, `ollama.rs`                  |
| Structs/Enums       | PascalCase           | `Config`, `Strategy`                   |
| Enum variants       | PascalCase           | `Strategy::Direct`                     |
| Functions/Variables | snake_case           | `get_staged_files()`, `max_diff_chars` |
| Constants           | SCREAMING_SNAKE_CASE | `MAX_RETRIES`                          |

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

<!-- gitnexus:start -->

## GitNexus — Code Intelligence

This project is indexed by GitNexus as **gac** (266 symbols, 500 relationships, 14 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> If any GitNexus tool warns the index is stale, run `npx gitnexus analyze` in terminal first.

### Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `gitnexus_impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `gitnexus_detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `gitnexus_query({query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `gitnexus_context({name: "symbolName"})`.

### When Debugging

1. `gitnexus_query({query: "<error or symptom>"})` — find execution flows related to the issue
2. `gitnexus_context({name: "<suspect function>"})` — see all callers, callees, and process participation
3. `READ gitnexus://repo/gac/process/{processName}` — trace the full execution flow step by step
4. For regressions: `gitnexus_detect_changes({scope: "compare", base_ref: "main"})` — see what your branch changed

### When Refactoring

- **Renaming**: MUST use `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` first. Review the preview — graph edits are safe, text_search edits need manual review. Then run with `dry_run: false`.
- **Extracting/Splitting**: MUST run `gitnexus_context({name: "target"})` to see all incoming/outgoing refs, then `gitnexus_impact({target: "target", direction: "upstream"})` to find all external callers before moving code.
- After any refactor: run `gitnexus_detect_changes({scope: "all"})` to verify only expected files changed.

### Never Do

- NEVER edit a function, class, or method without first running `gitnexus_impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `gitnexus_rename` which understands the call graph.
- NEVER commit changes without running `gitnexus_detect_changes()` to check affected scope.

### Tools Quick Reference

| Tool             | When to use                   | Command                                                                 |
| ---------------- | ----------------------------- | ----------------------------------------------------------------------- |
| `query`          | Find code by concept          | `gitnexus_query({query: "auth validation"})`                            |
| `context`        | 360-degree view of one symbol | `gitnexus_context({name: "validateUser"})`                              |
| `impact`         | Blast radius before editing   | `gitnexus_impact({target: "X", direction: "upstream"})`                 |
| `detect_changes` | Pre-commit scope check        | `gitnexus_detect_changes({scope: "staged"})`                            |
| `rename`         | Safe multi-file rename        | `gitnexus_rename({symbol_name: "old", new_name: "new", dry_run: true})` |
| `cypher`         | Custom graph queries          | `gitnexus_cypher({query: "MATCH ..."})`                                 |

### Impact Risk Levels

| Depth | Meaning                               | Action                |
| ----- | ------------------------------------- | --------------------- |
| d=1   | WILL BREAK — direct callers/importers | MUST update these     |
| d=2   | LIKELY AFFECTED — indirect deps       | Should test           |
| d=3   | MAY NEED TESTING — transitive         | Test if critical path |

### Resources

| Resource                             | Use for                                  |
| ------------------------------------ | ---------------------------------------- |
| `gitnexus://repo/gac/context`        | Codebase overview, check index freshness |
| `gitnexus://repo/gac/clusters`       | All functional areas                     |
| `gitnexus://repo/gac/processes`      | All execution flows                      |
| `gitnexus://repo/gac/process/{name}` | Step-by-step execution trace             |

### Self-Check Before Finishing

Before completing any code modification task, verify:

1. `gitnexus_impact` was run for all modified symbols
2. No HIGH/CRITICAL risk warnings were ignored
3. `gitnexus_detect_changes()` confirms changes match expected scope
4. All d=1 (WILL BREAK) dependents were updated

### Keeping the Index Fresh

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

### CLI

| Task                                         | Read this skill file                                        |
| -------------------------------------------- | ----------------------------------------------------------- |
| Understand architecture / "How does X work?" | `.claude/skills/gitnexus/gitnexus-exploring/SKILL.md`       |
| Blast radius / "What breaks if I change X?"  | `.claude/skills/gitnexus/gitnexus-impact-analysis/SKILL.md` |
| Trace bugs / "Why is X failing?"             | `.claude/skills/gitnexus/gitnexus-debugging/SKILL.md`       |
| Rename / extract / split / refactor          | `.claude/skills/gitnexus/gitnexus-refactoring/SKILL.md`     |
| Tools, resources, schema reference           | `.claude/skills/gitnexus/gitnexus-guide/SKILL.md`           |
| Index, status, clean, wiki CLI commands      | `.claude/skills/gitnexus/gitnexus-cli/SKILL.md`             |

<!-- gitnexus:end -->
