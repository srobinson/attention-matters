use std::ops::Mul;

use rand::Rng;
use serde::{Deserialize, Serialize};

use crate::constants::{EPSILON, SLERP_THRESHOLD};

/// Unit quaternion representing a point on S³.
///
/// Always normalized. Represents rotations and positions on the 3-sphere.
/// Antipodal quaternions (q and -q) represent the same rotation but different
/// points on S³ — the geodesic distance function handles this via abs(dot).
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct Quaternion {
    pub w: f64,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl PartialEq for Quaternion {
    fn eq(&self, other: &Self) -> bool {
        (self.w - other.w).abs() < EPSILON
            && (self.x - other.x).abs() < EPSILON
            && (self.y - other.y).abs() < EPSILON
            && (self.z - other.z).abs() < EPSILON
    }
}

impl Quaternion {
    /// Create a new quaternion, automatically normalized.
    pub fn new(w: f64, x: f64, y: f64, z: f64) -> Self {
        Self { w, x, y, z }.normalize()
    }

    /// Identity quaternion (1, 0, 0, 0).
    pub fn identity() -> Self {
        Self {
            w: 1.0,
            x: 0.0,
            y: 0.0,
            z: 0.0,
        }
    }

    /// Normalize to unit length. Returns identity if near-zero magnitude.
    pub fn normalize(self) -> Self {
        let norm = (self.w * self.w + self.x * self.x + self.y * self.y + self.z * self.z).sqrt();
        if norm < EPSILON {
            return Self::identity();
        }
        Self {
            w: self.w / norm,
            x: self.x / norm,
            y: self.y / norm,
            z: self.z / norm,
        }
    }

    /// 4D dot product.
    pub fn dot(self, other: Self) -> f64 {
        self.w * other.w + self.x * other.x + self.y * other.y + self.z * other.z
    }

    /// Geodesic distance on S³. Range: [0, π].
    /// Uses abs(dot) to handle antipodal equivalence.
    pub fn angular_distance(self, other: Self) -> f64 {
        let d = self.dot(other).abs().clamp(-1.0, 1.0);
        2.0 * d.acos()
    }

    /// Spherical linear interpolation with antipodal flip and NLERP fallback.
    pub fn slerp(self, other: Self, t: f64) -> Self {
        if t <= 0.0 {
            return self;
        }
        if t >= 1.0 {
            return other;
        }

        let mut dot = self.dot(other);
        let o;

        // Take shorter arc
        if dot < 0.0 {
            o = Self {
                w: -other.w,
                x: -other.x,
                y: -other.y,
                z: -other.z,
            };
            dot = -dot;
        } else {
            o = other;
        }

        // Near-parallel: NLERP fallback
        if dot > SLERP_THRESHOLD {
            return Self {
                w: self.w + t * (o.w - self.w),
                x: self.x + t * (o.x - self.x),
                y: self.y + t * (o.y - self.y),
                z: self.z + t * (o.z - self.z),
            }
            .normalize();
        }

        let theta = dot.clamp(-1.0, 1.0).acos();
        let sin_theta = theta.sin();

        let s0 = ((1.0 - t) * theta).sin() / sin_theta;
        let s1 = (t * theta).sin() / sin_theta;

        Self {
            w: s0 * self.w + s1 * o.w,
            x: s0 * self.x + s1 * o.x,
            y: s0 * self.y + s1 * o.y,
            z: s0 * self.z + s1 * o.z,
        }
        .normalize()
    }

    /// Uniform random quaternion on S³ using Shoemake's method.
    pub fn random(rng: &mut impl Rng) -> Self {
        let s1: f64 = rng.random();
        let t1 = std::f64::consts::TAU * rng.random::<f64>();
        let t2 = std::f64::consts::TAU * rng.random::<f64>();

        let r1 = (1.0 - s1).sqrt();
        let r2 = s1.sqrt();

        Self {
            w: r1 * t1.sin(),
            x: r1 * t1.cos(),
            y: r2 * t2.sin(),
            z: r2 * t2.cos(),
        }
        .normalize()
    }

    /// Random quaternion within `angular_radius` of `center` on S³.
    /// Uses Gaussian-distributed rotation axis and sqrt-corrected angle
    /// for uniform area distribution on the spherical cap.
    pub fn random_near(center: Self, angular_radius: f64, rng: &mut impl Rng) -> Self {
        // Random axis via Gaussian samples (Box-Muller)
        let ax = gauss_random(rng);
        let ay = gauss_random(rng);
        let az = gauss_random(rng);
        let ax_norm = (ax * ax + ay * ay + az * az).sqrt();

        if ax_norm < EPSILON {
            return center;
        }

        let ax = ax / ax_norm;
        let ay = ay / ax_norm;
        let az = az / ax_norm;

        // sqrt for uniform area distribution on spherical cap
        let angle = angular_radius * rng.random::<f64>().sqrt();
        let half_angle = angle / 2.0;
        let sin_half = half_angle.sin();
        let cos_half = half_angle.cos();

        // Rotation quaternion
        let rotation = Self {
            w: cos_half,
            x: ax * sin_half,
            y: ay * sin_half,
            z: az * sin_half,
        };

        // Hamilton product: rotation * center
        (rotation * center).normalize()
    }

    /// Convert to [w, x, y, z] array for serialization.
    pub fn to_array(self) -> [f64; 4] {
        [self.w, self.x, self.y, self.z]
    }

    /// Create from [w, x, y, z] array.
    pub fn from_array(arr: [f64; 4]) -> Self {
        Self::new(arr[0], arr[1], arr[2], arr[3])
    }

}

impl std::ops::Neg for Quaternion {
    type Output = Self;

    fn neg(self) -> Self {
        Self {
            w: -self.w,
            x: -self.x,
            y: -self.y,
            z: -self.z,
        }
    }
}

/// Hamilton product (quaternion multiplication).
impl Mul for Quaternion {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self {
        Self {
            w: self.w * rhs.w - self.x * rhs.x - self.y * rhs.y - self.z * rhs.z,
            x: self.w * rhs.x + self.x * rhs.w + self.y * rhs.z - self.z * rhs.y,
            y: self.w * rhs.y - self.x * rhs.z + self.y * rhs.w + self.z * rhs.x,
            z: self.w * rhs.z + self.x * rhs.y - self.y * rhs.x + self.z * rhs.w,
        }
    }
}

/// Box-Muller transform for generating Gaussian-distributed random numbers.
fn gauss_random(rng: &mut impl Rng) -> f64 {
    // Clamp u1 away from 0 to avoid ln(0) = -inf
    let u1: f64 = rng.random::<f64>().max(f64::MIN_POSITIVE);
    let u2: f64 = rng.random();
    (-2.0 * u1.ln()).sqrt() * (std::f64::consts::TAU * u2).cos()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn assert_unit(q: Quaternion) {
        let norm = (q.w * q.w + q.x * q.x + q.y * q.y + q.z * q.z).sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-10,
            "quaternion not unit: norm = {norm}"
        );
    }

    fn assert_approx_eq(a: Quaternion, b: Quaternion, tol: f64) {
        // Check both q and -q (antipodal equivalence for rotations)
        let direct = (a.w - b.w).abs().max((a.x - b.x).abs()).max((a.y - b.y).abs()).max((a.z - b.z).abs());
        let antipodal = (a.w + b.w).abs().max((a.x + b.x).abs()).max((a.y + b.y).abs()).max((a.z + b.z).abs());
        let min_diff = direct.min(antipodal);
        assert!(
            min_diff < tol,
            "quaternions not approx equal: {a:?} vs {b:?} (min_diff = {min_diff})"
        );
    }

    #[test]
    fn test_normalize() {
        let q = Quaternion::new(2.0, 0.0, 0.0, 0.0);
        assert_unit(q);
        assert!((q.w - 1.0).abs() < EPSILON);
    }

    #[test]
    fn test_normalize_near_zero() {
        let q = Quaternion::new(0.0, 0.0, 0.0, 0.0);
        assert_eq!(q, Quaternion::identity());
    }

    #[test]
    fn test_dot_product() {
        let a = Quaternion::identity();
        let b = Quaternion::identity();
        assert!((a.dot(b) - 1.0).abs() < EPSILON);

        let c = Quaternion::new(0.0, 1.0, 0.0, 0.0);
        assert!(a.dot(c).abs() < EPSILON);
    }

    #[test]
    fn test_angular_distance_identity() {
        let a = Quaternion::identity();
        let b = Quaternion::identity();
        assert!(a.angular_distance(b) < EPSILON);
    }

    #[test]
    fn test_angular_distance_antipodal() {
        let a = Quaternion::identity();
        let b = -a;
        // Antipodal points are distance 0 (abs(dot) = 1)
        assert!(a.angular_distance(b) < EPSILON);
    }

    #[test]
    fn test_angular_distance_orthogonal() {
        let a = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let b = Quaternion::new(0.0, 1.0, 0.0, 0.0);
        let dist = a.angular_distance(b);
        assert!(
            (dist - std::f64::consts::PI).abs() < 0.01,
            "expected ~π, got {dist}"
        );
    }

    #[test]
    fn test_slerp_endpoints() {
        let mut rng = rng();
        let a = Quaternion::random(&mut rng);
        let b = Quaternion::random(&mut rng);

        let s0 = a.slerp(b, 0.0);
        let s1 = a.slerp(b, 1.0);

        assert_approx_eq(s0, a, 1e-10);
        assert_approx_eq(s1, b, 1e-10);
    }

    #[test]
    fn test_slerp_identity() {
        let mut rng = rng();
        let q = Quaternion::random(&mut rng);

        // SLERP(q, q, t) ≈ q for any t
        for t in [0.0, 0.25, 0.5, 0.75, 1.0] {
            let result = q.slerp(q, t);
            assert_approx_eq(result, q, 1e-10);
        }
    }

    #[test]
    fn test_slerp_midpoint() {
        let a = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let b = Quaternion::new(0.707, 0.707, 0.0, 0.0);
        let mid = a.slerp(b, 0.5);
        assert_unit(mid);

        // Midpoint should be equidistant from both endpoints
        let da = a.angular_distance(mid);
        let db = mid.angular_distance(b);
        assert!(
            (da - db).abs() < 0.01,
            "midpoint not equidistant: {da} vs {db}"
        );
    }

    #[test]
    fn test_random_unit() {
        let mut rng = rng();
        for _ in 0..100 {
            let q = Quaternion::random(&mut rng);
            assert_unit(q);
        }
    }

    #[test]
    fn test_random_near_within_radius() {
        let mut rng = rng();
        let center = Quaternion::random(&mut rng);
        let radius = 1.0; // ~57 degrees

        for _ in 0..200 {
            let q = Quaternion::random_near(center, radius, &mut rng);
            assert_unit(q);
            let dist = center.angular_distance(q);
            assert!(
                dist <= radius + 0.01,
                "random_near exceeded radius: {dist} > {radius}"
            );
        }
    }

    #[test]
    fn test_hamilton_product_identity() {
        let mut rng = rng();
        let q = Quaternion::random(&mut rng);
        let id = Quaternion::identity();

        let result = q * id;
        assert_approx_eq(result, q, 1e-10);

        let result2 = id * q;
        assert_approx_eq(result2, q, 1e-10);
    }

    #[test]
    fn test_hamilton_product_associative() {
        let mut rng = rng();
        let a = Quaternion::random(&mut rng);
        let b = Quaternion::random(&mut rng);
        let c = Quaternion::random(&mut rng);

        let ab_c = (a * b) * c;
        let a_bc = a * (b * c);

        assert_approx_eq(ab_c, a_bc, 1e-10);
    }

    #[test]
    fn test_to_from_array_roundtrip() {
        let mut rng = rng();
        let q = Quaternion::random(&mut rng);
        let arr = q.to_array();
        let q2 = Quaternion::from_array(arr);
        assert_approx_eq(q, q2, 1e-10);
    }

    #[test]
    fn test_slerp_near_parallel_nlerp_fallback() {
        // Two very close quaternions to trigger NLERP path
        let a = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let b = Quaternion::new(1.0, 0.0001, 0.0, 0.0);
        let mid = a.slerp(b, 0.5);
        assert_unit(mid);
    }

    #[test]
    fn test_slerp_antipodal_flip() {
        // When dot < 0, SLERP should flip to take shorter arc
        let a = Quaternion::new(1.0, 0.0, 0.0, 0.0);
        let b = Quaternion::new(-0.9, -0.1, 0.0, 0.0);
        let mid = a.slerp(b, 0.5);
        assert_unit(mid);
    }
}
