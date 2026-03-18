use uuid::Uuid;

use crate::{ActivationStats, DAESystem, DaemonPhasor, Episode, Neighborhood, Quaternion};

/// Hexagonal port for DAE persistence.
///
/// Defines the storage surface required by `AmServer` (MCP tool handlers).
/// am-core owns the trait (port); am-store provides the adapter (`BrainStore`).
///
/// Sync signatures: rusqlite is synchronous and a single-client stdio server
/// gains nothing from async wrappers. If a future HTTP adapter needs async,
/// `spawn_blocking` wraps at that boundary.
///
/// Scope: MCP server operations only. CLI-only helpers (`forget_*`,
/// `import_json_file`, `export_json_file`, `mark_salient`) stay on concrete
/// store types and are not part of this trait.
pub trait AmStore {
    /// Error type for fallible operations.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Load the full `DAESystem` from persistence.
    ///
    /// # Errors
    /// Returns `Self::Error` if the underlying storage is unreachable or data is corrupt.
    fn load_system(&self) -> Result<DAESystem, Self::Error>;

    /// Persist a full `DAESystem` (DELETE + reinsert).
    ///
    /// Reserved for operations that replace the entire system state:
    /// import, CLI batch ingest, and data migration. MCP hot-path handlers
    /// should use targeted writes.
    ///
    /// # Errors
    /// Returns `Self::Error` if the write transaction fails.
    fn save_system(&self, system: &DAESystem) -> Result<(), Self::Error>;

    /// Persist a single episode without rewriting the entire system.
    ///
    /// # Errors
    /// Returns `Self::Error` if the write transaction fails.
    fn save_episode(&self, episode: &Episode) -> Result<(), Self::Error>;

    /// Persist a single neighborhood under an episode, creating the episode
    /// row if needed.
    ///
    /// # Errors
    /// Returns `Self::Error` if the write transaction fails.
    fn save_neighborhood(
        &self,
        episode: &Episode,
        neighborhood: &Neighborhood,
    ) -> Result<(), Self::Error>;

    /// Increment activation counts for a batch of occurrences.
    ///
    /// # Errors
    /// Returns `Self::Error` if the batch update fails.
    fn batch_increment_activation(&self, ids: &[Uuid]) -> Result<(), Self::Error>;

    /// Set activation counts to absolute values for a batch of occurrences.
    ///
    /// # Errors
    /// Returns `Self::Error` if the batch update fails.
    fn batch_set_activation_counts(&self, batch: &[(Uuid, u32)]) -> Result<(), Self::Error>;

    /// Persist position and phasor updates for a batch of occurrences.
    ///
    /// # Errors
    /// Returns `Self::Error` if the batch update fails.
    fn save_occurrence_positions(
        &self,
        batch: &[(Uuid, Quaternion, DaemonPhasor)],
    ) -> Result<(), Self::Error>;

    /// Mark a neighborhood as superseded by another.
    ///
    /// # Errors
    /// Returns `Self::Error` if the old neighborhood ID is not found.
    fn mark_superseded(&self, old_id: Uuid, new_id: Uuid) -> Result<(), Self::Error>;

    /// Append a user/assistant exchange to the conversation buffer.
    /// Returns the new buffer size.
    ///
    /// # Errors
    /// Returns `Self::Error` if the insert fails.
    fn append_buffer(&self, user: &str, assistant: &str) -> Result<usize, Self::Error>;

    /// Drain all buffered exchanges, returning them in insertion order.
    ///
    /// # Errors
    /// Returns `Self::Error` if the read or delete transaction fails.
    fn drain_buffer(&self) -> Result<Vec<(String, String)>, Self::Error>;

    /// Number of exchanges currently in the conversation buffer.
    ///
    /// # Errors
    /// Returns `Self::Error` if the count query fails.
    fn buffer_count(&self) -> Result<usize, Self::Error>;

    /// Summary statistics for occurrence activation counts.
    ///
    /// # Errors
    /// Returns `Self::Error` if the aggregation query fails.
    fn activation_distribution(&self) -> Result<ActivationStats, Self::Error>;

    /// Database file size in bytes (0 for in-memory stores).
    fn db_size(&self) -> u64;

    /// Verify the connection is still usable.
    ///
    /// # Errors
    /// Returns `Self::Error` if the connection check fails.
    fn health_check(&self) -> Result<(), Self::Error>;

    /// Flush WAL and truncate. Used during clean shutdown.
    ///
    /// # Errors
    /// Returns `Self::Error` if the checkpoint operation fails.
    fn checkpoint_truncate(&self) -> Result<(), Self::Error>;

    // --- CLI-facing methods (forget, import/export) ---

    /// Delete a subconscious episode and all its contents.
    /// Returns the number of occurrences removed (0 if not found).
    ///
    /// # Errors
    /// Returns `Self::Error` if the episode is conscious or the delete fails.
    fn forget_episode(&self, episode_id: &str) -> Result<u64, Self::Error>;

    /// Delete a conscious neighborhood by UUID.
    /// Returns the number of occurrences removed (0 if not found).
    ///
    /// # Errors
    /// Returns `Self::Error` if the neighborhood is not conscious or the delete fails.
    fn forget_conscious(&self, neighborhood_id: &str) -> Result<u64, Self::Error>;

    /// Delete all occurrences matching a word (case-insensitive) and clean empty structures.
    /// Returns `(removed_occurrences, removed_neighborhoods, removed_episodes)`.
    ///
    /// # Errors
    /// Returns `Self::Error` if the delete transaction fails.
    fn forget_term(&self, term: &str) -> Result<(u64, u64, u64), Self::Error>;

    /// Import a v0.7.2 JSON string into the store (replaces all state).
    ///
    /// # Errors
    /// Returns `Self::Error` if the JSON is invalid or the write fails.
    fn import_json_str(&self, json: &str) -> Result<(), Self::Error>;

    /// Export the store contents as a v0.7.2 JSON string.
    ///
    /// # Errors
    /// Returns `Self::Error` if serialization or the read fails.
    fn export_json_string(&self) -> Result<String, Self::Error>;
}
