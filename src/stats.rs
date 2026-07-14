// ── Raw stats captured from llama-swap ──────────────────────────────────

/// Token and timing stats from the final `/api/chat` done-chunk.
#[derive(Debug, Default)]
pub struct GenerationStats {
    /// Tokens in the prompt (system + user).
    pub input_tokens: u64,
    /// Tokens in the generated response.
    pub output_tokens: u64,
    /// Time spent evaluating the prompt (Millis).
    pub prompt_eval_ms: f64,
    /// Time spent generating the response (Millis).
    pub eval_ms: f64,
    /// Total end-to-end time including model load (Millis).
    pub total_ms: f64,
    pub tokens_per_second: f64,
    /// VRAM Stats
    pub vram_used_mb: Option<u32>,
    pub vram_total_mb: Option<u32>,
    pub vram_util_pct: Option<f64>,
}

impl GenerationStats {
    // ── Display ────────────────────────────────────────────────────────────

    pub fn print(&self) {
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
        eprintln!(
            "  {:20} {}",
            style("Speed").cyan().bold(),
            style(format!("{:.1} tok/s", self.tokens_per_second)).green(),
        );

        // Timing breakdown
        eprintln!(
            "  {:20} prompt {}ms  +  gen {}ms  =  total {}ms",
            style("Time").cyan().bold(),
            style(format!("{}", self.prompt_eval_ms)).yellow(),
            style(format!("{}", self.eval_ms)).green(),
            style(format!("{}", self.total_ms)).dim(),
        );

        // VRAM
        match self.vram_used_mb {
            Some(used) => match (self.vram_total_mb, self.vram_util_pct) {
                (Some(total), Some(pct)) => {
                    let bar = vram_bar(pct, 20);
                    eprintln!(
                        "  {:20} {} [{bar}] {:.0}%",
                        style("VRAM").cyan().bold(),
                        style(format!("{used:.0}/{total:.0} MB")).magenta(),
                        pct,
                    );
                }
                _ => {
                    eprintln!(
                        "  {:20} {}",
                        style("VRAM").cyan().bold(),
                        style(format!("{used:.0} MB used (total unknown)")).dim(),
                    );
                }
            },
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
