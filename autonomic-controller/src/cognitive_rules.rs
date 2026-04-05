//! Cognitive homeostasis rules.
//!
//! These rules monitor context pressure and token usage to prevent
//! context overflow and optimize model selection.
//!
//! `ContextPressureRule` uses multi-zone evaluation with quality-aware
//! soft zone to decide between Breathe, Dilate, Compress, and Emergency.

use autonomic_core::ModelTier;
use autonomic_core::context::{ContextCompressionAdvice, ContextRuling};
use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::{GatingDecision, HomeostaticRule};

/// Context pressure rule with multi-zone evaluation.
///
/// Zones:
/// - Below `soft_threshold` (0.60): Breathe — no action.
/// - `soft_threshold` to `hard_threshold` (0.60..0.85): Soft zone —
///   evaluate Nous quality, tool density, error streak to decide
///   Dilate vs. Compress.
/// - `hard_threshold` to `emergency_threshold` (0.85..0.95): Hard zone —
///   always Compress.
/// - Above `emergency_threshold` (0.95): Emergency — compact aggressively.
pub struct ContextPressureRule {
    /// Pressure below which no action is taken.
    pub soft_threshold: f32,
    /// Pressure above which compression is forced.
    pub hard_threshold: f32,
    /// Pressure above which emergency compaction triggers.
    pub emergency_threshold: f32,
    /// Target pressure after compression (as fraction of context window).
    pub target_fraction: f32,
    /// Target pressure after emergency compression.
    pub emergency_target_fraction: f32,
    /// Tool density above which the agent is considered to be in "deep work".
    pub deep_work_tool_density: f64,
    /// Error streak above which context is considered confused.
    pub error_streak_limit: u32,
    /// Turns since compact above which old context is considered stale.
    pub stale_turns_limit: u32,
}

impl Default for ContextPressureRule {
    fn default() -> Self {
        Self {
            soft_threshold: 0.60,
            hard_threshold: 0.85,
            emergency_threshold: 0.95,
            target_fraction: 0.35,
            emergency_target_fraction: 0.25,
            deep_work_tool_density: 2.0,
            error_streak_limit: 2,
            stale_turns_limit: 15,
        }
    }
}

impl ContextPressureRule {
    /// Create a rule with custom zone thresholds.
    pub fn new(soft: f32, hard: f32, emergency: f32) -> Self {
        Self {
            soft_threshold: soft,
            hard_threshold: hard,
            emergency_threshold: emergency,
            ..Default::default()
        }
    }

    /// Evaluate context pressure and return compression advice.
    ///
    /// This is the primary method — richer than `HomeostaticRule::evaluate`
    /// which can only return a `GatingDecision`.
    pub fn evaluate_compression(&self, state: &HomeostaticState) -> ContextCompressionAdvice {
        let pressure = state.cognitive.context_pressure;
        let total_ctx =
            (state.cognitive.tokens_remaining + state.cognitive.total_tokens_used) as f32;

        // Zone 1: Below soft threshold — breathe
        if pressure < self.soft_threshold {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Breathe,
                pressure,
                target_tokens: None,
                rationale: format!(
                    "pressure {:.0}% below soft threshold {:.0}%",
                    pressure * 100.0,
                    self.soft_threshold * 100.0
                ),
            };
        }

        // Zone 4: Emergency
        if pressure >= self.emergency_threshold {
            let target = (total_ctx * self.emergency_target_fraction) as usize;
            return ContextCompressionAdvice {
                ruling: ContextRuling::Emergency,
                pressure,
                target_tokens: Some(target),
                rationale: format!("pressure {:.0}% — emergency compaction", pressure * 100.0),
            };
        }

        // Zone 3: Hard zone — always compress
        if pressure >= self.hard_threshold {
            let target = (total_ctx * self.target_fraction) as usize;
            return ContextCompressionAdvice {
                ruling: ContextRuling::Compress,
                pressure,
                target_tokens: Some(target),
                rationale: format!(
                    "pressure {:.0}% exceeds hard threshold {:.0}%",
                    pressure * 100.0,
                    self.hard_threshold * 100.0
                ),
            };
        }

