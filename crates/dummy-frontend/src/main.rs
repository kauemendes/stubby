use anyhow::Result;
use std::time::Duration;
use stubby_dummy_frontend::{routes::router, FrontendConfig};
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

    let addr: std::net::SocketAddr = listen_addr().parse()?;
    let cfg = FrontendConfig::from_env();
    let app = router(cfg);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!(%addr, "stubby-dummy-frontend listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

/// Resolve the listen address.
///
/// Precedence: `STUBBY_FRONTEND_LISTEN` (full `host:port`, escape hatch) →
/// `STUBBY_PORT` (the webhook injects this to match `stubby.io/port`) →
/// `0.0.0.0:80` (the frontend default). The image runs non-root, so binding
/// a privileged port (<1024) needs the pod to run as root; the recommended
/// setup keeps the container on 8080 behind a Service `targetPort`.
fn listen_addr() -> String {
    if let Ok(full) = std::env::var("STUBBY_FRONTEND_LISTEN") {
        let full = full.trim();
        if !full.is_empty() {
            return full.to_string();
        }
    }
    match std::env::var("STUBBY_PORT") {
        Ok(p) if !p.trim().is_empty() => format!("0.0.0.0:{}", p.trim()),
        _ => "0.0.0.0:80".to_string(),
    }
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
