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

/// The three-pillar homeostatic state for an agent session.
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
}
