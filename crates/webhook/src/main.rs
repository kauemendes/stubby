use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::{Context, Result};
use axum_server::tls_rustls::RustlsConfig;
use stubby_webhook::config::ImageRefs;
use stubby_webhook::server::{router, AppState};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,stubby_webhook=info")),
        )
        .init();

    // Bind address: e.g. STUBBY_LISTEN=0.0.0.0:8443 (default if unset).
    let addr: SocketAddr = std::env::var("STUBBY_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8443".into())
        .parse()
        .context("STUBBY_LISTEN must be a valid <host>:<port> SocketAddr")?;

    // TLS material paths — defaults match the chart's Secret mount.
    let cert_path = std::env::var("STUBBY_TLS_CERT").unwrap_or_else(|_| "/tls/tls.crt".into());
    let key_path = std::env::var("STUBBY_TLS_KEY").unwrap_or_else(|_| "/tls/tls.key".into());

    let image_refs = ImageRefs::from_env().context("loading STUBBY_IMAGE_* env vars")?;
    let state = AppState {
        image_refs: Arc::new(image_refs),
    };
    let app = router(state);

    // Install the rustls process-wide crypto provider exactly once.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok(); // OK if already installed (e.g. in tests).

    let tls = RustlsConfig::from_pem_file(&cert_path, &key_path)
        .await
        .with_context(|| format!("loading TLS cert/key from {cert_path} + {key_path}"))?;

    info!(%addr, "stubby-webhook listening (TLS)");
    axum_server::bind_rustls(addr, tls)
        .serve(app.into_make_service())
        .await
        .context("axum-server bind_rustls failed")?;

    Ok(())
}
