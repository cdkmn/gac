use anyhow::{bail, Result};
use serde::Deserialize;
use std::{collections::HashMap, path::PathBuf};
use tracing::{debug, info};

// ── Scope definition ──────────────────────────────────────────────────────
/// A single named scope with optional glob patterns to auto-detect it.
///
/// TOML forms supported:
///
///   [scopes.api]                          # patterns only
///   paths = ["src/api/**", "src/routes/**"]
///
///   [scopes.auth]                         # full form
///   paths = ["src/auth/**"]
///
///   [scopes]                              # shorthand: bare string list
///   release = []                          # no paths → manual-only scope
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ScopeEntry {
    /// Full form: `{ paths = [...] }`
    Full { paths: Option<Vec<String>> },
    /// Shorthand: just a list of paths `["src/api/**"]`
    PathsOnly(Vec<String>),
}

impl ScopeEntry {
    pub fn paths(&self) -> &[String] {
        match self {
            ScopeEntry::Full { paths, .. } => paths.as_deref().unwrap_or(&[]),
            ScopeEntry::PathsOnly(v) => v,
        }
    }
}

// ── TOML file layout ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    /// llama-swap URL
    endpoint: Option<String>,
    /// Model name
    model: Option<String>,
    /// An upper bound for the number of tokens that can be generated for a completion, including visible output tokens and reasoning tokens.
    max_completion_tokens: Option<u64>,
    /// Patterns for excluding files
    exclude_patterns: Option<Vec<String>>,
    /// `[scopes]` table: scope name → entry
    scopes: Option<HashMap<String, ScopeEntry>>,
}

// ── Resolved config ───────────────────────────────────────────────────────
#[derive(Clone)]
pub struct Config {
    /// LLM name
    pub model: String,
    /// llama-swap base url
    pub endpoint: String,
    /// An upper bound for the number of tokens that can be generated for a completion, including visible output tokens and reasoning tokens.
    pub max_completion_tokens: u64,
    /// Excluded file patterns
    pub exclude_patterns: Vec<String>,
    /// All scopes defined in `.gac.toml`, keyed by name.
    pub scopes: HashMap<String, ScopeEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: "Gemma-4:E4B-QAT".to_string(),
            endpoint: "http://localhost:11438".to_string(),
            max_completion_tokens: 4096,
            exclude_patterns: default_excludes(),
            scopes: HashMap::new(),
        }
    }
}

fn default_excludes() -> Vec<String> {
    vec![
        "package-lock.json".into(),
        "yarn.lock".into(),
        "pnpm-lock.yaml".into(),
        "bun.lockb".into(),
        "Cargo.lock".into(),
        "poetry.lock".into(),
        "Gemfile.lock".into(),
        "composer.lock".into(),
        "go.sum".into(),
        "*.lock".into(),
    ]
}

fn find_project_config() -> Option<PathBuf> {
    let mut dir = std::env::current_dir().ok()?;

    loop {
        let candidate = dir.join(".gac.toml");

        if candidate.exists() {
            return Some(candidate);
        }

        if dir.join(".git").exists() || !dir.pop() {
            break;
        }
    }

    None
}

impl Config {
    pub fn load() -> Result<Self> {
        let mut base = Config::default();

        // 1. User-level: ~/.config/gac/config.toml
        if let Some(cfg_dir) = dirs::config_dir() {
            let user_cfg = cfg_dir.join("gac").join("config.toml");

            if user_cfg.exists() {
                info!(path = %user_cfg.display(), "loading user config");
                base.apply_file(&user_cfg)?;
            }
        }

        // 2. Project-level: .gac.toml (walks up from cwd to git root)
        if let Some(project_cfg) = find_project_config() {
            info!(path = %project_cfg.display(), "loading project config");
            base.apply_file(&project_cfg)?;
        }

        Ok(base)
    }

    fn apply_file(&mut self, path: &PathBuf) -> Result<()> {
        let raw = std::fs::read_to_string(path)?;
        let file: FileConfig = toml::from_str(&raw)
            .map_err(|e| anyhow::anyhow!("Invalid config at {}: {}", path.display(), e))?;

        if let Some(m) = file.model {
            self.model = m;
        }

        if let Some(e) = file.endpoint {
            self.endpoint = e;
        }

        if let Some(e) = file.max_completion_tokens {
            self.max_completion_tokens = e;
        }

        if let Some(v) = file.exclude_patterns {
            self.exclude_patterns = v;
        }

        // Scopes: project file REPLACES user-level scopes entirely so there
        // is no accidental cross-project scope bleed.
        if let Some(s) = file.scopes {
            self.scopes = s;
        }

        debug!(
            model = %self.model,
            max_completion_tokens = self.max_completion_tokens,
            scopes = self.scopes.len(),
            "config resolved"
        );

        Ok(())
    }

    pub fn apply_cli_overrides(&mut self, model: Option<String>) {
        if let Some(m) = model {
            self.model = m;
        }
    }

    /// Validate the resolved config. Call after loading and applying CLI overrides.
    pub fn validate(&self) -> Result<()> {
        if self.model.is_empty() {
            bail!("model name is empty — set it in .gac.toml or via --model");
        }

        if self.max_completion_tokens == 0 {
            bail!("max_completion_tokens must be > 0 (got 0)");
        }

        // Validate endpoint looks like a URL with a scheme
        let ep = self.endpoint.trim();
        if !ep.starts_with("http://") && !ep.starts_with("https://") {
            bail!("endpoint '{}' must start with http:// or https://", ep);
        }

        // Reject trailing slashes that often come from copy-paste
        if ep.ends_with('/') {
            bail!("endpoint '{}' has a trailing slash — remove it", ep);
        }

        Ok(())
    }
}
