//! `admission.k8s.io/v1` AdmissionReview types and the top-level handler.
//!
//! [`handle`] is the pure decision function: AdmissionReview + image refs →
//! AdmissionReview with an optional JSONPatch. Decode failures still return
//! `allowed: true` (with an explanatory [`AdmissionStatus`] message) so the
//! admission contract holds and the API server's `failurePolicy: Ignore`
//! has a chance to skip us.
//!
//! Only the fields used by `pods.stubby.io` are modelled — this is not a
//! general-purpose AdmissionReview client.
use crate::annotation::{parse_annotations, Decision};
use crate::config::ImageRefs;
use crate::patch::build_patch;
use base64::Engine;
use k8s_openapi::api::core::v1::Pod;
use serde::{Deserialize, Serialize};

/// Top-level `admission.k8s.io/v1` envelope.
///
/// `request` is set on the way in, `response` on the way out. We never
/// emit both at once.
#[derive(Debug, Deserialize, Serialize)]
pub struct AdmissionReview {
    #[serde(rename = "apiVersion")]
    pub api_version: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<AdmissionRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<AdmissionResponse>,
}

/// The half of an AdmissionReview the API server sends us.
///
/// `object` arrives as raw JSON so the handler can decide whether to decode
/// it as a Pod (and what to do if decoding fails).
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionRequest {
    pub uid: String,
    pub object: serde_json::Value,
}

/// The half of an AdmissionReview we send back.
#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionResponse {
    pub uid: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_type: Option<String>,
    /// Optional v1 status — used to surface diagnostics (e.g. a failed Pod
    /// decode) back to the API server so they appear in audit logs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AdmissionStatus>,
}

/// Optional diagnostic envelope attached to an [`AdmissionResponse`].
#[derive(Debug, Deserialize, Serialize)]
pub struct AdmissionStatus {
    pub message: String,
}

/// Decide what to do with a single AdmissionReview.
///
/// Always returns `allowed: true`. The shape of the response varies:
/// - missing/skipped pod → empty allow.
/// - malformed Pod object → allow + [`AdmissionStatus::message`].
/// - matched pod → allow + base64-encoded JSONPatch.
pub fn handle(review: AdmissionReview, imgs: &ImageRefs) -> AdmissionReview {
    let req = match review.request {
        Some(r) => r,
        None => return reply(None, true, None),
    };
    let uid = req.uid.clone();
    let pod: Pod = match serde_json::from_value(req.object) {
        Ok(p) => p,
        Err(e) => {
            return reply_status(
                Some(uid),
                true,
                format!("stubby: could not decode Pod object: {e}"),
            );
        }
    };
    let annotations = pod.metadata.annotations.clone().unwrap_or_default();
    let pod_name = pod.metadata.name.clone().unwrap_or_else(|| "pod".into());
    let cfg = match parse_annotations(&annotations, &pod_name) {
        Decision::Inject(c) => c,
        Decision::Skip => return reply(Some(uid), true, None),
    };
    let ops = build_patch(&pod, &cfg, imgs);
    if ops.is_empty() {
        return reply(Some(uid), true, None);
    }
    let json_patch =
        serde_json::to_string(&ops).expect("PatchOperation serialization is infallible");
    let b64 = base64::engine::general_purpose::STANDARD.encode(json_patch);
    reply(Some(uid), true, Some(b64))
}

fn reply(uid: Option<String>, allowed: bool, patch_b64: Option<String>) -> AdmissionReview {
    AdmissionReview {
        api_version: "admission.k8s.io/v1".into(),
        kind: "AdmissionReview".into(),
        request: None,
        response: Some(AdmissionResponse {
            uid: uid.unwrap_or_default(),
            allowed,
            patch_type: patch_b64.as_ref().map(|_| "JSONPatch".into()),
            patch: patch_b64,
            status: None,
        }),
    }
}

fn reply_status(uid: Option<String>, allowed: bool, message: String) -> AdmissionReview {
    AdmissionReview {
        api_version: "admission.k8s.io/v1".into(),
        kind: "AdmissionReview".into(),
        request: None,
        response: Some(AdmissionResponse {
            uid: uid.unwrap_or_default(),
            allowed,
            patch: None,
            patch_type: None,
            status: Some(AdmissionStatus { message }),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::json;

    fn refs() -> ImageRefs {
        ImageRefs {
            backend: "ghcr.io/test/be:1".into(),
            frontend: "ghcr.io/test/fe:1".into(),
        }
    }

    fn review(pod_obj: serde_json::Value, uid: &str) -> AdmissionReview {
        AdmissionReview {
            api_version: "admission.k8s.io/v1".into(),
            kind: "AdmissionReview".into(),
            request: Some(AdmissionRequest {
                uid: uid.into(),
                object: pod_obj,
            }),
            response: None,
        }
    }

    #[test]
    fn skips_when_no_annotation() {
        let pod = json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {"name":"p"},
            "spec": {"containers":[{"name":"app","image":"a:1"}]}
        });
        let r = handle(review(pod, "u1"), &refs());
        let resp = r.response.unwrap();
        assert_eq!(resp.uid, "u1");
        assert!(resp.allowed);
        assert!(resp.patch.is_none());
        assert!(resp.patch_type.is_none());
    }

    #[test]
    fn decode_failure_surfaces_status_message() {
        // `spec` must be an object — passing a string forces serde to reject
        // the value (an unknown top-level field would be silently ignored).
        let bogus = json!({"metadata": {"name":"p"}, "spec": "not-an-object"});
        let r = handle(review(bogus, "u-bad"), &refs());
        let resp = r.response.unwrap();
        assert_eq!(resp.uid, "u-bad");
        assert!(resp.allowed, "decode failures must not block creation");
        assert!(resp.patch.is_none());
        let msg = resp
            .status
            .expect("decode failure must include status")
            .message;
        assert!(
            msg.starts_with("stubby:"),
            "status message should be prefixed with 'stubby:', got {msg}"
        );
    }

    #[test]
    fn patches_when_backend_annotated() {
        let pod = json!({
            "apiVersion": "v1", "kind": "Pod",
            "metadata": {
                "name":"p",
                "annotations": {"stubby.io/type":"backend"}
            },
            "spec": {"containers":[{"name":"app","image":"a:1"}]}
        });
        let r = handle(review(pod, "u2"), &refs());
        let resp = r.response.unwrap();
        assert_eq!(resp.uid, "u2");
        assert!(resp.allowed);
        let b64 = resp.patch.expect("expected patch");
        let raw = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        let arr = parsed.as_array().unwrap();
        assert!(arr.iter().any(|op| op["op"] == "replace"
            && op["path"] == "/spec/containers/0/image"
            && op["value"] == "ghcr.io/test/be:1"));
        assert_eq!(resp.patch_type.as_deref(), Some("JSONPatch"));
    }
}
