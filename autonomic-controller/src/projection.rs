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

        // ── Typed knowledge operations ──
        EventKind::KnowledgeSearched { .. } => {
            state.cognitive.knowledge_search_count += 1;
        }

        EventKind::KnowledgeRetrieved { context_tokens, .. } => {
            state.cognitive.total_tokens_used += u64::from(*context_tokens);
        }

        EventKind::KnowledgeEvaluated {
            health_score,
            note_count,
            ..
        } => {
            state.cognitive.knowledge_health = *health_score;
            state.cognitive.knowledge_note_count = *note_count;
            state.cognitive.knowledge_last_indexed_ms = ts_ms;
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
            } else if event_type.starts_with(ANIMA_EVENT_PREFIX) {
                apply_anima_event(&mut state, event_type, data, ts_ms);
            } else if event_type.starts_with(STRATEGY_EVENT_PREFIX) {
                apply_strategy_event(&mut state, event_type, ts_ms);
            } else if event_type.starts_with(EVAL_EVENT_PREFIX) {
                apply_eval_event(&mut state, event_type, data, ts_ms);
            } else if event_type.starts_with(KNOWLEDGE_EVENT_PREFIX) {
                apply_knowledge_event(&mut state, event_type, data, ts_ms);
            }
        }

        // ── Memory lifecycle events ──
        EventKind::ObservationAppended { .. } => {
            state.cognitive.observation_count += 1;
        }

        EventKind::MemoryCommitted { .. } => {
            state.cognitive.memory_commit_count += 1;
        }

        EventKind::ReflectionCompacted { .. } => {
            state.cognitive.compaction_count += 1;
            // Compaction reduces context pressure — reset stale-context counter.
            state.cognitive.turns_since_compact = 0;
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

/// Prefix for Anima belief events emitted to Lago.
const ANIMA_EVENT_PREFIX: &str = "anima.";

/// Prefix for strategy custom events emitted by strategy skills to Lago.
const STRATEGY_EVENT_PREFIX: &str = "strategy.";

/// Prefix for evaluation events emitted by Nous to Lago.
const EVAL_EVENT_PREFIX: &str = "eval.";

/// Prefix for knowledge events from lago-knowledge.
const KNOWLEDGE_EVENT_PREFIX: &str = "knowledge.";

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

/// Apply an Anima belief event to state.
///
/// Anima events arrive as `EventKind::Custom` with `"anima."` prefix.
/// They update the `BeliefState` pillar with capability, trust, reputation,
/// and violation tracking. Economic belief updates also feed into the
/// economic pillar's balance.
fn apply_anima_event(
    state: &mut HomeostaticState,
    event_type: &str,
    data: &serde_json::Value,
    ts_ms: u64,
) {
    state.belief.last_belief_event_ms = ts_ms;

    match event_type {
        "anima.capability_granted" => {
            state.belief.capability_count += 1;
        }
        "anima.capability_revoked" => {
            state.belief.capability_count = state.belief.capability_count.saturating_sub(1);
        }
        "anima.trust_updated" => {
            if let Some(new_score) = data.get("new_score").and_then(serde_json::Value::as_f64) {
                // Recompute trust metrics incrementally.
                // We track peer count and running averages.
                let interaction_success = data
                    .get("interaction_success")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(true);

                if interaction_success {
                    // New peer or existing peer — update count if new.
                    // We can't perfectly track unique peers without a set,
                    // so we use the peer_count as reported or infer from the event.
                    // For a pure fold, we re-derive from the running stats.
                    let old_count = state.belief.trust_peer_count;
                    let is_new_peer = data.get("peer_id").is_some() && old_count == 0;

                    if is_new_peer || old_count == 0 {
                        state.belief.trust_peer_count = 1;
                        state.belief.average_trust = new_score;
                        state.belief.min_trust = new_score;
                    } else {
                        // Update running average: assume this is an update to
                        // an existing peer (conservative — avoids inflating count).
                        // The average moves toward new_score using EMA.
                        let alpha = 1.0 / f64::from(old_count + 1);
                        state.belief.average_trust =
                            alpha * new_score + (1.0 - alpha) * state.belief.average_trust;
                        if new_score < state.belief.min_trust {
                            state.belief.min_trust = new_score;
                        }
                    }
                } else {
                    // Failed interaction — trust decreased.
                    let old_count = state.belief.trust_peer_count;
                    if old_count == 0 {
                        state.belief.trust_peer_count = 1;
                        state.belief.average_trust = new_score;
                        state.belief.min_trust = new_score;
                    } else {
                        let alpha = 1.0 / f64::from(old_count + 1);
                        state.belief.average_trust =
                            alpha * new_score + (1.0 - alpha) * state.belief.average_trust;
                        if new_score < state.belief.min_trust {
                            state.belief.min_trust = new_score;
                        }
                    }
                }

                // Update reputation based on trust trend.
                // Reputation is a moving average of overall trust health.
                let total_interactions = state.belief.trust_peer_count;
                if total_interactions > 0 {
                    state.belief.reputation_score = state.belief.average_trust;
                }
            }
        }
        "anima.policy_violation_detected" => {
            state.belief.violations += 1;
            // Degrade reputation on violations.
            state.belief.reputation_score = (state.belief.reputation_score - 0.05).max(0.0);
        }
        "anima.economic_belief_updated" => {
            // Feed economic belief updates into the economic pillar.
            if let Some(balance) = data
                .get("balance_micro_credits")
                .and_then(serde_json::Value::as_i64)
            {
                state.economic.balance_micro_credits = balance;
            }
        }
        _ => {
            // Unknown anima event subtype — timestamp updated above.
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

/// Apply knowledge lifecycle events to cognitive state.
///
/// Knowledge events arrive as `EventKind::Custom` with `"knowledge."` prefix.
/// They update the memory/knowledge regulation fields in `CognitiveState`.
fn apply_knowledge_event(
    state: &mut HomeostaticState,
    event_type: &str,
    data: &serde_json::Value,
    _ts_ms: u64,
) {
    match event_type {
        "knowledge.indexed" => {
            if let Some(count) = data.get("note_count").and_then(|v| v.as_u64()) {
                state.cognitive.knowledge_note_count = count as u32;
            }
            if let Some(health) = data.get("health_score").and_then(|v| v.as_f64()) {
                state.cognitive.knowledge_health = health as f32;
            }
            state.cognitive.knowledge_last_indexed_ms = state.last_event_ms;
        }
        "knowledge.searched" => {
            state.cognitive.knowledge_search_count += 1;
        }
        _ => {}
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

    // ── Anima belief event tests ──

    fn anima_event(event_type: &str, data: serde_json::Value) -> EventKind {
        EventKind::Custom {
            event_type: event_type.to_owned(),
            data,
        }
    }

    #[test]
    fn fold_anima_capability_granted_increments_count() {
        let state = default_state();
        let kind = anima_event(
            "anima.capability_granted",
            serde_json::json!({
                "type": "capability_granted",
                "capability": "chat:send",
                "granted_by": "server-1",
                "expires_at": null,
                "constraints": {}
            }),
        );
        let new_state = fold(state, &kind, 1, 5000);
        assert_eq!(new_state.belief.capability_count, 1);
        assert_eq!(new_state.belief.last_belief_event_ms, 5000);
    }

    #[test]
    fn fold_anima_capability_revoked_decrements_count() {
        let mut state = default_state();
        state.belief.capability_count = 3;
        let kind = anima_event(
            "anima.capability_revoked",
            serde_json::json!({
                "type": "capability_revoked",
                "capability": "chat:send",
                "revoked_by": "server-1",
                "reason": "expired"
            }),
        );
        let new_state = fold(state, &kind, 1, 6000);
        assert_eq!(new_state.belief.capability_count, 2);
    }

    #[test]
    fn fold_anima_capability_revoked_saturates_at_zero() {
        let state = default_state();
        assert_eq!(state.belief.capability_count, 0);
        let kind = anima_event(
            "anima.capability_revoked",
            serde_json::json!({
                "type": "capability_revoked",
                "capability": "chat:send",
                "revoked_by": "server-1",
                "reason": "expired"
            }),
        );
        let new_state = fold(state, &kind, 1, 6000);
        assert_eq!(new_state.belief.capability_count, 0);
    }

    #[test]
    fn fold_anima_trust_updated_tracks_trust() {
        let state = default_state();
        let kind = anima_event(
            "anima.trust_updated",
            serde_json::json!({
                "type": "trust_updated",
                "peer_id": "peer-1",
                "new_score": 0.8,
                "interaction_success": true
            }),
        );
        let new_state = fold(state, &kind, 1, 7000);
        assert_eq!(new_state.belief.trust_peer_count, 1);
        assert!((new_state.belief.average_trust - 0.8).abs() < f64::EPSILON);
        assert!((new_state.belief.min_trust - 0.8).abs() < f64::EPSILON);
        assert_eq!(new_state.belief.last_belief_event_ms, 7000);
    }

    #[test]
    fn fold_anima_trust_updated_failed_interaction() {
        let state = default_state();
        let kind = anima_event(
            "anima.trust_updated",
            serde_json::json!({
                "type": "trust_updated",
                "peer_id": "peer-1",
                "new_score": 0.3,
                "interaction_success": false
            }),
        );
        let new_state = fold(state, &kind, 1, 7000);
        assert_eq!(new_state.belief.trust_peer_count, 1);
        assert!((new_state.belief.average_trust - 0.3).abs() < f64::EPSILON);
        assert!((new_state.belief.min_trust - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn fold_anima_policy_violation_increments_and_degrades_reputation() {
        let state = default_state();
        let kind = anima_event(
            "anima.policy_violation_detected",
            serde_json::json!({
                "type": "policy_violation_detected",
                "capability": "shell:exec",
                "reason": "exceeds ceiling",
                "blocked": true
            }),
        );
        let new_state = fold(state, &kind, 1, 8000);
        assert_eq!(new_state.belief.violations, 1);
        // Reputation should have decreased from 1.0 by 0.05.
        assert!((new_state.belief.reputation_score - 0.95).abs() < f64::EPSILON);
        assert_eq!(new_state.belief.last_belief_event_ms, 8000);
    }

    #[test]
    fn fold_anima_economic_belief_updated_feeds_economic_state() {
        let state = default_state();
        let kind = anima_event(
            "anima.economic_belief_updated",
            serde_json::json!({
                "type": "economic_belief_updated",
                "balance_micro_credits": 5_000_000,
                "burn_rate_per_hour": 100.0,
                "economic_mode": "conserving"
            }),
        );
        let new_state = fold(state, &kind, 1, 9000);
        assert_eq!(new_state.economic.balance_micro_credits, 5_000_000);
        assert_eq!(new_state.belief.last_belief_event_ms, 9000);
    }

    #[test]
    fn fold_unknown_anima_event_updates_timestamp_only() {
        let state = default_state();
        let kind = anima_event(
            "anima.soul_genesis",
            serde_json::json!({
                "type": "soul_genesis",
                "soul": {},
                "soul_hash": "abc"
            }),
        );
        let new_state = fold(state, &kind, 1, 10000);
        assert_eq!(new_state.belief.capability_count, 0);
        assert_eq!(new_state.belief.violations, 0);
        assert_eq!(new_state.belief.last_belief_event_ms, 10000);
    }

    #[test]
    fn fold_anima_events_do_not_affect_other_state() {
        let state = default_state();
        let kind = anima_event(
            "anima.capability_granted",
            serde_json::json!({
                "type": "capability_granted",
                "capability": "chat:send",
                "granted_by": "server-1",
                "expires_at": null,
                "constraints": {}
            }),
        );
        let new_state = fold(state, &kind, 1, 5000);
        assert_eq!(new_state.operational.error_streak, 0);
        assert_eq!(new_state.cognitive.total_tokens_used, 0);
        assert_eq!(new_state.strategy.drift_alerts, 0);
        assert_eq!(new_state.eval.inline_eval_count, 0);
    }

    #[test]
    fn fold_multiple_violations_then_belief_rule_fires() {
        use crate::belief_rules::BeliefRule;
        use autonomic_core::rules::HomeostaticRule;

        let mut state = default_state();
        // Fold 5 policy violations — reputation drops from 1.0 by 0.05 each.
        for i in 0..5 {
            let kind = anima_event(
                "anima.policy_violation_detected",
                serde_json::json!({
                    "type": "policy_violation_detected",
                    "capability": "shell:exec",
                    "reason": "exceeds ceiling",
                    "blocked": true
                }),
            );
            state = fold(state, &kind, i, (i + 1) * 1000);
        }

        // After 5 violations: reputation = 1.0 - 5*0.05 = 0.75.
        // 0.75 > 0.5, so default threshold won't fire.
        // Need more violations to push below 0.5.
        for i in 5..15 {
            let kind = anima_event(
                "anima.policy_violation_detected",
                serde_json::json!({
                    "type": "policy_violation_detected",
                    "capability": "shell:exec",
                    "reason": "repeated violation",
                    "blocked": true
                }),
            );
            state = fold(state, &kind, i, (i + 1) * 1000);
        }

        assert_eq!(state.belief.violations, 15);
        assert!(state.belief.reputation_score < 0.5);

        let rule = BeliefRule::default();
        let decision = rule.evaluate(&state).expect("rule should fire");
        assert_eq!(decision.restrict_side_effects, Some(true));
        assert!(decision.rationale.contains("policy violation"));
    }

    // ── Memory event tests ──

    #[test]
    fn fold_observation_appended_increments_count() {
        let state = default_state();
        let kind = EventKind::ObservationAppended {
            scope: aios_protocol::MemoryScope::Session,
            observation_ref: aios_protocol::BlobHash::from_hex("abc123"),
            source_run_id: None,
        };
        let new_state = fold(state, &kind, 1, 5000);
        assert_eq!(new_state.cognitive.observation_count, 1);
    }

    #[test]
    fn fold_memory_committed_increments_count() {
        let state = default_state();
        let kind = EventKind::MemoryCommitted {
            scope: aios_protocol::MemoryScope::Session,
            memory_id: aios_protocol::MemoryId::from_string("mem-1".to_string()),
            committed_ref: aios_protocol::BlobHash::from_hex("def456"),
            supersedes: None,
        };
        let new_state = fold(state, &kind, 1, 6000);
        assert_eq!(new_state.cognitive.memory_commit_count, 1);
    }

    #[test]
    fn fold_reflection_compacted_resets_turns_since_compact() {
        let mut state = default_state();
        state.cognitive.turns_since_compact = 15;
        let kind = EventKind::ReflectionCompacted {
            scope: aios_protocol::MemoryScope::Session,
            summary_ref: aios_protocol::BlobHash::from_hex("ghi789"),
            covers_through_seq: 100,
        };
        let new_state = fold(state, &kind, 1, 7000);
        assert_eq!(new_state.cognitive.compaction_count, 1);
        assert_eq!(new_state.cognitive.turns_since_compact, 0);
    }

    #[test]
    fn fold_multiple_observations_accumulate() {
        let mut state = default_state();
        for i in 0..5 {
            let kind = EventKind::ObservationAppended {
                scope: aios_protocol::MemoryScope::Session,
                observation_ref: aios_protocol::BlobHash::from_hex(format!("obs{i}")),
                source_run_id: None,
            };
            state = fold(state, &kind, i, (i + 1) * 1000);
        }
        assert_eq!(state.cognitive.observation_count, 5);
    }

    // ── Knowledge event tests ──

    fn knowledge_event(event_type: &str, data: serde_json::Value) -> EventKind {
        EventKind::Custom {
            event_type: event_type.to_owned(),
            data,
        }
    }

    #[test]
    fn fold_knowledge_indexed_updates_health_and_count() {
        let state = default_state();
        let kind = knowledge_event(
            "knowledge.indexed",
            serde_json::json!({
                "note_count": 150,
                "health_score": 0.82
            }),
        );
        let new_state = fold(state, &kind, 1, 5000);
        assert_eq!(new_state.cognitive.knowledge_note_count, 150);
        assert!((new_state.cognitive.knowledge_health - 0.82).abs() < f32::EPSILON);
        assert_eq!(new_state.cognitive.knowledge_last_indexed_ms, 5000);
    }

    #[test]
    fn fold_knowledge_searched_increments_count() {
        let state = default_state();
        let kind = knowledge_event("knowledge.searched", serde_json::json!({}));
        let new_state = fold(state, &kind, 1, 6000);
        assert_eq!(new_state.cognitive.knowledge_search_count, 1);
    }

    #[test]
    fn fold_multiple_knowledge_searches_accumulate() {
        let mut state = default_state();
        for i in 0..5 {
            let kind = knowledge_event("knowledge.searched", serde_json::json!({}));
            state = fold(state, &kind, i, (i + 1) * 1000);
        }
        assert_eq!(state.cognitive.knowledge_search_count, 5);
    }

    #[test]
    fn fold_knowledge_indexed_partial_data() {
        // Only note_count, no health_score
        let state = default_state();
        let kind = knowledge_event("knowledge.indexed", serde_json::json!({ "note_count": 42 }));
        let new_state = fold(state, &kind, 1, 5000);
        assert_eq!(new_state.cognitive.knowledge_note_count, 42);
        // Health should remain at default (1.0) since no health_score provided
        assert!((new_state.cognitive.knowledge_health - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn fold_unknown_knowledge_event_no_change() {
        let state = default_state();
        let kind = knowledge_event("knowledge.unknown_subtype", serde_json::json!({}));
        let new_state = fold(state, &kind, 1, 5000);
        assert_eq!(new_state.cognitive.knowledge_note_count, 0);
        assert_eq!(new_state.cognitive.knowledge_search_count, 0);
    }

    #[test]
    fn fold_typed_knowledge_searched_increments_count() {
        let state = default_state();
        let kind = EventKind::KnowledgeSearched {
            query: "temporal validity".into(),
            result_count: 3,
            top_relevance: 5.2,
            duration_ms: 18,
        };
        let new_state = fold(state, &kind, 1, 6000);
        assert_eq!(new_state.cognitive.knowledge_search_count, 1);
    }

    #[test]
    fn fold_typed_knowledge_retrieved_counts_context_tokens() {
        let state = default_state();
        let kind = EventKind::KnowledgeRetrieved {
            note_count: 3,
            context_tokens: 44,
            source: "tool_search".into(),
        };
        let new_state = fold(state, &kind, 1, 6100);
        assert_eq!(new_state.cognitive.total_tokens_used, 44);
    }

    #[test]
    fn fold_typed_knowledge_evaluated_updates_health_and_count() {
        let state = default_state();
        let kind = EventKind::KnowledgeEvaluated {
            health_score: 0.82,
            note_count: 64,
            contradictions: 1,
            missing_pages: 2,
            orphans: 3,
        };
        let new_state = fold(state, &kind, 1, 6200);
        assert_eq!(new_state.cognitive.knowledge_note_count, 64);
        assert!((new_state.cognitive.knowledge_health - 0.82).abs() < f32::EPSILON);
        assert_eq!(new_state.cognitive.knowledge_last_indexed_ms, 6200);
    }

    #[test]
    fn fold_knowledge_events_do_not_affect_other_state() {
        let state = default_state();
        let kind = knowledge_event(
            "knowledge.indexed",
            serde_json::json!({
                "note_count": 100,
                "health_score": 0.5
            }),
        );
        let new_state = fold(state, &kind, 1, 5000);
        // Other state pillars should be untouched.
        assert_eq!(new_state.operational.error_streak, 0);
        assert_eq!(new_state.cognitive.total_tokens_used, 0);
        assert_eq!(new_state.strategy.drift_alerts, 0);
        assert_eq!(new_state.eval.inline_eval_count, 0);
    }

    #[test]
    fn fold_knowledge_events_then_health_rule_fires() {
        use crate::knowledge_rules::KnowledgeHealthRule;
        use autonomic_core::rules::HomeostaticRule;

        let mut state = default_state();

        // Report low knowledge health
        let kind = knowledge_event(
            "knowledge.indexed",
            serde_json::json!({
                "note_count": 100,
                "health_score": 0.45
            }),
        );
        state = fold(state, &kind, 1, 5000);

        let rule = KnowledgeHealthRule::default();
        let decision = rule.evaluate(&state).expect("rule should fire");
        assert!(decision.rationale.contains("knowledge health"));
        assert_eq!(decision.restrict_expensive_tools, Some(true)); // < 0.5 → restrict
    }

    #[test]
    fn fold_memory_bloat_then_health_rule_fires() {
        use crate::knowledge_rules::KnowledgeHealthRule;
        use autonomic_core::rules::HomeostaticRule;

        let mut state = default_state();

        // Fold 60 observations with no compaction
        for i in 0..60 {
            let kind = EventKind::ObservationAppended {
                scope: aios_protocol::MemoryScope::Session,
                observation_ref: aios_protocol::BlobHash::from_hex(format!("obs{i}")),
                source_run_id: None,
            };
            state = fold(state, &kind, i, (i + 1) * 1000);
        }

        // Also report some knowledge notes so health check doesn't mislead
        let kind = knowledge_event(
            "knowledge.indexed",
            serde_json::json!({
                "note_count": 50,
                "health_score": 0.95
            }),
        );
        state = fold(state, &kind, 60, 61_000);

        let rule = KnowledgeHealthRule::default();
        let decision = rule.evaluate(&state).expect("rule should fire");
        assert!(decision.rationale.contains("uncompacted observations"));
    }
}
