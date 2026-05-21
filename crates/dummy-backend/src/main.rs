use anyhow::Result;
use stubby_dummy_backend::{routes::router, BackendConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().json().init();
    let addr: std::net::SocketAddr = std::env::var("STUBBY_BACKEND_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()?;
    let cfg = BackendConfig::from_env();
    let app = router(cfg);
    tracing::info!(%addr, "stubby-dummy-backend listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
