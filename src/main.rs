mod config;
mod diff;
mod git;
mod logging;
mod ollama;
mod prompt;
mod spinner;
mod stats;
mod validate;

use anyhow::Result;
use clap::{Parser, Subcommand};
use config::Config;
use diff::Strategy;
use futures_util::stream::{self, StreamExt};
use logging::LogLevel;
use std::{collections::HashMap, path::Path};
use tracing::{debug, info, warn};

const DEFAULT_CONFIG: &str = include_str!("../templates/gac.toml");

#[derive(Subcommand)]
enum Commands {
    Init,
}

enum Approval {
    Commit,
    Edit(String), // user edited the message inline
    Abort,
}

#[derive(Parser)]
#[command(
    name = "gac",
    about = "AI commit message generator — Ollama, low-VRAM",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long)]
    model: Option<String>,

    #[arg(long)]
    num_ctx: Option<u32>,

    /// Show info-level messages: config paths, strategy selection, scope detection
    #[arg(short, long, conflicts_with = "debug")]
    verbose: bool,

    /// Show debug-level messages: API fields, per-chunk details, raw config
    #[arg(long, conflicts_with = "verbose")]
    debug: bool,

    /// Suppress all output except the commit message and errors
    #[arg(short, long, conflicts_with = "verbose", conflicts_with = "debug")]
    quiet: bool,

    /// Print the generated message to stdout and exit without committing
    #[arg(long)]
    print: bool,
}

fn init_config() -> Result<()> {
    let path = ".gac.toml";

    if Path::new(path).exists() {
        anyhow::bail!("{path} already exists.");
    }

    std::fs::write(path, DEFAULT_CONFIG)?;

    info!(path, "project config created");

    Ok(())
}

