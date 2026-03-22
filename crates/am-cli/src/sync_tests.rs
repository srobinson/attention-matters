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
            main_assistant_text("The auth middleware uses JWT tokens stored in HTTP-only cookies."),
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
            main_assistant_text("The auth middleware uses JWT tokens stored in HTTP-only cookies."),
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
            main_assistant_text("SLERP produces constant-speed rotation between two quaternions."),
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
            main_user("# Orchestrator\\nYou are supervising a worker agent that executes tasks."),
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
            main_assistant_text("SLERP produces constant-speed rotation between two quaternions."),
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
    let json =
        r#"{"session_id":"abc-123","transcript_path":"/tmp/test.jsonl","hook_event_name":"Stop"}"#;
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
