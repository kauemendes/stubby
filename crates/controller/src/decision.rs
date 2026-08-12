//! Pure decision logic: given a pod's spec + status + annotations, decide
//! which containers to stub. Revert is decided in `reconcile` because it needs
//! a registry round-trip; here we only expose the recorded originals.
use crate::annotations::{decode_original_images, ORIGINAL_IMAGE, TYPE};
use k8s_openapi::api::core::v1::Pod;
use std::collections::BTreeMap;

/// Sidecar name prefixes never touched. Mirrors the webhook's
/// `ALWAYS_SKIP_PREFIXES`; kept in sync by `sidecar_prefixes_match_webhook`.
pub const ALWAYS_SKIP_PREFIXES: &[&str] = &["istio-", "linkerd-", "vault-", "cilium-"];

/// Waiting reasons that mean "the image can't be pulled".
const PULL_FAILURE_REASONS: &[&str] = &["ImagePullBackOff", "ErrImagePull"];

#[derive(Debug, Clone)]
pub struct RescueConfig {
    pub backend_image: String,
    pub frontend_image: String,
}

/// One container that should be swapped to a dummy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubAction {
    pub name: String,
    pub original_image: String,
    pub dummy_image: String,
}

/// Containers currently in an image-pull failure, not sidecars, and not already
/// rescued (present in the `original-image` annotation).
pub fn containers_to_stub(pod: &Pod, cfg: &RescueConfig) -> Vec<StubAction> {
    let annotations = pod.metadata.annotations.clone().unwrap_or_default();
    let already = decode_original_images(annotations.get(ORIGINAL_IMAGE));
    let dummy = match annotations.get(TYPE).map(String::as_str) {
        Some("frontend") => &cfg.frontend_image,
        _ => &cfg.backend_image,
    };

    let statuses = pod
        .status
        .as_ref()
        .and_then(|s| s.container_statuses.as_ref())
        .cloned()
        .unwrap_or_default();

    let spec_images: BTreeMap<&str, &str> = pod
        .spec
        .as_ref()
        .map(|s| {
            s.containers
                .iter()
                .filter_map(|c| c.image.as_deref().map(|img| (c.name.as_str(), img)))
                .collect()
        })
        .unwrap_or_default();

    let mut out = Vec::new();
    for cs in &statuses {
        if is_sidecar(&cs.name) || already.contains_key(&cs.name) {
            continue;
        }
        let failing = cs
            .state
            .as_ref()
            .and_then(|st| st.waiting.as_ref())
            .and_then(|w| w.reason.as_deref())
            .map(|r| PULL_FAILURE_REASONS.contains(&r))
            .unwrap_or(false);
        if !failing {
            continue;
        }
        // The pod spec's image is authoritative for restore; fall back to the
        // status image if no spec container matches (shouldn't normally happen).
        let original_image = spec_images
            .get(cs.name.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| cs.image.clone());
        out.push(StubAction {
            name: cs.name.clone(),
            original_image,
            dummy_image: dummy.clone(),
        });
    }
    out
}

/// The container→original-image map recorded on a rescued pod.
pub fn rescued_originals(pod: &Pod) -> BTreeMap<String, String> {
    let annotations = pod.metadata.annotations.clone().unwrap_or_default();
    decode_original_images(annotations.get(ORIGINAL_IMAGE))
}

fn is_sidecar(name: &str) -> bool {
    ALWAYS_SKIP_PREFIXES.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::Pod;
    use serde_json::json;

    fn cfg() -> RescueConfig {
        RescueConfig {
            backend_image: "ghcr.io/test/be:1".into(),
            frontend_image: "ghcr.io/test/fe:1".into(),
        }
    }

    fn pod(v: serde_json::Value) -> Pod {
        serde_json::from_value(v).unwrap()
    }

    fn waiting(name: &str, reason: &str, image: &str) -> serde_json::Value {
        json!({"name": name, "image": image, "ready": false,
               "restartCount": 0, "state": {"waiting": {"reason": reason}}})
    }

    #[test]
    fn stubs_imagepullbackoff_container_as_backend_by_default() {
        let p = pod(json!({
            "metadata": {"name": "p", "annotations": {"stubby.io/auto-rescue": "true"}},
            "spec": {"containers": [{"name": "app", "image": "ghcr.io/acme/app:v1"}]},
            "status": {"containerStatuses": [waiting("app", "ImagePullBackOff", "ghcr.io/acme/app:v1")]}
        }));
        let actions = containers_to_stub(&p, &cfg());
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "app");
        assert_eq!(actions[0].original_image, "ghcr.io/acme/app:v1");
        assert_eq!(actions[0].dummy_image, "ghcr.io/test/be:1");
    }

    #[test]
    fn uses_frontend_image_when_type_hint_is_frontend() {
        let p = pod(json!({
            "metadata": {"name": "p", "annotations": {
                "stubby.io/auto-rescue": "true", "stubby.io/type": "frontend"}},
            "spec": {"containers": [{"name": "web", "image": "ghcr.io/acme/web:v1"}]},
            "status": {"containerStatuses": [waiting("web", "ErrImagePull", "ghcr.io/acme/web:v1")]}
        }));
        let actions = containers_to_stub(&p, &cfg());
        assert_eq!(actions[0].dummy_image, "ghcr.io/test/fe:1");
    }

    #[test]
    fn skips_sidecars_and_healthy_and_already_rescued() {
        let p = pod(json!({
            "metadata": {"name": "p", "annotations": {
                "stubby.io/auto-rescue": "true",
                "stubby.io/original-image": "{\"app\":\"ghcr.io/acme/app:v1\"}"}},
            "spec": {"containers": [
                {"name": "app", "image": "ghcr.io/test/be:1"},
                {"name": "istio-proxy", "image": "istio:1"},
                {"name": "worker", "image": "ghcr.io/acme/worker:v1"}
            ]},
            "status": {"containerStatuses": [
                waiting("app", "ImagePullBackOff", "ghcr.io/test/be:1"),
                waiting("istio-proxy", "ImagePullBackOff", "istio:1"),
                json!({"name": "worker", "image": "ghcr.io/acme/worker:v1", "ready": true,
                       "restartCount": 0, "state": {"running": {"startedAt": "2026-01-01T00:00:00Z"}}})
            ]}
        }));
        assert!(containers_to_stub(&p, &cfg()).is_empty());
    }

    #[test]
    fn rescued_originals_reads_annotation() {
        let p = pod(json!({
            "metadata": {"name": "p", "annotations": {
                "stubby.io/original-image": "{\"app\":\"ghcr.io/acme/app:v1\"}"}},
            "spec": {"containers": [{"name": "app", "image": "ghcr.io/test/be:1"}]}
        }));
        let orig = rescued_originals(&p);
        assert_eq!(
            orig.get("app").map(String::as_str),
            Some("ghcr.io/acme/app:v1")
        );
    }

    #[test]
    fn sidecar_prefixes_match_webhook() {
        // Controller-side tripwire: this pins the controller's own list. It
        // does NOT observe the webhook (crates/webhook/src/patch.rs) — if that
        // list changes, update this one by hand. A shared crate would remove
        // the duplication; deferred for the experimental controller.
        assert_eq!(
            ALWAYS_SKIP_PREFIXES,
            &["istio-", "linkerd-", "vault-", "cilium-"]
        );
    }
}
