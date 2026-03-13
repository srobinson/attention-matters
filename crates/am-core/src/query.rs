use std::collections::HashMap;

use crate::constants::{PAIRWISE_DRIFT_MAX_MOBILE, THRESHOLD};
use crate::phasor::DaemonPhasor;
use crate::quaternion::Quaternion;
use crate::system::{ActivationResult, DAESystem, OccurrenceRef};
use crate::tokenizer::tokenize;

/// Single interference result between a subconscious and conscious occurrence.
pub struct InterferenceResult {
    pub sub_ref: OccurrenceRef,
    pub con_ref: OccurrenceRef,
    pub interference: f64,
}

/// Word group for Kuramoto coupling - a word present in both manifolds.
pub struct WordGroup {
    pub word: String,
    pub sub_refs: Vec<OccurrenceRef>,
    pub con_refs: Vec<OccurrenceRef>,
}

/// Full result from `process_query`.
pub struct QueryResult {
    pub activation: ActivationResult,
    pub interference: Vec<InterferenceResult>,
    pub word_groups: Vec<WordGroup>,
    /// Number of unique tokens in the original query (for density scoring).
    pub query_token_count: usize,
}

/// Stateless query processor operating on a `DAESystem`.
pub struct QueryEngine;

impl QueryEngine {
    /// Activate a query: tokenize, deduplicate, activate all matching occurrences.
    pub fn activate(system: &mut DAESystem, query: &str) -> ActivationResult {
        let tokens = tokenize(query);
        let mut seen = std::collections::HashSet::new();
        let unique: Vec<String> = tokens
            .into_iter()
            .filter(|t| seen.insert(t.to_lowercase()))
            .collect();

        let mut result = ActivationResult {
            subconscious: Vec::new(),
            conscious: Vec::new(),
        };

        for token in &unique {
            let activation = system.activate_word(token);
            result.subconscious.extend(activation.subconscious);
            result.conscious.extend(activation.conscious);
        }

        result
    }

    /// Full query pipeline: activate, drift, interference, Kuramoto, return.
    ///
    /// # Examples
    ///
    /// ```
    /// use am_core::{DAESystem, QueryEngine, ingest_text};
    /// use rand::SeedableRng;
    /// use rand::rngs::SmallRng;
    ///
    /// let mut system = DAESystem::new("test");
    /// let mut rng = SmallRng::seed_from_u64(42);
    /// let episode = ingest_text("Rust ownership and borrowing rules", None, &mut rng);
    /// system.add_episode(episode);
    ///
    /// let result = QueryEngine::process_query(&mut system, "ownership");
    /// // Activation should find at least one occurrence of "ownership"
    /// assert!(!result.activation.subconscious.is_empty());
    /// ```
    pub fn process_query(system: &mut DAESystem, query: &str) -> QueryResult {
        let activation = Self::activate(system, query);

        // Unique token count (matches activate's dedup and batch_query's HashSet)
        let query_token_count = {
            let tokens = tokenize(query);
            let unique: std::collections::HashSet<String> =
                tokens.into_iter().map(|t| t.to_lowercase()).collect();
            unique.len()
        };
        let total_nbhd = system.total_neighborhoods();

        let (drift_sub, drift_con) = if query_token_count > 50 {
            let weight_floor = 1.0 / (total_nbhd as f64 * 0.1).floor().max(1.0);
            // Clone words first to avoid borrow conflicts
            let sub_words: Vec<(OccurrenceRef, String)> = activation
                .subconscious
                .iter()
                .map(|r| (*r, system.get_occurrence(*r).word.clone()))
                .collect();
            let con_words: Vec<(OccurrenceRef, String)> = activation
                .conscious
                .iter()
                .map(|r| (*r, system.get_occurrence(*r).word.clone()))
                .collect();
            let sub: Vec<OccurrenceRef> = sub_words
                .into_iter()
                .filter(|(_, word)| system.get_word_weight(word) >= weight_floor)
                .map(|(r, _)| r)
                .collect();
            let con: Vec<OccurrenceRef> = con_words
                .into_iter()
                .filter(|(_, word)| system.get_word_weight(word) >= weight_floor)
                .map(|(r, _)| r)
                .collect();
            (sub, con)
        } else {
            (
                activation.subconscious.clone(),
                activation.conscious.clone(),
            )
        };

        Self::drift_and_consolidate(system, &drift_sub);
        Self::drift_and_consolidate(system, &drift_con);

        let (interference, word_groups) =
            Self::compute_interference(system, &activation.subconscious, &activation.conscious);

        Self::apply_kuramoto_coupling(system, &word_groups);

        QueryResult {
            activation,
            interference,
            word_groups,
            query_token_count,
        }
    }

