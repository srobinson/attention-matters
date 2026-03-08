//! Claude Code session transcript parsing and episode extraction.
//!
//! Reads `.jsonl` session transcripts and extracts episodes from them.
//! A session maps to one or more episodes depending on length.
//!
//! Content extraction rules:
//!   - Main chain: user text + assistant text + assistant thinking
//!   - Sidechains: user prompt + assistant text + assistant thinking
//!   - Ignored: tool_use blocks, tool_result messages, system prompts,
//!     file-history-snapshot entries, very short messages (< 20 chars)
//!
//! Long sessions are chunked into multiple episodes (EXCHANGES_PER_EPISODE
//! user turns per episode, matching the source DAE's episodic model).

use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::{env, fs};

use anyhow::{Context, Result};
use pulldown_cmark::{Event, Options, Parser, TagEnd};
use serde::Deserialize;

/// How many user turns per main-chain episode.
/// Matches the source DAE's "every 5 exchanges become a memory episode."
const EXCHANGES_PER_EPISODE: usize = 5;

/// Minimum content length for a text fragment to be included.
const MIN_TEXT_LEN: usize = 20;

/// Hook payload sent by Claude Code on stdin (PreCompact / Stop hooks).
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct HookInput {
    pub session_id: String,
    pub transcript_path: String,
    /// "PreCompact" or "Stop"
    #[serde(default)]
    pub hook_event_name: Option<String>,
}

/// Read hook input from stdin if it's piped (not a terminal).
/// Returns `None` when running interactively.
pub fn read_hook_input() -> Option<HookInput> {
    let stdin = io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut buf = String::new();
    stdin.lock().read_to_string(&mut buf).ok()?;
    let buf = buf.trim();
    if buf.is_empty() {
        return None;
    }
    serde_json::from_str(buf).ok()
}

/// A discovered session transcript file.
pub struct SessionInfo {
    pub session_id: String,
    pub path: PathBuf,
}

/// An episode extracted from a transcript.
pub struct ExtractedEpisode {
    pub name: String,
    pub text: String,
}

/// A single exchange: one user turn and everything the assistant produces
/// before the next user turn.
struct Exchange {
    parts: Vec<String>,
}

impl Exchange {
    fn new() -> Self {
        Self { parts: Vec::new() }
    }

    fn push(&mut self, text: String) {
        self.parts.push(text);
    }

    fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Episode extraction
// ---------------------------------------------------------------------------

/// Extract episodes from a session transcript.
///
/// Parses the JSONL transcript and produces one or more episodes:
///   - Main chain content is grouped into exchanges (one per user turn)
///     and chunked into episodes of EXCHANGES_PER_EPISODE.
///   - Each subagent's work (identified by slug) becomes its own episode.
pub fn extract_episodes(path: &Path, session_prefix: &str) -> Result<Vec<ExtractedEpisode>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;

    let mut main_exchanges: Vec<Exchange> = Vec::new();
    let mut current = Exchange::new();

    // Sidechain content grouped by agent slug/id
    let mut sidechains: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        let obj: serde_json::Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => continue,
        };

        let msg_type = obj.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if msg_type != "user" && msg_type != "assistant" {
            continue;
        }

        let is_sidechain = obj
            .get("isSidechain")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if is_sidechain {
            extract_sidechain_entry(&obj, msg_type, &mut sidechains);
        } else {
            extract_main_entry(&obj, msg_type, &mut main_exchanges, &mut current);
        }
    }

    // Flush the last exchange
    if !current.is_empty() {
        main_exchanges.push(current);
    }

    build_episodes(&main_exchanges, &sidechains, session_prefix)
}

/// Route a main-chain JSONL entry into the exchange list.
fn extract_main_entry(
    obj: &serde_json::Value,
    msg_type: &str,
    exchanges: &mut Vec<Exchange>,
    current: &mut Exchange,
) {
    match msg_type {
        "user" => {
            if let Some(text) = extract_user_text(obj)
                && text.len() >= MIN_TEXT_LEN
                && !is_system_prompt(&text)
            {
                // New user turn: close the previous exchange
                if !current.is_empty() {
                    let finished = std::mem::replace(current, Exchange::new());
                    exchanges.push(finished);
                }
                current.push(format!("[user]\n{}", strip_markdown(&text)));
            }
        }
        "assistant" => {
            let parts = extract_content_blocks(obj);
            let parts: Vec<String> = parts
                .into_iter()
                .filter(|p| p.len() >= MIN_TEXT_LEN)
                .collect();
            if !parts.is_empty() {
                current.push(format!("[assistant]\n{}", parts.join("\n")));
            }
        }
        _ => {}
    }
}

