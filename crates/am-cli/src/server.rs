use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::Instant;

use rustc_hash::FxHasher;
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;

use am_core::{
    AmStore, BatchQueryEngine, BatchQueryRequest, BudgetConfig, DAESystem, DaemonPhasor,
    FeedbackSignal, Quaternion, QueryEngine, QueryManifest, RecallCategory, apply_feedback,
    compose_context, compose_context_budgeted, compose_index, compute_surface, export_json,
    extract_salient, import_json, ingest_text, mark_salient_typed, retrieve_by_ids,
};
use rand::SeedableRng;
use rand::rngs::SmallRng;

use crate::jsonrpc::tool_result_text;

const BUFFER_THRESHOLD: usize = 3;
const DEDUP_WINDOW_SECS: u64 = 60;
/// Maximum input size for text-accepting MCP tools (1 MB).
const MAX_TOOL_INPUT_BYTES: usize = 1_048_576;

/// Reject input that exceeds the per-tool byte limit.
fn check_input_size(value: &str, field: &str) -> Result<(), String> {
    if value.len() > MAX_TOOL_INPUT_BYTES {
        return Err(format!(
            "{field} exceeds {} byte limit",
            MAX_TOOL_INPUT_BYTES
        ));
    }
    Ok(())
}

/// Convert a store error into a tool error string.
fn store_err_to_string(e: impl std::fmt::Display) -> String {
    format!("[store] {e}")
}

pub struct AmServer<S: AmStore> {
    state: Mutex<ServerState<S>>,
}

/// All mutable server state behind a single `std::sync::Mutex`.
///
/// # Concurrency model
///
/// Every MCP tool handler acquires `state.lock()` for its full duration.
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
struct ServerState<S: AmStore> {
    system: DAESystem,
    store: S,
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

    let id_set: rustc_hash::FxHashSet<Uuid> = ids.iter().copied().collect();
    let mut result = Vec::with_capacity(ids.len());

    let iter = system
        .episodes
        .iter()
        .chain(std::iter::once(&system.conscious_episode));

