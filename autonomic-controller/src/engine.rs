//! Controller engine: evaluates rules and merges decisions into a gating profile.
//!
//! The engine is pure — it takes a `HomeostaticState` and a `RuleSet`,
//! evaluates all rules, and produces an `AutonomicGatingProfile`.

use aios_protocol::event::RiskLevel;
use aios_protocol::mode::GatingProfile;
use autonomic_core::gating::{AutonomicGatingProfile, EconomicGates, HomeostaticState};
use autonomic_core::rules::{GatingDecision, RuleSet};
use autonomic_core::{EconomicMode, ModelTier};
use tracing::{Span, instrument};

/// Evaluate all rules and merge decisions into a final gating profile.
///
/// Merge strategy: most restrictive wins for each field.
#[instrument(skip(state, rules), fields(autonomic.economic_mode, autonomic.rules_checked))]
pub fn evaluate(state: &HomeostaticState, rules: &RuleSet) -> AutonomicGatingProfile {
    let decisions = rules.evaluate_all(state);
    let profile = merge_decisions(&decisions);

    let span = Span::current();
    span.record(
        "autonomic.economic_mode",
        tracing::field::debug(profile.economic.economic_mode),
    );
    span.record("autonomic.rules_checked", decisions.len());

    profile
}

/// Merge multiple gating decisions into a single profile.
///
/// For each field, the most restrictive value wins:
/// - `economic_mode`: highest severity (Hibernate > Hustle > Conserving > Sovereign)
/// - `max_tokens_next_turn`: minimum
/// - `preferred_model`: cheapest
/// - `restrict_*`: any true → true
/// - `max_tool_calls_per_tick`: minimum
fn merge_decisions(decisions: &[GatingDecision]) -> AutonomicGatingProfile {
    let mut profile = AutonomicGatingProfile::default();

    if decisions.is_empty() {
        return profile;
    }

    let mut rationale = Vec::new();
    let mut most_restrictive_mode = EconomicMode::Sovereign;
    let mut min_tokens: Option<u32> = None;
    let mut cheapest_model: Option<ModelTier> = None;
    let mut restrict_expensive = false;
    let mut restrict_side_effects = false;
    let mut min_tool_calls: Option<u32> = None;
    let mut advisory_events = Vec::new();

    for d in decisions {
        rationale.push(d.rationale.clone());
        advisory_events.extend(d.advisory_events.clone());

        if let Some(mode) = d.economic_mode
            && economic_mode_severity(mode) > economic_mode_severity(most_restrictive_mode)
        {
            most_restrictive_mode = mode;
        }

        if let Some(tokens) = d.max_tokens_next_turn {
            min_tokens = Some(min_tokens.map_or(tokens, |t: u32| t.min(tokens)));
        }

        if let Some(model) = d.preferred_model {
            cheapest_model = Some(cheapest_model.map_or(model, |m| cheaper_model(m, model)));
        }

        if d.restrict_expensive_tools == Some(true) {
            restrict_expensive = true;
        }

        if d.restrict_side_effects == Some(true) {
            restrict_side_effects = true;
        }

        if let Some(calls) = d.max_tool_calls_per_tick {
            min_tool_calls = Some(min_tool_calls.map_or(calls, |c: u32| c.min(calls)));
        }
    }

    // Apply merged values to profile
    profile.economic = EconomicGates {
        economic_mode: most_restrictive_mode,
        max_tokens_next_turn: min_tokens,
        preferred_model: cheapest_model,
        allow_expensive_tools: !restrict_expensive,
        allow_replication: most_restrictive_mode == EconomicMode::Sovereign,
    };

    // Apply operational overrides based on restrictions
    if restrict_side_effects {
        profile.operational.allow_side_effects = false;
        profile.operational.require_approval_for_risk = RiskLevel::Low;
    }

    if let Some(calls) = min_tool_calls {
        profile.operational.max_tool_calls_per_tick = calls;
    }

    // Tighten operational profile based on economic mode
    match most_restrictive_mode {
        EconomicMode::Hibernate => {
            profile.operational = GatingProfile {
                allow_side_effects: false,
                require_approval_for_risk: RiskLevel::Low,
                max_tool_calls_per_tick: 0,
                max_file_mutations_per_tick: 0,
                allow_network: false,
                allow_shell: false,
            };
        }
        EconomicMode::Hustle => {
            profile.operational.max_tool_calls_per_tick =
                profile.operational.max_tool_calls_per_tick.min(5);
            profile.operational.max_file_mutations_per_tick =
                profile.operational.max_file_mutations_per_tick.min(2);
        }
        _ => {}
    }

    profile.rationale = rationale;
    profile.advisory_events = advisory_events;
    profile
}

/// Map economic mode to severity (higher = more restrictive).
fn economic_mode_severity(mode: EconomicMode) -> u8 {
    match mode {
        EconomicMode::Sovereign => 0,
        EconomicMode::Conserving => 1,
        EconomicMode::Hustle => 2,
        EconomicMode::Hibernate => 3,
    }
}

/// Return the cheaper of two model tiers.
fn cheaper_model(a: ModelTier, b: ModelTier) -> ModelTier {
    let rank = |t: ModelTier| match t {
        ModelTier::Flagship => 2,
        ModelTier::Standard => 1,
        ModelTier::Budget => 0,
    };
    if rank(a) <= rank(b) { a } else { b }
}

