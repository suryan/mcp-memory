//! One process owns the Helix store. Helix fences an older writer when a newer
//! one opens the same database, so harness processes and the CLI never open it
//! themselves: they connect to this process over a Unix socket, and start it
//! when it is not running.

use std::fs::{DirBuilder, OpenOptions, Permissions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use rmcp::ServiceExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::watch;

use crate::access::Actor;
use crate::config::Config;
use crate::mcp::MemoryServer;
use crate::service::Service;

const SOCKET: &str = "mcp-memory.sock";
const LOCK: &str = "daemon.lock";
const LOG: &str = "daemon.log";
const LOG_LIMIT: u64 = 1024 * 1024;
/// The first line of a connection. Imports carry the whole document here.
const HELLO_LIMIT: u64 = 64 * 1024 * 1024;
/// Linux `sockaddr_un.sun_path` is 108 bytes including the terminator.
const SOCKET_PATH_LIMIT: usize = 107;
const START_TIMEOUT: Duration = Duration::from_secs(60);
const DEFAULT_IDLE: Duration = Duration::from_secs(300);

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Hello {
    Mcp,
    Cli { request: CliRequest },
}

/// A human CLI command, run by the store process as `Actor::Human`.
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case")]
pub enum CliRequest {
    /// Model, schema, and access summary.
    Health,
    /// Browse memories.
    List {
        /// `global` or `repository`.
        scope: Option<String>,
        /// Memory type name.
        memory_type: Option<String>,
        /// Case-insensitive substring.
        text: Option<String>,
    },
    /// One memory by id.
    Show {
        /// Memory id.
        id: String,
    },
    /// Recall as an agent would, for a repository path.
    Search {
        /// Recall query.
        query: String,
        /// Repository path. The client sends its current directory by default.
        project_path: Option<String>,
    },
    /// Delete one memory.
    Delete {
        /// Memory id.
        id: String,
    },
    /// The space as an export document.
    Export,
    /// Import an export document.
    Import {
        /// The parsed export document.
        document: Value,
    },
}

#[derive(Debug, Serialize, Deserialize)]
struct CliResponse {
    ok: bool,
    #[serde(default)]
    value: Value,
    #[serde(default)]
    error: Option<String>,
}

fn text(error: impl std::fmt::Display) -> String {
    error.to_string()
}

/// Socket and lock live in a 0700 directory, so only this user can connect.
fn run_dir(data_dir: &Path) -> Result<PathBuf, String> {
    let dir = data_dir.join("run");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)
        .map_err(text)?;
    std::fs::set_permissions(&dir, Permissions::from_mode(0o700)).map_err(text)?;
    Ok(dir)
}

fn socket_path(data_dir: &Path) -> Result<PathBuf, String> {
    let socket = run_dir(data_dir)?.join(SOCKET);
    if socket.as_os_str().len() > SOCKET_PATH_LIMIT {
        return Err(format!(
            "socket path {} is longer than {SOCKET_PATH_LIMIT} bytes; set MCP_MEMORY_HOME to a shorter directory",
            socket.display()
        ));
    }
    Ok(socket)
}

/// Where the store process writes its log.
pub fn log_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LOG)
}

fn idle_timeout() -> Duration {
    std::env::var("MCP_MEMORY_IDLE_SECS")
        .ok()
        .and_then(|value| value.parse().ok())
        .map_or(DEFAULT_IDLE, Duration::from_secs)
}

/// Run the store process. Returns quietly when another process already owns
/// the store, so racing starters leave exactly one owner.
///
/// # Errors
///
/// Fails when config, the run directory, the lock, the store, or the socket cannot be opened.
pub async fn run(config_path: PathBuf) -> Result<(), String> {
    let config = Config::load(&config_path)?;
    let socket = socket_path(&config.data_dir)?;
    let run = run_dir(&config.data_dir)?;
    let owner = std::fs::metadata(&run).map_err(text)?.uid();

    let mut lock = OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(run.join(LOCK))
        .map_err(text)?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            tracing::info!("another mcp-memory process owns this store");
            return Ok(());
        }
        Err(std::fs::TryLockError::Error(error)) => return Err(error.to_string()),
    }
    lock.set_len(0).map_err(text)?;
    write!(lock, "{}", std::process::id()).map_err(text)?;

    // Only the lock holder reaches this point, so a socket file here is stale.
    let _ = std::fs::remove_file(&socket);
    let service = Arc::new(Box::pin(Service::open(config_path)).await.map_err(text)?);
    let listener = UnixListener::bind(&socket).map_err(text)?;
    tracing::info!(socket = %socket.display(), pid = std::process::id(), "mcp-memory store ready");

    serve(listener, service, owner, idle_timeout()).await;

    let _ = std::fs::remove_file(&socket);
    tracing::info!("mcp-memory store stopped");
    drop(lock);
    Ok(())
}