    for ep in iter {
        for nbhd in &ep.neighborhoods {
            for occ in &nbhd.occurrences {
                if id_set.contains(&occ.id) {
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
    store: &impl AmStore,
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
    if !manifest.demoted_activations.is_empty()
        && let Err(e) = store.batch_set_activation_counts(&manifest.demoted_activations)
    {
        tracing::error!("failed to persist demoted activations after {context}: {e}");
    }
}

/// Flush orphaned buffer entries from the store into the system as a conversation episode.
///
/// Called at the start of query paths to ensure buffered exchanges from previous
/// sessions are ingested before recall. Persists the system state after ingestion.
fn flush_orphaned_buffer(store: &impl AmStore, system: &mut DAESystem, rng: &mut SmallRng) {
    let orphaned = store.buffer_count().unwrap_or(0);
    if orphaned > 0
        && let Ok(exchanges) = store.drain_buffer()
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

// --- Tool parameter types ---

#[derive(Debug, Deserialize)]
struct QueryRequest {
    /// The text to query the memory system with
    text: String,
    /// Optional maximum token budget for composed context.
    max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct ActivateResponseRequest {
    /// Response text to strengthen connections for
    text: String,
}

#[derive(Debug, Deserialize)]
struct SalientRequest {
    /// Text to mark as conscious memory (may contain salient tags)
    text: String,
    /// Optional list of neighborhood UUIDs that this new memory supersedes.
    #[serde(default)]
    supersedes: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct BufferRequest {
    /// User's message text
    user: String,
    /// Assistant's response text
    assistant: String,
}

#[derive(Debug, Deserialize)]
struct IngestRequest {
    /// Document text to ingest
    text: String,
    /// Optional name for the episode
    name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ImportRequest {
    /// Full state JSON to import
    state: serde_json::Value,
}

#[derive(Debug, Deserialize)]
struct FeedbackRequest {
    /// The original query text that produced the recall
    query: String,
    /// UUIDs of the neighborhoods that were recalled and shown to the user
    neighborhood_ids: Vec<String>,
    /// Feedback signal: "boost" if the recall was helpful, "demote" if not
    signal: String,
}

#[derive(Debug, Deserialize)]
struct BatchQueryItem {
    /// The query text
    query: String,
    /// Optional token budget for this query's context
    max_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct McpBatchQueryRequest {
    /// List of queries to process in a single batch.
    queries: Vec<BatchQueryItem>,
}

#[derive(Debug, Deserialize)]
struct QueryIndexRequest {
    /// The query text to search memory for
    text: String,
}

#[derive(Debug, Deserialize)]
struct RetrieveByIdsRequest {
    /// Neighborhood UUIDs to retrieve full content for
    ids: Vec<String>,
}

impl<S: AmStore> AmServer<S> {
    pub fn new(store: S) -> std::result::Result<Self, S::Error> {
        let system = store.load_system()?;
        let rng = SmallRng::from_os_rng();
        Ok(Self {
            state: Mutex::new(ServerState {
                system,
                store,
                rng,
                session_recalled: HashMap::new(),
                dedup_window: HashMap::new(),
            }),
        })
    }

    /// Explicitly flush WAL on the brain store.
    /// Belt-and-suspenders with Store::Drop, but ensures checkpoint runs
    /// before process exit.
    pub fn checkpoint_wal(&self) {
        let state = self.state.lock().expect("poisoned mutex");
        if let Err(e) = state.store.checkpoint_truncate() {
            tracing::warn!("WAL checkpoint failed: {e}");
        }
        tracing::info!("WAL checkpoint complete");
    }

    /// Dispatch a tool call by name. This is the single entry point wired
    /// into `jsonrpc::run_stdio_loop`.
    pub fn dispatch_tool(&self, name: &str, args: &Value) -> Result<Value, String> {
        match name {
            "am_query" => self.am_query(args),
            "am_query_index" => self.am_query_index(args),
            "am_retrieve" => self.am_retrieve(args),
            "am_activate_response" => self.am_activate_response(args),
            "am_salient" => self.am_salient(args),
            "am_buffer" => self.am_buffer(args),
            "am_ingest" => self.am_ingest(args),
            "am_stats" => self.am_stats(),
            "am_export" => self.am_export(),
            "am_import" => self.am_import(args),
            "am_feedback" => self.am_feedback(args),
            "am_batch_query" => self.am_batch_query(args),
            _ => Err(format!("unknown tool: {name}")),
        }
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

    fn stats_json(system: &DAESystem) -> serde_json::Value {
        let n = system.n();
        let episodes = system.episodes.len();
        let conscious = system.conscious_episode.neighborhoods.len();
        serde_json::json!({
            "n": n,
            "episodes": episodes,
            "conscious": conscious,
        })
    }

    // ── Tool handlers ────────────────────────────────────────────────
    //
    // Each handler takes &Value (the MCP arguments object) and returns
    // Result<Value, String>. On success, returns a tool_result_text value.
    // On error, returns Err(String) which the transport wraps as isError.

    fn am_query(&self, args: &Value) -> Result<Value, String> {
        let req: QueryRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState {
            system,
            store,
            rng,
            session_recalled,
            ..
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
                Some(session_recalled),
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
                Some(session_recalled),
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
            Some(session_recalled),
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
            *session_recalled.entry(id).or_insert(0) += 1;
        }

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    fn am_query_index(&self, args: &Value) -> Result<Value, String> {
        let req: QueryIndexRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState {
            system,
            store,
            rng,
            session_recalled,
            ..
        } = &mut *state;

        flush_orphaned_buffer(store, system, rng);

        let query_result = QueryEngine::process_query(system, &req.text);
        let surface = compute_surface(system, &query_result);

        let index = compose_index(
            system,
            &surface,
            &query_result,
            &query_result.interference,
            Some(session_recalled),
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

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    fn am_retrieve(&self, args: &Value) -> Result<Value, String> {
        let req: RetrieveByIdsRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;

        let mut state = self.state.lock().expect("poisoned mutex");
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

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    fn am_activate_response(&self, args: &Value) -> Result<Value, String> {
        let req: ActivateResponseRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
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
            demoted_activations: Vec::new(),
        };
        persist_manifest(store, system, &manifest, "activate_response");

        let result = serde_json::json!({
            "activated": all_refs.len(),
            "stats": Self::stats_json(system),
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    fn am_salient(&self, args: &Value) -> Result<Value, String> {
        let req: SalientRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
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
                        if let Err(e) = store.mark_superseded(old_id, new_id) {
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

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    fn am_buffer(&self, args: &Value) -> Result<Value, String> {
        let req: BufferRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;

        let total_len = req.user.len() + req.assistant.len();
        if total_len > MAX_TOOL_INPUT_BYTES {
            return Err(format!(
                "combined input exceeds {} byte limit",
                MAX_TOOL_INPUT_BYTES
            ));
        }

        let mut state = self.state.lock().expect("poisoned mutex");
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
                "buffer_size": store.buffer_count().unwrap_or(0),
            });
            return Ok(tool_result_text(
                &serde_json::to_string_pretty(&result).unwrap_or_default(),
            ));
        }
        dedup_window.insert(hash, Instant::now());

        let buffer_size = store
            .append_buffer(&req.user, &req.assistant)
            .map_err(store_err_to_string)?;

        let mut episode_created: Option<String> = None;

        if buffer_size >= BUFFER_THRESHOLD {
            let exchanges = store.drain_buffer().map_err(store_err_to_string)?;

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

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    fn am_ingest(&self, args: &Value) -> Result<Value, String> {
        let req: IngestRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.text, "text")?;

        let mut state = self.state.lock().expect("poisoned mutex");
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

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    fn am_stats(&self) -> Result<Value, String> {
        let state = self.state.lock().expect("poisoned mutex");
        let mut stats = Self::stats_json(&state.system);

        // Add store-level stats (DB size, activation distribution)
        let db_size = state.store.db_size();
        stats["db_size_bytes"] = serde_json::json!(db_size);
        if let Ok(activation) = state.store.activation_distribution() {
            stats["activation"] = serde_json::json!({
                "mean": activation.mean_activation,
                "max": activation.max_activation,
                "zero_count": activation.zero_activation,
            });
        }

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&stats).unwrap_or_default(),
        ))
    }

    fn am_export(&self) -> Result<Value, String> {
        let state = self.state.lock().expect("poisoned mutex");
        let json = export_json(&state.system).map_err(|e| format!("[serde] {e}"))?;
        Ok(tool_result_text(&json))
    }

    fn am_import(&self, args: &Value) -> Result<Value, String> {
        let req: ImportRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let json_str = serde_json::to_string(&req.state).map_err(|e| format!("[serde] {e}"))?;

        let imported = import_json(&json_str).map_err(|e| format!("[serde] {e}"))?;

        state.system = imported;

        // Intentional save_system: import replaces the entire DAE state,
        // so a full rewrite is the only correct persistence strategy.
        if let Err(e) = state.store.save_system(&state.system) {
            tracing::error!("failed to persist after import: {e}");
        }

        let result = serde_json::json!({
            "imported": true,
            "stats": Self::stats_json(&state.system),
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    fn am_feedback(&self, args: &Value) -> Result<Value, String> {
        let req: FeedbackRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;
        check_input_size(&req.query, "query")?;

        let mut state = self.state.lock().expect("poisoned mutex");
        let ServerState { system, store, .. } = &mut *state;

        let signal = match req.signal.to_lowercase().as_str() {
            "boost" => FeedbackSignal::Boost,
            "demote" => FeedbackSignal::Demote,
            other => {
                return Err(format!("signal must be 'boost' or 'demote', got '{other}'"));
            }
        };

        let neighborhood_ids: Vec<Uuid> = req
            .neighborhood_ids
            .iter()
            .filter_map(|s| Uuid::parse_str(s).ok())
            .collect();

        if neighborhood_ids.is_empty() {
            return Err("no valid neighborhood UUIDs provided".to_owned());
        }

        let feedback = apply_feedback(system, &req.query, &neighborhood_ids, signal);

        persist_manifest(store, system, &feedback.manifest, "feedback");

        let result = serde_json::json!({
            "boosted": feedback.boosted,
            "demoted": feedback.demoted,
            "centroid": feedback.centroid.map(|c| serde_json::json!({
                "w": c.w, "x": c.x, "y": c.y, "z": c.z
            })),
            "stats": Self::stats_json(system),
        });

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }

    fn am_batch_query(&self, args: &Value) -> Result<Value, String> {
        let req: McpBatchQueryRequest =
            serde_json::from_value(args.clone()).map_err(|e| format!("invalid params: {e}"))?;

        let total_len: usize = req.queries.iter().map(|q| q.query.len()).sum();
        if total_len > MAX_TOOL_INPUT_BYTES {
            return Err(format!(
                "aggregate query text ({total_len} bytes) exceeds {} byte limit",
                MAX_TOOL_INPUT_BYTES
            ));
        }

        let mut state = self.state.lock().expect("poisoned mutex");
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

        let batch_output = BatchQueryEngine::batch_query(system, &requests);

        persist_manifest(store, system, &batch_output.manifest, "batch_query");

        let results_json: Vec<serde_json::Value> = batch_output
            .results
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

        Ok(tool_result_text(
            &serde_json::to_string_pretty(&result).unwrap_or_default(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_store::BrainStore;

    fn make_server() -> AmServer<BrainStore> {
        let store = BrainStore::open_in_memory().unwrap();
        AmServer::new(store).unwrap()
    }

    fn parse_tool_result(result: &Value) -> serde_json::Value {
        let text = result["content"][0]["text"]
            .as_str()
            .expect("should have text content");
        serde_json::from_str(text).expect("handler should return valid JSON in text content")
    }

    #[test]
    fn test_am_stats_empty() {
        let server = make_server();
        let result = server.am_stats().unwrap();
        let json = parse_tool_result(&result);

        assert_eq!(json["n"], 0);
        assert_eq!(json["episodes"], 0);
        assert_eq!(json["conscious"], 0);
    }

    #[test]
    fn test_am_ingest() {
        let server = make_server();

        let result = server
            .am_ingest(&serde_json::json!({
                "text": "The quick brown fox jumps over the lazy dog. Sentence two here. And a third sentence for good measure.",
                "name": "test-doc"
            }))
            .unwrap();

        let json = parse_tool_result(&result);
        assert_eq!(json["episode"], "test-doc");
        assert!(json["neighborhoods"].as_u64().unwrap() >= 1);
        assert!(json["occurrences"].as_u64().unwrap() > 0);

        // Stats should reflect the ingestion
        let stats = parse_tool_result(&server.am_stats().unwrap());
        assert!(stats["n"].as_u64().unwrap() > 0);
        assert_eq!(stats["episodes"], 1);
    }

    #[test]
    fn test_am_query_response_structure() {
        let server = make_server();

        // Ingest content first
        server
            .am_ingest(&serde_json::json!({
                "text": "Quantum mechanics describes particle behavior at subatomic scales. Wave functions collapse on measurement.",
                "name": "science"
            }))
            .unwrap();

        // Add conscious content
        server
            .am_salient(&serde_json::json!({
                "text": "quantum computing is revolutionary"
            }))
            .unwrap();

        // Query
        let result = server
            .am_query(&serde_json::json!({
                "text": "quantum particles"
            }))
            .unwrap();

        let json = parse_tool_result(&result);

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

    #[test]
    fn test_am_salient_stores_conscious() {
        let server = make_server();

        let result = server
            .am_salient(&serde_json::json!({
                "text": "important insight about neural networks"
            }))
            .unwrap();

        let json = parse_tool_result(&result);
        assert_eq!(json["stored"], 1);

        // Stats should show conscious memory
        let stats = parse_tool_result(&server.am_stats().unwrap());
        assert!(
            stats["conscious"].as_u64().unwrap() >= 1,
            "should have at least one conscious neighborhood"
        );
    }

    #[test]
    fn test_am_salient_with_tags() {
        let server = make_server();

        let result = server
            .am_salient(&serde_json::json!({
                "text": "Normal text <salient>first insight</salient> middle <salient>second insight</salient> end"
            }))
            .unwrap();

        let json = parse_tool_result(&result);
        assert_eq!(json["stored"], 2);

        let stats = parse_tool_result(&server.am_stats().unwrap());
        assert!(stats["conscious"].as_u64().unwrap() >= 2);
    }

    #[test]
    fn test_am_activate_response() {
        let server = make_server();

        // Ingest content first
        server
            .am_ingest(&serde_json::json!({
                "text": "Machine learning enables pattern recognition in data. Neural networks learn representations.",
                "name": "ml-doc"
            }))
            .unwrap();

        let result = server
            .am_activate_response(&serde_json::json!({
                "text": "machine learning neural networks"
            }))
            .unwrap();

        let json = parse_tool_result(&result);
        assert!(json["activated"].as_u64().unwrap() > 0);
        assert!(json.get("stats").is_some());
    }

    #[test]
    fn test_am_buffer() {
        let server = make_server();

        // Buffer exchanges below threshold (threshold is 3)
        for i in 0..2 {
            let result = server
                .am_buffer(&serde_json::json!({
                    "user": format!("User message {i}"),
                    "assistant": format!("Assistant response {i}")
                }))
                .unwrap();

            let json = parse_tool_result(&result);
            assert_eq!(json["buffer_size"], i + 1);
            assert!(json["episode_created"].is_null());
        }

        // 3rd exchange should trigger episode creation
        let result = server
            .am_buffer(&serde_json::json!({
                "user": "User message 2",
                "assistant": "Assistant response 2"
            }))
            .unwrap();

        let json = parse_tool_result(&result);
        assert_eq!(json["buffer_size"], 3);
        assert!(
            json["episode_created"].is_string(),
            "should create episode after 3 exchanges"
        );

        let stats = parse_tool_result(&server.am_stats().unwrap());
        assert_eq!(stats["episodes"], 1);
    }

    #[test]
    fn test_am_export_import_roundtrip() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(&serde_json::json!({
                "text": "Roundtrip test content. Multiple sentences for neighborhoods. And one more sentence.",
                "name": "roundtrip"
            }))
            .unwrap();

        // Get stats before export
        let stats_before = parse_tool_result(&server.am_stats().unwrap());

        // Export
        let export_result = server.am_export().unwrap();
        let exported_json = export_result["content"][0]["text"]
            .as_str()
            .expect("export should return text");
        assert!(!exported_json.is_empty());

        // Create a fresh server and import
        let server2 = make_server();
        let state_value: serde_json::Value = serde_json::from_str(exported_json).unwrap();

        let import_result = server2
            .am_import(&serde_json::json!({ "state": state_value }))
            .unwrap();

        let import_json = parse_tool_result(&import_result);
        assert_eq!(import_json["imported"], true);

        // Verify stats match
        let stats_after = parse_tool_result(&server2.am_stats().unwrap());
        assert_eq!(stats_before["n"], stats_after["n"]);
        assert_eq!(stats_before["episodes"], stats_after["episodes"]);
    }

    #[test]
    fn test_am_stats_after_operations() {
        let server = make_server();

        // Ingest
        server
            .am_ingest(&serde_json::json!({
                "text": "First document about testing. With multiple sentences here. And a final line.",
                "name": "doc1"
            }))
            .unwrap();

        let stats1 = parse_tool_result(&server.am_stats().unwrap());
        let n1 = stats1["n"].as_u64().unwrap();
        assert!(n1 > 0);
        assert_eq!(stats1["episodes"], 1);

        // Ingest second document
        server
            .am_ingest(&serde_json::json!({
                "text": "Second document about verification. Has different content entirely. Nothing overlaps.",
                "name": "doc2"
            }))
            .unwrap();

        let stats2 = parse_tool_result(&server.am_stats().unwrap());
        assert!(stats2["n"].as_u64().unwrap() > n1);
        assert_eq!(stats2["episodes"], 2);

        // Mark salient
        server
            .am_salient(&serde_json::json!({
                "text": "key insight"
            }))
            .unwrap();

        let stats3 = parse_tool_result(&server.am_stats().unwrap());
        assert!(stats3["conscious"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn test_am_query_flushes_orphaned_buffer() {
        let server = make_server();

        // Buffer 2 exchanges (below threshold - simulates a session that ended early)
        for i in 0..2 {
            server
                .am_buffer(&serde_json::json!({
                    "user": format!("Orphaned user message {i}"),
                    "assistant": format!("Orphaned assistant response {i}")
                }))
                .unwrap();
        }

        // No episode yet
        let stats = parse_tool_result(&server.am_stats().unwrap());
        assert_eq!(stats["episodes"], 0);

        // Calling am_query (simulating next session start) should flush the orphaned buffer
        let result = server
            .am_query(&serde_json::json!({
                "text": "orphaned message"
            }))
            .unwrap();

        let json = parse_tool_result(&result);
        assert!(json.get("stats").is_some());
        // The orphaned buffer should have been flushed into an episode
        assert_eq!(json["stats"]["episodes"], 1);
    }

    #[test]
    fn test_am_salient_supersedes_old_memory() {
        let server = make_server();

        // Create an initial conscious memory
        let result1 = server
            .am_salient(&serde_json::json!({
                "text": "deployment uses monolith architecture pattern"
            }))
            .unwrap();
        let json1 = parse_tool_result(&result1);
        assert_eq!(json1["stored"], 1);

        // Query to get the recalled_ids of the old memory
        let query_result = server
            .am_query(&serde_json::json!({
                "text": "deployment architecture pattern"
            }))
            .unwrap();
        let query_json = parse_tool_result(&query_result);
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
            .am_salient(&serde_json::json!({
                "text": "deployment uses microservices architecture pattern",
                "supersedes": old_ids.clone()
            }))
            .unwrap();
        let json2 = parse_tool_result(&result2);
        assert_eq!(json2["stored"], 1);
        assert_eq!(
            json2["superseded"],
            serde_json::json!(old_ids.len()),
            "should report superseded count"
        );

        // Query again - the old memory should not appear
        let query_result2 = server
            .am_query(&serde_json::json!({
                "text": "deployment architecture pattern"
            }))
            .unwrap();
        let query_json2 = parse_tool_result(&query_result2);
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

    #[test]
    fn test_am_buffer_dedup_identical_content() {
        let server = make_server();

        // First buffer call - should succeed
        let result1 = server
            .am_buffer(&serde_json::json!({
                "user": "What is Rust?",
                "assistant": "Rust is a systems programming language."
            }))
            .unwrap();
        let json1 = parse_tool_result(&result1);
        assert_eq!(json1["buffer_size"], 1);
        assert!(json1.get("deduplicated").is_none());

        // Second buffer call with identical content - should be deduplicated
        let result2 = server
            .am_buffer(&serde_json::json!({
                "user": "What is Rust?",
                "assistant": "Rust is a systems programming language."
            }))
            .unwrap();
        let json2 = parse_tool_result(&result2);
        assert_eq!(json2["deduplicated"], true);
        assert_eq!(json2["buffer_size"], 1); // still 1, not 2

        // Third buffer call with different content - should succeed
        let result3 = server
            .am_buffer(&serde_json::json!({
                "user": "What is Go?",
                "assistant": "Go is a compiled programming language by Google."
            }))
            .unwrap();
        let json3 = parse_tool_result(&result3);
        assert_eq!(json3["buffer_size"], 2);
        assert!(json3.get("deduplicated").is_none());
    }

    #[test]
    fn test_am_buffer_dedup_different_content_creates_episodes() {
        let server = make_server();

        // Buffer 3 different exchanges - should create 1 episode
        for i in 0..3 {
            server
                .am_buffer(&serde_json::json!({
                    "user": format!("Unique question {i}"),
                    "assistant": format!("Unique answer {i}")
                }))
                .unwrap();
        }

        let stats = parse_tool_result(&server.am_stats().unwrap());
        assert_eq!(
            stats["episodes"], 1,
            "3 unique exchanges should create 1 episode"
        );

        // Now try to buffer the same first exchange again - should be deduplicated
        let result = server
            .am_buffer(&serde_json::json!({
                "user": "Unique question 0",
                "assistant": "Unique answer 0"
            }))
            .unwrap();
        let json = parse_tool_result(&result);
        assert_eq!(json["deduplicated"], true);
    }

    #[test]
    fn test_am_query_index_returns_compact_entries() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(&serde_json::json!({
                "text": "Quantum mechanics describes particle behavior at subatomic scales. Wave functions collapse on measurement.",
                "name": "science"
            }))
            .unwrap();

        server
            .am_salient(&serde_json::json!({
                "text": "quantum computing is revolutionary technology"
            }))
            .unwrap();

        // Query the index
        let result = server
            .am_query_index(&serde_json::json!({
                "text": "quantum particles"
            }))
            .unwrap();

        let json = parse_tool_result(&result);

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

    #[test]
    fn test_am_retrieve_returns_full_content() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(&serde_json::json!({
                "text": "Rust borrow checker enforces ownership rules at compile time. Lifetimes prevent dangling references.",
                "name": "rust-guide"
            }))
            .unwrap();

        // Get index to find IDs
        let index_result = server
            .am_query_index(&serde_json::json!({
                "text": "rust borrow checker"
            }))
            .unwrap();

        let index_json = parse_tool_result(&index_result);
        let entries = index_json["entries"].as_array().unwrap();
        assert!(!entries.is_empty(), "should have index entries");

        // Pick the first ID
        let first_id = entries[0]["id"].as_str().unwrap().to_string();

        // Retrieve full content
        let retrieve_result = server
            .am_retrieve(&serde_json::json!({
                "ids": [first_id.clone()]
            }))
            .unwrap();

        let retrieve_json = parse_tool_result(&retrieve_result);
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

    #[test]
    fn test_am_retrieve_handles_invalid_ids() {
        let server = make_server();

        let result = server
            .am_retrieve(&serde_json::json!({
                "ids": ["not-a-uuid"]
            }))
            .unwrap();

        let json = parse_tool_result(&result);
        assert_eq!(json["count"], 0, "invalid UUIDs should return empty");
    }

    #[test]
    fn test_am_query_includes_index() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(&serde_json::json!({
                "text": "Geometric memory uses hypersphere manifolds for associative recall. Neighborhoods cluster related concepts.",
                "name": "geo-memory"
            }))
            .unwrap();

        // Add conscious content
        server
            .am_salient(&serde_json::json!({
                "text": "hypersphere manifolds enable geometric reasoning"
            }))
            .unwrap();

        // Query (default path, no budget)
        let result = server
            .am_query(&serde_json::json!({
                "text": "geometric manifold memory"
            }))
            .unwrap();

        let json = parse_tool_result(&result);

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
            .am_query(&serde_json::json!({
                "text": "geometric manifold memory",
                "max_tokens": 500
            }))
            .unwrap();

        let budgeted_json = parse_tool_result(&budgeted_result);
        assert!(
            budgeted_json.get("index").is_some(),
            "budgeted query should also have index field"
        );
        let budgeted_index = budgeted_json["index"].as_array().unwrap();
        assert!(budgeted_index.len() <= 10);
    }

    #[test]
    fn test_dispatch_unknown_tool() {
        let server = make_server();
        let result = server.dispatch_tool("nonexistent", &serde_json::json!({}));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown tool"));
    }

    #[test]
    fn test_am_ingest_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server.am_ingest(&serde_json::json!({ "text": oversized }));
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    #[test]
    fn test_am_buffer_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server.am_buffer(&serde_json::json!({
            "user": oversized,
            "assistant": ""
        }));
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    #[test]
    fn test_am_salient_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server.am_salient(&serde_json::json!({ "text": oversized }));
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    /// Helper: ingest content and return neighborhood IDs from a query.
    fn ingest_and_get_neighborhood_ids(server: &AmServer<BrainStore>) -> Vec<String> {
        server
            .am_ingest(&serde_json::json!({
                "text": "Quantum mechanics describes particle behavior at subatomic scales. Wave functions collapse upon measurement. Entanglement connects distant particles.",
                "name": "quantum"
            }))
            .unwrap();

        let result = server
            .am_query(&serde_json::json!({
                "text": "quantum particles entanglement"
            }))
            .unwrap();
        let json = parse_tool_result(&result);
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

    #[test]
    fn test_am_feedback_boost() {
        let server = make_server();
        let ids = ingest_and_get_neighborhood_ids(&server);
        assert!(
            !ids.is_empty(),
            "query should recall at least one neighborhood"
        );

        let result = server
            .am_feedback(&serde_json::json!({
                "query": "quantum particles",
                "neighborhood_ids": ids,
                "signal": "boost"
            }))
            .unwrap();
        let json = parse_tool_result(&result);
        assert!(
            json["boosted"].as_u64().unwrap() > 0,
            "boost should affect at least one neighborhood"
        );
    }

    #[test]
    fn test_am_feedback_demote() {
        let server = make_server();
        let ids = ingest_and_get_neighborhood_ids(&server);
        assert!(
            !ids.is_empty(),
            "query should recall at least one neighborhood"
        );

        let result = server
            .am_feedback(&serde_json::json!({
                "query": "quantum particles",
                "neighborhood_ids": ids,
                "signal": "demote"
            }))
            .unwrap();
        let json = parse_tool_result(&result);
        assert!(
            json["demoted"].as_u64().unwrap() > 0,
            "demote should affect at least one neighborhood"
        );
    }

    #[test]
    fn test_am_feedback_unknown_signal() {
        let server = make_server();
        let result = server.am_feedback(&serde_json::json!({
            "query": "test",
            "neighborhood_ids": ["00000000-0000-0000-0000-000000000001"],
            "signal": "invalid_signal"
        }));
        assert!(result.is_err(), "unknown signal should return error");
    }

    #[test]
    fn test_am_feedback_empty_ids() {
        let server = make_server();
        let result = server.am_feedback(&serde_json::json!({
            "query": "test",
            "neighborhood_ids": [],
            "signal": "boost"
        }));
        assert!(
            result.is_err(),
            "empty neighborhood_ids should return error"
        );
    }

    #[test]
    fn test_am_batch_query_basic() {
        let server = make_server();

        // Ingest content
        server
            .am_ingest(&serde_json::json!({
                "text": "Rust is a systems programming language focused on safety and performance. Memory safety without garbage collection.",
                "name": "rust-lang"
            }))
            .unwrap();

        let result = server
            .am_batch_query(&serde_json::json!({
                "queries": [
                    { "query": "rust safety" },
                    { "query": "memory management" },
                    { "query": "performance optimization" }
                ]
            }))
            .unwrap();

        let json = parse_tool_result(&result);
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

    #[test]
    fn test_am_batch_query_empty_requests() {
        let server = make_server();

        let result = server
            .am_batch_query(&serde_json::json!({ "queries": [] }))
            .unwrap();

        let json = parse_tool_result(&result);
        let results = json["results"].as_array().expect("results should be array");
        assert!(
            results.is_empty(),
            "empty queries should produce empty results"
        );
    }

    #[test]
    fn test_am_batch_query_per_budget() {
        let server = make_server();

        // Ingest enough content to test budget limits
        server
            .am_ingest(&serde_json::json!({
                "text": "Quantum mechanics describes the behavior of particles at the smallest scales. Superposition allows particles to exist in multiple states simultaneously. Entanglement connects particles across vast distances instantaneously.",
                "name": "quantum"
            }))
            .unwrap();

        let result = server
            .am_batch_query(&serde_json::json!({
                "queries": [
                    { "query": "quantum entanglement", "max_tokens": 50 },
                    { "query": "quantum superposition", "max_tokens": 5000 }
                ]
            }))
            .unwrap();

        let json = parse_tool_result(&result);
        let results = json["results"].as_array().expect("results should be array");
        assert_eq!(results.len(), 2);

        // Both should have budget fields reflecting their token limits
        let budget_small = &results[0]["budget"];
        let budget_large = &results[1]["budget"];
        assert_eq!(budget_small["tokens_budget"], 50);
        assert_eq!(budget_large["tokens_budget"], 5000);
    }

    #[test]
    fn test_am_query_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server.am_query(&serde_json::json!({ "text": oversized }));
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    #[test]
    fn test_am_activate_response_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server.am_activate_response(&serde_json::json!({ "text": oversized }));
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    #[test]
    fn test_am_feedback_rejects_oversized_query() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server.am_feedback(&serde_json::json!({
            "query": oversized,
            "neighborhood_ids": [],
            "signal": "boost"
        }));
        assert!(result.is_err(), "should reject query exceeding size limit");
    }

    #[test]
    fn test_am_batch_query_rejects_oversized_aggregate() {
        let server = make_server();
        // Each query is half the limit; together they exceed it
        let half_plus = "x".repeat(MAX_TOOL_INPUT_BYTES / 2 + 1);
        let result = server.am_batch_query(&serde_json::json!({
            "queries": [
                { "query": half_plus.clone() },
                { "query": half_plus }
            ]
        }));
        assert!(
            result.is_err(),
            "should reject aggregate payload exceeding size limit"
        );
    }

    #[test]
    fn test_am_query_index_rejects_oversized_input() {
        let server = make_server();
        let oversized = "x".repeat(MAX_TOOL_INPUT_BYTES + 1);
        let result = server.am_query_index(&serde_json::json!({ "text": oversized }));
        assert!(result.is_err(), "should reject input exceeding size limit");
    }

    // --- Snapshot tests for MCP tool response shapes ---

    /// Server with pre-ingested content for snapshot tests requiring data.
    fn make_server_with_content() -> AmServer<BrainStore> {
        let server = make_server();
        server
            .am_ingest(&serde_json::json!({
                "text": "Rust ownership rules prevent data races at compile time. The borrow checker enforces exclusive mutable access. Lifetimes track reference validity statically.",
                "name": "rust-safety"
            }))
            .unwrap();
        server
    }

    #[test]
    fn snapshot_am_stats_empty() {
        let server = make_server();
        let result = server.am_stats().unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_stats_empty", json, {
            ".db_bytes" => "[db_bytes]",
        });
    }

    #[test]
    fn snapshot_am_stats_with_content() {
        let server = make_server_with_content();
        let result = server.am_stats().unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_stats_with_content", json, {
            ".db_bytes" => "[db_bytes]",
            ".activation.mean" => insta::rounded_redaction(2),
        });
    }

    #[test]
    fn snapshot_am_ingest() {
        let server = make_server();
        let result = server
            .am_ingest(&serde_json::json!({
                "text": "Testing snapshot output format.",
                "name": "snapshot-test"
            }))
            .unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_ingest", json, {
            ".neighborhoods" => "[count]",
            ".occurrences" => "[count]",
        });
    }

    #[test]
    fn snapshot_am_query() {
        let server = make_server_with_content();
        let result = server
            .am_query(&serde_json::json!({ "text": "rust borrow checker" }))
            .unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_query", json, {
            ".context" => "[context_text]",
            ".metrics.drift_magnitude" => "[float]",
            ".metrics.phase_coherence" => "[float]",
            ".metrics.interference_score" => "[float]",
            ".metrics.query_terms" => "[terms]",
            ".stats.**" => insta::dynamic_redaction(|value, _| {
                if value.as_f64().is_some() { insta::internals::Content::String("[number]".into()) }
                else { value.clone() }
            }),
            ".recalled_ids.**" => "[ids]",
            ".budget.**" => "[budget]",
            ".index" => "[index]",
        });
    }

    #[test]
    fn snapshot_am_query_index() {
        let server = make_server_with_content();
        let result = server
            .am_query_index(&serde_json::json!({ "text": "rust ownership" }))
            .unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_query_index", json, {
            ".entries[].id" => "[uuid]",
            ".entries[].score" => "[float]",
            ".entries[].preview" => "[text]",
            ".total" => "[count]",
        });
    }

    #[test]
    fn snapshot_am_salient() {
        let server = make_server();
        let result = server
            .am_salient(&serde_json::json!({ "text": "Important architectural decision" }))
            .unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_salient", json, {
            ".id" => "[uuid]",
            ".occurrences" => "[count]",
        });
    }

    #[test]
    fn snapshot_am_buffer() {
        let server = make_server();
        let result = server
            .am_buffer(&serde_json::json!({
                "user": "What is ownership?",
                "assistant": "Ownership is Rust's memory management system."
            }))
            .unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_buffer", json);
    }

    #[test]
    fn snapshot_am_activate_response() {
        let server = make_server_with_content();
        let result = server
            .am_activate_response(&serde_json::json!({
                "text": "The borrow checker prevents data races."
            }))
            .unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_activate_response", json, {
            ".activated" => "[count]",
            ".total_occurrences" => "[count]",
        });
    }

    #[test]
    fn snapshot_am_export() {
        let server = make_server_with_content();
        let result = server.am_export().unwrap();
        let json = parse_tool_result(&result);

        // Verify structure rather than snapshot (export contains non-deterministic
        // quaternion positions and UUIDs that change every run)
        assert_eq!(json["version"], "0.7.2");
        assert!(json["system"]["episodes"].is_array());
        assert!(json["system"]["consciousEpisode"].is_object());
        assert!(json["system"]["agentName"].is_string());
        assert!(json["system"]["N"].is_number());
        assert!(json["conversationBuffer"].is_array());
        assert!(json["conversationHistory"].is_array());
    }

    #[test]
    fn snapshot_am_import() {
        let server = make_server_with_content();
        // Export first, parse the JSON text back to a Value for import
        let export_result = server.am_export().unwrap();
        let export_text = export_result["content"][0]["text"].as_str().unwrap();
        let state_value: serde_json::Value = serde_json::from_str(export_text).unwrap();

        let server2 = make_server();
        let result = server2
            .am_import(&serde_json::json!({ "state": state_value }))
            .unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_import", json);
    }

    #[test]
    fn snapshot_am_feedback_boost() {
        let server = make_server_with_content();
        let ids = ingest_and_get_neighborhood_ids(&server);
        if ids.is_empty() {
            return; // Skip if no neighborhoods (determinism guard)
        }
        let result = server
            .am_feedback(&serde_json::json!({
                "query": "quantum",
                "neighborhood_ids": ids,
                "signal": "boost"
            }))
            .unwrap();
        let json = parse_tool_result(&result);

        // Structure assertion (neighborhood IDs are non-deterministic across runs)
        assert!(json["boosted"].is_number());
        assert!(json["demoted"].is_number());
        assert!(json.get("stats").is_some());
    }

    #[test]
    fn snapshot_am_batch_query() {
        let server = make_server_with_content();
        let result = server
            .am_batch_query(&serde_json::json!({
                "queries": [
                    { "query": "rust ownership" },
                    { "query": "borrow checker" }
                ]
            }))
            .unwrap();
        let json = parse_tool_result(&result);
        insta::assert_json_snapshot!("am_batch_query", json, {
            ".results[].context" => "[context_text]",
            ".results[].metrics.**" => "[metric]",
            ".results[].stats.**" => insta::dynamic_redaction(|value, _| {
                if value.as_f64().is_some() { insta::internals::Content::String("[number]".into()) }
                else { value.clone() }
            }),
            ".results[].recalled_ids.**" => "[ids]",
            ".results[].budget.**" => "[budget]",
            ".results[].token_estimate" => "[count]",
            ".results[].index" => "[index]",
        });
    }

    #[test]
    fn snapshot_am_retrieve() {
        let server = make_server_with_content();
        // Get neighborhood IDs by querying the ingested content
        let query_result = server
            .am_query(&serde_json::json!({ "text": "rust ownership borrow" }))
            .unwrap();
        let query_json = parse_tool_result(&query_result);
        let recalled = &query_json["recalled_ids"];
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
        if ids.is_empty() {
            return;
        }

        let result = server
            .am_retrieve(&serde_json::json!({ "ids": [ids[0]] }))
            .unwrap();
        let json = parse_tool_result(&result);

        // Structure assertion (neighborhood data is non-deterministic)
        let entries = json["entries"].as_array().unwrap();
        assert!(!entries.is_empty());
        for entry in entries {
            assert!(entry["id"].is_string());
            assert!(entry["episode"].is_string());
            assert!(entry["text"].is_string());
            assert!(entry["category"].is_string());
        }
        assert!(json["count"].is_number());
    }
}
