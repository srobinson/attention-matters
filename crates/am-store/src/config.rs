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
}

/// Resolved configuration with concrete values.
#[derive(Debug, Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub gc_enabled: bool,
    pub db_size_mb: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            data_dir: crate::project::default_base_dir(),
            gc_enabled: false,
            db_size_mb: DEFAULT_DB_SIZE_MB,
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
/// 1. Environment variables (`AM_DATA_DIR`, `AM_GC_ENABLED`, `AM_DB_SIZE_MB`)
/// 2. Project config (`$AM_DATA_DIR/.am.config.toml`, if AM_DATA_DIR is set)
/// 3. Global config (`~/.attention-matters/.am.config.toml`)
/// 4. Compiled defaults
pub fn load() -> Config {
    let mut cfg = Config::default();
    let default_dir = cfg.data_dir.clone();

    // Layer 1: global config (~/.attention-matters/.am.config.toml)
    apply_file_config(&mut cfg, &default_dir.join(".am.config.toml"));

    // Layer 2: project config ($AM_DATA_DIR/.am.config.toml)
    // Only read if AM_DATA_DIR points somewhere different from the default.
    if let Ok(dir) = env::var("AM_DATA_DIR") {
        let project_dir = expand_tilde(&dir);
        if project_dir != default_dir {
            apply_file_config(&mut cfg, &project_dir.join(".am.config.toml"));
        }
        cfg.data_dir = project_dir;
    }

    // Layer 3: env vars override everything
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

    cfg
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
}
