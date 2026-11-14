#![recursion_limit = "256"]

use std::path::PathBuf;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use rmcp::transport::stdio_pipes;
use rmcp::ServiceExt;

use mcp_memory::access::Actor;
use mcp_memory::config::{self, Config};
use mcp_memory::mcp::MemoryServer;
use mcp_memory::service::Service;

#[derive(Parser)]
#[command(name = "mcp-memory", version, about = "MCP memory backed by a HelixDB graph")]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Serve the MCP server on stdio.
    Serve,
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
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    if let Err(error) = run(cli.command.unwrap_or(Command::Serve)).await {
        eprintln!("mcp-memory: {error}");
        std::process::exit(1);
    }
}

async fn run(command: Command) -> Result<(), String> {
    let path = config::config_path();
    match command {
        Command::Serve => {
            let service = Arc::new(Service::open(path).await.map_err(|error| error.to_string())?);
            let server = MemoryServer::new(service);
            let running = server
                .serve(stdio_pipes())
                .await
                .map_err(|error| error.to_string())?;
            running.waiting().await.map_err(|error| error.to_string())?;
            Ok(())
        }
        Command::Health => {
            let service = Service::open(path).await.map_err(|error| error.to_string())?;
            println!(
                "{}",
                service.health().await.map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Command::List {
            scope,
            memory_type,
            text,
        } => {
            let service = Service::open(path).await.map_err(|error| error.to_string())?;
            println!(
                "{}",
                service
                    .list(
                        Actor::Human,
                        scope.as_deref(),
                        memory_type.as_deref(),
                        text.as_deref(),
                        None,
                    )
                    .await
                    .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Command::Show { id } => {
            let service = Service::open(path).await.map_err(|error| error.to_string())?;
            println!(
                "{}",
                service
                    .show(Actor::Human, &id)
                    .await
                    .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Command::Search { query, project_path } => {
            let service = Service::open(path).await.map_err(|error| error.to_string())?;
            println!(
                "{}",
                service
                    .recall(Actor::Human, &query, project_path.as_deref(), &[])
                    .await
                    .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Command::Delete { id } => {
            let service = Service::open(path).await.map_err(|error| error.to_string())?;
            println!(
                "{}",
                service
                    .forget(Actor::Human, &id)
                    .await
                    .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Command::Export { path: file } => {
            let service = Service::open(path).await.map_err(|error| error.to_string())?;
            let value = service.export_json().await.map_err(|error| error.to_string())?;
            std::fs::write(&file, serde_json::to_string_pretty(&value).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
            println!("exported {}", file.display());
            Ok(())
        }
        Command::Import { path: file } => {
            let service = Service::open(path).await.map_err(|error| error.to_string())?;
            let text = std::fs::read_to_string(&file).map_err(|error| error.to_string())?;
            let value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
            println!(
                "{}",
                service
                    .import_json(value)
                    .await
                    .map_err(|error| error.to_string())?
            );
            Ok(())
        }
        Command::Config { action } => match action {
            ConfigCommand::Show => {
                let config = Config::load(&path).map_err(|error| error.to_string())?;
                println!("access {}", config.access.as_str());
                println!("space {}", config.space_id);
                println!("data {}", config.data_dir.display());
                Ok(())
            }
            ConfigCommand::Set { access } => {
                let mut config = Config::load(&path).map_err(|error| error.to_string())?;
                config.access = mcp_memory::access::Access::parse(&access)
                    .ok_or_else(|| format!("unknown access {access}"))?;
                config.save(&path).map_err(|error| error.to_string())?;
                println!("access {}", config.access.as_str());
                Ok(())
            }
        },
    }
}
