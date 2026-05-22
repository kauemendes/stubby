//! HTTP layer — axum router, body extraction, and admission dispatch.
//!
//! The router wires four endpoints:
//! - `GET /healthz`, `GET /readyz` — liveness/readiness probes.
//! - `GET /metrics` — Prometheus exposition format.
//! - `POST /mutate` — the admission webhook itself.
//!
//! `/mutate` accepts the body as raw bytes (not `Json<...>`) so we can
//! return a synthetic `AdmissionResponse` with a status message on decode
//! errors instead of an HTTP 4xx that would trip `failurePolicy`.
use crate::admission::{handle, AdmissionResponse, AdmissionReview, AdmissionStatus};
use crate::config::ImageRefs;
use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{header, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

/// Generous upper bound for admission payloads. Pods carrying many sidecars
/// or large env blocks can exceed axum's 2 MiB default; the API server's
/// `--max-request-bytes` defaults to ~3 MiB, so 8 MiB has plenty of head room.
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

/// Shared request state — currently only the dummy image references.
#[derive(Clone)]
pub struct AppState {
    pub image_refs: Arc<ImageRefs>,
}

/// Build the application router, wiring all four routes with the body-size limit.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route("/metrics", get(metrics_handler))
        .route("/mutate", post(mutate))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .with_state(state)
}

async fn metrics_handler() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            "text/plain; version=0.0.4; charset=utf-8",
        )],
        crate::observability::render(),
    )
}

fn classify(r: &AdmissionReview) -> (&'static str, &'static str) {
    match r.response.as_ref().and_then(|x| x.patch.as_ref()) {
        Some(_) => ("inject", "pod"),
        None => ("skip", "pod"),
    }
}

/// Admission webhooks must always reply with HTTP 200 carrying an
/// `AdmissionReview` envelope — non-200 makes the API server fall back to
/// `failurePolicy`. We therefore handle malformed bodies ourselves and turn
/// them into an allowed-but-status-tagged response.
async fn mutate(State(state): State<AppState>, body: Bytes) -> impl IntoResponse {
    let review: AdmissionReview = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(e) => {
            let synthetic = AdmissionReview {
                api_version: "admission.k8s.io/v1".into(),
                kind: "AdmissionReview".into(),
                request: None,
                response: Some(AdmissionResponse {
                    uid: String::new(),
                    allowed: true,
                    patch: None,
                    patch_type: None,
                    status: Some(AdmissionStatus {
                        message: format!("stubby: malformed AdmissionReview body: {e}"),
                    }),
                }),
            };
            crate::observability::record_admission("error", "pod");
            return (StatusCode::OK, Json(synthetic));
        }
    };
    let out = handle(review, state.image_refs.as_ref());
    let (decision, kind) = classify(&out);
    crate::observability::record_admission(decision, kind);
    (StatusCode::OK, Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn state() -> AppState {
        AppState {
            image_refs: Arc::new(ImageRefs {
                backend: "ghcr.io/test/be:1".into(),
                frontend: "ghcr.io/test/fe:1".into(),
            }),
        }
    }

    async fn body_bytes(resp: axum::response::Response) -> Vec<u8> {
        axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap()
            .to_vec()
    }

    #[tokio::test]
    async fn healthz_returns_200() {
        let app = router(state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn readyz_returns_200() {
        let app = router(state());
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/readyz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn mutate_returns_review_response() {
        let app = router(state());
        let body = serde_json::json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "abc",
                "object": {
                    "apiVersion":"v1","kind":"Pod",
                    "metadata":{"name":"p","annotations":{"stubby.io/type":"backend"}},
                    "spec":{"containers":[{"name":"app","image":"orig:1"}]}
                }
            }
        })
        .to_string();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mutate")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = body_bytes(resp).await;
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["response"]["uid"], "abc");
        assert_eq!(v["response"]["allowed"], true);
        assert!(v["response"]["patch"].is_string());
    }

    #[tokio::test]
    async fn metrics_endpoint_exposes_admission_counter_after_request() {
        crate::observability::init_metrics();
        let app = router(state());

        // Drive one /mutate to make the counter visible in the scrape body.
        let body = serde_json::json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "metrics-1",
                "object": {
                    "apiVersion":"v1","kind":"Pod",
                    "metadata":{"name":"p","annotations":{"stubby.io/type":"backend"}},
                    "spec":{"containers":[{"name":"app","image":"orig:1"}]}
                }
            }
        })
        .to_string();
        let _ = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mutate")
                    .header("content-type", "application/json")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()
                .get(axum::http::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok()),
            Some("text/plain; version=0.0.4; charset=utf-8"),
        );
        let bytes = body_bytes(resp).await;
        let text = std::str::from_utf8(&bytes).unwrap();
        assert!(
            text.contains("stubby_admissions_total"),
            "scrape should expose the counter, got: {text}"
        );
        assert!(text.contains("decision=\"inject\""));
    }

    #[tokio::test]
    async fn mutate_malformed_body_returns_200_with_status() {
        let app = router(state());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mutate")
                    .body(Body::from("not json at all"))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Admission webhook contract: always 200 with an AdmissionReview body,
        // not a 4xx that the API server would treat as a webhook failure.
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = body_bytes(resp).await;
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["kind"], "AdmissionReview");
        assert_eq!(v["response"]["allowed"], true);
        let msg = v["response"]["status"]["message"].as_str().unwrap();
        assert!(
            msg.starts_with("stubby:"),
            "status message should be prefixed, got {msg}"
        );
    }
}
