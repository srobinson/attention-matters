use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::constants::{M, THRESHOLD};
use crate::phasor::DaemonPhasor;
use crate::quaternion::Quaternion;

/// A single word instance positioned on the S³ manifold.
///
/// Each occurrence has a position (quaternion), phase (phasor), and activation
/// count tracking how many times it has been referenced by queries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Occurrence {
    pub word: String,
    pub position: Quaternion,
    pub phasor: DaemonPhasor,
    pub activation_count: u32,
    pub id: Uuid,
    pub neighborhood_id: Uuid,
}

impl Occurrence {
    pub fn new(word: String, position: Quaternion, phasor: DaemonPhasor, neighborhood_id: Uuid) -> Self {
        Self {
            word,
            position,
            phasor,
            activation_count: 0,
            id: Uuid::new_v4(),
            neighborhood_id,
        }
    }

    /// Increment activation count.
    pub fn activate(&mut self) {
        self.activation_count += 1;
    }

    /// OpenClaw drift rate formula: ratio / THRESHOLD, capped at 0.
    /// Fresh words (c=0) don't drift. Drift increases with activation
    /// until anchored at c/C > THRESHOLD.
    pub fn drift_rate(&self, container_activation: u32) -> f64 {
        if container_activation == 0 {
            return 0.0;
        }
        let ratio = self.activation_count as f64 / container_activation as f64;
        if ratio > THRESHOLD {
            return 0.0;
        }
        ratio / THRESHOLD
    }

    /// Plasticity: 1 / (1 + ln(1 + c))
    /// Diminishing returns — each activation contributes less.
    pub fn plasticity(&self) -> f64 {
        1.0 / (1.0 + (1.0 + self.activation_count as f64).ln())
    }

    /// Whether this occurrence is anchored (drift rate = 0).
    pub fn is_anchored(&self, container_activation: u32) -> bool {
        if container_activation == 0 {
            return true;
        }
        (self.activation_count as f64 / container_activation as f64) > THRESHOLD
    }

    /// Mass contribution: activation_count / N * M
    pub fn mass(&self, n: usize) -> f64 {
        if n == 0 {
            return 0.0;
        }
        (self.activation_count as f64 / n as f64) * M
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_occ(word: &str, activation_count: u32) -> Occurrence {
        let mut occ = Occurrence::new(
            word.to_string(),
            Quaternion::identity(),
            DaemonPhasor::new(0.0),
            Uuid::new_v4(),
        );
        occ.activation_count = activation_count;
        occ
    }

    #[test]
    fn test_activate() {
        let mut occ = make_occ("hello", 0);
        assert_eq!(occ.activation_count, 0);
        occ.activate();
        assert_eq!(occ.activation_count, 1);
        occ.activate();
        assert_eq!(occ.activation_count, 2);
    }

    #[test]
    fn test_plasticity_values() {
        let cases = [
            (0, 1.0),
            (1, 0.591),
            (10, 0.294),
            (100, 0.178),
        ];

        for (c, expected) in cases {
            let occ = make_occ("test", c);
            let p = occ.plasticity();
            assert!(
                (p - expected).abs() < 0.001,
                "plasticity(c={c}): expected {expected}, got {p}"
            );
        }
    }

    #[test]
    fn test_drift_rate_fresh_zero() {
        let occ = make_occ("test", 0);
        assert_eq!(occ.drift_rate(10), 0.0);
    }

    #[test]
    fn test_drift_rate_increases() {
        let occ1 = make_occ("test", 1);
        let occ2 = make_occ("test", 3);
        let d1 = occ1.drift_rate(10);
        let d2 = occ2.drift_rate(10);
        assert!(d2 > d1, "drift should increase with activation: {d1} vs {d2}");
    }

    #[test]
    fn test_drift_rate_anchored() {
        let occ = make_occ("test", 6);
        assert_eq!(occ.drift_rate(10), 0.0);
    }

    #[test]
    fn test_drift_rate_at_threshold() {
        // c/C = 0.5 exactly → ratio == THRESHOLD, NOT > THRESHOLD, so still drifting
        // drift_rate = 0.5 / 0.5 = 1.0
        let occ = make_occ("test", 5);
        assert!((occ.drift_rate(10) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_drift_rate_just_below_threshold() {
        let occ = make_occ("test", 4);
        let rate = occ.drift_rate(10);
        assert!((rate - 0.8).abs() < 1e-10, "expected 0.8, got {rate}");
    }

    #[test]
    fn test_is_anchored() {
        assert!(make_occ("test", 6).is_anchored(10));
        assert!(!make_occ("test", 4).is_anchored(10));
    }

    #[test]
    fn test_mass() {
        let occ = make_occ("test", 10);
        let m = occ.mass(100);
        assert!((m - 0.1).abs() < 1e-10);
    }

    #[test]
    fn test_drift_rate_zero_container() {
        let occ = make_occ("test", 5);
        assert_eq!(occ.drift_rate(0), 0.0);
    }
}
