mod colors;
#[path = "generated_help.rs"]
mod generated_help;
mod jsonrpc;
mod server;
mod sync;
mod sync_dispatch;

use sync_dispatch::{safe_prefix, truncate_text};

use std::path::PathBuf;

use std::io::Write;

use am_core::{
    compose::compose_context, query::QueryEngine, serde_compat::export_json, store_trait::AmStore,
    surface::compute_surface, tokenizer::ingest_text,
};
use am_store::{config::Config, project::BrainStore};
use anyhow::{Context, Result};
use clap::{ColorChoice, Parser, Subcommand, ValueEnum};
use rand::SeedableRng;
use rand::rngs::SmallRng;

#[derive(Parser)]
#[command(
    name = "am",
    about = generated_help::CLI_ABOUT,
    long_about = generated_help::CLI_LONG_ABOUT,
    after_help = generated_help::CLI_AFTER_HELP,
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
    #[command(
        about = generated_help::SERVE_ABOUT,
        long_about = generated_help::SERVE_LONG_ABOUT,
        after_help = generated_help::SERVE_AFTER_HELP,
    )]
    Serve,

    #[command(
        about = generated_help::QUERY_ABOUT,
        long_about = generated_help::QUERY_LONG_ABOUT,
        after_help = generated_help::QUERY_AFTER_HELP,
    )]
    Query {
        #[arg(help = generated_help::QUERY_TEXT_HELP)]
        text: String,
    },

    #[command(
        about = generated_help::INGEST_ABOUT,
        long_about = generated_help::INGEST_LONG_ABOUT,
        after_help = generated_help::INGEST_AFTER_HELP,
    )]
    Ingest {
        /// File path(s) to ingest
        #[arg(required_unless_present = "dir")]
        files: Vec<PathBuf>,

        /// Ingest .txt/.md/.html files from this directory
        #[arg(long)]
        dir: Option<PathBuf>,
    },

    #[command(
        about = generated_help::STATS_ABOUT,
        long_about = generated_help::STATS_LONG_ABOUT,
        after_help = generated_help::STATS_AFTER_HELP,
    )]
    Stats,

    #[command(
        about = generated_help::EXPORT_ABOUT,
        long_about = generated_help::EXPORT_LONG_ABOUT,
        after_help = generated_help::EXPORT_AFTER_HELP,
    )]
    Export {
        /// Output file path
        path: PathBuf,
    },

    #[command(
        about = generated_help::IMPORT_ABOUT,
        long_about = generated_help::IMPORT_LONG_ABOUT,
        after_help = generated_help::IMPORT_AFTER_HELP,
    )]
    Import {
        /// Input file path
        path: PathBuf,
    },

    #[command(
        about = generated_help::INSPECT_ABOUT,
        long_about = generated_help::INSPECT_LONG_ABOUT,
        after_help = generated_help::INSPECT_AFTER_HELP,
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

    #[command(
        about = generated_help::SYNC_ABOUT,
        long_about = generated_help::SYNC_LONG_ABOUT,
        after_help = generated_help::SYNC_AFTER_HELP,
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

    #[command(
        about = generated_help::GC_ABOUT,
        long_about = generated_help::GC_LONG_ABOUT,
        after_help = generated_help::GC_AFTER_HELP,
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

    #[command(
        about = generated_help::FORGET_ABOUT,
        long_about = generated_help::FORGET_LONG_ABOUT,
        after_help = generated_help::FORGET_AFTER_HELP,
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

    #[command(
        about = generated_help::INIT_ABOUT,
        long_about = generated_help::INIT_LONG_ABOUT,
        after_help = generated_help::INIT_AFTER_HELP,
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

pub(crate) fn load_config() -> Result<Config> {
    am_store::config::load().context("invalid configuration")
}

