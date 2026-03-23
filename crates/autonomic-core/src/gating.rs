//! Gating profiles and homeostatic state.
//!
//! `AutonomicGatingProfile` extends the canonical `GatingProfile` with
//! economic regulation. The three-pillar `HomeostaticState` captures
//! operational, cognitive, and economic health.

use aios_protocol::mode::{GatingProfile, OperatingMode};
use serde::{Deserialize, Serialize};

use crate::economic::{EconomicMode, EconomicState, ModelTier};

/// Economic gates — extensions to the canonical `GatingProfile`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicGates {
    /// Current economic operating mode.
    pub economic_mode: EconomicMode,
    /// Maximum tokens allowed for the next turn (advisory).
    pub max_tokens_next_turn: Option<u32>,
    /// Preferred model tier for cost control.
    pub preferred_model: Option<ModelTier>,
    /// Whether expensive tools (e.g., web search, code execution) are allowed.
    pub allow_expensive_tools: bool,
    /// Whether agent replication is allowed.
    pub allow_replication: bool,
}

impl Default for EconomicGates {
    fn default() -> Self {
        Self {
            economic_mode: EconomicMode::Sovereign,
            max_tokens_next_turn: None,
            preferred_model: None,
            allow_expensive_tools: true,
            allow_replication: true,
        }
    }
}

/// The full gating profile emitted by the Autonomic controller.
///
/// Embeds the canonical `GatingProfile` for operational gates and adds
/// economic regulation on top.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AutonomicGatingProfile {
    /// Canonical operational gates (from aios-protocol).
    pub operational: GatingProfile,
    /// Economic regulation gates (Autonomic extension).
    pub economic: EconomicGates,
    /// Human-readable rationale for why this profile was chosen.
    pub rationale: Vec<String>,
}

/// Operational health state — derived from `AgentStateVector` events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalState {
    /// Current operating mode.
    pub mode: OperatingMode,
    /// Consecutive error count.
    pub error_streak: u32,
    /// Total errors seen.
    pub total_errors: u32,
    /// Total successful actions.
    pub total_successes: u32,
    /// Timestamp of last tick (ms since epoch).
    pub last_tick_ms: u64,
}

impl Default for OperationalState {
    fn default() -> Self {
        Self {
            mode: OperatingMode::Execute,
            error_streak: 0,
            total_errors: 0,
            total_successes: 0,
            last_tick_ms: 0,
        }
    }
}

/// Cognitive health state — tracks context and token usage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveState {
    /// Total tokens consumed in the session.
    pub total_tokens_used: u64,
    /// Tokens remaining from budget.
    pub tokens_remaining: u64,
    /// Context pressure (0.0 = empty, 1.0 = full).
    pub context_pressure: f32,
    /// Number of model turns completed.
    pub turns_completed: u32,
}

impl Default for CognitiveState {
    fn default() -> Self {
        Self {
            total_tokens_used: 0,
            tokens_remaining: 120_000,
            context_pressure: 0.0,
            turns_completed: 0,
        }
    }
}

/// Strategy event tracking state.
///
/// Accumulated from `strategy.*` custom events emitted by strategy skills
/// to Lago. Used by advisory rules to inform risk assessment and suggest
/// setpoint reviews.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StrategyState {
    /// Count of drift-check alerts received.
    pub drift_alerts: u32,
    /// Count of decisions logged.
    pub decisions_logged: u32,
    /// Count of strategy critiques completed.
    pub critiques_completed: u32,
    /// Timestamp of the most recent strategy event (ms since epoch).
    pub last_strategy_event_ms: u64,
}

/// Evaluation quality tracking state.
///
/// Accumulated from `eval.*` custom events emitted by Nous evaluators.
/// Used by the `EvalQualityRule` to gate agent behavior based on quality scores.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalState {
    /// Count of inline evaluations completed.
    pub inline_eval_count: u32,
    /// Count of async evaluations completed.
    pub async_eval_count: u32,
    /// Aggregate quality score (0.0..1.0), exponential moving average.
    pub aggregate_quality_score: f64,
    /// Quality trend (positive = improving, negative = degrading).
    pub quality_trend: f64,
    /// Timestamp of the last evaluation (ms since epoch).
    pub last_eval_ms: u64,
}

