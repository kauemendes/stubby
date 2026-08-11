//! JSON Patch (RFC 6902) builder for the mutation step.
//!
//! [`build_patch`] walks each container in the pod, skipping known sidecars
//! and user-supplied exclusions, and overlays five fields: `image`, `ports`,
//! `livenessProbe`, `readinessProbe`, and `env`. `resources` defaults are
//! emitted only if the manifest did not already declare them, so the
//! operator's choices win.
//!
//! Operations use `add` rather than `replace` for optional fields because
//! `replace` errors out when the target is absent (RFC 6902 §4.3); pod
//! specs frequently ship without `ports`/`probes` until stubby fills them in.
use crate::annotation::{DummyType, StubbyConfig};
use crate::config::ImageRefs;
use json_patch::PatchOperation;
use k8s_openapi::api::core::v1::{Pod, Volume};
use std::collections::BTreeSet;

/// Container-name prefixes that are never mutated.
///
/// These are common service-mesh and secret-injector sidecars that ship
/// their own image and break if rewritten.
pub const ALWAYS_SKIP_PREFIXES: &[&str] = &["istio-", "linkerd-", "vault-", "cilium-"];

/// Build the RFC 6902 patch list that turns `pod` into a stubby-mutated pod.
///
/// Returns an empty vector when every container is skipped — callers should
/// treat that as "respond without a patch", not as an error.
pub fn build_patch(pod: &Pod, cfg: &StubbyConfig, imgs: &ImageRefs) -> Vec<PatchOperation> {
    let containers = pod
        .spec
        .as_ref()
        .map(|s| s.containers.as_slice())
        .unwrap_or(&[]);

    let mut ops = Vec::new();
    for (i, c) in containers.iter().enumerate() {
        if should_skip(&c.name, cfg) {
            continue;
        }
        let base = format!("/spec/containers/{i}");
        let image = chosen_image(cfg, imgs);

        // image
        ops.push(replace_op(
            &format!("{base}/image"),
            serde_json::Value::String(image),
        ));

        // ports — `add` creates the field if absent (RFC 6902 §4.1) which is
        // the common case for stubby-targeted Deployments; if the operator
        // had ports defined, `add` replaces them, matching our intent.
        ops.push(add_op(
            &format!("{base}/ports"),
            serde_json::json!([{
                "containerPort": cfg.port,
                "name": "http",
                "protocol": "TCP"
            }]),
        ));

        // liveness probe — `add` for same reason as ports
        ops.push(add_op(
            &format!("{base}/livenessProbe"),
            serde_json::json!({
                "httpGet": {"path": "/health", "port": cfg.port},
                "initialDelaySeconds": 1,
                "periodSeconds": 10
            }),
        ));

        // readiness probe — `add` for same reason as ports
        ops.push(add_op(
            &format!("{base}/readinessProbe"),
            serde_json::json!({
                "httpGet": {"path": "/ready", "port": cfg.port},
                "initialDelaySeconds": 1,
                "periodSeconds": 5
            }),
        ));

        // command / args removed if present
        if c.command.is_some() {
            ops.push(remove_op(&format!("{base}/command")));
        }
        if c.args.is_some() {
            ops.push(remove_op(&format!("{base}/args")));
        }

        // env: inject STUBBY_APP_NAME (display name) and STUBBY_PORT (so the
        // dummy binary listens on the same port the ports/probes target — the
        // annotated `stubby.io/port`). Append to an existing `env` array,
        // otherwise create it.
        let injected_env = serde_json::json!([
            {"name": "STUBBY_APP_NAME", "value": cfg.app_name},
            {"name": "STUBBY_PORT", "value": cfg.port.to_string()},
        ]);
        if c.env.is_some() {
            for v in injected_env.as_array().expect("literal array") {
                ops.push(add_op(&format!("{base}/env/-"), v.clone()));
            }
        } else {
            ops.push(add_op(&format!("{base}/env"), injected_env));
        }

        // envFrom: strip by default. A `secretRef`/`configMapRef` whose target
        // doesn't exist yet (the norm before the real app is provisioned) puts
        // the pod into `CreateContainerConfigError` — trading ImagePullBackOff
        // for another red state. The dummy needs no real config, so drop it.
        // Opt out with `stubby.io/keep-env-from: "true"`.
        if !cfg.keep_env_from && c.env_from.is_some() {
            ops.push(remove_op(&format!("{base}/envFrom")));
        }

        // volumeMounts: strip by default, same rationale — a mount backed by a
        // missing secret/configMap wedges the pod in `ContainerCreating`. The
        // matching pod-level volumes are pruned after the loop (see below).
        // Opt out with `stubby.io/keep-volumes: "true"`.
        if !cfg.keep_volumes && c.volume_mounts.is_some() {
            ops.push(remove_op(&format!("{base}/volumeMounts")));
        }

        // resources: defaults only if missing
        if c.resources.is_none() {
            ops.push(add_op(
                &format!("{base}/resources"),
                serde_json::json!({
                    "requests": {"cpu": "10m", "memory": "32Mi"},
                    "limits":   {"cpu": "100m", "memory": "64Mi"}
                }),
            ));
        }
    }

    // Pod-level: prune orphaned secret/configMap/projected volumes left behind
    // once mutated containers no longer mount them (see `prune_orphan_volumes`).
    if let Some(op) = prune_orphan_volumes(pod, cfg) {
        ops.push(op);
    }
    ops
}

