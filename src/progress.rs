use std::{collections::HashMap, time::Duration};

use convert_case::{Case, Casing};
use indicatif::{HumanDuration, MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};

use crate::{config::ScopeHash, git::ScopeMatch};

const TICK_MS: u64 = 40;

pub enum BarStyleType {
    Spinner,
    SummarizeBar,
}

pub fn get_style(bar_type: BarStyleType) -> ProgressStyle {
    match bar_type {
        BarStyleType::Spinner => ProgressStyle::with_template("{spinner:.dim} {wide_msg}")
            .expect("valid template")
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✔️"]),
        BarStyleType::SummarizeBar => {
            ProgressStyle::with_template("{spinner:.dim} {msg} {bar:20.cyan/blue} {pos}/{len} ")
                .expect("valid template")
                .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏", "✔️"])
                .progress_chars("█░")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ProgressStep {
    Staging,
    Scope,
    Strategy,
    Context,
    SummarizeProgress,
    Generation,
}

#[derive(Clone)]
pub struct Progress {
    mp: MultiProgress,
    pbs: HashMap<ProgressStep, ProgressBar>,
}

impl Progress {
    fn create_spinner(&mut self, key: ProgressStep, msg: String) -> ProgressBar {
        let pb = self.mp.add(ProgressBar::new_spinner());
        self.pbs.insert(key, pb.clone());
        pb.set_style(get_style(BarStyleType::Spinner));
        pb.enable_steady_tick(Duration::from_millis(TICK_MS));
        pb.set_message(msg);
        pb
    }

    fn create_progress(
        &mut self,
        key: ProgressStep,
        len: u64,
        style: ProgressStyle,
    ) -> ProgressBar {
        let pb = self.mp.add(ProgressBar::new(len));
        self.pbs.insert(key, pb.clone());
        pb.set_style(style);
        pb.enable_steady_tick(Duration::from_millis(TICK_MS));
        pb
    }

    pub fn new() -> Self {
        Self {
            mp: MultiProgress::with_draw_target(ProgressDrawTarget::stderr_with_hz(25)),
            pbs: HashMap::new(),
        }
    }

    pub fn println(&self, msg: String) -> std::io::Result<()> {
        self.mp.println(msg)
    }

    pub fn start_staging(&mut self) {
        self.create_spinner(ProgressStep::Staging, "Reading staged files".to_string());
    }

    pub fn finish_staging(&mut self, all_staged: &[String], excluded: &[String]) {
        if let Some(pb) = self.pbs.get(&ProgressStep::Staging) {
            let msg = if !excluded.is_empty() {
                format!(
                    "✔️ {} files ({}) staged ({} filtered)",
                    console::style(all_staged.len()).yellow(),
                    all_staged
                        .iter()
                        .map(|f| console::style(f).yellow().to_string())
                        .collect::<Vec<String>>()
                        .join(", "),
                    excluded
                        .iter()
                        .map(|f| console::style(f).red().to_string())
                        .collect::<Vec<String>>()
                        .join(", ")
                )
            } else {
                format!(
                    "✔️ {} files ({}) staged",
                    console::style(all_staged.len()).yellow(),
                    all_staged
                        .iter()
                        .map(|f| console::style(f).yellow().to_string())
                        .collect::<Vec<String>>()
                        .join(", ")
                )
            };
            self.mp.suspend(|| {
                eprintln!("{msg}");
            });
            pb.finish_and_clear();
            self.mp.remove(pb);
            self.pbs.remove(&ProgressStep::Staging);
        }
    }

    pub fn start_scope(&mut self) {
        self.create_spinner(ProgressStep::Scope, "Matching scopes".to_string());
    }

    pub fn finish_scope(&mut self, scope_hash: &ScopeHash, scope_match: &ScopeMatch) {
        if let Some(pb) = self.pbs.get(&ProgressStep::Scope) {
            let msg = if !scope_hash.is_empty() {
                if !scope_match.matched.is_empty() {
                    format!(
                        "✔️ Dedected scopes: {}",
                        scope_match
                            .matched
                            .iter()
                            .map(|v| console::style(v).yellow().to_string())
                            .collect::<Vec<String>>()
                            .join(", ")
                    )
                } else {
                    format!(
                        "⚠️ No scopes auto-matched. Available scopes: {}",
                        scope_match
                            .unmatched
                            .iter()
                            .map(|v| console::style(v).yellow().to_string())
                            .collect::<Vec<String>>()
                            .join(", ")
                    )
                }
            } else {
                console::style("⚠️ Scopes not defined").red().to_string()
            };
            self.mp.suspend(|| {
                eprintln!("{msg}");
            });
            pb.finish_and_clear();
            self.mp.remove(pb);
            self.pbs.remove(&ProgressStep::Scope);
        }
    }

    pub fn start_strategy(&mut self) {
        self.create_spinner(ProgressStep::Strategy, "Deciding strategy".to_string());
    }

    pub fn finish_strategy(&mut self, strategy: &str, files: usize, chars: usize, tokens: usize) {
        if let Some(pb) = self.pbs.get(&ProgressStep::Strategy) {
            self.mp.suspend(|| {
                eprintln!(
                    "✔️ {} strategy selected for {} files with {} / {}",
                    console::style(strategy.to_case(Case::Title)).yellow(),
                    console::style(files).yellow(),
                    console::style(format!("{} char(s)", chars)).yellow(),
                    console::style(format!("{} token(s)", tokens)).yellow()
                );
            });
            pb.finish_and_clear();
            self.mp.remove(pb);
            self.pbs.remove(&ProgressStep::Strategy);
        }
    }

    pub fn start_context(&mut self) {
        self.create_spinner(ProgressStep::Context, "Creating context".to_string());
    }

    pub fn finish_context(&mut self) {
        if let Some(pb) = self.pbs.get(&ProgressStep::Context) {
            self.mp.suspend(|| {
                eprintln!("✔️ Context created");
            });
            pb.finish_and_clear();
            self.mp.remove(pb);
            self.pbs.remove(&ProgressStep::Context);
        }
    }

    pub fn start_summarize(&mut self, len: u64) {
        let pb = self.create_progress(
            ProgressStep::SummarizeProgress,
            len,
            get_style(BarStyleType::SummarizeBar),
        );
        pb.set_message("Summarizing");
    }

    pub fn inc_summarize(&self, delta: u64) {
        if let Some(pb) = self.pbs.get(&ProgressStep::SummarizeProgress) {
            pb.inc(delta);
        }
    }

    pub fn finish_summarize(&mut self) {
        if let Some(pb) = self.pbs.get(&ProgressStep::SummarizeProgress) {
            pb.finish_and_clear();
            self.mp.remove(pb);
            self.pbs.remove(&ProgressStep::SummarizeProgress);
        }
    }

    pub fn start_generation(&mut self) {
        self.create_spinner(
            ProgressStep::Generation,
            "💭 Commit message generation starting".to_string(),
        );
    }

    pub fn set_msg_generation(&self, msg: &str, reasoning: bool) {
        if let Some(pb) = self.pbs.get(&ProgressStep::Generation) {
            if reasoning {
                pb.set_message(format!("💭 Thinking about commit message: {msg}"));
            } else {
                pb.set_message(format!("💭 Generating commit message: {msg}"));
            }
        }
    }

    pub fn finish_generation(&mut self, duration: std::time::Instant) {
        if let Some(pb) = self.pbs.get(&ProgressStep::Generation) {
            self.mp.suspend(|| {
                eprintln!(
                    "✔️ Commit message generated in {}",
                    console::style(HumanDuration(duration.elapsed())).yellow()
                );
            });
            pb.finish_and_clear();
            self.mp.remove(pb);
            self.pbs.remove(&ProgressStep::Generation);
        }
    }
}
