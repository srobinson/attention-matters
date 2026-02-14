//! Claude Code session transcript discovery and parsing.
//!
//! Reads `.jsonl` session transcripts from Claude Code's project directory
//! and extracts substantive text (user questions + assistant responses),
//! filtering out tool calls, thinking blocks, system messages, and file
//! history snapshots.

use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result};

/// A discovered session transcript file.
pub struct SessionInfo {
    pub session_id: String,
    pub path: PathBuf,
}

/// Resolve the Claude Code config directory.
/// Priority: explicit override > CLAUDE_CONFIG_DIR env > ~/.claude
pub fn resolve_claude_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(dir) = override_dir {
        return dir.to_path_buf();
    }
    env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            env::var("HOME")
                .or_else(|_| env::var("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("."))
                .join(".claude")
        })
}

/// Find the Claude project directory matching the current working directory.
///
/// Claude Code encodes project paths as directory names by replacing `/` with `-`.
/// For example, `/Users/foo/my-project` becomes `-Users-foo-my-project`.
///
/// We also check git worktree roots and the main repo root to handle
/// worktree-based workflows.
pub fn find_project_dir(claude_dir: &Path) -> Option<PathBuf> {
    let projects_dir = claude_dir.join("projects");
    if !projects_dir.is_dir() {
        return None;
    }

    let cwd = env::current_dir().ok()?;

    // Build candidate paths to check (CWD first, then git root)
    let mut candidates = vec![cwd.clone()];

    // Also try the git repo root (handles worktrees)
    if let Ok(output) = std::process::Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(&cwd)
        .output()
        && output.status.success() {
            let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !root.is_empty() {
                let root_path = PathBuf::from(&root);
                if root_path != cwd {
                    candidates.push(root_path);
                }
            }
        }

    for candidate in &candidates {
        let encoded = encode_path(candidate);
        let project_path = projects_dir.join(&encoded);
        if project_path.is_dir() {
            return Some(project_path);
        }
    }

    None
}

/// Encode a filesystem path into Claude Code's directory name format.
/// `/Users/foo/bar` → `-Users-foo-bar`
fn encode_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    s.replace('/', "-")
}

/// Discover all session transcript files in a Claude project directory.
pub fn discover_sessions(project_dir: &Path) -> Result<Vec<SessionInfo>> {
    let entries = fs::read_dir(project_dir)
        .with_context(|| format!("failed to read {}", project_dir.display()))?;

    let mut sessions = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
            continue;
        }
        if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
            sessions.push(SessionInfo {
                session_id: stem.to_string(),
                path,
            });
        }
    }

    sessions.sort_by(|a, b| a.session_id.cmp(&b.session_id));
    Ok(sessions)
}

/// Extract substantive text from a session transcript.
///
/// Reads a JSONL file and extracts user message text and assistant response
/// text. Filters out:
/// - `file-history-snapshot` entries
/// - `tool_result` entries
/// - `thinking` blocks in assistant messages
/// - `tool_use` blocks in assistant messages
/// - `system` and `progress` entries
/// - Very short messages (< 20 chars)
pub fn extract_session_text(path: &Path) -> Result<String> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    let mut parts: Vec<String> = Vec::new();

    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        let obj: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");

        match msg_type {
            "user" => {
                if let Some(text) = extract_user_text(&obj)
                    && text.len() >= 20 && !is_system_prompt(&text) {
                        parts.push(text);
                    }
            }
            "assistant" => {
                if let Some(text) = extract_assistant_text(&obj)
                    && text.len() >= 20 {
                        parts.push(text);
                    }
            }
            _ => continue,
        }
    }

    Ok(parts.join("\n\n"))
}

/// Extract text content from a user message.
fn extract_user_text(obj: &serde_json::Value) -> Option<String> {
    let content = obj.get("message")?.get("content")?;

    // Content can be a string or an array of content blocks
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    if let Some(arr) = content.as_array() {
        let texts: Vec<&str> = arr
            .iter()
            .filter_map(|block| {
                if block.get("type")?.as_str()? == "text" {
                    block.get("text")?.as_str()
                } else {
                    None
                }
            })
            .collect();
        if texts.is_empty() {
            return None;
        }
        return Some(texts.join("\n"));
    }

    None
}

/// Extract text-only content from an assistant message (no thinking/tool_use).
fn extract_assistant_text(obj: &serde_json::Value) -> Option<String> {
    let content = obj.get("message")?.get("content")?.as_array()?;

    let texts: Vec<&str> = content
        .iter()
        .filter_map(|block| {
            let block_type = block.get("type")?.as_str()?;
            if block_type == "text" {
                let text = block.get("text")?.as_str()?;
                if !text.trim().is_empty() {
                    return Some(text);
                }
            }
            None
        })
        .collect();

    if texts.is_empty() {
        return None;
    }
    Some(texts.join("\n"))
}

