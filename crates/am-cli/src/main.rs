mod colors;
mod server;
mod sync;
mod sync_dispatch;

use sync_dispatch::{safe_prefix, truncate_text};

use std::path::PathBuf;

use std::io::Write;

use am_core::{QueryEngine, compose_context, compute_surface, export_json, ingest_text};
use am_store::{BrainStore, Config};
use anyhow::{Context, Result};
use clap::{ColorChoice, Parser, Subcommand, ValueEnum};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rmcp::{ServiceExt, transport::stdio};

#[derive(Parser)]
#[command(
    name = "am",
    about = "Geometric memory for AI agents - persistent recall across sessions",
    long_about = "\
am - Geometric memory for AI agents

Models memory as points on a 3-sphere (S³ manifold) using quaternion positions,
golden-angle phasors, IDF-weighted drift, and Kuramoto phase coupling. Memories
aren't stored in flat text - they're positioned in geometric space where related
concepts naturally cluster through physics-inspired dynamics.

How it works:
  - Words are placed on S³ as quaternion positions within neighborhoods
  - Querying activates matching words and drifts them closer via SLERP
  - Phase coupling synchronizes related concepts across sessions
  - Conscious memories (marked salient) persist globally across projects

As an MCP server (primary mode):
  Claude Code runs `am serve` automatically. The AI calls these tools:
    am_query              Recall context at session start
    am_activate_response  Strengthen connections after responses
    am_salient            Mark insights as conscious memory
    am_buffer             Buffer exchanges, auto-create episodes
    am_ingest             Ingest documents as memory episodes
    am_stats              Memory system diagnostics
    am_export / am_import Portable state backup and restore

As a CLI (for humans):
  Query, ingest, inspect, and manage memories directly.",
    after_help = "Setup with Claude Code:
  claude mcp add am -- npx -y attention-matters serve

Quick start:
  am ingest README.md              # Feed a document into memory
  am query \"authentication flow\"   # Recall relevant context
  am inspect                       # See what's in memory
  am inspect conscious             # Browse conscious memories
  am stats                         # System diagnostics

Data location:  ~/.attention-matters/brain.db
  Single unified brain - one product, one memory.

Configuration:  ~/.attention-matters/.am.config.toml
  Environment variables override file values:
    AM_DATA_DIR     Base directory for brain.db and config
    AM_GC_ENABLED   Enable automatic GC on startup (default: false)
    AM_DB_SIZE_MB   DB size limit in MB for GC threshold (default: 50)

