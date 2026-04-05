//! Trust scoring computation — maps homeostatic state to a composite trust score.
//!
//! Pure function: no I/O. Given a `HomeostaticState`, produces a `TrustScore`
//! with normalized pillar scores and derived tier/trajectory.

use chrono::Utc;

use autonomic_core::gating::HomeostaticState;
use autonomic_core::trust::{
    CognitiveComponent, CognitiveFactors, EconomicComponent, EconomicFactors, OperationalComponent,
    OperationalFactors, TierThresholds, TrustComponents, TrustScore, TrustTier, TrustTrajectory,
};

/// Weight for the operational pillar in composite scoring.
const OPERATIONAL_WEIGHT: f64 = 0.35;
/// Weight for the cognitive pillar in composite scoring.
const COGNITIVE_WEIGHT: f64 = 0.30;
/// Weight for the economic pillar in composite scoring.
const ECONOMIC_WEIGHT: f64 = 0.35;

/// Compute a composite trust score from a homeostatic state projection.
///
/// Maps the three-pillar state (operational, cognitive, economic) to
/// normalized 0-1 scores and derives tier and trajectory.
pub fn compute_trust_score(state: &HomeostaticState) -> TrustScore {
    let operational = compute_operational(state);
    let cognitive = compute_cognitive(state);
    let economic = compute_economic(state);

    let composite = operational.score * OPERATIONAL_WEIGHT
        + cognitive.score * COGNITIVE_WEIGHT
        + economic.score * ECONOMIC_WEIGHT;

    let tier = TrustTier::from_score(composite);
    let trajectory = compute_trajectory(state);

    TrustScore {
        agent_id: state.agent_id.clone(),
        score: round_to_2dp(composite),
        tier,
        components: TrustComponents {
            operational,
            cognitive,
            economic,
        },
        tier_thresholds: TierThresholds::default(),
        trajectory,
        assessed_at: Utc::now(),
    }
}

/// Compute the operational pillar score.
///
/// Factors:
/// - `uptime_ratio`: successes / (successes + errors), 1.0 if no events
/// - `error_rate`: errors / (successes + errors), 0.0 if no events
/// - `avg_latency_ms`: last tick timestamp (proxy for recency)
///
/// Score = `uptime_ratio` * 0.6 + (1.0 - `error_rate_penalty`) * 0.4
fn compute_operational(state: &HomeostaticState) -> OperationalComponent {
    let total_events = state.operational.total_successes + state.operational.total_errors;

    let (uptime_ratio, error_rate) = if total_events > 0 {
        let total = f64::from(total_events);
        let successes = f64::from(state.operational.total_successes);
        let errors = f64::from(state.operational.total_errors);
        (successes / total, errors / total)
    } else {
        // No events yet: optimistic default
        (1.0, 0.0)
    };

    // Error streak penalty: consecutive errors reduce score further.
    // Each consecutive error subtracts 0.05 from the base, capped at 0.5.
    let streak_penalty = (f64::from(state.operational.error_streak) * 0.05).min(0.5);

    let score = (uptime_ratio * 0.6 + (1.0 - error_rate) * 0.4 - streak_penalty).clamp(0.0, 1.0);

    OperationalComponent {
        score: round_to_2dp(score),
        factors: OperationalFactors {
            uptime_ratio: round_to_2dp(uptime_ratio),
            error_rate: round_to_2dp(error_rate),
            avg_latency_ms: state.operational.last_tick_ms,
        },
    }
}

/// Compute the cognitive pillar score.
///
/// Factors:
/// - `task_completion_rate`: `turns_completed` / (`turns_completed` + `error_streak`), capped at 1.0
/// - `context_utilization`: 1.0 - `context_pressure` (lower pressure = better utilization)
///
/// Score = `task_completion_rate` * 0.5 + `context_efficiency` * 0.5
fn compute_cognitive(state: &HomeostaticState) -> CognitiveComponent {
    let task_completion_rate = if state.cognitive.turns_completed > 0 {
        let completed = f64::from(state.cognitive.turns_completed);
        let failed = f64::from(state.operational.error_streak);
        (completed / (completed + failed)).min(1.0)
    } else {
        // No turns completed: neutral starting point
        0.5
    };

    // Context utilization: how efficiently the agent uses its context window.
    // Low pressure = good utilization efficiency.
    let context_utilization = 1.0 - f64::from(state.cognitive.context_pressure);

    let score = (task_completion_rate * 0.5 + context_utilization * 0.5).clamp(0.0, 1.0);

    CognitiveComponent {
        score: round_to_2dp(score),
        factors: CognitiveFactors {
            task_completion_rate: round_to_2dp(task_completion_rate),
            context_utilization: round_to_2dp(context_utilization),
        },
    }
}

