use anyhow::Result;
use futures_util::StreamExt;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::{
    config::{Config, OllamaOptions},
    prompt::Prompt,
    stats::GenerationStats,
};

// ── Chat message ──────────────────────────────────────────────────────────

#[derive(Serialize, Clone)]
struct Message {
    role: &'static str,
    content: String,
}

// ── /api/chat request ─────────────────────────────────────────────────────

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
    think: bool,
    options: OllamaOptions,
}

// ── /api/chat streaming chunk ─────────────────────────────────────────────
//
// Ollama emits one JSON object per line.
// The final chunk (done=true) carries all the stat fields.
#[derive(Deserialize, Default)]
struct ChatChunk {
    message: Option<ChatChunkMessage>,
    done: bool,

    // Stat fields — only present in the final (done=true) chunk
    #[serde(default)]
    prompt_eval_count: u64,
    #[serde(default)]
    prompt_eval_duration: u64,
    #[serde(default)]
    eval_count: u64,
    #[serde(default)]
    eval_duration: u64,
    #[serde(default)]
    total_duration: u64,
}

#[derive(Deserialize)]
struct ChatChunkMessage {
    content: String,
}

// ── /api/chat non-streaming response ─────────────────────────────────────

#[derive(Deserialize)]
struct ChatResponse {
    message: ChatChunkMessage,
}

// ── /api/ps response ──────────────────────────────────────────────────────
//
// GET /api/ps returns the list of currently loaded models with their
// memory footprint. We match on the model name to find ours.

#[derive(Deserialize)]
struct PsResponse {
    models: Vec<PsModel>,
}

#[derive(Deserialize)]
struct PsModel {
    name: String,
    /// Total size of the model in bytes (weights + KV cache etc.)
    #[serde(default)]
    size: u64,
    /// Portion of `size` that lives on the GPU.
    #[serde(default)]
    size_vram: u64,
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn build_messages(prompt: &Prompt) -> Vec<Message> {
    vec![
        Message {
            role: "system",
            content: prompt.system.clone(),
        },
        Message {
            role: "user",
            content: prompt.user.clone(),
        },
    ]
}

async fn check_response(response: reqwest::Response) -> Result<reqwest::Response> {
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        anyhow::bail!("Ollama API error {status}: {body}");
    }

    Ok(response)
}

/// Query `/api/ps` and return `(size_vram, size_total)` for `model_name`.
/// Returns `(None, None)` on any error — stats are best-effort.
async fn query_vram(base_url: &str, model_name: &str) -> Option<u64> {
    let Ok(resp) = reqwest::get(format!("{base_url}/api/ps")).await else {
        warn!("could not reach /api/ps — VRAM stats unavailable");
        return None;
    };
    let Ok(ps) = resp.json::<PsResponse>().await else {
        warn!("failed to parse /api/ps response — VRAM stats unavailable");
        return None;
    };

    debug!(models = ps.models.len(), "queried /api/ps");

    // Strip the tag for a fuzzy match: "qwen2.5-coder:3b" matches "qwen2.5-coder:3b"
    // but also the bare name "qwen2.5-coder" so short config names still resolve.
    let base_name = model_name.split(':').next().unwrap_or(model_name);
    let model = ps
        .models
        .iter()
        .find(|m| m.name == model_name || m.name.starts_with(base_name));

    match &model {
        Some(m) => debug!(
            name       = %m.name,
            size_mb    = m.size / 1_048_576,
            vram_mb    = m.size_vram / 1_048_576,
            "matched model in /api/ps"
        ),
        None => warn!(
            model = %model_name,
            "model not found in /api/ps — is it loaded?"
        ),
    }

    // size_vram = VRAM used by this model.
    // size      = total allocation (VRAM + RAM for partial GPU offload).
    // We report size_vram as "model VRAM" and size as "total" only when
    // they differ (i.e. partial offload is in effect).
    model.map(|m| m.size_vram)
}

// ── Streaming generation — final commit message ───────────────────────────

/// Generate a commit message with streaming output.
/// Returns the generated text and a populated `GenerationStats`.
pub async fn generate_streaming(
    config: &Config,
    prompt: &Prompt,
) -> Result<(String, GenerationStats)> {
    let client = Client::new();

    let req = ChatRequest {
        model: config.model.clone(),
        messages: build_messages(prompt),
        stream: true,
        think: config.think,
        options: config.options.clone(),
    };

    info!(model = %config.model, num_ctx = config.options.num_ctx, "sending chat request");
    debug!(system = prompt.system, user = prompt.user);

    let response = check_response(
        client
            .post(format!("{}/api/chat", config.ollama_url))
            .json(&req)
            .send()
            .await?,
    )
    .await?;

    let mut stream = response.bytes_stream();
    let mut result = String::new();
    let mut stats = GenerationStats::default();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;

        for line in String::from_utf8_lossy(&bytes).lines() {
            if line.trim().is_empty() {
                continue;
            }

            if let Ok(parsed) = serde_json::from_str::<ChatChunk>(line) {
                if let Some(msg) = &parsed.message {
                    result.push_str(&msg.content);
                }

                if parsed.done {
                    // Capture all stat fields from the final chunk
                    stats.input_tokens = parsed.prompt_eval_count;
                    stats.output_tokens = parsed.eval_count;
                    stats.prompt_eval_ns = parsed.prompt_eval_duration;
                    stats.eval_ns = parsed.eval_duration;
                    stats.total_ns = parsed.total_duration;
                    break;
                }
            }
        }
    }

    // VRAM query is best-effort — runs after streaming completes
    let vram_used = query_vram(&config.ollama_url, &config.model).await;
    stats.vram_bytes = vram_used;

    Ok((result.trim().to_string(), stats))
}

// ── Non-streaming summarization — per-file pass ───────────────────────────

/// Summarize a single file diff. Returns text only — stats are not shown
/// for the per-file pass to keep the progress display clean.
pub async fn summarize(config: &Config, prompt: &Prompt) -> Result<String> {
    let client = Client::new();
    let mut opts = config.options.clone();
    opts.num_ctx = opts.num_ctx.min(1024);
    opts.num_predict = 128;

    debug!(
        model    = %config.model,
        num_ctx  = opts.num_ctx,
        "sending summarize request (non-streaming)"
    );
    debug!(system = prompt.system, user = prompt.user);

    let req = ChatRequest {
        model: config.model.clone(),
        messages: build_messages(prompt),
        stream: false,
        think: config.think,
        options: opts,
    };

    let response = check_response(
        client
            .post(format!("{}/api/chat", config.ollama_url))
            .json(&req)
            .send()
            .await?,
    )
    .await?;

    let body: ChatResponse = response.json().await?;
    Ok(body.message.content.trim().to_string())
}
