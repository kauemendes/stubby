# stubby v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build `stubby`, a Kubernetes Mutating Admission Webhook in Rust (`kube-rs`) that swaps pod container images for dummy backend/frontend images when pods carry a `stubby.io/type` annotation, distributed as a Helm chart with images published to GHCR.

**Architecture:** Cargo workspace with three crates (`webhook`, `dummy-backend`, `dummy-frontend`). Webhook implements `AdmissionReview/v1`, returns a JSONPatch that rewrites `image`, `ports`, probes, `command`, `args`, `env`. Dummy images are distroless Rust binary (backend) and `nginx:alpine` (frontend). Helm chart bundles `MutatingWebhookConfiguration`, RBAC, Deployment, Service, and TLS bootstrap (cert-manager or self-signed Job).

**Tech Stack:**
- **Rust** 1.85.0 (pinned via `rust-toolchain.toml`; required minimum because some transitive dependencies use Rust edition 2024)
- **axum 0.7** — HTTP framework (both webhook and dummy-backend)
- **kube 0.96** + **k8s-openapi 0.23** (feature `v1_30`) — Kubernetes API types
- **tokio 1.x** — async runtime
- **serde / serde_json** — AdmissionReview & JSONPatch serialization
- **json-patch 2.x** — JSONPatch types
- **tracing / tracing-subscriber** — structured logging
- **metrics / metrics-exporter-prometheus** — Prometheus metrics
- **rustls / rustls-pemfile** — TLS without OpenSSL dependency
- **insta** — snapshot tests for dummy-frontend HTML generation
- **helm** + **helm-unittest** — chart and chart tests
- **kind** — local k8s for integration tests
- **GitHub Actions** + **docker buildx** + **cosign** — CI/CD

**Reference spec:** `docs/superpowers/specs/2026-05-20-stubby-design.md`

**Conventions:**
- Each task follows red → green → refactor. Steps separate "write failing test", "see it fail", "implement", "see it pass", "commit".
- Commit messages use Conventional Commits (`feat:`, `test:`, `chore:`, `docs:`, `ci:`, `refactor:`).
- A task is **done** only when `cargo test --workspace`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo fmt --check` all pass.
- Always run `cargo fmt` before commit.

---

## File Structure (target end state)

```
stubby/
├── Cargo.toml                       # workspace
├── Cargo.lock
├── rust-toolchain.toml              # pin stable
├── .gitignore
├── LICENSE                          # MIT
├── README.md                        # public intro
│
├── crates/
│   ├── webhook/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs              # bin entrypoint; config + server bind
│   │   │   ├── lib.rs               # exposes modules for integration tests
│   │   │   ├── config.rs            # ImageRefs, ServerConfig (from env)
│   │   │   ├── annotation.rs        # parse_annotations() — annotation → Decision
│   │   │   ├── patch.rs             # build_patch() — Pod + Config → Vec<PatchOp>
│   │   │   ├── admission.rs         # handle_admission() — orchestrates parse + patch
│   │   │   ├── server.rs            # axum app, routes, TLS bind
│   │   │   ├── observability.rs     # tracing init + Prometheus recorder
│   │   │   └── error.rs             # WebhookError enum
│   │   └── tests/
│   │       └── fixtures/            # JSON AdmissionReview samples
│   │
│   ├── dummy-backend/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── lib.rs
│   │   │   ├── routes.rs            # health/ready/metrics/openapi/docs + catch-all
│   │   │   └── openapi.rs           # static OpenAPI doc generator
│   │   ├── assets/
│   │   │   └── swagger-ui/          # checked-in swagger-ui-dist (or downloaded in build.rs)
│   │   └── tests/
│   │       └── handlers.rs
│   │
│   └── dummy-frontend/
│       ├── Cargo.toml               # only build.rs + tests; no runtime binary
│       ├── build.rs                 # renders templates → OUT_DIR/dist/
│       ├── templates/
│       │   ├── index.html.tmpl
│       │   └── style.css
│       ├── nginx/
│       │   ├── default.conf
│       │   └── entrypoint.sh
│       └── tests/
│           └── render.rs            # insta snapshots
│
├── docker/
│   ├── webhook.Dockerfile
│   ├── dummy-backend.Dockerfile
│   └── dummy-frontend.Dockerfile
│
├── charts/stubby/
│   ├── Chart.yaml
│   ├── values.yaml
│   ├── values.schema.json
│   ├── templates/
│   │   ├── _helpers.tpl
│   │   ├── deployment.yaml
│   │   ├── service.yaml
│   │   ├── serviceaccount.yaml
│   │   ├── rbac.yaml
│   │   ├── mutatingwebhookconfiguration.yaml
│   │   ├── tls-cert-manager.yaml
│   │   └── tls-self-signed-job.yaml
│   └── tests/                       # helm unittest
│       ├── deployment_test.yaml
│       ├── mwc_test.yaml
│       └── tls_test.yaml
│
├── examples/
│   ├── backend.yaml
│   ├── frontend.yaml
│   └── off.yaml
│
├── test/
│   ├── kind-config.yaml
│   └── e2e/
│       ├── run.sh
│       └── cases/
│           ├── backend.sh
│           └── frontend.sh
│
├── docs/
│   ├── installation.md
│   ├── annotations.md
│   ├── troubleshooting.md
│   └── superpowers/
│       ├── specs/2026-05-20-stubby-design.md
│       └── plans/2026-05-20-stubby-implementation.md   # this file
│
└── .github/
    └── workflows/
        ├── ci.yaml
        └── release.yaml
```

---

## Core Type Contracts (used across tasks)

These types are referenced by multiple tasks below. Defined in `crates/webhook` and reused via `lib.rs` re-exports.

```rust
// crates/webhook/src/annotation.rs
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DummyType { Backend, Frontend }

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

// crates/webhook/src/config.rs
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRefs {
    pub backend: String,
    pub frontend: String,
}

// crates/webhook/src/patch.rs
pub use json_patch::{Patch, PatchOperation};
```

Built-in sidecar name prefixes always skipped (defined once in `patch.rs`):

```rust
pub const ALWAYS_SKIP_PREFIXES: &[&str] = &["istio-", "linkerd-", "vault-", "cilium-"];
```

---

# Phase 0 — Bootstrap

## Task 0.1: Initialize Cargo workspace and toolchain

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `.gitignore`
- Create: `LICENSE`
- Create: `README.md`

- [ ] **Step 1: Write `rust-toolchain.toml`**

```toml
[toolchain]
channel = "1.85.0"
components = ["rustfmt", "clippy"]
profile = "minimal"
```

- [ ] **Step 2: Write workspace `Cargo.toml`**

```toml
[workspace]
resolver = "2"
members = [
    "crates/webhook",
    "crates/dummy-backend",
    "crates/dummy-frontend",
]

[workspace.package]
edition = "2021"
rust-version = "1.85"
license = "MIT"
repository = "https://github.com/kauemendes/stubby"
authors = ["Kauê Mendes"]

[workspace.dependencies]
axum = "0.7"
tokio = { version = "1", features = ["full"] }
tower = "0.5"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter", "json"] }
k8s-openapi = { version = "0.23", features = ["v1_30"] }
json-patch = "2"
anyhow = "1"
metrics = "0.23"
metrics-exporter-prometheus = "0.15"
rustls = "0.23"
rustls-pemfile = "2"
tokio-rustls = "0.26"
insta = { version = "1", features = ["json"] }

