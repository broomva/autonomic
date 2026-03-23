//! Belief-aware homeostatic rules.
//!
//! These rules monitor the `BeliefState` (populated from Anima `anima.*` events)
//! and produce gating decisions when trust or reputation degrades, or when
//! policy violations accumulate.
//!
//! The rule has two tiers:
//! - **Advisory**: low trust or low reputation triggers a rationale note
//!   and suggests switching to a budget model / capping tokens.
//! - **Restrictive**: violations combined with degraded reputation restrict
//!   side effects to prevent damage from an untrusted agent.

use autonomic_core::economic::ModelTier;
use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::{GatingDecision, HomeostaticRule};

/// Rule that monitors belief state (trust, reputation, violations) and
/// gates behavior when the agent's trust network or policy compliance
/// degrades.
pub struct BeliefRule {
    /// Average trust below which advisory action triggers.
    pub low_trust_threshold: f64,
    /// Reputation below which violations trigger restriction.
    pub low_reputation_threshold: f64,
    /// Token cap to suggest when trust is low.
    pub low_trust_token_cap: u32,
}

impl Default for BeliefRule {
    fn default() -> Self {
        Self {
            low_trust_threshold: 0.3,
            low_reputation_threshold: 0.5,
            low_trust_token_cap: 2048,
        }
    }
}

impl HomeostaticRule for BeliefRule {
    fn rule_id(&self) -> &str {
        "belief_regulation"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let belief = &state.belief;

        // Tier 1 (restrictive): violations + degraded reputation → restrict side effects.
        if belief.violations > 0 && belief.reputation_score < self.low_reputation_threshold {
            return Some(GatingDecision {
                rule_id: self.rule_id().into(),
                restrict_side_effects: Some(true),
                preferred_model: Some(ModelTier::Budget),
                max_tokens_next_turn: Some(self.low_trust_token_cap),
                rationale: format!(
                    "belief: {} policy violation(s) with reputation {:.2} (< {:.2}), \
                     restricting side effects",
                    belief.violations, belief.reputation_score, self.low_reputation_threshold
                ),
                ..GatingDecision::noop(self.rule_id())
            });
        }

        // Tier 2 (advisory): low average trust → suggest budget model + cap tokens.
        // Only fires when there are peers to evaluate (trust_peer_count > 0).
        if belief.trust_peer_count > 0 && belief.average_trust < self.low_trust_threshold {
            return Some(GatingDecision {
                rule_id: self.rule_id().into(),
                preferred_model: Some(ModelTier::Budget),
                max_tokens_next_turn: Some(self.low_trust_token_cap),
                rationale: format!(
                    "belief: average trust {:.2} across {} peer(s) is below {:.2}, \
                     suggesting budget model and token cap",
                    belief.average_trust, belief.trust_peer_count, self.low_trust_threshold
                ),
                ..GatingDecision::noop(self.rule_id())
            });
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_belief(
        violations: u64,
        reputation: f64,
        avg_trust: f64,
        peer_count: u32,
    ) -> HomeostaticState {
        let mut state = HomeostaticState::for_agent("test");
        state.belief.violations = violations;
        state.belief.reputation_score = reputation;
        state.belief.average_trust = avg_trust;
        state.belief.trust_peer_count = peer_count;
        state
    }

    #[test]
    fn clean_state_does_not_fire() {
        let rule = BeliefRule::default();
        let state = HomeostaticState::for_agent("test");
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn violations_with_low_reputation_restricts_side_effects() {
        let rule = BeliefRule::default();
        let state = state_with_belief(2, 0.3, 0.8, 3);
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.restrict_side_effects, Some(true));
        assert_eq!(decision.preferred_model, Some(ModelTier::Budget));
        assert_eq!(decision.max_tokens_next_turn, Some(2048));
        assert!(decision.rationale.contains("policy violation"));
        assert!(decision.rationale.contains("restricting side effects"));
    }

    #[test]
    fn violations_with_high_reputation_does_not_fire() {
        let rule = BeliefRule::default();
        // 1 violation but reputation still good (0.7 > 0.5)
        let state = state_with_belief(1, 0.7, 0.8, 3);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn zero_violations_with_low_reputation_does_not_restrict() {
        let rule = BeliefRule::default();
        // Reputation is low but no violations — tier 1 should not fire.
        let state = state_with_belief(0, 0.3, 0.8, 3);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn low_trust_with_peers_suggests_budget() {
        let rule = BeliefRule::default();
        let state = state_with_belief(0, 0.9, 0.2, 5);
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.preferred_model, Some(ModelTier::Budget));
        assert_eq!(decision.max_tokens_next_turn, Some(2048));
        assert!(decision.rationale.contains("average trust"));
        assert!(decision.rationale.contains("budget model"));
        // Advisory — no side-effect restriction.
        assert!(decision.restrict_side_effects.is_none());
    }

    #[test]
    fn low_trust_without_peers_does_not_fire() {
        let rule = BeliefRule::default();
        // No peers means trust metrics are meaningless.
        let state = state_with_belief(0, 0.9, 0.2, 0);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn violations_take_priority_over_low_trust() {
        let rule = BeliefRule::default();
        // Both conditions true — violations + low reputation fires first.
        let state = state_with_belief(3, 0.2, 0.1, 5);
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.restrict_side_effects, Some(true));
        assert!(decision.rationale.contains("policy violation"));
    }

    #[test]
    fn custom_thresholds() {
        let rule = BeliefRule {
            low_trust_threshold: 0.5,
            low_reputation_threshold: 0.8,
            low_trust_token_cap: 1024,
        };
        // Reputation 0.6 < 0.8 threshold, with violations → restrict.
        let state = state_with_belief(1, 0.6, 0.9, 3);
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.restrict_side_effects, Some(true));
        assert_eq!(decision.max_tokens_next_turn, Some(1024));
    }

    #[test]
    fn boundary_values() {
        let rule = BeliefRule::default();

        // Exactly at low_reputation_threshold with violations — should NOT fire tier 1
        // (< is strict, 0.5 is not < 0.5).
        let state = state_with_belief(1, 0.5, 0.8, 3);
        assert!(rule.evaluate(&state).is_none());

        // Just below reputation threshold with violations — fires.
        let state = state_with_belief(1, 0.49, 0.8, 3);
        assert!(rule.evaluate(&state).is_some());

        // Exactly at trust threshold with peers — should NOT fire tier 2.
        let state = state_with_belief(0, 0.9, 0.3, 3);
        assert!(rule.evaluate(&state).is_none());

        // Just below trust threshold — fires.
        let state = state_with_belief(0, 0.9, 0.29, 3);
        assert!(rule.evaluate(&state).is_some());
    }
}
