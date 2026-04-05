#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

if [ ! -d .git ]; then
  echo "error: not a git repository: $root" >&2
  exit 1
fi

chmod +x .githooks/pre-commit .githooks/pre-push 2>/dev/null || true
git config core.hooksPath .githooks
echo "Git hooks installed: core.hooksPath=.githooks"