impl Default for EvalState {
    fn default() -> Self {
        Self {
            inline_eval_count: 0,
            async_eval_count: 0,
            aggregate_quality_score: 1.0, // Optimistic start
            quality_trend: 0.0,
            last_eval_ms: 0,
        }
    }
}

/// Belief state — tracks Anima agent belief metrics.
///
/// Accumulated from `anima.*` custom events emitted to Lago.
/// Provides the Autonomic controller with visibility into the
/// agent's capability set, trust network, and policy compliance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BeliefState {
    /// Number of currently granted capabilities.
    pub capability_count: u32,
    /// Number of peers with trust scores.
    pub trust_peer_count: u32,
    /// Average trust score across all peers (0.0..1.0).
    pub average_trust: f64,
    /// Minimum trust score across all peers (0.0..1.0).
    pub min_trust: f64,
    /// Overall reputation score (0.0..1.0).
    pub reputation_score: f64,
    /// Number of policy violations detected.
    pub violations: u64,
    /// Timestamp of the last belief-related event (ms since epoch).
    pub last_belief_event_ms: u64,
}

impl Default for BeliefState {
    fn default() -> Self {
        Self {
            capability_count: 0,
            trust_peer_count: 0,
            average_trust: 1.0, // Optimistic start (no peers = full trust)
            min_trust: 1.0,
            reputation_score: 1.0, // Optimistic start
            violations: 0,
            last_belief_event_ms: 0,
        }
    }
}

/// The homeostatic state for an agent session.
///
/// This is the projection state: accumulated from the event stream
/// and used as input to the rule engine.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HomeostaticState {
    /// Agent/session identifier.
    pub agent_id: String,
    /// Operational health.
    pub operational: OperationalState,
    /// Cognitive health.
    pub cognitive: CognitiveState,
    /// Economic health.
    pub economic: EconomicState,
    /// Strategy event tracking.
    pub strategy: StrategyState,
    /// Evaluation quality tracking.
    pub eval: EvalState,
    /// Anima belief tracking.
    pub belief: BeliefState,
    /// Sequence number of the last event processed.
    pub last_event_seq: u64,
    /// Timestamp of the last event processed (ms since epoch).
    pub last_event_ms: u64,
}

