# attention-matters

Geometric memory for AI coding agents. No embeddings, no vectors, no cloud. Just math.

**Version:** 0.1.18
**License:** MIT
**Repository:** https://github.com/srobinson/attention-matters
**npm:** `npx -y attention-matters serve`

---

## What It Is

attention-matters implements the DAE (Daemon Attention Engine) geometric memory model as a Rust workspace. Memory is not stored as flat text or vector embeddings. Words are placed as quaternion positions on a 3-sphere (S³ manifold), where related concepts cluster through physics-inspired dynamics across sessions.

The mathematical framework was created by Smaxforn ([@smaxforn](https://x.com/smaxforn)). This codebase is a Rust port of his original JavaScript DAE v0.7.2 engine, maintaining numerical compatibility with the original wire format.

---

## Architecture

### Crate Topology

```
attention-matters/            Cargo workspace (resolver = "3", edition 2024)
  crates/
    am-core/                  Pure math engine — zero I/O
    am-store/                 SQLite persistence layer
    am-cli/                   CLI binary + MCP server
  npm/
    attention-matters/        npm wrapper for npx distribution
```

Dependency direction is strictly acyclic:

```
am-cli  →  am-store  →  am-core
am-cli  →  am-core
```

am-core has no knowledge of persistence or transport. am-store has no knowledge of the CLI or MCP protocol. This contract is upheld throughout — the only minor impurity is `am-core/src/time.rs` calling `SystemTime::now()`.

### am-core

Pure math engine. 74 public exports. No I/O.

| Module | Purpose |
|---|---|
| `constants` | φ, golden angle, neighborhood radius, thresholds, GC defaults |
| `quaternion` | S³ math: SLERP, Hamilton product, geodesic distance, random generation |
| `phasor` | Golden-angle phase distribution, circular interpolation |
| `occurrence` | Word instance on manifold with quaternion position, phasor phase, activation count |
| `neighborhood` | Cluster of occurrences around a seed quaternion, source text, epoch |
| `episode` | Collection of neighborhoods representing a document or conversation |
| `system` | `DAESystem` — top-level container with lazy-rebuilt word/neighborhood indexes |
| `tokenizer` | Regex tokenizer, sentence chunking, 3-sentence neighborhood batching |
| `query` | `QueryEngine` — activate, drift (SLERP), interference, Kuramoto coupling |
| `surface` | Vivid neighborhood/episode selection, fragment extraction |
| `compose` | Context composition: conscious + subconscious + novel recall, budget-aware |
| `batch` | `BatchQueryEngine` — amortized IDF across multiple concurrent queries |
| `feedback` | `apply_feedback` — boost (SLERP toward query centroid) / demote (activation decay) |
| `time` | ISO8601 and Unix second timestamp utilities |
| `scoring` | Composite scoring: activation, recency, interference, IDF weighting |
| `recency` | Recency-aware scoring with epoch and timestamp normalization |
| `salient` | Salient neighborhood extraction for conscious promotion |
| `activation_stats` | Activation statistics aggregation |
| `store_trait` | `AmStore` trait: hexagonal port for persistence abstraction |
| `serde_compat` | v0.7.2 JSON wire format import/export |

### am-store

SQLite-backed persistence. Schema version 7. WAL mode with 5-second busy timeout and 400KB autocheckpoint.

| Module | Purpose |
|---|---|
| `store` | `Store` — episode/neighborhood/occurrence CRUD, activation updates, GC |
| `project` | `BrainStore` — single unified brain at `~/.attention-matters/brain.db` |
| `config` | `Config` + `RetentionPolicy` — TOML config loading with env var overrides |
| `schema` | DDL, pragma setup, additive ALTER TABLE migrations |
| `json_bridge` | Serialization bridge for v0.7.2 JSON import/export |
| `memory_store` | `MemoryStore` - concrete `AmStore` impl wrapping `Store` + `DAESystem` |
| `error` | `StoreError`, `Result` |

### am-cli

Binary: `am`. 11 CLI subcommands and an MCP server over stdio.

| Module | Purpose |
|---|---|
| `main` | Clap command definitions, CLI handler implementations |
| `server` | `AmServer` - 12 MCP tool handlers over JSON-RPC 2.0 |
| `jsonrpc` | Custom JSON-RPC 2.0 server (stdio transport, MCP protocol) |
| `sync` | Claude Code `.jsonl` transcript parsing and episode extraction |
| `sync_dispatch` | Sync orchestration: session discovery, dispatch, logging |
| `colors` | ANSI color constants for CLI output |
| `generated_help` | Pre-rendered help strings for MCP tool descriptions |
| `generated_schema` | JSON Schema definitions for MCP tool parameters |

---

## The Math

### S³ Manifold Model

Memory is modeled as a closed S³ manifold with fixed total mass M = 1. Each word occurrence occupies a quaternion position (w, x, y, z) on the unit 3-sphere. Related concepts cluster geometrically through repeated activation and drift.

### Key Constants

| Constant | Value | Derivation |
|---|---|---|
| `PHI` | 1.618033988749895 | (1 + √5) / 2 |
| `GOLDEN_ANGLE` | 2.3999632297286533 rad | 2π / φ² |
| `NEIGHBORHOOD_RADIUS` | 1.9416135460476878 rad | π / φ |
| `THRESHOLD` | 0.5 | Activation anchoring threshold |
| `M` | 1.0 | Manifold mass (closed system) |
| `EPSILON` | 1e-10 | Near-zero comparison guard |
| `SLERP_THRESHOLD` | 0.9995 | Near-parallel SLERP fallback |

No magic numbers. Every constant derives from φ or π.

### Query Pipeline

1. **Tokenize** — regex tokenizer strips stopwords, lowercases, deduplicates
2. **Activate** — matching words across subconscious episodes and conscious manifold increment their `activation_count`
3. **Drift** — IDF-weighted SLERP pulls activated occurrences toward query centroid (OpenClaw variant: `ratio / THRESHOLD`)
4. **Interference** — phasor products between subconscious and conscious occurrences of the same word produce interference amplitude
5. **Kuramoto coupling** — phase coupling synchronizes related concepts across manifolds
6. **Surface** — vivid neighborhoods (high activation density) and vivid episodes are selected
7. **Compose** — neighborhoods are scored, ranked, and formatted into three recall categories: conscious, subconscious, novel

### Ingest Pipeline

Text is split into 3-sentence chunks. Each chunk becomes one neighborhood: words are placed on S³ using golden-angle phasor spacing. The neighborhood is assigned the current epoch counter, then added to the active episode.

### Feedback Loop

`apply_feedback(system, query, neighborhood_ids, signal)`:

- **Boost** — occurrences in recalled neighborhoods SLERP toward the IDF-weighted query centroid by `BOOST_DRIFT_FACTOR = 0.15`. Helpful memories migrate toward the region of the manifold where they were needed.
- **Demote** — occurrences in recalled neighborhoods lose `DEMOTE_DECAY = 2` activation counts. Lower activation means less drift influence in future queries and eventual GC eligibility.

### Conscious vs. Subconscious

Two manifolds coexist in one `DAESystem`:

- **Subconscious** — all ingested episodes. Words here compete by IDF weight and activation count.
- **Conscious** — single `conscious_episode`. Neighborhoods marked salient via `am_salient` live here. Conscious memories persist globally across all projects and are never auto-evicted by GC.

The `EpisodeRef::Conscious` variant in `OccurrenceRef` and `NeighborhoodRef` identifies conscious occurrences (replaces the former `usize::MAX` sentinel).

---

## Database Schema

Schema version 7. SQLite with WAL journal mode.

```sql
metadata          (key TEXT PK, value TEXT)

episodes          (id TEXT PK, name TEXT, is_conscious INTEGER, timestamp TEXT)

neighborhoods     (id TEXT PK, episode_id TEXT → episodes,
                   seed_w/x/y/z REAL,        -- seed quaternion
                   source_text TEXT,
                   neighborhood_type TEXT,    -- 'memory' | 'salient' | ...
                   epoch INTEGER,
                   superseded_by TEXT)        -- UUID of replacement, nullable

occurrences       (id TEXT PK, neighborhood_id TEXT → neighborhoods,
                   word TEXT,
                   pos_w/x/y/z REAL,          -- quaternion position on S³
                   phasor_theta REAL,
                   activation_count INTEGER)

conversation_buffer (id INTEGER PK AUTOINCREMENT,
                     user_text TEXT, assistant_text TEXT,
                     created_at TEXT)
```

Existing indexes: `idx_occ_word`, `idx_occ_neighborhood`, `idx_nbhd_episode`.

Startup sequence: WAL mode → foreign keys → busy timeout 5s → autocheckpoint 100 pages → TRUNCATE checkpoint → DDL (CREATE IF NOT EXISTS) → additive ALTER TABLE migrations.

---

## Configuration

Precedence (highest wins):
1. Environment variables (`AM_DATA_DIR`, `AM_GC_ENABLED`, `AM_DB_SIZE_MB`, `AM_SYNC_LOG_DIR`)
2. Config file (first found): `$CWD/.am.config.toml` > `$AM_DATA_DIR/.am.config.toml` > `~/.attention-matters/.am.config.toml`
3. Compiled defaults

```toml
# ~/.attention-matters/.am.config.toml
data_dir    = "~/.attention-matters"
gc_enabled  = false
db_size_mb  = 50

[retention]
grace_epochs       = 50     # epochs — newest N epochs are GC-exempt
retention_days     = 3      # days — recent neighborhoods are GC-exempt
min_neighborhoods  = 100    # skip GC entirely below this count
recency_weight     = 2.0    # bonus weight for newer neighborhoods in scoring
```

Environment variable overrides: `AM_DATA_DIR`, `AM_GC_ENABLED`, `AM_DB_SIZE_MB`.

Generate a fully-commented config with `am init` or `am init --global`.

---

## CLI Reference

```
am serve                          Start MCP server on stdio (primary mode)
am query <text>                   Query memory and display recall
am ingest <files...> [--dir DIR]  Ingest .txt/.md/.html files
am stats                          Memory system diagnostics
am export <path>                  Export to v0.7.2-compatible JSON
am import <path>                  Import from exported JSON
am inspect [mode] [--query TEXT]  Browse memory contents
am sync [--all] [--dry-run]       Ingest Claude Code session transcripts
am gc [--floor N] [--target-mb N] Garbage collect cold memories
am forget [term|--episode|--conscious] Remove specific memories
am init [--global] [--force]      Generate default config file
```

### inspect modes

```
am inspect                        Overview — top words, recent episodes
am inspect conscious              List all conscious memories
am inspect episodes [--limit N]   Subconscious episodes with stats
am inspect neighborhoods          All neighborhoods ranked by activation
am inspect --query "auth flow"    Full query recall breakdown
```

---

## MCP Server

`am serve` starts a JSON-RPC 2.0 server on stdio using a custom protocol implementation (`jsonrpc.rs`). Claude Code spawns the process and owns the pipe. Zero network exposure. No authentication surface.

### Lifecycle Protocol

Agents should follow this pattern:

```
Session start  →  am_query (recall relevant past context)
After response →  am_activate_response (strengthen connections)
Key insight    →  am_salient (mark as conscious memory)
Exchange pairs →  am_buffer (auto-create episodes from conversation)
Documents      →  am_ingest (add documents as memory episodes)
On outcome     →  am_feedback (boost helpful / demote irrelevant recalls)
```

### MCP Tools

| Tool | Description |
|---|---|
| `am_query` | Recall context. Returns conscious, subconscious, and novel fragments |
| `am_query_index` | Phase 1 of two-phase retrieval: returns scored neighborhood index |
| `am_retrieve` | Phase 2: fetch full text for selected neighborhoods |
| `am_activate_response` | Strengthen manifold connections after a meaningful response |
| `am_salient` | Mark a neighborhood as conscious (persistent, globally-scoped) |
| `am_buffer` | Buffer a user/assistant exchange; auto-flushes to episode at threshold |
| `am_ingest` | Ingest arbitrary text as a memory episode |
| `am_batch_query` | Multiple queries with amortized IDF computation |
| `am_feedback` | Apply boost or demote signal to recalled neighborhood IDs |
| `am_stats` | System diagnostics: N, episode count, conscious count, DB size |
| `am_export` | Export full state as portable JSON |
| `am_import` | Import previously exported state |

### Claude Code Setup

```
claude mcp add am -- npx -y attention-matters serve
```

### Claude Code Hooks (sync)

Register hooks to automatically ingest session transcripts:

```json
{
  "hooks": {
    "PreCompact": [{ "matcher": "", "hooks": [{ "type": "command", "command": "am sync" }] }],
    "Stop":       [{ "matcher": "", "hooks": [{ "type": "command", "command": "am sync" }] }]
  }
}
```

`am sync` reads `session_id` and `transcript_path` from hook stdin, parses the `.jsonl` transcript, chunks it into episodes of 5 user turns each, and ingests each as a subconscious episode. Replace semantics prevent duplicates on re-sync.

---

## Sync: Transcript Ingestion

`sync.rs` parses Claude Code `.jsonl` session files.

**Content rules:**
- Included: user text, assistant text, assistant thinking blocks (main chain and sidechains)
- Excluded: tool_use blocks, tool_result messages, system prompts, file-history-snapshot entries, messages shorter than 20 characters
- Markdown formatting is stripped via pulldown-cmark before ingest

**Chunking:** 5 user turns per episode (`EXCHANGES_PER_EPISODE`). Long sessions produce multiple episodes, matching the source DAE's episodic model.

**Modes:**
- `am sync` (stdin) — hook-triggered, ingests a single session
- `am sync --all` — walks `~/.claude/projects/<encoded-cwd>/` and re-ingests all transcripts

---

## Garbage Collection

GC removes cold occurrences and reclaims disk space. Conscious memories are never evicted.

**Floor pass:** removes occurrences with `activation_count <= floor` (default 1 in CLI, 0 compiled floor). Cleans up empty neighborhoods, then empty episodes.

**Aggressive pass** (triggered when DB exceeds `db_size_mb`): composite eviction scoring ranks neighborhoods by `activation_count + recency_weight * normalized_epoch`. The lowest-scoring neighborhoods are evicted until the database reaches `DB_GC_TARGET_RATIO` (80%) of the soft limit.

**GC exemptions:**
- Conscious episode and all its neighborhoods
- Neighborhoods within `grace_epochs` of the current max epoch
- Neighborhoods newer than `retention_days` days
- Entire GC is skipped if total neighborhoods < `min_neighborhoods`

```
am gc                    # Remove zero-activation occurrences (floor=1)
am gc --floor 2          # Remove occurrences activated 2 or fewer times
am gc --target-mb 10     # Shrink to ~10 MB
am gc --dry-run          # Preview without changes
```

---

## npm Distribution

The npm package `attention-matters` wraps the compiled Rust binary for distribution via `npx`. The `postinstall` script (`scripts/install.js`) downloads the appropriate platform binary from GitHub Releases.

Supported platforms: macOS arm64, macOS x64, Linux x64, Linux arm64.

```
npx -y attention-matters serve       # Run MCP server
npx -y attention-matters query "..."  # Query from command line
```

---

## Development

### Commands

```sh
just check    # cargo clippy --workspace -- -D warnings
just build    # cargo build --workspace
just test     # cargo nextest run --workspace && cargo test --workspace --doc
just fmt      # cargo fmt --all
```

### Dependencies

| Crate | Version | Purpose |
|---|---|---|
| tokio | 1 | Async runtime (multi-thread, io-std, signal, time) |
| clap | 4 | CLI argument parsing (derive) |
| rusqlite | 0.32 (bundled) | SQLite storage |
| serde / serde_json | 1 | Serialization |
| uuid | 1 (v4, serde) | Episode/neighborhood/occurrence IDs |
| rand | 0.9 | Quaternion random generation |
| regex | 1 | Tokenization |
| anyhow | 1 | Error handling in am-cli |
| tracing / tracing-subscriber | 0.1 / 0.3 | Structured logging |
| toml | 0.8 | Config file parsing |
| pulldown-cmark | 0.13 | Markdown stripping in sync |
| libc | 0.2 | Unix process signaling (PID check) |
| rustc-hash | 2 | FxHasher for stable, fast deduplication |
| thiserror | 2 | Error type derivation for StoreError |
| approx | 0.5 (dev) | Float comparison in am-core tests |

### Conventions

- All floating point: `f64` (matches JS Number for v0.7.2 numerical compatibility)
- `Quaternion` is `#[derive(Clone, Copy)]` — lightweight value type, passed by value
- Constants: derived from φ and π, documented with formula in `constants.rs`
- OpenClaw drift variant: `ratio / THRESHOLD` (2c/C)
- SLERP near-parallel threshold: 0.9995
- No magic numbers in am-core

### Test Coverage

- **am-core:** 217 tests (unit + integration + property-based via `proptest.rs`)
- **am-store:** 80 tests (store operations, schema migrations, config)
- **am-cli:** 121 tests (CLI integration, MCP protocol via `mcp_protocol_test.rs`, shutdown contract)
- **Total:** 418 tests across the workspace

Key test files: `tests/proptest.rs` (quaternion invariants), `tests/mcp_protocol_test.rs` (JSON-RPC protocol compliance), `tests/shutdown.rs` (OS-level shutdown contract: WAL checkpoint, pidfile lifecycle, pre-handshake EOF).

---

## Known Issues

Issues from the March 2026 review, updated 2026-03-21. Fixed items removed.

### High

| Location | Issue |
|---|---|
| `query.rs` / `system.rs` | `retrieve_by_ids` does linear scan despite `neighborhood_index` providing O(1) lookup |
| `batch.rs` | Batch activation inflation - words shared across N batch queries get `activation_count` bumped N times |
| `store.rs` | Full O(N) serialize-to-SQLite on every write - primary scaling constraint for large systems |

### Medium

| Location | Issue |
|---|---|
| `feedback.rs` + `query.rs` | Duplicate centroid computation (R4 weighted sum + normalize-to-S3) |

### Low

| Location | Issue |
|---|---|
| `error.rs` | Missing `Io` variant - file I/O errors collapse into `InvalidData` |
| `schema.rs` | Migration not version-gated - probes all columns on every startup |
| `main.rs` | No unit tests; large handlers only covered by integration tests |
| `main.rs` | ANSI escape codes unconditional in help strings - renders as garbage in CI/piped output |
| CLI | `ingest --dir` + positional file in same directory creates duplicate episodes |

### Fixed since last review

| Issue | Resolution |
|---|---|
| `partial_cmp().unwrap()` NaN panics | Replaced with `f64::total_cmp()` throughout `compose.rs` |
| N+1 `load_system` (2101 queries) | Single 3-way JOIN in `store.rs` |
| `format!()` SQL injection surface | All SQL now uses parameterized queries |
| `drain_buffer` crash window | Atomic transaction with range-based DELETE |
| Missing indexes | Added in schema v6/v7: `idx_ep_conscious`, `idx_occ_activation`, `idx_nbhd_episode_epoch`, `idx_occ_nbhd_activation` |
| `activation_count += 1` wrapping | Uses `saturating_add` |
| `token_count()` Vec allocation | Uses `.count()` iterator |
| `DefaultHasher` instability | Replaced with `FxHasher` (rustc-hash) |
| No input size limits | `check_input_size` (1MB cap) on all text-accepting endpoints |
| `pub fn conn()` raw exposure | Removed |
| `compose.rs` god module (2,959 LOC) | Extracted to `scoring.rs`, `recency.rs`, `salient.rs` (now 2,478 LOC) |
| Sync orchestration in `main.rs` | Extracted to `sync_dispatch.rs` |
| `angular_distance` undocumented | Doc comments explain SO(3) vs S3 tradeoff |

---

## Release History (recent)

| Version | Highlights |
|---|---|
| 0.1.18 | Architecture alignment: rmcp replaced with custom JSON-RPC, AmStore trait, quality pass |
| 0.1.17 | Incremental persistence, core math hardening, CLI/server fixes |
| 0.1.16 | Scoring extraction, GC improvements, schema v6/v7 indexes |
| 0.1.15 | Strip markdown from sync episode text; role headers |
| 0.1.14 | Location-based config resolution for `.am.config.toml` |
| 0.1.13 | `am init` command to generate default config |
| 0.1.12 | `sync_log_dir` config option |
| 0.1.11 | Transcript-based episode extraction on SessionEnd |
| 0.1.10 | Epoch-aware retention policy for GC |
| 0.1.9 | Configurable `.am.config.toml`, GC disabled by default |
| 0.1.8 | Unified brain — single `brain.db`, removed per-project concept |
| 0.1.7 | Recency-aware recall — timestamps, backfill, conscious boost |
| 0.1.6 | Conscious recall pipeline fix — interference and vividness wired |
| 0.1.5 | Feedback loop — recalled neighborhood IDs surfaced for feedback |
| 0.1.4 | Unified brain — per-project concept removed |
