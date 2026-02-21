//! CLI command integration tests.
//! Each test uses a temp directory via AM_DATA_DIR for full isolation.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

fn am_cmd(data_dir: &TempDir) -> Command {
    #[allow(deprecated)]
    let mut cmd = Command::cargo_bin("am").unwrap();
    cmd.env("AM_DATA_DIR", data_dir.path());
    cmd
}

#[test]
fn stats_fresh_db() {
    let dir = TempDir::new().unwrap();
    am_cmd(&dir)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("N:          0"))
        .stdout(predicate::str::contains("episodes:   0"))
        .stdout(predicate::str::contains("conscious:  0"));
}

#[test]
fn ingest_file_then_stats() {
    let dir = TempDir::new().unwrap();

    // Create a temp file to ingest
    let input = dir.path().join("doc.txt");
    std::fs::write(
        &input,
        "The quick brown fox jumps over the lazy dog. \
         Sentence two provides more content. \
         A third sentence completes the paragraph.",
    )
    .unwrap();

    // Ingest
    am_cmd(&dir)
        .args(["ingest"])
        .arg(&input)
        .assert()
        .success()
        .stdout(predicate::str::contains("ingested"))
        .stdout(predicate::str::contains("done. N="));

    // Stats should show data
    let output = am_cmd(&dir).args(["stats"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let n = extract_stat_value(&stdout, "N:");
    assert_ne!(n, "0", "N should be non-zero after ingest");
    assert_eq!(extract_stat_value(&stdout, "episodes:"), "1");
}

#[test]
fn query_after_ingest() {
    let dir = TempDir::new().unwrap();

    let input = dir.path().join("science.txt");
    std::fs::write(
        &input,
        "Quantum mechanics describes particle behavior at subatomic scales. \
         Wave functions collapse upon measurement producing outcomes. \
         The uncertainty principle limits knowledge of position and momentum.",
    )
    .unwrap();

    // Ingest
    am_cmd(&dir).args(["ingest"]).arg(&input).assert().success();

    // Query should succeed and produce output (not crash or hang)
    am_cmd(&dir)
        .args(["query", "quantum particles"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty().not());
}

#[test]
fn export_import_roundtrip() {
    let dir = TempDir::new().unwrap();

    let input = dir.path().join("data.txt");
    std::fs::write(
        &input,
        "Neural networks learn representations from data. \
         Deep learning enables complex pattern recognition. \
         Backpropagation computes gradients for weight updates.",
    )
    .unwrap();

    // Ingest
    am_cmd(&dir).args(["ingest"]).arg(&input).assert().success();

    // Get stats before export
    let stats_before = am_cmd(&dir).args(["stats"]).output().unwrap();
    let stats_before_str = String::from_utf8_lossy(&stats_before.stdout);

    // Export
    let export_path = dir.path().join("export.json");
    am_cmd(&dir)
        .args(["export"])
        .arg(&export_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("exported to"));

    assert!(export_path.exists(), "export file should exist");

    // Import (overwrites the same brain.db)
    am_cmd(&dir)
        .args(["import"])
        .arg(&export_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("imported from"));

    // Stats after import should match
    let stats_after = am_cmd(&dir).args(["stats"]).output().unwrap();
    let stats_after_str = String::from_utf8_lossy(&stats_after.stdout);

    // Extract N values
    let n_a = extract_stat_value(&stats_before_str, "N:");
    let n_b = extract_stat_value(&stats_after_str, "N:");
    assert_eq!(n_a, n_b, "N should match after import");

    let ep_a = extract_stat_value(&stats_before_str, "episodes:");
    let ep_b = extract_stat_value(&stats_after_str, "episodes:");
    assert_eq!(ep_a, ep_b, "episode count should match after import");
}

fn extract_stat_value(output: &str, prefix: &str) -> String {
    output
        .lines()
        .find(|l| l.contains(prefix))
        .unwrap_or_else(|| panic!("stat line containing '{prefix}' not found in output:\n{output}"))
        .split_whitespace()
        .last()
        .unwrap()
        .to_string()
}

#[test]
fn ingest_dir() {
    let dir = TempDir::new().unwrap();

    let docs_dir = dir.path().join("docs");
    std::fs::create_dir(&docs_dir).unwrap();

    std::fs::write(
        docs_dir.join("first.md"),
        "First document about alpha and beta. Second sentence here. Third sentence final.",
    )
    .unwrap();
    std::fs::write(
        docs_dir.join("second.md"),
        "Second document about gamma and delta. Another sentence follows. Done with this one.",
    )
    .unwrap();
    // Non-matching extension should be skipped
    std::fs::write(docs_dir.join("ignore.json"), "{}").unwrap();

    // Use a dummy positional arg (first.md) since `files` is required,
    // then --dir scans for additional .md/.txt files (second.md).
    // first.md appears both as positional and from dir scan → 3 episodes.
    am_cmd(&dir)
        .args(["ingest", "--dir"])
        .arg(&docs_dir)
        .arg(docs_dir.join("first.md"))
        .assert()
        .success()
        .stdout(predicate::str::contains("ingested"));

    let output = am_cmd(&dir).args(["stats"]).output().unwrap();
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let episodes: usize = extract_stat_value(&stdout, "episodes:")
        .parse()
        .unwrap_or(0);
    // 3 episodes: first.md (positional) + first.md (dir scan) + second.md (dir scan)
    // .json file is correctly skipped
    assert_eq!(
        episodes, 3,
        "expected 3 episodes (first.md twice + second.md), got {episodes}"
    );
}

#[test]
fn missing_required_args() {
    let dir = TempDir::new().unwrap();

    // query without text
    am_cmd(&dir)
        .args(["query"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));

    // ingest without files
    am_cmd(&dir)
        .args(["ingest"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));

    // export without path
    am_cmd(&dir)
        .args(["export"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));

    // import without path
    am_cmd(&dir)
        .args(["import"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn inspect_overview_fresh_db() {
    let dir = TempDir::new().unwrap();
    am_cmd(&dir)
        .args(["inspect"])
        .assert()
        .success()
        .stdout(predicate::str::contains("MEMORY OVERVIEW"))
        .stdout(predicate::str::contains("occurrences:"))
        .stdout(predicate::str::contains("episodes:"))
        .stdout(predicate::str::contains("conscious:"));
}

#[test]
fn inspect_after_ingest() {
    let dir = TempDir::new().unwrap();

    let input = dir.path().join("inspect.txt");
    std::fs::write(
        &input,
        "Geometric memory models encode information on manifolds. \
         Quaternion representations enable smooth interpolation. \
         Phase coupling synchronizes distributed memory traces.",
    )
    .unwrap();

    am_cmd(&dir).args(["ingest"]).arg(&input).assert().success();

    // Overview should show data
    am_cmd(&dir)
        .args(["inspect"])
        .assert()
        .success()
        .stdout(predicate::str::contains("RECENT EPISODES"))
        .stdout(predicate::str::contains("TOP WORDS"));

    // Episodes view
    am_cmd(&dir)
        .args(["inspect", "episodes"])
        .assert()
        .success()
        .stdout(predicate::str::contains("EPISODES"))
        .stdout(predicate::str::contains("neighborhoods"));

    // Neighborhoods view
    am_cmd(&dir)
        .args(["inspect", "neighborhoods"])
        .assert()
        .success()
        .stdout(predicate::str::contains("NEIGHBORHOODS"));

    // Conscious view (empty after just ingest)
    am_cmd(&dir)
        .args(["inspect", "conscious"])
        .assert()
        .success()
        .stdout(predicate::str::contains("CONSCIOUS MEMORIES"));

    // JSON output
    am_cmd(&dir)
        .args(["inspect", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"total_occurrences\""))
        .stdout(predicate::str::contains("\"episodes\""));

    // Query mode
    am_cmd(&dir)
        .args(["inspect", "--query", "geometric memory"])
        .assert()
        .success()
        .stdout(predicate::str::contains("RECALL"));
}

#[test]
fn inspect_json_outputs() {
    let dir = TempDir::new().unwrap();

    let input = dir.path().join("jsontest.txt");
    std::fs::write(
        &input,
        "Testing JSON output format for inspect command. \
         Multiple sentences help create neighborhoods. \
         This third sentence fills the minimum requirement.",
    )
    .unwrap();

    am_cmd(&dir).args(["ingest"]).arg(&input).assert().success();

    // Episodes JSON
    am_cmd(&dir)
        .args(["inspect", "episodes", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"name\""));

    // Neighborhoods JSON
    am_cmd(&dir)
        .args(["inspect", "neighborhoods", "--json"])
        .assert()
        .success()
        .stdout(predicate::str::contains("\"source_text\""));
}

#[test]
fn sync_dry_run() {
    let dir = TempDir::new().unwrap();

    // Create a fake Claude project directory structure.
    // sync --dir points at the Claude config dir; inside it needs projects/<encoded-cwd>/
    let claude_dir = dir.path().join("fake-claude");
    // Encode the CWD the same way sync.rs does: replace / with -
    let cwd = std::env::current_dir().unwrap();
    let encoded = cwd.to_string_lossy().replace('/', "-");
    let project_dir = claude_dir.join("projects").join(&encoded);
    std::fs::create_dir_all(&project_dir).unwrap();

    // Write a session transcript with substantive content
    let session_path = project_dir.join("abc-12345678.jsonl");
    let mut f = std::fs::File::create(&session_path).unwrap();
    use std::io::Write;
    writeln!(f, "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"How does the geometric memory system handle quaternion drift on the S3 manifold?\"}}}}").unwrap();
    writeln!(f, "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"The drift mechanism uses IDF-weighted SLERP to move occurrences closer to query centroids.\"}}]}}}}").unwrap();

    // Dry run should find the session but not ingest
    am_cmd(&dir)
        .args(["sync", "--dry-run", "--dir"])
        .arg(&claude_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Found 1"))
        .stdout(predicate::str::contains("sync"))
        .stdout(predicate::str::contains("Dry run"));

    // Stats should still be empty after dry run
    am_cmd(&dir)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("episodes:   0"));
}

#[test]
fn sync_ingests_sessions() {
    let dir = TempDir::new().unwrap();

    let claude_dir = dir.path().join("fake-claude2");
    let cwd = std::env::current_dir().unwrap();
    let encoded = cwd.to_string_lossy().replace('/', "-");
    let project_dir = claude_dir.join("projects").join(&encoded);
    std::fs::create_dir_all(&project_dir).unwrap();

    // Two sessions
    use std::io::Write;
    let mut f1 = std::fs::File::create(project_dir.join("sess-aaaaaaaa.jsonl")).unwrap();
    writeln!(f1, "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"Explain the Kuramoto phase coupling model for memory synchronization.\"}}}}").unwrap();
    writeln!(f1, "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"Kuramoto coupling synchronizes phasor phases across neighborhoods that co-activate frequently.\"}}]}}}}").unwrap();

    let mut f2 = std::fs::File::create(project_dir.join("sess-bbbbbbbb.jsonl")).unwrap();
    writeln!(f2, "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"What is the golden angle spacing for phasor distribution on the manifold?\"}}}}").unwrap();
    writeln!(f2, "{{\"type\":\"assistant\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"Golden angle is approximately 2.399 radians, derived from the golden ratio phi to maximize separation.\"}}]}}}}").unwrap();

    // Real sync
    am_cmd(&dir)
        .args(["sync", "--dir"])
        .arg(&claude_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("Found 2"))
        .stdout(predicate::str::contains("synced"))
        .stdout(predicate::str::contains("Done."));

    // Stats should show 2 episodes
    am_cmd(&dir)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("episodes:   2"));

    // Re-sync should say all synced
    am_cmd(&dir)
        .args(["sync", "--dir"])
        .arg(&claude_dir)
        .assert()
        .success()
        .stdout(predicate::str::contains("already synced"));
}

#[test]
fn sync_no_project_dir() {
    let dir = TempDir::new().unwrap();

    // Point at a dir with no projects/ subdirectory
    let empty_claude = dir.path().join("empty-claude");
    std::fs::create_dir_all(&empty_claude).unwrap();

    am_cmd(&dir)
        .args(["sync", "--dir"])
        .arg(&empty_claude)
        .assert()
        .success()
        .stdout(predicate::str::contains("No Claude Code project directory"));
}

#[test]
fn unified_brain() {
    // All data goes to the same brain.db regardless of where you run from
    let dir = TempDir::new().unwrap();

    let input = dir.path().join("unified.txt");
    std::fs::write(
        &input,
        "Unique content for unified brain testing. More sentences needed. And a third one.",
    )
    .unwrap();

    // Ingest
    am_cmd(&dir).args(["ingest"]).arg(&input).assert().success();

    // Stats shows the data
    am_cmd(&dir)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("episodes:   1"));
}

#[test]
fn gc_fresh_db() {
    let dir = TempDir::new().unwrap();
    am_cmd(&dir)
        .args(["gc"])
        .assert()
        .success()
        .stdout(predicate::str::contains("GC complete"))
        .stdout(predicate::str::contains("evicted occurrences:"));
}

#[test]
fn gc_dry_run() {
    let dir = TempDir::new().unwrap();

    let input = dir.path().join("gc-data.txt");
    std::fs::write(
        &input,
        "Memory garbage collection testing with enough words. \
         Second sentence provides additional content. \
         Third sentence completes the paragraph for chunking.",
    )
    .unwrap();

    am_cmd(&dir).args(["ingest"]).arg(&input).assert().success();

    am_cmd(&dir)
        .args(["gc", "--dry-run"])
        .assert()
        .success()
        .stdout(predicate::str::contains("GC dry run"))
        .stdout(predicate::str::contains("eligible for eviction"))
        .stdout(predicate::str::contains("No changes made"));
}

#[test]
fn gc_evicts_cold_occurrences() {
    let dir = TempDir::new().unwrap();

    let input = dir.path().join("gc-cold.txt");
    std::fs::write(
        &input,
        "Quantum entanglement connects particles across spacetime. \
         Bell inequality violations confirm nonlocal correlations. \
         Decoherence destroys quantum superposition in macroscopic systems.",
    )
    .unwrap();

    am_cmd(&dir).args(["ingest"]).arg(&input).assert().success();

    // With floor=99, everything should be evicted (no occurrence has count > 99)
    am_cmd(&dir)
        .args(["gc", "--floor", "99"])
        .assert()
        .success()
        .stdout(predicate::str::contains("GC complete"));

    // Stats should show 0 episodes after evicting everything
    am_cmd(&dir)
        .args(["stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("episodes:   0"));
}

#[test]
fn forget_term() {
    let dir = TempDir::new().unwrap();

    let input = dir.path().join("forget.txt");
    std::fs::write(
        &input,
        "Authentication uses JWT tokens for session management. \
         Password hashing employs bcrypt with salt rounds. \
         Token refresh handles expiration transparently.",
    )
    .unwrap();

    am_cmd(&dir).args(["ingest"]).arg(&input).assert().success();

    am_cmd(&dir)
        .args(["forget", "password"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Forgot"))
        .stdout(predicate::str::contains("password"));
}

#[test]
fn forget_term_not_found() {
    let dir = TempDir::new().unwrap();
    am_cmd(&dir)
        .args(["forget", "nonexistent"])
        .assert()
        .success()
        .stdout(predicate::str::contains("No occurrences"));
}

#[test]
fn forget_requires_argument() {
    let dir = TempDir::new().unwrap();
    // No term, no --episode, no --conscious
    am_cmd(&dir).args(["forget"]).assert().failure();
}
