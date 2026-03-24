use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use anyhow::{Context, Result};
use axum::extract::{MatchedPath, Path, State};
use axum::http::Request;
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get, routing::post};
use serde::Deserialize;
use serde_json::Value;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing::field::Empty;

use am_store::project::BrainStore;

use crate::server::AmServer;

// --- Shared state ---

/// Shared application state passed to all HTTP handlers.
#[derive(Clone)]
pub(crate) struct AppState {
    pub server: Arc<AmServer<BrainStore>>,
}

/// Unwrap a tool_result_text Value into the inner JSON.
///
/// dispatch_tool returns `{"content": [{"type":"text","text":"<json>"}]}`.
/// HTTP handlers need the raw parsed JSON.
fn unwrap_tool_result(result: &Value) -> Value {
    result
        .get("content")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|item| item.get("text"))
        .and_then(|t| t.as_str())
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_else(|| result.clone())
}

/// Dispatch a tool call and unwrap the MCP envelope.
fn dispatch(server: &AmServer<BrainStore>, tool: &str, args: &Value) -> Result<Value, String> {
    server
        .dispatch_tool(tool, args)
        .map(|v| unwrap_tool_result(&v))
}

// --- Bind / Serve ---

pub(crate) async fn bind_http(port: u16) -> Result<TcpListener> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    tracing::info!(%addr, "binding HTTP server listener");
    TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind HTTP server to {addr} (port may be in use)"))
}

pub(crate) async fn serve_http(
    listener: TcpListener,
    server: Arc<AmServer<BrainStore>>,
    cancel: CancellationToken,
) -> Result<()> {
    let app_state = AppState { server };
    tracing::info!("building HTTP router");

    let app = Router::new()
        .route("/api/health", get(health_check))
        .route("/api/am/query", post(handle_query))
        .route("/api/am/query-index", post(handle_query_index))
        .route("/api/am/retrieve", post(handle_retrieve))
        .route("/api/am/buffer", post(handle_buffer))
        .route(
            "/api/am/ingest",
            post(handle_ingest).layer(axum::extract::DefaultBodyLimit::max(1024 * 1024)),
        )
        .route("/api/am/activate", post(handle_activate))
        .route("/api/am/salient", post(handle_salient))
        .route("/api/am/feedback", post(handle_feedback))
        .route("/api/am/batch-query", post(handle_batch_query))
        .route(
            "/api/am/import",
            post(handle_import).layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .route("/api/am/stats", get(handle_stats))
        .route("/api/am/export", get(handle_export))
        .route("/api/am/episodes", get(handle_episodes))
        .route(
            "/api/am/episodes/{id}/neighborhoods",
            get(handle_episode_neighborhoods),
        )
        .route("/api/chat", post(crate::llm_proxy::handle_chat))
        .fallback(handle_not_found)
        .with_state(app_state)
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _req_head| {
                    is_local_origin(origin.as_bytes())
                }))
                .allow_methods(Any)
                .allow_headers([CONTENT_TYPE, AUTHORIZATION]),
        )
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(|request: &Request<_>| {
                    let matched_path = request
                        .extensions()
                        .get::<MatchedPath>()
                        .map(MatchedPath::as_str);
                    let user_agent = request
                        .headers()
                        .get(axum::http::header::USER_AGENT)
                        .and_then(|value| value.to_str().ok())
                        .unwrap_or("-");
                    tracing::info_span!(
                        "http.request",
                        request_id = %uuid::Uuid::new_v4(),
                        method = %request.method(),
                        uri = %request.uri(),
                        matched_path = matched_path.unwrap_or("-"),
                        user_agent = %user_agent,
                        status = Empty,
                        latency_ms = Empty,
                    )
                })
                .on_request(|request: &Request<_>, _span: &tracing::Span| {
                    tracing::trace!(
                        method = %request.method(),
                        uri = %request.uri(),
                        "received HTTP request"
                    );
                })
                .on_response(
                    |response: &axum::response::Response,
                     latency: std::time::Duration,
                     span: &tracing::Span| {
                        span.record("status", response.status().as_u16());
                        span.record("latency_ms", latency.as_millis() as i64);
                        tracing::info!(
                            status = response.status().as_u16(),
                            latency_ms = latency.as_millis(),
                            "completed HTTP request"
                        );
                    },
                )
                .on_failure(
                    |error: tower_http::classify::ServerErrorsFailureClass,
                     latency: std::time::Duration,
                     span: &tracing::Span| {
                        span.record("latency_ms", latency.as_millis() as i64);
                        tracing::warn!(
                            latency_ms = latency.as_millis(),
                            failure = ?error,
                            "HTTP request failed"
                        );
                    },
                ),
        );

    let addr = listener.local_addr()?;
    tracing::info!(%addr, "HTTP server listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await
        .context("HTTP server error")?;

    tracing::info!("HTTP server shut down");
    Ok(())
}

