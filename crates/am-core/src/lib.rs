//! DAE (Daemon Attention Engine) geometric memory engine.
//!
//! Models memory as a closed S³ manifold (3-sphere) with fixed mass M=1.
//! Uses quaternion-manifold activation, IDF-weighted SLERP drift, phasor
//! interference, and Kuramoto phase coupling across conscious/subconscious
//! manifolds.
//!
//! Zero I/O — pure math engine with no opinions about transport or persistence.

pub mod constants;
pub mod quaternion;
pub mod phasor;
pub mod occurrence;
pub mod neighborhood;
pub mod episode;
pub mod system;
pub mod tokenizer;
pub mod query;
pub mod surface;
pub mod compose;
pub mod serde_compat;

pub use compose::{compose_context, extract_salient, ContextMetrics, ContextResult};
pub use constants::{EPSILON, GOLDEN_ANGLE, M, NEIGHBORHOOD_RADIUS, PHI, SLERP_THRESHOLD, THRESHOLD};
pub use episode::Episode;
pub use neighborhood::Neighborhood;
pub use occurrence::Occurrence;
pub use phasor::DaemonPhasor;
pub use quaternion::Quaternion;
pub use query::{QueryEngine, QueryResult};
pub use serde_compat::{export_json, import_json, CURRENT_VERSION};
pub use surface::{compute_surface, SurfaceResult};
pub use system::{DAESystem, NeighborhoodRef, OccurrenceRef};
pub use tokenizer::{ingest_text, tokenize};
