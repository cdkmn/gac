use tracing::Level;
use tracing_subscriber::{
    fmt::{format::Writer, FmtContext, FormatEvent, FormatFields},
    registry::LookupSpan,
    EnvFilter,
};

// ── Log level ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    /// Only errors.
    Quiet,
    /// Errors + warnings (default).
    Normal,
    /// + info messages: config loading, strategy selection, scope detection.
    Verbose,
    /// + debug messages: per-chunk details, raw API fields.
    Debug,
}

impl LogLevel {
    fn to_filter(self) -> &'static str {
        match self {
            LogLevel::Quiet => "error",
            LogLevel::Normal => "warn",
            LogLevel::Verbose => "info",
            LogLevel::Debug => "debug",
        }
    }
}

// ── Custom event formatter ────────────────────────────────────────────────
//
// Default tracing-subscriber output is designed for servers:
//   2024-01-15T10:30:00.123Z  INFO gac::git: reading diff
//
// For a CLI we want something much leaner:
//   ● reading staged diff
//   ⚠ no scope matched
//   ✖ git diff failed: ...
//
// We use a leading symbol for the level and omit timestamps, targets,
// and thread IDs entirely unless --debug is active.

struct CliFormatter {
    /// When true, emit target (module path) and level text alongside the symbol.
    verbose: bool,
}

impl<S, N> FormatEvent<S, N> for CliFormatter
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &tracing::Event<'_>,
    ) -> std::fmt::Result {
        use dialoguer::console::style;

        let level = *event.metadata().level();

        // Coloured level symbol
        let symbol = match level {
            Level::ERROR => style("✖").red().bold().to_string(),
            Level::WARN => style("⚠").yellow().bold().to_string(),
            Level::INFO => style("●").cyan().to_string(),
            Level::DEBUG => style("◆").dim().to_string(),
            Level::TRACE => style("·").dim().to_string(),
        };

        write!(writer, "{symbol} ")?;

        // Optional: module target in dim grey (shown only in verbose/debug mode)
        if self.verbose {
            let target = event.metadata().target();
            // Strip the crate name prefix for brevity: "gac::git" → "git"
            let short_target = target.strip_prefix("gac::").unwrap_or(target);
            write!(writer, "{} ", style(format!("[{short_target}]")).dim())?;
        }

        // The actual message fields
        ctx.field_format().format_fields(writer.by_ref(), event)?;

        writeln!(writer)
    }
}

// ── Initialiser ───────────────────────────────────────────────────────────

/// Call once at the top of `main()` before any logging occurs.
///
/// Priority for log level (highest wins):
///   RUST_LOG env var  >  --debug flag  >  --verbose flag  >  default (Normal)
pub fn init(level: LogLevel) {
    // RUST_LOG always wins — lets power users override without recompiling
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level.to_filter()));
    let verbose_format = level >= LogLevel::Debug;

    tracing_subscriber::fmt()
        .event_format(CliFormatter {
            verbose: verbose_format,
        })
        .with_env_filter(filter)
        // Write to stderr so stdout stays clean for --print-only piping
        .with_writer(std::io::stderr)
        .init();
}
