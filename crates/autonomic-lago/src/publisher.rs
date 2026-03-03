//! Event publisher: writes Autonomic decisions back to Lago.
//!
//! This allows the Autonomic controller's decisions to be persisted
//! in the event journal, enabling replay and audit.

use std::sync::Arc;

use autonomic_core::events::AutonomicEvent;
use lago_core::event::EventEnvelope;
use lago_core::id::{BranchId, EventId, SeqNo, SessionId};
use lago_core::journal::Journal;
use tracing::warn;

/// Publish an Autonomic event to the Lago journal.
pub async fn publish_event(
    journal: Arc<dyn Journal>,
    session_id: &str,
    branch_id: &str,
    event: AutonomicEvent,
) -> Result<SeqNo, lago_core::error::LagoError> {
    let envelope = EventEnvelope {
        event_id: EventId::new(),
        session_id: SessionId::from_string(session_id),
        branch_id: BranchId::from_string(branch_id),
        run_id: None,
        seq: 0, // Journal assigns the actual sequence number
        timestamp: lago_core::event::EventEnvelope::now_micros(),
        parent_id: None,
        payload: event.into_event_kind(),
        metadata: std::collections::HashMap::new(),
        schema_version: 1,
    };

    journal.append(envelope).await
}

/// Publish a batch of Autonomic events atomically.
pub async fn publish_events(
    journal: Arc<dyn Journal>,
    session_id: &str,
    branch_id: &str,
    events: Vec<AutonomicEvent>,
) -> Result<SeqNo, lago_core::error::LagoError> {
    if events.is_empty() {
        warn!("publish_events called with empty event list");
        // Return 0 as a no-op sequence number
        return Ok(0);
    }

    let envelopes: Vec<EventEnvelope> = events
        .into_iter()
        .map(|event| EventEnvelope {
            event_id: EventId::new(),
            session_id: SessionId::from_string(session_id),
            branch_id: BranchId::from_string(branch_id),
            run_id: None,
            seq: 0,
            timestamp: lago_core::event::EventEnvelope::now_micros(),
            parent_id: None,
            payload: event.into_event_kind(),
            metadata: std::collections::HashMap::new(),
            schema_version: 1,
        })
        .collect();

    journal.append_batch(envelopes).await
}