/// Build a single patch op that drops pod `volumes` which would block startup
/// after mutation, or `None` if there is nothing to prune.
///
/// A `secret`, `configMap`, or `projected` volume whose backing object is
/// missing wedges the pod in `ContainerCreating` (kubelet must set every
/// pod volume up, even ones no started container mounts). Since mutated
/// containers have their `volumeMounts` stripped, such a volume is orphaned —
/// unless a container we *keep* (a skipped sidecar or an init container) still
/// references it, in which case we leave it alone. `emptyDir`, PVCs, etc. are
/// never pruned: they don't fail this way and removing a PVC could hide real
/// intent. No-op when `stubby.io/keep-volumes` is set.
fn prune_orphan_volumes(pod: &Pod, cfg: &StubbyConfig) -> Option<PatchOperation> {
    if cfg.keep_volumes {
        return None;
    }
    let spec = pod.spec.as_ref()?;
    let volumes = spec.volumes.as_ref()?;
    if volumes.is_empty() {
        return None;
    }

    // Volume names still referenced after mutation: by kept (skipped)
    // containers and by any init container. Mutated containers no longer count
    // because their volumeMounts are removed.
    let mut needed: BTreeSet<&str> = BTreeSet::new();
    for c in &spec.containers {
        if should_skip(&c.name, cfg) {
            if let Some(mounts) = &c.volume_mounts {
                for m in mounts {
                    needed.insert(m.name.as_str());
                }
            }
        }
    }
    if let Some(inits) = &spec.init_containers {
        for c in inits {
            if let Some(mounts) = &c.volume_mounts {
                for m in mounts {
                    needed.insert(m.name.as_str());
                }
            }
        }
    }

    let kept: Vec<&Volume> = volumes
        .iter()
        .filter(|v| {
            let blocks_on_missing =
                v.secret.is_some() || v.config_map.is_some() || v.projected.is_some();
            !blocks_on_missing || needed.contains(v.name.as_str())
        })
        .collect();

    if kept.len() == volumes.len() {
        return None; // nothing orphaned
    }
    if kept.is_empty() {
        Some(remove_op("/spec/volumes"))
    } else {
        Some(replace_op(
            "/spec/volumes",
            serde_json::to_value(kept).expect("Volume serializes to JSON"),
        ))
    }
}

fn replace_op(path: &str, value: serde_json::Value) -> PatchOperation {
    serde_json::from_value(serde_json::json!({
        "op": "replace",
        "path": path,
        "value": value,
    }))
    .expect("hard-coded JSON Patch op is well-formed")
}

fn add_op(path: &str, value: serde_json::Value) -> PatchOperation {
    serde_json::from_value(serde_json::json!({
        "op": "add",
        "path": path,
        "value": value,
    }))
    .expect("hard-coded JSON Patch op is well-formed")
}

fn remove_op(path: &str) -> PatchOperation {
    serde_json::from_value(serde_json::json!({
        "op": "remove",
        "path": path,
    }))
    .expect("hard-coded JSON Patch op is well-formed")
}

fn should_skip(name: &str, cfg: &StubbyConfig) -> bool {
    ALWAYS_SKIP_PREFIXES.iter().any(|p| name.starts_with(p))
        || cfg.skip_containers.iter().any(|n| n == name)
}

