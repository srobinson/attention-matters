# attention-matters

Pure Rust implementation of the **DAE** (Daemon Attention Engine) — a geometric memory system that models recall as activation on a closed S³ manifold.

Memories aren't retrieved, they're *surfaced* through quaternion drift, phasor interference, and phase coupling across conscious and subconscious manifolds. The math decides what matters.

## Architecture

```
┌──────────────────────────────────────────────────────────┐
│  DAESystem                                               │
│                                                          │
│  ┌──────────────────┐       ┌─────────────────────────┐  │
│  │  Conscious       │       │  Subconscious           │  │
│  │  Episode         │       │  Episodes (N)           │  │
│  │                  │       │                         │  │
│  │  Neighborhoods   │       │  Neighborhoods          │  │
│  │    → Occurrences │       │    → Occurrences        │  │
│  └─────────┬────────┘       └────────────┬────────────┘  │
│            │                             │               │
│            └──── phasor interference ────┘               │
│                                                          │
│  QueryEngine: activate → drift → interfere → surface     │
└──────────────────────────────────────────────────────────┘
```

**Workspace crates:**

| Crate | Purpose |
|------------|---------|
| `am-core` | Math engine — quaternions, phasors, drift, interference, Kuramoto coupling, context composition. Zero I/O. |
| `am-store` | Persistence (stub) |
| `am-cli` | CLI interface (stub) |

## The Math

All constants derive from φ (golden ratio) and π — no magic numbers.

- **Quaternion positions** on S³ give each memory a geometric location
- **SLERP drift** pulls query attention toward relevant content, weighted by IDF
- **Golden-angle phasors** distribute activation evenly across the manifold
- **Cross-manifold interference** lets subconscious memories resonate with conscious ones
- **Kuramoto coupling** synchronizes phases between co-activated neighborhoods, modulated by plasticity
- **Drift rate**: OpenClaw variant `2c/C` (ratio / THRESHOLD) — physically intuitive

## Quick Start

```rust
use am_core::{DAESystem, QueryEngine, ingest_text, compose_context, compute_surface};

// Create a two-manifold system
let mut system = DAESystem::new();

// Ingest text into the conscious manifold
ingest_text(&mut system, "The cat sat on the mat.");
ingest_text(&mut system, "Quantum mechanics describes nature at the atomic scale.");

// Query — surfaces relevant memories through geometric activation
let mut engine = QueryEngine::new();
let query_result = engine.query(&mut system, "cat");

// Compose human-readable context from activation
let surface = compute_surface(&system, &query_result);
let context = compose_context(&mut system, &surface, &query_result, "cat");
println!("{}", context.context);
```

### v0.7.2 JSON Import

```rust
use am_core::{import_json, export_json};

let json_str = std::fs::read_to_string("daemon-state.json").unwrap();
let mut system = import_json(&json_str).unwrap();

// Query against imported state
let mut engine = QueryEngine::new();
let result = engine.query(&mut system, "attention");
```

## Development

Requires Rust 2024 edition. Uses [just](https://github.com/casey/just) as task runner.

```bash
just check    # clippy (warnings = errors)
just build    # cargo build
just test     # 89 tests
just fmt      # rustfmt
```

## License

MIT
