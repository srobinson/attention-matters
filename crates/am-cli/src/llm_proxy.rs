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
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::http_server::AppState;
use crate::server::ServerState;

const DEFAULT_MODEL: &str = "anthropic/claude-3.5-haiku";
const DEFAULT_MAX_TOKENS: u32 = 4096;
const OPENROUTER_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
const KEEPALIVE_INTERVAL_SECS: u64 = 15;

// --- Chat mode system prompt templates ---

const EXPLORER_SYSTEM_PROMPT: &str = "\
You are a window into a geometric memory system built on the 3-sphere manifold.
The following context was recalled from memory based on the user's message.
Use it to answer questions about what you remember. Surface connections between
memories, trace how they relate, and help the user understand their memory landscape.
Do not fabricate memories. If the recall is empty or irrelevant, say so.

{dae_context}";

const ASSISTANT_SYSTEM_PROMPT: &str = "\
You are a helpful assistant with access to a geometric memory system.
The following context was recalled from memory and may be relevant.
Use it naturally in your responses. When you identify an important insight,
decision, or preference, wrap it in <salient> tags to store it as conscious memory.

{dae_context}";

// --- SSE context event schema ---

/// Schema for the `event: context` SSE payload.
/// Emitted before any LLM content tokens so the frontend can display recall metadata.
#[derive(Debug, Serialize)]
pub(crate) struct ContextEvent {
    /// Counts of recalled neighborhoods by category.
    pub metrics: Option<serde_json::Value>,
    /// UUIDs of recalled neighborhoods by category.
    pub recalled_ids: Option<serde_json::Value>,
    /// Approximate token counts by recall category.
    pub token_estimate: Option<serde_json::Value>,
    /// Top index entries (neighborhood ID, category, score, summary).
    pub index: Option<serde_json::Value>,
}

// --- Request types ---

#[derive(Debug, Deserialize)]
pub(crate) struct ChatRequest {
    /// The user's message
    pub message: String,
    /// Conversation history
    #[serde(default)]
    pub conversation: Vec<ChatMessage>,
    /// OpenRouter model identifier
    pub model: Option<String>,
    /// Chat mode: "explorer" or "assistant"
    #[serde(default = "default_mode")]
    pub mode: String,
    /// Maximum tokens for the response
    pub max_tokens: Option<u32>,
}

fn default_mode() -> String {
    "explorer".to_string()
}

#[derive(Debug, Clone, Deserialize, serde::Serialize)]
pub(crate) struct ChatMessage {
    pub role: String,
    pub content: String,
}

// --- SSE helpers ---

fn sse_event(event: &str, data: &str) -> String {
    format!("event: {event}\ndata: {data}\n\n")
}

fn sse_data(data: &str) -> String {
    format!("data: {data}\n\n")
}

fn sse_keepalive() -> String {
    ": keepalive\n\n".to_string()
}

fn sse_error(code: &str, message: &str) -> String {
    let json = serde_json::json!({"code": code, "message": message});
    sse_event("error", &json.to_string())
}

/// Extract <salient> tagged content from text.
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

/// Resolve the API key: prefer Authorization header, fall back to env var.
fn resolve_api_key(headers: &HeaderMap) -> Option<String> {
    // Check Authorization header first
    if let Some(auth) = headers.get("authorization")
        && let Ok(val) = auth.to_str()
        && let Some(token) = val.strip_prefix("Bearer ")
        && !token.is_empty()
    {
        return Some(token.to_string());
    }
    // Fall back to env var
    std::env::var("OPENROUTER_API_KEY").ok()
}

