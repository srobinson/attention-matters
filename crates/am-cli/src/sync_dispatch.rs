use std::fmt::Write as _;

use am_core::ingest_text;
use anyhow::{Context, Result};
use rand::SeedableRng;
use rand::rngs::SmallRng;

use crate::sync;
use crate::{Cli, load_config, open_store};

/// Safe prefix slice - returns `&s[..n]` if ASCII-safe, otherwise
/// falls back to char iteration to avoid panicking on UTF-8 boundaries.
pub(crate) fn safe_prefix(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else if s.is_char_boundary(n) {
        &s[..n]
    } else {
        // Fallback: find the last valid char boundary at or before n
        let end = (0..=n).rev().find(|&i| s.is_char_boundary(i)).unwrap_or(0);
        &s[..end]
    }
}

pub(crate) fn truncate_text(text: &str, max_len: usize) -> String {
    // Collapse whitespace and truncate by char count (not bytes) to avoid
    // panicking on multi-byte UTF-8 boundaries
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= max_len {
        collapsed
    } else {
        let truncated: String = collapsed.chars().take(max_len.saturating_sub(3)).collect();
        format!("{truncated}...")
    }
}

pub(crate) fn cmd_sync(
    cli: &Cli,
    all: bool,
    dry_run: bool,
    dir_override: Option<&std::path::Path>,
) -> Result<()> {
    let hook_input = sync::read_hook_input();

    if let Some(hook) = hook_input
        && !all
    {
        // Stdin mode: hook-triggered single-session ingest
        return cmd_sync_single(cli, hook, dry_run);
    }

    if all {
        // Discovery mode: bulk re-ingest via filesystem walk
        cmd_sync_discover(cli, dry_run, dir_override)
    } else {
        // Interactive terminal, no --all flag - print usage hint
        println!("Usage: pipe hook JSON on stdin, or use --all for bulk discovery.\n");
        println!("  echo '{{\"session_id\":\"...\",\"transcript_path\":\"...\"}}' | am sync");
        println!("  am sync --all");
        println!("  am sync --all --dry-run");
        Ok(())
    }
}

/// Ingest a session transcript as one or more episodes.
///
/// SessionEnd is the canonical episode boundary. The transcript is the sole
/// source of truth. Main-chain content is chunked into episodes of 5 exchanges.
/// Each subagent's work becomes its own episode. Thinking blocks are captured
/// alongside text. Tool interactions are excluded.
fn cmd_sync_single(cli: &Cli, hook: sync::HookInput, dry_run: bool) -> Result<()> {
    let crate::colors::Colors {
        bold, dim, reset, ..
    } = crate::colors::Colors::stdout();

    let session_prefix = safe_prefix(&hook.session_id, 8);

    // Resolve transcript path
    let raw_path = if hook.transcript_path.starts_with("~/") {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        format!("{}{}", home, &hook.transcript_path[1..])
    } else {
        hook.transcript_path.clone()
    };
    let path = std::path::PathBuf::from(&raw_path);

    if !path.exists() {
        eprintln!("Transcript not found: {}", path.display());
        return Ok(());
    }

    let extracted = sync::extract_episodes(&path, session_prefix)
        .with_context(|| format!("failed to parse {}", path.display()))?;

    if extracted.is_empty() {
        println!("  {dim}skip{reset} {session_prefix} (no substantive content)",);
        return Ok(());
    }

    if dry_run {
        for ep in &extracted {
            let preview = truncate_text(&ep.text, 60);
            println!(
                "  {bold}episode{reset} {} ({} chars) {dim}{preview}{reset}",
                ep.name,
                ep.text.len()
            );
        }
        println!(
            "\n{dim}Dry run: {} episode(s), no changes made.{reset}",
            extracted.len()
        );
        return Ok(());
    }

    let store = open_store(cli)?;
    let mut system = store.load_system().context("failed to load system")?;
    let mut rng = SmallRng::from_os_rng();

    // Drain any leftover conversation buffer (from am_buffer calls during
    // this session). The transcript is the canonical source, so we discard
    // the buffer to avoid double-counting.
    let _ = store.store().drain_buffer();

    let mut total_neighborhoods = 0usize;

    for ep in &extracted {
        // Replace semantics: remove existing episode with same name
        system.episodes.retain(|e| e.name != ep.name);

        let episode = ingest_text(&ep.text, Some(&ep.name), &mut rng);
        let nbhd_count = episode.neighborhoods.len();
        total_neighborhoods += nbhd_count;
        system.add_episode(episode);

        let preview = truncate_text(&ep.text, 60);
        println!(
            "  {bold}episode{reset} {} -> {nbhd_count} neighborhoods {dim}{preview}{reset}",
            ep.name,
        );
    }

    store
        .save_system(&system)
        .context("failed to save system")?;

    println!(
        "\n{bold}Done.{reset} {} episode(s), {total_neighborhoods} neighborhoods, N={}, total episodes={}",
        extracted.len(),
        system.n(),
        system.episodes.len()
    );

    // Write debug log if sync_log_dir is configured
    let config = load_config();
    if let Some(ref log_dir) = config.sync_log_dir
        && let Err(e) = write_sync_log(log_dir, session_prefix, &extracted)
    {
        eprintln!("sync log failed: {e}");
    }

    Ok(())
}

