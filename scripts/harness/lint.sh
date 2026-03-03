#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

echo "[harness:lint] cargo clippy --workspace -- -D warnings"
cargo clippy --workspace -- -D warnings
echo "[harness:lint] PASS"