https://github.com/srobinson/attention-matters",
    version,
    color = ColorChoice::Auto
)]
pub(crate) struct Cli {
    /// Enable verbose debug output
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start MCP server on stdio transport
    #[command(
        long_about = "Start the MCP (Model Context Protocol) server on stdio transport.\n\n\
            This is the primary mode - Claude Code launches this automatically\n\
            when configured as an MCP server. The server exposes 8 tools that\n\
            the AI agent calls to build and query geometric memory.",
        after_help = "Setup:\n  \
            claude mcp add am -- npx -y attention-matters serve\n\n\
            The server exposes:\n  \
            am_query, am_activate_response, am_salient, am_buffer,\n  \
            am_ingest, am_stats, am_export, am_import"
    )]
    Serve,

    /// Query the memory system and show recall
    #[command(
        long_about = "Query the geometric memory system.\n\n\
            Activates matching words on the S³ manifold, drifts related\n\
            concepts closer via IDF-weighted SLERP, computes phasor\n\
            interference, and returns composed context split into:\n\
            • Conscious recall (previously marked salient)\n\
            • Subconscious recall (from ingested documents/conversations)\n\
            • Novel connections (lateral associations via interference)",
        after_help = "Examples:\n  \
            am query \"authentication middleware\"\n  \
            am query \"database schema migration\" --verbose"
    )]
    Query {
        /// Text to query (natural language)
        text: String,
    },

    /// Ingest documents into geometric memory
    #[command(
        long_about = "Ingest document files as memory episodes.\n\n\
            Text is split into 3-sentence chunks, each becoming a\n\
            neighborhood of word occurrences placed on the S³ manifold\n\
            with golden-angle phasor spacing. Supports .txt, .md, .html.",
        after_help = "Examples:\n  \
            am ingest README.md ARCHITECTURE.md\n  \
            am ingest --dir ./docs\n  \
            am ingest --dir ./docs notes.txt"
    )]
    Ingest {
        /// File path(s) to ingest
        #[arg(required_unless_present = "dir")]
        files: Vec<PathBuf>,

        /// Ingest .txt/.md/.html files from this directory
        #[arg(long)]
        dir: Option<PathBuf>,
    },

    /// Show memory system statistics
    #[command(
        long_about = "Display memory statistics.\n\n\
            Shows total occurrences (N), episode count, conscious memory\n\
            count, database size, and activation distribution.",
        after_help = "Example:\n  \
            am stats"
    )]
    Stats,

    /// Export memory state to portable JSON
    #[command(
        long_about = "Export the full memory state as v0.7.2-compatible JSON.\n\n\
            The exported file contains all episodes, neighborhoods,\n\
            occurrences, and conscious memories. Can be imported on\n\
            another machine or into a different project.",
        after_help = "Example:\n  \
            am export backup.json"
    )]
    Export {
        /// Output file path
        path: PathBuf,
    },

    /// Import memory state from JSON
    #[command(
        long_about = "Import a previously exported memory state.\n\n\
            Replaces the current memory with the imported state.\n\
            All memories are stored in the unified brain database.",
        after_help = "Example:\n  \
            am import backup.json"
    )]
    Import {
        /// Input file path
        path: PathBuf,
    },

    /// Browse memories, episodes, and neighborhoods
    #[command(
        long_about = "Inspect the contents of geometric memory.\n\n\
            Five modes let you see exactly what's stored:\n\
            • overview (default) - summary with top words and recent episodes\n\
            • conscious - list all conscious (salient) memories\n\
            • episodes - list subconscious episodes with stats\n\
            • neighborhoods - all neighborhoods ranked by activation\n\
            • --query - run a query and show the full recall breakdown\n\n\
            Trust requires transparency. This command shows you\n\
            what the AI remembers and why.",
        after_help = "Examples:\n  \
            am inspect                        # Overview\n  \
            am inspect conscious              # List conscious memories\n  \
            am inspect episodes --limit 50    # More episodes\n  \
            am inspect neighborhoods --json   # Machine-readable\n  \
            am inspect --query \"auth flow\"    # Query with full breakdown"
    )]
    Inspect {
        /// What to inspect
        #[arg(value_enum, default_value_t = InspectMode::Overview)]
        mode: InspectMode,

        /// Run a query and show full recall breakdown
        #[arg(long, short)]
        query: Option<String>,

        /// Maximum items to display
        #[arg(long, default_value_t = 20)]
        limit: usize,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Ingest Claude Code session transcripts into memory
    #[command(
        long_about = "Sync Claude Code session transcripts into geometric memory.\n\n\
            Two modes:\n\
            1. Stdin (hook-triggered): reads transcript_path + session_id from\n\
               JSON on stdin and ingests that single session. Used by Claude Code\n\
               PreCompact/Stop hooks.\n\
            2. Discovery (--all): walks the filesystem to discover and re-ingest\n\
               all session transcripts. For manual bulk re-sync.\n\n\
            Replace semantics: if an episode with the same name already exists,\n\
            it is replaced (not duplicated).",
        after_help = "Examples:\n  \
            echo '{...}' | am sync     # Ingest single session from hook stdin\n  \
            am sync --all              # Discover and re-ingest all transcripts\n  \
            am sync --all --dry-run    # Show what would be ingested\n  \
            am sync --all --dir ~/.claude  # Custom Claude config directory"
    )]
    Sync {
        /// Discover and ingest all transcripts via filesystem walk
        #[arg(long)]
        all: bool,

        /// Show what would be ingested without actually ingesting
        #[arg(long)]
        dry_run: bool,

        /// Override Claude config directory (default: ~/.claude or CLAUDE_CONFIG_DIR)
        #[arg(long)]
        dir: Option<PathBuf>,
    },

    /// Garbage collect: prune cold occurrences and compact storage
    #[command(
        long_about = "Run garbage collection on the memory database.\n\n\
            Removes low-activation occurrences (below the activation floor),\n\
            cleans up empty neighborhoods and episodes, then VACUUMs the\n\
            SQLite database to reclaim disk space.\n\n\
            Conscious memories are never auto-evicted.",
        after_help = "Examples:\n  \
            am gc                     # Default: floor=1 (remove zero-activation)\n  \
            am gc --floor 2           # Remove occurrences activated ≤2 times\n  \
            am gc --dry-run           # Preview what would be removed\n  \
            am gc --target-mb 10      # Shrink DB to ~10 MB"
    )]
    Gc {
        /// Activation floor: remove occurrences with count ≤ this value
        #[arg(long, default_value_t = 1)]
        floor: u32,

        /// Target database size in MB (aggressive mode if floor pass isn't enough)
        #[arg(long)]
        target_mb: Option<u64>,

        /// Show what would be cleaned without doing it
        #[arg(long)]
        dry_run: bool,
    },

    /// Selectively forget memories by term, episode, or conscious ID
    #[command(
        long_about = "Remove specific memories from the database.\n\n\
            Three modes:\n\
            • By term: removes all occurrences of a word across all episodes\n\
            • By episode: removes an entire subconscious episode by UUID\n\
            • By conscious ID: removes a specific conscious memory by UUID\n\n\
            Use `am inspect` to find IDs before forgetting.",
        after_help = "Examples:\n  \
            am forget password            # Remove all occurrences of \"password\"\n  \
            am forget --episode abc123    # Remove episode by ID\n  \
            am forget --conscious def456  # Remove conscious memory by ID"
    )]
    Forget {
        /// Word/term to forget (removes all occurrences)
        term: Option<String>,

        /// Episode UUID to remove entirely
        #[arg(long, conflicts_with = "term", conflicts_with = "conscious")]
        episode: Option<String>,

        /// Conscious memory (neighborhood) UUID to remove
        #[arg(long, conflicts_with = "term", conflicts_with = "episode")]
        conscious: Option<String>,
    },

    /// Generate a default .am.config.toml
    #[command(
        long_about = "Generate a fully commented .am.config.toml with all fields\n\
            and their compiled defaults. Writes to the current directory\n\
            by default, or to ~/.attention-matters/ with --global.\n\
            If a config file already exists, prompts before overwriting.",
        after_help = "Examples:\n  \
            am init                 # Write config to current directory\n  \
            am init --global        # Write config to ~/.attention-matters/\n  \
            am init --force         # Overwrite without prompting"
    )]
    Init {
        /// Write to ~/.attention-matters/ instead of the current directory
        #[arg(long)]
        global: bool,

        /// Overwrite existing config without prompting
        #[arg(long)]
        force: bool,
    },
}

