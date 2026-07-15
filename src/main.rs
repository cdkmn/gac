#![warn(unused_crate_dependencies)]

mod config;
mod diff;
mod git;
mod llamaswap;
mod logging;
mod prompt;
mod spinner;
mod stats;
mod validate;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use config::Config;
use diff::Strategy;
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
    about = "AI commit message generator — llama-swap, low-VRAM",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    #[arg(short, long)]
    model: Option<String>,

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

    /// Pass --no-verify to git commit (skip pre-commit hooks)
    #[arg(long)]
    no_verify: bool,
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
            let editor = std::env::var("GAC_EDITOR")
                .or_else(|_| std::env::var("EDITOR"))
                .or_else(|_| std::env::var("VISUAL"))
                .unwrap_or_else(|_| "nvim".into());
            let edited = Editor::new()
                .executable(&editor)
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
    let client = llamaswap::create_client();

    // ── Config ────────────────────────────────────────────────────────────
    let mut config = Config::load()?;
    config.apply_cli_overrides(cli.model);
    config.validate()?;

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
    let file_diffs = diff::parse_diff(&raw_diff);
    let (strategy, ctx) = diff::select_strategy(&client, &config, &raw_diff, &scope_match, &stat)
        .await
        .context("failed to select diff strategy")?;

    info!(
        diff_chars = raw_diff.len(),
        files      = file_diffs.len(),
        strategy   = %strategy,
        "diff strategy selected"
    );

    // ── Build context via chosen strategy ─────────────────────────────────
    let context = match &strategy {
        Strategy::Direct => ctx,
        Strategy::Summarize => {
            info!(files = file_diffs.len(), "summarizing files individually");

            // One shared progress bar for the whole summarize pass
            let bar = spinner::summarize_bar(&mp, file_diffs.len());
            let props = llamaswap::model_props(&client, &config)
                .await
                .context("failed to fetch model properties")?;
            let budget = props.default_generation_settings.n_ctx;
            let mut summaries: HashMap<String, String> = HashMap::new();
            let mut completed = 0u64;

            for fd in &file_diffs {
                let tokens = llamaswap::tokenize(&client, &config, &fd.content)
                    .await
                    .context("failed to tokenize file diff")?;

                let truncated;
                let diff_ref: &str = if tokens.len() > (budget / 2) as usize {
                    let detokenized =
                        llamaswap::detokenize(&client, &config, &tokens[..(budget / 2) as usize])
                            .await
                            .context("failed to detokenize truncated diff")?;
                    truncated = format!("{}\n[... truncated ...]", detokenized);
                    &truncated
                } else {
                    &fd.content
                };

                let p = prompt::build_file_summary_prompt(&fd.path, diff_ref);
                let result = llamaswap::summarize(&client, &config, &p).await;

                completed += 1;
                bar.set_position(completed);

                match result {
                    Ok(s) => {
                        debug!(path = %fd.path, summary = %s, "file summarized");
                        summaries.insert(fd.path.clone(), s);
                    }
                    Err(e) => {
                        warn!(path = %fd.path, error = %e, "file summarization failed");
                        summaries.insert(
                            fd.path.clone(),
                            "(summary failed — see stat for details)".into(),
                        );
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
        "generating commit message"
    );

    // mp is passed in so the generation spinner shares the same draw target
    // as any bars that were active during the summarize pass.
    let (mut message, gen_stats) =
        llamaswap::generate_streaming(&client, &config, &commit_prompt, &mp).await?;
    // Print stats immediately after generation, before the approval dialog
    gen_stats.print();

    // ── Validate and auto-retry if needed ─────────────────────────────────
    const MAX_VALIDATION_RETRIES: usize = 2;
    let mut retries = 0;

    loop {
        match validate::validate_conventional_commit(&message) {
            Ok(()) => break,
            Err(reason) if retries < MAX_VALIDATION_RETRIES => {
                retries += 1;
                warn!(
                    reason,
                    message = &message,
                    retry = retries,
                    "commit message validation failed — retrying"
                );

                let retry_prompt =
                    prompt::build_retry_prompt(&context, &candidates, &message, &reason);
                let (retry_msg, _) =
                    llamaswap::generate_streaming(&client, &config, &retry_prompt, &mp).await?;
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
    if git::commit(&final_message, cli.no_verify)? {
        info!("committed successfully");
    } else {
        anyhow::bail!("git commit failed");
    }

    Ok(())
}
