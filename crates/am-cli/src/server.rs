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

const BUFFER_THRESHOLD: usize = 3;

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
        description = "Query geometric memory. Call this at the START of every session with the user's first message to recall relevant context from past sessions. Returns conscious recall (insights you previously marked important), subconscious recall (relevant past conversations/documents), and novel connections (lateral associations). Use the returned context silently — weave it into your response naturally without announcing 'I remember...'."
    )]
    async fn am_query(
        &self,
        Parameters(req): Parameters<QueryRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        // Flush any orphaned buffer from previous sessions into an episode
        let orphaned = store.project_store().buffer_count().unwrap_or(0);
        if orphaned > 0
            && let Ok(exchanges) = store.project_store().drain_buffer()
        {
            let combined: String = exchanges
                .iter()
                .map(|(u, a)| format!("{u}\n{a}"))
                .collect::<Vec<_>>()
                .join("\n\n");
            let episode = ingest_text(&combined, Some("conversation"), rng);
            system.add_episode(episode);
            if let Err(e) = store.save_project_system(system) {
                tracing::error!("failed to persist flushed buffer episode: {e}");
            }
        }

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
        description = "Strengthen memory connections from your response text. Call this after giving a substantive response — it activates matching memories, drifts related concepts closer together on the manifold, and applies phase coupling. This is how the memory system consolidates over time. Not needed for every response — use after meaningful technical exchanges, not simple acknowledgements."
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
        description = "Mark an insight as conscious memory — something worth remembering across sessions and across projects. Use for: architecture decisions, user preferences, recurring patterns, hard-won debugging insights, project conventions. These surface as CONSCIOUS RECALL in future queries. Be selective — mark only genuinely reusable insights, not routine facts. Writes to both project-local and global (cross-project) memory."
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
        description = "Buffer a conversation exchange. Call with each substantive user/assistant exchange pair. After 3 exchanges, automatically creates a memory episode on the geometric manifold. This is how conversations become searchable memories in future sessions. Skip trivial exchanges (greetings, confirmations) — buffer the ones with real content."
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
        description = "Ingest a document as a memory episode. Use when the user shares important reference material (design docs, specs, READMEs) that should be searchable in future sessions. Text is chunked into neighborhoods and placed on the geometric manifold."
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
        description = "Get memory system statistics: total occurrences (N), episode count, and conscious memory count. Useful for understanding memory state. Not needed routinely — call when the user asks about memory or for diagnostics."
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
                "You have persistent geometric memory across sessions via the DAE attention engine.\n\n\
                 SESSION LIFECYCLE:\n\
                 1. RECALL: At session start, call am_query with the user's first message or task description. \
                    Use returned context silently — integrate naturally, never announce \"I remember...\".\n\
                 2. ENGAGE: During the session, call am_buffer with substantive exchange pairs. \
                    Skip trivial exchanges (greetings, yes/no). After 3 buffered exchanges, \
                    a memory episode is created automatically. Any leftover buffer is flushed \
                    into an episode at the start of the next session.\n\
                 3. STRENGTHEN: After giving a meaningful technical response, call am_activate_response \
                    with your response text to consolidate related memories.\n\
                 4. MARK INSIGHTS: When you discover architecture decisions, user preferences, \
                    recurring patterns, or hard-won insights, call am_salient to store them as \
                    conscious memory. These persist globally across all projects.\n\n\
                 PRINCIPLES:\n\
                 - Memory should be invisible to the user. Don't mention the memory system unless asked.\n\
                 - Be selective with am_salient — mark genuinely reusable insights, not routine facts.\n\
                 - If am_query returns empty, that's fine — the project is new. Don't mention it.\n\
                 - Novel connections in query results are lateral associations — use them for creative leaps."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_server() -> AmServer {
        let store = ProjectStore::open_in_memory().unwrap();
        AmServer::new(store)
    }

    fn text_from_result(result: &CallToolResult) -> String {
        result
            .content
            .iter()
            .filter_map(|c| match &c.raw {
                RawContent::Text(t) => Some(t.text.clone()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("")
    }

    fn parse_result(result: &CallToolResult) -> serde_json::Value {
        let text = text_from_result(result);
        serde_json::from_str(&text).expect("handler should return valid JSON")
    }

    #[tokio::test]
    async fn test_am_stats_empty() {
        let server = make_server();
        let result = server.am_stats().await.unwrap();
        let json = parse_result(&result);

        assert_eq!(json["n"], 0);
        assert_eq!(json["episodes"], 0);
        assert_eq!(json["conscious"], 0);
    }

    #[tokio::test]
    async fn test_am_ingest() {
        let server = make_server();

        let result = server
            .am_ingest(Parameters(IngestRequest {
                text: "The quick brown fox jumps over the lazy dog. Sentence two here. And a third sentence for good measure.".to_string(),
                name: Some("test-doc".to_string()),
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        assert_eq!(json["episode"], "test-doc");
        assert!(json["neighborhoods"].as_u64().unwrap() >= 1);
        assert!(json["occurrences"].as_u64().unwrap() > 0);

        // Stats should reflect the ingestion
        let stats = parse_result(&server.am_stats().await.unwrap());
        assert!(stats["n"].as_u64().unwrap() > 0);
        assert_eq!(stats["episodes"], 1);
    }

    #[tokio::test]
    async fn test_am_query_response_structure() {
        let server = make_server();

        // Ingest content first
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Quantum mechanics describes particle behavior at subatomic scales. Wave functions collapse on measurement.".to_string(),
                name: Some("science".to_string()),
            }))
            .await
            .unwrap();

        // Add conscious content
        server
            .am_salient(Parameters(SalientRequest {
                text: "quantum computing is revolutionary".to_string(),
            }))
            .await
            .unwrap();

        // Query
        let result = server
            .am_query(Parameters(QueryRequest {
                text: "quantum particles".to_string(),
            }))
            .await
            .unwrap();

        let json = parse_result(&result);

        // Verify response structure has required fields
        assert!(json.get("context").is_some(), "should have context field");
        assert!(json.get("metrics").is_some(), "should have metrics field");
        assert!(json.get("stats").is_some(), "should have stats field");

        let metrics = &json["metrics"];
        assert!(metrics.get("conscious").is_some());
        assert!(metrics.get("subconscious").is_some());
        assert!(metrics.get("novel").is_some());

        let stats = &json["stats"];
        assert!(stats["n"].as_u64().unwrap() > 0);
    }

    #[tokio::test]
    async fn test_am_salient_stores_conscious() {
        let server = make_server();

        let result = server
            .am_salient(Parameters(SalientRequest {
                text: "important insight about neural networks".to_string(),
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        assert_eq!(json["stored"], 1);

        // Stats should show conscious memory
        let stats = parse_result(&server.am_stats().await.unwrap());
        assert!(
            stats["conscious"].as_u64().unwrap() >= 1,
            "should have at least one conscious neighborhood"
        );
    }

    #[tokio::test]
    async fn test_am_salient_with_tags() {
        let server = make_server();

        let result = server
            .am_salient(Parameters(SalientRequest {
                text: "Normal text <salient>first insight</salient> middle <salient>second insight</salient> end".to_string(),
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        assert_eq!(json["stored"], 2);

        let stats = parse_result(&server.am_stats().await.unwrap());
        assert!(stats["conscious"].as_u64().unwrap() >= 2);
    }

    #[tokio::test]
    async fn test_am_activate_response() {
        let server = make_server();

        // Ingest content first
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Machine learning enables pattern recognition in data. Neural networks learn representations.".to_string(),
                name: Some("ml-doc".to_string()),
            }))
            .await
            .unwrap();

        let result = server
            .am_activate_response(Parameters(ActivateResponseRequest {
                text: "machine learning neural networks".to_string(),
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        assert!(json["activated"].as_u64().unwrap() > 0);
        assert!(json.get("stats").is_some());
    }

    #[tokio::test]
    async fn test_am_buffer() {
        let server = make_server();

        // Buffer exchanges below threshold (threshold is 3)
        for i in 0..2 {
            let result = server
                .am_buffer(Parameters(BufferRequest {
                    user: format!("User message {i}"),
                    assistant: format!("Assistant response {i}"),
                }))
                .await
                .unwrap();

            let json = parse_result(&result);
            assert_eq!(json["buffer_size"], i + 1);
            assert!(json["episode_created"].is_null());
        }

        // 3rd exchange should trigger episode creation
        let result = server
            .am_buffer(Parameters(BufferRequest {
                user: "User message 2".to_string(),
                assistant: "Assistant response 2".to_string(),
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        assert_eq!(json["buffer_size"], 3);
        assert!(
            json["episode_created"].is_string(),
            "should create episode after 3 exchanges"
        );

        let stats = parse_result(&server.am_stats().await.unwrap());
        assert_eq!(stats["episodes"], 1);
    }

    #[tokio::test]
    async fn test_am_export_import_roundtrip() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Roundtrip test content. Multiple sentences for neighborhoods. And one more sentence.".to_string(),
                name: Some("roundtrip".to_string()),
            }))
            .await
            .unwrap();

        // Get stats before export
        let stats_before = parse_result(&server.am_stats().await.unwrap());

        // Export
        let export_result = server.am_export().await.unwrap();
        let exported_json = text_from_result(&export_result);
        assert!(!exported_json.is_empty());

        // Create a fresh server and import
        let server2 = make_server();
        let state_value: serde_json::Value = serde_json::from_str(&exported_json).unwrap();

        let import_result = server2
            .am_import(Parameters(ImportRequest { state: state_value }))
            .await
            .unwrap();

        let import_json = parse_result(&import_result);
        assert_eq!(import_json["imported"], true);

        // Verify stats match
        let stats_after = parse_result(&server2.am_stats().await.unwrap());
        assert_eq!(stats_before["n"], stats_after["n"]);
        assert_eq!(stats_before["episodes"], stats_after["episodes"]);
    }

    #[tokio::test]
    async fn test_am_stats_after_operations() {
        let server = make_server();

        // Ingest
        server
            .am_ingest(Parameters(IngestRequest {
                text:
                    "First document about testing. With multiple sentences here. And a final line."
                        .to_string(),
                name: Some("doc1".to_string()),
            }))
            .await
            .unwrap();

        let stats1 = parse_result(&server.am_stats().await.unwrap());
        let n1 = stats1["n"].as_u64().unwrap();
        assert!(n1 > 0);
        assert_eq!(stats1["episodes"], 1);

        // Ingest second document
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Second document about verification. Has different content entirely. Nothing overlaps.".to_string(),
                name: Some("doc2".to_string()),
            }))
            .await
            .unwrap();

        let stats2 = parse_result(&server.am_stats().await.unwrap());
        assert!(stats2["n"].as_u64().unwrap() > n1);
        assert_eq!(stats2["episodes"], 2);

        // Mark salient
        server
            .am_salient(Parameters(SalientRequest {
                text: "key insight".to_string(),
            }))
            .await
            .unwrap();

        let stats3 = parse_result(&server.am_stats().await.unwrap());
        assert!(stats3["conscious"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn test_am_query_flushes_orphaned_buffer() {
        let server = make_server();

        // Buffer 2 exchanges (below threshold — simulates a session that ended early)
        for i in 0..2 {
            server
                .am_buffer(Parameters(BufferRequest {
                    user: format!("Orphaned user message {i}"),
                    assistant: format!("Orphaned assistant response {i}"),
                }))
                .await
                .unwrap();
        }

        // No episode yet
        let stats = parse_result(&server.am_stats().await.unwrap());
        assert_eq!(stats["episodes"], 0);

        // Calling am_query (simulating next session start) should flush the orphaned buffer
        let result = server
            .am_query(Parameters(QueryRequest {
                text: "orphaned message".to_string(),
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        assert!(json.get("stats").is_some());
        // The orphaned buffer should have been flushed into an episode
        assert_eq!(json["stats"]["episodes"], 1);
    }

    #[test]
    fn test_tool_registration() {
        let server = make_server();
        let info = server.get_info();

        assert!(info.instructions.is_some());
        assert!(info.capabilities.tools.is_some());
    }
}