/// Compute the economic pillar score.
///
/// Factors:
/// - `payment_history_score`: based on economic mode (Sovereign=1.0, Conserving=0.75, Hustle=0.5, Hibernate=0.1)
/// - `credit_utilization`: `lifetime_costs` / (`lifetime_costs` + balance), lower is better
///
/// Score = `payment_history_score` * 0.5 + (1.0 - `credit_utilization`) * 0.5
fn compute_economic(state: &HomeostaticState) -> EconomicComponent {
    use autonomic_core::EconomicMode;

    let payment_history_score = match state.economic.mode {
        EconomicMode::Sovereign => 1.0,
        EconomicMode::Conserving => 0.75,
        EconomicMode::Hustle => 0.50,
        EconomicMode::Hibernate => 0.10,
    };

    // Credit utilization: how much of available credit has been consumed.
    // Lower utilization = healthier finances.
    let total_capacity =
        state.economic.lifetime_costs + state.economic.balance_micro_credits.max(0);
    let credit_utilization = if total_capacity > 0 {
        state.economic.lifetime_costs as f64 / total_capacity as f64
    } else {
        0.0
    };

    let score = (payment_history_score * 0.5 + (1.0 - credit_utilization) * 0.5).clamp(0.0, 1.0);

    let economic_mode = format!("{:?}", state.economic.mode).to_lowercase();

    EconomicComponent {
        score: round_to_2dp(score),
        factors: EconomicFactors {
            payment_history_score: round_to_2dp(payment_history_score),
            credit_utilization: round_to_2dp(credit_utilization),
            economic_mode,
        },
    }
}

/// Compute trajectory from eval quality trend and error streak.
///
/// Uses the eval quality trend as a signal when available,
/// falls back to error streak analysis.
fn compute_trajectory(state: &HomeostaticState) -> TrustTrajectory {
    // Primary signal: eval quality trend (if evaluations have occurred)
    if state.eval.inline_eval_count + state.eval.async_eval_count > 0 {
        if state.eval.quality_trend > 0.01 {
            return TrustTrajectory::Improving;
        } else if state.eval.quality_trend < -0.01 {
            return TrustTrajectory::Degrading;
        }
    }

    // Fallback: error streak analysis
    if state.operational.error_streak >= 3 {
        return TrustTrajectory::Degrading;
    }

    // If agent has been active and mostly successful, it's improving
    let total = state.operational.total_successes + state.operational.total_errors;
    if total > 5 && state.operational.total_errors == 0 {
        return TrustTrajectory::Improving;
    }

    TrustTrajectory::Stable
}

