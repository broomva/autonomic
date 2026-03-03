# Control Loop

## Feedback Cycle

```
measure → compare → decide → act → verify
```

1. **Measure**: Run gate scripts, collect pass/fail status
2. **Compare**: Check against setpoints (100% pass rate target)
3. **Decide**: If failing, determine recovery action
4. **Act**: Execute recovery (auto-format, escalate for manual fix)
5. **Verify**: Re-run gates to confirm recovery

## Setpoints

| Metric | Target | Sensor |
|--------|--------|--------|
| `pass_at_1` | 100% | `cargo test --workspace` exit code |
| `clippy_clean` | 0 warnings | `cargo clippy -- -D warnings` exit code |
| `fmt_clean` | No drift | `cargo fmt --check` exit code |

## Escalation

If automatic recovery fails (e.g., clippy warnings that can't be auto-fixed), the recover script exits non-zero and reports the number of remaining issues for manual intervention.
