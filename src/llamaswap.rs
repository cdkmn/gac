use anyhow::Result;
use futures_util::StreamExt;
use indicatif::MultiProgress;
pub use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::{config::Config, prompt::Prompt, spinner, stats::GenerationStats};

const MAX_RETRIES: usize = 2;
const INITIAL_RETRY_DELAY_MS: u64 = 1000;

#[derive(Serialize, Clone)]
struct Message {
    role: &'static str,
    content: String,
}

#[derive(Serialize, Clone)]
struct ChatRequest {
    model: String,
    messages: Vec<Message>,
    stream: bool,
    max_completion_tokens: Option<u64>,
}

#[derive(Deserialize)]
pub struct DefaultGenerationSettings {
    // pub params: DefaultGenerationParams,
    pub n_ctx: u32,
}

#[derive(Deserialize)]
pub struct ModelPropResponse {
    pub default_generation_settings: DefaultGenerationSettings,
}

#[derive(Serialize, Clone)]
struct ApplyTemplateRequest {
    messages: Vec<Message>,
}

#[derive(Deserialize)]
struct ApplyTemplateResponse {
    prompt: String,
}

#[derive(Serialize, Clone)]
struct TokenizeRequest {
    content: String,
}

#[derive(Deserialize)]
struct TokenizeResponse {
    tokens: Vec<u32>,
}

// detokenize
#[derive(Serialize, Clone)]
struct DetokenizeRequest {
    tokens: Vec<u32>,
}

#[derive(Deserialize)]
struct DetokenizeResponse {
    content: String,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    FunctionCall,
}

#[derive(Deserialize)]
struct ChatChunkDelta {
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatChunkChoice {
    finish_reason: Option<FinishReason>,
    delta: ChatChunkDelta,
}

#[derive(Deserialize)]
struct ChatChunkTimings {
    // cache_n: u64,
    prompt_n: u64,
    prompt_ms: f64,
    // prompt_per_token_ms: f64,
    // prompt_per_second: f64,
    predicted_n: u64,
    predicted_ms: f64,
    // predicted_per_token_ms: f64,
    predicted_per_second: f64,
}

// llama-swap emits one JSON object per line.
// The final chunk (done=true) carries all the stat fields.
#[derive(Deserialize, Default)]
struct ChatChunk {
    choices: Vec<ChatChunkChoice>,
    timings: Option<ChatChunkTimings>,
}

#[derive(Deserialize)]
struct ChatResChoiceMsg {
    // role: String,
    content: Option<String>,
}

#[derive(Deserialize)]
struct ChatResChoice {
    message: ChatResChoiceMsg,
}

// ── /v1/chat/completions non-streaming response ─────────────────────────────────────

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<ChatResChoice>,
}

#[derive(Deserialize, Clone, Copy)]
struct PerfGpuStat {
    mem_used_mb: u32,
    mem_total_mb: u32,
    mem_util_pct: f64,
}

#[derive(Deserialize)]
struct PerformanceRes {
    gpu_stats: Vec<PerfGpuStat>,
}

// ── Helpers ───────────────────────────────────────────────────────────────

/// Create a reusable HTTP client with sensible defaults.
/// Call once per run and pass the reference to all API functions.
pub fn create_client() -> Client {
    Client::builder()
        .timeout(std::time::Duration::from_secs(300))
        .build()
        .expect("failed to create HTTP client")
}

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
        anyhow::bail!("API error {status}: {body}");
    }

    Ok(response)
}

async fn query_vram(base_url: &str, now: String) -> Option<PerfGpuStat> {
    let Ok(resp) = reqwest::get(format!("{base_url}/api/performance?after={now}")).await else {
        warn!("could not reach /api/performance — VRAM stats unavailable");
        return None;
    };
    let Ok(ps) = resp.json::<PerformanceRes>().await else {
        warn!("failed to parse /api/performance response — VRAM stats unavailable");
        return None;
    };

    let stat = ps.gpu_stats.last()?;

    Some(*stat)
}

