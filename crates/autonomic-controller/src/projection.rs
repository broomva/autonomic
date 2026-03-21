//! Projection reducer: fold events into `HomeostaticState`.
//!
//! This is a pure function with no I/O. Given a state and an event,
//! it produces an updated state. Deterministic: same event sequence
//! always produces the same state.

use aios_protocol::event::{EventKind, SpanStatus, TokenUsage};
use autonomic_core::ModelCostRates;
use autonomic_core::events::AutonomicEvent;
use autonomic_core::gating::HomeostaticState;
use tracing::{Span, instrument};

/// Default cost rates used when no model-specific rates are available.
const DEFAULT_RATES: ModelCostRates = ModelCostRates {
    input_per_token: 3,
    output_per_token: 15,
};

/// Apply a single event to the homeostatic state, returning the updated state.
///
/// This is the core projection function — a pure fold.
#[instrument(level = "debug", skip(state, kind), fields(life.event_seq = seq, life.event_kind))]
pub fn fold(
    mut state: HomeostaticState,
    kind: &EventKind,
    seq: u64,
    ts_ms: u64,
) -> HomeostaticState {
    // Extract variant name from Debug representation (e.g. "RunFinished { .. }" → "RunFinished")
    let kind_dbg = format!("{kind:?}");
    let variant_name = kind_dbg
        .split_once([' ', '{'])
        .map_or(kind_dbg.as_str(), |(name, _)| name);
    Span::current().record("life.event_kind", variant_name);

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
            } else if event_type.starts_with(STRATEGY_EVENT_PREFIX) {
                apply_strategy_event(&mut state, event_type, ts_ms);
            } else if event_type.starts_with(EVAL_EVENT_PREFIX) {
                apply_eval_event(&mut state, event_type, data, ts_ms);
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

/// Prefix for strategy custom events emitted by strategy skills to Lago.
const STRATEGY_EVENT_PREFIX: &str = "strategy.";

/// Prefix for evaluation events emitted by Nous to Lago.
const EVAL_EVENT_PREFIX: &str = "eval.";

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
            let ts_ms = state.last_event_ms;

            // Route through hysteresis gate to prevent mode flapping.
            // The gate enforces a minimum hold duration between transitions.
            // We use the gate's timing mechanism: if the last transition was
            // too recent, the gate blocks the change.
            let elapsed = ts_ms.saturating_sub(state.economic.mode_gate.last_transition_ms);
            let allow = elapsed >= state.economic.mode_gate.min_hold_ms;

            if allow && *to != state.economic.mode {
                state.economic.mode = *to;
                state.economic.mode_gate.last_transition_ms = ts_ms;
            }
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

/// Apply a strategy event to state.
///
/// Strategy events arrive as `EventKind::Custom` with `"strategy."` prefix.
/// They are count-based accumulators — pure and deterministic.
fn apply_strategy_event(state: &mut HomeostaticState, event_type: &str, ts_ms: u64) {
    state.strategy.last_strategy_event_ms = ts_ms;

    match event_type {
        "strategy.drift_detected" => {
            state.strategy.drift_alerts += 1;
        }
        "strategy.decision_logged" => {
            state.strategy.decisions_logged += 1;
        }
        "strategy.critique_completed" => {
            state.strategy.critiques_completed += 1;
        }
        _ => {
            // Unknown strategy event subtype — no state change,
            // but timestamp is still updated above.
        }
    }
}

/// Exponential moving average smoothing factor for eval quality scores.
const EVAL_EMA_ALPHA: f64 = 0.3;

/// Apply an evaluation event to state.
///
/// Eval events arrive as `EventKind::Custom` with `"eval."` prefix.
/// They update the `EvalState` with quality score tracking.
fn apply_eval_event(
    state: &mut HomeostaticState,
    event_type: &str,
    data: &serde_json::Value,
    ts_ms: u64,
) {
    state.eval.last_eval_ms = ts_ms;

    match event_type {
        "eval.InlineCompleted" => {
            state.eval.inline_eval_count += 1;
            if let Some(score) = data.get("score").and_then(serde_json::Value::as_f64) {
                let prev = state.eval.aggregate_quality_score;
                state.eval.aggregate_quality_score =
                    EVAL_EMA_ALPHA * score + (1.0 - EVAL_EMA_ALPHA) * prev;
                state.eval.quality_trend = state.eval.aggregate_quality_score - prev;
            }
        }
        "eval.AsyncCompleted" => {
            state.eval.async_eval_count += 1;
            // Aggregate from scores array if present.
            if let Some(scores) = data.get("scores").and_then(|v| v.as_array()) {
                for score_obj in scores {
                    if let Some(score) = score_obj.get("value").and_then(serde_json::Value::as_f64)
                    {
                        let prev = state.eval.aggregate_quality_score;
                        state.eval.aggregate_quality_score =
                            EVAL_EMA_ALPHA * score + (1.0 - EVAL_EMA_ALPHA) * prev;
                        state.eval.quality_trend = state.eval.aggregate_quality_score - prev;
                    }
                }
            }
        }
        "eval.QualityChanged" => {
            // Direct quality update from Nous.
            if let Some(quality) = data
                .get("aggregate_quality")
                .and_then(serde_json::Value::as_f64)
            {
                let prev = state.eval.aggregate_quality_score;
                state.eval.aggregate_quality_score = quality;
                state.eval.quality_trend = quality - prev;
            }
        }
        _ => {
            // Unknown eval event subtype — timestamp updated above.
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

    #[test]
    fn fold_economic_mode_change_hysteresis_prevents_flapping() {
        let mut state = default_state();
        // Use 5s min_hold; timestamps offset well past the initial settle period
        state.economic.mode_gate = autonomic_core::hysteresis::HysteresisGate::new(0.7, 0.3, 5_000);

        // Escalate Sovereign → Hustle at t=100_000 (severity 0.8 > enter 0.7)
        let event = AutonomicEvent::EconomicModeChanged {
            from: autonomic_core::EconomicMode::Sovereign,
            to: autonomic_core::EconomicMode::Hustle,
            reason: "balance low".into(),
        };
        let state = fold(state, &event.into_event_kind(), 1, 100_000);
        assert_eq!(state.economic.mode, autonomic_core::EconomicMode::Hustle);

        // Try to relax Hustle → Sovereign at t=101_000 (only 1s later, min_hold=5s)
        // Gate is still active because min_hold hasn't elapsed, so relaxation is blocked
        let event = AutonomicEvent::EconomicModeChanged {
            from: autonomic_core::EconomicMode::Hustle,
            to: autonomic_core::EconomicMode::Sovereign,
            reason: "got credit".into(),
        };
        let state = fold(state, &event.into_event_kind(), 2, 101_000);
        assert_eq!(
            state.economic.mode,
            autonomic_core::EconomicMode::Hustle,
            "hysteresis should prevent rapid relaxation"
        );
    }

    #[test]
    fn fold_economic_mode_change_allowed_after_hold_duration() {
        let mut state = default_state();
        state.economic.mode_gate = autonomic_core::hysteresis::HysteresisGate::new(0.7, 0.3, 1_000);

        // Escalate at t=100_000 (well past initial settle period)
        let event = AutonomicEvent::EconomicModeChanged {
            from: autonomic_core::EconomicMode::Sovereign,
            to: autonomic_core::EconomicMode::Hustle,
            reason: "balance low".into(),
        };
        let state = fold(state, &event.into_event_kind(), 1, 100_000);
        assert_eq!(state.economic.mode, autonomic_core::EconomicMode::Hustle);

        // Relax at t=102_000 (2s later, well past 1s min_hold)
        let event = AutonomicEvent::EconomicModeChanged {
            from: autonomic_core::EconomicMode::Hustle,
            to: autonomic_core::EconomicMode::Sovereign,
            reason: "balance recovered".into(),
        };
        let state = fold(state, &event.into_event_kind(), 2, 102_000);
        assert_eq!(
            state.economic.mode,
            autonomic_core::EconomicMode::Sovereign,
            "transition allowed after hold duration elapsed"
        );
    }

    // ── Strategy event tests ──

    fn strategy_event(event_type: &str) -> EventKind {
        EventKind::Custom {
            event_type: event_type.to_owned(),
            data: serde_json::json!({}),
        }
    }

    #[test]
    fn fold_strategy_drift_detected_increments_counter() {
        let state = default_state();
        let kind = strategy_event("strategy.drift_detected");
        let new_state = fold(state, &kind, 1, 5000);
        assert_eq!(new_state.strategy.drift_alerts, 1);
        assert_eq!(new_state.strategy.last_strategy_event_ms, 5000);
    }

    #[test]
    fn fold_strategy_decision_logged_increments_counter() {
        let state = default_state();
        let kind = strategy_event("strategy.decision_logged");
        let new_state = fold(state, &kind, 1, 6000);
        assert_eq!(new_state.strategy.decisions_logged, 1);
        assert_eq!(new_state.strategy.last_strategy_event_ms, 6000);
    }

    #[test]
    fn fold_strategy_critique_completed_increments_counter() {
        let state = default_state();
        let kind = strategy_event("strategy.critique_completed");
        let new_state = fold(state, &kind, 1, 7000);
        assert_eq!(new_state.strategy.critiques_completed, 1);
        assert_eq!(new_state.strategy.last_strategy_event_ms, 7000);
    }

    #[test]
    fn fold_multiple_strategy_drift_events_accumulate() {
        let mut state = default_state();
        for i in 0..5 {
            let kind = strategy_event("strategy.drift_detected");
            state = fold(state, &kind, i, (i + 1) * 1000);
        }
        assert_eq!(state.strategy.drift_alerts, 5);
        assert_eq!(state.strategy.last_strategy_event_ms, 5000);
    }

    #[test]
    fn fold_unknown_strategy_event_updates_timestamp_only() {
        let state = default_state();
        let kind = strategy_event("strategy.unknown_subtype");
        let new_state = fold(state, &kind, 1, 8000);
        assert_eq!(new_state.strategy.drift_alerts, 0);
        assert_eq!(new_state.strategy.decisions_logged, 0);
        assert_eq!(new_state.strategy.critiques_completed, 0);
        assert_eq!(new_state.strategy.last_strategy_event_ms, 8000);
    }

    #[test]
    fn fold_strategy_events_do_not_affect_other_state() {
        let state = default_state();
        let kind = strategy_event("strategy.drift_detected");
        let new_state = fold(state, &kind, 1, 5000);
        // Operational, cognitive, economic state should be untouched
        assert_eq!(new_state.operational.error_streak, 0);
        assert_eq!(new_state.operational.total_errors, 0);
        assert_eq!(new_state.cognitive.total_tokens_used, 0);
        assert_eq!(
            new_state.economic.mode,
            autonomic_core::EconomicMode::Sovereign
        );
    }

    #[test]
    fn fold_drift_events_then_strategy_rule_fires() {
        use crate::strategy_rules::StrategyRule;
        use autonomic_core::rules::HomeostaticRule;

        // Fold 4 drift events into state (threshold default is 3)
        let mut state = default_state();
        for i in 0..4 {
            let kind = strategy_event("strategy.drift_detected");
            state = fold(state, &kind, i, (i + 1) * 1000);
        }

        // Evaluate the strategy rule against the projected state
        let rule = StrategyRule::default();
        let decision = rule.evaluate(&state).expect("rule should fire");
        assert!(decision.rationale.contains("drift alerts"));
        assert!(decision.rationale.contains("reviewing setpoints"));
        // Verify it's advisory-only
        assert!(decision.economic_mode.is_none());
        assert!(decision.restrict_side_effects.is_none());
    }

    #[test]
    fn fold_mixed_strategy_events_then_rule_produces_combined_rationale() {
        use crate::strategy_rules::StrategyRule;
        use autonomic_core::rules::HomeostaticRule;

        let mut state = default_state();
        // 5 drift events, 12 decisions, 2 critiques
        for i in 0..5 {
            state = fold(
                state,
                &strategy_event("strategy.drift_detected"),
                i,
                i * 100,
            );
        }
        for i in 5..17 {
            state = fold(
                state,
                &strategy_event("strategy.decision_logged"),
                i,
                i * 100,
            );
        }
        for i in 17..19 {
            state = fold(
                state,
                &strategy_event("strategy.critique_completed"),
                i,
                i * 100,
            );
        }

        let rule = StrategyRule::default();
        let decision = rule.evaluate(&state).expect("rule should fire");
        assert!(decision.rationale.contains("drift alerts"));
        assert!(decision.rationale.contains("high decision velocity"));
        assert!(decision.rationale.contains("critiques completed"));
    }

    // ── Eval event tests ──

    fn eval_event(event_type: &str, data: serde_json::Value) -> EventKind {
        EventKind::Custom {
            event_type: event_type.to_owned(),
            data,
        }
    }

    #[test]
    fn fold_eval_inline_completed_updates_quality() {
        let state = default_state();
        let kind = eval_event(
            "eval.InlineCompleted",
            serde_json::json!({
                "evaluator": "token_efficiency",
                "score": 0.8,
                "label": "good",
                "layer": "execution",
                "session_id": "s1"
            }),
        );
        let new_state = fold(state, &kind, 1, 5000);
        assert_eq!(new_state.eval.inline_eval_count, 1);
        assert!(new_state.eval.aggregate_quality_score < 1.0); // EMA moved from 1.0 toward 0.8
        assert_eq!(new_state.eval.last_eval_ms, 5000);
    }

    #[test]
    fn fold_eval_async_completed_updates_quality() {
        let state = default_state();
        let kind = eval_event(
            "eval.AsyncCompleted",
            serde_json::json!({
                "evaluator": "plan_quality",
                "scores": [
                    {"evaluator": "plan_quality", "value": 0.7, "label": "warning", "layer": "reasoning"}
                ],
                "session_id": "s1",
                "duration_ms": 500
            }),
        );
        let new_state = fold(state, &kind, 1, 6000);
        assert_eq!(new_state.eval.async_eval_count, 1);
        assert!(new_state.eval.aggregate_quality_score < 1.0);
        assert_eq!(new_state.eval.last_eval_ms, 6000);
    }

    #[test]
    fn fold_eval_quality_changed_sets_directly() {
        let state = default_state();
        let kind = eval_event(
            "eval.QualityChanged",
            serde_json::json!({
                "session_id": "s1",
                "aggregate_quality": 0.72,
                "trend": -0.05,
                "inline_count": 10,
                "async_count": 2
            }),
        );
        let new_state = fold(state, &kind, 1, 7000);
        assert!((new_state.eval.aggregate_quality_score - 0.72).abs() < f64::EPSILON);
    }

    #[test]
    fn fold_eval_events_then_quality_rule_fires() {
        use crate::eval_rules::EvalQualityRule;
        use autonomic_core::rules::HomeostaticRule;

        // Fold multiple low-quality inline eval events to bring quality down.
        let mut state = default_state();
        for i in 0..10 {
            let kind = eval_event(
                "eval.InlineCompleted",
                serde_json::json!({
                    "evaluator": "tool_correctness",
                    "score": 0.3,
                    "label": "critical",
                    "layer": "action",
                    "session_id": "s1"
                }),
            );
            state = fold(state, &kind, i, (i + 1) * 1000);
        }

        // Quality should have degraded significantly via EMA.
        assert!(state.eval.aggregate_quality_score < 0.5);

        let rule = EvalQualityRule::default();
        let decision = rule.evaluate(&state).expect("rule should fire");
        assert!(
            decision.rationale.contains("critically low")
                || decision.rationale.contains("below warning")
        );
    }

    #[test]
    fn fold_eval_events_do_not_affect_other_state() {
        let state = default_state();
        let kind = eval_event(
            "eval.InlineCompleted",
            serde_json::json!({
                "evaluator": "test",
                "score": 0.5,
                "label": "warning",
                "layer": "execution",
                "session_id": "s1"
            }),
        );
        let new_state = fold(state, &kind, 1, 5000);
        // Other state pillars should be untouched.
        assert_eq!(new_state.operational.error_streak, 0);
        assert_eq!(new_state.cognitive.total_tokens_used, 0);
        assert_eq!(new_state.strategy.drift_alerts, 0);
    }
}
