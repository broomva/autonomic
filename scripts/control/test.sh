#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

if [ -n "${CONTROL_TEST_CMD:-}" ]; then
  eval "$CONTROL_TEST_CMD"
  exit 0
fi

if [ -f Cargo.toml ] && command -v cargo >/dev/null 2>&1; then
  cargo test --workspace --quiet
  exit 0
fi

echo "No test command detected. Set CONTROL_TEST_CMD."
exit 1
