use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

use am_core::DAESystem;

use crate::error::{Result, StoreError};
use crate::store::Store;

/// Default base directory for all am storage.
fn default_base_dir() -> PathBuf {
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
            && let Some(end) = s[start + 1..].find(quote) {
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
                        && !name.is_empty() {
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
        && let Some(name) = extract_toml_name(&content, "package") {
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
                        && !name.is_empty() {
                            return Some(name.to_string());
                        }
                }
            }
        }
    }

    // pyproject.toml → [project] name
    let pyproject = dir.join("pyproject.toml");
    if let Ok(content) = fs::read_to_string(&pyproject)
        && let Some(name) = extract_toml_name(&content, "project") {
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

/// Manages per-project storage with a global conscious layer.
///
/// Layout:
/// ```text
/// ~/.attention-matters/
/// ├── global.db
/// └── projects/
///     ├── <project-hash>.db
///     └── ...
/// ```
pub struct ProjectStore {
    project: Store,
    global: Store,
    project_id: String,
}

impl ProjectStore {
    /// Open project and global stores, creating directories as needed.
    /// `project_name`: explicit project name (overrides auto-detection).
    /// `base_dir`: override the base directory (for testing).
    pub fn open(project_name: Option<&str>, base_dir: Option<&Path>) -> Result<Self> {
        let base = base_dir.map(PathBuf::from).unwrap_or_else(default_base_dir);
        let projects_dir = base.join("projects");

        fs::create_dir_all(&projects_dir).map_err(|e| {
            StoreError::InvalidData(format!("failed to create {}: {e}", projects_dir.display()))
        })?;

        let project_id = resolve_project_id(project_name);
        let project_path = projects_dir.join(format!("{project_id}.db"));
        let global_path = base.join("global.db");

        let project = Store::open(&project_path)?;
        let global = Store::open(&global_path)?;

        Ok(Self {
            project,
            global,
            project_id,
        })
    }

    /// Open with in-memory stores (for testing).
    pub fn open_in_memory() -> Result<Self> {
        Ok(Self {
            project: Store::open_in_memory()?,
            global: Store::open_in_memory()?,
            project_id: "test".to_string(),
        })
    }

    pub fn project_id(&self) -> &str {
        &self.project_id
    }

    pub fn project_store(&self) -> &Store {
        &self.project
    }

    pub fn global_store(&self) -> &Store {
        &self.global
    }

    /// Load the project's full DAESystem.
    pub fn load_project_system(&self) -> Result<DAESystem> {
        self.project.load_system()
    }

    /// Save a full DAESystem to the project store.
    pub fn save_project_system(&self, system: &DAESystem) -> Result<()> {
        self.project.save_system(system)
    }

    /// Mark text as salient (conscious). Writes to BOTH project and global stores.
    /// Returns the neighborhood ID.
    pub fn mark_salient(
        &self,
        system: &mut DAESystem,
        text: &str,
        rng: &mut impl rand::Rng,
    ) -> Result<uuid::Uuid> {
        let nbhd_id = system.add_to_conscious(text, rng);

        // Save full project state (includes the new conscious neighborhood)
        self.project.save_system(system)?;

        // Replicate conscious neighborhood to global store
        self.replicate_conscious_to_global(system)?;

        Ok(nbhd_id)
    }

    /// Write the conscious episode from the project system to the global store.
    /// Creates or updates the conscious episode in global.
    fn replicate_conscious_to_global(&self, system: &DAESystem) -> Result<()> {
        // Load current global state
        let mut global_system = self.global.load_system()?;

        // Copy conscious neighborhoods from project that aren't already in global
        let existing_ids: std::collections::HashSet<uuid::Uuid> = global_system
            .conscious_episode
            .neighborhoods
            .iter()
            .map(|n| n.id)
            .collect();

        for nbhd in &system.conscious_episode.neighborhoods {
            if !existing_ids.contains(&nbhd.id) {
                global_system
                    .conscious_episode
                    .add_neighborhood(nbhd.clone());
            }
        }

        global_system.agent_name = system.agent_name.clone();
        self.global.save_system(&global_system)
    }

    /// Import a v0.7.2 JSON file into the project store.
    pub fn import_json_file(&self, path: &Path) -> Result<()> {
        self.project.import_json_file(path)?;

        // Also replicate conscious memories to global
        let system = self.project.load_system()?;
        self.replicate_conscious_to_global(&system)
    }

    /// Export the project store to a v0.7.2 JSON file.
    pub fn export_json_file(&self, path: &Path) -> Result<()> {
        self.project.export_json_file(path)
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
    fn test_project_isolation() {
        let ps_a = ProjectStore::open_in_memory().unwrap();
        let ps_b = ProjectStore::open_in_memory().unwrap();

        let sys_a = make_system();
        ps_a.save_project_system(&sys_a).unwrap();

        // Project B should have empty state
        let sys_b = ps_b.load_project_system().unwrap();
        assert!(sys_b.episodes.is_empty());
        assert_eq!(sys_b.n(), 0);

        // Project A should have data
        let loaded_a = ps_a.load_project_system().unwrap();
        assert_eq!(loaded_a.episodes.len(), 1);
    }

    #[test]
    fn test_salient_writes_to_both() {
        let ps = ProjectStore::open_in_memory().unwrap();
        let mut sys = make_system();
        let mut rng = rng();

        ps.mark_salient(&mut sys, "important insight", &mut rng)
            .unwrap();

        // Project should have conscious neighborhoods
        let project_sys = ps.load_project_system().unwrap();
        assert_eq!(project_sys.conscious_episode.neighborhoods.len(), 1);

        // Global should also have conscious neighborhoods
        let global_sys = ps.global_store().load_system().unwrap();
        assert_eq!(global_sys.conscious_episode.neighborhoods.len(), 1);
    }

    #[test]
    fn test_salient_deduplication() {
        let ps = ProjectStore::open_in_memory().unwrap();
        let mut sys = make_system();
        let mut rng = rng();

        ps.mark_salient(&mut sys, "first insight", &mut rng)
            .unwrap();
        ps.mark_salient(&mut sys, "second insight", &mut rng)
            .unwrap();

        let global_sys = ps.global_store().load_system().unwrap();
        assert_eq!(global_sys.conscious_episode.neighborhoods.len(), 2);
    }

    #[test]
    fn test_subconscious_not_in_global() {
        let ps = ProjectStore::open_in_memory().unwrap();
        let sys = make_system();
        ps.save_project_system(&sys).unwrap();

        let global_sys = ps.global_store().load_system().unwrap();
        assert!(
            global_sys.episodes.is_empty(),
            "subconscious should not leak to global"
        );
    }

    #[test]
    fn test_directory_creation() {
        let dir = std::env::temp_dir().join("am-store-test-dirs");
        let _ = fs::remove_dir_all(&dir);

        let ps = ProjectStore::open(Some("test-project"), Some(&dir)).unwrap();
        assert_eq!(ps.project_id(), "test-project");

        assert!(dir.join("global.db").exists());
        assert!(dir.join("projects").exists());
        assert!(dir.join("projects/test-project.db").exists());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_project_name_override() {
        let dir = std::env::temp_dir().join("am-store-test-override");
        let _ = fs::remove_dir_all(&dir);

        let ps = ProjectStore::open(Some("my-project"), Some(&dir)).unwrap();
        assert_eq!(ps.project_id(), "my-project");

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

    #[test]
    fn test_import_replicates_conscious() {
        let ps = ProjectStore::open_in_memory().unwrap();
        let mut rng = rng();

        // Build a system with conscious data
        let mut sys = make_system();
        sys.add_to_conscious("conscious memory", &mut rng);
        let json = am_core::export_json(&sys).unwrap();

        // Import via ProjectStore
        ps.project_store().import_json_str(&json).unwrap();
        let loaded = ps.project_store().load_system().unwrap();

        // Manually replicate (import_json_file would do this)
        ps.replicate_conscious_to_global(&loaded).unwrap();

        let global = ps.global_store().load_system().unwrap();
        assert!(
            !global.conscious_episode.neighborhoods.is_empty(),
            "conscious should be replicated to global"
        );
    }
}
