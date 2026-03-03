//! Error types for the Autonomic homeostasis controller.

use thiserror::Error;

/// Top-level error type for the Autonomic system.
#[derive(Debug, Error)]
pub enum AutonomicError {
    /// A rule evaluation failed.
    #[error("rule evaluation failed: {0}")]
    RuleEvaluation(String),

    /// Projection state is stale or missing.
    #[error("projection not found for session: {0}")]
    ProjectionNotFound(String),

    /// Serialization/deserialization error.
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    /// Event store interaction failed.
    #[error("event store error: {0}")]
    EventStore(String),

    /// Configuration error.
    #[error("configuration error: {0}")]
    Config(String),
}

pub type AutonomicResult<T> = Result<T, AutonomicError>;
