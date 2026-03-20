//! Autonomic daemon — homeostasis controller service.
//!
//! Starts the HTTP API server with configurable setpoints.
//! In production, connects to a Lago journal for event subscription.
//! In standalone mode, operates with in-memory projections only.

mod config;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use autonomic_api::{AppState, AuthConfig, build_router_with_auth};
use autonomic_controller::{
    BudgetExhaustionRule, ContextPressureRule, ErrorStreakRule, SpendVelocityRule, StrategyRule,
    SurvivalRule, TokenExhaustionRule,
};
use autonomic_core::rules::RuleSet;
use clap::Parser;
use config::{AutonomicConfig, CliArgs};
use lago_core::journal::Journal;
use lago_journal::RedbJournal;
use tracing::info;
use vigil::VigConfig;

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

    // Strategy advisory rules
    rules.add(Box::new(StrategyRule::default()));

    rules
}

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize telemetry via Vigil (structured logging + optional OTel export)
    let _guard = vigil::init_telemetry(VigConfig::for_service("autonomic").with_env_overrides())?;

    let args = CliArgs::parse();

    // Load config from file or use defaults
    let mut config: AutonomicConfig = if let Some(config_path) = &args.config {
        let content = std::fs::read_to_string(config_path)?;
        toml::from_str(&content)?
    } else {
        AutonomicConfig {
            bind: args.bind.clone(),
            ..Default::default()
        }
    };

    // CLI flag overrides config file
    if args.lago_data_dir.is_some() {
        config.lago_data_dir = args.lago_data_dir;
    }

    info!(bind = %config.bind, "starting autonomicd");

    let rules = build_rule_set(&config);
    let projections = autonomic_lago::new_projection_map();

    // Open Lago journal if configured
    let journal: Option<Arc<dyn Journal>> = if let Some(data_dir) = &config.lago_data_dir {
        std::fs::create_dir_all(data_dir)?;
        let db_path = PathBuf::from(data_dir).join("autonomic.redb");
        let j = RedbJournal::open(&db_path)?;
        info!(path = %db_path.display(), "Lago journal opened");
        Some(Arc::new(j) as Arc<dyn Journal>)
    } else {
        info!("running in standalone mode (no Lago journal)");
        None
    };

    let state = if let Some(journal) = journal {
        AppState::with_journal(projections, rules, journal)
    } else {
        AppState::with_projections(projections, rules)
    };

    let auth_config = AuthConfig::from_env();
    let app = build_router_with_auth(state, auth_config);

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
