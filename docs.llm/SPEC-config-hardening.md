# SPEC: Config Management Hardening

Origin: peer review from context-matters agent (2026-03-21)
Status: approved
Linear: ALP-1621 (parent), ALP-1622 (P2), ALP-1623 (P3)

## Problem

The CONFIG_MANAGEMENT.md spec and its reference implementation have three gaps that violate the spec's own principles, plus four minor issues worth addressing.

## Decisions

These decisions are locked for this spec. They are not left to the assignee.

1. **Semantic config validation fails closed.** Parse errors in the config file still warn and fall back. Resolved config values that violate safety invariants return an error.
2. **`am_store::config::load()` becomes fallible.** The runtime contract is `crate::error::Result<Config>`, not `Config`.
3. **Reuse `StoreError::InvalidData(String)`.** This spec does not introduce a separate `ConfigError` type.
4. **P3 depends on P2.** P2 establishes the `load() -> Result<Config>` contract. P3 implements home and default path resolution under that contract. P3 does not reopen the API decision.

## Changes

### P1: Warn on env var parse failures

**Current behavior:** `AM_DB_SIZE_MB=fifty` silently falls back to the default. No diagnostic output.

**Problem:** Violates user expectations. Someone setting an env var reasonably expects feedback when the value is invalid. Silent fallback makes debugging painful.

**Proposed spec change:** Replace "If parse fails, silently skip" with "If parse fails, log `tracing::warn!` with the var name, raw value, and expected type, then fall back to the file/default value."

**Code change:** Add `tracing::warn!` to the else branches of the `if let Ok(val) = val.parse::<T>()` chains in `config.rs:106-118`.

**Complexity:** Low. Direct code change, no design decisions.

### P2: Post-load validation

**Current behavior:** No semantic checks after config resolution. `db_size_mb=0` with `gc_enabled=true` causes GC to target zero bytes every startup. `data_dir=""` resolves to CWD.

**Problem:** Parseable but nonsensical values create failure modes that are hard to diagnose. Violates principle 5 (no silent data destruction) since GC with a zero-byte target would evict everything.

**Decision:** Fail closed on semantic validation errors. No auto-correct path in v1.

**Proposed spec change:** Update `CONFIG_MANAGEMENT.md` to distinguish:
- syntax and deserialization errors: warn and fall back
- semantic validation errors on the resolved config: return error

Define:
- `pub fn load() -> crate::error::Result<Config>`
- `fn validate(&self) -> crate::error::Result<()>`

`load()` merges defaults, file config, and env vars, then calls `validate()` before returning.

**Validation rules:**
- `db_size_mb` must be >= 1 (or GC disabled)
- `data_dir` must not be empty
- `data_dir` must be absolute after tilde expansion (reject relative paths)

**Error type:** Return `StoreError::InvalidData(...)` with a concrete message naming the offending field and rule.

**Caller behavior:**
- All CLI commands that call `load_config()` or `open_store()` exit non-zero on validation failure.
- `am serve` fails startup before opening the database or entering the MCP loop.
- `am init` is unaffected because it does not load runtime config.

**Required code changes:**
- Change `am_store::config::load()` to return `crate::error::Result<Config>`
- Add `Config::validate()` in `crates/am-store/src/config.rs`
- Change `am-cli` `load_config()` to return `anyhow::Result<Config>`
- Propagate the error through `open_store()` and all call sites that currently assume config loading is infallible

**Acceptance criteria:**
1. `gc_enabled=true` and `db_size_mb=0` causes `am stats` to exit non-zero before store open, with stderr mentioning `db_size_mb` and the minimum allowed value.
2. `data_dir=""` in `.am.config.toml` causes `am query "x"` to exit non-zero before `create_dir_all`.
3. `AM_DATA_DIR=relative/path` causes `am stats` to exit non-zero with an error that names `data_dir` and requires an absolute path.
4. A malformed TOML file still logs a warning and falls back, preserving the fail-open behavior for parse errors.
5. Add config unit tests for each validation rule and at least one CLI integration test that asserts startup failure on invalid resolved config.

**Complexity:** Medium. New validation function, `load()` signature change, caller integration.

### P3: expand_tilde fallback

**Current behavior:** If `$HOME` and `$USERPROFILE` are both unset, `~/data` resolves to `./data` (CWD-relative). The same fallback exists in both `config.rs` and `project.rs`, and `project.rs` also uses it for the runtime default base directory.

**Problem:** Violates principle 5 (no silent data destruction). CWD-relative paths mean the database could be written anywhere depending on how the process was launched. Especially dangerous in MCP stdio mode where CWD is unpredictable.

**Decision:** Under the `load() -> crate::error::Result<Config>` contract from P2, there is no `"."` fallback anywhere in runtime path resolution.

