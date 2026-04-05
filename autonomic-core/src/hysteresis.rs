//! Hysteresis gate primitive for preventing mode flapping.
//!
//! A `HysteresisGate` uses separate enter/exit thresholds and a minimum
//! hold duration to create stable state transitions. Without hysteresis,
//! a metric oscillating near a threshold causes rapid state changes.

use serde::{Deserialize, Serialize};
use tracing::{Span, instrument};

/// A hysteresis gate with separate enter/exit thresholds and min-hold duration.
///
/// The gate activates when `metric >= threshold_enter` and deactivates when
/// `metric <= threshold_exit`, provided `min_hold_ms` has elapsed since the
/// last transition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HysteresisGate {
    /// Metric value at which the gate activates.
    pub threshold_enter: f64,
    /// Metric value at which the gate deactivates (must be < `threshold_enter`).
    pub threshold_exit: f64,
    /// Minimum time (ms) the gate must remain in its current state before transitioning.
    pub min_hold_ms: u64,
    /// Whether the gate is currently active.
    pub active: bool,
    /// Timestamp (ms since epoch) of the last state transition.
    pub last_transition_ms: u64,
}

impl HysteresisGate {
    /// Create a new inactive gate.
    pub fn new(threshold_enter: f64, threshold_exit: f64, min_hold_ms: u64) -> Self {
        Self {
            threshold_enter,
            threshold_exit,
            min_hold_ms,
            active: false,
            last_transition_ms: 0,
        }
    }

    /// Evaluate the gate with the given metric value at the given timestamp.
    ///
    /// Returns the (potentially updated) active state.
    #[instrument(skip(self), fields(autonomic.hysteresis.metric = metric, autonomic.hysteresis.active))]
    pub fn evaluate(&mut self, metric: f64, now_ms: u64) -> bool {
        let was_active = self.active;
        let held_long_enough = now_ms.saturating_sub(self.last_transition_ms) >= self.min_hold_ms;

        if !self.active && metric >= self.threshold_enter && held_long_enough {
            self.active = true;
            self.last_transition_ms = now_ms;
        } else if self.active && metric <= self.threshold_exit && held_long_enough {
            self.active = false;
            self.last_transition_ms = now_ms;
        }

        let span = Span::current();
        span.record("autonomic.hysteresis.active", self.active);
        if was_active != self.active {
            tracing::debug!(
                from = was_active,
                to = self.active,
                "hysteresis gate state changed"
            );
        }

        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gate_activates_at_enter_threshold() {
        let mut gate = HysteresisGate::new(0.8, 0.6, 0);
        assert!(!gate.active);

        // Below enter threshold — stays inactive
        assert!(!gate.evaluate(0.7, 100));
        assert!(!gate.active);

        // At enter threshold — activates
        assert!(gate.evaluate(0.8, 200));
        assert!(gate.active);
    }

    #[test]
    fn gate_deactivates_at_exit_threshold() {
        let mut gate = HysteresisGate::new(0.8, 0.6, 0);
        gate.active = true;
        gate.last_transition_ms = 0;

        // Above exit threshold — stays active
        assert!(gate.evaluate(0.7, 100));

        // At exit threshold — deactivates
        assert!(!gate.evaluate(0.6, 200));
        assert!(!gate.active);
    }

    #[test]
    fn gate_respects_min_hold_duration() {
        let mut gate = HysteresisGate::new(0.8, 0.6, 1000);

        // Activate
        assert!(gate.evaluate(0.9, 1000));
        assert!(gate.active);

        // Try to deactivate too soon — min_hold not met
        assert!(gate.evaluate(0.5, 1500));
        assert!(gate.active); // still active

        // Now enough time has passed
        assert!(!gate.evaluate(0.5, 2000));
        assert!(!gate.active);
    }

    #[test]
    fn gate_hysteresis_prevents_flapping() {
        let mut gate = HysteresisGate::new(0.8, 0.6, 0);

        // Metric between thresholds: activate first
        gate.evaluate(0.9, 100);
        assert!(gate.active);

        // Metric drops to 0.7 — between enter and exit — stays active
        gate.evaluate(0.7, 200);
        assert!(gate.active);

        // Metric drops to 0.6 — at exit — deactivates
        gate.evaluate(0.6, 300);
        assert!(!gate.active);

        // Metric rises to 0.7 — between thresholds — stays inactive
        gate.evaluate(0.7, 400);
        assert!(!gate.active);
    }

    #[test]
    fn gate_serde_roundtrip() {
        let gate = HysteresisGate::new(0.8, 0.6, 5000);
        let json = serde_json::to_string(&gate).unwrap();
        let back: HysteresisGate = serde_json::from_str(&json).unwrap();
        assert!((back.threshold_enter - 0.8).abs() < f64::EPSILON);
        assert!((back.threshold_exit - 0.6).abs() < f64::EPSILON);
        assert_eq!(back.min_hold_ms, 5000);
    }
}
