use std::collections::{HashMap, HashSet};

use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::episode::Episode;
use crate::neighborhood::Neighborhood;
use crate::tokenizer::tokenize;

/// Reference to an occurrence by its location in the hierarchy.
/// (episode_idx, neighborhood_idx, occurrence_idx)
/// episode_idx: usize::MAX means conscious_episode, otherwise index into episodes vec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OccurrenceRef {
    pub episode_idx: usize,
    pub neighborhood_idx: usize,
    pub occurrence_idx: usize,
}

impl OccurrenceRef {
    pub fn is_conscious(&self) -> bool {
        self.episode_idx == usize::MAX
    }
}

/// Reference to a neighborhood by its location in the hierarchy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NeighborhoodRef {
    pub episode_idx: usize,
    pub neighborhood_idx: usize,
}

impl NeighborhoodRef {
    pub fn is_conscious(&self) -> bool {
        self.episode_idx == usize::MAX
    }
}

/// Result of activating a word across both manifolds.
pub struct ActivationResult {
    pub subconscious: Vec<OccurrenceRef>,
    pub conscious: Vec<OccurrenceRef>,
}

/// Top-level DAE system container with lazy-rebuilt indexes.
///
/// Episodes are the subconscious manifold. The conscious_episode is the
/// single conscious manifold. Indexes map words to their locations for
/// fast lookup during activation and IDF computation.
#[derive(Serialize, Deserialize)]
pub struct DAESystem {
    pub episodes: Vec<Episode>,
    pub conscious_episode: Episode,
    pub agent_name: String,

    #[serde(skip)]
    word_neighborhood_index: HashMap<String, HashSet<Uuid>>,
    #[serde(skip)]
    word_occurrence_index: HashMap<String, Vec<OccurrenceRef>>,
    #[serde(skip)]
    neighborhood_index: HashMap<Uuid, NeighborhoodRef>,
    #[serde(skip)]
    neighborhood_episode_index: HashMap<Uuid, usize>,
    #[serde(skip)]
    index_dirty: bool,
}

impl DAESystem {
    pub fn new(agent_name: &str) -> Self {
        Self {
            episodes: Vec::new(),
            conscious_episode: Episode::new_conscious(),
            agent_name: agent_name.to_string(),
            word_neighborhood_index: HashMap::new(),
            word_occurrence_index: HashMap::new(),
            neighborhood_index: HashMap::new(),
            neighborhood_episode_index: HashMap::new(),
            index_dirty: true,
        }
    }

    /// Total occurrence count across all episodes (both manifolds).
    pub fn n(&self) -> usize {
        let sub: usize = self.episodes.iter().map(|e| e.count()).sum();
        sub + self.conscious_episode.count()
    }

    /// Rebuild all indexes from scratch. Skips if not dirty.
    pub fn rebuild_indexes(&mut self) {
        if !self.index_dirty {
            return;
        }

        self.word_neighborhood_index.clear();
        self.word_occurrence_index.clear();
        self.neighborhood_index.clear();
        self.neighborhood_episode_index.clear();

        // Index subconscious episodes
        for (ep_idx, episode) in self.episodes.iter().enumerate() {
            for (n_idx, neighborhood) in episode.neighborhoods.iter().enumerate() {
                let n_ref = NeighborhoodRef {
                    episode_idx: ep_idx,
                    neighborhood_idx: n_idx,
                };
                self.neighborhood_index.insert(neighborhood.id, n_ref);
                self.neighborhood_episode_index
                    .insert(neighborhood.id, ep_idx);

                for (o_idx, occ) in neighborhood.occurrences.iter().enumerate() {
                    let word = occ.word.to_lowercase();
                    self.word_neighborhood_index
                        .entry(word.clone())
                        .or_default()
                        .insert(neighborhood.id);
                    self.word_occurrence_index
                        .entry(word)
                        .or_default()
                        .push(OccurrenceRef {
                            episode_idx: ep_idx,
                            neighborhood_idx: n_idx,
                            occurrence_idx: o_idx,
                        });
                }
            }
        }

        // Index conscious episode
        for (n_idx, neighborhood) in self.conscious_episode.neighborhoods.iter().enumerate() {
            let n_ref = NeighborhoodRef {
                episode_idx: usize::MAX,
                neighborhood_idx: n_idx,
            };
            self.neighborhood_index.insert(neighborhood.id, n_ref);
            self.neighborhood_episode_index
                .insert(neighborhood.id, usize::MAX);

            for (o_idx, occ) in neighborhood.occurrences.iter().enumerate() {
                let word = occ.word.to_lowercase();
                self.word_neighborhood_index
                    .entry(word.clone())
                    .or_default()
                    .insert(neighborhood.id);
                self.word_occurrence_index
                    .entry(word)
                    .or_default()
                    .push(OccurrenceRef {
                        episode_idx: usize::MAX,
                        neighborhood_idx: n_idx,
                        occurrence_idx: o_idx,
                    });
            }
        }

        self.index_dirty = false;
    }

