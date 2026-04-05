//! Context compression regulation types.
//!
//! `ContextRuling` is the Autonomic controller's decision about whether
//! to compress, dilate, or hold the conversation context. It replaces
//! hard-coded token thresholds with a regulated control signal.

use serde::{Deserialize, Serialize};

/// The Autonomic controller's ruling on context compression.
///
/// Maps to biological analogy: breathing. The context window is a lung —
/// it fills (inspiration) and must periodically release (expiration).
/// Autonomic regulation decides the breathing rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextRuling {
    /// Context pressure is low. No action needed.
    Breathe,
    /// Context filling but agent doing valuable work. Delay compression.
    Dilate,
    /// Context should be compressed. Extract memories and compact.
    Compress,
    /// Critical pressure. Compact immediately to avoid API errors.
    Emergency,
}

/// Advice package returned by the `ContextCompressionRule`.
///
/// Contains the ruling plus the rationale signals that informed it,
/// so the shell can log why a particular decision was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextCompressionAdvice {
    /// The ruling: what the shell should do.
    pub ruling: ContextRuling,
    /// Context pressure that triggered evaluation (0.0..1.0).
    pub pressure: f32,
    /// Target token count if compression is needed.
    pub target_tokens: Option<usize>,
    /// Human-readable rationale for the ruling.
    pub rationale: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_ruling_serde_roundtrip() {
        for ruling in [
            ContextRuling::Breathe,
            ContextRuling::Dilate,
            ContextRuling::Compress,
            ContextRuling::Emergency,
        ] {
            let json = serde_json::to_string(&ruling).unwrap();
            let back: ContextRuling = serde_json::from_str(&json).unwrap();
            assert_eq!(ruling, back);
        }
    }

    #[test]
    fn context_compression_advice_serde_roundtrip() {
        let advice = ContextCompressionAdvice {
            ruling: ContextRuling::Dilate,
            pressure: 0.68,
            target_tokens: None,
            rationale: "high tool density, quality stable".into(),
        };
        let json = serde_json::to_string(&advice).unwrap();
        let back: ContextCompressionAdvice = serde_json::from_str(&json).unwrap();
        assert_eq!(back.ruling, ContextRuling::Dilate);
        assert!((back.pressure - 0.68).abs() < f32::EPSILON);
        assert!(back.target_tokens.is_none());
    }

    #[test]
    fn compress_advice_has_target() {
        let advice = ContextCompressionAdvice {
            ruling: ContextRuling::Compress,
            pressure: 0.75,
            target_tokens: Some(70_000),
            rationale: "quality degrading".into(),
        };
        assert_eq!(advice.target_tokens, Some(70_000));
    }
}
