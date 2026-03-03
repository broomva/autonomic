# Autonomic Control Architecture

## Gate Sequence

```
smoke (cargo check) → check (fmt + clippy) → test (cargo test)
                                ↓
                          audit (all gates)
```

## Components

| Component | Location | Role |
|-----------|----------|------|
| Policy | `.control/policy.yaml` | RBAC rules, capability gates, escalation |
| Commands | `.control/commands.yaml` | Canonical command definitions with setpoints |
| Topology | `.control/topology.yaml` | Repository structure, agent zones |
| State | `.control/state.json` | Current gate status tracking |
| Control Scripts | `scripts/control/` | Gate implementations (smoke, check, test, recover) |
| Harness Scripts | `scripts/harness/` | Deterministic CI scripts (smoke, test, lint, typecheck) |
| Git Hooks | `.githooks/` | Pre-commit (smoke), pre-push (check) |

## Dependencies

Control gates are sequential: smoke must pass before check, check before test.
Recovery (`scripts/control/recover.sh`) attempts auto-fix (cargo fmt) then re-validates.

## Makefile Targets

- `make -f Makefile.control smoke` — Quick build check
- `make -f Makefile.control check` — Format + lint
- `make -f Makefile.control test` — Full test suite
- `make -f Makefile.control recover` — Auto-recovery
- `make -f Makefile.control audit` — All gates sequentially
- `make -f Makefile.harness ci` — lint + typecheck + test
