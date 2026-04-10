//! HTTP router for the Autonomic API.
//!
//! Endpoints:
//! - `GET /health` — health check (unprotected)
//! - `GET /trust-score/{agent_id}` — public trust score (unprotected)
//! - `GET /gating/{session_id}` — evaluate rules and return gating profile (auth-protected)
//! - `GET /projection/{session_id}` — return raw homeostatic state (auth-protected)

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::{Router, middleware, routing::get};
use serde::Serialize;
use serde_json::json;
use tracing::instrument;

use autonomic_controller::{compute_trust_score, evaluate};
use autonomic_core::AutonomicEvent;
use autonomic_core::gating::{AutonomicGatingProfile, HomeostaticState};
use autonomic_core::trust::TrustScore;

use crate::auth::{AuthConfig, auth_middleware};
use crate::state::AppState;

/// Build the axum router with all endpoints.
///
/// Health and trust-score endpoints are always unprotected (public).
/// Gating and projection endpoints are protected with JWT auth when a secret is configured.
pub fn build_router(state: AppState) -> Router {
    build_router_with_auth(state, AuthConfig::from_env())
}

/// Build the router with an explicit auth config (used in tests).
pub fn build_router_with_auth(state: AppState, auth_config: AuthConfig) -> Router {
    // Protected routes: gating + projection
    let protected = Router::new()
        .route("/gating/{session_id}", get(get_gating))
        .route("/projection/{session_id}", get(get_projection))
        .route_layer(middleware::from_fn_with_state(auth_config, auth_middleware))
        .with_state(state.clone());

    // Public routes: health + trust-score
    let public = Router::new()
        .route("/health", get(health))
        .route("/trust-score/{agent_id}", get(get_trust_score))
        .with_state(state);

    public.merge(protected)
}

async fn health(State(state): State<AppState>) -> Json<serde_json::Value> {
    let uptime_seconds = state.started_at.elapsed().as_secs();
    let otlp_configured = std::env::var("OTEL_EXPORTER_OTLP_ENDPOINT").is_ok();
    Json(json!({
        "status": "ok",
        "service": "autonomic",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_seconds": uptime_seconds,
        "telemetry": {
            "sdk": "life-vigil",
            "otlp_configured": otlp_configured,
        },
    }))
}

/// Gating response with staleness indicator.
#[derive(Serialize)]
pub struct GatingResponse {
    pub session_id: String,
    pub profile: AutonomicGatingProfile,
    pub last_event_seq: u64,
    pub last_event_ms: u64,
}

