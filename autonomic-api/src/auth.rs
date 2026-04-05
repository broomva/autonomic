//! JWT authentication middleware for the Autonomic HTTP API.
//!
//! Uses `lago-auth` JWT validation primitives to protect sensitive endpoints.
//! Auth is optional: if no JWT secret is configured, requests pass through
//! unauthenticated (local dev mode). If a secret IS configured, a valid
//! `Authorization: Bearer <token>` header is required.

use std::sync::Arc;

use axum::extract::Request;
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use lago_auth::jwt::{extract_bearer_token, validate_jwt};

/// Auth configuration for the Autonomic API.
#[derive(Clone)]
pub struct AuthConfig {
    /// JWT secret for token validation. `None` means auth is disabled.
    inner: Option<Arc<String>>,
}

impl AuthConfig {
    /// Create an auth config from the environment.
    ///
    /// Reads `AUTONOMIC_JWT_SECRET` first, then falls back to `AUTH_SECRET`.
    /// If neither is set, auth is disabled (local dev mode) and a warning is logged.
    pub fn from_env() -> Self {
        let secret = std::env::var("AUTONOMIC_JWT_SECRET")
            .or_else(|_| std::env::var("AUTH_SECRET"))
            .ok();

        match &secret {
            Some(_) => {
                tracing::info!("JWT auth enabled for protected endpoints");
            }
            None => {
                tracing::warn!(
                    "No AUTONOMIC_JWT_SECRET or AUTH_SECRET configured — \
                     protected endpoints are UNPROTECTED. \
                     Set one of these env vars in production."
                );
            }
        }

        Self {
            inner: secret.map(Arc::new),
        }
    }

    /// Create an auth config with a specific secret (for testing).
    pub fn with_secret(secret: impl Into<String>) -> Self {
        Self {
            inner: Some(Arc::new(secret.into())),
        }
    }

    /// Create an auth config with auth disabled (for testing).
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Whether auth is enabled.
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }
}

/// Auth error response body.
#[derive(Serialize)]
struct AuthErrorBody {
    error: String,
    message: String,
}

fn auth_error(status: StatusCode, message: impl Into<String>) -> Response {
    let body = AuthErrorBody {
        error: "unauthorized".to_string(),
        message: message.into(),
    };
    (status, axum::Json(body)).into_response()
}

/// Axum middleware that validates JWT bearer tokens.
///
/// If no secret is configured (auth disabled), requests pass through.
/// If a secret IS configured, the `Authorization: Bearer <token>` header
/// must contain a valid JWT signed with that secret.
pub async fn auth_middleware(
    axum::extract::State(config): axum::extract::State<AuthConfig>,
    request: Request,
    next: Next,
) -> Response {
    // If auth is disabled, pass through
    let Some(secret) = &config.inner else {
        return next.run(request).await;
    };

    // Extract Authorization header
    let auth_header = match request.headers().get("authorization") {
        Some(h) => match h.to_str() {
            Ok(s) => s.to_string(),
            Err(_) => return auth_error(StatusCode::UNAUTHORIZED, "invalid authorization header"),
        },
        None => return auth_error(StatusCode::UNAUTHORIZED, "missing authorization header"),
    };

    // Extract bearer token
    let token = match extract_bearer_token(&auth_header) {
        Ok(t) => t,
        Err(e) => return auth_error(StatusCode::UNAUTHORIZED, e.to_string()),
    };

    // Validate JWT
    match validate_jwt(token, secret) {
        Ok(_claims) => {
            // Token is valid — proceed. We don't inject user context
            // since Autonomic doesn't need per-user session mapping.
            next.run(request).await
        }
        Err(e) => auth_error(StatusCode::UNAUTHORIZED, e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_config_disabled() {
        let config = AuthConfig::disabled();
        assert!(!config.is_enabled());
    }

    #[test]
    fn auth_config_with_secret() {
        let config = AuthConfig::with_secret("test-secret");
        assert!(config.is_enabled());
    }
}
