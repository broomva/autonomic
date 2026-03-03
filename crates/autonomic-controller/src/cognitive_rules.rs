//! Cognitive homeostasis rules.
//!
//! These rules monitor context pressure and token usage to prevent
//! context overflow and optimize model selection.

use autonomic_core::ModelTier;
use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::{GatingDecision, HomeostaticRule};

/// Context pressure rule: when context is filling up, suggest model downgrade.
///
/// High context pressure means the agent is consuming a large fraction
/// of available tokens. Downgrading to a cheaper/faster model or reducing
/// output length helps avoid hitting context limits.
pub struct ContextPressureRule {
    /// Pressure threshold above which the rule fires (0.0-1.0).
    pub threshold: f32,
}

impl ContextPressureRule {
    pub fn new(threshold: f32) -> Self {
        Self { threshold }
    }
}

impl Default for ContextPressureRule {
    fn default() -> Self {
        Self::new(0.8) // 80% context usage
    }
}

impl HomeostaticRule for ContextPressureRule {
    fn rule_id(&self) -> &str {
        "context_pressure"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        if state.cognitive.context_pressure > self.threshold {
            Some(GatingDecision {
                rule_id: self.rule_id().into(),
                preferred_model: Some(ModelTier::Standard),
                max_tokens_next_turn: Some(2048),
                rationale: format!(
                    "context pressure {:.0}% exceeds threshold {:.0}%",
                    state.cognitive.context_pressure * 100.0,
                    self.threshold * 100.0
                ),
                ..GatingDecision::noop(self.rule_id())
            })
        } else {
            None
        }
    }
}

/// Token exhaustion rule: when tokens remaining are critically low,
/// restrict tool calls to conserve budget.
pub struct TokenExhaustionRule {
    /// Fraction of tokens remaining below which the rule fires (0.0-1.0).
    pub threshold_fraction: f64,
    /// Maximum tool calls allowed when rule fires.
    pub max_tool_calls: u32,
}

impl TokenExhaustionRule {
    pub fn new(threshold_fraction: f64, max_tool_calls: u32) -> Self {
        Self {
            threshold_fraction,
            max_tool_calls,
        }
    }
}

impl Default for TokenExhaustionRule {
    fn default() -> Self {
        Self::new(0.1, 2) // 10% remaining → max 2 tool calls
    }
}

impl HomeostaticRule for TokenExhaustionRule {
    fn rule_id(&self) -> &str {
        "token_exhaustion"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let total = state.cognitive.total_tokens_used + state.cognitive.tokens_remaining;
        if total == 0 {
            return None;
        }

        let remaining_fraction = state.cognitive.tokens_remaining as f64 / total as f64;

        if remaining_fraction < self.threshold_fraction {
            Some(GatingDecision {
                rule_id: self.rule_id().into(),
                max_tool_calls_per_tick: Some(self.max_tool_calls),
                max_tokens_next_turn: Some(1024),
                rationale: format!(
                    "tokens {:.0}% remaining — limiting to {} tool calls",
                    remaining_fraction * 100.0,
                    self.max_tool_calls
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
    fn context_pressure_below_threshold() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.5;
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn context_pressure_above_threshold() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.85;
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.preferred_model, Some(ModelTier::Standard));
        assert_eq!(decision.max_tokens_next_turn, Some(2048));
    }

    #[test]
    fn token_exhaustion_plenty_remaining() {
        let rule = TokenExhaustionRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.total_tokens_used = 50_000;
        state.cognitive.tokens_remaining = 70_000;
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn token_exhaustion_low_remaining() {
        let rule = TokenExhaustionRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.total_tokens_used = 110_000;
        state.cognitive.tokens_remaining = 10_000; // ~8.3%
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.max_tool_calls_per_tick, Some(2));
        assert_eq!(decision.max_tokens_next_turn, Some(1024));
    }
}
