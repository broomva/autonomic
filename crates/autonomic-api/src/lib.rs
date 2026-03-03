//! HTTP API server for the Autonomic homeostasis controller.
//!
//! Provides REST endpoints for querying gating profiles and projection state.

pub mod router;
pub mod state;

pub use router::build_router;
pub use state::AppState;
