# attention-matters

A geometric memory engine built on the S³ hypersphere. Memories aren't retrieved — they're surfaced through quaternion drift, phasor interference, and Kuramoto phase coupling across dual manifolds.

Every query reshapes the manifold. Every recall changes what gets recalled next. The geometry does the thinking.

## How it works

```
Query: "quaternion drift"
    │
    ▼
┌─ activate ──── drift ──── interfere ──── couple ──── surface ──── compose ─┐
│                                                                             │
│  Words activate      Occurrences    Conscious &     Phases         Score,   │
│  on the manifold     SLERP toward   subconscious    synchronize    rank,    │
│  with IDF weights    each other     phasors         via Kuramoto   return   │
└─────────────────────────────────────────────────────────────────────────────┘
```

Words live as points on a 4D unit sphere. When a query activates them, they drift toward each other along geodesics. Interference between conscious and subconscious manifolds determines what surfaces. Phase coupling synchronizes related memories over time. The system has genuine history — it learns from being used.

Two conservation laws keep it grounded: total mass M=1 (finite attention budget) and coupling constants K_CON + K_SUB = 1 (zero-sum attention between manifolds). Every constant derives from φ and π.

## Install

```bash
npx -y attention-matters          # CLI
npx -y attention-matters serve    # MCP server
```

Or build from source:

```bash
cargo install --path crates/am-cli
```

## MCP server

Runs as a Model Context Protocol server, giving AI agents persistent geometric memory across sessions.

```bash
am serve
```

Tools: `am_query`, `am_buffer`, `am_ingest`, `am_salient`, `am_feedback`, `am_activate_response`, `am_batch_query`, `am_export`, `am_import`, `am_stats`

## CLI

```bash
am query "what did we decide about the API"    # recall from memory
am ingest document.md                          # add to the manifold
am stats                                       # system state
am inspect neighborhoods --limit 5             # peek at the geometry
am export > state.json                         # portable state
am import < state.json                         # restore
am sync                                        # sync session transcripts
```

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  DAESystem                                                    │
│                                                               │
│  ┌───────────────────┐         ┌───────────────────────────┐  │
│  │  Conscious         │         │  Subconscious             │  │
│  │  Episode           │         │  Episodes (N)             │  │
│  │                    │         │                           │  │
│  │  Neighborhoods     │         │  Neighborhoods            │  │
│  │    → Occurrences   │         │    → Occurrences          │  │
│  └────────┬───────────┘         └─────────────┬─────────────┘  │
│           │                                   │                │
│           └───── phasor interference ─────────┘                │
│                                                               │
│  QueryEngine: activate → drift → interfere → couple → surface │
└──────────────────────────────────────────────────────────────┘
```

Three crates, clean separation:

| Crate | What it does |
|-------|-------------|
| `am-core` | Pure math. Quaternions, phasors, drift, interference, Kuramoto coupling, context composition. Zero I/O, 201 tests. |
| `am-store` | Persistence. SQLite-backed brain.db — one database per developer, queryable from any project. |
| `am-cli` | CLI + MCP server. Session sync, import/export, inspection tools. |

## The math

All constants derive from φ (golden ratio) and π.

| Mechanism | What it does |
|-----------|-------------|
| **Quaternion positions** | Each word instance lives on S³. SLERP interpolation along geodesics. |
| **IDF-weighted drift** | Query activation pulls related occurrences closer. The manifold reshapes with every query. |
| **Golden-angle phasors** | Phase distribution follows phyllotaxis (2π/φ²) for maximal separation. |
| **Cross-manifold interference** | Subconscious memories resonate with conscious ones via cos(phase_diff). |
| **Kuramoto coupling** | Phases synchronize between co-activated neighborhoods, modulated by plasticity. |
| **Anchoring** | High-activation occurrences crystallize in place — stable structures emerge. |

The system has two conservation laws:
- **M = 1** — total mass is conserved. Attention is a finite resource.
- **K_CON + K_SUB = 1** — coupling between manifolds is zero-sum.

## Development

Rust 2024 edition. [just](https://github.com/casey/just) as task runner.

```bash
just check    # clippy (warnings = errors)
just build    # cargo build --workspace
just test     # 253 tests
just fmt      # rustfmt
```

## License

MIT
