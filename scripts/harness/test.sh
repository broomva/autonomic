#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

echo "[harness:test] cargo test --workspace"
cargo test --workspace
echo "[harness:test] PASS"
