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
        .args(["stats", "--project", "test-stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("N:         0"))
        .stdout(predicate::str::contains("episodes:  0"))
        .stdout(predicate::str::contains("conscious: 0"));
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
        .args(["ingest", "--project", "test-ingest"])
        .arg(&input)
        .assert()
        .success()
        .stdout(predicate::str::contains("ingested"))
        .stdout(predicate::str::contains("done. N="));

    // Stats should show data
    let output = am_cmd(&dir)
        .args(["stats", "--project", "test-ingest"])
        .output()
        .unwrap();
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
    am_cmd(&dir)
        .args(["ingest", "--project", "test-query"])
        .arg(&input)
        .assert()
        .success();

    // Query should succeed and produce output (not crash or hang)
    am_cmd(&dir)
        .args(["query", "--project", "test-query", "quantum particles"])
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

    // Ingest into project A
    am_cmd(&dir)
        .args(["ingest", "--project", "proj-export"])
        .arg(&input)
        .assert()
        .success();

    // Get stats from project A
    let stats_a = am_cmd(&dir)
        .args(["stats", "--project", "proj-export"])
        .output()
        .unwrap();
    let stats_a_str = String::from_utf8_lossy(&stats_a.stdout);

    // Export from project A
    let export_path = dir.path().join("export.json");
    am_cmd(&dir)
        .args(["export", "--project", "proj-export"])
        .arg(&export_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("exported to"));

    assert!(export_path.exists(), "export file should exist");

    // Import into project B
    am_cmd(&dir)
        .args(["import", "--project", "proj-import"])
        .arg(&export_path)
        .assert()
        .success()
        .stdout(predicate::str::contains("imported from"));

    // Stats from project B should match
    let stats_b = am_cmd(&dir)
        .args(["stats", "--project", "proj-import"])
        .output()
        .unwrap();
    let stats_b_str = String::from_utf8_lossy(&stats_b.stdout);

    // Extract N values
    let n_a = extract_stat_value(&stats_a_str, "N:");
    let n_b = extract_stat_value(&stats_b_str, "N:");
    assert_eq!(n_a, n_b, "N should match after import");

    let ep_a = extract_stat_value(&stats_a_str, "episodes:");
    let ep_b = extract_stat_value(&stats_b_str, "episodes:");
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
        .args(["ingest", "--project", "test-dir", "--dir"])
        .arg(&docs_dir)
        .arg(docs_dir.join("first.md"))
        .assert()
        .success()
        .stdout(predicate::str::contains("ingested"));

    let output = am_cmd(&dir)
        .args(["stats", "--project", "test-dir"])
        .output()
        .unwrap();
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
fn project_isolation() {
    let dir = TempDir::new().unwrap();

    let input = dir.path().join("isolated.txt");
    std::fs::write(
        &input,
        "Unique content for project isolation testing. More sentences needed. And a third one.",
    )
    .unwrap();

    // Ingest into project A
    am_cmd(&dir)
        .args(["ingest", "--project", "isolated-a"])
        .arg(&input)
        .assert()
        .success();

    // Project A has data
    am_cmd(&dir)
        .args(["stats", "--project", "isolated-a"])
        .assert()
        .success()
        .stdout(predicate::str::contains("episodes:  1"));

    // Project B should be empty
    am_cmd(&dir)
        .args(["stats", "--project", "isolated-b"])
        .assert()
        .success()
        .stdout(predicate::str::contains("episodes:  0"));
}