        // Zone 2: Soft zone (soft_threshold..hard_threshold) — evaluate signals
        let target = (total_ctx * self.target_fraction) as usize;

        // Signal 1: Error streak indicates confused context
        if state.operational.error_streak >= self.error_streak_limit {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Compress,
                pressure,
                target_tokens: Some(target),
                rationale: format!(
                    "pressure {:.0}% + {} consecutive errors — context may be confusing model",
                    pressure * 100.0,
                    state.operational.error_streak
                ),
            };
        }

        // Signal 2: Quality degrading — compress to help model focus
        if state.eval.quality_trend < -0.02 || state.eval.aggregate_quality_score < 0.65 {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Compress,
                pressure,
                target_tokens: Some(target),
                rationale: format!(
                    "pressure {:.0}% + quality degrading (score={:.2}, trend={:.3})",
                    pressure * 100.0,
                    state.eval.aggregate_quality_score,
                    state.eval.quality_trend
                ),
            };
        }

        // Signal 3: Stale context with low tool activity — compress
        if state.cognitive.turns_since_compact >= self.stale_turns_limit
            && state.cognitive.tool_density < self.deep_work_tool_density
        {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Compress,
                pressure,
                target_tokens: Some(target),
                rationale: format!(
                    "pressure {:.0}% + {} turns since compact (stale, low tool activity)",
                    pressure * 100.0,
                    state.cognitive.turns_since_compact
                ),
            };
        }

        // Signal 4: High tool density + good quality — dilate (deep work)
        if state.cognitive.tool_density >= self.deep_work_tool_density
            && state.eval.quality_trend >= 0.0
        {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Dilate,
                pressure,
                target_tokens: None,
                rationale: format!(
                    "pressure {:.0}% but deep work (tool_density={:.1}, quality stable) — dilating",
                    pressure * 100.0,
                    state.cognitive.tool_density
                ),
            };
        }

        // Signal 5: Quality stable/improving — dilate
        if state.eval.quality_trend >= 0.0 && state.eval.aggregate_quality_score >= 0.75 {
            return ContextCompressionAdvice {
                ruling: ContextRuling::Dilate,
                pressure,
                target_tokens: None,
                rationale: format!(
                    "pressure {:.0}% but quality good (score={:.2}, trend={:.3}) — dilating",
                    pressure * 100.0,
                    state.eval.aggregate_quality_score,
                    state.eval.quality_trend
                ),
            };
        }

        // Default in soft zone: hold (treated as breathe — no action)
        ContextCompressionAdvice {
            ruling: ContextRuling::Breathe,
            pressure,
            target_tokens: None,
            rationale: format!(
                "pressure {:.0}% in soft zone, signals inconclusive — holding",
                pressure * 100.0
            ),
        }
    }
}

// Keep `HomeostaticRule` impl for compatibility with the engine's `evaluate_all`.
impl HomeostaticRule for ContextPressureRule {
    fn rule_id(&self) -> &str {
        "context_pressure"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let advice = self.evaluate_compression(state);
        match advice.ruling {
            ContextRuling::Breathe | ContextRuling::Dilate => None,
            ContextRuling::Compress | ContextRuling::Emergency => Some(GatingDecision {
                rule_id: self.rule_id().into(),
                preferred_model: Some(ModelTier::Standard),
                max_tokens_next_turn: Some(2048),
                rationale: advice.rationale,
                ..GatingDecision::noop(self.rule_id())
            }),
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
    use autonomic_core::context::ContextRuling;

    // --- ContextPressureRule: multi-zone tests ---

    #[test]
    fn context_pressure_low_returns_breathe() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.40;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Breathe);
    }

