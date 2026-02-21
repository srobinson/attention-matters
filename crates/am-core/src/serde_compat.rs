//! JSON serde for the v0.7.2 wire format.
//!
//! The wire format uses camelCase field names and stores quaternions as
//! `[w, x, y, z]` arrays and phasors as bare f64 theta values.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::episode::Episode;
use crate::neighborhood::{Neighborhood, NeighborhoodType};
use crate::occurrence::Occurrence;
use crate::phasor::DaemonPhasor;
use crate::quaternion::Quaternion;
use crate::system::DAESystem;

pub const CURRENT_VERSION: &str = "0.7.2";

// --- Wire format types ---

#[derive(Serialize, Deserialize, Debug)]
pub struct WireExport {
    pub version: String,
    pub timestamp: String,
    pub system: WireSystem,
    #[serde(rename = "conversationBuffer", default)]
    pub conversation_buffer: Vec<Vec<String>>,
    #[serde(rename = "conversationHistory", default)]
    pub conversation_history: Vec<ConversationMessage>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct ConversationMessage {
    pub role: String,
    pub content: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WireSystem {
    pub episodes: Vec<WireEpisode>,
    #[serde(rename = "consciousEpisode")]
    pub conscious_episode: WireEpisode,
    #[serde(rename = "N", default)]
    pub n: usize,
    #[serde(rename = "totalActivation", default)]
    pub total_activation: u64,
    #[serde(rename = "agentName", default)]
    pub agent_name: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WireEpisode {
    pub name: String,
    #[serde(rename = "isConscious", default)]
    pub is_conscious: bool,
    pub id: String,
    #[serde(default)]
    pub timestamp: String,
    pub neighborhoods: Vec<WireNeighborhood>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WireNeighborhood {
    pub seed: [f64; 4],
    pub id: String,
    #[serde(rename = "sourceText", default)]
    pub source_text: String,
    #[serde(rename = "neighborhoodType", default)]
    pub neighborhood_type: String,
    pub occurrences: Vec<WireOccurrence>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct WireOccurrence {
    pub word: String,
    pub position: [f64; 4],
    /// Phase angle — accepts both "phasor" and "theta" field names.
    #[serde(alias = "theta")]
    pub phasor: f64,
    #[serde(rename = "activationCount", default)]
    pub activation_count: u32,
    #[serde(rename = "neighborhoodId", default)]
    pub neighborhood_id: String,
}

// --- Conversion: Wire → Domain ---

impl WireExport {
    /// Convert wire format to domain DAESystem.
    pub fn into_system(self) -> DAESystem {
        let mut sys = DAESystem::new(&self.system.agent_name);

        // Convert subconscious episodes
        for wire_ep in self.system.episodes {
            sys.add_episode(wire_episode_to_domain(wire_ep));
        }

        // Convert conscious episode
        sys.conscious_episode = wire_episode_to_domain(self.system.conscious_episode);
        sys.conscious_episode.is_conscious = true;

        sys.mark_dirty();
        sys
    }

    /// Create wire export from domain DAESystem.
    pub fn from_system(system: &DAESystem) -> Self {
        let conscious = domain_episode_to_wire(&system.conscious_episode);
        let episodes: Vec<WireEpisode> =
            system.episodes.iter().map(domain_episode_to_wire).collect();

        let total_activation: u64 = system
            .episodes
            .iter()
            .map(|e| e.total_activation() as u64)
            .sum::<u64>()
            + system.conscious_episode.total_activation() as u64;

        WireExport {
            version: CURRENT_VERSION.to_string(),
            timestamp: String::new(),
            system: WireSystem {
                episodes,
                conscious_episode: conscious,
                n: system.n(),
                total_activation,
                agent_name: system.agent_name.clone(),
            },
            conversation_buffer: Vec::new(),
            conversation_history: Vec::new(),
        }
    }
}

fn wire_episode_to_domain(wire: WireEpisode) -> Episode {
    let mut ep = Episode::new(&wire.name);
    ep.id = Uuid::parse_str(&wire.id).unwrap_or_else(|_| Uuid::new_v4());
    ep.is_conscious = wire.is_conscious;
    ep.timestamp = wire.timestamp;

    for wire_nbhd in wire.neighborhoods {
        ep.add_neighborhood(wire_neighborhood_to_domain(wire_nbhd));
    }

    ep
}

fn wire_neighborhood_to_domain(wire: WireNeighborhood) -> Neighborhood {
    let seed = Quaternion::from_array(wire.seed);
    let mut nbhd = Neighborhood::new(seed, wire.source_text);
    nbhd.id = Uuid::parse_str(&wire.id).unwrap_or_else(|_| Uuid::new_v4());
    nbhd.neighborhood_type = NeighborhoodType::from_str_lossy(&wire.neighborhood_type);

    for wire_occ in wire.occurrences {
        let mut occ = Occurrence::new(
            wire_occ.word,
            Quaternion::from_array(wire_occ.position),
            DaemonPhasor::new(wire_occ.phasor),
            nbhd.id,
        );
        occ.activation_count = wire_occ.activation_count;
        if let Ok(id) = Uuid::parse_str(&wire_occ.neighborhood_id) {
            occ.neighborhood_id = id;
        }
        nbhd.occurrences.push(occ);
    }

    nbhd
}

fn domain_episode_to_wire(ep: &Episode) -> WireEpisode {
    WireEpisode {
        name: ep.name.clone(),
        is_conscious: ep.is_conscious,
        id: ep.id.to_string(),
        timestamp: ep.timestamp.clone(),
        neighborhoods: ep
            .neighborhoods
            .iter()
            .map(domain_neighborhood_to_wire)
            .collect(),
    }
}

fn domain_neighborhood_to_wire(nbhd: &Neighborhood) -> WireNeighborhood {
    WireNeighborhood {
        seed: nbhd.seed.to_array(),
        id: nbhd.id.to_string(),
        source_text: nbhd.source_text.clone(),
        neighborhood_type: nbhd.neighborhood_type.as_str().to_string(),
        occurrences: nbhd
            .occurrences
            .iter()
            .map(|occ| WireOccurrence {
                word: occ.word.clone(),
                position: occ.position.to_array(),
                phasor: occ.phasor.theta,
                activation_count: occ.activation_count,
                neighborhood_id: occ.neighborhood_id.to_string(),
            })
            .collect(),
    }
}

/// Deserialize a v0.7.2 JSON export into a DAESystem.
pub fn import_json(json: &str) -> Result<DAESystem, serde_json::Error> {
    let wire: WireExport = serde_json::from_str(json)?;
    Ok(wire.into_system())
}

/// Serialize a DAESystem to v0.7.2 JSON wire format.
pub fn export_json(system: &DAESystem) -> Result<String, serde_json::Error> {
    let wire = WireExport::from_system(system);
    serde_json::to_string_pretty(&wire)
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
        words.iter().map(|s| s.to_string()).collect()
    }

    fn make_test_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test-agent");

        let mut ep = Episode::new("memories");
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["hello", "world"]),
            None,
            "hello world",
            &mut rng,
        ));
        ep.add_neighborhood(Neighborhood::from_tokens(
            &to_tokens(&["rust", "is", "great"]),
            None,
            "rust is great",
            &mut rng,
        ));
        sys.add_episode(ep);
        sys.add_to_conscious("test conscious", &mut rng);

        sys
    }

