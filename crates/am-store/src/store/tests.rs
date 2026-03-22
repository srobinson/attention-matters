use super::*;
use am_core::{
    episode::Episode, neighborhood::Neighborhood, phasor::DaemonPhasor, quaternion::Quaternion,
    system::DAESystem,
};
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

    let mut ep1 = Episode::new("episode-1");
    let tokens = to_tokens(&["hello", "world", "test"]);
    let n = Neighborhood::from_tokens(&tokens, None, "hello world test", &mut rng);
    ep1.add_neighborhood(n);
    sys.add_episode(ep1);

    sys.add_to_conscious("conscious thought", &mut rng);

    sys
}

#[test]
fn test_save_and_load_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    let original = make_system();

    store.save_system(&original).unwrap();
    let loaded = store.load_system().unwrap();

    assert_eq!(loaded.agent_name, "test-agent");
    assert_eq!(loaded.episodes.len(), 1);
    assert_eq!(loaded.episodes[0].name, "episode-1");
    assert_eq!(loaded.episodes[0].neighborhoods.len(), 1);
    assert_eq!(loaded.episodes[0].neighborhoods[0].occurrences.len(), 3);
    assert!(loaded.conscious_episode.is_conscious);
    assert_eq!(loaded.conscious_episode.neighborhoods.len(), 1);
}

#[test]
fn test_quaternion_precision_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    let original = make_system();

    let orig_pos = original.episodes[0].neighborhoods[0].occurrences[0].position;

    store.save_system(&original).unwrap();
    let loaded = store.load_system().unwrap();

    let loaded_pos = loaded.episodes[0].neighborhoods[0].occurrences[0].position;
    let dist = orig_pos.angular_distance(loaded_pos);
    assert!(dist < 1e-10, "quaternion drift: {dist}");
}

#[test]
fn test_phasor_roundtrip() {
    let store = Store::open_in_memory().unwrap();
    let original = make_system();

    let orig_theta = original.episodes[0].neighborhoods[0].occurrences[0]
        .phasor
        .theta;

    store.save_system(&original).unwrap();
    let loaded = store.load_system().unwrap();

    let loaded_theta = loaded.episodes[0].neighborhoods[0].occurrences[0]
        .phasor
        .theta;
    assert!(
        (orig_theta - loaded_theta).abs() < 1e-10,
        "phasor drift: {} vs {}",
        orig_theta,
        loaded_theta
    );
}

#[test]
fn test_increment_activation() {
    let store = Store::open_in_memory().unwrap();
    let system = make_system();
    store.save_system(&system).unwrap();

    let occ_id = system.episodes[0].neighborhoods[0].occurrences[0].id;

    store.increment_activation(occ_id).unwrap();
    store.increment_activation(occ_id).unwrap();

    let loaded = store.load_system().unwrap();
    let loaded_count = loaded.episodes[0].neighborhoods[0].occurrences[0].activation_count;
    assert_eq!(loaded_count, 2);
}

#[test]
fn test_batch_increment_activation() {
    let store = Store::open_in_memory().unwrap();
    let system = make_system();
    store.save_system(&system).unwrap();

    let occ0 = system.episodes[0].neighborhoods[0].occurrences[0].id;
    let occ1 = system.episodes[0].neighborhoods[0].occurrences[1].id;

    // Batch increment both occurrences twice
    store.batch_increment_activation(&[occ0, occ1]).unwrap();
    store.batch_increment_activation(&[occ0, occ1]).unwrap();

    let loaded = store.load_system().unwrap();
    let c0 = loaded.episodes[0].neighborhoods[0].occurrences[0].activation_count;
    let c1 = loaded.episodes[0].neighborhoods[0].occurrences[1].activation_count;
    assert_eq!(c0, 2, "first occurrence should have activation_count 2");
    assert_eq!(c1, 2, "second occurrence should have activation_count 2");
}

#[test]
fn test_batch_increment_activation_empty() {
    let store = Store::open_in_memory().unwrap();
    // Empty batch should be a no-op
    store.batch_increment_activation(&[]).unwrap();
}

