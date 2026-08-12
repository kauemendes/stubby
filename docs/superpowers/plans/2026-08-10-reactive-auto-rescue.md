# Reactive auto-rescue controller — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship an optional, experimental controller that stubs pods stuck in `ImagePullBackOff` (opt-in via `stubby.io/auto-rescue`) and reverts them in place once the real image appears in the registry.

**Architecture:** A new `crates/controller` binary uses `kube-rs`'s `Controller` to reconcile pods annotated `stubby.io/auto-rescue: "true"`. It patches only the pod's `image` field (the sole mutable container field on a live pod), records the original image in an annotation, and on a periodic requeue checks the registry (using the pod's `imagePullSecrets`) to decide when to revert. It never touches the Deployment, so it doesn't fight GitOps. Shipped behind `controller.enabled` (default false).

**Tech Stack:** Rust, `kube` 0.96, `k8s-openapi` 0.23 (v1_30), `oci-client` 0.14, `axum` (metrics endpoint), `metrics` + `metrics-exporter-prometheus`, `tokio`, `tracing`. Helm for packaging, kind + a local OCI registry for e2e.

Full design: `docs/superpowers/specs/2026-08-10-reactive-auto-rescue-design.md`.

---

## File Structure

```
crates/controller/
  Cargo.toml                 # crate manifest
  src/
    main.rs                  # kube client, Controller wiring, metrics server, shutdown
    lib.rs                   # module declarations + shared re-exports
    config.rs                # env-sourced config (dummy images, check interval)
    annotations.rs           # annotation keys + original-image (de)serialization
    decision.rs              # PURE state machine: pod -> stub/revert candidates
    imageref.rs              # PURE image-reference parsing
    auth.rs                  # PURE pull-secret (dockerconfigjson) -> credentials
    registry.rs              # registry availability check (network, uses auth.rs)
    reconcile.rs             # ties decision + registry + kube patches together
    observability.rs         # metrics init/render + counters/gauge
docker/controller.Dockerfile # distroless non-root image
charts/stubby/
  values.yaml                # + controller.* block
  templates/controller-deployment.yaml
  templates/controller-rbac.yaml
  tests/controller_test.yaml # helm unittest
test/e2e/cases/defect-none-autorescue.sh   # e2e (named 'autorescue' not a defect)
examples/autorescue.yaml
```

Root `Cargo.toml` gains `crates/controller` in `members` and new `[workspace.dependencies]`.

---

## Phase 1 — Crate skeleton, config, annotations

### Task 1: Add the controller crate to the workspace

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Create: `crates/controller/Cargo.toml`
- Create: `crates/controller/src/lib.rs`
- Create: `crates/controller/src/main.rs`

- [ ] **Step 1: Add member + workspace deps**

Edit root `Cargo.toml` `members` to include `"crates/controller"`, and add to `[workspace.dependencies]`:

```toml
kube = { version = "0.96", default-features = false, features = ["client", "runtime", "rustls-tls"] }
oci-client = { version = "0.14", default-features = false, features = ["rustls-tls"] }
futures = "0.3"
base64 = "0.22"
```

- [ ] **Step 2: Create the crate manifest**

`crates/controller/Cargo.toml`:

```toml
[package]
name = "stubby-controller"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lib]
path = "src/lib.rs"

[[bin]]
name = "stubby-controller"
path = "src/main.rs"

[dependencies]
kube.workspace = true
k8s-openapi.workspace = true
oci-client.workspace = true
tokio.workspace = true
futures.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
anyhow.workspace = true
thiserror.workspace = true
base64.workspace = true
axum.workspace = true
metrics.workspace = true
metrics-exporter-prometheus.workspace = true

[dev-dependencies]
```

- [ ] **Step 3: Stub lib.rs and main.rs so the crate compiles**

`crates/controller/src/lib.rs`:

```rust
//! `stubby-controller` — reactive auto-rescue controller (experimental).
//!
//! Watches pods annotated `stubby.io/auto-rescue: "true"`; stubs those stuck
//! in `ImagePullBackOff` and reverts them once the real image is available.
pub mod annotations;
pub mod auth;
pub mod config;
pub mod decision;
pub mod imageref;
pub mod observability;
pub mod reconcile;
pub mod registry;
```

`crates/controller/src/main.rs` (temporary, replaced in Task 12):

```rust
fn main() {
    println!("stubby-controller placeholder");
}
```

Create empty module files so `lib.rs` compiles:

```bash
for m in annotations auth config decision imageref observability reconcile registry; do
  echo "//! placeholder" > "crates/controller/src/$m.rs"
done
```

- [ ] **Step 4: Verify it builds**

Run: `cargo build -p stubby-controller`
Expected: compiles (downloads kube/oci-client on first run).

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml Cargo.lock crates/controller
git commit -m "feat(controller): scaffold stubby-controller crate"
```

---

### Task 2: Annotation keys and original-image (de)serialization

**Files:**
- Modify: `crates/controller/src/annotations.rs`

- [ ] **Step 1: Write the failing test**

Put at the bottom of `crates/controller/src/annotations.rs`:

```rust
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
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p stubby-controller annotations`
Expected: FAIL (functions not defined).

- [ ] **Step 3: Implement**

Replace the placeholder in `crates/controller/src/annotations.rs`:

```rust
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p stubby-controller annotations`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/controller/src/annotations.rs
git commit -m "feat(controller): annotation keys and original-image codec"
```

---

### Task 3: Config from environment

**Files:**
- Modify: `crates/controller/src/config.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_check_interval_seconds() {
        assert_eq!(parse_interval_secs(Some("30")), std::time::Duration::from_secs(30));
        assert_eq!(parse_interval_secs(Some("bad")), std::time::Duration::from_secs(60));
        assert_eq!(parse_interval_secs(None), std::time::Duration::from_secs(60));
    }
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p stubby-controller config`
Expected: FAIL.

- [ ] **Step 3: Implement**

`crates/controller/src/config.rs`:

```rust
//! Process configuration, sourced from environment variables set by the chart.
use anyhow::Context;
use std::time::Duration;

/// Dummy image refs (same env names as the webhook) plus the registry
/// re-check interval.
#[derive(Debug, Clone)]
pub struct ControllerConfig {
    pub backend_image: String,
    pub frontend_image: String,
    pub check_interval: Duration,
}

impl ControllerConfig {
    pub fn from_env() -> anyhow::Result<Self> {
        Ok(Self {
            backend_image: required("STUBBY_IMAGE_BACKEND")?,
            frontend_image: required("STUBBY_IMAGE_FRONTEND")?,
            check_interval: parse_interval_secs(std::env::var("STUBBY_CHECK_INTERVAL_SECS").ok().as_deref()),
        })
    }
}

fn required(key: &str) -> anyhow::Result<String> {
    let v = std::env::var(key).with_context(|| format!("{key} must be set"))?;
    let v = v.trim().to_string();
    anyhow::ensure!(!v.is_empty(), "{key} must not be empty");
    Ok(v)
}

/// Parse a whole-seconds interval; fall back to 60s on absent/garbage input.
pub fn parse_interval_secs(raw: Option<&str>) -> Duration {
    raw.and_then(|s| s.trim().parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(60))
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p stubby-controller config`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/controller/src/config.rs
git commit -m "feat(controller): environment configuration"
```

---

## Phase 2 — Pure logic (image refs, decision state machine)

### Task 4: Image-reference parsing

**Files:**
- Modify: `crates/controller/src/imageref.rs`

- [ ] **Step 1: Write the failing test**

```rust
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
        assert_eq!(ImageRef::parse("localhost:5000/app:1").registry, "localhost:5000");
        assert_eq!(ImageRef::parse("registry:5000/x/app:1").registry, "registry:5000");
    }
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p stubby-controller imageref`
Expected: FAIL.

- [ ] **Step 3: Implement**

`crates/controller/src/imageref.rs`:

```rust
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
            Some((first, rest)) if is_registry(first) => (Some(first.to_string()), rest.to_string()),
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

        Self { registry, repository, reference }
    }
}

