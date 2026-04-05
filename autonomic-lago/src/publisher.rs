//! Event publisher: writes Autonomic decisions back to Lago.
//!
//! This allows the Autonomic controller's decisions to be persisted
//! in the event journal, enabling replay and audit.

use std::sync::Arc;

use autonomic_core::events::AutonomicEvent;
use lago_core::event::EventEnvelope;
use lago_core::id::{BranchId, EventId, SeqNo, SessionId};
use lago_core::journal::Journal;
use tracing::{instrument, warn};

/// Publish an Autonomic event to the Lago journal.
#[instrument(skip(journal, event), fields(lago.stream_id = %session_id))]
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
#[instrument(skip(journal, events), fields(lago.stream_id = %session_id, lago.event_count = events.len()))]
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

#[cfg(test)]
mod tests {
    use super::*;
    use autonomic_core::economic::{CostReason, EconomicMode};
    use autonomic_core::events::AutonomicEvent;
    use lago_core::journal::EventQuery;
    use lago_journal::RedbJournal;

    fn open_journal(dir: &std::path::Path) -> Arc<dyn Journal> {
        let db_path = dir.join("test.redb");
        Arc::new(RedbJournal::open(db_path).unwrap()) as Arc<dyn Journal>
    }

    #[tokio::test]
    async fn publish_single_event_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());

        let event = AutonomicEvent::CostCharged {
            amount_micro_credits: 500,
            reason: CostReason::ModelInference {
                model: "claude-sonnet".into(),
                prompt_tokens: 100,
                completion_tokens: 50,
            },
            balance_after: 9_999_500,
        };

        let seq = publish_event(journal.clone(), "sess-1", "main", event)
            .await
            .unwrap();
        assert!(seq > 0);

        // Read back and verify
        let query = EventQuery::new()
            .session(lago_core::id::SessionId::from_string("sess-1"))
            .branch(lago_core::id::BranchId::from_string("main"));
        let events = journal.read(query).await.unwrap();
        assert_eq!(events.len(), 1);

        if let aios_protocol::event::EventKind::Custom { event_type, data } = &events[0].payload {
            assert_eq!(event_type, "autonomic.CostCharged");
            assert_eq!(data["amount_micro_credits"], 500);
            assert_eq!(data["balance_after"], 9_999_500);
        } else {
            panic!("expected Custom event kind");
        }
    }

    #[tokio::test]
    async fn publish_batch_monotonic_sequences() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());

        let events = vec![
            AutonomicEvent::CostCharged {
                amount_micro_credits: 100,
                reason: CostReason::ToolExecution {
                    tool_name: "read_file".into(),
                },
                balance_after: 9_999_900,
            },
            AutonomicEvent::CostCharged {
                amount_micro_credits: 200,
                reason: CostReason::ToolExecution {
                    tool_name: "write_file".into(),
                },
                balance_after: 9_999_700,
            },
            AutonomicEvent::EconomicModeChanged {
                from: EconomicMode::Sovereign,
                to: EconomicMode::Conserving,
                reason: "balance dropping".into(),
            },
        ];

        let final_seq = publish_events(journal.clone(), "sess-2", "main", events)
            .await
            .unwrap();
        assert!(final_seq >= 3);

        // Verify all 3 events persisted with monotonic sequences
        let query = EventQuery::new()
            .session(lago_core::id::SessionId::from_string("sess-2"))
            .branch(lago_core::id::BranchId::from_string("main"));
        let stored = journal.read(query).await.unwrap();
        assert_eq!(stored.len(), 3);

        for window in stored.windows(2) {
            assert!(window[1].seq > window[0].seq, "sequences must be monotonic");
        }
    }

    #[tokio::test]
    async fn publish_empty_batch_noop() {
        let dir = tempfile::tempdir().unwrap();
        let journal = open_journal(dir.path());

        let seq = publish_events(journal, "sess-3", "main", vec![])
            .await
            .unwrap();
        assert_eq!(seq, 0);
    }

    #[tokio::test]
    async fn autonomic_event_converts_to_custom_kind() {
        let event = AutonomicEvent::CreditDeposited {
            amount_micro_credits: 5_000_000,
            source: "grant".into(),
            balance_after: 15_000_000,
        };
        let kind = event.into_event_kind();
        match kind {
            aios_protocol::event::EventKind::Custom { event_type, .. } => {
                assert!(event_type.starts_with("autonomic."));
                assert_eq!(event_type, "autonomic.CreditDeposited");
            }
            _ => panic!("expected Custom variant"),
        }
    }
}
