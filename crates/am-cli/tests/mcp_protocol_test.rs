//! MCP protocol integration tests.
//!
//! Spawns `am serve` as a subprocess, sends JSON-RPC messages over stdin,
//! and asserts on stdout responses. Validates the full MCP wire protocol
//! from initialize through tool calls.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tempfile::TempDir;

fn am_binary() -> std::path::PathBuf {
    assert_cmd::cargo::cargo_bin!("am").into()
}

/// Spawn `am serve` with piped stdin/stdout and a fresh data directory.
fn spawn_serve(data_dir: &TempDir) -> std::process::Child {
    Command::new(am_binary())
        .args(["serve"])
        .env("AM_DATA_DIR", data_dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn am serve")
}

/// Send a JSON-RPC message as a single newline-terminated line.
fn send(stdin: &mut impl Write, msg: &serde_json::Value) {
    let line = serde_json::to_string(msg).unwrap();
    writeln!(stdin, "{line}").unwrap();
    stdin.flush().unwrap();
}

/// Default timeout for reading a response from the subprocess.
const RECV_TIMEOUT: Duration = Duration::from_secs(10);

/// Read one JSON-RPC response line from stdout with a timeout.
///
/// Spawns a background thread for the blocking `read_line` and uses
/// `mpsc::recv_timeout` to avoid hanging forever if the server stalls.
/// Moves the reader into the thread and returns it alongside the parsed value.
fn recv(
    reader: BufReader<std::process::ChildStdout>,
) -> (serde_json::Value, BufReader<std::process::ChildStdout>) {
    recv_timeout(reader, RECV_TIMEOUT)
}

fn recv_timeout(
    reader: BufReader<std::process::ChildStdout>,
    timeout: Duration,
) -> (serde_json::Value, BufReader<std::process::ChildStdout>) {
    let (tx, rx) = mpsc::channel();

    std::thread::spawn(move || {
        let mut reader = reader;
        let mut line = String::new();
        let result = reader.read_line(&mut line);
        let _ = tx.send((result, line, reader));
    });

    match rx.recv_timeout(timeout) {
        Ok((result, line, reader)) => {
            result.expect("read stdout");
            let val = serde_json::from_str(line.trim()).expect("parse JSON-RPC response");
            (val, reader)
        }
        Err(_) => panic!("recv timed out after {timeout:?} - server may be hung"),
    }
}

/// Send initialize + initialized, return the initialize response and the reader.
fn handshake(
    stdin: &mut impl Write,
    reader: BufReader<std::process::ChildStdout>,
) -> (serde_json::Value, BufReader<std::process::ChildStdout>) {
    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "test", "version": "0.1.0" }
            }
        }),
    );
    let (init_resp, reader) = recv(reader);

    // Send initialized notification (no response expected)
    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    );

    (init_resp, reader)
}

/// Helper: call a tool and return the full JSON-RPC response + reader.
fn call_tool(
    stdin: &mut impl Write,
    reader: BufReader<std::process::ChildStdout>,
    id: u64,
    name: &str,
    arguments: serde_json::Value,
) -> (serde_json::Value, BufReader<std::process::ChildStdout>) {
    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }),
    );
    let (resp, reader) = recv(reader);
    assert_eq!(resp["id"], id);
    (resp, reader)
}

/// Extract the first text content from a tools/call response, parsed as JSON.
fn extract_tool_json(resp: &serde_json::Value) -> serde_json::Value {
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .expect("tool response should have text content");
    serde_json::from_str(text).expect("tool content should be valid JSON")
}

/// Spawn a server, handshake, ingest test data, and return everything needed for tool tests.
fn setup_with_data() -> (
    std::process::Child,
    BufReader<std::process::ChildStdout>,
    TempDir,
) {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (_resp, reader) = handshake(stdin, reader);

    // Ingest a document so tools have data to work with
    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 100,
            "method": "tools/call",
            "params": {
                "name": "am_ingest",
                "arguments": {
                    "text": "Rust ownership rules prevent data races at compile time. The borrow checker enforces these rules automatically. Memory safety without garbage collection is a key advantage.",
                    "name": "rust-safety"
                }
            }
        }),
    );
    let (_resp, reader) = recv(reader);

    (child, reader, dir)
}

