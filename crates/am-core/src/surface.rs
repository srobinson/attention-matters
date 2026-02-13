use std::collections::{HashMap, HashSet};

use uuid::Uuid;

use crate::constants::THRESHOLD;
use crate::query::QueryResult;
use crate::system::{DAESystem, OccurrenceRef};

/// Surfaced content from interference and novelty analysis.
pub struct SurfaceResult {
    /// Isolated surfaced occurrences not in vivid structures.
    pub fragments: Vec<OccurrenceRef>,
    /// Neighborhoods where >50% of occurrences surfaced.
    pub vivid_neighborhood_ids: HashSet<Uuid>,
    /// Episodes where >50% activated AND >50% mass.
    pub vivid_episode_ids: HashSet<Uuid>,
    /// All surfaced occurrence refs.
    pub surfaced: HashSet<OccurrenceRef>,
    /// Per-neighborhood count of surfaced occurrences.
    pub neighborhood_surfaced_counts: HashMap<Uuid, usize>,
}

/// Compute which content surfaces from activation and interference.
pub fn compute_surface(system: &DAESystem, query_result: &QueryResult) -> SurfaceResult {
    let n = system.n();
    let mut surfaced: HashSet<OccurrenceRef> = HashSet::new();

    // Step 1: Occurrences with positive interference
    for ir in &query_result.interference {
        if ir.interference > 0.0 {
            surfaced.insert(ir.sub_ref);
        }
    }

    // Step 2: Novel occurrences (words in subconscious but NOT in conscious)
    let conscious_words: HashSet<String> = query_result
        .activation
        .conscious
        .iter()
        .map(|r| system.get_occurrence(*r).word.to_lowercase())
        .collect();

    for r in &query_result.activation.subconscious {
        let word = system.get_occurrence(*r).word.to_lowercase();
        if !conscious_words.contains(&word) {
            surfaced.insert(*r);
        }
    }

    // Step 3: Group surfaced by neighborhood
    let mut neighborhood_surfaced_counts: HashMap<Uuid, usize> = HashMap::new();
    for r in &surfaced {
        let nbhd = system.get_neighborhood_for_occurrence(*r);
        *neighborhood_surfaced_counts.entry(nbhd.id).or_default() += 1;
    }

    // Step 4: Identify vivid neighborhoods and episodes
    let mut vivid_neighborhood_ids: HashSet<Uuid> = HashSet::new();
    let mut vivid_episode_ids: HashSet<Uuid> = HashSet::new();

    for episode in &system.episodes {
        let mut episode_activated = 0usize;

        for neighborhood in &episode.neighborhoods {
            let n_activated = neighborhood_surfaced_counts
                .get(&neighborhood.id)
                .copied()
                .unwrap_or(0);
            episode_activated += n_activated;

            if neighborhood.count() > 0 {
                let ratio = n_activated as f64 / neighborhood.count() as f64;
                if ratio > THRESHOLD {
                    vivid_neighborhood_ids.insert(neighborhood.id);
                }
            }
        }

        if episode.count() > 0 && n > 0 {
            let e_ratio = episode_activated as f64 / episode.count() as f64;
            if e_ratio > THRESHOLD && episode.mass(n) > THRESHOLD {
                vivid_episode_ids.insert(episode.id);
            }
        }
    }

    // Step 5: Fragments — surfaced but not in vivid structures
    let fragments: Vec<OccurrenceRef> = surfaced
        .iter()
        .filter(|r| {
            let nbhd = system.get_neighborhood_for_occurrence(**r);
            if vivid_neighborhood_ids.contains(&nbhd.id) {
                return false;
            }
            let ep = system.get_episode_for_occurrence(**r);
            !vivid_episode_ids.contains(&ep.id)
        })
        .copied()
        .collect();

    SurfaceResult {
        fragments,
        vivid_neighborhood_ids,
        vivid_episode_ids,
        surfaced,
        neighborhood_surfaced_counts,
    }
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

    #[test]
    fn test_positive_interference_surfaces() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        let mut ep = Episode::new("memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics"]),
            None,
            "quantum physics",
            &mut rng,
        ));
        sys.add_episode(ep);
        sys.add_to_conscious("quantum mechanics", &mut rng);

        let result = QueryEngine::process_query(&mut sys, "quantum");
        let surface = compute_surface(&sys, &result);

        // "quantum" is in both manifolds — should have interference results
        assert!(!surface.surfaced.is_empty());
    }

    #[test]
    fn test_novel_words_surface() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        let mut ep = Episode::new("memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["quantum", "physics", "novel"]),
            None,
            "quantum physics novel",
            &mut rng,
        ));
        sys.add_episode(ep);
        sys.add_to_conscious("quantum mechanics", &mut rng);

        let result = QueryEngine::process_query(&mut sys, "quantum physics novel");
        let surface = compute_surface(&sys, &result);

        // "novel" is only in subconscious — should be surfaced as novel
        let novel_surfaced = surface
            .surfaced
            .iter()
            .any(|r| sys.get_occurrence(*r).word == "novel");
        assert!(novel_surfaced, "novel word should be surfaced");
    }
}
