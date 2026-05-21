pub fn doc(app_name: &str) -> String {
    serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": format!("{app_name} (dummy)"),
            "version": "0.0.0",
            "description": "Placeholder spec served by stubby-dummy-backend."
        },
        "paths": {
            "/health": {"get": {"responses": {"200": {"description":"ok"}}}},
            "/ready":  {"get": {"responses": {"200": {"description":"ok"}}}}
        }
    })
    .to_string()
}
