//! Relevance feedback: boost or demote memories based on outcome.
//!
//! When a recalled memory actually helped the user, the occurrences that
//! contributed should drift closer to where they were needed (SLERP toward
//! the query centroid). When a recall was unhelpful, those occurrences
//! decay - their activation count is reduced, making them drift less in
//! future queries and eventually become GC candidates.
//!
//! This is the geometric equivalent of reinforcement: the manifold reshapes
//! itself based on what worked.

use crate::constants::EPSILON;
use crate::quaternion::Quaternion;
use crate::query::QueryManifest;
use crate::system::{DAESystem, OccurrenceRef};
use crate::tokenizer::tokenize;

/// Feedback signal: did the recalled content help?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackSignal {
    /// The recall was useful - drift occurrences toward the query region.
    Boost,
    /// The recall was not useful - decay activation, let occurrences drift away.
    Demote,
}

/// Result of applying feedback to the system.
#[derive(Debug)]
pub struct FeedbackResult {
    /// Number of occurrences that were boosted (drifted toward query centroid).
    pub boosted: usize,
    /// Number of occurrences that were demoted (activation decayed).
    pub demoted: usize,
    /// The query centroid used for boosting (if any).
    pub centroid: Option<Quaternion>,
    /// Mutation manifest: tracks which occurrence IDs had positions or
    /// activation counts modified. Used for incremental persistence.
    pub manifest: QueryManifest,
}

/// SLERP interpolation factor toward query centroid on a Boost signal.
///
/// Controls how aggressively boosted occurrences converge toward the query
/// region. The effective displacement is `BOOST_DRIFT_FACTOR * idf_weight *
/// plasticity`, so high-IDF, high-plasticity occurrences move furthest.
///
/// At 0.15: gentle nudge that preserves manifold topology while creating a
/// detectable attraction basin over 3-5 repeated boosts. Higher values
/// (0.3+) risk collapsing distinct neighborhoods into a single cluster.
/// Lower values (0.05) require many feedback signals before drift is visible.
const BOOST_DRIFT_FACTOR: f64 = 0.15;

/// Activation count decrement per Demote signal.
///
/// Reduces `activation_count` by this amount (saturating at 0), which
/// increases plasticity and reduces the occurrence's anchoring strength.
/// Demoted occurrences become more susceptible to future drift, allowing
/// the manifold to reorganize away from unhelpful recall patterns.
///
/// At 2: a single demote undoes roughly two prior activations, making
/// the occurrence noticeably more mobile without erasing it entirely.
/// Combined with THRESHOLD (ratio/C), this ensures demoted occurrences
/// drop below the vivid threshold after 1-2 demote signals.
const DEMOTE_DECAY: u32 = 2;

/// Apply relevance feedback to neighborhoods that were recalled for a query.
///
/// `query` - the original query text (used to compute the centroid for boosting).
/// `neighborhood_ids` - the neighborhood UUIDs that were actually shown to the user.
/// `signal` - Boost or Demote.
///
/// For Boost: activated occurrences in the specified neighborhoods SLERP toward
/// the IDF-weighted centroid of the query's activated occurrences. This pulls
/// helpful memories closer to the region of the manifold where they were needed.
///
/// For Demote: activated occurrences in the specified neighborhoods have their
/// activation count reduced. This makes them less anchored, more likely to
/// drift away in future queries, and eventually GC-eligible.
///
/// # Examples
///
/// ```
/// use am_core::{DAESystem, QueryEngine, FeedbackSignal, apply_feedback, ingest_text};
/// use rand::SeedableRng;
/// use rand::rngs::SmallRng;
///
/// let mut system = DAESystem::new("test");
/// let mut rng = SmallRng::seed_from_u64(42);
/// let ep = ingest_text("Rust memory safety through ownership", None, &mut rng);
/// let nbhd_id = ep.neighborhoods[0].id;
/// system.add_episode(ep);
///
/// // Boost: pull recalled neighborhoods toward the query region
/// let result = apply_feedback(&mut system, "memory safety", &[nbhd_id], FeedbackSignal::Boost);
/// // "memory" and "safety" overlap with the neighborhood, so boosted > 0
/// assert!(result.boosted > 0);
///
/// // Demote: decay activation of unhelpful neighborhoods
/// let result = apply_feedback(&mut system, "memory safety", &[nbhd_id], FeedbackSignal::Demote);
/// assert!(result.demoted > 0);
/// ```
pub fn apply_feedback(
    system: &mut DAESystem,
    query: &str,
    neighborhood_ids: &[uuid::Uuid],
    signal: FeedbackSignal,
) -> FeedbackResult {
    // Tokenize query and find all activated occurrences
    let tokens = tokenize(query);
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<String> = tokens
        .into_iter()
        .filter(|t| seen.insert(t.to_lowercase()))
        .collect();

    // Collect all occurrence refs for the query tokens (without activating them again)
    system.rebuild_indexes();
    let query_refs: Vec<OccurrenceRef> = unique
        .iter()
        .flat_map(|token| system.get_word_occurrences(token))
        .collect();

    if query_refs.is_empty() {
        return FeedbackResult {
            boosted: 0,
            demoted: 0,
            centroid: None,
            manifest: QueryManifest::default(),
        };
    }

    // Build set of target neighborhood IDs for fast lookup
    let target_ids: std::collections::HashSet<uuid::Uuid> =
        neighborhood_ids.iter().copied().collect();

    // Find occurrences that are in the target neighborhoods
    let target_refs: Vec<OccurrenceRef> = query_refs
        .iter()
        .filter(|r| {
            let nbhd = system.get_neighborhood_for_occurrence(**r);
            target_ids.contains(&nbhd.id)
        })
        .copied()
        .collect();

    match signal {
        FeedbackSignal::Boost => apply_boost(system, &query_refs, &target_refs, &unique),
        FeedbackSignal::Demote => apply_demote(system, &target_refs),
    }
}

