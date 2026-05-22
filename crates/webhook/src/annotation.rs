//! Parsing of the `stubby.io/*` annotation set off a pod's metadata.
//!
//! [`parse_annotations`] turns the raw annotation map into a [`Decision`]:
//! either inject the dummy (with a fully-resolved [`StubbyConfig`]) or skip
//! the pod entirely. Unknown values of `stubby.io/type` (including the
//! special value `off`) yield [`Decision::Skip`].
use std::collections::BTreeMap;

/// Which dummy variant a pod has opted into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DummyType {
    /// Backend dummy — an `axum` HTTP server with `/health`, `/openapi.json`, etc.
    Backend,
    /// Frontend dummy — nginx serving a templated HTML page.
    Frontend,
}

/// Fully-resolved configuration extracted from a pod's annotations.
///
/// `app_name` falls back to the pod name. `port` falls back to 8080
/// (backend) or 80 (frontend). `image_override` and `skip_containers`
/// are optional.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubbyConfig {
    pub dummy_type: DummyType,
    pub app_name: String,
    pub port: u16,
    pub image_override: Option<String>,
    pub skip_containers: Vec<String>,
}

/// What the webhook decided to do with a pod.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Mutate the pod using these settings.
    Inject(StubbyConfig),
    /// Leave the pod untouched (no `stubby.io/type`, or value is `off`/unknown).
    Skip,
}

/// Translate a pod's annotation map into a [`Decision`].
///
/// `pod_name` is only used as the fallback for `stubby.io/app-name`.
/// The function never returns an error — anything malformed (e.g. a
/// non-numeric port) silently falls back to its default.
pub fn parse_annotations(annotations: &BTreeMap<String, String>, pod_name: &str) -> Decision {
    let raw_type = match annotations.get("stubby.io/type") {
        Some(v) => v.as_str(),
        None => return Decision::Skip,
    };

    let dummy_type = match raw_type {
        "backend" => DummyType::Backend,
        "frontend" => DummyType::Frontend,
        _ => return Decision::Skip,
    };

    let app_name = annotations
        .get("stubby.io/app-name")
        .cloned()
        .unwrap_or_else(|| pod_name.to_string());

    let port = annotations
        .get("stubby.io/port")
        .and_then(|s| s.parse::<u16>().ok())
        .unwrap_or(match dummy_type {
            DummyType::Backend => 8080,
            DummyType::Frontend => 80,
        });

    let image_override = annotations.get("stubby.io/image-override").cloned();

    let skip_containers = annotations
        .get("stubby.io/skip-containers")
        .map(|csv| csv.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();

    Decision::Inject(StubbyConfig {
        dummy_type,
        app_name,
        port,
        image_override,
        skip_containers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ann(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn missing_type_skips() {
        let a = ann(&[]);
        assert_eq!(parse_annotations(&a, "pod"), Decision::Skip);
    }

    #[test]
    fn type_off_skips() {
        let a = ann(&[("stubby.io/type", "off")]);
        assert_eq!(parse_annotations(&a, "pod"), Decision::Skip);
    }

    #[test]
    fn invalid_type_skips() {
        let a = ann(&[("stubby.io/type", "worker")]);
        assert_eq!(parse_annotations(&a, "pod"), Decision::Skip);
    }

    #[test]
    fn type_backend_with_defaults() {
        let a = ann(&[("stubby.io/type", "backend")]);
        let got = parse_annotations(&a, "orders-api-7f");
        assert_eq!(
            got,
            Decision::Inject(StubbyConfig {
                dummy_type: DummyType::Backend,
                app_name: "orders-api-7f".to_string(),
                port: 8080,
                image_override: None,
                skip_containers: vec![],
            })
        );
    }

    #[test]
    fn type_frontend_with_defaults() {
        let a = ann(&[("stubby.io/type", "frontend")]);
        let got = parse_annotations(&a, "site");
        assert_eq!(
            got,
            Decision::Inject(StubbyConfig {
                dummy_type: DummyType::Frontend,
                app_name: "site".to_string(),
                port: 80,
                image_override: None,
                skip_containers: vec![],
            })
        );
    }

    #[test]
    fn port_override_valid() {
        let a = ann(&[("stubby.io/type", "backend"), ("stubby.io/port", "9090")]);
        let Decision::Inject(cfg) = parse_annotations(&a, "p") else {
            panic!()
        };
        assert_eq!(cfg.port, 9090);
    }

    #[test]
    fn port_override_invalid_falls_back_to_default() {
        let a = ann(&[
            ("stubby.io/type", "backend"),
            ("stubby.io/port", "notanumber"),
        ]);
        let Decision::Inject(cfg) = parse_annotations(&a, "p") else {
            panic!()
        };
        assert_eq!(cfg.port, 8080);
    }

    #[test]
    fn port_override_out_of_range_falls_back_to_default() {
        let a = ann(&[("stubby.io/type", "backend"), ("stubby.io/port", "99999")]);
        let Decision::Inject(cfg) = parse_annotations(&a, "p") else {
            panic!()
        };
        assert_eq!(cfg.port, 8080);
    }

    #[test]
    fn app_name_override() {
        let a = ann(&[
            ("stubby.io/type", "frontend"),
            ("stubby.io/app-name", "Orders"),
        ]);
        let Decision::Inject(cfg) = parse_annotations(&a, "any") else {
            panic!()
        };
        assert_eq!(cfg.app_name, "Orders");
    }

    #[test]
    fn image_override_passes_through() {
        let a = ann(&[
            ("stubby.io/type", "backend"),
            ("stubby.io/image-override", "ghcr.io/me/myimg:tag"),
        ]);
        let Decision::Inject(cfg) = parse_annotations(&a, "p") else {
            panic!()
        };
        assert_eq!(cfg.image_override.as_deref(), Some("ghcr.io/me/myimg:tag"));
    }

    #[test]
    fn skip_containers_csv_parsed() {
        let a = ann(&[
            ("stubby.io/type", "backend"),
            ("stubby.io/skip-containers", "sidecar, audit ,telemetry"),
        ]);
        let Decision::Inject(cfg) = parse_annotations(&a, "p") else {
            panic!()
        };
        assert_eq!(cfg.skip_containers, vec!["sidecar", "audit", "telemetry"]);
    }
}
