use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::constants::M;
use crate::neighborhood::Neighborhood;
use crate::time::now_iso8601;

/// A collection of neighborhoods representing one document or conversation segment.
/// The system has multiple subconscious episodes plus one conscious episode.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Episode {
    pub id: Uuid,
    pub name: String,
    pub is_conscious: bool,
    pub timestamp: String,
    pub neighborhoods: Vec<Neighborhood>,
}

impl Episode {
    pub fn new(name: &str) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.to_string(),
            is_conscious: false,
            timestamp: now_iso8601(),
            neighborhoods: Vec::new(),
        }
    }

    pub fn new_conscious() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: "conscious".to_string(),
            is_conscious: true,
            timestamp: now_iso8601(),
            neighborhoods: Vec::new(),
        }
    }

    pub fn add_neighborhood(&mut self, neighborhood: Neighborhood) {
        self.neighborhoods.push(neighborhood);
    }

    /// Total occurrence count across all neighborhoods.
    pub fn count(&self) -> usize {
        self.neighborhoods.iter().map(|n| n.count()).sum()
    }

    /// Total activation across all neighborhoods.
    pub fn total_activation(&self) -> u32 {
        self.neighborhoods
            .iter()
            .map(|n| n.total_activation())
            .sum()
    }

    /// Episode mass: count/N * M
    pub fn mass(&self, n: usize) -> f64 {
        if n == 0 {
            return 0.0;
        }
        (self.count() as f64 / n as f64) * M
    }

    /// Display name for context composition output.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            "Memory"
        } else {
            &self.name
        }
    }

    /// Iterate over all occurrences across all neighborhoods.
    pub fn all_occurrences(&self) -> impl Iterator<Item = &crate::occurrence::Occurrence> {
        self.neighborhoods.iter().flat_map(|n| n.occurrences.iter())
    }

    /// Mutable iteration over all occurrences.
    pub fn all_occurrences_mut(
        &mut self,
    ) -> impl Iterator<Item = &mut crate::occurrence::Occurrence> {
        self.neighborhoods
            .iter_mut()
            .flat_map(|n| n.occurrences.iter_mut())
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

    fn make_episode(name: &str, word_counts: &[usize]) -> Episode {
        let mut rng = rng();
        let mut ep = Episode::new(name);
        for &wc in word_counts {
            let tokens: Vec<String> = (0..wc).map(|i| format!("word{i}")).collect();
            let n = Neighborhood::from_tokens(&tokens, None, "", &mut rng);
            ep.add_neighborhood(n);
        }
        ep
    }

    #[test]
    fn test_count_across_neighborhoods() {
        let ep = make_episode("test", &[3, 5, 2]);
        assert_eq!(ep.count(), 10);
    }

    #[test]
    fn test_mass() {
        let ep = make_episode("test", &[10]);
        let m = ep.mass(100);
        assert!((m - 0.1).abs() < 1e-10);
    }

    #[test]
    fn test_all_occurrences() {
        let ep = make_episode("test", &[3, 2]);
        let all: Vec<_> = ep.all_occurrences().collect();
        assert_eq!(all.len(), 5);
    }

    #[test]
    fn test_display_name() {
        let ep = Episode::new("My Episode");
        assert_eq!(ep.display_name(), "My Episode");

        let ep2 = Episode::new("");
        assert_eq!(ep2.display_name(), "Memory");
    }

    #[test]
    fn test_conscious_episode() {
        let ep = Episode::new_conscious();
        assert!(ep.is_conscious);
    }

    #[test]
    fn test_serde_roundtrip() {
        let ep = make_episode("test ep", &[2, 3]);
        let json = serde_json::to_string(&ep).unwrap();
        let ep2: Episode = serde_json::from_str(&json).unwrap();
        assert_eq!(ep.count(), ep2.count());
        assert_eq!(ep.name, ep2.name);
    }

    #[test]
    fn test_total_activation() {
        let mut ep = make_episode("test", &[2]);
        for occ in ep.all_occurrences_mut() {
            occ.activation_count = 5;
        }
        assert_eq!(ep.total_activation(), 10);
    }
}
