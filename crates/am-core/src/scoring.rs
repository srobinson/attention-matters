//! Scoring internals for context composition.
//!
//! Scores, ranks, and filters candidate neighborhoods for recall.
//! Handles IDF-weighted activation scoring, phasor interference modulation,
//! vividness boosting, recency decay, overlap suppression, and density bonuses.

use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::compose::RecallCategory;
use crate::neighborhood::NeighborhoodType;
use crate::query::{InterferenceResult, QueryResult};
use crate::recency::{RECENCY_DECAY_RATE, days_since_episode};
use crate::surface::SurfaceResult;
use crate::system::{DAESystem, OccurrenceRef};
use crate::tokenizer::token_count;

/// Multiplier for Decision/Preference neighborhoods.
/// Decisions that genuinely match the query score this many times higher.
pub(crate) const DECISION_MULTIPLIER: f64 = 3.0;

/// Minimum overlap threshold for conscious recall.
/// At least this fraction of query tokens must match for a conscious neighborhood
/// to surface. Prevents stop-word-only matches from dominating results.
pub(crate) const CONSCIOUS_MIN_OVERLAP: f64 = 0.2;

/// Weight for phasor interference contribution to scoring.
/// Positive interference (in-phase) boosts, negative (anti-phase) suppresses.
pub(crate) const INTERFERENCE_WEIGHT: f64 = 0.3;

/// Boost multiplier for vivid neighborhoods (>50% surfaced occurrences).
pub(crate) const VIVIDNESS_BOOST: f64 = 1.5;

/// IDF-weighted word overlap threshold for contradiction detection.
/// Pairs of neighborhoods above this threshold are considered overlapping.
pub(crate) const OVERLAP_THRESHOLD: f64 = 0.3;

/// Score multiplier applied to older neighborhoods in overlapping groups.
pub(crate) const OVERLAP_SUPPRESSION: f64 = 0.1;

/// Minimum score threshold for inclusion in recall results.
/// Candidates scoring below this are excluded to avoid padding with weak matches.
pub(crate) const MIN_SCORE_THRESHOLD: f64 = 1.0;

pub(crate) struct ScoredNeighborhood {
    pub neighborhood_id: Uuid,
    pub episode_idx: usize, // usize::MAX for conscious
    pub neighborhood_idx: usize,
    pub score: f64,
    pub activated_count: usize,
    pub words: HashSet<String>,
    pub max_word_weight: f64,
    pub max_plasticity: f64,
    pub neighborhood_type: NeighborhoodType,
    pub epoch: u64,
}

pub(crate) struct RankedCandidate {
    pub neighborhood_id: Uuid,
    pub episode_idx: usize,
    pub category: RecallCategory,
    pub score: f64,
    pub text: String,
    pub tokens: usize,
    pub neighborhood_type: NeighborhoodType,
}