/// Write sync results to a debug log file.
fn write_sync_log(
    log_dir: &std::path::Path,
    session_prefix: &str,
    episodes: &[sync::ExtractedEpisode],
) -> Result<()> {
    std::fs::create_dir_all(log_dir)
        .with_context(|| format!("failed to create {}", log_dir.display()))?;

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let filename = format!("sync-{session_prefix}-{timestamp}.log");
    let path = log_dir.join(filename);

    let mut out = String::new();
    writeln!(out, "session: {session_prefix}")?;
    writeln!(out, "timestamp: {timestamp}")?;
    writeln!(out, "episodes: {}", episodes.len())?;
    writeln!(out)?;

    for ep in episodes {
        writeln!(out, "--- {} ({} chars) ---", ep.name, ep.text.len())?;
        writeln!(out, "{}", ep.text)?;
        writeln!(out)?;
    }

    std::fs::write(&path, &out).with_context(|| format!("failed to write {}", path.display()))?;

    Ok(())
}

/// Discover and re-ingest all sessions via filesystem walk.
fn cmd_sync_discover(
    cli: &Cli,
    dry_run: bool,
    dir_override: Option<&std::path::Path>,
) -> Result<()> {
    let claude_dir = sync::resolve_claude_dir(dir_override);
    let project_dir = match sync::find_project_dir(&claude_dir) {
        Some(dir) => dir,
        None => {
            println!(
                "No Claude Code project directory found for current working directory.\n\
                 Searched: {}/projects/",
                claude_dir.display()
            );
            println!(
                "\nTip: Run this from your project root, or use --dir to specify the Claude config directory."
            );
            return Ok(());
        }
    };

    let sessions = sync::discover_sessions(&project_dir).context("failed to discover sessions")?;

    let crate::colors::Colors {
        bold, dim, reset, ..
    } = crate::colors::Colors::stdout();

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!("{bold}Found {}{reset} session(s) to sync\n", sessions.len());

    // Defer store/system loading until we know we need to write. In dry-run
    // mode this avoids creating brain.db as a side effect.
    let mut store_state: Option<(am_store::BrainStore, am_core::DAESystem, SmallRng)> = None;

    let mut total_episodes = 0u32;
    let mut total_text_len = 0usize;

    for session in &sessions {
        let text = match sync::extract_session_text(&session.path) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("  warning: failed to parse {}: {e}", session.path.display());
                continue;
            }
        };

        if text.is_empty() {
            if dry_run {
                println!(
                    "  {dim}skip{reset} {} (no substantive content)",
                    safe_prefix(&session.session_id, 8)
                );
            }
            continue;
        }

        let episode_name = format!("session-{}", safe_prefix(&session.session_id, 8));
        let text_preview = truncate_text(&text, 60);
        total_text_len += text.len();

        if dry_run {
            println!(
                "  {bold}sync{reset} {} ({} chars) {dim}{text_preview}{reset}",
                safe_prefix(&session.session_id, 8),
                text.len()
            );
        } else {
            let (_, system, rng) = match &mut store_state {
                Some(s) => s,
                None => {
                    let store = open_store(cli)?;
                    let system = store.load_system().context("failed to load system")?;
                    let rng = SmallRng::from_os_rng();
                    store_state.insert((store, system, rng))
                }
            };

            // Replace semantics: remove existing episode with same name
            system.episodes.retain(|e| e.name != episode_name);

            let episode = ingest_text(&text, Some(&episode_name), rng);
            let nbhd_count = episode.neighborhoods.len();
            system.add_episode(episode);
            total_episodes += 1;

            println!(
                "  {bold}synced{reset} {} → {} neighborhoods {dim}{text_preview}{reset}",
                safe_prefix(&session.session_id, 8),
                nbhd_count,
            );
        }
    }

    if dry_run {
        println!(
            "\n{dim}Dry run: would ingest ~{} chars from {} sessions.{reset}",
            total_text_len,
            sessions.len()
        );
    } else if let Some((store, system, _)) = &store_state {
        if total_episodes > 0 {
            store.save_system(system).context("failed to save system")?;
        }

        println!(
            "\n{bold}Done.{reset} Ingested {total_episodes} episode(s). N={}, episodes={}",
            system.n(),
            system.episodes.len()
        );
    }

    Ok(())
}
