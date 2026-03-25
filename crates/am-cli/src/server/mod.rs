mod activation;
mod episodes;
mod ingestion;
mod query;
mod system;

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Mutex;
use std::time::Instant;

use rustc_hash::FxHasher;
use serde_json::Value;
use uuid::Uuid;

use am_core::{
    phasor::DaemonPhasor, quaternion::Quaternion, query::QueryManifest, store_trait::AmStore,
    system::DAESystem, tokenizer::ingest_text,
};
use rand::SeedableRng;
use rand::rngs::SmallRng;

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
            "am_episodes" => self.am_episodes(),
            "am_episode_neighborhoods" => self.am_episode_neighborhoods(args),
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
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
