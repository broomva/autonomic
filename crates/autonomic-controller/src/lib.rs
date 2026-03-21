//! Pure rule engine and projection reducer for the Autonomic homeostasis controller.
//!
//! This crate has no I/O dependencies. It provides:
//! - A deterministic projection reducer that folds events into `HomeostaticState`
//! - A rule engine that evaluates homeostatic rules against state
//! - Concrete rule implementations for economic, cognitive, and operational regulation

pub mod cognitive_rules;
pub mod economic_rules;
pub mod engine;
pub mod eval_rules;
pub mod operational_rules;
pub mod projection;
pub mod strategy_rules;

// Re-exports
pub use cognitive_rules::{ContextPressureRule, TokenExhaustionRule};
pub use economic_rules::{BudgetExhaustionRule, SpendVelocityRule, SurvivalRule};
pub use engine::evaluate;
pub use eval_rules::EvalQualityRule;
pub use operational_rules::ErrorStreakRule;
pub use projection::fold;
pub use strategy_rules::StrategyRule;
