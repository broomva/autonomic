//! Strategy advisory rules.
//!
//! These rules monitor strategy event counters (drift alerts, decision
//! velocity, critiques) and produce **advisory-only** gating decisions.
//! They add rationale to the gating profile but never restrict gating
//! or change economic modes.

use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::{GatingDecision, HomeostaticRule};

/// Strategy rule: monitors drift alerts and decision velocity to produce
/// soft advisory notes in the gating rationale.
///
/// This rule is advisory-only — it never restricts tools, side effects,
/// or changes economic modes. Its decisions carry only a rationale string
/// that downstream consumers can use for logging or UI display.
pub struct StrategyRule {
    /// Number of drift alerts above which the rule suggests a setpoint review.
    pub drift_alert_threshold: u32,
    /// Number of decisions per evaluation window above which the rule notes
    /// high decision velocity.
    pub decision_velocity_threshold: u32,
}

impl StrategyRule {
    pub fn new(drift_alert_threshold: u32, decision_velocity_threshold: u32) -> Self {
        Self {
            drift_alert_threshold,
            decision_velocity_threshold,
        }
    }
}

impl Default for StrategyRule {
    fn default() -> Self {
        Self::new(3, 10)
    }
}

impl HomeostaticRule for StrategyRule {
    fn rule_id(&self) -> &str {
        "strategy_advisory"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let mut notes: Vec<String> = Vec::new();

        // Frequent drift alerts → suggest reviewing setpoints
        if state.strategy.drift_alerts > self.drift_alert_threshold {
            notes.push(format!(
                "strategy: {} drift alerts detected (threshold {}), consider reviewing setpoints",
                state.strategy.drift_alerts, self.drift_alert_threshold
            ));
        }

        // High decision velocity → note for awareness
        if state.strategy.decisions_logged > self.decision_velocity_threshold {
            notes.push(format!(
                "strategy: high decision velocity ({} decisions logged, threshold {})",
                state.strategy.decisions_logged, self.decision_velocity_threshold
            ));
        }

        // Strategy critiques inform risk awareness
        if state.strategy.critiques_completed > 0 {
            notes.push(format!(
                "strategy: {} critiques completed — factoring into risk assessment",
                state.strategy.critiques_completed
            ));
        }

        if notes.is_empty() {
            return None;
        }

        // Advisory only — no gating overrides, just rationale
        Some(GatingDecision {
            rule_id: self.rule_id().into(),
            rationale: notes.join("; "),
            ..GatingDecision::noop(self.rule_id())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_strategy(drift: u32, decisions: u32, critiques: u32) -> HomeostaticState {
        let mut state = HomeostaticState::for_agent("test");
        state.strategy.drift_alerts = drift;
        state.strategy.decisions_logged = decisions;
        state.strategy.critiques_completed = critiques;
        state
    }

    #[test]
    fn no_strategy_events_does_not_fire() {
        let rule = StrategyRule::default();
        let state = HomeostaticState::for_agent("test");
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn below_thresholds_does_not_fire() {
        let rule = StrategyRule::default();
        let state = state_with_strategy(2, 5, 0);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn drift_alerts_above_threshold_fires() {
        let rule = StrategyRule::default();
        let state = state_with_strategy(4, 0, 0);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("drift alerts"));
        assert!(decision.rationale.contains("reviewing setpoints"));
        // Advisory only — no gating overrides
        assert!(decision.economic_mode.is_none());
        assert!(decision.restrict_side_effects.is_none());
        assert!(decision.restrict_expensive_tools.is_none());
        assert!(decision.max_tokens_next_turn.is_none());
        assert!(decision.max_tool_calls_per_tick.is_none());
    }

    #[test]
    fn high_decision_velocity_fires() {
        let rule = StrategyRule::default();
        let state = state_with_strategy(0, 15, 0);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("high decision velocity"));
        // Advisory only
        assert!(decision.economic_mode.is_none());
    }

    #[test]
    fn critiques_add_risk_note() {
        let rule = StrategyRule::default();
        let state = state_with_strategy(0, 0, 3);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("critiques completed"));
        assert!(decision.rationale.contains("risk assessment"));
    }

    #[test]
    fn multiple_conditions_combine_rationale() {
        let rule = StrategyRule::default();
        let state = state_with_strategy(5, 12, 2);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("drift alerts"));
        assert!(decision.rationale.contains("high decision velocity"));
        assert!(decision.rationale.contains("critiques completed"));
    }

    #[test]
    fn custom_thresholds_respected() {
        let rule = StrategyRule::new(1, 5);
        let state = state_with_strategy(2, 6, 0);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("drift alerts"));
        assert!(decision.rationale.contains("high decision velocity"));
    }

    #[test]
    fn advisory_does_not_restrict_gating() {
        let rule = StrategyRule::default();
        let state = state_with_strategy(10, 20, 5);
        let decision = rule.evaluate(&state).unwrap();
        // Even with extreme values, no gating overrides
        assert!(decision.economic_mode.is_none());
        assert!(decision.restrict_side_effects.is_none());
        assert!(decision.restrict_expensive_tools.is_none());
        assert!(decision.max_tokens_next_turn.is_none());
        assert!(decision.max_tool_calls_per_tick.is_none());
        assert!(decision.preferred_model.is_none());
    }
}
