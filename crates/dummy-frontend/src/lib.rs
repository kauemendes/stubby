const TEMPLATE: &str = include_str!("../templates/index.html.tmpl");

/// Renders the dummy-frontend index page with `app_name` HTML-escaped and
/// substituted in place of every `{{APP_NAME}}` token.
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