    /// Ensure indexes are current.
    fn ensure_indexes(&mut self) {
        if self.index_dirty {
            self.rebuild_indexes();
        }
    }

    /// IDF weight: 1.0 / number of neighborhoods containing the word.
    pub fn get_word_weight(&mut self, word: &str) -> f64 {
        self.ensure_indexes();
        let word_lower = word.to_lowercase();
        match self.word_neighborhood_index.get(&word_lower) {
            Some(neighborhoods) if !neighborhoods.is_empty() => 1.0 / neighborhoods.len() as f64,
            _ => 1.0,
        }
    }

    /// Activate a word across both manifolds. Returns refs split by manifold.
    pub fn activate_word(&mut self, word: &str) -> ActivationResult {
        self.ensure_indexes();
        let word_lower = word.to_lowercase();

        let refs = match self.word_occurrence_index.get(&word_lower) {
            Some(refs) => refs.clone(),
            None => {
                return ActivationResult {
                    subconscious: vec![],
                    conscious: vec![],
                };
            }
        };

        let mut subconscious = Vec::new();
        let mut conscious = Vec::new();

        for occ_ref in refs {
            // Increment activation count
            let occ = self.get_occurrence_mut(occ_ref);
            occ.activate();

            if occ_ref.is_conscious() {
                conscious.push(occ_ref);
            } else {
                subconscious.push(occ_ref);
            }
        }

        ActivationResult {
            subconscious,
            conscious,
        }
    }

    /// Add text to the conscious episode. Tokenizes, creates neighborhood,
    /// pre-activates all occurrences once.
    pub fn add_to_conscious(&mut self, text: &str, rng: &mut impl Rng) -> Uuid {
        let tokens = tokenize(text);
        let mut neighborhood = Neighborhood::from_tokens(&tokens, None, text, rng);

        for occ in &mut neighborhood.occurrences {
            occ.activate();
        }

        let id = neighborhood.id;
        self.conscious_episode.add_neighborhood(neighborhood);
        self.index_dirty = true;
        id
    }

    /// Add a subconscious episode.
    pub fn add_episode(&mut self, episode: Episode) {
        self.episodes.push(episode);
        self.index_dirty = true;
    }

    /// Get immutable occurrence by ref.
    pub fn get_occurrence(&self, r: OccurrenceRef) -> &crate::occurrence::Occurrence {
        let episode = if r.is_conscious() {
            &self.conscious_episode
        } else {
            &self.episodes[r.episode_idx]
        };
        &episode.neighborhoods[r.neighborhood_idx].occurrences[r.occurrence_idx]
    }

    /// Get mutable occurrence by ref.
    pub fn get_occurrence_mut(&mut self, r: OccurrenceRef) -> &mut crate::occurrence::Occurrence {
        let episode = if r.is_conscious() {
            &mut self.conscious_episode
        } else {
            &mut self.episodes[r.episode_idx]
        };
        &mut episode.neighborhoods[r.neighborhood_idx].occurrences[r.occurrence_idx]
    }

    /// Get neighborhood by its UUID.
    pub fn get_neighborhood_ref(&mut self, id: Uuid) -> Option<NeighborhoodRef> {
        self.ensure_indexes();
        self.neighborhood_index.get(&id).copied()
    }

    /// Get neighborhood by ref.
    pub fn get_neighborhood(&self, r: NeighborhoodRef) -> &Neighborhood {
        let episode = if r.is_conscious() {
            &self.conscious_episode
        } else {
            &self.episodes[r.episode_idx]
        };
        &episode.neighborhoods[r.neighborhood_idx]
    }

    /// Get neighborhood that contains an occurrence.
    pub fn get_neighborhood_for_occurrence(&self, r: OccurrenceRef) -> &Neighborhood {
        let episode = if r.is_conscious() {
            &self.conscious_episode
        } else {
            &self.episodes[r.episode_idx]
        };
        &episode.neighborhoods[r.neighborhood_idx]
    }

    /// Get episode that contains an occurrence.
    pub fn get_episode_for_occurrence(&self, r: OccurrenceRef) -> &Episode {
        if r.is_conscious() {
            &self.conscious_episode
        } else {
            &self.episodes[r.episode_idx]
        }
    }

    /// Get episode index for a neighborhood UUID.
    pub fn get_episode_idx_for_neighborhood(&mut self, neighborhood_id: Uuid) -> Option<usize> {
        self.ensure_indexes();
        self.neighborhood_episode_index
            .get(&neighborhood_id)
            .copied()
    }

