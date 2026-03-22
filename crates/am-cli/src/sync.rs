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
#[path = "sync_tests.rs"]
mod tests;
