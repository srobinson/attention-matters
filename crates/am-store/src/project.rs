use std::path::{Path, PathBuf};
use std::{env, fs};

use am_core::{DAESystem, Episode};

use crate::config::Config;
use crate::error::Result;
use crate::store::Store;

/// Default base directory for all am storage.
pub fn default_base_dir() -> PathBuf {
    dirs_home().join(".attention-matters")
}

fn dirs_home() -> PathBuf {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

// ---------------------------------------------------------------------------
// Startup GC - automatic size management
// ---------------------------------------------------------------------------

/// Run GC if the project DB exceeds the configured size limit.
pub(crate) fn run_gc(store: &Store, config: &Config) {
    let limit = config.db_size_limit_bytes();
    let db_size = store.db_size();
    if db_size < limit {
        return;
    }

    tracing::info!(
        "DB size {}MB exceeds {}MB limit - running GC",
        db_size / (1024 * 1024),
        config.db_size_mb,
    );

    // Phase 1: evict occurrences with zero activation
    match store.gc_pass(am_core::ACTIVATION_FLOOR, &config.retention) {
        Ok(result) => {
            tracing::info!(
                "GC phase 1: evicted {} occurrences (activation <= {}), \
                 removed {} empty episodes. DB: {}MB -> {}MB",
                result.evicted_occurrences,
                am_core::ACTIVATION_FLOOR,
                result.removed_episodes,
                result.before_size / (1024 * 1024),
                result.after_size / (1024 * 1024),
            );

            // Phase 2: if still over limit, aggressively evict coldest
            if result.after_size >= limit {
                let target = (limit as f64 * am_core::DB_GC_TARGET_RATIO) as u64;
                match store.gc_to_target_size(target, &config.retention) {
                    Ok(r2) => {
                        tracing::info!(
                            "GC phase 2 (aggressive): evicted {} more occurrences, \
                             removed {} episodes. DB: {}MB -> {}MB",
                            r2.evicted_occurrences,
                            r2.removed_episodes,
                            r2.before_size / (1024 * 1024),
                            r2.after_size / (1024 * 1024),
                        );
                    }
                    Err(e) => tracing::warn!("GC phase 2 failed: {e}"),
                }
            }
        }
        Err(e) => tracing::warn!("GC phase 1 failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Migration - one-time merge from old multi-DB layout to single brain.db
// ---------------------------------------------------------------------------

/// Migrate the old `projects/*.db` + `global.db` layout into a single `brain.db`.
///
/// Only runs when `projects/` exists and `brain.db` does not. After merging,
/// renames `projects/` to `projects.migrated/` and `global.db` to
/// `global.db.migrated` (belt and suspenders - never deletes).
fn migrate_old_layout(base: &Path, brain_path: &Path) {
    let projects_dir = base.join("projects");
    let global_path = base.join("global.db");

    // Only migrate if brain.db doesn't exist yet
    if brain_path.exists() {
        return;
    }

    tracing::info!("migrating old layout to brain.db");

    let brain_store = match Store::open(brain_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("failed to open brain.db for migration: {e}");
            return;
        }
    };

    let mut brain_system = brain_store
        .load_system()
        .unwrap_or_else(|_| DAESystem::new("am"));

    // Merge all project DBs
    if let Ok(entries) = fs::read_dir(&projects_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("db") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            match Store::open(&path) {
                Ok(project_store) => match project_store.load_system() {
                    Ok(project_system) => {
                        let ep_count = project_system.episodes.len();
                        for episode in project_system.episodes {
                            brain_system.add_episode(episode);
                        }
                        // Merge conscious (deduplicate by ID)
                        let existing_ids: std::collections::HashSet<uuid::Uuid> = brain_system
                            .conscious_episode
                            .neighborhoods
                            .iter()
                            .map(|n| n.id)
                            .collect();
                        for nbhd in project_system.conscious_episode.neighborhoods {
                            if !existing_ids.contains(&nbhd.id) {
                                brain_system.conscious_episode.add_neighborhood(nbhd);
                            }
                        }
                        tracing::info!("merged {} episodes from {}", ep_count, stem);
                    }
                    Err(e) => tracing::warn!("failed to load {}: {e}", path.display()),
                },
                Err(e) => tracing::warn!("failed to open {}: {e}", path.display()),
            }
        }
    }

    // Also merge global.db conscious memories
    if global_path.exists()
        && let Ok(global_store) = Store::open(&global_path)
        && let Ok(global_system) = global_store.load_system()
    {
        let existing_ids: std::collections::HashSet<uuid::Uuid> = brain_system
            .conscious_episode
            .neighborhoods
            .iter()
            .map(|n| n.id)
            .collect();
        let mut merged = 0;
        for nbhd in global_system.conscious_episode.neighborhoods {
            if !existing_ids.contains(&nbhd.id) {
                brain_system.conscious_episode.add_neighborhood(nbhd);
                merged += 1;
            }
        }
        tracing::info!("merged {} conscious neighborhoods from global.db", merged);
    }

    // Intentional save_system: migration merges data from the legacy
    // per-project layout into a single brain.db. Full rewrite is correct.
    brain_system.mark_dirty();
    if let Err(e) = brain_store.save_system(&brain_system) {
        tracing::warn!("failed to save brain.db during migration: {e}");
        return;
    }

    // Rename old dirs to .migrated (don't delete - belt and suspenders)
    let migrated_dir = base.join("projects.migrated");
    if let Err(e) = fs::rename(&projects_dir, &migrated_dir) {
        tracing::warn!("failed to rename projects/ → projects.migrated/: {e}");
    }
    if global_path.exists() {
        let migrated_global = base.join("global.db.migrated");
        if let Err(e) = fs::rename(&global_path, &migrated_global) {
            tracing::warn!("failed to rename global.db → global.db.migrated: {e}");
        }
    }

    tracing::info!(
        "migration complete: {} episodes, {} conscious in brain.db",
        brain_system.episodes.len(),
        brain_system.conscious_episode.neighborhoods.len()
    );
}

// ---------------------------------------------------------------------------
// BrainStore - single brain.db for all developer memory
// ---------------------------------------------------------------------------

/// Single-database store for all developer memory.
///
/// Layout:
/// ```text
/// ~/.attention-matters/
/// └── brain.db          # unified brain - one product, one memory
/// ```
pub struct BrainStore {
    store: Store,
}

impl BrainStore {
    /// Open the brain store using the provided configuration.
    pub fn open(config: &Config) -> Result<Self> {
        let base = &config.data_dir;
        fs::create_dir_all(base)?;

        let brain_path = base.join("brain.db");

        // Startup migration: if old layout exists, merge into brain.db
        let projects_dir = base.join("projects");
        if projects_dir.exists() {
            migrate_old_layout(base, &brain_path);
        }

        let store = Store::open(&brain_path)?;

        if config.gc_enabled {
            run_gc(&store, config);
        }

        Ok(Self { store })
    }

    /// Open with an in-memory store (for testing).
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            store: Store::open_in_memory()?,
        })
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Load the full DAESystem from brain.db.
    pub fn load_system(&self) -> Result<DAESystem> {
        self.store.load_system()
    }

    /// Save a full DAESystem to brain.db (DELETE + reinsert all data).
    ///
    /// Reserved for operations that replace the entire system state:
    /// import, CLI batch ingest, and data migration. MCP hot-path handlers
    /// should use targeted writes (`save_episode`, `save_neighborhood`,
    /// `save_occurrence_positions`, `batch_increment_activation`).
    pub fn save_system(&self, system: &DAESystem) -> Result<()> {
        self.store.save_system(system)
    }

    /// Persist a single episode without rewriting the entire system.
    pub fn save_episode(&self, episode: &Episode) -> Result<()> {
        self.store.save_episode(episode)
    }

    /// Persist a single neighborhood under an episode, creating the episode
    /// row if needed.
    pub fn save_neighborhood(
        &self,
        episode: &am_core::Episode,
        neighborhood: &am_core::Neighborhood,
    ) -> Result<()> {
        self.store.save_neighborhood(episode, neighborhood)
    }

    /// Increment activation counts for a batch of occurrences.
    pub fn batch_increment_activation(&self, ids: &[uuid::Uuid]) -> Result<()> {
        self.store.batch_increment_activation(ids)
    }

    /// Set activation counts to absolute values for a batch of occurrences.
    pub fn batch_set_activation_counts(&self, batch: &[(uuid::Uuid, u32)]) -> Result<()> {
        self.store.batch_set_activation_counts(batch)
    }

    /// Persist position and phasor updates for a batch of occurrences.
    pub fn save_occurrence_positions(
        &self,
        batch: &[(uuid::Uuid, am_core::Quaternion, am_core::DaemonPhasor)],
    ) -> Result<()> {
        self.store.save_occurrence_positions(batch)
    }

    /// Mark text as salient (conscious). Returns the neighborhood ID.
    ///
    /// Uses `save_system` because this convenience method is not on the MCP
    /// hot path (the server handler uses `save_neighborhood` directly).
    /// Only called from CLI code and tests.
    pub fn mark_salient(
        &self,
        system: &mut DAESystem,
        text: &str,
        rng: &mut impl rand::Rng,
    ) -> Result<uuid::Uuid> {
        let nbhd_id = system.add_to_conscious(text, rng);
        // Intentional save_system: convenience method for CLI/test use.
        // The MCP handler (am_salient) uses targeted save_neighborhood.
        self.store.save_system(system)?;
        Ok(nbhd_id)
    }

    /// Import a v0.7.2 JSON file into the brain store.
    pub fn import_json_file(&self, path: &Path) -> Result<()> {
        self.store.import_json_file(path)
    }

    /// Export the brain store to a v0.7.2 JSON file.
    pub fn export_json_file(&self, path: &Path) -> Result<()> {
        self.store.export_json_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_core::{Episode, Neighborhood};
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn make_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test-agent");

        let mut ep = Episode::new("episode-1");
        let tokens = to_tokens(&["hello", "world"]);
        ep.add_neighborhood(Neighborhood::from_tokens(
            &tokens,
            None,
            "hello world",
            &mut rng,
        ));
        sys.add_episode(ep);

        sys
    }

    #[test]
    fn test_brain_salient_queryable() {
        let bs = BrainStore::open_in_memory().unwrap();
        let mut sys = make_system();
        let mut rng = rng();

        bs.mark_salient(&mut sys, "important insight", &mut rng)
            .unwrap();

        let loaded = bs.load_system().unwrap();
        assert_eq!(loaded.conscious_episode.neighborhoods.len(), 1);
    }

    #[test]
    fn test_brain_roundtrip() {
        let bs = BrainStore::open_in_memory().unwrap();
        let sys = make_system();
        bs.save_system(&sys).unwrap();

        let loaded = bs.load_system().unwrap();
        assert_eq!(loaded.episodes.len(), 1);
        assert_eq!(loaded.n(), sys.n());
    }

    #[test]
    fn test_directory_creation() {
        let dir = std::env::temp_dir().join("am-brain-store-test-dirs");
        let _ = fs::remove_dir_all(&dir);

        let config = Config {
            data_dir: dir.clone(),
            ..Config::default()
        };
        let _bs = BrainStore::open(&config).unwrap();

        assert!(dir.join("brain.db").exists());

        let _ = fs::remove_dir_all(&dir);
    }
}
