//! Daemon configuration.

use clap::Parser;
use serde::{Deserialize, Serialize};

/// Autonomic daemon CLI arguments.
#[derive(Debug, Parser)]
#[command(name = "autonomicd", version, about = "Autonomic homeostasis controller daemon")]
pub struct CliArgs {
    /// Bind address for the HTTP API.
    #[arg(long, default_value = "127.0.0.1:3002")]
    pub bind: String,

    /// Path to configuration file (TOML).
    #[arg(long, short)]
    pub config: Option<String>,

    /// Path to Lago data directory. Enables Lago journal for event persistence.
    /// When omitted, runs in standalone mode with in-memory projections only.
    #[arg(long)]
    pub lago_data_dir: Option<String>,
}

/// Daemon configuration loaded from TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutonomicConfig {
    /// HTTP bind address.
    #[serde(default = "default_bind")]
    pub bind: String,

    /// Path to Lago data directory. Enables Lago journal for event persistence.
    #[serde(default)]
    pub lago_data_dir: Option<String>,

    /// Economic setpoints.
    #[serde(default)]
    pub economic: EconomicSetpoints,

    /// Cognitive setpoints.
    #[serde(default)]
    pub cognitive: CognitiveSetpoints,

    /// Operational setpoints.
    #[serde(default)]
    pub operational: OperationalSetpoints,
}

impl Default for AutonomicConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            lago_data_dir: None,
            economic: EconomicSetpoints::default(),
            cognitive: CognitiveSetpoints::default(),
            operational: OperationalSetpoints::default(),
        }
    }
}

fn default_bind() -> String {
    "127.0.0.1:3002".into()
}

/// Economic regulation setpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EconomicSetpoints {
    /// Spend velocity threshold (micro-credits per 5 minutes).
    #[serde(default = "default_spend_velocity_threshold")]
    pub spend_velocity_threshold: i64,

    /// Budget exhaustion threshold (fraction remaining).
    #[serde(default = "default_budget_exhaustion_threshold")]
    pub budget_exhaustion_threshold: f64,
}

impl Default for EconomicSetpoints {
    fn default() -> Self {
        Self {
            spend_velocity_threshold: default_spend_velocity_threshold(),
            budget_exhaustion_threshold: default_budget_exhaustion_threshold(),
        }
    }
}

fn default_spend_velocity_threshold() -> i64 {
    500_000
}

fn default_budget_exhaustion_threshold() -> f64 {
    0.2
}

/// Cognitive regulation setpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CognitiveSetpoints {
    /// Context pressure threshold (0.0-1.0).
    #[serde(default = "default_context_pressure_threshold")]
    pub context_pressure_threshold: f32,

    /// Token exhaustion threshold (fraction remaining).
    #[serde(default = "default_token_exhaustion_threshold")]
    pub token_exhaustion_threshold: f64,
}

impl Default for CognitiveSetpoints {
    fn default() -> Self {
        Self {
            context_pressure_threshold: default_context_pressure_threshold(),
            token_exhaustion_threshold: default_token_exhaustion_threshold(),
        }
    }
}

fn default_context_pressure_threshold() -> f32 {
    0.8
}

fn default_token_exhaustion_threshold() -> f64 {
    0.1
}

/// Operational regulation setpoints.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationalSetpoints {
    /// Error rate threshold (fraction).
    #[serde(default = "default_error_rate_threshold")]
    pub error_rate_threshold: f64,

    /// Minimum events before error rate rule fires.
    #[serde(default = "default_min_events")]
    pub min_events: u32,
}

impl Default for OperationalSetpoints {
    fn default() -> Self {
        Self {
            error_rate_threshold: default_error_rate_threshold(),
            min_events: default_min_events(),
        }
    }
}

fn default_error_rate_threshold() -> f64 {
    0.3
}

fn default_min_events() -> u32 {
    5
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config() {
        let config = AutonomicConfig::default();
        assert_eq!(config.bind, "127.0.0.1:3002");
        assert_eq!(config.economic.spend_velocity_threshold, 500_000);
        assert!((config.cognitive.context_pressure_threshold - 0.8).abs() < f32::EPSILON);
    }

    #[test]
    fn config_serde_roundtrip() {
        let config = AutonomicConfig::default();
        let toml_str = toml::to_string(&config).unwrap();
        let back: AutonomicConfig = toml::from_str(&toml_str).unwrap();
        assert_eq!(back.bind, config.bind);
    }
}
