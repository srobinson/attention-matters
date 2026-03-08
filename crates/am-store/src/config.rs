use std::path::{Path, PathBuf};
use std::{env, fs};

use serde::Deserialize;

/// Default DB size limit for GC (50 MB).
const DEFAULT_DB_SIZE_MB: u64 = 50;

/// Partial config deserialized from TOML. All fields optional so that
/// missing keys fall through to defaults.
#[derive(Deserialize, Default)]
struct FileConfig {
    data_dir: Option<String>,
    gc_enabled: Option<bool>,
    db_size_mb: Option<u64>,
    sync_log_dir: Option<String>,
    retention: Option<FileRetentionConfig>,
}

/// Partial retention config from TOML.
#[derive(Deserialize, Default)]
struct FileRetentionConfig {
    grace_epochs: Option<u64>,
    retention_days: Option<u64>,
    min_neighborhoods: Option<u64>,
    recency_weight: Option<f64>,
}

/// Resolved retention policy with concrete values.
#[derive(Debug, Clone)]
pub struct RetentionPolicy {
    /// Neighborhoods within this many epochs of the max are GC-exempt.
    pub grace_epochs: u64,
    /// Neighborhoods newer than this many days are GC-exempt.
    pub retention_days: u64,
    /// Skip GC entirely if total neighborhoods are below this count.
    pub min_neighborhoods: u64,
    /// Recency bonus weight in composite eviction scoring.
    pub recency_weight: f64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        Self {
            grace_epochs: am_core::DEFAULT_GRACE_EPOCHS,
            retention_days: am_core::DEFAULT_RETENTION_DAYS,
            min_neighborhoods: am_core::DEFAULT_MIN_NEIGHBORHOODS,
            recency_weight: am_core::DEFAULT_RECENCY_WEIGHT,
        }
    }
}

/// Resolved configuration with concrete values.
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub gc_enabled: bool,
    pub db_size_mb: u64,
    pub sync_log_dir: Option<PathBuf>,
    pub retention: RetentionPolicy,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: crate::project::default_base_dir(),
            gc_enabled: false,
            db_size_mb: DEFAULT_DB_SIZE_MB,
            sync_log_dir: None,
            retention: RetentionPolicy::default(),
        }
    }
}

impl Config {
    /// DB size limit in bytes.
    pub fn db_size_limit_bytes(&self) -> u64 {
        self.db_size_mb * 1024 * 1024
    }
}

/// Load configuration with the following precedence (highest wins):
///
/// 1. Environment variables (`AM_DATA_DIR`, `AM_GC_ENABLED`, `AM_DB_SIZE_MB`, `AM_SYNC_LOG_DIR`)
/// 2. Config file (first found wins):
///    a. `$CWD/.am.config.toml` (project-local)
///    b. `$AM_DATA_DIR/.am.config.toml` (if env var is set)
///    c. `~/.attention-matters/.am.config.toml` (global fallback)
/// 3. Compiled defaults
///
/// The config file's `data_dir` field controls where the database lives.
/// `AM_DATA_DIR` overrides `data_dir` from the file.
pub fn load() -> Config {
    let mut cfg = Config::default();

    // Find config file: CWD first, then global fallback
    let config_path = find_config_file();
    if let Some(path) = &config_path {
        apply_file_config(&mut cfg, path);
    }

    // Env vars override everything
    if let Ok(dir) = env::var("AM_DATA_DIR") {
        cfg.data_dir = expand_tilde(&dir);
    }
    if let Ok(val) = env::var("AM_GC_ENABLED")
        && let Ok(b) = val.parse::<bool>()
    {
        cfg.gc_enabled = b;
    }
    if let Ok(val) = env::var("AM_DB_SIZE_MB")
        && let Ok(mb) = val.parse::<u64>()
    {
        cfg.db_size_mb = mb;
    }
    if let Ok(val) = env::var("AM_SYNC_LOG_DIR") {
        cfg.sync_log_dir = Some(expand_tilde(&val));
    }

    cfg
}

/// Find the config file (first match wins):
///   1. CWD/.am.config.toml
///   2. $AM_DATA_DIR/.am.config.toml (if set)
///   3. ~/.attention-matters/.am.config.toml
fn find_config_file() -> Option<PathBuf> {
    const CONFIG_NAME: &str = ".am.config.toml";

    // Check CWD
    if let Ok(cwd) = env::current_dir() {
        let local = cwd.join(CONFIG_NAME);
        if local.exists() {
            return Some(local);
        }
    }

    // Check AM_DATA_DIR
    if let Ok(dir) = env::var("AM_DATA_DIR") {
        let project = expand_tilde(&dir).join(CONFIG_NAME);
        if project.exists() {
            return Some(project);
        }
    }

    // Fall back to global
    let global = crate::project::default_base_dir().join(CONFIG_NAME);
    if global.exists() {
        return Some(global);
    }

    None
}

fn apply_file_config(cfg: &mut Config, path: &Path) {
    if let Some(file_cfg) = read_config_file(path) {
        if let Some(dir) = file_cfg.data_dir {
            cfg.data_dir = expand_tilde(&dir);
        }
        if let Some(gc) = file_cfg.gc_enabled {
            cfg.gc_enabled = gc;
        }
        if let Some(size) = file_cfg.db_size_mb {
            cfg.db_size_mb = size;
        }
        if let Some(dir) = file_cfg.sync_log_dir {
            cfg.sync_log_dir = Some(expand_tilde(&dir));
        }
        if let Some(ret) = file_cfg.retention {
            if let Some(v) = ret.grace_epochs {
                cfg.retention.grace_epochs = v;
            }
            if let Some(v) = ret.retention_days {
                cfg.retention.retention_days = v;
            }
            if let Some(v) = ret.min_neighborhoods {
                cfg.retention.min_neighborhoods = v;
            }
            if let Some(v) = ret.recency_weight {
                cfg.retention.recency_weight = v;
            }
        }
    }
}