fn chosen_image(cfg: &StubbyConfig, imgs: &ImageRefs) -> String {
    if let Some(o) = &cfg.image_override {
        return o.clone();
    }
    match cfg.dummy_type {
        DummyType::Backend => imgs.backend.clone(),
        DummyType::Frontend => imgs.frontend.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn refs() -> ImageRefs {
        ImageRefs {
            backend: "ghcr.io/test/be:1".into(),
            frontend: "ghcr.io/test/fe:1".into(),
        }
    }

    fn pod_with_containers(containers: serde_json::Value) -> Pod {
        let v = json!({
            "apiVersion": "v1",
            "kind": "Pod",
            "metadata": {"name": "p"},
            "spec": {"containers": containers}
        });
        serde_json::from_value(v).unwrap()
    }

    fn backend_cfg() -> StubbyConfig {
        StubbyConfig {
            dummy_type: DummyType::Backend,
            app_name: "myapp".into(),
            port: 8080,
            image_override: None,
            skip_containers: vec![],
            keep_env_from: false,
            keep_volumes: false,
        }
    }

    #[test]
    fn single_backend_container_image_replaced() {
        let pod = pod_with_containers(json!([{"name":"app","image":"orig:1"}]));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let images: Vec<_> = ops
            .iter()
            .filter_map(|op| match op {
                PatchOperation::Replace(r) if r.path.to_string().ends_with("/image") => {
                    Some(r.value.as_str().unwrap().to_string())
                }
                _ => None,
            })
            .collect();
        assert_eq!(images, vec!["ghcr.io/test/be:1"]);
    }

    fn ops_to_json(ops: &[PatchOperation]) -> serde_json::Value {
        serde_json::to_value(ops).unwrap()
    }

    #[test]
    fn backend_replaces_ports_probes_env_and_removes_command() {
        let pod = pod_with_containers(json!([{
            "name": "app",
            "image": "orig:1",
            "command": ["/bin/old"],
            "args": ["--flag"],
            "env": [{"name": "FOO", "value": "1"}]
        }]));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();

        let find = |op: &str, path_suffix: &str| -> Option<serde_json::Value> {
            arr.iter()
                .find(|x| x["op"] == op && x["path"].as_str().unwrap().ends_with(path_suffix))
                .cloned()
        };

        assert!(find("replace", "/image").is_some());
        assert!(find("add", "/ports").is_some(), "ports not patched");
        let ports = find("add", "/ports").unwrap()["value"].clone();
        assert_eq!(
            ports,
            json!([{"containerPort": 8080, "name": "http", "protocol": "TCP"}])
        );

        let lp = find("add", "/livenessProbe").unwrap()["value"].clone();
        assert_eq!(lp["httpGet"]["path"], "/health");
        assert_eq!(lp["httpGet"]["port"], 8080);

        let rp = find("add", "/readinessProbe").unwrap()["value"].clone();
        assert_eq!(rp["httpGet"]["path"], "/ready");
        assert_eq!(rp["httpGet"]["port"], 8080);

        assert!(find("remove", "/command").is_some());
        assert!(find("remove", "/args").is_some());

        let add_env = arr
            .iter()
            .find(|x| x["op"] == "add" && x["path"].as_str().unwrap().ends_with("/env/-"))
            .unwrap();
        assert_eq!(add_env["value"]["name"], "STUBBY_APP_NAME");
        assert_eq!(add_env["value"]["value"], "myapp");
    }

    #[test]
    fn frontend_uses_port_80_by_default() {
        let cfg = StubbyConfig {
            dummy_type: DummyType::Frontend,
            port: 80,
            ..backend_cfg()
        };
        let pod = pod_with_containers(json!([{"name":"web","image":"orig:1"}]));
        let ops = build_patch(&pod, &cfg, &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        let lp = arr
            .iter()
            .find(|x| x["op"] == "add" && x["path"].as_str().unwrap().ends_with("/livenessProbe"))
            .unwrap();
        assert_eq!(lp["value"]["httpGet"]["port"], 80);
    }

    #[test]
    fn adds_default_resources_when_missing() {
        let pod = pod_with_containers(json!([{"name":"app","image":"orig:1"}]));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        let r = arr
            .iter()
            .find(|x| x["path"].as_str().unwrap().ends_with("/resources"))
            .unwrap();
        assert_eq!(r["op"], "add");
        assert_eq!(r["value"]["requests"]["cpu"], "10m");
        assert_eq!(r["value"]["limits"]["memory"], "64Mi");
    }

    #[test]
    fn preserves_existing_resources() {
        let pod = pod_with_containers(json!([{
            "name": "app",
            "image": "orig:1",
            "resources": {"requests": {"cpu": "500m"}}
        }]));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        assert!(arr
            .iter()
            .all(|x| !x["path"].as_str().unwrap().ends_with("/resources")));
    }

    #[test]
    fn multi_container_patches_each_non_sidecar() {
        let pod = pod_with_containers(json!([
            {"name": "app", "image": "orig:1"},
            {"name": "audit", "image": "audit:1"}
        ]));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        let imgs: Vec<_> = arr
            .iter()
            .filter(|x| x["op"] == "replace" && x["path"].as_str().unwrap().ends_with("/image"))
            .map(|x| x["value"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(imgs, vec!["ghcr.io/test/be:1", "ghcr.io/test/be:1"]);
    }

    #[test]
    fn skips_known_sidecar_prefixes() {
        let pod = pod_with_containers(json!([
            {"name": "app", "image": "orig:1"},
            {"name": "istio-proxy", "image": "istio:1"},
            {"name": "linkerd-init", "image": "linkerd:1"}
        ]));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        let paths: Vec<_> = arr
            .iter()
            .filter(|x| x["op"] == "replace" && x["path"].as_str().unwrap().ends_with("/image"))
            .map(|x| x["path"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(paths, vec!["/spec/containers/0/image"]);
    }

    #[test]
    fn skips_user_provided_skip_containers() {
        let cfg = StubbyConfig {
            skip_containers: vec!["telemetry".into()],
            ..backend_cfg()
        };
        let pod = pod_with_containers(json!([
            {"name": "app", "image": "orig:1"},
            {"name": "telemetry", "image": "tel:1"}
        ]));
        let ops = build_patch(&pod, &cfg, &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        let paths: Vec<_> = arr
            .iter()
            .filter(|x| x["op"] == "replace" && x["path"].as_str().unwrap().ends_with("/image"))
            .map(|x| x["path"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(paths, vec!["/spec/containers/0/image"]);
    }

    #[test]
    fn image_override_used_instead_of_default() {
        let cfg = StubbyConfig {
            image_override: Some("ghcr.io/me/custom:dev".into()),
            ..backend_cfg()
        };
        let pod = pod_with_containers(json!([{"name":"app","image":"orig:1"}]));
        let ops = build_patch(&pod, &cfg, &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        let img = arr
            .iter()
            .find(|x| x["op"] == "replace" && x["path"].as_str().unwrap().ends_with("/image"))
            .unwrap()["value"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(img, "ghcr.io/me/custom:dev");
    }

    fn pod_from_spec(spec: serde_json::Value) -> Pod {
        let v = json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name": "p"},
            "spec": spec
        });
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn injects_stubby_port_env_matching_config() {
        let cfg = StubbyConfig {
            port: 9090,
            ..backend_cfg()
        };
        let pod = pod_with_containers(json!([{"name":"app","image":"orig:1"}]));
        let ops = build_patch(&pod, &cfg, &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        // No pre-existing env: a single `add /env` carries both vars.
        let env_add = arr
            .iter()
            .find(|x| x["op"] == "add" && x["path"].as_str().unwrap().ends_with("/env"))
            .expect("env add op");
        let vals = env_add["value"].as_array().unwrap();
        let port = vals
            .iter()
            .find(|v| v["name"] == "STUBBY_PORT")
            .expect("STUBBY_PORT injected");
        assert_eq!(port["value"], "9090");
        assert!(vals.iter().any(|v| v["name"] == "STUBBY_APP_NAME"));
    }

    #[test]
    fn appends_both_env_vars_when_env_present() {
        let pod = pod_with_containers(
            json!([{"name":"app","image":"orig:1","env":[{"name":"FOO","value":"1"}]}]),
        );
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        let appended: Vec<_> = arr
            .iter()
            .filter(|x| x["op"] == "add" && x["path"].as_str().unwrap().ends_with("/env/-"))
            .map(|x| x["value"]["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(appended, vec!["STUBBY_APP_NAME", "STUBBY_PORT"]);
    }

    #[test]
    fn strips_env_from_by_default() {
        let pod = pod_with_containers(json!([{
            "name":"app","image":"orig:1",
            "envFrom":[{"secretRef":{"name":"not-created-yet"}}]
        }]));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        assert!(
            arr.iter()
                .any(|x| x["op"] == "remove" && x["path"] == "/spec/containers/0/envFrom"),
            "orphan envFrom must be removed so the pod boots green"
        );
    }

    #[test]
    fn keeps_env_from_when_opted_in() {
        let cfg = StubbyConfig {
            keep_env_from: true,
            ..backend_cfg()
        };
        let pod = pod_with_containers(json!([{
            "name":"app","image":"orig:1",
            "envFrom":[{"secretRef":{"name":"real"}}]
        }]));
        let ops = build_patch(&pod, &cfg, &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        assert!(arr
            .iter()
            .all(|x| x["path"].as_str().unwrap() != "/spec/containers/0/envFrom"));
    }

    #[test]
    fn no_env_from_op_when_container_has_none() {
        let pod = pod_with_containers(json!([{"name":"app","image":"orig:1"}]));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        assert!(arr
            .iter()
            .all(|x| !x["path"].as_str().unwrap().ends_with("/envFrom")));
    }

    #[test]
    fn strips_volume_mounts_and_prunes_orphan_secret_volume() {
        let pod = pod_from_spec(json!({
            "containers": [{
                "name":"app","image":"orig:1",
                "volumeMounts":[{"name":"cfg","mountPath":"/etc/cfg"}]
            }],
            "volumes":[{"name":"cfg","secret":{"secretName":"not-created-yet"}}]
        }));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        assert!(
            arr.iter()
                .any(|x| x["op"] == "remove" && x["path"] == "/spec/containers/0/volumeMounts"),
            "volumeMounts must be stripped"
        );
        assert!(
            arr.iter()
                .any(|x| x["op"] == "remove" && x["path"] == "/spec/volumes"),
            "the sole orphan secret volume must be pruned"
        );
    }

    #[test]
    fn prunes_only_orphan_volume_and_keeps_the_rest() {
        let pod = pod_from_spec(json!({
            "containers":[{
                "name":"app","image":"orig:1",
                "volumeMounts":[
                    {"name":"cfg","mountPath":"/etc/cfg"},
                    {"name":"scratch","mountPath":"/tmp"}
                ]
            }],
            "volumes":[
                {"name":"cfg","secret":{"secretName":"not-created-yet"}},
                {"name":"scratch","emptyDir":{}}
            ]
        }));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        let vol_op = arr
            .iter()
            .find(|x| x["path"] == "/spec/volumes")
            .expect("expected a /spec/volumes op");
        assert_eq!(vol_op["op"], "replace");
        let kept = vol_op["value"].as_array().unwrap();
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0]["name"], "scratch");
    }

    #[test]
    fn does_not_prune_emptydir_volume() {
        let pod = pod_from_spec(json!({
            "containers":[{
                "name":"app","image":"orig:1",
                "volumeMounts":[{"name":"scratch","mountPath":"/tmp"}]
            }],
            "volumes":[{"name":"scratch","emptyDir":{}}]
        }));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        assert!(arr
            .iter()
            .any(|x| x["op"] == "remove" && x["path"] == "/spec/containers/0/volumeMounts"));
        assert!(
            arr.iter()
                .all(|x| x["path"].as_str().unwrap() != "/spec/volumes"),
            "emptyDir does not block on a missing object, so keep it"
        );
    }

    #[test]
    fn preserves_secret_volume_still_used_by_skipped_sidecar() {
        let pod = pod_from_spec(json!({
            "containers":[
                {"name":"app","image":"orig:1",
                 "volumeMounts":[{"name":"cfg","mountPath":"/etc/cfg"}]},
                {"name":"istio-proxy","image":"istio:1",
                 "volumeMounts":[{"name":"cfg","mountPath":"/etc/cfg"}]}
            ],
            "volumes":[{"name":"cfg","secret":{"secretName":"mesh-certs"}}]
        }));
        let ops = build_patch(&pod, &backend_cfg(), &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        assert!(
            arr.iter()
                .all(|x| x["path"].as_str().unwrap() != "/spec/volumes"),
            "volume is still mounted by the skipped sidecar; must not be pruned"
        );
        assert!(arr
            .iter()
            .any(|x| x["op"] == "remove" && x["path"] == "/spec/containers/0/volumeMounts"));
    }

    #[test]
    fn keeps_volumes_and_mounts_when_opted_in() {
        let cfg = StubbyConfig {
            keep_volumes: true,
            ..backend_cfg()
        };
        let pod = pod_from_spec(json!({
            "containers":[{
                "name":"app","image":"orig:1",
                "volumeMounts":[{"name":"cfg","mountPath":"/etc/cfg"}]
            }],
            "volumes":[{"name":"cfg","secret":{"secretName":"real"}}]
        }));
        let ops = build_patch(&pod, &cfg, &refs());
        let j = ops_to_json(&ops);
        let arr = j.as_array().unwrap();
        assert!(arr
            .iter()
            .all(|x| x["path"].as_str().unwrap() != "/spec/volumes"));
        assert!(arr
            .iter()
            .all(|x| x["path"].as_str().unwrap() != "/spec/containers/0/volumeMounts"));
    }
}
