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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_score_file_source() {
        assert_eq!(score_file("src/main.rs"), 90);
        assert_eq!(score_file("lib/utils.py"), 90);
        assert_eq!(score_file("app/handler.ts"), 75);
    }

    #[test]
    fn test_score_file_test() {
        assert_eq!(score_file("src/main.test.ts"), 40);
        assert_eq!(score_file("tests/unit.rs"), 40);
        assert_eq!(score_file("src/__tests__/app.spec.js"), 40);
    }

    #[test]
    fn test_score_file_low_priority() {
        assert_eq!(score_file("package.json"), 20);
        assert_eq!(score_file("Cargo.lock"), 0);
        assert_eq!(score_file("README.md"), 20);
    }

    #[test]
    fn test_score_file_unknown() {
        assert_eq!(score_file("Makefile"), 50);
        assert_eq!(score_file("Dockerfile"), 50);
    }

    #[test]
    fn test_parse_diff_single_file() {
        let raw = "diff --git a/src/main.rs b/src/main.rs\nindex abc123..def456 100644\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1,3 +1,4 @@\n+new line\n fn main() {}\n";
        let files = parse_diff(raw);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/main.rs");
        assert!(files[0].content.contains("new line"));
    }

    #[test]
    fn test_parse_diff_multiple_files() {
        let raw = "diff --git a/src/main.rs b/src/main.rs\n--- a/src/main.rs\n+++ b/src/main.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n@@ -1 +1 @@\n-old readme\n+new readme\n";
        let files = parse_diff(raw);
        assert_eq!(files.len(), 2);
        // Should be sorted by priority (src/main.rs is higher than README.md)
        assert_eq!(files[0].path, "src/main.rs");
        assert_eq!(files[1].path, "README.md");
        assert!(files[0].priority > files[1].priority);
    }

    #[test]
    fn test_select_strategy_direct() {
        let files = vec![FileDiff {
            path: "src/main.rs".into(),
            priority: 90,
            char_count: 1000,
            content: "small diff".into(),
        }];
        assert_eq!(select_strategy(&files, 6000), Strategy::Direct);
    }

    #[test]
    fn test_select_strategy_summarize() {
        let files: Vec<FileDiff> = (0..5)
            .map(|i| FileDiff {
                path: format!("src/file{i}.rs"),
                priority: 90,
                char_count: 2000,
                content: "diff".into(),
            })
            .collect();
        // Total = 10000 > 6000, but only 5 files (< 20)
        assert_eq!(select_strategy(&files, 6000), Strategy::Summarize);
    }

    #[test]
    fn test_select_strategy_stat_only() {
        let files: Vec<FileDiff> = (0..25)
            .map(|i| FileDiff {
                path: format!("src/file{i}.rs"),
                priority: 90,
                char_count: 1000,
                content: "diff".into(),
            })
            .collect();
        // 25 files > 20 threshold
        match select_strategy(&files, 6000) {
            Strategy::StatOnly { top_n } => assert!(top_n >= 1),
            _ => panic!("expected StatOnly strategy"),
        }
    }

    #[test]
    fn test_build_summary_context() {
        let mut summaries = HashMap::new();
        summaries.insert("src/main.rs".into(), "Added main function".into());
        summaries.insert("src/lib.rs".into(), "Updated imports".into());

        let context = build_summary_context(&summaries);
        assert!(context.contains("src/lib.rs"));
        assert!(context.contains("src/main.rs"));
        assert!(context.contains("Per-file change summaries"));
    }
}