#[derive(Clone, ValueEnum)]
enum InspectMode {
    /// Summary with top words and recent episodes
    Overview,
    /// List all conscious (salient) memories
    Conscious,
    /// List subconscious episodes with stats
    Episodes,
    /// All neighborhoods ranked by activation
    Neighborhoods,
}

pub(crate) fn load_config() -> Config {
    am_store::config::load()
}

pub(crate) fn open_store(_cli: &Cli) -> Result<BrainStore> {
    let config = load_config();
    BrainStore::open(&config).context("failed to open brain store")
}

fn init_tracing(verbose: bool) {
    use tracing_subscriber::EnvFilter;

    let filter = if verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::from_default_env().add_directive(tracing::Level::WARN.into())
    };

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();
}

// Tokio multi-thread runtime is used for I/O concurrency (async stdin/stdout
// for the MCP stdio transport), not for parallel tool execution. All tool
// handlers serialize through ServerState's single Mutex. See server.rs for
// the concurrency model documentation.
#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match &cli.command {
        Commands::Serve => cmd_serve(&cli).await,
        Commands::Query { text } => cmd_query(&cli, text),
        Commands::Ingest { files, dir } => cmd_ingest(&cli, files, dir.as_deref()),
        Commands::Stats => cmd_stats(&cli),
        Commands::Export { path } => cmd_export(&cli, path),
        Commands::Import { path } => cmd_import(&cli, path),
        Commands::Inspect {
            mode,
            query,
            limit,
            json,
        } => cmd_inspect(&cli, mode, query.as_deref(), *limit, *json),
        Commands::Sync { all, dry_run, dir } => {
            sync_dispatch::cmd_sync(&cli, *all, *dry_run, dir.as_deref())
        }
        Commands::Gc {
            floor,
            target_mb,
            dry_run,
        } => cmd_gc(&cli, *floor, *target_mb, *dry_run),
        Commands::Forget {
            term,
            episode,
            conscious,
        } => cmd_forget(
            &cli,
            term.as_deref(),
            episode.as_deref(),
            conscious.as_deref(),
        ),
        Commands::Init { global, force } => cmd_init(*global, *force),
    }
}

// ---------------------------------------------------------------------------
// Advisory pidfile for observability
// ---------------------------------------------------------------------------

