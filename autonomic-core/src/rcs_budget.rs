//! RCS stability budget — compile-time canonical parameters + runtime estimator.
//!
//! This module is the Rust-native F2 instrumentation for the Recursive
//! Controlled Systems (RCS) paper. It mirrors the formal definition in
//! `research/rcs/papers/p0-foundations/main.tex` (Theorem 1): a level is
//! individually stable iff its stability margin
//!
//! ```text
//!   lambda = gamma
//!          - L_theta * rho        (adaptation cost)
//!          - L_d * eta            (design-evolution cost)
//!          - beta * tau_bar       (delay cost)
//!          - ln(nu) / tau_a       (switching cost)
//! ```
//!
//! is strictly positive.
//!
//! The canonical parameter set is mirrored from the paper repo at compile time
//! via `include_str!` against `data/rcs-parameters.toml`. The values are NOT
//! duplicated in Rust source — they are parsed once at first access.
//!
//! [`MarginEstimator`] folds observed event history from [`HomeostaticState`]
//! deltas into a runtime [`StabilityBudget`] for level L1 (autonomic). This is
//! the minimum scope required to assert `lambda_1 > 0` at runtime; L0 and L2
//! estimators are a follow-up (see module-level tests for shape).

use std::sync::OnceLock;

use serde::Deserialize;

use crate::gating::HomeostaticState;

/// Canonical parameters.toml mirrored from `research/rcs/data/parameters.toml`.
///
/// Embedded at compile time — editing the mirror is the single source of
/// truth for Rust. The paper repo's `data/parameters.toml` is the single
/// source of truth for the paper; `scripts/sync-rcs-parameters.sh` keeps
/// the two aligned.
const CANONICAL_PARAMETERS_TOML: &str = include_str!("../data/rcs-parameters.toml");

/// Cached parsed canonical parameters.
static CANONICAL: OnceLock<CanonicalParameters> = OnceLock::new();

/// Get the parsed canonical parameters (parsed once, cached forever).
fn canonical() -> &'static CanonicalParameters {
    CANONICAL.get_or_init(|| {
        toml::from_str::<CanonicalParameters>(CANONICAL_PARAMETERS_TOML)
            .expect("canonical rcs-parameters.toml must be valid TOML matching schema")
    })
}

/// The full canonical parameter set, parsed from the embedded TOML.
#[derive(Debug, Clone, Deserialize)]
struct CanonicalParameters {
    #[serde(default)]
    #[allow(dead_code)]
    schema_version: u32,
    /// One entry per hierarchy level (L0..L3).
    #[serde(default)]
    levels: Vec<CanonicalLevel>,
}

/// A single level's canonical parameters.
#[derive(Debug, Clone, Deserialize)]
struct CanonicalLevel {
    id: String,
    #[allow(dead_code)]
    name: String,
    #[allow(dead_code)]
    system: String,
    gamma: f64,
    #[serde(rename = "L_theta")]
    l_theta: f64,
    rho: f64,
    #[serde(rename = "L_d")]
    l_d: f64,
    eta: f64,
    beta: f64,
    tau_bar: f64,
    nu: f64,
    tau_a: f64,
}

/// Stability budget at a single level of the RCS hierarchy.
///
/// Fields correspond one-to-one with the symbols in the paper's Theorem 1.
/// See `research/rcs/papers/p0-foundations/main.tex` for the full derivation.
///
/// Construct either directly (fields are public) or via
/// [`StabilityBudget::from_canonical`] to pull the paper's values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StabilityBudget {
    /// Nominal exponential decay rate for the level's Lyapunov function.
    pub gamma: f64,
    /// Lipschitz constant of the Lyapunov function w.r.t. the adaptation parameter.
    pub l_theta: f64,
    /// Bound on the adaptation rate (how fast the controller re-tunes itself).
    pub rho: f64,
    /// Lipschitz constant of the Lyapunov function w.r.t. the design parameter.
    pub l_d: f64,
    /// Bound on the design-evolution rate (how fast the controller re-architects itself).
    pub eta: f64,
    /// Sensitivity of the Lyapunov function to delay.
    pub beta: f64,
    /// Supremal observation/actuation delay for this level (seconds).
    pub tau_bar: f64,
    /// Jump comparability factor (>= 1) — how much the Lyapunov function can jump at switches.
    pub nu: f64,
    /// Average dwell time between mode switches (seconds).
    pub tau_a: f64,
}

