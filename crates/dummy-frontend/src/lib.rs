//! `stubby-dummy-frontend` — the frontend dummy: a tiny axum server that
//! renders a single "I'm a dummy" HTML page.
//!
//! The page is rendered from an embedded template ([`render_index`]) and the
//! stylesheet is embedded too ([`STYLE_CSS`]), so the running container writes
//! nothing to disk. That is what lets a mutated pod satisfy Pod Security
//! "restricted" — `readOnlyRootFilesystem: true`, `runAsNonRoot`, and
//! `drop: [ALL]` — which the previous nginx image could not (its entrypoint
//! rendered the template into `/usr/share/nginx/html` at startup).
//!
//! The listen port is taken from `STUBBY_PORT` (injected by the webhook to
//! match `stubby.io/port`) so the container always listens on the port the
//! probes and Service target. See [`routes::router`] for the HTTP surface.

pub mod routes;

const TEMPLATE: &str = include_str!("../templates/index.html.tmpl");

/// The embedded stylesheet, served verbatim at `GET /style.css`.
///
/// Kept as a separate asset (rather than inlined into the template) so the
/// snapshot tests for [`render_index`] stay byte-stable.
pub const STYLE_CSS: &str = include_str!("../templates/style.css");

/// Per-process configuration sourced from environment variables.
#[derive(Clone, Debug)]
pub struct FrontendConfig {
    /// Display name substituted into the rendered page. Provided by the webhook
    /// via `STUBBY_APP_NAME`; falls back to `"stubby"`.
    pub app_name: String,
}

impl FrontendConfig {
    /// Reads `STUBBY_APP_NAME` from the environment. The webhook always injects
    /// this on pods it mutates, so a missing value typically means the binary
    /// was started outside of stubby's wiring — log a warning to make that
    /// visible without making the binary unbootable.
    pub fn from_env() -> Self {
        match std::env::var("STUBBY_APP_NAME") {
            Ok(name) if !name.trim().is_empty() => Self {
                app_name: name.trim().to_string(),
            },
            _ => {
                tracing::warn!(
                    "STUBBY_APP_NAME unset or blank; falling back to 'stubby' (the webhook normally injects this)"
                );
                Self {
                    app_name: "stubby".into(),
                }
            }
        }
    }
}

/// Render the dummy-frontend index page.
///
/// `app_name` is HTML-escaped before substitution into every `{{APP_NAME}}`
/// token. The escape set is `& < > " '`.
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