/// Build the OpenRouter timeout from env var or default.
fn openrouter_timeout() -> Duration {
    let secs: u64 = std::env::var("OPENROUTER_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(120);
    Duration::from_secs(secs)
}

// --- Handler ---

pub(crate) async fn handle_chat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(req): Json<ChatRequest>,
) -> impl IntoResponse {
    let api_key = match resolve_api_key(&headers) {
        Some(key) => key,
        None => {
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

    // Step 1: Query DAE for memory context
    let (dae_context_str, context_event) = {
        let mut s = state.inner.lock().await;
        let result = s.do_query(&user_message, None);
        let context = result["context"].as_str().unwrap_or("").to_string();
        let event = ContextEvent {
            metrics: result.get("metrics").cloned(),
            recalled_ids: result.get("recalled_ids").cloned(),
            token_estimate: result.get("token_estimate").cloned(),
            index: result.get("index").cloned(),
        };
        (context, event)
    };

    // Step 2: Build system prompt from mode template
    let template = match mode.as_str() {
        "assistant" => ASSISTANT_SYSTEM_PROMPT,
        _ => EXPLORER_SYSTEM_PROMPT,
    };
    let system_prompt = template.replace("{dae_context}", &dae_context_str);

    // Step 3: Build messages array
    let mut messages: Vec<serde_json::Value> = Vec::new();

    // Prepend or replace system message
    let mut system_injected = false;
    for msg in &req.conversation {
        if msg.role == "system" && !system_injected {
            // Replace existing system message with DAE-enriched one
            messages.push(serde_json::json!({"role": "system", "content": system_prompt}));
            system_injected = true;
        } else {
            messages.push(serde_json::json!({"role": msg.role, "content": msg.content}));
        }
    }
    if !system_injected {
        messages.insert(
            0,
            serde_json::json!({"role": "system", "content": system_prompt}),
        );
    }
    // Append current user message
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
        return (
            StatusCode::BAD_GATEWAY,
            sse_error(
                "LLM_ERROR",
                &format!("OpenRouter returned {status}: {body}"),
            ),
        )
            .into_response();
    }

    // Step 5-6: Stream SSE response with client disconnect cancellation.
    //
    // When the client drops the SSE connection, axum stops polling the stream.
    // The generator is then dropped, which triggers the CancellationToken via
    // the _cancel_guard. This cancels the `tokio::select!` branch waiting on
    // OpenRouter, causing the reqwest response to be dropped (closing the
    // upstream HTTP connection and stopping token consumption).
    let cancel = CancellationToken::new();
    let inner_state = Arc::clone(&state.inner);
    let user_msg_clone = user_message.clone();
    let context_json = serde_json::to_string(&context_event).unwrap_or_default();

    let stream = async_stream::stream! {
        // Guard: when this stream is dropped (client disconnect), cancel upstream.
        let _cancel_guard = cancel.clone().drop_guard();

        // Emit typed context metadata before any LLM tokens
        yield Ok::<_, std::convert::Infallible>(sse_event("context", &context_json));

        let byte_stream = openrouter_resp.bytes_stream();
        let mut event_stream = byte_stream.eventsource();
        let mut full_response = String::new();
        let mut keepalive_interval = tokio::time::interval(Duration::from_secs(KEEPALIVE_INTERVAL_SECS));
        let mut first_chunk_received = false;
        let mut client_disconnected = false;

        loop {
            tokio::select! {
                maybe_event = event_stream.next() => {
                    match maybe_event {
                        Some(Ok(event)) => {
                            first_chunk_received = true;
                            if event.data == "[DONE]" {
                                yield Ok(sse_data("[DONE]"));
                                break;
                            }
                            // Parse OpenRouter SSE chunk
                            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&event.data)
                                && let Some(content) = parsed["choices"][0]["delta"]["content"].as_str()
                                    && !content.is_empty() {
                                        full_response.push_str(content);
                                        yield Ok(sse_data(content));
                                    }
                        }
                        Some(Err(e)) => {
                            yield Ok(sse_error("LLM_STREAM_ERROR", &format!("Stream error: {e}")));
                            break;
                        }
                        None => {
                            // Stream ended without [DONE]
                            yield Ok(sse_data("[DONE]"));
                            break;
                        }
                    }
                }
                _ = keepalive_interval.tick(), if !first_chunk_received => {
                    yield Ok(sse_keepalive());
                }
                _ = cancel.cancelled() => {
                    tracing::info!("client disconnected, cancelling upstream OpenRouter request");
                    client_disconnected = true;
                    break;
                }
            }
        }

        // Post-response memory operations only if the client stayed connected
        // and we received a meaningful response.
        if !client_disconnected {
            post_response_ops(&inner_state, &user_msg_clone, &full_response).await;
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
/// All errors are logged, never propagated.
async fn post_response_ops(
    state: &Arc<Mutex<ServerState>>,
    user_msg: &str,
    assistant_response: &str,
) {
    if assistant_response.is_empty() {
        return;
    }

    let mut s = state.lock().await;

    // Buffer the exchange
    if let Err(e) = s.do_buffer(user_msg, assistant_response) {
        tracing::warn!("post-response buffer failed: {e}");
    }

    // Activate response connections
    let _ = s.do_activate(assistant_response);

    // Extract and store salient tags
    let salient_tags = extract_salient_tags(assistant_response);
    for tag_content in salient_tags {
        let _ = s.do_salient(&tag_content, &[]);
    }
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
    fn test_explorer_prompt_contains_context() {
        let prompt = EXPLORER_SYSTEM_PROMPT.replace("{dae_context}", "test context");
        assert!(prompt.contains("test context"));
        assert!(prompt.contains("geometric memory"));
        assert!(prompt.contains("Do not fabricate"));
    }

    #[test]
    fn test_assistant_prompt_contains_salient() {
        let prompt = ASSISTANT_SYSTEM_PROMPT.replace("{dae_context}", "test context");
        assert!(prompt.contains("test context"));
        assert!(prompt.contains("<salient>"));
    }
}