fn pidfile_path() -> PathBuf {
    let base = std::env::var("AM_DATA_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(am_store::default_base_dir);
    base.join("am-serve.pid")
}

/// Check for an existing pidfile and log accordingly, then write our own.
fn acquire_pidfile() -> Option<PathBuf> {
    let path = pidfile_path();
    if let Ok(content) = std::fs::read_to_string(&path)
        && let Ok(pid) = content.trim().parse::<u32>()
    {
        if is_process_alive(pid) {
            tracing::warn!(
                "another am serve (PID {pid}) is running - coexisting with busy_timeout"
            );
        } else {
            tracing::info!("cleaned up stale pidfile (PID {pid} is dead)");
            let _ = std::fs::remove_file(&path);
        }
    }

    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    match std::fs::File::create(&path) {
        Ok(mut f) => {
            let _ = write!(f, "{}", std::process::id());
            tracing::info!("wrote pidfile: {}", path.display());
            Some(path)
        }
        Err(e) => {
            tracing::warn!("failed to write pidfile: {e}");
            None
        }
    }
}

fn release_pidfile(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    tracing::info!("removed pidfile: {}", path.display());
}

#[cfg(unix)]
fn is_process_alive(pid: u32) -> bool {
    // kill(pid, 0) checks existence without sending a signal
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn is_process_alive(_pid: u32) -> bool {
    false // conservative: assume dead on non-unix
}

async fn cmd_serve(cli: &Cli) -> Result<()> {
    let store = open_store(cli)?;
    tracing::info!("starting MCP server");

    let pidfile = acquire_pidfile();

    let server = server::AmServer::new(store).map_err(|e| anyhow::anyhow!("{e}"))?;
    let server_handle = server.clone(); // Arc clone - cheap; used for shutdown checkpoint
    let service = match server.serve(stdio()).await {
        Ok(s) => s,
        Err(e) => {
            // stdin closed before MCP init completed - treat as clean shutdown
            tracing::info!("MCP server exited during init: {e}");
            if let Some(path) = pidfile {
                release_pidfile(&path);
            }
            return Ok(());
        }
    };

    // Race stdin EOF against OS signals - whichever fires first triggers shutdown
    let shutdown_reason = tokio::select! {
        result = service.waiting() => {
            if let Err(e) = result {
                tracing::warn!("MCP server error: {e}");
            }
            "stdin EOF"
        }
        _ = shutdown_signal() => {
            "signal"
        }
    };
    tracing::info!("shutdown triggered by {shutdown_reason}");

    // Clean shutdown with 5s timeout - an orphan is worse than a dirty exit
    let pidfile_clone = pidfile.clone();
    let clean = tokio::time::timeout(std::time::Duration::from_secs(5), async move {
        // Explicit WAL checkpoint via the server's store (belt + suspenders with Drop)
        server_handle.checkpoint_wal().await;
        // Pidfile cleanup
        if let Some(path) = pidfile_clone {
            release_pidfile(&path);
        }
    })
    .await;

    if clean.is_err() {
        eprintln!("[am] shutdown timeout, forcing exit");
        // Still try to remove pidfile even on timeout
        if let Some(path) = pidfile {
            release_pidfile(&path);
        }
        std::process::exit(1);
    }

    Ok(())
}

/// Wait for SIGTERM, SIGINT, or SIGHUP (Unix) / ctrl_c (all platforms)
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).expect("SIGTERM handler");
        let mut sighup = signal(SignalKind::hangup()).expect("SIGHUP handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("received SIGINT");
            }
            _ = sigterm.recv() => {
                tracing::info!("received SIGTERM");
            }
            _ = sighup.recv() => {
                tracing::info!("received SIGHUP");
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.expect("ctrl_c handler");
        tracing::info!("received ctrl_c");
    }
}

fn cmd_query(cli: &Cli, text: &str) -> Result<()> {
    let store = open_store(cli)?;
    let mut system = store.load_system().context("failed to load system")?;

    let query_result = QueryEngine::process_query(&mut system, text);
    let surface = compute_surface(&system, &query_result);
    let composed = compose_context(
        &mut system,
        &surface,
        &query_result,
        &query_result.interference,
        None,
    );

    if composed.context.is_empty() {
        println!("(no memories found)");
    } else {
        println!("{}", composed.context);
    }

    if cli.verbose {
        eprintln!(
            "--- metrics: conscious={}, subconscious={}, novel={} ---",
            composed.metrics.conscious, composed.metrics.subconscious, composed.metrics.novel
        );
        eprintln!(
            "--- stats: N={}, episodes={}, conscious={} ---",
            system.n(),
            system.episodes.len(),
            system.conscious_episode.neighborhoods.len()
        );
    }

    Ok(())
}

fn cmd_ingest(cli: &Cli, files: &[PathBuf], dir: Option<&std::path::Path>) -> Result<()> {
    let store = open_store(cli)?;
    let mut system = store.load_system().context("failed to load system")?;
    let mut rng = SmallRng::from_os_rng();

    let mut paths: Vec<PathBuf> = files.to_vec();

    if let Some(dir) = dir {
        let entries = std::fs::read_dir(dir)
            .with_context(|| format!("failed to read dir {}", dir.display()))?;
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && let Some(ext) = p.extension().and_then(|e| e.to_str())
                && matches!(ext, "txt" | "md" | "html")
            {
                paths.push(p);
            }
        }
    }

    // Deduplicate by canonical path so files listed both as positional args
    // and found via --dir scan are only ingested once.
    let mut seen = std::collections::HashSet::new();
    paths.retain(|p| {
        let key = p.canonicalize().unwrap_or_else(|_| p.clone());
        seen.insert(key)
    });

    for path in &paths {
        let content = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read {}", path.display()))?;
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("unnamed");
        let episode = ingest_text(&content, Some(name), &mut rng);
        let nbhd_count = episode.neighborhoods.len();
        let occ_count: usize = episode
            .neighborhoods
            .iter()
            .map(|n| n.occurrences.len())
            .sum();
        system.add_episode(episode);
        println!(
            "ingested {} → {} neighborhoods, {} occurrences",
            path.display(),
            nbhd_count,
            occ_count
        );
    }

    store
        .save_system(&system)
        .context("failed to save system")?;

    println!("done. N={}, episodes={}", system.n(), system.episodes.len());
    Ok(())
}

