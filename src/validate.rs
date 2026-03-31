use tracing::{debug, warn};

/// Validates that a commit message conforms to the Conventional Commits specification.
///
/// Returns `Ok(())` if valid, or `Err(String)` with a description of what's wrong.
pub fn validate_conventional_commit(message: &str) -> Result<(), String> {
    let trimmed = message.trim();

    if trimmed.is_empty() {
        return Err("Empty commit message".to_string());
    }

    // Extract the first line (subject line)
    let first_line = trimmed.lines().next().unwrap_or("");

    // Conventional commit regex: type(scope): subject
    // type is required, scope is optional
    let conventional_pattern = regex::Regex::new(
        r"^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([^)]+\))?: .{1,120}$",
    )
    .expect("valid regex");

    if !conventional_pattern.is_match(first_line) {
        // Provide specific feedback about what's wrong
        if !first_line.contains(": ") {
            return Err(
                "Missing ': ' separator. Expected format: type(scope): subject".to_string(),
            );
        }

        let parts: Vec<&str> = first_line.splitn(2, ": ").collect();
        let type_part = parts[0];

        // Check if type is valid (with or without scope)
        let type_with_scope = regex::Regex::new(
            r"^(feat|fix|docs|style|refactor|perf|test|build|ci|chore|revert)(\([^)]+\))?$",
        )
        .expect("valid regex");

        if !type_with_scope.is_match(type_part) {
            return Err(format!(
                "Invalid type '{type_part}'. Allowed: feat, fix, docs, style, refactor, perf, test, build, ci, chore, revert"
            ));
        }

        // Check subject length
        if parts.len() > 1 && parts[1].len() > 120 {
            return Err(format!(
                "Subject line too long ({} chars, max 120)",
                parts[1].len()
            ));
        }

        return Err(format!(
            "Does not match conventional commit format. Got: '{first_line}'"
        ));
    }

    // Additional checks
    if first_line.ends_with('.') {
        return Err("Subject line should not end with a period".to_string());
    }

    // Check for lowercase after colon
    if let Some(subject) = first_line.split_once(": ").map(|x| x.1) {
        if let Some(first_char) = subject.chars().next() {
            if first_char.is_uppercase() {
                warn!("subject starts with uppercase letter");
                // This is a warning, not an error — some teams prefer uppercase
            }
        }
    }

    debug!("commit message validation passed");
    Ok(())
}

/// Attempts to fix common conventional commit format issues.
///
/// Returns the corrected message, or the original if no fix is possible.
pub fn try_fix_commit_message(message: &str) -> String {
    let trimmed = message.trim().to_string();
    let first_line = trimmed.lines().next().unwrap_or("");

    // Try to extract type and subject
    if let Some(colon_pos) = first_line.find(": ") {
        let type_part = &first_line[..colon_pos];
        let subject = &first_line[colon_pos + 2..];

        // Clean up type part (remove parentheses if malformed)
        let cleaned_type = type_part
            .trim()
            .to_lowercase()
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '(' || *c == ')' || *c == '-')
            .collect::<String>();

        // Validate the cleaned type
        let valid_types = [
            "feat", "fix", "docs", "style", "refactor", "perf", "test", "build", "ci", "chore",
            "revert",
        ];
        let base_type = cleaned_type.split('(').next().unwrap_or("");

        if valid_types.contains(&base_type) {
            // Fix subject capitalization (lowercase first letter)
            let fixed_subject = if let Some(first_char) = subject.chars().next() {
                if first_char.is_uppercase() {
                    let mut chars = subject.chars();
                    chars.next().unwrap().to_lowercase().collect::<String>() + chars.as_str()
                } else {
                    subject.to_string()
                }
            } else {
                subject.to_string()
            };

            // Remove trailing period
            let fixed_subject = fixed_subject.strip_suffix('.').unwrap_or(&fixed_subject);

            let mut result = format!("{cleaned_type}: {fixed_subject}");

            // Append remaining lines if any
            let remaining_lines: Vec<&str> = trimmed.lines().skip(1).collect();
            if !remaining_lines.is_empty() {
                result.push('\n');
                result.push_str(&remaining_lines.join("\n"));
            }

            debug!(original = %first_line, fixed = %result.lines().next().unwrap_or(""), "auto-fixed commit message");
            return result;
        }
    }

    // If we can't fix it, return original
    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_conventional_commit() {
        assert!(validate_conventional_commit("feat(auth): add JWT support").is_ok());
        assert!(validate_conventional_commit("fix: resolve memory leak").is_ok());
        assert!(
            validate_conventional_commit("docs(readme): update installation instructions").is_ok()
        );
    }

    #[test]
    fn test_invalid_type() {
        assert!(validate_conventional_commit("update(auth): add JWT support").is_err());
        assert!(validate_conventional_commit("add: new feature").is_err());
    }

    #[test]
    fn test_missing_separator() {
        assert!(validate_conventional_commit("feat(auth) add JWT support").is_err());
    }

    #[test]
    fn test_trailing_period() {
        assert!(validate_conventional_commit("feat: add feature.").is_err());
    }

    #[test]
    fn test_empty_message() {
        assert!(validate_conventional_commit("").is_err());
        assert!(validate_conventional_commit("   ").is_err());
    }

    #[test]
    fn test_try_fix_capitalization() {
        let fixed = try_fix_commit_message("feat: Add new feature");
        assert_eq!(fixed, "feat: add new feature");
    }

    #[test]
    fn test_try_fix_trailing_period() {
        let fixed = try_fix_commit_message("fix: resolve bug.");
        assert_eq!(fixed, "fix: resolve bug");
    }

    #[test]
    fn test_try_fix_both() {
        let fixed = try_fix_commit_message("feat(auth): Add JWT support.");
        assert_eq!(fixed, "feat(auth): add JWT support");
    }
}