#[test]
fn test_batch_increment_activation_skips_unknown() {
    let store = Store::open_in_memory().unwrap();
    let system = make_system();
    store.save_system(&system).unwrap();

    let occ0 = system.episodes[0].neighborhoods[0].occurrences[0].id;
    let unknown = Uuid::new_v4();

    // Mixed batch: one real, one unknown. Should succeed without error.
    store.batch_increment_activation(&[occ0, unknown]).unwrap();

    let loaded = store.load_system().unwrap();
    let c0 = loaded.episodes[0].neighborhoods[0].occurrences[0].activation_count;
    assert_eq!(c0, 1, "known occurrence should be incremented");
}

#[test]
fn test_batch_set_activation_counts() {
    let store = Store::open_in_memory().unwrap();
    let system = make_system();
    store.save_system(&system).unwrap();

    let occ0 = system.episodes[0].neighborhoods[0].occurrences[0].id;
    let occ1 = system.episodes[0].neighborhoods[0].occurrences[1].id;

    // Set absolute activation counts
    store
        .batch_set_activation_counts(&[(occ0, 42), (occ1, 7)])
        .unwrap();

    let loaded = store.load_system().unwrap();
    let c0 = loaded.episodes[0].neighborhoods[0].occurrences[0].activation_count;
    let c1 = loaded.episodes[0].neighborhoods[0].occurrences[1].activation_count;
    assert_eq!(c0, 42, "first occurrence should have activation_count 42");
    assert_eq!(c1, 7, "second occurrence should have activation_count 7");
}

#[test]
fn test_batch_set_activation_counts_empty() {
    let store = Store::open_in_memory().unwrap();
    // Empty batch should be a no-op
    store.batch_set_activation_counts(&[]).unwrap();
}

#[test]
fn test_increment_activation_nonexistent() {
    let store = Store::open_in_memory().unwrap();
    let result = store.increment_activation(Uuid::new_v4());
    assert!(result.is_err());
}

#[test]
fn test_get_occurrences_by_word() {
    let store = Store::open_in_memory().unwrap();
    let system = make_system();
    store.save_system(&system).unwrap();

    let occs = store.get_occurrences_by_word("hello").unwrap();
    assert_eq!(occs.len(), 1);
    assert_eq!(occs[0].word, "hello");

    let none = store.get_occurrences_by_word("nonexistent").unwrap();
    assert!(none.is_empty());
}

#[test]
fn test_get_neighborhood_ids_by_word() {
    let store = Store::open_in_memory().unwrap();
    let system = make_system();
    store.save_system(&system).unwrap();

    let ids = store.get_neighborhood_ids_by_word("hello").unwrap();
    assert_eq!(ids.len(), 1);
}

#[test]
fn test_save_occurrence_positions() {
    let store = Store::open_in_memory().unwrap();
    let system = make_system();
    store.save_system(&system).unwrap();

    let occ = &system.episodes[0].neighborhoods[0].occurrences[0];
    let new_pos = Quaternion::new(0.5, 0.5, 0.5, 0.5);
    let new_phasor = DaemonPhasor::new(1.23);

    store
        .save_occurrence_positions(&[(occ.id, new_pos, new_phasor)])
        .unwrap();

    let loaded = store.load_system().unwrap();
    let loaded_occ = &loaded.episodes[0].neighborhoods[0].occurrences[0];
    let dist = new_pos.angular_distance(loaded_occ.position);
    assert!(dist < 1e-10, "position not updated: {dist}");
    assert!(
        (loaded_occ.phasor.theta - 1.23).abs() < 1e-10,
        "phasor not updated"
    );
}

#[test]
fn test_metadata() {
    let store = Store::open_in_memory().unwrap();

    assert!(store.get_metadata("foo").unwrap().is_none());

    store.set_metadata("foo", "bar").unwrap();
    assert_eq!(store.get_metadata("foo").unwrap(), Some("bar".to_string()));

    store.set_metadata("foo", "baz").unwrap();
    assert_eq!(store.get_metadata("foo").unwrap(), Some("baz".to_string()));
}

#[test]
fn test_save_overwrites_previous() {
    let store = Store::open_in_memory().unwrap();
    let system = make_system();

    store.save_system(&system).unwrap();
    store.save_system(&system).unwrap();

    let loaded = store.load_system().unwrap();
    assert_eq!(loaded.episodes.len(), 1);
}

#[test]
fn test_load_empty_db() {
    let store = Store::open_in_memory().unwrap();
    let system = store.load_system().unwrap();
    assert_eq!(system.agent_name, "unknown");
    assert!(system.episodes.is_empty());
    assert!(system.conscious_episode.is_conscious);
}

