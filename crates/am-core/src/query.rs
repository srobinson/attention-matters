use std::collections::HashMap;

use uuid::Uuid;

use crate::constants::{PAIRWISE_DRIFT_MAX_MOBILE, THRESHOLD};
use crate::phasor::DaemonPhasor;
use crate::quaternion::Quaternion;
use crate::system::{ActivationResult, DAESystem, OccurrenceRef};
use crate::tokenizer::tokenize;

/// Manifest of mutations applied to the `DAESystem` during a query.
///
/// Captures which occurrences had their positions/phasors drifted and which
/// had their activation counts incremented. Required by incremental persistence
/// (Phase 2) to issue targeted `SQLite` writes instead of full `save_system`.
#[derive(Debug, Default)]
pub struct QueryManifest {
    /// Occurrence IDs whose position or phasor was modified by drift or Kuramoto coupling.
    pub drifted: Vec<Uuid>,
    /// Occurrence IDs whose `activation_count` was incremented.
    pub activated: Vec<Uuid>,
    /// Occurrence IDs with absolute activation counts after demotion.
    /// Used by feedback demote where activation is decremented, not incremented.
    pub demoted_activations: Vec<(Uuid, u32)>,
}

/// Single interference result between a subconscious and conscious occurrence.
pub(crate) struct InterferenceResult {
    pub sub_ref: OccurrenceRef,
    pub con_ref: OccurrenceRef,
    pub interference: f64,
}

/// Word group for Kuramoto coupling - a word present in both manifolds.
pub(crate) struct WordGroup {
    pub word: String,
    pub sub_refs: Vec<OccurrenceRef>,
    pub con_refs: Vec<OccurrenceRef>,
}

/// Full result from `process_query`.
pub struct QueryResult {
    pub activation: ActivationResult,
    pub(crate) interference: Vec<InterferenceResult>,
    /// Number of unique tokens in the original query (for density scoring).
    pub query_token_count: usize,
    /// Manifest of all mutations applied to the system during this query.
    pub manifest: QueryManifest,
}

/// Stateless query processor operating on a `DAESystem`.
pub struct QueryEngine;

impl QueryEngine {
    /// Activate a query: tokenize, deduplicate, activate all matching occurrences.
    ///
    /// Returns the activation result and a list of activated occurrence UUIDs.
    pub fn activate(system: &mut DAESystem, query: &str) -> (ActivationResult, Vec<Uuid>) {
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

        let activated_ids: Vec<Uuid> = result
            .subconscious
            .iter()
            .chain(result.conscious.iter())
            .map(|r| system.get_occurrence(*r).id)
            .collect();

        (result, activated_ids)
    }

    /// Full query pipeline: activate, drift, interference, Kuramoto, return.
    ///
    /// # Examples
    ///
    /// ```
    /// use am_core::{system::DAESystem, query::QueryEngine, tokenizer::ingest_text};
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
        let (activation, activated_ids) = Self::activate(system, query);

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

        let mut drifted = Self::drift_and_consolidate(system, &drift_sub);
        drifted.extend(Self::drift_and_consolidate(system, &drift_con));

        let (interference, word_groups) =
            Self::compute_interference(system, &activation.subconscious, &activation.conscious);

        drifted.extend(Self::apply_kuramoto_coupling(system, &word_groups));

