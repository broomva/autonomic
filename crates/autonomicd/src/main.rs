//! Autonomic daemon — homeostasis controller service.
//!
//! Starts the HTTP API server with configurable setpoints.
//! In production, connects to a Lago journal for event subscription.
//! In standalone mode, operates with in-memory projections only.

mod config;

use anyhow::Result;
use autonomic_api::{AppState, build_router};
use autonomic_controller::{
    BudgetExhaustionRule, ContextPressureRule, ErrorStreakRule, SpendVelocityRule, SurvivalRule,
    TokenExhaustionRule,
};
use autonomic_core::rules::RuleSet;
use clap::Parser;
use config::{AutonomicConfig, CliArgs};
use tracing::info;

fn build_rule_set(config: &AutonomicConfig) -> RuleSet {
    let mut rules = RuleSet::new();

    // Economic rules
    rules.add(Box::new(SurvivalRule::new()));
    rules.add(Box::new(SpendVelocityRule::new(
        config.economic.spend_velocity_threshold,
    )));
    rules.add(Box::new(BudgetExhaustionRule::new(
        config.economic.budget_exhaustion_threshold,
    )));

    // Cognitive rules
    rules.add(Box::new(ContextPressureRule::new(
        config.cognitive.context_pressure_threshold,
    )));
    rules.add(Box::new(TokenExhaustionRule::new(
        config.cognitive.token_exhaustion_threshold,
        2,
    )));

    // Operational rules
    rules.add(Box::new(ErrorStreakRule::new(
        config.operational.error_rate_threshold,
        config.operational.min_events,
    )));

    rules
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "autonomicd=info".into()),
        )
        .init();

    let args = CliArgs::parse();

    // Load config from file or use defaults
    let config = if let Some(config_path) = &args.config {
        let content = std::fs::read_to_string(config_path)?;
        toml::from_str(&content)?
    } else {
        AutonomicConfig {
            bind: args.bind.clone(),
            ..Default::default()
        }
    };

    info!(bind = %config.bind, "starting autonomicd");

    let rules = build_rule_set(&config);
    let state = AppState::new(rules);
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(&config.bind).await?;
    info!(addr = %config.bind, "autonomicd listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    info!("autonomicd stopped");
    Ok(())
}

async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install CTRL+C handler");
    info!("shutdown signal received");
}
