//! Economic identity for agents.
//!
//! Each agent has a unique economic identity derived from a secp256k1 keypair.
//! The identity enables future payment flows and inter-agent credit transfers.

use serde::{Deserialize, Serialize};

/// An agent's economic identity — the anchor for all financial operations.
///
/// In Phase 0, this is a simple address placeholder. Phase 2 adds keypair
/// generation via `k256` and encrypted key storage as Lago blobs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EconomicIdentity {
    /// Hex-encoded address derived from the public key.
    pub address: String,
    /// Blob ID where the encrypted private key is stored (if any).
    pub key_blob_id: Option<String>,
    /// Timestamp when this identity was created (ms since epoch).
    pub created_at: u64,
}

impl EconomicIdentity {
    /// Create a placeholder identity with the given address.
    pub fn placeholder(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            key_blob_id: None,
            created_at: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_serde_roundtrip() {
        let id = EconomicIdentity {
            address: "0xdeadbeef".into(),
            key_blob_id: Some("blob-123".into()),
            created_at: 1700000000000,
        };
        let json = serde_json::to_string(&id).unwrap();
        let back: EconomicIdentity = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }

    #[test]
    fn identity_placeholder() {
        let id = EconomicIdentity::placeholder("0xabc");
        assert_eq!(id.address, "0xabc");
        assert!(id.key_blob_id.is_none());
    }
}
