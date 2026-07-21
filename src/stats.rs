// ── Raw stats captured from llama-swap ──────────────────────────────────

use std::time::Duration;

use console::style;
use indicatif::HumanDuration;

/// Token and timing stats from the final `/api/chat` done-chunk.
#[derive(Debug, Default)]
pub struct GenerationStats {
    /// Tokens in the prompt (system + user).
    pub input_tokens: u64,
    /// Tokens in the generated response.
    pub output_tokens: u64,
    /// Time spent evaluating the prompt (Millisecond).
    pub prompt_eval_ms: f64,
    /// Time spent generating the response (Millisecond).
    pub eval_ms: f64,
    /// Total end-to-end time including model load (Millisecond).
    pub total_ms: f64,
    pub tokens_per_second: f64,
    /// VRAM Stats
    pub vram_used_mb: Option<u32>,
    pub vram_total_mb: Option<u32>,
    pub vram_util_pct: Option<f64>,
}

impl GenerationStats {
    pub fn print(&self) {
        let title = format!("── Generation Stats {}", "─".repeat(35));
        eprintln!("\n{}", style(title).dim());

        // VRAM
        match self.vram_used_mb {
            Some(used) => match (self.vram_total_mb, self.vram_util_pct) {
                (Some(total), Some(pct)) => {
                    let bar = vram_bar(pct, 20);
                    eprintln!(
                        " {:10} {} [{bar}] {:.0}%",
                        style("VRAM").cyan().bold(),
                        style(format!("{used:.0}/{total:.0} MB")).magenta(),
                        pct,
                    );
                }
                _ => {
                    eprintln!(
                        " {:10} {}",
                        style("VRAM").cyan().bold(),
                        style(format!("{used:.0} MB used (total unknown)")).dim(),
                    );
                }
            },
            _ => {
                eprintln!(
                    " {:10} {}",
                    style("VRAM").cyan().bold(),
                    style("unavailable (is the model loaded?)").dim(),
                );
            }
        }

        // Timing breakdown
        eprintln!(
            " {:10} Prompt: {} + Gen: {} = Total: {}",
            style("Time").cyan().bold(),
            style(format!(
                "{:#}",
                HumanDuration(Duration::from_micros((self.prompt_eval_ms * 1000.0) as u64))
            ))
            .yellow(),
            style(format!(
                "{:#}",
                HumanDuration(Duration::from_micros((self.eval_ms * 1000.0) as u64))
            ))
            .green(),
            style(format!(
                "{:#}",
                HumanDuration(Duration::from_micros((self.total_ms * 1000.0) as u64))
            ))
            .cyan(),
        );

        // Tokens
        eprintln!(
            " {:10} {} → {} ({}) ({})",
            style("Tokens").cyan().bold(),
            style(format!("{} In", self.input_tokens)).yellow(),
            style(format!("{} Out", self.output_tokens)).green(),
            style(format!("{:.1} tok/s", self.tokens_per_second)).cyan(),
            style(format!("{} Total", self.input_tokens + self.output_tokens)).dim(),
        );

        eprintln!("{}", style("─".repeat(55)).dim());
    }
}

/// Renders a small Unicode block bar, e.g.: [████████████░░░░░░░░] 62%
/// Color shifts green → yellow → red as usage rises.
fn vram_bar(pct: f64, width: usize) -> String {
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
