use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Installs the Prometheus recorder exactly once for the process. Subsequent
/// calls (e.g. from tests sharing the binary) reuse the existing handle.
pub fn init_metrics() -> &'static PrometheusHandle {
    HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .install_recorder()
            .expect("install prometheus recorder")
    })
}

/// Returns the current scrape body, or an empty string if `init_metrics`
/// was never called (typical only in tests that don't exercise /metrics).
pub fn render() -> String {
    HANDLE.get().map(|h| h.render()).unwrap_or_default()
}

/// Records one admission decision. `decision` is `"inject"` or `"skip"`;
/// `kind` is the admitted resource (`"pod"` for now).
pub fn record_admission(decision: &'static str, kind: &'static str) {
    metrics::counter!(
        "stubby_admissions_total",
        "type" => kind,
        "decision" => decision,
    )
    .increment(1);
}