    /// Get the total number of neighborhoods across all episodes.
    pub fn total_neighborhoods(&self) -> usize {
        let sub: usize = self.episodes.iter().map(|e| e.neighborhoods.len()).sum();
        sub + self.conscious_episode.neighborhoods.len()
    }

    /// Mark indexes as needing rebuild.
    pub fn mark_dirty(&mut self) {
        self.index_dirty = true;
    }

    /// Get word occurrence refs (read-only, requires indexes to be current).
    pub fn get_word_occurrences(&mut self, word: &str) -> Vec<OccurrenceRef> {
        self.ensure_indexes();
        self.word_occurrence_index
            .get(&word.to_lowercase())
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::neighborhood::Neighborhood;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn make_system_with_data() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");

        // Episode 1: two neighborhoods
        let mut ep1 = Episode::new("episode1");
        let tokens1 = to_tokens(&["hello", "world"]);
        let n1 = Neighborhood::from_tokens(&tokens1, None, "hello world", &mut rng);
        ep1.add_neighborhood(n1);
        let tokens2 = to_tokens(&["hello", "rust"]);
        let n2 = Neighborhood::from_tokens(&tokens2, None, "hello rust", &mut rng);
        ep1.add_neighborhood(n2);
        sys.add_episode(ep1);

        // Conscious: one neighborhood
        sys.add_to_conscious("hello test", &mut rng);

        sys
    }

    #[test]
    fn test_n_counts_both_manifolds() {
        let sys = make_system_with_data();
        // subconscious: 2+2 = 4, conscious: 2 = total 6
        assert_eq!(sys.n(), 6);
    }

    #[test]
    fn test_lazy_index_rebuild() {
        let mut sys = make_system_with_data();
        // First access rebuilds
        let w = sys.get_word_weight("hello");
        assert!(w > 0.0);

        // Adding episode marks dirty
        let mut rng = rng();
        let mut ep = Episode::new("ep2");
        let tokens = to_tokens(&["new", "word"]);
        ep.add_neighborhood(Neighborhood::from_tokens(&tokens, None, "", &mut rng));
        sys.add_episode(ep);

        // Next access rebuilds again
        let w2 = sys.get_word_weight("new");
        assert!((w2 - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_idf_weight() {
        let mut sys = make_system_with_data();
        // "hello" in 3 neighborhoods (2 subconscious + 1 conscious)
        let w_hello = sys.get_word_weight("hello");
        assert!(
            (w_hello - 1.0 / 3.0).abs() < 1e-10,
            "expected 1/3, got {w_hello}"
        );

        // "rust" in 1 neighborhood
        let w_rust = sys.get_word_weight("rust");
        assert!((w_rust - 1.0).abs() < 1e-10);

        // unknown word
        let w_unknown = sys.get_word_weight("unknown");
        assert!((w_unknown - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_activate_word_partitions() {
        let mut sys = make_system_with_data();
        let result = sys.activate_word("hello");

        // "hello" appears in 2 subconscious neighborhoods, 1 conscious
        assert_eq!(result.subconscious.len(), 2);
        assert_eq!(result.conscious.len(), 1);

        // Verify activation counts incremented
        for r in &result.subconscious {
            assert_eq!(sys.get_occurrence(*r).activation_count, 1);
        }
        // Conscious occurrences were pre-activated in add_to_conscious (+1), then activated again (+1) = 2
        for r in &result.conscious {
            assert_eq!(sys.get_occurrence(*r).activation_count, 2);
        }
    }

    #[test]
    fn test_add_to_conscious_pre_activates() {
        let mut rng = rng();
        let mut sys = DAESystem::new("test");
        sys.add_to_conscious("hello world", &mut rng);

        // All conscious occurrences should have activation_count = 1
        for occ in sys.conscious_episode.all_occurrences() {
            assert_eq!(
                occ.activation_count, 1,
                "conscious occ '{}' not pre-activated",
                occ.word
            );
        }
    }

    #[test]
    fn test_get_neighborhood_for_occurrence() {
        let mut sys = make_system_with_data();
        let result = sys.activate_word("hello");

        let r = result.subconscious[0];
        let nbhd = sys.get_neighborhood_for_occurrence(r);
        assert!(nbhd.count() > 0);
    }

    #[test]
    fn test_get_episode_for_occurrence() {
        let mut sys = make_system_with_data();
        let result = sys.activate_word("hello");

        let sub_ep = sys.get_episode_for_occurrence(result.subconscious[0]);
        assert!(!sub_ep.is_conscious);

        let con_ep = sys.get_episode_for_occurrence(result.conscious[0]);
        assert!(con_ep.is_conscious);
    }
}
