//! In-memory `AmStore` implementation for testing.
//!
//! Provides a minimal, HashMap-backed store that exercises tool handler
//! logic without requiring SQLite. Not intended for production use.

use std::sync::Mutex;

use am_core::{
    activation_stats::ActivationStats,
    episode::Episode,
    neighborhood::Neighborhood,
    phasor::DaemonPhasor,
    quaternion::Quaternion,
    serde_compat::{export_json, import_json},
    store_trait::AmStore,
    system::DAESystem,
};
use uuid::Uuid;

/// Lightweight error type for the in-memory store.
#[derive(Debug, thiserror::Error)]
pub enum MemoryStoreError {
    #[error("{0}")]
    Other(String),
}

/// In-memory `AmStore` for tool handler unit tests.
///
/// Uses `Mutex` for interior mutability since the `AmStore` trait
/// takes `&self` (matching the rusqlite `Connection` pattern).
/// `Mutex` (rather than `RefCell`) makes this type `Send`, which is
/// required by `AmServer<S: AmStore + Send>`.
///
/// System state is stored as serialized JSON and deserialized on load,
/// mirroring the SQLite round-trip behavior of `BrainStore`.
pub struct InMemoryStore {
    state: Mutex<MemoryState>,
}

struct MemoryState {
    /// Serialized JSON representation of the system (None = empty store).
    system_json: Option<String>,
    buffer: Vec<(String, String)>,
}

impl InMemoryStore {
    /// Create a new empty in-memory store.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(MemoryState {
                system_json: None,
                buffer: Vec::new(),
            }),
        }
    }

    /// Create a store pre-loaded with a system.
    ///
    /// # Panics
    /// Panics if the system cannot be serialized (should never happen).
    #[must_use]
    pub fn with_system(system: &DAESystem) -> Self {
        let json = export_json(system).expect("DAESystem serialization should not fail");
        Self {
            state: Mutex::new(MemoryState {
                system_json: Some(json),
                buffer: Vec::new(),
            }),
        }
    }

    fn load_system_inner(json: &str) -> Result<DAESystem, MemoryStoreError> {
        import_json(json).map_err(|e| MemoryStoreError::Other(format!("deserialize: {e}")))
    }
}

impl Default for InMemoryStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AmStore for InMemoryStore {
    type Error = MemoryStoreError;

    fn load_system(&self) -> Result<DAESystem, Self::Error> {
        let state = self.state.lock().unwrap();
        match &state.system_json {
            Some(json) => Self::load_system_inner(json),
            None => Err(MemoryStoreError::Other("no system loaded".into())),
        }
    }

    fn save_system(&self, system: &DAESystem) -> Result<(), Self::Error> {
        let json =
            export_json(system).map_err(|e| MemoryStoreError::Other(format!("serialize: {e}")))?;
        self.state.lock().unwrap().system_json = Some(json);
        Ok(())
    }

    fn save_episode(&self, episode: &Episode) -> Result<(), Self::Error> {
        let mut system = self.load_system()?;
        if let Some(pos) = system.episodes.iter().position(|e| e.id == episode.id) {
            system.episodes[pos] = episode.clone();
        } else {
            system.episodes.push(episode.clone());
        }
        self.save_system(&system)
    }

    fn save_neighborhood(
        &self,
        episode: &Episode,
        neighborhood: &Neighborhood,
    ) -> Result<(), Self::Error> {
        let mut system = self.load_system()?;

        let ep = if episode.is_conscious {
            &mut system.conscious_episode
        } else if let Some(ep) = system.episodes.iter_mut().find(|e| e.id == episode.id) {
            ep
        } else {
            system.episodes.push(episode.clone());
            system.episodes.last_mut().unwrap()
        };

        if let Some(pos) = ep
            .neighborhoods
            .iter()
            .position(|n| n.id == neighborhood.id)
        {
            ep.neighborhoods[pos] = neighborhood.clone();
        } else {
            ep.neighborhoods.push(neighborhood.clone());
        }
        self.save_system(&system)
    }

