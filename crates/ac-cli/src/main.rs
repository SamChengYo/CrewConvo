//! Management commands: migrations, search index rebuild, Decider evaluation.

mod eval;

use std::path::PathBuf;

use ac_store::{DbConfig, Store};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "ac-cli", about = "CrewConvo management CLI")]
struct Cli {
    /// Override the database name (PG_DBNAME)
    #[arg(long, global = true)]
    db: Option<String>,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run database migrations
    Migrate,
    /// Search index maintenance
    Search {
        #[command(subcommand)]
        cmd: SearchCmd,
    },
    /// Decider tools
    Decider {
        #[command(subcommand)]
        cmd: DeciderCmd,
    },
}

#[derive(Subcommand)]
enum SearchCmd {
    /// Rebuild the full-text index from the database (stop the server first: the index is locked while it runs)
    Reindex {
        #[arg(long, env = "SEARCH_INDEX_DIR", default_value = "data/search-index")]
        index_dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum DeciderCmd {
    /// Create the default Decider list for a workspace that has none (keys from GEMINI_API_KEY / OPENAI_API_KEY)
    Seed {
        /// Workspace (default: the first workspace)
        #[arg(long)]
        workspace: Option<uuid::Uuid>,
    },
    /// Evaluate Decider configs on a JSONL fixture set and suggest thresholds
    Eval {
        fixtures: PathBuf,
        /// Only this config (default: every enabled config of the workspace)
        #[arg(long)]
        config: Option<uuid::Uuid>,
        /// Workspace whose configs to use (default: the first workspace)
        #[arg(long)]
        workspace: Option<uuid::Uuid>,
        /// Use built-in configs from environment keys (GEMINI_API_KEY / OPENAI_API_KEY / ANTHROPIC_API_KEY)
        /// instead of the database
        #[arg(long)]
        env_configs: bool,
        /// Write a Markdown report here
        #[arg(long)]
        out: Option<PathBuf>,
        /// Write raw per-fixture results (JSON) here
        #[arg(long)]
        raw: Option<PathBuf>,
        #[arg(long, default_value_t = 4)]
        concurrency: usize,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let _ = dotenvy::dotenv();
    let cli = Cli::parse();
    let connect = || async {
        let cfg = DbConfig::from_env(cli.db.as_deref()).map_err(anyhow::Error::msg)?;
        anyhow::Ok(Store::connect(&cfg, 4).await?)
    };
    match cli.cmd {
        Cmd::Migrate => {
            connect().await?.migrate().await?;
            println!("migrations applied");
        }
        Cmd::Search { cmd: SearchCmd::Reindex { index_dir } } => {
            let store = connect().await?;
            let svc = ac_search::SearchService::open(store, Some(index_dir.clone())).map_err(|e| {
                anyhow::anyhow!(
                    "cannot open index at {} ({e}); is the server running? stop it first",
                    index_dir.display()
                )
            })?;
            let n = svc.reindex_all().await?;
            println!("indexed {n} messages into {}", index_dir.display());
        }
        Cmd::Decider { cmd: DeciderCmd::Seed { workspace } } => {
            let store = connect().await?;
            let ws = match workspace {
                Some(w) => w,
                None => store.default_workspace().await?.ok_or_else(|| anyhow::anyhow!("no workspace"))?.id,
            };
            let n = eval::seed(&store, ws).await?;
            println!("created {n} decider configs for workspace {ws}");
        }
        Cmd::Decider { cmd: DeciderCmd::Eval { fixtures, config, workspace, env_configs, out, raw, concurrency } } => {
            let store = if env_configs { None } else { Some(connect().await?) };
            eval::run(eval::EvalArgs { fixtures, config, workspace, env_configs, out, raw, concurrency }, store)
                .await?;
        }
    }
    Ok(())
}
