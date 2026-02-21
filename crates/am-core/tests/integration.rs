//! Integration tests exercising the full DAE pipeline:
//! ingest → query → surface → compose, across crate boundaries.

use am_core::{
    DAESystem, QueryEngine, compose_context, compute_surface, export_json, extract_salient,
    import_json, ingest_text,
};
use rand::SeedableRng;
use rand::rngs::SmallRng;

fn rng() -> SmallRng {
    SmallRng::seed_from_u64(42)
}

const SCIENCE_TEXT: &str = "\
Quantum mechanics describes the behavior of particles at the subatomic scale. \
Wave functions collapse upon measurement, producing definite outcomes. \
The uncertainty principle limits simultaneous knowledge of position and momentum. \
Entangled particles share correlations that persist across vast distances. \
Superposition allows particles to exist in multiple states until observed. \
Decoherence explains why quantum effects vanish at macroscopic scales.";

const COOKING_TEXT: &str = "\
The Maillard reaction creates complex flavors when proteins and sugars are heated. \
Caramelization occurs at higher temperatures, breaking down pure sugars. \
Emulsification binds oil and water using lecithin from egg yolks. \
Fermentation transforms sugars into alcohol and carbon dioxide through yeast activity. \
Blanching preserves color and texture by briefly immersing vegetables in boiling water. \
Deglazing lifts fond from the pan surface using wine or stock.";

const HISTORY_TEXT: &str = "\
The Renaissance began in Florence during the fourteenth century. \
Artists like Leonardo da Vinci and Michelangelo transformed visual culture. \
The printing press invented by Gutenberg revolutionized the spread of knowledge. \
Maritime exploration expanded European contact with distant civilizations. \
The scientific revolution challenged centuries of received wisdom. \
Enlightenment thinkers championed reason and individual liberty.";

/// Test 1: Ingest text, query with overlapping terms, verify surfaced content.
#[test]
fn ingest_query_roundtrip() {
    let mut rng = rng();
    let mut system = DAESystem::new("test");

    let episode = ingest_text(SCIENCE_TEXT, Some("science"), &mut rng);
    assert!(
        episode.neighborhoods.len() >= 2,
        "should chunk into multiple neighborhoods"
    );
    let occ_count: usize = episode
        .neighborhoods
        .iter()
        .map(|n| n.occurrences.len())
        .sum();
    assert!(occ_count > 0);
    system.add_episode(episode);

    // Add conscious content with overlapping terms
    system.add_to_conscious("quantum particles measurement observation", &mut rng);

    let query_result = QueryEngine::process_query(&mut system, "quantum particles wave function");
    let surface = compute_surface(&system, &query_result);
    let composed = compose_context(
        &mut system,
        &surface,
        &query_result,
        &query_result.interference,
        None,
    );

    // Should have non-empty context since query terms overlap with ingested text
    assert!(
        !composed.context.is_empty(),
        "composed context should not be empty after ingesting relevant text"
    );
    assert!(
        composed.context.contains("CONSCIOUS RECALL:"),
        "should have conscious recall since 'quantum' overlaps"
    );
    // At least some metric should be non-zero
    assert!(
        composed.metrics.conscious > 0 || composed.metrics.subconscious > 0,
        "should have at least one recall type"
    );
}

/// Test 2: Conscious memory flow — mark salient, verify it appears in composed context.
#[test]
fn conscious_memory_flow() {
    let mut rng = rng();
    let mut system = DAESystem::new("test");

    // Ingest subconscious content
    let episode = ingest_text(SCIENCE_TEXT, Some("science"), &mut rng);
    system.add_episode(episode);

    // Mark specific text as salient (conscious)
    let count = extract_salient(
        &mut system,
        "Regular text <salient>quantum entanglement enables teleportation of quantum states</salient> more text",
        &mut rng,
    );
    assert_eq!(count, 1);
    assert!(
        !system.conscious_episode.neighborhoods.is_empty(),
        "conscious episode should have neighborhoods after salient extraction"
    );

    // Query with terms from the salient text
    let query_result =
        QueryEngine::process_query(&mut system, "quantum entanglement teleportation");
    let surface = compute_surface(&system, &query_result);
    let composed = compose_context(
        &mut system,
        &surface,
        &query_result,
        &query_result.interference,
        None,
    );

    assert!(
        composed.context.contains("CONSCIOUS RECALL:"),
        "conscious recall should appear when querying salient terms"
    );
    assert!(
        composed.metrics.conscious > 0,
        "conscious metric should be non-zero"
    );
}

/// Test 3: Multi-episode recall — ingest 3 documents, query spanning multiple.
#[test]
fn multi_episode_recall() {
    let mut rng = rng();
    let mut system = DAESystem::new("test");

    system.add_episode(ingest_text(SCIENCE_TEXT, Some("science"), &mut rng));
    system.add_episode(ingest_text(COOKING_TEXT, Some("cooking"), &mut rng));
    system.add_episode(ingest_text(HISTORY_TEXT, Some("history"), &mut rng));

    assert_eq!(system.episodes.len(), 3);
    assert!(system.n() > 50, "should have substantial occurrence count");

    // Query with terms spanning science + cooking
    system.add_to_conscious("particles and sugars react under heat", &mut rng);
    let query_result =
        QueryEngine::process_query(&mut system, "particles sugars temperature reaction");
    let surface = compute_surface(&system, &query_result);
    let composed = compose_context(
        &mut system,
        &surface,
        &query_result,
        &query_result.interference,
        None,
    );

    assert!(
        !composed.context.is_empty(),
        "multi-episode query should produce results"
    );
    // Should have subconscious recall from the ingested episodes
    assert!(
        composed.metrics.subconscious > 0 || composed.metrics.conscious > 0,
        "should recall content from at least one episode"
    );
}

