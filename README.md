<p align="center">
  <img src="docs/logo.svg" alt="stubby logo" width="120">
</p>

<h1 align="center">stubby</h1>

<p align="center">
  <em>A Kubernetes mutating admission webhook that swaps pod container images for
  ready-to-go dummy backends and frontends — so you can stand up
  Deployments, Services, and Ingresses before the real app is built.</em>
</p>

<p align="center">
  <a href="https://github.com/kauemendes/stubby/actions/workflows/ci.yaml"><img alt="CI" src="https://github.com/kauemendes/stubby/actions/workflows/ci.yaml/badge.svg"></a>
  <a href="LICENSE"><img alt="License: MIT" src="https://img.shields.io/badge/license-MIT-blue.svg"></a>
  <a href="https://github.com/kauemendes/stubby/pkgs/container/stubby-webhook"><img alt="GHCR" src="https://img.shields.io/badge/ghcr-stubby--webhook-blueviolet?logo=docker"></a>
  <img alt="Rust 1.85" src="https://img.shields.io/badge/rust-1.85-orange?logo=rust">
  <img alt="K8s 1.29+" src="https://img.shields.io/badge/k8s-1.29%20%7C%201.30%20%7C%201.31-blue?logo=kubernetes">
</p>

---

## Contents

- [Why stubby?](#why-stubby)
- [How it works](#how-it-works)
- [Quick start](#quick-start)
- [Annotation reference](#annotation-reference)
- [What gets injected](#what-gets-injected)
- [Configuration](#configuration)
- [Observability](#observability)
- [Security](#security)
- [Documentation](#documentation)
- [Development](#development)
- [Contributing](#contributing)
- [License](#license)

## Why stubby?

You're building a new service. The platform team wants the Deployment,
Service, and Ingress merged on Monday. The application image won't be
ready until Friday. With `stubby` you ship the manifests on Monday
anyway — every pod boots green, every readiness probe answers `ok`,
every `/openapi.json` returns a valid (placeholder) schema, and the
frontend pod serves an obvious "I'm a dummy" landing page.

When the real image is ready, flip the `stubby.io/type` annotation to
`off` and reapply. No template churn, no separate placeholder chart,
no `nginx:hello` hand-rolled image to maintain.

## How it works

```text
┌────────────────┐   AdmissionReview     ┌─────────────────────┐
│  kube-apiserver│ ────────────────────▶ │  stubby-webhook     │
│                │      pod CREATE       │  (axum, TLS, Rust)  │
│                │ ◀──────────────────── │                     │
└────────────────┘   JSONPatch (base64)  └─────────────────────┘
        │                                          │
        ▼                                          │
┌────────────────┐                                 │
│ Pod admitted   │ ◀───────────────────────────────┘
│  - image: stubby-dummy-backend (or -frontend)
│  - ports, probes, env, resources overlaid
└────────────────┘
```

On every `Pod CREATE`, the API server forwards an `AdmissionReview` to
the webhook. If the pod's annotations declare a `stubby.io/type`, the
webhook returns a JSONPatch that rewrites the container's `image`,
`ports`, probes, env, and resources to point at one of two pre-built
images:

| Type     | Image                              | Behavior |
|----------|------------------------------------|----------|
| `backend`  | `stubby-dummy-backend`           | `axum` HTTP server exposing `/health`, `/ready`, `/openapi.json`, `/docs`. Catch-all returns `{"status":"dummy"}`. |
| `frontend` | `stubby-dummy-frontend`          | `axum` HTTP server rendering a single HTML page (in memory) with an XSS-safe templated app name, plus `/health` and `/ready`. |
| `off`      | _(no injection)_                 | Skip the pod entirely. Useful once the real image lands. |

Containers whose name starts with a known sidecar prefix
(`istio-`, `linkerd-`, `vault-`, `cilium-`) are always skipped, and you
can extend that list per-pod via `stubby.io/skip-containers`.

`failurePolicy: Ignore` is the chart default — a stubby outage never
blocks pod creation.

## Quick start

### Install the chart

```bash
helm repo add stubby https://kauemendes.github.io/stubby
helm install stubby stubby/stubby \
  --namespace stubby-system --create-namespace
```

The chart ships with `tls.mode: self-signed` — a CA + leaf certificate
are generated at template time, persisted into the cluster via
`lookup`, and reused across upgrades. For production, switch to
[`tls.mode: cert-manager`](docs/installation.md#tls-modes).

### Annotate a pod

```yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  name: orders-api
spec:
  replicas: 1
  selector:
    matchLabels: { app: orders-api }
  template:
    metadata:
      labels: { app: orders-api }
      annotations:
        stubby.io/type: backend
        stubby.io/app-name: "Orders API"
        stubby.io/port: "8080"
    spec:
      containers:
        - name: orders
          image: ghcr.io/example/orders-api:not-built-yet
```

Apply. The pod starts green within seconds. `curl
http://orders-api:8080/openapi.json` returns a synthetic OpenAPI doc
titled `Orders API (dummy)`.

When the real image lands, replace the annotation:

```yaml
annotations:
  stubby.io/type: "off"
```

`stubby` skips the pod and your real image runs.

## Annotation reference

| Annotation                  | Required | Default     | Description |
|-----------------------------|----------|-------------|-------------|
| `stubby.io/type`            | ✅       | _none_      | One of `backend`, `frontend`, `off`. |
| `stubby.io/app-name`        |          | pod name    | Displayed on the dummy frontend page and embedded in the dummy OpenAPI title. |
| `stubby.io/port`            |          | `8080`/`80` | HTTP port the dummy listens on. |
| `stubby.io/image-override`  |          | _none_      | Use a fully-qualified image reference instead of the bundled dummies. The rest of the injection (ports/probes/env) still applies. |
| `stubby.io/skip-containers` |          | _empty_     | Comma-separated container names that must not be mutated, in addition to the built-in sidecar prefixes. |
| `stubby.io/keep-env-from`   |          | `false`     | Keep the container's `envFrom` instead of stripping it. |
| `stubby.io/keep-volumes`    |          | `false`     | Keep `volumeMounts` and orphaned `volumes` instead of pruning them. |

Full table with examples: [docs/annotations.md](docs/annotations.md).

## What gets injected

The JSONPatch overlays these fields on each matched container:

1. `image` — `stubby.io/image-override` if set, else the type-specific
   default from the chart values (`dummyImages.backend` or
   `dummyImages.frontend`).
2. `ports` — a single `containerPort` matching `stubby.io/port`.
3. `livenessProbe` / `readinessProbe` — `httpGet` on `/health` and
   `/ready` respectively, scheme `HTTP`, both on `stubby.io/port`.
4. `env` — appends `STUBBY_APP_NAME=<stubby.io/app-name>` and
   `STUBBY_PORT=<stubby.io/port>`. The dummy binaries listen on
   `STUBBY_PORT`, so the container always binds the exact port the
   probes and Service target.
5. `resources` — defaults applied **only** when the manifest doesn't
   already define them, so your real workload's requests/limits win.

and removes plumbing the dummy doesn't need — and that would otherwise
keep the pod red while the real app is unprovisioned:

6. `envFrom` is dropped (a dangling `secretRef`/`configMapRef` causes
   `CreateContainerConfigError`). Opt out with
   `stubby.io/keep-env-from: "true"`.
7. `volumeMounts` are dropped, and any now-orphaned `secret` /
   `configMap` / `projected` pod `volumes` are pruned (a missing backing
   object wedges the pod in `ContainerCreating`). Volumes still mounted
   by a skipped sidecar or an init container are left intact. Opt out
   with `stubby.io/keep-volumes: "true"`.
8. `command` / `args` are removed if present, so the dummy's own
   entrypoint runs.

JSON Patch operations use `add` (not `replace`) for optional fields so
previously-absent fields don't fail with RFC 6902 conflicts.

## Configuration

The Helm chart is the supported install path. Most useful values:

| Key                            | Default                                          | Notes |
|--------------------------------|--------------------------------------------------|-------|
| `replicaCount`                 | `2`                                              | Two replicas for HA; both behind the webhook Service. |
| `tls.mode`                     | `self-signed`                                    | `self-signed` (in-template) or `cert-manager`. |
| `tls.selfSigned.validityDays`  | `3650`                                           | Honoured on first install; `lookup` reuses the cert thereafter. |
| `webhook.failurePolicy`        | `Ignore`                                         | API server skips the webhook if stubby is down. |
| `webhook.namespaceSelector`    | excludes `kube-system` and `stubby.io/exclude=true` | Tweak to narrow scope. |
| `dummyImages.backend`          | `ghcr.io/kauemendes/stubby-dummy-backend:0.1.0`  | Override to vendor your own dummy. |
| `dummyImages.frontend`         | `ghcr.io/kauemendes/stubby-dummy-frontend:0.1.0` | Same as above. |
| `resources`                    | `cpu: 50m–200m`, `memory: 64–128Mi`              | Burstable. |
| `logLevel`                     | `info`                                           | Maps to `RUST_LOG`; accepts module filters. |

Full schema: [`charts/stubby/values.yaml`](charts/stubby/values.yaml)
and [`values.schema.json`](charts/stubby/values.schema.json).

## Observability

- **Metrics** — `GET /metrics` exposes the Prometheus counter
  `stubby_admissions_total{type, decision}` where `decision ∈ {inject,
  skip, error}`. The endpoint shares the same TLS listener as `/mutate`.
- **Logs** — structured JSON via `tracing`, filtered by `RUST_LOG`.
  Sample queries:
  - `{kubernetes_namespace="stubby-system"} | json` (Loki)
  - `kubernetes.labels.app=stubby AND level=ERROR` (Elastic)
- **Health** — `/healthz` and `/readyz` both serve over TLS; the chart
  wires them into liveness/readiness probes.

See [docs/troubleshooting.md](docs/troubleshooting.md) for the
runbooks.

## Security

- Pod Security "restricted" profile compliant — `runAsNonRoot`,
  `readOnlyRootFilesystem`, `drop ALL` capabilities, seccomp
  `RuntimeDefault`. This holds for the webhook's own pods **and** for the
  pods it mutates: both dummy images are distroless, run as non-root, and
  render everything in memory (no writes to the root filesystem). The
  dummies default to port `8080` precisely so they never need to bind a
  privileged port; to expose `:80` externally, map a Service `port: 80`
  to `targetPort: http`. (A distroless non-root process can't bind `<1024`
  without file/ambient capabilities, so listening on `80` in-container
  requires running that pod as root.)
- TLS-only ingress; certs hot-reloaded every 60s without restart.
- `failurePolicy: Ignore` is the safe default — failed-open, not
  failed-closed.
- Multi-arch container images are signed with `cosign` keyless and
  ship SBOM + SLSA provenance attestations.
- Disclose vulnerabilities privately: see [SECURITY.md](SECURITY.md).

## Documentation

- [Installation](docs/installation.md) — prerequisites, TLS modes,
  air-gap notes, uninstall procedure.
- [Annotation reference](docs/annotations.md) — every annotation, all
  fields, with examples for each type.
- [Troubleshooting](docs/troubleshooting.md) — runbooks for "webhook
  not mutating", x509 cert mismatches, decode errors, OOM, etc.
- [Design spec](docs/superpowers/specs/2026-05-20-stubby-design.md) —
  the architectural rationale behind the v1 design.
- [Implementation plan](docs/superpowers/plans/2026-05-20-stubby-implementation.md)
  — phase-by-phase breakdown of how v1 was built.

## Development

```bash
git clone https://github.com/kauemendes/stubby
cd stubby

# Lint + tests
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test  --workspace
helm lint        charts/stubby
helm unittest    charts/stubby

# End-to-end (requires kind, docker, helm)
bash test/e2e/run.sh         # creates a kind cluster, builds images, helm-installs
KEEP=1 bash test/e2e/run.sh  # ... and leaves the cluster up afterwards
```

Detailed contribution flow: [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributing

Contributions are welcome. The short version:

1. Open an issue describing the change.
2. Fork → branch → push → PR. Match the [Conventional
   Commits](https://www.conventionalcommits.org/) style already used
   in the log (`feat:`, `fix:`, `ci:`, `docs:`, `test:`, `refactor:`).
3. `cargo fmt` + `cargo clippy -D warnings` + `helm unittest` must all
   pass; CI enforces them.
4. Add or update a test for any behaviour change.

The long version, with environment setup, commit-message guidance, and
review process: [CONTRIBUTING.md](CONTRIBUTING.md).

## License

[MIT](LICENSE). Copyright © 2026 Kauê Mendes.
