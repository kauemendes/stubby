use crate::BackendConfig;
use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Router};

pub fn router(cfg: BackendConfig) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/openapi.json", get(openapi))
        .route("/docs", get(docs))
        .fallback(catchall)
        .with_state(cfg)
}

async fn health() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn ready() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

async fn openapi(State(cfg): State<BackendConfig>) -> impl IntoResponse {
    let body = crate::openapi::doc(&cfg.app_name);
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
}

async fn docs() -> impl IntoResponse {
    let html = r#"<!doctype html><html><head><title>stubby docs</title>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
</head><body><div id="swagger"></div>
<script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
<script>SwaggerUIBundle({url:'/openapi.json',dom_id:'#swagger'})</script>
</body></html>"#;
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/html; charset=utf-8")],
        html,
    )
}

async fn catchall(
    State(cfg): State<BackendConfig>,
    req: axum::http::Request<axum::body::Body>,
) -> impl IntoResponse {
    let path = req.uri().path().to_string();
    let body = serde_json::json!({
        "status": "dummy",
        "app": cfg.app_name,
        "path": path,
    });
    (StatusCode::OK, axum::Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn cfg() -> BackendConfig {
        BackendConfig {
            app_name: "demo".into(),
        }
    }

    async fn get(path: &str) -> (StatusCode, String) {
        let app = router(cfg());
        let resp = app
            .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let s = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .unwrap();
        (s, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test]
    async fn health_ok() {
        let (s, b) = get("/health").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b, "ok");
    }

    #[tokio::test]
    async fn ready_ok() {
        let (s, _) = get("/ready").await;
        assert_eq!(s, StatusCode::OK);
    }

    #[tokio::test]
    async fn openapi_includes_app_name() {
        let (s, b) = get("/openapi.json").await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["info"]["title"], "demo (dummy)");
    }

    #[tokio::test]
    async fn docs_renders_html() {
        let (s, b) = get("/docs").await;
        assert_eq!(s, StatusCode::OK);
        assert!(b.contains("swagger-ui"));
    }

    #[tokio::test]
    async fn catchall_returns_dummy() {
        let (s, b) = get("/anything/else").await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["status"], "dummy");
        assert_eq!(v["app"], "demo");
        assert_eq!(v["path"], "/anything/else");
    }
}