/// Retry a fallible async operation with exponential backoff.
async fn with_retry<T, F, Fut>(mut operation: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T>>,
{
    let mut attempt = 0;

    loop {
        match operation().await {
            Ok(result) => return Ok(result),
            Err(e) if attempt < MAX_RETRIES => {
                attempt += 1;
                let delay_ms = INITIAL_RETRY_DELAY_MS * 2u64.pow(attempt as u32 - 1);
                warn!(attempt, delay_ms, error = %e, "API call failed — retrying");
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Generate a commit message with streaming output.
/// Returns the generated text and a populated `GenerationStats`.
pub async fn generate_streaming(
    client: &Client,
    config: &Config,
    prompt: &Prompt,
    mp: &MultiProgress,
) -> Result<(String, GenerationStats)> {
    let req = ChatRequest {
        model: config.model.clone(),
        messages: build_messages(prompt),
        stream: true,
        max_completion_tokens: Some(config.max_completion_tokens),
    };

    info!(model = %config.model, "sending chat request");
    debug!(system = &prompt.system[..30], user = &prompt.user[..30]);

    // Start spinner before the network call so latency is always covered
    let spin = spinner::generation_spinner(mp, &config.model);
    let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Nanos, true);

    let response = with_retry(|| {
        let client = client.clone();
        let url = format!("{}/v1/chat/completions", config.endpoint);
        let req = req.clone();
        async move { check_response(client.post(&url).json(&req).send().await?).await }
    })
    .await;

    let response = match response {
        Ok(r) => r,
        Err(e) => {
            spinner::fail(&spin, format!("request failed: {e}"));
            return Err(e);
        }
    };
    let mut stream = response.bytes_stream();
    let mut result = String::new();
    let mut stats = GenerationStats::default();
    let mut first_token = true;
    let mut line_buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = chunk?;
        line_buffer.push_str(&String::from_utf8_lossy(&bytes));

        // Process all complete lines; leave any trailing partial line in the buffer.
        while let Some(newline_pos) = line_buffer.find('\n') {
            let line = line_buffer[..newline_pos].trim().to_string();
            line_buffer.drain(..=newline_pos);

            if line.is_empty() {
                continue;
            }

            if line == "data: [DONE]" {
                break;
            }

            let Some(json_str) = line.strip_prefix("data: ") else {
                continue;
            };

            let Ok(parsed) = serde_json::from_str::<ChatChunk>(json_str) else {
                continue;
            };

            for choice in parsed.choices {
                if first_token {
                    // Clear the spinner before printing anything to stdout
                    // so the two streams don't interleave visually.
                    spinner::clear(&spin);
                    print!("\n💬 ");
                    first_token = false;
                }
                match choice.finish_reason {
                    None => {
                        result.push_str(choice.delta.content.unwrap_or_default().as_str());
                    }
                    _ => {
                        if let Some(timings) = &parsed.timings {
                            stats.input_tokens = timings.prompt_n;
                            stats.output_tokens = timings.predicted_n;
                            stats.prompt_eval_ms = timings.prompt_ms;
                            stats.eval_ms = timings.predicted_ms;
                            stats.total_ms = stats.prompt_eval_ms + stats.eval_ms;
                            stats.tokens_per_second = timings.predicted_per_second;
                        }
                    }
                }
            }
        }
    }

    // Always ensure a clean line after streaming, whether we got tokens or not
    if !first_token {
        println!();
    } else {
        // No tokens arrived at all — clean up the spinner as an error
        spinner::fail(&spin, "no response received from model");
    }

    // VRAM query after streaming — best-effort, never blocks the happy path
    let vram_spin = spinner::step_spinner(mp, "querying VRAM usage…");
    let gpu_stat = query_vram(&config.endpoint, now).await;
    spinner::clear(&vram_spin);

    if let Some(gpu_stat) = gpu_stat {
        stats.vram_total_mb = Some(gpu_stat.mem_total_mb);
        stats.vram_used_mb = Some(gpu_stat.mem_used_mb);
        stats.vram_util_pct = Some(gpu_stat.mem_util_pct);
    }

    Ok((result.trim().to_string(), stats))
}

/// Model props.
pub async fn model_props(client: &Client, config: &Config) -> Result<ModelPropResponse> {
    debug!(model= %config.model, "sending model props request");

    let response = check_response(
        client
            .get(format!("{}/props?model={}", config.endpoint, config.model))
            .send()
            .await?,
    )
    .await?;
    let body: ModelPropResponse = response.json().await?;

    Ok(body)
}

/// Apply the modeltemplate to the prompt and send it to the model for token counts.
pub async fn apply_template(client: &Client, config: &Config, prompt: &Prompt) -> Result<String> {
    debug!(model= %config.model, "sending apply template request");
    debug!(system = &prompt.system[..30], user = &prompt.user[..30]);

    let req = ApplyTemplateRequest {
        messages: build_messages(prompt),
    };

    let response = check_response(
        client
            .post(format!(
                "{}/upstream/{}/apply-template",
                config.endpoint, config.model
            ))
            .json(&req)
            .send()
            .await?,
    )
    .await?;
    let body: ApplyTemplateResponse = response.json().await?;

    Ok(body.prompt)
}

/// Count the number of tokens in the prompt using the model's tokenizer.
pub async fn tokenize(client: &Client, config: &Config, content: String) -> Result<Vec<u32>> {
    debug!(model= %config.model,"sending tokenize request");

    let req = TokenizeRequest { content };
    let response = check_response(
        client
            .post(format!(
                "{}/upstream/{}/tokenize",
                config.endpoint, config.model
            ))
            .json(&req)
            .send()
            .await?,
    )
    .await?;

    let body: TokenizeResponse = response.json().await?;
    Ok(body.tokens)
}

/// Count the number of tokens in the prompt using the model's tokenizer.
pub async fn detokenize(client: &Client, config: &Config, tokens: Vec<u32>) -> Result<String> {
    debug!(model= %config.model,"sending detokenize request");

    let req = DetokenizeRequest { tokens };
    let response = check_response(
        client
            .post(format!(
                "{}/upstream/{}/detokenize",
                config.endpoint, config.model
            ))
            .json(&req)
            .send()
            .await?,
    )
    .await?;
    let body: DetokenizeResponse = response.json().await?;

    Ok(body.content)
}

/// Count the number of tokens in the prompt using the model's tokenizer.
pub async fn token_counts(client: &Client, config: &Config, prompt: &Prompt) -> Result<usize> {
    let content = apply_template(client, config, prompt).await?;
    let tokens = tokenize(client, config, content).await?;
    Ok(tokens.len())
}

/// Summarize a single file diff. Returns text only — stats are not shown
/// for the per-file pass to keep the progress display clean.
pub async fn summarize(client: &Client, config: &Config, prompt: &Prompt) -> Result<String> {
    debug!(
        model    = %config.model,
        "sending summarize request (non-streaming)"
    );
    debug!(system = &prompt.system[..30], user = &prompt.user[..30]);

    let req = ChatRequest {
        model: config.model.clone(),
        messages: build_messages(prompt),
        stream: false,
        max_completion_tokens: Some(config.max_completion_tokens),
    };

    let response = check_response(
        client
            .post(format!("{}/v1/chat/completions", config.endpoint))
            .json(&req)
            .send()
            .await?,
    )
    .await?;

    let body: ChatResponse = response.json().await?;
    let choice = body
        .choices
        .first()
        .ok_or_else(|| anyhow::anyhow!("API returned empty choices array"))?;
    let content = choice
        .message
        .content
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("API returned choice with null content"))?
        .trim()
        .to_string();
    Ok(content)
}
