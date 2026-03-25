//! OpenRouter LLM proxy with DAE memory context injection.
//!
//! Orchestrates: query memory -> build prompt -> stream from OpenRouter -> post-response ops.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{Instrument, info_span};
use uuid::Uuid;

use am_store::project::BrainStore;

use crate::http_server::AppState;
use crate::server::AmServer;

const DEFAULT_MODEL: &str = "anthropic/claude-sonnet-4-20250514";
const DEFAULT_MAX_TOKENS: u32 = 4096;
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const KEEPALIVE_INTERVAL_SECS: u64 = 15;

// --- Chat mode system prompt templates ---

const EXPLORER_SYSTEM_PROMPT: &str = "\
You are a window into a geometric memory system built on the 3-sphere manifold.
Your job is to answer the user's question in clean markdown using the supplied
AM memory context. Surface connections between memories, trace how they relate,
and help the user understand their memory landscape.
Do not fabricate memories. If the recall is sparse, noisy, or fragmentary, say
that the memory is partial and summarize only the stable, high-confidence points.
Never output raw note fragments, dangling clauses, or stitched-together snippets
as if they were polished facts. Prefer a coherent synopsis over exhaustive recall dumps.";

const ASSISTANT_SYSTEM_PROMPT: &str = "\
You are a helpful assistant with access to a geometric memory system.
Your job is to answer the user's question in clean markdown using the supplied
AM memory context. When you identify an important insight, decision, or preference,
wrap it in <salient> tags to store it as conscious memory.
If recalled context is fragmentary or low-confidence, acknowledge the uncertainty
briefly and do not present incomplete snippets as settled fact.
Lead with the clearest answer first, then organize supporting detail under
useful markdown headings when the question calls for depth.";

// --- SSE context event schema ---

#[derive(Debug, Serialize, Deserialize, Clone)]
pub(crate) struct ContextRecallItem {
    pub id: String,
    pub seed: String,
    pub score: f64,
    pub text: String,
    pub is_conscious: bool,
    pub category: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub epoch: u64,
    pub token_estimate: usize,
}

#[derive(Debug, Serialize)]
pub(crate) struct ContextEvent {
    pub conscious: Vec<ContextRecallItem>,
    pub subconscious: Vec<ContextRecallItem>,
    pub novel: Vec<ContextRecallItem>,
}

// --- Request types ---

#[derive(Debug, Deserialize)]
pub(crate) struct ChatRequest {
    pub message: String,
    #[serde(default)]
    pub conversation: Vec<ChatMessage>,
    pub model: Option<String>,
    #[serde(default = "default_mode")]
    pub mode: String,
    pub max_tokens: Option<u32>,
}

