#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

echo "[harness:smoke] cargo check --workspace"
cargo check --workspace --quiet
echo "[harness:smoke] PASS"
