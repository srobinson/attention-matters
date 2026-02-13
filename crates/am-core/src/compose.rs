use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use rand::Rng;
use regex::Regex;
use uuid::Uuid;

use crate::query::{InterferenceResult, QueryResult};
use crate::surface::SurfaceResult;
use crate::system::{DAESystem, OccurrenceRef};

/// Metrics about the composed context.
pub struct ContextMetrics {
    pub conscious: u32,
    pub subconscious: u32,
    pub novel: u32,
}

/// Result of context composition.
pub struct ContextResult {
    pub context: String,
    pub metrics: ContextMetrics,
}

/// Internal scored neighborhood for ranking.
struct ScoredNeighborhood {
    neighborhood_id: Uuid,
    episode_idx: usize, // usize::MAX for conscious
    score: f64,
    activated_count: usize,
    words: HashSet<String>,
    max_word_weight: f64,
    max_plasticity: f64,
}

/// Compose human-readable context from surface and activation results.
pub fn compose_context(
    system: &mut DAESystem,
    _surface: &SurfaceResult,
    query_result: &QueryResult,
    _interference: &[InterferenceResult],
) -> ContextResult {
    let mut parts: Vec<String> = Vec::new();
    let mut metrics = ContextMetrics {
        conscious: 0,
        subconscious: 0,
        novel: 0,
    };
    let mut selected_ids: HashSet<Uuid> = HashSet::new();

    // Build conscious word set
    let conscious_words: HashSet<String> = query_result
        .activation
        .conscious
        .iter()
        .map(|r| system.get_occurrence(*r).word.to_lowercase())
        .collect();

    // Step 1: Score conscious neighborhoods
    let con_scored = score_neighborhoods(system, &query_result.activation.conscious, true);

    // Step 2: Score subconscious neighborhoods
    let sub_scored = score_neighborhoods(system, &query_result.activation.subconscious, false);

    // Step 3: CONSCIOUS RECALL (top 1)
    let mut con_ranked: Vec<_> = con_scored.values().collect();
    con_ranked.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    if let Some(best) = con_ranked.first() {
        selected_ids.insert(best.neighborhood_id);
        let text = get_neighborhood_text(system, best.neighborhood_id, best.episode_idx);

        parts.push("CONSCIOUS RECALL:".to_string());
        parts.push("[Source: Previously marked salient]".to_string());
        parts.push(format!("\"{}\"", text));
        metrics.conscious = 1;
    }

    // Step 4: SUBCONSCIOUS RECALL (top 2)
    let mut sub_ranked: Vec<_> = sub_scored
        .values()
        .filter(|s| !selected_ids.contains(&s.neighborhood_id))
        .collect();
    sub_ranked.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap());

    for (i, entry) in sub_ranked.iter().take(2).enumerate() {
        selected_ids.insert(entry.neighborhood_id);
        let text = get_neighborhood_text(system, entry.neighborhood_id, entry.episode_idx);
        let ep_name = get_episode_name(system, entry.episode_idx);

        if !parts.is_empty() {
            parts.push(String::new());
        }
        parts.push(format!("SUBCONSCIOUS RECALL {}:", i + 1));
        parts.push(format!("[Source: {}]", ep_name));
        parts.push(format!("\"{}\"", text));
        metrics.subconscious += 1;
    }

    // Step 5: NOVEL CONNECTION (top 1 by novelty score)
    let novel_candidates: Vec<_> = sub_scored
        .values()
        .filter(|s| {
            if selected_ids.contains(&s.neighborhood_id) {
                return false;
            }
            if s.activated_count > 2 {
                return false;
            }
            // No words in common with conscious
            !s.words.iter().any(|w| conscious_words.contains(w))
        })
        .collect();

    if let Some(best_novel) = novel_candidates.iter().max_by(|a, b| {
        let novelty_a =
            a.max_word_weight * a.max_plasticity * (1.0 / a.activated_count.max(1) as f64);
        let novelty_b =
            b.max_word_weight * b.max_plasticity * (1.0 / b.activated_count.max(1) as f64);
        novelty_a.partial_cmp(&novelty_b).unwrap()
    }) {
        selected_ids.insert(best_novel.neighborhood_id);
        let text =
            get_neighborhood_text(system, best_novel.neighborhood_id, best_novel.episode_idx);
        let ep_name = get_episode_name(system, best_novel.episode_idx);

        if !parts.is_empty() {
            parts.push(String::new());
        }
        parts.push("NOVEL CONNECTION:".to_string());
        parts.push(format!("[Source: {}]", ep_name));
        parts.push(format!("\"{}\"", text));
        metrics.novel = 1;
    }

    ContextResult {
        context: parts.join("\n"),
        metrics,
    }
}

