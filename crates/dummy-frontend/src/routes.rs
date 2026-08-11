//! axum router for the frontend dummy.
//!
//! Mirrors `stubby-dummy-backend`'s shape: `/health` and `/ready` return a
//! plain `ok`, `/style.css` serves the embedded stylesheet, and every other
//! path returns the rendered index page (SPA-style catch-all, matching the
//! `try_files … /index.html` behaviour of the old nginx config).
use crate::{render_index, FrontendConfig, STYLE_CSS};
use axum::{
    extract::State, http::header, http::StatusCode, response::IntoResponse, routing::get, Router,
};

pub fn router(cfg: FrontendConfig) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/style.css", get(style))
        .fallback(index)
        .with_state(cfg)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn ready() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn style() -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        STYLE_CSS,
    )
}

/// Serve the rendered dummy page for `/` and any unmatched path. Returning the
/// page (rather than a 404) keeps a frontend behind an Ingress from surfacing
/// hard errors while the real app is still being built.
async fn index(State(cfg): State<FrontendConfig>) -> impl IntoResponse {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        render_index(&cfg.app_name),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn cfg() -> FrontendConfig {
        FrontendConfig {
            app_name: "Storefront".into(),
        }
    }

    async fn get_path(path: &str) -> (StatusCode, Option<String>, String) {
        let app = router(cfg());
        let resp = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = resp.status();
        let ctype = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        (status, ctype, String::from_utf8(bytes.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn health_ok() {
        let (s, _, b) = get_path("/health").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b, "ok");
    }

    #[tokio::test]
    async fn ready_ok() {
        let (s, _, b) = get_path("/ready").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b, "ok");
    }

    #[tokio::test]
    async fn style_css_served_with_css_content_type() {
        let (s, ctype, b) = get_path("/style.css").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(ctype.as_deref(), Some("text/css; charset=utf-8"));
        assert_eq!(b, STYLE_CSS);
    }

    #[tokio::test]
    async fn root_serves_rendered_index_with_app_name() {
        let (s, ctype, b) = get_path("/").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(ctype.as_deref(), Some("text/html; charset=utf-8"));
        assert!(b.contains("Storefront"), "index should contain app name");
        assert!(b.contains("<!doctype html>"));
    }

    #[tokio::test]
    async fn catchall_serves_index() {
        let (s, ctype, b) = get_path("/some/deep/spa/route").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(ctype.as_deref(), Some("text/html; charset=utf-8"));
        assert!(b.contains("Storefront"));
    }
}
