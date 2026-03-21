//! Evaluation quality rules.
//!
//! These rules monitor the `EvalState` (populated from Nous `eval.*` events)
//! and produce gating decisions when quality degrades below thresholds.

use autonomic_core::economic::ModelTier;
use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::{GatingDecision, HomeostaticRule};

/// Rule that fires when aggregate quality score drops below a threshold.
///
/// Unlike strategy rules (advisory-only), this rule can restrict tool usage
/// and suggest model tier changes when quality is critically low.
pub struct EvalQualityRule {
    /// Quality score below which the rule starts advising caution (0.0..1.0).
    pub warning_threshold: f64,
    /// Quality score below which the rule restricts expensive tools (0.0..1.0).
    pub critical_threshold: f64,
    /// Minimum number of evaluations before the rule can fire.
    pub min_eval_count: u32,
}

impl Default for EvalQualityRule {
    fn default() -> Self {
        Self {
            warning_threshold: 0.6,
            critical_threshold: 0.4,
            min_eval_count: 3,
        }
    }
}

impl HomeostaticRule for EvalQualityRule {
    fn rule_id(&self) -> &str {
        "eval_quality"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let total_evals = state.eval.inline_eval_count + state.eval.async_eval_count;
        if total_evals < self.min_eval_count {
            return None;
        }

        let quality = state.eval.aggregate_quality_score;

        if quality < self.critical_threshold {
            // Critical: restrict expensive tools, suggest cheaper model.
            Some(GatingDecision {
                rule_id: self.rule_id().into(),
                restrict_expensive_tools: Some(true),
                preferred_model: Some(ModelTier::Budget),
                max_tool_calls_per_tick: Some(3),
                rationale: format!(
                    "eval quality critically low ({quality:.2} < {:.2}), \
                     restricting tools and suggesting budget model",
                    self.critical_threshold
                ),
                ..GatingDecision::noop(self.rule_id())
            })
        } else if quality < self.warning_threshold {
            // Warning: advisory note, no restrictions.
            Some(GatingDecision {
                rule_id: self.rule_id().into(),
                rationale: format!(
                    "eval quality below warning threshold ({quality:.2} < {:.2}), \
                     trend: {:.3}",
                    self.warning_threshold, state.eval.quality_trend
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

    fn state_with_eval(quality: f64, inline: u32, async_count: u32) -> HomeostaticState {
        let mut state = HomeostaticState::for_agent("test");
        state.eval.aggregate_quality_score = quality;
        state.eval.inline_eval_count = inline;
        state.eval.async_eval_count = async_count;
        state.eval.quality_trend = 0.0;
        state
    }

    #[test]
    fn high_quality_does_not_fire() {
        let rule = EvalQualityRule::default();
        let state = state_with_eval(0.85, 5, 0);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn too_few_evals_does_not_fire() {
        let rule = EvalQualityRule::default();
        let state = state_with_eval(0.2, 1, 0);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn warning_quality_fires_advisory() {
        let rule = EvalQualityRule::default();
        let state = state_with_eval(0.5, 5, 0);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("below warning threshold"));
        // Advisory only — no tool restrictions.
        assert!(decision.restrict_expensive_tools.is_none());
        assert!(decision.preferred_model.is_none());
    }

    #[test]
    fn critical_quality_restricts_tools() {
        let rule = EvalQualityRule::default();
        let state = state_with_eval(0.3, 5, 2);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("critically low"));
        assert_eq!(decision.restrict_expensive_tools, Some(true));
        assert_eq!(decision.preferred_model, Some(ModelTier::Budget));
        assert_eq!(decision.max_tool_calls_per_tick, Some(3));
    }

    #[test]
    fn custom_thresholds() {
        let rule = EvalQualityRule {
            warning_threshold: 0.8,
            critical_threshold: 0.5,
            min_eval_count: 1,
        };
        let state = state_with_eval(0.7, 2, 0);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("below warning threshold"));
    }

    #[test]
    fn boundary_values() {
        let rule = EvalQualityRule::default();

        // Exactly at warning threshold — should NOT fire.
        let state = state_with_eval(0.6, 5, 0);
        assert!(rule.evaluate(&state).is_none());

        // Just below warning threshold — should fire.
        let state = state_with_eval(0.59, 5, 0);
        assert!(rule.evaluate(&state).is_some());

        // Exactly at critical threshold — should fire at warning level.
        let state = state_with_eval(0.4, 5, 0);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("below warning threshold"));

        // Just below critical — should fire at critical level.
        let state = state_with_eval(0.39, 5, 0);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("critically low"));
    }
}