#[test]
fn test_activation_count_preserved() {
    let store = Store::open_in_memory().unwrap();
    let mut system = make_system();

    // Pre-activate conscious occurrences are already at 1
    // Subconscious at 0
    system.episodes[0].neighborhoods[0].occurrences[0].activation_count = 42;

    store.save_system(&system).unwrap();
    let loaded = store.load_system().unwrap();

    assert_eq!(
        loaded.episodes[0].neighborhoods[0].occurrences[0].activation_count,
        42
    );
}

/// Regression guard for the single-JOIN load_system implementation.
/// Builds a system with 500+ occurrences across multiple episodes and
/// neighborhoods, round-trips through SQLite, and asserts structural
/// and numerical equivalence.
#[test]
fn test_load_system_roundtrip_500_occurrences() {
    let store = Store::open_in_memory().unwrap();
    let mut rng = rng();
    let mut sys = DAESystem::new("roundtrip-agent");

    // 10 episodes x 5 neighborhoods x 12 tokens = 600 occurrences
    let words: Vec<String> = (0..12).map(|i| format!("word{i}")).collect();
    for ep_idx in 0..10 {
        let mut ep = Episode::new(&format!("ep-{ep_idx}"));
        for _ in 0..5 {
            let n = Neighborhood::from_tokens(&words, None, "source text", &mut rng);
            ep.add_neighborhood(n);
        }
        sys.add_episode(ep);
    }
    // Add conscious content
    sys.add_to_conscious("conscious roundtrip content", &mut rng);

    // Set varied activation counts to test numeric fidelity
    for (i, ep) in sys.episodes.iter_mut().enumerate() {
        for nbhd in &mut ep.neighborhoods {
            for (j, occ) in nbhd.occurrences.iter_mut().enumerate() {
                occ.activation_count = (i * 100 + j) as u32;
            }
        }
    }

    let total_before: usize = sys
        .episodes
        .iter()
        .chain(std::iter::once(&sys.conscious_episode))
        .map(|e| {
            e.neighborhoods
                .iter()
                .map(|n| n.occurrences.len())
                .sum::<usize>()
        })
        .sum();
    assert!(
        total_before >= 500,
        "precondition: need 500+ occurrences, got {total_before}"
    );

    store.save_system(&sys).unwrap();
    let loaded = store.load_system().unwrap();

    // Structural equivalence
    assert_eq!(loaded.agent_name, "roundtrip-agent");
    assert_eq!(loaded.episodes.len(), sys.episodes.len());
    assert!(loaded.conscious_episode.is_conscious);

    let total_after: usize = loaded
        .episodes
        .iter()
        .chain(std::iter::once(&loaded.conscious_episode))
        .map(|e| {
            e.neighborhoods
                .iter()
                .map(|n| n.occurrences.len())
                .sum::<usize>()
        })
        .sum();
    assert_eq!(total_before, total_after);

    // Per-episode neighborhood count
    for (orig, loaded_ep) in sys.episodes.iter().zip(loaded.episodes.iter()) {
        assert_eq!(orig.neighborhoods.len(), loaded_ep.neighborhoods.len());
        assert_eq!(orig.name, loaded_ep.name);
    }

    // Spot-check activation counts survive roundtrip
    assert_eq!(
        loaded.episodes[3].neighborhoods[2].occurrences[1].activation_count,
        (3 * 100 + 1) as u32,
    );

    // Quaternion precision
    let orig_pos = sys.episodes[0].neighborhoods[0].occurrences[0].position;
    let load_pos = loaded.episodes[0].neighborhoods[0].occurrences[0].position;
    assert!(
        orig_pos.angular_distance(load_pos) < 1e-10,
        "quaternion drift on roundtrip"
    );
}

#[test]
fn test_health_check() {
    let store = Store::open_in_memory().unwrap();
    assert!(store.health_check().is_ok());
}

// --- GC tests ---

/// Permissive retention policy that disables all protection for testing.
fn no_retention() -> crate::config::RetentionPolicy {
    crate::config::RetentionPolicy {
        grace_epochs: 0,
        retention_days: 0,
        min_neighborhoods: 0,
        recency_weight: 0.0,
    }
}