fn default_mode() -> String {
    "assistant".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct ChatMessage {
    pub role: String,
    pub content: String,
}

// --- SSE helpers ---

fn sse_event(event: &str, data: &str) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn sse_data(data: &str) -> String {
    if data.contains('\n') {
        // SSE spec: multi-line data uses multiple data: lines,
        // joined with \n on receive.
        let mut result = String::new();
        for line in data.split('\n') {
            result.push_str("data: ");
            result.push_str(line);
            result.push('\n');
        }
        result.push('\n');
        result
    } else {
        format!("data: {data}\n\n")
    }
}

fn sse_keepalive() -> String {
    ": keepalive\n\n".to_string()
}

fn sse_error(code: &str, message: &str) -> String {
    let json = serde_json::json!({"code": code, "message": message});
    sse_event("error", &json.to_string())
}

fn parse_context_items(
    value: Option<&serde_json::Value>,
) -> Result<Vec<ContextRecallItem>, serde_json::Error> {
    match value {
        Some(v) => serde_json::from_value(v.clone()),
        None => Ok(Vec::new()),
    }
}

fn format_recall_section(items: &[ContextRecallItem]) -> String {
    if items.is_empty() {
        return "- None recalled.\n".to_string();
    }

    items
        .iter()
        .map(|item| {
            format!(
                "- **{}**  \n  score: {:.2} | type: {} | epoch: {} | id: {}  \n  {}",
                item.seed,
                item.score,
                item.kind,
                item.epoch,
                item.id,
                item.text.trim()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn build_memory_context_message(
    mode: &str,
    dae_context: &str,
    context_event: &ContextEvent,
) -> String {
    let mode_guidance = match mode {
        "explorer" => {
            "Focus on explaining what the memory system appears to know, where the memories came from, and how the pieces connect."
        }
        _ => {
            "Use the recalled memory as evidence. Synthesize it into a coherent answer instead of echoing fragments verbatim."
        }
    };

    format!(
        concat!(
            "AM_MEMORY_CONTEXT\n\n",
            "Use this as supporting evidence for the next user question.\n",
            "{mode_guidance}\n\n",
            "## Conscious\n",
            "{conscious}",
            "\n## Subconscious\n",
            "{subconscious}",
            "\n## Novel\n",
            "{novel}",
            "\n## Composed Recall\n",
            "{dae_context}\n"
        ),
        mode_guidance = mode_guidance,
        conscious = format_recall_section(&context_event.conscious),
        subconscious = format_recall_section(&context_event.subconscious),
        novel = format_recall_section(&context_event.novel),
        dae_context = if dae_context.trim().is_empty() {
            "No composed recall available."
        } else {
            dae_context.trim()
        },
    )
}

fn extract_salient_tags(text: &str) -> Vec<String> {
    let mut results = Vec::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("<salient>") {
        let after_tag = &remaining[start + 9..];
        if let Some(end) = after_tag.find("</salient>") {
            let content = after_tag[..end].trim().to_string();
            if !content.is_empty() {
                results.push(content);
            }
            remaining = &after_tag[end + 10..];
        } else {
            break;
        }
    }
    results
}

fn resolve_api_key(headers: &HeaderMap) -> Option<String> {
    if let Some(auth) = headers.get("authorization")
        && let Ok(val) = auth.to_str()
        && let Some(token) = val.strip_prefix("Bearer ")
        && !token.is_empty()
    {
        return Some(token.to_string());
    }
    std::env::var("OPENROUTER_API_KEY").ok()
}

fn openrouter_timeout() -> Duration {
    let secs: u64 = std::env::var("OPENROUTER_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    Duration::from_secs(secs)
}

struct CancelOnDrop(CancellationToken);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

/// Unwrap a tool_result_text Value into the inner JSON.
fn unwrap_tool_result(result: &serde_json::Value) -> serde_json::Value {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| result.clone())
}

// --- Handler ---

pub(crate) async fn handle_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let request_id = Uuid::new_v4().to_string();
    let api_key = match resolve_api_key(&headers) {
        Some(key) => key,
        None => {
            tracing::warn!(
                request_id,
                "chat request rejected: missing OpenRouter API key"
            );
            return (
                StatusCode::BAD_REQUEST,
                sse_error(
                    "NO_API_KEY",
                    "No API key. Set OPENROUTER_API_KEY or pass Authorization: Bearer <key>",
                ),
            )
                .into_response();
        }
    };

    let model = req.model.as_deref().unwrap_or(DEFAULT_MODEL).to_string();
    let max_tokens = req.max_tokens.unwrap_or(DEFAULT_MAX_TOKENS);
    let mode = req.mode.clone();
    let user_message = req.message.clone();
    tracing::info!(
        request_id,
        mode,
        model,
        max_tokens,
        conversation_messages = req.conversation.len(),
        user_message_chars = user_message.chars().count(),
        "chat request accepted"
    );

    // Step 1: Query DAE for memory context via dispatch_tool
    let context_started = std::time::Instant::now();
    let query_args = serde_json::json!({"text": user_message});
    let (dae_context_str, context_event) = {
        let result = match state.server.dispatch_tool("am_query", &query_args) {
            Ok(v) => unwrap_tool_result(&v),
            Err(e) => {
                tracing::warn!(request_id, error = %e, "memory query failed");
                serde_json::json!({})
            }
        };
        let context = result["context"].as_str().unwrap_or("").to_string();
        let event = ContextEvent {
            conscious: parse_context_items(result.get("conscious")).unwrap_or_default(),
            subconscious: parse_context_items(result.get("subconscious")).unwrap_or_default(),
            novel: parse_context_items(result.get("novel")).unwrap_or_default(),
        };
        (context, event)
    };
    tracing::info!(
        request_id,
        elapsed_ms = context_started.elapsed().as_millis(),
        conscious = context_event.conscious.len(),
        subconscious = context_event.subconscious.len(),
        novel = context_event.novel.len(),
        "memory context prepared"
    );

    // Step 2: Build system prompt and memory context message
    let template = match mode.as_str() {
        "assistant" => ASSISTANT_SYSTEM_PROMPT,
        _ => EXPLORER_SYSTEM_PROMPT,
    };
    let system_prompt = template.to_string();
    let memory_context_message =
        build_memory_context_message(&mode, &dae_context_str, &context_event);

    // Step 3: Build messages array
    let mut messages: Vec<serde_json::Value> = Vec::new();
    let mut system_replaced = false;
    for msg in &req.conversation {
        if msg.role == "system" && !system_replaced {
            messages.push(serde_json::json!({"role": "system", "content": system_prompt}));
            system_replaced = true;
        } else {
            messages.push(serde_json::json!({"role": msg.role, "content": msg.content}));
        }
    }
    if !system_replaced {
        messages.insert(
            0,
            serde_json::json!({"role": "system", "content": system_prompt}),
        );
    }
    messages.push(serde_json::json!({
        "role": "user",
        "content": memory_context_message
    }));
    messages.push(serde_json::json!({"role": "user", "content": user_message}));

    // Step 4: Forward to OpenRouter
    let client = reqwest::Client::builder()
        .timeout(openrouter_timeout())
        .build()
        .unwrap_or_default();

    let openrouter_body = serde_json::json!({
        "model": model,
        "messages": messages,
        "max_tokens": max_tokens,
        "stream": true,
    });
    tracing::info!(request_id, "dispatching OpenRouter streaming request");

    let upstream_started = std::time::Instant::now();
    let openrouter_resp = match client
        .post(OPENROUTER_URL)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .json(&openrouter_body)
        .send()
        .await
    {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!(request_id, error = %e, "OpenRouter request failed");
            return (
                StatusCode::BAD_GATEWAY,
                sse_error(
                    "LLM_REQUEST_FAILED",
                    &format!("OpenRouter request failed: {e}"),
                ),
            )
                .into_response();
        }
    };

    if !openrouter_resp.status().is_success() {
        let status = openrouter_resp.status();
        let body = openrouter_resp.text().await.unwrap_or_default();
        tracing::warn!(request_id, status = %status, "OpenRouter returned error");
        return (
            StatusCode::BAD_GATEWAY,
            sse_error(
                "LLM_ERROR",
                &format!("OpenRouter returned {status}: {body}"),
            ),
        )
            .into_response();
    }
    tracing::info!(
        request_id,
        elapsed_ms = upstream_started.elapsed().as_millis(),
        "OpenRouter stream established"
    );

    let server_clone = Arc::clone(&state.server);
    let user_msg_clone = user_message.clone();
    let context_json = serde_json::to_string(&context_event).unwrap_or_default();
    let (tx, mut rx) = mpsc::channel::<String>(32);
    let disconnect_token = CancellationToken::new();
    let worker_cancel = disconnect_token.clone();
    let worker_request_id = request_id.clone();

    tokio::spawn(async move {
        // Emit context metadata before LLM tokens
        if tx.send(sse_event("context", &context_json)).await.is_err() {
            return;
        }

        let byte_stream = openrouter_resp.bytes_stream();
        let mut event_stream = byte_stream.eventsource();
        let mut full_response = String::new();
        let mut chunk_count = 0usize;
        let mut keepalive_interval =
            tokio::time::interval(Duration::from_secs(KEEPALIVE_INTERVAL_SECS));
        let mut first_chunk_received = false;
        let stream_started = std::time::Instant::now();

        loop {
            tokio::select! {
                _ = worker_cancel.cancelled() => {
                    tracing::info!(request_id = worker_request_id, "client disconnected");
                    return;
                }
                maybe_event = event_stream.next() => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            first_chunk_received = true;
                            if event.data == "[DONE]" {
                                let _ = tx.send(sse_data("[DONE]")).await;
                                break;
                            }
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event.data)
                                && let Some(content) = parsed["choices"][0]["delta"]["content"].as_str()
                                && !content.is_empty() {
                                    chunk_count += 1;
                                    full_response.push_str(content);
                                    if tx.send(sse_data(content)).await.is_err() {
                                        return;
                                    }
                                }
                        }
                        Some(Err(e)) => {
                            tracing::warn!(request_id = worker_request_id, error = %e, "stream error");
                            let _ = tx
                                .send(sse_error("LLM_STREAM_ERROR", &format!("Stream error: {e}")))
                                .await;
                            return;
                        }
                        None => {
                            let _ = tx.send(sse_data("[DONE]")).await;
                            break;
                        }
                    }
                }
                _ = keepalive_interval.tick(), if !first_chunk_received => {
                    if tx.send(sse_keepalive()).await.is_err() {
                        return;
                    }
                }
            }
        }

        tracing::info!(
            request_id = worker_request_id,
            elapsed_ms = stream_started.elapsed().as_millis(),
            chunk_count,
            response_chars = full_response.chars().count(),
            "stream completed"
        );
        post_response_ops(&server_clone, &user_msg_clone, &full_response, &worker_request_id);
    }.instrument(info_span!("chat_stream", request_id = %request_id, mode = %mode, model = %model)));

    let stream = async_stream::stream! {
        let _cancel_on_drop = CancelOnDrop(disconnect_token);
        while let Some(frame) = rx.recv().await {
            yield Ok::<_, std::convert::Infallible>(frame);
        }
    };

    let body = Body::from_stream(stream);
    (
        StatusCode::OK,
        [
            ("content-type", "text/event-stream"),
            ("cache-control", "no-cache"),
            ("connection", "keep-alive"),
        ],
        body,
    )
        .into_response()
}

