//! Lago bridge for the Autonomic homeostasis controller.
//!
//! Provides event subscription (reading from Lago journal) and
//! event publishing (writing Autonomic decisions back to Lago).

pub mod publisher;
pub mod subscriber;

pub use publisher::{publish_event, publish_events};
pub use subscriber::{ProjectionMap, load_projection, new_projection_map, subscribe_session};