fn cmd_stats(cli: &Cli) -> Result<()> {
    let store = open_store(cli)?;
    let system = store.load_system().context("failed to load system")?;

    let db_size = store.store().db_size();
    let activation = store
        .store()
        .activation_distribution()
        .context("failed to get activation stats")?;

    println!("N:          {}", system.n());
    println!("episodes:   {}", system.episodes.len());
    println!(
        "conscious:  {}",
        system.conscious_episode.neighborhoods.len()
    );
    println!("db_size:    {:.1}MB", db_size as f64 / (1024.0 * 1024.0));
    println!(
        "activation: mean={:.2}, max={}, zero={}/{}",
        activation.mean_activation,
        activation.max_activation,
        activation.zero_activation,
        activation.total,
    );
    Ok(())
}

fn cmd_inspect(
    cli: &Cli,
    mode: &InspectMode,
    query: Option<&str>,
    limit: usize,
    json: bool,
) -> Result<()> {
    // --query flag overrides mode
    if let Some(text) = query {
        return cmd_inspect_query(cli, text);
    }

    let store = open_store(cli)?;

    match mode {
        InspectMode::Overview => inspect_overview(&store, limit, json),
        InspectMode::Conscious => inspect_conscious(&store, limit, json),
        InspectMode::Episodes => inspect_episodes(&store, limit, json),
        InspectMode::Neighborhoods => inspect_neighborhoods(&store, limit, json),
    }
}

fn inspect_overview(store: &BrainStore, limit: usize, json: bool) -> Result<()> {
    let episodes = store
        .store()
        .list_episodes()
        .context("failed to list episodes")?;
    let activation = store
        .store()
        .activation_distribution()
        .context("failed to get activation stats")?;
    let db_size = store.store().db_size();
    let unique_words = store
        .store()
        .unique_word_count()
        .context("failed to count words")?;
    let top_words = store
        .store()
        .top_words(limit)
        .context("failed to get top words")?;
    let conscious = store
        .store()
        .list_conscious_neighborhoods()
        .context("failed to list conscious")?;

    let sub_episodes: Vec<_> = episodes.iter().filter(|e| !e.is_conscious).collect();

    if json {
        let top_words_json: Vec<serde_json::Value> = top_words
            .iter()
            .map(|(word, act, count)| {
                serde_json::json!({"word": word, "activation": act, "occurrences": count})
            })
            .collect();
        let conscious_json: Vec<serde_json::Value> = conscious
            .iter()
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "text": truncate_text(&n.source_text, 200),
                    "occurrences": n.occurrence_count,
                    "activation": n.total_activation,
                })
            })
            .collect();

        let out = serde_json::json!({
            "total_occurrences": activation.total,
            "unique_words": unique_words,
            "episodes": sub_episodes.len(),
            "conscious_memories": conscious.len(),
            "db_size_bytes": db_size,
            "activation": {
                "mean": activation.mean_activation,
                "max": activation.max_activation,
                "zero_count": activation.zero_activation,
            },
            "top_words": top_words_json,
            "conscious": conscious_json,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap());
        return Ok(());
    }

    let colors::Colors {
        bold,
        dim,
        reset,
        cyan,
        ..
    } = colors::Colors::stdout();

    println!("{bold}MEMORY OVERVIEW{reset}");
    println!("{dim}───────────────────────────────{reset}");
    println!(
        "  occurrences:  {bold}{}{reset} {dim}({} unique words){reset}",
        activation.total, unique_words
    );
    println!("  episodes:     {bold}{}{reset}", sub_episodes.len());
    println!("  conscious:    {bold}{}{reset}", conscious.len());
    println!(
        "  db size:      {bold}{:.1}MB{reset}",
        db_size as f64 / (1024.0 * 1024.0)
    );
    println!(
        "  activation:   mean={:.2}, max={}, zero={}/{}",
        activation.mean_activation,
        activation.max_activation,
        activation.zero_activation,
        activation.total
    );

    if !conscious.is_empty() {
        println!();
        println!(
            "{bold}CONSCIOUS MEMORIES{reset} {dim}({}){reset}",
            conscious.len()
        );
        println!("{dim}───────────────────────────────{reset}");
        for (i, nbhd) in conscious.iter().take(5).enumerate() {
            let text = truncate_text(&nbhd.source_text, 80);
            println!("  {cyan}{}. {reset}{text}", i + 1);
        }
        if conscious.len() > 5 {
            println!(
                "  {dim}... and {} more (use `am inspect conscious`){reset}",
                conscious.len() - 5
            );
        }
    }

    if !top_words.is_empty() {
        println!();
        println!("{bold}TOP WORDS{reset} {dim}(by activation){reset}");
        println!("{dim}───────────────────────────────{reset}");
        for (word, act, count) in top_words.iter().take(10) {
            println!("  {cyan}{:<20}{reset} act={:<5} ×{}", word, act, count);
        }
    }

    if !sub_episodes.is_empty() {
        println!();
        println!(
            "{bold}RECENT EPISODES{reset} {dim}({}){reset}",
            sub_episodes.len()
        );
        println!("{dim}───────────────────────────────{reset}");
        for (i, ep) in sub_episodes.iter().take(5).enumerate() {
            let name = if ep.name.is_empty() {
                "(unnamed)"
            } else {
                &ep.name
            };
            println!(
                "  {cyan}{}. {reset}{name} {dim}- {} neighborhoods, {} occurrences{reset}",
                i + 1,
                ep.neighborhood_count,
                ep.occurrence_count
            );
        }
        if sub_episodes.len() > 5 {
            println!(
                "  {dim}... and {} more (use `am inspect episodes`){reset}",
                sub_episodes.len() - 5
            );
        }
    }

    Ok(())
}

