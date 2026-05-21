use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("invalid AdmissionReview body: {0}")]
    InvalidBody(#[from] serde_json::Error),
    #[error("TLS setup error: {0}")]
    Tls(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
