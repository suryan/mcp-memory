//! Load benchmark against an on-disk Helix store.
//!
//! Ignored by default so `cargo test` and precommit stay fast. Run with:
//!
//! ```bash
//! bash scripts/benchmark.sh
//! ```
//!
//! Environment (optional):
//! - `MCP_MEMORY_BENCH_WRITES` — how many memories to create (default 500)
//! - `MCP_MEMORY_BENCH_RECALLS` — timed sequential recalls after warmup (default 200)
//! - `MCP_MEMORY_BENCH_CONCURRENT` — parallel recalls (default 50)
//! - `MCP_MEMORY_BENCH_REPOS` — distinct repository paths to spread writes (default 10)
//! - `MCP_MEMORY_BENCH_WARMUP` — untimed recalls before measuring (default 5)
//! - `MCP_MEMORY_BENCH_FAIL_ON_GATE` — `1` fails the test when a PRD gate is missed (default 1)
//!
//! Gates (from the PRD single-node scale targets, at this smaller fixture size):
//! - write rate at least 20/s
//! - recall p95 under 300 ms after warmup
//! - concurrent recalls all succeed

#![allow(
    clippy::unwrap_used,
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "benchmark reports metrics to the console"
)]

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use mcp_memory::access::{Access, Actor};
use mcp_memory::config::Config;
use mcp_memory::identity::repo_key;
use mcp_memory::model::{content_hash, ExportDocument, MemoryRecord, SCHEMA_VERSION};
use mcp_memory::service::Service;
use mcp_memory::store::Graph;

const SPACE: &str = "bench";

struct BenchConfig {
    writes: usize,
    recalls: usize,
    concurrent: usize,
    repos: usize,
    warmup: usize,
    fail_on_gate: bool,
}

