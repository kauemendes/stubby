use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use axum_server::Handle;
use stubby_webhook::config::ImageRefs;
use stubby_webhook::server::{router, AppState};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};

/// How often we reload the TLS material from disk. The chart's self-signed
/// Job (or cert-manager) rewrites the Secret on rotation; without this loop
/// the pod would keep serving a stale cert until it restarts.
const TLS_RELOAD_INTERVAL: Duration = Duration::from_secs(60);

/// How long the server waits for in-flight requests when SIGTERM arrives.
/// `helm uninstall` and rolling updates send SIGTERM; we don't want to drop
/// admission reviews mid-flight.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let raw_listen = std::env::var("STUBBY_LISTEN").unwrap_or_else(|_| "0.0.0.0:8443".into());
    let addr: SocketAddr = raw_listen.parse().with_context(|| {
        format!("STUBBY_LISTEN={raw_listen:?} is not a valid <host>:<port> SocketAddr")
    })?;

    let cert_path = std::env::var("STUBBY_TLS_CERT").unwrap_or_else(|_| "/tls/tls.crt".into());
    let key_path = std::env::var("STUBBY_TLS_KEY").unwrap_or_else(|_| "/tls/tls.key".into());

    let image_refs = ImageRefs::from_env().context("loading STUBBY_IMAGE_* env vars")?;
    let state = AppState {
        image_refs: Arc::new(image_refs),
    };
    let app = router(state);

    // Install the rustls process-wide crypto provider exactly once. `.ok()`
    // because in a fresh main() it cannot have been pre-installed.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();

    let tls = RustlsConfig::from_pem_file(&cert_path, &key_path)
        .await
        .with_context(|| format!("loading TLS cert/key from {cert_path} + {key_path}"))?;

    spawn_cert_reloader(tls.clone(), cert_path.clone(), key_path.clone());

    let handle = Handle::new();
    spawn_shutdown_listener(handle.clone());

    info!(%addr, "stubby-webhook listening (TLS)");
    axum_server::bind_rustls(addr, tls)
        .handle(handle)
        .serve(app.into_make_service())
        .await
        .context("axum-server bind_rustls failed")?;

    Ok(())
}

fn spawn_cert_reloader(tls: RustlsConfig, cert_path: String, key_path: String) {
    tokio::spawn(async move {
        let mut ticker = tokio::time::interval(TLS_RELOAD_INTERVAL);
        ticker.tick().await; // skip the immediate first tick
        loop {
            ticker.tick().await;
            match tls.reload_from_pem_file(&cert_path, &key_path).await {
                Ok(()) => info!("reloaded TLS material from disk"),
                Err(e) => warn!(err=%e, "TLS reload failed; keeping previous material"),
            }
        }
    });
}

fn spawn_shutdown_listener(handle: Handle) {
    tokio::spawn(async move {
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
        handle.graceful_shutdown(Some(SHUTDOWN_GRACE));
    });
}