    fn batch_increment_activation(&self, ids: &[Uuid]) -> Result<(), Self::Error> {
        let mut system = self.load_system()?;
        for id in ids {
            for ep in
                std::iter::once(&mut system.conscious_episode).chain(system.episodes.iter_mut())
            {
                for nbhd in &mut ep.neighborhoods {
                    for occ in &mut nbhd.occurrences {
                        if occ.id == *id {
                            occ.activation_count += 1;
                        }
                    }
                }
            }
        }
        self.save_system(&system)
    }

    fn batch_set_activation_counts(&self, batch: &[(Uuid, u32)]) -> Result<(), Self::Error> {
        let mut system = self.load_system()?;
        for (id, count) in batch {
            for ep in
                std::iter::once(&mut system.conscious_episode).chain(system.episodes.iter_mut())
            {
                for nbhd in &mut ep.neighborhoods {
                    for occ in &mut nbhd.occurrences {
                        if occ.id == *id {
                            occ.activation_count = *count;
                        }
                    }
                }
            }
        }
        self.save_system(&system)
    }

    fn save_occurrence_positions(
        &self,
        batch: &[(Uuid, Quaternion, DaemonPhasor)],
    ) -> Result<(), Self::Error> {
        let mut system = self.load_system()?;
        for (id, pos, phasor) in batch {
            for ep in
                std::iter::once(&mut system.conscious_episode).chain(system.episodes.iter_mut())
            {
                for nbhd in &mut ep.neighborhoods {
                    for occ in &mut nbhd.occurrences {
                        if occ.id == *id {
                            occ.position = *pos;
                            occ.phasor = *phasor;
                        }
                    }
                }
            }
        }
        self.save_system(&system)
    }

    fn mark_superseded(&self, old_id: Uuid, new_id: Uuid) -> Result<(), Self::Error> {
        let mut system = self.load_system()?;
        for ep in std::iter::once(&mut system.conscious_episode).chain(system.episodes.iter_mut()) {
            for nbhd in &mut ep.neighborhoods {
                if nbhd.id == old_id {
                    nbhd.superseded_by = Some(new_id);
                    self.save_system(&system)?;
                    return Ok(());
                }
            }
        }
        Err(MemoryStoreError::Other(format!(
            "neighborhood not found: {old_id}"
        )))
    }

    fn append_buffer(&self, user: &str, assistant: &str) -> Result<usize, Self::Error> {
        let mut state = self.state.lock().unwrap();
        state.buffer.push((user.to_owned(), assistant.to_owned()));
        Ok(state.buffer.len())
    }

    fn drain_buffer(&self) -> Result<Vec<(String, String)>, Self::Error> {
        let mut state = self.state.lock().unwrap();
        Ok(std::mem::take(&mut state.buffer))
    }

    fn buffer_count(&self) -> Result<usize, Self::Error> {
        Ok(self.state.lock().unwrap().buffer.len())
    }

    fn activation_distribution(&self) -> Result<ActivationStats, Self::Error> {
        let system = self.load_system()?;

        let mut total: u64 = 0;
        let mut zero_count: u64 = 0;
        let mut max_act: u32 = 0;
        let mut sum: u64 = 0;

        for ep in std::iter::once(&system.conscious_episode).chain(system.episodes.iter()) {
            for nbhd in &ep.neighborhoods {
                for occ in &nbhd.occurrences {
                    total += 1;
                    let act = occ.activation_count;
                    if act == 0 {
                        zero_count += 1;
                    }
                    if act > max_act {
                        max_act = act;
                    }
                    sum += u64::from(act);
                }
            }
        }

        Ok(ActivationStats {
            total,
            zero_activation: zero_count,
            max_activation: max_act,
            mean_activation: if total > 0 {
                sum as f64 / total as f64
            } else {
                0.0
            },
        })
    }

    fn db_size(&self) -> u64 {
        0
    }