fn read_config_file(path: &Path) -> Option<FileConfig> {
    let content = fs::read_to_string(path).ok()?;
    match toml::from_str(&content) {
        Ok(cfg) => Some(cfg),
        Err(e) => {
            tracing::warn!("failed to parse {}: {e}", path.display());
            None
        }
    }
}

/// Generate a fully commented default config file.
pub fn generate_default_toml() -> String {
    let defaults = Config::default();
    let ret = &defaults.retention;
    let data_dir = defaults
        .data_dir
        .to_string_lossy()
        .replace(&std::env::var("HOME").unwrap_or_default(), "~");

    format!(
        r#"# attention-matters configuration
#
# Config file resolution (first found wins):
#   1. $CWD/.am.config.toml   (project-local)
#   2. ~/.attention-matters/.am.config.toml  (global fallback)
#
# Environment variables override all file settings:
#   AM_DATA_DIR, AM_GC_ENABLED, AM_DB_SIZE_MB, AM_SYNC_LOG_DIR

# Directory where the database and state files are stored.
# This is how you point a project at a specific brain.
# Override with AM_DATA_DIR env var.
# data_dir = "{data_dir}"

# Enable automatic garbage collection.
# Override with AM_GC_ENABLED env var.
# gc_enabled = {gc_enabled}

# Database size limit in MB for GC target sizing.
# Override with AM_DB_SIZE_MB env var.
# db_size_mb = {db_size_mb}

# Directory to write sync logs into. Disabled when unset.
# Override with AM_SYNC_LOG_DIR env var.
# sync_log_dir = "{data_dir}/sync-logs"

[retention]
# Neighborhoods within this many epochs of the max are GC-exempt.
# grace_epochs = {grace_epochs}

# Neighborhoods newer than this many days are GC-exempt.
# retention_days = {retention_days}

# Skip GC entirely if total neighborhoods are below this count.
# min_neighborhoods = {min_neighborhoods}

# Recency bonus weight in composite eviction scoring.
# recency_weight = {recency_weight}
"#,
        data_dir = data_dir,
        gc_enabled = defaults.gc_enabled,
        db_size_mb = defaults.db_size_mb,
        grace_epochs = ret.grace_epochs,
        retention_days = ret.retention_days,
        min_neighborhoods = ret.min_neighborhoods,
        recency_weight = ret.recency_weight,
    )
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let cfg = Config::default();
        assert!(!cfg.gc_enabled);
        assert_eq!(cfg.db_size_mb, 50);
        assert_eq!(cfg.db_size_limit_bytes(), 50 * 1024 * 1024);
    }

    #[test]
    fn expand_tilde_works() {
        let expanded = expand_tilde("~/foo/bar");
        assert!(!expanded.to_string_lossy().starts_with("~/"));
        assert!(expanded.to_string_lossy().ends_with("foo/bar"));
    }

    #[test]
    fn expand_tilde_absolute_passthrough() {
        let p = expand_tilde("/absolute/path");
        assert_eq!(p, PathBuf::from("/absolute/path"));
    }

    #[test]
    fn parse_toml_partial() {
        let content = "gc_enabled = true\n";
        let file_cfg: FileConfig = toml::from_str(content).unwrap();
        assert_eq!(file_cfg.gc_enabled, Some(true));
        assert_eq!(file_cfg.data_dir, None);
        assert_eq!(file_cfg.db_size_mb, None);
    }

    #[test]
    fn parse_toml_retention() {
        let content = r#"
[retention]
grace_epochs = 25
retention_days = 7
min_neighborhoods = 200
recency_weight = 3.0
"#;
        let file_cfg: FileConfig = toml::from_str(content).unwrap();
        let ret = file_cfg.retention.unwrap();
        assert_eq!(ret.grace_epochs, Some(25));
        assert_eq!(ret.retention_days, Some(7));
        assert_eq!(ret.min_neighborhoods, Some(200));
        assert!((ret.recency_weight.unwrap() - 3.0).abs() < 1e-10);
    }

    #[test]
    fn parse_toml_retention_partial() {
        let content = "[retention]\ngrace_epochs = 10\n";
        let file_cfg: FileConfig = toml::from_str(content).unwrap();
        let ret = file_cfg.retention.unwrap();
        assert_eq!(ret.grace_epochs, Some(10));
        assert_eq!(ret.retention_days, None);
        assert_eq!(ret.min_neighborhoods, None);
        assert_eq!(ret.recency_weight, None);
    }

    #[test]
    fn parse_toml_sync_log_dir() {
        let content = "sync_log_dir = \"~/logs/am-sync\"\n";
        let file_cfg: FileConfig = toml::from_str(content).unwrap();
        assert_eq!(file_cfg.sync_log_dir.as_deref(), Some("~/logs/am-sync"));
    }

    #[test]
    fn retention_defaults() {
        let policy = RetentionPolicy::default();
        assert_eq!(policy.grace_epochs, am_core::DEFAULT_GRACE_EPOCHS);
        assert_eq!(policy.retention_days, am_core::DEFAULT_RETENTION_DAYS);
        assert_eq!(policy.min_neighborhoods, am_core::DEFAULT_MIN_NEIGHBORHOODS);
        assert!((policy.recency_weight - am_core::DEFAULT_RECENCY_WEIGHT).abs() < 1e-10);
    }
}