/// Score and categorize all activated neighborhoods into ranked candidates.
/// Conscious neighborhoods scored by IDF-weighted activation.
/// Subconscious neighborhoods scored by IDF-weighted activation.
/// Novel candidates: subconscious with `activated_count` <= 2, no words in common
/// with conscious, scored by `max_word_weight` * `max_plasticity` / `activated_count`.
pub(crate) fn rank_candidates(
    system: &mut DAESystem,
    query_result: &QueryResult,
    interference: &[InterferenceResult],
    surface: &SurfaceResult,
) -> Vec<RankedCandidate> {
    let conscious_words: HashSet<String> = query_result
        .activation
        .conscious
        .iter()
        .map(|r| system.get_occurrence(*r).word.to_lowercase())
        .collect();

    let qtc = query_result.query_token_count;
    let mut con_scored = score_neighborhoods(system, &query_result.activation.conscious, true, qtc);
    let mut sub_scored =
        score_neighborhoods(system, &query_result.activation.subconscious, false, qtc);

    // Suppress older neighborhoods that overlap with newer ones (contradiction handling)
    overlap_suppress(&mut con_scored, &mut sub_scored, system);

    // Apply phasor interference to scores
    let net_interference = aggregate_interference(system, interference);

    // Conscious: strong anti-phase suppression
    for sn in con_scored.values_mut() {
        if let Some(&net) = net_interference.get(&sn.neighborhood_id)
            && net < -0.5
        {
            sn.score *= 0.5;
        }
    }

    // Subconscious: continuous interference modulation
    for sn in sub_scored.values_mut() {
        if let Some(&net) = net_interference.get(&sn.neighborhood_id) {
            sn.score *= 1.0 + net * INTERFERENCE_WEIGHT;
        }
    }

    // Boost vivid neighborhoods (>50% surfaced occurrences)
    for sn in con_scored.values_mut() {
        if surface.vivid_neighborhood_ids.contains(&sn.neighborhood_id) {
            sn.score *= VIVIDNESS_BOOST;
        }
    }
    for sn in sub_scored.values_mut() {
        if surface.vivid_neighborhood_ids.contains(&sn.neighborhood_id) {
            sn.score *= VIVIDNESS_BOOST;
        }
    }

    let mut candidates = Vec::new();
    let mut selected_for_novel: HashSet<Uuid> = HashSet::new();

    // Conscious candidates
    for sn in con_scored.values() {
        let text = get_neighborhood_text(
            system,
            sn.neighborhood_id,
            sn.episode_idx,
            sn.neighborhood_idx,
        );
        let tokens = token_count(&text);
        candidates.push(RankedCandidate {
            neighborhood_id: sn.neighborhood_id,
            episode_idx: sn.episode_idx,
            category: RecallCategory::Conscious,
            score: sn.score,
            text,
            tokens,
            neighborhood_type: sn.neighborhood_type,
        });
    }

    // Subconscious candidates
    for sn in sub_scored.values() {
        let text = get_neighborhood_text(
            system,
            sn.neighborhood_id,
            sn.episode_idx,
            sn.neighborhood_idx,
        );
        let tokens = token_count(&text);
        candidates.push(RankedCandidate {
            neighborhood_id: sn.neighborhood_id,
            episode_idx: sn.episode_idx,
            category: RecallCategory::Subconscious,
            score: sn.score,
            text,
            tokens,
            neighborhood_type: sn.neighborhood_type,
        });

        // Check if this is also a novel candidate
        if sn.activated_count <= 2 && !sn.words.iter().any(|w| conscious_words.contains(w)) {
            selected_for_novel.insert(sn.neighborhood_id);
        }
    }

    // Add novel candidates (these are subconscious neighborhoods that qualify)
    for sn in sub_scored.values() {
        if !selected_for_novel.contains(&sn.neighborhood_id) {
            continue;
        }
        let novelty_score =
            sn.max_word_weight * sn.max_plasticity / sn.activated_count.max(1) as f64;
        let text = get_neighborhood_text(
            system,
            sn.neighborhood_id,
            sn.episode_idx,
            sn.neighborhood_idx,
        );
        let tokens = token_count(&text);
        candidates.push(RankedCandidate {
            neighborhood_id: sn.neighborhood_id,
            episode_idx: sn.episode_idx,
            category: RecallCategory::Novel,
            score: novelty_score,
            text,
            tokens,
            neighborhood_type: sn.neighborhood_type,
        });
    }

    candidates
}

/// Aggregate per-neighborhood mean interference from pairwise results.
/// Returns map of `neighborhood_id` -> mean `cos(phase_diff)`.
/// Aggregates both sides of each pair so conscious and subconscious
/// neighborhoods both receive interference values.
pub(crate) fn aggregate_interference(
    system: &DAESystem,
    interference: &[InterferenceResult],
) -> HashMap<Uuid, f64> {
    let mut sums: HashMap<Uuid, (f64, usize)> = HashMap::new();
    for ir in interference {
        // Subconscious side
        let sub_nbhd = system.get_neighborhood_for_occurrence(ir.sub_ref);
        let entry = sums.entry(sub_nbhd.id).or_insert((0.0, 0));
        entry.0 += ir.interference;
        entry.1 += 1;
        // Conscious side
        let con_nbhd = system.get_neighborhood_for_occurrence(ir.con_ref);
        let entry = sums.entry(con_nbhd.id).or_insert((0.0, 0));
        entry.0 += ir.interference;
        entry.1 += 1;
    }
    sums.into_iter()
        .map(|(id, (sum, count))| (id, sum / count as f64))
        .collect()
}

