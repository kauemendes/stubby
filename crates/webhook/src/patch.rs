use crate::annotation::{DummyType, StubbyConfig};
use crate::config::ImageRefs;
use json_patch::PatchOperation;
use k8s_openapi::api::core::v1::Pod;

pub const ALWAYS_SKIP_PREFIXES: &[&str] = &["istio-", "linkerd-", "vault-", "cilium-"];

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

        // ports
        ops.push(replace_op(
            &format!("{base}/ports"),
            serde_json::json!([{
                "containerPort": cfg.port,
                "name": "http",
                "protocol": "TCP"
            }]),
        ));

        // liveness probe
        ops.push(replace_op(
            &format!("{base}/livenessProbe"),
            serde_json::json!({
                "httpGet": {"path": "/health", "port": cfg.port},
                "initialDelaySeconds": 1,
                "periodSeconds": 10
            }),
        ));

        // readiness probe
        ops.push(replace_op(
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

        // env: append STUBBY_APP_NAME if env array exists; otherwise create it
        let env_value = serde_json::json!({
            "name": "STUBBY_APP_NAME",
            "value": cfg.app_name
        });
        if c.env.is_some() {
            ops.push(add_op(&format!("{base}/env/-"), env_value));
        } else {
            ops.push(add_op(
                &format!("{base}/env"),
                serde_json::json!([env_value]),
            ));
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
    ops
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
        assert!(find("replace", "/ports").is_some(), "ports not patched");
        let ports = find("replace", "/ports").unwrap()["value"].clone();
        assert_eq!(
            ports,
            json!([{"containerPort": 8080, "name": "http", "protocol": "TCP"}])
        );

        let lp = find("replace", "/livenessProbe").unwrap()["value"].clone();
        assert_eq!(lp["httpGet"]["path"], "/health");
        assert_eq!(lp["httpGet"]["port"], 8080);

        let rp = find("replace", "/readinessProbe").unwrap()["value"].clone();
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
            .find(|x| {
                x["op"] == "replace" && x["path"].as_str().unwrap().ends_with("/livenessProbe")
            })
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
}
