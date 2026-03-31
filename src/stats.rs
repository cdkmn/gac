// ── Raw stats captured from Ollama ────────────────────────────────────────

/// Token and timing stats from the final `/api/chat` done-chunk.
#[derive(Debug, Default)]
pub struct GenerationStats {
    /// Tokens in the prompt (system + user).
    pub input_tokens: u64,
    /// Tokens in the generated response.
    pub output_tokens: u64,
    /// Time spent evaluating the prompt (nanoseconds from Ollama).
    pub prompt_eval_ns: u64,
    /// Time spent generating the response (nanoseconds from Ollama).
    pub eval_ns: u64,
    /// Total end-to-end time including model load (nanoseconds from Ollama).
    pub total_ns: u64,
    /// VRAM used by the model in bytes (from /api/ps, None if unavailable).
    pub vram_bytes: Option<u64>,
}

impl GenerationStats {
    /// Tokens per second for the generation phase only.
    pub fn tokens_per_second(&self) -> Option<f64> {
        let secs = self.eval_ns as f64 / 1_000_000_000.0;
        if secs > 0.0 {
            Some(self.output_tokens as f64 / secs)
        } else {
            None
        }
    }

    // ── Display ────────────────────────────────────────────────────────────

    pub fn print(&self, total_vram: u64) {
        use dialoguer::console::style;

        eprintln!(
            "\n{}",
            style("── Generation stats ──────────────────────").dim()
        );

        // Tokens
        eprintln!(
            "  {:20} {}  →  {}  (total: {})",
            style("Tokens").cyan().bold(),
            style(format!("{} in", self.input_tokens)).yellow(),
            style(format!("{} out", self.output_tokens)).green(),
            style(format!("{} total", self.input_tokens + self.output_tokens)).dim(),
        );

        // Speed
        if let Some(tps) = self.tokens_per_second() {
            eprintln!(
                "  {:20} {}",
                style("Speed").cyan().bold(),
                style(format!("{tps:.1} tok/s")).green(),
            );
        }

        // Timing breakdown
        let prompt_ms = self.prompt_eval_ns / 1_000_000;
        let gen_ms = self.eval_ns / 1_000_000;
        let total_ms = self.total_ns / 1_000_000;

        eprintln!(
            "  {:20} prompt {}ms  +  gen {}ms  =  total {}ms",
            style("Time").cyan().bold(),
            style(format!("{prompt_ms}")).yellow(),
            style(format!("{gen_ms}")).green(),
            style(format!("{total_ms}")).dim(),
        );

        // VRAM
        match self.vram_bytes {
            Some(used) => {
                let used_mb = used as f64 / 1_048_576.0;
                let total_mb = if total_vram > 0 {
                    total_vram as f64 / 1_048_576.0
                } else {
                    // Fallback: estimate from model size
                    used_mb * 1.2
                };
                let pct = if total_vram > 0 {
                    (used as f64 / total_vram as f64) * 100.0
                } else {
                    (used_mb / (used_mb * 1.2)) * 100.0
                };
                let bar = vram_bar(pct, 20);
                eprintln!(
                    "  {:20} {} [{bar}] {:.0}%",
                    style("VRAM").cyan().bold(),
                    style(format!("{used_mb:.0}/{total_mb:.0} MB")).magenta(),
                    pct,
                );
            }
            _ => {
                eprintln!(
                    "  {:20} {}",
                    style("VRAM").cyan().bold(),
                    style("unavailable (is the model loaded?)").dim(),
                );
            }
        }

        eprintln!(
            "{}",
            style("──────────────────────────────────────────").dim()
        );
    }
}

// ── Visual VRAM bar ───────────────────────────────────────────────────────
//
// Renders a small Unicode block bar, e.g.:  [████████████░░░░░░░░]  62%
// Colour shifts green → yellow → red as usage rises.

fn vram_bar(pct: f64, width: usize) -> String {
    use dialoguer::console::style;

    let filled = ((pct / 100.0) * width as f64).round() as usize;
    let filled = filled.min(width);
    let empty = width - filled;
    let bar = format!("{}{}", "█".repeat(filled), "░".repeat(empty));
    let coloured = match pct as u64 {
        0..=59 => style(bar).green(),
        60..=84 => style(bar).yellow(),
        _ => style(bar).red(),
    };

    coloured.to_string()
}