fn score_neighborhoods(
    system: &mut DAESystem,
    refs: &[OccurrenceRef],
    is_conscious: bool,
    query_token_count: usize,
) -> HashMap<Uuid, ScoredNeighborhood> {
    // Pre-collect data to avoid borrow conflicts.
    // Superseded neighborhoods are excluded - they've been explicitly replaced.
    struct OccData {
        nbhd_id: Uuid,
        episode_idx: usize,
        neighborhood_idx: usize,
        word: String,
        activation_count: u32,
        plasticity: f64,
        nbhd_type: NeighborhoodType,
        epoch: u64,
    }

    let data: Vec<OccData> = refs
        .iter()
        .filter_map(|r| {
            let occ = system.get_occurrence(*r);
            let nbhd = system.get_neighborhood_for_occurrence(*r);
            if nbhd.superseded_by.is_some() {
                return None;
            }
            Some(OccData {
                nbhd_id: nbhd.id,
                episode_idx: if r.is_conscious() {
                    usize::MAX
                } else {
                    r.episode_idx
                },
                neighborhood_idx: r.neighborhood_idx,
                word: occ.word.to_lowercase(),
                activation_count: occ.activation_count,
                plasticity: occ.plasticity(),
                nbhd_type: nbhd.neighborhood_type,
                epoch: nbhd.epoch,
            })
        })
        .collect();

    // Pre-collect recency decay per episode_idx
    let recency_cache: HashMap<usize, f64> = data
        .iter()
        .map(|d| d.episode_idx)
        .collect::<HashSet<_>>()
        .into_iter()
        .map(|ep_idx| {
            let days = days_since_episode(system, ep_idx);
            let decay = 1.0 / (1.0 + days * RECENCY_DECAY_RATE);
            (ep_idx, decay)
        })
        .collect();

    // For conscious neighborhoods, compute recency boost based on position.
    // Later neighborhoods (higher index) were added more recently.
    let conscious_count = if data.iter().any(|d| d.episode_idx == usize::MAX) {
        system.conscious_episode.neighborhoods.len() as f64
    } else {
        1.0
    };
    let conscious_recency: HashMap<Uuid, f64> = if conscious_count > 1.0 {
        system
            .conscious_episode
            .neighborhoods
            .iter()
            .enumerate()
            .map(|(i, nbhd)| {
                // Newest neighborhood (last) gets boost 2.0, oldest gets 1.0
                let recency = 1.0 + (i as f64 / conscious_count);
                (nbhd.id, recency)
            })
            .collect()
    } else {
        HashMap::new()
    };

    let mut scored: HashMap<Uuid, ScoredNeighborhood> = HashMap::new();
    for d in &data {
        let weight = system.get_word_weight(&d.word);

        let entry = scored
            .entry(d.nbhd_id)
            .or_insert_with(|| ScoredNeighborhood {
                neighborhood_id: d.nbhd_id,
                episode_idx: d.episode_idx,
                neighborhood_idx: d.neighborhood_idx,
                score: 0.0,
                activated_count: 0,
                words: HashSet::new(),
                max_word_weight: 0.0,
                max_plasticity: 0.0,
                neighborhood_type: d.nbhd_type,
                epoch: d.epoch,
            });

        entry.score += weight * f64::from(d.activation_count);
        entry.words.insert(d.word.clone());
        entry.activated_count += 1;
        if weight > entry.max_word_weight {
            entry.max_word_weight = weight;
        }
        if d.plasticity > entry.max_plasticity {
            entry.max_plasticity = d.plasticity;
        }
    }

    // Post-process: density bonus, recency decay, then decision/preference competitive scoring
    for sn in scored.values_mut() {
        // Co-occurrence density bonus: neighborhoods matching more query tokens score higher
        if query_token_count > 0 {
            let density_bonus = sn.activated_count as f64 / query_token_count as f64;
            sn.score *= 1.0 + density_bonus;
        }
        // All neighborhoods get recency decay
        let decay = recency_cache.get(&sn.episode_idx).copied().unwrap_or(1.0);
        sn.score *= decay;
        // For conscious neighborhoods, apply recency boost (newer = higher score)
        if sn.episode_idx == usize::MAX {
            let boost = conscious_recency
                .get(&sn.neighborhood_id)
                .copied()
                .unwrap_or(1.0);
            sn.score *= boost;
        }
        // Decision/Preference: competitive scoring with floor
        // Decision/Preference types get a multiplier boost but no floor -
        // they must earn their score through genuine query overlap
        match sn.neighborhood_type {
            NeighborhoodType::Decision | NeighborhoodType::Preference => {
                sn.score *= DECISION_MULTIPLIER;
            }
            _ => {}
        }
    }

    // Gate conscious recall: require minimum query token overlap
    if is_conscious && query_token_count > 0 {
        scored.retain(|_, sn| {
            sn.activated_count as f64 / query_token_count as f64 >= CONSCIOUS_MIN_OVERLAP
        });
    }

    scored
}

