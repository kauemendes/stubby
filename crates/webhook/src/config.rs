#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRefs {
    pub backend: String,
    pub frontend: String,
}

impl ImageRefs {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            backend: std::env::var("STUBBY_IMAGE_BACKEND")
                .unwrap_or_else(|_| "ghcr.io/kauemendes/stubby-dummy-backend:latest".into()),
            frontend: std::env::var("STUBBY_IMAGE_FRONTEND")
                .unwrap_or_else(|_| "ghcr.io/kauemendes/stubby-dummy-frontend:latest".into()),
        })
    }
}
