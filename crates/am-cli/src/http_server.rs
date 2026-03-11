use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Json, Router, routing::get};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use crate::server::ServerState;

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
        .route("/api/health", get(health_check))
        .with_state(app_state);

    let addr = listener.local_addr()?;
    tracing::info!("HTTP server listening on {addr}");

    axum::serve(listener, app)
        .with_graceful_shutdown(cancel.cancelled_owned())
        .await
        .context("HTTP server error")?;

    tracing::info!("HTTP server shut down");
    Ok(())
}

async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    // Lock briefly to verify the state is accessible (confirms brain.db is live)
    let _guard = state.inner.lock().await;
    (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
}
