/// Golden ratio: (1 + √5) / 2
pub const PHI: f64 = 1.618_033_988_749_895;

/// Golden angle in radians: 2π / φ²
pub const GOLDEN_ANGLE: f64 = 2.399_963_229_728_653_3;

/// Neighborhood radius: π / φ (radians on S³)
pub const NEIGHBORHOOD_RADIUS: f64 = 1.941_613_546_047_687_8;

/// Activation threshold for anchoring and vividity checks
pub const THRESHOLD: f64 = 0.5;

/// Total system mass (closed S³ manifold)
pub const M: f64 = 1.0;

/// Numerical epsilon for near-zero comparisons
pub const EPSILON: f64 = 1e-10;

/// SLERP near-parallel threshold (OpenClaw standard)
pub const SLERP_THRESHOLD: f64 = 0.9995;

/// GC: minimum activation count to survive eviction.
/// Occurrences at or below this are candidates for garbage collection.
pub const ACTIVATION_FLOOR: u32 = 0;

/// GC: per-project DB size soft limit before GC triggers (50MB)
pub const DB_SOFT_LIMIT_BYTES: u64 = 50 * 1024 * 1024;

/// GC: target ratio of soft limit after aggressive eviction (80%)
pub const DB_GC_TARGET_RATIO: f64 = 0.8;

/// GC: grace window in epochs. Neighborhoods within this many epochs of the
/// current max are exempt from garbage collection regardless of activation.
pub const DEFAULT_GRACE_EPOCHS: u64 = 50;

/// GC: minimum neighborhood count below which GC is skipped entirely.
pub const DEFAULT_MIN_NEIGHBORHOODS: u64 = 100;

/// GC: timestamp backstop in days. Neighborhoods newer than this are exempt
/// from GC even if outside the epoch grace window. Protects idle projects.
pub const DEFAULT_RETENTION_DAYS: u64 = 3;

/// GC: recency weight for composite eviction scoring. Higher values give
/// newer neighborhoods more protection in aggressive GC. A value of 2.0
/// means a neighborhood at the current epoch gets a bonus equivalent to
/// 2 activation counts.
pub const DEFAULT_RECENCY_WEIGHT: f64 = 2.0;
