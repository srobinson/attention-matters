//! Relevance feedback: boost or demote memories based on outcome.
//!
//! When a recalled memory actually helped the user, the occurrences that
//! contributed should drift closer to where they were needed (SLERP toward
//! the query centroid). When a recall was unhelpful, those occurrences
//! decay — their activation count is reduced, making them drift less in
//! future queries and eventually become GC candidates.
//!
//! This is the geometric equivalent of reinforcement: the manifold reshapes
//! itself based on what worked.

use crate::constants::EPSILON;
use crate::quaternion::Quaternion;
use crate::system::{DAESystem, OccurrenceRef};
use crate::tokenizer::tokenize;

/// Feedback signal: did the recalled content help?
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedbackSignal {
    /// The recall was useful — drift occurrences toward the query region.
    Boost,
    /// The recall was not useful — decay activation, let occurrences drift away.
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
}

/// How much to SLERP toward the query centroid on a Boost signal.
/// Moderate — we don't want to collapse the manifold, just nudge.
const BOOST_DRIFT_FACTOR: f64 = 0.15;

/// How much activation to decay on a Demote signal.
/// Floor at 0 — we never go negative.
const DEMOTE_DECAY: u32 = 2;

/// Apply relevance feedback to neighborhoods that were recalled for a query.
///
/// `query` — the original query text (used to compute the centroid for boosting).
/// `neighborhood_ids` — the neighborhood UUIDs that were actually shown to the user.
/// `signal` — Boost or Demote.
///
/// For Boost: activated occurrences in the specified neighborhoods SLERP toward
/// the IDF-weighted centroid of the query's activated occurrences. This pulls
/// helpful memories closer to the region of the manifold where they were needed.
///
/// For Demote: activated occurrences in the specified neighborhoods have their
/// activation count reduced. This makes them less anchored, more likely to
/// drift away in future queries, and eventually GC-eligible.
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

    let centroid = compute_weighted_centroid(&positions, &weights);

    let centroid = match centroid {
        Some(c) => c,
        None => {
            return FeedbackResult {
                boosted: 0,
                demoted: 0,
                centroid: None,
            };
        }
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
    // Factor scales with IDF weight — rare words get pulled harder
    let mut boosted = 0usize;
    for (i, r) in target_refs.iter().enumerate() {
        let occ = system.get_occurrence(*r);
        let plasticity = occ.plasticity();
        let factor = BOOST_DRIFT_FACTOR * target_weights[i] * plasticity;

        if factor > EPSILON {
            let new_pos = occ.position.slerp(centroid, factor);
            let occ = system.get_occurrence_mut(*r);
            occ.position = new_pos;
            // Also bump activation — this memory proved useful
            occ.activation_count = occ.activation_count.saturating_add(1);
            boosted += 1;
        }
    }

    FeedbackResult {
        boosted,
        demoted: 0,
        centroid: Some(centroid),
    }
}

/// Demote: decay activation on target occurrences.
fn apply_demote(system: &mut DAESystem, target_refs: &[OccurrenceRef]) -> FeedbackResult {
    let mut demoted = 0usize;

    for r in target_refs {
        let occ = system.get_occurrence_mut(*r);
        let before = occ.activation_count;
        occ.activation_count = occ.activation_count.saturating_sub(DEMOTE_DECAY);
        if occ.activation_count != before {
            demoted += 1;
        }
    }

    FeedbackResult {
        boosted: 0,
        demoted,
        centroid: None,
    }
}

/// Compute IDF-weighted centroid in R⁴, project to S³.
fn compute_weighted_centroid(positions: &[Quaternion], weights: &[f64]) -> Option<Quaternion> {
    if positions.is_empty() || positions.len() != weights.len() {
        return None;
    }

    let mut sum_w = 0.0f64;
    let mut sum_x = 0.0f64;
    let mut sum_y = 0.0f64;
    let mut sum_z = 0.0f64;
    let mut total_weight = 0.0f64;

    for (pos, w) in positions.iter().zip(weights.iter()) {
        sum_w += pos.w * w;
        sum_x += pos.x * w;
        sum_y += pos.y * w;
        sum_z += pos.z * w;
        total_weight += w;
    }

    if total_weight < EPSILON {
        return None;
    }

    let cw = sum_w / total_weight;
    let cx = sum_x / total_weight;
    let cy = sum_y / total_weight;
    let cz = sum_z / total_weight;

    let norm = (cw * cw + cx * cx + cy * cy + cz * cz).sqrt();
    if norm < EPSILON {
        return None;
    }

    Some(Quaternion::new(cw / norm, cx / norm, cy / norm, cz / norm))
}

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
        words.iter().map(|s| s.to_string()).collect()
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
        let result = apply_feedback(
            &mut sys,
            "hello",
            &[nbhd_id],
            FeedbackSignal::Demote,
        );

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

        let result = apply_feedback(
            &mut sys,
            "quantum",
            &[fake_id],
            FeedbackSignal::Boost,
        );

        assert_eq!(result.boosted, 0);
    }

    #[test]
    fn test_feedback_empty_query() {
        let mut sys = make_feedback_system();
        let nbhd_id = sys.episodes[0].neighborhoods[0].id;

        let result = apply_feedback(
            &mut sys,
            "",
            &[nbhd_id],
            FeedbackSignal::Boost,
        );

        assert_eq!(result.boosted, 0);
    }

    #[test]
    fn test_weighted_centroid_basic() {
        let p1 = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let p2 = Quaternion::new(0.0, 1.0, 0.0, 0.0);

        // Equal weights — centroid should be between them
        let centroid = compute_weighted_centroid(&[p1, p2], &[1.0, 1.0]).unwrap();
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

        // Heavy weight on p1 — centroid should be closer to p1
        let centroid = compute_weighted_centroid(&[p1, p2], &[10.0, 1.0]).unwrap();
        let d1 = p1.angular_distance(centroid);
        let d2 = p2.angular_distance(centroid);
        assert!(
            d1 < d2,
            "centroid should be closer to heavily-weighted point: {d1} vs {d2}"
        );
    }
}
