use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::constants::{M, NEIGHBORHOOD_RADIUS, THRESHOLD};
use crate::occurrence::Occurrence;
use crate::phasor::DaemonPhasor;
use crate::quaternion::Quaternion;

/// Classification of a neighborhood's content.
/// Decisions and preferences get special treatment in scoring and composition.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NeighborhoodType {
    /// Default: ordinary memory from conversations or documents.
    #[default]
    Memory,
    /// A settled decision that should not be re-litigated.
    Decision,
    /// A user preference that should be respected.
    Preference,
    /// A marked insight (default for am_salient without prefix).
    Insight,
}

impl NeighborhoodType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Decision => "decision",
            Self::Preference => "preference",
            Self::Insight => "insight",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "decision" => Self::Decision,
            "preference" => Self::Preference,
            "insight" => Self::Insight,
            _ => Self::Memory,
        }
    }
}

/// A cluster of word occurrences around a seed position on S³.
/// Represents a chunk of text (typically 3 sentences).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Neighborhood {
    pub id: Uuid,
    pub seed: Quaternion,
    pub occurrences: Vec<Occurrence>,
    pub source_text: String,
    #[serde(default)]
    pub neighborhood_type: NeighborhoodType,
}

impl Neighborhood {
    pub fn new(seed: Quaternion, source_text: String) -> Self {
        Self {
            id: Uuid::new_v4(),
            seed,
            occurrences: Vec::new(),
            source_text,
            neighborhood_type: NeighborhoodType::default(),
        }
    }

    /// Create a neighborhood from tokens, placing each word within
    /// NEIGHBORHOOD_RADIUS of the seed with golden-angle phasor spacing.
    pub fn from_tokens(
        tokens: &[String],
        seed: Option<Quaternion>,
        source_text: &str,
        rng: &mut impl Rng,
    ) -> Self {
        let seed = seed.unwrap_or_else(|| Quaternion::random(rng));
        let mut neighborhood = Self::new(seed, source_text.to_string());

        for (i, token) in tokens.iter().enumerate() {
            let position = Quaternion::random_near(seed, NEIGHBORHOOD_RADIUS, rng);
            let phasor = DaemonPhasor::from_index(i, 0.0);
            let occ = Occurrence::new(token.clone(), position, phasor, neighborhood.id);
            neighborhood.occurrences.push(occ);
        }

        neighborhood
    }

    pub fn count(&self) -> usize {
        self.occurrences.len()
    }

    pub fn total_activation(&self) -> u32 {
        self.occurrences.iter().map(|o| o.activation_count).sum()
    }

    pub fn mass(&self, n: usize) -> f64 {
        if n == 0 {
            return 0.0;
        }
        (self.count() as f64 / n as f64) * M
    }

    /// Vivid if more than THRESHOLD of occurrences are activated relative to episode count.
    pub fn is_vivid(&self, episode_count: usize) -> bool {
        if episode_count == 0 {
            return false;
        }
        self.count() as f64 > episode_count as f64 * THRESHOLD
    }

    /// Activate all occurrences matching `word` (case-insensitive). Returns count activated.
    pub fn activate_word(&mut self, word: &str) -> Vec<usize> {
        let word_lower = word.to_lowercase();
        let mut activated = Vec::new();
        for (i, occ) in self.occurrences.iter_mut().enumerate() {
            if occ.word.to_lowercase() == word_lower {
                occ.activate();
                activated.push(i);
            }
        }
        activated
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_from_tokens_placement() {
        let mut rng = rng();
        let tokens = to_tokens(&["hello", "world", "test"]);
        let n = Neighborhood::from_tokens(&tokens, None, "hello world test", &mut rng);

        assert_eq!(n.count(), 3);
        for occ in &n.occurrences {
            let dist = n.seed.angular_distance(occ.position);
            assert!(
                dist <= NEIGHBORHOOD_RADIUS + 0.01,
                "occurrence outside radius: {dist} > {NEIGHBORHOOD_RADIUS}"
            );
        }
    }

    #[test]
    fn test_golden_angle_phasor_spacing() {
        let mut rng = rng();
        let tokens: Vec<String> = (0..5).map(|i| format!("word{i}")).collect();
        let n = Neighborhood::from_tokens(&tokens, None, "", &mut rng);

        for i in 0..4 {
            let diff = n.occurrences[i + 1].phasor.theta - n.occurrences[i].phasor.theta;
            let wrapped = diff.rem_euclid(std::f64::consts::TAU);
            let golden = crate::constants::GOLDEN_ANGLE;
            assert!(
                (wrapped - golden).abs() < 1e-10
                    || (wrapped - golden + std::f64::consts::TAU).abs() < 1e-10,
                "phasor spacing not golden angle at index {i}: {wrapped}"
            );
        }
    }

    #[test]
    fn test_mass_hierarchy() {
        let mut rng = rng();
        let tokens = to_tokens(&["a", "b", "c"]);
        let n = Neighborhood::from_tokens(&tokens, None, "a b c", &mut rng);

        let total_n = 100;
        let neighborhood_mass = n.mass(total_n);
        let expected = 3.0 / 100.0 * M;
        assert!((neighborhood_mass - expected).abs() < 1e-10);
    }

    #[test]
    fn test_activate_word() {
        let mut rng = rng();
        let tokens = to_tokens(&["hello", "world", "hello"]);
        let mut n = Neighborhood::from_tokens(&tokens, None, "hello world hello", &mut rng);

        let activated = n.activate_word("hello");
        assert_eq!(activated.len(), 2);
        assert_eq!(n.occurrences[0].activation_count, 1);
        assert_eq!(n.occurrences[1].activation_count, 0);
        assert_eq!(n.occurrences[2].activation_count, 1);
    }

    #[test]
    fn test_total_activation() {
        let mut rng = rng();
        let tokens = to_tokens(&["a", "b"]);
        let mut n = Neighborhood::from_tokens(&tokens, None, "a b", &mut rng);
        n.occurrences[0].activation_count = 3;
        n.occurrences[1].activation_count = 7;
        assert_eq!(n.total_activation(), 10);
    }

    #[test]
    fn test_serde_roundtrip() {
        let mut rng = rng();
        let tokens = to_tokens(&["hello", "world"]);
        let n = Neighborhood::from_tokens(&tokens, None, "hello world", &mut rng);

        let json = serde_json::to_string(&n).unwrap();
        let n2: Neighborhood = serde_json::from_str(&json).unwrap();
        assert_eq!(n.count(), n2.count());
        assert_eq!(n.occurrences[0].word, n2.occurrences[0].word);
    }
}