fn inspect_conscious(store: &BrainStore, limit: usize, json: bool) -> Result<()> {
    let conscious = store
        .store()
        .list_conscious_neighborhoods()
        .context("failed to list conscious memories")?;

    if json {
        let items: Vec<serde_json::Value> = conscious
            .iter()
            .take(limit)
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "text": n.source_text,
                    "occurrences": n.occurrence_count,
                    "activation": n.total_activation,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items).unwrap());
        return Ok(());
    }

    let colors::Colors {
        bold, dim, reset, ..
    } = colors::Colors::stdout();

    println!(
        "{bold}CONSCIOUS MEMORIES{reset} {dim}({}){reset}",
        conscious.len()
    );
    println!("{dim}───────────────────────────────{reset}");

    if conscious.is_empty() {
        println!("  (no conscious memories)");
        println!();
        println!("  {dim}Use am_salient to mark important insights.{reset}");
        return Ok(());
    }

    for (i, nbhd) in conscious.iter().take(limit).enumerate() {
        let text = if nbhd.source_text.is_empty() {
            "(no source text)".to_string()
        } else {
            nbhd.source_text.clone()
        };
        println!("  {bold}{}. {reset}{text}", i + 1);
        println!(
            "     {dim}id={} · {} words · activation={}{reset}",
            safe_prefix(&nbhd.id, 8),
            nbhd.occurrence_count,
            nbhd.total_activation
        );
    }

    if conscious.len() > limit {
        println!(
            "\n  {dim}Showing {limit} of {} (use --limit to see more){reset}",
            conscious.len()
        );
    }

    Ok(())
}

fn inspect_episodes(store: &BrainStore, limit: usize, json: bool) -> Result<()> {
    let episodes = store
        .store()
        .list_episodes()
        .context("failed to list episodes")?;

    let sub_episodes: Vec<_> = episodes.iter().filter(|e| !e.is_conscious).collect();

    if json {
        let items: Vec<serde_json::Value> = sub_episodes
            .iter()
            .take(limit)
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "name": e.name,
                    "timestamp": e.timestamp,
                    "neighborhoods": e.neighborhood_count,
                    "occurrences": e.occurrence_count,
                    "activation": e.total_activation,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items).unwrap());
        return Ok(());
    }

    let colors::Colors {
        bold,
        dim,
        reset,
        cyan,
        ..
    } = colors::Colors::stdout();

    println!("{bold}EPISODES{reset} {dim}({}){reset}", sub_episodes.len());
    println!("{dim}───────────────────────────────{reset}");

    if sub_episodes.is_empty() {
        println!("  (no episodes)");
        println!();
        println!(
            "  {dim}Episodes are created by am_buffer (after 3 exchanges) or am ingest.{reset}"
        );
        return Ok(());
    }

    for (i, ep) in sub_episodes.iter().take(limit).enumerate() {
        let name = if ep.name.is_empty() {
            "(unnamed)"
        } else {
            &ep.name
        };
        let ts = if ep.timestamp.is_empty() {
            ""
        } else {
            &ep.timestamp
        };
        println!("{cyan}  {}. {reset}{bold}{name}{reset}", i + 1);
        println!(
            "     {dim}{} neighborhoods · {} occurrences · activation={} {ts}{reset}",
            ep.neighborhood_count, ep.occurrence_count, ep.total_activation,
        );
    }

    if sub_episodes.len() > limit {
        println!(
            "\n  {dim}Showing {limit} of {} (use --limit to see more){reset}",
            sub_episodes.len()
        );
    }

    Ok(())
}