#[test]
fn initialize_response_has_required_fields() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (resp, _reader) = handshake(stdin, reader);

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);

    let result = &resp["result"];
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert_eq!(result["serverInfo"]["name"], "am");
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["instructions"].is_string());

    drop(child.stdin.take());
    let status = child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn tools_list_returns_all_12_tools() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (_resp, reader) = handshake(stdin, reader);

    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }),
    );
    let (resp, _reader) = recv(reader);

    assert_eq!(resp["id"], 2);
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 12, "should have exactly 12 tools");

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();

    let expected = [
        "am_query",
        "am_query_index",
        "am_retrieve",
        "am_activate_response",
        "am_salient",
        "am_buffer",
        "am_ingest",
        "am_stats",
        "am_export",
        "am_import",
        "am_feedback",
        "am_batch_query",
    ];
    for name in &expected {
        assert!(names.contains(name), "missing tool: {name}");
    }

    for tool in tools {
        let name = tool["name"].as_str().unwrap();
        assert!(
            tool["description"].is_string(),
            "{name} should have description"
        );
        assert!(
            tool["inputSchema"].is_object(),
            "{name} should have inputSchema"
        );
        assert_eq!(
            tool["inputSchema"]["type"], "object",
            "{name} inputSchema should be type:object"
        );
    }

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_stats_on_empty_db() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (_resp, reader) = handshake(stdin, reader);

    let (resp, _reader) = call_tool(stdin, reader, 3, "am_stats", serde_json::json!({}));
    let stats = extract_tool_json(&resp);

    assert_eq!(stats["n"], 0);
    assert_eq!(stats["episodes"], 0);
    assert_eq!(stats["conscious"], 0);

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_ingest_then_query() {
    let (mut child, reader, _dir) = setup_with_data();
    let stdin = child.stdin.as_mut().unwrap();

    // Query the ingested content
    let (resp, _reader) = call_tool(
        stdin,
        reader,
        5,
        "am_query",
        serde_json::json!({ "text": "rust borrow checker ownership" }),
    );
    let query_json = extract_tool_json(&resp);

    assert!(query_json.get("context").is_some());
    assert!(query_json.get("metrics").is_some());
    assert!(query_json.get("stats").is_some());
    assert!(
        query_json["stats"]["n"].as_u64().unwrap() > 0,
        "should have occurrences after ingest"
    );

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_unknown_tool_returns_error() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (_resp, reader) = handshake(stdin, reader);

    let (resp, _reader) = call_tool(stdin, reader, 6, "nonexistent_tool", serde_json::json!({}));
    assert_eq!(resp["result"]["isError"], true);

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn ping_returns_empty_object() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (_resp, reader) = handshake(stdin, reader);

    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "ping"
        }),
    );
    let (resp, _reader) = recv(reader);

    assert_eq!(resp["id"], 7);
    assert!(resp["result"].is_object());
    assert!(resp["error"].is_null());

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn unknown_method_returns_error() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (_resp, reader) = handshake(stdin, reader);

    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "resources/list"
        }),
    );
    let (resp, _reader) = recv(reader);

    assert_eq!(resp["id"], 8);
    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32601);

    drop(child.stdin.take());
    child.wait().unwrap();
}

// --- Additional tool coverage tests ---

