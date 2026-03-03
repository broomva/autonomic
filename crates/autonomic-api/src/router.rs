//! HTTP router for the Autonomic API.
//!
//! Endpoints:
//! - `GET /health` — health check
//! - `GET /gating/{session_id}` — evaluate rules and return gating profile
//! - `GET /projection/{session_id}` — return raw homeostatic state

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Json;
use axum::{Router, routing::get};
use serde::Serialize;

use autonomic_controller::evaluate;
use autonomic_core::gating::{AutonomicGatingProfile, HomeostaticState};

use crate::state::AppState;

/// Build the axum router with all endpoints.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/gating/{session_id}", get(get_gating))
        .route("/projection/{session_id}", get(get_projection))
        .with_state(state)
}

/// Health check response.
#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
}

async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}

/// Gating response with staleness indicator.
#[derive(Serialize)]
pub struct GatingResponse {
    pub session_id: String,
    pub profile: AutonomicGatingProfile,
    pub last_event_seq: u64,
    pub last_event_ms: u64,
}

async fn get_gating(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
) -> Result<Json<GatingResponse>, StatusCode> {
    let projections = state.projections.read().await;

    let homeostatic_state = projections
        .get(&session_id)
        .cloned()
        .unwrap_or_else(|| HomeostaticState::for_agent(&session_id));

    let profile = evaluate(&homeostatic_state, &state.rules);

    Ok(Json(GatingResponse {
        session_id,
        profile,
        last_event_seq: homeostatic_state.last_event_seq,
        last_event_ms: homeostatic_state.last_event_ms,
    }))
}

/// Projection response.
#[derive(Serialize)]
pub struct ProjectionResponse {
    pub session_id: String,
    pub state: HomeostaticState,
    pub found: bool,
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use autonomic_core::rules::RuleSet;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn test_state() -> AppState {
        AppState::new(RuleSet::new())
    }

    async fn body_json(resp: axum::http::Response<Body>) -> serde_json::Value {
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn health_endpoint() {
        let app = build_router(test_state());
        let req = Request::builder()
            .uri("/health")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["status"], "ok");
    }

    #[tokio::test]
    async fn gating_endpoint_default_session() {
        let app = build_router(test_state());
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

    #[tokio::test]
    async fn projection_endpoint_not_found() {
        let app = build_router(test_state());
        let req = Request::builder()
            .uri("/projection/nonexistent")
            .body(Body::empty())
            .unwrap();

        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        let json = body_json(resp).await;
        assert_eq!(json["found"], false);
        assert_eq!(json["session_id"], "nonexistent");
    }

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

        let app = build_router(state);
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
}
