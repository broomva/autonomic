# Autonomic - Homeostasis Controller for the Agent OS

Homeostasis controller and simulation kernel for agent stability regulation.
Three-pillar regulation: operational, cognitive, and economic homeostasis.

## Build & Verify
```bash
cargo fmt && cargo clippy --workspace -- -D warnings && cargo test --workspace
```

## Stack
Rust 2024 | axum (HTTP API) | aios-protocol (canonical contract) | lago (event subscription)

## Crates
- `autonomic-core` - Types, traits, errors (economic modes, gating profiles, hysteresis gates, rules)
- `autonomic-controller` - Pure rule engine: projection reducer + rule evaluation (no I/O)
- `autonomic-lago` - Lago bridge: event subscription + publishing
- `autonomic-api` - axum HTTP server: /gating, /projection, /health endpoints
- `autonomicd` - Daemon binary with config and signal handling

## Critical Patterns
- Economics is a core concern from crate zero, not a bolt-on
- Economic events use `EventKind::Custom` with `"autonomic."` prefix (forward-compatible)
- `AutonomicGatingProfile` embeds canonical `GatingProfile` + economic extensions
- `HysteresisGate` prevents mode flapping with enter/exit thresholds + min-hold duration
- Controller is pure (no I/O) — projection is a deterministic fold over events
- Autonomic is advisory — Arcan consults via HTTP GET, failures are non-fatal

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

## Rules
- **Formatting**: `cargo fmt` before every commit
- **Linting**: `cargo clippy --workspace -- -D warnings`
- **Testing**: All new code requires tests; `cargo test --workspace` must pass
- **Safe Rust**: No `unsafe` unless absolutely necessary
- **Error handling**: `thiserror` for libraries, `anyhow` for binaries
- **Naming**: `snake_case` (functions/files), `PascalCase` (types/traits), `SCREAMING_SNAKE_CASE` (constants)
- **Rust 2024 Edition**: `gen` is reserved keyword; `set_var`/`remove_var` are `unsafe`
- **Module style**: Use `name.rs` file-based modules (not `mod.rs`)
