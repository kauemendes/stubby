//! The reconcile function and its pure planner.
//!
//! `plan_action` chooses stub / revert / nothing from already-computed inputs
//! (stub candidates, recorded originals, and per-image availability) and is
//! unit-tested. `reconcile` gathers those inputs from the cluster and applies
//! the resulting patch.
use crate::annotations::{encode_original_images, ORIGINAL_IMAGE, RESCUED_AT};
use crate::config::ControllerConfig;
use crate::decision::{containers_to_stub, rescued_originals, RescueConfig, StubAction};
use crate::{observability, registry};
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::api::{Api, Patch, PatchParams};
use kube::runtime::controller::Action;
use kube::ResourceExt;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

pub struct Ctx {
    pub client: kube::Client,
    pub cfg: ControllerConfig,
}

#[derive(Debug)]
pub enum Plan {
    /// (container_name, original_image, dummy_image) tuples to stub.
    Stub(Vec<(String, String, String)>),
    /// (container_name, original_image) tuples to restore.
    Revert(Vec<(String, String)>),
    Nothing,
}

/// Pure branch selection. Stub wins over revert so a newly-failing container is
/// rescued even while another is being restored.
pub fn plan_action(
    stub_candidates: &[(String, String, String)],
    rescued: &BTreeMap<String, String>,
    availability: &BTreeMap<String, bool>,
) -> Plan {
    if !stub_candidates.is_empty() {
        return Plan::Stub(stub_candidates.to_vec());
    }
    if rescued.is_empty() {
        return Plan::Nothing;
    }
    let all_available = rescued
        .values()
        .all(|img| *availability.get(img).unwrap_or(&false));
    if all_available {
        Plan::Revert(
            rescued
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        )
    } else {
        Plan::Nothing
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("kube api error: {0}")]
    Kube(#[from] kube::Error),
}

/// Reconcile one pod. Returns the requeue delay.
pub async fn reconcile(pod: Arc<Pod>, ctx: Arc<Ctx>) -> Result<Action, Error> {
    // Cheap early-out: ignore pods that didn't opt in.
    if pod
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(crate::annotations::AUTO_RESCUE))
        .map(String::as_str)
        != Some("true")
    {
        return Ok(Action::await_change());
    }

    let ns = pod.namespace().unwrap_or_else(|| "default".into());
    let name = pod.name_any();
    let pods: Api<Pod> = Api::namespaced(ctx.client.clone(), &ns);

    let rescue_cfg = RescueConfig {
        backend_image: ctx.cfg.backend_image.clone(),
        frontend_image: ctx.cfg.frontend_image.clone(),
    };
    let candidates: Vec<(String, String, String)> = containers_to_stub(&pod, &rescue_cfg)
        .into_iter()
        .map(|a: StubAction| (a.name, a.original_image, a.dummy_image))
        .collect();
    let rescued = rescued_originals(&pod);

    let availability = if candidates.is_empty() && !rescued.is_empty() {
        let dockerconfigs = pull_secrets(&ctx.client, &ns, &pod).await;
        let mut map = BTreeMap::new();
        for img in rescued.values() {
            map.insert(
                img.clone(),
                registry::is_available(img, &dockerconfigs).await,
            );
        }
        map
    } else {
        BTreeMap::new()
    };

    match plan_action(&candidates, &rescued, &availability) {
        Plan::Stub(items) => {
            apply_stub(&pods, &name, &pod, &items).await?;
            observability::record_stub();
            tracing::info!(%ns, %name, count = items.len(), "stubbed image-pull failures");
        }
        Plan::Revert(items) => {
            apply_revert(&pods, &name, &items).await?;
            observability::record_revert();
            tracing::info!(%ns, %name, count = items.len(), "reverted to original images");
        }
        Plan::Nothing => {}
    }

    Ok(Action::requeue(ctx.cfg.check_interval))
}

/// Requeue with a fixed backoff on error rather than hot-looping.
pub fn error_policy(_pod: Arc<Pod>, err: &Error, _ctx: Arc<Ctx>) -> Action {
    tracing::warn!(error = %err, "reconcile failed; backing off");
    Action::requeue(Duration::from_secs(30))
}

/// Merge the recorded originals with any new stubs, then patch image +
/// annotations in one strategic merge (container `name` is the merge key).
async fn apply_stub(
    pods: &Api<Pod>,
    name: &str,
    pod: &Pod,
    items: &[(String, String, String)],
) -> Result<(), Error> {
    let originals = rescued_originals(pod);
    let patch = build_stub_patch(&originals, items);
    pods.patch(name, &PatchParams::default(), &Patch::Strategic(patch))
        .await?;
    Ok(())
}

