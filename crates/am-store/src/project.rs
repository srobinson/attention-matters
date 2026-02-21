use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use am_core::DAESystem;

use crate::error::{Result, StoreError};
use crate::store::Store;

/// Default base directory for all am storage.
pub fn default_base_dir() -> PathBuf {
    dirs_home().join(".attention-matters")
}

fn dirs_home() -> PathBuf {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

// ---------------------------------------------------------------------------
// Pure parsing helpers (no I/O, fully unit-testable)
// ---------------------------------------------------------------------------

/// Extract the value between the first pair of matching quotes.
/// Handles both `"value"` and `'value'`.
fn extract_quoted(s: &str) -> Option<&str> {
    for quote in ['"', '\''] {
        if let Some(start) = s.find(quote)
            && let Some(end) = s[start + 1..].find(quote)
        {
            return Some(&s[start + 1..start + 1 + end]);
        }
    }
    None
}

/// Get the path portion of a `scheme://...` URL (everything after `://host/`).
fn extract_url_path(url: &str) -> Option<&str> {
    let after_scheme = url.find("://").map(|i| &url[i + 3..])?;
    let after_host = after_scheme.find('/').map(|i| &after_scheme[i + 1..])?;
    if after_host.is_empty() {
        None
    } else {
        Some(after_host)
    }
}

/// Parse a git remote URL into a `org_repo` identifier.
///
/// Handles:
/// - `git@github.com:org/repo.git` (SCP-style SSH)
/// - `https://github.com/org/repo.git` (HTTPS)
/// - `ssh://git@github.com/org/repo.git` (SSH with scheme)
/// - GitLab subgroups: uses last two path segments
/// - Strips `.git` suffix
fn parse_remote_url(url: &str) -> Option<String> {
    let path = if let Some(colon_pos) = url.find(':') {
        // SCP-style: git@host:path — but not scheme://
        if url[..colon_pos].contains("//") {
            // Has scheme → extract path after host
            extract_url_path(url)?
        } else {
            &url[colon_pos + 1..]
        }
    } else {
        return None;
    };

    // Strip .git suffix and leading/trailing slashes
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);

    if path.is_empty() {
        return None;
    }

    // Take last two segments (handles GitLab subgroups: a/b/c/repo → c_repo)
    let segments: Vec<&str> = path.split('/').collect();
    let identity = if segments.len() >= 2 {
        format!(
            "{}_{}",
            segments[segments.len() - 2],
            segments[segments.len() - 1]
        )
    } else {
        segments[0].to_string()
    };

    if identity.is_empty() {
        None
    } else {
        Some(identity)
    }
}

