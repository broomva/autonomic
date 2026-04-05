# Autonomic

[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/Rust-2024_Edition-orange.svg)](https://www.rust-lang.org/)
[![Tests](https://img.shields.io/badge/tests-passing-green.svg)](#)
[![docs](https://img.shields.io/badge/docs-broomva.tech-purple.svg)](https://docs.broomva.tech/docs/life/autonomic)

Homeostasis controller for the [Agent OS](https://github.com/broomva) -- self-regulation, resource management, and health checks for agent sessions.

Autonomic implements three-pillar regulation (operational, cognitive, and economic homeostasis) to keep agents running within safe operational bounds. It acts as an advisory service that Arcan consults via HTTP to determine gating profiles, economic modes, and budget enforcement.

## Architecture

```
aios-protocol (canonical contract)
    |
autonomic-core          Types, traits, errors (economic modes, gating profiles, hysteresis gates)
    |            \
autonomic-controller    autonomic-lago
    |            /      Lago bridge: event subscription + publishing
autonomic-api           axum HTTP server: /gating, /projection, /health
    |
autonomicd              Daemon binary with CLI, TOML config, optional Lago journal
```

## Key Features

- **Three-pillar homeostasis** -- operational, cognitive, and economic regulation with hysteresis gates to prevent mode flapping.
- **Pure rule engine** -- the controller is a deterministic fold over events with no I/O, making it fully testable.
- **Advisory architecture** -- Arcan consults Autonomic via HTTP GET; failures are non-fatal (fail-open).
- **On-demand bootstrapping** -- session state is lazily loaded from the Lago journal on first access.
- **Six built-in rules** -- survival mode, spend velocity, budget exhaustion, context pressure, token exhaustion, error streak.

## Getting Started

```bash
# Run all 69 tests
cargo test --workspace

# Standalone mode (empty projections, for testing)
cargo run -p autonomicd -- --bind 127.0.0.1:3002

# With Lago persistence (production)
cargo run -p autonomicd -- --bind 127.0.0.1:3002 --lago-data-dir /path/to/data
```

## Crates

| Crate | Tests | Purpose |
|-------|-------|---------|
| `autonomic-core` | 24 | Types, traits, and errors |
| `autonomic-controller` | 31 | Pure rule engine: projection reducer + rule evaluation |
| `autonomic-lago` | 8 | Lago bridge: event subscription + publishing |
| `autonomic-api` | 4 | axum HTTP server with gating, projection, and health endpoints |
| `autonomicd` | 2 | Daemon binary with CLI args, TOML config, signal handling |

## API Endpoints

- `GET /health` -- health check
- `GET /gating/{session_id}` -- get gating profile (bootstraps from Lago on first access)
- `GET /projection/{session_id}` -- get raw homeostatic state

## Requirements

- Rust 2024 edition (MSRV 1.85)
- Depends on `aios-protocol`, `lago-core`, and `lago-journal` from the Agent OS stack

## Documentation

Full documentation: [docs.broomva.tech/docs/life/autonomic](https://docs.broomva.tech/docs/life/autonomic)

## License

[MIT](LICENSE)
