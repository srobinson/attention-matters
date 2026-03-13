use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use rustc_hash::FxHasher;
use std::sync::Arc;
use std::time::Instant;

use am_core::{
    BatchQueryEngine, BatchQueryRequest, BudgetConfig, DAESystem, DaemonPhasor, FeedbackSignal,
    Quaternion, QueryEngine, QueryManifest, RecallCategory, apply_feedback, compose_context,
    compose_context_budgeted, compose_index, compute_surface, export_json, extract_salient,
    import_json, ingest_text, mark_salient_typed, retrieve_by_ids,
};
use am_store::{BrainStore, StoreError};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::*;
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::sync::Mutex;
use uuid::Uuid;

const BUFFER_THRESHOLD: usize = 3;
const DEDUP_WINDOW_SECS: u64 = 60;
/// Maximum input size for text-accepting MCP tools (1 MB).
const MAX_TOOL_INPUT_BYTES: usize = 1_048_576;

/// Reject input that exceeds the per-tool byte limit.
fn check_input_size(value: &str, field: &str) -> Result<(), McpError> {
    if value.len() > MAX_TOOL_INPUT_BYTES {
        return Err(McpError::invalid_params(
            format!("{field} exceeds {} byte limit", MAX_TOOL_INPUT_BYTES),
            None,
        ));
    }
    Ok(())
}

/// Convert a `StoreError` to an `McpError`, preserving the variant name as a
/// machine-readable prefix so callers can distinguish error classes without
/// parsing free-form text.
fn store_err_to_mcp(e: StoreError) -> McpError {
    let (category, detail) = match &e {
        StoreError::Sqlite(inner) => ("sqlite", inner.to_string()),
        StoreError::Io(inner) => ("io", inner.to_string()),
        StoreError::InvalidData(msg) => ("invalid_data", msg.clone()),
    };
    McpError::internal_error(format!("[{category}] {detail}"), None)
}

/// Convert a serialization error to an `McpError` with a `[serde]` prefix.
fn serde_err_to_mcp(e: impl std::fmt::Display) -> McpError {
    McpError::internal_error(format!("[serde] {e}"), None)
}

#[derive(Clone)]
pub struct AmServer {
    state: Arc<Mutex<ServerState>>,
    tool_router: ToolRouter<Self>,
}

/// All mutable server state behind a single `tokio::sync::Mutex`.
///
/// # Concurrency model
///
/// Every MCP tool handler acquires `state.lock().await` for its full duration.
/// This serializes all tool calls: no two tools execute concurrently. This is
/// correct and intentional for the current deployment model (single client via
/// stdio transport, one Claude Code session per process).
///
/// # What changes for multi-client support
///
/// If the transport changes to SSE or WebSocket with concurrent clients, the
/// single mutex becomes a throughput bottleneck. The recommended decomposition:
///
/// - `RwLock<DAESystem>` for the in-memory system (readers: am_query, am_stats,
///   am_export; writers: am_ingest, am_salient, am_feedback, am_activate_response)
/// - `Mutex<Store>` for SQLite writes (rusqlite::Connection is !Sync, requires
///   exclusive access or a connection pool)
/// - Separate `Mutex<SessionState>` for session_recalled and dedup_window
///   (per-session state that does not interact with the core system)
///
/// The `SmallRng` would move to per-request construction (already cheap) or
/// thread-local storage.
struct ServerState {
    system: DAESystem,
    store: BrainStore,
    rng: SmallRng,
    /// Neighborhood recall counts this session (process lifetime).
    /// Tracks how many times each neighborhood has been returned.
    /// Non-decision neighborhoods get diminishing returns on repeated recalls.
    session_recalled: HashMap<Uuid, u32>,
    /// Content hashes with timestamps for dedup within a time window.
    /// Prevents duplicate episodes when am_buffer is called with identical content.
    dedup_window: HashMap<u64, Instant>,
}

/// Collect current `(Uuid, Quaternion, DaemonPhasor)` tuples for a set of occurrence IDs.
///
/// Scans all episodes (including conscious) to find occurrences matching the
/// given UUIDs. Used to prepare data for `save_occurrence_positions` after
/// drift or Kuramoto coupling has modified positions/phasors in memory.
fn collect_occurrence_positions(
    system: &DAESystem,
    ids: &[Uuid],
) -> Vec<(Uuid, Quaternion, DaemonPhasor)> {
    if ids.is_empty() {
        return Vec::new();
    }
    let target: std::collections::HashSet<Uuid> = ids.iter().copied().collect();
    let mut result = Vec::with_capacity(ids.len());

    let all_episodes = system
        .episodes
        .iter()
        .chain(std::iter::once(&system.conscious_episode));

    for episode in all_episodes {
        for nbhd in &episode.neighborhoods {
            for occ in &nbhd.occurrences {
                if target.contains(&occ.id) {
                    result.push((occ.id, occ.position, occ.phasor));
                }
            }
        }
    }

    result
}

/// Persist query manifest mutations to the store: drifted positions and
/// activated occurrence counts.
fn persist_manifest(
    store: &BrainStore,
    system: &DAESystem,
    manifest: &QueryManifest,
    context: &str,
) {
    if !manifest.drifted.is_empty() {
        let positions = collect_occurrence_positions(system, &manifest.drifted);
        if let Err(e) = store.save_occurrence_positions(&positions) {
            tracing::error!("failed to persist drifted positions after {context}: {e}");
        }
    }
    if !manifest.activated.is_empty()
        && let Err(e) = store.batch_increment_activation(&manifest.activated)
    {
        tracing::error!("failed to persist activations after {context}: {e}");
    }
}

