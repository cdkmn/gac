use std::time::Duration;

use anyhow::Result;
use futures_util::StreamExt;
pub use reqwest::Client;
use serde::{Deserialize, Serialize};

use crate::{config::Config, progress::Progress, prompt::Prompt, stats::GenerationStats};

const MAX_API_RETRIES: usize = 2;
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
pub struct ModelInfo {
    pub id: String,
    pub context_length: Option<u64>,
}

#[derive(Deserialize)]
pub struct ModelPropResponse {
    pub data: Vec<ModelInfo>,
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
    reasoning_content: Option<String>,
}

#[derive(Deserialize)]
struct ChatChunkChoice {
    finish_reason: Option<FinishReason>,
    delta: ChatChunkDelta,
}

#[derive(Deserialize)]
struct ChatChunkTimings {
    prompt_n: u64,
    prompt_ms: f64,
    predicted_n: u64,
    predicted_ms: f64,
    predicted_per_second: f64,
}

#[derive(Deserialize, Default)]
struct ChatChunk {
    choices: Vec<ChatChunkChoice>,
    timings: Option<ChatChunkTimings>,
}

#[derive(Deserialize)]
struct ChatResChoiceMsg {
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
/// If `api_key` is provided, all requests will include `Authorization: Bearer <key>`.
pub fn create_client(api_key: Option<&str>) -> Client {
    let mut builder = Client::builder().timeout(Duration::from_secs(300));

    if let Some(key) = api_key {
        let mut headers = reqwest::header::HeaderMap::new();
        let value = reqwest::header::HeaderValue::from_str(&format!("Bearer {key}"))
            .expect("valid Authorization header value");
        headers.insert(reqwest::header::AUTHORIZATION, value);
        builder = builder.default_headers(headers);
    }

    builder.build().expect("failed to create HTTP client")
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
        let body = response
            .text()
            .await
            .unwrap_or_else(|e| format!("(failed to read response body: {e})"));
        anyhow::bail!("API error {status}: {body}");
    }

    Ok(response)
}