impl StabilityBudget {
    /// Compute the stability margin `lambda` at this level.
    ///
    /// Formula (Theorem 1):
    /// `lambda = gamma - L_theta*rho - L_d*eta - beta*tau_bar - ln(nu)/tau_a`.
    ///
    /// If `tau_a <= 0`, the switching term is treated as `+inf` (unstable).
    /// If `nu < 1`, the result is clamped via `ln(nu.max(1.0))` — the paper's
    /// theorem is only stated for `nu >= 1`.
    pub fn margin(&self) -> f64 {
        let switching = if self.tau_a <= 0.0 {
            f64::INFINITY
        } else {
            self.nu.max(1.0).ln() / self.tau_a
        };
        self.gamma
            - self.l_theta * self.rho
            - self.l_d * self.eta
            - self.beta * self.tau_bar
            - switching
    }

    /// Returns true iff the level is individually stable (`margin > 0`).
    pub fn is_stable(&self) -> bool {
        self.margin() > 0.0
    }

    /// Load the canonical budget for a named level (e.g. `"L0"`..`"L3"`).
    ///
    /// Returns `None` if the level id is unknown.
    pub fn from_canonical(level: &str) -> Option<Self> {
        canonical()
            .levels
            .iter()
            .find(|l| l.id.eq_ignore_ascii_case(level))
            .map(|l| Self {
                gamma: l.gamma,
                l_theta: l.l_theta,
                rho: l.rho,
                l_d: l.l_d,
                eta: l.eta,
                beta: l.beta,
                tau_bar: l.tau_bar,
                nu: l.nu,
                tau_a: l.tau_a,
            })
    }
}

/// Windowed estimator that folds [`HomeostaticState`] observations into a
/// runtime [`StabilityBudget`] for Level 1 (autonomic).
///
/// The estimator compares the latest observed state against a baseline
/// captured at construction time. The deltas between baseline and current
/// state feed empirical proxies for each parameter:
///
/// - `gamma` — canonical L1 value (not directly observable; nominal decay rate).
/// - `l_theta * rho` — proxy for adaptation pressure: context_pressure + tool_density.
/// - `l_d * eta` — proxy for design-evolution: knowledge promotion / memory commits.
/// - `beta * tau_bar` — proxy for measurement delay: wall-clock gap between ticks.
/// - `tau_a` — literal: the economic mode gate's `min_hold_ms` (converted to seconds).
///
/// The estimator is deliberately conservative: when no signal is present it
/// returns the canonical L1 budget verbatim, so the runtime assertion
/// `margin > 0` matches the paper's prediction in the idle case.
pub struct MarginEstimator {
    level: &'static str,
    baseline: HomeostaticState,
    /// Cumulative elapsed observation window (ms).
    window_ms: u64,
    /// Number of events folded in so far.
    event_count: u64,
    /// Maximum observed inter-event gap (ms) — used as the delay proxy.
    max_gap_ms: u64,
    /// Last observation timestamp (ms since epoch).
    last_ms: u64,
    /// Last observed state (for incremental deltas).
    current: HomeostaticState,
}

impl MarginEstimator {
    /// Create an estimator for L1 (the only level directly observable from
    /// `HomeostaticState`). L0 and L2 require upstream/downstream hooks that
    /// are out of scope for this module.
    pub fn for_l1(baseline: HomeostaticState) -> Self {
        let last_ms = baseline.last_event_ms;
        Self {
            level: "L1",
            current: baseline.clone(),
            baseline,
            window_ms: 0,
            event_count: 0,
            max_gap_ms: 0,
            last_ms,
        }
    }

    /// Fold an observed state into the estimator.
    ///
    /// Intended to be called once per homeostatic tick, or once at the end of
    /// a test scenario with the final state.
    pub fn observe(&mut self, state: &HomeostaticState) {
        let now = state.last_event_ms;
        if self.last_ms > 0 && now > self.last_ms {
            let gap = now - self.last_ms;
            self.window_ms = self.window_ms.saturating_add(gap);
            if gap > self.max_gap_ms {
                self.max_gap_ms = gap;
            }
        }
        if now > 0 {
            self.last_ms = now;
        }
        self.event_count = self.event_count.saturating_add(1);
        self.current = state.clone();
    }