**Proposed spec change:** Introduce one shared helper for home resolution, for example:
- `fn resolve_home_dir() -> crate::error::Result<PathBuf>`

Use it for:
- `expand_tilde`
- runtime default data directory resolution
- `am init --global`

If home cannot be determined and the runtime needs it, return `StoreError::InvalidData(...)` with a clear message. Do not fall back to the current working directory.

**Code change:**
- Unify the duplicated home-resolution logic in `config.rs` and `project.rs`
- Change `expand_tilde` to return `crate::error::Result<PathBuf>`
- Add `fn runtime_defaults() -> crate::error::Result<Config>` in `config.rs` and use it as the runtime seed instead of `Config::default()`
- Change `project::default_base_dir()` to return `crate::error::Result<PathBuf>` and update runtime load plus `am init --global` to propagate that error
- Keep `generate_default_toml()` independent of runtime home lookup. It should render the documented default path literally rather than depending on the live environment

**Acceptance criteria:**
1. With `HOME` and `USERPROFILE` unset, and no explicit absolute `data_dir`, `am stats` exits non-zero before database open with an error explaining that the home directory could not be resolved.
2. With `HOME` and `USERPROFILE` unset, `AM_DATA_DIR=/tmp/am-test` succeeds because no home lookup is needed.
3. With `HOME` and `USERPROFILE` unset, `AM_DATA_DIR=~/am-test` fails non-zero with a tilde-resolution error.
4. With `HOME` and `USERPROFILE` unset, a CWD `.am.config.toml` containing an absolute `data_dir` succeeds.
5. With `HOME` and `USERPROFILE` unset, `am init --global` exits non-zero and writes nothing.
6. After implementation, there is exactly one home-resolution helper in `am-store`; no `unwrap_or_else(|_| ".".to_string())` or equivalent CWD fallback remains.
7. Add unit tests for shared home resolution and integration tests for `am init --global` and one runtime command under an unset-home environment.

**Complexity:** Medium. Shared helper extraction, runtime-default change, `load()` and `init --global` caller updates.

### M1: init --force transparency

**Current behavior:** `init --force` silently overwrites. Does not show what existed before.

**Proposed spec change:** `init --force` prints the path being overwritten. No diff, no confirmation (that is the point of --force), but the user sees what happened.

**Complexity:** Low.

### M2: WAL checkpoint note

**Current behavior:** `PRAGMA wal_checkpoint(TRUNCATE)` runs on every `Store::open()`. Failure is silently swallowed.

**Proposed spec change:** Add a note that this pragma blocks concurrent readers and should log at debug level on failure rather than swallowing silently. Current behavior is acceptable for single-writer use cases (which is what am-store is).

**Complexity:** Low. Logging change only.

### M3: CWD-first resolution caveat

**Current behavior:** `$CWD/.am.config.toml` is checked first.

**Proposed spec change:** Add a caveat in the spec noting that CWD-first is unpredictable in MCP stdio deployments where the parent process controls CWD. Recommend that MCP server entry points set `AM_DATA_DIR` explicitly rather than relying on file resolution.

**Complexity:** None (spec-only).

### M4: No --data-dir CLI arg

**Current behavior:** No CLI-level config override exists.

**Assessment:** Intentional omission. Env vars serve this purpose (`AM_DATA_DIR`). Adding `--data-dir` creates a fourth precedence layer. Not worth the complexity for v1.

**Proposed spec change:** Add a note documenting this as a deliberate choice, not an oversight. If a fourth layer is needed later, it goes above env vars.

**Complexity:** None (spec-only).

## Triage

| Item | Linear issue? | Rationale |
|------|---------------|-----------|
| P1: Warn on parse failures | No | One-line tracing::warn additions |
| P2: Post-load validation | Yes | Defines the fail-closed validation contract, updates `load()`, and sets caller behavior |
| P3: expand_tilde fallback | Yes | Implements safe home and default-path resolution under the P2 contract |
| M1: init --force transparency | No | One print statement |
| M2: WAL checkpoint logging | No | One tracing::debug change |
| M3: CWD caveat | No | Spec text only |
| M4: No --data-dir note | No | Spec text only |

## Issue Order

1. **P2 first.** This establishes the `load() -> Result<Config>` contract and the CLI/MCP startup behavior for invalid config.
2. **P3 second.** This uses the P2 contract to remove all CWD-relative home fallbacks.
3. **P1, M1, M2 can land independently.**
4. **M3 and M4 stay in the spec update.**

## Execution plan

1. **Spec update:** Update CONFIG_MANAGEMENT.md with all changes (after approval)
2. **Branch:** `feat/config-hardening`
3. **Direct fixes (P1, M1, M2):** Implement on branch
4. **Linear issue P2:** Create and execute the validation contract change
5. **Linear issue P3:** Create and execute the home/path hardening change after P2 lands
6. **Spec-only (M3, M4):** Include in the spec update
