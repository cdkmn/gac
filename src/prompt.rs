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

fn commit_system(scope_candidates: &[&str]) -> String {
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

fn commit_user(context: &str) -> String {
    format!("Please analyze the following diff and generate commit message based on the changes:\n\n{context}")
}

pub fn build_commit_prompt(context: &str, scope_candidates: &[&str]) -> Prompt {
    Prompt {
        system: commit_system(scope_candidates),
        user: commit_user(context),
    }
}

// ── Per-file summarization ────────────────────────────────────────────────

fn summary_system() -> &'static str {
    include_str!("../templates/summary_system.md")
}

fn summary_user(path: &str, diff: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── commit_system ─────────────────────────────────────────────────────

    #[test]
    fn commit_system_no_candidates_includes_empty_scope_rule() {
        let result = commit_system(&[]);
        assert!(result.contains("Scope"));
        // The empty_scope.md content should be present
        assert!(
            result.contains("Infer the most specific scope"),
            "should contain empty scope inference rule"
        );
    }

    #[test]
    fn commit_system_single_candidate() {
        let result = commit_system(&["auth"]);
        assert!(result.contains("auth"));
        assert!(result.contains("Scope"));
    }

    #[test]
    fn commit_system_multiple_candidates() {
        let result = commit_system(&["api", "auth", "db"]);
        assert!(result.contains("api"));
        assert!(result.contains("auth"));
        assert!(result.contains("db"));
        assert!(result.contains("Scope"));
    }

    #[test]
    fn commit_system_contains_allowed_types() {
        let result = commit_system(&[]);
        assert!(result.contains("feat"));
        assert!(result.contains("fix"));
        assert!(result.contains("chore"));
        assert!(result.contains("revert"));
    }

    // ── commit_user ───────────────────────────────────────────────────────

    #[test]
    fn commit_user_contains_context() {
        let result = commit_user("diff content here");
        assert!(result.contains("diff content here"));
        assert!(result.contains("commit message"));
    }

    // ── build_commit_prompt ───────────────────────────────────────────────

    #[test]
    fn build_commit_prompt_has_system_and_user() {
        let prompt = build_commit_prompt("some diff", &["api"]);
        assert!(!prompt.system.is_empty());
        assert!(prompt.user.contains("some diff"));
        assert!(prompt.system.contains("api"));
    }

    // ── summary_system ────────────────────────────────────────────────────

    #[test]
    fn summary_system_returns_non_empty_static() {
        let s = summary_system();
        assert!(!s.is_empty());
        assert!(s.contains("1-2 sentences"));
    }

    // ── summary_user ──────────────────────────────────────────────────────

    #[test]
    fn summary_user_contains_path_and_diff() {
        let result = summary_user("src/main.rs", "+added\n-removed");
        assert!(result.contains("src/main.rs"));
        assert!(result.contains("+added"));
        assert!(result.contains("-removed"));
    }

    // ── build_file_summary_prompt ─────────────────────────────────────────

    #[test]
    fn build_file_summary_prompt_has_system_and_user() {
        let prompt = build_file_summary_prompt("src/lib.rs", "diff here");
        assert!(!prompt.system.is_empty());
        assert!(prompt.user.contains("src/lib.rs"));
        assert!(prompt.user.contains("diff here"));
    }

    // ── build_retry_prompt ────────────────────────────────────────────────

    #[test]
    fn build_retry_prompt_includes_error_and_original() {
        let prompt = build_retry_prompt(
            "some diff",
            &["feat"],
            "update: stuff",
            "Invalid type 'update'",
        );
        assert!(prompt.user.contains("Invalid type 'update'"));
        assert!(prompt.user.contains("update: stuff"));
        assert!(prompt.user.contains("some diff"));
        assert!(prompt.system.contains("feat"));
    }
}