#[instrument(skip(state), fields(life.session_id = %session_id))]
async fn get_gating(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<GatingResponse>, StatusCode> {
    let homeostatic_state = get_or_bootstrap(&state, &session_id).await;

    let profile = evaluate(&homeostatic_state, &state.rules);
    let published_watermark =
        publish_advisory_events(&state, &session_id, profile.advisory_events.clone()).await;
    let (last_event_seq, last_event_ms) = published_watermark.unwrap_or((
        homeostatic_state.last_event_seq,
        homeostatic_state.last_event_ms,
    ));

    Ok(Json(GatingResponse {
        session_id,
        profile,
        last_event_seq,
        last_event_ms,
    }))
}

/// Projection response.
#[derive(Serialize)]
pub struct ProjectionResponse {
    pub session_id: String,
    pub state: HomeostaticState,
    pub found: bool,
}

#[instrument(skip(state), fields(life.session_id = %session_id))]
async fn get_projection(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Json<ProjectionResponse> {
    let projections = state.projections.read().await;

    let (homeostatic_state, found) = match projections.get(&session_id) {
        Some(s) => (s.clone(), true),
        None => (HomeostaticState::for_agent(&session_id), false),
    };

    Json(ProjectionResponse {
        session_id,
        state: homeostatic_state,
        found,
    })
}

/// Trust score response — public, no auth required.
#[instrument(skip(state), fields(life.agent_id = %agent_id))]
async fn get_trust_score(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> Json<TrustScore> {
    let projections = state.projections.read().await;

    let homeostatic_state = match projections.get(&agent_id) {
        Some(s) => s.clone(),
        None => HomeostaticState::for_agent(&agent_id),
    };

    let trust_score = compute_trust_score(&homeostatic_state);
    Json(trust_score)
}

/// Get projection from cache, or bootstrap from Lago journal if available.
#[instrument(skip(state), fields(life.session_id = %session_id))]
async fn get_or_bootstrap(state: &AppState, session_id: &str) -> HomeostaticState {
    // Fast path: already in projection map
    {
        let projections = state.projections.read().await;
        if let Some(s) = projections.get(session_id) {
            return s.clone();
        }
    }

    // Slow path: bootstrap from Lago journal if available
    if let Some(journal) = &state.journal {
        match autonomic_lago::load_projection(journal.clone(), session_id, "main").await {
            Ok(loaded) => {
                let mut projections = state.projections.write().await;
                projections.insert(session_id.to_owned(), loaded.clone());

                // Spawn live subscriber for ongoing updates (tracked for graceful shutdown)
                let j = journal.clone();
                let sid = session_id.to_owned();
                let p = state.projections.clone();
                state.task_tracker.spawn(async move {
                    autonomic_lago::subscribe_session(j, sid, "main".into(), p).await;
                });

                return loaded;
            }
            Err(e) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %e,
                    "failed to bootstrap projection from Lago"
                );
            }
        }
    }

    HomeostaticState::for_agent(session_id)
}

async fn publish_advisory_events(
    state: &AppState,
    session_id: &str,
    events: Vec<AutonomicEvent>,
) -> Option<(u64, u64)> {
    if events.is_empty() {
        return None;
    }

    let Some(journal) = &state.journal else {
        return None;
    };

    let mut watermark = None;
    for event in events {
        let payload = event.clone().into_event_kind();
        match autonomic_lago::publish_event(journal.clone(), session_id, "main", event).await {
            Ok(seq) => {
                let ts_ms = lago_core::event::EventEnvelope::now_micros() / 1_000;
                let mut projections = state.projections.write().await;
                let projected = projections
                    .entry(session_id.to_owned())
                    .or_insert_with(|| HomeostaticState::for_agent(session_id));
                *projected = autonomic_controller::fold(projected.clone(), &payload, seq, ts_ms);
                watermark = Some((seq, ts_ms));
            }
            Err(error) => {
                tracing::warn!(
                    session_id = %session_id,
                    error = %error,
                    "failed to publish Autonomic advisory event"
                );
            }
        }
    }

    watermark
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};

    use autonomic_core::rules::{GatingDecision, HomeostaticRule, RuleSet};
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use lago_auth::jwt::BroomvaClaims;
    use lago_core::error::{LagoError, LagoResult};
    use lago_core::event::EventEnvelope;
    use lago_core::id::{BranchId, EventId, SeqNo, SessionId};
    use lago_core::journal::{EventQuery, EventStream, Journal};
    use lago_core::session::Session;
    use tower::ServiceExt;

    const TEST_SECRET: &str = "autonomic-test-secret-32bytes!!";

    fn test_state() -> AppState {
        AppState::new(RuleSet::new())
    }

    fn app_no_auth() -> Router {
        build_router_with_auth(test_state(), AuthConfig::disabled())
    }

    fn app_with_auth() -> Router {
        build_router_with_auth(test_state(), AuthConfig::with_secret(TEST_SECRET))
    }

    fn make_token(secret: &str) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let claims = BroomvaClaims {
            sub: "agent-1".to_string(),
            email: "agent@broomva.tech".to_string(),
            exp: now + 3600,
            iat: now,
        };
        let key = jsonwebtoken::EncodingKey::from_secret(secret.as_bytes());
        jsonwebtoken::encode(&jsonwebtoken::Header::default(), &claims, &key).unwrap()
    }

    async fn body_json(resp: axum::http::Response<Body>) -> serde_json::Value {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[derive(Clone)]
    enum AppendBehavior {
        Success(SeqNo),
        Failure(&'static str),
    }

    struct MockJournal {
        behavior: AppendBehavior,
        appended: Arc<Mutex<Vec<EventEnvelope>>>,
    }

    impl MockJournal {
        fn success(seq: SeqNo) -> Arc<Self> {
            Arc::new(Self {
                behavior: AppendBehavior::Success(seq),
                appended: Arc::new(Mutex::new(Vec::new())),
            })
        }

        fn failure(message: &'static str) -> Arc<Self> {
            Arc::new(Self {
                behavior: AppendBehavior::Failure(message),
                appended: Arc::new(Mutex::new(Vec::new())),
            })
        }
    }

    impl Journal for MockJournal {
        fn append(
            &self,
            event: EventEnvelope,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
            let behavior = self.behavior.clone();
            let appended = self.appended.clone();
            Box::pin(async move {
                match behavior {
                    AppendBehavior::Success(seq) => {
                        appended.lock().unwrap().push(event);
                        Ok(seq)
                    }
                    AppendBehavior::Failure(message) => Err(LagoError::Journal(message.into())),
                }
            })
        }

        fn append_batch(
            &self,
            events: Vec<EventEnvelope>,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
            let behavior = self.behavior.clone();
            let appended = self.appended.clone();
            Box::pin(async move {
                match behavior {
                    AppendBehavior::Success(seq) => {
                        appended.lock().unwrap().extend(events);
                        Ok(seq)
                    }
                    AppendBehavior::Failure(message) => Err(LagoError::Journal(message.into())),
                }
            })
        }

        fn read(
            &self,
            _query: EventQuery,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Vec<EventEnvelope>>> + Send + '_>>
        {
            let appended = self.appended.clone();
            Box::pin(async move { Ok(appended.lock().unwrap().clone()) })
        }

        fn get_event(
            &self,
            _event_id: &EventId,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Option<EventEnvelope>>> + Send + '_>>
        {
            Box::pin(async { Ok(None) })
        }

        fn head_seq(
            &self,
            _session_id: &SessionId,
            _branch_id: &BranchId,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<SeqNo>> + Send + '_>> {
            Box::pin(async { Ok(0) })
        }

        fn stream(
            &self,
            _session_id: SessionId,
            _branch_id: BranchId,
            _after_seq: SeqNo,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<EventStream>> + Send + '_>>
        {
            Box::pin(async { Err(LagoError::Journal("stream unsupported in mock".into())) })
        }

        fn put_session(
            &self,
            _session: Session,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<()>> + Send + '_>> {
            Box::pin(async { Ok(()) })
        }

        fn get_session(
            &self,
            _session_id: &SessionId,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Option<Session>>> + Send + '_>>
        {
            Box::pin(async { Ok(None) })
        }

        fn list_sessions(
            &self,
        ) -> Pin<Box<dyn std::future::Future<Output = LagoResult<Vec<Session>>> + Send + '_>>
        {
            Box::pin(async { Ok(Vec::new()) })
        }
    }

    struct AdvisoryRule;

    impl HomeostaticRule for AdvisoryRule {
        fn rule_id(&self) -> &str {
            "advisory"
        }

        fn evaluate(&self, _state: &HomeostaticState) -> Option<GatingDecision> {
            Some(GatingDecision {
                advisory_events: vec![AutonomicEvent::RollbackRequested {
                    artifact: "knowledge_thresholds".into(),
                    rollback_to: "v1".into(),
                    reason: "regression".into(),
                }],
                rationale: "emit advisory".into(),
                ..GatingDecision::noop(self.rule_id())
            })
        }
    }

    // --- Health endpoint (always unprotected) ---

    #[tokio::test]
    async fn health_without_token_returns_200() {
        let app = app_with_auth();
        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["status"], "ok");
    }

    // --- Gating endpoint: auth disabled (local dev) ---

    #[tokio::test]
    async fn gating_no_auth_no_token_returns_200() {
        let app = app_no_auth();
        let req = Request::builder()
            .uri("/gating/test-session")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["session_id"], "test-session");
        assert_eq!(json["profile"]["operational"]["allow_side_effects"], true);
    }

    // --- Gating endpoint: auth enabled ---

    #[tokio::test]
    async fn gating_auth_enabled_no_token_returns_401() {
        let app = app_with_auth();
        let req = Request::builder()
            .uri("/gating/test-session")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

        let json = body_json(resp).await;
        assert_eq!(json["error"], "unauthorized");
    }

    #[tokio::test]
    async fn gating_auth_enabled_invalid_token_returns_401() {
        let app = app_with_auth();
        let req = Request::builder()
            .uri("/gating/test-session")
            .header("authorization", "Bearer invalid-garbage-token")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn gating_auth_enabled_wrong_secret_returns_401() {
        let token = make_token("wrong-secret-not-the-right-one!!");
        let app = app_with_auth();
        let req = Request::builder()
            .uri("/gating/test-session")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn gating_auth_enabled_valid_token_returns_200() {
        let token = make_token(TEST_SECRET);
        let app = app_with_auth();
        let req = Request::builder()
            .uri("/gating/test-session")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["session_id"], "test-session");
    }

    #[tokio::test]
    async fn gating_returns_fresh_watermark_after_advisory_publish() {
        let journal = MockJournal::success(42);
        let mut rules = RuleSet::new();
        rules.add(Box::new(AdvisoryRule));
        let projections = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        {
            let mut state = HomeostaticState::for_agent("sess-rollback");
            state.cognitive.knowledge_promotion.active_version = Some("v2".into());
            state.cognitive.knowledge_promotion.rollback_target = Some("v1".into());
            projections
                .write()
                .await
                .insert("sess-rollback".into(), state);
        }
        let state = AppState::with_journal(projections.clone(), rules, journal.clone());
        let app = build_router_with_auth(state, AuthConfig::disabled());

        let req = Request::builder()
            .uri("/gating/sess-rollback")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["last_event_seq"], 42);
        assert!(json["last_event_ms"].as_u64().unwrap() > 0);
        assert_eq!(journal.appended.lock().unwrap().len(), 1);

        let projected = projections.read().await;
        assert!(
            projected
                .get("sess-rollback")
                .unwrap()
                .cognitive
                .knowledge_promotion
                .rollback_requested
        );
    }

    #[tokio::test]
    async fn publish_advisory_events_updates_projection_on_success() {
        let journal = MockJournal::success(7);
        let projections = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        {
            let mut state = HomeostaticState::for_agent("sess-1");
            state.cognitive.knowledge_promotion.active_version = Some("v2".into());
            state.cognitive.knowledge_promotion.rollback_target = Some("v1".into());
            projections.write().await.insert("sess-1".into(), state);
        }
        let state = AppState::with_journal(projections.clone(), RuleSet::new(), journal.clone());

        let watermark = publish_advisory_events(
            &state,
            "sess-1",
            vec![AutonomicEvent::RollbackRequested {
                artifact: "knowledge_thresholds".into(),
                rollback_to: "v1".into(),
                reason: "regression".into(),
            }],
        )
        .await
        .expect("publish should return watermark");

        assert_eq!(watermark.0, 7);
        assert!(watermark.1 > 0);
        assert_eq!(journal.appended.lock().unwrap().len(), 1);
        let projected = projections.read().await;
        let promotion = &projected
            .get("sess-1")
            .unwrap()
            .cognitive
            .knowledge_promotion;
        assert!(promotion.rollback_requested);
        assert_eq!(promotion.rollback_target.as_deref(), Some("v1"));
    }

    #[tokio::test]
    async fn publish_advisory_events_leaves_projection_unchanged_on_failure() {
        let journal = MockJournal::failure("disk unavailable");
        let projections = Arc::new(tokio::sync::RwLock::new(HashMap::new()));
        {
            let mut state = HomeostaticState::for_agent("sess-1");
            state.last_event_seq = 5;
            state.cognitive.knowledge_promotion.active_version = Some("v2".into());
            state.cognitive.knowledge_promotion.rollback_target = Some("v1".into());
            projections.write().await.insert("sess-1".into(), state);
        }
        let state = AppState::with_journal(projections.clone(), RuleSet::new(), journal.clone());

        let watermark = publish_advisory_events(
            &state,
            "sess-1",
            vec![AutonomicEvent::RollbackRequested {
                artifact: "knowledge_thresholds".into(),
                rollback_to: "v1".into(),
                reason: "regression".into(),
            }],
        )
        .await;

        assert!(watermark.is_none());
        assert!(journal.appended.lock().unwrap().is_empty());
        let projected = projections.read().await;
        let session = projected.get("sess-1").unwrap();
        assert_eq!(session.last_event_seq, 5);
        assert!(!session.cognitive.knowledge_promotion.rollback_requested);
    }

    #[tokio::test]
    async fn publish_advisory_events_noops_without_journal() {
        let state = test_state();
        let watermark = publish_advisory_events(
            &state,
            "sess-1",
            vec![AutonomicEvent::RollbackRequested {
                artifact: "knowledge_thresholds".into(),
                rollback_to: "v1".into(),
                reason: "regression".into(),
            }],
        )
        .await;

        assert!(watermark.is_none());
        assert!(state.projections.read().await.is_empty());
    }

    // --- Projection endpoint: auth enabled ---

    #[tokio::test]
    async fn projection_auth_enabled_no_token_returns_401() {
        let app = app_with_auth();
        let req = Request::builder()
            .uri("/projection/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn projection_auth_enabled_valid_token_returns_200() {
        let token = make_token(TEST_SECRET);
        let app = app_with_auth();
        let req = Request::builder()
            .uri("/projection/nonexistent")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["found"], false);
    }

    // --- Projection endpoint with data (auth disabled for backward-compat) ---

    #[tokio::test]
    async fn projection_endpoint_with_data() {
        let state = test_state();
        {
            let mut map = state.projections.write().await;
            let mut hs = HomeostaticState::for_agent("sess-1");
            hs.cognitive.total_tokens_used = 5000;
            hs.last_event_seq = 42;
            map.insert("sess-1".into(), hs);
        }

        let app = build_router_with_auth(state, AuthConfig::disabled());
        let req = Request::builder()
            .uri("/projection/sess-1")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["found"], true);
        assert_eq!(json["state"]["cognitive"]["total_tokens_used"], 5000);
        assert_eq!(json["state"]["last_event_seq"], 42);
    }

    // --- Trust score endpoint (always unprotected) ---

    #[tokio::test]
    async fn trust_score_without_token_returns_200() {
        let app = app_with_auth();
        let req = Request::builder()
            .uri("/trust-score/agent-123")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["agent_id"], "agent-123");
        assert!(json["score"].as_f64().is_some());
        assert!(json["tier"].as_str().is_some());
        assert!(json["trajectory"].as_str().is_some());
        assert!(json["assessed_at"].as_str().is_some());
        assert!(json["tier_thresholds"]["certified"].as_f64().is_some());
        assert!(
            json["components"]["operational"]["score"]
                .as_f64()
                .is_some()
        );
        assert!(json["components"]["cognitive"]["score"].as_f64().is_some());
        assert!(json["components"]["economic"]["score"].as_f64().is_some());
    }

    #[tokio::test]
    async fn trust_score_unknown_agent_returns_unverified_default() {
        let app = app_no_auth();
        let req = Request::builder()
            .uri("/trust-score/nonexistent-agent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["agent_id"], "nonexistent-agent");
        // Default state has no completed turns so cognitive starts at 0.5
        // which pulls the composite score down — but operational and economic are high
        assert!(json["score"].as_f64().unwrap() > 0.0);
    }

    #[tokio::test]
    async fn trust_score_with_projection_data() {
        let state = test_state();
        {
            let mut map = state.projections.write().await;
            let mut hs = HomeostaticState::for_agent("agent-scored");
            hs.operational.total_successes = 50;
            hs.operational.total_errors = 2;
            hs.cognitive.turns_completed = 30;
            hs.cognitive.context_pressure = 0.3;
            map.insert("agent-scored".into(), hs);
        }

        let app = build_router_with_auth(state, AuthConfig::disabled());
        let req = Request::builder()
            .uri("/trust-score/agent-scored")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["agent_id"], "agent-scored");
        let score = json["score"].as_f64().unwrap();
        assert!(
            score > 0.7,
            "expected high score for healthy agent, got {score}"
        );
        assert!(
            json["components"]["operational"]["factors"]["uptime_ratio"]
                .as_f64()
                .unwrap()
                > 0.9
        );
    }
}
