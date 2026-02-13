use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
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

/// Hash a path to a short hex string for use as a project db filename.
fn hash_path(path: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    path.to_string_lossy().hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

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

/// Resolve a project identifier to a database filename.
/// If `project_name` is Some, use it directly as the filename stem.
/// Otherwise, detect git root and hash it.
/// Falls back to hashing cwd if not in a git repo.
fn resolve_project_id(project_name: Option<&str>) -> String {
    if let Some(name) = project_name {
        return sanitize_name(name);
    }

    let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    if let Some(git_root) = detect_git_root(&cwd) {
        hash_path(&git_root)
    } else {
        hash_path(&cwd)
    }
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
    fn test_hash_deterministic() {
        let path = Path::new("/home/user/project");
        assert_eq!(hash_path(path), hash_path(path));
    }

    #[test]
    fn test_hash_different_paths() {
        let a = hash_path(Path::new("/a/b"));
        let b = hash_path(Path::new("/c/d"));
        assert_ne!(a, b);
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