    /// Drift activated occurrences toward each other.
    /// Pairwise O(n²) for <200 mobile, centroid O(n) for >=200.
    pub fn drift_and_consolidate(system: &mut DAESystem, activated: &[OccurrenceRef]) {
        if activated.len() < 2 {
            return;
        }

        // Cache container activations
        let container_activations: HashMap<OccurrenceRef, u32> = activated
            .iter()
            .map(|r| {
                let nbhd = system.get_neighborhood_for_occurrence(*r);
                (*r, nbhd.total_activation())
            })
            .collect();

        // Pre-filter: only mobile (drift rate > 0)
        let mobile: Vec<OccurrenceRef> = activated
            .iter()
            .filter(|r| {
                let occ = system.get_occurrence(**r);
                let ca = container_activations[r];
                occ.drift_rate(ca) > 0.0
            })
            .copied()
            .collect();

        if mobile.len() < 2 {
            return;
        }

        if mobile.len() >= PAIRWISE_DRIFT_MAX_MOBILE {
            Self::centroid_drift(system, &mobile, &container_activations);
        } else {
            Self::pairwise_drift(system, &mobile, &container_activations);
        }
    }

    /// Pairwise drift: O(n²). Each pair of mobile occurrences drifts toward
    /// a weighted meeting point. Both position and phasor drift.
    fn pairwise_drift(
        system: &mut DAESystem,
        mobile: &[OccurrenceRef],
        container_activations: &HashMap<OccurrenceRef, u32>,
    ) {
        // Snapshot current state to avoid read-after-write issues
        let states: Vec<(Quaternion, DaemonPhasor, f64, String)> = mobile
            .iter()
            .map(|r| {
                let occ = system.get_occurrence(*r);
                let ca = container_activations[r];
                let dr = occ.drift_rate(ca);
                (occ.position, occ.phasor, dr, occ.word.clone())
            })
            .collect();

        // Compute IDF weights
        let weights: Vec<f64> = states
            .iter()
            .map(|(_, _, _, w)| system.get_word_weight(w))
            .collect();

        // Collect all deltas
        let n = mobile.len();
        let mut position_deltas: Vec<Vec<(Quaternion, f64)>> = vec![Vec::new(); n];
        let mut phasor_deltas: Vec<Vec<(DaemonPhasor, f64)>> = vec![Vec::new(); n];

        for i in 0..n {
            let (pos1, phasor1, dr1, _) = &states[i];
            let t1 = dr1 * weights[i];

            for j in (i + 1)..n {
                let (pos2, phasor2, dr2, _) = &states[j];
                let t2 = dr2 * weights[j];

                if t1 <= 0.0 && t2 <= 0.0 {
                    continue;
                }

                let total = t1 + t2;
                if total <= 0.0 {
                    continue;
                }

                let weight = t1 / total;
                let meeting = pos1.slerp(*pos2, weight);

                if t1 > 0.0 {
                    let factor = t1 * THRESHOLD;
                    position_deltas[i].push((meeting, factor));
                    phasor_deltas[i].push((*phasor2, factor));
                }
                if t2 > 0.0 {
                    let factor = t2 * THRESHOLD;
                    position_deltas[j].push((meeting, factor));
                    phasor_deltas[j].push((*phasor1, factor));
                }
            }
        }

        // Apply all deltas
        for (idx, r) in mobile.iter().enumerate() {
            let (mut pos, mut phasor, _, _) = states[idx];

            for (target, factor) in &position_deltas[idx] {
                pos = pos.slerp(*target, *factor);
            }
            for (target, factor) in &phasor_deltas[idx] {
                phasor = phasor.slerp(*target, *factor);
            }

            let occ = system.get_occurrence_mut(*r);
            occ.position = pos;
            occ.phasor = phasor;
        }
    }

