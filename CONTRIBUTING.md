# Contributing to stubby

Thanks for considering a contribution. `stubby` is a learning-grade
Kubernetes lab project, but the same conventions that keep production
projects healthy apply here too: tests-first, small focused commits,
and an opinionated CI gate.

## Table of contents

- [Code of conduct](#code-of-conduct)
- [Reporting bugs](#reporting-bugs)
- [Suggesting changes](#suggesting-changes)
- [Local development](#local-development)
- [Coding conventions](#coding-conventions)
- [Commit messages](#commit-messages)
- [Pull-request process](#pull-request-process)
- [Releases](#releases)

## Code of conduct

This project adheres to the [Contributor Covenant](CODE_OF_CONDUCT.md).
By participating you agree to uphold it. Report unacceptable behaviour
to **kaue.mendes@gmail.com**.

## Reporting bugs

Use the [Bug report](.github/ISSUE_TEMPLATE/bug_report.yml) issue
template. Include:

- the chart version (`helm list -A`),
- the webhook image tag,
- the Kubernetes server version (`kubectl version -o yaml`),
- the pod or AdmissionReview that triggered the issue,
- redacted logs from `kubectl logs -n stubby-system -l app.kubernetes.io/name=stubby`.

If the bug is security-related, **do not** open a public issue. See
[SECURITY.md](SECURITY.md) for private disclosure.

## Suggesting changes

For non-trivial changes (new annotation, new injected field, breaking
chart change), open a [Feature
request](.github/ISSUE_TEMPLATE/feature_request.yml) first so we can
agree on shape before code is written. Small fixes can skip the issue
and go straight to a PR.

## Local development

### Prerequisites

| Tool             | Version       | Why                          |
|------------------|---------------|------------------------------|
| Rust toolchain   | **1.85.0**    | Pinned via `rust-toolchain.toml`. |
| `cargo-llvm-cov` | **0.6.21**    | Coverage gate (`fail-under-lines 80`). |
| Helm             | `v3.16.4`+    | Chart linting + install.     |
| `helm-unittest`  | **v1.0.3**    | Chart unit tests.            |
| Docker           | `24`+         | Image builds; `buildx` for multi-arch. |
| `kind`           | `v0.24.0`+    | Local e2e clusters.          |

Install commands:

```bash
rustup toolchain install 1.85.0
cargo install cargo-llvm-cov --locked --version 0.6.21
helm plugin install --version v1.0.3 https://github.com/helm-unittest/helm-unittest
```

### Workflow

```bash
git clone https://github.com/kauemendes/stubby
cd stubby

# Fast iteration on code:
cargo fmt --all
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test  --workspace

# Coverage (mirrors CI):
cargo llvm-cov --package stubby-webhook --fail-under-lines 80

# Helm:
helm lint     charts/stubby
helm unittest charts/stubby

# End-to-end (kind cluster, full install, case scripts):
bash test/e2e/run.sh           # destroys the cluster at the end
KEEP=1 bash test/e2e/run.sh    # leaves it up so you can `kubectl` around
```

### Repository layout

```
crates/
  webhook/         the admission webhook (axum, TLS, JSONPatch)
  dummy-backend/   axum HTTP server used as the backend dummy
  dummy-frontend/  nginx + HTML template used as the frontend dummy
charts/stubby/     Helm chart, schema, unittest suites
docker/            Dockerfiles for the three images
docs/              user-facing documentation
examples/          ready-to-apply manifest examples
test/              kind harness + e2e case scripts
.github/workflows/ CI + release automation
```

## Coding conventions

- **Rust** — `cargo fmt --check` and `cargo clippy -D warnings` are
  required to pass. Avoid `unwrap()` outside tests; prefer `?` with
  `anyhow::Context`.
- **No `unsafe`** anywhere in `crates/webhook`.
- **Errors** — never use `anyhow::Error` in library APIs; bubble
  domain-specific errors and let `main` translate them.
- **Tests** — TDD red → green → refactor. New behaviour gets a failing
  test first, then the change that makes it pass.
- **Helm** — every templated file gets at least one assertion in
  `charts/stubby/tests/`. Prefer `containsDocument` over `isKind` for
  multi-doc templates, and explicit `documentIndex:` qualifiers.
- **Chart values** — every new value must show up in
  `values.schema.json` so `helm install --dry-run` catches typos.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/):

```
<type>(<scope>): <imperative one-liner>

<body explaining WHY, not what>

Co-Authored-By: ...
```

Types used in the log: `feat`, `fix`, `ci`, `docs`, `test`, `refactor`,
`build`, `chore`. Scopes are usually crate names (`webhook`,
`dummy-backend`, …) or chart sections (`chart`).

Examples:

```
feat(webhook): expose Prometheus /metrics endpoint and admission counters
fix(chart): pin alpine/openssl tag and allow apk to write rootfs
docs(examples): backend, frontend, and off-mode manifests
```

## Pull-request process

1. Branch off `main`. Name the branch after the change (`feat/...`,
   `fix/...`, etc.).
2. Open a PR against `main`. Fill in the [PR
   template](.github/PULL_REQUEST_TEMPLATE.md).
3. CI runs three jobs in parallel: Rust (fmt + clippy + tests +
   coverage), Helm (lint + unittest), and the e2e matrix across
   Kubernetes 1.29 / 1.30 / 1.31. All three must be green.
4. Address review comments by pushing new commits (not amends). The PR
   will be squashed or merged depending on commit hygiene.
5. Once merged, the branch is auto-deleted.

## Releases

Releases are tag-driven: pushing a `vX.Y.Z` tag triggers the release
workflow, which builds multi-arch images, signs them with `cosign`
keyless, generates SBOM + provenance attestations, and publishes the
Helm chart to the `gh-pages` branch via `chart-releaser-action`.

See [CHANGELOG.md](CHANGELOG.md) for the changelog format. Every
release PR updates the `## [Unreleased]` section into a dated release.