    #[test]
    fn test_roundtrip() {
        let sys = make_test_system();
        let json = export_json(&sys).unwrap();
        let sys2 = import_json(&json).unwrap();

        assert_eq!(sys.n(), sys2.n());
        assert_eq!(sys.episodes.len(), sys2.episodes.len());
        assert_eq!(sys.agent_name, sys2.agent_name);

        // Check occurrence words match
        let words1: Vec<String> = sys.episodes[0]
            .all_occurrences()
            .map(|o| o.word.clone())
            .collect();
        let words2: Vec<String> = sys2.episodes[0]
            .all_occurrences()
            .map(|o| o.word.clone())
            .collect();
        assert_eq!(words1, words2);
    }

    #[test]
    fn test_version_field() {
        let sys = make_test_system();
        let json = export_json(&sys).unwrap();
        let wire: WireExport = serde_json::from_str(&json).unwrap();
        assert_eq!(wire.version, CURRENT_VERSION);
    }

    #[test]
    fn test_theta_alias() {
        // Simulate older format using "theta" instead of "phasor"
        let json = r#"{
            "version": "0.7.2",
            "timestamp": "",
            "system": {
                "episodes": [{
                    "name": "test",
                    "isConscious": false,
                    "id": "00000000-0000-0000-0000-000000000001",
                    "timestamp": "",
                    "neighborhoods": [{
                        "seed": [1.0, 0.0, 0.0, 0.0],
                        "id": "00000000-0000-0000-0000-000000000002",
                        "sourceText": "hello",
                        "occurrences": [{
                            "word": "hello",
                            "position": [1.0, 0.0, 0.0, 0.0],
                            "theta": 1.234,
                            "activationCount": 5,
                            "neighborhoodId": "00000000-0000-0000-0000-000000000002"
                        }]
                    }]
                }],
                "consciousEpisode": {
                    "name": "conscious",
                    "isConscious": true,
                    "id": "00000000-0000-0000-0000-000000000003",
                    "neighborhoods": []
                },
                "agentName": "echo"
            }
        }"#;

        let sys = import_json(json).unwrap();
        let occ = &sys.episodes[0].neighborhoods[0].occurrences[0];
        assert_eq!(occ.word, "hello");
        assert!((occ.phasor.theta - 1.234).abs() < 1e-10);
        assert_eq!(occ.activation_count, 5);
    }

    #[test]
    fn test_conversation_fields() {
        let sys = make_test_system();
        let json = export_json(&sys).unwrap();
        let wire: WireExport = serde_json::from_str(&json).unwrap();

        // Should have empty conversation fields (not missing)
        assert!(wire.conversation_buffer.is_empty());
        assert!(wire.conversation_history.is_empty());
    }

    #[test]
    fn test_n_and_activation_in_export() {
        let sys = make_test_system();
        let wire = WireExport::from_system(&sys);
        assert_eq!(wire.system.n, sys.n());
        assert_eq!(wire.system.n, sys.n());
    }

    #[test]
    fn test_position_quaternion_roundtrip() {
        let sys = make_test_system();
        let json = export_json(&sys).unwrap();
        let sys2 = import_json(&json).unwrap();

        let pos1 = sys.episodes[0].neighborhoods[0].occurrences[0].position;
        let pos2 = sys2.episodes[0].neighborhoods[0].occurrences[0].position;

        assert!(
            pos1.angular_distance(pos2) < 1e-10,
            "quaternion position not preserved in roundtrip"
        );
    }

    #[test]
    fn test_phasor_roundtrip() {
        let sys = make_test_system();
        let json = export_json(&sys).unwrap();
        let sys2 = import_json(&json).unwrap();

        let p1 = sys.episodes[0].neighborhoods[0].occurrences[0].phasor.theta;
        let p2 = sys2.episodes[0].neighborhoods[0].occurrences[0]
            .phasor
            .theta;

        assert!(
            (p1 - p2).abs() < 1e-10,
            "phasor theta not preserved: {p1} vs {p2}"
        );
    }
}