impl HomeostaticState {
    /// Create a new state for the given agent.
    pub fn for_agent(agent_id: impl Into<String>) -> Self {
        Self {
            agent_id: agent_id.into(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autonomic_gating_profile_default() {
        let profile = AutonomicGatingProfile::default();
        assert!(profile.operational.allow_side_effects);
        assert!(profile.economic.allow_expensive_tools);
        assert_eq!(profile.economic.economic_mode, EconomicMode::Sovereign);
        assert!(profile.rationale.is_empty());
    }

    #[test]
    fn autonomic_gating_profile_serde_roundtrip() {
        let profile = AutonomicGatingProfile {
            operational: GatingProfile::default(),
            economic: EconomicGates {
                economic_mode: EconomicMode::Conserving,
                max_tokens_next_turn: Some(4096),
                preferred_model: Some(ModelTier::Budget),
                allow_expensive_tools: false,
                allow_replication: false,
            },
            rationale: vec!["balance low".into(), "reducing spend".into()],
        };
        let json = serde_json::to_string(&profile).unwrap();
        let back: AutonomicGatingProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(back.economic.economic_mode, EconomicMode::Conserving);
        assert_eq!(back.economic.max_tokens_next_turn, Some(4096));
        assert!(!back.economic.allow_expensive_tools);
        assert_eq!(back.rationale.len(), 2);
    }

    #[test]
    fn homeostatic_state_for_agent() {
        let state = HomeostaticState::for_agent("agent-1");
        assert_eq!(state.agent_id, "agent-1");
        assert_eq!(state.operational.mode, OperatingMode::Execute);
        assert_eq!(state.economic.mode, EconomicMode::Sovereign);
    }

    #[test]
    fn strategy_state_default_is_zeroed() {
        let strategy = StrategyState::default();
        assert_eq!(strategy.drift_alerts, 0);
        assert_eq!(strategy.decisions_logged, 0);
        assert_eq!(strategy.critiques_completed, 0);
        assert_eq!(strategy.last_strategy_event_ms, 0);
    }

    #[test]
    fn homeostatic_state_includes_strategy() {
        let state = HomeostaticState::for_agent("agent-1");
        assert_eq!(state.strategy.drift_alerts, 0);
        assert_eq!(state.strategy.decisions_logged, 0);
        assert_eq!(state.strategy.critiques_completed, 0);
        assert_eq!(state.strategy.last_strategy_event_ms, 0);
    }

    #[test]
    fn strategy_state_serde_roundtrip() {
        let strategy = StrategyState {
            drift_alerts: 5,
            decisions_logged: 12,
            critiques_completed: 3,
            last_strategy_event_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&strategy).unwrap();
        let back: StrategyState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.drift_alerts, 5);
        assert_eq!(back.decisions_logged, 12);
        assert_eq!(back.critiques_completed, 3);
        assert_eq!(back.last_strategy_event_ms, 1_700_000_000_000);
    }

    #[test]
    fn eval_state_default_optimistic() {
        let eval = EvalState::default();
        assert_eq!(eval.inline_eval_count, 0);
        assert_eq!(eval.async_eval_count, 0);
        assert!((eval.aggregate_quality_score - 1.0).abs() < f64::EPSILON);
        assert!((eval.quality_trend).abs() < f64::EPSILON);
        assert_eq!(eval.last_eval_ms, 0);
    }

    #[test]
    fn eval_state_serde_roundtrip() {
        let eval = EvalState {
            inline_eval_count: 15,
            async_eval_count: 3,
            aggregate_quality_score: 0.78,
            quality_trend: -0.02,
            last_eval_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&eval).unwrap();
        let back: EvalState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.inline_eval_count, 15);
        assert_eq!(back.async_eval_count, 3);
        assert!((back.aggregate_quality_score - 0.78).abs() < f64::EPSILON);
        assert!((back.quality_trend - (-0.02)).abs() < f64::EPSILON);
    }

    #[test]
    fn homeostatic_state_includes_eval() {
        let state = HomeostaticState::for_agent("test");
        assert_eq!(state.eval.inline_eval_count, 0);
        assert!((state.eval.aggregate_quality_score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn belief_state_default_optimistic() {
        let belief = BeliefState::default();
        assert_eq!(belief.capability_count, 0);
        assert_eq!(belief.trust_peer_count, 0);
        assert!((belief.average_trust - 1.0).abs() < f64::EPSILON);
        assert!((belief.min_trust - 1.0).abs() < f64::EPSILON);
        assert!((belief.reputation_score - 1.0).abs() < f64::EPSILON);
        assert_eq!(belief.violations, 0);
        assert_eq!(belief.last_belief_event_ms, 0);
    }

    #[test]
    fn belief_state_serde_roundtrip() {
        let belief = BeliefState {
            capability_count: 5,
            trust_peer_count: 3,
            average_trust: 0.72,
            min_trust: 0.45,
            reputation_score: 0.88,
            violations: 2,
            last_belief_event_ms: 1_700_000_000_000,
        };
        let json = serde_json::to_string(&belief).unwrap();
        let back: BeliefState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.capability_count, 5);
        assert_eq!(back.trust_peer_count, 3);
        assert!((back.average_trust - 0.72).abs() < f64::EPSILON);
        assert!((back.min_trust - 0.45).abs() < f64::EPSILON);
        assert!((back.reputation_score - 0.88).abs() < f64::EPSILON);
        assert_eq!(back.violations, 2);
    }

    #[test]
    fn homeostatic_state_includes_belief() {
        let state = HomeostaticState::for_agent("test");
        assert_eq!(state.belief.capability_count, 0);
        assert_eq!(state.belief.violations, 0);
        assert!((state.belief.reputation_score - 1.0).abs() < f64::EPSILON);
    }
}