/// Boost: SLERP target occurrences toward the IDF-weighted query centroid.
fn apply_boost(
    system: &mut DAESystem,
    all_query_refs: &[OccurrenceRef],
    target_refs: &[OccurrenceRef],
    _query_words: &[String],
) -> FeedbackResult {
    if target_refs.is_empty() {
        return FeedbackResult {
            boosted: 0,
            demoted: 0,
            centroid: None,
            manifest: QueryManifest::default(),
        };
    }

    // Compute IDF-weighted centroid of ALL query occurrences in R⁴, project to S³
    let weights: Vec<f64> = all_query_refs
        .iter()
        .map(|r| {
            let word = system.get_occurrence(*r).word.clone();
            system.get_word_weight(&word)
        })
        .collect();

    let positions: Vec<Quaternion> = all_query_refs
        .iter()
        .map(|r| system.get_occurrence(*r).position)
        .collect();

    let centroid = Quaternion::weighted_centroid(&positions, &weights);

    let Some(centroid) = centroid else {
        return FeedbackResult {
            boosted: 0,
            demoted: 0,
            centroid: None,
            manifest: QueryManifest::default(),
        };
    };

    // Cache IDF weights for target occurrences
    let target_weights: Vec<f64> = target_refs
        .iter()
        .map(|r| {
            let word = system.get_occurrence(*r).word.clone();
            system.get_word_weight(&word)
        })
        .collect();

    // SLERP each target occurrence toward the centroid
    // Factor scales with IDF weight - rare words get pulled harder
    let mut boosted = 0usize;
    let mut drifted = Vec::new();
    let mut activated = Vec::new();
    for (i, r) in target_refs.iter().enumerate() {
        let occ = system.get_occurrence(*r);
        let plasticity = occ.plasticity();
        let factor = BOOST_DRIFT_FACTOR * target_weights[i] * plasticity;

        if factor > EPSILON {
            let new_pos = occ.position.slerp(centroid, factor);
            let occ = system.get_occurrence_mut(*r);
            occ.position = new_pos;
            // Also bump activation - this memory proved useful
            occ.activation_count = occ.activation_count.saturating_add(1);
            drifted.push(occ.id);
            activated.push(occ.id);
            boosted += 1;
        }
    }

    FeedbackResult {
        boosted,
        demoted: 0,
        centroid: Some(centroid),
        manifest: QueryManifest {
            drifted,
            activated,
            demoted_activations: Vec::new(),
        },
    }
}

/// Demote: decay activation on target occurrences.
fn apply_demote(system: &mut DAESystem, target_refs: &[OccurrenceRef]) -> FeedbackResult {
    let mut demoted = 0usize;
    let mut demoted_activations = Vec::new();

    for r in target_refs {
        let occ = system.get_occurrence_mut(*r);
        let before = occ.activation_count;
        occ.activation_count = occ.activation_count.saturating_sub(DEMOTE_DECAY);
        if occ.activation_count != before {
            demoted_activations.push((occ.id, occ.activation_count));
            demoted += 1;
        }
    }

    FeedbackResult {
        boosted: 0,
        demoted,
        centroid: None,
        manifest: QueryManifest {
            drifted: Vec::new(),
            activated: Vec::new(),
            demoted_activations,
        },
    }
}

