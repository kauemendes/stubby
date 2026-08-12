//! Prometheus metrics for the controller. Mirrors the webhook's approach:
//! a global recorder plus a `render()` for the `/metrics` handler.
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install the global recorder. Safe to call more than once (tests do).
pub fn init_metrics() {
    let _ = HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .install_recorder()
            .expect("install prometheus recorder")
    });
}

pub fn render() -> String {
    HANDLE.get().map(|h| h.render()).unwrap_or_default()
}

/// A pod was stubbed: bump the action counter and the live gauge.
pub fn record_stub() {
    metrics::counter!("stubby_rescue_actions_total", "action" => "stub").increment(1);
    metrics::gauge!("stubby_rescued_pods").increment(1.0);
}

/// A pod was reverted: bump the counter and drop the live gauge.
pub fn record_revert() {
    metrics::counter!("stubby_rescue_actions_total", "action" => "revert").increment(1);
    metrics::gauge!("stubby_rescued_pods").decrement(1.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_contains_series_after_actions() {
        init_metrics();
        record_stub();
        record_revert();
        let out = render();
        assert!(out.contains("stubby_rescue_actions_total"));
    }
}
