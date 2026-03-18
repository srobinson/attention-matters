//! In-memory `AmStore` implementation for testing.
//!
//! Provides a minimal, HashMap-backed store that exercises tool handler
//! logic without requiring SQLite. Not intended for production use.

use std::cell::RefCell;

use am_core::{
    ActivationStats, AmStore, DAESystem, DaemonPhasor, Episode, Neighborhood, Quaternion,
    export_json, import_json,
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
/// Uses `RefCell` for interior mutability since the `AmStore` trait
/// takes `&self` (matching the rusqlite `Connection` pattern).
/// Single-threaded test use only.
///
/// System state is stored as serialized JSON and deserialized on load,
/// mirroring the SQLite round-trip behavior of `BrainStore`.
pub struct InMemoryStore {
    state: RefCell<MemoryState>,
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
            state: RefCell::new(MemoryState {
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
            state: RefCell::new(MemoryState {
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
        let state = self.state.borrow();
        match &state.system_json {
            Some(json) => Self::load_system_inner(json),
            None => Err(MemoryStoreError::Other("no system loaded".into())),
        }
    }

    fn save_system(&self, system: &DAESystem) -> Result<(), Self::Error> {
        let json =
            export_json(system).map_err(|e| MemoryStoreError::Other(format!("serialize: {e}")))?;
        self.state.borrow_mut().system_json = Some(json);
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
        let mut state = self.state.borrow_mut();
        state.buffer.push((user.to_owned(), assistant.to_owned()));
        Ok(state.buffer.len())
    }

    fn drain_buffer(&self) -> Result<Vec<(String, String)>, Self::Error> {
        let mut state = self.state.borrow_mut();
        Ok(std::mem::take(&mut state.buffer))
    }

    fn buffer_count(&self) -> Result<usize, Self::Error> {
        Ok(self.state.borrow().buffer.len())
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