/// Build the strategic-merge patch that stubs `items` (container_name,
/// original_image, dummy_image), recording the merged originals map.
pub fn build_stub_patch(
    originals: &BTreeMap<String, String>,
    items: &[(String, String, String)],
) -> serde_json::Value {
    let mut merged = originals.clone();
    let mut containers = Vec::new();
    for (cname, orig, dummy) in items {
        merged.insert(cname.clone(), orig.clone());
        containers.push(serde_json::json!({"name": cname, "image": dummy}));
    }
    serde_json::json!({
        "metadata": {"annotations": {
            ORIGINAL_IMAGE: encode_original_images(&merged),
            RESCUED_AT: now_rfc3339(),
        }},
        "spec": {"containers": containers}
    })
}

/// Restore each container's original image and delete the controller
/// annotations (null in a merge patch removes the key).
async fn apply_revert(
    pods: &Api<Pod>,
    name: &str,
    items: &[(String, String)],
) -> Result<(), Error> {
    let patch = build_revert_patch(items);
    pods.patch(name, &PatchParams::default(), &Patch::Strategic(patch))
        .await?;
    Ok(())
}

/// Build the strategic-merge patch that restores `items` (container_name,
/// original_image) and deletes the controller annotations.
pub fn build_revert_patch(items: &[(String, String)]) -> serde_json::Value {
    let containers: Vec<_> = items
        .iter()
        .map(|(cname, orig)| serde_json::json!({"name": cname, "image": orig}))
        .collect();
    serde_json::json!({
        "metadata": {"annotations": {
            ORIGINAL_IMAGE: serde_json::Value::Null,
            RESCUED_AT: serde_json::Value::Null,
        }},
        "spec": {"containers": containers}
    })
}

/// Read every referenced pull secret's `.dockerconfigjson` payload. Missing or
/// unreadable secrets are skipped (best-effort; the check just stays "not
/// available").
async fn pull_secrets(client: &kube::Client, ns: &str, pod: &Pod) -> Vec<Vec<u8>> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), ns);
    let mut names: Vec<String> = pod
        .spec
        .as_ref()
        .and_then(|s| s.image_pull_secrets.as_ref())
        .map(|refs| refs.iter().map(|r| r.name.clone()).collect())
        .unwrap_or_default();
    names.sort();
    names.dedup();

    let mut out = Vec::new();
    for n in names {
        if let Ok(secret) = secrets.get(&n).await {
            if let Some(data) = secret.data {
                if let Some(bytes) = data.get(".dockerconfigjson") {
                    out.push(bytes.0.clone());
                }
            }
        }
    }
    out
}

fn now_rfc3339() -> String {
    k8s_openapi::chrono::Utc::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn plans_stub_when_candidates_present() {
        let plan = plan_action(
            &[(
                "app".to_string(),
                "orig:1".to_string(),
                "dummy:1".to_string(),
            )],
            &BTreeMap::new(),
            &BTreeMap::new(),
        );
        assert!(matches!(plan, Plan::Stub(_)));
    }

    #[test]
    fn plans_revert_only_when_all_originals_available() {
        let mut originals = BTreeMap::new();
        originals.insert("app".to_string(), "orig:1".to_string());
        originals.insert("worker".to_string(), "orig-w:1".to_string());

        let mut avail = BTreeMap::new();
        avail.insert("orig:1".to_string(), true);
        avail.insert("orig-w:1".to_string(), false);
        assert!(matches!(
            plan_action(&[], &originals, &avail),
            Plan::Nothing
        ));

        avail.insert("orig-w:1".to_string(), true);
        assert!(matches!(
            plan_action(&[], &originals, &avail),
            Plan::Revert(_)
        ));
    }

    #[test]
    fn stub_takes_priority_over_revert() {
        let mut originals = BTreeMap::new();
        originals.insert("app".to_string(), "orig:1".to_string());
        let mut avail = BTreeMap::new();
        avail.insert("orig:1".to_string(), true);
        let plan = plan_action(
            &[(
                "new".to_string(),
                "orig2:1".to_string(),
                "dummy:1".to_string(),
            )],
            &originals,
            &avail,
        );
        assert!(matches!(plan, Plan::Stub(_)));
    }

    #[test]
    fn stub_patch_sets_dummy_image_and_records_original() {
        let originals = BTreeMap::new();
        let items = vec![(
            "app".to_string(),
            "orig:1".to_string(),
            "dummy:1".to_string(),
        )];
        let p = build_stub_patch(&originals, &items);
        assert_eq!(p["spec"]["containers"][0]["name"], "app");
        assert_eq!(p["spec"]["containers"][0]["image"], "dummy:1");
        let recorded: BTreeMap<String, String> = serde_json::from_str(
            p["metadata"]["annotations"]["stubby.io/original-image"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(recorded.get("app").map(String::as_str), Some("orig:1"));
    }

    #[test]
    fn revert_patch_restores_image_and_nulls_annotations() {
        let items = vec![("app".to_string(), "orig:1".to_string())];
        let p = build_revert_patch(&items);
        assert_eq!(p["spec"]["containers"][0]["image"], "orig:1");
        assert!(p["metadata"]["annotations"]["stubby.io/original-image"].is_null());
        assert!(p["metadata"]["annotations"]["stubby.io/rescued-at"].is_null());
    }
}
