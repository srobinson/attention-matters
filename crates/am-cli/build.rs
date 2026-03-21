//! Build script - reads tools.toml and generates:
//!   src/generated_schema.rs  - MCP tool list JSON
//!   src/generated_help.rs    - CLI help string constants

use indexmap::IndexMap;
use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Deserialize)]
struct ToolsToml {
    #[allow(dead_code)]
    skill: Option<SkillConfig>,
    cli: Option<CliConfig>,
    tools: IndexMap<String, ToolDef>,
    #[serde(default)]
    commands: IndexMap<String, CommandDef>,
}

#[derive(Deserialize)]
#[allow(dead_code)]
struct SkillConfig {
    workflow: String,
}

/// Root CLI help (am --help).
#[derive(Deserialize)]
struct CliConfig {
    about: String,
    long_about: Option<String>,
    after_help: Option<String>,
}

/// MCP tool definition. Exposed via MCP protocol and optionally as a CLI subcommand.
#[derive(Deserialize)]
struct ToolDef {
    cli_name: String,
    mcp_description: String,
    cli_about: String,
    cli_long_about: Option<String>,
    cli_after_help: Option<String>,
    #[serde(default)]
    params: Vec<ParamDef>,
}

/// CLI-only command. No MCP exposure.
#[derive(Deserialize)]
struct CommandDef {
    cli_name: String,
    cli_about: String,
    cli_long_about: Option<String>,
    cli_after_help: Option<String>,
}

#[derive(Deserialize)]
struct ParamDef {
    name: String,
    #[serde(rename = "type")]
    type_: String,
    #[serde(default)]
    required: bool,
    #[serde(rename = "enum")]
    enum_values: Option<Vec<String>>,
    mcp_description: String,
    cli_help: Option<String>,
    #[allow(dead_code)]
    cli_flag: Option<String>,
    /// For array params, the scalar type of each element (e.g. "string").
    /// When absent on an array param, the items schema is an inline object.
    items_type: Option<String>,
}

fn main() {
    println!("cargo:rerun-if-changed=tools.toml");
    println!("cargo:rerun-if-changed=build.rs");

    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let tools_toml_path = Path::new(&manifest_dir).join("tools.toml");

    let content = fs::read_to_string(&tools_toml_path)
        .unwrap_or_else(|e| panic!("Failed to read tools.toml: {e}"));

    let parsed: ToolsToml =
        toml::from_str(&content).unwrap_or_else(|e| panic!("Failed to parse tools.toml: {e}"));

    let schema_rs = generate_mcp_schema(&parsed.tools);
    let help_rs = generate_cli_help(&parsed);

    write_if_changed(
        &Path::new(&manifest_dir).join("src/generated_schema.rs"),
        &schema_rs,
    );
    write_if_changed(
        &Path::new(&manifest_dir).join("src/generated_help.rs"),
        &help_rs,
    );
}

/// Only write if the content has changed to avoid spurious rebuilds.
fn write_if_changed(path: &Path, content: &str) {
    if let Ok(existing) = fs::read_to_string(path)
        && existing == content
    {
        return;
    }
    fs::write(path, content).unwrap_or_else(|e| panic!("Failed to write {}: {e}", path.display()));
}

// ---------------------------------------------------------------------------
// MCP schema generator
// ---------------------------------------------------------------------------

