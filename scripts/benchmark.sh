#!/usr/bin/env bash
# Run the load benchmark against an on-disk Helix store.
# Usage: bash scripts/benchmark.sh
#
# Defaults (override with env):
#   MCP_MEMORY_BENCH_WRITES=500
#   MCP_MEMORY_BENCH_RECALLS=200
#   MCP_MEMORY_BENCH_CONCURRENT=50
#   MCP_MEMORY_BENCH_REPOS=10
#   MCP_MEMORY_BENCH_WARMUP=5
#   MCP_MEMORY_BENCH_FAIL_ON_GATE=1
#
# Fixture writes use import_json (unique ids). Gates (PRD single-node targets
# at this fixture size): write rate >= 20/s, recall p95 <= 300ms after warmup,
# concurrent recalls all succeed.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}"
export TMPDIR="${TMPDIR:-$root/target/tmp}"
mkdir -p "$TMPDIR"

echo "=== mcp-memory load benchmark (release) ==="
cargo test --release --test load_bench -- --ignored --nocapture
echo "=== benchmark complete ==="
