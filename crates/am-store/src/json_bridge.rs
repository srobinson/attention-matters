use std::fs;
use std::path::Path;

use am_core::{export_json, import_json};

use crate::error::{Result, StoreError};
use crate::store::Store;

impl Store {
    /// Import a v0.7.2 JSON export file into this store.
    /// Handles both "phasor" and "theta" field names (via am-core serde alias).
    pub fn import_json_file(&self, path: &Path) -> Result<()> {
        let json = fs::read_to_string(path).map_err(|e| {
            StoreError::InvalidData(format!("failed to read {}: {e}", path.display()))
        })?;
        let system = import_json(&json)
            .map_err(|e| StoreError::InvalidData(format!("invalid JSON: {e}")))?;
        self.save_system(&system)
    }

    /// Import a v0.7.2 JSON string into this store.
    pub fn import_json_str(&self, json: &str) -> Result<()> {
        let system =
            import_json(json).map_err(|e| StoreError::InvalidData(format!("invalid JSON: {e}")))?;
        self.save_system(&system)
    }

    /// Export the store contents to a v0.7.2 JSON file.
    pub fn export_json_file(&self, path: &Path) -> Result<()> {
        let json = self.export_json_string()?;
        fs::write(path, json).map_err(|e| {
            StoreError::InvalidData(format!("failed to write {}: {e}", path.display()))
        })
    }

    /// Export the store contents as a v0.7.2 JSON string.
    pub fn export_json_string(&self) -> Result<String> {
        let system = self.load_system()?;
        export_json(&system)
            .map_err(|e| StoreError::InvalidData(format!("JSON export failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_core::{DAESystem, Episode, Neighborhood};
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

        let mut ep = Episode::new("memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["hello", "world", "rust"]),
            None,
            "hello world rust",
            &mut rng,
        ));
        sys.add_episode(ep);
        sys.add_to_conscious("conscious thought here", &mut rng);

        sys
    }

    #[test]
    fn test_import_export_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let original = make_system();

        // Export from am-core to JSON, import into store
        let json = export_json(&original).unwrap();
        store.import_json_str(&json).unwrap();

        // Export back from store to JSON
        let exported = store.export_json_string().unwrap();

        // Parse both and compare structure
        let reimported = import_json(&exported).unwrap();
        assert_eq!(original.agent_name, reimported.agent_name);
        assert_eq!(original.episodes.len(), reimported.episodes.len());
        assert_eq!(original.n(), reimported.n());
    }

    #[test]
    fn test_import_preserves_data() {
        let store = Store::open_in_memory().unwrap();
        let original = make_system();
        let json = export_json(&original).unwrap();

        store.import_json_str(&json).unwrap();
        let loaded = store.load_system().unwrap();

        // Check words preserved
        let orig_words: Vec<&str> = original.episodes[0]
            .all_occurrences()
            .map(|o| o.word.as_str())
            .collect();
        let loaded_words: Vec<&str> = loaded.episodes[0]
            .all_occurrences()
            .map(|o| o.word.as_str())
            .collect();
        assert_eq!(orig_words, loaded_words);

        // Check quaternion precision
        let orig_pos = original.episodes[0].neighborhoods[0].occurrences[0].position;
        let loaded_pos = loaded.episodes[0].neighborhoods[0].occurrences[0].position;
        assert!(
            orig_pos.angular_distance(loaded_pos) < 1e-10,
            "quaternion precision lost"
        );

        // Check phasor precision
        let orig_theta = original.episodes[0].neighborhoods[0].occurrences[0]
            .phasor
            .theta;
        let loaded_theta = loaded.episodes[0].neighborhoods[0].occurrences[0]
            .phasor
            .theta;
        assert!(
            (orig_theta - loaded_theta).abs() < 1e-10,
            "phasor precision lost"
        );
    }

    #[test]
    fn test_export_matches_v072_format() {
        let store = Store::open_in_memory().unwrap();
        let original = make_system();
        let json = export_json(&original).unwrap();

        store.import_json_str(&json).unwrap();
        let exported = store.export_json_string().unwrap();

        // Parse as wire format to verify structure
        let wire: serde_json::Value = serde_json::from_str(&exported).unwrap();
        assert_eq!(wire["version"], "0.7.2");
        assert!(wire["system"]["episodes"].is_array());
        assert!(wire["system"]["consciousEpisode"].is_object());
        assert!(wire["system"]["agentName"].is_string());
        assert!(wire["system"]["N"].is_number());
        assert!(wire["conversationBuffer"].is_array());
        assert!(wire["conversationHistory"].is_array());
    }

    #[test]
    fn test_import_theta_alias() {
        let json = r#"{
            "version": "0.7.2",
            "timestamp": "",
            "system": {
                "episodes": [{
                    "name": "test",
                    "isConscious": false,
                    "id": "00000000-0000-0000-0000-000000000001",
                    "neighborhoods": [{
                        "seed": [1.0, 0.0, 0.0, 0.0],
                        "id": "00000000-0000-0000-0000-000000000002",
                        "occurrences": [{
                            "word": "hello",
                            "position": [1.0, 0.0, 0.0, 0.0],
                            "theta": 2.345,
                            "activationCount": 3,
                            "neighborhoodId": "00000000-0000-0000-0000-000000000002"
                        }]
                    }]
                }],
                "consciousEpisode": {
                    "name": "conscious",
                    "isConscious": true,
                    "id": "00000000-0000-0000-0000-000000000003",
                    "neighborhoods": []
                },
                "agentName": "echo"
            }
        }"#;

        let store = Store::open_in_memory().unwrap();
        store.import_json_str(json).unwrap();

        let loaded = store.load_system().unwrap();
        let occ = &loaded.episodes[0].neighborhoods[0].occurrences[0];
        assert_eq!(occ.word, "hello");
        assert!((occ.phasor.theta - 2.345).abs() < 1e-10);
        assert_eq!(occ.activation_count, 3);
    }

    #[test]
    fn test_import_export_file_roundtrip() {
        let dir = std::env::temp_dir().join("am-store-test-json");
        let _ = fs::create_dir_all(&dir);
        let json_path = dir.join("test_export.json");

        let store = Store::open_in_memory().unwrap();
        let original = make_system();
        store.save_system(&original).unwrap();

        // Export to file
        store.export_json_file(&json_path).unwrap();
        assert!(json_path.exists());

        // Import from file into a fresh store
        let store2 = Store::open_in_memory().unwrap();
        store2.import_json_file(&json_path).unwrap();

        let loaded = store2.load_system().unwrap();
        assert_eq!(loaded.agent_name, original.agent_name);
        assert_eq!(loaded.n(), original.n());

        // Cleanup
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_import_invalid_json() {
        let store = Store::open_in_memory().unwrap();
        let result = store.import_json_str("not valid json");
        assert!(result.is_err());
    }

    #[test]
    fn test_sqlite_to_json_to_sqlite_roundtrip() {
        // Full round-trip: system → SQLite → JSON → SQLite → verify
        let store1 = Store::open_in_memory().unwrap();
        let original = make_system();
        store1.save_system(&original).unwrap();

        let json = store1.export_json_string().unwrap();

        let store2 = Store::open_in_memory().unwrap();
        store2.import_json_str(&json).unwrap();

        let loaded = store2.load_system().unwrap();
        assert_eq!(loaded.agent_name, original.agent_name);
        assert_eq!(loaded.episodes.len(), original.episodes.len());
        assert_eq!(loaded.n(), original.n());
        assert_eq!(
            loaded.conscious_episode.neighborhoods.len(),
            original.conscious_episode.neighborhoods.len()
        );
    }
}