/// Route a sidechain JSONL entry into the agent map.
fn extract_sidechain_entry(
    obj: &serde_json::Value,
    msg_type: &str,
    sidechains: &mut BTreeMap<String, Vec<String>>,
) {
    let agent_key = obj
        .get("slug")
        .or_else(|| obj.get("agentId"))
        .and_then(|v| v.as_str())
        .unwrap_or("agent")
        .to_string();

    match msg_type {
        "user" => {
            if let Some(text) = extract_user_text(obj)
                && text.len() >= MIN_TEXT_LEN
                && !is_system_prompt(&text)
            {
                sidechains
                    .entry(agent_key)
                    .or_default()
                    .push(format!("[user]\n{}", strip_markdown(&text)));
            }
        }
        "assistant" => {
            let parts = extract_content_blocks(obj);
            let parts: Vec<String> = parts
                .into_iter()
                .filter(|p| p.len() >= MIN_TEXT_LEN)
                .collect();
            if !parts.is_empty() {
                sidechains
                    .entry(agent_key)
                    .or_default()
                    .push(format!("[assistant]\n{}", parts.join("\n")));
            }
        }
        _ => {}
    }
}

/// Assemble extracted content into named episodes.
fn build_episodes(
    main_exchanges: &[Exchange],
    sidechains: &BTreeMap<String, Vec<String>>,
    session_prefix: &str,
) -> Result<Vec<ExtractedEpisode>> {
    let mut episodes = Vec::new();

    // Main chain: chunk exchanges into episodes
    if !main_exchanges.is_empty() {
        let chunks: Vec<Vec<String>> = main_exchanges
            .chunks(EXCHANGES_PER_EPISODE)
            .map(|chunk| {
                chunk
                    .iter()
                    .flat_map(|ex| ex.parts.iter().cloned())
                    .collect()
            })
            .collect();

        if chunks.len() == 1 {
            let text = chunks.into_iter().next().unwrap().join("\n\n");
            if !text.is_empty() {
                episodes.push(ExtractedEpisode {
                    name: format!("session-{session_prefix}"),
                    text,
                });
            }
        } else {
            for (i, parts) in chunks.into_iter().enumerate() {
                let text = parts.join("\n\n");
                if !text.is_empty() {
                    episodes.push(ExtractedEpisode {
                        name: format!("session-{session_prefix}-{}", i + 1),
                        text,
                    });
                }
            }
        }
    }

    // Each sidechain agent becomes its own episode
    for (agent, parts) in sidechains {
        if parts.is_empty() {
            continue;
        }
        let text = parts.join("\n\n");
        if text.len() >= MIN_TEXT_LEN {
            // Truncate slug to keep episode names reasonable
            let safe_agent: String = agent.chars().take(30).collect();
            episodes.push(ExtractedEpisode {
                name: format!("session-{session_prefix}-{safe_agent}"),
                text,
            });
        }
    }

    Ok(episodes)
}

// ---------------------------------------------------------------------------
// Markdown stripping
// ---------------------------------------------------------------------------