fn score_neighborhoods(
    system: &mut DAESystem,
    refs: &[OccurrenceRef],
    _is_conscious: bool,
) -> HashMap<Uuid, ScoredNeighborhood> {
    let mut scored: HashMap<Uuid, ScoredNeighborhood> = HashMap::new();

    // Pre-collect data to avoid borrow conflicts
    let data: Vec<(Uuid, usize, String, u32)> = refs
        .iter()
        .map(|r| {
            let occ = system.get_occurrence(*r);
            let nbhd = system.get_neighborhood_for_occurrence(*r);
            (
                nbhd.id,
                if r.is_conscious() { usize::MAX } else { r.episode_idx },
                occ.word.to_lowercase(),
                occ.activation_count,
            )
        })
        .collect();

    for (nbhd_id, ep_idx, word, activation_count) in &data {
        let weight = system.get_word_weight(word);
        let plasticity = 1.0 / (1.0 + (1.0 + *activation_count as f64).ln());

        let entry = scored.entry(*nbhd_id).or_insert_with(|| ScoredNeighborhood {
            neighborhood_id: *nbhd_id,
            episode_idx: *ep_idx,
            score: 0.0,
            activated_count: 0,
            words: HashSet::new(),
            max_word_weight: 0.0,
            max_plasticity: 0.0,
        });

        entry.score += weight * *activation_count as f64;
        entry.words.insert(word.clone());
        entry.activated_count += 1;
        if weight > entry.max_word_weight {
            entry.max_word_weight = weight;
        }
        if plasticity > entry.max_plasticity {
            entry.max_plasticity = plasticity;
        }
    }

    scored
}

fn get_neighborhood_text(system: &DAESystem, neighborhood_id: Uuid, episode_idx: usize) -> String {
    let episode = if episode_idx == usize::MAX {
        &system.conscious_episode
    } else {
        &system.episodes[episode_idx]
    };

    for nbhd in &episode.neighborhoods {
        if nbhd.id == neighborhood_id {
            if !nbhd.source_text.is_empty() {
                return nbhd.source_text.clone();
            }
            return nbhd
                .occurrences
                .iter()
                .map(|o| o.word.as_str())
                .collect::<Vec<_>>()
                .join(" ");
        }
    }

    String::new()
}

fn get_episode_name(system: &DAESystem, episode_idx: usize) -> String {
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

static SALIENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<salient>(.*?)</salient>").unwrap());

/// Extract salient-tagged content and add to conscious episode.
pub fn extract_salient(system: &mut DAESystem, text: &str, rng: &mut impl Rng) -> u32 {
    let mut count = 0u32;
    for cap in SALIENT_RE.captures_iter(text) {
        if let Some(content) = cap.get(1) {
            system.add_to_conscious(content.as_str(), rng);
            count += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::episode::Episode;
    use crate::neighborhood::Neighborhood;
    use crate::query::QueryEngine;
    use crate::surface::compute_surface;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn make_full_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Subconscious memories
        let mut ep = Episode::new("Science memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics", "particle", "wave"]),
            None,
            "quantum physics particle wave",
            &mut rng,
        ));
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["neural", "network", "deep", "learning"]),
            None,
            "neural network deep learning",
            &mut rng,
        ));
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["biology", "cell", "membrane", "protein"]),
            None,
            "biology cell membrane protein",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Conscious
        sys.add_to_conscious("quantum computing research", &mut rng);

        sys
    }

    #[test]
    fn test_compose_includes_recall_types() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum physics neural");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference);

        assert!(ctx.context.contains("CONSCIOUS RECALL:"));
        assert!(ctx.context.contains("SUBCONSCIOUS RECALL"));
    }

    #[test]
    fn test_absent_types_omitted() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Only subconscious, no conscious overlap
        let mut ep = Episode::new("memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta"]),
            None, "alpha beta", &mut rng,
        ));
        sys.add_episode(ep);

        let result = QueryEngine::process_query(&mut sys, "alpha");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference);

        // No conscious recall since no conscious content matches
        assert!(!ctx.context.contains("CONSCIOUS RECALL:"));
    }

    #[test]
    fn test_metrics() {
        let mut sys = make_full_system();
        let result = QueryEngine::process_query(&mut sys, "quantum");
        let surface = compute_surface(&sys, &result);
        let ctx = compose_context(&mut sys, &surface, &result, &result.interference);

        assert!(ctx.metrics.conscious <= 1);
        assert!(ctx.metrics.subconscious <= 2);
        assert!(ctx.metrics.novel <= 1);
    }

    #[test]
    fn test_extract_salient_basic() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let count = extract_salient(
            &mut sys,
            "Normal text <salient>important insight</salient> more text",
            &mut rng,
        );
        assert_eq!(count, 1);
        assert_eq!(sys.conscious_episode.neighborhoods.len(), 1);
    }

    #[test]
    fn test_extract_salient_multiline() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let count = extract_salient(
            &mut sys,
            "Text <salient>line one\nline two</salient> rest",
            &mut rng,
        );
        assert_eq!(count, 1);
    }

    #[test]
    fn test_extract_salient_multiple() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        let count = extract_salient(
            &mut sys,
            "<salient>first</salient> gap <salient>second</salient>",
            &mut rng,
        );
        assert_eq!(count, 2);
        assert_eq!(sys.conscious_episode.neighborhoods.len(), 2);
    }

    #[test]
    fn test_deterministic_scoring() {
        let mut sys1 = make_full_system();
        let result1 = QueryEngine::process_query(&mut sys1, "quantum");
        let surface1 = compute_surface(&sys1, &result1);
        let ctx1 = compose_context(&mut sys1, &surface1, &result1, &result1.interference);

        let mut sys2 = make_full_system();
        let result2 = QueryEngine::process_query(&mut sys2, "quantum");
        let surface2 = compute_surface(&sys2, &result2);
        let ctx2 = compose_context(&mut sys2, &surface2, &result2, &result2.interference);

        assert_eq!(ctx1.context, ctx2.context);
    }
}
