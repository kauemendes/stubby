//! Process-level configuration read at startup.
//!
//! Misconfiguration (missing or blank `STUBBY_IMAGE_*`) surfaces as an
//! `anyhow::Error` so `main` can bail before the webhook accepts traffic.
use anyhow::Context;

/// Fully-qualified dummy image references plumbed through the chart's
/// `dummyImages.*` values via `STUBBY_IMAGE_BACKEND` / `STUBBY_IMAGE_FRONTEND`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRefs {
    pub backend: String,
    pub frontend: String,
}

impl ImageRefs {
    /// Reads `STUBBY_IMAGE_BACKEND` and `STUBBY_IMAGE_FRONTEND` from the environment.
    /// Both are required and must be non-empty (whitespace is trimmed).
    /// Errors are surfaced to the binary's `main` so misconfigured deployments
    /// fail fast at startup instead of injecting a useless placeholder.
    pub fn from_env() -> anyhow::Result<Self> {
        let backend = required_env("STUBBY_IMAGE_BACKEND")?;
        let frontend = required_env("STUBBY_IMAGE_FRONTEND")?;
        Ok(Self { backend, frontend })
    }
}

fn required_env(key: &str) -> anyhow::Result<String> {
    let raw = std::env::var(key)
        .with_context(|| format!("{key} must be set to a fully qualified image ref"))?;
    let trimmed = raw.trim();
    anyhow::ensure!(!trimmed.is_empty(), "{key} must not be empty");
    Ok(trimmed.to_string())
}