/// A leading path segment is a registry if it looks like a host: contains a
/// dot, a colon (port), or is exactly `localhost`.
fn is_registry(segment: &str) -> bool {
    segment == "localhost" || segment.contains('.') || segment.contains(':')
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p stubby-controller imageref`
Expected: PASS (5 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/controller/src/imageref.rs
git commit -m "feat(controller): image-reference parser"
```

---

### Task 5: Decision state machine (pure)

**Files:**
- Modify: `crates/controller/src/decision.rs`

- [ ] **Step 1: Write the failing test**

```rust
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
        // app already rescued (in original-image), istio-proxy is a sidecar,
        // worker is running -> nothing to stub.
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
        assert_eq!(orig.get("app").map(String::as_str), Some("ghcr.io/acme/app:v1"));
    }
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p stubby-controller decision`
Expected: FAIL.

- [ ] **Step 3: Implement**

`crates/controller/src/decision.rs`:

```rust
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
        // The image the pod is trying (and failing) to pull is the original.
        let original_image = cs.image.clone();
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p stubby-controller decision`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/controller/src/decision.rs
git commit -m "feat(controller): pure stub/revert decision logic"
```

---

## Phase 3 — Registry availability

### Task 6: Pull-secret credential assembly (pure)

**Files:**
- Modify: `crates/controller/src/auth.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;

    fn dockercfg(host: &str, user: &str, pass: &str) -> Vec<u8> {
        let auth = base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
        serde_json::to_vec(&serde_json::json!({
            "auths": { host: { "auth": auth } }
        })).unwrap()
    }

    #[test]
    fn extracts_credentials_for_host() {
        let creds = credentials_for("ghcr.io", &[dockercfg("ghcr.io", "u", "p")]);
        assert_eq!(creds, Some(("u".to_string(), "p".to_string())));
    }

    #[test]
    fn matches_docker_io_aliases() {
        // Docker writes the index host; a check for docker.io must match it.
        let cfg = dockercfg("https://index.docker.io/v1/", "u", "p");
        let creds = credentials_for("docker.io", &[cfg]);
        assert_eq!(creds, Some(("u".to_string(), "p".to_string())));
    }

    #[test]
    fn none_when_no_match_or_garbage() {
        assert_eq!(credentials_for("ghcr.io", &[b"not json".to_vec()]), None);
        assert_eq!(credentials_for("ghcr.io", &[dockercfg("other.io", "u", "p")]), None);
    }
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p stubby-controller auth`
Expected: FAIL.

- [ ] **Step 3: Implement**

`crates/controller/src/auth.rs`:

```rust
//! Turn `kubernetes.io/dockerconfigjson` secret payloads into a
//! `(username, password)` credential for a given registry host. Pure — the
//! caller fetches the secret bytes from the API.
use base64::Engine;
use serde_json::Value;

/// Find credentials for `host` across the provided dockerconfigjson blobs.
/// Returns the first match; `None` if none apply or all are malformed.
pub fn credentials_for(host: &str, dockerconfigs: &[Vec<u8>]) -> Option<(String, String)> {
    for raw in dockerconfigs {
        let Ok(cfg): Result<Value, _> = serde_json::from_slice(raw) else {
            continue;
        };
        let Some(auths) = cfg.get("auths").and_then(Value::as_object) else {
            continue;
        };
        for (entry_host, entry) in auths {
            if !host_matches(host, entry_host) {
                continue;
            }
            if let Some(creds) = decode_entry(entry) {
                return Some(creds);
            }
        }
    }
    None
}

/// A registry host matches a dockerconfig key if they're equal, or both are
/// Docker Hub under any of its aliases.
fn host_matches(host: &str, entry_host: &str) -> bool {
    if host == entry_host {
        return true;
    }
    let docker_aliases = [
        "docker.io",
        "index.docker.io",
        "https://index.docker.io/v1/",
        "registry-1.docker.io",
    ];
    docker_aliases.contains(&host) && docker_aliases.contains(&entry_host)
}

/// Decode either an `auth` (base64 `user:pass`) or explicit `username`/`password`.
fn decode_entry(entry: &Value) -> Option<(String, String)> {
    if let Some(auth) = entry.get("auth").and_then(Value::as_str) {
        let decoded = base64::engine::general_purpose::STANDARD.decode(auth).ok()?;
        let s = String::from_utf8(decoded).ok()?;
        let (u, p) = s.split_once(':')?;
        return Some((u.to_string(), p.to_string()));
    }
    let u = entry.get("username").and_then(Value::as_str)?;
    let p = entry.get("password").and_then(Value::as_str)?;
    Some((u.to_string(), p.to_string()))
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p stubby-controller auth`
Expected: PASS (3 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/controller/src/auth.rs
git commit -m "feat(controller): dockerconfigjson credential assembly"
```

---

### Task 7: Registry availability check

**Files:**
- Modify: `crates/controller/src/registry.rs`

This layer does network I/O, so it is covered by the e2e (Task 16), not a unit test. Keep it small and total (never panics; all errors → "not available").

- [ ] **Step 1: Implement**

`crates/controller/src/registry.rs`:

```rust
//! "Is this image pullable now?" — a manifest HEAD against the registry using
//! credentials assembled from the pod's pull secrets. Any error is reported as
//! "not available yet" so the controller retries rather than crashing.
use crate::auth::credentials_for;
use crate::imageref::ImageRef;
use oci_client::{secrets::RegistryAuth, Client, Reference};

/// Returns `true` only if the image's manifest is fetchable right now.
pub async fn is_available(image: &str, dockerconfigs: &[Vec<u8>]) -> bool {
    let parsed = ImageRef::parse(image);
    let auth = match credentials_for(&parsed.registry, dockerconfigs) {
        Some((u, p)) => RegistryAuth::Basic(u, p),
        None => RegistryAuth::Anonymous,
    };

    let reference: Reference = match image.parse() {
        Ok(r) => r,
        Err(e) => {
            tracing::debug!(image, err=%e, "unparseable image reference");
            return false;
        }
    };

    let client = Client::new(oci_client::client::ClientConfig::default());
    match client.fetch_manifest_digest(&reference, &auth).await {
        Ok(_digest) => true,
        Err(e) => {
            tracing::debug!(image, err=%e, "image not available yet");
            false
        }
    }
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p stubby-controller`
Expected: compiles. (If `oci-client` 0.14's method/type names differ — e.g. `fetch_manifest_digest` signature — adjust to the installed version; the intent is "fetch the manifest digest, success = available".)

- [ ] **Step 3: Commit**

```bash
git add crates/controller/src/registry.rs
git commit -m "feat(controller): registry availability check via oci-client"
```

---

## Phase 4 — Observability, reconcile, main

### Task 8: Metrics

**Files:**
- Modify: `crates/controller/src/observability.rs`

- [ ] **Step 1: Write the failing test**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_contains_series_after_actions() {
        init_metrics();
        record_stub();
        record_revert();
        let out = render();
        assert!(out.contains("stubby_rescue_actions_total"));
    }
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p stubby-controller observability`
Expected: FAIL.

- [ ] **Step 3: Implement**

`crates/controller/src/observability.rs`:

```rust
//! Prometheus metrics for the controller. Mirrors the webhook's approach:
//! a global recorder plus a `render()` for the `/metrics` handler.
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

/// Install the global recorder. Safe to call more than once (tests do).
pub fn init_metrics() {
    let _ = HANDLE.get_or_init(|| {
        PrometheusBuilder::new()
            .install_recorder()
            .expect("install prometheus recorder")
    });
}

pub fn render() -> String {
    HANDLE.get().map(|h| h.render()).unwrap_or_default()
}

/// A pod was stubbed: bump the action counter and the live gauge.
pub fn record_stub() {
    metrics::counter!("stubby_rescue_actions_total", "action" => "stub").increment(1);
    metrics::gauge!("stubby_rescued_pods").increment(1.0);
}

/// A pod was reverted: bump the counter and drop the live gauge.
pub fn record_revert() {
    metrics::counter!("stubby_rescue_actions_total", "action" => "revert").increment(1);
    metrics::gauge!("stubby_rescued_pods").decrement(1.0);
}
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p stubby-controller observability`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/controller/src/observability.rs
git commit -m "feat(controller): prometheus metrics"
```

---

### Task 9: Reconcile — context, patch helpers, and the reconcile function

**Files:**
- Modify: `crates/controller/src/reconcile.rs`

Reconcile combines the pure decision with kube patches and the registry check. The pure branch-selection (`what to do given stub candidates, rescued originals, and per-image availability`) is unit-tested; the actual API calls are covered by e2e.

- [ ] **Step 1: Write the failing test for the pure planner**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn plans_stub_when_candidates_present() {
        let plan = plan_action(
            &[("app".to_string(), "orig:1".to_string(), "dummy:1".to_string())],
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
        assert!(matches!(plan_action(&[], &originals, &avail), Plan::Nothing));

        avail.insert("orig-w:1".to_string(), true);
        assert!(matches!(plan_action(&[], &originals, &avail), Plan::Revert(_)));
    }

    #[test]
    fn stub_takes_priority_over_revert() {
        let mut originals = BTreeMap::new();
        originals.insert("app".to_string(), "orig:1".to_string());
        let mut avail = BTreeMap::new();
        avail.insert("orig:1".to_string(), true);
        let plan = plan_action(
            &[("new".to_string(), "orig2:1".to_string(), "dummy:1".to_string())],
            &originals,
            &avail,
        );
        assert!(matches!(plan, Plan::Stub(_)));
    }
}
```

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p stubby-controller reconcile`
Expected: FAIL.

- [ ] **Step 3: Implement the planner + the reconcile glue**

`crates/controller/src/reconcile.rs`:

```rust
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
        Plan::Revert(rescued.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
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

    // Only pay for registry checks when there's something to potentially revert
    // and nothing new to stub.
    let availability = if candidates.is_empty() && !rescued.is_empty() {
        let dockerconfigs = pull_secrets(&pods, &ctx.client, &ns, &pod).await;
        let mut map = BTreeMap::new();
        for img in rescued.values() {
            map.insert(img.clone(), registry::is_available(img, &dockerconfigs).await);
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
    let mut originals = rescued_originals(pod);
    let mut containers = Vec::new();
    for (cname, orig, dummy) in items {
        originals.insert(cname.clone(), orig.clone());
        containers.push(serde_json::json!({"name": cname, "image": dummy}));
    }
    let patch = serde_json::json!({
        "metadata": {"annotations": {
            ORIGINAL_IMAGE: encode_original_images(&originals),
            RESCUED_AT: now_rfc3339(),
        }},
        "spec": {"containers": containers}
    });
    pods.patch(name, &PatchParams::default(), &Patch::Strategic(patch)).await?;
    Ok(())
}

/// Restore each container's original image and delete the controller
/// annotations (null in a merge patch removes the key).
async fn apply_revert(
    pods: &Api<Pod>,
    name: &str,
    items: &[(String, String)],
) -> Result<(), Error> {
    let containers: Vec<_> = items
        .iter()
        .map(|(cname, orig)| serde_json::json!({"name": cname, "image": orig}))
        .collect();
    let patch = serde_json::json!({
        "metadata": {"annotations": {
            ORIGINAL_IMAGE: serde_json::Value::Null,
            RESCUED_AT: serde_json::Value::Null,
        }},
        "spec": {"containers": containers}
    });
    pods.patch(name, &PatchParams::default(), &Patch::Strategic(patch)).await?;
    Ok(())
}

/// Read every referenced pull secret's `.dockerconfigjson` payload. Missing or
/// unreadable secrets are skipped (best-effort; the check just stays "not
/// available").
async fn pull_secrets(
    _pods: &Api<Pod>,
    client: &kube::Client,
    ns: &str,
    pod: &Pod,
) -> Vec<Vec<u8>> {
    let secrets: Api<Secret> = Api::namespaced(client.clone(), ns);
    let mut names: Vec<String> = pod
        .spec
        .as_ref()
        .and_then(|s| s.image_pull_secrets.as_ref())
        .map(|refs| refs.iter().filter_map(|r| r.name.clone()).collect())
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
    // Avoid extra deps: format the UNIX epoch seconds as a coarse timestamp.
    // Purely observational; not parsed anywhere.
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("@{secs}")
}
```

- [ ] **Step 4: Run tests + build**

Run: `cargo test -p stubby-controller reconcile && cargo build -p stubby-controller`
Expected: PASS + compiles. (If `image_pull_secrets` field access or `Secret.data` byte type differs in the installed k8s-openapi, adjust; `ByteString`'s inner bytes are `.0`.)

- [ ] **Step 5: Commit**

```bash
git add crates/controller/src/reconcile.rs
git commit -m "feat(controller): reconcile planner and stub/revert patches"
```

---

### Task 10: Cross-crate sidecar-prefix parity test

**Files:**
- Modify: `crates/controller/src/decision.rs`

- [ ] **Step 1: Add a parity test**

Append to the `tests` module in `decision.rs`:

```rust
    #[test]
    fn sidecar_prefixes_match_webhook() {
        // Keep this list identical to crates/webhook/src/patch.rs
        // ALWAYS_SKIP_PREFIXES. If the webhook adds a prefix, add it here too.
        assert_eq!(
            ALWAYS_SKIP_PREFIXES,
            &["istio-", "linkerd-", "vault-", "cilium-"]
        );
    }
```

- [ ] **Step 2: Run**

Run: `cargo test -p stubby-controller decision`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add crates/controller/src/decision.rs
git commit -m "test(controller): lock sidecar prefix parity with the webhook"
```

---

### Task 11: main.rs — client, Controller, metrics server, shutdown

**Files:**
- Modify: `crates/controller/src/main.rs`

Covered by e2e, not unit tests.

- [ ] **Step 1: Implement**

`crates/controller/src/main.rs`:

```rust
use anyhow::Result;
use futures::StreamExt;
use k8s_openapi::api::core::v1::Pod;
use kube::api::Api;
use kube::runtime::controller::Controller;
use kube::runtime::watcher;
use kube::Client;
use std::sync::Arc;
use stubby_controller::annotations::AUTO_RESCUE;
use stubby_controller::config::ControllerConfig;
use stubby_controller::observability;
use stubby_controller::reconcile::{error_policy, reconcile, Ctx};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    observability::init_metrics();
    let cfg = ControllerConfig::from_env()?;
    let client = Client::try_default().await?;
    let pods: Api<Pod> = Api::all(client.clone());
    let ctx = Arc::new(Ctx { client: client.clone(), cfg });

    // Serve /metrics on a plain HTTP port (no TLS; scrape target only).
    tokio::spawn(serve_metrics());

    tracing::info!("stubby-controller starting");
    Controller::new(pods, watcher::Config::default())
        .run(reconcile, error_policy, ctx)
        .for_each(|res| async move {
            match res {
                Ok(o) => tracing::debug!(?o, "reconciled"),
                Err(e) => tracing::warn!(error = %e, "reconcile stream error"),
            }
        })
        .await;
    Ok(())
}

async fn serve_metrics() {
    use axum::{routing::get, Router};
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route(
            "/metrics",
            get(|| async {
                (
                    [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4")],
                    observability::render(),
                )
            }),
        );
    let listener = match tokio::net::TcpListener::bind("0.0.0.0:9090").await {
        Ok(l) => l,
        Err(e) => {
            tracing::error!(err = %e, "failed to bind metrics port");
            return;
        }
    };
    let _ = axum::serve(listener, app).await;
}
```

Note: the reconciler ignores pods without the annotation cheaply — but to avoid reconciling every pod in the cluster, filter early inside `reconcile` by returning `Action::await_change()` when `AUTO_RESCUE` is not `"true"`. Add at the very top of `reconcile` in `reconcile.rs`:

```rust
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
```

(and remove the now-unused `AUTO_RESCUE` import from `main.rs` if the compiler flags it).

- [ ] **Step 2: Build + fmt + clippy**

Run:
```bash
cargo build -p stubby-controller
cargo fmt --all
cargo clippy -p stubby-controller --all-targets -- -D warnings
```
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add crates/controller/src/main.rs crates/controller/src/reconcile.rs
git commit -m "feat(controller): main loop with kube Controller and metrics server"
```

---

## Phase 5 — Packaging

### Task 12: Controller Dockerfile

**Files:**
- Create: `docker/controller.Dockerfile`

- [ ] **Step 1: Write it (mirror the webhook image)**

```dockerfile
# syntax=docker/dockerfile:1.7
FROM rust:1.85-bookworm AS builder
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p stubby-controller && \
    cp /src/target/release/stubby-controller /tmp/app

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /tmp/app /usr/local/bin/app
USER nonroot
EXPOSE 9090
ENTRYPOINT ["/usr/local/bin/app"]
```

- [ ] **Step 2: Build it**

Run: `DOCKER_BUILDKIT=1 docker build -f docker/controller.Dockerfile -t local/stubby-controller:dev .`
Expected: image builds.

- [ ] **Step 3: Commit**

```bash
git add docker/controller.Dockerfile
git commit -m "build(controller): distroless non-root image"
```

---

### Task 13: Helm — values and templates

**Files:**
- Modify: `charts/stubby/values.yaml`
- Create: `charts/stubby/templates/controller-rbac.yaml`
- Create: `charts/stubby/templates/controller-deployment.yaml`

- [ ] **Step 1: Add values**

Append to `charts/stubby/values.yaml`:

```yaml
# Experimental reactive auto-rescue controller. Disabled by default.
controller:
  enabled: false
  replicaCount: 1
  image:
    repository: ghcr.io/kauemendes/stubby-controller
    tag: ""              # defaults to .Chart.AppVersion when empty
    pullPolicy: IfNotPresent
  checkIntervalSeconds: 60
  resources:
    requests:
      cpu: 20m
      memory: 32Mi
    limits:
      cpu: 100m
      memory: 64Mi
```

- [ ] **Step 2: RBAC template**

`charts/stubby/templates/controller-rbac.yaml`:

```yaml
{{- if .Values.controller.enabled }}
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "stubby.fullname" . }}-controller
  labels:
    {{- include "stubby.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: {{ include "stubby.fullname" . }}-controller
  labels:
    {{- include "stubby.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
rules:
  - apiGroups: [""]
    resources: ["pods"]
    verbs: ["get", "list", "watch", "patch"]
  - apiGroups: [""]
    resources: ["events"]
    verbs: ["create", "patch"]
  # Needed to read imagePullSecrets for registry availability checks.
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: {{ include "stubby.fullname" . }}-controller
  labels:
    {{- include "stubby.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: {{ include "stubby.fullname" . }}-controller
subjects:
  - kind: ServiceAccount
    name: {{ include "stubby.fullname" . }}-controller
    namespace: {{ .Release.Namespace }}
{{- end }}
```

(If the chart's `_helpers.tpl` uses different helper names than `stubby.fullname`/`stubby.labels`, match the existing ones — check `charts/stubby/templates/_helpers.tpl` and the existing `deployment.yaml`.)

- [ ] **Step 3: Deployment template**

`charts/stubby/templates/controller-deployment.yaml`:

```yaml
{{- if .Values.controller.enabled }}
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "stubby.fullname" . }}-controller
  labels:
    {{- include "stubby.labels" . | nindent 4 }}
    app.kubernetes.io/component: controller
spec:
  replicas: {{ .Values.controller.replicaCount }}
  selector:
    matchLabels:
      {{- include "stubby.selectorLabels" . | nindent 6 }}
      app.kubernetes.io/component: controller
  template:
    metadata:
      labels:
        {{- include "stubby.selectorLabels" . | nindent 8 }}
        app.kubernetes.io/component: controller
    spec:
      serviceAccountName: {{ include "stubby.fullname" . }}-controller
      securityContext:
        runAsNonRoot: true
        seccompProfile:
          type: RuntimeDefault
      containers:
        - name: controller
          image: "{{ .Values.controller.image.repository }}:{{ .Values.controller.image.tag | default .Chart.AppVersion }}"
          imagePullPolicy: {{ .Values.controller.image.pullPolicy }}
          securityContext:
            readOnlyRootFilesystem: true
            allowPrivilegeEscalation: false
            capabilities:
              drop: ["ALL"]
          env:
            - name: STUBBY_IMAGE_BACKEND
              value: {{ .Values.dummyImages.backend | quote }}
            - name: STUBBY_IMAGE_FRONTEND
              value: {{ .Values.dummyImages.frontend | quote }}
            - name: STUBBY_CHECK_INTERVAL_SECS
              value: {{ .Values.controller.checkIntervalSeconds | quote }}
            - name: RUST_LOG
              value: {{ .Values.logLevel | default "info" | quote }}
          ports:
            - name: metrics
              containerPort: 9090
          livenessProbe:
            httpGet: { path: /healthz, port: metrics }
          readinessProbe:
            httpGet: { path: /readyz, port: metrics }
          resources:
            {{- toYaml .Values.controller.resources | nindent 12 }}
{{- end }}
```

- [ ] **Step 4: Lint**

Run:
```bash
helm lint charts/stubby
helm template charts/stubby --set controller.enabled=true | grep -c "kind: Deployment"
```
Expected: lint passes; two Deployments rendered (webhook + controller).

- [ ] **Step 5: Commit**

```bash
git add charts/stubby/values.yaml charts/stubby/templates/controller-rbac.yaml charts/stubby/templates/controller-deployment.yaml
git commit -m "feat(chart): optional controller deployment + RBAC"
```

---

### Task 14: Helm unittest for the controller

**Files:**
- Create: `charts/stubby/tests/controller_test.yaml`

- [ ] **Step 1: Write tests**

```yaml
suite: controller
templates:
  - templates/controller-deployment.yaml
  - templates/controller-rbac.yaml
tests:
  - it: renders nothing when disabled
    set:
      controller.enabled: false
    asserts:
      - hasDocuments:
          count: 0

  - it: renders deployment and rbac when enabled
    set:
      controller.enabled: true
    asserts:
      - hasDocuments:
          count: 1
        template: templates/controller-deployment.yaml
      - containsDocument:
          kind: ClusterRole
          apiVersion: rbac.authorization.k8s.io/v1
        template: templates/controller-rbac.yaml

  - it: runs restricted and reads the check interval
    set:
      controller.enabled: true
      controller.checkIntervalSeconds: 15
    template: templates/controller-deployment.yaml
    asserts:
      - equal:
          path: spec.template.spec.containers[0].securityContext.readOnlyRootFilesystem
          value: true
      - contains:
          path: spec.template.spec.containers[0].env
          content:
            name: STUBBY_CHECK_INTERVAL_SECS
            value: "15"
```

- [ ] **Step 2: Run**

Run: `helm unittest charts/stubby`
Expected: all suites pass (existing 3 + controller).

- [ ] **Step 3: Commit**

```bash
git add charts/stubby/tests/controller_test.yaml
git commit -m "test(chart): helm unittest for the controller"
```

---

## Phase 6 — Docs and e2e

### Task 15: Documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/annotations.md`
- Modify: `docs/troubleshooting.md`
- Modify: `CHANGELOG.md`

- [ ] **Step 1: README — add an "Auto-rescue (experimental)" subsection**

After the "What gets injected" section, add:

```markdown
## Auto-rescue (experimental)

Instead of the proactive opt-in, you can let stubby react: point your
Deployment at the **real** image from day one and annotate the pod
`stubby.io/auto-rescue: "true"`. While the tag doesn't exist the pod would
sit in `ImagePullBackOff`; the optional controller swaps the image for a
dummy in place, then reverts once the tag is published to the registry.

Enable it with `--set controller.enabled=true`. It patches only the pod's
`image` (never the Deployment), so it does not conflict with GitOps.

**Experimental limitation:** only `image` is mutable on a live pod, so the
dummy inherits the pod's existing `ports`/`probes`/`env`. It listens on
`8080` (backend) / `80` (frontend), or on `STUBBY_PORT` if you declare
that env var on the container yourself. If a probe targets a different
port, declare `STUBBY_PORT` to match.
```

- [ ] **Step 2: annotations.md — document `stubby.io/auto-rescue`**

Add a row to the table:

```markdown
| `stubby.io/auto-rescue` | `true` \| `false` | `false` | Experimental. Requires the controller. When the container is stuck in `ImagePullBackOff`, swap it to a dummy in place and revert once the real image is available. `stubby.io/type`/`port` act as hints. |
```

- [ ] **Step 3: troubleshooting.md — add a runbook**

```markdown
## Auto-rescued pod isn't reverting

The controller re-checks the registry every `controller.checkIntervalSeconds`.
If a pod stays on the dummy after the real image is published:

1. Confirm the controller is running: `kubectl get deploy -l app.kubernetes.io/component=controller -A`.
2. Check its logs for `image not available yet` — usually a pull-secret or
   registry-auth problem. The controller reads the pod's `imagePullSecrets`.
3. Confirm the recorded original: `kubectl get pod <pod> -o jsonpath='{.metadata.annotations.stubby\.io/original-image}'`.
```

- [ ] **Step 4: CHANGELOG — under `[Unreleased]` → `Added`**

```markdown
- **Experimental auto-rescue controller** (`controller.enabled`, off by
  default) — reacts to `ImagePullBackOff` on pods annotated
  `stubby.io/auto-rescue: "true"`, swaps the image for a dummy in place,
  and reverts once the real image is published to the registry. In-place
  patch only (GitOps-safe); registry checks use the pod's imagePullSecrets.
```

- [ ] **Step 5: Commit**

```bash
git add README.md docs/annotations.md docs/troubleshooting.md CHANGELOG.md
git commit -m "docs: document the experimental auto-rescue controller"
```

---

### Task 16: e2e — stub then revert against a local registry

**Files:**
- Create: `examples/autorescue.yaml`
- Create: `test/e2e/cases/autorescue.sh`
- Modify: `test/e2e/run.sh`

- [ ] **Step 1: Example manifest**

`examples/autorescue.yaml`:

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: reactive-api
spec:
  replicas: 1
  selector:
    matchLabels:
      app: reactive-api
  template:
    metadata:
      labels:
        app: reactive-api
      annotations:
        stubby.io/auto-rescue: "true"
        stubby.io/type: backend
    spec:
      containers:
        - name: api
          # Points at the local registry tag published only mid-test.
          image: localhost:5000/reactive-api:v1
---
apiVersion: v1
kind: Service
metadata:
  name: reactive-api
spec:
  selector:
    app: reactive-api
  ports:
    - port: 8080
      targetPort: 8080
```

- [ ] **Step 2: Wire a local registry + build/enable controller in run.sh**

In `test/e2e/run.sh`, add the controller image alongside the others:

```bash
CONTROLLER_IMG=local/stubby-controller:e2e
```
build/load it next to the webhook image:
```bash
docker build -f docker/controller.Dockerfile -t "$CONTROLLER_IMG" .
kind load docker-image "$CONTROLLER_IMG" --name "$CLUSTER"
```
and enable the controller in the `helm upgrade`:
```bash
      --set controller.enabled=true \
      --set controller.image.repository=local/stubby-controller \
      --set controller.image.tag=e2e \
      --set controller.image.pullPolicy=Never \
      --set controller.checkIntervalSeconds=10 \
```

Add a kind-reachable registry. Simplest within the existing single-node
kind: run a registry container on the kind network and load images into it.
Add near the cluster-creation block:

```bash
echo "==> ensuring local registry"
REG_NAME=kind-registry
REG_PORT=5000
if ! docker inspect "$REG_NAME" >/dev/null 2>&1; then
  docker run -d --restart=always -p "127.0.0.1:${REG_PORT}:5000" --name "$REG_NAME" registry:2
fi
docker network connect kind "$REG_NAME" 2>/dev/null || true
```

so `localhost:5000` on the host pushes, and configure the node to treat it
as the registry. (If this proves fiddly on the target Docker, an accepted
fallback is to run the registry container attached to the kind network and
reference it by `kind-registry:5000` in the manifest; document whichever
works in the case script.)

- [ ] **Step 3: The case script**

`test/e2e/cases/autorescue.sh`:

```bash
#!/usr/bin/env bash
# Auto-rescue: deploy a pod pointing at a tag that doesn't exist yet ->
# controller stubs it -> pod Running on the dummy. Then push the real image
# under that tag -> controller reverts -> pod runs the original image.
set -euo pipefail
NS=default
BACKEND_IMG=local/stubby-dummy-backend:e2e
REG=localhost:5000
REAL=reactive-api:v1

dump() {
  rc=$?
  echo "==> autorescue.sh diagnostics (exit $rc)" >&2
  kubectl get -n "$NS" deploy,pod -o wide >&2 || true
  kubectl describe -n "$NS" pod -l app=reactive-api >&2 || true
  kubectl logs -n stubby-system -l app.kubernetes.io/component=controller --tail=100 >&2 || true
  exit $rc
}
trap dump ERR

# Ensure the tag is absent to start (ignore errors if registry has no delete).
kubectl apply -n "$NS" -f examples/autorescue.yaml

# 1) Controller should stub the pull-failing pod within a few check intervals.
for i in $(seq 1 24); do
  IMG=$(kubectl get -n "$NS" pod -l app=reactive-api -o jsonpath='{.items[0].spec.containers[0].image}' 2>/dev/null || true)
  [[ "$IMG" == "$BACKEND_IMG" ]] && break
  sleep 5
done
kubectl rollout status -n "$NS" deploy/reactive-api --timeout=120s
POD=$(kubectl get -n "$NS" pod -l app=reactive-api -o jsonpath='{.items[0].metadata.name}')
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')
[[ "$IMG" == "$BACKEND_IMG" ]] || { echo "FAIL: not stubbed; image=$IMG" >&2; exit 1; }
echo "autorescue: stubbed OK"

# 2) Publish the real tag: retag the dummy backend as the "real" image.
docker tag "$BACKEND_IMG" "$REG/$REAL"
docker push "$REG/$REAL"

# 3) Controller should revert once the tag is available.
for i in $(seq 1 24); do
  IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}' 2>/dev/null || true)
  [[ "$IMG" == "$REG/$REAL" ]] && break
  sleep 5
done
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')
[[ "$IMG" == "$REG/$REAL" ]] || { echo "FAIL: not reverted; image=$IMG" >&2; exit 1; }

echo "autorescue (stub then revert) OK"
```

Make it executable: `chmod +x test/e2e/cases/autorescue.sh`.

- [ ] **Step 4: Run the full e2e**

Run: `bash test/e2e/run.sh`
Expected: all cases pass, including `autorescue`. (Iterate on the registry
wiring until the node can pull `localhost:5000`/`kind-registry:5000`.)

- [ ] **Step 5: Commit**

```bash
git add examples/autorescue.yaml test/e2e/cases/autorescue.sh test/e2e/run.sh
git commit -m "test(e2e): auto-rescue stub-then-revert against a local registry"
```

---

## Phase 7 — Ship

### Task 17: Full green + open PR

- [ ] **Step 1: Run every gate**

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
helm lint charts/stubby
helm unittest charts/stubby
bash test/e2e/run.sh
```
Expected: all green.

- [ ] **Step 2: Push and open the PR**

```bash
git push -u origin feat/reactive-auto-rescue
gh pr create --title "feat: experimental reactive auto-rescue controller" \
  --body-file <(cat <<'BODY'
Implements the design in docs/superpowers/specs/2026-08-10-reactive-auto-rescue-design.md.

Optional controller (controller.enabled, default off) that stubs pods stuck
in ImagePullBackOff (opt-in stubby.io/auto-rescue) and reverts in place once
the real image is published. In-place image patch only (GitOps-safe);
registry checks use the pod's imagePullSecrets. Experimental: inherits the
pod's ports/probes/env, escape hatch is declaring STUBBY_PORT.

Green: cargo fmt/clippy/test, helm lint/unittest, e2e (stub then revert).
No chart version bump.
BODY
)
```

---

## Self-Review

**Spec coverage:**
- Trigger (ImagePullBackOff) → Task 5. Opt-in flag → Tasks 2, 11. Registry-check revert → Tasks 6, 7, 9. In-place patch → Task 9. Separate optional Deployment + RBAC → Tasks 13, 14. Port limitation + escape hatch → Tasks 9 (inherits), 15 (docs). Observability → Task 8. Unit tests → Tasks 2–10. e2e → Task 16. Docs + CHANGELOG → Task 15. No chart bump → Task 17. All covered.

**Placeholder scan:** the only intentionally open item is the e2e local-registry wiring (Task 16 Step 2), which lists the concrete approach plus a documented fallback — not a "TODO in code". All code steps contain real code.

**Type consistency:** `RescueConfig{backend_image,frontend_image}`, `StubAction{name,original_image,dummy_image}`, `Plan::{Stub,Revert,Nothing}`, `Ctx{client,cfg}`, and the annotation constants (`AUTO_RESCUE`, `ORIGINAL_IMAGE`, `RESCUED_AT`, `TYPE`) are used consistently across `decision.rs`, `reconcile.rs`, and `main.rs`. `encode_original_images`/`decode_original_images` signatures match their call sites.

**Known API-version caveats flagged inline:** `oci-client` manifest method name (Task 7), `Secret.data` byte access `.0` and `image_pull_secrets` (Task 9), and chart helper names (Task 13) — each step tells the implementer to reconcile with the installed versions.
