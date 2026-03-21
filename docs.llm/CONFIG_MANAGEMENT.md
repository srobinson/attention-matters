# Config Management Standard

Specification for configuration management across the helioy ecosystem. Derived from the attention-matters reference implementation (`am-store/src/config.rs`).

Every helioy project that accepts runtime configuration MUST follow this standard.

## Principles

1. **Zero config by default.** A fresh install with no config file and no env vars MUST produce working behavior with sensible defaults.
2. **Three layers, strict precedence.** Compiled defaults < config file < environment variables. No exceptions.
3. **Partial configs are valid.** A config file that sets one field and omits the rest MUST work. Missing fields inherit from the layer below.
4. **Fail open on config errors.** A malformed config file logs a warning and falls back to defaults. A missing config file is not an error.
5. **No silent data destruction.** Config loading MUST NOT cause data loss. Guard overwrite paths explicitly.

## Layering

```
Priority (highest wins)
========================
3. Environment variables     AM_*  / PROJECT_*
2. Config file               .project.config.toml
1. Compiled defaults         Default::default()
```

### Layer 1: Compiled defaults

Every config field has a `Default` impl. Defaults live in the code, not in a shipped config file. Domain-relevant constants (thresholds, retention windows) live in the core crate and are re-exported so the config layer references them by name.

```rust
impl Default for Config {
    fn default() -> Self {
        let data_dir = project::default_base_dir()
            .unwrap_or_else(|_| PathBuf::from("~/.attention-matters"));
        Self {
            data_dir,
            gc_enabled: false,
            db_size_mb: 50,
            // ...
        }
    }
}
```

`Default::default()` is a best-effort fallback for contexts that cannot propagate errors (e.g. struct update syntax). The runtime load path uses `runtime_defaults()` instead, which returns `Result` and avoids the `unwrap_or_else`.

### Layer 2: Config file

**Format:** TOML. Parsed with `serde::Deserialize`.

**File resolution** (first match wins):

1. `$CWD/.<project>.config.toml` - project-local override
2. `$<PROJECT>_DATA_DIR/.<project>.config.toml` - custom data directory
3. `~/.<project>/.<project>.config.toml` - global fallback

**Caveat:** CWD-first resolution is unpredictable in MCP stdio deployments where the parent process controls CWD. MCP server entry points should set the `<PROJECT>_DATA_DIR` env var explicitly rather than relying on file resolution.

**Partial deserialization pattern:** Use a separate `FileConfig` struct where every field is `Option<T>`. This decouples the file schema from the resolved config and means missing keys silently pass through to defaults.

```rust
#[derive(Deserialize, Default)]
struct FileConfig {
    data_dir: Option<String>,
    gc_enabled: Option<bool>,
    db_size_mb: Option<u64>,
    retention: Option<FileRetentionConfig>,
}
```

Apply file values with explicit `if let Some(v)` per field. This keeps the merge logic visible and auditable.

### Layer 3: Environment variables

**Naming convention:** `<PROJECT>_<FIELD>` in SCREAMING_SNAKE_CASE.

| Pattern | Example |
|---------|---------|
| Base data directory | `AM_DATA_DIR` |
| Boolean toggle | `AM_GC_ENABLED` |
| Numeric limit | `AM_DB_SIZE_MB` |
| Optional path | `AM_SYNC_LOG_DIR` |

**Parsing rules:**
- Use `env::var("NAME")` with `.ok()` or `if let Ok(val)` - missing vars are not errors
- Parse typed values explicitly: `val.parse::<bool>()`, `val.parse::<u64>()`
- If parse fails, log `tracing::warn!` with the var name, raw value, and expected type, then fall back to the file/default value
- Path values support tilde expansion (`~/` replaced with `$HOME`)

## Validation

Config loading distinguishes two categories of error:

- **Syntax/deserialization errors** (malformed TOML, unknown keys): warn and fall back to defaults. Fail open.
- **Semantic validation errors** (parseable but nonsensical resolved values): return error. Fail closed.

After `load()` merges all three layers, it calls `validate()` before returning. Validation rules catch values that would cause silent data loss or undefined behavior at runtime.

### Required validation rules

