//! `stubby-dummy-frontend` — HTML page renderer for the frontend dummy.
//!
//! The runtime image (nginx) substitutes `{{APP_NAME}}` at container startup
//! using the same HTML-escape set as [`render_index`]. This crate keeps the
//! Rust path so the templating is testable and the escape rules live in one
//! place (see `nginx/entrypoint.sh` for the shell version).

const TEMPLATE: &str = include_str!("../templates/index.html.tmpl");

/// Render the dummy-frontend index page.
///
/// `app_name` is HTML-escaped before substitution into every `{{APP_NAME}}`
/// token. The escape set is `& < > " '` — matching the shell entrypoint so
/// both runtimes produce byte-identical output.
pub fn render_index(app_name: &str) -> String {
    TEMPLATE.replace("{{APP_NAME}}", &html_escape(app_name))
}

fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#x27;"),
            c => out.push(c),
        }
    }
    out
}
