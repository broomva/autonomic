//! Projection reducer: fold events into `HomeostaticState`.
//!
//! This is a pure function with no I/O. Given a state and an event,
//! it produces an updated state. Deterministic: same event sequence
//! always produces the same state.

use aios_protocol::event::{EventKind, SpanStatus, TokenUsage};
use autonomic_core::ModelCostRates;
use autonomic_core::events::AutonomicEvent;
use autonomic_core::gating::HomeostaticState;

/// Default cost rates used when no model-specific rates are available.
const DEFAULT_RATES: ModelCostRates = ModelCostRates {
    input_per_token: 3,
    output_per_token: 15,
};

/// Apply a single event to the homeostatic state, returning the updated state.
///
/// This is the core projection function — a pure fold.
pub fn fold(
    mut state: HomeostaticState,
    kind: &EventKind,
    seq: u64,
    ts_ms: u64,
) -> HomeostaticState {
    state.last_event_seq = seq;
    state.last_event_ms = ts_ms;

    match kind {
        // ── Run lifecycle ──
        EventKind::RunFinished { usage, .. } => {
            if let Some(usage) = usage {
                apply_token_usage(&mut state, usage);
            }
            state.operational.total_successes += 1;
            state.operational.error_streak = 0;
            state.operational.last_tick_ms = ts_ms;
            state.cognitive.turns_completed += 1;
        }

        EventKind::RunErrored { .. } => {
            state.operational.error_streak += 1;
            state.operational.total_errors += 1;
            state.operational.last_tick_ms = ts_ms;
        }

        // ── Tool lifecycle ──
        EventKind::ToolCallCompleted {
            status,
            duration_ms: _,
            ..
        } => {
            if *status == SpanStatus::Ok {
                state.operational.total_successes += 1;
                state.operational.error_streak = 0;
            } else {
                state.operational.error_streak += 1;
                state.operational.total_errors += 1;
            }
        }

        EventKind::ToolCallFailed { .. } => {
            state.operational.error_streak += 1;
            state.operational.total_errors += 1;
        }

        // ── Text streaming (token tracking) ──
        EventKind::AssistantMessageCommitted { token_usage, .. }
        | EventKind::Message { token_usage, .. } => {
            if let Some(usage) = token_usage {
                apply_token_usage(&mut state, usage);
            }
        }

        // ── Homeostasis events from aiOS ──
        EventKind::StateEstimated { state: sv, mode } => {
            state.operational.mode = *mode;
            state.cognitive.context_pressure = sv.context_pressure;
            state.cognitive.tokens_remaining = sv.budget.tokens_remaining;
        }

        EventKind::BudgetUpdated { budget, .. } => {
            state.cognitive.tokens_remaining = budget.tokens_remaining;
        }

        EventKind::ModeChanged { to, .. } => {
            state.operational.mode = *to;
        }

        EventKind::CircuitBreakerTripped { error_streak, .. } => {
            state.operational.error_streak = *error_streak;
        }

        // ── Autonomic custom events ──
        EventKind::Custom { event_type, data } => {
            if let Some(autonomic_event) = AutonomicEvent::from_custom(event_type, data) {
                apply_autonomic_event(&mut state, &autonomic_event);
            }
        }

        // All other events — no state change
        _ => {}
    }

    state
}

/// Apply token usage to cognitive and economic state.
fn apply_token_usage(state: &mut HomeostaticState, usage: &TokenUsage) {
    let total = u64::from(usage.total_tokens);
    state.cognitive.total_tokens_used += total;
    state.cognitive.tokens_remaining = state.cognitive.tokens_remaining.saturating_sub(total);

    // Estimate cost using default rates
    let cost = i64::from(usage.prompt_tokens) * DEFAULT_RATES.input_per_token
        + i64::from(usage.completion_tokens) * DEFAULT_RATES.output_per_token;
    state.economic.balance_micro_credits -= cost;
    state.economic.lifetime_costs += cost;

    // Update context pressure based on token consumption
    if state.cognitive.tokens_remaining > 0 {
        let total_budget = state.cognitive.total_tokens_used + state.cognitive.tokens_remaining;
        state.cognitive.context_pressure =
            state.cognitive.total_tokens_used as f32 / total_budget as f32;
    }
}

