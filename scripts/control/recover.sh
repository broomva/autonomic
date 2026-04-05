#!/usr/bin/env bash
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
cd "$root"

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m'

info()  { echo -e "${GREEN}[recover]${NC} $1"; }
warn()  { echo -e "${YELLOW}[recover]${NC} $1"; }
fail()  { echo -e "${RED}[recover]${NC} $1"; }

recovered=0
actions_taken=()

# Phase 1: Diagnose
info "Phase 1: Diagnosing gate failures..."

smoke_ok=true
check_ok=true

if ! cargo check --workspace --quiet 2>/dev/null; then
  fail "  smoke FAIL: cargo check failed"
  smoke_ok=false
fi

if $smoke_ok; then
  info "  smoke: PASS"

  if ! cargo fmt --check --quiet 2>/dev/null; then
    warn "  check FAIL: format drift detected"
    check_ok=false
  fi
  if ! cargo clippy --workspace --quiet -- -D warnings 2>/dev/null; then
    warn "  check FAIL: clippy warnings"
    check_ok=false
  fi

  if $check_ok; then
    info "  check: PASS"
  else
    fail "  check: FAIL"
  fi
fi

echo

# Phase 2: Automatic recovery
info "Phase 2: Attempting automatic recovery..."

if ! $check_ok; then
  if ! cargo fmt --check --quiet 2>/dev/null; then
    info "  Applying cargo fmt..."
    cargo fmt
    actions_taken+=("cargo fmt")
    recovered=$((recovered + 1))
  fi
fi

echo

# Phase 3: Re-validate
info "Phase 3: Re-validating..."

final_failures=0

if ! cargo check --workspace --quiet 2>/dev/null; then
  fail "  still fails cargo check"
  final_failures=$((final_failures + 1))
elif ! cargo fmt --check --quiet 2>/dev/null; then
  fail "  still has format issues"
  final_failures=$((final_failures + 1))
elif ! cargo clippy --workspace --quiet -- -D warnings 2>/dev/null; then
  warn "  clippy warnings remain (manual fix required)"
  final_failures=$((final_failures + 1))
else
  info "  All gates: PASS"
fi

echo

# Phase 4: Report
info "Recovery summary:"
echo "  Actions taken: ${#actions_taken[@]}"
for action in "${actions_taken[@]+"${actions_taken[@]}"}"; do
  echo "    - $action"
done
echo "  Recovered: $recovered"
echo "  Remaining failures: $final_failures"

if [ "$final_failures" -gt 0 ]; then
  echo
  fail "ESCALATION: $final_failures issue(s) require manual intervention."
  fail "Run 'cargo clippy --workspace' to see details."
  exit 1
fi

info "All gates recovered successfully."
