//! Core types, traits, and errors for the Autonomic homeostasis controller.
//!
//! This crate defines the three-pillar homeostatic state model (operational,
//! cognitive, economic), gating profiles, hysteresis primitives, rule traits,
//! and event constructors. It has zero I/O dependencies.

pub mod context;
pub mod economic;
pub mod error;
pub mod events;
pub mod gating;
pub mod hysteresis;
pub mod identity;
pub mod rules;
pub mod trust;

// Re-exports for convenience
pub use context::{ContextCompressionAdvice, ContextRuling};
pub use economic::{CostReason, EconomicMode, EconomicState, ModelCostRates, ModelTier};
pub use error::{AutonomicError, AutonomicResult};
pub use events::AutonomicEvent;
pub use gating::{
    AutonomicGatingProfile, BeliefState, CognitiveState, EconomicGates, EvalState,
    HomeostaticState, OperationalState, StrategyState,
};
pub use hysteresis::HysteresisGate;
pub use identity::EconomicIdentity;
pub use rules::{GatingDecision, HomeostaticRule, RuleSet};
pub use trust::{TrustScore, TrustTier, TrustTrajectory};