fn make_system_with_activations() -> DAESystem {
    let mut rng = rng();
    let mut sys = DAESystem::new("test-agent");

    let mut ep1 = Episode::new("episode-cold");
    let tokens = to_tokens(&["cold", "unused", "stale"]);
    let n = Neighborhood::from_tokens(&tokens, None, "cold unused stale", &mut rng);
    ep1.add_neighborhood(n);
    sys.add_episode(ep1);

    let mut ep2 = Episode::new("episode-warm");
    let tokens = to_tokens(&["warm", "active"]);
    let mut n = Neighborhood::from_tokens(&tokens, None, "warm active", &mut rng);
    // Activate these occurrences
    for occ in &mut n.occurrences {
        occ.activation_count = 5;
    }
    ep2.add_neighborhood(n);
    sys.add_episode(ep2);

    // Add conscious memory (should never be GC'd)
    sys.add_to_conscious("protected insight", &mut rng);

    sys
}

#[test]
fn test_gc_evicts_cold_occurrences() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system_with_activations();
    store.save_system(&sys).unwrap();

    // Before GC: 3 cold (activation=0) + 2 warm (activation=5) + conscious
    let before = store.occurrence_count().unwrap();
    assert!(before >= 5);

    // Evict occurrences with activation_count <= 0
    let result = store.gc_pass(0, &no_retention()).unwrap();
    assert_eq!(
        result.evicted_occurrences, 3,
        "should evict 3 cold occurrences"
    );

    // After GC: warm + conscious should remain
    let loaded = store.load_system().unwrap();
    assert_eq!(loaded.episodes.len(), 1, "cold episode should be removed");
    assert_eq!(loaded.episodes[0].name, "episode-warm");
    assert!(
        !loaded.conscious_episode.neighborhoods.is_empty(),
        "conscious should survive GC"
    );
}

#[test]
fn test_gc_preserves_conscious() {
    let store = Store::open_in_memory().unwrap();
    let mut rng = rng();
    let mut sys = DAESystem::new("test-agent");

    // Only conscious memory, no subconscious episodes
    sys.add_to_conscious("precious insight", &mut rng);
    store.save_system(&sys).unwrap();

    let result = store.gc_pass(0, &no_retention()).unwrap();
    assert_eq!(
        result.evicted_occurrences, 0,
        "conscious should never be evicted"
    );

    let loaded = store.load_system().unwrap();
    assert!(
        !loaded.conscious_episode.neighborhoods.is_empty(),
        "conscious should survive"
    );
}

#[test]
fn test_gc_removes_empty_episodes() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system_with_activations();
    store.save_system(&sys).unwrap();

    let result = store.gc_pass(0, &no_retention()).unwrap();
    assert_eq!(result.removed_episodes, 1, "episode-cold should be removed");
    assert_eq!(result.removed_neighborhoods, 1);
}

#[test]
fn test_gc_grace_epochs_protects_fresh_data() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system_with_activations();
    store.save_system(&sys).unwrap();

    // Grace window of 100 epochs covers everything - nothing should be evicted
    let policy = crate::config::RetentionPolicy {
        grace_epochs: 100,
        retention_days: 0,
        min_neighborhoods: 0,
        recency_weight: 0.0,
    };
    let result = store.gc_pass(0, &policy).unwrap();
    assert_eq!(
        result.evicted_occurrences, 0,
        "grace window should protect all neighborhoods"
    );
}

#[test]
fn test_gc_min_neighborhoods_prevents_gc() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system_with_activations();
    store.save_system(&sys).unwrap();

    // min_neighborhoods higher than what exists - GC should be skipped
    let policy = crate::config::RetentionPolicy {
        grace_epochs: 0,
        retention_days: 0,
        min_neighborhoods: 1000,
        recency_weight: 0.0,
    };
    let result = store.gc_pass(0, &policy).unwrap();
    assert_eq!(
        result.evicted_occurrences, 0,
        "min_neighborhoods floor should prevent GC"
    );
}

#[test]
fn test_activation_distribution() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system_with_activations();
    store.save_system(&sys).unwrap();

    let stats = store.activation_distribution().unwrap();
    assert!(stats.total >= 5);
    assert!(stats.zero_activation >= 3); // cold occurrences
    assert_eq!(stats.max_activation, 5); // warm occurrences
    assert!(stats.mean_activation > 0.0);
}

#[test]
fn test_gc_noop_when_empty() {
    let store = Store::open_in_memory().unwrap();
    // No data saved - empty DB
    let result = store.gc_pass(0, &no_retention()).unwrap();
    assert_eq!(result.evicted_occurrences, 0);
    assert_eq!(result.removed_episodes, 0);
}