/// Apply an Autonomic-specific event to state.
fn apply_autonomic_event(state: &mut HomeostaticState, event: &AutonomicEvent) {
    match event {
        AutonomicEvent::CostCharged {
            amount_micro_credits,
            balance_after,
            ..
        } => {
            state.economic.balance_micro_credits = *balance_after;
            state.economic.lifetime_costs += amount_micro_credits;
            state.economic.last_cost_event_ms = state.last_event_ms;
        }
        AutonomicEvent::EconomicModeChanged { to, .. } => {
            state.economic.mode = *to;
        }
        AutonomicEvent::CreditDeposited {
            amount_micro_credits,
            balance_after,
            ..
        } => {
            state.economic.balance_micro_credits = *balance_after;
            state.economic.lifetime_revenue += amount_micro_credits;
        }
        AutonomicEvent::GatingDecision { .. } => {
            // Informational — no state mutation needed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aios_protocol::event::TokenUsage;
    use aios_protocol::mode::OperatingMode;
    use aios_protocol::state::{AgentStateVector, BudgetState};
    use autonomic_core::gating::HomeostaticState;

    fn default_state() -> HomeostaticState {
        HomeostaticState::for_agent("test-agent")
    }

    #[test]
    fn fold_run_finished_updates_tokens() {
        let state = default_state();
        let kind = EventKind::RunFinished {
            reason: "done".into(),
            total_iterations: 1,
            final_answer: None,
            usage: Some(TokenUsage {
                prompt_tokens: 100,
                completion_tokens: 50,
                total_tokens: 150,
            }),
        };
        let new_state = fold(state, &kind, 1, 1000);
        assert_eq!(new_state.cognitive.total_tokens_used, 150);
        assert_eq!(new_state.cognitive.turns_completed, 1);
        assert_eq!(new_state.operational.error_streak, 0);
        assert_eq!(new_state.operational.total_successes, 1);
    }

    #[test]
    fn fold_run_errored_increments_error_streak() {
        let state = default_state();
        let kind = EventKind::RunErrored {
            error: "timeout".into(),
        };
        let new_state = fold(state, &kind, 1, 1000);
        assert_eq!(new_state.operational.error_streak, 1);
        assert_eq!(new_state.operational.total_errors, 1);
    }

    #[test]
    fn fold_tool_completed_ok_resets_error_streak() {
        let mut state = default_state();
        state.operational.error_streak = 3;
        let kind = EventKind::ToolCallCompleted {
            tool_run_id: "tr1".into(),
            call_id: Some("c1".into()),
            tool_name: "read_file".into(),
            result: serde_json::json!("ok"),
            duration_ms: 50,
            status: SpanStatus::Ok,
        };
        let new_state = fold(state, &kind, 1, 1000);
        assert_eq!(new_state.operational.error_streak, 0);
    }

    #[test]
    fn fold_tool_failed_increments_errors() {
        let state = default_state();
        let kind = EventKind::ToolCallFailed {
            call_id: "c1".into(),
            tool_name: "write_file".into(),
            error: "permission denied".into(),
        };
        let new_state = fold(state, &kind, 1, 1000);
        assert_eq!(new_state.operational.error_streak, 1);
        assert_eq!(new_state.operational.total_errors, 1);
    }

    #[test]
    fn fold_state_estimated_updates_mode_and_pressure() {
        let state = default_state();
        let sv = AgentStateVector {
            context_pressure: 0.75,
            budget: BudgetState {
                tokens_remaining: 30_000,
                ..Default::default()
            },
            ..Default::default()
        };
        let kind = EventKind::StateEstimated {
            state: sv,
            mode: OperatingMode::Verify,
        };
        let new_state = fold(state, &kind, 1, 1000);
        assert_eq!(new_state.operational.mode, OperatingMode::Verify);
        assert!((new_state.cognitive.context_pressure - 0.75).abs() < f32::EPSILON);
        assert_eq!(new_state.cognitive.tokens_remaining, 30_000);
    }

    #[test]
    fn fold_mode_changed() {
        let state = default_state();
        let kind = EventKind::ModeChanged {
            from: OperatingMode::Execute,
            to: OperatingMode::Recover,
            reason: "error streak".into(),
        };
        let new_state = fold(state, &kind, 1, 1000);
        assert_eq!(new_state.operational.mode, OperatingMode::Recover);
    }

    #[test]
    fn fold_autonomic_cost_charged() {
        let state = default_state();
        let event = AutonomicEvent::CostCharged {
            amount_micro_credits: 500,
            reason: autonomic_core::CostReason::ModelInference {
                model: "sonnet".into(),
                prompt_tokens: 100,
                completion_tokens: 50,
            },
            balance_after: 9_999_500,
        };
        let kind = event.into_event_kind();
        let new_state = fold(state, &kind, 1, 1000);
        assert_eq!(new_state.economic.balance_micro_credits, 9_999_500);
    }

    #[test]
    fn fold_sequence_produces_deterministic_state() {
        let events = vec![
            EventKind::RunFinished {
                reason: "done".into(),
                total_iterations: 1,
                final_answer: None,
                usage: Some(TokenUsage {
                    prompt_tokens: 100,
                    completion_tokens: 50,
                    total_tokens: 150,
                }),
            },
            EventKind::ToolCallCompleted {
                tool_run_id: "tr1".into(),
                call_id: Some("c1".into()),
                tool_name: "read".into(),
                result: serde_json::json!("ok"),
                duration_ms: 10,
                status: SpanStatus::Ok,
            },
            EventKind::RunErrored {
                error: "fail".into(),
            },
        ];

        // Run the fold twice — must produce identical results
        let fold_once = |events: &[EventKind]| {
            let mut state = HomeostaticState::for_agent("test");
            for (i, kind) in events.iter().enumerate() {
                state = fold(state, kind, i as u64, (i as u64) * 100);
            }
            state
        };

        let state1 = fold_once(&events);
        let state2 = fold_once(&events);

        assert_eq!(
            state1.cognitive.total_tokens_used,
            state2.cognitive.total_tokens_used
        );
        assert_eq!(
            state1.operational.error_streak,
            state2.operational.error_streak
        );
        assert_eq!(
            state1.operational.total_successes,
            state2.operational.total_successes
        );
        assert_eq!(
            state1.operational.total_errors,
            state2.operational.total_errors
        );
        assert_eq!(state1.last_event_seq, state2.last_event_seq);
    }
}
