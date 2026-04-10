//! Autonomic event constructors.
//!
//! Economic events use `EventKind::Custom` with `"autonomic."` prefix.
//! This is forward-compatible — Custom events round-trip through Lago.
//! Events will be promoted to canonical `EventKind` variants once stabilized.

use aios_protocol::event::EventKind;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::economic::{CostReason, EconomicMode};

/// Prefix for all Autonomic custom events.
pub const AUTONOMIC_EVENT_PREFIX: &str = "autonomic.";

/// Autonomic-specific event types that wrap as `EventKind::Custom`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "PascalCase")]
pub enum AutonomicEvent {
    /// A cost was charged to the agent.
    CostCharged {
        amount_micro_credits: i64,
        reason: CostReason,
        balance_after: i64,
    },
    /// The economic mode changed.
    EconomicModeChanged {
        from: EconomicMode,
        to: EconomicMode,
        reason: String,
    },
    /// A gating decision was made by the controller.
    GatingDecision {
        session_id: String,
        rationale: Vec<String>,
        economic_mode: EconomicMode,
    },
    /// A controller detected sustained regression and requests artifact rollback.
    RollbackRequested {
        artifact: String,
        rollback_to: String,
        reason: String,
    },
    /// Credits were deposited (revenue, grant, transfer).
    CreditDeposited {
        amount_micro_credits: i64,
        source: String,
        balance_after: i64,
    },
}

impl AutonomicEvent {
    /// Convert this event into a canonical `EventKind::Custom`.
    pub fn into_event_kind(self) -> EventKind {
        let (event_type, data) = match &self {
            Self::CostCharged {
                amount_micro_credits,
                reason,
                balance_after,
            } => (
                "autonomic.CostCharged",
                json!({
                    "amount_micro_credits": amount_micro_credits,
                    "reason": reason,
                    "balance_after": balance_after,
                }),
            ),
            Self::EconomicModeChanged { from, to, reason } => (
                "autonomic.EconomicModeChanged",
                json!({
                    "from": from,
                    "to": to,
                    "reason": reason,
                }),
            ),
            Self::GatingDecision {
                session_id,
                rationale,
                economic_mode,
            } => (
                "autonomic.GatingDecision",
                json!({
                    "session_id": session_id,
                    "rationale": rationale,
                    "economic_mode": economic_mode,
                }),
            ),
            Self::RollbackRequested {
                artifact,
                rollback_to,
                reason,
            } => (
                "autonomic.RollbackRequested",
                json!({
                    "artifact": artifact,
                    "rollback_to": rollback_to,
                    "reason": reason,
                }),
            ),
            Self::CreditDeposited {
                amount_micro_credits,
                source,
                balance_after,
            } => (
                "autonomic.CreditDeposited",
                json!({
                    "amount_micro_credits": amount_micro_credits,
                    "source": source,
                    "balance_after": balance_after,
                }),
            ),
        };
        EventKind::Custom {
            event_type: event_type.to_owned(),
            data,
        }
    }

    /// Check if a `Custom` event is an Autonomic event by its prefix.
    pub fn is_autonomic_event(event_type: &str) -> bool {
        event_type.starts_with(AUTONOMIC_EVENT_PREFIX)
    }

