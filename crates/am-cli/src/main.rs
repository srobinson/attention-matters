mod server;

use std::path::PathBuf;

use std::io::Write;

use am_core::{QueryEngine, compose_context, compute_surface, export_json, ingest_text};
use am_store::ProjectStore;
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use rand::SeedableRng;
use rand::rngs::SmallRng;
use rmcp::{ServiceExt, transport::stdio};

#[derive(Parser)]
#[command(name = "am", about = "DAE attention engine CLI and MCP server")]
struct Cli {
    /// Override project auto-detection
    #[arg(long, global = true)]
    project: Option<String>,

    /// Enable verbose debug output
    #[arg(long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start MCP server on stdio transport
    Serve,

    /// Query the memory system
    Query {
        /// Text to query
        text: String,
    },

    /// Ingest a document file (.txt, .md, .html)
    Ingest {
        /// File path(s) to ingest
        #[arg(required = true)]
        files: Vec<PathBuf>,

        /// Ingest all matching files in a directory
        #[arg(long)]
        dir: Option<PathBuf>,
    },

    /// Show system statistics
    Stats,

    /// Export state to a JSON file
    Export {
        /// Output file path
        path: PathBuf,
    },

    /// Import state from a JSON file
    Import {
        /// Input file path
        path: PathBuf,
    },
}

fn open_store(cli: &Cli) -> Result<ProjectStore> {
    let base_dir = std::env::var("AM_DATA_DIR")
        .ok()
        .map(std::path::PathBuf::from);
    ProjectStore::open(cli.project.as_deref(), base_dir.as_deref())
        .context("failed to open project store")
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
                "another am serve (PID {pid}) is running — coexisting with busy_timeout"
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
    tracing::info!("starting MCP server for project '{}'", store.project_id());

    let pidfile = acquire_pidfile();

    let server = server::AmServer::new(store).map_err(|e| anyhow::anyhow!("{e}"))?;
    let service = server
        .serve(stdio())
        .await
        .context("failed to start MCP server")?;
    service.waiting().await?;

    if let Some(path) = pidfile {
        release_pidfile(&path);
    }
    Ok(())
}

fn cmd_query(cli: &Cli, text: &str) -> Result<()> {
    let store = open_store(cli)?;
    let mut system = store
        .load_project_system()
        .context("failed to load system")?;

    let query_result = QueryEngine::process_query(&mut system, text);
    let surface = compute_surface(&system, &query_result);
    let composed = compose_context(
        &mut system,
        &surface,
        &query_result,
        &query_result.interference,
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
    let mut system = store
        .load_project_system()
        .context("failed to load system")?;
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
        .save_project_system(&system)
        .context("failed to save system")?;

    println!("done. N={}, episodes={}", system.n(), system.episodes.len());
    Ok(())
}

fn cmd_stats(cli: &Cli) -> Result<()> {
    let store = open_store(cli)?;
    let system = store
        .load_project_system()
        .context("failed to load system")?;

    let db_size = store.project_store().db_size();
    let activation = store
        .project_store()
        .activation_distribution()
        .context("failed to get activation stats")?;

    println!("project:    {}", store.project_id());
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

fn cmd_export(cli: &Cli, path: &std::path::Path) -> Result<()> {
    let store = open_store(cli)?;
    let system = store
        .load_project_system()
        .context("failed to load system")?;

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
        .load_project_system()
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
