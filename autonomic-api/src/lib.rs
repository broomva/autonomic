//! HTTP API server for the Autonomic homeostasis controller.
//!
//! Provides REST endpoints for querying gating profiles and projection state.
//! Supports optional JWT authentication for protecting sensitive endpoints.

pub mod auth;
pub mod router;
pub mod state;

pub use auth::AuthConfig;
pub use router::{build_router, build_router_with_auth};
pub use state::AppState;
