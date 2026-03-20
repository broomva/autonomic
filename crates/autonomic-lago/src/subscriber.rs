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
use tracing::{info, instrument, warn};

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
#[instrument(skip(journal, projections), fields(lago.stream_id = %session_id))]
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

/// Load the initial projection state by reading all existing events for a session.
#[instrument(skip(journal), fields(lago.stream_id = %session_id))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use autonomic_core::economic::{CostReason, EconomicMode};
    use autonomic_core::events::AutonomicEvent;
    use lago_core::event::EventEnvelope;
    use lago_core::id::{BranchId, EventId, SessionId};
    use lago_journal::RedbJournal;

    fn open_journal(dir: &std::path::Path) -> Arc<dyn Journal> {
        let db_path = dir.join("test.redb");
        Arc::new(RedbJournal::open(db_path).unwrap()) as Arc<dyn Journal>
    }

    /// Write autonomic events directly to the journal as `EventEnvelope`s.
    async fn write_events(
        journal: &Arc<dyn Journal>,
        session_id: &str,
        events: Vec<AutonomicEvent>,
    ) {
        let envelopes: Vec<EventEnvelope> = events
            .into_iter()
            .map(|event| EventEnvelope {
                event_id: EventId::new(),
                session_id: SessionId::from_string(session_id),
                branch_id: BranchId::from_string("main"),
                run_id: None,
                seq: 0,
                timestamp: EventEnvelope::now_micros(),
                parent_id: None,
                payload: event.into_event_kind(),
                metadata: std::collections::HashMap::new(),
                schema_version: 1,
            })
            .collect();
        journal.append_batch(envelopes).await.unwrap();
    }

    #[tokio::test]
    async fn load_projection_empty_session() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());

        let state = load_projection(journal, "empty-session", "main")
            .await
            .unwrap();

        assert_eq!(state.agent_id, "empty-session");
        assert_eq!(state.last_event_seq, 0);
        assert_eq!(state.economic.balance_micro_credits, 10_000_000); // default
    }

    #[tokio::test]
    async fn load_projection_replays_events() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());

        // Write a CostCharged event
        write_events(
            &journal,
            "replay-sess",
            vec![AutonomicEvent::CostCharged {
                amount_micro_credits: 500,
                reason: CostReason::ModelInference {
                    model: "sonnet".into(),
                    prompt_tokens: 100,
                    completion_tokens: 50,
                },
                balance_after: 9_999_500,
            }],
        )
        .await;

        let state = load_projection(journal, "replay-sess", "main")
            .await
            .unwrap();

        assert_eq!(state.economic.balance_micro_credits, 9_999_500);
        assert_eq!(state.economic.lifetime_costs, 500);
        assert!(state.last_event_seq > 0);
    }

    #[tokio::test]
    async fn load_projection_preserves_economic_state() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());

        // Write multiple autonomic events in sequence
        write_events(
            &journal,
            "econ-sess",
            vec![
                AutonomicEvent::CostCharged {
                    amount_micro_credits: 1000,
                    reason: CostReason::ToolExecution {
                        tool_name: "search".into(),
                    },
                    balance_after: 9_999_000,
                },
                AutonomicEvent::EconomicModeChanged {
                    from: EconomicMode::Sovereign,
                    to: EconomicMode::Conserving,
                    reason: "balance declining".into(),
                },
                AutonomicEvent::CreditDeposited {
                    amount_micro_credits: 5_000_000,
                    source: "grant".into(),
                    balance_after: 14_999_000,
                },
            ],
        )
        .await;

        let state = load_projection(journal, "econ-sess", "main").await.unwrap();

        // After CostCharged: balance=9_999_000, costs=1000
        // After EconomicModeChanged: mode=Conserving
        // After CreditDeposited: balance=14_999_000, revenue=5_000_000
        assert_eq!(state.economic.balance_micro_credits, 14_999_000);
        assert_eq!(state.economic.mode, EconomicMode::Conserving);
        assert_eq!(state.economic.lifetime_costs, 1000);
        assert_eq!(state.economic.lifetime_revenue, 5_000_000);
    }

    #[tokio::test]
    async fn new_projection_map_is_empty() {
        let map = new_projection_map();
        let inner = map.read().await;
        assert!(inner.is_empty());
    }
}
