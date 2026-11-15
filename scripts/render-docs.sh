#!/usr/bin/env bash
# Render docs/architecture.d2 and docs/flow.d2 to PNG with the ELK layout engine.
# Usage: scripts/render-docs.sh
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$root"

if ! command -v d2 >/dev/null 2>&1; then
  echo "FAIL: d2 is not installed (https://d2lang.com/tour/install)"
  exit 1
fi

# d2 renders PNGs with Playwright's Chromium (~165 MB). Sandboxed agent shells point
# PLAYWRIGHT_BROWSERS_PATH at /tmp, a small tmpfs, where the download runs out of space.
export PLAYWRIGHT_BROWSERS_PATH="$HOME/.cache/ms-playwright"

echo "Rendering docs/architecture.d2"
d2 --layout=elk docs/architecture.d2 docs/architecture.png

echo "Rendering docs/flow.d2"
d2 --layout=elk docs/flow.d2 docs/flow.png

echo "Rendered docs/architecture.png and docs/flow.png"