        QueryResult {
            activation,
            interference,
            query_token_count,
            manifest: QueryManifest {
                drifted,
                activated: activated_ids,
                demoted_activations: Vec::new(),
            },
        }
    }

    /// Drift activated occurrences toward each other.
    /// Pairwise O(n^2) for <200 mobile, centroid O(n) for >=200.
    ///
    /// Returns the UUIDs of occurrences whose position or phasor changed.
    pub fn drift_and_consolidate(system: &mut DAESystem, activated: &[OccurrenceRef]) -> Vec<Uuid> {
        if activated.len() < 2 {
            return Vec::new();
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
            return Vec::new();
        }

        if mobile.len() >= PAIRWISE_DRIFT_MAX_MOBILE {
            Self::centroid_drift(system, &mobile, &container_activations)
        } else {
            Self::pairwise_drift(system, &mobile, &container_activations)
        }
    }

    /// Pairwise drift: O(n^2). Each pair of mobile occurrences drifts toward
    /// a weighted meeting point. Both position and phasor drift.
    ///
    /// Returns UUIDs of all mobile occurrences (all receive position/phasor updates).
    fn pairwise_drift(
        system: &mut DAESystem,
        mobile: &[OccurrenceRef],
        container_activations: &HashMap<OccurrenceRef, u32>,
    ) -> Vec<Uuid> {
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

        // All mobile occurrences received position/phasor updates
        mobile
            .iter()
            .map(|r| system.get_occurrence(*r).id)
            .collect()
    }

    /// Centroid drift: O(n). IDF-weighted leave-one-out centroid in R^4,
    /// project to S^3. No phasor drift.
    ///
    /// Uses `Quaternion::weighted_sum` for R^4 accumulation and
    /// `WeightedSum::leave_one_out` for per-element centroid exclusion,
    /// sharing the same primitives as `Quaternion::weighted_centroid`.
    ///
    /// Returns UUIDs of occurrences that actually moved (factor > 0).
    fn centroid_drift(
        system: &mut DAESystem,
        mobile: &[OccurrenceRef],
        container_activations: &HashMap<OccurrenceRef, u32>,
    ) -> Vec<Uuid> {
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

        // Compute weighted sum in R^4 using the shared utility
        let Some(sum) = Quaternion::weighted_sum(&positions, &idf_weights) else {
            return Vec::new();
        };

        let mut drifted_ids = Vec::new();

        // Apply leave-one-out centroid drift
        for (idx, r) in mobile.iter().enumerate() {
            let Some(target) = sum.leave_one_out(positions[idx], idf_weights[idx]) else {
                continue;
            };

            let factor = drift_rates[idx] * idf_weights[idx] * 0.5;
            if factor > 0.0 {
                let occ = system.get_occurrence_mut(*r);
                occ.position = occ.position.slerp(target, factor);
                drifted_ids.push(occ.id);
            }
        }

        drifted_ids
    }

    /// Compute interference and apply Kuramoto phase coupling in one step.
    ///
    /// Combines `compute_interference` and `apply_kuramoto_coupling` into a
    /// single public method, keeping the intermediate types crate-internal.
    /// Returns UUIDs of occurrences whose phasor was modified by coupling.
    pub fn couple_phases(
        system: &mut DAESystem,
        subconscious: &[OccurrenceRef],
        conscious: &[OccurrenceRef],
    ) -> Vec<Uuid> {
        let (_, word_groups) = Self::compute_interference(system, subconscious, conscious);
        Self::apply_kuramoto_coupling(system, &word_groups)
    }

    /// Compute interference between subconscious and conscious occurrences.
    /// Returns interference results and word groups for Kuramoto.
    #[must_use]
    pub(crate) fn compute_interference(
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
    ///
    /// Returns UUIDs of occurrences whose phasor was modified.
    pub(crate) fn apply_kuramoto_coupling(
        system: &mut DAESystem,
        word_groups: &[WordGroup],
    ) -> Vec<Uuid> {
        if word_groups.is_empty() {
            return Vec::new();
        }

        let n_con = system.conscious_episode.count().max(1);
        let n_total = system.n().max(1);
        let n_sub = n_total.saturating_sub(n_con).max(1);

        let k_con = n_sub as f64 / n_total as f64;
        let k_sub = n_con as f64 / n_total as f64;

        let mut coupled_ids = Vec::new();

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

            // Phase difference wrapped to [-pi, pi]
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
                coupled_ids.push(occ.id);
            }
            for r in &group.con_refs {
                let occ = system.get_occurrence_mut(*r);
                let plasticity = occ.plasticity();
                occ.phasor = DaemonPhasor::new(occ.phasor.theta + base_delta_con * plasticity);
                coupled_ids.push(occ.id);
            }
        }

        coupled_ids
    }
}

#[cfg(test)]
#[path = "query_tests.rs"]
mod tests;