// --- CORS origin validation ---

fn is_local_origin(origin: &[u8]) -> bool {
    fn check(origin: &[u8], prefix: &[u8]) -> bool {
        if !origin.starts_with(prefix) {
            return false;
        }
        let rest = &origin[prefix.len()..];
        rest.is_empty()
            || (rest.starts_with(b":")
                && rest.len() > 1
                && rest[1..].iter().all(|b| b.is_ascii_digit()))
    }
    check(origin, b"http://localhost") || check(origin, b"http://127.0.0.1")
}

// --- Error response helper ---

struct ApiError {
    code: String,
    message: String,
    status: StatusCode,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let request_id = uuid::Uuid::new_v4().to_string();
        tracing::warn!(
            code = %self.code,
            status = self.status.as_u16(),
            request_id = %request_id,
            message = %self.message,
            "returning API error"
        );
        let body = serde_json::json!({
            "message": self.message,
            "code": self.code,
            "request_id": request_id,
        });
        (self.status, Json(body)).into_response()
    }
}

fn bad_request(message: impl Into<String>) -> ApiError {
    ApiError {
        code: "INVALID_REQUEST".to_string(),
        message: message.into(),
        status: StatusCode::BAD_REQUEST,
    }
}

fn internal_error(message: impl Into<String>) -> ApiError {
    ApiError {
        code: "INTERNAL_ERROR".to_string(),
        message: message.into(),
        status: StatusCode::INTERNAL_SERVER_ERROR,
    }
}

async fn handle_not_found() -> impl IntoResponse {
    ApiError {
        code: "NOT_FOUND".to_string(),
        message: "endpoint not found".to_string(),
        status: StatusCode::NOT_FOUND,
    }
}

// --- Request types for HTTP endpoints ---

