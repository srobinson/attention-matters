//! DAE (Daemon Attention Engine) geometric memory engine.
//!
//! Models memory as a closed S³ manifold (3-sphere) with fixed mass M=1.
//! Uses quaternion-manifold activation, IDF-weighted SLERP drift, phasor
//! interference, and Kuramoto phase coupling across conscious/subconscious
//! manifolds.
//!
//! Zero I/O — pure math engine with no opinions about transport or persistence.

pub mod compose;
pub mod constants;
pub mod episode;
pub mod neighborhood;
pub mod occurrence;
pub mod phasor;
pub mod quaternion;
pub mod query;
pub mod serde_compat;
pub mod surface;
pub mod system;
pub mod tokenizer;

pub use compose::{ContextMetrics, ContextResult, compose_context, extract_salient};
pub use constants::{
    EPSILON, GOLDEN_ANGLE, M, NEIGHBORHOOD_RADIUS, PHI, SLERP_THRESHOLD, THRESHOLD,
};
pub use episode::Episode;
pub use neighborhood::Neighborhood;
pub use occurrence::Occurrence;
pub use phasor::DaemonPhasor;
pub use quaternion::Quaternion;
pub use query::{QueryEngine, QueryResult};
pub use serde_compat::{CURRENT_VERSION, export_json, import_json};
pub use surface::{SurfaceResult, compute_surface};
pub use system::{DAESystem, NeighborhoodRef, OccurrenceRef};
pub use tokenizer::{ingest_text, tokenize};