pub(crate) fn open_store(_cli: &Cli) -> Result<BrainStore> {
    let config = load_config()?;
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

fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    match &cli.command {
        Commands::Serve => cmd_serve(&cli),
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

fn pidfile_path() -> Option<PathBuf> {
    let base = std::env::var("AM_DATA_DIR")
        .ok()
        .map(PathBuf::from)
        .or_else(|| am_store::project::default_base_dir().ok())?;
    Some(base.join("am-serve.pid"))
}

/// Check for an existing pidfile and log accordingly, then write our own.
fn acquire_pidfile() -> Option<PathBuf> {
    let path = pidfile_path()?;
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

fn cmd_serve(cli: &Cli) -> Result<()> {
    let store = open_store(cli)?;
    tracing::info!("starting MCP server");

    let pidfile = acquire_pidfile();

    let server = server::AmServer::new(store).map_err(|e| anyhow::anyhow!("{e}"))?;

    // Install signal handlers that close stdin to unblock the stdio loop.
    // Without this, SIGTERM would kill the process before cleanup runs.
    install_signal_handlers();

    // Run the JSON-RPC stdio loop. Blocks until stdin closes or I/O error.
    let result = jsonrpc::run_stdio_loop(|name, args| server.dispatch_tool(name, args));

    // Clean shutdown: WAL checkpoint + pidfile cleanup
    server.checkpoint_wal();
    if let Some(path) = pidfile {
        release_pidfile(&path);
    }

    result
}

/// Install signal handlers that close stdin to unblock the blocking stdio loop.
///
/// On Unix, SIGTERM/SIGHUP/SIGINT close `/dev/stdin` via dup2, causing
/// `BufRead::lines()` to return `None` and the loop to exit cleanly.
fn install_signal_handlers() {
    #[cfg(unix)]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static SIGNALED: AtomicBool = AtomicBool::new(false);

        unsafe extern "C" fn handler(_sig: libc::c_int) {
            unsafe {
                if SIGNALED.swap(true, Ordering::SeqCst) {
                    // Second signal: force exit
                    libc::_exit(1);
                }
                // Close stdin to unblock the blocking read in the stdio loop.
                // This causes `lines()` to yield `None`, ending the loop cleanly.
                libc::close(0);
            }
        }

        unsafe {
            libc::signal(libc::SIGTERM, handler as *const () as libc::sighandler_t);
            libc::signal(libc::SIGHUP, handler as *const () as libc::sighandler_t);
            libc::signal(libc::SIGINT, handler as *const () as libc::sighandler_t);
        }
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

    // Intentional save_system: CLI batch ingest processes multiple files
    // into a fresh system. A full write is acceptable for this offline path.
    store
        .save_system(&system)
        .context("failed to save system")?;

    println!("done. N={}, episodes={}", system.n(), system.episodes.len());
    Ok(())
}

fn cmd_stats(cli: &Cli) -> Result<()> {
    let store = open_store(cli)?;
    let system = store.load_system().context("failed to load system")?;

    let db_size = store.db_size();
    let activation = store
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
    let config = load_config()?;
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
    let colors::Colors { bold, reset, .. } = colors::Colors::stdout();

    if let Some(id) = episode_id {
        let removed = store
            .forget_episode(id)
            .context("failed to forget episode")?;
        if removed == 0 {
            println!("Episode not found: {id}");
        } else {
            println!("{bold}Forgot{reset} episode {id} ({removed} occurrences removed)");
        }
    } else if let Some(id) = conscious_id {
        let removed = store
            .forget_conscious(id)
            .context("failed to forget conscious memory")?;
        if removed == 0 {
            println!("Conscious memory not found: {id}");
        } else {
            println!("{bold}Forgot{reset} conscious memory {id} ({removed} occurrences removed)");
        }
    } else if let Some(word) = term {
        let (removed_occs, removed_nbhds, removed_eps) =
            store.forget_term(word).context("failed to forget term")?;
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
        am_store::project::default_base_dir().context("cannot determine global config directory")?
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
