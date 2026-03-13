//! Property-based tests for quaternion and phasor geometric invariants.

use std::f64::consts::{PI, TAU};

use am_core::{DaemonPhasor, Quaternion};
use proptest::prelude::*;

const EPSILON: f64 = 1e-10;

/// Strategy producing unit quaternions via the random() constructor.
fn arb_unit_quaternion() -> impl Strategy<Value = Quaternion> {
    // Use 4 arbitrary f64s in [-1, 1] and normalize
    (
        prop::num::f64::NORMAL.prop_map(|v| v.rem_euclid(2.0) - 1.0),
        prop::num::f64::NORMAL.prop_map(|v| v.rem_euclid(2.0) - 1.0),
        prop::num::f64::NORMAL.prop_map(|v| v.rem_euclid(2.0) - 1.0),
        prop::num::f64::NORMAL.prop_map(|v| v.rem_euclid(2.0) - 1.0),
    )
        .prop_filter("non-zero quaternion", |(w, x, y, z)| {
            w * w + x * x + y * y + z * z > 1e-20
        })
        .prop_map(|(w, x, y, z)| Quaternion::new(w, x, y, z).normalize())
}

/// Strategy for t in [0, 1].
fn arb_t() -> impl Strategy<Value = f64> {
    (0u32..=1000u32).prop_map(|v| f64::from(v) / 1000.0)
}

/// Strategy for arbitrary theta values.
fn arb_theta() -> impl Strategy<Value = f64> {
    prop::num::f64::NORMAL.prop_map(|v| v.rem_euclid(TAU * 10.0) - TAU * 5.0)
}

fn norm(q: Quaternion) -> f64 {
    (q.w * q.w + q.x * q.x + q.y * q.y + q.z * q.z).sqrt()
}

fn approx_eq(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() < eps
}

fn quat_approx_eq(a: Quaternion, b: Quaternion, eps: f64) -> bool {
    // Quaternions q and -q represent the same rotation on S3
    let direct = (a.w - b.w).abs() < eps
        && (a.x - b.x).abs() < eps
        && (a.y - b.y).abs() < eps
        && (a.z - b.z).abs() < eps;
    let negated = (a.w + b.w).abs() < eps
        && (a.x + b.x).abs() < eps
        && (a.y + b.y).abs() < eps
        && (a.z + b.z).abs() < eps;
    direct || negated
}

// --- Quaternion invariants ---

proptest! {
    /// 1. random() via normalize always produces a unit quaternion.
    #[test]
    fn random_produces_unit_quaternion(q in arb_unit_quaternion()) {
        let n = norm(q);
        prop_assert!((n - 1.0).abs() < EPSILON, "norm was {n}, expected 1.0");
    }

    /// 2. slerp(q1, q2, 0.0) equals q1.
    #[test]
    fn slerp_at_zero_returns_start(q1 in arb_unit_quaternion(), q2 in arb_unit_quaternion()) {
        let result = Quaternion::slerp(q1, q2, 0.0);
        prop_assert!(
            quat_approx_eq(result, q1, 1e-6),
            "slerp(q1, q2, 0.0) = ({}, {}, {}, {}) != q1 = ({}, {}, {}, {})",
            result.w, result.x, result.y, result.z,
            q1.w, q1.x, q1.y, q1.z
        );
    }

    /// 3. slerp(q1, q2, 1.0) equals q2.
    #[test]
    fn slerp_at_one_returns_end(q1 in arb_unit_quaternion(), q2 in arb_unit_quaternion()) {
        let result = Quaternion::slerp(q1, q2, 1.0);
        prop_assert!(
            quat_approx_eq(result, q2, 1e-6),
            "slerp(q1, q2, 1.0) = ({}, {}, {}, {}) != q2 = ({}, {}, {}, {})",
            result.w, result.x, result.y, result.z,
            q2.w, q2.x, q2.y, q2.z
        );
    }

    /// 4. slerp always produces a unit quaternion for any t in [0, 1].
    #[test]
    fn slerp_preserves_unit(q1 in arb_unit_quaternion(), q2 in arb_unit_quaternion(), t in arb_t()) {
        let result = Quaternion::slerp(q1, q2, t);
        let n = norm(result);
        prop_assert!((n - 1.0).abs() < 1e-6, "slerp norm was {n} at t={t}");
    }

    /// 5. angular_distance(q, q) == 0 for any unit quaternion.
    /// Tolerance is 1e-7 due to f64 precision in acos near 1.0.
    #[test]
    fn self_distance_is_zero(q in arb_unit_quaternion()) {
        let d = q.angular_distance(q);
        prop_assert!(d.abs() < 1e-7, "self-distance was {d}");
    }

    /// 6. Triangle inequality: dist(a,c) <= dist(a,b) + dist(b,c).
    #[test]
    fn triangle_inequality(
        a in arb_unit_quaternion(),
        b in arb_unit_quaternion(),
        c in arb_unit_quaternion()
    ) {
        let ab = a.angular_distance(b);
        let bc = b.angular_distance(c);
        let ac = a.angular_distance(c);
        // Generous epsilon for floating point accumulation in acos near boundary values
        prop_assert!(
            ac <= ab + bc + 1e-6,
            "triangle inequality violated: dist(a,c)={ac} > dist(a,b)={ab} + dist(b,c)={bc}"
        );
    }

    /// 7. Hamilton product with identity is identity: q * identity == q.
    #[test]
    fn hamilton_product_identity(q in arb_unit_quaternion()) {
        let identity = Quaternion::identity();
        let result = q * identity;
        prop_assert!(
            quat_approx_eq(result, q, EPSILON),
            "q * identity != q"
        );
    }

    // --- Phasor invariants ---

    /// 8. new() normalization keeps theta in [0, 2pi).
    #[test]
    fn phasor_theta_normalized(theta in arb_theta()) {
        let p = DaemonPhasor::new(theta);
        prop_assert!(
            p.theta >= 0.0 && p.theta < TAU,
            "theta {} not in [0, 2pi) for input {theta}",
            p.theta
        );
    }

    /// 9. In-phase interference returns cos(0) == 1.0.
    #[test]
    fn in_phase_interference(theta in arb_theta()) {
        let p = DaemonPhasor::new(theta);
        let interference = p.interference(p);
        prop_assert!(
            approx_eq(interference, 1.0, 1e-10),
            "in-phase interference was {interference}, expected 1.0"
        );
    }

    /// 10. Anti-phase interference returns cos(pi) == -1.0.
    #[test]
    fn anti_phase_interference(theta in arb_theta()) {
        let p1 = DaemonPhasor::new(theta);
        let p2 = DaemonPhasor::new(theta + PI);
        let interference = p1.interference(p2);
        prop_assert!(
            approx_eq(interference, -1.0, 1e-10),
            "anti-phase interference was {interference}, expected -1.0"
        );
    }
}
