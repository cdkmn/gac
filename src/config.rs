use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::PathBuf};
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OllamaOptions {
    pub num_ctx: u32,
    pub num_predict: u32,
    pub temperature: f32,
    pub top_p: f32,
    pub num_gpu: i32,
}

impl Default for OllamaOptions {
    fn default() -> Self {
        Self {
            num_ctx: 2048,
            num_predict: 256,
            temperature: 0.2,
            top_p: 0.9,
            num_gpu: 999,
        }
    }
}

// ── Scope definition ──────────────────────────────────────────────────────
/// A single named scope with optional glob patterns to auto-detect it.
///
/// TOML forms supported:
///
///   [scopes.api]                          # patterns only
///   paths = ["src/api/**", "src/routes/**"]
///
///   [scopes.auth]                         # patterns + description
///   paths       = ["src/auth/**"]
///   description = "Authentication & authorization"
///
///   [scopes]                              # shorthand: bare string list
///   release = []                          # no paths → manual-only scope
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum ScopeEntry {
    /// Full form: `{ paths = [...], description = "..." }`
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
#[derive(Debug, Deserialize)]
struct ModelConfig {
    name: Option<String>,
    ollama_url: Option<String>,
    think: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct DiffConfig {
    max_chars: Option<usize>,
    exclude_patterns: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default)]
struct FileConfig {
    model: Option<ModelConfig>,
    options: Option<OllamaOptions>,
    diff: Option<DiffConfig>,
    /// `[scopes]` table: scope name → entry
    scopes: Option<HashMap<String, ScopeEntry>>,
}

// ── Resolved config ───────────────────────────────────────────────────────
pub struct Config {
    pub model: String,
    pub ollama_url: String,
    pub think: bool,
    pub options: OllamaOptions,
    pub max_diff_chars: usize,
    pub exclude_patterns: Vec<String>,
    /// All scopes defined in `.gac.toml`, keyed by name.
    pub scopes: HashMap<String, ScopeEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            model: "cogito:8b".to_string(),
            ollama_url: "http://localhost:11434".to_string(),
            think: false,
            options: OllamaOptions::default(),
            max_diff_chars: 6000,
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
            if let Some(v) = m.name {
                self.model = v;
            }

            if let Some(v) = m.ollama_url {
                self.ollama_url = v;
            }

            if let Some(v) = m.think {
                self.think = v;
            }
        }

        if let Some(o) = file.options {
            self.options = o;
        }

        if let Some(d) = file.diff {
            if let Some(v) = d.max_chars {
                self.max_diff_chars = v;
            }

            if let Some(v) = d.exclude_patterns {
                self.exclude_patterns = v;
            }
        }

        // Scopes: project file REPLACES user-level scopes entirely so there
        // is no accidental cross-project scope bleed.
        if let Some(s) = file.scopes {
            self.scopes = s;
        }

        debug!(
            model       = %self.model,
            num_ctx     = self.options.num_ctx,
            max_chars   = self.max_diff_chars,
            scopes      = self.scopes.len(),
            "config resolved"
        );

        Ok(())
    }

    pub fn apply_cli_overrides(&mut self, model: Option<String>, num_ctx: Option<u32>) {
        if let Some(m) = model {
            self.model = m;
        }

        if let Some(c) = num_ctx {
            self.options.num_ctx = c;
        }
    }
}