#[cfg(test)]
mod tests {
    use super::*;
    use autonomic_core::rules::{GatingDecision, HomeostaticRule, RuleSet};

    struct MockRule {
        id: String,
        decision: Option<GatingDecision>,
    }

    impl HomeostaticRule for MockRule {
        fn rule_id(&self) -> &str {
            &self.id
        }

        fn evaluate(&self, _state: &HomeostaticState) -> Option<GatingDecision> {
            self.decision.clone()
        }
    }

    #[test]
    fn empty_rules_produce_default_profile() {
        let rules = RuleSet::new();
        let state = HomeostaticState::for_agent("test");
        let profile = evaluate(&state, &rules);
        assert_eq!(profile.economic.economic_mode, EconomicMode::Sovereign);
        assert!(profile.operational.allow_side_effects);
    }

    #[test]
    fn most_restrictive_mode_wins() {
        let mut rules = RuleSet::new();
        rules.add(Box::new(MockRule {
            id: "r1".into(),
            decision: Some(GatingDecision {
                economic_mode: Some(EconomicMode::Conserving),
                rationale: "low balance".into(),
                ..GatingDecision::noop("r1")
            }),
        }));
        rules.add(Box::new(MockRule {
            id: "r2".into(),
            decision: Some(GatingDecision {
                economic_mode: Some(EconomicMode::Hustle),
                rationale: "very low balance".into(),
                ..GatingDecision::noop("r2")
            }),
        }));

        let state = HomeostaticState::for_agent("test");
        let profile = evaluate(&state, &rules);
        assert_eq!(profile.economic.economic_mode, EconomicMode::Hustle);
    }

    #[test]
    fn min_tokens_wins() {
        let mut rules = RuleSet::new();
        rules.add(Box::new(MockRule {
            id: "r1".into(),
            decision: Some(GatingDecision {
                max_tokens_next_turn: Some(4096),
                rationale: "moderate".into(),
                ..GatingDecision::noop("r1")
            }),
        }));
        rules.add(Box::new(MockRule {
            id: "r2".into(),
            decision: Some(GatingDecision {
                max_tokens_next_turn: Some(1024),
                rationale: "tight".into(),
                ..GatingDecision::noop("r2")
            }),
        }));

        let state = HomeostaticState::for_agent("test");
        let profile = evaluate(&state, &rules);
        assert_eq!(profile.economic.max_tokens_next_turn, Some(1024));
    }

    #[test]
    fn restrict_side_effects_propagates() {
        let mut rules = RuleSet::new();
        rules.add(Box::new(MockRule {
            id: "r1".into(),
            decision: Some(GatingDecision {
                restrict_side_effects: Some(true),
                rationale: "high error rate".into(),
                ..GatingDecision::noop("r1")
            }),
        }));

        let state = HomeostaticState::for_agent("test");
        let profile = evaluate(&state, &rules);
        assert!(!profile.operational.allow_side_effects);
        assert_eq!(
            profile.operational.require_approval_for_risk,
            RiskLevel::Low
        );
    }

    #[test]
    fn advisory_events_propagate_to_profile() {
        let mut rules = RuleSet::new();
        rules.add(Box::new(MockRule {
            id: "rollback".into(),
            decision: Some(GatingDecision {
                advisory_events: vec![autonomic_core::AutonomicEvent::RollbackRequested {
                    artifact: "knowledge_thresholds".into(),
                    rollback_to: "v1".into(),
                    reason: "regression".into(),
                }],
                rationale: "rollback requested".into(),
                ..GatingDecision::noop("rollback")
            }),
        }));

        let state = HomeostaticState::for_agent("test");
        let profile = evaluate(&state, &rules);
        assert_eq!(profile.advisory_events.len(), 1);
    }

    #[test]
    fn hibernate_locks_down_everything() {
        let mut rules = RuleSet::new();
        rules.add(Box::new(MockRule {
            id: "r1".into(),
            decision: Some(GatingDecision {
                economic_mode: Some(EconomicMode::Hibernate),
                rationale: "bankrupt".into(),
                ..GatingDecision::noop("r1")
            }),
        }));

        let state = HomeostaticState::for_agent("test");
        let profile = evaluate(&state, &rules);
        assert!(!profile.operational.allow_side_effects);
        assert!(!profile.operational.allow_network);
        assert!(!profile.operational.allow_shell);
        assert_eq!(profile.operational.max_tool_calls_per_tick, 0);
    }

    #[test]
    fn cheapest_model_wins() {
        let mut rules = RuleSet::new();
        rules.add(Box::new(MockRule {
            id: "r1".into(),
            decision: Some(GatingDecision {
                preferred_model: Some(ModelTier::Standard),
                rationale: "moderate".into(),
                ..GatingDecision::noop("r1")
            }),
        }));
        rules.add(Box::new(MockRule {
            id: "r2".into(),
            decision: Some(GatingDecision {
                preferred_model: Some(ModelTier::Budget),
                rationale: "tight".into(),
                ..GatingDecision::noop("r2")
            }),
        }));

        let state = HomeostaticState::for_agent("test");
        let profile = evaluate(&state, &rules);
        assert_eq!(profile.economic.preferred_model, Some(ModelTier::Budget));
    }
}
