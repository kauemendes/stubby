//! Minimal OCI image-reference parser (registry / repository / tag-or-digest).
//!
//! Follows Docker's defaulting rules closely enough for availability checks:
//! bare names default to `docker.io`, single-segment names get the `library/`
//! prefix, and a missing tag defaults to `latest`.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRef {
    pub registry: String,
    pub repository: String,
    /// Tag (`v1`) or digest (`sha256:...`).
    pub reference: String,
}

impl ImageRef {
    pub fn parse(image: &str) -> Self {
        let (maybe_registry, rest) = match image.split_once('/') {
            Some((first, rest)) if is_registry(first) => {
                (Some(first.to_string()), rest.to_string())
            }
            _ => (None, image.to_string()),
        };
        let registry = maybe_registry.unwrap_or_else(|| "docker.io".to_string());

        // Split reference: prefer digest (`@`), else tag (`:` after the last `/`).
        let (name, reference) = if let Some((n, d)) = rest.split_once('@') {
            (n.to_string(), d.to_string())
        } else if let Some(idx) = rest.rfind(':') {
            (rest[..idx].to_string(), rest[idx + 1..].to_string())
        } else {
            (rest.clone(), "latest".to_string())
        };

        let repository = if registry == "docker.io" && !name.contains('/') {
            format!("library/{name}")
        } else {
            name
        };

        Self {
            registry,
            repository,
            reference,
        }
    }
}

/// A leading path segment is a registry if it looks like a host: contains a
/// dot, a colon (port), or is exactly `localhost`.
fn is_registry(segment: &str) -> bool {
    segment == "localhost" || segment.contains('.') || segment.contains(':')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_ref() {
        let r = ImageRef::parse("ghcr.io/acme/app:v1.2.3");
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "acme/app");
        assert_eq!(r.reference, "v1.2.3");
    }

    #[test]
    fn defaults_registry_and_tag() {
        let r = ImageRef::parse("nginx");
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.repository, "library/nginx");
        assert_eq!(r.reference, "latest");
    }

    #[test]
    fn library_expansion_only_for_docker_io_single_segment() {
        let r = ImageRef::parse("acme/app:1");
        assert_eq!(r.registry, "docker.io");
        assert_eq!(r.repository, "acme/app");
        assert_eq!(r.reference, "1");
    }

    #[test]
    fn handles_digest() {
        let r = ImageRef::parse("ghcr.io/acme/app@sha256:abc");
        assert_eq!(r.registry, "ghcr.io");
        assert_eq!(r.repository, "acme/app");
        assert_eq!(r.reference, "sha256:abc");
    }

    #[test]
    fn registry_detected_by_dot_or_port_or_localhost() {
        assert_eq!(
            ImageRef::parse("localhost:5000/app:1").registry,
            "localhost:5000"
        );
        assert_eq!(
            ImageRef::parse("registry:5000/x/app:1").registry,
            "registry:5000"
        );
    }
}
