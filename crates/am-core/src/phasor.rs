use serde::{Deserialize, Serialize};

use crate::constants::GOLDEN_ANGLE;

/// Phase angle on the unit circle, representing temporal/frequency position.
/// Golden-angle spacing ensures maximal separation between successive phasors.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct DaemonPhasor {
    pub theta: f64,
}

impl DaemonPhasor {
    /// Create a phasor with the given phase angle, normalized to [0, 2π).
    pub fn new(theta: f64) -> Self {
        Self {
            theta: theta.rem_euclid(std::f64::consts::TAU),
        }
    }

    /// Create phasor from index using golden-angle spacing.
    /// theta = base_theta + index * GOLDEN_ANGLE
    pub fn from_index(index: usize, base_theta: f64) -> Self {
        Self::new(base_theta + index as f64 * GOLDEN_ANGLE)
    }

    /// Phasor interference: cos(self.theta - other.theta).
    /// Range: [-1, +1]. +1 = in phase, -1 = out of phase.
    pub fn interference(self, other: Self) -> f64 {
        (self.theta - other.theta).cos()
    }

    /// Circular interpolation along the shortest arc.
    pub fn slerp(self, other: Self, t: f64) -> Self {
        let mut diff = other.theta - self.theta;
        // Wrap to [-π, π] for shortest path
        while diff > std::f64::consts::PI {
            diff -= std::f64::consts::TAU;
        }
        while diff < -std::f64::consts::PI {
            diff += std::f64::consts::TAU;
        }
        Self::new(self.theta + t * diff)
    }
}

impl PartialEq for DaemonPhasor {
    fn eq(&self, other: &Self) -> bool {
        (self.theta - other.theta).abs() < crate::constants::EPSILON
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_phase_normalization() {
        let p = DaemonPhasor::new(-1.0);
        assert!(p.theta >= 0.0 && p.theta < std::f64::consts::TAU);

        let p2 = DaemonPhasor::new(10.0);
        assert!(p2.theta >= 0.0 && p2.theta < std::f64::consts::TAU);
    }

    #[test]
    fn test_golden_angle_spacing() {
        let p0 = DaemonPhasor::from_index(0, 0.0);
        let p1 = DaemonPhasor::from_index(1, 0.0);
        let diff = (p1.theta - p0.theta).abs();
        assert!(
            (diff - GOLDEN_ANGLE).abs() < 1e-10,
            "expected golden angle spacing: got {diff}"
        );
    }

    #[test]
    fn test_interference_in_phase() {
        let a = DaemonPhasor::new(1.0);
        let b = DaemonPhasor::new(1.0);
        assert!((a.interference(b) - 1.0).abs() < 1e-10);
    }

    #[test]
    fn test_interference_out_of_phase() {
        let a = DaemonPhasor::new(0.0);
        let b = DaemonPhasor::new(std::f64::consts::PI);
        assert!((a.interference(b) - (-1.0)).abs() < 1e-10);
    }

    #[test]
    fn test_interference_orthogonal() {
        let a = DaemonPhasor::new(0.0);
        let b = DaemonPhasor::new(std::f64::consts::FRAC_PI_2);
        assert!(a.interference(b).abs() < 1e-10);
    }

    #[test]
    fn test_slerp_endpoints() {
        let a = DaemonPhasor::new(0.5);
        let b = DaemonPhasor::new(2.0);
        assert_eq!(a.slerp(b, 0.0), a);
        assert_eq!(a.slerp(b, 1.0), b);
    }

    #[test]
    fn test_slerp_shortest_arc() {
        // From 0.1 to 6.0 (near 2π) should go backward, not forward
        let a = DaemonPhasor::new(0.1);
        let b = DaemonPhasor::new(6.0);
        let mid = a.slerp(b, 0.5);
        // Midpoint should be near 0/2π, not near 3
        let dist_from_zero = mid.theta.min(std::f64::consts::TAU - mid.theta);
        assert!(
            dist_from_zero < 1.0,
            "expected near 0/2π, got theta = {}",
            mid.theta
        );
    }

    #[test]
    fn test_golden_angle_maximizes_separation() {
        let phasors: Vec<DaemonPhasor> = (0..10).map(|i| DaemonPhasor::from_index(i, 0.0)).collect();
        for i in 0..phasors.len() {
            for j in (i + 1)..phasors.len() {
                let mut diff = (phasors[i].theta - phasors[j].theta).abs();
                if diff > std::f64::consts::PI {
                    diff = std::f64::consts::TAU - diff;
                }
                assert!(
                    diff > 0.25,
                    "phasors {i} and {j} too close: {diff} rad"
                );
            }
        }
    }
}
