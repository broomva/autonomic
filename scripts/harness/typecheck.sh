#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

echo "[harness:typecheck] cargo check --workspace"
cargo check --workspace
echo "[harness:typecheck] PASS"