/// Find `name = "value"` under a `[section]` header in TOML content.
/// Stops searching at the next section header.
fn extract_toml_name(content: &str, section: &str) -> Option<String> {
    let header = format!("[{section}]");
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == header {
            in_section = true;
            continue;
        }
        if in_section {
            if trimmed.starts_with('[') {
                break; // Hit next section
            }
            if let Some(rest) = trimmed.strip_prefix("name") {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix('=')
                    && let Some(name) = extract_quoted(rest)
                    && !name.is_empty()
                {
                    return Some(name.to_string());
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// I/O wrappers (thin shells around pure logic)
// ---------------------------------------------------------------------------

/// Detect git root from a directory by running `git rev-parse --show-toplevel`.
fn detect_git_root(from: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .arg("rev-parse")
        .arg("--show-toplevel")
        .current_dir(from)
        .output()
        .ok()?;

    if output.status.success() {
        let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if root.is_empty() {
            None
        } else {
            Some(PathBuf::from(root))
        }
    } else {
        None
    }
}

/// Shell out to `git remote get-url origin` and parse the result.
fn detect_git_remote_identity(from: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(from)
        .output()
        .ok()?;

    if output.status.success() {
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        parse_remote_url(&url)
    } else {
        None
    }
}

/// Check for Cargo.toml, package.json, or pyproject.toml and extract the project name.
fn detect_manifest_name(dir: &Path) -> Option<String> {
    // Cargo.toml → [package] name
    let cargo = dir.join("Cargo.toml");
    if let Ok(content) = fs::read_to_string(&cargo)
        && let Some(name) = extract_toml_name(&content, "package")
    {
        return Some(name);
    }

    // package.json → "name": "value"
    let pkg = dir.join("package.json");
    if let Ok(content) = fs::read_to_string(&pkg) {
        for line in content.lines() {
            let trimmed = line.trim();
            if let Some(rest) = trimmed.strip_prefix("\"name\"") {
                let rest = rest.trim_start();
                if let Some(rest) = rest.strip_prefix(':') {
                    let rest = rest.trim_start().trim_end_matches(',');
                    if let Some(name) = extract_quoted(rest)
                        && !name.is_empty()
                    {
                        return Some(name.to_string());
                    }
                }
            }
        }
    }

    // pyproject.toml → [project] name
    let pyproject = dir.join("pyproject.toml");
    if let Ok(content) = fs::read_to_string(&pyproject)
        && let Some(name) = extract_toml_name(&content, "project")
    {
        return Some(name);
    }

    None
}

// ---------------------------------------------------------------------------
// Project identity resolution
// ---------------------------------------------------------------------------

/// Resolve a project identifier to a human-readable database filename stem.
///
/// Priority chain:
/// 1. Explicit `--project` name
/// 2. Git remote origin → `org_repo`
/// 3. Git repo root directory basename
/// 4. Manifest name (Cargo.toml, package.json, pyproject.toml)
/// 5. CWD basename (last resort, never hash)
fn resolve_project_id(project_name: Option<&str>) -> String {
    // 1. Explicit override
    if let Some(name) = project_name {
        let sanitized = sanitize_name(name);
        if !sanitized.is_empty() {
            return sanitized;
        }
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    if let Some(git_root) = detect_git_root(&cwd) {
        // 2. Git remote origin identity
        if let Some(identity) = detect_git_remote_identity(&git_root) {
            return sanitize_name(&identity);
        }

        // 3. Git root basename
        if let Some(basename) = git_root.file_name() {
            let name = sanitize_name(&basename.to_string_lossy());
            if !name.is_empty() {
                return name;
            }
        }
    }

    // 4. Manifest name
    if let Some(name) = detect_manifest_name(&cwd) {
        let sanitized = sanitize_name(&name);
        if !sanitized.is_empty() {
            return sanitized;
        }
    }

    // 5. CWD basename (never hash)
    cwd.file_name()
        .map(|n| sanitize_name(&n.to_string_lossy()))
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "unnamed".to_string())
}

/// Sanitize a project name for use as a filename.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Startup GC — automatic size management
// ---------------------------------------------------------------------------

/// Run automatic GC if the project DB exceeds the soft size limit.
fn startup_gc(store: &Store) {
    let db_size = store.db_size();
    if db_size < am_core::DB_SOFT_LIMIT_BYTES {
        return;
    }

    tracing::info!(
        "DB size {}MB exceeds {}MB soft limit — running GC",
        db_size / (1024 * 1024),
        am_core::DB_SOFT_LIMIT_BYTES / (1024 * 1024),
    );

    // Phase 1: evict occurrences at or below the activation floor
    match store.gc_pass(am_core::ACTIVATION_FLOOR) {
        Ok(result) => {
            tracing::info!(
                "GC phase 1: evicted {} occurrences (activation <= {}), \
                 removed {} empty episodes. DB: {}MB → {}MB",
                result.evicted_occurrences,
                am_core::ACTIVATION_FLOOR,
                result.removed_episodes,
                result.before_size / (1024 * 1024),
                result.after_size / (1024 * 1024),
            );

            // Phase 2: if still over limit, aggressively evict coldest
            if result.after_size >= am_core::DB_SOFT_LIMIT_BYTES {
                let target =
                    (am_core::DB_SOFT_LIMIT_BYTES as f64 * am_core::DB_GC_TARGET_RATIO) as u64;
                match store.gc_to_target_size(target) {
                    Ok(r2) => {
                        tracing::info!(
                            "GC phase 2 (aggressive): evicted {} more occurrences, \
                             removed {} episodes. DB: {}MB → {}MB",
                            r2.evicted_occurrences,
                            r2.removed_episodes,
                            r2.before_size / (1024 * 1024),
                            r2.after_size / (1024 * 1024),
                        );
                    }
                    Err(e) => tracing::warn!("GC phase 2 failed: {e}"),
                }
            }
        }
        Err(e) => tracing::warn!("GC phase 1 failed: {e}"),
    }
}

// ---------------------------------------------------------------------------
// Migration — one-time merge from old multi-DB layout to single brain.db
// ---------------------------------------------------------------------------

/// Migrate the old `projects/*.db` + `global.db` layout into a single `brain.db`.
///
/// Only runs when `projects/` exists and `brain.db` does not. After merging,
/// renames `projects/` to `projects.migrated/` and `global.db` to
/// `global.db.migrated` (belt and suspenders — never deletes).
fn migrate_old_layout(base: &Path, brain_path: &Path, _current_project_id: &str) {
    let projects_dir = base.join("projects");
    let global_path = base.join("global.db");

    // Only migrate if brain.db doesn't exist yet
    if brain_path.exists() {
        return;
    }

    tracing::info!("migrating old layout to brain.db");

    let brain_store = match Store::open(brain_path) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("failed to open brain.db for migration: {e}");
            return;
        }
    };

    let mut brain_system = brain_store
        .load_system()
        .unwrap_or_else(|_| DAESystem::new("am"));

    // Merge all project DBs
    if let Ok(entries) = fs::read_dir(&projects_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("db") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();

            match Store::open(&path) {
                Ok(project_store) => match project_store.load_system() {
                    Ok(project_system) => {
                        let ep_count = project_system.episodes.len();
                        for mut episode in project_system.episodes {
                            if episode.project_id.is_empty() {
                                episode.project_id = stem.clone();
                            }
                            brain_system.add_episode(episode);
                        }
                        // Merge conscious (deduplicate by ID)
                        let existing_ids: std::collections::HashSet<uuid::Uuid> = brain_system
                            .conscious_episode
                            .neighborhoods
                            .iter()
                            .map(|n| n.id)
                            .collect();
                        for nbhd in project_system.conscious_episode.neighborhoods {
                            if !existing_ids.contains(&nbhd.id) {
                                brain_system.conscious_episode.add_neighborhood(nbhd);
                            }
                        }
                        tracing::info!("merged {} episodes from {}", ep_count, stem);
                    }
                    Err(e) => tracing::warn!("failed to load {}: {e}", path.display()),
                },
                Err(e) => tracing::warn!("failed to open {}: {e}", path.display()),
            }
        }
    }

    // Also merge global.db conscious memories
    if global_path.exists()
        && let Ok(global_store) = Store::open(&global_path)
        && let Ok(global_system) = global_store.load_system()
    {
        let existing_ids: std::collections::HashSet<uuid::Uuid> = brain_system
            .conscious_episode
            .neighborhoods
            .iter()
            .map(|n| n.id)
            .collect();
        let mut merged = 0;
        for nbhd in global_system.conscious_episode.neighborhoods {
            if !existing_ids.contains(&nbhd.id) {
                brain_system.conscious_episode.add_neighborhood(nbhd);
                merged += 1;
            }
        }
        tracing::info!("merged {} conscious neighborhoods from global.db", merged);
    }

    brain_system.mark_dirty();
    if let Err(e) = brain_store.save_system(&brain_system) {
        tracing::warn!("failed to save brain.db during migration: {e}");
        return;
    }

    // Rename old dirs to .migrated (don't delete — belt and suspenders)
    let migrated_dir = base.join("projects.migrated");
    if let Err(e) = fs::rename(&projects_dir, &migrated_dir) {
        tracing::warn!("failed to rename projects/ → projects.migrated/: {e}");
    }
    if global_path.exists() {
        let migrated_global = base.join("global.db.migrated");
        if let Err(e) = fs::rename(&global_path, &migrated_global) {
            tracing::warn!("failed to rename global.db → global.db.migrated: {e}");
        }
    }

    tracing::info!(
        "migration complete: {} episodes, {} conscious in brain.db",
        brain_system.episodes.len(),
        brain_system.conscious_episode.neighborhoods.len()
    );
}

// ---------------------------------------------------------------------------
// BrainStore — single brain.db, project_id as a tag
// ---------------------------------------------------------------------------

/// Single-database store for all developer memory.
///
/// Layout:
/// ```text
/// ~/.attention-matters/
/// └── brain.db          # all projects, project_id as tag
/// ```
pub struct BrainStore {
    store: Store,
    project_id: String,
}

impl BrainStore {
    /// Open the brain store, creating directories as needed.
    /// `project_name`: explicit project name (overrides auto-detection).
    /// `base_dir`: override the base directory (for testing).
    pub fn open(project_name: Option<&str>, base_dir: Option<&Path>) -> Result<Self> {
        let base = base_dir.map(PathBuf::from).unwrap_or_else(default_base_dir);
        fs::create_dir_all(&base).map_err(|e| {
            StoreError::InvalidData(format!("failed to create {}: {e}", base.display()))
        })?;

        let project_id = resolve_project_id(project_name);
        let brain_path = base.join("brain.db");

        // Startup migration: if old layout exists, merge into brain.db
        let projects_dir = base.join("projects");
        if projects_dir.exists() {
            migrate_old_layout(&base, &brain_path, &project_id);
        }

        let store = Store::open(&brain_path)?;
        startup_gc(&store);

        Ok(Self { store, project_id })
    }

    /// Open with an in-memory store (for testing).
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            store: Store::open_in_memory()?,
            project_id: "test".to_string(),
        })
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Load the full DAESystem from brain.db.
    pub fn load_system(&self) -> Result<DAESystem> {
        self.store.load_system()
    }

    /// Save a full DAESystem to brain.db.
    pub fn save_system(&self, system: &DAESystem) -> Result<()> {
        self.store.save_system(system)
    }

    /// Mark text as salient (conscious). Returns the neighborhood ID.
    pub fn mark_salient(
        &self,
        system: &mut DAESystem,
        text: &str,
        rng: &mut impl rand::Rng,
    ) -> Result<uuid::Uuid> {
        let nbhd_id = system.add_to_conscious(text, rng);
        self.store.save_system(system)?;
        Ok(nbhd_id)
    }

    /// Import a v0.7.2 JSON file into the brain store.
    pub fn import_json_file(&self, path: &Path) -> Result<()> {
        self.store.import_json_file(path)
    }

    /// Export the brain store to a v0.7.2 JSON file.
    pub fn export_json_file(&self, path: &Path) -> Result<()> {
        self.store.export_json_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use am_core::{Episode, Neighborhood};
    use rand::SeedableRng;
    use rand::rngs::SmallRng;

    fn rng() -> SmallRng {
        SmallRng::seed_from_u64(42)
    }

    fn to_tokens(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    fn make_system() -> DAESystem {
        let mut rng = rng();
        let mut sys = DAESystem::new("test-agent");

        let mut ep = Episode::new("episode-1");
        let tokens = to_tokens(&["hello", "world"]);
        ep.add_neighborhood(Neighborhood::from_tokens(
            &tokens,
            None,
            "hello world",
            &mut rng,
        ));
        sys.add_episode(ep);

        sys
    }

    #[test]
    fn test_brain_salient_queryable() {
        let bs = BrainStore::open_in_memory().unwrap();
        let mut sys = make_system();
        let mut rng = rng();

        bs.mark_salient(&mut sys, "important insight", &mut rng)
            .unwrap();

        let loaded = bs.load_system().unwrap();
        assert_eq!(loaded.conscious_episode.neighborhoods.len(), 1);
    }

    #[test]
    fn test_brain_roundtrip() {
        let bs = BrainStore::open_in_memory().unwrap();
        let sys = make_system();
        bs.save_system(&sys).unwrap();

        let loaded = bs.load_system().unwrap();
        assert_eq!(loaded.episodes.len(), 1);
        assert_eq!(loaded.n(), sys.n());
    }

    #[test]
    fn test_directory_creation() {
        let dir = std::env::temp_dir().join("am-brain-store-test-dirs");
        let _ = fs::remove_dir_all(&dir);

        let bs = BrainStore::open(Some("test-project"), Some(&dir)).unwrap();
        assert_eq!(bs.project_id(), "test-project");

        assert!(dir.join("brain.db").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_project_name_override() {
        let dir = std::env::temp_dir().join("am-brain-store-test-override");
        let _ = fs::remove_dir_all(&dir);

        let bs = BrainStore::open(Some("my-project"), Some(&dir)).unwrap();
        assert_eq!(bs.project_id(), "my-project");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_project_name_sanitization() {
        assert_eq!(sanitize_name("hello world"), "hello_world");
        assert_eq!(sanitize_name("my/project"), "my_project");
        assert_eq!(sanitize_name("valid-name_123"), "valid-name_123");
    }

    #[test]
    fn test_empty_name_falls_through_to_auto() {
        let id = resolve_project_id(Some(""));
        assert!(
            !id.is_empty(),
            "empty project name should fall through to auto-detection"
        );
        // Should be a readable name, not a hex hash
        assert!(
            id.chars().any(|c| c.is_alphabetic()),
            "auto-detected id should contain letters, got: {id}"
        );
    }

    #[test]
    fn test_resolve_explicit_wins() {
        let id = resolve_project_id(Some("my-explicit-project"));
        assert_eq!(id, "my-explicit-project");
    }

    #[test]
    fn test_resolve_sanitizes_explicit() {
        let id = resolve_project_id(Some("my/project name!"));
        assert_eq!(id, "my_project_name_");
    }

    #[test]
    fn test_resolve_fallback_produces_readable() {
        // Even without git, should produce a readable basename
        let id = resolve_project_id(None);
        assert!(!id.is_empty());
        assert!(
            id.chars().any(|c| c.is_alphabetic()),
            "fallback id should be readable, got: {id}"
        );
    }

    // -- parse_remote_url --

    #[test]
    fn test_parse_remote_ssh_scp() {
        assert_eq!(
            parse_remote_url("git@github.com:srobinson/attention-matters.git"),
            Some("srobinson_attention-matters".to_string())
        );
    }

    #[test]
    fn test_parse_remote_https() {
        assert_eq!(
            parse_remote_url("https://github.com/srobinson/attention-matters.git"),
            Some("srobinson_attention-matters".to_string())
        );
    }

    #[test]
    fn test_parse_remote_ssh_scheme() {
        assert_eq!(
            parse_remote_url("ssh://git@github.com/srobinson/attention-matters.git"),
            Some("srobinson_attention-matters".to_string())
        );
    }

    #[test]
    fn test_parse_remote_no_git_suffix() {
        assert_eq!(
            parse_remote_url("https://github.com/org/repo"),
            Some("org_repo".to_string())
        );
    }

    #[test]
    fn test_parse_remote_gitlab_subgroups() {
        assert_eq!(
            parse_remote_url("git@gitlab.com:group/subgroup/repo.git"),
            Some("subgroup_repo".to_string())
        );
    }

    #[test]
    fn test_parse_remote_garbage() {
        assert_eq!(parse_remote_url("not-a-url"), None);
        assert_eq!(parse_remote_url(""), None);
    }

    // -- extract_toml_name --

    #[test]
    fn test_extract_toml_cargo() {
        let content = r#"
[package]
name = "attention-matters"
version = "0.1.0"
"#;
        assert_eq!(
            extract_toml_name(content, "package"),
            Some("attention-matters".to_string())
        );
    }

    #[test]
    fn test_extract_toml_single_quotes() {
        let content = "[package]\nname = 'my-crate'\n";
        assert_eq!(
            extract_toml_name(content, "package"),
            Some("my-crate".to_string())
        );
    }

    #[test]
    fn test_extract_toml_workspace_no_package() {
        let content = "[workspace]\nmembers = [\"crates/*\"]\n";
        assert_eq!(extract_toml_name(content, "package"), None);
    }

    #[test]
    fn test_extract_toml_pyproject_ignores_wrong_section() {
        let content = r#"
[tool.poetry]
name = "wrong"

[project]
name = "correct"
"#;
        assert_eq!(
            extract_toml_name(content, "project"),
            Some("correct".to_string())
        );
    }

    // -- extract_quoted --

    #[test]
    fn test_extract_quoted_double() {
        assert_eq!(extract_quoted(r#""hello""#), Some("hello"));
    }

    #[test]
    fn test_extract_quoted_single() {
        assert_eq!(extract_quoted("'world'"), Some("world"));
    }

    #[test]
    fn test_extract_quoted_none() {
        assert_eq!(extract_quoted("bare_value"), None);
    }

    #[test]
    fn test_extract_quoted_empty() {
        assert_eq!(extract_quoted(r#""""#), Some(""));
    }
}
