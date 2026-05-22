# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

[Unreleased]: https://github.com/kauemendes/stubby/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/kauemendes/stubby/releases/tag/v0.1.0
