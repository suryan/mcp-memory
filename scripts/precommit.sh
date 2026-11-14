#!/usr/bin/env bash
# Pre-commit gate: tests, diagrams, and the osv-scanner CVE check.
# Usage: bash scripts/precommit.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

echo "=== Pre-commit checks ==="

echo "[1/3] Build and tests..."
export CARGO_TARGET_DIR="$root/target"
# /tmp is a small tmpfs. Cargo and d2's headless browser both fail there.
export TMPDIR="$root/target/tmp"
mkdir -p "$TMPDIR"
cargo test

echo "[2/3] Docs diagrams..."
scripts/render-docs.sh

echo "[3/3] Security audit (osv-scanner)..."
if command -v osv-scanner >/dev/null 2>&1; then
  osv-scanner scan source --recursive . || {
    echo "  FAIL: vulnerabilities found (see above)"
    exit 1
  }
elif [ "${ALLOW_MISSING_OSV_SCANNER:-0}" = "1" ]; then
  echo "  WARN: osv-scanner not found; skipped via ALLOW_MISSING_OSV_SCANNER=1 (local only, never CI)"
else
  echo "  FAIL: osv-scanner not found — CVE gate cannot be skipped (install: https://github.com/google/osv-scanner)"
  exit 1
fi

echo ""
echo "=== Pre-commit checks complete ==="
