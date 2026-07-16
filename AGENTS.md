# gac — Agent Guidelines

Rust CLI tool that generates AI-powered commit messages using llama-swap (llama.cpp proxy).

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
```

There is no `cargo CI` alias — run fmt, clippy, and test as separate steps.

Windows: same commands in PowerShell/CMD.

## Project Structure

```text
gac/
├── src/
│   ├── main.rs       # Entry point, CLI parsing, orchestration
│   ├── config.rs     # Config loading (.gac.toml), scope definitions
│   ├── crypto.rs     # AES-256-GCM + Argon2id API key encryption
│   ├── diff.rs       # Git diff parsing, priority scoring, strategy selection
│   ├── git.rs        # Git operations (staged files, scopes, excludes)
│   ├── logging.rs    # Custom CLI logging formatter
│   ├── llamaswap.rs  # llama-swap API client (streaming, tokenize/detokenize, retry)
│   ├── prompt.rs     # Prompt building with Askama templates
│   ├── spinner.rs    # Progress spinners/bars (indicatif)
│   ├── stats.rs      # Generation stats display (tokens, VRAM, timing)
│   └── validate.rs   # Conventional commit validation + auto-fix
├── templates/        # Askama templates for system prompts (.md extension)
├── Cargo.toml
├── ARCHITECTURE.md   # Detailed architecture docs (GitNexus-generated)
└── .gac.toml         # Project config
```

## Key Architecture

- **Diff strategies**: Direct (fits context) → Summarize (per-file API calls) → StatOnly (top-N files). The threshold is 20 files before StatOnly kicks in.
- **Config cascade**: defaults → user config (`~/.config/gac/config.toml`) → project config (`.gac.toml`). CLI only overrides `model` via `--model`.
- **Commit validation**: regex check → up to 2 retries with error context → auto-fix fallback → user approval.
- **llama-swap API**: Uses `/v1/chat/completions` for generation, plus `/tokenize`, `/detokenize`, and `/props` endpoints. Not Ollama-compatible.

## Conventions

- **Templates**: Askama templates live in `templates/` with `.md` extension. Include via `include_str!("../templates/file.md")`.
- **Imports**: Group: std → external crates (alphabetical) → `crate::` local imports. Blank line between groups.
- **Errors**: `anyhow::Result<T>` throughout, `bail!()` for early returns, `?` for propagation. No `unwrap()` on user input or network responses.
- **Logging**: `tracing` crate — `error!`, `warn!`, `info!`, `debug!` with structured fields. Never log sensitive data.
- **Release profile**: Aggressive optimization — `strip = true`, `lto = true`, `panic = "abort"`, `codegen-units = 1`.

## Gotchas

- `.gac.toml` is flat (top-level `endpoint`, `model`, `max_completion_tokens`), not nested `[model]`/`[options]` sections as the README suggests.
- The `gac init` command copies `templates/gac.toml` as the default config — edit that template file to change defaults.
- The `--print` flag outputs the generated message to stdout without committing or showing the approval dialog.
- `dialoguer::Editor` is hardcoded to `nvim` in the approval flow.

<!-- gitnexus:start -->

## GitNexus — Code Intelligence

This project is indexed by GitNexus as **gac** (379 symbols, 681 relationships, 33 execution flows). Use the GitNexus MCP tools to understand code, assess impact, and navigate safely.

> Index stale? Run `node .gitnexus/run.cjs analyze` from the project root — it auto-selects an available runner. No `.gitnexus/run.cjs` yet? `npx gitnexus analyze` (npm 11 crash → `npm i -g gitnexus`; #1939).

### Always Do

- **MUST run impact analysis before editing any symbol.** Before modifying a function, class, or method, run `impact({target: "symbolName", direction: "upstream"})` and report the blast radius (direct callers, affected processes, risk level) to the user.
- **MUST run `detect_changes()` before committing** to verify your changes only affect expected symbols and execution flows. For regression review, compare against the default branch: `detect_changes({scope: "compare", base_ref: "main"})`.
- **MUST warn the user** if impact analysis returns HIGH or CRITICAL risk before proceeding with edits.
- When exploring unfamiliar code, use `query({search_query: "concept"})` to find execution flows instead of grepping. It returns process-grouped results ranked by relevance.
- When you need full context on a specific symbol — callers, callees, which execution flows it participates in — use `context({name: "symbolName"})`.
- For security review, `explain({target: "fileOrSymbol"})` lists taint findings (source→sink flows; needs `analyze --pdg`).

### Never Do

- NEVER edit a function, class, or method without first running `impact` on it.
- NEVER ignore HIGH or CRITICAL risk warnings from impact analysis.
- NEVER rename symbols with find-and-replace — use `rename` which understands the call graph.
- NEVER commit changes without running `detect_changes()` to check affected scope.

### Resources

| Resource                             | Use for                                  |
| ------------------------------------ | ---------------------------------------- |
| `gitnexus://repo/gac/context`        | Codebase overview, check index freshness |
| `gitnexus://repo/gac/clusters`       | All functional areas                     |
| `gitnexus://repo/gac/processes`      | All execution flows                      |
| `gitnexus://repo/gac/process/{name}` | Step-by-step execution trace             |

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