impl BenchConfig {
    fn from_env() -> Self {
        Self {
            writes: env_usize("MCP_MEMORY_BENCH_WRITES", 500),
            recalls: env_usize("MCP_MEMORY_BENCH_RECALLS", 200),
            concurrent: env_usize("MCP_MEMORY_BENCH_CONCURRENT", 50),
            repos: env_usize("MCP_MEMORY_BENCH_REPOS", 10).max(1),
            warmup: env_usize("MCP_MEMORY_BENCH_WARMUP", 5),
            fail_on_gate: env_flag("MCP_MEMORY_BENCH_FAIL_ON_GATE", true),
        }
    }
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_flag(name: &str, default: bool) -> bool {
    match std::env::var(name) {
        Ok(value) => matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"),
        Err(_) => default,
    }
}

fn percentile(sorted: &[Duration], pct: u8) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let last = u32::try_from(sorted.len().saturating_sub(1)).unwrap_or(u32::MAX);
    let rank = u32::from(pct).saturating_mul(last) / 100;
    sorted[usize::try_from(rank).unwrap_or(sorted.len() - 1)]
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

async fn open_service(dir: &std::path::Path) -> Service {
    let config = Config {
        access: Access::Learn,
        data_dir: dir.to_path_buf(),
        space_id: SPACE.into(),
        ..Config::default()
    };
    let path = dir.join("config.toml");
    config.save(&path).unwrap();
    let graph = Graph::open_disk(&dir.join("helix")).await.unwrap();
    Service::open_graph(graph, path)
}

fn repo_path(index: usize) -> String {
    format!("/tmp/mcp-memory-bench/repo-{index}")
}

fn memory_record(index: usize, repos: usize) -> MemoryRecord {
    let path = repo_path(index % repos);
    let key = repo_key(std::path::Path::new(&path), &[]);
    let id = uuid::Uuid::new_v4().to_string();
    let title = format!("bench-fact-{index:06}");
    let summary = format!("Unique summary for bench fact {index:06}");
    let mut content = String::with_capacity(256);
    let _ = write!(
        content,
        "Benchmark body {id}. Query cue: seed fixture integration tests. Repo path {path}."
    );
    let hash = content_hash(&title, &summary, &content);
    let now = chrono::Utc::now().to_rfc3339();
    MemoryRecord {
        id,
        title,
        summary,
        content,
        memory_type: "fact".into(),
        scope: "repository".into(),
        repo_key: key,
        space_id: SPACE.into(),
        principal_id: "bench".into(),
        status: "active".into(),
        content_hash: hash,
        created_at: now.clone(),
        updated_at: now,
        source: "import".into(),
    }
}

async fn run_writes(service: &Service, config: &BenchConfig) -> f64 {
    let memories: Vec<MemoryRecord> = (0..config.writes)
        .map(|index| memory_record(index, config.repos))
        .collect();
    let document = ExportDocument {
        version: SCHEMA_VERSION,
        memories,
        entities: Vec::new(),
        edges: Vec::new(),
    };
    let value = serde_json::to_value(document).unwrap();
    let write_started = Instant::now();
    let result = service.import_json(value).await.unwrap();
    let write_elapsed = write_started.elapsed();
    assert_eq!(result["action"], "imported", "{result}");
    assert_eq!(result["count"], config.writes, "{result}");
    let write_count = f64::from(u32::try_from(config.writes).unwrap_or(u32::MAX));
    let write_rate = write_count / write_elapsed.as_secs_f64().max(f64::EPSILON);
    println!(
        "writes: {} in {:.2}s ({write_rate:.1}/s)",
        config.writes,
        write_elapsed.as_secs_f64()
    );
    write_rate
}

async fn warm_recalls(service: &Service, config: &BenchConfig) {
    for index in 0..config.warmup {
        let repo = repo_path(index % config.repos);
        let _ = service
            .recall(
                Actor::Agent,
                "seed fixture integration tests",
                Some(&repo),
                &[],
            )
            .await
            .unwrap();
    }
}

async fn run_sequential_recalls(service: &Service, config: &BenchConfig) -> Duration {
    let mut latencies = Vec::with_capacity(config.recalls);
    for index in 0..config.recalls {
        let repo = repo_path(index % config.repos);
        let started = Instant::now();
        let result = service
            .recall(
                Actor::Agent,
                "seed fixture integration tests",
                Some(&repo),
                &[],
            )
            .await
            .unwrap();
        latencies.push(started.elapsed());
        assert_eq!(result["action"], "recalled", "recall {index}: {result}");
    }
    latencies.sort();
    let p50 = percentile(&latencies, 50);
    let p95 = percentile(&latencies, 95);
    let p99 = percentile(&latencies, 99);
    println!(
        "recall sequential: n={} p50={:.1}ms p95={:.1}ms p99={:.1}ms",
        config.recalls,
        ms(p50),
        ms(p95),
        ms(p99)
    );
    p95
}

async fn run_concurrent_recalls(service: &Arc<Service>, config: &BenchConfig) -> usize {
    let concurrent_started = Instant::now();
    let mut handles = Vec::with_capacity(config.concurrent);
    for index in 0..config.concurrent {
        let service = Arc::clone(service);
        let repo = repo_path(index % config.repos);
        handles.push(tokio::spawn(async move {
            service
                .recall(
                    Actor::Agent,
                    "seed fixture integration tests",
                    Some(&repo),
                    &[],
                )
                .await
        }));
    }
    let mut succeeded = 0usize;
    for handle in handles {
        let result = handle.await.unwrap().unwrap();
        assert_eq!(result["action"], "recalled", "{result}");
        succeeded += 1;
    }
    println!(
        "recall concurrent: {succeeded}/{} in {:.2}s",
        config.concurrent,
        concurrent_started.elapsed().as_secs_f64()
    );
    succeeded
}

#[tokio::test(flavor = "multi_thread", worker_threads = 8)]
#[ignore = "run via bash scripts/benchmark.sh"]
async fn load_benchmark() {
    let config = BenchConfig::from_env();
    let dir = tempfile::tempdir().unwrap();
    let service = Arc::new(open_service(dir.path()).await);

    println!("mcp-memory load benchmark");
    println!("  data_dir={}", dir.path().display());
    println!(
        "  writes={} recalls={} concurrent={} repos={} warmup={}",
        config.writes, config.recalls, config.concurrent, config.repos, config.warmup
    );

    let write_rate = run_writes(&service, &config).await;
    warm_recalls(&service, &config).await;
    let p95 = run_sequential_recalls(&service, &config).await;
    let concurrent_ok = run_concurrent_recalls(&service, &config).await;

    let writes_ok = write_rate >= 20.0;
    let latency_ok = p95 <= Duration::from_millis(300);
    let parallel_ok = concurrent_ok == config.concurrent;
    println!("gates:");
    println!(
        "  write_rate >= 20/s: {} ({write_rate:.1}/s)",
        if writes_ok { "pass" } else { "FAIL" }
    );
    println!(
        "  recall p95 <= 300ms: {} ({:.1}ms)",
        if latency_ok { "pass" } else { "FAIL" },
        ms(p95)
    );
    println!(
        "  concurrent recalls ok: {} ({concurrent_ok}/{})",
        if parallel_ok { "pass" } else { "FAIL" },
        config.concurrent
    );

    if config.fail_on_gate {
        assert!(writes_ok, "write rate {write_rate:.1}/s below 20/s");
        assert!(latency_ok, "recall p95 {:.1}ms above 300ms", ms(p95));
        assert!(
            parallel_ok,
            "concurrent recalls {concurrent_ok}/{}",
            config.concurrent
        );
    }
}