/// Heuristic: detect system prompts / orchestrator prompts that shouldn't
/// be ingested as memory (they're boilerplate, not project knowledge).
fn is_system_prompt(text: &str) -> bool {
    let start = &text[..text.len().min(200)];
    start.contains("# Orchestrator")
        || start.contains("# Authority")
        || start.contains("# System")
        || start.starts_with("<system")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_encode_path() {
        assert_eq!(
            encode_path(Path::new("/Users/foo/my-project")),
            "-Users-foo-my-project"
        );
        assert_eq!(encode_path(Path::new("/a/b/c")), "-a-b-c");
    }

    #[test]
    fn test_resolve_claude_dir_override() {
        let dir = resolve_claude_dir(Some(Path::new("/custom/dir")));
        assert_eq!(dir, PathBuf::from("/custom/dir"));
    }

    #[test]
    fn test_resolve_claude_dir_default() {
        let dir = resolve_claude_dir(None);
        // Should end with .claude (unless CLAUDE_CONFIG_DIR is set)
        let dir_str = dir.to_string_lossy();
        assert!(
            dir_str.ends_with(".claude") || dir_str.contains("claude"),
            "expected .claude dir, got: {dir_str}"
        );
    }

    #[test]
    fn test_discover_sessions() {
        let dir = TempDir::new().unwrap();

        // Create some fake session files
        fs::write(dir.path().join("abc-123.jsonl"), "{\"type\":\"user\"}").unwrap();
        fs::write(dir.path().join("def-456.jsonl"), "{\"type\":\"user\"}").unwrap();
        // Non-jsonl file should be skipped
        fs::write(dir.path().join("readme.txt"), "hello").unwrap();
        // Directory should be skipped
        fs::create_dir(dir.path().join("some-dir")).unwrap();

        let sessions = discover_sessions(dir.path()).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "abc-123");
        assert_eq!(sessions[1].session_id, "def-456");
    }

    #[test]
    fn test_extract_session_text_user_and_assistant() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jsonl");
        let mut f = fs::File::create(&path).unwrap();

        // User message with string content
        writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"How does authentication work in this codebase? I need to understand the middleware chain."}}}}"#).unwrap();
        // Assistant message with text content
        writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"The authentication middleware uses JWT tokens stored in HTTP-only cookies."}}]}}}}"#).unwrap();
        // File snapshot (should be skipped)
        writeln!(f, r#"{{"type":"file-history-snapshot","snapshot":{{}}}}"#).unwrap();
        // Tool use (should be skipped - no text blocks)
        writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","name":"Read","input":{{}}}}]}}}}"#).unwrap();
        // Thinking block (should be skipped)
        writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"thinking","thinking":"let me think..."}}]}}}}"#).unwrap();

        let text = extract_session_text(&path).unwrap();
        assert!(text.contains("authentication"));
        assert!(text.contains("JWT tokens"));
        assert!(!text.contains("file-history-snapshot"));
        assert!(!text.contains("tool_use"));
        assert!(!text.contains("let me think"));
    }

    #[test]
    fn test_extract_session_text_filters_short_messages() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jsonl");
        let mut f = fs::File::create(&path).unwrap();

        // Short message (< 20 chars) — should be filtered
        writeln!(
            f,
            r#"{{"type":"user","message":{{"role":"user","content":"yes"}}}}"#
        )
        .unwrap();
        // Long enough message
        writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"This is a substantive question about the architecture of the system."}}}}"#).unwrap();

        let text = extract_session_text(&path).unwrap();
        assert!(!text.contains("yes"));
        assert!(text.contains("substantive question"));
    }

    #[test]
    fn test_extract_session_text_filters_system_prompts() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jsonl");
        let mut f = fs::File::create(&path).unwrap();

        writeln!(f, "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"# Orchestrator\\n\\nYou are supervising a worker agent...\"}}}}").unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"What does the authentication middleware do in this project?"}}}}"#).unwrap();

        let text = extract_session_text(&path).unwrap();
        assert!(!text.contains("Orchestrator"));
        assert!(text.contains("authentication middleware"));
    }

    #[test]
    fn test_extract_session_text_empty_session() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jsonl");
        let mut f = fs::File::create(&path).unwrap();

        writeln!(f, r#"{{"type":"file-history-snapshot","snapshot":{{}}}}"#).unwrap();
        writeln!(f, r#"{{"type":"file-history-snapshot","snapshot":{{}}}}"#).unwrap();

        let text = extract_session_text(&path).unwrap();
        assert!(text.is_empty());
    }

    #[test]
    fn test_is_system_prompt() {
        assert!(is_system_prompt("# Orchestrator\nSome instructions"));
        assert!(is_system_prompt("# Authority\nThe Human controls this"));
        assert!(is_system_prompt(
            "<system-reminder>something</system-reminder>"
        ));
        assert!(!is_system_prompt("How does the database migration work?"));
    }

    #[test]
    fn test_user_content_array_format() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.jsonl");
        let mut f = fs::File::create(&path).unwrap();

        // User message with array content (some Claude versions use this)
        writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"text","text":"How does the query engine work in this codebase?"}}]}}}}"#).unwrap();

        let text = extract_session_text(&path).unwrap();
        assert!(text.contains("query engine"));
    }
}