#[test]
fn tools_call_am_export_returns_json() {
    let (mut child, reader, _dir) = setup_with_data();
    let stdin = child.stdin.as_mut().unwrap();

    let (resp, _reader) = call_tool(stdin, reader, 10, "am_export", serde_json::json!({}));

    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let export: serde_json::Value =
        serde_json::from_str(text).expect("export should be valid JSON");
    // Exported wire format has version, timestamp, system
    assert!(export.get("version").is_some());
    assert!(export.get("system").is_some());

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_import_replaces_state() {
    let (mut child, reader, _dir) = setup_with_data();
    let stdin = child.stdin.as_mut().unwrap();

    // Export current state
    let (export_resp, reader) = call_tool(stdin, reader, 11, "am_export", serde_json::json!({}));
    let export_text = export_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let state: serde_json::Value = serde_json::from_str(export_text).unwrap();

    // Import the same state back (replaces current)
    let (import_resp, reader) = call_tool(
        stdin,
        reader,
        12,
        "am_import",
        serde_json::json!({ "state": state }),
    );
    // Should succeed (no isError)
    assert!(
        import_resp["result"]["isError"].is_null() || import_resp["result"]["isError"] == false
    );

    // Verify state is intact via stats
    let (stats_resp, _reader) = call_tool(stdin, reader, 13, "am_stats", serde_json::json!({}));
    let stats = extract_tool_json(&stats_resp);
    assert!(
        stats["n"].as_u64().unwrap() > 0,
        "state should survive import roundtrip"
    );

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_buffer() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (_resp, reader) = handshake(stdin, reader);

    let (resp, _reader) = call_tool(
        stdin,
        reader,
        20,
        "am_buffer",
        serde_json::json!({
            "user": "What is Rust ownership?",
            "assistant": "Ownership is a set of rules governing memory management."
        }),
    );
    let buffer_json = extract_tool_json(&resp);
    assert!(
        buffer_json.get("buffer_size").is_some(),
        "buffer response should have buffer_size"
    );

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_activate_response() {
    let (mut child, reader, _dir) = setup_with_data();
    let stdin = child.stdin.as_mut().unwrap();

    let (resp, _reader) = call_tool(
        stdin,
        reader,
        30,
        "am_activate_response",
        serde_json::json!({ "text": "Rust's ownership model prevents data races" }),
    );
    // Should succeed without error
    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "activate_response should succeed"
    );
    assert!(
        resp["result"]["content"].is_array(),
        "should return content array"
    );

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_query_index_then_retrieve() {
    let (mut child, reader, _dir) = setup_with_data();
    let stdin = child.stdin.as_mut().unwrap();

    // Phase 1: query_index
    let (idx_resp, reader) = call_tool(
        stdin,
        reader,
        40,
        "am_query_index",
        serde_json::json!({ "text": "rust ownership" }),
    );
    let idx_json = extract_tool_json(&idx_resp);
    assert!(
        idx_json.get("entries").is_some(),
        "query_index should return entries"
    );

    let entries = idx_json["entries"].as_array().unwrap();
    if !entries.is_empty() {
        // Phase 2: retrieve with IDs from the index
        let ids: Vec<&str> = entries.iter().filter_map(|e| e["id"].as_str()).collect();
        assert!(!ids.is_empty(), "entries should have IDs");

        let (ret_resp, _reader) = call_tool(
            stdin,
            reader,
            41,
            "am_retrieve",
            serde_json::json!({ "ids": ids }),
        );
        let ret_json = extract_tool_json(&ret_resp);
        assert!(
            ret_json.get("entries").is_some(),
            "retrieve should return entries"
        );
        assert!(
            ret_json.get("count").is_some(),
            "retrieve should return count"
        );
    }

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_feedback() {
    let (mut child, reader, _dir) = setup_with_data();
    let stdin = child.stdin.as_mut().unwrap();

    // Query first to get neighborhood IDs for feedback
    let (query_resp, reader) = call_tool(
        stdin,
        reader,
        50,
        "am_query",
        serde_json::json!({ "text": "rust ownership" }),
    );
    let query_json = extract_tool_json(&query_resp);

    // Extract recalled neighborhood IDs
    let recalled_ids = query_json
        .get("recalled_ids")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if !recalled_ids.is_empty() {
        let (fb_resp, _reader) = call_tool(
            stdin,
            reader,
            51,
            "am_feedback",
            serde_json::json!({
                "query": "rust ownership",
                "neighborhood_ids": recalled_ids,
                "signal": "boost"
            }),
        );
        assert!(
            fb_resp["result"]["isError"].is_null() || fb_resp["result"]["isError"] == false,
            "feedback should succeed"
        );
    }

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_batch_query() {
    let (mut child, reader, _dir) = setup_with_data();
    let stdin = child.stdin.as_mut().unwrap();

    let (resp, _reader) = call_tool(
        stdin,
        reader,
        60,
        "am_batch_query",
        serde_json::json!({
            "queries": [
                { "query": "rust ownership" },
                { "query": "borrow checker" }
            ]
        }),
    );
    let batch_json = extract_tool_json(&resp);
    assert!(
        batch_json.get("results").is_some(),
        "batch_query should return results array"
    );
    let results = batch_json["results"].as_array().unwrap();
    assert_eq!(results.len(), 2, "should have one result per query");

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_salient() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let reader = BufReader::new(stdout);

    let (_resp, reader) = handshake(stdin, reader);

    let (resp, _reader) = call_tool(
        stdin,
        reader,
        70,
        "am_salient",
        serde_json::json!({ "text": "Geometric memory uses quaternions on S3 manifold" }),
    );
    assert!(
        resp["result"]["isError"].is_null() || resp["result"]["isError"] == false,
        "salient should succeed"
    );

    drop(child.stdin.take());
    child.wait().unwrap();
}
