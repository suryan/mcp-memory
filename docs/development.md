# Development

Local MCP memory server. The binary speaks MCP over stdio and stores statements in an embedded HelixDB graph on disk.

## Toolchain

HelixDB v3.7.0 needs a current Rust toolchain. This repo pins `1.99.1` in `rust-toolchain.toml`. `rustup` installs it on the first `cargo` command.

## Build and test

```bash
cargo test
```

`scripts/precommit.sh` runs `cargo clippy --all-targets -- -D warnings`, then `cargo test`, re-renders the diagrams, then runs `osv-scanner` against the lockfile. `osv-scanner` must be installed (https://github.com/google/osv-scanner). The gate fails if it is missing, and it fails if any advisory remains. `ALLOW_MISSING_OSV_SCANNER=1` is a local-only escape hatch and must not be set in CI.

Lints live in `Cargo.toml` under `[lints]`: `unsafe_code` is denied, clippy `all` and `pedantic` are denied, and `unwrap`, `expect`, and `panic` are denied outside tests (`clippy.toml` allows them in test functions). Public items need docs, and public functions returning `Result` need an `# Errors` section.

Transitive crates that the scanner flagged are replaced in `third_party/` rather than ignored. `object_store` 0.12.5 is built against `quick-xml` 0.41. Tantivy is built against `lru` 0.18.5. Foyer memory cache type aliases replace the `paste` macro. Figment's YAML provider uses `serde_saphyr` instead of `serde_yaml` / `unsafe-libyaml`. The `bincode` 1.3.3 codec is maintained here as `store_codec`.

| Script | Purpose |
|---|---|
| `scripts/render-docs.sh` | Render `docs/architecture.d2` and `docs/flow.d2` to PNG with ELK |
| `scripts/precommit.sh` | clippy, `cargo test`, diagrams, then `osv-scanner` |
| `scripts/benchmark.sh` | Release-mode load benchmark (writes, recall p95, concurrent recalls) |

## Benchmark

`tests/load_bench.rs` is ignored by default so ordinary `cargo test` stays fast. Run it with:

```bash
bash scripts/benchmark.sh
```

It opens an on-disk Helix store in a temp directory, loads a fixture through `import_json` (default 500 memories across 10 repos; unique ids so near-duplicate detection does not collapse the set), warms recall, then measures sequential recall latency and 50 concurrent recalls. It fails if write rate is under 20/s, recall p95 is over 300 ms, or a concurrent recall errors — the PRD single-node gates at this smaller fixture size. Override size with `MCP_MEMORY_BENCH_WRITES`, `MCP_MEMORY_BENCH_RECALLS`, `MCP_MEMORY_BENCH_CONCURRENT`, `MCP_MEMORY_BENCH_REPOS`, and `MCP_MEMORY_BENCH_WARMUP`. Set `MCP_MEMORY_BENCH_FAIL_ON_GATE=0` to print metrics without failing.

This is not the full PRD million-node / 100-space gate; that remains a larger run you opt into by raising the write count.

## Run

How to install the binary, connect a harness, and use the CLI is in [usage.md](usage.md).

With no subcommand the binary serves MCP on stdio by forwarding to the store process (`mcp-memory daemon`), starting it if needed. The CLI commands below go through the same process. Only the store process opens the graph. Its log is `daemon.log` in the data directory. `MCP_MEMORY_IDLE_SECS` sets how long it stays up with no connections (default 300). Before opening the graph it sets `HELIX_TELEMETRY_LEVEL=off` when that variable is unset, so memory text is not sent off the machine.

`tests/shared_store.rs` runs the built binary: two harness processes race to start the store, both write and read, and the CLI lists while both are connected.

```bash
cargo run -- health
cargo run -- list
cargo run -- search "integration tests"
cargo run -- delete <id>
cargo run -- export memories.json
cargo run -- import memories.json
cargo run -- config show
cargo run -- config set learn
```

Access values: `off`, `read_only`, `read_write`, `learn`. The default is `learn`. CLI commands run as the human actor, so delete works when the agent is read-only.

Data lives under `$MCP_MEMORY_HOME`, or `$XDG_DATA_HOME/mcp-memory`, or `~/.local/share/mcp-memory`. `config.toml`, `daemon.log`, the graph in `helix/`, and the socket and lock in `run/` are all in that directory.

## What this build does

- Tools: `memory_recall`, `memory_remember`, `memory_forget`, `memory_list`, `memory_graph`. Resource: `memory://digest`. Server instructions are in `src/instructions.rs`.
- One personal space (`personal`) and principal (`local`).
- Several harnesses and the CLI share the store through one store process per data directory. Each harness connection is its own MCP session.
- Recall is one Helix read of the caller's space and repositories, then rank in process. An unrelated query returns no hits.
- Writes reject secret-shaped text and do not log the content.
- The vector index and text indexes are created on open and the server waits until each build reports success.

## Not in this build

- A semantic embedding model. `token-hash-v1` keeps shared words closer than unrelated text. It does not do paraphrase recall. Replacing it is a reindex at the same 384 dimensions.
- Admin HTTP API, authentication, and the Helix Cloud / enterprise server profile.
- The scale gate (1M active memories, recall p95 under 300ms). Do not call the server production-ready at that size until it is measured.
- Export of entity edges. Import rebuilds embeddings and is idempotent on memory id. Divergent content on the same id becomes `conflicted` and is left out of recall.
