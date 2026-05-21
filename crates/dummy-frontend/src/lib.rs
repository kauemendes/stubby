// crates/dummy-frontend/src/lib.rs
pub fn render_index(app_name: &str) -> String {
    format!("<!doctype html><title>{}</title>", app_name)
}
