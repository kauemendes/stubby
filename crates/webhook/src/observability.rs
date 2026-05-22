//! Prometheus metrics + the `stubby_admissions_total` counter.
//!
//! The recorder is installed once at startup via [`init_metrics`].
//! [`render`] backs the `GET /metrics` endpoint; [`record_admission`]
//! is called from the admission handler on every decision.
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Installs the Prometheus recorder exactly once for the process. `OnceLock`
/// guarantees only one installation; subsequent calls are cheap no-ops that
/// return the same handle. Panics only if the very first install fails
/// (e.g. another recorder already claimed the global slot via a different
/// init path) — which is an unrecoverable configuration error.
pub fn init_metrics() {
    HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .install_recorder()
            .expect("install prometheus recorder")
    });
}

/// Returns the current scrape body, or an empty string if `init_metrics`
/// was never called (typical only in tests that don't exercise /metrics).
pub fn render() -> String {
    HANDLE.get().map(|h| h.render()).unwrap_or_default()
}

/// Records one admission decision. `kind` is the admitted resource type
/// (`"pod"` for now); `decision` is one of:
///   - `"inject"` — webhook returned a JSONPatch.
///   - `"skip"`   — webhook allowed the pod unchanged (no annotation, type=off, etc.).
///   - `"error"`  — webhook couldn't decode the AdmissionReview body; it
///                  still allowed the request but with a status message.
pub fn record_admission(decision: &'static str, kind: &'static str) {
    metrics::counter!(
        "stubby_admissions_total",
        "type" => kind,
        "decision" => decision,
    )
    .increment(1);
}
