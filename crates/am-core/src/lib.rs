//! DAE (Daemon Attention Engine) geometric memory engine.
//!
//! Models memory as a closed S³ manifold (3-sphere) with fixed mass M=1.
//! Uses quaternion-manifold activation, IDF-weighted SLERP drift, phasor
//! interference, and Kuramoto phase coupling across conscious/subconscious
//! manifolds.
//!
//! The mathematical framework was created by Smaxforn ([@smaxforn](https://x.com/smaxforn)).
//! This crate is a Rust port of his original JavaScript DAE v0.7.2 engine,
//! maintaining numerical compatibility with the original wire format.
//!
//! Zero I/O - pure math engine with no opinions about transport or persistence.

#![warn(clippy::pedantic)]
// Geometric memory operates on counts and indices that fit comfortably within
// f64's 52-bit mantissa (max ~4.5e15). The casts from usize/u64/i64 to f64
// are intentional and safe at any realistic scale.
#![allow(clippy::cast_precision_loss)]
// u64-to-i64 wraps occur only in calendar math (time.rs) and epoch counters
// where values are bounded well below i64::MAX.
#![allow(clippy::cast_possible_wrap)]
// i64-to-u64 sign loss occurs only in calendar math after range validation.
#![allow(clippy::cast_sign_loss)]
// Many functions operate on the full DAESystem and are inherently complex.
// Splitting them further would reduce locality without improving clarity.
#![allow(clippy::too_many_lines)]
// f64 exact comparisons are intentional in specific geometric contexts
// (e.g., checking for zero, sentinel values). Each use is validated.
#![allow(clippy::float_cmp)]
// Internal functions accept HashMap with the default hasher. Generalizing
// over BuildHasher adds complexity with no benefit since all call sites
// use std::collections::HashMap.
#![allow(clippy::implicit_hasher)]

pub mod activation_stats;
pub mod batch;
pub mod compose;
pub mod constants;
pub mod episode;
pub mod feedback;
pub mod neighborhood;
pub mod occurrence;
pub mod phasor;
pub mod quaternion;
pub mod query;
pub(crate) mod recency;
pub mod salient;
pub(crate) mod scoring;
pub mod serde_compat;
pub mod store_trait;
pub mod surface;
pub mod system;
pub mod time;
pub mod tokenizer;
