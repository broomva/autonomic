//! Homeostatic rule trait and rule set.
//!
//! Rules are pure functions: given `HomeostaticState`, they produce
//! an optional `GatingDecision`. The controller evaluates all rules
//! and merges their decisions into a final `AutonomicGatingProfile`.

use serde::{Deserialize, Serialize};

use crate::economic::{EconomicMode, ModelTier};
use crate::events::AutonomicEvent;
use crate::gating::HomeostaticState;

/// A decision produced by a homeostatic rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatingDecision {
    /// Which rule produced this decision.
    pub rule_id: String,
    /// Whether to override the economic mode.
    pub economic_mode: Option<EconomicMode>,
    /// Whether to cap tokens for the next turn.
    pub max_tokens_next_turn: Option<u32>,
    /// Whether to suggest a model tier.
    pub preferred_model: Option<ModelTier>,
    /// Whether to restrict expensive tools.
    pub restrict_expensive_tools: Option<bool>,
    /// Whether to restrict side effects.
    pub restrict_side_effects: Option<bool>,
    /// Override for max tool calls per tick.
    pub max_tool_calls_per_tick: Option<u32>,
    /// Human-readable rationale.
    pub rationale: String,
    /// Advisory events that a controller may persist after evaluation.
    pub advisory_events: Vec<AutonomicEvent>,
}

impl GatingDecision {
    /// Create a no-op decision (used when a rule doesn't fire).
    pub fn noop(rule_id: impl Into<String>) -> Self {
        Self {
            rule_id: rule_id.into(),
            economic_mode: None,
            max_tokens_next_turn: None,
            preferred_model: None,
            restrict_expensive_tools: None,
            restrict_side_effects: None,
            max_tool_calls_per_tick: None,
            rationale: String::new(),
            advisory_events: Vec::new(),
        }
    }
}

/// A homeostatic rule that evaluates state and optionally produces a gating decision.
pub trait HomeostaticRule: Send + Sync {
    /// Unique identifier for this rule.
    fn rule_id(&self) -> &str;

    /// Evaluate the rule against the current homeostatic state.
    ///
    /// Returns `Some(decision)` if the rule fires, `None` if it doesn't apply.
    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision>;
}

/// An ordered collection of homeostatic rules.
pub struct RuleSet {
    rules: Vec<Box<dyn HomeostaticRule>>,
}

impl RuleSet {
    /// Create an empty rule set.
    pub fn new() -> Self {
        Self { rules: Vec::new() }
    }

    /// Add a rule to the set.
    pub fn add(&mut self, rule: Box<dyn HomeostaticRule>) {
        self.rules.push(rule);
    }

    /// Evaluate all rules and collect decisions.
    pub fn evaluate_all(&self, state: &HomeostaticState) -> Vec<GatingDecision> {
        self.rules
            .iter()
            .filter_map(|rule| rule.evaluate(state))
            .collect()
    }

    /// Number of rules in the set.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether the set is empty.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }
}

impl Default for RuleSet {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gating::HomeostaticState;

    struct AlwaysFireRule;

    impl HomeostaticRule for AlwaysFireRule {
        fn rule_id(&self) -> &str {
            "always_fire"
        }

        fn evaluate(&self, _state: &HomeostaticState) -> Option<GatingDecision> {
            Some(GatingDecision {
                rule_id: self.rule_id().into(),
                economic_mode: Some(EconomicMode::Conserving),
                rationale: "always fires".into(),
                ..GatingDecision::noop(self.rule_id())
            })
        }
    }

    struct NeverFireRule;

    impl HomeostaticRule for NeverFireRule {
        fn rule_id(&self) -> &str {
            "never_fire"
        }

        fn evaluate(&self, _state: &HomeostaticState) -> Option<GatingDecision> {
            None
        }
    }

    #[test]
    fn rule_set_evaluates_all() {
        let mut set = RuleSet::new();
        set.add(Box::new(AlwaysFireRule));
        set.add(Box::new(NeverFireRule));
        set.add(Box::new(AlwaysFireRule));

        let state = HomeostaticState::default();
        let decisions = set.evaluate_all(&state);
        assert_eq!(decisions.len(), 2);
    }

    #[test]
    fn rule_set_empty() {
        let set = RuleSet::new();
        assert!(set.is_empty());
        assert_eq!(set.len(), 0);
    }

    #[test]
    fn gating_decision_noop() {
        let decision = GatingDecision::noop("test");
        assert_eq!(decision.rule_id, "test");
        assert!(decision.economic_mode.is_none());
        assert!(decision.max_tokens_next_turn.is_none());
    }
}