    /// Try to parse an `EventKind::Custom` back into an `AutonomicEvent`.
    pub fn from_custom(event_type: &str, data: &serde_json::Value) -> Option<Self> {
        if !Self::is_autonomic_event(event_type) {
            return None;
        }

        match event_type {
            "autonomic.CostCharged" => {
                let amount = data.get("amount_micro_credits")?.as_i64()?;
                let reason: CostReason =
                    serde_json::from_value(data.get("reason")?.clone()).ok()?;
                let balance = data.get("balance_after")?.as_i64()?;
                Some(Self::CostCharged {
                    amount_micro_credits: amount,
                    reason,
                    balance_after: balance,
                })
            }
            "autonomic.EconomicModeChanged" => {
                let from: EconomicMode = serde_json::from_value(data.get("from")?.clone()).ok()?;
                let to: EconomicMode = serde_json::from_value(data.get("to")?.clone()).ok()?;
                let reason = data.get("reason")?.as_str()?.to_owned();
                Some(Self::EconomicModeChanged { from, to, reason })
            }
            "autonomic.GatingDecision" => {
                let session_id = data.get("session_id")?.as_str()?.to_owned();
                let rationale: Vec<String> =
                    serde_json::from_value(data.get("rationale")?.clone()).ok()?;
                let economic_mode: EconomicMode =
                    serde_json::from_value(data.get("economic_mode")?.clone()).ok()?;
                Some(Self::GatingDecision {
                    session_id,
                    rationale,
                    economic_mode,
                })
            }
            "autonomic.RollbackRequested" => {
                let artifact = data.get("artifact")?.as_str()?.to_owned();
                let rollback_to = data.get("rollback_to")?.as_str()?.to_owned();
                let reason = data.get("reason")?.as_str()?.to_owned();
                Some(Self::RollbackRequested {
                    artifact,
                    rollback_to,
                    reason,
                })
            }
            "autonomic.CreditDeposited" => {
                let amount = data.get("amount_micro_credits")?.as_i64()?;
                let source = data.get("source")?.as_str()?.to_owned();
                let balance = data.get("balance_after")?.as_i64()?;
                Some(Self::CreditDeposited {
                    amount_micro_credits: amount,
                    source,
                    balance_after: balance,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cost_charged_to_event_kind() {
        let event = AutonomicEvent::CostCharged {
            amount_micro_credits: 150,
            reason: CostReason::ModelInference {
                model: "claude-sonnet".into(),
                prompt_tokens: 100,
                completion_tokens: 50,
            },
            balance_after: 9_999_850,
        };
        let kind = event.into_event_kind();
        if let EventKind::Custom { event_type, data } = &kind {
            assert_eq!(event_type, "autonomic.CostCharged");
            assert_eq!(data["amount_micro_credits"], 150);
            assert_eq!(data["balance_after"], 9_999_850);
        } else {
            panic!("expected Custom variant");
        }
    }

    #[test]
    fn event_kind_roundtrip_through_custom() {
        let event = AutonomicEvent::EconomicModeChanged {
            from: EconomicMode::Sovereign,
            to: EconomicMode::Conserving,
            reason: "balance dropping".into(),
        };
        let kind = event.into_event_kind();

        // Serialize and deserialize as EventKind
        let json = serde_json::to_string(&kind).unwrap();
        let back: EventKind = serde_json::from_str(&json).unwrap();

        if let EventKind::Custom { event_type, data } = back {
            assert_eq!(event_type, "autonomic.EconomicModeChanged");
            let parsed = AutonomicEvent::from_custom(&event_type, &data).unwrap();
            assert!(matches!(
                parsed,
                AutonomicEvent::EconomicModeChanged {
                    to: EconomicMode::Conserving,
                    ..
                }
            ));
        } else {
            panic!("expected Custom variant after roundtrip");
        }
    }

    #[test]
    fn is_autonomic_event_prefix() {
        assert!(AutonomicEvent::is_autonomic_event("autonomic.CostCharged"));
        assert!(AutonomicEvent::is_autonomic_event("autonomic.Anything"));
        assert!(!AutonomicEvent::is_autonomic_event("other.Event"));
        assert!(!AutonomicEvent::is_autonomic_event("CostCharged"));
    }

    #[test]
    fn from_custom_returns_none_for_non_autonomic() {
        let result = AutonomicEvent::from_custom("other.Event", &json!({}));
        assert!(result.is_none());
    }

    #[test]
    fn credit_deposited_roundtrip() {
        let event = AutonomicEvent::CreditDeposited {
            amount_micro_credits: 5_000_000,
            source: "grant".into(),
            balance_after: 15_000_000,
        };
        let kind = event.into_event_kind();
        if let EventKind::Custom { event_type, data } = kind {
            let parsed = AutonomicEvent::from_custom(&event_type, &data).unwrap();
            assert!(matches!(
                parsed,
                AutonomicEvent::CreditDeposited {
                    amount_micro_credits: 5_000_000,
                    ..
                }
            ));
        } else {
            panic!("expected Custom");
        }
    }

    #[test]
    fn rollback_requested_roundtrip() {
        let event = AutonomicEvent::RollbackRequested {
            artifact: "knowledge_thresholds".into(),
            rollback_to: "v1".into(),
            reason: "post-promotion regression detected".into(),
        };
        let kind = event.into_event_kind();
        if let EventKind::Custom { event_type, data } = kind {
            assert_eq!(event_type, "autonomic.RollbackRequested");
            assert_eq!(data["artifact"], "knowledge_thresholds");
            assert_eq!(data["rollback_to"], "v1");

            let parsed = AutonomicEvent::from_custom(&event_type, &data).unwrap();
            assert!(matches!(
                parsed,
                AutonomicEvent::RollbackRequested {
                    rollback_to,
                    ..
                } if rollback_to == "v1"
            ));
        } else {
            panic!("expected Custom");
        }
    }
}
