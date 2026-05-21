use stubby_dummy_frontend::render_index;

#[test]
fn simple_name_snapshot() {
    let html = render_index("Orders");
    insta::assert_snapshot!("simple_name", html);
}

#[test]
fn xss_attempt_is_escaped() {
    let html = render_index("<script>alert(1)</script>");
    assert!(!html.contains("<script>"), "raw <script> must not leak");
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    insta::assert_snapshot!("xss_escaped", html);
}

#[test]
fn empty_name_renders_literal_empty_heading() {
    let html = render_index("");
    assert!(html.contains("<h1></h1>"), "expected empty h1 in:\n{html}");
}