// --- Inspection query tests ---

#[test]
fn test_list_episodes() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let episodes = store.list_episodes().unwrap();
    // 1 subconscious + 1 conscious
    assert_eq!(episodes.len(), 2);

    let conscious: Vec<_> = episodes.iter().filter(|e| e.is_conscious).collect();
    assert_eq!(conscious.len(), 1);
    assert!(conscious[0].occurrence_count > 0);

    let sub: Vec<_> = episodes.iter().filter(|e| !e.is_conscious).collect();
    assert_eq!(sub.len(), 1);
    assert_eq!(sub[0].name, "episode-1");
    assert_eq!(sub[0].neighborhood_count, 1);
    assert_eq!(sub[0].occurrence_count, 3);
}

#[test]
fn test_list_conscious_neighborhoods() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let conscious = store.list_conscious_neighborhoods().unwrap();
    assert_eq!(conscious.len(), 1);
    assert_eq!(conscious[0].source_text, "conscious thought");
    assert!(conscious[0].occurrence_count > 0);
}

#[test]
fn test_list_neighborhoods() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let all = store.list_neighborhoods().unwrap();
    // 1 subconscious + 1 conscious neighborhood
    assert_eq!(all.len(), 2);
}

#[test]
fn test_top_words() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system_with_activations();
    store.save_system(&sys).unwrap();

    let top = store.top_words(3).unwrap();
    assert!(!top.is_empty());
    // "warm" and "active" have activation=5 each, should be at top
    let first_activation = top[0].1;
    assert!(first_activation >= 5);
}

#[test]
fn test_unique_word_count() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let count = store.unique_word_count().unwrap();
    // "hello", "world", "test" + conscious words
    assert!(count >= 3);
}

#[test]
fn test_list_episodes_empty() {
    let store = Store::open_in_memory().unwrap();
    let episodes = store.list_episodes().unwrap();
    assert!(episodes.is_empty());
}

#[test]
fn test_list_conscious_empty() {
    let store = Store::open_in_memory().unwrap();
    let conscious = store.list_conscious_neighborhoods().unwrap();
    assert!(conscious.is_empty());
}

#[test]
fn test_forget_episode() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let episodes = store.list_episodes().unwrap();
    // 2 episodes: 1 subconscious + 1 conscious
    assert_eq!(episodes.len(), 2);

    let sub_ep = episodes.iter().find(|e| !e.is_conscious).unwrap();
    let before = store.occurrence_count().unwrap();
    let removed = store.forget_episode(&sub_ep.id).unwrap();
    assert!(removed > 0);
    assert_eq!(store.occurrence_count().unwrap(), before - removed);

    // Only conscious episode should remain
    let after = store.list_episodes().unwrap();
    assert_eq!(after.len(), 1);
    assert!(after[0].is_conscious);
}

#[test]
fn test_forget_episode_not_found() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let removed = store
        .forget_episode("00000000-0000-0000-0000-000000000000")
        .unwrap();
    assert_eq!(removed, 0);
}

#[test]
fn test_forget_conscious() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let conscious = store.list_conscious_neighborhoods().unwrap();
    assert!(!conscious.is_empty());

    let removed = store.forget_conscious(&conscious[0].id).unwrap();
    assert!(removed > 0);

    let after = store.list_conscious_neighborhoods().unwrap();
    assert!(after.is_empty());
}

#[test]
fn test_forget_conscious_rejects_subconscious() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let _episodes = store.list_episodes().unwrap();
    // Get a neighborhood from a subconscious episode
    let neighborhoods = store.list_neighborhoods().unwrap();
    let sub_nbhd = neighborhoods
        .iter()
        .find(|n| n.episode_name != "conscious")
        .unwrap();

    let result = store.forget_conscious(&sub_nbhd.id);
    assert!(result.is_err());
}

#[test]
fn test_forget_term() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let before = store.occurrence_count().unwrap();
    let (removed_occs, _, _) = store.forget_term("hello").unwrap();
    assert!(removed_occs > 0);
    assert!(store.occurrence_count().unwrap() < before);
}

#[test]
fn test_forget_term_not_found() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let (removed, _, _) = store.forget_term("nonexistent").unwrap();
    assert_eq!(removed, 0);
}