/// Test 4: Drift mechanics — verify occurrences actually move after query.
#[test]
fn drift_moves_occurrences() {
    let mut rng = rng();
    let mut system = DAESystem::new("test");

    let episode = ingest_text(SCIENCE_TEXT, Some("science"), &mut rng);
    system.add_episode(episode);

    // Snapshot positions of "quantum" occurrences before query
    let refs_before = system.get_word_occurrences("quantum");
    assert!(
        !refs_before.is_empty(),
        "'quantum' should have occurrences after ingesting science text"
    );
    let positions_before: Vec<_> = refs_before
        .iter()
        .map(|r| system.get_occurrence(*r).position)
        .collect();

    // Process query to trigger drift
    let _ = QueryEngine::process_query(&mut system, "quantum particles wave measurement");

    // Check positions changed
    let refs_after = system.get_word_occurrences("quantum");
    let positions_after: Vec<_> = refs_after
        .iter()
        .map(|r| system.get_occurrence(*r).position)
        .collect();

    assert_eq!(positions_before.len(), positions_after.len());

    let mut any_moved = false;
    for (before, after) in positions_before.iter().zip(positions_after.iter()) {
        if before.angular_distance(*after) > 1e-15 {
            any_moved = true;
            break;
        }
    }
    assert!(
        any_moved,
        "at least one occurrence should have moved after drift"
    );
}

/// Test 5: Serde roundtrip with query — export, import, verify identical results.
#[test]
fn serde_roundtrip_with_query() {
    let mut rng = rng();
    let mut system = DAESystem::new("test");

    let episode = ingest_text(SCIENCE_TEXT, Some("science"), &mut rng);
    system.add_episode(episode);
    system.add_to_conscious("quantum measurement observation", &mut rng);

    // Export to JSON
    let json = export_json(&system).expect("export should succeed");
    assert!(!json.is_empty());

    // Import into a new system
    let mut system2 = import_json(&json).expect("import should succeed");

    // Both systems should have same structure
    assert_eq!(system.n(), system2.n());
    assert_eq!(system.episodes.len(), system2.episodes.len());
    assert_eq!(
        system.conscious_episode.neighborhoods.len(),
        system2.conscious_episode.neighborhoods.len()
    );

    // Run same query on both and compare results
    let query = "quantum particles wave";
    let result1 = QueryEngine::process_query(&mut system, query);
    let result2 = QueryEngine::process_query(&mut system2, query);

    let surface1 = compute_surface(&system, &result1);
    let surface2 = compute_surface(&system2, &result2);

    let composed1 = compose_context(
        &mut system,
        &surface1,
        &result1,
        &result1.interference,
        None,
    );
    let composed2 = compose_context(
        &mut system2,
        &surface2,
        &result2,
        &result2.interference,
        None,
    );

    assert_eq!(
        composed1.context, composed2.context,
        "query results should be identical after serde roundtrip"
    );
    assert_eq!(composed1.metrics.conscious, composed2.metrics.conscious);
    assert_eq!(
        composed1.metrics.subconscious,
        composed2.metrics.subconscious
    );
    assert_eq!(composed1.metrics.novel, composed2.metrics.novel);
}

/// Test 6: Activation counts increase with repeated queries.
#[test]
fn repeated_queries_increase_activation() {
    let mut rng = rng();
    let mut system = DAESystem::new("test");

    let episode = ingest_text(SCIENCE_TEXT, Some("science"), &mut rng);
    system.add_episode(episode);

    // First activation
    let refs = system.get_word_occurrences("quantum");
    assert!(!refs.is_empty());
    let count_before: Vec<u32> = refs
        .iter()
        .map(|r| system.get_occurrence(*r).activation_count)
        .collect();

    // Process query (activates words)
    let _ = QueryEngine::process_query(&mut system, "quantum");

    let refs2 = system.get_word_occurrences("quantum");
    let count_after: Vec<u32> = refs2
        .iter()
        .map(|r| system.get_occurrence(*r).activation_count)
        .collect();

    for (before, after) in count_before.iter().zip(count_after.iter()) {
        assert!(
            after > before,
            "activation count should increase: {} -> {}",
            before,
            after
        );
    }
}

/// Test 7: Empty system produces empty results gracefully.
#[test]
fn empty_system_query() {
    let mut system = DAESystem::new("test");

    let query_result = QueryEngine::process_query(&mut system, "anything at all");
    let surface = compute_surface(&system, &query_result);
    let composed = compose_context(
        &mut system,
        &surface,
        &query_result,
        &query_result.interference,
        None,
    );

    assert!(
        composed.context.is_empty(),
        "empty system should produce empty context"
    );
    assert_eq!(composed.metrics.conscious, 0);
    assert_eq!(composed.metrics.subconscious, 0);
    assert_eq!(composed.metrics.novel, 0);
}
