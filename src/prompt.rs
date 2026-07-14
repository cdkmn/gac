use askama::Template; // bring trait in scope

/// A fully constructed prompt ready to send to the model.
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
        [single] => SingleScope { single }
            .render()
            .expect("failed to render single scope template"),
        candidates => ScopeTemplate { candidates }
            .render()
            .expect("failed to render scope template"),
    };

    CommitSystem {
        scope_rule: &scope_rule,
    }
    .render()
    .expect("failed to render commit system template")
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

// ── Retry prompt for validation failures ──────────────────────────────────

/// Build a prompt that asks the model to retry after a validation failure.
pub fn build_retry_prompt(
    context: &str,
    scope_candidates: &[&str],
    original: &str,
    error: &str,
) -> Prompt {
    let system = commit_system(scope_candidates);
    let user = format!(
        "Your previous response did not conform to the required format.\n\
         Error: {error}\n\
         Your response was:\n{original}\n\n\
         Please try again and output ONLY a valid conventional commit message.\n\n\
         Diff context:\n{context}"
    );

    Prompt { system, user }
}