#[derive(Debug, Deserialize, serde::Serialize)]
struct QueryRequest {
    text: String,
    max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct QueryIndexRequest {
    text: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct RetrieveByIdsRequest {
    ids: Vec<String>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct BufferRequest {
    user: String,
    assistant: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct IngestRequest {
    text: String,
    name: Option<String>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct ActivateResponseRequest {
    text: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct SalientRequest {
    text: String,
    #[serde(default)]
    supersedes: Vec<String>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct FeedbackRequest {
    query: String,
    neighborhood_ids: Vec<String>,
    signal: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct BatchQueryItem {
    query: String,
    max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct McpBatchQueryRequest {
    queries: Vec<BatchQueryItem>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct ImportRequest {
    state: serde_json::Value,
}

// --- Handlers ---

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    let started = Instant::now();
    // dispatch a lightweight stats call to verify the brain is accessible
    let _ = state
        .server
        .dispatch_tool("am_stats", &serde_json::json!({}));
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis(),
        "health check ok"
    );
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

async fn handle_query(
    State(state): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let started = Instant::now();
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_query", &args).map_err(internal_error)?;
    tracing::info!(
        elapsed_ms = started.elapsed().as_millis(),
        "query completed"
    );
    Ok(Json(result))
}

async fn handle_query_index(
    State(state): State<AppState>,
    Json(req): Json<QueryIndexRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_query_index", &args).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_retrieve(
    State(state): State<AppState>,
    Json(req): Json<RetrieveByIdsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_retrieve", &args).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_buffer(
    State(state): State<AppState>,
    Json(req): Json<BufferRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_buffer", &args).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_ingest(
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_ingest", &args).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_activate(
    State(state): State<AppState>,
    Json(req): Json<ActivateResponseRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_activate_response", &args).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_salient(
    State(state): State<AppState>,
    Json(req): Json<SalientRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_salient", &args).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_feedback(
    State(state): State<AppState>,
    Json(req): Json<FeedbackRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_feedback", &args).map_err(bad_request)?;
    Ok(Json(result))
}

async fn handle_batch_query(
    State(state): State<AppState>,
    Json(req): Json<McpBatchQueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_batch_query", &args).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_stats(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let result =
        dispatch(&state.server, "am_stats", &serde_json::json!({})).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_export(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let result =
        dispatch(&state.server, "am_export", &serde_json::json!({})).map_err(internal_error)?;
    let json_str = serde_json::to_string(&result).map_err(|e| internal_error(e.to_string()))?;
    Ok((
        StatusCode::OK,
        [("content-type", "application/json")],
        json_str,
    ))
}

async fn handle_import(
    State(state): State<AppState>,
    Json(req): Json<ImportRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::to_value(&req).map_err(|e| internal_error(e.to_string()))?;
    let result = dispatch(&state.server, "am_import", &args).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_episodes(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let result =
        dispatch(&state.server, "am_episodes", &serde_json::json!({})).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_episode_neighborhoods(
    State(state): State<AppState>,
    Path(episode_id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let args = serde_json::json!({"episode_id": episode_id});
    let result =
        dispatch(&state.server, "am_episode_neighborhoods", &args).map_err(|msg| ApiError {
            code: "NOT_FOUND".to_string(),
            message: msg,
            status: StatusCode::NOT_FOUND,
        })?;
    Ok(Json(result))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_local_origin_exact_localhost() {
        assert!(is_local_origin(b"http://localhost"));
    }

    #[test]
    fn test_local_origin_localhost_with_port() {
        assert!(is_local_origin(b"http://localhost:3000"));
        assert!(is_local_origin(b"http://localhost:8080"));
    }

    #[test]
    fn test_local_origin_exact_ip() {
        assert!(is_local_origin(b"http://127.0.0.1"));
    }

    #[test]
    fn test_local_origin_ip_with_port() {
        assert!(is_local_origin(b"http://127.0.0.1:3001"));
    }

    #[test]
    fn test_local_origin_rejects_subdomain() {
        assert!(!is_local_origin(b"http://localhost.evil.com"));
        assert!(!is_local_origin(b"http://localhost.com"));
    }

    #[test]
    fn test_local_origin_rejects_path() {
        assert!(!is_local_origin(b"http://localhost/foo"));
    }

    #[test]
    fn test_local_origin_rejects_empty_port() {
        assert!(!is_local_origin(b"http://localhost:"));
    }

    #[test]
    fn test_local_origin_rejects_non_digit_port() {
        assert!(!is_local_origin(b"http://localhost:abc"));
    }

    #[test]
    fn test_local_origin_rejects_other_hosts() {
        assert!(!is_local_origin(b"http://example.com"));
        assert!(!is_local_origin(b"https://localhost:3000"));
    }

    #[test]
    fn test_unwrap_tool_result_normal() {
        let wrapped = serde_json::json!({
            "content": [{"type": "text", "text": "{\"key\":\"value\"}"}]
        });
        let unwrapped = unwrap_tool_result(&wrapped);
        assert_eq!(unwrapped, serde_json::json!({"key": "value"}));
    }

    #[test]
    fn test_unwrap_tool_result_passthrough() {
        let raw = serde_json::json!({"key": "value"});
        let unwrapped = unwrap_tool_result(&raw);
        assert_eq!(unwrapped, raw);
    }
}
