use crate::admission::{handle, AdmissionReview};
use crate::config::ImageRefs;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub image_refs: Arc<ImageRefs>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route("/mutate", post(mutate))
        .with_state(state)
}

async fn mutate(
    State(state): State<AppState>,
    Json(review): Json<AdmissionReview>,
) -> impl IntoResponse {
    let out = handle(review, state.image_refs.as_ref());
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
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["response"]["uid"], "abc");
        assert_eq!(v["response"]["allowed"], true);
        assert!(v["response"]["patch"].is_string());
    }
}