/// Round to 2 decimal places for clean JSON output.
fn round_to_2dp(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use autonomic_core::EconomicMode;
    use autonomic_core::gating::HomeostaticState;

    fn default_state() -> HomeostaticState {
        HomeostaticState::for_agent("test-agent")
    }

    #[test]
    fn default_state_produces_high_trust_score() {
        let state = default_state();
        let score = compute_trust_score(&state);
        assert_eq!(score.agent_id, "test-agent");
        // Default state has no errors, full balance, no pressure
        // Should be Trusted or Certified
        assert!(
            score.score >= 0.75,
            "expected high score, got {}",
            score.score
        );
        assert!(
            score.tier == TrustTier::Trusted || score.tier == TrustTier::Certified,
            "expected Trusted or Certified, got {:?}",
            score.tier
        );
    }

    #[test]
    fn high_error_rate_reduces_operational_score() {
        let mut state = default_state();
        state.operational.total_errors = 8;
        state.operational.total_successes = 2;
        state.operational.error_streak = 3;

        let score = compute_trust_score(&state);
        assert!(
            score.components.operational.score < 0.5,
            "expected low operational score, got {}",
            score.components.operational.score
        );
    }

    #[test]
    fn hibernate_mode_reduces_economic_score() {
        let mut state = default_state();
        state.economic.mode = EconomicMode::Hibernate;
        state.economic.balance_micro_credits = 0;
        state.economic.lifetime_costs = 10_000_000; // Agent has burned through its budget

        let score = compute_trust_score(&state);
        assert!(
            score.components.economic.score < 0.5,
            "expected low economic score, got {}",
            score.components.economic.score
        );
    }

    #[test]
    fn high_context_pressure_reduces_cognitive_score() {
        let mut state = default_state();
        state.cognitive.context_pressure = 0.95;
        state.cognitive.turns_completed = 10;

        let score = compute_trust_score(&state);
        assert!(
            score.components.cognitive.factors.context_utilization < 0.1,
            "expected low context_utilization, got {}",
            score.components.cognitive.factors.context_utilization
        );
    }

    #[test]
    fn tier_derivation_matches_score() {
        let mut state = default_state();

        // Certified: high performance
        state.operational.total_successes = 100;
        state.operational.total_errors = 0;
        state.cognitive.turns_completed = 50;
        state.cognitive.context_pressure = 0.1;
        let score = compute_trust_score(&state);
        assert_eq!(score.tier, TrustTier::from_score(score.score));
    }

    #[test]
    fn trajectory_degrading_with_error_streak() {
        let mut state = default_state();
        state.operational.error_streak = 5;
        state.operational.total_errors = 5;

        let score = compute_trust_score(&state);
        assert_eq!(score.trajectory, TrustTrajectory::Degrading);
    }

    #[test]
    fn trajectory_improving_with_eval_trend() {
        let mut state = default_state();
        state.eval.inline_eval_count = 5;
        state.eval.quality_trend = 0.05;

        let score = compute_trust_score(&state);
        assert_eq!(score.trajectory, TrustTrajectory::Improving);
    }

    #[test]
    fn trajectory_degrading_with_eval_trend() {
        let mut state = default_state();
        state.eval.inline_eval_count = 5;
        state.eval.quality_trend = -0.05;

        let score = compute_trust_score(&state);
        assert_eq!(score.trajectory, TrustTrajectory::Degrading);
    }

    #[test]
    fn trajectory_stable_default() {
        let state = default_state();
        let score = compute_trust_score(&state);
        assert_eq!(score.trajectory, TrustTrajectory::Stable);
    }

    #[test]
    fn composite_weight_sum_is_one() {
        assert!(
            (OPERATIONAL_WEIGHT + COGNITIVE_WEIGHT + ECONOMIC_WEIGHT - 1.0).abs() < f64::EPSILON
        );
    }

    #[test]
    fn score_is_always_between_0_and_1() {
        // Worst case scenario
        let mut state = default_state();
        state.operational.total_errors = 100;
        state.operational.total_successes = 0;
        state.operational.error_streak = 10;
        state.economic.mode = EconomicMode::Hibernate;
        state.economic.balance_micro_credits = 0;
        state.economic.lifetime_costs = 10_000_000;
        state.cognitive.context_pressure = 1.0;
        state.cognitive.turns_completed = 0;

        let score = compute_trust_score(&state);
        assert!(score.score >= 0.0 && score.score <= 1.0);
        assert!(score.components.operational.score >= 0.0);
        assert!(score.components.cognitive.score >= 0.0);
        assert!(score.components.economic.score >= 0.0);
    }

    #[test]
    fn economic_mode_string_format() {
        let mut state = default_state();
        state.economic.mode = EconomicMode::Sovereign;
        let score = compute_trust_score(&state);
        assert_eq!(score.components.economic.factors.economic_mode, "sovereign");

        state.economic.mode = EconomicMode::Conserving;
        let score = compute_trust_score(&state);
        assert_eq!(
            score.components.economic.factors.economic_mode,
            "conserving"
        );
    }
}
