use anyhow::{Context, Result};
use glob::Pattern;
use std::{collections::HashMap, process::Command};
use tracing::{debug, info, warn};

use crate::config::ScopeEntry;

// ── Staged diff ───────────────────────────────────────────────────────────

/// Returns `(stat, raw_diff)`. The stat is always the full summary;
/// the diff body is un-truncated so `diff::parse_diff` can split it properly.
pub fn get_staged_stat_and_diff(exclude_patterns: &[String]) -> anyhow::Result<(String, String)> {
    info!("reading staged diff");

    debug!(
        exclude_count = exclude_patterns.len(),
        "applying path exclusions"
    );

    let excludes = build_excludes(exclude_patterns);
    let refs: Vec<&str> = excludes.iter().map(|s| s.as_str()).collect();
    let stat = run_git_diff(&["diff", "--cached", "--stat", "--"], &refs)?;

    debug!(stat = %stat.trim(), "git diff --stat");

    if stat.trim().is_empty() {
        anyhow::bail!("No staged changes found. Run `git add` first.");
    }

    let diff = run_git_diff(
        &[
            "diff",
            "--cached",
            "--no-color",
            "--diff-algorithm=minimal",
            "-U3",
            "--",
        ],
        &refs,
    )?;

    if diff.trim().is_empty() {
        anyhow::bail!("Staged changes are only in filtered files.");
    }

    Ok((stat, diff))
}

// ── Staged file list ──────────────────────────────────────────────────────

/// Returns every file currently in the index, including excluded ones.
pub fn get_staged_files() -> Vec<String> {
    match Command::new("git")
        .args(["diff", "--cached", "--name-only"])
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.to_string())
            .collect(),
        Err(e) => {
            warn!(error = %e, "failed to run git diff --cached --name-only");
            Vec::new()
        }
    }
}

/// Returns files that were excluded by the exclude_patterns filter.
pub fn get_excluded_files(all: &[String], exclude_patterns: &[String]) -> Vec<String> {
    let excludes = build_excludes(exclude_patterns);
    let exclude_refs: Vec<&str> = excludes.iter().map(|s| s.as_str()).collect();

    let kept: std::collections::HashSet<String> = match Command::new("git")
        .args(
            ["diff", "--cached", "--name-only", "--"]
                .iter()
                .chain(exclude_refs.iter())
                .copied(),
        )
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .map(|l| l.to_string())
            .collect(),
        Err(e) => {
            warn!(error = %e, "failed to run git diff with excludes");
            std::collections::HashSet::new()
        }
    };

    all.iter().filter(|f| !kept.contains(*f)).cloned().collect()
}

// ── Scope detection ───────────────────────────────────────────────────────

/// Result of matching staged files against project scope definitions.
#[derive(Debug, Default)]
pub struct ScopeMatch {
    /// Scopes whose path globs matched at least one staged file.
    pub matched: Vec<String>,
    /// Scopes defined in the config but with no path patterns (manual-only).
    pub unmatched: Vec<String>,
}

impl ScopeMatch {
    /// Best guess for the prompt: matched scopes first, else all defined scopes.
    pub fn best_candidates(&self) -> Vec<&str> {
        if !self.matched.is_empty() {
            self.matched.iter().map(|s| s.as_str()).collect()
        } else {
            self.unmatched.iter().map(|s| s.as_str()).collect()
        }
    }
}

/// Match `staged_files` against every scope's glob patterns.
/// A scope matches if ANY staged file matches ANY of its patterns.
pub fn detect_scopes(staged_files: &[String], scopes: &HashMap<String, ScopeEntry>) -> ScopeMatch {
    let mut matched = Vec::new();
    let mut unmatched = Vec::new();
    // Sort for deterministic output
    let mut names: Vec<&String> = scopes.keys().collect();
    names.sort();

    for name in names {
        let entry = &scopes[name];
        let patterns = entry.paths();

        if patterns.is_empty() {
            unmatched.push(name.clone());
            continue;
        }

        let mut compiled = Vec::new();
        for p in patterns {
            match Pattern::new(p) {
                Ok(pat) => compiled.push(pat),
                Err(e) => warn!(
                    scope = %name,
                    pattern = %p,
                    error = %e,
                    "invalid glob pattern in scope definition — skipped"
                ),
            }
        }

        let hits = staged_files
            .iter()
            .any(|file| compiled.iter().any(|pat| pat.matches(file)));

        if hits {
            matched.push(name.clone());
        } else {
            unmatched.push(name.clone());
        }
    }

    debug!(
        defined  = scopes.len(),
        matched  = ?matched,
        "scope detection complete"
    );

    ScopeMatch { matched, unmatched }
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn build_excludes(patterns: &[String]) -> Vec<String> {
    patterns.iter().map(|p| format!(":(exclude){p}")).collect()
}

fn run_git_diff(base_args: &[&str], extra: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(base_args.iter().chain(extra.iter()).copied())
        .output()
        .context("Failed to run git diff")?;
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}
