//! Knowledge health regulation rule.
//!
//! Monitors knowledge graph health and memory pressure, producing
//! advisory signals when the agent's knowledge is degraded.

use autonomic_core::AutonomicEvent;
use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::{GatingDecision, HomeostaticRule};

/// Rule that monitors knowledge graph health and memory accumulation.
///
/// Fires when:
/// - Knowledge health drops below 0.70 (broken links, contradictions)
/// - Observation count is high without corresponding compaction (memory bloat)
/// - Knowledge index is stale (>1 hour since last indexing)
pub struct KnowledgeHealthRule {
    /// Minimum acceptable knowledge health score (default: 0.70).
    pub health_threshold: f32,
    /// Maximum observations before compaction is recommended (default: 50).
    pub max_observations_before_compact: u32,
    /// Maximum time since last knowledge indexing in ms (default: 3_600_000 = 1 hour).
    pub stale_index_ms: u64,
}

impl Default for KnowledgeHealthRule {
    fn default() -> Self {
        Self {
            health_threshold: 0.70,
            max_observations_before_compact: 50,
            stale_index_ms: 3_600_000,
        }
    }
}

impl HomeostaticRule for KnowledgeHealthRule {
    fn rule_id(&self) -> &str {
        "knowledge_health"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let mut issues = Vec::new();

        // Check knowledge graph health
        if state.cognitive.knowledge_note_count > 0
            && state.cognitive.knowledge_health < self.health_threshold
        {
            issues.push(format!(
                "knowledge health {:.0}% below {:.0}% threshold ({} notes)",
                state.cognitive.knowledge_health * 100.0,
                self.health_threshold * 100.0,
                state.cognitive.knowledge_note_count,
            ));
        }

        // Check memory bloat (many observations, few compactions)
        let uncompacted = state
            .cognitive
            .observation_count
            .saturating_sub(state.cognitive.compaction_count * 10); // each compaction covers ~10 observations
        if uncompacted > self.max_observations_before_compact {
            issues.push(format!(
                "{} uncompacted observations (compactions: {})",
                uncompacted, state.cognitive.compaction_count,
            ));
        }

        // Check knowledge staleness
        if state.cognitive.knowledge_last_indexed_ms > 0 {
            let age_ms = state
                .last_event_ms
                .saturating_sub(state.cognitive.knowledge_last_indexed_ms);
            if age_ms > self.stale_index_ms {
                issues.push(format!(
                    "knowledge index is {} min old (threshold: {} min)",
                    age_ms / 60_000,
                    self.stale_index_ms / 60_000,
                ));
            }
        }

        if issues.is_empty() {
            return None;
        }

        let rationale = format!("knowledge regulation: {}", issues.join("; "));

        Some(GatingDecision {
            rule_id: self.rule_id().to_string(),
            restrict_expensive_tools: if state.cognitive.knowledge_health < 0.5 {
                Some(true) // Severely degraded knowledge → restrict expensive operations
            } else {
                None
            },
            rationale,
            ..GatingDecision::noop(self.rule_id())
        })
    }
}

/// Rule that requests rollback after sustained post-promotion knowledge regression.
///
/// Promotion remains advisory-first: this rule does not mutate configuration.
/// Instead it emits an `autonomic.RollbackRequested` advisory event that EGRI
/// can consume and use to restore the previous artifact version.
pub struct KnowledgeRegressionRule {
    /// Number of consecutive below-threshold evaluations required before rollback.
    pub min_regression_evaluations: u32,
}

impl KnowledgeRegressionRule {
    pub fn new(min_regression_evaluations: u32) -> Self {
        Self {
            min_regression_evaluations,
        }
    }
}

impl Default for KnowledgeRegressionRule {
    fn default() -> Self {
        // Spec B requires rollback only after >3 consecutive regressions.
        Self::new(4)
    }
}

impl HomeostaticRule for KnowledgeRegressionRule {
    fn rule_id(&self) -> &str {
        "knowledge_regression"
    }

