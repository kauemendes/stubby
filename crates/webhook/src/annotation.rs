use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DummyType {
    Backend,
    Frontend,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StubbyConfig {
    pub dummy_type: DummyType,
    pub app_name: String,
    pub port: u16,
    pub image_override: Option<String>,
    pub skip_containers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Inject(StubbyConfig),
    Skip,
}

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
}