[profile.release]
strip = "symbols"
lto = "thin"
codegen-units = 1
```

- [ ] **Step 3: Write `.gitignore`**

```
/target
**/*.rs.bk
Cargo.lock.bak
.DS_Store
*.swp
*.swo
.idea/
.vscode/
```

We **do** commit `Cargo.lock` — all three crates produce binaries.

- [ ] **Step 4: Write `LICENSE` (MIT)**

```
MIT License

Copyright (c) 2026 Kauê Mendes

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

- [ ] **Step 5: Write minimal `README.md`**

```markdown
# stubby

Kubernetes Mutating Admission Webhook that injects dummy backend/frontend
images into pods carrying a `stubby.io/type` annotation. Useful as a
placeholder while the real image isn't built yet.

> Status: under construction. See `docs/superpowers/specs/2026-05-20-stubby-design.md`
> for the design and `docs/superpowers/plans/` for the implementation plan.

## Quick start (target UX)

```yaml
# orders.yaml
apiVersion: apps/v1
kind: Deployment
metadata: { name: orders-api }
spec:
  replicas: 1
  selector: { matchLabels: { app: orders-api } }
  template:
    metadata:
      labels: { app: orders-api }
      annotations:
        stubby.io/type: backend
    spec:
      containers:
        - name: orders
          image: ghcr.io/example/orders-api:latest
```

## License

MIT
```

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml rust-toolchain.toml .gitignore LICENSE README.md
git commit -m "chore: bootstrap cargo workspace and project metadata"
```

---

## Task 0.2: Create empty crate skeletons (so workspace resolves)

**Files:**
- Create: `crates/webhook/Cargo.toml`
- Create: `crates/webhook/src/lib.rs`
- Create: `crates/webhook/src/main.rs`
- Create: `crates/dummy-backend/Cargo.toml`
- Create: `crates/dummy-backend/src/main.rs`
- Create: `crates/dummy-backend/src/lib.rs`
- Create: `crates/dummy-frontend/Cargo.toml`
- Create: `crates/dummy-frontend/build.rs`
- Create: `crates/dummy-frontend/src/lib.rs`

- [ ] **Step 1: Write `crates/webhook/Cargo.toml`**

```toml
[package]
name = "stubby-webhook"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[lib]
path = "src/lib.rs"

[[bin]]
name = "stubby-webhook"
path = "src/main.rs"

[dependencies]
axum.workspace = true
tokio.workspace = true
tower.workspace = true
serde.workspace = true
serde_json.workspace = true
thiserror.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
k8s-openapi.workspace = true
json-patch.workspace = true
anyhow.workspace = true
metrics.workspace = true
metrics-exporter-prometheus.workspace = true
rustls.workspace = true
rustls-pemfile.workspace = true
tokio-rustls.workspace = true
```

- [ ] **Step 2: Write `crates/webhook/src/lib.rs` (empty placeholder)**

```rust
// Re-exports added in later tasks.
```

- [ ] **Step 3: Write `crates/webhook/src/main.rs` (empty placeholder)**

```rust
fn main() {
    eprintln!("stubby-webhook: not yet implemented");
}
```

- [ ] **Step 4: Write `crates/dummy-backend/Cargo.toml`**

```toml
[package]
name = "stubby-dummy-backend"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true

[lib]
path = "src/lib.rs"

[[bin]]
name = "stubby-dummy-backend"
path = "src/main.rs"

[dependencies]
axum.workspace = true
tokio.workspace = true
tower.workspace = true
serde.workspace = true
serde_json.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
anyhow.workspace = true
```

- [ ] **Step 5: Write `crates/dummy-backend/src/main.rs` (empty placeholder)**

```rust
fn main() {
    eprintln!("stubby-dummy-backend: not yet implemented");
}
```

- [ ] **Step 5b: Write `crates/dummy-backend/src/lib.rs` (empty placeholder)**

The Cargo.toml declares `[lib] path = "src/lib.rs"` so the file must exist for the crate to compile. Later tasks (e.g. dummy backend routes) will re-export modules from here.

```rust
// Re-exports added in later tasks.
```

- [ ] **Step 6: Write `crates/dummy-frontend/Cargo.toml`**

`render_index` returns `String` (no fallible IO), so the runtime crate intentionally carries no `anyhow` dependency. `build.rs` may grow fallible logic later (template generation in Task 7.1), so `anyhow` stays in `[build-dependencies]`.

```toml
[package]
name = "stubby-dummy-frontend"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
build = "build.rs"

[lib]
path = "src/lib.rs"

[dependencies]
# (runtime currently has no fallible IO; render_index returns String)

[build-dependencies]
anyhow.workspace = true

[dev-dependencies]
insta.workspace = true
```

- [ ] **Step 7: Write `crates/dummy-frontend/build.rs` and `src/lib.rs` (placeholders)**

```rust
// crates/dummy-frontend/build.rs
fn main() {
    println!("cargo:rerun-if-changed=templates");
}
```

```rust
// crates/dummy-frontend/src/lib.rs
pub fn render_index(app_name: &str) -> String {
    format!("<!doctype html><title>{}</title>", app_name)
}
```

(Real template-driven render comes in Task 5.1; this is just enough for the workspace to compile.)

- [ ] **Step 8: Verify workspace compiles**

Run: `cargo check --workspace`
Expected: success, three packages compile.

- [ ] **Step 9: Commit**

```bash
git add crates/
git commit -m "chore: add empty crate skeletons (webhook, dummy-backend, dummy-frontend)"
```

---

# Phase 1 — Webhook: Annotation Parsing

## Task 1.1: Parse `stubby.io/type` annotation

**Files:**
- Create: `crates/webhook/src/annotation.rs`
- Modify: `crates/webhook/src/lib.rs`

- [ ] **Step 1: Wire module into `lib.rs`**

Edit `crates/webhook/src/lib.rs` to:

```rust
pub mod annotation;
```

- [ ] **Step 2: Write failing tests in `crates/webhook/src/annotation.rs`**

```rust
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

pub fn parse_annotations(
    _annotations: &BTreeMap<String, String>,
    _pod_name: &str,
) -> Decision {
    unimplemented!()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ann(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
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
```

- [ ] **Step 3: Run tests; verify they fail (compile or panic)**

Run: `cargo test -p stubby-webhook annotation::`
Expected: 5 tests, all fail at `unimplemented!()`.

- [ ] **Step 4: Implement `parse_annotations` to make tests pass**

Replace the stub with:

```rust
pub fn parse_annotations(
    annotations: &BTreeMap<String, String>,
    pod_name: &str,
) -> Decision {
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
```

- [ ] **Step 5: Run tests; verify they pass**

Run: `cargo test -p stubby-webhook annotation::`
Expected: 5 passed.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/webhook/src/annotation.rs crates/webhook/src/lib.rs
git commit -m "feat(webhook): parse stubby.io annotations into Decision"
```

---

## Task 1.2: Cover overrides (port, app-name, image-override, skip-containers)

**Files:**
- Modify: `crates/webhook/src/annotation.rs`

- [ ] **Step 1: Add failing tests**

Append to `mod tests` in `annotation.rs`:

```rust
#[test]
fn port_override_valid() {
    let a = ann(&[("stubby.io/type", "backend"), ("stubby.io/port", "9090")]);
    let Decision::Inject(cfg) = parse_annotations(&a, "p") else { panic!() };
    assert_eq!(cfg.port, 9090);
}

#[test]
fn port_override_invalid_falls_back_to_default() {
    let a = ann(&[("stubby.io/type", "backend"), ("stubby.io/port", "notanumber")]);
    let Decision::Inject(cfg) = parse_annotations(&a, "p") else { panic!() };
    assert_eq!(cfg.port, 8080);
}

#[test]
fn port_override_out_of_range_falls_back_to_default() {
    let a = ann(&[("stubby.io/type", "backend"), ("stubby.io/port", "99999")]);
    let Decision::Inject(cfg) = parse_annotations(&a, "p") else { panic!() };
    assert_eq!(cfg.port, 8080);
}

#[test]
fn app_name_override() {
    let a = ann(&[("stubby.io/type", "frontend"), ("stubby.io/app-name", "Orders")]);
    let Decision::Inject(cfg) = parse_annotations(&a, "any") else { panic!() };
    assert_eq!(cfg.app_name, "Orders");
}

#[test]
fn image_override_passes_through() {
    let a = ann(&[
        ("stubby.io/type", "backend"),
        ("stubby.io/image-override", "ghcr.io/me/myimg:tag"),
    ]);
    let Decision::Inject(cfg) = parse_annotations(&a, "p") else { panic!() };
    assert_eq!(cfg.image_override.as_deref(), Some("ghcr.io/me/myimg:tag"));
}

#[test]
fn skip_containers_csv_parsed() {
    let a = ann(&[
        ("stubby.io/type", "backend"),
        ("stubby.io/skip-containers", "sidecar, audit ,telemetry"),
    ]);
    let Decision::Inject(cfg) = parse_annotations(&a, "p") else { panic!() };
    assert_eq!(cfg.skip_containers, vec!["sidecar", "audit", "telemetry"]);
}
```

- [ ] **Step 2: Run; verify only `port_override_out_of_range...` and `port_override_invalid...` fail (or none — depending on previous impl)**

Run: `cargo test -p stubby-webhook annotation::`
Expected: `port_override_out_of_range_falls_back_to_default` may fail (`u16::parse` already rejects 99999). Confirm — it should be: `99999` overflows `u16::MAX` (65535), so `parse::<u16>()` returns `Err`, and our `.ok()` makes it fall back. **All tests pass on first run**. If they all pass, that's still useful documentation. If any fail, fix.

- [ ] **Step 3: If any fail, refine `parse_annotations`**

(Likely no change needed.)

- [ ] **Step 4: Run tests; verify all pass**

Run: `cargo test -p stubby-webhook annotation::`
Expected: 11 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/webhook/src/annotation.rs
git commit -m "test(webhook): cover annotation overrides (port, app-name, image, skip-containers)"
```

---

# Phase 2 — Webhook: JSONPatch Generation

## Task 2.1: Patch builder skeleton + sidecar skip list

**Files:**
- Create: `crates/webhook/src/patch.rs`
- Create: `crates/webhook/src/config.rs`
- Modify: `crates/webhook/src/lib.rs`

- [ ] **Step 1: Wire modules into `lib.rs`**

```rust
pub mod annotation;
pub mod config;
pub mod patch;
```

- [ ] **Step 2: Write `crates/webhook/src/config.rs`**

Both `STUBBY_IMAGE_BACKEND` and `STUBBY_IMAGE_FRONTEND` are **required** and must point to fully qualified, pinned image refs (e.g. `ghcr.io/org/stubby-dummy-backend:v0.1.0`). The webhook fails fast at startup if either is missing or blank — better than silently injecting `:latest` into a cluster. The Helm chart's `deployment.yaml` (Task 8.2) sets these env vars from `values.yaml`.

```rust
use anyhow::Context;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageRefs {
    pub backend: String,
    pub frontend: String,
}

impl ImageRefs {
    /// Reads `STUBBY_IMAGE_BACKEND` and `STUBBY_IMAGE_FRONTEND` from the environment.
    /// Both are required and must be non-empty (whitespace is trimmed).
    /// Errors are surfaced to the binary's `main` so misconfigured deployments
    /// fail fast at startup instead of injecting a useless placeholder.
    pub fn from_env() -> anyhow::Result<Self> {
        let backend = required_env("STUBBY_IMAGE_BACKEND")?;
        let frontend = required_env("STUBBY_IMAGE_FRONTEND")?;
        Ok(Self { backend, frontend })
    }
}

fn required_env(key: &str) -> anyhow::Result<String> {
    let raw = std::env::var(key)
        .with_context(|| format!("{key} must be set to a fully qualified image ref"))?;
    let trimmed = raw.trim();
    anyhow::ensure!(!trimmed.is_empty(), "{key} must not be empty");
    Ok(trimmed.to_string())
}
```

Coverage for the env contract is deferred to the integration test in Task 9.2 (the kind cluster sets both env vars via the chart, so a regression here surfaces as a startup failure). We avoid unit tests that mutate process env because they race with the parallel default of `cargo test`.

- [ ] **Step 3: Write failing tests in `crates/webhook/src/patch.rs`**

```rust
use crate::annotation::{DummyType, StubbyConfig};
use crate::config::ImageRefs;
use json_patch::PatchOperation;
use k8s_openapi::api::core::v1::Pod;

pub const ALWAYS_SKIP_PREFIXES: &[&str] = &["istio-", "linkerd-", "vault-", "cilium-"];

pub fn build_patch(_pod: &Pod, _cfg: &StubbyConfig, _imgs: &ImageRefs) -> Vec<PatchOperation> {
    unimplemented!()
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
```

- [ ] **Step 4: Run tests; verify failure**

Run: `cargo test -p stubby-webhook patch::`
Expected: 1 test fails at `unimplemented!()`.

- [ ] **Step 5: Implement minimal `build_patch` for single backend container**

```rust
pub fn build_patch(pod: &Pod, cfg: &StubbyConfig, imgs: &ImageRefs) -> Vec<PatchOperation> {
    use json_patch::{ReplaceOperation, PatchOperation};
    use jsonptr::Pointer;

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
        let path = Pointer::parse(&format!("/spec/containers/{i}/image")).unwrap();
        ops.push(PatchOperation::Replace(ReplaceOperation {
            path,
            value: serde_json::Value::String(image),
        }));
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
```

No new direct dep — `jsonptr 0.4` is already available transitively via `json-patch 2.x`.

- [ ] **Step 6: Run tests; verify pass**

Run: `cargo test -p stubby-webhook patch::single_backend`
Expected: 1 passed.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add crates/webhook/
git commit -m "feat(webhook): replace container image via JSONPatch (single container)"
```

---

## Task 2.2: Patch ports, probes, command/args, env, resources

**Files:**
- Modify: `crates/webhook/src/patch.rs`

- [ ] **Step 1: Add failing tests**

Append to `mod tests`:

```rust
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

    // Helpers
    let find = |op: &str, path_suffix: &str| -> Option<serde_json::Value> {
        arr.iter().find(|x| x["op"] == op && x["path"].as_str().unwrap().ends_with(path_suffix)).cloned()
    };

    assert!(find("replace", "/image").is_some());
    assert!(find("replace", "/ports").is_some(), "ports not patched");
    let ports = find("replace", "/ports").unwrap()["value"].clone();
    assert_eq!(ports, json!([{"containerPort": 8080, "name": "http", "protocol": "TCP"}]));

    let lp = find("replace", "/livenessProbe").unwrap()["value"].clone();
    assert_eq!(lp["httpGet"]["path"], "/health");
    assert_eq!(lp["httpGet"]["port"], 8080);

    let rp = find("replace", "/readinessProbe").unwrap()["value"].clone();
    assert_eq!(rp["httpGet"]["path"], "/ready");
    assert_eq!(rp["httpGet"]["port"], 8080);

    assert!(find("remove", "/command").is_some());
    assert!(find("remove", "/args").is_some());

    // STUBBY_APP_NAME appended without removing existing env
    let add_env = arr.iter().find(|x|
        x["op"] == "add" && x["path"].as_str().unwrap().ends_with("/env/-")
    ).unwrap();
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
    let lp = arr.iter().find(|x|
        x["op"] == "replace" && x["path"].as_str().unwrap().ends_with("/livenessProbe")
    ).unwrap();
    assert_eq!(lp["value"]["httpGet"]["port"], 80);
}

#[test]
fn adds_default_resources_when_missing() {
    let pod = pod_with_containers(json!([{"name":"app","image":"orig:1"}]));
    let ops = build_patch(&pod, &backend_cfg(), &refs());
    let j = ops_to_json(&ops);
    let arr = j.as_array().unwrap();
    let r = arr.iter().find(|x|
        x["path"].as_str().unwrap().ends_with("/resources")
    ).unwrap();
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
    assert!(arr.iter().all(|x| !x["path"].as_str().unwrap().ends_with("/resources")));
}
```

- [ ] **Step 2: Run; verify they fail**

Run: `cargo test -p stubby-webhook patch::`
Expected: 4 new tests fail.

- [ ] **Step 3: Extend `build_patch`**

Replace the per-container loop body:

```rust
for (i, c) in containers.iter().enumerate() {
    if should_skip(&c.name, cfg) {
        continue;
    }
    let base = format!("/spec/containers/{i}");
    let image = chosen_image(cfg, imgs);

    // image
    ops.push(replace_op(&format!("{base}/image"), serde_json::Value::String(image)));

    // ports
    ops.push(replace_op(&format!("{base}/ports"), serde_json::json!([{
        "containerPort": cfg.port,
        "name": "http",
        "protocol": "TCP"
    }])));

    // probes
    ops.push(replace_op(&format!("{base}/livenessProbe"), serde_json::json!({
        "httpGet": {"path": "/health", "port": cfg.port},
        "initialDelaySeconds": 1,
        "periodSeconds": 10
    })));
    ops.push(replace_op(&format!("{base}/readinessProbe"), serde_json::json!({
        "httpGet": {"path": "/ready", "port": cfg.port},
        "initialDelaySeconds": 1,
        "periodSeconds": 5
    })));

    // command + args removed if present
    if c.command.is_some() {
        ops.push(remove_op(&format!("{base}/command")));
    }
    if c.args.is_some() {
        ops.push(remove_op(&format!("{base}/args")));
    }

    // env: append STUBBY_APP_NAME if env array exists; otherwise replace whole env
    let env_value = serde_json::json!({"name": "STUBBY_APP_NAME", "value": cfg.app_name});
    if c.env.is_some() {
        ops.push(add_op(&format!("{base}/env/-"), env_value));
    } else {
        ops.push(add_op(&format!("{base}/env"), serde_json::json!([env_value])));
    }

    // resources: only add defaults if missing
    if c.resources.is_none() {
        ops.push(add_op(&format!("{base}/resources"), serde_json::json!({
            "requests": {"cpu": "10m", "memory": "32Mi"},
            "limits":   {"cpu": "100m", "memory": "64Mi"}
        })));
    }
}
```

Add helper fns at module level. We construct `PatchOperation` values via `serde_json::from_value` to avoid naming the transitive `jsonptr` crate directly — Task 2.1 established this idiom because `json-patch 2.x` does not re-export `jsonptr` and depending on its name from outside is fragile.

```rust
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
```

- [ ] **Step 4: Run tests; verify pass**

Run: `cargo test -p stubby-webhook patch::`
Expected: 5 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/webhook/src/patch.rs
git commit -m "feat(webhook): patch ports, probes, env, resources, and remove command/args"
```

---

## Task 2.3: Multi-container pods and skip rules

**Files:**
- Modify: `crates/webhook/src/patch.rs`

- [ ] **Step 1: Add failing tests**

```rust
#[test]
fn multi_container_patches_each_non_sidecar() {
    let pod = pod_with_containers(json!([
        {"name": "app", "image": "orig:1"},
        {"name": "audit", "image": "audit:1"}
    ]));
    let ops = build_patch(&pod, &backend_cfg(), &refs());
    let j = ops_to_json(&ops);
    let arr = j.as_array().unwrap();
    let imgs: Vec<_> = arr.iter()
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
    let paths: Vec<_> = arr.iter()
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
    let paths: Vec<_> = arr.iter()
        .filter(|x| x["op"] == "replace" && x["path"].as_str().unwrap().ends_with("/image"))
        .map(|x| x["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths, vec!["/spec/containers/0/image"]);
}
```

- [ ] **Step 2: Run; verify pass**

Run: `cargo test -p stubby-webhook patch::`
Expected: 8 passed (existing implementation already handles these).

- [ ] **Step 3: Commit**

```bash
cargo fmt
git add crates/webhook/src/patch.rs
git commit -m "test(webhook): cover multi-container and sidecar skip rules"
```

---

## Task 2.4: Image override

**Files:**
- Modify: `crates/webhook/src/patch.rs`

- [ ] **Step 1: Add failing test**

```rust
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
    let img = arr.iter().find(|x|
        x["op"] == "replace" && x["path"].as_str().unwrap().ends_with("/image")
    ).unwrap()["value"].as_str().unwrap().to_string();
    assert_eq!(img, "ghcr.io/me/custom:dev");
}
```

- [ ] **Step 2: Run; verify pass**

Run: `cargo test -p stubby-webhook patch::image_override`
Expected: 1 passed (handled in Task 2.1).

- [ ] **Step 3: Commit**

```bash
git add crates/webhook/src/patch.rs
git commit -m "test(webhook): cover image-override annotation"
```

---

# Phase 3 — Webhook: AdmissionReview Handler

## Task 3.1: Handler that orchestrates parse + patch

**Files:**
- Create: `crates/webhook/src/admission.rs`
- Modify: `crates/webhook/src/lib.rs`

- [ ] **Step 1: Wire module**

In `lib.rs`:

```rust
pub mod admission;
```

- [ ] **Step 2: Write failing tests**

```rust
// crates/webhook/src/admission.rs
use crate::annotation::{parse_annotations, Decision};
use crate::config::ImageRefs;
use crate::patch::build_patch;
use base64::Engine;
use k8s_openapi::api::core::v1::Pod;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct AdmissionReview {
    pub api_version: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request: Option<AdmissionRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<AdmissionResponse>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionRequest {
    pub uid: String,
    pub object: serde_json::Value,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdmissionResponse {
    pub uid: String,
    pub allowed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_type: Option<String>,
}

pub fn handle(review: AdmissionReview, imgs: &ImageRefs) -> AdmissionReview {
    let req = match review.request {
        Some(r) => r,
        None => return reply(None, true, None),
    };
    let uid = req.uid.clone();
    let pod: Pod = match serde_json::from_value(req.object) {
        Ok(p) => p,
        Err(_) => return reply(Some(uid), true, None),
    };
    let annotations = pod.metadata.annotations.clone().unwrap_or_default();
    let pod_name = pod.metadata.name.clone().unwrap_or_else(|| "pod".into());
    let cfg = match parse_annotations(&annotations.into_iter().collect(), &pod_name) {
        Decision::Inject(c) => c,
        Decision::Skip => return reply(Some(uid), true, None),
    };
    let ops = build_patch(&pod, &cfg, imgs);
    if ops.is_empty() {
        return reply(Some(uid), true, None);
    }
    let json_patch = serde_json::to_string(&ops).unwrap();
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
        let raw = base64::engine::general_purpose::STANDARD.decode(b64).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        let arr = json.as_array().unwrap();
        assert!(arr.iter().any(|op|
            op["op"] == "replace"
            && op["path"] == "/spec/containers/0/image"
            && op["value"] == "ghcr.io/test/be:1"
        ));
        assert_eq!(resp.patch_type.as_deref(), Some("JSONPatch"));
    }
}
```

- [ ] **Step 3: Add deps**

In `crates/webhook/Cargo.toml`:

```toml
base64 = "0.22"
```

- [ ] **Step 4: Run; verify pass**

Run: `cargo test -p stubby-webhook admission::`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/webhook/
git commit -m "feat(webhook): AdmissionReview handler with JSONPatch base64 encoding"
```

---

# Phase 4 — Webhook: HTTP Server

## Task 4.1: axum server skeleton (HTTP only first, TLS in next task)

**Files:**
- Create: `crates/webhook/src/server.rs`
- Create: `crates/webhook/src/error.rs`
- Modify: `crates/webhook/src/lib.rs`
- Modify: `crates/webhook/src/main.rs`

- [ ] **Step 1: Wire modules**

`lib.rs`:

```rust
pub mod admission;
pub mod annotation;
pub mod config;
pub mod error;
pub mod patch;
pub mod server;
```

- [ ] **Step 2: Write `error.rs`**

```rust
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("invalid AdmissionReview body: {0}")]
    InvalidBody(#[from] serde_json::Error),
    #[error("TLS setup error: {0}")]
    Tls(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}
```

- [ ] **Step 3: Write failing test for `/mutate`**

```rust
// crates/webhook/src/server.rs
use crate::admission::{handle, AdmissionReview};
use crate::config::ImageRefs;
use axum::{
    extract::State, http::StatusCode, response::IntoResponse, routing::{get, post},
    Json, Router,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub image_refs: Arc<ImageRefs>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route("/mutate", post(mutate))
        .with_state(state)
}

async fn mutate(
    State(state): State<AppState>,
    Json(review): Json<AdmissionReview>,
) -> impl IntoResponse {
    let out = handle(review, state.image_refs.as_ref());
    (StatusCode::OK, Json(out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    fn state() -> AppState {
        AppState {
            image_refs: Arc::new(ImageRefs {
                backend: "ghcr.io/test/be:1".into(),
                frontend: "ghcr.io/test/fe:1".into(),
            }),
        }
    }

    #[tokio::test]
    async fn healthz_returns_200() {
        let app = router(state());
        let resp = app
            .oneshot(Request::builder().uri("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn mutate_returns_review_response() {
        let app = router(state());
        let body = serde_json::json!({
            "apiVersion": "admission.k8s.io/v1",
            "kind": "AdmissionReview",
            "request": {
                "uid": "abc",
                "object": {
                    "apiVersion":"v1","kind":"Pod",
                    "metadata":{"name":"p","annotations":{"stubby.io/type":"backend"}},
                    "spec":{"containers":[{"name":"app","image":"orig:1"}]}
                }
            }
        }).to_string();
        let resp = app
            .oneshot(Request::builder()
                .method("POST")
                .uri("/mutate")
                .header("content-type", "application/json")
                .body(Body::from(body)).unwrap())
            .await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["response"]["uid"], "abc");
        assert_eq!(v["response"]["allowed"], true);
        assert!(v["response"]["patch"].is_string());
    }
}
```

- [ ] **Step 4: Run; verify pass**

Run: `cargo test -p stubby-webhook server::`
Expected: 2 passed.

- [ ] **Step 5: Commit**

```bash
cargo fmt
git add crates/webhook/
git commit -m "feat(webhook): axum router with /healthz, /readyz, /mutate"
```

---

## Task 4.2: TLS bind in `main.rs`

**Files:**
- Modify: `crates/webhook/src/main.rs`
- Modify: `crates/webhook/Cargo.toml`

- [ ] **Step 1: Replace `main.rs`**

```rust
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::{Context, Result};
use stubby_webhook::config::ImageRefs;
use stubby_webhook::server::{router, AppState};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let addr = std::env::var("STUBBY_LISTEN").unwrap_or_else(|_| "0.0.0.0:8443".into());
    let cert_path = PathBuf::from(
        std::env::var("STUBBY_TLS_CERT").unwrap_or_else(|_| "/tls/tls.crt".into()),
    );
    let key_path = PathBuf::from(
        std::env::var("STUBBY_TLS_KEY").unwrap_or_else(|_| "/tls/tls.key".into()),
    );

    let state = AppState { image_refs: Arc::new(ImageRefs::from_env()?) };
    let app = router(state);

    let tls = build_tls_config(&cert_path, &key_path).context("TLS setup")?;
    let acceptor = TlsAcceptor::from(Arc::new(tls));
    let listener = TcpListener::bind(&addr).await?;
    info!(%addr, "stubby-webhook listening (TLS)");

    loop {
        let (stream, _) = listener.accept().await?;
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let tls_stream = match acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => { tracing::warn!(err=%e, "TLS accept failed"); return; }
            };
            if let Err(e) = axum::serve(
                tokio::io::BufStream::new(tls_stream),
                app.into_make_service(),
            ).await {
                tracing::warn!(err=%e, "serve loop ended");
            }
        });
    }
}

fn build_tls_config(cert: &std::path::Path, key: &std::path::Path) -> Result<rustls::ServerConfig> {
    let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(std::fs::File::open(cert)?))
        .collect::<std::io::Result<Vec<_>>>()?;
    let keys = rustls_pemfile::pkcs8_private_keys(&mut std::io::BufReader::new(std::fs::File::open(key)?))
        .collect::<std::io::Result<Vec<_>>>()?;
    let key = keys.into_iter().next().context("no PKCS8 key in file")?;
    let cfg = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, rustls::pki_types::PrivateKeyDer::Pkcs8(key))
        .map_err(|e| anyhow::anyhow!("rustls: {e}"))?;
    Ok(cfg)
}
```

> **Note:** axum 0.7 + rustls integration uses `axum-server` typically. If this main fails to compile due to `axum::serve` expecting a `TcpListener`, fall back to the `axum-server = { version = "0.7", features = ["tls-rustls"] }` crate and use `axum_server::bind_rustls(addr, RustlsConfig::from_pem_file(cert, key).await?)`. Add dep accordingly and replace the loop with `axum_server::bind_rustls(addr, cfg).serve(app.into_make_service()).await?;`.

Use the simpler `axum-server` path (recommended):

```toml
# crates/webhook/Cargo.toml
axum-server = { version = "0.7", features = ["tls-rustls"] }
```

Simplified `main.rs`:

```rust
use std::net::SocketAddr;
use std::sync::Arc;
use anyhow::Result;
use axum_server::tls_rustls::RustlsConfig;
use stubby_webhook::config::ImageRefs;
use stubby_webhook::server::{router, AppState};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().json()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let addr: SocketAddr = std::env::var("STUBBY_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8443".into())
        .parse()?;
    let cert = std::env::var("STUBBY_TLS_CERT").unwrap_or_else(|_| "/tls/tls.crt".into());
    let key  = std::env::var("STUBBY_TLS_KEY").unwrap_or_else(|_| "/tls/tls.key".into());

    let state = AppState { image_refs: Arc::new(ImageRefs::from_env()?) };
    let app = router(state);

    let cfg = RustlsConfig::from_pem_file(&cert, &key).await?;
    info!(%addr, "stubby-webhook listening (TLS)");
    axum_server::bind_rustls(addr, cfg)
        .serve(app.into_make_service())
        .await?;
    Ok(())
}
```

- [ ] **Step 2: Run cargo check**

Run: `cargo check -p stubby-webhook`
Expected: success.

- [ ] **Step 3: Commit**

```bash
cargo fmt
git add crates/webhook/
git commit -m "feat(webhook): TLS bind via axum-server + rustls"
```

---

## Task 4.3: Add Prometheus metrics

**Files:**
- Create: `crates/webhook/src/observability.rs`
- Modify: `crates/webhook/src/lib.rs`
- Modify: `crates/webhook/src/server.rs`
- Modify: `crates/webhook/src/main.rs`

- [ ] **Step 1: Wire module**

`lib.rs`:

```rust
pub mod observability;
```

- [ ] **Step 2: Write failing test in `server.rs`**

```rust
#[tokio::test]
async fn metrics_endpoint_responds() {
    crate::observability::init_metrics();
    let app = router(state());
    let resp = app
        .oneshot(Request::builder().uri("/metrics").body(Body::empty()).unwrap())
        .await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024).await.unwrap();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(text.contains("stubby_admissions_total") || text.is_empty(),
        "metrics body unexpected: {text}");
}
```

- [ ] **Step 3: Run; verify failure**

Run: `cargo test -p stubby-webhook server::metrics_endpoint`
Expected: 404 (no route yet).

- [ ] **Step 4: Implement `observability.rs`**

```rust
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use std::sync::OnceLock;

static HANDLE: OnceLock<PrometheusHandle> = OnceLock::new();

pub fn init_metrics() -> &'static PrometheusHandle {
    HANDLE.get_or_init(|| {
        let builder = PrometheusBuilder::new();
        builder.install_recorder().expect("install prometheus recorder")
    })
}

pub fn render() -> String {
    HANDLE
        .get()
        .map(|h| h.render())
        .unwrap_or_default()
}

pub fn record_admission(decision: &'static str, kind: &'static str) {
    metrics::counter!("stubby_admissions_total", "type" => kind, "decision" => decision).increment(1);
}
```

- [ ] **Step 5: Add `/metrics` route**

In `server.rs`, modify `router`:

```rust
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(|| async { "ok" }))
        .route("/metrics", get(metrics))
        .route("/mutate", post(mutate))
        .with_state(state)
}

async fn metrics() -> impl IntoResponse {
    (StatusCode::OK, crate::observability::render())
}
```

In `mutate`, record metrics:

```rust
async fn mutate(
    State(state): State<AppState>,
    Json(review): Json<AdmissionReview>,
) -> impl IntoResponse {
    let out = handle(review, state.image_refs.as_ref());
    let (decision, kind) = inspect(&out);
    crate::observability::record_admission(decision, kind);
    (StatusCode::OK, Json(out))
}

fn inspect(r: &AdmissionReview) -> (&'static str, &'static str) {
    match r.response.as_ref().and_then(|x| x.patch.as_ref()) {
        Some(_) => ("inject", "pod"),
        None => ("skip", "pod"),
    }
}
```

In `main.rs`, call `init_metrics()` after tracing init:

```rust
stubby_webhook::observability::init_metrics();
```

- [ ] **Step 6: Run tests; verify pass**

Run: `cargo test -p stubby-webhook`
Expected: all tests pass.

- [ ] **Step 7: Commit**

```bash
cargo fmt
git add crates/webhook/
git commit -m "feat(webhook): expose Prometheus /metrics endpoint and admission counters"
```

---

# Phase 5 — Webhook Dockerfile

## Task 5.1: Multi-stage Dockerfile (distroless)

**Files:**
- Create: `docker/webhook.Dockerfile`

- [ ] **Step 1: Write Dockerfile**

```dockerfile
# syntax=docker/dockerfile:1.7
FROM rust:1.85-bookworm AS builder
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p stubby-webhook && \
    cp /src/target/release/stubby-webhook /tmp/stubby-webhook

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /tmp/stubby-webhook /usr/local/bin/stubby-webhook
USER nonroot
EXPOSE 8443
ENTRYPOINT ["/usr/local/bin/stubby-webhook"]
```

- [ ] **Step 2: Local smoke build**

Run: `docker build -f docker/webhook.Dockerfile -t stubby-webhook:dev .`
Expected: image built. (Skip in pure CI envs without Docker.)

- [ ] **Step 3: Commit**

```bash
git add docker/webhook.Dockerfile
git commit -m "build(webhook): multi-stage distroless Dockerfile"
```

---

# Phase 6 — Dummy Backend

## Task 6.1: Backend routes (health, ready, metrics, openapi, docs, catch-all)

**Files:**
- Create: `crates/dummy-backend/src/lib.rs`
- Create: `crates/dummy-backend/src/routes.rs`
- Create: `crates/dummy-backend/src/openapi.rs`
- Modify: `crates/dummy-backend/src/main.rs`

- [ ] **Step 1: Write `lib.rs`**

```rust
pub mod openapi;
pub mod routes;

#[derive(Clone, Debug)]
pub struct BackendConfig {
    pub app_name: String,
}

impl BackendConfig {
    pub fn from_env() -> Self {
        Self {
            app_name: std::env::var("STUBBY_APP_NAME").unwrap_or_else(|_| "stubby".into()),
        }
    }
}
```

- [ ] **Step 2: Write failing tests in `routes.rs`**

```rust
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::get,
    Router,
};
use crate::BackendConfig;

pub fn router(cfg: BackendConfig) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/ready", get(ready))
        .route("/openapi.json", get(openapi))
        .route("/docs", get(docs))
        .fallback(catchall)
        .with_state(cfg)
}

async fn health() -> impl IntoResponse { (StatusCode::OK, "ok") }
async fn ready() -> impl IntoResponse  { (StatusCode::OK, "ok") }

async fn openapi(State(cfg): State<BackendConfig>) -> impl IntoResponse {
    let body = crate::openapi::doc(&cfg.app_name);
    (StatusCode::OK, [("content-type", "application/json")], body)
}

async fn docs() -> impl IntoResponse {
    let html = r#"<!doctype html><html><head><title>stubby docs</title>
<link rel="stylesheet" href="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui.css">
</head><body><div id="swagger"></div>
<script src="https://cdn.jsdelivr.net/npm/swagger-ui-dist@5/swagger-ui-bundle.js"></script>
<script>SwaggerUIBundle({url:'/openapi.json',dom_id:'#swagger'})</script>
</body></html>"#;
    (StatusCode::OK, [("content-type", "text/html")], html)
}

async fn catchall(State(cfg): State<BackendConfig>, req: axum::http::Request<axum::body::Body>) -> impl IntoResponse {
    let path = req.uri().path().to_string();
    let body = serde_json::json!({
        "status": "dummy",
        "app": cfg.app_name,
        "path": path
    });
    (StatusCode::OK, axum::Json(body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn cfg() -> BackendConfig { BackendConfig { app_name: "demo".into() } }

    async fn get(path: &str) -> (StatusCode, String) {
        let app = router(cfg());
        let resp = app.oneshot(Request::builder().uri(path).body(Body::empty()).unwrap()).await.unwrap();
        let s = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), 64*1024).await.unwrap();
        (s, String::from_utf8(body.to_vec()).unwrap())
    }

    #[tokio::test] async fn health_ok() {
        let (s, b) = get("/health").await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(b, "ok");
    }
    #[tokio::test] async fn ready_ok() {
        let (s, _) = get("/ready").await;
        assert_eq!(s, StatusCode::OK);
    }
    #[tokio::test] async fn openapi_includes_app_name() {
        let (s, b) = get("/openapi.json").await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["info"]["title"], "demo (dummy)");
    }
    #[tokio::test] async fn docs_renders_html() {
        let (s, b) = get("/docs").await;
        assert_eq!(s, StatusCode::OK);
        assert!(b.contains("swagger-ui"));
    }
    #[tokio::test] async fn catchall_returns_dummy() {
        let (s, b) = get("/anything/else").await;
        assert_eq!(s, StatusCode::OK);
        let v: serde_json::Value = serde_json::from_str(&b).unwrap();
        assert_eq!(v["status"], "dummy");
        assert_eq!(v["app"], "demo");
        assert_eq!(v["path"], "/anything/else");
    }
}
```

- [ ] **Step 3: Write `openapi.rs`**

```rust
pub fn doc(app_name: &str) -> String {
    serde_json::json!({
        "openapi": "3.0.0",
        "info": {
            "title": format!("{app_name} (dummy)"),
            "version": "0.0.0",
            "description": "Placeholder spec served by stubby-dummy-backend."
        },
        "paths": {
            "/health": {"get": {"responses": {"200": {"description":"ok"}}}},
            "/ready":  {"get": {"responses": {"200": {"description":"ok"}}}}
        }
    }).to_string()
}
```

- [ ] **Step 4: Replace `main.rs`**

```rust
use anyhow::Result;
use stubby_dummy_backend::{routes::router, BackendConfig};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().json().init();
    let addr: std::net::SocketAddr = std::env::var("STUBBY_BACKEND_LISTEN")
        .unwrap_or_else(|_| "0.0.0.0:8080".into())
        .parse()?;
    let cfg = BackendConfig::from_env();
    let app = router(cfg);
    tracing::info!(%addr, "stubby-dummy-backend listening");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}
```

Add to `crates/dummy-backend/Cargo.toml`:

```toml
[lib]
path = "src/lib.rs"
```

(reorder existing keys accordingly)

- [ ] **Step 5: Run tests; verify pass**

Run: `cargo test -p stubby-dummy-backend`
Expected: 5 passed.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/dummy-backend/
git commit -m "feat(dummy-backend): health, ready, openapi, docs, and catch-all routes"
```

---

## Task 6.2: Backend Dockerfile

**Files:**
- Create: `docker/dummy-backend.Dockerfile`

- [ ] **Step 1: Write Dockerfile**

```dockerfile
# syntax=docker/dockerfile:1.7
FROM rust:1.85-bookworm AS builder
WORKDIR /src
COPY rust-toolchain.toml Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release -p stubby-dummy-backend && \
    cp /src/target/release/stubby-dummy-backend /tmp/app

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder /tmp/app /usr/local/bin/app
USER nonroot
EXPOSE 8080
ENTRYPOINT ["/usr/local/bin/app"]
```

- [ ] **Step 2: Commit**

```bash
git add docker/dummy-backend.Dockerfile
git commit -m "build(dummy-backend): distroless Dockerfile"
```

---

# Phase 7 — Dummy Frontend

## Task 7.1: HTML template rendering with snapshots

**Files:**
- Create: `crates/dummy-frontend/templates/index.html.tmpl`
- Create: `crates/dummy-frontend/templates/style.css`
- Modify: `crates/dummy-frontend/src/lib.rs`
- Create: `crates/dummy-frontend/tests/render.rs`

- [ ] **Step 1: Write the HTML template**

`crates/dummy-frontend/templates/index.html.tmpl`:

```html
<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <title>{{APP_NAME}} — stubby</title>
  <link rel="stylesheet" href="/style.css">
</head>
<body>
  <main>
    <span class="badge">dummy mode</span>
    <h1>{{APP_NAME}}</h1>
    <p>This service is running on the <strong>stubby</strong> placeholder image.
       The real implementation isn't deployed yet.</p>
    <footer>
      <a href="https://github.com/kauemendes/stubby">github.com/kauemendes/stubby</a>
    </footer>
  </main>
</body>
</html>
```

`crates/dummy-frontend/templates/style.css`:

```css
:root { color-scheme: light dark; font-family: system-ui, sans-serif; }
body { max-width: 720px; margin: 4rem auto; padding: 0 1rem; line-height: 1.6; }
.badge { display:inline-block; padding: 0.15rem 0.5rem; border-radius: 4px;
         background: #ffec99; color: #333; font-size: 0.8rem; }
h1 { margin-top: 0.5rem; }
footer { margin-top: 3rem; font-size: 0.85rem; opacity: 0.7; }
```

- [ ] **Step 2: Replace `src/lib.rs` with real renderer + HTML-escape**

```rust
const TEMPLATE: &str = include_str!("../templates/index.html.tmpl");

pub fn render_index(app_name: &str) -> String {
    TEMPLATE.replace("{{APP_NAME}}", &html_escape(app_name))
}

fn html_escape(s: &str) -> String {
    s.chars().map(|c| match c {
        '<' => "&lt;".to_string(),
        '>' => "&gt;".to_string(),
        '&' => "&amp;".to_string(),
        '"' => "&quot;".to_string(),
        '\'' => "&#x27;".to_string(),
        c => c.to_string(),
    }).collect()
}
```

- [ ] **Step 3: Write failing snapshot tests**

`crates/dummy-frontend/tests/render.rs`:

```rust
use stubby_dummy_frontend::render_index;

#[test]
fn simple_name_snapshot() {
    let html = render_index("Orders");
    insta::assert_snapshot!("simple_name", html);
}

#[test]
fn xss_attempt_is_escaped() {
    let html = render_index("<script>alert(1)</script>");
    assert!(!html.contains("<script>"), "must escape");
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;"));
    insta::assert_snapshot!("xss_escaped", html);
}

#[test]
fn empty_name_uses_literal_empty() {
    let html = render_index("");
    assert!(html.contains("<h1></h1>"));
}
```

- [ ] **Step 4: Run; verify snapshots are missing (insta creates `.new` files)**

Run: `cargo test -p stubby-dummy-frontend`
Expected: tests fail with snapshot mismatch; run `cargo insta review` (or set `INSTA_UPDATE=always` locally) to accept.

For CI, set `INSTA_UPDATE=no`. For the plan execution: run locally with `INSTA_UPDATE=always cargo test -p stubby-dummy-frontend` once to accept initial snapshots, then commit `.snap` files alongside the test.

- [ ] **Step 5: Re-run tests; verify pass**

Run: `cargo test -p stubby-dummy-frontend`
Expected: 3 passed.

- [ ] **Step 6: Commit**

```bash
cargo fmt
git add crates/dummy-frontend/
git commit -m "feat(dummy-frontend): HTML template rendering with XSS-safe escaping and snapshots"
```

---

## Task 7.2: Frontend Dockerfile + nginx entrypoint

**Files:**
- Create: `crates/dummy-frontend/nginx/default.conf`
- Create: `crates/dummy-frontend/nginx/entrypoint.sh`
- Create: `docker/dummy-frontend.Dockerfile`

- [ ] **Step 1: Write nginx config**

```nginx
# crates/dummy-frontend/nginx/default.conf
server {
    listen 80;
    server_name _;

    location = /health { return 200 'ok'; add_header Content-Type text/plain; }
    location = /ready  { return 200 'ok'; add_header Content-Type text/plain; }

    location / {
        root /usr/share/nginx/html;
        index index.html;
        try_files $uri $uri/ /index.html;
    }
}
```

- [ ] **Step 2: Write entrypoint**

```sh
#!/bin/sh
# crates/dummy-frontend/nginx/entrypoint.sh
set -e

APP="${STUBBY_APP_NAME:-stubby}"

# Escape HTML-special chars for safe substitution.
escape_html() {
    printf '%s' "$1" | sed \
        -e 's/&/\&amp;/g' \
        -e 's/</\&lt;/g' \
        -e 's/>/\&gt;/g' \
        -e 's/"/\&quot;/g' \
        -e "s/'/\&#x27;/g"
}

ESCAPED=$(escape_html "$APP")
sed "s|{{APP_NAME}}|$ESCAPED|g" /etc/stubby/index.html.tmpl > /usr/share/nginx/html/index.html
cp /etc/stubby/style.css /usr/share/nginx/html/style.css

exec nginx -g 'daemon off;'
```

- [ ] **Step 3: Write Dockerfile**

```dockerfile
# syntax=docker/dockerfile:1.7
# docker/dummy-frontend.Dockerfile
FROM nginx:1.27-alpine
COPY crates/dummy-frontend/templates/index.html.tmpl /etc/stubby/index.html.tmpl
COPY crates/dummy-frontend/templates/style.css       /etc/stubby/style.css
COPY crates/dummy-frontend/nginx/default.conf        /etc/nginx/conf.d/default.conf
COPY crates/dummy-frontend/nginx/entrypoint.sh       /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh
EXPOSE 80
ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
```

- [ ] **Step 4: Local smoke build**

Run: `docker build -f docker/dummy-frontend.Dockerfile -t stubby-dummy-frontend:dev .`
Expected: image built.

- [ ] **Step 5: Commit**

```bash
chmod +x crates/dummy-frontend/nginx/entrypoint.sh
git add crates/dummy-frontend/nginx/ docker/dummy-frontend.Dockerfile
git commit -m "build(dummy-frontend): nginx Dockerfile with envsubst-style entrypoint"
```

---

# Phase 8 — Helm Chart

## Task 8.1: Chart skeleton + values + helpers

**Files:**
- Create: `charts/stubby/Chart.yaml`
- Create: `charts/stubby/values.yaml`
- Create: `charts/stubby/values.schema.json`
- Create: `charts/stubby/templates/_helpers.tpl`

- [ ] **Step 1: Write `Chart.yaml`**

```yaml
apiVersion: v2
name: stubby
description: Kubernetes mutating webhook that injects dummy backend/frontend images via annotation
type: application
version: 0.1.0
appVersion: "0.1.0"
home: https://github.com/kauemendes/stubby
sources:
  - https://github.com/kauemendes/stubby
maintainers:
  - name: Kauê Mendes
icon: https://raw.githubusercontent.com/kauemendes/stubby/main/docs/logo.svg
keywords:
  - kubernetes
  - webhook
  - dummy
  - placeholder
  - testing
```

- [ ] **Step 2: Write `values.yaml`**

```yaml
nameOverride: ""
fullnameOverride: ""

image:
  repository: ghcr.io/kauemendes/stubby-webhook
  tag: ""              # defaults to .Chart.AppVersion
  pullPolicy: IfNotPresent

dummyImages:
  backend: ghcr.io/kauemendes/stubby-dummy-backend:latest
  frontend: ghcr.io/kauemendes/stubby-dummy-frontend:latest

replicaCount: 2

service:
  port: 443
  targetPort: 8443

webhook:
  failurePolicy: Ignore
  reinvocationPolicy: Never
  namespaceSelector:
    matchExpressions:
      - key: kubernetes.io/metadata.name
        operator: NotIn
        values: [kube-system]
      - key: stubby.io/exclude
        operator: NotIn
        values: ["true"]

tls:
  mode: self-signed     # one of: self-signed, cert-manager
  certManager:
    issuerRef:
      kind: Issuer
      name: stubby-selfsigned
  selfSigned:
    image: alpine/openssl:3.3
    validityDays: 365

resources:
  requests: { cpu: 50m, memory: 64Mi }
  limits:   { cpu: 200m, memory: 128Mi }

logLevel: info
```

- [ ] **Step 3: Write `values.schema.json`**

```json
{
  "$schema": "http://json-schema.org/draft-07/schema",
  "type": "object",
  "additionalProperties": true,
  "required": ["image", "dummyImages", "tls"],
  "properties": {
    "tls": {
      "type": "object",
      "required": ["mode"],
      "properties": {
        "mode": { "type": "string", "enum": ["self-signed", "cert-manager"] }
      }
    }
  }
}
```

- [ ] **Step 4: Write `_helpers.tpl`**

```yaml
{{/* templates/_helpers.tpl */}}
{{- define "stubby.fullname" -}}
{{- if .Values.fullnameOverride -}}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s" .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "stubby.labels" -}}
app.kubernetes.io/name: stubby
app.kubernetes.io/instance: {{ .Release.Name }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
helm.sh/chart: {{ printf "%s-%s" .Chart.Name .Chart.Version }}
{{- end -}}

{{- define "stubby.selectorLabels" -}}
app.kubernetes.io/name: stubby
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "stubby.image" -}}
{{ .Values.image.repository }}:{{ .Values.image.tag | default .Chart.AppVersion }}
{{- end -}}
```

- [ ] **Step 5: Verify chart parses (lint deferred until templates exist)**

Run: `helm show chart charts/stubby`
Expected: chart metadata prints with name `stubby`, version `0.1.0`. (Running `helm lint` here fails with "chart contains no templates" — lint moves to Task 8.2 once templates exist.)

- [ ] **Step 6: Commit**

```bash
git add charts/stubby/
git commit -m "feat(chart): chart skeleton with values, schema, and helpers"
```

---

## Task 8.2: Deployment + Service + ServiceAccount + RBAC

**Files:**
- Create: `charts/stubby/templates/deployment.yaml`
- Create: `charts/stubby/templates/service.yaml`
- Create: `charts/stubby/templates/serviceaccount.yaml`
- Create: `charts/stubby/templates/rbac.yaml`

- [ ] **Step 1: Write `serviceaccount.yaml`**

```yaml
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "stubby.fullname" . }}
  labels: {{- include "stubby.labels" . | nindent 4 }}
