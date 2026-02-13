# attention-matters

Rust workspace implementing the DAE (Daemon Attention Engine) geometric memory system.

## Architecture

- `am-core` — Pure math engine. Zero I/O. Models memory as S³ manifold with quaternion positions, golden-angle phasors, IDF-weighted drift, and Kuramoto phase coupling.
- `am-store` — Persistence layer (SQLite-backed state storage).
- `am-cli` — CLI interface for ingestion, querying, and import/export.

## Conventions

- All floating point: `f64` (matches JS Number for numerical compatibility with v0.7.2 reference)
- Quaternion is `#[derive(Clone, Copy)]` — lightweight value type
- Constants derived from φ and π — no magic numbers
- OpenClaw drift variant: `ratio / THRESHOLD` (2c/C)
- SLERP near-parallel threshold: `0.9995`

## Commands

```sh
just check    # clippy with warnings-as-errors
just build    # cargo build --workspace
just test     # cargo test --workspace
just fmt      # cargo fmt
```

## Module Map (am-core)

| Module | Purpose |
|--------|---------|
| `constants` | φ, golden angle, neighborhood radius, thresholds |
| `quaternion` | S³ math: SLERP, random, Hamilton product, geodesic distance |
| `phasor` | Golden-angle phase distribution, circular interpolation |
| `occurrence` | Word instance on manifold with activation, drift, plasticity |
| `neighborhood` | Cluster of occurrences around a seed quaternion |
| `episode` | Collection of neighborhoods (document/conversation) |
| `system` | DAESystem with lazy indexes, IDF weights, activation |
| `tokenizer` | Regex tokenizer + sentence chunking for ingestion |
| `query` | QueryEngine: drift, interference, Kuramoto coupling |
| `surface` | Surface computation: vivid neighborhoods/episodes, fragments |
| `compose` | Context composition: conscious/subconscious/novel recall |
| `serde_compat` | v0.7.2 JSON wire format import/export |
