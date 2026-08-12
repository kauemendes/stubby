//! Annotation keys used by the controller and helpers for the
//! `original-image` map it stores on rescued pods.
use std::collections::BTreeMap;

/// Opt-in: only pods with this set to `"true"` are considered.
pub const AUTO_RESCUE: &str = "stubby.io/auto-rescue";
/// JSON object `{container_name: original_image}` recorded when stubbing.
pub const ORIGINAL_IMAGE: &str = "stubby.io/original-image";
/// RFC3339 timestamp of the most recent stub action (observability only).
pub const RESCUED_AT: &str = "stubby.io/rescued-at";
/// Dummy type hint, mirrors the webhook: `backend` (default) or `frontend`.
pub const TYPE: &str = "stubby.io/type";

/// Serialize the container→original-image map for the [`ORIGINAL_IMAGE`]
/// annotation. Deterministic (BTreeMap) so patches are stable.
pub fn encode_original_images(map: &BTreeMap<String, String>) -> String {
    serde_json::to_string(map).expect("BTreeMap<String,String> always serializes")
}

/// Parse the [`ORIGINAL_IMAGE`] annotation. Anything missing or malformed
/// yields an empty map — the controller then treats the pod as "not rescued".
pub fn decode_original_images(raw: Option<&String>) -> BTreeMap<String, String> {
    raw.and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn roundtrips_original_images() {
        let mut m = BTreeMap::new();
        m.insert("app".to_string(), "ghcr.io/acme/app:v1".to_string());
        m.insert("worker".to_string(), "ghcr.io/acme/worker:v1".to_string());
        let encoded = encode_original_images(&m);
        let decoded = decode_original_images(Some(&encoded));
        assert_eq!(decoded, m);
    }

    #[test]
    fn decode_missing_or_garbage_is_empty() {
        assert!(decode_original_images(None).is_empty());
        assert!(decode_original_images(Some(&"not json".to_string())).is_empty());
    }
}