/// Post-response operations: buffer exchange, activate response, extract salient tags.
fn post_response_ops(
    server: &Arc<AmServer<BrainStore>>,
    user_msg: &str,
    assistant_response: &str,
    request_id: &str,
) {
    if assistant_response.is_empty() {
        return;
    }

    let started = std::time::Instant::now();

    // Buffer the exchange
    let buffer_args = serde_json::json!({
        "user": user_msg,
        "assistant": assistant_response,
    });
    if let Err(e) = server.dispatch_tool("am_buffer", &buffer_args) {
        tracing::warn!(request_id, error = %e, "post-response buffer failed");
    }

    // Activate response connections
    let activate_args = serde_json::json!({"text": assistant_response});
    let _ = server.dispatch_tool("am_activate_response", &activate_args);

    // Extract and store salient tags
    let salient_tags = extract_salient_tags(assistant_response);
    for tag_content in salient_tags {
        let salient_args = serde_json::json!({"text": tag_content, "supersedes": []});
        let _ = server.dispatch_tool("am_salient", &salient_args);
    }

    tracing::info!(
        request_id,
        elapsed_ms = started.elapsed().as_millis(),
        "post-response operations complete"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_salient_tags_single() {
        let text = "Some text <salient>important insight</salient> more text";
        let tags = extract_salient_tags(text);
        assert_eq!(tags, vec!["important insight"]);
    }

    #[test]
    fn test_extract_salient_tags_multiple() {
        let text = "<salient>first</salient> middle <salient>second</salient>";
        let tags = extract_salient_tags(text);
        assert_eq!(tags, vec!["first", "second"]);
    }

    #[test]
    fn test_extract_salient_tags_none() {
        let text = "no tags here";
        let tags = extract_salient_tags(text);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_extract_salient_tags_unclosed() {
        let text = "<salient>unclosed tag without end";
        let tags = extract_salient_tags(text);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_extract_salient_tags_empty() {
        let text = "<salient></salient>";
        let tags = extract_salient_tags(text);
        assert!(tags.is_empty());
    }

    #[test]
    fn test_memory_context_message_contains_structured_recall() {
        let context = ContextEvent {
            conscious: vec![ContextRecallItem {
                id: "abc".to_string(),
                seed: "Helioy Crew".to_string(),
                score: 0.91,
                text: "Three-layer orchestration stack".to_string(),
                is_conscious: true,
                category: "Conscious".to_string(),
                kind: "Decision".to_string(),
                epoch: 7,
                token_estimate: 42,
            }],
            subconscious: Vec::new(),
            novel: Vec::new(),
        };

        let message = build_memory_context_message("assistant", "Composed test context", &context);
        assert!(message.contains("AM_MEMORY_CONTEXT"));
        assert!(message.contains("## Conscious"));
        assert!(message.contains("Helioy Crew"));
        assert!(message.contains("Composed test context"));
    }
}
