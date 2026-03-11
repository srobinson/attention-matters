use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get, routing::post};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;

use crate::server::{
    ActivateResponseRequest, BufferRequest, FeedbackRequest, ImportRequest, IngestRequest,
    McpBatchQueryRequest, QueryIndexRequest, QueryRequest, RetrieveByIdsRequest, SalientRequest,
    ServerState,
};

/// Shared application state passed to all HTTP handlers.
#[derive(Clone)]
pub(crate) struct AppState {
    pub inner: Arc<Mutex<ServerState>>,
}

/// Bind to `127.0.0.1:<port>` and return the listener.
///
/// Separated from `serve_http` so the caller can detect port conflicts
/// before spawning background tasks. A bind failure here gives the user
/// a clear error and non-zero exit code.
pub(crate) async fn bind_http(port: u16) -> Result<TcpListener> {
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind HTTP server to {addr} (port may be in use)"))
}

/// Serve HTTP on an already-bound listener.
///
/// Shuts down gracefully when `cancel` is triggered.
pub(crate) async fn serve_http(
    listener: TcpListener,
    state: Arc<Mutex<ServerState>>,
    cancel: CancellationToken,
) -> Result<()> {
    let app_state = AppState { inner: state };

    let app = Router::new()
        // Health check
        .route("/api/health", get(health_check))
        // AM memory tools (POST endpoints)
        .route("/api/am/query", post(handle_query))
        .route("/api/am/query-index", post(handle_query_index))
        .route("/api/am/retrieve", post(handle_retrieve))
        .route("/api/am/buffer", post(handle_buffer))
        .route(
            "/api/am/ingest",
            post(handle_ingest).layer(axum::extract::DefaultBodyLimit::max(1024 * 1024)), // 1MB
        )
        .route("/api/am/activate", post(handle_activate))
        .route("/api/am/salient", post(handle_salient))
        .route("/api/am/feedback", post(handle_feedback))
        .route("/api/am/batch-query", post(handle_batch_query))
        .route(
            "/api/am/import",
            post(handle_import).layer(axum::extract::DefaultBodyLimit::max(50 * 1024 * 1024)), // 50MB
        )
        // AM memory tools (GET endpoints)
        .route("/api/am/stats", get(handle_stats))
        .route("/api/am/export", get(handle_export))
        .route("/api/am/episodes", get(handle_episodes))
        // LLM proxy with SSE streaming
        .route("/api/chat", post(crate::llm_proxy::handle_chat))
        // Fallback for unmatched routes
        .fallback(handle_not_found)
        .with_state(app_state)
        // CORS: allow localhost/127.0.0.1 origins for local development.
        // Strict matching prevents localhost.evil.com from passing the check.
        .layer(
            CorsLayer::new()
                .allow_origin(AllowOrigin::predicate(|origin, _req_head| {
                    is_local_origin(origin.as_bytes())
                }))
                .allow_methods(Any)
                .allow_headers([CONTENT_TYPE, AUTHORIZATION]),
        )
        // Request/response tracing (URI, method, status code)
        .layer(TraceLayer::new_for_http());

    let addr = listener.local_addr()?;
    tracing::info!("HTTP server listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await
        .context("HTTP server error")?;

    tracing::info!("HTTP server shut down");
    Ok(())
}

// --- CORS origin validation ---

/// Check that an origin is exactly `http://localhost` or `http://127.0.0.1`,
/// optionally followed by `:<port>`. Rejects origins like `http://localhost.evil.com`.
fn is_local_origin(origin: &[u8]) -> bool {
    fn check(origin: &[u8], prefix: &[u8]) -> bool {
        if !origin.starts_with(prefix) {
            return false;
        }
        let rest = &origin[prefix.len()..];
        // Exact match (no port) or colon followed by 1+ digits
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
        let body = serde_json::json!({
            "error": self.message,
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

// --- Handlers ---

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    // Lock briefly to verify the state is accessible (confirms brain.db is live)
    let _guard = state.inner.lock().await;
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}

async fn handle_query(
    State(state): State<AppState>,
    Json(req): Json<QueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s.do_query(&req.text, req.max_tokens);
    Ok(Json(result))
}

async fn handle_query_index(
    State(state): State<AppState>,
    Json(req): Json<QueryIndexRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s.do_query_index(&req.text);
    Ok(Json(result))
}

async fn handle_retrieve(
    State(state): State<AppState>,
    Json(req): Json<RetrieveByIdsRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s.do_retrieve(&req.ids);
    Ok(Json(result))
}

async fn handle_buffer(
    State(state): State<AppState>,
    Json(req): Json<BufferRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s
        .do_buffer(&req.user, &req.assistant)
        .map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_ingest(
    State(state): State<AppState>,
    Json(req): Json<IngestRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s.do_ingest(&req.text, req.name.as_deref());
    Ok(Json(result))
}

async fn handle_activate(
    State(state): State<AppState>,
    Json(req): Json<ActivateResponseRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s.do_activate(&req.text);
    Ok(Json(result))
}

async fn handle_salient(
    State(state): State<AppState>,
    Json(req): Json<SalientRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s.do_salient(&req.text, &req.supersedes);
    Ok(Json(result))
}

async fn handle_feedback(
    State(state): State<AppState>,
    Json(req): Json<FeedbackRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s
        .do_feedback(&req.query, &req.neighborhood_ids, &req.signal)
        .map_err(bad_request)?;
    Ok(Json(result))
}

async fn handle_batch_query(
    State(state): State<AppState>,
    Json(req): Json<McpBatchQueryRequest>,
) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s.do_batch_query(&req.queries);
    Ok(Json(result))
}

async fn handle_stats(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let mut s = state.inner.lock().await;
    let result = s.do_stats();
    Ok(Json(result))
}

async fn handle_export(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let s = state.inner.lock().await;
    let json_str = s.do_export().map_err(internal_error)?;
    // Return raw JSON string with correct content type
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
    let mut s = state.inner.lock().await;
    let result = s.do_import(&req.state).map_err(internal_error)?;
    Ok(Json(result))
}

async fn handle_episodes(State(state): State<AppState>) -> Result<impl IntoResponse, ApiError> {
    let s = state.inner.lock().await;
    let result = s.do_episodes();
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
}