```

- [ ] **Step 2: Write `rbac.yaml`** (used by self-signed Job; webhook itself needs no RBAC)

```yaml
{{- if eq .Values.tls.mode "self-signed" }}
apiVersion: rbac.authorization.k8s.io/v1
kind: Role
metadata:
  name: {{ include "stubby.fullname" . }}-tls
  labels: {{- include "stubby.labels" . | nindent 4 }}
rules:
  - apiGroups: [""]
    resources: ["secrets"]
    verbs: ["get", "create", "update", "patch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: RoleBinding
metadata:
  name: {{ include "stubby.fullname" . }}-tls
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: Role
  name: {{ include "stubby.fullname" . }}-tls
subjects:
  - kind: ServiceAccount
    name: {{ include "stubby.fullname" . }}-tls
    namespace: {{ .Release.Namespace }}
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRole
metadata:
  name: {{ include "stubby.fullname" . }}-tls-cluster
rules:
  - apiGroups: ["admissionregistration.k8s.io"]
    resources: ["mutatingwebhookconfigurations"]
    resourceNames: ["{{ include "stubby.fullname" . }}"]
    verbs: ["get", "patch"]
---
apiVersion: rbac.authorization.k8s.io/v1
kind: ClusterRoleBinding
metadata:
  name: {{ include "stubby.fullname" . }}-tls-cluster
roleRef:
  apiGroup: rbac.authorization.k8s.io
  kind: ClusterRole
  name: {{ include "stubby.fullname" . }}-tls-cluster
subjects:
  - kind: ServiceAccount
    name: {{ include "stubby.fullname" . }}-tls
    namespace: {{ .Release.Namespace }}
---
apiVersion: v1
kind: ServiceAccount
metadata:
  name: {{ include "stubby.fullname" . }}-tls
{{- end }}
```

- [ ] **Step 3: Write `deployment.yaml`**

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: {{ include "stubby.fullname" . }}
  labels: {{- include "stubby.labels" . | nindent 4 }}
spec:
  replicas: {{ .Values.replicaCount }}
  selector:
    matchLabels: {{- include "stubby.selectorLabels" . | nindent 6 }}
  template:
    metadata:
      labels: {{- include "stubby.selectorLabels" . | nindent 8 }}
      annotations:
        stubby.io/type: "off"   # never inject into ourselves
    spec:
      serviceAccountName: {{ include "stubby.fullname" . }}
      containers:
        - name: webhook
          image: {{ include "stubby.image" . }}
          imagePullPolicy: {{ .Values.image.pullPolicy }}
          env:
            - { name: STUBBY_IMAGE_BACKEND,  value: {{ .Values.dummyImages.backend | quote }} }
            - { name: STUBBY_IMAGE_FRONTEND, value: {{ .Values.dummyImages.frontend | quote }} }
            - { name: RUST_LOG, value: {{ .Values.logLevel | quote }} }
            - { name: STUBBY_LISTEN, value: "0.0.0.0:8443" }
          ports:
            - { containerPort: 8443, name: https }
          volumeMounts:
            - { name: tls, mountPath: /tls, readOnly: true }
          livenessProbe:
            httpGet: { path: /healthz, port: https, scheme: HTTPS }
          readinessProbe:
            httpGet: { path: /readyz, port: https, scheme: HTTPS }
          resources: {{- toYaml .Values.resources | nindent 12 }}
      volumes:
        - name: tls
          secret:
            secretName: {{ include "stubby.fullname" . }}-tls
```

- [ ] **Step 4: Write `service.yaml`**

```yaml
apiVersion: v1
kind: Service
metadata:
  name: {{ include "stubby.fullname" . }}
  labels: {{- include "stubby.labels" . | nindent 4 }}
spec:
  type: ClusterIP
  selector: {{- include "stubby.selectorLabels" . | nindent 4 }}
  ports:
    - port: {{ .Values.service.port }}
      targetPort: {{ .Values.service.targetPort }}
      protocol: TCP
      name: https
```

- [ ] **Step 5: Lint + template render**

Run: `helm lint charts/stubby && helm template stubby ./charts/stubby --debug | head -80`
Expected: 0 errors; output shows the resources.

- [ ] **Step 6: Commit**

```bash
git add charts/stubby/templates/
git commit -m "feat(chart): Deployment, Service, ServiceAccount, and RBAC"
```

---

## Task 8.3: MutatingWebhookConfiguration

**Files:**
- Create: `charts/stubby/templates/mutatingwebhookconfiguration.yaml`

- [ ] **Step 1: Write template**

```yaml
apiVersion: admissionregistration.k8s.io/v1
kind: MutatingWebhookConfiguration
metadata:
  name: {{ include "stubby.fullname" . }}
  labels: {{- include "stubby.labels" . | nindent 4 }}
  {{- if eq .Values.tls.mode "cert-manager" }}
  annotations:
    cert-manager.io/inject-ca-from: "{{ .Release.Namespace }}/{{ include "stubby.fullname" . }}"
  {{- end }}
webhooks:
  - name: pods.stubby.io
    sideEffects: None
    failurePolicy: {{ .Values.webhook.failurePolicy }}
    reinvocationPolicy: {{ .Values.webhook.reinvocationPolicy }}
    admissionReviewVersions: ["v1"]
    rules:
      - apiGroups: [""]
        apiVersions: ["v1"]
        resources: ["pods"]
        operations: ["CREATE"]
        scope: Namespaced
    namespaceSelector: {{- toYaml .Values.webhook.namespaceSelector | nindent 6 }}
    clientConfig:
      service:
        name: {{ include "stubby.fullname" . }}
        namespace: {{ .Release.Namespace }}
        path: /mutate
        port: {{ .Values.service.port }}
      {{- if eq .Values.tls.mode "self-signed" }}
      caBundle: ""  # patched in-place by the self-signed TLS Job
      {{- end }}
```

- [ ] **Step 2: Lint + render**

Run: `helm template stubby ./charts/stubby --set tls.mode=cert-manager | grep cert-manager.io/inject-ca-from`
Expected: line present.

Run: `helm template stubby ./charts/stubby --set tls.mode=self-signed | grep 'caBundle:'`
Expected: line present with empty value.

- [ ] **Step 3: Commit**

```bash
git add charts/stubby/templates/mutatingwebhookconfiguration.yaml
git commit -m "feat(chart): MutatingWebhookConfiguration with TLS-mode-aware caBundle"
```

---

## Task 8.4: TLS templates (cert-manager + self-signed Job)

**Files:**
- Create: `charts/stubby/templates/tls-cert-manager.yaml`
- Create: `charts/stubby/templates/tls-self-signed-job.yaml`

- [ ] **Step 1: Write `tls-cert-manager.yaml`**

```yaml
{{- if eq .Values.tls.mode "cert-manager" }}
apiVersion: cert-manager.io/v1
kind: Issuer
metadata:
  name: {{ .Values.tls.certManager.issuerRef.name }}
spec:
  selfSigned: {}
---
apiVersion: cert-manager.io/v1
kind: Certificate
metadata:
  name: {{ include "stubby.fullname" . }}
spec:
  secretName: {{ include "stubby.fullname" . }}-tls
  issuerRef: {{- toYaml .Values.tls.certManager.issuerRef | nindent 4 }}
  dnsNames:
    - {{ include "stubby.fullname" . }}.{{ .Release.Namespace }}.svc
    - {{ include "stubby.fullname" . }}.{{ .Release.Namespace }}.svc.cluster.local
{{- end }}
```

- [ ] **Step 2: Write `tls-self-signed-job.yaml`**

```yaml
{{- if eq .Values.tls.mode "self-signed" }}
apiVersion: batch/v1
kind: Job
metadata:
  name: {{ include "stubby.fullname" . }}-tls-bootstrap
  annotations:
    helm.sh/hook: pre-install,pre-upgrade
    helm.sh/hook-weight: "-5"
    helm.sh/hook-delete-policy: before-hook-creation
spec:
  ttlSecondsAfterFinished: 300
  template:
    spec:
      serviceAccountName: {{ include "stubby.fullname" . }}-tls
      restartPolicy: OnFailure
      containers:
        - name: bootstrap
          image: {{ .Values.tls.selfSigned.image }}
          env:
            - { name: NAMESPACE, value: {{ .Release.Namespace }} }
            - { name: SECRET_NAME, value: "{{ include "stubby.fullname" . }}-tls" }
            - { name: SERVICE, value: "{{ include "stubby.fullname" . }}" }
            - { name: MWC_NAME, value: "{{ include "stubby.fullname" . }}" }
            - { name: VALIDITY_DAYS, value: "{{ .Values.tls.selfSigned.validityDays }}" }
          command:
            - sh
            - -c
            - |
              set -eu
              apk add --no-cache curl jq >/dev/null
              CN="${SERVICE}.${NAMESPACE}.svc"
              DNS="${CN},${CN}.cluster.local"

              # Skip if existing secret is still valid (>30 days remaining)
              # (omitted for brevity in this Job; safe to regenerate on every install)

              cd /tmp
              cat >openssl.cnf <<EOF
              [req]
              prompt = no
              distinguished_name = req
              req_extensions = ext
              [req]
              CN = ${CN}
              [ext]
              subjectAltName = DNS:${SERVICE}.${NAMESPACE}.svc, DNS:${SERVICE}.${NAMESPACE}.svc.cluster.local
              EOF

              openssl genrsa -out ca.key 2048
              openssl req -x509 -new -nodes -key ca.key -subj "/CN=stubby-ca" -days "${VALIDITY_DAYS}" -out ca.crt
              openssl genrsa -out tls.key 2048
              openssl req -new -key tls.key -out tls.csr -config openssl.cnf
              openssl x509 -req -in tls.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
                -out tls.crt -days "${VALIDITY_DAYS}" -extensions ext -extfile openssl.cnf

              CA_B64=$(base64 -w0 < ca.crt)
              TLS_CRT_B64=$(base64 -w0 < tls.crt)
              TLS_KEY_B64=$(base64 -w0 < tls.key)

              TOKEN=$(cat /var/run/secrets/kubernetes.io/serviceaccount/token)
              CA=/var/run/secrets/kubernetes.io/serviceaccount/ca.crt
              API=https://kubernetes.default.svc

              # Upsert secret
              curl -fsSk --cacert "$CA" -H "Authorization: Bearer $TOKEN" \
                -H "Content-Type: application/json" \
                -X PATCH "$API/api/v1/namespaces/${NAMESPACE}/secrets/${SECRET_NAME}" \
                -d "$(jq -n --arg c "$TLS_CRT_B64" --arg k "$TLS_KEY_B64" '{type:"kubernetes.io/tls", data:{"tls.crt":$c,"tls.key":$k}}')" \
              || curl -fsSk --cacert "$CA" -H "Authorization: Bearer $TOKEN" \
                -H "Content-Type: application/json" \
                -X POST "$API/api/v1/namespaces/${NAMESPACE}/secrets" \
                -d "$(jq -n --arg n "$SECRET_NAME" --arg c "$TLS_CRT_B64" --arg k "$TLS_KEY_B64" \
                       '{apiVersion:"v1",kind:"Secret",metadata:{name:$n},type:"kubernetes.io/tls",data:{"tls.crt":$c,"tls.key":$k}}')"

              # Patch MutatingWebhookConfiguration caBundle
              curl -fsSk --cacert "$CA" -H "Authorization: Bearer $TOKEN" \
                -H "Content-Type: application/json-patch+json" \
                -X PATCH "$API/apis/admissionregistration.k8s.io/v1/mutatingwebhookconfigurations/${MWC_NAME}" \
                -d "[{\"op\":\"replace\",\"path\":\"/webhooks/0/clientConfig/caBundle\",\"value\":\"$CA_B64\"}]"
{{- end }}
```

- [ ] **Step 3: Render and inspect**

Run: `helm template stubby ./charts/stubby --set tls.mode=self-signed | grep -A2 'tls-bootstrap'`
Expected: Job present.

Run: `helm template stubby ./charts/stubby --set tls.mode=cert-manager | grep 'kind: Certificate'`
Expected: Certificate present.

- [ ] **Step 4: Commit**

```bash
git add charts/stubby/templates/
git commit -m "feat(chart): TLS bootstrap via cert-manager or self-signed Job"
```

---

## Task 8.5: helm-unittest fixtures

**Files:**
- Create: `charts/stubby/tests/deployment_test.yaml`
- Create: `charts/stubby/tests/mwc_test.yaml`
- Create: `charts/stubby/tests/tls_test.yaml`

- [ ] **Step 1: Write `deployment_test.yaml`**

```yaml
suite: deployment
templates:
  - deployment.yaml
tests:
  - it: sets default replicas
    asserts:
      - equal:
          path: spec.replicas
          value: 2
  - it: mounts tls volume
    asserts:
      - contains:
          path: spec.template.spec.volumes
          content:
            name: tls
            secret:
              secretName: RELEASE-NAME-tls
```

- [ ] **Step 2: Write `mwc_test.yaml`**

```yaml
suite: mutatingwebhookconfiguration
templates:
  - mutatingwebhookconfiguration.yaml
tests:
  - it: targets pods CREATE
    asserts:
      - equal:
          path: webhooks[0].rules[0].resources
          value: ["pods"]
      - equal:
          path: webhooks[0].rules[0].operations
          value: ["CREATE"]
      - equal:
          path: webhooks[0].failurePolicy
          value: Ignore
  - it: adds cert-manager annotation in cert-manager mode
    set:
      tls.mode: cert-manager
    asserts:
      - isNotEmpty:
          path: metadata.annotations["cert-manager.io/inject-ca-from"]
  - it: empty caBundle in self-signed mode
    set:
      tls.mode: self-signed
    asserts:
      - equal:
          path: webhooks[0].clientConfig.caBundle
          value: ""
```

- [ ] **Step 3: Write `tls_test.yaml`**

```yaml
suite: tls
tests:
  - it: cert-manager mode generates Certificate
    templates: [tls-cert-manager.yaml]
    set:
      tls.mode: cert-manager
    asserts:
      - hasDocuments: { count: 2 }
      - containsDocument:
          kind: Certificate
          apiVersion: cert-manager.io/v1

  - it: self-signed mode generates Job
    templates: [tls-self-signed-job.yaml]
    set:
      tls.mode: self-signed
    asserts:
      - hasDocuments: { count: 1 }
      - containsDocument: { kind: Job, apiVersion: batch/v1 }
```

- [ ] **Step 4: Run `helm unittest`**

Run: `helm unittest charts/stubby`
Expected: all suites pass.

- [ ] **Step 5: Commit**

```bash
git add charts/stubby/tests/
git commit -m "test(chart): helm-unittest suites for deployment, MWC, and TLS"
```

---

# Phase 9 — Examples + Integration Tests

## Task 9.1: Example manifests

**Files:**
- Create: `examples/backend.yaml`
- Create: `examples/frontend.yaml`
- Create: `examples/off.yaml`

- [ ] **Step 1: Write `examples/backend.yaml`**

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: orders-api
spec:
  replicas: 1
  selector: { matchLabels: { app: orders-api } }
  template:
    metadata:
      labels: { app: orders-api }
      annotations:
        stubby.io/type: backend
        stubby.io/app-name: "Orders API"
    spec:
      containers:
        - name: orders
          image: ghcr.io/example/orders-api:notbuilt
          ports: [{ containerPort: 9999 }]   # will be replaced by webhook
---
apiVersion: v1
kind: Service
metadata: { name: orders-api }
spec:
  selector: { app: orders-api }
  ports: [{ port: 8080, targetPort: http }]
```

- [ ] **Step 2: Write `examples/frontend.yaml`**

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: storefront
spec:
  replicas: 1
  selector: { matchLabels: { app: storefront } }
  template:
    metadata:
      labels: { app: storefront }
      annotations:
        stubby.io/type: frontend
        stubby.io/app-name: "Storefront"
    spec:
      containers:
        - name: site
          image: ghcr.io/example/storefront:notbuilt
---
apiVersion: v1
kind: Service
metadata: { name: storefront }
spec:
  selector: { app: storefront }
  ports: [{ port: 80, targetPort: http }]
```

- [ ] **Step 3: Write `examples/off.yaml`**

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: real-app
spec:
  replicas: 1
  selector: { matchLabels: { app: real-app } }
  template:
    metadata:
      labels: { app: real-app }
      annotations:
        stubby.io/type: "off"
    spec:
      containers:
        - name: real
          image: nginx:1.27-alpine
```

- [ ] **Step 4: Commit**

```bash
git add examples/
git commit -m "docs(examples): backend, frontend, and off-mode manifests"
```

---

## Task 9.2: kind config + e2e harness

**Files:**
- Create: `test/kind-config.yaml`
- Create: `test/e2e/run.sh`
- Create: `test/e2e/cases/backend.sh`
- Create: `test/e2e/cases/frontend.sh`

- [ ] **Step 1: Write `test/kind-config.yaml`**

```yaml
kind: Cluster
apiVersion: kind.x-k8s.io/v1alpha4
nodes:
  - role: control-plane
    extraPortMappings: []
```

- [ ] **Step 2: Write `test/e2e/run.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail

CLUSTER=${CLUSTER:-stubby-e2e}
NS=stubby-system

# 1. Spin up kind
kind create cluster --name "$CLUSTER" --config test/kind-config.yaml

# 2. Build and load images
docker build -f docker/webhook.Dockerfile -t local/stubby-webhook:e2e .
docker build -f docker/dummy-backend.Dockerfile -t local/stubby-dummy-backend:e2e .
docker build -f docker/dummy-frontend.Dockerfile -t local/stubby-dummy-frontend:e2e .
kind load docker-image local/stubby-webhook:e2e --name "$CLUSTER"
kind load docker-image local/stubby-dummy-backend:e2e --name "$CLUSTER"
kind load docker-image local/stubby-dummy-frontend:e2e --name "$CLUSTER"

# 3. Install chart
kubectl create namespace "$NS"
helm install stubby ./charts/stubby \
  --namespace "$NS" \
  --set image.repository=local/stubby-webhook \
  --set image.tag=e2e \
  --set image.pullPolicy=Never \
  --set dummyImages.backend=local/stubby-dummy-backend:e2e \
  --set dummyImages.frontend=local/stubby-dummy-frontend:e2e \
  --set tls.mode=self-signed \
  --wait --timeout=2m

# 4. Run case scripts
for case in test/e2e/cases/*.sh; do
  echo "=== $case ==="
  bash "$case"
done

# 5. Cleanup (caller may keep cluster with KEEP=1)
if [[ "${KEEP:-0}" != "1" ]]; then
  kind delete cluster --name "$CLUSTER"
fi
```

- [ ] **Step 3: Write `test/e2e/cases/backend.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
NS=default

kubectl apply -n "$NS" -f examples/backend.yaml
kubectl rollout status -n "$NS" deploy/orders-api --timeout=60s

POD=$(kubectl get -n "$NS" pod -l app=orders-api -o jsonpath='{.items[0].metadata.name}')
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')

[[ "$IMG" == "local/stubby-dummy-backend:e2e" ]] || { echo "image not mutated: $IMG"; exit 1; }

kubectl run curl --rm -i --image=curlimages/curl:8 --restart=Never -n "$NS" -- \
  curl -sf http://orders-api.default.svc:8080/health | grep -q ok
echo "backend OK"
```

- [ ] **Step 4: Write `test/e2e/cases/frontend.sh`**

```bash
#!/usr/bin/env bash
set -euo pipefail
NS=default

kubectl apply -n "$NS" -f examples/frontend.yaml
kubectl rollout status -n "$NS" deploy/storefront --timeout=60s

POD=$(kubectl get -n "$NS" pod -l app=storefront -o jsonpath='{.items[0].metadata.name}')
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')

[[ "$IMG" == "local/stubby-dummy-frontend:e2e" ]] || { echo "image not mutated: $IMG"; exit 1; }

kubectl run curl --rm -i --image=curlimages/curl:8 --restart=Never -n "$NS" -- \
  curl -s http://storefront.default.svc:80/ | grep -q 'Storefront'
echo "frontend OK"
```

> The Service objects required by these case scripts are already included in `examples/backend.yaml` and `examples/frontend.yaml` (added in Task 9.1).

- [ ] **Step 5: Make scripts executable**

```bash
chmod +x test/e2e/run.sh test/e2e/cases/*.sh
```

- [ ] **Step 6: Local run (optional, requires Docker + kind)**

Run: `bash test/e2e/run.sh`
Expected: cluster comes up, chart installs, both cases print `... OK`.

- [ ] **Step 7: Commit**

```bash
git add test/ examples/
git commit -m "test(e2e): kind harness with backend and frontend assertions"
```

---

# Phase 10 — CI/CD

## Task 10.1: CI workflow (PR + push gates)

**Files:**
- Create: `.github/workflows/ci.yaml`

- [ ] **Step 1: Write the workflow**

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

jobs:
  fmt-clippy-test:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@v2
      - run: cargo fmt --all -- --check
      - run: cargo clippy --workspace --all-targets --all-features -- -D warnings
      - name: install cargo-llvm-cov
        run: cargo install cargo-llvm-cov --locked
      - run: cargo llvm-cov --workspace --fail-under-lines 80 --package stubby-webhook

  helm:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: azure/setup-helm@v4
      - name: install helm-unittest
        run: helm plugin install https://github.com/helm-unittest/helm-unittest.git
      - run: helm lint charts/stubby
      - run: helm unittest charts/stubby

  e2e:
    runs-on: ubuntu-latest
    needs: [fmt-clippy-test, helm]
    strategy:
      matrix:
        k8s: [v1.29.10, v1.30.6, v1.31.2]
    steps:
      - uses: actions/checkout@v4
      - uses: helm/kind-action@v1
        with:
          version: v0.24.0
          node_image: kindest/node:${{ matrix.k8s }}
          config: test/kind-config.yaml
          cluster_name: stubby-e2e
      - name: build images
        run: |
          docker build -f docker/webhook.Dockerfile -t local/stubby-webhook:e2e .
          docker build -f docker/dummy-backend.Dockerfile -t local/stubby-dummy-backend:e2e .
          docker build -f docker/dummy-frontend.Dockerfile -t local/stubby-dummy-frontend:e2e .
          kind load docker-image local/stubby-webhook:e2e --name stubby-e2e
          kind load docker-image local/stubby-dummy-backend:e2e --name stubby-e2e
          kind load docker-image local/stubby-dummy-frontend:e2e --name stubby-e2e
      - uses: azure/setup-helm@v4
      - name: install + assert
        env: { KEEP: "1" }
        run: bash test/e2e/run.sh
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/ci.yaml
git commit -m "ci: workflow with fmt, clippy, llvm-cov, helm tests, and e2e matrix"
```

---

## Task 10.2: Release workflow

**Files:**
- Create: `.github/workflows/release.yaml`

- [ ] **Step 1: Write the workflow**

```yaml
name: Release

on:
  push:
    tags: ["v*.*.*"]

permissions:
  contents: write
  packages: write
  id-token: write   # for cosign keyless

jobs:
  images:
    runs-on: ubuntu-latest
    strategy:
      matrix:
        component: [webhook, dummy-backend, dummy-frontend]
    steps:
      - uses: actions/checkout@v4
      - uses: docker/setup-qemu-action@v3
      - uses: docker/setup-buildx-action@v3
      - uses: docker/login-action@v3
        with:
          registry: ghcr.io
          username: ${{ github.actor }}
          password: ${{ secrets.GITHUB_TOKEN }}
      - uses: docker/metadata-action@v5
        id: meta
        with:
          images: ghcr.io/${{ github.repository_owner }}/stubby-${{ matrix.component }}
          tags: |
            type=semver,pattern={{version}}
            type=semver,pattern={{major}}.{{minor}}
            type=raw,value=latest
      - uses: docker/build-push-action@v6
        id: build
        with:
          context: .
          file: docker/${{ matrix.component }}.Dockerfile
          platforms: linux/amd64,linux/arm64
          push: true
          tags: ${{ steps.meta.outputs.tags }}
          labels: ${{ steps.meta.outputs.labels }}
      - uses: sigstore/cosign-installer@v3
      - name: cosign sign
        run: |
          for tag in ${{ steps.meta.outputs.tags }}; do
            cosign sign --yes "ghcr.io/${{ github.repository_owner }}/stubby-${{ matrix.component }}@${{ steps.build.outputs.digest }}"
          done

  chart:
    runs-on: ubuntu-latest
    needs: images
    steps:
      - uses: actions/checkout@v4
        with: { fetch-depth: 0 }
      - uses: azure/setup-helm@v4
      - name: package chart
        run: |
          mkdir dist
          helm package charts/stubby --destination dist
      - name: publish to gh-pages
        uses: helm/chart-releaser-action@v1.6.0
        with:
          charts_dir: charts
          skip_packaging: "true"
        env:
          CR_TOKEN: ${{ secrets.GITHUB_TOKEN }}
          CR_PACKAGE_PATH: dist
```

- [ ] **Step 2: Commit**

```bash
git add .github/workflows/release.yaml
git commit -m "ci(release): multi-arch images, cosign signing, and helm chart publishing"
```

---

# Phase 11 — Docs

## Task 11.1: README, installation, annotations

**Files:**
- Modify: `README.md`
- Create: `docs/installation.md`
- Create: `docs/annotations.md`
- Create: `docs/troubleshooting.md`

- [ ] **Step 1: Replace `README.md`**

```markdown
# stubby

> Kubernetes mutating webhook that swaps pod container images for dummy
> backend/frontend images when the pod carries a `stubby.io/type` annotation.

Useful as a placeholder while the real image isn't built yet — your team
can stand up a Deployment / Service / Ingress before the application code
is ready, and the cluster is happy because the pods come up green.

## Quick start

```bash
helm repo add stubby https://kauemendes.github.io/stubby
helm install stubby stubby/stubby --namespace stubby-system --create-namespace
```

```yaml
apiVersion: apps/v1
kind: Deployment
metadata: { name: orders-api }
spec:
  replicas: 1
  selector: { matchLabels: { app: orders-api } }
  template:
    metadata:
      labels: { app: orders-api }
      annotations:
        stubby.io/type: backend
        stubby.io/app-name: "Orders API"
    spec:
      containers:
        - name: orders
          image: ghcr.io/example/orders-api:notbuilt
```

The pod boots immediately with the `stubby-dummy-backend` image. Hit
`/health`, `/openapi.json`, or `/docs` — they all answer. When your real
image is ready, change `stubby.io/type` to `off` and reapply.

## Docs

- [Installation](docs/installation.md)
- [Annotation reference](docs/annotations.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Design spec](docs/superpowers/specs/2026-05-20-stubby-design.md)

## License

MIT
```

- [ ] **Step 2: Write `docs/installation.md`**

```markdown
# Installation

## Prerequisites

- Kubernetes ≥ v1.29
- Helm ≥ 3.13
- Optional: cert-manager (if `tls.mode=cert-manager`)

## Install via Helm

```bash
helm repo add stubby https://kauemendes.github.io/stubby
helm install stubby stubby/stubby \
  --namespace stubby-system --create-namespace
```

## TLS

`stubby` ships with two TLS modes selected via `values.tls.mode`:

| Mode | When to use |
|------|-------------|
| `self-signed` (default) | No extra prerequisites; a pre-install Job generates a CA and patches the `caBundle`. |
| `cert-manager` | If you already run cert-manager in the cluster. |

Switch with:

```bash
helm upgrade --install stubby stubby/stubby --set tls.mode=cert-manager
```

## Uninstall

```bash
helm uninstall stubby -n stubby-system
kubectl delete mutatingwebhookconfiguration stubby
kubectl delete namespace stubby-system
```

## Air-gapped / private images

Set `image.repository` and `dummyImages.{backend,frontend}` to your mirror:

```bash
helm install stubby ./stubby \
  --set image.repository=registry.internal/stubby-webhook \
  --set dummyImages.backend=registry.internal/stubby-dummy-backend:1.0.0 \
  --set dummyImages.frontend=registry.internal/stubby-dummy-frontend:1.0.0
```
```

- [ ] **Step 3: Write `docs/annotations.md`**

```markdown
# Annotation reference

All annotations live on the **Pod** (or `spec.template.metadata.annotations`
of the Deployment/StatefulSet/Job/etc.).

| Annotation | Type | Default | Meaning |
|---|---|---|---|
| `stubby.io/type` | `backend` \| `frontend` \| `off` | absent → skip | Selects dummy image. `off` (or absent) disables injection. |
| `stubby.io/app-name` | string | `metadata.name` | Display name in OpenAPI title and HTML page. |
| `stubby.io/port` | u16 | `8080` (backend), `80` (frontend) | Container port that the dummy listens on. |
| `stubby.io/image-override` | `registry/image:tag` | (none) | Use your own image instead of the official dummy. |
| `stubby.io/skip-containers` | CSV | (none) | Container names within the pod to leave untouched. |

## Skipped sidecars

Containers whose `name` starts with `istio-`, `linkerd-`, `vault-`, or
`cilium-` are always skipped.

## Examples

```yaml
metadata:
  annotations:
    stubby.io/type: backend
    stubby.io/app-name: "Orders API"
    stubby.io/port: "9000"
```

```yaml
metadata:
  annotations:
    stubby.io/type: backend
    stubby.io/image-override: ghcr.io/me/my-custom-dummy:dev
```
```

- [ ] **Step 4: Write `docs/troubleshooting.md`**

```markdown
# Troubleshooting

## Webhook isn't mutating

1. Confirm the webhook is up:
   ```
   kubectl get pods -n stubby-system
   kubectl logs -n stubby-system deploy/stubby
   ```
2. Confirm the `MutatingWebhookConfiguration` exists and has a non-empty
   `caBundle`:
   ```
   kubectl get mutatingwebhookconfiguration stubby -o yaml | grep caBundle
   ```
3. Check `namespaceSelector` — the default excludes `kube-system` and any
   namespace labeled `stubby.io/exclude=true`.

## `tls: bad certificate`

The `caBundle` and the secret are out of sync. Re-run the bootstrap Job:

```
kubectl delete job -n stubby-system stubby-tls-bootstrap || true
helm upgrade stubby ./stubby --reuse-values
```

## Metrics

```
kubectl port-forward -n stubby-system svc/stubby 9090:443
curl -k https://localhost:9090/metrics
```
```

- [ ] **Step 5: Commit**

```bash
git add README.md docs/installation.md docs/annotations.md docs/troubleshooting.md
git commit -m "docs: README and reference docs (installation, annotations, troubleshooting)"
```

---

## Done — what to do after the last task

1. Tag `v0.1.0` and push the tag — the release workflow publishes images + chart.
2. Update `README.md` quickstart with the actual `helm repo add` URL once `gh-pages` is live.
3. File issues for roadmap items declared as non-goals: worker dummies, gRPC dummies, real-image-arrived detection.