struct Active(watch::Sender<usize>);

impl Active {
    fn enter(sender: &watch::Sender<usize>) -> Self {
        sender.send_modify(|count| *count += 1);
        Self(sender.clone())
    }
}

impl Drop for Active {
    fn drop(&mut self) {
        self.0.send_modify(|count| *count -= 1);
    }
}

async fn idle(mut active: watch::Receiver<usize>, timeout: Duration) {
    loop {
        if *active.borrow_and_update() == 0 {
            tokio::select! {
                () = tokio::time::sleep(timeout) => return,
                _ = active.changed() => {}
            }
        } else if active.changed().await.is_err() {
            return;
        }
    }
}

async fn terminated() {
    use tokio::signal::unix::{signal, SignalKind};
    let Ok(mut term) = signal(SignalKind::terminate()) else {
        return std::future::pending().await;
    };
    tokio::select! {
        _ = term.recv() => {}
        _ = tokio::signal::ctrl_c() => {}
    }
}

async fn serve(listener: UnixListener, service: Arc<Service>, owner: u32, timeout: Duration) {
    let (active, watcher) = watch::channel(0usize);
    let idle = idle(watcher, timeout);
    tokio::pin!(idle);
    let terminated = terminated();
    tokio::pin!(terminated);
    loop {
        tokio::select! {
            accepted = listener.accept() => match accepted {
                Ok((stream, _)) => {
                    let service = Arc::clone(&service);
                    let guard = Active::enter(&active);
                    tokio::spawn(async move {
                        if let Err(error) = handle(stream, service, owner).await {
                            tracing::warn!(%error, "connection ended with an error");
                        }
                        drop(guard);
                    });
                }
                Err(error) => tracing::warn!(%error, "accept failed"),
            },
            () = &mut idle => {
                tracing::info!(seconds = timeout.as_secs(), "no connections; exiting");
                return;
            }
            () = &mut terminated => return,
        }
    }
}