/// Compute IDF-weighted word overlap between two word sets.
/// Returns sum(IDF(w)) for intersection / sum(IDF(w)) for union.
pub(crate) fn idf_weighted_overlap(
    words_a: &HashSet<String>,
    words_b: &HashSet<String>,
    system: &mut DAESystem,
) -> f64 {
    let intersection: f64 = words_a
        .intersection(words_b)
        .map(|w| system.get_word_weight(w))
        .sum();
    let union: f64 = words_a
        .union(words_b)
        .map(|w| system.get_word_weight(w))
        .sum();
    if union < f64::EPSILON {
        return 0.0;
    }
    intersection / union
}

/// Detect overlapping neighborhoods across conscious and subconscious scores
/// and suppress older ones. For each pair with IDF-weighted overlap above
/// `OVERLAP_THRESHOLD`, the lower-epoch neighborhood gets its score multiplied
/// by `OVERLAP_SUPPRESSION` (0.1x). This ensures that when contradicting memories
/// exist, only the newest version ranks highly.
fn overlap_suppress(
    con_scored: &mut HashMap<Uuid, ScoredNeighborhood>,
    sub_scored: &mut HashMap<Uuid, ScoredNeighborhood>,
    system: &mut DAESystem,
) {
    // Collect references to word sets and epochs - no cloning needed since
    // we only read words during pairwise comparison, then mutate scores after.
    let mut info: Vec<(Uuid, &HashSet<String>, u64)> = Vec::new();
    let mut seen: HashSet<Uuid> = HashSet::new();
    for (id, sn) in con_scored.iter().chain(sub_scored.iter()) {
        if seen.insert(*id) {
            info.push((*id, &sn.words, sn.epoch));
        }
    }

    if info.len() < 2 {
        return;
    }

    // Pairwise comparison - O(k^2) but k is bounded (top candidates only)
    let mut suppress: HashSet<Uuid> = HashSet::new();
    for i in 0..info.len() {
        for j in (i + 1)..info.len() {
            let overlap = idf_weighted_overlap(info[i].1, info[j].1, system);
            if overlap > OVERLAP_THRESHOLD {
                let epoch_i = info[i].2;
                let epoch_j = info[j].2;
                if epoch_i < epoch_j {
                    suppress.insert(info[i].0);
                } else if epoch_j < epoch_i {
                    suppress.insert(info[j].0);
                }
                // Same epoch: leave both unsuppressed
            }
        }
    }

    // Apply suppression factor to affected neighborhoods
    for id in &suppress {
        if let Some(sn) = con_scored.get_mut(id) {
            sn.score *= OVERLAP_SUPPRESSION;
        }
        if let Some(sn) = sub_scored.get_mut(id) {
            sn.score *= OVERLAP_SUPPRESSION;
        }
    }
}

/// Extract the text for a neighborhood via direct O(1) indexing.
///
/// Falls back to a linear scan if `neighborhood_idx` is out of bounds or
/// points to a different neighborhood (can happen if episodes were mutated
/// after index construction).
pub(crate) fn get_neighborhood_text(
    system: &DAESystem,
    neighborhood_id: Uuid,
    episode_idx: usize,
    neighborhood_idx: usize,
) -> String {
    fn extract_text(nbhd: &crate::neighborhood::Neighborhood) -> String {
        if nbhd.source_text.is_empty() {
            nbhd.occurrences
                .iter()
                .map(|o| o.word.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        } else {
            nbhd.source_text.clone()
        }
    }

    let episode = if episode_idx == usize::MAX {
        &system.conscious_episode
    } else {
        &system.episodes[episode_idx]
    };

    // Fast path: direct index
    if let Some(nbhd) = episode.neighborhoods.get(neighborhood_idx)
        && nbhd.id == neighborhood_id
    {
        return extract_text(nbhd);
    }

    // Fallback: linear scan (should rarely trigger)
    for nbhd in &episode.neighborhoods {
        if nbhd.id == neighborhood_id {
            return extract_text(nbhd);
        }
    }

    String::new()
}

pub(crate) fn get_episode_name(system: &DAESystem, episode_idx: usize) -> String {
    if episode_idx == usize::MAX {
        "Previously marked salient".to_string()
    } else {
        let ep = &system.episodes[episode_idx];
        if ep.name.is_empty() {
            "Memory".to_string()
        } else {
            ep.name.clone()
        }
    }
}