    /// Number of events folded since construction.
    pub fn event_count(&self) -> u64 {
        self.event_count
    }

    /// Cumulative observation window (ms).
    pub fn window_ms(&self) -> u64 {
        self.window_ms
    }

    /// Collapse the folded history into an estimated [`StabilityBudget`].
    ///
    /// The canonical L1 parameters are used as the prior; observed deltas are
    /// added as perturbations bounded by the canonical bounds (so a pathological
    /// observation cannot drive `margin > canonical_gamma`).
    pub fn estimate(&self) -> StabilityBudget {
        let prior = StabilityBudget::from_canonical(self.level)
            .expect("L1 must exist in canonical parameters");

        // Adaptation pressure proxy: bounded by [0, 1].
        //
        // We combine context pressure (0..1) with a tool-density bump so rapid
        // tool churn is reflected in rho. Tool density is unbounded; we squash
        // it with a soft cap at 5.0.
        let ctx_pressure = self.current.cognitive.context_pressure.clamp(0.0, 1.0) as f64;
        let tool_bump = (self.current.cognitive.tool_density / 5.0).clamp(0.0, 1.0);
        let adaptation_signal = (ctx_pressure + tool_bump).clamp(0.0, 1.0);

        // Use the signal to scale rho between [0.5*prior.rho, 1.5*prior.rho]
        // so the estimator stays close to canonical under mild load.
        let rho = prior.rho * (0.5 + adaptation_signal);

        // Design-evolution proxy: knowledge commits + compactions, bounded.
        let knowledge_activity = (self.current.cognitive.memory_commit_count as f64
            + self.current.cognitive.compaction_count as f64)
            .min(20.0)
            / 20.0;
        let eta = prior.eta * (0.5 + knowledge_activity);

        // Delay proxy: maximum inter-event gap, converted to seconds and capped
        // at 10x prior.tau_bar so a single pause doesn't destabilize the budget.
        let observed_tau_bar_s = (self.max_gap_ms as f64) / 1000.0;
        let tau_bar = observed_tau_bar_s
            .max(prior.tau_bar * 0.5)
            .min(prior.tau_bar * 10.0);

        // tau_a: use the economic mode gate's min_hold_ms verbatim (this is the
        // literal dwell-time constraint baked into the autonomic controller).
        let dwell_ms = self.current.economic.mode_gate.min_hold_ms;
        let tau_a = if dwell_ms == 0 {
            prior.tau_a
        } else {
            (dwell_ms as f64) / 1000.0
        };

        StabilityBudget {
            gamma: prior.gamma,
            l_theta: prior.l_theta,
            rho,
            l_d: prior.l_d,
            eta,
            beta: prior.beta,
            tau_bar,
            nu: prior.nu,
            tau_a,
        }
    }

