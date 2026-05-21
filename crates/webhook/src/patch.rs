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
        let image = chosen_image(cfg, imgs);
        let op: PatchOperation = serde_json::from_value(serde_json::json!({
            "op": "replace",
            "path": format!("/spec/containers/{i}/image"),
            "value": image,
        }))
        .expect("hard-coded JSON Patch op is well-formed");
        ops.push(op);
    }
    ops
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
}