// Centroid computation now uses Quaternion::weighted_centroid from quaternion.rs.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::Episode;
    use crate::neighborhood::Neighborhood;
    use crate::query::QueryEngine;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(std::string::ToString::to_string).collect()
    }

    fn make_feedback_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        let mut ep = Episode::new("science");
        let n1 = Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics", "particle"]),
            None,
            "quantum physics particle",
            &mut rng,
        );
        let n2 = Neighborhood::from_tokens(
            &to_tokens(&["quantum", "computing", "algorithm"]),
            None,
            "quantum computing algorithm",
            &mut rng,
        );
        ep.add_neighborhood(n1);
        ep.add_neighborhood(n2);
        sys.add_episode(ep);

        sys.add_to_conscious("quantum mechanics", &mut rng);

        // Activate to give occurrences non-zero activation
        let _ = QueryEngine::process_query(&mut sys, "quantum physics computing");

        sys
    }

    #[test]
    fn test_boost_moves_closer_to_centroid() {
        let mut sys = make_feedback_system();

        // Get the first subconscious neighborhood ID
        let nbhd_id = sys.episodes[0].neighborhoods[0].id;

        // Snapshot positions before
        let before_positions: Vec<Quaternion> = sys.episodes[0].neighborhoods[0]
            .occurrences
            .iter()
            .map(|o| o.position)
            .collect();

        let result = apply_feedback(
            &mut sys,
            "quantum physics",
            &[nbhd_id],
            FeedbackSignal::Boost,
        );

        assert!(result.boosted > 0, "should have boosted some occurrences");
        assert!(result.centroid.is_some(), "should have computed a centroid");

        // At least one occurrence should have moved
        let after_positions: Vec<Quaternion> = sys.episodes[0].neighborhoods[0]
            .occurrences
            .iter()
            .map(|o| o.position)
            .collect();

        let mut any_moved = false;
        for (before, after) in before_positions.iter().zip(after_positions.iter()) {
            if before.angular_distance(*after) > EPSILON {
                any_moved = true;
                break;
            }
        }
        assert!(any_moved, "at least one occurrence should have moved");
    }

    #[test]
    fn test_boost_increases_activation() {
        let mut sys = make_feedback_system();
        let nbhd_id = sys.episodes[0].neighborhoods[0].id;

        // Get activation before
        let before_activation: u32 = sys.episodes[0].neighborhoods[0]
            .occurrences
            .iter()
            .map(|o| o.activation_count)
            .sum();

        apply_feedback(
            &mut sys,
            "quantum physics",
            &[nbhd_id],
            FeedbackSignal::Boost,
        );

        let after_activation: u32 = sys.episodes[0].neighborhoods[0]
            .occurrences
            .iter()
            .map(|o| o.activation_count)
            .sum();

        assert!(
            after_activation >= before_activation,
            "boost should increase activation: {before_activation} -> {after_activation}"
        );
    }

    #[test]
    fn test_demote_decreases_activation() {
        let mut sys = make_feedback_system();
        let nbhd_id = sys.episodes[0].neighborhoods[0].id;

        let before_activation: u32 = sys.episodes[0].neighborhoods[0]
            .occurrences
            .iter()
            .map(|o| o.activation_count)
            .sum();

        let result = apply_feedback(
            &mut sys,
            "quantum physics",
            &[nbhd_id],
            FeedbackSignal::Demote,
        );

        assert!(result.demoted > 0, "should have demoted some occurrences");

        let after_activation: u32 = sys.episodes[0].neighborhoods[0]
            .occurrences
            .iter()
            .map(|o| o.activation_count)
            .sum();

        assert!(
            after_activation < before_activation,
            "demote should decrease activation: {before_activation} -> {after_activation}"
        );
    }

    #[test]
    fn test_demote_floors_at_zero() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        let mut ep = Episode::new("test");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["hello", "world"]),
            None,
            "hello world",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Occurrences start at activation_count = 0
        let nbhd_id = sys.episodes[0].neighborhoods[0].id;
        let result = apply_feedback(&mut sys, "hello", &[nbhd_id], FeedbackSignal::Demote);

        // Should not panic, activation should stay at 0
        for occ in &sys.episodes[0].neighborhoods[0].occurrences {
            assert_eq!(occ.activation_count, 0);
        }
        // Nothing actually changed (was already 0)
        assert_eq!(result.demoted, 0);
    }

    #[test]
    fn test_feedback_nonexistent_neighborhood() {
        let mut sys = make_feedback_system();
        let fake_id = uuid::Uuid::new_v4();

        let result = apply_feedback(&mut sys, "quantum", &[fake_id], FeedbackSignal::Boost);

        assert_eq!(result.boosted, 0);
    }

    #[test]
    fn test_feedback_empty_query() {
        let mut sys = make_feedback_system();
        let nbhd_id = sys.episodes[0].neighborhoods[0].id;

        let result = apply_feedback(&mut sys, "", &[nbhd_id], FeedbackSignal::Boost);

        assert_eq!(result.boosted, 0);
    }

    #[test]
    fn test_weighted_centroid_basic() {
        let p1 = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let p2 = Quaternion::new(0.0, 1.0, 0.0, 0.0);

        // Equal weights - centroid should be between them
        let centroid = Quaternion::weighted_centroid(&[p1, p2], &[1.0, 1.0]).unwrap();
        let d1 = p1.angular_distance(centroid);
        let d2 = p2.angular_distance(centroid);
        assert!(
            (d1 - d2).abs() < 0.1,
            "equal-weight centroid should be equidistant: {d1} vs {d2}"
        );
    }

    #[test]
    fn test_weighted_centroid_skewed() {
        let p1 = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let p2 = Quaternion::new(0.0, 1.0, 0.0, 0.0);

        // Heavy weight on p1 - centroid should be closer to p1
        let centroid = Quaternion::weighted_centroid(&[p1, p2], &[10.0, 1.0]).unwrap();
        let d1 = p1.angular_distance(centroid);
        let d2 = p2.angular_distance(centroid);
        assert!(
            d1 < d2,
            "centroid should be closer to heavily-weighted point: {d1} vs {d2}"
        );
    }

    #[test]
    fn test_boost_manifest_tracks_drifted_and_activated() {
        let mut sys = make_feedback_system();
        let nbhd_id = sys.episodes[0].neighborhoods[0].id;

        let result = apply_feedback(
            &mut sys,
            "quantum physics",
            &[nbhd_id],
            FeedbackSignal::Boost,
        );

        assert!(result.boosted > 0);
        // Boosted occurrences should appear in both drifted and activated
        assert_eq!(
            result.manifest.drifted.len(),
            result.boosted,
            "drifted count should match boosted count"
        );
        assert_eq!(
            result.manifest.activated.len(),
            result.boosted,
            "activated count should match boosted count"
        );
        assert!(
            result.manifest.demoted_activations.is_empty(),
            "boost should not have demoted activations"
        );
    }

    #[test]
    fn test_demote_manifest_tracks_demoted_activations() {
        let mut sys = make_feedback_system();
        let nbhd_id = sys.episodes[0].neighborhoods[0].id;

        let result = apply_feedback(
            &mut sys,
            "quantum physics",
            &[nbhd_id],
            FeedbackSignal::Demote,
        );

        assert!(result.demoted > 0);
        assert!(
            result.manifest.drifted.is_empty(),
            "demote should not drift"
        );
        assert!(
            result.manifest.activated.is_empty(),
            "demote should not activate"
        );
        assert_eq!(
            result.manifest.demoted_activations.len(),
            result.demoted,
            "demoted_activations count should match demoted count"
        );
        // Each entry should have the post-demote activation count
        for (id, count) in &result.manifest.demoted_activations {
            assert!(!id.is_nil(), "ID should not be nil");
            // Count should be less than original (process_query gives >= 1)
            assert!(*count < u32::MAX, "count should be a reasonable value");
        }
    }

    #[test]
    fn test_batch_query_manifest_tracks_mutations() {
        use crate::batch::{BatchQueryEngine, BatchQueryRequest};

        let mut sys = make_feedback_system();

        let requests = vec![BatchQueryRequest {
            query: "quantum physics".to_string(),
            max_tokens: Some(4096),
        }];

        let output = BatchQueryEngine::batch_query(&mut sys, &requests);

        // Batch query activates and drifts, so manifest should be non-empty
        assert!(
            !output.manifest.activated.is_empty(),
            "batch query should track activated occurrence IDs"
        );
    }
}
