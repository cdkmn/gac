use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::time::{Duration, Instant};

// ── Spinner frames ────────────────────────────────────────────────────────
//
// Two sets: a braille-dot kinetic spinner for active work, and a static
// symbol set for done/fail states.  Chosen to be legible on both light
// and dark terminals and to degrade gracefully in dumb terminals.

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const TICK_MS: u64 = 80;

// ── Shared MultiProgress ──────────────────────────────────────────────────
//
// All progress bars must share one MultiProgress so they don't collide
// when writing to the same stderr.  We expose it as a module-level
// constructor so callers never manage it directly.

/// Create a MultiProgress that writes to stderr.
/// Call once per run and pass it to the spinner/bar constructors.
pub fn multi() -> MultiProgress {
    MultiProgress::with_draw_target(ProgressDrawTarget::stderr())
}

// ── Summarize progress bar ────────────────────────────────────────────────
//
// Used during the Strategy::Summarize pass (one API call per file).
// Shows a compact progress bar with the current filename.
//
// Visual:
//   ⠹ [3/12] ██████░░░░░░░░░░░░ src/auth/jwt.rs

pub fn summarize_bar(mp: &MultiProgress, total: usize) -> ProgressBar {
    let pb = mp.add(ProgressBar::new(total as u64));
    pb.set_draw_target(ProgressDrawTarget::stderr());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan} [{pos}/{len}] {bar:20.green/dim} {wide_msg}")
            .expect("valid template")
            .tick_strings(SPINNER_FRAMES)
            .progress_chars("█░"),
    );
    pb.enable_steady_tick(Duration::from_millis(TICK_MS));
    pb
}

// ── Inline step spinner ───────────────────────────────────────────────────
//
// Lightweight single-line spinner for short operations that don't need
// a progress counter: reading the diff, querying /api/ps, etc.
//
// Visual:
//   ⠸ Reading staged diff…

pub fn step_spinner(mp: &MultiProgress, msg: &str) -> ProgressBar {
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_draw_target(ProgressDrawTarget::stderr());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.dim} {msg}")
            .expect("valid template")
            .tick_strings(SPINNER_FRAMES),
    );
    pb.set_message(msg.to_string());
    pb.enable_steady_tick(Duration::from_millis(TICK_MS));
    pb
}

// ── Finish helpers ────────────────────────────────────────────────────────
//
// Centralised so the checkmark/cross glyph and colour are consistent
// across every call site.

// ── Step Tracker ────────────────────────────────────────────────────────
//
// Persistent multi-line step indicator showing all pipeline steps.
// Completed steps show ✔, active shows ⠹ with detail, pending shows ○.
// The Generate step has embedded thinking→streaming transition.
//
// Visual:
//   ✔ ● Read diff        12 files, 2.4k chars
//   ✔ ● Parse & score    12 files scored
//   ✔ ● Select strategy  direct
//   ✔ ● Build prompt     342 tokens
//   ⠹ ● Generate         [██████░░░░░░] 42%  118 tok  42.3 tok/s
//     ○ Validate
//     ○ Commit

/// Step names for the pipeline.
pub const STEP_NAMES: &[&str] = &[
    "Read diff",
    "Parse & score",
    "Select strategy",
    "Build prompt",
    "Generate",
    "Validate",
    "Commit",
];

/// Index of the Generate step (special handling for thinking/streaming).
const GENERATE_STEP: usize = 4;

/// Step state tracking.
#[derive(Debug, Clone)]
pub enum StepState {
    Pending,
    Active,
    Done(String),
}

/// A single step in the pipeline.
#[derive(Debug, Clone)]
pub struct Step {
    name: &'static str,
    state: StepState,
}

/// Persistent multi-line step tracker.
pub struct StepTracker {
    pb: ProgressBar,
    steps: Vec<Step>,
    current: usize,
    max_tokens: u64,
    generate_start: Option<Instant>,
    thinking_start: Option<Instant>,
}

impl StepTracker {
    /// Create a new step tracker with all pipeline steps.
    /// Starts rendering immediately with the first step active.
    pub fn new(mp: &MultiProgress, max_tokens: u64) -> Self {
        let pb = mp.add(ProgressBar::new_spinner());
        pb.set_style(
            ProgressStyle::with_template("{spinner:.dim} {msg}")
                .expect("valid template")
                .tick_strings(SPINNER_FRAMES),
        );
        pb.enable_steady_tick(Duration::from_millis(TICK_MS));

        let steps: Vec<Step> = STEP_NAMES
            .iter()
            .enumerate()
            .map(|(i, &name)| Step {
                name,
                state: if i == 0 {
                    StepState::Active
                } else {
                    StepState::Pending
                },
            })
            .collect();

        let mut tracker = Self {
            pb,
            steps,
            current: 0,
            max_tokens,
            generate_start: None,
            thinking_start: None,
        };
        tracker.render();
        tracker
    }

    /// Mark current step as done with detail, advance to next.
    pub fn finish(&mut self, detail: impl Into<String>) {
        self.steps[self.current].state = StepState::Done(detail.into());
        self.current += 1;
        if self.current < self.steps.len() {
            self.steps[self.current].state = StepState::Active;
        }
        self.render();
    }