    #[test]
    fn context_pressure_soft_zone_high_tool_density_dilates() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.70;
        state.cognitive.tool_density = 3.0;
        state.eval.aggregate_quality_score = 0.85;
        state.eval.quality_trend = 0.01;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Dilate);
    }

    #[test]
    fn context_pressure_soft_zone_degrading_quality_compresses() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.70;
        state.cognitive.tool_density = 0.5;
        state.eval.aggregate_quality_score = 0.60;
        state.eval.quality_trend = -0.05;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Compress);
        assert!(advice.target_tokens.is_some());
    }

    #[test]
    fn context_pressure_hard_zone_always_compresses() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.88;
        state.cognitive.tool_density = 5.0;
        state.eval.aggregate_quality_score = 0.95;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Compress);
    }

    #[test]
    fn context_pressure_emergency_zone() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.96;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Emergency);
    }

    #[test]
    fn context_pressure_soft_zone_stale_turns_compresses() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.65;
        state.cognitive.turns_since_compact = 20;
        state.cognitive.tool_density = 0.2;
        state.eval.quality_trend = -0.01;
        state.eval.aggregate_quality_score = 0.80;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Compress);
    }

    #[test]
    fn context_pressure_soft_zone_error_streak_compresses() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.70;
        state.operational.error_streak = 3;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Compress);
    }

    #[test]
    fn context_pressure_soft_zone_quality_good_dilates() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.70;
        state.cognitive.tool_density = 0.5;
        state.eval.aggregate_quality_score = 0.85;
        state.eval.quality_trend = 0.02;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Dilate);
    }

    #[test]
    fn context_pressure_soft_zone_inconclusive_holds() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.65;
        state.cognitive.tool_density = 1.0;
        state.eval.aggregate_quality_score = 0.70;
        state.eval.quality_trend = -0.005; // slightly negative but above -0.02
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Breathe);
        assert!(advice.rationale.contains("inconclusive"));
    }

    #[test]
    fn context_pressure_compress_target_tokens_calculated() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.88;
        state.cognitive.total_tokens_used = 176_000;
        state.cognitive.tokens_remaining = 24_000;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Compress);
        // target = 200_000 * 0.35 = 70_000
        assert_eq!(advice.target_tokens, Some(70_000));
    }

    #[test]
    fn context_pressure_emergency_target_tokens_calculated() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.96;
        state.cognitive.total_tokens_used = 192_000;
        state.cognitive.tokens_remaining = 8_000;
        let advice = rule.evaluate_compression(&state);
        assert_eq!(advice.ruling, ContextRuling::Emergency);
        // target = 200_000 * 0.25 = 50_000
        assert_eq!(advice.target_tokens, Some(50_000));
    }

    // --- ContextPressureRule: HomeostaticRule trait compat ---

    #[test]
    fn homeostatic_rule_returns_none_for_breathe() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.40;
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn homeostatic_rule_returns_none_for_dilate() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.70;
        state.cognitive.tool_density = 3.0;
        state.eval.quality_trend = 0.01;
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn homeostatic_rule_returns_some_for_compress() {
        let rule = ContextPressureRule::default();
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.context_pressure = 0.88;
        let decision = rule.evaluate(&state).unwrap();
        assert_eq!(decision.preferred_model, Some(ModelTier::Standard));
        assert_eq!(decision.max_tokens_next_turn, Some(2048));
    }

    #[test]
    fn custom_thresholds() {
        let rule = ContextPressureRule::new(0.50, 0.70, 0.90);
        let mut state = HomeostaticState::for_agent("test");
        // At 0.55 — below default soft (0.60) but above custom (0.50)
        state.cognitive.context_pressure = 0.55;
        state.eval.aggregate_quality_score = 0.85;
        state.eval.quality_trend = 0.01;
        let advice = rule.evaluate_compression(&state);
        // Should be in soft zone with custom rule, dilate due to good quality
        assert_eq!(advice.ruling, ContextRuling::Dilate);
    }

    // --- TokenExhaustionRule: unchanged tests ---

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
