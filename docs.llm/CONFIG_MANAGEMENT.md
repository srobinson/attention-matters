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
        Self {
            data_dir: project::default_base_dir(),
            gc_enabled: false,
            db_size_mb: 50,
            // ...
        }
    }
}
```

### Layer 2: Config file

**Format:** TOML. Parsed with `serde::Deserialize`.

**File resolution** (first match wins):

1. `$CWD/.<project>.config.toml` - project-local override
2. `$<PROJECT>_DATA_DIR/.<project>.config.toml` - custom data directory
3. `~/.<project>/.<project>.config.toml` - global fallback

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
- If parse fails, silently skip (the file or default value stands)
- Path values support tilde expansion (`~/` replaced with `$HOME`)

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

Every project exposes a single `pub fn load() -> Config` that orchestrates all three layers:

```rust
pub fn load() -> Config {
    let mut cfg = Config::default();           // Layer 1

    if let Some(path) = find_config_file() {   // Layer 2
        apply_file_config(&mut cfg, &path);
    }

    // Layer 3: env vars override everything
    if let Ok(dir) = env::var("PROJECT_DATA_DIR") {
        cfg.data_dir = expand_tilde(&dir);
    }
    // ... remaining env var overrides

    cfg
}
```

**Key properties:**
- Pure function of filesystem + environment state
- No global mutable state
- Can be called multiple times safely
- Returns owned `Config`, not a reference to a singleton

## Path Handling

All path config values MUST support tilde expansion:

```rust
fn expand_tilde(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(rest)
    } else {
        PathBuf::from(path)
    }
}
```

- `~/` expands to `$HOME` (Unix) or `$USERPROFILE` (Windows)
- Absolute paths pass through unchanged
- Fallback to `.` if neither home var is set

## Error Handling

| Scenario | Behavior |
|----------|----------|
| Config file missing | No error. Use defaults. |
| Config file malformed | Log warning. Use defaults. |
| Env var missing | Skip silently. |
| Env var parse failure | Skip silently. File/default value stands. |
| Data directory missing | Create it (at database init time, not config load). |

Config loading MUST NOT panic. Config loading MUST NOT return `Result` - it always succeeds by falling through to defaults.

## Init Command

Every CLI project SHOULD provide an `init` subcommand that generates a fully commented default config:

```
project init          # writes .project.config.toml in CWD
project init --global # writes to ~/.project/.project.config.toml
project init --force  # overwrite existing
```

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
PRAGMA wal_checkpoint(TRUNCATE);   -- startup only
```

Database path derives from `config.data_dir`, not from a separate config field. The convention is `{data_dir}/{db_name}.db`.

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

All projects MUST:
- Default to `WARN`
- Support `--verbose` for `DEBUG`
- Respect `RUST_LOG` for fine-grained control
- Write to stderr (never stdout, which is reserved for program output)

## Testing

Config modules MUST include tests for:

1. **Default sanity** - `Config::default()` produces valid, expected values
2. **Tilde expansion** - `~/foo` expands, `/abs/path` passes through
3. **Partial TOML** - A file with one field set parses correctly
4. **Nested sections** - Subsections (like `[retention]`) parse independently
5. **Default anchoring** - Config defaults match core crate constants

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
- [ ] `load()` function with three-layer precedence
- [ ] `find_config_file()` with CWD-first resolution
- [ ] `expand_tilde()` for path values
- [ ] Env var overrides with `<PROJECT>_*` prefix
- [ ] `init` subcommand generating commented defaults
- [ ] Config tests (defaults, tilde, partial TOML, nested sections)
- [ ] Tracing setup (WARN default, --verbose, RUST_LOG)
- [ ] SQLite pragmas (if applicable)
- [ ] No panics in config loading path
