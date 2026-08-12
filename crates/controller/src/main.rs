use anyhow::Result;
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::Api;
use kube::runtime::controller::Controller;
use kube::runtime::watcher;
use kube::Client;
use std::sync::Arc;
use stubby_controller::config::ControllerConfig;
use stubby_controller::observability;
use stubby_controller::reconcile::{error_policy, reconcile, Ctx};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    observability::init_metrics();

    // Install the rustls process-wide crypto provider exactly once before
    // building the kube client. kube's rustls-tls stack pulls in more than one
    // provider transitively, so rustls cannot auto-pick and panics unless a
    // default is installed. `.ok()` because in a fresh main() it cannot already
    // be installed. Mirrors crates/webhook/src/main.rs.
    rustls::crypto::aws_lc_rs::default_provider()
        .install_default()
        .ok();

    let cfg = ControllerConfig::from_env()?;
    let client = Client::try_default().await?;
    let pods: Api<Pod> = Api::all(client.clone());
    let ctx = Arc::new(Ctx { client, cfg });

    // Serve /metrics on a plain HTTP port (no TLS; scrape target only).
    tokio::spawn(serve_metrics());

    tracing::info!("stubby-controller starting");
    Controller::new(pods, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            match res {
                Ok((obj, action)) => tracing::debug!(?obj, ?action, "reconciled"),
                Err(e) => tracing::warn!(error = %e, "reconcile stream error"),
            }
        })
        .await;
    Ok(())
}

async fn serve_metrics() {
    use axum::{routing::get, Router};
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route(
            "/metrics",
            get(|| async {
                (
                    [(
                        axum::http::header::CONTENT_TYPE,
                        "text/plain; version=0.0.4",
                    )],
                    observability::render(),
                )
            }),
        );
    let listener = match tokio::net::TcpListener::bind("0.0.0.0:9090").await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(err = %e, "failed to bind metrics port");
            return;
        }
    };
    if let Err(e) = axum::serve(listener, app).await {
        tracing::error!(err = %e, "metrics server exited");
    }
}
