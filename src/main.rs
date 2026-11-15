//! `mcp-memory` command line: the stdio MCP front end, the store process, and human commands.

#![recursion_limit = "256"]

use std::path::PathBuf;

use clap::{Parser, Subcommand};

use mcp_memory::config::{self, Config};
use mcp_memory::daemon::{self, CliRequest};

#[derive(Parser)]
#[command(name = "mcp-memory", version, about = "MCP memory backed by a HelixDB graph")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the MCP server on stdio. Starts the shared store process if needed.
    Serve,
    /// Run the shared store process. `serve` and the CLI start it on demand.
    Daemon,
    /// Show store health.
    Health,
    /// List memories.
    List {
        #[arg(long)]
        scope: Option<String>,
        #[arg(long, name = "type")]
        memory_type: Option<String>,
        #[arg(long)]
        text: Option<String>,
    },
    /// Show one memory.
    Show { id: String },
    /// Recall memories for a query.
    Search {
        query: String,
        #[arg(long)]
        project_path: Option<String>,
    },
    /// Delete one memory. This works even when the agent is read-only.
    Delete { id: String },
    /// Export memories as JSON. Embeddings are omitted.
    Export { path: PathBuf },
    /// Import a JSON export and rebuild embeddings.
    Import { path: PathBuf },
    /// Read or change access mode.
    Config {
        #[command(subcommand)]
        action: ConfigCommand,
    },
}

#[derive(Subcommand)]
enum ConfigCommand {
    Show,
    Set { access: String },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn,mcp_memory=info")),
        )
        .init();

    let cli = Cli::parse();
    let code = match run(cli.command.unwrap_or(Command::Serve)).await {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("mcp-memory: {error}");
            1
        }
    };
    // The stdin reader is a blocking thread that would hold the runtime open.
    std::process::exit(code);
}

fn current_dir() -> Option<String> {
    std::env::current_dir()
        .ok()
        .map(|path| path.display().to_string())
}

async fn print(path: &std::path::Path, request: CliRequest) -> Result<(), String> {
    println!("{}", daemon::request(path, request).await?);
    Ok(())
}

async fn run(command: Command) -> Result<(), String> {
    let path = config::config_path();
    match command {
        Command::Serve => daemon::proxy_stdio(&path).await,
        Command::Daemon => daemon::run(path).await,
        Command::Health => print(&path, CliRequest::Health).await,
        Command::List {
            scope,
            memory_type,
            text,
        } => {
            print(
                &path,
                CliRequest::List {
                    scope,
                    memory_type,
                    text,
                },
            )
            .await
        }
        Command::Show { id } => print(&path, CliRequest::Show { id }).await,
        Command::Search {
            query,
            project_path,
        } => {
            print(
                &path,
                CliRequest::Search {
                    query,
                    project_path: project_path.or_else(current_dir),
                },
            )
            .await
        }
        Command::Delete { id } => print(&path, CliRequest::Delete { id }).await,
        Command::Export { path: file } => {
            let value = daemon::request(&path, CliRequest::Export).await?;
            let body = serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?;
            std::fs::write(&file, body).map_err(|error| error.to_string())?;
            println!("exported {}", file.display());
            Ok(())
        }
        Command::Import { path: file } => {
            let text = std::fs::read_to_string(&file).map_err(|error| error.to_string())?;
            let document = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            print(&path, CliRequest::Import { document }).await
        }
        Command::Config { action } => match action {
            ConfigCommand::Show => {
                let config = Config::load(&path)?;
                println!("access {}", config.access.as_str());
                println!("space {}", config.space_id);
                println!("data {}", config.data_dir.display());
                Ok(())
            }
            ConfigCommand::Set { access } => {
                let mut config = Config::load(&path)?;
                config.access = mcp_memory::access::Access::parse(&access)
                    .ok_or_else(|| format!("unknown access {access}"))?;
                config.save(&path)?;
                println!("access {}", config.access.as_str());
                Ok(())
            }
        },
    }
}
