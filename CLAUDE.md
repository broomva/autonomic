# Autonomic - Homeostasis Controller for the Agent OS

**Version**: 0.1.0 | **Date**: 2026-03-03 | **Status**: Phase 0 COMPLETE
**Tests**: 69 passing | 5 crates | Rust 2024 Edition (MSRV 1.85)

Homeostasis controller for agent stability regulation.
Three-pillar regulation: operational, cognitive, and economic homeostasis.

## Build & Verify
```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace
```

## Stack
Rust 2024 | axum (HTTP API) | aios-protocol (canonical contract) | lago (event subscription)

## Crates
- `autonomic-core` (24 tests) - Types, traits, errors (economic modes, gating profiles, hysteresis gates, rules)
- `autonomic-controller` (31 tests) - Pure rule engine: projection reducer + rule evaluation (no I/O)
- `autonomic-lago` (8 tests) - Lago bridge: event subscription + publishing
- `autonomic-api` (4 tests) - axum HTTP server: /gating, /projection, /health endpoints
- `autonomicd` (2 tests) - Daemon binary with config, signal handling, optional Lago journal

## Running

```bash
# Standalone mode (empty projections, for testing)
cargo run -p autonomicd -- --bind 127.0.0.1:3002

# With Lago persistence (production — reads events from journal)
cargo run -p autonomicd -- --bind 127.0.0.1:3002 --lago-data-dir /path/to/data

# With TOML config
cargo run -p autonomicd -- --config autonomic.toml
```

### API Endpoints
- `GET /health` — health check
- `GET /gating/{session_id}` — get gating profile (bootstraps from Lago on first access)
- `GET /projection/{session_id}` — get raw homeostatic state

## Critical Patterns
- Economics is a core concern from crate zero, not a bolt-on
- Economic events use `EventKind::Custom` with `"autonomic."` prefix (forward-compatible)
- `AutonomicGatingProfile` embeds canonical `GatingProfile` + economic extensions
- `HysteresisGate` prevents mode flapping with time-based cooldown (30s min-hold)
- Controller is pure (no I/O) — projection is a deterministic fold over events
- Autonomic is advisory — Arcan consults via HTTP GET, failures are non-fatal
- On-demand session bootstrapping: `/gating/{session_id}` loads projection from Lago journal + spawns live subscriber

## Dependency Order
```
aios-protocol (canonical contract)
    |
autonomic-core (types + traits)
    |          \
autonomic-controller    autonomic-lago (+ lago-core, lago-journal)
    |          /
autonomic-api (axum)
    |
autonomicd (binary)
```

## Phase 0 Completion (2026-03-03)

- [x] Core types, traits, and errors (economic modes, gating profiles, hysteresis gates)
- [x] Pure rule engine with 6 rules (survival, spend velocity, budget exhaustion, context pressure, token exhaustion, error streak)
- [x] Deterministic projection fold over canonical EventKind variants
- [x] Lago bridge with publisher and subscriber (8 tests)
- [x] HTTP API with gating, projection, and health endpoints
- [x] Daemon binary with CLI args, TOML config, optional Lago journal
- [x] On-demand session bootstrapping from Lago journal
- [x] Hysteresis gates wired into projection fold (time-based cooldown)
- [x] Registered as git submodule in life/ monorepo
- [x] Architecture dependency audit passing

## Known Gaps (Post Phase 0)

- Not yet consulted by Arcan agent loop (HTTP client integration pending — R5)
- No observability (metrics/traces) — R2
- Identity system is placeholder
- No E2E integration test with live Lago journal
- No DM/private gating channel

## Rules
- **Formatting**: `cargo fmt` before every commit
- **Linting**: `cargo clippy --workspace -- -D warnings`
- **Testing**: All new code requires tests; `cargo test --workspace` must pass
- **Safe Rust**: No `unsafe` unless absolutely necessary
- **Error handling**: `thiserror` for libraries, `anyhow` for binaries
- **Naming**: `snake_case` (functions/files), `PascalCase` (types/traits), `SCREAMING_SNAKE_CASE` (constants)
- **Rust 2024 Edition**: `gen` is reserved keyword; `set_var`/`remove_var` are `unsafe`
- **Module style**: Use `name.rs` file-based modules (not `mod.rs`)