async fn handle(stream: UnixStream, service: Arc<Service>, owner: u32) -> Result<(), String> {
    let peer = stream.peer_cred().map_err(text)?;
    if peer.uid() != owner {
        return Err(format!("refused connection from uid {}", peer.uid()));
    }
    let (read, mut write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut line = String::new();
    (&mut reader)
        .take(HELLO_LIMIT)
        .read_line(&mut line)
        .await
        .map_err(text)?;
    if line.is_empty() {
        return Ok(());
    }
    match serde_json::from_str::<Hello>(&line).map_err(text)? {
        Hello::Mcp => {
            let running = MemoryServer::new(service)
                .serve((reader, write))
                .await
                .map_err(text)?;
            running.waiting().await.map_err(text)?;
        }
        Hello::Cli { request } => {
            let response = match execute(&service, request).await {
                Ok(value) => CliResponse {
                    ok: true,
                    value,
                    error: None,
                },
                Err(error) => CliResponse {
                    ok: false,
                    value: Value::Null,
                    error: Some(error),
                },
            };
            let mut body = serde_json::to_string(&response).map_err(text)?;
            body.push('\n');
            write.write_all(body.as_bytes()).await.map_err(text)?;
            write.shutdown().await.map_err(text)?;
        }
    }
    Ok(())
}

async fn execute(service: &Service, request: CliRequest) -> Result<Value, String> {
    let human = Actor::Human;
    match request {
        CliRequest::Health => service.health(),
        CliRequest::List {
            scope,
            memory_type,
            text,
        } => {
            service
                .list(human, scope.as_deref(), memory_type.as_deref(), text.as_deref(), None)
                .await
        }
        CliRequest::Show { id } => service.show(human, &id).await,
        CliRequest::Search {
            query,
            project_path,
        } => service.recall(human, &query, project_path.as_deref(), &[]).await,
        CliRequest::Delete { id } => service.forget(human, &id).await,
        CliRequest::Export => service.export_json().await,
        CliRequest::Import { document } => service.import_json(document).await,
    }
    .map_err(text)
}

fn spawn(data_dir: &Path) -> Result<std::process::Child, String> {
    let exe = std::env::current_exe().map_err(text)?;
    let log = log_path(data_dir);
    if std::fs::metadata(&log).is_ok_and(|meta| meta.len() > LOG_LIMIT) {
        let _ = std::fs::remove_file(&log);
    }
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .map_err(text)?;
    std::process::Command::new(exe)
        .arg("daemon")
        .current_dir(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log_file)
        // Leave the harness's process group so its Ctrl-C does not stop the store.
        .process_group(0)
        .spawn()
        .map_err(text)
}

/// Connect to the store process, starting it when no process is listening.
async fn connect(config_path: &Path) -> Result<UnixStream, String> {
    let config = Config::load(config_path)?;
    let socket = socket_path(&config.data_dir)?;
    let deadline = Instant::now() + START_TIMEOUT;
    let mut child: Option<std::process::Child> = None;
    loop {
        if let Ok(stream) = UnixStream::connect(&socket).await {
            if let Some(mut child) = child {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
            }
            return Ok(stream);
        }
        let starting = match child.as_mut() {
            Some(process) => match process.try_wait().map_err(text)? {
                None => true,
                Some(status) if status.success() => false,
                Some(status) => {
                    return Err(format!(
                        "mcp-memory store exited with {status}; see {}",
                        log_path(&config.data_dir).display()
                    ))
                }
            },
            None => false,
        };
        if Instant::now() >= deadline {
            return Err(format!(
                "mcp-memory store did not start within {}s; see {}",
                START_TIMEOUT.as_secs(),
                log_path(&config.data_dir).display()
            ));
        }
        // A start that exits cleanly lost the lock to a store that is still
        // shutting down. Start again until one owns the store.
        if !starting {
            child = Some(spawn(&config.data_dir)?);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn send_hello(write: &mut (impl AsyncWrite + Unpin), hello: &Hello) -> Result<(), String> {
    let mut line = serde_json::to_string(hello).map_err(text)?;
    line.push('\n');
    write.write_all(line.as_bytes()).await.map_err(text)?;
    write.flush().await.map_err(text)
}

async fn pump(
    mut from: impl AsyncRead + Unpin,
    mut to: impl AsyncWrite + Unpin,
) -> std::io::Result<()> {
    let mut buffer = vec![0u8; 64 * 1024];
    loop {
        let read = from.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        to.write_all(&buffer[..read]).await?;
        to.flush().await?;
    }
}

/// Serve one harness: forward stdio to the store process until it closes.
///
/// # Errors
///
/// Fails when the store process cannot be reached or stdout cannot be written.
pub async fn proxy_stdio(config_path: &Path) -> Result<(), String> {
    let stream = connect(config_path).await?;
    let (read, mut write) = stream.into_split();
    send_hello(&mut write, &Hello::Mcp).await?;
    tokio::spawn(async move {
        let _ = pump(tokio::io::stdin(), &mut write).await;
        let _ = write.shutdown().await;
    });
    pump(read, tokio::io::stdout()).await.map_err(text)
}

/// Run one human CLI command in the store process.
///
/// # Errors
///
/// Fails when the store process cannot be reached or the command fails there.
pub async fn request(config_path: &Path, request: CliRequest) -> Result<Value, String> {
    let stream = connect(config_path).await?;
    let (read, mut write) = stream.into_split();
    send_hello(&mut write, &Hello::Cli { request }).await?;
    write.shutdown().await.map_err(text)?;
    let mut body = String::new();
    BufReader::new(read)
        .read_to_string(&mut body)
        .await
        .map_err(text)?;
    let response: CliResponse = serde_json::from_str(&body)
        .map_err(|_| "mcp-memory store closed the connection without a reply".to_string())?;
    if response.ok {
        Ok(response.value)
    } else {
        Err(response.error.unwrap_or_else(|| "request failed".into()))
    }
}
