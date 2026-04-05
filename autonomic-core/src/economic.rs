//! Economic homeostasis types.
//!
//! Economics is a core concern from crate zero. Agents need survival pressure:
//! budget accountability, cost-aware model selection, and identity-based payments.

use serde::{Deserialize, Serialize};

use crate::hysteresis::HysteresisGate;

/// The agent's economic operating mode, determined by balance-to-burn ratio.
///
/// Transitions use hysteresis to prevent flapping.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EconomicMode {
    /// Balance > 2x monthly burn. Full autonomy.
    #[default]
    Sovereign,
    /// 1-2x monthly burn. Prefer cheaper models, limit expensive tools.
    Conserving,
    /// 0-1x monthly burn. Cheapest model only, no expensive tools.
    Hustle,
    /// Balance <= 0. Skip LLM calls, heartbeats only.
    Hibernate,
}

/// LLM model tier for cost-aware selection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelTier {
    /// Flagship models (Claude Opus, GPT-4).
    Flagship,
    /// Mid-tier models (Claude Sonnet, GPT-4o).
    #[default]
    Standard,
    /// Budget models (Claude Haiku, GPT-4o-mini).
    Budget,
}

/// The agent's economic state, accumulated from cost events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicState {
    /// Agent's economic identity address (if registered).
    pub identity_address: Option<String>,
    /// Current balance in micro-credits (1 credit = `1_000_000` micro-credits).
    pub balance_micro_credits: i64,
    /// Total revenue earned over agent lifetime.
    pub lifetime_revenue: i64,
    /// Total costs incurred over agent lifetime.
    pub lifetime_costs: i64,
    /// Estimated monthly burn rate in micro-credits.
    pub monthly_burn_estimate: i64,
    /// Current economic mode.
    pub mode: EconomicMode,
    /// Cost accumulated in the last 5 minutes (micro-credits).
    pub cost_last_5min: i64,
    /// Timestamp of the last cost event (ms since epoch).
    pub last_cost_event_ms: u64,
    /// Hysteresis gate for economic mode transitions — prevents flapping.
    pub mode_gate: HysteresisGate,
}

impl Default for EconomicState {
    fn default() -> Self {
        Self {
            identity_address: None,
            balance_micro_credits: 10_000_000, // 10 credits initial
            lifetime_revenue: 0,
            lifetime_costs: 0,
            monthly_burn_estimate: 0,
            mode: EconomicMode::Sovereign,
            cost_last_5min: 0,
            last_cost_event_ms: 0,
            // Mode gate: activates (escalates) when severity metric ≥ 0.7,
            // deactivates (relaxes) when ≤ 0.3, with 30s min-hold.
            mode_gate: HysteresisGate::new(0.7, 0.3, 30_000),
        }
    }
}

impl EconomicState {
    /// Compute the balance-to-burn ratio. Returns `f64::INFINITY` if burn is zero.
    pub fn balance_to_burn_ratio(&self) -> f64 {
        if self.monthly_burn_estimate <= 0 {
            return f64::INFINITY;
        }
        self.balance_micro_credits as f64 / self.monthly_burn_estimate as f64
    }
}

/// Per-model cost rates in micro-credits per token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCostRates {
    pub input_per_token: i64,
    pub output_per_token: i64,
}

impl Default for ModelCostRates {
    fn default() -> Self {
        // Default to roughly Sonnet-class pricing
        Self {
            input_per_token: 3,   // ~$3/M tokens
            output_per_token: 15, // ~$15/M tokens
        }
    }
}

/// Reason for a cost charge.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostReason {
    /// LLM inference cost.
    ModelInference {
        model: String,
        prompt_tokens: u32,
        completion_tokens: u32,
    },
    /// Tool execution cost.
    ToolExecution { tool_name: String },
    /// Storage cost.
    Storage { bytes: u64 },
    /// Manual adjustment.
    Adjustment { description: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn economic_mode_serde_roundtrip() {
        for mode in [
            EconomicMode::Sovereign,
            EconomicMode::Conserving,
            EconomicMode::Hustle,
            EconomicMode::Hibernate,
        ] {
            let json = serde_json::to_string(&mode).unwrap();
            let back: EconomicMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, back);
        }
    }

    #[test]
    fn model_tier_serde_roundtrip() {
        for tier in [ModelTier::Flagship, ModelTier::Standard, ModelTier::Budget] {
            let json = serde_json::to_string(&tier).unwrap();
            let back: ModelTier = serde_json::from_str(&json).unwrap();
            assert_eq!(tier, back);
        }
    }

    #[test]
    fn economic_state_default() {
        let state = EconomicState::default();
        assert_eq!(state.balance_micro_credits, 10_000_000);
        assert_eq!(state.mode, EconomicMode::Sovereign);
        assert_eq!(state.lifetime_costs, 0);
    }

    #[test]
    fn balance_to_burn_ratio_zero_burn() {
        let state = EconomicState::default();
        assert!(state.balance_to_burn_ratio().is_infinite());
    }

    #[test]
    fn balance_to_burn_ratio_normal() {
        let state = EconomicState {
            balance_micro_credits: 2_000_000,
            monthly_burn_estimate: 1_000_000,
            ..Default::default()
        };
        assert!((state.balance_to_burn_ratio() - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn cost_reason_serde_roundtrip() {
        let reason = CostReason::ModelInference {
            model: "claude-sonnet".into(),
            prompt_tokens: 100,
            completion_tokens: 50,
        };
        let json = serde_json::to_string(&reason).unwrap();
        let back: CostReason = serde_json::from_str(&json).unwrap();
        assert!(matches!(back, CostReason::ModelInference { .. }));
    }
}
