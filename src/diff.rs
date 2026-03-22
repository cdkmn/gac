use std::collections::HashMap;
use tracing::debug;

// ── Priority scoring ──────────────────────────────────────────────────────
//
// High priority  → source code the model should read carefully
// Low priority   → generated / config / docs the model can skip or skim

static HIGH_PRIORITY_EXT: &[&str] = &[
    "rs", "go", "py", "ts", "tsx", "js", "jsx", "c", "cpp", "h", "hpp", "java", "kt", "swift",
    "cs", "rb", "php", "scala", "zig", "ex", "exs",
];
static LOW_PRIORITY_EXT: &[&str] = &[
    "json", "yaml", "yml", "toml", "xml", "ini", "cfg", "md", "txt", "rst", "html", "css", "scss",
    "less", "svg", "png", "jpg", "jpeg", "gif", "ico", "woff", "lock", "sum", "snap",
];
static LOW_PRIORITY_NAMES: &[&str] = &[
    "package-lock.json",
    "yarn.lock",
    "pnpm-lock.yaml",
    "bun.lockb",
    "Cargo.lock",
    "poetry.lock",
    "Gemfile.lock",
    "composer.lock",
    "go.sum",
    "go.mod",
];

const SUMMARIZE_THRESHOLD: usize = 20; // max files before switching to StatOnly

// ── Per-file diff chunk ────────────────────────────────────────────────────
#[derive(Debug, Clone)]
pub struct FileDiff {
    pub path: String,
    pub priority: u8, // 0 (low) – 100 (high); higher = include first
    pub char_count: usize,
    pub content: String, // full diff for this file including header
}

// ── Strategy ──────────────────────────────────────────────────────────────
#[derive(Debug, Clone, PartialEq)]
pub enum Strategy {
    /// Diff fits in the context window — send it as-is.
    Direct,
    /// Too large: summarize each file chunk in a separate call, then
    /// combine the per-file summaries into one commit prompt.
    Summarize,
    /// Massive diff (many files): use `--stat` + highest-priority file
    /// diffs only; everything else is described by its filename.
    StatOnly { top_n: usize },
}

impl std::fmt::Display for Strategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Strategy::Direct => write!(f, "direct"),
            Strategy::Summarize => write!(f, "per-file summarize"),
            Strategy::StatOnly { top_n } => write!(f, "stat + top-{top_n} files"),
        }
    }
}

/// Score a file path 0-100.
pub fn score_file(path: &str) -> u8 {
    let name = std::path::Path::new(path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    if LOW_PRIORITY_NAMES.contains(&name) {
        return 0;
    }

    // Test files: useful but lower priority than production source
    if path.contains("test") || path.contains("spec") || path.contains("__tests__") {
        return 40;
    }

    if HIGH_PRIORITY_EXT.contains(&ext.as_str()) {
        // Bump source files inside src/ or lib/
        if path.starts_with("src/") || path.starts_with("lib/") {
            return 90;
        }

        return 75;
    }

    if LOW_PRIORITY_EXT.contains(&ext.as_str()) {
        return 20;
    }

    50 // unknown type — neutral
}

// ── Diff parser ───────────────────────────────────────────────────────────

/// Parse a raw `git diff` string into per-file chunks.
pub fn parse_diff(raw: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut current_path = String::new();
    let mut current_lines: Vec<&str> = Vec::new();

    for line in raw.lines() {
        if line.starts_with("diff --git ") {
            // Flush previous file
            if !current_path.is_empty() {
                flush(&mut files, &current_path, &current_lines);
            }
            // Extract path from `diff --git a/foo b/foo`
            current_path = extract_path(line);
            current_lines = vec![line];
        } else {
            current_lines.push(line);
        }
    }

    // Flush last file
    if !current_path.is_empty() {
        flush(&mut files, &current_path, &current_lines);
    }

    // Sort highest priority first
    files.sort_by(|a, b| b.priority.cmp(&a.priority));
    files
}

fn flush(files: &mut Vec<FileDiff>, path: &str, lines: &[&str]) {
    let content = lines.join("\n");
    files.push(FileDiff {
        priority: score_file(path),
        char_count: content.len(),
        content,
        path: path.to_string(),
    });
}

fn extract_path(line: &str) -> String {
    // `diff --git a/src/main.rs b/src/main.rs` → `src/main.rs`
    line.split_whitespace()
        .nth(3) // "b/src/main.rs"
        .unwrap_or("")
        .trim_start_matches("b/")
        .to_string()
}

// ── Strategy selector ─────────────────────────────────────────────────────

pub fn select_strategy(files: &[FileDiff], max_chars: usize) -> Strategy {
    let total: usize = files.iter().map(|f| f.char_count).sum();

    debug!(
        total_chars = total,
        file_count = files.len(),
        budget = max_chars,
        "selecting diff strategy"
    );

    if total <= max_chars {
        return Strategy::Direct;
    }

    if files.len() <= SUMMARIZE_THRESHOLD {
        return Strategy::Summarize;
    }

    // How many top-priority files can we fit in the budget?
    let mut budget = max_chars;
    let mut top_n = 0;

    for f in files {
        if f.char_count > budget {
            break;
        }

        budget -= f.char_count;
        top_n += 1;
    }

    Strategy::StatOnly {
        top_n: top_n.max(1),
    }
}

// ── Context builders ──────────────────────────────────────────────────────

/// Build a direct diff string from the sorted file list, respecting max_chars.
/// Only used by `Strategy::Direct` (guaranteed to fit, but guard anyway).
pub fn build_direct_context(files: &[FileDiff], max_chars: usize) -> String {
    let mut out = String::new();

    for f in files {
        if out.len() + f.char_count > max_chars {
            break;
        }

        out.push_str(&f.content);
        out.push('\n');
    }

    out
}

/// Build a combined context from per-file summaries.
/// Each summary is a compact prose description generated by Ollama.
pub fn build_summary_context(summaries: &HashMap<String, String>) -> String {
    let mut out = String::from("Per-file change summaries:\n\n");
    let mut paths: Vec<&String> = summaries.keys().collect();
    paths.sort();

    for path in paths {
        out.push_str(&format!("• {path}:\n  {}\n\n", summaries[path]));
    }

    out
}

/// Build a stat-only context: full --stat + top-N file diffs.
pub fn build_stat_context(stat: &str, files: &[FileDiff], top_n: usize) -> String {
    let mut out = format!("=== Stat (all changed files) ===\n{stat}\n\n");
    out.push_str("=== Full diff (highest-priority files only) ===\n");

    for f in files.iter().take(top_n) {
        out.push_str(&f.content);
        out.push('\n');
    }

    let skipped = files.len().saturating_sub(top_n);

    if skipped > 0 {
        out.push_str(&format!(
            "\n[{skipped} lower-priority file(s) omitted — visible in --stat above]\n"
        ));
    }

    out
}
