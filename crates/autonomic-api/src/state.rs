//! Shared application state for the HTTP API.

use std::collections::HashMap;
use std::sync::Arc;

use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::RuleSet;
use tokio::sync::RwLock;

/// Shared state for the axum HTTP server.
#[derive(Clone)]
pub struct AppState {
    /// Per-session homeostatic projections.
    pub projections: Arc<RwLock<HashMap<String, HomeostaticState>>>,
    /// The rule set used for evaluation.
    pub rules: Arc<RuleSet>,
}

impl AppState {
    /// Create a new application state with the given rule set.
    pub fn new(rules: RuleSet) -> Self {
        Self {
            projections: Arc::new(RwLock::new(HashMap::new())),
            rules: Arc::new(rules),
        }
    }

    /// Create an application state with a pre-populated projection map.
    pub fn with_projections(
        projections: Arc<RwLock<HashMap<String, HomeostaticState>>>,
        rules: RuleSet,
    ) -> Self {
        Self {
            projections,
            rules: Arc::new(rules),
        }
    }
}