#[test]
fn test_drain_buffer_idempotent() {
    let store = Store::open_in_memory().unwrap();
    store.append_buffer("hello", "world").unwrap();
    store.append_buffer("foo", "bar").unwrap();

    let first = store.drain_buffer().unwrap();
    assert_eq!(first.len(), 2);
    assert_eq!(first[0], ("hello".to_string(), "world".to_string()));
    assert_eq!(first[1], ("foo".to_string(), "bar".to_string()));

    // Second drain returns empty: rows were deleted atomically
    let second = store.drain_buffer().unwrap();
    assert!(second.is_empty(), "second drain should return empty");
}

// --- Tests for ALP-1645: 7 untested store methods ---

#[test]
fn test_open_file_based() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("test.db");

    // First open: creates DB and schema
    {
        let store = Store::open(&db_path).unwrap();
        let sys = make_system();
        store.save_system(&sys).unwrap();
    }
    // File should exist
    assert!(db_path.exists(), "DB file should be created on disk");

    // Re-open: reads back saved data
    {
        let store = Store::open(&db_path).unwrap();
        let loaded = store.load_system().unwrap();
        assert_eq!(loaded.agent_name, "test-agent");
        assert_eq!(loaded.episodes.len(), 1);
        assert_eq!(loaded.episodes[0].neighborhoods[0].occurrences.len(), 3);
    }
}

#[test]
fn test_checkpoint_truncate() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    // checkpoint_truncate should succeed without error
    store.checkpoint_truncate().unwrap();

    // Data should still be intact after checkpoint
    let loaded = store.load_system().unwrap();
    assert_eq!(loaded.agent_name, "test-agent");
    assert_eq!(loaded.episodes.len(), 1);
}

#[test]
fn test_save_episode_incremental() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    // Verify baseline: 1 subconscious episode
    let loaded = store.load_system().unwrap();
    assert_eq!(loaded.episodes.len(), 1);

    // Incrementally add a second episode
    let mut rng = rng();
    let mut ep2 = Episode::new("episode-2");
    let tokens = to_tokens(&["extra", "data"]);
    let n = Neighborhood::from_tokens(&tokens, None, "extra data", &mut rng);
    ep2.add_neighborhood(n);
    store.save_episode(&ep2).unwrap();

    // Reload and verify both episodes present
    let reloaded = store.load_system().unwrap();
    assert_eq!(reloaded.episodes.len(), 2);
    let names: Vec<&str> = reloaded.episodes.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"episode-1"));
    assert!(names.contains(&"episode-2"));
}

#[test]
fn test_save_neighborhood_incremental() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    // Baseline: conscious episode has 1 neighborhood
    let loaded = store.load_system().unwrap();
    let conscious_occs_before: usize = loaded
        .conscious_episode
        .neighborhoods
        .iter()
        .map(|n| n.occurrences.len())
        .sum();
    assert!(conscious_occs_before > 0);

    // Add a new neighborhood to the conscious episode
    let mut rng = rng();
    let tokens = to_tokens(&["new", "insight"]);
    let nbhd = Neighborhood::from_tokens(&tokens, None, "new insight", &mut rng);
    store
        .save_neighborhood(&loaded.conscious_episode, &nbhd)
        .unwrap();

    // Reload and verify occurrence count grew
    let reloaded = store.load_system().unwrap();
    let conscious_occs_after: usize = reloaded
        .conscious_episode
        .neighborhoods
        .iter()
        .map(|n| n.occurrences.len())
        .sum();
    assert!(
        conscious_occs_after > conscious_occs_before,
        "occurrence count should grow: {conscious_occs_before} -> {conscious_occs_after}"
    );
    assert_eq!(reloaded.conscious_episode.neighborhoods.len(), 2);
}

#[test]
fn test_mark_superseded() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system();
    store.save_system(&sys).unwrap();

    let old_id = sys.episodes[0].neighborhoods[0].id;
    let new_id = Uuid::new_v4();

    // Before: superseded_by is None
    let loaded = store.load_system().unwrap();
    assert!(loaded.episodes[0].neighborhoods[0].superseded_by.is_none());

    store.mark_superseded(old_id, new_id).unwrap();

    // After: superseded_by points to new_id
    let reloaded = store.load_system().unwrap();
    assert_eq!(
        reloaded.episodes[0].neighborhoods[0].superseded_by,
        Some(new_id)
    );
}

#[test]
fn test_mark_superseded_not_found() {
    let store = Store::open_in_memory().unwrap();
    let result = store.mark_superseded(Uuid::new_v4(), Uuid::new_v4());
    assert!(result.is_err());
}

