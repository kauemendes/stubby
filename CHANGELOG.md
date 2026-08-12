# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] — 2026-08-12

Pod Security "restricted" compliance fixes for mutated pods, plus an
experimental reactive auto-rescue controller.

### Changed

- **Dummy frontend is now a Rust/`axum` binary** instead of nginx. It
  renders the HTML page in memory (no writes to the root filesystem) and
  listens on the injected port, which fixes two things that broke Pod
  Security "restricted" workloads:
  - `stubby.io/port` is now honoured by the frontend. Previously nginx
    always listened on `80`, so any other port (including `8080`, the
    backend default) failed the probes with `connection refused` and
    CrashLooped.
  - a mutated frontend now survives `readOnlyRootFilesystem: true` +
    `runAsNonRoot` + `drop: [ALL]`. The old nginx entrypoint wrote the
    rendered page under `/usr/share/nginx/html` at startup and died on a
    read-only rootfs.
  The frontend image is now distroless/non-root, matching the backend.

### Added

- **`STUBBY_PORT` injection** — the webhook injects `STUBBY_PORT` (from
  `stubby.io/port`) so both dummy binaries bind exactly the port the
  `containerPort`, probes, and Service target. The backend also honours
  it (previously it only listened on `8080` unless
  `STUBBY_BACKEND_LISTEN` was set).
- **Orphaned config is stripped so pods boot green** — when injecting a
  dummy, the webhook now removes `envFrom` and `volumeMounts` from
  mutated containers and prunes now-orphaned `secret`/`configMap`/
  `projected` pod `volumes`. Without this, a pod referencing not-yet-
  created config left `ImagePullBackOff` only to land in
  `CreateContainerConfigError`. Opt out with the new
  `stubby.io/keep-env-from` and `stubby.io/keep-volumes` annotations.
  Volumes still used by a skipped sidecar or init container are kept.
- **e2e regression cases** for each of the three defects above
  (`test/e2e/cases/defect{1,2,3}-*.sh`) plus example manifests.
- **Experimental auto-rescue controller** (`controller.enabled`, off by
  default) — reacts to `ImagePullBackOff` on pods annotated
  `stubby.io/auto-rescue: "true"`, swaps the image for a dummy in place,
  and reverts once the real image is published to the registry. In-place
  patch only (GitOps-safe); registry checks use the pod's imagePullSecrets.

### Fixed

- **Docs** — the injected liveness/readiness probes target `/health`
  and `/ready` (not `/healthz`/`/readyz`, which are the webhook's *own*
  endpoints). Corrected in `README.md`; `docs/annotations.md` and
  `docs/troubleshooting.md` updated for the frontend rewrite and the new
  stripping behaviour.

## [0.1.0] — 2026-05-22

The first cut of `stubby`. A Kubernetes Mutating Admission Webhook in
Rust that swaps pod container images for ready-to-go dummy backends
and frontends.

### Added

- **Webhook crate** (`stubby-webhook`) — `axum` + `axum-server` TLS
  listener serving:
  - `POST /mutate` — accepts an `AdmissionReview`, returns a JSONPatch
    that overlays image, ports, probes, env, and resources on each
    matched container.
  - `GET /healthz`, `GET /readyz` — liveness/readiness probes.
  - `GET /metrics` — Prometheus exposition format with the
    `stubby_admissions_total{type, decision}` counter, where
    `decision ∈ {inject, skip, error}`.
- **Dummy backend** (`stubby-dummy-backend`) — `axum` HTTP server with
  `/health`, `/ready`, `/openapi.json`, `/docs`, and a JSON catch-all
  that always returns `{"status":"dummy", ...}`.
- **Dummy frontend** (`stubby-dummy-frontend`) — nginx serving a
  single HTML page with XSS-safe templated app name. The escape set
  is identical to the Rust `render_index` path so both runtimes agree.
- **Annotations** — `stubby.io/type` (`backend|frontend|off`),
  `stubby.io/app-name`, `stubby.io/port`, `stubby.io/image-override`,
  `stubby.io/skip-containers`.
- **Sidecar safety** — containers whose name begins with `istio-`,
  `linkerd-`, `vault-`, or `cilium-` are always skipped.
- **Helm chart** (`charts/stubby`) — Deployment, Service, MWC, plus
  `tls.mode: self-signed` (cert generated in-template via
  `genCA`/`genSignedCert`, persisted across upgrades with `lookup`)
  and `tls.mode: cert-manager` (Issuer + Certificate).
- **Pod Security restricted** profile compliance — `runAsNonRoot`,
  `readOnlyRootFilesystem`, `drop ALL` capabilities, seccomp
  `RuntimeDefault`.
- **TLS hot reload** — webhook reloads its certificate every 60s
  without restart.
- **Graceful shutdown** — SIGTERM/SIGINT drain with a 30-second
  grace period.
- **Multi-arch images** — `linux/amd64` and `linux/arm64`, published
  to GHCR.
- **Signed releases** — `cosign` keyless signatures + SBOM + SLSA
  provenance attestations, against the GitHub Actions OIDC issuer.
- **CI** — `cargo fmt`, `cargo clippy -D warnings`, `cargo test`,
  `cargo llvm-cov --fail-under-lines 80`, `helm lint`, `helm unittest`,
  and an e2e matrix on Kubernetes 1.29 / 1.30 / 1.31.
- **End-to-end harness** — `test/e2e/run.sh` spins up a kind cluster,
  builds and loads the three images, helm-installs in self-signed
  mode, and runs case scripts that assert both the backend and the
  frontend got injected correctly.
- **Documentation** — `README.md`, `docs/installation.md`,
  `docs/annotations.md`, `docs/troubleshooting.md`, plus the design
  spec and implementation plan under `docs/superpowers/`.

### Security

- `failurePolicy: Ignore` is the chart default — a stubby outage does
  not block pod creation.
- The webhook ServiceAccount holds **no** RBAC verbs.
- AdmissionReview bodies are bounded to 8 MiB; malformed bodies still
  return HTTP 200 with an `AdmissionResponse.status` so the contract
  holds.

[Unreleased]: https://github.com/kauemendes/stubby/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/kauemendes/stubby/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/kauemendes/stubby/releases/tag/v0.1.0