    fn health_check(&self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn checkpoint_truncate(&self) -> Result<(), Self::Error> {
        Ok(())
    }

    fn forget_episode(&self, episode_id: &str) -> Result<u64, Self::Error> {
        let uuid: Uuid = episode_id
            .parse()
            .map_err(|e| MemoryStoreError::Other(format!("invalid UUID: {e}")))?;

        let mut system = self.load_system()?;

        let pos = system.episodes.iter().position(|ep| ep.id == uuid);
        match pos {
            None => Ok(0),
            Some(i) => {
                if system.episodes[i].is_conscious {
                    return Err(MemoryStoreError::Other(
                        "use forget_conscious to remove conscious memories".into(),
                    ));
                }
                let removed: u64 = system.episodes[i]
                    .neighborhoods
                    .iter()
                    .map(|n| n.occurrences.len() as u64)
                    .sum();
                system.episodes.remove(i);
                self.save_system(&system)?;
                Ok(removed)
            }
        }
    }

    fn forget_conscious(&self, neighborhood_id: &str) -> Result<u64, Self::Error> {
        let uuid: Uuid = neighborhood_id
            .parse()
            .map_err(|e| MemoryStoreError::Other(format!("invalid UUID: {e}")))?;

        let mut system = self.load_system()?;

        let pos = system
            .conscious_episode
            .neighborhoods
            .iter()
            .position(|n| n.id == uuid);
        match pos {
            None => Ok(0),
            Some(i) => {
                let removed = system.conscious_episode.neighborhoods[i].occurrences.len() as u64;
                system.conscious_episode.neighborhoods.remove(i);
                self.save_system(&system)?;
                Ok(removed)
            }
        }
    }

    fn forget_term(&self, term: &str) -> Result<(u64, u64, u64), Self::Error> {
        let word_lower = term.to_lowercase();
        let mut system = self.load_system()?;
        let mut removed_occs: u64 = 0;

        // Remove matching occurrences from all episodes (including conscious)
        for ep in std::iter::once(&mut system.conscious_episode).chain(system.episodes.iter_mut()) {
            for nbhd in &mut ep.neighborhoods {
                let before = nbhd.occurrences.len();
                nbhd.occurrences
                    .retain(|occ| occ.word.to_lowercase() != word_lower);
                removed_occs += (before - nbhd.occurrences.len()) as u64;
            }
        }

        // Clean empty neighborhoods
        let mut removed_nbhds: u64 = 0;
        for ep in std::iter::once(&mut system.conscious_episode).chain(system.episodes.iter_mut()) {
            let before = ep.neighborhoods.len();
            ep.neighborhoods.retain(|n| !n.occurrences.is_empty());
            removed_nbhds += (before - ep.neighborhoods.len()) as u64;
        }

        // Clean empty non-conscious episodes
        let before = system.episodes.len();
        system.episodes.retain(|ep| !ep.neighborhoods.is_empty());
        let removed_eps = (before - system.episodes.len()) as u64;

        self.save_system(&system)?;
        Ok((removed_occs, removed_nbhds, removed_eps))
    }

    fn import_json_str(&self, json: &str) -> Result<(), Self::Error> {
        let system = am_core::serde_compat::import_json(json)
            .map_err(|e| MemoryStoreError::Other(format!("invalid JSON: {e}")))?;
        self.save_system(&system)
    }

    fn export_json_string(&self) -> Result<String, Self::Error> {
        let system = self.load_system()?;
        am_core::serde_compat::export_json(&system)
            .map_err(|e| MemoryStoreError::Other(format!("JSON export failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Compile-time assertion: InMemoryStore must be Send (required by AmServer<S: Send>).
    const _: () = {
        const fn assert_send<T: Send>() {}
        assert_send::<InMemoryStore>();
    };

    #[test]
    fn test_buffer_roundtrip() {
        let store = InMemoryStore::new();
        assert_eq!(store.buffer_count().unwrap(), 0);

        let count = store.append_buffer("hello", "world").unwrap();
        assert_eq!(count, 1);
        assert_eq!(store.buffer_count().unwrap(), 1);

        let drained = store.drain_buffer().unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0], ("hello".to_owned(), "world".to_owned()));
        assert_eq!(store.buffer_count().unwrap(), 0);
    }

    #[test]
    fn test_system_roundtrip() {
        let store = InMemoryStore::new();
        assert!(store.load_system().is_err());

        let system = DAESystem::new("test");
        store.save_system(&system).unwrap();

        let loaded = store.load_system().unwrap();
        assert_eq!(loaded.episodes.len(), system.episodes.len());
    }

    #[test]
    fn test_with_system_constructor() {
        let system = DAESystem::new("test");
        let store = InMemoryStore::with_system(&system);
        let loaded = store.load_system().unwrap();
        assert_eq!(loaded.episodes.len(), system.episodes.len());
    }

    #[test]
    fn test_health_check_and_checkpoint() {
        let store = InMemoryStore::new();
        store.health_check().unwrap();
        store.checkpoint_truncate().unwrap();
        assert_eq!(store.db_size(), 0);
    }

    #[test]
    fn test_activation_distribution_empty() {
        let system = DAESystem::new("test");
        let store = InMemoryStore::with_system(&system);
        let stats = store.activation_distribution().unwrap();
        assert_eq!(stats.total, 0);
        assert_eq!(stats.zero_activation, 0);
        assert_eq!(stats.max_activation, 0);
    }

    fn make_populated_store() -> InMemoryStore {
        use am_core::{episode::Episode, neighborhood::Neighborhood};
        use rand::SeedableRng;
        use rand::rngs::SmallRng;

        let mut rng = SmallRng::seed_from_u64(42);
        let mut sys = DAESystem::new("test-agent");

        let tokens: Vec<String> = ["hello", "world", "rust"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let mut ep = Episode::new("ep-1");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &tokens,
            None,
            "hello world rust",
            &mut rng,
        ));
        sys.add_episode(ep);
        sys.add_to_conscious("conscious thought here", &mut rng);

        InMemoryStore::with_system(&sys)
    }

    #[test]
    fn test_forget_episode() {
        let store = make_populated_store();
        let sys = store.load_system().unwrap();
        let ep_id = sys.episodes[0].id.to_string();

        let removed = store.forget_episode(&ep_id).unwrap();
        assert!(removed > 0);

        let after = store.load_system().unwrap();
        assert!(after.episodes.is_empty());
    }

    #[test]
    fn test_forget_episode_not_found() {
        let store = make_populated_store();
        let removed = store
            .forget_episode("00000000-0000-0000-0000-000000000099")
            .unwrap();
        assert_eq!(removed, 0);
    }

    #[test]
    fn test_forget_conscious() {
        let store = make_populated_store();
        let sys = store.load_system().unwrap();
        assert!(!sys.conscious_episode.neighborhoods.is_empty());

        let nbhd_id = sys.conscious_episode.neighborhoods[0].id.to_string();
        let removed = store.forget_conscious(&nbhd_id).unwrap();
        assert!(removed > 0);

        let after = store.load_system().unwrap();
        assert!(after.conscious_episode.neighborhoods.is_empty());
    }

    #[test]
    fn test_forget_term() {
        let store = make_populated_store();
        let (occs, _nbhds, _eps) = store.forget_term("hello").unwrap();
        assert!(occs > 0);

        // Verify the term is gone
        let sys = store.load_system().unwrap();
        for ep in std::iter::once(&sys.conscious_episode).chain(sys.episodes.iter()) {
            for nbhd in &ep.neighborhoods {
                for occ in &nbhd.occurrences {
                    assert_ne!(occ.word.to_lowercase(), "hello");
                }
            }
        }
    }

    #[test]
    fn test_forget_term_not_found() {
        let store = make_populated_store();
        let (occs, nbhds, eps) = store.forget_term("nonexistent").unwrap();
        assert_eq!(occs, 0);
        assert_eq!(nbhds, 0);
        assert_eq!(eps, 0);
    }

    #[test]
    fn test_import_export_json_roundtrip() {
        let store = make_populated_store();
        let json = store.export_json_string().unwrap();

        let store2 = InMemoryStore::new();
        store2.import_json_str(&json).unwrap();

        let sys1 = store.load_system().unwrap();
        let sys2 = store2.load_system().unwrap();
        assert_eq!(sys1.n(), sys2.n());
        assert_eq!(sys1.episodes.len(), sys2.episodes.len());
        assert_eq!(sys1.agent_name, sys2.agent_name);
    }

    #[test]
    fn test_import_invalid_json() {
        let store = InMemoryStore::new();
        let result = store.import_json_str("not valid json");
        assert!(result.is_err());
    }
}