fn approval_dialog(message: &str) -> anyhow::Result<Approval> {
    use dialoguer::{theme::ColorfulTheme, Editor, Select};

    let theme = ColorfulTheme::default();

    // ── Primary confirmation ───────────────────────────────────────────────
    // Show the generated message clearly before asking anything.
    println!(
        "\n{}\n",
        dialoguer::console::style("── Generated commit message ──").dim()
    );
    println!("{message}");
    println!(
        "{}",
        dialoguer::console::style("─────────────────────────────").dim()
    );

    let choices = &[
        "✅  Commit — use this message",
        "✏️   Edit  — open in $EDITOR",
        "✗   Abort — discard",
    ];
    let selection = Select::with_theme(&theme)
        .with_prompt("What would you like to do?")
        .items(choices)
        .default(0) // default to Commit on Enter
        .interact_opt()?; // None if user pressed Esc/q

    match selection {
        Some(0) => Ok(Approval::Commit),
        Some(1) => {
            // Open $EDITOR pre-filled with the generated message.
            // dialoguer::Editor returns None if the user saves an empty file.
            let edited = Editor::new()
                .executable("nvim")
                .require_save(true) // treat empty save as abort
                .edit(message)?;

            match edited {
                Some(msg) => {
                    let trimmed = msg.trim().to_string();
                    if trimmed.is_empty() {
                        warn!("empty message after edit — aborting");
                        Ok(Approval::Abort)
                    } else {
                        Ok(Approval::Edit(trimmed))
                    }
                }
                None => {
                    warn!("editor closed without saving — aborting");
                    Ok(Approval::Abort)
                }
            }
        }
        // index 2 or None (Esc / q)
        _ => Ok(Approval::Abort),
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Initialise logging before anything else
    let log_level = match (cli.quiet, cli.verbose, cli.debug) {
        (true, _, _) => LogLevel::Quiet,
        (_, _, true) => LogLevel::Debug,
        (_, true, _) => LogLevel::Verbose,
        _ => LogLevel::Normal,
    };

    logging::init(log_level);

    if let Some(cmd) = cli.command {
        return match cmd {
            Commands::Init => init_config(),
        };
    }

    // ── Shared MultiProgress — all spinners/bars share one instance ───────
    let mp = spinner::multi();

    // ── HTTP client (reused across all API calls) ─────────────────────────
    let client = ollama::create_client();

    // ── Config ────────────────────────────────────────────────────────────
    let mut config = Config::load()?;
    config.apply_cli_overrides(cli.model, cli.num_ctx);

    // ── Staged files ──────────────────────────────────────────────────────
    let all_staged = git::get_staged_files();
    debug!(count = all_staged.len(), "found staged files");

    let excluded = git::get_excluded_files(&all_staged, &config.exclude_patterns);

    if !excluded.is_empty() {
        warn!(
            files = %excluded.join(", "),
            "filtered lock/generated files"
        );
    }

    // ── Scope detection ───────────────────────────────────────────────────
    let scope_match = git::detect_scopes(&all_staged, &config.scopes);

    if !config.scopes.is_empty() {
        if !scope_match.matched.is_empty() {
            info!(scopes = %scope_match.matched.join(", "), "detected scopes");
        } else {
            info!(
                available = %scope_match.unmatched.join(", "),
                "no scope auto-matched"
            );
        }
    }

    // ── Raw diff ──────────────────────────────────────────────────────────
    let diff_spin = spinner::step_spinner(&mp, "Reading staged diff…");
    let (stat, raw_diff) =
        git::get_staged_stat_and_diff(&config.exclude_patterns).inspect_err(|e| {
            spinner::fail(&diff_spin, e.to_string());
        })?;
    spinner::done(
        &diff_spin,
        format!(
            "read diff — {} chars across {} file(s)",
            raw_diff.len(),
            raw_diff
                .lines()
                .filter(|l| l.starts_with("diff --git"))
                .count()
        ),
    );
    // ── Parse + score ─────────────────────────────────────────────────────
    let file_diffs = diff::parse_diff(&raw_diff);
    let strategy = diff::select_strategy(&file_diffs, config.max_diff_chars);

    info!(
        diff_chars = raw_diff.len(),
        files      = file_diffs.len(),
        strategy   = %strategy,
        "diff strategy selected"
    );

    // ── Build context via chosen strategy ─────────────────────────────────
    let context = match &strategy {
        Strategy::Direct => {
            debug!("using direct diff — fits within context budget");
            let body = diff::build_direct_context(&file_diffs, config.max_diff_chars);
            format!("=== Stat ===\n{stat}\n=== Diff ===\n{body}")
        }
        Strategy::Summarize => {
            info!(
                files = file_diffs.len(),
                "summarizing files individually (parallel)"
            );

            // One shared progress bar for the whole summarize pass
            let bar = spinner::summarize_bar(&mp, file_diffs.len());

            // Prepare all tasks
            let tasks: Vec<_> = file_diffs
                .iter()
                .map(|fd| {
                    let client = client.clone();
                    let config = config.clone();
                    let fd = fd.clone();
                    async move {
                        let chunk = if fd.char_count > config.max_diff_chars / 2 {
                            format!(
                                "{}\n[... truncated ...]",
                                &fd.content[..config.max_diff_chars / 2]
                            )
                        } else {
                            fd.content.clone()
                        };

                        let p = prompt::build_file_summary_prompt(&fd.path, &chunk);
                        let result = ollama::summarize(&client, &config, &p).await;
                        (fd.path.clone(), result)
                    }
                })
                .collect();

            // Run with bounded concurrency (3 parallel API calls)
            let mut summaries: HashMap<String, String> = HashMap::new();
            let mut stream = stream::iter(tasks).buffer_unordered(3);
            let mut completed = 0u64;

            while let Some((path, result)) = stream.next().await {
                completed += 1;
                bar.set_position(completed);

                match result {
                    Ok(s) => {
                        debug!(path = %path, summary = %s, "file summarized");
                        summaries.insert(path, s);
                    }
                    Err(e) => {
                        warn!(path = %path, error = %e, "file summarization failed");
                        summaries.insert(path, "(summary failed — see stat for details)".into());
                    }
                }
            }

            // Finish bar at 100 %
            bar.set_position(file_diffs.len() as u64);
            spinner::done(&bar, format!("summarized {} files", file_diffs.len()));

            let body = diff::build_summary_context(&summaries);
            format!("=== Stat ===\n{stat}\n\n{body}")
        }
        Strategy::StatOnly { top_n } => {
            info!(
                top_n = top_n,
                total = file_diffs.len(),
                skipped = file_diffs.len().saturating_sub(*top_n),
                "using stat-only strategy"
            );
            diff::build_stat_context(&stat, &file_diffs, *top_n)
        }
    };

    // ── Generate commit message ───────────────────────────────────────────
    let candidates = scope_match.best_candidates();
    let commit_prompt = prompt::build_commit_prompt(&context, &candidates);

    info!(
        model   = %config.model,
        num_ctx = config.options.num_ctx,
        "generating commit message"
    );

    // mp is passed in so the generation spinner shares the same draw target
    // as any bars that were active during the summarize pass.
    let (mut message, gen_stats) =
        ollama::generate_streaming(&client, &config, &commit_prompt, &mp).await?;
    // Print stats immediately after generation, before the approval dialog
    gen_stats.print(config.total_vram);

    // ── Validate and auto-retry if needed ─────────────────────────────────
    const MAX_RETRIES: usize = 2;
    let mut retries = 0;

    loop {
        match validate::validate_conventional_commit(&message) {
            Ok(()) => break,
            Err(reason) if retries < MAX_RETRIES => {
                retries += 1;
                warn!(
                    reason,
                    retry = retries,
                    "commit message validation failed — retrying"
                );

                let retry_prompt =
                    prompt::build_retry_prompt(&context, &candidates, &message, &reason);
                let (retry_msg, _) =
                    ollama::generate_streaming(&client, &config, &retry_prompt, &mp).await?;
                message = retry_msg;
            }
            Err(reason) => {
                warn!(
                    reason,
                    "commit message validation failed after retries — attempting auto-fix"
                );
                let fixed = validate::try_fix_commit_message(&message);
                if validate::validate_conventional_commit(&fixed).is_ok() {
                    info!("auto-fix succeeded");
                    message = fixed;
                } else {
                    warn!("auto-fix failed — user will need to edit manually");
                }
                break;
            }
        }
    }

    // ── Approval + commit ─────────────────────────────────────────────────
    let final_message = if cli.print {
        // Print-only mode: output to stdout and exit
        println!("{message}");
        return Ok(());
    } else {
        match approval_dialog(&message)? {
            Approval::Commit => {
                debug!("user confirmed commit");
                message
            }
            Approval::Edit(edited) => {
                info!("user edited commit message");
                // Show the edited message before committing
                println!(
                    "\n{}\n{}\n{}",
                    dialoguer::console::style("── Edited commit message ──").dim(),
                    edited,
                    dialoguer::console::style("───────────────────────────").dim(),
                );

                // One final confirmation after editing so the user can't
                // accidentally commit a half-finished message.
                let confirmed =
                    dialoguer::Confirm::with_theme(&dialoguer::theme::ColorfulTheme::default())
                        .with_prompt("Commit with edited message?")
                        .default(true)
                        .interact()?;

                if confirmed {
                    edited
                } else {
                    info!("user aborted after edit");
                    return Ok(());
                }
            }
            Approval::Abort => {
                info!("user aborted");
                return Ok(());
            }
        }
    };

    debug!(message = %final_message, "running git commit");
    let status = std::process::Command::new("git")
        .args(["commit", "-m", &final_message])
        .status()?;

    if status.success() {
        info!("committed successfully");
    } else {
        anyhow::bail!("git commit failed");
    }

    Ok(())
}
