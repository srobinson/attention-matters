//! Manual JSON-RPC over stdio transport for the MCP server.
//!
//! Replaces rmcp with a direct readline loop. Single-client sequential
//! request/response protocol over stdin/stdout.

use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[path = "generated_schema.rs"]
mod generated_schema;

// ── Constants ────────────────────────────────────────────────────

const PROTOCOL_VERSION: &str = "2024-11-05";

const SERVER_INSTRUCTIONS: &str = "\
Query geometric memory at the START of every session with am_query. \
Buffer substantive exchanges with am_buffer. Mark important insights \
with am_salient. Use am_feedback to reinforce helpful recall.";

// ── JSON-RPC Types ───────────────────────────────────────────────

#[derive(Debug, Deserialize)]
pub struct JsonRpcRequest {
    #[serde(rename = "jsonrpc")]
    pub _jsonrpc: String,
    pub id: Option<Value>,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

impl JsonRpcResponse {
    pub fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Value, error: JsonRpcError) -> Self {
        Self {
            jsonrpc: "2.0".to_owned(),
            id,
            result: None,
            error: Some(error),
        }
    }
}

// ── Tool call result helpers ─────────────────────────────────────

/// Build a successful MCP tool call result with text content.
pub fn tool_result_text(text: &str) -> Value {
    serde_json::json!({
        "content": [{"type": "text", "text": text}]
    })
}

/// Build an error MCP tool call result with text content.
/// Uses text content (not isError) for Claude Code compatibility.
pub fn tool_result_error(message: &str) -> Value {
    serde_json::json!({
        "content": [{"type": "text", "text": message}],
        "isError": true
    })
}

// ── Transport ────────────────────────────────────────────────────

/// Run the JSON-RPC stdio loop.
///
/// `dispatch_tool` is called for each `tools/call` request with the
/// tool name and arguments. Returns `Ok(Value)` on success or
/// `Err(String)` on tool-level error.
///
/// # Errors
/// Returns an error if stdin/stdout I/O fails (not for protocol errors,
/// which are handled inline).
pub fn run_stdio_loop<F>(mut dispatch_tool: F) -> anyhow::Result<()>
where
    F: FnMut(&str, &Value) -> Result<Value, String>,
{
    let stdin = io::stdin();
    let mut stdout = io::stdout();

    for line in stdin.lock().lines() {
        let line = line?;
        if line.is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(req) => req,
            Err(e) => {
                let resp = JsonRpcResponse::error(
                    Value::Null,
                    JsonRpcError {
                        code: -32700,
                        message: format!("Parse error: {e}"),
                        data: None,
                    },
                );
                writeln!(stdout, "{}", serde_json::to_string(&resp)?)?;
                stdout.flush()?;
                continue;
            }
        };

        let id = request.id.clone().unwrap_or(Value::Null);

        let response = match request.method.as_str() {
            "initialize" => Some(JsonRpcResponse::success(id, handle_initialize())),
            _ if request.method.starts_with("notifications/") => None,
            "tools/list" => Some(JsonRpcResponse::success(
                id,
                generated_schema::generated_tool_list(),
            )),
            "tools/call" => Some(handle_tool_call(id, &request.params, &mut dispatch_tool)),
            "ping" => Some(JsonRpcResponse::success(id, serde_json::json!({}))),
            _ => Some(JsonRpcResponse::error(
                id,
                JsonRpcError {
                    code: -32601,
                    message: format!("Method not found: {}", request.method),
                    data: None,
                },
            )),
        };

        if let Some(resp) = response {
            let write_result =
                writeln!(stdout, "{}", serde_json::to_string(&resp)?).and_then(|()| stdout.flush());
            if let Err(e) = write_result {
                if e.kind() == std::io::ErrorKind::BrokenPipe {
                    return Ok(());
                }
                return Err(e.into());
            }
        }
    }

    Ok(())
}

fn handle_initialize() -> Value {
    serde_json::json!({
        "protocolVersion": PROTOCOL_VERSION,
        "capabilities": {
            "tools": {}
        },
        "serverInfo": {
            "name": "am",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": SERVER_INSTRUCTIONS
    })
}

fn handle_tool_call<F>(id: Value, params: &Option<Value>, dispatch_tool: &mut F) -> JsonRpcResponse
where
    F: FnMut(&str, &Value) -> Result<Value, String>,
{
    let Some(params) = params.as_ref() else {
        return JsonRpcResponse::error(
            id,
            JsonRpcError {
                code: -32602,
                message: "Missing params".to_owned(),
                data: None,
            },
        );
    };

    let Some(tool_name) = params.get("name").and_then(Value::as_str) else {
        return JsonRpcResponse::error(
            id,
            JsonRpcError {
                code: -32602,
                message: "Missing tool name".to_owned(),
                data: None,
            },
        );
    };

    let arguments = params
        .get("arguments")
        .cloned()
        .unwrap_or(serde_json::json!({}));

    match dispatch_tool(tool_name, &arguments) {
        Ok(result) => JsonRpcResponse::success(id, result),
        Err(msg) => JsonRpcResponse::success(id, tool_result_error(&msg)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initialize_response_shape() {
        let resp = handle_initialize();
        assert_eq!(resp["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(resp["serverInfo"]["name"], "am");
        assert!(resp["capabilities"]["tools"].is_object());
    }

    #[test]
    fn test_tool_list_has_12_tools() {
        let list = generated_schema::generated_tool_list();
        let tools = list["tools"].as_array().expect("tools should be an array");
        assert_eq!(tools.len(), 12);
    }

    #[test]
    fn test_tool_result_text_shape() {
        let result = tool_result_text("hello");
        let content = result["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "hello");
    }

    #[test]
    fn test_tool_result_error_shape() {
        let result = tool_result_error("oops");
        assert_eq!(result["isError"], true);
        assert_eq!(result["content"][0]["text"], "oops");
    }

    #[test]
    fn test_handle_tool_call_missing_params() {
        let resp = handle_tool_call(Value::Number(1.into()), &None, &mut |_, _| {
            Ok(serde_json::json!({}))
        });
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn test_handle_tool_call_dispatch() {
        let params = serde_json::json!({
            "name": "am_stats",
            "arguments": {}
        });
        let resp = handle_tool_call(
            Value::Number(1.into()),
            &Some(params),
            &mut |name, _args| {
                assert_eq!(name, "am_stats");
                Ok(tool_result_text("ok"))
            },
        );
        assert!(resp.result.is_some());
        assert!(resp.error.is_none());
    }
}
