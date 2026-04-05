//! Operational homeostasis rules.
//!
//! These rules monitor error streaks, error rates, and operational
//! health to override operating modes and restrict dangerous actions.

use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::{GatingDecision, HomeostaticRule};

/// Error streak rule: when the agent hits too many consecutive errors,
/// restrict side effects and suggest mode change.
pub struct ErrorStreakRule {
    /// Error rate threshold (errors / total) above which the rule fires.
    pub error_rate_threshold: f64,
    /// Minimum events before the rule can fire (avoid false positives).
    pub min_events: u32,
}

impl ErrorStreakRule {
    pub fn new(error_rate_threshold: f64, min_events: u32) -> Self {
        Self {
            error_rate_threshold,
            min_events,
        }
    }
}

impl Default for ErrorStreakRule {
    fn default() -> Self {
        Self::new(0.3, 5) // 30% error rate after at least 5 events
    }
}

impl HomeostaticRule for ErrorStreakRule {
    fn rule_id(&self) -> &str {
        "error_streak"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let total = state.operational.total_errors + state.operational.total_successes;
        if total < self.min_events {
            return None;
        }

        let error_rate = state.operational.total_errors as f64 / total as f64;

        if error_rate > self.error_rate_threshold {
            Some(GatingDecision {
                rule_id: self.rule_id().into(),
                restrict_side_effects: Some(true),
                max_tool_calls_per_tick: Some(3),
                rationale: format!(
                    "error rate {:.0}% ({}/{}) exceeds threshold {:.0}%",
                    error_rate * 100.0,
                    state.operational.total_errors,
                    total,
                    self.error_rate_threshold * 100.0
                ),
                ..GatingDecision::noop(self.rule_id())
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_streak_below_min_events() {
        let rule = ErrorStreakRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.operational.total_errors = 2;
        state.operational.total_successes = 1;
        // Only 3 total events, min is 5
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn error_streak_below_threshold() {
        let rule = ErrorStreakRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.operational.total_errors = 1;
        state.operational.total_successes = 9;
        // 10% error rate, threshold is 30%
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn error_streak_above_threshold() {
        let rule = ErrorStreakRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.operational.total_errors = 4;
        state.operational.total_successes = 6;
        // 40% error rate, threshold is 30%
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.restrict_side_effects, Some(true));
        assert_eq!(decision.max_tool_calls_per_tick, Some(3));
    }
}
