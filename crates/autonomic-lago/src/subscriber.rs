//! Lago journal subscriber.
//!
//! Subscribes to a Lago journal via `Journal.stream()` and feeds events
//! to the projection reducer to maintain per-session `HomeostaticState`.

use std::collections::HashMap;
use std::sync::Arc;

use autonomic_controller::fold;
use autonomic_core::gating::HomeostaticState;
use lago_core::journal::Journal;
use tokio::sync::RwLock;
use tokio_stream::StreamExt;
use tracing::{info, warn};

/// Shared projection state across all sessions.
pub type ProjectionMap = Arc<RwLock<HashMap<String, HomeostaticState>>>;

/// Create a new empty projection map.
pub fn new_projection_map() -> ProjectionMap {
    Arc::new(RwLock::new(HashMap::new()))
}

/// Subscribe to a Lago journal for a specific session and continuously
/// update the projection map.
///
/// This function runs until the stream ends or an error occurs.
/// It should be spawned as a tokio task.
pub async fn subscribe_session(
    journal: Arc<dyn Journal>,
    session_id: String,
    branch_id: String,
    projections: ProjectionMap,
) {
    let lago_session_id = lago_core::id::SessionId::from_string(&session_id);
    let lago_branch_id = lago_core::id::BranchId::from_string(&branch_id);

    // Get starting sequence from existing projection
    let after_seq = {
        let map = projections.read().await;
        map.get(&session_id).map_or(0, |s| s.last_event_seq)
    };

    info!(
        session_id = %session_id,
        branch_id = %branch_id,
        after_seq = after_seq,
        "subscribing to Lago journal"
    );

    let stream_result = journal
        .stream(lago_session_id, lago_branch_id, after_seq)
        .await;

    let mut stream = match stream_result {
        Ok(s) => s,
        Err(e) => {
            warn!(
                session_id = %session_id,
                error = %e,
                "failed to subscribe to Lago journal"
            );
            return;
        }
    };

    while let Some(result) = stream.next().await {
        match result {
            Ok(envelope) => {
                let seq = envelope.seq;
                let ts_ms = envelope.timestamp / 1000; // microseconds → milliseconds

                let mut map = projections.write().await;
                let state = map
                    .entry(session_id.clone())
                    .or_insert_with(|| HomeostaticState::for_agent(&session_id));
                *state = fold(state.clone(), &envelope.payload, seq, ts_ms);
            }
            Err(e) => {
                warn!(
                    session_id = %session_id,
                    error = %e,
                    "error reading from Lago journal stream"
                );
            }
        }
    }

    info!(session_id = %session_id, "Lago journal stream ended");
}

/// Load initial projection state by reading all existing events for a session.
pub async fn load_projection(
    journal: Arc<dyn Journal>,
    session_id: &str,
    branch_id: &str,
) -> Result<HomeostaticState, lago_core::error::LagoError> {
    let query = lago_core::journal::EventQuery::new()
        .session(lago_core::id::SessionId::from_string(session_id))
        .branch(lago_core::id::BranchId::from_string(branch_id));

    let events = journal.read(query).await?;

    let mut state = HomeostaticState::for_agent(session_id);
    for envelope in &events {
        let ts_ms = envelope.timestamp / 1000;
        state = fold(state, &envelope.payload, envelope.seq, ts_ms);
    }

    Ok(state)
}
