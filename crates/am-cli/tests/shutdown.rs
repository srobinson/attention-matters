//! Integration tests for graceful shutdown of `am serve`.
//! Verifies that closing stdin (EOF) and sending signals cause clean exit.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn am_binary() -> std::path::PathBuf {
    assert_cmd::cargo::cargo_bin!("am").into()
}

fn spawn_serve(data_dir: &TempDir) -> std::process::Child {
    Command::new(am_binary())
        .args(["serve", "--project", "test-shutdown"])
        .env("AM_DATA_DIR", data_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn am serve")
}

/// Wait for the pidfile to appear, indicating the server process has started.
fn wait_for_pidfile(data_dir: &TempDir) {
    let pidfile = data_dir.path().join("am-serve.pid");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if pidfile.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Send the MCP initialize handshake so the server enters its main loop.
fn mcp_handshake(child: &mut std::process::Child) {
    let stdin = child.stdin.as_mut().expect("stdin pipe");

    let init_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    });
    send_jsonrpc(stdin, &init_req);

    // Wait for server to process initialize
    std::thread::sleep(Duration::from_millis(300));

    // Send initialized notification to complete handshake
    let initialized = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });
    send_jsonrpc(stdin, &initialized);

    // Wait for server to enter main loop
    std::thread::sleep(Duration::from_millis(200));
}

/// Send a JSON-RPC message as newline-delimited JSON (rmcp 0.15 stdio format).
fn send_jsonrpc(stdin: &mut impl Write, msg: &serde_json::Value) {
    let line = serde_json::to_string(msg).unwrap();
    writeln!(stdin, "{line}").unwrap();
    stdin.flush().unwrap();
}

/// Closing stdin before MCP init should still exit cleanly (code 0).
#[test]
fn serve_exits_on_early_stdin_eof() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    wait_for_pidfile(&dir);

    // Close stdin before MCP handshake
    drop(child.stdin.take());

    let start = Instant::now();
    let output = child.wait_with_output().expect("wait");
    let elapsed = start.elapsed();

    assert!(
        output.status.success(),
        "early stdin EOF should exit 0, got {}",
        output.status
    );
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}

/// After full MCP handshake, closing stdin should trigger clean shutdown.
#[test]
fn serve_exits_on_stdin_eof() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    wait_for_pidfile(&dir);
    mcp_handshake(&mut child);

    // Close stdin → triggers EOF
    drop(child.stdin.take());

    let start = Instant::now();
    let output = child.wait_with_output().expect("wait");
    let elapsed = start.elapsed();

    assert!(
        output.status.success(),
        "am serve should exit 0 on stdin EOF, got {}",
        output.status
    );
    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}

#[cfg(unix)]
#[test]
fn serve_exits_on_sigterm() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    wait_for_pidfile(&dir);
    mcp_handshake(&mut child);

    unsafe {
        libc::kill(child.id() as libc::pid_t, libc::SIGTERM);
    }

    let start = Instant::now();
    let _ = child.wait().expect("wait");
    let elapsed = start.elapsed();

    assert!(elapsed < Duration::from_secs(2), "took {elapsed:?}");
}

#[test]
fn pidfile_created_and_removed_on_exit() {
    let dir = TempDir::new().unwrap();
    let pidfile = dir.path().join("am-serve.pid");

    let mut child = spawn_serve(&dir);
    wait_for_pidfile(&dir);

    assert!(pidfile.exists(), "pidfile should exist while server runs");

    let content = std::fs::read_to_string(&pidfile).unwrap();
    let file_pid: u32 = content
        .trim()
        .parse()
        .expect("pidfile should contain a PID");
    assert_eq!(file_pid, child.id(), "pidfile PID should match child PID");

    mcp_handshake(&mut child);

    // Close stdin → clean shutdown
    drop(child.stdin.take());
    child.wait().expect("wait");

    assert!(
        !pidfile.exists(),
        "pidfile should be removed after clean shutdown"
    );
}

#[test]
fn wal_checkpoint_on_exit() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    wait_for_pidfile(&dir);
    mcp_handshake(&mut child);

    // Send am_ingest to create WAL data
    let stdin = child.stdin.as_mut().expect("stdin pipe");
    let ingest_req = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "am_ingest",
            "arguments": {
                "text": "WAL checkpoint test content. Multiple sentences for neighborhoods. And one more sentence for good measure.",
                "name": "wal-test"
            }
        }
    });
    send_jsonrpc(stdin, &ingest_req);

    // Wait for ingestion to process and response to be written
    std::thread::sleep(Duration::from_millis(500));

    // Close stdin → triggers clean shutdown with WAL checkpoint
    drop(child.stdin.take());
    child.wait().expect("wait");

    // After clean exit, WAL files should be empty or non-existent (TRUNCATE checkpoint)
    let projects_dir = dir.path().join("projects");
    if projects_dir.exists() {
        for entry in std::fs::read_dir(&projects_dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "db") {
                let wal_path = path.with_extension("db-wal");
                if wal_path.exists() {
                    let wal_size = std::fs::metadata(&wal_path).unwrap().len();
                    assert_eq!(
                        wal_size, 0,
                        "WAL should be empty after TRUNCATE checkpoint, was {wal_size} bytes"
                    );
                }
            }
        }
    }
}
