use std::sync::Arc;

use am_core::{
    DAESystem, QueryEngine, compose_context, compute_surface, export_json, extract_salient,
    import_json, ingest_text,
};
use am_store::ProjectStore;
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::Mutex;

const BUFFER_THRESHOLD: usize = 5;

#[derive(Clone)]
pub struct AmServer {
    state: Arc<Mutex<ServerState>>,
    tool_router: ToolRouter<Self>,
}

struct ServerState {
    system: DAESystem,
    store: ProjectStore,
    rng: SmallRng,
}

impl AmServer {
    pub fn new(store: ProjectStore) -> Self {
        let system = store
            .load_project_system()
            .unwrap_or_else(|_| DAESystem::new("am"));
        let rng = SmallRng::from_os_rng();
        Self {
            state: Arc::new(Mutex::new(ServerState { system, store, rng })),
            tool_router: Self::tool_router(),
        }
    }

    fn stats_json(system: &mut DAESystem) -> serde_json::Value {
        let n = system.n();
        let episodes = system.episodes.len();
        let conscious = system.conscious_episode.neighborhoods.len();
        serde_json::json!({
            "n": n,
            "episodes": episodes,
            "conscious": conscious,
        })
    }
}

// --- Tool parameter types ---

#[derive(Debug, Deserialize, JsonSchema)]
struct QueryRequest {
    /// The text to query the memory system with
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ActivateResponseRequest {
    /// Response text to strengthen connections for
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct SalientRequest {
    /// Text to mark as conscious memory (may contain salient tags)
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct BufferRequest {
    /// User's message text
    user: String,
    /// Assistant's response text
    assistant: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct IngestRequest {
    /// Document text to ingest
    text: String,
    /// Optional name for the episode
    name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct ImportRequest {
    /// Full state JSON to import
    state: serde_json::Value,
}

#[tool_router]
impl AmServer {
    #[tool(
        description = "Query the DAE geometric memory system. Returns composed context with conscious, subconscious, and novel recall sections."
    )]
    async fn am_query(
        &self,
        Parameters(req): Parameters<QueryRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let system = &mut state.system;

        let query_result = QueryEngine::process_query(system, &req.text);
        let surface = compute_surface(system, &query_result);
        let composed = compose_context(system, &surface, &query_result, &query_result.interference);

        let result = serde_json::json!({
            "context": composed.context,
            "metrics": {
                "conscious": composed.metrics.conscious,
                "subconscious": composed.metrics.subconscious,
                "novel": composed.metrics.novel,
            },
            "stats": Self::stats_json(system),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Strengthen memory connections from response text. Activates words, applies drift and Kuramoto phase coupling, then persists to storage."
    )]
    async fn am_activate_response(
        &self,
        Parameters(req): Parameters<ActivateResponseRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let ServerState { system, store, .. } = &mut *state;

        let activation = QueryEngine::activate(system, &req.text);
        let all_refs: Vec<_> = activation
            .subconscious
            .iter()
            .chain(activation.conscious.iter())
            .copied()
            .collect();
        QueryEngine::drift_and_consolidate(system, &all_refs);
        let (_, word_groups) = QueryEngine::compute_interference(
            system,
            &activation.subconscious,
            &activation.conscious,
        );
        QueryEngine::apply_kuramoto_coupling(system, &word_groups);

        if let Err(e) = store.save_project_system(system) {
            tracing::error!("failed to persist after activate_response: {e}");
        }

        let result = serde_json::json!({
            "activated": all_refs.len(),
            "stats": Self::stats_json(system),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Mark text as a conscious memory (salient insight). Extracts <salient> tags if present, otherwise stores the full text. Writes to both project and global databases."
    )]
    async fn am_salient(
        &self,
        Parameters(req): Parameters<SalientRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        let stored = extract_salient(system, &req.text, rng);
        let stored = if stored == 0 {
            store
                .mark_salient(system, &req.text, rng)
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;
            1u32
        } else {
            if let Err(e) = store.save_project_system(system) {
                tracing::error!("failed to persist after salient: {e}");
            }
            stored
        };

        let result = serde_json::json!({
            "stored": stored,
            "stats": Self::stats_json(system),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Buffer a conversation exchange (user + assistant messages). After 5 exchanges, automatically creates a new episode from the buffer."
    )]
    async fn am_buffer(
        &self,
        Parameters(req): Parameters<BufferRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        let buffer_size = store
            .project_store()
            .append_buffer(&req.user, &req.assistant)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let mut episode_created: Option<String> = None;

        if buffer_size >= BUFFER_THRESHOLD {
            let exchanges = store
                .project_store()
                .drain_buffer()
                .map_err(|e| McpError::internal_error(e.to_string(), None))?;

            let combined: String = exchanges
                .iter()
                .map(|(u, a)| format!("{u}\n{a}"))
                .collect::<Vec<_>>()
                .join("\n\n");

            let episode = ingest_text(&combined, Some("conversation"), rng);
            let name = episode.name.clone();
            system.add_episode(episode);

            if let Err(e) = store.save_project_system(system) {
                tracing::error!("failed to persist after buffer episode: {e}");
            }

            episode_created = Some(name);
        }

        let result = serde_json::json!({
            "buffer_size": buffer_size,
            "episode_created": episode_created,
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Ingest a document as a new episode. Text is split into sentence chunks, tokenized, and placed on the geometric manifold. Persists to storage."
    )]
    async fn am_ingest(
        &self,
        Parameters(req): Parameters<IngestRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        let episode = ingest_text(&req.text, req.name.as_deref(), rng);
        let ep_name = episode.name.clone();
        let neighborhoods = episode.neighborhoods.len();
        let occurrences: usize = episode
            .neighborhoods
            .iter()
            .map(|n| n.occurrences.len())
            .sum();

        system.add_episode(episode);

        if let Err(e) = store.save_project_system(system) {
            tracing::error!("failed to persist after ingest: {e}");
        }

        let result = serde_json::json!({
            "episode": ep_name,
            "neighborhoods": neighborhoods,
            "occurrences": occurrences,
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Get DAE system diagnostics: total occurrences (N), episode count, and conscious memory count."
    )]
    async fn am_stats(&self) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let stats = Self::stats_json(&mut state.system);

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&stats).unwrap_or_default(),
        )]))
    }

    #[tool(description = "Export the full DAE system state as v0.7.2 compatible JSON.")]
    async fn am_export(&self) -> Result<CallToolResult, McpError> {
        let state = self.state.lock().await;
        let json = export_json(&state.system)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        Ok(CallToolResult::success(vec![Content::text(json)]))
    }

    #[tool(
        description = "Import a full DAE system state from v0.7.2 compatible JSON. Replaces current state."
    )]
    async fn am_import(
        &self,
        Parameters(req): Parameters<ImportRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let json_str = serde_json::to_string(&req.state)
            .map_err(|e| McpError::internal_error(e.to_string(), None))?;

        let imported = import_json(&json_str)
            .map_err(|e| McpError::internal_error(format!("invalid state JSON: {e}"), None))?;

        state.system = imported;

        if let Err(e) = state.store.save_project_system(&state.system) {
            tracing::error!("failed to persist after import: {e}");
        }

        let result = serde_json::json!({
            "imported": true,
            "stats": Self::stats_json(&mut state.system),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }
}

#[tool_handler]
impl ServerHandler for AmServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "DAE (Daemon Attention Engine) geometric memory system. \
                 Query memories, strengthen connections, mark salient insights, \
                 buffer conversations, ingest documents, and manage state."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