/// Strip markdown formatting to plain text.
///
/// Uses pulldown-cmark to parse and extract only text content.
/// Tables, headers, bold/italic, links, lists, and code spans are
/// all reduced to their text content with structural whitespace.
fn strip_markdown(md: &str) -> String {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);

    let mut out = String::with_capacity(md.len());

    for event in Parser::new_ext(md, opts) {
        match event {
            Event::Text(t) | Event::Code(t) => out.push_str(&t),
            Event::SoftBreak | Event::HardBreak => out.push('\n'),
            Event::End(TagEnd::TableCell) => out.push(' '),
            Event::End(TagEnd::TableRow)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Item) => out.push('\n'),
            _ => {}
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Content extraction helpers
// ---------------------------------------------------------------------------

/// Extract text content from a user message.
///
/// Handles both string content and array-of-blocks content.
/// Returns None for tool_result messages (array content where no blocks
/// have type "text").
fn extract_user_text(obj: &serde_json::Value) -> Option<String> {
    let content = obj.get("message")?.get("content")?;

    // String content (common for main-chain user messages and subagent prompts)
    if let Some(text) = content.as_str() {
        return Some(text.to_string());
    }

    // Array content: extract only "text" blocks (skips tool_result blocks)
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

/// Extract text and thinking blocks from an assistant message.
///
/// Returns all substantive content. Skips tool_use blocks entirely.
fn extract_content_blocks(obj: &serde_json::Value) -> Vec<String> {
    let Some(content) = obj
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_array())
    else {
        return Vec::new();
    };

    content
        .iter()
        .filter_map(|block| {
            let block_type = block.get("type")?.as_str()?;
            let raw = match block_type {
                "text" => block.get("text")?.as_str()?,
                "thinking" => block.get("thinking")?.as_str()?,
                _ => return None,
            };
            let text = strip_markdown(raw);
            let trimmed = text.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .collect()
}

/// Heuristic: detect system prompts / orchestrator prompts that shouldn't
/// be ingested as memory (they're boilerplate, not project knowledge).
fn is_system_prompt(text: &str) -> bool {
    // Take up to 200 chars (not bytes) to avoid panicking on multi-byte UTF-8
    let start: String = text.chars().take(200).collect();
    start.contains("# Orchestrator")
        || start.contains("# Authority")
        || start.contains("# System")
        || start.starts_with("<system")
}

// ---------------------------------------------------------------------------
// Legacy: flat text extraction (used by --all bulk re-ingest)
// ---------------------------------------------------------------------------

/// Extract substantive text from a session transcript.
///
/// Simpler extraction that concatenates all user/assistant text into a
/// single string. Used by the --all discovery path for bulk re-ingest.
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
                    && text.len() >= MIN_TEXT_LEN
                    && !is_system_prompt(&text)
                {
                    parts.push(format!("[user]\n{}", strip_markdown(&text)));
                }
            }
            "assistant" => {
                let blocks: Vec<String> = extract_content_blocks(&obj)
                    .into_iter()
                    .filter(|p| p.len() >= MIN_TEXT_LEN)
                    .collect();
                if !blocks.is_empty() {
                    parts.push(format!("[assistant]\n{}", blocks.join("\n")));
                }
            }
            _ => continue,
        }
    }

    Ok(parts.join("\n\n"))
}

