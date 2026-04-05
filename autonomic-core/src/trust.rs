//! Trust scoring types for the Autonomic public API.
//!
//! A `TrustScore` is a composite reliability score for an agent derived
//! from the three-pillar homeostatic state (operational, cognitive, economic).
//! The score is public and requires no authentication.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Composite trust score for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustScore {
    /// Agent identifier.
    pub agent_id: String,
    /// Composite reliability score (0.0 - 1.0).
    pub score: f64,
    /// Trust tier derived from composite score.
    pub tier: TrustTier,
    /// Per-pillar component scores.
    pub components: TrustComponents,
    /// Tier threshold definitions.
    pub tier_thresholds: TierThresholds,
    /// Score trajectory based on recent history.
    pub trajectory: TrustTrajectory,
    /// Timestamp of assessment.
    pub assessed_at: DateTime<Utc>,
}

/// Per-pillar component breakdown.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustComponents {
    /// Operational health component.
    pub operational: OperationalComponent,
    /// Cognitive health component.
    pub cognitive: CognitiveComponent,
    /// Economic health component.
    pub economic: EconomicComponent,
}

/// Operational health component score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalComponent {
    /// Normalized score (0.0 - 1.0).
    pub score: f64,
    /// Contributing factors.
    pub factors: OperationalFactors,
}

/// Factors contributing to the operational score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalFactors {
    /// Ratio of successful operations to total (0.0 - 1.0).
    pub uptime_ratio: f64,
    /// Current error rate (0.0 - 1.0).
    pub error_rate: f64,
    /// Average latency in milliseconds (derived from last tick delta).
    pub avg_latency_ms: u64,
}

/// Cognitive health component score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveComponent {
    /// Normalized score (0.0 - 1.0).
    pub score: f64,
    /// Contributing factors.
    pub factors: CognitiveFactors,
}

/// Factors contributing to the cognitive score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveFactors {
    /// Task completion rate (0.0 - 1.0).
    pub task_completion_rate: f64,
    /// Context utilization efficiency (0.0 - 1.0).
    pub context_utilization: f64,
}

/// Economic health component score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicComponent {
    /// Normalized score (0.0 - 1.0).
    pub score: f64,
    /// Contributing factors.
    pub factors: EconomicFactors,
}

/// Factors contributing to the economic score.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicFactors {
    /// Payment history reliability score (0.0 - 1.0).
    pub payment_history_score: f64,
    /// Credit utilization ratio (0.0 - 1.0).
    pub credit_utilization: f64,
    /// Current economic operating mode.
    pub economic_mode: String,
}

/// Tier threshold definitions for transparency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierThresholds {
    /// Minimum score for Certified tier.
    pub certified: f64,
    /// Minimum score for Trusted tier.
    pub trusted: f64,
    /// Minimum score for Provisional tier.
    pub provisional: f64,
    /// Minimum score for Unverified tier.
    pub unverified: f64,
}

impl Default for TierThresholds {
    fn default() -> Self {
        Self {
            certified: 0.90,
            trusted: 0.75,
            provisional: 0.50,
            unverified: 0.0,
        }
    }
}

/// Trust tier derived from composite score.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTier {
    /// No data or score below 0.50.
    Unverified,
    /// Score 0.50 - 0.75.
    Provisional,
    /// Score 0.75 - 0.90.
    Trusted,
    /// Score >= 0.90.
    Certified,
}

impl TrustTier {
    /// Derive tier from a composite score.
    pub fn from_score(score: f64) -> Self {
        if score >= 0.90 {
            Self::Certified
        } else if score >= 0.75 {
            Self::Trusted
        } else if score >= 0.50 {
            Self::Provisional
        } else {
            Self::Unverified
        }
    }
}

/// Score trajectory indicating improvement or degradation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustTrajectory {
    /// Score is improving over recent events.
    Improving,
    /// Score is stable.
    Stable,
    /// Score is degrading.
    Degrading,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_tier_from_score_certified() {
        assert_eq!(TrustTier::from_score(0.95), TrustTier::Certified);
        assert_eq!(TrustTier::from_score(0.90), TrustTier::Certified);
    }

    #[test]
    fn trust_tier_from_score_trusted() {
        assert_eq!(TrustTier::from_score(0.82), TrustTier::Trusted);
        assert_eq!(TrustTier::from_score(0.75), TrustTier::Trusted);
    }

    #[test]
    fn trust_tier_from_score_provisional() {
        assert_eq!(TrustTier::from_score(0.60), TrustTier::Provisional);
        assert_eq!(TrustTier::from_score(0.50), TrustTier::Provisional);
    }

    #[test]
    fn trust_tier_from_score_unverified() {
        assert_eq!(TrustTier::from_score(0.49), TrustTier::Unverified);
        assert_eq!(TrustTier::from_score(0.0), TrustTier::Unverified);
    }

    #[test]
    fn trust_tier_serde_roundtrip() {
        for tier in [
            TrustTier::Unverified,
            TrustTier::Provisional,
            TrustTier::Trusted,
            TrustTier::Certified,
        ] {
            let json = serde_json::to_string(&tier).unwrap();
            let back: TrustTier = serde_json::from_str(&json).unwrap();
            assert_eq!(tier, back);
        }
    }

    #[test]
    fn trust_trajectory_serde_roundtrip() {
        for trajectory in [
            TrustTrajectory::Improving,
            TrustTrajectory::Stable,
            TrustTrajectory::Degrading,
        ] {
            let json = serde_json::to_string(&trajectory).unwrap();
            let back: TrustTrajectory = serde_json::from_str(&json).unwrap();
            assert_eq!(trajectory, back);
        }
    }

    #[test]
    fn tier_thresholds_default() {
        let thresholds = TierThresholds::default();
        assert!((thresholds.certified - 0.90).abs() < f64::EPSILON);
        assert!((thresholds.trusted - 0.75).abs() < f64::EPSILON);
        assert!((thresholds.provisional - 0.50).abs() < f64::EPSILON);
        assert!((thresholds.unverified - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn trust_score_serde_roundtrip() {
        let score = TrustScore {
            agent_id: "agent-123".into(),
            score: 0.82,
            tier: TrustTier::Trusted,
            components: TrustComponents {
                operational: OperationalComponent {
                    score: 0.90,
                    factors: OperationalFactors {
                        uptime_ratio: 0.95,
                        error_rate: 0.02,
                        avg_latency_ms: 450,
                    },
                },
                cognitive: CognitiveComponent {
                    score: 0.78,
                    factors: CognitiveFactors {
                        task_completion_rate: 0.85,
                        context_utilization: 0.72,
                    },
                },
                economic: EconomicComponent {
                    score: 0.79,
                    factors: EconomicFactors {
                        payment_history_score: 0.88,
                        credit_utilization: 0.65,
                        economic_mode: "sovereign".into(),
                    },
                },
            },
            tier_thresholds: TierThresholds::default(),
            trajectory: TrustTrajectory::Improving,
            assessed_at: Utc::now(),
        };

        let json = serde_json::to_string(&score).unwrap();
        let back: TrustScore = serde_json::from_str(&json).unwrap();
        assert_eq!(back.agent_id, "agent-123");
        assert!((back.score - 0.82).abs() < f64::EPSILON);
        assert_eq!(back.tier, TrustTier::Trusted);
        assert_eq!(back.trajectory, TrustTrajectory::Improving);
    }
}