fn generate_mcp_schema(tools: &IndexMap<String, ToolDef>) -> String {
    let mut tool_jsons = Vec::new();

    for (tool_name, tool) in tools {
        let mut properties = serde_json::Map::new();
        let mut required: Vec<String> = Vec::new();

        for param in &tool.params {
            let mut prop = serde_json::Map::new();

            if param.type_ == "array" {
                prop.insert(
                    "type".to_string(),
                    serde_json::Value::String("array".to_string()),
                );
                let items_schema = match &param.items_type {
                    Some(scalar) if scalar == "object" => {
                        // am_batch_query.queries: array of {query, max_tokens?}
                        batch_query_item_schema(tool_name)
                    }
                    Some(scalar) => serde_json::json!({"type": scalar}),
                    None => serde_json::json!({"type": "string"}),
                };
                prop.insert("items".to_string(), items_schema);
            } else {
                prop.insert(
                    "type".to_string(),
                    serde_json::Value::String(param.type_.clone()),
                );
            }

            prop.insert(
                "description".to_string(),
                serde_json::Value::String(param.mcp_description.clone()),
            );

            if let Some(ev) = &param.enum_values {
                prop.insert(
                    "enum".to_string(),
                    serde_json::Value::Array(
                        ev.iter()
                            .map(|s| serde_json::Value::String(s.clone()))
                            .collect(),
                    ),
                );
            }

            properties.insert(param.name.clone(), serde_json::Value::Object(prop));

            if param.required {
                required.push(param.name.clone());
            }
        }

        let mut input_schema = serde_json::json!({
            "type": "object",
            "properties": properties
        });
        if !required.is_empty() {
            input_schema["required"] = serde_json::Value::Array(
                required
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            );
        }

        tool_jsons.push(serde_json::json!({
            "name": tool_name,
            "description": tool.mcp_description,
            "inputSchema": input_schema
        }));
    }

    let json_val = serde_json::json!({ "tools": tool_jsons });
    let json_str = serde_json::to_string_pretty(&json_val).expect("JSON serialization failed");

    format!(
        "// AUTO-GENERATED by build.rs from tools.toml - do not edit\n\
         #![allow(clippy::all)]\n\
         #[rustfmt::skip]\n\
         \n\
         pub fn generated_tool_list() -> serde_json::Value {{\n\
             serde_json::from_str(r##\"{}\"##).expect(\"generated tool list is valid JSON\")\n\
         }}\n",
        json_str
    )
}

/// Inline schema for batch query items: {query: string, max_tokens?: integer}
fn batch_query_item_schema(tool_name: &str) -> serde_json::Value {
    if tool_name == "am_batch_query" {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The query text"
                },
                "max_tokens": {
                    "type": "integer",
                    "description": "Optional token budget for this query's context"
                }
            },
            "required": ["query"]
        })
    } else {
        serde_json::json!({"type": "object"})
    }
}

// ---------------------------------------------------------------------------
// CLI help constants generator
// ---------------------------------------------------------------------------

fn generate_cli_help(parsed: &ToolsToml) -> String {
    let mut lines = vec![
        "// AUTO-GENERATED by build.rs from tools.toml - do not edit".to_string(),
        "#![allow(dead_code, unused)]".to_string(),
        "#![allow(clippy::all)]".to_string(),
    ];

    // Root CLI help
    if let Some(cli) = &parsed.cli {
        emit_const(&mut lines, "CLI", "ABOUT", &cli.about);
        if let Some(v) = &cli.long_about {
            emit_const(&mut lines, "CLI", "LONG_ABOUT", v);
        }
        if let Some(v) = &cli.after_help {
            emit_const(&mut lines, "CLI", "AFTER_HELP", v);
        }
        lines.push(String::new());
    }

    // MCP tools
    for tool in parsed.tools.values() {
        let prefix = tool.cli_name.to_uppercase().replace('-', "_");
        emit_const(&mut lines, &prefix, "ABOUT", &tool.cli_about);
        if let Some(v) = &tool.cli_long_about {
            emit_const(&mut lines, &prefix, "LONG_ABOUT", v);
        }
        if let Some(v) = &tool.cli_after_help {
            emit_const(&mut lines, &prefix, "AFTER_HELP", v);
        }
        for param in &tool.params {
            if let Some(help) = &param.cli_help {
                let param_upper = param.name.to_uppercase().replace('-', "_");
                emit_const(&mut lines, &prefix, &format!("{param_upper}_HELP"), help);
            }
        }
        lines.push(String::new());
    }

    // CLI-only commands
    for cmd in parsed.commands.values() {
        let prefix = cmd.cli_name.to_uppercase().replace('-', "_");
        emit_const(&mut lines, &prefix, "ABOUT", &cmd.cli_about);
        if let Some(v) = &cmd.cli_long_about {
            emit_const(&mut lines, &prefix, "LONG_ABOUT", v);
        }
        if let Some(v) = &cmd.cli_after_help {
            emit_const(&mut lines, &prefix, "AFTER_HELP", v);
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

/// Emit a single `pub const PREFIX_SUFFIX: &str = "...";` line.
fn emit_const(lines: &mut Vec<String>, prefix: &str, suffix: &str, value: &str) {
    let escaped = rust_escape(value);
    lines.push("#[rustfmt::skip]".to_string());
    lines.push(format!(
        "pub const {prefix}_{suffix}: &str = \"{escaped}\";"
    ));
}

/// Escape a string for embedding in a Rust double-quoted string literal.
fn rust_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}