    fn evaluate(&self, state: &HomeostaticState) -> Option<GatingDecision> {
        let promotion = &state.cognitive.knowledge_promotion;
        let version = promotion.active_version.as_ref()?;

        if promotion.rollback_requested
            || promotion.regression_evaluations < self.min_regression_evaluations
        {
            return None;
        }

        let rollback_to = match &promotion.rollback_target {
            Some(target) => target.clone(),
            None => {
                return Some(GatingDecision {
                    rule_id: self.rule_id().into(),
                    rationale: format!(
                        "post-promotion regression detected for knowledge thresholds {version}, \
                         but no rollback target is available"
                    ),
                    restrict_side_effects: Some(true),
                    ..GatingDecision::noop(self.rule_id())
                });
            }
        };
        let threshold = promotion.health_threshold.unwrap_or(0.70);
        let score = promotion
            .last_regression_score
            .unwrap_or(state.cognitive.knowledge_health);
        let reason = format!(
            "post-promotion regression detected: knowledge thresholds {version} had {} \
             consecutive evaluations below {:.0}% health threshold (latest {:.0}%), \
             requesting rollback to {rollback_to}",
            promotion.regression_evaluations,
            threshold * 100.0,
            score * 100.0
        );

        Some(GatingDecision {
            rule_id: self.rule_id().into(),
            restrict_side_effects: Some(true),
            rationale: reason.clone(),
            advisory_events: vec![AutonomicEvent::RollbackRequested {
                artifact: "knowledge_thresholds".into(),
                rollback_to,
                reason,
            }],
            ..GatingDecision::noop(self.rule_id())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state_with_knowledge(
        health: f32,
        note_count: u32,
        observations: u32,
        compactions: u32,
        last_indexed_ms: u64,
        last_event_ms: u64,
    ) -> HomeostaticState {
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.knowledge_health = health;
        state.cognitive.knowledge_note_count = note_count;
        state.cognitive.observation_count = observations;
        state.cognitive.compaction_count = compactions;
        state.cognitive.knowledge_last_indexed_ms = last_indexed_ms;
        state.last_event_ms = last_event_ms;
        state
    }

    #[test]
    fn healthy_state_does_not_fire() {
        let rule = KnowledgeHealthRule::default();
        let state = state_with_knowledge(0.95, 100, 10, 1, 1000, 2000);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn low_health_fires() {
        let rule = KnowledgeHealthRule::default();
        let state = state_with_knowledge(0.45, 100, 10, 1, 1000, 2000);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("knowledge health"));
        assert!(decision.rationale.contains("45%"));
        // Severely degraded (< 0.5) → restrict expensive tools
        assert_eq!(decision.restrict_expensive_tools, Some(true));
    }

    #[test]
    fn low_health_above_severe_threshold_no_tool_restriction() {
        let rule = KnowledgeHealthRule::default();
        let state = state_with_knowledge(0.60, 100, 10, 1, 1000, 2000);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("knowledge health"));
        // 0.60 >= 0.5 → no tool restriction
        assert!(decision.restrict_expensive_tools.is_none());
    }

    #[test]
    fn zero_notes_with_low_health_does_not_fire() {
        let rule = KnowledgeHealthRule::default();
        // No notes means health metric is meaningless
        let state = state_with_knowledge(0.30, 0, 10, 1, 1000, 2000);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn memory_bloat_fires() {
        let rule = KnowledgeHealthRule::default();
        // 60 observations, 0 compactions → 60 uncompacted > 50 threshold
        let state = state_with_knowledge(0.95, 100, 60, 0, 1000, 2000);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("uncompacted observations"));
        assert!(decision.rationale.contains("60"));
    }

    #[test]
    fn compacted_observations_do_not_fire() {
        let rule = KnowledgeHealthRule::default();
        // 60 observations, 1 compaction (covers ~10) → 50 uncompacted = threshold
        let state = state_with_knowledge(0.95, 100, 60, 1, 1000, 2000);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn stale_index_fires() {
        let rule = KnowledgeHealthRule::default();
        // Last indexed at t=1000, current event at t=4_601_000 (>1 hour later)
        let state = state_with_knowledge(0.95, 100, 10, 1, 1_000, 3_601_001);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("knowledge index is"));
        assert!(decision.rationale.contains("min old"));
    }

    #[test]
    fn fresh_index_does_not_fire_staleness() {
        let rule = KnowledgeHealthRule::default();
        // Last indexed at t=1000, current at t=2000 (1 second later)
        let state = state_with_knowledge(0.95, 100, 10, 1, 1_000, 2_000);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn never_indexed_does_not_fire_staleness() {
        let rule = KnowledgeHealthRule::default();
        // knowledge_last_indexed_ms = 0 → never indexed, don't fire
        let state = state_with_knowledge(0.95, 100, 10, 1, 0, 5_000_000);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn multiple_issues_produce_combined_rationale() {
        let rule = KnowledgeHealthRule::default();
        // Low health + memory bloat + stale index
        let state = state_with_knowledge(0.40, 50, 60, 0, 1_000, 5_000_000);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("knowledge health"));
        assert!(decision.rationale.contains("uncompacted observations"));
        assert!(decision.rationale.contains("knowledge index is"));
    }

    #[test]
    fn custom_thresholds() {
        let rule = KnowledgeHealthRule {
            health_threshold: 0.90,
            max_observations_before_compact: 20,
            stale_index_ms: 600_000, // 10 minutes
        };
        // Health 0.85 < 0.90 custom threshold
        let state = state_with_knowledge(0.85, 50, 10, 1, 1_000, 2_000);
        let decision = rule.evaluate(&state).unwrap();
        assert!(decision.rationale.contains("knowledge health"));
    }

    #[test]
    fn boundary_at_health_threshold() {
        let rule = KnowledgeHealthRule::default();
        // Exactly at threshold (0.70) — should NOT fire (< is strict)
        let state = state_with_knowledge(0.70, 100, 10, 1, 1_000, 2_000);
        assert!(rule.evaluate(&state).is_none());

        // Just below threshold
        let state = state_with_knowledge(0.699, 100, 10, 1, 1_000, 2_000);
        assert!(rule.evaluate(&state).is_some());
    }

    #[test]
    fn boundary_at_observation_threshold() {
        let rule = KnowledgeHealthRule::default();
        // Exactly at threshold (50 uncompacted) — should NOT fire (> is strict)
        let state = state_with_knowledge(0.95, 100, 50, 0, 1_000, 2_000);
        assert!(rule.evaluate(&state).is_none());

        // Just above threshold
        let state = state_with_knowledge(0.95, 100, 51, 0, 1_000, 2_000);
        assert!(rule.evaluate(&state).is_some());
    }

    fn state_with_promotion(regressions: u32) -> HomeostaticState {
        let mut state = HomeostaticState::for_agent("test");
        state.cognitive.knowledge_health = 0.62;
        state.cognitive.knowledge_note_count = 100;
        state.cognitive.knowledge_promotion.active_version = Some("v2".into());
        state.cognitive.knowledge_promotion.rollback_target = Some("v1".into());
        state.cognitive.knowledge_promotion.health_threshold = Some(0.70);
        state.cognitive.knowledge_promotion.regression_evaluations = regressions;
        state.cognitive.knowledge_promotion.last_regression_score = Some(0.62);
        state
    }

    #[test]
    fn regression_rule_does_not_fire_before_promotion() {
        let rule = KnowledgeRegressionRule::default();
        let state = HomeostaticState::for_agent("test");
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn regression_rule_waits_for_sustained_regression() {
        let rule = KnowledgeRegressionRule::default();
        let state = state_with_promotion(3);
        assert!(rule.evaluate(&state).is_none());
    }

    #[test]
    fn regression_rule_emits_rollback_request() {
        let rule = KnowledgeRegressionRule::default();
        let state = state_with_promotion(4);
        let decision = rule.evaluate(&state).expect("rule should fire");

        assert!(
            decision
                .rationale
                .contains("post-promotion regression detected")
        );
        assert_eq!(decision.restrict_side_effects, Some(true));
        assert_eq!(decision.advisory_events.len(), 1);
        match &decision.advisory_events[0] {
            AutonomicEvent::RollbackRequested {
                artifact,
                rollback_to,
                ..
            } => {
                assert_eq!(artifact, "knowledge_thresholds");
                assert_eq!(rollback_to, "v1");
            }
            _ => panic!("expected rollback request"),
        }
    }

    #[test]
    fn regression_rule_is_idempotent_after_rollback_requested() {
        let rule = KnowledgeRegressionRule::default();
        let mut state = state_with_promotion(4);
        state.cognitive.knowledge_promotion.rollback_requested = true;
        assert!(rule.evaluate(&state).is_none());
    }
}
