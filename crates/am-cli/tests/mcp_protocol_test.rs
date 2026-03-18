//! MCP protocol integration tests.
//!
//! Spawns `am serve` as a subprocess, sends JSON-RPC messages over stdin,
//! and asserts on stdout responses. Validates the full MCP wire protocol
//! from initialize through tool calls.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};
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

/// Read one JSON-RPC response line from stdout (blocking with timeout).
fn recv(reader: &mut BufReader<std::process::ChildStdout>) -> serde_json::Value {
    let mut line = String::new();
    reader.read_line(&mut line).expect("read stdout");
    serde_json::from_str(line.trim()).expect("parse JSON-RPC response")
}

/// Send initialize + initialized, return the initialize response.
fn handshake(
    stdin: &mut impl Write,
    reader: &mut BufReader<std::process::ChildStdout>,
) -> serde_json::Value {
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
    let init_resp = recv(reader);

    // Send initialized notification (no response expected)
    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }),
    );

    init_resp
}

#[test]
fn initialize_response_has_required_fields() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    let resp = handshake(stdin, &mut reader);

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);

    let result = &resp["result"];
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert_eq!(result["serverInfo"]["name"], "am");
    assert!(result["capabilities"]["tools"].is_object());
    assert!(result["instructions"].is_string());

    drop(child.stdin.take()); // close stdin to exit
    let status = child.wait().unwrap();
    assert!(status.success());
}

#[test]
fn tools_list_returns_all_12_tools() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    handshake(stdin, &mut reader);

    // Request tools/list
    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        }),
    );
    let resp = recv(&mut reader);

    assert_eq!(resp["id"], 2);
    let tools = resp["result"]["tools"].as_array().expect("tools array");
    assert_eq!(tools.len(), 12, "should have exactly 12 tools");

    // Verify all expected tool names are present
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

    // Each tool should have a description and inputSchema
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
    let mut reader = BufReader::new(stdout);

    handshake(stdin, &mut reader);

    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {
                "name": "am_stats",
                "arguments": {}
            }
        }),
    );
    let resp = recv(&mut reader);

    assert_eq!(resp["id"], 3);
    let content = &resp["result"]["content"];
    assert!(content.is_array(), "result should have content array");

    let text = content[0]["text"].as_str().expect("text content");
    let stats: serde_json::Value = serde_json::from_str(text).expect("stats JSON");

    assert_eq!(stats["n"], 0);
    assert_eq!(stats["episodes"], 0);
    assert_eq!(stats["conscious"], 0);

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn tools_call_am_ingest_then_query() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    handshake(stdin, &mut reader);

    // Ingest a document
    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {
                "name": "am_ingest",
                "arguments": {
                    "text": "Rust ownership rules prevent data races at compile time. The borrow checker enforces these rules automatically.",
                    "name": "rust-safety"
                }
            }
        }),
    );
    let ingest_resp = recv(&mut reader);
    assert_eq!(ingest_resp["id"], 4);

    let ingest_text = ingest_resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap();
    let ingest_json: serde_json::Value = serde_json::from_str(ingest_text).unwrap();
    assert_eq!(ingest_json["episode"], "rust-safety");
    assert!(ingest_json["neighborhoods"].as_u64().unwrap() >= 1);

    // Query the ingested content
    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "tools/call",
            "params": {
                "name": "am_query",
                "arguments": {
                    "text": "rust borrow checker ownership"
                }
            }
        }),
    );
    let query_resp = recv(&mut reader);
    assert_eq!(query_resp["id"], 5);

    let query_text = query_resp["result"]["content"][0]["text"].as_str().unwrap();
    let query_json: serde_json::Value = serde_json::from_str(query_text).unwrap();

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
    let mut reader = BufReader::new(stdout);

    handshake(stdin, &mut reader);

    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 6,
            "method": "tools/call",
            "params": {
                "name": "nonexistent_tool",
                "arguments": {}
            }
        }),
    );
    let resp = recv(&mut reader);

    assert_eq!(resp["id"], 6);
    // Unknown tool returns isError in the MCP result (not a JSON-RPC error)
    let result = &resp["result"];
    assert_eq!(result["isError"], true);

    drop(child.stdin.take());
    child.wait().unwrap();
}

#[test]
fn ping_returns_empty_object() {
    let dir = TempDir::new().unwrap();
    let mut child = spawn_serve(&dir);
    let stdin = child.stdin.as_mut().unwrap();
    let stdout = child.stdout.take().unwrap();
    let mut reader = BufReader::new(stdout);

    handshake(stdin, &mut reader);

    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "method": "ping"
        }),
    );
    let resp = recv(&mut reader);

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
    let mut reader = BufReader::new(stdout);

    handshake(stdin, &mut reader);

    send(
        stdin,
        &serde_json::json!({
            "jsonrpc": "2.0",
            "id": 8,
            "method": "resources/list"
        }),
    );
    let resp = recv(&mut reader);

    assert_eq!(resp["id"], 8);
    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32601);

    drop(child.stdin.take());
    child.wait().unwrap();
}
