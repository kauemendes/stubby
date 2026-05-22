use anyhow::Result;
use std::time::Duration;
use stubby_dummy_backend::{routes::router, BackendConfig};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};

const SHUTDOWN_GRACE: Duration = Duration::from_secs(15);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let addr: std::net::SocketAddr = std::env::var("STUBBY_BACKEND_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()?;
    let cfg = BackendConfig::from_env();
    let app = router(cfg);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "stubby-dummy-backend listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let mut term = match signal(SignalKind::terminate()) {
        Ok(s) => s,
        Err(e) => {
            warn!(err=%e, "failed to install SIGTERM handler; shutdown won't be graceful");
            return;
        }
    };
    let mut int = match signal(SignalKind::interrupt()) {
        Ok(s) => s,
        Err(e) => {
            warn!(err=%e, "failed to install SIGINT handler");
            return;
        }
    };
    tokio::select! {
        _ = term.recv() => info!("SIGTERM received; draining for {SHUTDOWN_GRACE:?}"),
        _ = int.recv() => info!("SIGINT received; draining for {SHUTDOWN_GRACE:?}"),
    }
    tokio::time::sleep(SHUTDOWN_GRACE).await;
}