#[test]
fn test_gc_eligible_count() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system_with_activations();
    store.save_system(&sys).unwrap();

    // Floor 0: only activation_count == 0 are eligible (3 cold occurrences)
    let count = store.gc_eligible_count(0).unwrap();
    assert_eq!(count, 3, "3 cold occurrences with activation_count=0");

    // Floor 4: cold (0) + warm are still <= 4? No, warm has activation=5
    // So only the 3 cold are eligible at floor=4
    let count4 = store.gc_eligible_count(4).unwrap();
    assert_eq!(count4, 3, "warm (activation=5) not eligible at floor=4");

    // Floor 5: all subconscious eligible (3 cold + 2 warm with activation=5)
    let count5 = store.gc_eligible_count(5).unwrap();
    assert_eq!(
        count5, 5,
        "all subconscious occurrences eligible at floor=5"
    );

    // Verify this is a dry run: data unchanged
    let loaded = store.load_system().unwrap();
    let total: usize = loaded
        .episodes
        .iter()
        .map(|e| {
            e.neighborhoods
                .iter()
                .map(|n| n.occurrences.len())
                .sum::<usize>()
        })
        .sum();
    assert_eq!(
        total, 5,
        "no occurrences should be mutated by gc_eligible_count"
    );
}

#[test]
fn test_gc_to_target_size() {
    let store = Store::open_in_memory().unwrap();
    let sys = make_system_with_activations();
    store.save_system(&sys).unwrap();

    // Set target to 0 bytes, forcing maximum eviction
    let result = store.gc_to_target_size(0, &no_retention()).unwrap();
    assert!(
        result.evicted_occurrences > 0,
        "should evict some occurrences"
    );

    // Conscious memory must survive
    let loaded = store.load_system().unwrap();
    assert!(
        !loaded.conscious_episode.neighborhoods.is_empty(),
        "conscious should survive aggressive GC"
    );
}

/// Regression test for ALP-1239: drain_buffer atomicity.
///
/// The pre-fix implementation performed SELECT then DELETE without a
/// transaction, creating a crash window where rows could be deleted from
/// the database but never returned to the caller. The fix wraps both
/// operations in a single transaction so they commit atomically.
///
/// This test verifies the data integrity invariant: every buffered row
/// is returned exactly once across interleaved append/drain cycles, with
/// buffer_count staying consistent at each step. The pre-fix code could
/// violate this invariant under concurrent access or crash recovery.
#[test]
fn test_drain_buffer_atomicity_no_lost_rows() {
    let store = Store::open_in_memory().unwrap();

    // Phase 1: buffer 5 entries, drain, verify all returned and count is 0
    for i in 0..5 {
        store
            .append_buffer(&format!("user_{i}"), &format!("asst_{i}"))
            .unwrap();
    }
    assert_eq!(store.buffer_count().unwrap(), 5);

    let drained = store.drain_buffer().unwrap();
    assert_eq!(drained.len(), 5, "all 5 rows must be returned");
    assert_eq!(
        store.buffer_count().unwrap(),
        0,
        "buffer must be empty after drain"
    );

    // Verify exact content and ordering
    for (i, (user, asst)) in drained.iter().enumerate() {
        assert_eq!(user, &format!("user_{i}"));
        assert_eq!(asst, &format!("asst_{i}"));
    }

    // Phase 2: interleave appends and drains
    store.append_buffer("a", "1").unwrap();
    store.append_buffer("b", "2").unwrap();
    assert_eq!(store.buffer_count().unwrap(), 2);

    let batch1 = store.drain_buffer().unwrap();
    assert_eq!(batch1.len(), 2);
    assert_eq!(store.buffer_count().unwrap(), 0);

    // Drain on empty is safe
    let empty = store.drain_buffer().unwrap();
    assert!(empty.is_empty());
    assert_eq!(store.buffer_count().unwrap(), 0);

    // Phase 3: append after drain, verify no ghost rows from phase 1 or 2
    store.append_buffer("c", "3").unwrap();
    assert_eq!(store.buffer_count().unwrap(), 1);

    let batch2 = store.drain_buffer().unwrap();
    assert_eq!(batch2.len(), 1, "only the newly appended row should appear");
    assert_eq!(batch2[0], ("c".to_string(), "3".to_string()));
    assert_eq!(store.buffer_count().unwrap(), 0);
}
