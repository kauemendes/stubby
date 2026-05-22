//! Error types surfaced by the webhook's internal modules.
use thiserror::Error;

/// All recoverable failure modes the webhook can hit.
///
/// Decode failures (`InvalidBody`) are translated to an `AdmissionResponse`
/// with `allowed: true` and an explanatory `status.message` rather than a
/// non-200 reply — that keeps the admission contract intact.
#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("invalid AdmissionReview body: {0}")]
    InvalidBody(#[from] serde_json::Error),
    #[error("TLS setup error: {0}")]
    Tls(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