    /// Centroid drift: O(n). IDF-weighted leave-one-out centroid in R⁴,
    /// project to S³. No phasor drift.
    ///
    /// Uses `Quaternion::weighted_sum` for R⁴ accumulation and
    /// `WeightedSum::leave_one_out` for per-element centroid exclusion,
    /// sharing the same primitives as `Quaternion::weighted_centroid`.
    fn centroid_drift(
        system: &mut DAESystem,
        mobile: &[OccurrenceRef],
        container_activations: &HashMap<OccurrenceRef, u32>,
    ) {
        // Snapshot in separate passes to avoid borrow conflicts
        let words: Vec<String> = mobile
            .iter()
            .map(|r| system.get_occurrence(*r).word.clone())
            .collect();
        let idf_weights: Vec<f64> = words.iter().map(|w| system.get_word_weight(w)).collect();
        let positions: Vec<Quaternion> = mobile
            .iter()
            .map(|r| system.get_occurrence(*r).position)
            .collect();
        let drift_rates: Vec<f64> = mobile
            .iter()
            .map(|r| {
                let occ = system.get_occurrence(*r);
                let ca = container_activations[r];
                occ.drift_rate(ca)
            })
            .collect();

        // Compute weighted sum in R⁴ using the shared utility
        let Some(sum) = Quaternion::weighted_sum(&positions, &idf_weights) else {
            return;
        };

        // Apply leave-one-out centroid drift
        for (idx, r) in mobile.iter().enumerate() {
            let Some(target) = sum.leave_one_out(positions[idx], idf_weights[idx]) else {
                continue;
            };

            let factor = drift_rates[idx] * idf_weights[idx] * 0.5;
            if factor > 0.0 {
                let occ = system.get_occurrence_mut(*r);
                occ.position = occ.position.slerp(target, factor);
            }
        }
    }

    /// Compute interference between subconscious and conscious occurrences.
    /// Returns interference results and word groups for Kuramoto.
    #[must_use]
    pub fn compute_interference(
        system: &DAESystem,
        subconscious: &[OccurrenceRef],
        conscious: &[OccurrenceRef],
    ) -> (Vec<InterferenceResult>, Vec<WordGroup>) {
        // Group by word
        let mut sub_by_word: HashMap<String, Vec<OccurrenceRef>> = HashMap::new();
        let mut con_by_word: HashMap<String, Vec<OccurrenceRef>> = HashMap::new();

        for r in subconscious {
            let word = system.get_occurrence(*r).word.to_lowercase();
            sub_by_word.entry(word).or_default().push(*r);
        }
        for r in conscious {
            let word = system.get_occurrence(*r).word.to_lowercase();
            con_by_word.entry(word).or_default().push(*r);
        }

        let mut results = Vec::new();
        let mut word_groups = Vec::new();

        for (word, sub_refs) in &sub_by_word {
            let Some(con_refs) = con_by_word.get(word) else {
                continue;
            };

            // Circular mean phase of conscious occurrences
            let mut sin_sum = 0.0;
            let mut cos_sum = 0.0;
            for r in con_refs {
                let theta = system.get_occurrence(*r).phasor.theta;
                sin_sum += theta.sin();
                cos_sum += theta.cos();
            }
            let count = con_refs.len() as f64;
            let mean_con_phase = (sin_sum / count).atan2(cos_sum / count);

            // Per-subconscious-occurrence interference against conscious mean
            for sub_ref in sub_refs {
                let sub_theta = system.get_occurrence(*sub_ref).phasor.theta;
                let mut diff = (sub_theta - mean_con_phase).abs();
                if diff > std::f64::consts::PI {
                    diff = std::f64::consts::TAU - diff;
                }
                let interference = diff.cos();

                results.push(InterferenceResult {
                    sub_ref: *sub_ref,
                    con_ref: con_refs[0],
                    interference,
                });
            }

            word_groups.push(WordGroup {
                word: word.clone(),
                sub_refs: sub_refs.clone(),
                con_refs: con_refs.clone(),
            });
        }

        (results, word_groups)
    }

