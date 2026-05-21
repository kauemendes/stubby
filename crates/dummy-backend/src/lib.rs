pub mod openapi;
pub mod routes;

#[derive(Clone, Debug)]
pub struct BackendConfig {
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