| Field | Rule | Rationale |
|-------|------|-----------|
| `db_size_mb` | >= 1 when `gc_enabled == true` | Zero-byte GC target evicts everything |
| `data_dir` | Must not be empty | Empty string resolves to CWD |
| `data_dir` | Must be absolute after tilde expansion | Relative paths write to unpredictable locations |

### Error behavior

Return `StoreError::InvalidData(String)` with a message naming the offending field and the rule it violated. Callers (CLI commands, MCP server startup) exit non-zero before opening the database.

`am init` is exempt because it generates config rather than consuming it.

## Config Structs

Every project needs two config types:

### Resolved config (public API)

```rust
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    // all fields concrete types, no Options
}
```

- All fields are concrete (no `Option` except genuinely optional features)
- Implements `Default`
- This is what the rest of the codebase consumes

### File config (internal)

```rust
#[derive(Deserialize, Default)]
struct FileConfig {
    data_dir: Option<String>,
    // every field Option<T>
}
```

- All fields `Option<T>`
- `#[derive(Deserialize, Default)]`
- Never exposed outside the config module
- Nested sections get their own `FileXxxConfig` struct

## Loading Function

Every project exposes a single `pub fn load() -> Result<Config>` that orchestrates all three layers and validates the result:

```rust
pub fn load() -> crate::error::Result<Config> {
    // Layer 1: runtime defaults. If home is unresolvable, use an empty
    // data_dir placeholder. File config or env vars may override it
    // before validation catches an invalid final state.
    let mut cfg = match runtime_defaults() {
        Ok(defaults) => defaults,
        Err(_) => Config {
            data_dir: PathBuf::new(),
            ..Default::default()
        },
    };

    if let Some(path) = find_config_file() {    // Layer 2
        apply_file_config(&mut cfg, &path)?;
    }

    // Layer 3: env vars override everything
    if let Ok(dir) = env::var("PROJECT_DATA_DIR") {
        cfg.data_dir = expand_tilde(&dir)?;
    }
    // ... remaining env var overrides

    cfg.validate()?;
    Ok(cfg)
}
```

**Key properties:**
- Pure function of filesystem + environment state
- No global mutable state
- Can be called multiple times safely
- Returns `Result<Config>` - validation errors are propagated, not swallowed
- Returns owned `Config`, not a reference to a singleton
- Home resolution failure is not immediately fatal: file config or env vars can supply an absolute `data_dir`, bypassing the need for home. Validation catches unresolved paths at the end.

## Path Handling

All path config values MUST support tilde expansion:

```rust
pub fn resolve_home_dir() -> crate::error::Result<PathBuf> {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .map_err(|_| StoreError::InvalidData(
            "Cannot resolve home directory: HOME is not set".into()
        ))
}

fn expand_tilde(path: &str) -> crate::error::Result<PathBuf> {
    if let Some(rest) = path.strip_prefix("~/") {
        Ok(resolve_home_dir()?.join(rest))
    } else {
        Ok(PathBuf::from(path))
    }
}
```

- `~/` expands to `$HOME` (Unix) or `$USERPROFILE` (Windows)
- Absolute paths pass through unchanged
- If neither home var is set, return an error. Do not fall back to CWD.
- One shared `resolve_home_dir()` helper; no duplicated fallback logic

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Config file missing | No error. Use defaults. |
| Config file malformed | Log warning. Use defaults. |
| Env var missing | Skip silently. |
| Env var parse failure | Log `tracing::warn!` with var name, raw value, and expected type. Fall back to file/default value. |
| Semantic validation failure | Return `StoreError::InvalidData`. Caller exits non-zero. |
| Home directory unresolvable | Use empty placeholder for `data_dir`. File config or env vars can override with an absolute path. If `data_dir` is still empty or relative after all layers merge, `validate()` returns `StoreError::InvalidData`. No CWD fallback. |
| Data directory missing | Create it (at database init time, not config load). |

Config loading MUST NOT panic. Config loading returns `Result` - syntax/deserialization errors fall through to defaults (fail open), but semantic validation errors on the resolved config are returned as errors (fail closed).

## Init Command

Every CLI project SHOULD provide an `init` subcommand that generates a fully commented default config:

```
project init          # writes .project.config.toml in CWD
project init --global # writes to ~/.project/.project.config.toml
project init --force  # overwrite existing (prints path being overwritten)
```

