use askama::Template; // bring trait in scope

/// A fully constructed prompt ready to send to Ollama.
pub struct Prompt {
    pub system: String,
    pub user: String,
}

#[derive(Template)]
#[template(path = "commit_system.md")]
struct CommitSystem<'a> {
    scope_rule: &'a str,
}

#[derive(Template)]
#[template(path = "single_scope.md")]
struct SingleScope<'a> {
    single: &'a str,
}

#[derive(Template)]
#[template(path = "scopes.md")]
struct ScopeTemplate<'a> {
    candidates: &'a [&'a str],
}

// ── Commit message generation ─────────────────────────────────────────────

/// System prompt for the final commit generation pass.
pub fn commit_system(scope_candidates: &[&str]) -> String {
    let scope_rule = match scope_candidates {
        [] => include_str!("../templates/empty_scope.md").to_string(),
        [single] => SingleScope { single }.render().unwrap(),
        candidates => ScopeTemplate { candidates }.render().unwrap(),
    };

    CommitSystem {
        scope_rule: &scope_rule,
    }
    .render()
    .unwrap()
}

/// User message for the final commit generation pass.
/// Kept minimal — all rules live in the system prompt.
pub fn commit_user(context: &str) -> String {
    format!("Please analyze the following diff and generate commit message based on the changes:\n\n{context}")
}

pub fn build_commit_prompt(context: &str, scope_candidates: &[&str]) -> Prompt {
    Prompt {
        system: commit_system(scope_candidates),
        user: commit_user(context),
    }
}

// ── Per-file summarization ────────────────────────────────────────────────

/// System prompt for the cheap per-file summarization pass.
///
/// Intentionally much shorter than the commit system prompt — this pass runs
/// N times (once per file) and feeds into the commit pass, not the user.
/// Precision and brevity matter more than style enforcement here.
pub fn summary_system() -> &'static str {
    include_str!("../templates/summary_system.md")
}

/// User message for the per-file summarization pass.
pub fn summary_user(path: &str, diff: &str) -> String {
    format!("Summarize the changes in `{path}`:\n\n{diff}")
}

pub fn build_file_summary_prompt(path: &str, diff: &str) -> Prompt {
    Prompt {
        system: summary_system().to_string(),
        user: summary_user(path, diff),
    }
}