    /// Enter "thinking" sub-phase of Generate step.
    pub fn generate_thinking(&mut self) {
        self.thinking_start = Some(Instant::now());
        self.render();
    }

    /// Update Generate step during streaming phase.
    pub fn generate_streaming(&mut self, pct: u64, count: u64, speed: f64, elapsed: Duration) {
        self.steps[self.current].state = StepState::Active;
        self.render_streaming(pct, count, speed, elapsed);
    }

    /// Show a validation retry on the Validate step without advancing.
    /// Resets the Generate step back to Active so it can re-run.
    pub fn show_retry(&mut self, detail: impl Into<String>) {
        // Mark the current step (Validate) as Done with retry info
        self.steps[self.current].state = StepState::Done(detail.into());
        // Reset current back to Generate so generate_done can advance correctly
        self.current = GENERATE_STEP;
        self.steps[GENERATE_STEP].state = StepState::Active;
        self.render();
    }

    /// Mark Generate step as done with stats summary.
    pub fn generate_done(&mut self, output_tokens: u64, speed: f64, elapsed: Duration) {
        let detail = format!(
            "{} tok, {:.1} tok/s, {}",
            output_tokens,
            speed,
            fmt_elapsed(elapsed)
        );
        self.steps[self.current].state = StepState::Done(detail);
        self.current += 1;
        if self.current < self.steps.len() {
            self.steps[self.current].state = StepState::Active;
        }
        self.thinking_start = None;
        self.generate_start = None;
        self.render();
    }

    /// Clear the entire tracker from the terminal.
    pub fn clear(&self) {
        self.pb.finish_and_clear();
    }

    /// Build and display the multi-line step output.
    fn render(&mut self) {
        let mut lines = Vec::new();
        for (i, step) in self.steps.iter().enumerate() {
            let line = match &step.state {
                StepState::Pending => format!("  ○ {}", step.name),
                StepState::Active if i == GENERATE_STEP => {
                    // Generate step thinking phase
                    if let Some(start) = self.thinking_start {
                        let elapsed = start.elapsed();
                        if self.generate_start.is_none() {
                            self.generate_start = Some(start);
                        }
                        format!("⠹ ● Generate  thinking… {}", fmt_elapsed(elapsed))
                    } else {
                        "⠹ ● Generate".to_string()
                    }
                }
                StepState::Active => {
                    // Non-generate active step - need detail from finish() call
                    // This is a placeholder; actual detail is set via finish()
                    format!("⠹ ● {}", step.name)
                }
                StepState::Done(detail) => format!("✔ ● {}  {}", step.name, style_detail(detail)),
            };
            lines.push(line);
        }
        self.pb.set_message(lines.join("\n"));
    }

    /// Render Generate step in streaming mode (separate to avoid rebuild).
    fn render_streaming(&self, pct: u64, count: u64, speed: f64, elapsed: Duration) {
        let mut lines = Vec::new();
        for (i, step) in self.steps.iter().enumerate() {
            let line = if i == GENERATE_STEP && matches!(step.state, StepState::Active) {
                if self.max_tokens > 0 {
                    format!(
                        "⠹ ● Generate  [{bar}] {pct}%  {count} tok  {speed:.1} tok/s",
                        bar = build_bar(pct, 20)
                    )
                } else {
                    format!(
                        "⠹ ● Generate  {count} tok  {}  {speed:.1} tok/s",
                        fmt_elapsed(elapsed)
                    )
                }
            } else {
                match &step.state {
                    StepState::Pending => format!("  ○ {}", step.name),
                    StepState::Active => format!("⠹ ● {}", step.name),
                    StepState::Done(detail) => {
                        format!("✔ ● {}  {}", step.name, style_detail(detail))
                    }
                }
            };
            lines.push(line);
        }
        self.pb.set_message(lines.join("\n"));
    }
}

/// Build a Unicode progress bar string.
fn build_bar(pct: u64, width: usize) -> String {
    let filled = ((pct as f64 / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    format!("{}{}", "█".repeat(filled), "░".repeat(empty))
}

/// Format a duration as a compact clock: "1.2s", "12.3s", "1m04s".
pub fn fmt_elapsed(d: Duration) -> String {
    let s = d.as_secs_f64();
    if s < 60.0 {
        format!("{s:.1}s")
    } else {
        let m = s as u64 / 60;
        let rem = s as u64 % 60;
        format!("{m}m{rem:02}s")
    }
}

/// Format detail string with dim styling.
fn style_detail(detail: &str) -> String {
    // For now, return as-is; color is handled by indicatif template
    detail.to_string()
}

/// Finish with ✔ <msg> in green.
pub fn done(pb: &ProgressBar, msg: impl Into<String>) {
    pb.set_style(
        ProgressStyle::with_template("{prefix:.green.bold} {msg}").expect("valid template"),
    );
    pb.set_prefix("✔");
    pb.finish_with_message(msg.into());
}

/// Finish and erase the line entirely.
/// Use this before printing multi-line output (streaming tokens, stats).
pub fn clear(pb: &ProgressBar) {
    pb.finish_and_clear();
}
