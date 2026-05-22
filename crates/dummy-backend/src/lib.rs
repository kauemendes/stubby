//! `stubby-dummy-backend` — drop-in axum HTTP server used as the backend dummy.
//!
//! The webhook swaps an annotated pod's container image to this binary's
//! image so that Deployments and Services can come up green before the
//! real application is built. Exposes `/health`, `/ready`, `/openapi.json`,
//! and `/docs` plus a JSON catch-all on every other path.
pub mod openapi;
pub mod routes;

/// Per-process configuration sourced from environment variables.
#[derive(Clone, Debug)]
pub struct BackendConfig {
    /// Display name embedded in the dummy OpenAPI title (`<app> (dummy)`).
    /// Provided by the webhook via `STUBBY_APP_NAME`; falls back to `"stubby"`.
    pub app_name: String,
}

impl BackendConfig {
    /// Reads `STUBBY_APP_NAME` from the environment. The webhook always injects
    /// this on pods it mutates, so a missing value typically means the binary
    /// was started outside of stubby's wiring — log a warning to make that
    /// visible without making the binary unbootable.
    pub fn from_env() -> Self {
        match std::env::var("STUBBY_APP_NAME") {
            Ok(name) if !name.trim().is_empty() => Self {
                app_name: name.trim().to_string(),
            },
            _ => {
                tracing::warn!(
                    "STUBBY_APP_NAME unset or blank; falling back to 'stubby' (the webhook normally injects this)"
                );
                Self {
                    app_name: "stubby".into(),
                }
            }
        }
    }
}
