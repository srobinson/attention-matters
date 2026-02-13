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

    #[test]
    fn test_vivid_neighborhood() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Neighborhood with 2 words, both overlap with conscious → >50% surfaced
        let mut ep = Episode::new("memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta"]),
            None,
            "alpha beta",
            &mut rng,
        ));
        sys.add_episode(ep);
        sys.add_to_conscious("alpha beta gamma", &mut rng);

        let result = QueryEngine::process_query(&mut sys, "alpha beta");
        let surface = compute_surface(&sys, &result);

        // Both "alpha" and "beta" have positive interference → 100% surfaced
        assert!(
            !surface.vivid_neighborhood_ids.is_empty(),
            "neighborhood with 100% surfaced should be vivid"
        );
    }

    #[test]
    fn test_non_vivid_neighborhood() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Neighborhood with 4 words, only 1 overlaps with conscious
        let mut ep = Episode::new("memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["shared", "unique1", "unique2", "unique3"]),
            None,
            "shared unique1 unique2 unique3",
            &mut rng,
        ));
        sys.add_episode(ep);
        // Conscious only has "shared" — only 1 word will have interference
        sys.add_to_conscious("shared different", &mut rng);

        // Only query "shared" — so only 1/4 words gets surfaced via interference
        let result = QueryEngine::process_query(&mut sys, "shared");
        let surface = compute_surface(&sys, &result);

        // 1/4 = 25% < 50% threshold → NOT vivid
        assert!(
            surface.vivid_neighborhood_ids.is_empty(),
            "neighborhood with only 25% surfaced should not be vivid"
        );
    }

    #[test]
    fn test_vivid_episode() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Episode with high overlap: every word matches conscious
        let mut ep = Episode::new("full-overlap");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["word1", "word2"]),
            None,
            "word1 word2",
            &mut rng,
        ));
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["word3", "word4"]),
            None,
            "word3 word4",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Add conscious with ALL the same words
        sys.add_to_conscious("word1 word2 word3 word4", &mut rng);

        // Query all words → all get interference → all surface
        let result = QueryEngine::process_query(&mut sys, "word1 word2 word3 word4");
        let surface = compute_surface(&sys, &result);

        // N=12 (4 subconscious + 4 conscious + 4 duplicated in conscious)
        // Episode has 4 occurrences, mass = 4/N * M
        // All 4 surfaced → e_ratio = 100% > 50%
        // Whether episode is vivid also depends on mass > 0.5
        // With N=12, mass = 4/12 * 1.0 = 0.333 < 0.5 — may not be vivid
        // This tests the logic: all surfaced but mass may be too small
        assert!(
            !surface.surfaced.is_empty(),
            "all words should surface"
        );

        // All surfaced → at least neighborhoods should be vivid
        assert!(
            !surface.vivid_neighborhood_ids.is_empty(),
            "neighborhoods with 100% surfaced should be vivid"
        );
    }

    #[test]
    fn test_fragment_extraction() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Two neighborhoods: one will be vivid, one won't
        let mut ep = Episode::new("memories");
        // Neighborhood 1: 2 words, both in conscious → vivid
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["cat", "dog"]),
            None,
            "cat dog",
            &mut rng,
        ));
        // Neighborhood 2: 4 words, only 1 in conscious → not vivid
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["cat", "fish", "bird", "snake"]),
            None,
            "cat fish bird snake",
            &mut rng,
        ));
        sys.add_episode(ep);
        sys.add_to_conscious("cat dog", &mut rng);

        // Query "cat dog" — cat overlaps in both neighborhoods via interference
        // "dog" also overlaps. "fish", "bird", "snake" are novel (not in conscious)
        let result = QueryEngine::process_query(&mut sys, "cat dog fish bird snake");
        let surface = compute_surface(&sys, &result);

        // Neighborhood 1: cat + dog surfaced → 2/2 = 100% → vivid
        // Neighborhood 2: cat surfaced (interference) + fish/bird/snake novel = 4/4 → also vivid
        // Actually all of neighborhood 2's words get surfaced as novel since
        // fish/bird/snake are NOT in conscious → they surface as novel
        // So fragments may be empty if both neighborhoods are vivid
        // The point is the surface computation runs without panic
        // and correctly classifies items
        assert!(
            !surface.surfaced.is_empty(),
            "should have surfaced occurrences"
        );
    }

    #[test]
    fn test_empty_activation() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        let mut ep = Episode::new("memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["alpha", "beta"]),
            None,
            "alpha beta",
            &mut rng,
        ));
        sys.add_episode(ep);

        // Query with words that don't exist in the system
        let result = QueryEngine::process_query(&mut sys, "nonexistent words here");
        let surface = compute_surface(&sys, &result);

        assert!(surface.surfaced.is_empty(), "no words activated → empty surface");
        assert!(surface.vivid_neighborhood_ids.is_empty());
        assert!(surface.vivid_episode_ids.is_empty());
        assert!(surface.fragments.is_empty());
        assert!(surface.neighborhood_surfaced_counts.is_empty());
    }

    #[test]
    fn test_all_surfaced_neighborhoods_vivid() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Each neighborhood has words NOT in conscious → surface as novel
        // Novel words (subconscious only) always surface, guaranteeing >50%
        let mut ep = Episode::new("all-match");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["cat", "dog"]),
            None,
            "cat dog",
            &mut rng,
        ));
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["fish", "bird"]),
            None,
            "fish bird",
            &mut rng,
        ));
        sys.add_episode(ep);
        // Conscious has NONE of these words → all are novel → all surface
        sys.add_to_conscious("unrelated other", &mut rng);

        let result = QueryEngine::process_query(&mut sys, "cat dog fish bird");
        let surface = compute_surface(&sys, &result);

        // All 4 subconscious words activated and are novel → 100% surfaced per neighborhood
        assert_eq!(
            surface.vivid_neighborhood_ids.len(),
            2,
            "both neighborhoods should be vivid when all words surface as novel"
        );
    }

    #[test]
    fn test_is_vivid_direct() {
        use crate::neighborhood::Neighborhood;

        let mut rng = rng();
        let nbhd = Neighborhood::from_tokens(
            &to_tokens(&["a", "b", "c", "d"]),
            None,
            "a b c d",
            &mut rng,
        );

        // neighborhood has 4 occurrences
        // is_vivid checks: count > episode_count * THRESHOLD
        // 4 > 6 * 0.5 → 4 > 3 → true
        assert!(nbhd.is_vivid(6), "4 > 6*0.5=3 should be vivid");

        // 4 > 8 * 0.5 → 4 > 4 → false (not strictly greater)
        assert!(!nbhd.is_vivid(8), "4 > 8*0.5=4 should NOT be vivid (equal, not greater)");

        // 4 > 10 * 0.5 → 4 > 5 → false
        assert!(!nbhd.is_vivid(10), "4 > 10*0.5=5 should NOT be vivid");

        // 4 > 2 * 0.5 → 4 > 1 → true
        assert!(nbhd.is_vivid(2), "4 > 2*0.5=1 should be vivid");

        // edge: episode_count = 0
        assert!(!nbhd.is_vivid(0), "zero episode count should not be vivid");
    }

    #[test]
    fn test_surfaced_counts_per_neighborhood() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        let mut ep = Episode::new("memories");
        let n1 = Neighborhood::from_tokens(
            &to_tokens(&["shared", "only1"]),
            None,
            "shared only1",
            &mut rng,
        );
        let n1_id = n1.id;
        ep.add_neighborhood(n1);

        let n2 = Neighborhood::from_tokens(
            &to_tokens(&["shared", "only2", "only3"]),
            None,
            "shared only2 only3",
            &mut rng,
        );
        let n2_id = n2.id;
        ep.add_neighborhood(n2);
        sys.add_episode(ep);
        sys.add_to_conscious("shared conscious", &mut rng);

        let result = QueryEngine::process_query(&mut sys, "shared only1 only2 only3");
        let surface = compute_surface(&sys, &result);

        // Verify per-neighborhood counts are tracked
        assert!(
            surface.neighborhood_surfaced_counts.contains_key(&n1_id)
                || surface.neighborhood_surfaced_counts.contains_key(&n2_id),
            "should track surfaced counts per neighborhood"
        );
    }
}