// ---------------------------------------------------------------------------
// Filesystem helpers
// ---------------------------------------------------------------------------

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
        && output.status.success()
    {
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
/// `/Users/foo/bar` -> `-Users-foo-bar`
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    // --- Helper to write JSONL test transcripts ---

    fn main_user(content: &str) -> String {
        format!(
            r#"{{"type":"user","isSidechain":false,"message":{{"role":"user","content":"{content}"}}}}"#
        )
    }

    fn main_assistant_text(text: &str) -> String {
        format!(
            r#"{{"type":"assistant","isSidechain":false,"message":{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}]}}}}"#
        )
    }

    fn main_assistant_thinking(thinking: &str) -> String {
        format!(
            r#"{{"type":"assistant","isSidechain":false,"message":{{"role":"assistant","content":[{{"type":"thinking","thinking":"{thinking}"}}]}}}}"#
        )
    }

    fn main_assistant_tool_use() -> String {
        r#"{"type":"assistant","isSidechain":false,"message":{"role":"assistant","content":[{"type":"tool_use","name":"Read","input":{}}]}}"#.to_string()
    }

    fn sidechain_user(slug: &str, content: &str) -> String {
        format!(
            r#"{{"type":"user","isSidechain":true,"slug":"{slug}","message":{{"role":"user","content":"{content}"}}}}"#
        )
    }

    fn sidechain_assistant_text(slug: &str, text: &str) -> String {
        format!(
            r#"{{"type":"assistant","isSidechain":true,"slug":"{slug}","message":{{"role":"assistant","content":[{{"type":"text","text":"{text}"}}]}}}}"#
        )
    }

    fn sidechain_tool_result(slug: &str) -> String {
        format!(
            r#"{{"type":"user","isSidechain":true,"slug":"{slug}","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"toolu_123","content":"output data"}}]}}}}"#
        )
    }

    fn write_transcript(dir: &TempDir, lines: &[String]) -> PathBuf {
        let path = dir.path().join("test.jsonl");
        let mut f = fs::File::create(&path).unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
        path
    }

    // --- extract_episodes tests ---

    #[test]
    fn test_short_session_single_episode() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("How does authentication work in this codebase?"),
                main_assistant_text(
                    "The auth middleware uses JWT tokens stored in HTTP-only cookies.",
                ),
                main_user("What about the refresh token flow?"),
                main_assistant_text(
                    "Refresh tokens are rotated on each use with a 7-day sliding window.",
                ),
            ],
        );

        let episodes = extract_episodes(&path, "abc12345").unwrap();
        assert_eq!(episodes.len(), 1);
        assert_eq!(episodes[0].name, "session-abc12345");
        assert!(episodes[0].text.contains("authentication"));
        assert!(episodes[0].text.contains("JWT tokens"));
        assert!(episodes[0].text.contains("refresh token"));
    }

    #[test]
    fn test_role_headers_in_episodes() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("How does authentication work in this codebase?"),
                main_assistant_text(
                    "The auth middleware uses JWT tokens stored in HTTP-only cookies.",
                ),
            ],
        );

        let episodes = extract_episodes(&path, "role1234").unwrap();
        assert_eq!(episodes.len(), 1);
        assert!(episodes[0].text.contains("[user]\nHow does authentication"));
        assert!(
            episodes[0]
                .text
                .contains("[assistant]\nThe auth middleware")
        );
    }

    #[test]
    fn test_role_headers_in_session_text() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("Explain quaternion SLERP interpolation in detail."),
                main_assistant_text(
                    "SLERP produces constant-speed rotation between two quaternions.",
                ),
            ],
        );

        let text = extract_session_text(&path).unwrap();
        assert!(text.contains("[user]\nExplain quaternion"));
        assert!(text.contains("[assistant]\nSLERP produces"));
    }

    /// Build a properly escaped assistant text JSONL line.
    fn main_assistant_text_raw(text: &str) -> String {
        serde_json::json!({
            "type": "assistant",
            "isSidechain": false,
            "message": {
                "role": "assistant",
                "content": [{"type": "text", "text": text}]
            }
        })
        .to_string()
    }

    #[test]
    fn test_markdown_table_stripped() {
        let dir = TempDir::new().unwrap();
        let table_md = "Here are the results:\n\n| Module | LOC | Exports |\n|--------|-----|--------|\n| core | 500 | 12 |\n| store | 300 | 8 |\n\nThat covers the workspace.";
        let path = write_transcript(
            &dir,
            &[
                main_user("Show me the module stats in a table."),
                main_assistant_text_raw(table_md),
            ],
        );

        let episodes = extract_episodes(&path, "table123").unwrap();
        assert_eq!(episodes.len(), 1);
        let text = &episodes[0].text;
        // Table pipes and separator rows should be gone
        assert!(!text.contains('|'), "pipes still present: {text}");
        assert!(!text.contains("---"), "separator still present: {text}");
        // Cell content should remain
        assert!(text.contains("core"));
        assert!(text.contains("500"));
        assert!(text.contains("store"));
    }

    #[test]
    fn test_markdown_formatting_stripped() {
        let dir = TempDir::new().unwrap();
        let md = "## Architecture\n\nThe **query engine** uses `SLERP` for *drift*.\n\n- Step one\n- Step two\n\nSee [the docs](https://example.com) for details.";
        let path = write_transcript(
            &dir,
            &[
                main_user("Explain the architecture with formatting."),
                main_assistant_text_raw(md),
            ],
        );

        let episodes = extract_episodes(&path, "fmt12345").unwrap();
        let text = &episodes[0].text;
        // Markdown syntax should be gone
        assert!(!text.contains("##"), "heading markers present: {text}");
        assert!(!text.contains("**"), "bold markers present: {text}");
        assert!(!text.contains('`'), "backticks present: {text}");
        assert!(!text.contains("]("), "link syntax present: {text}");
        // Content should remain
        assert!(text.contains("Architecture"));
        assert!(text.contains("query engine"));
        assert!(text.contains("SLERP"));
        assert!(text.contains("drift"));
        assert!(text.contains("Step one"));
        assert!(text.contains("the docs"));
    }

    #[test]
    fn test_long_session_chunked_into_episodes() {
        let dir = TempDir::new().unwrap();
        let mut lines = Vec::new();
        // 12 user turns: should produce 3 episodes (5 + 5 + 2)
        for i in 0..12 {
            lines.push(main_user(&format!(
                "Question number {i} about the architecture of this system"
            )));
            lines.push(main_assistant_text(&format!(
                "Detailed answer number {i} covering the architecture topic"
            )));
        }

        let path = write_transcript(&dir, &lines);
        let episodes = extract_episodes(&path, "longsess").unwrap();

        assert_eq!(episodes.len(), 3);
        assert_eq!(episodes[0].name, "session-longsess-1");
        assert_eq!(episodes[1].name, "session-longsess-2");
        assert_eq!(episodes[2].name, "session-longsess-3");
    }

    #[test]
    fn test_thinking_blocks_captured() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("Explain the geometric memory model in detail."),
                main_assistant_thinking(
                    "The user wants to understand the quaternion manifold and how words drift via SLERP.",
                ),
                main_assistant_text(
                    "Memory is modeled as points on a 3-sphere using quaternion positions.",
                ),
            ],
        );

        let episodes = extract_episodes(&path, "think123").unwrap();
        assert_eq!(episodes.len(), 1);
        assert!(episodes[0].text.contains("quaternion manifold"));
        assert!(episodes[0].text.contains("3-sphere"));
    }

    #[test]
    fn test_tool_use_blocks_excluded() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("Read the configuration file and explain it."),
                main_assistant_tool_use(),
                main_assistant_text(
                    "The config uses TOML format with three sections for database, cache, and logging.",
                ),
            ],
        );

        let episodes = extract_episodes(&path, "tools123").unwrap();
        assert_eq!(episodes.len(), 1);
        assert!(!episodes[0].text.contains("tool_use"));
        assert!(episodes[0].text.contains("TOML format"));
    }

    #[test]
    fn test_sidechain_becomes_own_episode() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("Run the test suite and check for lint warnings."),
                main_assistant_text("Spawning a subagent to run the tests in parallel."),
                sidechain_user(
                    "quiet-skipping-bonbon",
                    "Run the full test suite for this Rust project and report results.",
                ),
                sidechain_assistant_text(
                    "quiet-skipping-bonbon",
                    "All 147 tests passed. No lint warnings from clippy.",
                ),
                sidechain_tool_result("quiet-skipping-bonbon"),
            ],
        );

        let episodes = extract_episodes(&path, "side1234").unwrap();
        // One main episode + one sidechain episode
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].name, "session-side1234");
        assert!(episodes[0].text.contains("test suite"));
        assert_eq!(episodes[1].name, "session-side1234-quiet-skipping-bonbon");
        assert!(episodes[1].text.contains("147 tests passed"));
    }

    #[test]
    fn test_sidechain_tool_result_excluded() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                sidechain_user("agent-abc", "Investigate the parser module structure."),
                sidechain_tool_result("agent-abc"),
                sidechain_assistant_text(
                    "agent-abc",
                    "The parser uses a registry pattern with tree-sitter grammars.",
                ),
            ],
        );

        let episodes = extract_episodes(&path, "sc123456").unwrap();
        assert_eq!(episodes.len(), 1);
        assert!(!episodes[0].text.contains("output data"));
        assert!(episodes[0].text.contains("registry pattern"));
    }

    #[test]
    fn test_system_prompts_filtered() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user(
                    "# Orchestrator\\nYou are supervising a worker agent that executes tasks.",
                ),
                main_user("What does the authentication middleware do in this project?"),
                main_assistant_text("The middleware validates bearer tokens on every request."),
            ],
        );

        let episodes = extract_episodes(&path, "sys12345").unwrap();
        assert_eq!(episodes.len(), 1);
        assert!(!episodes[0].text.contains("Orchestrator"));
        assert!(episodes[0].text.contains("bearer tokens"));
    }

    #[test]
    fn test_short_messages_filtered() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("yes"),
                main_user("ok sure"),
                main_user("This is a substantive question about the architecture of the system."),
                main_assistant_text("Here is a detailed explanation of the system architecture."),
            ],
        );

        let episodes = extract_episodes(&path, "short123").unwrap();
        assert_eq!(episodes.len(), 1);
        assert!(!episodes[0].text.contains("yes"));
        assert!(!episodes[0].text.contains("ok sure"));
        assert!(episodes[0].text.contains("substantive question"));
    }

    #[test]
    fn test_empty_session_no_episodes() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                r#"{"type":"file-history-snapshot","snapshot":{}}"#.to_string(),
                main_assistant_tool_use(),
            ],
        );

        let episodes = extract_episodes(&path, "empty123").unwrap();
        assert!(episodes.is_empty());
    }

    #[test]
    fn test_multiple_sidechains_separate_episodes() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("Run tests and also check the documentation for errors."),
                sidechain_user("test-runner", "Execute cargo test and report all results."),
                sidechain_assistant_text("test-runner", "All 32 tests passed in the workspace."),
                sidechain_user(
                    "doc-checker",
                    "Scan all markdown files for broken links and formatting issues.",
                ),
                sidechain_assistant_text(
                    "doc-checker",
                    "Found 2 broken links in README.md pointing to removed sections.",
                ),
            ],
        );

        let episodes = extract_episodes(&path, "multi123").unwrap();
        assert_eq!(episodes.len(), 3); // main + 2 sidechains

        let names: Vec<&str> = episodes.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"session-multi123"));
        assert!(names.contains(&"session-multi123-test-runner"));
        assert!(names.contains(&"session-multi123-doc-checker"));
    }

    #[test]
    fn test_user_content_array_format() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                r#"{"type":"user","isSidechain":false,"message":{"role":"user","content":[{"type":"text","text":"How does the query engine work in this codebase?"}]}}"#.to_string(),
                main_assistant_text("The query engine uses IDF-weighted activation and SLERP drift."),
            ],
        );

        let episodes = extract_episodes(&path, "arr12345").unwrap();
        assert_eq!(episodes.len(), 1);
        assert!(episodes[0].text.contains("query engine"));
    }

    // --- Legacy extract_session_text tests ---

    #[test]
    fn test_extract_session_text_includes_thinking() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("Explain quaternion SLERP interpolation in detail."),
                main_assistant_thinking(
                    "SLERP computes shortest-arc interpolation on the unit sphere.",
                ),
                main_assistant_text(
                    "SLERP produces constant-speed rotation between two quaternions.",
                ),
            ],
        );

        let text = extract_session_text(&path).unwrap();
        assert!(text.contains("shortest-arc"));
        assert!(text.contains("constant-speed"));
    }

    #[test]
    fn test_extract_session_text_filters_system_prompts() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[
                main_user("# Orchestrator\\nYou are supervising a worker agent..."),
                main_user("What does the authentication middleware do in this project?"),
                main_assistant_text("The middleware validates bearer tokens and checks expiry."),
            ],
        );

        let text = extract_session_text(&path).unwrap();
        assert!(!text.contains("Orchestrator"));
        assert!(text.contains("authentication middleware"));
    }

    #[test]
    fn test_extract_session_text_empty_session() {
        let dir = TempDir::new().unwrap();
        let path = write_transcript(
            &dir,
            &[r#"{"type":"file-history-snapshot","snapshot":{}}"#.to_string()],
        );

        let text = extract_session_text(&path).unwrap();
        assert!(text.is_empty());
    }

    // --- Existing unit tests ---

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
        let dir_str = dir.to_string_lossy();
        assert!(
            dir_str.ends_with(".claude") || dir_str.contains("claude"),
            "expected .claude dir, got: {dir_str}"
        );
    }

    #[test]
    fn test_discover_sessions() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("abc-123.jsonl"), "{\"type\":\"user\"}").unwrap();
        fs::write(dir.path().join("def-456.jsonl"), "{\"type\":\"user\"}").unwrap();
        fs::write(dir.path().join("readme.txt"), "hello").unwrap();
        fs::create_dir(dir.path().join("some-dir")).unwrap();

        let sessions = discover_sessions(dir.path()).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].session_id, "abc-123");
        assert_eq!(sessions[1].session_id, "def-456");
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
    fn test_hook_input_stop_payload() {
        let json = r#"{"session_id":"abc-123","transcript_path":"/tmp/test.jsonl","hook_event_name":"Stop"}"#;
        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.session_id, "abc-123");
        assert_eq!(input.transcript_path, "/tmp/test.jsonl");
        assert_eq!(input.hook_event_name.as_deref(), Some("Stop"));
    }

    #[test]
    fn test_hook_input_precompact_payload() {
        let json = r#"{"session_id":"def-456","transcript_path":"/home/user/.claude/projects/foo/def-456.jsonl","hook_event_name":"PreCompact"}"#;
        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.session_id, "def-456");
        assert_eq!(
            input.transcript_path,
            "/home/user/.claude/projects/foo/def-456.jsonl"
        );
        assert_eq!(input.hook_event_name.as_deref(), Some("PreCompact"));
    }

    #[test]
    fn test_hook_input_extra_fields_ignored() {
        let json = r#"{"session_id":"xyz","transcript_path":"/tmp/x.jsonl","hook_event_name":"Stop","extra_field":"ignored","cwd":"/foo"}"#;
        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.session_id, "xyz");
        assert_eq!(input.transcript_path, "/tmp/x.jsonl");
    }

    #[test]
    fn test_hook_input_missing_event_name() {
        let json = r#"{"session_id":"abc","transcript_path":"/tmp/t.jsonl"}"#;
        let input: HookInput = serde_json::from_str(json).unwrap();
        assert_eq!(input.session_id, "abc");
        assert!(input.hook_event_name.is_none());
    }
}
