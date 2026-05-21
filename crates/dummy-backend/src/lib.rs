pub mod openapi;
pub mod routes;

#[derive(Clone, Debug)]
pub struct BackendConfig {
    pub app_name: String,
}

impl BackendConfig {
    pub fn from_env() -> Self {
        Self {
            app_name: std::env::var("STUBBY_APP_NAME").unwrap_or_else(|_| "stubby".into()),
        }
    }
}
