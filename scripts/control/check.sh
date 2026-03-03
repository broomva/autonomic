#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

if [ -n "${CONTROL_CHECK_CMD:-}" ]; then
  eval "$CONTROL_CHECK_CMD"
  exit 0
fi

if [ -f Cargo.toml ] && command -v cargo >/dev/null 2>&1; then
  cargo fmt --check
  cargo clippy --workspace -- -D warnings
  exit 0
fi

echo "No check command detected. Set CONTROL_CHECK_CMD."
exit 1
