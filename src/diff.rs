use std::collections::HashMap;
use tracing::debug;

use crate::{config::Config, git::ScopeMatch, llamaswap, prompt};

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
    pub priority: u8,    // 0 (low) – 100 (high); higher = include first
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
fn score_file(path: &str) -> u8 {
    let path_buf = std::path::Path::new(path);
    let name = path_buf
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(path);
    let ext = path_buf
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
        // Bump source files inside src/ or lib/ (use Path components for cross-platform)
        let dominated = path_buf
            .components()
            .any(|c| matches!(c, std::path::Component::Normal(s) if s == "src" || s == "lib"));
        if dominated {
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
    files.sort_by_key(|b| std::cmp::Reverse(b.priority));
    files
}

fn flush(files: &mut Vec<FileDiff>, path: &str, lines: &[&str]) {
    let content = lines.join("\n");
    files.push(FileDiff {
        priority: score_file(path),
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

pub async fn select_strategy(
    client: &llamaswap::Client,
    config: &Config,
    raw_diff: &str,
    scope_match: &ScopeMatch,
    stat: &str,
) -> anyhow::Result<(Strategy, String)> {
    let props = llamaswap::model_props(client, config).await?;
    let mut budget = props.default_generation_settings.n_ctx;
    let files = parse_diff(raw_diff);
    let body = build_direct_context(&files);
    let context = format!("=== Stat ===\n{stat}\n=== Diff ===\n{body}");
    let candidates = scope_match.best_candidates();
    let commit_prompt = prompt::build_commit_prompt(&context, &candidates);
    let tokens = llamaswap::token_counts(client, config, &commit_prompt).await?;

    debug!(
        tokens = tokens,
        file_count = files.len(),
        budget = budget,
        "selecting diff strategy"
    );

    if tokens < budget as usize {
        return Ok((Strategy::Direct, context));
    }

    if files.len() <= SUMMARIZE_THRESHOLD {
        return Ok((Strategy::Summarize, "".to_string()));
    }

    // How many top-priority files can we fit in the budget?
    let mut top_n = 0;

    for f in files {
        let tokens = llamaswap::tokenize(client, config, &f.content).await?;

        if tokens.len() > budget as usize {
            break;
        }

        budget -= tokens.len() as u32;
        top_n += 1;
    }

    Ok((
        Strategy::StatOnly {
            top_n: top_n.max(1),
        },
        "".to_string(),
    ))
}

// ── Context builders ──────────────────────────────────────────────────────

/// Build a direct diff string from the sorted file list.
/// Only used by `Strategy::Direct` (guaranteed to fit).
fn build_direct_context(files: &[FileDiff]) -> String {
    let mut out = String::new();

    for f in files {
        out.push_str(&f.content);
        out.push('\n');
    }

    out
}

/// Build a combined context from per-file summaries.
/// Each summary is a compact prose description generated by the model.
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

    // ── score_file ─────────────────────────────────────────────────────────

    #[test]
    fn score_low_priority_name_returns_zero() {
        assert_eq!(score_file("Cargo.lock"), 0);
        assert_eq!(score_file("package-lock.json"), 0);
        assert_eq!(score_file("go.sum"), 0);
    }

    #[test]
    fn score_source_in_src_is_ninety() {
        assert_eq!(score_file("src/main.rs"), 90);
        assert_eq!(score_file("src/auth/jwt.ts"), 90);
        assert_eq!(score_file("lib/utils.py"), 90);
    }

    #[test]
    fn score_source_in_src_windows_separators() {
        assert_eq!(score_file("src\\main.rs"), 90);
        assert_eq!(score_file("src\\auth\\jwt.ts"), 90);
        assert_eq!(score_file("lib\\utils.py"), 90);
    }

    #[test]
    fn score_source_outside_src_is_seventy_five() {
        assert_eq!(score_file("app/server.go"), 75);
    }

    #[test]
    fn score_test_files_are_forty() {
        assert_eq!(score_file("src/foo_test.rs"), 40);
        assert_eq!(score_file("src/bar.spec.ts"), 40);
        assert_eq!(score_file("__tests__/unit.js"), 40);
    }

    #[test]
    fn score_low_priority_ext_is_twenty() {
        assert_eq!(score_file("config.yaml"), 20);
        assert_eq!(score_file("README.md"), 20);
        assert_eq!(score_file("style.css"), 20);
    }

    #[test]
    fn score_unknown_ext_is_fifty() {
        assert_eq!(score_file("Makefile"), 50);
        assert_eq!(score_file("Dockerfile"), 50);
    }

    // ── parse_diff ─────────────────────────────────────────────────────────

    #[test]
    fn parse_diff_empty_string() {
        assert!(parse_diff("").is_empty());
    }

    #[test]
    fn parse_diff_single_file() {
        let raw = "\
diff --git a/src/main.rs b/src/main.rs
index abc..def 100644
--- a/src/main.rs
+++ b/src/main.rs
@@ -1,3 +1,3 @@
-fn old() {}
+fn new() {}
";
        let files = parse_diff(raw);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "src/main.rs");
        assert!(files[0].content.contains("fn new() {}"));
    }

    #[test]
    fn parse_diff_multiple_files() {
        let raw = "\
diff --git a/src/a.rs b/src/a.rs
index aaa..bbb 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1 +1 @@
-old
+new
diff --git a/src/b.rs b/src/b.rs
index ccc..ddd 100644
--- a/src/b.rs
+++ b/src/b.rs
@@ -1 +1 @@
-old2
+new2
";
        let files = parse_diff(raw);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[1].path, "src/b.rs");
    }

    #[test]
    fn parse_diff_extracts_path_from_b_prefix() {
        let raw = "\
diff --git a/old_name.rs b/new_name.rs
index aaa..bbb 100644
--- a/old_name.rs
+++ b/new_name.rs
";
        let files = parse_diff(raw);
        assert_eq!(files[0].path, "new_name.rs");
    }

    // ── build_direct_context ───────────────────────────────────────────────

    #[test]
    fn build_direct_context_concatenates_content() {
        let files = vec![
            FileDiff {
                path: "a.rs".into(),
                priority: 90,
                content: "diff-a".into(),
            },
            FileDiff {
                path: "b.rs".into(),
                priority: 75,
                content: "diff-b".into(),
            },
        ];
        let ctx = build_direct_context(&files);
        assert_eq!(ctx, "diff-a\ndiff-b\n");
    }

    #[test]
    fn build_direct_context_empty() {
        assert_eq!(build_direct_context(&[]), "");
    }

    // ── build_summary_context ──────────────────────────────────────────────

    #[test]
    fn build_summary_context_sorted_by_path() {
        let mut summaries = HashMap::new();
        summaries.insert("b.rs".into(), "changed b".into());
        summaries.insert("a.rs".into(), "changed a".into());
        let ctx = build_summary_context(&summaries);
        let a_pos = ctx.find("a.rs").unwrap();
        let b_pos = ctx.find("b.rs").unwrap();
        assert!(a_pos < b_pos, "a.rs should appear before b.rs");
    }

    #[test]
    fn build_summary_context_header() {
        let summaries = HashMap::new();
        let ctx = build_summary_context(&summaries);
        assert!(ctx.starts_with("Per-file change summaries:"));
    }

    // ── build_stat_context ─────────────────────────────────────────────────

    #[test]
    fn build_stat_context_includes_top_n_files() {
        let files = vec![
            FileDiff {
                path: "a.rs".into(),
                priority: 90,
                content: "diff-a".into(),
            },
            FileDiff {
                path: "b.rs".into(),
                priority: 75,
                content: "diff-b".into(),
            },
            FileDiff {
                path: "c.rs".into(),
                priority: 50,
                content: "diff-c".into(),
            },
        ];
        let ctx = build_stat_context("stat output", &files, 2);
        assert!(ctx.contains("diff-a"));
        assert!(ctx.contains("diff-b"));
        assert!(!ctx.contains("diff-c"));
        assert!(ctx.contains("1 lower-priority file(s) omitted"));
    }

    #[test]
    fn build_stat_context_no_skipped_when_top_n_covers_all() {
        let files = vec![FileDiff {
            path: "a.rs".into(),
            priority: 90,
            content: "diff-a".into(),
        }];
        let ctx = build_stat_context("stat", &files, 5);
        assert!(!ctx.contains("omitted"));
    }

    // ── Strategy Display ───────────────────────────────────────────────────

    #[test]
    fn strategy_display() {
        assert_eq!(Strategy::Direct.to_string(), "direct");
        assert_eq!(Strategy::Summarize.to_string(), "per-file summarize");
        assert_eq!(
            Strategy::StatOnly { top_n: 3 }.to_string(),
            "stat + top-3 files"
        );
    }
}