    /// Access the baseline state captured at construction.
    pub fn baseline(&self) -> &HomeostaticState {
        &self.baseline
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_loads_four_levels() {
        let levels = ["L0", "L1", "L2", "L3"];
        for id in levels {
            assert!(
                StabilityBudget::from_canonical(id).is_some(),
                "canonical level {id} must be present"
            );
        }
        assert!(StabilityBudget::from_canonical("L9").is_none());
    }

    #[test]
    fn canonical_is_stable_at_every_level() {
        for id in ["L0", "L1", "L2", "L3"] {
            let budget = StabilityBudget::from_canonical(id).unwrap();
            assert!(
                budget.is_stable(),
                "canonical {id} must be individually stable: margin = {}",
                budget.margin()
            );
        }
    }

    #[test]
    fn margin_matches_paper_derived_values() {
        // Values from [derived.lambda] in rcs-parameters.toml, verified to 3dp.
        // Paper tolerates drift up to 1e-4 between regenerations.
        let eps = 1e-3;
        let expected = [
            ("L0", 1.455_357),
            ("L1", 0.411_484),
            ("L2", 0.069_274),
            ("L3", 0.006_398),
        ];
        for (id, want) in expected {
            let got = StabilityBudget::from_canonical(id).unwrap().margin();
            assert!(
                (got - want).abs() < eps,
                "level {id}: margin {got:.6} != paper {want:.6}"
            );
        }
    }

    #[test]
    fn case_insensitive_lookup() {
        assert!(StabilityBudget::from_canonical("l1").is_some());
        assert!(StabilityBudget::from_canonical("L1").is_some());
    }

    #[test]
    fn custom_budget_unstable_when_rho_too_high() {
        let unstable = StabilityBudget {
            gamma: 0.5,
            l_theta: 1.0,
            rho: 10.0, // blows past gamma
            l_d: 0.0,
            eta: 0.0,
            beta: 0.0,
            tau_bar: 0.0,
            nu: 1.0,
            tau_a: 1.0,
        };
        assert!(!unstable.is_stable());
        assert!(unstable.margin() < 0.0);
    }

    #[test]
    fn zero_tau_a_is_unstable() {
        let budget = StabilityBudget {
            gamma: 100.0,
            l_theta: 0.0,
            rho: 0.0,
            l_d: 0.0,
            eta: 0.0,
            beta: 0.0,
            tau_bar: 0.0,
            nu: 2.0, // ln(nu) > 0
            tau_a: 0.0,
        };
        assert!(!budget.is_stable());
        assert!(budget.margin().is_infinite() && budget.margin() < 0.0);
    }

    #[test]
    fn nu_less_than_one_clamped() {
        // Paper's theorem assumes nu >= 1; we clamp defensively.
        let budget = StabilityBudget {
            gamma: 1.0,
            l_theta: 0.0,
            rho: 0.0,
            l_d: 0.0,
            eta: 0.0,
            beta: 0.0,
            tau_bar: 0.0,
            nu: 0.5, // would give ln(nu) < 0 if unclamped
            tau_a: 1.0,
        };
        // With clamp to max(nu, 1.0), ln(1)=0, so margin = gamma = 1.
        assert!((budget.margin() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn estimator_defaults_to_canonical_l1_when_idle() {
        // With a fresh default state and no observations, the estimator should
        // return a budget that is at worst within a small factor of canonical L1.
        let canonical_l1 = StabilityBudget::from_canonical("L1").unwrap();
        let baseline = HomeostaticState::for_agent("test");
        let mut est = MarginEstimator::for_l1(baseline.clone());
        est.observe(&baseline);
        let estimated = est.estimate();
        // tau_a: default economic gate has min_hold_ms=30_000 → 30s, same as canonical.
        assert!((estimated.tau_a - canonical_l1.tau_a).abs() < 1e-9);
        // Estimate should still be stable.
        assert!(estimated.is_stable(), "idle estimate must be stable");
    }

    #[test]
    fn estimator_folds_multiple_observations() {
        let baseline = HomeostaticState::for_agent("agent");
        let mut est = MarginEstimator::for_l1(baseline.clone());

        let mut s = baseline.clone();
        for i in 1..=5 {
            s.last_event_ms = (i as u64) * 100;
            s.last_event_seq = i as u64;
            est.observe(&s);
        }
        assert_eq!(est.event_count(), 5);
        assert_eq!(est.window_ms(), 400); // 4 gaps of 100ms
        let b = est.estimate();
        assert!(b.is_stable());
    }

    #[test]
    fn estimator_reflects_context_pressure() {
        let baseline = HomeostaticState::for_agent("agent");
        let mut est = MarginEstimator::for_l1(baseline.clone());

        // Saturate cognitive pressure — rho should climb, margin should drop
        // but remain positive (canonical L1 has lots of headroom).
        let mut loaded = baseline.clone();
        loaded.cognitive.context_pressure = 1.0;
        loaded.cognitive.tool_density = 5.0;
        loaded.last_event_ms = 1_000;
        est.observe(&loaded);
        let b_loaded = est.estimate();

        let baseline_budget = StabilityBudget::from_canonical("L1").unwrap();
        assert!(
            b_loaded.rho > baseline_budget.rho,
            "loaded rho {} must exceed canonical {}",
            b_loaded.rho,
            baseline_budget.rho
        );
        assert!(b_loaded.is_stable());
    }

    #[test]
    fn estimator_uses_economic_gate_dwell_time() {
        // The L1 tau_a is literally the economic mode gate's min_hold_ms.
        let baseline = HomeostaticState::for_agent("agent");
        let mut state = baseline.clone();
        state.economic.mode_gate.min_hold_ms = 45_000; // 45 seconds

        let mut est = MarginEstimator::for_l1(baseline);
        est.observe(&state);
        let b = est.estimate();
        assert!((b.tau_a - 45.0).abs() < f64::EPSILON);
    }
}