fn inspect_neighborhoods(store: &BrainStore, limit: usize, json: bool) -> Result<()> {
    let neighborhoods = store
        .store()
        .list_neighborhoods()
        .context("failed to list neighborhoods")?;

    if json {
        let items: Vec<serde_json::Value> = neighborhoods
            .iter()
            .take(limit)
            .map(|n| {
                serde_json::json!({
                    "id": n.id,
                    "source_text": n.source_text,
                    "episode": n.episode_name,
                    "is_conscious": n.is_conscious,
                    "occurrences": n.occurrence_count,
                    "total_activation": n.total_activation,
                    "max_activation": n.max_activation,
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&items).unwrap());
        return Ok(());
    }

    let colors::Colors {
        bold,
        dim,
        reset,
        cyan,
        yellow,
    } = colors::Colors::stdout();

    println!(
        "{bold}NEIGHBORHOODS{reset} {dim}({} total, by activation){reset}",
        neighborhoods.len()
    );
    println!("{dim}───────────────────────────────{reset}");

    if neighborhoods.is_empty() {
        println!("  (no neighborhoods)");
        return Ok(());
    }

    for (i, nbhd) in neighborhoods.iter().take(limit).enumerate() {
        let tag = if nbhd.is_conscious {
            format!("{yellow}[conscious]{reset}")
        } else {
            format!("{dim}[{}]{reset}", nbhd.episode_name)
        };
        let text = truncate_text(&nbhd.source_text, 70);
        println!("  {cyan}{}. {reset}{text} {tag}", i + 1);
        println!(
            "     {dim}{} words · activation: total={} max={}{reset}",
            nbhd.occurrence_count, nbhd.total_activation, nbhd.max_activation,
        );
    }

    if neighborhoods.len() > limit {
        println!(
            "\n  {dim}Showing {limit} of {} (use --limit to see more){reset}",
            neighborhoods.len()
        );
    }

    Ok(())
}

fn cmd_inspect_query(cli: &Cli, text: &str) -> Result<()> {
    let store = open_store(cli)?;
    let mut system = store.load_system().context("failed to load system")?;

    let query_result = QueryEngine::process_query(&mut system, text);
    let surface = compute_surface(&system, &query_result);
    let composed = compose_context(
        &mut system,
        &surface,
        &query_result,
        &query_result.interference,
        None,
    );

    let colors::Colors {
        bold, dim, reset, ..
    } = colors::Colors::stdout();

    println!("{bold}RECALL{reset} for {dim}\"{text}\"{reset}");
    println!("{dim}───────────────────────────────{reset}");

    if composed.context.is_empty() {
        println!("  (no memories match this query)");
    } else {
        for line in composed.context.lines() {
            println!("  {line}");
        }
    }

    println!();
    println!(
        "{dim}metrics: conscious={}, subconscious={}, novel={}{reset}",
        composed.metrics.conscious, composed.metrics.subconscious, composed.metrics.novel
    );
    println!(
        "{dim}system:  N={}, episodes={}, conscious={}{reset}",
        system.n(),
        system.episodes.len(),
        system.conscious_episode.neighborhoods.len()
    );

    Ok(())
}

fn cmd_gc(cli: &Cli, floor: u32, target_mb: Option<u64>, dry_run: bool) -> Result<()> {
    let store = open_store(cli)?;
    let db = store.store();
    let colors::Colors {
        bold, dim, reset, ..
    } = colors::Colors::stdout();

    let stats = db
        .activation_distribution()
        .context("failed to read stats")?;
    let db_size = db.db_size();

    if dry_run {
        // Show what would happen
        let eligible: u64 = db
            .gc_eligible_count(floor)
            .context("failed to query eligible occurrences")?;

        println!("{bold}GC dry run{reset}\n");
        println!("  total occurrences:   {}", stats.total);
        println!("  activation floor:    ≤{floor}");
        println!("  eligible for eviction: {eligible}");
        println!("  database size:       {:.1} KB", db_size as f64 / 1024.0);
        if let Some(mb) = target_mb {
            println!("  target size:         {mb} MB");
        }
        println!("\n{dim}No changes made. Remove --dry-run to execute.{reset}");
        return Ok(());
    }

    // Run activation-floor GC pass
    let config = load_config();
    let result = db.gc_pass(floor, &config.retention).context("GC failed")?;

    println!("{bold}GC complete{reset}\n");
    println!("  evicted occurrences:    {}", result.evicted_occurrences);
    println!("  removed neighborhoods:  {}", result.removed_neighborhoods);
    println!("  removed episodes:       {}", result.removed_episodes);

    // If target_mb specified and still over budget, run aggressive pass
    if let Some(mb) = target_mb {
        let target_bytes = mb * 1024 * 1024;
        let current_size = db.db_size();
        if current_size > target_bytes {
            let aggressive = db
                .gc_to_target_size(target_bytes, &config.retention)
                .context("aggressive GC failed")?;
            println!(
                "\n  {bold}aggressive pass:{reset} evicted {} more occurrences",
                aggressive.evicted_occurrences
            );
        }
    }

    let after_size = db.db_size();
    println!(
        "\n  size: {:.1} KB → {:.1} KB",
        result.before_size as f64 / 1024.0,
        after_size as f64 / 1024.0,
    );

    Ok(())
}

fn cmd_forget(
    cli: &Cli,
    term: Option<&str>,
    episode_id: Option<&str>,
    conscious_id: Option<&str>,
) -> Result<()> {
    let store = open_store(cli)?;
    let db = store.store();
    let colors::Colors { bold, reset, .. } = colors::Colors::stdout();

    if let Some(id) = episode_id {
        let removed = db.forget_episode(id).context("failed to forget episode")?;
        if removed == 0 {
            println!("Episode not found: {id}");
        } else {
            println!("{bold}Forgot{reset} episode {id} ({removed} occurrences removed)");
        }
    } else if let Some(id) = conscious_id {
        let removed = db
            .forget_conscious(id)
            .context("failed to forget conscious memory")?;
        if removed == 0 {
            println!("Conscious memory not found: {id}");
        } else {
            println!("{bold}Forgot{reset} conscious memory {id} ({removed} occurrences removed)");
        }
    } else if let Some(word) = term {
        let (removed_occs, removed_nbhds, removed_eps) =
            db.forget_term(word).context("failed to forget term")?;
        if removed_occs == 0 {
            println!("No occurrences of \"{word}\" found.");
        } else {
            println!(
                "{bold}Forgot{reset} \"{word}\": {removed_occs} occurrences, \
                 {removed_nbhds} neighborhoods, {removed_eps} episodes removed"
            );
        }
    } else {
        anyhow::bail!("specify a term, --episode <id>, or --conscious <id> to forget");
    }

    Ok(())
}

fn cmd_init(global: bool, force: bool) -> Result<()> {
    let dir = if global {
        am_store::project::default_base_dir()
    } else {
        std::env::current_dir().context("failed to get current directory")?
    };
    let config_path = dir.join(".am.config.toml");

    // Ensure the target directory exists (relevant for --global)
    std::fs::create_dir_all(&dir).with_context(|| format!("failed to create {}", dir.display()))?;

    if config_path.exists() && !force {
        eprint!(
            "{} already exists. Overwrite? [y/N] ",
            config_path.display()
        );
        std::io::stderr().flush()?;
        let mut answer = String::new();
        std::io::stdin().read_line(&mut answer)?;
        if !answer.trim().eq_ignore_ascii_case("y") {
            println!("aborted");
            return Ok(());
        }
    }

    let content = am_store::config::generate_default_toml();
    std::fs::write(&config_path, &content)
        .with_context(|| format!("failed to write {}", config_path.display()))?;

    println!("wrote {}", config_path.display());
    Ok(())
}

fn cmd_export(cli: &Cli, path: &std::path::Path) -> Result<()> {
    if path.extension().is_none_or(|ext| ext != "json") {
        anyhow::bail!("export path must end in .json (got {})", path.display());
    }
    let store = open_store(cli)?;
    let system = store.load_system().context("failed to load system")?;

    let json = export_json(&system).context("failed to serialize state")?;
    std::fs::write(path, &json).with_context(|| format!("failed to write {}", path.display()))?;

    println!("exported to {}", path.display());
    Ok(())
}

fn cmd_import(cli: &Cli, path: &std::path::Path) -> Result<()> {
    let store = open_store(cli)?;
    store
        .import_json_file(path)
        .context("failed to import JSON")?;

    let system = store
        .load_system()
        .context("failed to load system after import")?;

    println!(
        "imported from {}. N={}, episodes={}, conscious={}",
        path.display(),
        system.n(),
        system.episodes.len(),
        system.conscious_episode.neighborhoods.len()
    );
    Ok(())
}
