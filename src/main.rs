#![warn(unused_crate_dependencies)]

mod config;
mod crypto;
mod diff;
mod git;
mod llamaswap;
mod progress;
mod prompt;
mod stats;
mod validate;

use std::{collections::HashMap, fs, path::Path, process::Stdio, time::Instant};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use config::Config;
use diff::Strategy;

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

    /// API key for llama-swap authentication
    #[arg(long, env = "GAC_API_KEY")]
    api_key: Option<String>,

    /// Pass --no-verify to git commit (skip pre-commit hooks)
    #[arg(long)]
    no_verify: bool,
}

fn init_config() -> Result<()> {
    let path = ".gac.toml";

    if Path::new(path).exists() {
        anyhow::bail!("{path} already exists.");
    }

    fs::write(path, DEFAULT_CONFIG)?;

    println!("Configuration created at {}", console::style(path).yellow());

    Ok(())
}

fn command_exists(cmd: &str) -> bool {
    std::process::Command::new(cmd)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

fn approval_dialog(message: &str) -> anyhow::Result<Approval> {
    use dialoguer::{theme::ColorfulTheme, Editor, Select};

    let theme = ColorfulTheme::default();

    // ── Primary confirmation ───────────────────────────────────────────────
    // Show the generated message clearly before asking anything.
    println!(
        "\n{}\n",
        console::style(format!("── Generated Commit Message {}", "─".repeat(27))).dim()
    );
    println!("{message}");
    println!("{}", console::style("─".repeat(55)).dim());

    let choices = &["✅ Commit", "✏️ Edit", "❌ Abort"];
    let selection = Select::with_theme(&theme)
        .with_prompt("What would you like to do?")
        .items(choices)
        .default(0) // default to Commit on Enter
        .interact_opt()?; // None if user pressed Esc/q

    match selection {
        Some(0) => Ok(Approval::Commit),
        Some(1) => {
            let editor = if command_exists("nvim") {
                "nvim".into()
            } else {
                std::env::var("GAC_EDITOR")
                    .or_else(|_| std::env::var("EDITOR"))
                    .or_else(|_| std::env::var("VISUAL"))
                    .unwrap_or_else(|_| {
                        if cfg!(target_os = "windows") {
                            "start notepad.exe".into()
                        } else {
                            "open -t".into()
                        }
                    })
            };
            let edited = Editor::new()
                .executable(&editor)
                .require_save(true) // treat empty save as abort
                .edit(message)?;

            match edited {
                Some(msg) => {
                    let trimmed = msg.trim().to_string();
                    if trimmed.is_empty() {
                        println!(
                            "{}",
                            console::style("⚠️ Empty message after edit. Aborting.")
                        );
                        Ok(Approval::Abort)
                    } else {
                        Ok(Approval::Edit(trimmed))
                    }
                }
                None => {
                    println!(
                        "{}",
                        console::style("⚠️ Editor closed without saving. Aborting.")
                    );
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

    if let Some(cmd) = cli.command {
        return match cmd {
            Commands::Init => init_config(),
        };
    }

    // ── Config ────────────────────────────────────────────────────────────
    let mut config = Config::load()?;
    config.apply_cli_overrides(cli.model);
    config.validate()?;

    // ── API Key Resolution ───────────────────────────────────────────────
    // Priority: CLI flag (--api-key) > env var (GAC_API_KEY) > config file > prompt
    let api_key = cli.api_key.or_else(|| config.api_key.clone());

    let api_key = match api_key {
        Some(key) if !key.is_empty() => Some(key),
        _ => {
            use dialoguer::Password;

            eprintln!("{}", console::style("⚠️ No API key configured.").yellow());
            let key = Password::new()
                .with_prompt("Enter your API key")
                .interact()?;

            if key.is_empty() {
                bail!("API key cannot be empty");
            }

            Config::save_api_key(&key)?;
            eprintln!(
                "{}",
                console::style("✔️ API key saved to ~/.config/gac/config.toml")
                    .green()
                    .bold()
            );
            Some(key)
        }
    };

    // ── HTTP Client (Reused across all API calls) ─────────────────────────
    let client = llamaswap::create_client(api_key.as_deref());

    let mut prog = progress::Progress::new();

    // ── Staged Files ──────────────────────────────────────────────────────
    prog.start_staging();
    let all_staged = git::get_staged_files()?;
    let excluded = git::get_excluded_files(&all_staged, &config.exclude_patterns);
    prog.finish_staging(&all_staged, &excluded);

    // ── Scope Detection ───────────────────────────────────────────────────
    prog.start_scope();
    let scope_match = git::detect_scopes(prog.clone(), &all_staged, &config.scopes);
    prog.finish_scope(&config.scopes, &scope_match);

    // ── Raw Diff ──────────────────────────────────────────────────────────
    prog.start_strategy();
    let (stat, raw_diff) = git::get_staged_stat_and_diff(&config.exclude_patterns)?;
    let file_diffs = diff::parse_diff(&raw_diff);
    let (strategy, ctx, tokens) = diff::select_strategy(
        &client,
        &config,
        &raw_diff,
        &file_diffs,
        &scope_match,
        &stat,
    )
    .await
    .context("failed to select diff strategy")?;
    prog.finish_strategy(
        &strategy.to_string(),
        file_diffs.len(),
        raw_diff.len(),
        tokens,
    );

    // ── Build context via chosen strategy ─────────────────────────────────
    prog.start_context();
    let context = match &strategy {
        Strategy::Direct => ctx,
        Strategy::Summarize => {
            prog.start_summarize(file_diffs.len() as u64);
            let started = Instant::now();
            let budget = llamaswap::model_ctx_len(&client, &config)
                .await
                .context("failed to fetch model properties")?;
            let mut summaries: HashMap<String, String> = HashMap::new();

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

                prog.inc_summarize(1);

                match result {
                    Ok(s) => {
                        summaries.insert(fd.path.clone(), s);
                    }
                    Err(e) => {
                        prog.println(
                            console::style(format!("Summarization failed for file `{}`", fd.path))
                                .red()
                                .to_string(),
                        )
                        .ok();
                        prog.println(
                            console::style(format!("Summarization Error: {}", e))
                                .dim()
                                .to_string(),
                        )
                        .ok();
                        summaries.insert(
                            fd.path.clone(),
                            "(summary failed — see stat for details)".into(),
                        );
                    }
                }
            }

            let body = diff::build_summary_context(&summaries);
            prog.println(format!(
                "✔️ {} file(s) summarized in {}",
                console::style(file_diffs.len()).yellow(),
                console::style(indicatif::HumanDuration(started.elapsed())).yellow()
            ))
            .ok();
            prog.finish_summarize();
            format!("=== Stat ===\n{stat}\n\n{body}")
        }
        Strategy::StatOnly { top_n } => {
            prog.println(format!(
                "⚠️ Using stat-only strategy. Top-N: {} Total: {} Skipped: {}",
                console::style(top_n).yellow(),
                console::style(file_diffs.len()).yellow(),
                console::style(file_diffs.len().saturating_sub(*top_n)).yellow()
            ))
            .ok();
            diff::build_stat_context(&stat, &file_diffs, *top_n)
        }
    };
    prog.finish_context();

    // ── Generate commit message ───────────────────────────────────────────
    prog.start_generation();
    let started = Instant::now();
    let candidates = scope_match.best_candidates();
    let commit_prompt = prompt::build_commit_prompt(&context, &candidates);
    let (mut message, mut gen_stats) =
        llamaswap::generate_streaming(&client, &config, &commit_prompt, &prog).await?;

    // ── Validate and auto-retry if needed ─────────────────────────────────
    const MAX_VALIDATION_RETRIES: usize = 2;
    let mut retries = 0;

    loop {
        match validate::validate_conventional_commit(&message) {
            Ok(()) => {
                break;
            }
            Err(reason) if retries < MAX_VALIDATION_RETRIES => {
                retries += 1;
                prog.println(
                    console::style(format!(
                        "⚠️ Commit message validation failed. Retrying: {}",
                        retries
                    ))
                    .yellow()
                    .to_string(),
                )
                .ok();
                prog.println(
                    console::style(format!("⚠️ Reason: {}", reason))
                        .dim()
                        .to_string(),
                )
                .ok();

                let retry_prompt =
                    prompt::build_retry_prompt(&context, &candidates, &message, &reason);
                let (retry_msg, retry_stats) =
                    llamaswap::generate_streaming(&client, &config, &retry_prompt, &prog).await?;
                message = retry_msg;
                gen_stats = retry_stats;
            }
            Err(reason) => {
                prog.println(
                    console::style(
                        "⚠️ Commit message validation failed after retries. Attempting auto-fix.",
                    )
                    .yellow()
                    .to_string(),
                )
                .ok();
                prog.println(
                    console::style(format!("⚠️ Reason: {}", reason))
                        .dim()
                        .to_string(),
                )
                .ok();

                let fixed = validate::try_fix_commit_message(&message);

                if validate::validate_conventional_commit(&fixed).is_ok() {
                    message = fixed;
                } else {
                    prog.println(
                        console::style("⚠️ Auto-fix failed. User will need to edit manually.")
                            .yellow()
                            .to_string(),
                    )
                    .ok();
                }
                break;
            }
        }
    }

    prog.finish_generation(started);
    gen_stats.print();

    // ── Approval + Commit ─────────────────────────────────────────────────
    let final_message = match approval_dialog(&message)? {
        Approval::Commit => message,
        Approval::Edit(edited) => {
            // Show the edited message before committing
            println!(
                "\n{}\n{}\n{}",
                dialoguer::console::style(format!("── Edited Commit Message {}", "─".repeat(30)))
                    .dim(),
                edited,
                dialoguer::console::style("─".repeat(55)).dim(),
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
                return Ok(());
            }
        }
        Approval::Abort => {
            return Ok(());
        }
    };

    if git::commit(&final_message, cli.no_verify)? {
        eprintln!("Committed successfully.");
    } else {
        anyhow::bail!("Git commit failed.");
    }

    Ok(())
}