`init --force` overwrites without prompting. The output always prints the path that was written, so the user sees what happened. No diff or confirmation (that is the purpose of --force).

The generated file comments out all values with their defaults, serving as inline documentation:

```toml
# project-name configuration
#
# Config file resolution (first found wins):
#   1. $CWD/.project.config.toml   (project-local)
#   2. ~/.project/.project.config.toml  (global fallback)
#
# Environment variables override all file settings:
#   PROJECT_DATA_DIR, PROJECT_GC_ENABLED, PROJECT_DB_SIZE_MB

# Directory where the database and state files are stored.
# Override with PROJECT_DATA_DIR env var.
# data_dir = "~/.project"
```

## Database Configuration

Projects using SQLite MUST configure these pragmas at connection time:

```sql
PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;
PRAGMA busy_timeout = 5000;
PRAGMA wal_autocheckpoint = 100;
PRAGMA wal_checkpoint(TRUNCATE);   -- startup only, blocks readers
```

Database path derives from `config.data_dir`, not from a separate config field. The convention is `{data_dir}/{db_name}.db`.

**WAL checkpoint note:** `PRAGMA wal_checkpoint(TRUNCATE)` at startup blocks concurrent readers for the duration of the checkpoint. This is acceptable for single-writer use cases. Log checkpoint failures at `tracing::debug!` rather than swallowing them silently.

## Tracing Configuration

```rust
// Default: WARN level, stderr output
// --verbose flag: DEBUG level
// RUST_LOG env var: full control via EnvFilter

let filter = if verbose {
    EnvFilter::new("debug")
} else {
    EnvFilter::from_default_env().add_directive(LevelFilter::WARN.into())
};

tracing_subscriber::fmt()
    .with_env_filter(filter)
    .with_writer(std::io::stderr)
    .init();
```

**Note on CLI arg overrides:** There is no `--data-dir` or `--config` CLI flag by design. Env vars serve this purpose. Adding CLI args would introduce a fourth precedence layer. If a fourth layer is needed in the future, it goes above env vars.

All projects MUST:
- Default to `WARN`
- Support `--verbose` for `DEBUG`
- Respect `RUST_LOG` for fine-grained control
- Write to stderr (never stdout, which is reserved for program output)

## Testing

Config modules MUST include tests for:

1. **Default sanity** - `Config::default()` produces valid, expected values
2. **Tilde expansion** - `~/foo` expands, `/abs/path` passes through, unset `$HOME` returns error
3. **Partial TOML** - A file with one field set parses correctly
4. **Nested sections** - Subsections (like `[retention]`) parse independently
5. **Default anchoring** - Config defaults match core crate constants
6. **Validation rules** - Each semantic validation rule (db_size_mb minimum, data_dir non-empty, data_dir absolute)
7. **Startup failure** - At least one CLI integration test asserting non-zero exit on invalid resolved config

```rust
#[test]
fn defaults_are_sane() {
    let cfg = Config::default();
    assert!(!cfg.gc_enabled);
    assert_eq!(cfg.db_size_mb, 50);
}

#[test]
fn parse_toml_partial() {
    let content = "gc_enabled = true\n";
    let file_cfg: FileConfig = toml::from_str(content).unwrap();
    assert_eq!(file_cfg.gc_enabled, Some(true));
    assert_eq!(file_cfg.data_dir, None);
}
```

## Checklist for New Projects

- [ ] `Config` struct with `Default` impl
- [ ] `FileConfig` struct with all-`Option` fields
- [ ] `load()` function returning `Result<Config>` with three-layer precedence
- [ ] `validate()` with semantic checks (db_size_mb, data_dir)
- [ ] `find_config_file()` with CWD-first resolution
- [ ] `resolve_home_dir()` shared helper (no CWD fallback)
- [ ] `expand_tilde()` returning `Result<PathBuf>`
- [ ] Env var overrides with `<PROJECT>_*` prefix (warn on parse failure)
- [ ] `init` subcommand generating commented defaults (--force overwrites without prompting, prints written path)
- [ ] Config tests (defaults, tilde, partial TOML, nested sections, validation rules, startup failure)
- [ ] Tracing setup (WARN default, --verbose, RUST_LOG)
- [ ] SQLite pragmas (if applicable, log checkpoint failures at debug)
- [ ] No panics in config loading path
