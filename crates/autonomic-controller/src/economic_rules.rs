//! Economic homeostasis rules.
//!
//! These rules evaluate the agent's economic state and produce gating
//! decisions that regulate spending behavior.

use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::{GatingDecision, HomeostaticRule};
use autonomic_core::{EconomicMode, ModelTier};

/// Survival rule: maps balance/burn ratio to `EconomicMode`.
///
/// Thresholds:
/// - Sovereign:  ratio >= 2.0
/// - Conserving: 1.0 <= ratio < 2.0
/// - Hustle:     0 < ratio < 1.0
/// - Hibernate:  balance <= 0
pub struct SurvivalRule;

impl SurvivalRule {
    pub fn new() -> Self {
        Self
    }
}

impl Default for SurvivalRule {
    fn default() -> Self {
        Self::new()
    }
}

impl HomeostaticRule for SurvivalRule {
    fn rule_id(&self) -> &str {
        "survival"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let ratio = state.economic.balance_to_burn_ratio();
        let balance = state.economic.balance_micro_credits;

        // Determine target mode without hysteresis for simplicity in the trait impl
        let target_mode = if balance <= 0 {
            EconomicMode::Hibernate
        } else if ratio < 1.0 {
            EconomicMode::Hustle
        } else if ratio < 2.0 {
            EconomicMode::Conserving
        } else {
            EconomicMode::Sovereign
        };

        if target_mode == state.economic.mode {
            return None; // No change needed
        }

        let (preferred_model, allow_expensive) = match target_mode {
            EconomicMode::Sovereign => (None, true),
            EconomicMode::Conserving => (Some(ModelTier::Standard), true),
            EconomicMode::Hustle => (Some(ModelTier::Budget), false),
            EconomicMode::Hibernate => (Some(ModelTier::Budget), false),
        };

        Some(GatingDecision {
            rule_id: self.rule_id().into(),
            economic_mode: Some(target_mode),
            preferred_model,
            restrict_expensive_tools: Some(!allow_expensive),
            rationale: format!("balance/burn ratio {ratio:.2} → mode {:?}", target_mode),
            ..GatingDecision::noop(self.rule_id())
        })
    }
}

/// Spend velocity rule: triggers when cost in the last 5 minutes exceeds threshold.
pub struct SpendVelocityRule {
    /// Threshold in micro-credits for 5-minute window.
    pub threshold_5min: i64,
}

impl SpendVelocityRule {
    pub fn new(threshold_5min: i64) -> Self {
        Self { threshold_5min }
    }
}

impl Default for SpendVelocityRule {
    fn default() -> Self {
        Self::new(500_000) // 0.5 credits per 5 min
    }
}

impl HomeostaticRule for SpendVelocityRule {
    fn rule_id(&self) -> &str {
        "spend_velocity"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        if state.economic.cost_last_5min > self.threshold_5min {
            Some(GatingDecision {
                rule_id: self.rule_id().into(),
                preferred_model: Some(ModelTier::Budget),
                max_tokens_next_turn: Some(2048),
                rationale: format!(
                    "spend velocity {}mc/5min exceeds threshold {}mc",
                    state.economic.cost_last_5min, self.threshold_5min
                ),
                ..GatingDecision::noop(self.rule_id())
            })
        } else {
            None
        }
    }
}

/// Budget exhaustion rule: triggers when remaining budget is low.
pub struct BudgetExhaustionRule {
    /// Fraction of budget remaining that triggers conservative mode (0.0-1.0).
    pub threshold_fraction: f64,
}

impl BudgetExhaustionRule {
    pub fn new(threshold_fraction: f64) -> Self {
        Self { threshold_fraction }
    }
}

impl Default for BudgetExhaustionRule {
    fn default() -> Self {
        Self::new(0.2) // 20% remaining
    }
}

impl HomeostaticRule for BudgetExhaustionRule {
    fn rule_id(&self) -> &str {
        "budget_exhaustion"
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
                preferred_model: Some(ModelTier::Budget),
                max_tokens_next_turn: Some(1024),
                restrict_expensive_tools: Some(true),
                rationale: format!(
                    "budget {:.0}% remaining (threshold {:.0}%)",
                    remaining_fraction * 100.0,
                    self.threshold_fraction * 100.0
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
    use autonomic_core::economic::EconomicState;
    use autonomic_core::gating::HomeostaticState;

    fn state_with_economics(balance: i64, burn: i64, mode: EconomicMode) -> HomeostaticState {
        let mut state = HomeostaticState::for_agent("test");
        state.economic = EconomicState {
            balance_micro_credits: balance,
            monthly_burn_estimate: burn,
            mode,
            ..Default::default()
        };
        state
    }

    #[test]
    fn survival_rule_sovereign_no_change() {
        let rule = SurvivalRule::new();
        let state = state_with_economics(3_000_000, 1_000_000, EconomicMode::Sovereign);
        assert!(rule.evaluate(&state).is_none()); // ratio 3.0, already Sovereign
    }

    #[test]
    fn survival_rule_triggers_conserving() {
        let rule = SurvivalRule::new();
        let state = state_with_economics(1_500_000, 1_000_000, EconomicMode::Sovereign);
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.economic_mode, Some(EconomicMode::Conserving));
    }

    #[test]
    fn survival_rule_triggers_hustle() {
        let rule = SurvivalRule::new();
        let state = state_with_economics(500_000, 1_000_000, EconomicMode::Sovereign);
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.economic_mode, Some(EconomicMode::Hustle));
    }

    #[test]
    fn survival_rule_triggers_hibernate() {
        let rule = SurvivalRule::new();
        let state = state_with_economics(0, 1_000_000, EconomicMode::Hustle);
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.economic_mode, Some(EconomicMode::Hibernate));
    }

    #[test]
    fn spend_velocity_rule_below_threshold() {
        let rule = SpendVelocityRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.economic.cost_last_5min = 100_000;
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn spend_velocity_rule_above_threshold() {
        let rule = SpendVelocityRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.economic.cost_last_5min = 600_000;
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.preferred_model, Some(ModelTier::Budget));
        assert_eq!(decision.max_tokens_next_turn, Some(2048));
    }

    #[test]
    fn budget_exhaustion_rule_plenty_remaining() {
        let rule = BudgetExhaustionRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.total_tokens_used = 10_000;
        state.cognitive.tokens_remaining = 110_000;
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn budget_exhaustion_rule_low_remaining() {
        let rule = BudgetExhaustionRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.total_tokens_used = 100_000;
        state.cognitive.tokens_remaining = 10_000; // ~9% remaining
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.preferred_model, Some(ModelTier::Budget));
        assert_eq!(decision.restrict_expensive_tools, Some(true));
    }
}
