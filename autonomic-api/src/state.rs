//! Shared application state for the HTTP API.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use autonomic_core::gating::HomeostaticState;
use autonomic_core::rules::RuleSet;
use lago_core::journal::Journal;
use tokio::sync::RwLock;
use tokio_util::task::TaskTracker;

/// Shared state for the axum HTTP server.
#[derive(Clone)]
pub struct AppState {
    /// Per-session homeostatic projections.
    pub projections: Arc<RwLock<HashMap<String, HomeostaticState>>>,
    /// The rule set used for evaluation.
    pub rules: Arc<RuleSet>,
    /// Optional Lago journal for on-demand session bootstrapping.
    pub journal: Option<Arc<dyn Journal>>,
    /// Daemon startup time for uptime reporting.
    pub started_at: Instant,
    /// Tracks spawned background tasks for graceful shutdown.
    pub task_tracker: TaskTracker,
}

impl AppState {
    /// Create a new application state with the given rule set (standalone mode).
    pub fn new(rules: RuleSet) -> Self {
        Self {
            projections: Arc::new(RwLock::new(HashMap::new())),
            rules: Arc::new(rules),
            journal: None,
            started_at: Instant::now(),
            task_tracker: TaskTracker::new(),
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
            journal: None,
            started_at: Instant::now(),
            task_tracker: TaskTracker::new(),
        }
    }

    /// Create an application state with a Lago journal for on-demand bootstrapping.
    pub fn with_journal(
        projections: Arc<RwLock<HashMap<String, HomeostaticState>>>,
        rules: RuleSet,
        journal: Arc<dyn Journal>,
    ) -> Self {
        Self {
            projections,
            rules: Arc::new(rules),
            journal: Some(journal),
            started_at: Instant::now(),
            task_tracker: TaskTracker::new(),
        }
    }
}
