# Observability

## Logging

The `autonomicd` daemon uses `tracing` with `tracing-subscriber` for structured logging.

Log levels:
- `ERROR`: Gate failures, unrecoverable errors
- `WARN`: Degraded states, recovery attempts
- `INFO`: Normal operations, gate pass/fail
- `DEBUG`: Rule evaluations, projection updates
- `TRACE`: Individual event processing

## Metrics (Planned)

| Metric | Type | Source |
|--------|------|--------|
| `autonomic_projection_events_total` | Counter | Event fold in projection.rs |
| `autonomic_gating_requests_total` | Counter | GET /gating endpoint |
| `autonomic_rule_evaluations_total` | Counter | Engine evaluate() |
| `autonomic_economic_mode` | Gauge | HomeostaticState.economic.mode |
| `autonomic_balance_micro_credits` | Gauge | HomeostaticState.economic.balance |

## Audit Trail

All gating decisions are published back to Lago as `EventKind::Custom { event_type: "autonomic.GatingDecision" }` events, creating a full audit trail of regulation decisions.