    /// Apply Kuramoto phase coupling across manifolds.
    pub fn apply_kuramoto_coupling(system: &mut DAESystem, word_groups: &[WordGroup]) {
        if word_groups.is_empty() {
            return;
        }

        let n_con = system.conscious_episode.count().max(1);
        let n_total = system.n().max(1);
        let n_sub = n_total.saturating_sub(n_con).max(1);

        let k_con = n_sub as f64 / n_total as f64;
        let k_sub = n_con as f64 / n_total as f64;

        for group in word_groups {
            let w = system.get_word_weight(&group.word);
            let coupling = w * w;

            // Circular mean phases
            let (mean_phase_sub, mean_phase_con) = {
                let mut sin_sub = 0.0;
                let mut cos_sub = 0.0;
                for r in &group.sub_refs {
                    let theta = system.get_occurrence(*r).phasor.theta;
                    sin_sub += theta.sin();
                    cos_sub += theta.cos();
                }
                let count_sub = group.sub_refs.len() as f64;

                let mut sin_con = 0.0;
                let mut cos_con = 0.0;
                for r in &group.con_refs {
                    let theta = system.get_occurrence(*r).phasor.theta;
                    sin_con += theta.sin();
                    cos_con += theta.cos();
                }
                let count_con = group.con_refs.len() as f64;

                (
                    (sin_sub / count_sub).atan2(cos_sub / count_sub),
                    (sin_con / count_con).atan2(cos_con / count_con),
                )
            };

            // Phase difference wrapped to [-π, π]
            let phase_diff = ((mean_phase_con - mean_phase_sub) + std::f64::consts::PI)
                .rem_euclid(std::f64::consts::TAU)
                - std::f64::consts::PI;

            let sin_diff = phase_diff.sin();
            let base_delta_sub = k_con * coupling * sin_diff;
            let base_delta_con = -k_sub * coupling * sin_diff;

            // Apply with plasticity modulation
            for r in &group.sub_refs {
                let occ = system.get_occurrence_mut(*r);
                let plasticity = occ.plasticity();
                occ.phasor = DaemonPhasor::new(occ.phasor.theta + base_delta_sub * plasticity);
            }
            for r in &group.con_refs {
                let occ = system.get_occurrence_mut(*r);
                let plasticity = occ.plasticity();
                occ.phasor = DaemonPhasor::new(occ.phasor.theta + base_delta_con * plasticity);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::Episode;
    use crate::neighborhood::Neighborhood;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(std::string::ToString::to_string).collect()
    }

    fn make_test_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Subconscious episode with shared words
        let mut ep = Episode::new("memories");
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
        let n3 = Neighborhood::from_tokens(
            &to_tokens(&["neural", "network", "learning"]),
            None,
            "neural network learning",
            &mut rng,
        );
        ep.add_neighborhood(n1);
        ep.add_neighborhood(n2);
        ep.add_neighborhood(n3);
        sys.add_episode(ep);

        // Conscious: overlap on "quantum"
        sys.add_to_conscious("quantum mechanics", &mut rng);

        sys
    }

    #[test]
    fn test_pairwise_drift_moves_closer() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Create neighborhoods where activating multiple words ensures
        // occurrences have non-zero drift rate (ratio < THRESHOLD)
        let mut ep = Episode::new("test");
        let n1 = Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta", "gamma", "delta"]),
            None,
            "alpha beta gamma delta",
            &mut rng,
        );
        let n2 = Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta", "epsilon", "zeta"]),
            None,
            "alpha beta epsilon zeta",
            &mut rng,
        );
        ep.add_neighborhood(n1);
        ep.add_neighborhood(n2);
        sys.add_episode(ep);

        // Activate multiple words so container activation is high enough
        // that individual ratio < THRESHOLD
        let activation = QueryEngine::activate(&mut sys, "alpha beta gamma delta epsilon zeta");

        // Get refs for "alpha" which is in both neighborhoods
        let alpha_refs: Vec<_> = activation
            .subconscious
            .iter()
            .filter(|r| sys.get_occurrence(**r).word == "alpha")
            .copied()
            .collect();
        assert!(
            alpha_refs.len() >= 2,
            "need alpha in at least 2 neighborhoods"
        );

        let pos_before_0 = sys.get_occurrence(alpha_refs[0]).position;
        let pos_before_1 = sys.get_occurrence(alpha_refs[1]).position;
        let dist_before = pos_before_0.angular_distance(pos_before_1);

        QueryEngine::drift_and_consolidate(&mut sys, &activation.subconscious);

        let pos_after_0 = sys.get_occurrence(alpha_refs[0]).position;
        let pos_after_1 = sys.get_occurrence(alpha_refs[1]).position;
        let dist_after = pos_after_0.angular_distance(pos_after_1);

        assert!(
            dist_after < dist_before,
            "drift should move occurrences closer: {dist_before} -> {dist_after}"
        );
    }

    #[test]
    fn test_anchored_dont_move() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        let mut ep = Episode::new("test");
        let mut n = Neighborhood::from_tokens(
            &to_tokens(&["word1", "word2"]),
            None,
            "word1 word2",
            &mut rng,
        );
        // Make word1 anchored by setting high activation relative to container
        n.occurrences[0].activation_count = 100;
        n.occurrences[1].activation_count = 1;
        ep.add_neighborhood(n);
        sys.add_episode(ep);

        let refs = sys.get_word_occurrences("word1");
        let pos_before = sys.get_occurrence(refs[0]).position;

        // Activate and drift
        let activation = QueryEngine::activate(&mut sys, "word1 word2");
        QueryEngine::drift_and_consolidate(&mut sys, &activation.subconscious);

        let pos_after = sys.get_occurrence(refs[0]).position;
        assert_eq!(pos_before, pos_after, "anchored word should not move");
    }

    #[test]
    fn test_interference_computation() {
        let mut sys = make_test_system();
        let activation = QueryEngine::activate(&mut sys, "quantum");

        let (interference, word_groups) = QueryEngine::compute_interference(
            &sys,
            &activation.subconscious,
            &activation.conscious,
        );

        assert!(!interference.is_empty(), "should have interference results");
        assert!(!word_groups.is_empty(), "should have word groups");
        assert_eq!(word_groups[0].word, "quantum");

        for ir in &interference {
            assert!(
                ir.interference >= -1.0 && ir.interference <= 1.0,
                "interference out of range: {}",
                ir.interference
            );
        }
    }

    #[test]
    fn test_kuramoto_coupling_constants() {
        let sys = make_test_system();
        let n_con = sys.conscious_episode.count().max(1);
        let n_total = sys.n().max(1);
        let n_sub = n_total.saturating_sub(n_con).max(1);

        let k_con = n_sub as f64 / n_total as f64;
        let k_sub = n_con as f64 / n_total as f64;

        assert!(
            (k_con + k_sub - 1.0).abs() < 0.01,
            "K_CON + K_SUB should ≈ 1: {} + {} = {}",
            k_con,
            k_sub,
            k_con + k_sub
        );
    }

    #[test]
    fn test_kuramoto_pulls_phases() {
        let mut sys = make_test_system();
        let activation = QueryEngine::activate(&mut sys, "quantum");

        // Get initial phase diff
        let sub_refs = activation.subconscious.clone();
        let con_refs = activation.conscious.clone();

        if sub_refs.is_empty() || con_refs.is_empty() {
            return; // Skip if no overlap
        }

        let sub_theta_before = sys.get_occurrence(sub_refs[0]).phasor.theta;
        let con_theta_before = sys.get_occurrence(con_refs[0]).phasor.theta;
        let diff_before = (sub_theta_before - con_theta_before).abs();

        let (_, word_groups) = QueryEngine::compute_interference(&sys, &sub_refs, &con_refs);
        QueryEngine::apply_kuramoto_coupling(&mut sys, &word_groups);

        let sub_theta_after = sys.get_occurrence(sub_refs[0]).phasor.theta;
        let con_theta_after = sys.get_occurrence(con_refs[0]).phasor.theta;
        let mut diff_after = (sub_theta_after - con_theta_after).abs();
        if diff_after > std::f64::consts::PI {
            diff_after = std::f64::consts::TAU - diff_after;
        }
        let mut diff_before_wrapped = diff_before;
        if diff_before_wrapped > std::f64::consts::PI {
            diff_before_wrapped = std::f64::consts::TAU - diff_before_wrapped;
        }

        // Kuramoto should reduce or maintain phase difference
        // (it pulls toward alignment)
        assert!(
            diff_after <= diff_before_wrapped + 0.01,
            "Kuramoto should pull phases closer: {diff_before_wrapped} -> {diff_after}"
        );
    }

    #[test]
    fn test_full_pipeline() {
        let mut sys = make_test_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics");

        assert!(!result.activation.subconscious.is_empty());
        assert!(!result.activation.conscious.is_empty());
        assert!(!result.interference.is_empty());
    }

    #[test]
    fn test_idf_rare_words_drift_more() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // "rare" appears once, "common" appears in many neighborhoods
        let mut ep = Episode::new("test");
        for i in 0..5 {
            let tokens = if i == 0 {
                to_tokens(&["rare", "common"])
            } else {
                to_tokens(&["common", &format!("filler{i}")])
            };
            let n = Neighborhood::from_tokens(&tokens, None, "", &mut rng);
            ep.add_neighborhood(n);
        }
        sys.add_episode(ep);

        let w_rare = sys.get_word_weight("rare");
        let w_common = sys.get_word_weight("common");

        assert!(
            w_rare > w_common,
            "rare word should have higher IDF weight: {w_rare} vs {w_common}"
        );
    }

    /// Generate a query string with >50 unique tokens.
    fn make_large_query(unique_words: &[&str], filler_count: usize) -> String {
        let mut words: Vec<String> = unique_words.iter().map(|w| (*w).to_string()).collect();
        for i in 0..filler_count {
            words.push(format!("filler{i}"));
        }
        words.join(" ")
    }

    #[test]
    fn large_query_filters_common_words_from_drift() {
        // Build a system with >10 neighborhoods so weight_floor < 1.0.
        // "common" appears in every neighborhood (low IDF weight).
        // Verify the weight_floor computation is correct and the pipeline
        // completes without panic for >50 token queries.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let mut ep = Episode::new("test");
        for i in 0..12 {
            let tokens = if i == 0 {
                to_tokens(&["rare", "common"])
            } else {
                to_tokens(&["common", &format!("word{i}")])
            };
            let n = Neighborhood::from_tokens(&tokens, None, "", &mut rng);
            ep.add_neighborhood(n);
        }
        sys.add_episode(ep);

        let total_nbhd = sys.total_neighborhoods();
        assert!(total_nbhd >= 10, "need >= 10 neighborhoods");

        // Verify the weight_floor math: 1.0 / floor(12 * 0.1) = 1.0
        let weight_floor = 1.0 / (total_nbhd as f64 * 0.1).floor().max(1.0);
        assert!(
            (weight_floor - 1.0).abs() < f64::EPSILON,
            "weight_floor should be 1.0 for 12 neighborhoods, got {weight_floor}"
        );

        // "common" IDF = 1/12 < weight_floor, should be excluded from drift
        let common_weight = sys.get_word_weight("common");
        assert!(
            common_weight < weight_floor,
            "common (weight {common_weight}) should be below floor ({weight_floor})"
        );

        // "rare" IDF = 1.0 >= weight_floor, should be included in drift
        let rare_weight = sys.get_word_weight("rare");
        assert!(
            rare_weight >= weight_floor,
            "rare (weight {rare_weight}) should be at or above floor ({weight_floor})"
        );

        // The full pipeline should complete without panic
        let query = make_large_query(&["rare", "common"], 55);
        let result = QueryEngine::process_query(&mut sys, &query);

        // Activation should contain both (filtering only affects drift)
        assert!(
            result.query_token_count > 50,
            "query should have >50 unique tokens"
        );
        assert!(
            !result.activation.subconscious.is_empty(),
            "activation should contain occurrences"
        );
    }

    #[test]
    fn large_query_weight_floor_one_with_few_neighborhoods() {
        // Build a system with <10 neighborhoods so weight_floor = 1.0.
        // Only words in exactly 1 neighborhood pass drift filtering.
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let mut ep = Episode::new("test");

        // 3 neighborhoods: "unique_a" in one, "shared" in all three
        let n1 = Neighborhood::from_tokens(&to_tokens(&["unique_a", "shared"]), None, "", &mut rng);
        let n2 = Neighborhood::from_tokens(&to_tokens(&["unique_b", "shared"]), None, "", &mut rng);
        let n3 = Neighborhood::from_tokens(&to_tokens(&["unique_c", "shared"]), None, "", &mut rng);
        ep.add_neighborhood(n1);
        ep.add_neighborhood(n2);
        ep.add_neighborhood(n3);
        sys.add_episode(ep);

        let total_nbhd = sys.total_neighborhoods();
        assert!(total_nbhd < 10, "need < 10 neighborhoods");

        // Verify edge case: floor(3 * 0.1) = 0, max(1.0) = 1.0
        let weight_floor = 1.0 / (total_nbhd as f64 * 0.1).floor().max(1.0);
        assert!(
            (weight_floor - 1.0).abs() < f64::EPSILON,
            "weight_floor should be 1.0 for <10 neighborhoods, got {weight_floor}"
        );

        // "shared" IDF = 1/3 < 1.0 - excluded from drift
        let shared_weight = sys.get_word_weight("shared");
        assert!(
            shared_weight < weight_floor,
            "shared (weight {shared_weight}) should be below 1.0"
        );

        // unique words: IDF = 1.0 - included in drift
        let unique_weight = sys.get_word_weight("unique_a");
        assert!(
            unique_weight >= weight_floor,
            "unique_a (weight {unique_weight}) should pass floor"
        );

        // The full pipeline should not crash or produce empty results
        let query = make_large_query(&["unique_a", "unique_b", "unique_c", "shared"], 55);
        let result = QueryEngine::process_query(&mut sys, &query);

        assert!(
            result.query_token_count > 50,
            "query should have >50 unique tokens"
        );
        assert!(
            !result.activation.subconscious.is_empty(),
            "pipeline should produce non-empty activation"
        );
    }
}
