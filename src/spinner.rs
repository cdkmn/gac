use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use std::time::Duration;

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

// ── Generation spinner ────────────────────────────────────────────────────
//
// Shown while waiting for the first token from Ollama.
// Cleared (not finished with a symbol) the moment streaming begins so
// the token output appears on a clean line.
//
// Visual:
//   ⠹ Waiting for model…
//   ⠸ Waiting for model…        ← ticks every 80 ms
//   [spinner clears]
//   💬 feat(auth): add JWT…     ← tokens stream in below

pub fn generation_spinner(mp: &MultiProgress, model: &str) -> ProgressBar {
    let pb = mp.add(ProgressBar::new_spinner());
    pb.set_draw_target(ProgressDrawTarget::stderr());
    pb.set_style(
        ProgressStyle::with_template("{spinner:.cyan.bold} {msg}")
            .expect("valid template")
            .tick_strings(SPINNER_FRAMES),
    );
    pb.set_message(format!("Waiting for {model}…"));
    pb.enable_steady_tick(Duration::from_millis(TICK_MS));
    pb
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

/// Finish with ✔ <msg> in green.
pub fn done(pb: &ProgressBar, msg: impl Into<String>) {
    pb.set_style(
        ProgressStyle::with_template("{prefix:.green.bold} {msg}").expect("valid template"),
    );
    pb.set_prefix("✔");
    pb.finish_with_message(msg.into());
}

/// Finish with ✖ <msg> in red.
pub fn fail(pb: &ProgressBar, msg: impl Into<String>) {
    pb.set_style(ProgressStyle::with_template("{prefix:.red.bold} {msg}").expect("valid template"));
    pb.set_prefix("✖");
    pb.finish_with_message(msg.into());
}

/// Finish and erase the line entirely.
/// Use this before printing multi-line output (streaming tokens, stats).
pub fn clear(pb: &ProgressBar) {
    pb.finish_and_clear();
}