async fn query_vram(client: &Client, endpoint: &str, now: &str) -> Option<PerfGpuStat> {
    let url = format!("{endpoint}/api/performance?after={now}");
    let Ok(resp) = client.get(&url).send().await else {
        return None;
    };
    let Ok(ps) = resp.json::<PerformanceRes>().await else {
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
            Err(_) if attempt < MAX_API_RETRIES => {
                attempt += 1;
                let delay_ms = INITIAL_RETRY_DELAY_MS * 2u64.pow(attempt as u32 - 1);
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
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
    prog: &Progress,
) -> Result<(String, GenerationStats)> {
    let req = ChatRequest {
        model: config.model.clone(),
        messages: build_messages(prompt),
        stream: true,
        max_completion_tokens: Some(config.max_completion_tokens),
    };

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
            return Err(e);
        }
    };
    let mut stream = response.bytes_stream();
    let mut result = String::new();
    let mut stats = GenerationStats::default();
    let mut first_token = true;
    let mut line_buffer = String::new();

    while let Some(chunk) = stream.next().await {
        let bytes = match chunk {
            Ok(b) => b,
            Err(e) => {
                return Err(e.into());
            }
        };
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
                    first_token = false;
                }

                match choice.finish_reason {
                    None => {
                        let reasoning = choice
                            .delta
                            .reasoning_content
                            .unwrap_or_default()
                            .replace("\n", "")
                            .replace("\r", "");
                        let content = choice.delta.content.unwrap_or_default();
                        let clean_content = &content.replace("\n", "").replace("\r", "");

                        if !reasoning.trim().is_empty() {
                            prog.set_msg_generation(&reasoning, true);
                        }

                        if !clean_content.trim().is_empty() {
                            prog.set_msg_generation(clean_content, false);
                        }

                        result.push_str(&content);
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

    // Final flush: process any remaining content in the line buffer
    if !line_buffer.is_empty() {
        for line in line_buffer.lines() {
            let line = line.trim();

            if line.is_empty() || line == "data: [DONE]" {
                continue;
            }

            if let Some(json_str) = line.strip_prefix("data: ") {
                if let Ok(parsed) = serde_json::from_str::<ChatChunk>(json_str) {
                    for choice in parsed.choices {
                        let reasoning = choice
                            .delta
                            .reasoning_content
                            .unwrap_or_default()
                            .replace("\n", "")
                            .replace("\r", "");
                        let content = choice.delta.content.unwrap_or_default();
                        let clean_content = &content.replace("\n", "").replace("\r", "");

                        if !reasoning.trim().is_empty() {
                            prog.set_msg_generation(&reasoning, true);
                        }

                        if !clean_content.trim().is_empty() {
                            prog.set_msg_generation(clean_content, false);
                        }

                        if !content.is_empty() {
                            result.push_str(&content);
                        }

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

    // VRAM query after streaming — best-effort, never blocks the happy path
    let gpu_stat = query_vram(client, &config.endpoint, &now).await;

    if let Some(gpu_stat) = gpu_stat {
        stats.vram_total_mb = Some(gpu_stat.mem_total_mb);
        stats.vram_used_mb = Some(gpu_stat.mem_used_mb);
        stats.vram_util_pct = Some(gpu_stat.mem_util_pct);
    }

    Ok((result.trim().to_string(), stats))
}

/// Get the maximum context length for a given model
pub async fn model_ctx_len(client: &Client, config: &Config) -> Result<u64> {
    let response = check_response(
        client
            .get(format!("{}/v1/models", config.endpoint))
            .send()
            .await?,
    )
    .await?;
    let body: ModelPropResponse = response.json().await?;
    let Some(len) = body.data.iter().find_map(|mi| {
        mi.id
            .eq(&config.model)
            .then(|| mi.context_length.unwrap_or(131072))
    }) else {
        anyhow::bail!("Model not found");
    };

    Ok(len)
}

/// Apply the model template to the prompt and send it to the model for token counts.
pub async fn apply_template(client: &Client, config: &Config, prompt: &Prompt) -> Result<String> {
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

/// Tokenize a string into token IDs using the model's tokenizer.
pub async fn tokenize(client: &Client, config: &Config, content: &str) -> Result<Vec<u32>> {
    let req = TokenizeRequest {
        content: content.to_string(),
    };
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

/// Convert token IDs back to text using the model's tokenizer.
pub async fn detokenize(client: &Client, config: &Config, tokens: &[u32]) -> Result<String> {
    let req = DetokenizeRequest {
        tokens: tokens.to_vec(),
    };
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

/// Count the number of tokens in a prompt (apply template then tokenize).
pub async fn token_counts(client: &Client, config: &Config, prompt: &Prompt) -> Result<usize> {
    let content = apply_template(client, config, prompt).await?;
    let tokens = tokenize(client, config, &content).await?;
    Ok(tokens.len())
}

/// Summarize a single file diff. Returns text only — stats are not shown
/// for the per-file pass to keep the progress display clean.
pub async fn summarize(client: &Client, config: &Config, prompt: &Prompt) -> Result<String> {
    let req = ChatRequest {
        model: config.model.clone(),
        messages: build_messages(prompt),
        stream: false,
        max_completion_tokens: Some(config.max_completion_tokens),
    };

    let response = with_retry(|| {
        let client = client.clone();
        let url = format!("{}/v1/chat/completions", config.endpoint);
        let req = req.clone();
        async move { check_response(client.post(&url).json(&req).send().await?).await }
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── build_messages ───────────────────────────────────────────────────

    #[test]
    fn build_messages_creates_system_and_user() {
        let prompt = Prompt {
            system: "sys".into(),
            user: "usr".into(),
        };
        let msgs = build_messages(&prompt);
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[0].content, "sys");
        assert_eq!(msgs[1].role, "user");
        assert_eq!(msgs[1].content, "usr");
    }

    // ── ChatChunk deserialization ─────────────────────────────────────────

    #[test]
    fn chat_chunk_deserialize_content_delta() {
        let json =
            r#"{"choices":[{"delta":{"content":"hello"},"finish_reason":null}],"timings":null}"#;
        let chunk: ChatChunk = serde_json::from_str(json).unwrap();
        assert_eq!(chunk.choices.len(), 1);
        assert_eq!(chunk.choices[0].delta.content.as_deref(), Some("hello"));
        assert!(chunk.choices[0].finish_reason.is_none());
        assert!(chunk.timings.is_none());
    }

    #[test]
    fn chat_chunk_deserialize_with_timings() {
        let json = r#"{"choices":[{"delta":{"content":null},"finish_reason":"stop"}],"timings":{"prompt_n":10,"prompt_ms":5.0,"predicted_n":20,"predicted_ms":10.0,"predicted_per_second":2.0}}"#;
        let chunk: ChatChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices[0].delta.content.is_none());
        assert!(chunk.choices[0].finish_reason.is_some());
        let t = chunk.timings.unwrap();
        assert_eq!(t.prompt_n, 10);
        assert_eq!(t.predicted_n, 20);
        assert!((t.predicted_per_second - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn chat_chunk_deserialize_done_signal() {
        let json = r#"{"choices":[],"timings":null}"#;
        let chunk: ChatChunk = serde_json::from_str(json).unwrap();
        assert!(chunk.choices.is_empty());
    }

    #[test]
    fn chat_chunk_default_is_empty() {
        let chunk = ChatChunk::default();
        assert!(chunk.choices.is_empty());
        assert!(chunk.timings.is_none());
    }

    // ── FinishReason deserialization ──────────────────────────────────────

    #[test]
    fn finish_reason_stop_deserializes() {
        let json = r#""stop""#;
        let reason: FinishReason = serde_json::from_str(json).unwrap();
        assert!(matches!(reason, FinishReason::Stop));
    }

    #[test]
    fn finish_reason_length_deserializes() {
        let json = r#""length""#;
        let reason: FinishReason = serde_json::from_str(json).unwrap();
        assert!(matches!(reason, FinishReason::Length));
    }

    // ── ChatResponse deserialization ──────────────────────────────────────

    #[test]
    fn chat_response_with_content() {
        let json = r#"{"choices":[{"message":{"content":"feat: add foo"}}]}"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.choices.len(), 1);
        assert_eq!(
            resp.choices[0].message.content.as_deref(),
            Some("feat: add foo")
        );
    }

    #[test]
    fn chat_response_with_null_content() {
        let json = r#"{"choices":[{"message":{"content":null}}]}"#;
        let resp: ChatResponse = serde_json::from_str(json).unwrap();
        assert!(resp.choices[0].message.content.is_none());
    }
}