/// Flush orphaned buffer entries from the store into the system as a conversation episode.
///
/// Called at the start of query paths to ensure buffered exchanges from previous
/// sessions are ingested before recall. Persists the system state after ingestion.
fn flush_orphaned_buffer(store: &BrainStore, system: &mut DAESystem, rng: &mut SmallRng) {
    let orphaned = store.store().buffer_count().unwrap_or(0);
    if orphaned > 0
        && let Ok(exchanges) = store.store().drain_buffer()
    {
        let combined: String = exchanges
            .iter()
            .map(|(u, a)| format!("{u}\n{a}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let episode = ingest_text(&combined, Some("conversation"), rng);
        system.add_episode(episode);
        if let Err(e) = store.save_episode(system.episodes.last().unwrap()) {
            tracing::error!("failed to persist flushed buffer episode: {e}");
        }
    }
}

impl AmServer {
    pub fn new(store: BrainStore) -> std::result::Result<Self, StoreError> {
        let system = store.load_system()?;
        let rng = SmallRng::from_os_rng();
        Ok(Self {
            state: Arc::new(Mutex::new(ServerState {
                system,
                store,
                rng,
                session_recalled: HashMap::new(),
                dedup_window: HashMap::new(),
            })),
            tool_router: Self::tool_router(),
        })
    }

    /// Explicitly flush WAL on the brain store.
    /// Belt-and-suspenders with Store::Drop, but ensures checkpoint runs
    /// even when the tokio runtime is shutting down.
    pub async fn checkpoint_wal(&self) {
        let state = self.state.lock().await;
        if let Err(e) = state.store.store().checkpoint_truncate() {
            tracing::warn!("WAL checkpoint failed: {e}");
        }
        tracing::info!("WAL checkpoint complete");
    }

    /// Compute a deterministic content hash for dedup.
    ///
    /// Uses `FxHasher` from `rustc-hash`, which produces stable output across
    /// Rust releases and process restarts (unlike `DefaultHasher`).
    fn content_hash(user: &str, assistant: &str) -> u64 {
        let mut hasher = FxHasher::default();
        user.hash(&mut hasher);
        b"\n".hash(&mut hasher);
        assistant.hash(&mut hasher);
        hasher.finish()
    }

    /// Remove expired entries from the dedup window.
    fn clean_dedup_window(window: &mut HashMap<u64, Instant>) {
        let cutoff = Instant::now() - std::time::Duration::from_secs(DEDUP_WINDOW_SECS);
        window.retain(|_, ts| *ts > cutoff);
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
    /// Optional maximum token budget for composed context. When provided,
    /// uses budget-aware composition that fits the best-scoring fragments
    /// within the token limit. Nancy's prompt compiler uses this to say
    /// "give me the best context that fits in N tokens".
    max_tokens: Option<usize>,
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
    /// Optional list of neighborhood UUIDs that this new memory supersedes.
    /// Superseded neighborhoods are permanently excluded from future recall.
    /// Use recalled_ids from am_query to identify which memories to replace.
    #[serde(default)]
    supersedes: Vec<String>,
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

#[derive(Debug, Deserialize, JsonSchema)]
struct FeedbackRequest {
    /// The original query text that produced the recall
    query: String,
    /// UUIDs of the neighborhoods that were recalled and shown to the user
    neighborhood_ids: Vec<String>,
    /// Feedback signal: "boost" if the recall was helpful, "demote" if not
    signal: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct BatchQueryItem {
    /// The query text
    query: String,
    /// Optional token budget for this query's context
    max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct McpBatchQueryRequest {
    /// List of queries to process in a single batch. IDF computation
    /// is amortized across all queries - much more efficient than
    /// querying one at a time when dispatching to multiple workers.
    queries: Vec<BatchQueryItem>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct QueryIndexRequest {
    /// The query text to search memory for
    text: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct RetrieveByIdsRequest {
    /// Neighborhood UUIDs to retrieve full content for (from am_query_index results)
    ids: Vec<String>,
}

#[tool_router]
impl AmServer {
    #[tool(
        description = "Query geometric memory. Call this at the START of every session with the user's first message to recall relevant context from past sessions. Returns conscious recall (insights you previously marked important), subconscious recall (relevant past conversations/documents), and novel connections (lateral associations). Use the returned context silently - weave it into your response naturally without announcing 'I remember...'."
    )]
    async fn am_query(
        &self,
        Parameters(req): Parameters<QueryRequest>,
    ) -> Result<CallToolResult, McpError> {
        check_input_size(&req.text, "text")?;
        let mut state = self.state.lock().await;
        // Snapshot session_recalled before destructuring (avoids borrow conflict)
        let session_recalled_snapshot = state.session_recalled.clone();
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        flush_orphaned_buffer(store, system, rng);

        let query_result = QueryEngine::process_query(system, &req.text);
        let surface = compute_surface(system, &query_result);

        let (mut result, new_ids) = if let Some(max_tokens) = req.max_tokens {
            // Budgeted query: Nancy's prompt compiler uses this
            let budget = BudgetConfig {
                max_tokens,
                min_conscious: 1,
                min_subconscious: 1,
                min_novel: 0,
            };
            let composed = compose_context_budgeted(
                system,
                &surface,
                &query_result,
                &query_result.interference,
                &budget,
                Some(&session_recalled_snapshot),
            );
            let ids: Vec<Uuid> = composed
                .included
                .iter()
                .map(|f| f.neighborhood_id)
                .collect();
            // Categorize IDs from IncludedFragment for feedback tracking
            let mut con_ids = Vec::new();
            let mut sub_ids = Vec::new();
            let mut nov_ids = Vec::new();
            for f in &composed.included {
                match f.category {
                    RecallCategory::Conscious => con_ids.push(f.neighborhood_id.to_string()),
                    RecallCategory::Subconscious => sub_ids.push(f.neighborhood_id.to_string()),
                    RecallCategory::Novel => nov_ids.push(f.neighborhood_id.to_string()),
                }
            }
            let json = serde_json::json!({
                "context": composed.context,
                "metrics": {
                    "conscious": composed.metrics.conscious,
                    "subconscious": composed.metrics.subconscious,
                    "novel": composed.metrics.novel,
                },
                "recalled_ids": {
                    "conscious": con_ids,
                    "subconscious": sub_ids,
                    "novel": nov_ids,
                },
                "token_estimate": {
                    "conscious": composed.token_estimate.conscious,
                    "subconscious": composed.token_estimate.subconscious,
                    "novel": composed.token_estimate.novel,
                    "total": composed.token_estimate.total,
                },
                "budget": {
                    "tokens_used": composed.tokens_used,
                    "tokens_budget": composed.tokens_budget,
                    "included_count": composed.included.len(),
                    "excluded_count": composed.excluded_count,
                },
                "stats": Self::stats_json(system),
            });
            (json, ids)
        } else {
            // Default: fixed-size composition
            let composed = compose_context(
                system,
                &surface,
                &query_result,
                &query_result.interference,
                Some(&session_recalled_snapshot),
            );
            let ids = composed.included_ids.clone();
            let recalled = &composed.recalled_ids;
            let json = serde_json::json!({
                "context": composed.context,
                "metrics": {
                    "conscious": composed.metrics.conscious,
                    "subconscious": composed.metrics.subconscious,
                    "novel": composed.metrics.novel,
                },
                "recalled_ids": {
                    "conscious": recalled.conscious.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                    "subconscious": recalled.subconscious.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                    "novel": recalled.novel.iter().map(|id| id.to_string()).collect::<Vec<_>>(),
                },
                "token_estimate": {
                    "conscious": composed.token_estimate.conscious,
                    "subconscious": composed.token_estimate.subconscious,
                    "novel": composed.token_estimate.novel,
                    "total": composed.token_estimate.total,
                },
                "stats": Self::stats_json(system),
            });
            (json, ids)
        };

        // Compose compact index summary (top 10 entries, most recent first)
        let index = compose_index(
            system,
            &surface,
            &query_result,
            &query_result.interference,
            Some(&session_recalled_snapshot),
        );
        let mut sorted_entries = index.entries;
        sorted_entries.sort_by(|a, b| b.epoch.cmp(&a.epoch));
        let index_entries: Vec<serde_json::Value> = sorted_entries
            .iter()
            .take(10)
            .map(|e| {
                serde_json::json!({
                    "id": e.neighborhood_id.to_string(),
                    "category": format!("{:?}", e.category),
                    "type": format!("{:?}", e.neighborhood_type),
                    "score": (e.score * 100.0).round() / 100.0,
                    "epoch": e.epoch,
                    "summary": e.summary,
                    "token_estimate": e.token_estimate,
                })
            })
            .collect();
        result["index"] = serde_json::json!(index_entries);

        persist_manifest(store, system, &query_result.manifest, "query");

        // Increment recall count for returned neighborhood IDs (diminishing returns)
        for id in new_ids {
            *state.session_recalled.entry(id).or_insert(0) += 1;
        }

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Two-phase retrieval: get a compact index of matching memories without full content. Returns neighborhood IDs, types, scores, summaries (first 100 chars), and token estimates. Use this first to see what's available (~50-100 tokens/entry vs ~500-1000 for full content), then call am_retrieve with selected IDs to fetch only the memories you need. Reduces context pollution for large manifolds."
    )]
    async fn am_query_index(
        &self,
        Parameters(req): Parameters<QueryIndexRequest>,
    ) -> Result<CallToolResult, McpError> {
        check_input_size(&req.text, "text")?;
        let mut state = self.state.lock().await;
        let session_recalled_snapshot = state.session_recalled.clone();
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        flush_orphaned_buffer(store, system, rng);

        let query_result = QueryEngine::process_query(system, &req.text);
        let surface = compute_surface(system, &query_result);

        let index = compose_index(
            system,
            &surface,
            &query_result,
            &query_result.interference,
            Some(&session_recalled_snapshot),
        );

        persist_manifest(store, system, &query_result.manifest, "query_index");

        let entries_json: Vec<serde_json::Value> = index
            .entries
            .iter()
            .map(|e| {
                serde_json::json!({
                    "id": e.neighborhood_id.to_string(),
                    "category": format!("{:?}", e.category),
                    "type": format!("{:?}", e.neighborhood_type),
                    "score": (e.score * 100.0).round() / 100.0,
                    "epoch": e.epoch,
                    "summary": e.summary,
                    "token_estimate": e.token_estimate,
                })
            })
            .collect();

        let result = serde_json::json!({
            "entries": entries_json,
            "total_candidates": index.stats_snapshot.total_candidates,
            "total_tokens_if_fetched": index.stats_snapshot.total_tokens_if_fetched,
            "stats": Self::stats_json(system),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Retrieve full content for specific neighborhood IDs. Phase 2 of two-phase retrieval: after reviewing am_query_index results, call this with the IDs of memories you want to see in full. Returns complete text for each requested neighborhood."
    )]
    async fn am_retrieve(
        &self,
        Parameters(req): Parameters<RetrieveByIdsRequest>,
    ) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let ServerState { system, .. } = &mut *state;

        let ids: Vec<Uuid> = req
            .ids
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect();

        let fragments = retrieve_by_ids(system, &ids);

        // Track these as recalled for diminishing returns
        for f in &fragments {
            *state.session_recalled.entry(f.neighborhood_id).or_insert(0) += 1;
        }

        let entries_json: Vec<serde_json::Value> = fragments
            .iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.neighborhood_id.to_string(),
                    "category": format!("{:?}", f.category),
                    "type": format!("{:?}", f.neighborhood_type),
                    "episode": f.episode_name,
                    "tokens": f.tokens,
                    "text": f.text,
                })
            })
            .collect();

        let result = serde_json::json!({
            "entries": entries_json,
            "count": fragments.len(),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Strengthen memory connections from your response text. Call this after giving a substantive response - it activates matching memories, drifts related concepts closer together on the manifold, and applies phase coupling. This is how the memory system consolidates over time. Not needed for every response - use after meaningful technical exchanges, not simple acknowledgements."
    )]
    async fn am_activate_response(
        &self,
        Parameters(req): Parameters<ActivateResponseRequest>,
    ) -> Result<CallToolResult, McpError> {
        check_input_size(&req.text, "text")?;
        let mut state = self.state.lock().await;
        let ServerState { system, store, .. } = &mut *state;

        let (activation, activated_ids) = QueryEngine::activate(system, &req.text);
        let all_refs: Vec<_> = activation
            .subconscious
            .iter()
            .chain(activation.conscious.iter())
            .copied()
            .collect();
        let mut drifted = QueryEngine::drift_and_consolidate(system, &all_refs);
        let (_, word_groups) = QueryEngine::compute_interference(
            system,
            &activation.subconscious,
            &activation.conscious,
        );
        drifted.extend(QueryEngine::apply_kuramoto_coupling(system, &word_groups));

        let manifest = QueryManifest {
            drifted,
            activated: activated_ids,
        };
        persist_manifest(store, system, &manifest, "activate_response");

        let result = serde_json::json!({
            "activated": all_refs.len(),
            "stats": Self::stats_json(system),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Mark an insight as conscious memory - something worth remembering across sessions and across projects. Use for: architecture decisions, user preferences, recurring patterns, hard-won debugging insights, project conventions. These surface as CONSCIOUS RECALL in future queries. Be selective - mark only genuinely reusable insights, not routine facts. Writes to brain-wide memory, queryable from any project. To replace outdated memories, pass their UUIDs (from am_query recalled_ids) in the supersedes array."
    )]
    async fn am_salient(
        &self,
        Parameters(req): Parameters<SalientRequest>,
    ) -> Result<CallToolResult, McpError> {
        check_input_size(&req.text, "text")?;
        let mut state = self.state.lock().await;
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        // Track how many neighborhoods exist before adding new ones
        let nbhd_before = system.conscious_episode.neighborhoods.len();

        let stored = extract_salient(system, &req.text, rng);
        let new_id = if stored == 0 {
            // No <salient> tags found - mark the whole text as salient
            // with automatic type detection from DECISION:/PREFERENCE: prefix
            let id = mark_salient_typed(system, &req.text, rng);
            Some(id)
        } else {
            None
        };

        // Persist only the newly added neighborhoods
        for nbhd in &system.conscious_episode.neighborhoods[nbhd_before..] {
            if let Err(e) = store.save_neighborhood(&system.conscious_episode, nbhd) {
                tracing::error!("failed to persist conscious neighborhood: {e}");
            }
        }
        let stored = if stored == 0 { 1u32 } else { stored };

        // Process supersedes: mark old neighborhoods as superseded by the new one
        let mut superseded_count = 0u32;
        if let Some(new_id) = new_id {
            for old_id_str in &req.supersedes {
                if let Ok(old_id) = Uuid::parse_str(old_id_str) {
                    // Update in-memory
                    if system.mark_superseded(old_id, new_id) {
                        // Persist targeted update to SQLite
                        if let Err(e) = store.store().mark_superseded(old_id, new_id) {
                            tracing::error!("failed to persist supersession: {e}");
                        }
                        superseded_count += 1;
                    } else {
                        tracing::warn!("supersedes target not found: {old_id_str}");
                    }
                } else {
                    tracing::warn!("invalid UUID in supersedes: {old_id_str}");
                }
            }
        } else if !req.supersedes.is_empty() {
            tracing::warn!(
                "supersedes ignored: multiple salient tags produce multiple neighborhoods, \
                 supersession only applies to single-neighborhood salient calls"
            );
        }

        let mut result = serde_json::json!({
            "stored": stored,
            "stats": Self::stats_json(system),
        });
        if superseded_count > 0 {
            result["superseded"] = serde_json::json!(superseded_count);
        }

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Buffer a conversation exchange. Call with each substantive user/assistant exchange pair. After 3 exchanges, automatically creates a memory episode on the geometric manifold. This is how conversations become searchable memories in future sessions. Skip trivial exchanges (greetings, confirmations) - buffer the ones with real content."
    )]
    async fn am_buffer(
        &self,
        Parameters(req): Parameters<BufferRequest>,
    ) -> Result<CallToolResult, McpError> {
        let total_len = req.user.len() + req.assistant.len();
        if total_len > MAX_TOOL_INPUT_BYTES {
            return Err(McpError::invalid_params(
                format!("combined input exceeds {} byte limit", MAX_TOOL_INPUT_BYTES),
                None,
            ));
        }
        let mut state = self.state.lock().await;
        let ServerState {
            system,
            store,
            rng,
            dedup_window,
            ..
        } = &mut *state;

        // Dedup check: hash the exchange and check against recent hashes
        let hash = Self::content_hash(&req.user, &req.assistant);
        Self::clean_dedup_window(dedup_window);

        if dedup_window.contains_key(&hash) {
            let result = serde_json::json!({
                "deduplicated": true,
                "buffer_size": store.store().buffer_count().unwrap_or(0),
            });
            return Ok(CallToolResult::success(vec![Content::text(
                serde_json::to_string_pretty(&result).unwrap_or_default(),
            )]));
        }
        dedup_window.insert(hash, Instant::now());

        let buffer_size = store
            .store()
            .append_buffer(&req.user, &req.assistant)
            .map_err(store_err_to_mcp)?;

        let mut episode_created: Option<String> = None;

        if buffer_size >= BUFFER_THRESHOLD {
            let exchanges = store.store().drain_buffer().map_err(store_err_to_mcp)?;

            let combined: String = exchanges
                .iter()
                .map(|(u, a)| format!("{u}\n{a}"))
                .collect::<Vec<_>>()
                .join("\n\n");

            let episode = ingest_text(&combined, Some("conversation"), rng);
            let name = episode.name.clone();
            system.add_episode(episode);

            if let Err(e) = store.save_episode(system.episodes.last().unwrap()) {
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
        check_input_size(&req.text, "text")?;
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

        if let Err(e) = store.save_episode(system.episodes.last().unwrap()) {
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
        description = "Get memory system statistics: total occurrences (N), episode count, and conscious memory count. Useful for understanding memory state. Not needed routinely - call when the user asks about memory or for diagnostics."
    )]
    async fn am_stats(&self) -> Result<CallToolResult, McpError> {
        let mut state = self.state.lock().await;
        let mut stats = Self::stats_json(&mut state.system);

        // Add store-level stats (DB size, activation distribution)
        let db_size = state.store.store().db_size();
        stats["db_size_bytes"] = serde_json::json!(db_size);
        if let Ok(activation) = state.store.store().activation_distribution() {
            stats["activation"] = serde_json::json!({
                "mean": activation.mean_activation,
                "max": activation.max_activation,
                "zero_count": activation.zero_activation,
            });
        }

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&stats).unwrap_or_default(),
        )]))
    }

    #[tool(description = "Export the full DAE system state as v0.7.2 compatible JSON.")]
    async fn am_export(&self) -> Result<CallToolResult, McpError> {
        let state = self.state.lock().await;
        let json = export_json(&state.system).map_err(serde_err_to_mcp)?;

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
        let json_str = serde_json::to_string(&req.state).map_err(serde_err_to_mcp)?;

        let imported = import_json(&json_str).map_err(serde_err_to_mcp)?;

        state.system = imported;

        if let Err(e) = state.store.save_system(&state.system) {
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

    #[tool(
        description = "Provide relevance feedback on recalled memories. Call this when you know whether a recalled memory was actually helpful (boost) or unhelpful (demote). Boost drifts the memory's occurrences closer to where they were needed on the manifold and increases activation. Demote decays activation, making the memory less prominent in future queries. This is how the memory system learns what works."
    )]
    async fn am_feedback(
        &self,
        Parameters(req): Parameters<FeedbackRequest>,
    ) -> Result<CallToolResult, McpError> {
        check_input_size(&req.query, "query")?;
        let mut state = self.state.lock().await;
        let ServerState { system, store, .. } = &mut *state;

        let signal = match req.signal.to_lowercase().as_str() {
            "boost" => FeedbackSignal::Boost,
            "demote" => FeedbackSignal::Demote,
            other => {
                return Err(McpError::invalid_params(
                    format!("signal must be 'boost' or 'demote', got '{other}'"),
                    None,
                ));
            }
        };

        let neighborhood_ids: Vec<Uuid> = req
            .neighborhood_ids
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect();

        if neighborhood_ids.is_empty() {
            return Err(McpError::invalid_params(
                "no valid neighborhood UUIDs provided".to_string(),
                None,
            ));
        }

        let feedback = apply_feedback(system, &req.query, &neighborhood_ids, signal);

        if let Err(e) = store.save_system(system) {
            tracing::error!("failed to persist after feedback: {e}");
        }

        let result = serde_json::json!({
            "boosted": feedback.boosted,
            "demoted": feedback.demoted,
            "centroid": feedback.centroid.map(|c| serde_json::json!({
                "w": c.w, "x": c.x, "y": c.y, "z": c.z
            })),
            "stats": Self::stats_json(system),
        });

        Ok(CallToolResult::success(vec![Content::text(
            serde_json::to_string_pretty(&result).unwrap_or_default(),
        )]))
    }

    #[tool(
        description = "Batch query: process multiple queries in a single pass with amortized IDF computation. Use when dispatching context to multiple workers simultaneously - activates the union of all query tokens once, drifts once, then partitions results per query. Much more efficient than N separate am_query calls. Each query can have its own token budget."
    )]
    async fn am_batch_query(
        &self,
        Parameters(req): Parameters<McpBatchQueryRequest>,
    ) -> Result<CallToolResult, McpError> {
        let total_len: usize = req.queries.iter().map(|q| q.query.len()).sum();
        if total_len > MAX_TOOL_INPUT_BYTES {
            return Err(McpError::invalid_params(
                format!(
                    "aggregate query text ({total_len} bytes) exceeds {} byte limit",
                    MAX_TOOL_INPUT_BYTES
                ),
                None,
            ));
        }
        let mut state = self.state.lock().await;
        let ServerState {
            system, store, rng, ..
        } = &mut *state;

        flush_orphaned_buffer(store, system, rng);

        let requests: Vec<BatchQueryRequest> = req
            .queries
            .iter()
            .map(|q| BatchQueryRequest {
                query: q.query.clone(),
                max_tokens: q.max_tokens,
            })
            .collect();

        let results = BatchQueryEngine::batch_query(system, &requests);

        if let Err(e) = store.save_system(system) {
            tracing::error!("failed to persist after batch query: {e}");
        }

        let results_json: Vec<serde_json::Value> = results
            .iter()
            .map(|r| {
                let mut con_ids = Vec::new();
                let mut sub_ids = Vec::new();
                let mut nov_ids = Vec::new();
                for f in &r.context.included {
                    match f.category {
                        RecallCategory::Conscious => con_ids.push(f.neighborhood_id.to_string()),
                        RecallCategory::Subconscious => sub_ids.push(f.neighborhood_id.to_string()),
                        RecallCategory::Novel => nov_ids.push(f.neighborhood_id.to_string()),
                    }
                }

                serde_json::json!({
                    "query": r.query,
                    "context": r.context.context,
                    "metrics": {
                        "conscious": r.context.metrics.conscious,
                        "subconscious": r.context.metrics.subconscious,
                        "novel": r.context.metrics.novel,
                    },
                    "recalled_ids": {
                        "conscious": con_ids,
                        "subconscious": sub_ids,
                        "novel": nov_ids,
                    },
                    "token_estimate": {
                        "conscious": r.context.token_estimate.conscious,
                        "subconscious": r.context.token_estimate.subconscious,
                        "novel": r.context.token_estimate.novel,
                        "total": r.context.token_estimate.total,
                    },
                    "budget": {
                        "tokens_used": r.context.tokens_used,
                        "tokens_budget": r.context.tokens_budget,
                        "included_count": r.context.included.len(),
                        "excluded_count": r.context.excluded_count,
                    },
                    "activated_count": r.activated_count,
                })
            })
            .collect();

        let result = serde_json::json!({
            "results": results_json,
            "batch_size": results_json.len(),
            "stats": Self::stats_json(system),
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
                    Use returned context silently - integrate naturally, never announce \"I remember...\".\n\
                 2. STRENGTHEN: After giving a meaningful technical response, call am_activate_response \
                    with your response text to consolidate related memories.\n\
                 3. MARK INSIGHTS: When you discover architecture decisions, user preferences, \
                    recurring patterns, or hard-won insights, call am_salient to store them as \
                    conscious memory. These persist globally across all projects.\n\
                 4. FEEDBACK: When you know a recalled memory was helpful (led to a correct solution) \
                    or unhelpful (was irrelevant or misleading), call am_feedback with the original \
                    query, the neighborhood IDs from the recall, and signal 'boost' or 'demote'. \
                    This reshapes the manifold - helpful memories drift toward where they were needed, \
                    unhelpful ones fade. The system literally learns from outcomes.\n\n\
                 PRINCIPLES:\n\
                 - CRITICAL: Always call am_query BEFORE exploring the filesystem. When asked contextual questions \
                   (\"where are we?\", \"what do you know about X?\"), query memory first. Only fall back to file \
                   exploration if memory returns nothing relevant. If the first query returns stale results, \
                   retry with more specific terms.\n\
                 - Memory should be invisible to the user. Don't mention the memory system unless asked.\n\
                 - Be selective with am_salient - mark genuinely reusable insights, not routine facts.\n\
                 - If am_query returns empty, that's fine - the project is new. Don't mention it.\n\
                 - Novel connections in query results are lateral associations - use them for creative leaps."
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
        let store = BrainStore::open_in_memory().unwrap();
        AmServer::new(store).unwrap()
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
                supersedes: vec![],
            }))
            .await
            .unwrap();

        // Query
        let result = server
            .am_query(Parameters(QueryRequest {
                text: "quantum particles".to_string(),
                max_tokens: None,
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

        // Verify token_estimate field exists with per-category breakdown
        let te = &json["token_estimate"];
        assert!(
            te.get("conscious").is_some(),
            "should have token_estimate.conscious"
        );
        assert!(
            te.get("subconscious").is_some(),
            "should have token_estimate.subconscious"
        );
        assert!(
            te.get("novel").is_some(),
            "should have token_estimate.novel"
        );
        assert!(
            te.get("total").is_some(),
            "should have token_estimate.total"
        );
        // Total should be sum of categories
        let total = te["total"].as_u64().unwrap();
        let sum = te["conscious"].as_u64().unwrap()
            + te["subconscious"].as_u64().unwrap()
            + te["novel"].as_u64().unwrap();
        assert_eq!(total, sum, "total should equal sum of categories");
        // With content ingested, total should be > 0
        assert!(total > 0, "token estimate should be positive with content");
    }

    #[tokio::test]
    async fn test_am_salient_stores_conscious() {
        let server = make_server();

        let result = server
            .am_salient(Parameters(SalientRequest {
                text: "important insight about neural networks".to_string(),
                supersedes: vec![],
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
                supersedes: vec![],
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
                supersedes: vec![],
            }))
            .await
            .unwrap();

        let stats3 = parse_result(&server.am_stats().await.unwrap());
        assert!(stats3["conscious"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn test_am_query_flushes_orphaned_buffer() {
        let server = make_server();

        // Buffer 2 exchanges (below threshold - simulates a session that ended early)
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
                max_tokens: None,
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        assert!(json.get("stats").is_some());
        // The orphaned buffer should have been flushed into an episode
        assert_eq!(json["stats"]["episodes"], 1);
    }

    #[tokio::test]
    async fn test_am_salient_supersedes_old_memory() {
        let server = make_server();

        // Create an initial conscious memory
        let result1 = server
            .am_salient(Parameters(SalientRequest {
                text: "deployment uses monolith architecture pattern".to_string(),
                supersedes: vec![],
            }))
            .await
            .unwrap();
        let json1 = parse_result(&result1);
        assert_eq!(json1["stored"], 1);

        // Query to get the recalled_ids of the old memory
        let query_result = server
            .am_query(Parameters(QueryRequest {
                text: "deployment architecture pattern".to_string(),
                max_tokens: None,
            }))
            .await
            .unwrap();
        let query_json = parse_result(&query_result);
        let old_ids: Vec<String> = query_json["recalled_ids"]["conscious"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert!(
            !old_ids.is_empty(),
            "should have conscious recall IDs from the first memory"
        );

        // Create a new memory that supersedes the old one
        let result2 = server
            .am_salient(Parameters(SalientRequest {
                text: "deployment uses microservices architecture pattern".to_string(),
                supersedes: old_ids.clone(),
            }))
            .await
            .unwrap();
        let json2 = parse_result(&result2);
        assert_eq!(json2["stored"], 1);
        assert_eq!(
            json2["superseded"],
            serde_json::json!(old_ids.len()),
            "should report superseded count"
        );

        // Query again - the old memory should not appear
        let query_result2 = server
            .am_query(Parameters(QueryRequest {
                text: "deployment architecture pattern".to_string(),
                max_tokens: None,
            }))
            .await
            .unwrap();
        let query_json2 = parse_result(&query_result2);
        let context = query_json2["context"].as_str().unwrap_or("");

        assert!(
            !context.contains("monolith"),
            "superseded memory should not appear in recall, got:\n{}",
            context,
        );
        assert!(
            context.contains("microservices"),
            "replacement memory should appear in recall, got:\n{}",
            context,
        );
    }

    #[tokio::test]
    async fn test_am_buffer_dedup_identical_content() {
        let server = make_server();

        // First buffer call - should succeed
        let result1 = server
            .am_buffer(Parameters(BufferRequest {
                user: "What is Rust?".to_string(),
                assistant: "Rust is a systems programming language.".to_string(),
            }))
            .await
            .unwrap();
        let json1 = parse_result(&result1);
        assert_eq!(json1["buffer_size"], 1);
        assert!(json1.get("deduplicated").is_none());

        // Second buffer call with identical content - should be deduplicated
        let result2 = server
            .am_buffer(Parameters(BufferRequest {
                user: "What is Rust?".to_string(),
                assistant: "Rust is a systems programming language.".to_string(),
            }))
            .await
            .unwrap();
        let json2 = parse_result(&result2);
        assert_eq!(json2["deduplicated"], true);
        assert_eq!(json2["buffer_size"], 1); // still 1, not 2

        // Third buffer call with different content - should succeed
        let result3 = server
            .am_buffer(Parameters(BufferRequest {
                user: "What is Go?".to_string(),
                assistant: "Go is a compiled programming language by Google.".to_string(),
            }))
            .await
            .unwrap();
        let json3 = parse_result(&result3);
        assert_eq!(json3["buffer_size"], 2);
        assert!(json3.get("deduplicated").is_none());
    }

    #[tokio::test]
    async fn test_am_buffer_dedup_different_content_creates_episodes() {
        let server = make_server();

        // Buffer 3 different exchanges - should create 1 episode
        for i in 0..3 {
            server
                .am_buffer(Parameters(BufferRequest {
                    user: format!("Unique question {i}"),
                    assistant: format!("Unique answer {i}"),
                }))
                .await
                .unwrap();
        }

        let stats = parse_result(&server.am_stats().await.unwrap());
        assert_eq!(
            stats["episodes"], 1,
            "3 unique exchanges should create 1 episode"
        );

        // Now try to buffer the same first exchange again - should be deduplicated
        let result = server
            .am_buffer(Parameters(BufferRequest {
                user: "Unique question 0".to_string(),
                assistant: "Unique answer 0".to_string(),
            }))
            .await
            .unwrap();
        let json = parse_result(&result);
        assert_eq!(json["deduplicated"], true);
    }

    #[tokio::test]
    async fn test_am_query_index_returns_compact_entries() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Quantum mechanics describes particle behavior at subatomic scales. Wave functions collapse on measurement.".to_string(),
                name: Some("science".to_string()),
            }))
            .await
            .unwrap();

        server
            .am_salient(Parameters(SalientRequest {
                text: "quantum computing is revolutionary technology".to_string(),
                supersedes: vec![],
            }))
            .await
            .unwrap();

        // Query the index
        let result = server
            .am_query_index(Parameters(QueryIndexRequest {
                text: "quantum particles".to_string(),
            }))
            .await
            .unwrap();

        let json = parse_result(&result);

        // Verify response structure
        assert!(json.get("entries").is_some(), "should have entries");
        assert!(
            json.get("total_candidates").is_some(),
            "should have total_candidates"
        );
        assert!(
            json.get("total_tokens_if_fetched").is_some(),
            "should have total_tokens_if_fetched"
        );
        assert!(json.get("stats").is_some(), "should have stats");

        let entries = json["entries"].as_array().unwrap();
        assert!(!entries.is_empty(), "should have matching entries");

        // Verify each entry has compact structure
        for entry in entries {
            assert!(entry.get("id").is_some(), "entry should have id");
            assert!(
                entry.get("category").is_some(),
                "entry should have category"
            );
            assert!(entry.get("type").is_some(), "entry should have type");
            assert!(entry.get("score").is_some(), "entry should have score");
            assert!(entry.get("epoch").is_some(), "entry should have epoch");
            assert!(entry.get("summary").is_some(), "entry should have summary");
            assert!(
                entry.get("token_estimate").is_some(),
                "entry should have token_estimate"
            );

            // Summary should be compact (<=103 chars: 100 + "...")
            let summary = entry["summary"].as_str().unwrap();
            assert!(
                summary.len() <= 103,
                "summary should be truncated, got {} chars",
                summary.len()
            );
        }
    }

    #[tokio::test]
    async fn test_am_retrieve_returns_full_content() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Rust borrow checker enforces ownership rules at compile time. Lifetimes prevent dangling references.".to_string(),
                name: Some("rust-guide".to_string()),
            }))
            .await
            .unwrap();

        // Get index to find IDs
        let index_result = server
            .am_query_index(Parameters(QueryIndexRequest {
                text: "rust borrow checker".to_string(),
            }))
            .await
            .unwrap();

        let index_json = parse_result(&index_result);
        let entries = index_json["entries"].as_array().unwrap();
        assert!(!entries.is_empty(), "should have index entries");

        // Pick the first ID
        let first_id = entries[0]["id"].as_str().unwrap().to_string();

        // Retrieve full content
        let retrieve_result = server
            .am_retrieve(Parameters(RetrieveByIdsRequest {
                ids: vec![first_id.clone()],
            }))
            .await
            .unwrap();

        let retrieve_json = parse_result(&retrieve_result);
        assert_eq!(retrieve_json["count"], 1);

        let retrieved = &retrieve_json["entries"].as_array().unwrap()[0];
        assert_eq!(retrieved["id"], first_id);
        assert!(retrieved.get("text").is_some(), "should have full text");
        assert!(
            !retrieved["text"].as_str().unwrap().is_empty(),
            "text should be non-empty"
        );
        assert!(
            retrieved.get("episode").is_some(),
            "should have episode name"
        );
    }

    #[tokio::test]
    async fn test_am_retrieve_handles_invalid_ids() {
        let server = make_server();

        let result = server
            .am_retrieve(Parameters(RetrieveByIdsRequest {
                ids: vec!["not-a-uuid".to_string()],
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        assert_eq!(json["count"], 0, "invalid UUIDs should return empty");
    }

    #[tokio::test]
    async fn test_am_query_includes_index() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Geometric memory uses hypersphere manifolds for associative recall. Neighborhoods cluster related concepts.".to_string(),
                name: Some("geo-memory".to_string()),
            }))
            .await
            .unwrap();

        // Add conscious content
        server
            .am_salient(Parameters(SalientRequest {
                text: "hypersphere manifolds enable geometric reasoning".to_string(),
                supersedes: vec![],
            }))
            .await
            .unwrap();

        // Query (default path, no budget)
        let result = server
            .am_query(Parameters(QueryRequest {
                text: "geometric manifold memory".to_string(),
                max_tokens: None,
            }))
            .await
            .unwrap();

        let json = parse_result(&result);

        // Verify index field exists
        assert!(json.get("index").is_some(), "should have index field");
        let index = json["index"].as_array().unwrap();

        // At most 10 entries
        assert!(index.len() <= 10, "index should have at most 10 entries");

        // Should have at least one entry (we ingested content + salient)
        assert!(!index.is_empty(), "index should have entries");

        // Verify each entry has the expected compact structure
        for entry in index {
            assert!(entry.get("id").is_some(), "entry should have id");
            assert!(
                entry.get("category").is_some(),
                "entry should have category"
            );
            assert!(entry.get("type").is_some(), "entry should have type");
            assert!(entry.get("score").is_some(), "entry should have score");
            assert!(entry.get("epoch").is_some(), "entry should have epoch");
            assert!(entry.get("summary").is_some(), "entry should have summary");
            assert!(
                entry.get("token_estimate").is_some(),
                "entry should have token_estimate"
            );
        }

        // Verify budgeted path also includes index
        let budgeted_result = server
            .am_query(Parameters(QueryRequest {
                text: "geometric manifold memory".to_string(),
                max_tokens: Some(500),
            }))
            .await
            .unwrap();

        let budgeted_json = parse_result(&budgeted_result);
        assert!(
            budgeted_json.get("index").is_some(),
            "budgeted query should also have index field"
        );
        let budgeted_index = budgeted_json["index"].as_array().unwrap();
        assert!(budgeted_index.len() <= 10);
    }

    #[test]
    fn test_tool_registration() {
        let server = make_server();
        let info = server.get_info();

        assert!(info.instructions.is_some());
        assert!(info.capabilities.tools.is_some());
    }

    #[tokio::test]
    async fn test_am_ingest_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server
            .am_ingest(Parameters(IngestRequest {
                text: oversized,
                name: None,
            }))
            .await;
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    #[tokio::test]
    async fn test_am_buffer_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server
            .am_buffer(Parameters(BufferRequest {
                user: oversized,
                assistant: String::new(),
            }))
            .await;
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    #[tokio::test]
    async fn test_am_salient_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server
            .am_salient(Parameters(SalientRequest {
                text: oversized,
                supersedes: vec![],
            }))
            .await;
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    /// Helper: ingest content and return neighborhood IDs from a query.
    async fn ingest_and_get_neighborhood_ids(server: &AmServer) -> Vec<String> {
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Quantum mechanics describes particle behavior at subatomic scales. Wave functions collapse upon measurement. Entanglement connects distant particles.".to_string(),
                name: Some("quantum".to_string()),
            }))
            .await
            .unwrap();

        let result = server
            .am_query(Parameters(QueryRequest {
                text: "quantum particles entanglement".to_string(),
                max_tokens: None,
            }))
            .await
            .unwrap();
        let json = parse_result(&result);
        let recalled = &json["recalled_ids"];
        let mut ids = Vec::new();
        for cat in &["conscious", "subconscious", "novel"] {
            if let Some(arr) = recalled[cat].as_array() {
                for id in arr {
                    if let Some(s) = id.as_str() {
                        ids.push(s.to_string());
                    }
                }
            }
        }
        ids
    }

    #[tokio::test]
    async fn test_am_feedback_boost() {
        let server = make_server();
        let ids = ingest_and_get_neighborhood_ids(&server).await;
        assert!(
            !ids.is_empty(),
            "query should recall at least one neighborhood"
        );

        let result = server
            .am_feedback(Parameters(FeedbackRequest {
                query: "quantum particles".to_string(),
                neighborhood_ids: ids,
                signal: "boost".to_string(),
            }))
            .await
            .unwrap();
        let json = parse_result(&result);
        assert!(
            json["boosted"].as_u64().unwrap() > 0,
            "boost should affect at least one neighborhood"
        );
    }

    #[tokio::test]
    async fn test_am_feedback_demote() {
        let server = make_server();
        let ids = ingest_and_get_neighborhood_ids(&server).await;
        assert!(
            !ids.is_empty(),
            "query should recall at least one neighborhood"
        );

        let result = server
            .am_feedback(Parameters(FeedbackRequest {
                query: "quantum particles".to_string(),
                neighborhood_ids: ids,
                signal: "demote".to_string(),
            }))
            .await
            .unwrap();
        let json = parse_result(&result);
        assert!(
            json["demoted"].as_u64().unwrap() > 0,
            "demote should affect at least one neighborhood"
        );
    }

    #[tokio::test]
    async fn test_am_feedback_unknown_signal() {
        let server = make_server();
        let result = server
            .am_feedback(Parameters(FeedbackRequest {
                query: "test".to_string(),
                neighborhood_ids: vec!["00000000-0000-0000-0000-000000000001".to_string()],
                signal: "invalid_signal".to_string(),
            }))
            .await;
        assert!(result.is_err(), "unknown signal should return error");
    }

    #[tokio::test]
    async fn test_am_feedback_empty_ids() {
        let server = make_server();
        let result = server
            .am_feedback(Parameters(FeedbackRequest {
                query: "test".to_string(),
                neighborhood_ids: vec![],
                signal: "boost".to_string(),
            }))
            .await;
        assert!(
            result.is_err(),
            "empty neighborhood_ids should return error"
        );
    }

    #[tokio::test]
    async fn test_am_batch_query_basic() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Rust is a systems programming language focused on safety and performance. Memory safety without garbage collection.".to_string(),
                name: Some("rust-lang".to_string()),
            }))
            .await
            .unwrap();

        let result = server
            .am_batch_query(Parameters(McpBatchQueryRequest {
                queries: vec![
                    BatchQueryItem {
                        query: "rust safety".to_string(),
                        max_tokens: None,
                    },
                    BatchQueryItem {
                        query: "memory management".to_string(),
                        max_tokens: None,
                    },
                    BatchQueryItem {
                        query: "performance optimization".to_string(),
                        max_tokens: None,
                    },
                ],
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        let results = json["results"].as_array().expect("results should be array");
        assert_eq!(results.len(), 3, "should have 3 results for 3 queries");

        for (i, r) in results.iter().enumerate() {
            assert!(r.get("query").is_some(), "result {i} should have query");
            assert!(r.get("context").is_some(), "result {i} should have context");
            assert!(r.get("metrics").is_some(), "result {i} should have metrics");
            assert!(
                r.get("recalled_ids").is_some(),
                "result {i} should have recalled_ids"
            );
            assert!(
                r.get("token_estimate").is_some(),
                "result {i} should have token_estimate"
            );
        }
    }

    #[tokio::test]
    async fn test_am_batch_query_empty_requests() {
        let server = make_server();

        let result = server
            .am_batch_query(Parameters(McpBatchQueryRequest { queries: vec![] }))
            .await
            .unwrap();

        let json = parse_result(&result);
        let results = json["results"].as_array().expect("results should be array");
        assert!(
            results.is_empty(),
            "empty queries should produce empty results"
        );
    }

    #[tokio::test]
    async fn test_am_batch_query_per_budget() {
        let server = make_server();

        // Ingest enough content to test budget limits
        server
            .am_ingest(Parameters(IngestRequest {
                text: "Quantum mechanics describes the behavior of particles at the smallest scales. Superposition allows particles to exist in multiple states simultaneously. Entanglement connects particles across vast distances instantaneously.".to_string(),
                name: Some("quantum".to_string()),
            }))
            .await
            .unwrap();

        let result = server
            .am_batch_query(Parameters(McpBatchQueryRequest {
                queries: vec![
                    BatchQueryItem {
                        query: "quantum entanglement".to_string(),
                        max_tokens: Some(50),
                    },
                    BatchQueryItem {
                        query: "quantum superposition".to_string(),
                        max_tokens: Some(5000),
                    },
                ],
            }))
            .await
            .unwrap();

        let json = parse_result(&result);
        let results = json["results"].as_array().expect("results should be array");
        assert_eq!(results.len(), 2);

        // Both should have budget fields reflecting their token limits
        let budget_small = &results[0]["budget"];
        let budget_large = &results[1]["budget"];
        assert_eq!(budget_small["tokens_budget"], 50);
        assert_eq!(budget_large["tokens_budget"], 5000);
    }

    #[tokio::test]
    async fn test_am_query_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server
            .am_query(Parameters(QueryRequest {
                text: oversized,
                max_tokens: None,
            }))
            .await;
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    #[tokio::test]
    async fn test_am_activate_response_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server
            .am_activate_response(Parameters(ActivateResponseRequest { text: oversized }))
            .await;
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    #[tokio::test]
    async fn test_am_feedback_rejects_oversized_query() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server
            .am_feedback(Parameters(FeedbackRequest {
                query: oversized,
                neighborhood_ids: vec![],
                signal: "boost".to_string(),
            }))
            .await;
        assert!(result.is_err(), "should reject query exceeding size limit");
    }

    #[tokio::test]
    async fn test_am_batch_query_rejects_oversized_aggregate() {
        let server = make_server();
        // Each query is half the limit; together they exceed it
        let half_plus = "x".repeat(MAX_TOOL_INPUT_BYTES / 2 + 1);
        let result = server
            .am_batch_query(Parameters(McpBatchQueryRequest {
                queries: vec![
                    BatchQueryItem {
                        query: half_plus.clone(),
                        max_tokens: None,
                    },
                    BatchQueryItem {
                        query: half_plus,
                        max_tokens: None,
                    },
                ],
            }))
            .await;
        assert!(
            result.is_err(),
            "should reject aggregate payload exceeding size limit"
        );
    }

    #[tokio::test]
    async fn test_am_query_index_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server
            .am_query_index(Parameters(QueryIndexRequest { text: oversized }))
            .await;
        assert!(result.is_err(), "should reject input exceeding size limit");
    }
}
