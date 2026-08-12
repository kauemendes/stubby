# Design — Reactive auto-rescue controller (experimental)

**Status:** approved (brainstorming, 2026-08-10)
**Scope:** one implementation plan.

## Problem

Today stubbing is opt-in and manual: you set `stubby.io/type` and stubby
swaps the image at admission. The Achilles heel is that someone has to
*remember* to remove the annotation once the real image lands — forget,
and stubby silently keeps serving a dummy over a perfectly good image.

We want the inverse, dynamic flow: deploy the **real** image reference
from day one (e.g. `:v1.2.3`). While CI hasn't published that tag the pod
sits in `ImagePullBackOff`; stubby should step in automatically, and then
step back out on its own once the tag exists in the registry.

An admission webhook cannot do this: it fires on `CREATE`, can't reach
the registry, and has no way to observe a later `ImagePullBackOff`. So
this is a **separate, reactive controller** that watches pod status.

## Chosen model (decisions from brainstorming)

1. **Trigger:** CI publishes the expected tag. The workload already
   points at the real image; the controller reacts to pull failures.
2. **Opt-in:** a new `stubby.io/auto-rescue: "true"` annotation. The
   existing proactive `stubby.io/type` path is unchanged and additive.
   `stubby.io/type` (default `backend`) and `stubby.io/port` act as
   *hints* for which dummy to use.
3. **Revert detection:** the controller checks the registry for the
   original image (using the pod's `imagePullSecrets`) and reverts when
   it appears. No flapping.
4. **Actuation:** in-place — the controller patches only the pod's
   `image` field (the sole mutable container field on a live pod). No
   rollout, no edits to the Deployment, so it does not fight GitOps
   controllers (Argo/Flux) that own the workload spec.

## Non-goals (this plan)

- Leader election (run `replicaCount: 1`; documented).
- `stubby.io/expires-at` and the proactive guard-rails (WARN when
  injecting over an image that already exists).
- Changing the webhook's proactive behaviour.
- Rescuing failures other than image-pull (`CreateContainerConfigError`,
  etc. are the webhook's job at admission).
- Init-container image-pull failures: v1 only inspects
  `status.container_statuses` (app containers), not
  `status.init_container_statuses` — a documented experimental
  limitation.

## Architecture

New workspace crate `crates/controller`, binary `stubby-controller`,
deployed as its own **optional** Deployment (`controller.enabled: false`
by default). It uses `kube-rs` to watch `Pods` and reconciles those
annotated `stubby.io/auto-rescue: "true"`.

```
                          watch Pods (auto-rescue=true)
   ┌───────────────┐  ───────────────────────────────▶  ┌────────────────────┐
   │ kube-apiserver│                                     │ stubby-controller  │
   │               │  ◀── patch pod .image (stub/revert) │ (kube-rs, reconcile│
   └───────────────┘                                     │  + registry check) │
          ▲                                              └────────────────────┘
          │ read imagePullSecrets                                   │
          └────────────────────── registry HEAD manifest ──────────┘
```

The controller is intentionally separate from the webhook: different
failure domain, different RBAC, and it can be shipped/toggled
independently while it matures.

## Reconcile logic (per rescuable container)

A container is *rescuable* if its name is not a skipped sidecar
(reuse `ALWAYS_SKIP_PREFIXES` from the webhook — factor it into a shared
spot or duplicate the small list with a test asserting parity).

State is derived from pod annotations + container statuses:

- **Not rescued** and the container's `state.waiting.reason` is
  `ImagePullBackOff` or `ErrImagePull`:
  → **STUB**.
    1. Record the original image once, into annotation
       `stubby.io/original-image` as a JSON object `{container: image}`
       (survives controller restarts; supports multi-container pods).
    2. Patch `spec.containers[i].image` to the dummy image
       (`dummyImages.backend` unless `stubby.io/type: frontend`).
    3. Set `stubby.io/rescued-at` (RFC3339) for observability.
    4. Increment the `stubby_rescued_pods` gauge and
       `stubby_rescue_actions_total{action="stub"}` (Events deferred; see
       Observability).

- **Rescued** (has `stubby.io/original-image`):
  → every `checkInterval`, for each recorded original image, run the
    **registry check**. When *all* recorded originals are present:
    → **REVERT**: patch each `image` back to its original, delete the
      `stubby.io/original-image` / `stubby.io/rescued-at` annotations,
      decrement the gauge, increment
      `stubby_rescue_actions_total{action="revert"}`.

Patches use a JSON-merge/strategic patch with the observed
`resourceVersion` for optimistic concurrency; a conflict just requeues.
The `stubby.io/original-image` annotation is the idempotency guard — a
container is never double-stubbed.

### Port / probe limitation (accepted; experimental)

Only `image` is mutable on a live pod, so the rescued container keeps its
original `ports`, `probes`, `env`, and `volumeMounts`. The dummy listens
on its default port (`8080` backend / `80` frontend), or on `STUBBY_PORT`
**if that env var is already declared on the container** (the real app
ignores it). Therefore:

- Pods with **no probes**, or probes/`containerPort` on the dummy's port,
  rescue cleanly.
- A probe targeting a different port without a pre-declared `STUBBY_PORT`
  will fail and CrashLoop the dummy.

This is why the feature ships **experimental**. The documented escape
hatch is to declare `STUBBY_PORT` in the container's own `env` (one line,
declarative, ignored by the real image). `stubby.io/port` remains a hint
for readers and future work. (A future non-experimental mode could flip
the workload template so the webhook does a full-fidelity mutation, at
the cost of a rollout and GitOps friction — explicitly out of scope now.)

## Registry check

Input: an image reference string + the pod's pull secrets.

1. Parse the reference into `registry / repository : tag-or-digest`
   (default registry `docker.io`, default tag `latest`).
2. Assemble auth: read `spec.imagePullSecrets` **and** the pod's
   ServiceAccount `imagePullSecrets`; load each `kubernetes.io/dockerconfigjson`
   Secret and index credentials by registry host.
3. Issue a manifest `HEAD`/`GET` for the reference via a registry client
   crate (`oci-client`, née `oci-distribution`). `200` ⇒ available.
4. Any error (network, 401/403, 404) ⇒ treat as "not yet available",
   log at `debug`, and retry next cycle. Never block or crash on it.

Registry auth failures must never wedge the loop — a stubbed pod simply
stays stubbed until the next successful check.

## Helm chart

Everything gated on `.Values.controller.enabled` (default `false`):

- `templates/controller-deployment.yaml`
- `templates/controller-rbac.yaml` — `ServiceAccount`, `ClusterRole`
  (`pods`: get/list/watch/patch; `secrets`: get — required to read pull
  secrets), `ClusterRoleBinding`. No `events` verb (Events are deferred;
  see Observability).
- New values:
  - `controller.enabled: false`
  - `controller.image.repository` / `.tag` / `.pullPolicy`
  - `controller.replicaCount: 1`
  - `controller.checkIntervalSeconds: 60`
  - `controller.resources` (small burstable default)

`secrets: get` is cluster-wide by default because auto-rescue can be used
in any namespace. This is sensitive and is called out in the docs; a
`controller.watchNamespaces` value can later narrow both the watch and a
namespaced `Role` instead of the `ClusterRole`. **No chart version bump
without asking the maintainer.**

Controller pod itself runs Pod Security "restricted": distroless
non-root, read-only rootfs, `drop: [ALL]`, seccomp `RuntimeDefault`.

## Observability

- Gauge `stubby_rescued_pods` — currently-stubbed pods.
- Counter `stubby_rescue_actions_total{action="stub"|"revert"}`.
- Kubernetes Events on the pod (`Stubbed`, `Reverted`, `RescueFailed`) are
  **deferred (future work)** for the experimental controller; for now,
  observability is via structured logs plus the two Prometheus metrics
  above. Consequently the RBAC below does not request the `events` verb.
- Structured JSON logs via `tracing`, consistent with the webhook.

## Testing

**Unit (pure functions, no cluster):**
- Decision function: `(container statuses, annotations, cfg) → Action`
  (Stub / Revert / Nothing) across: pull-backoff→stub, already-rescued +
  original-present→revert, already-rescued + original-absent→nothing,
  sidecar skipped, multi-container mixed states.
- Image-reference parsing (registry/repo/tag/digest, defaults).
- Pull-secret/auth assembly from `dockerconfigjson` (fixture secrets).
- Registry response → available/not-available mapping.

**e2e (`test/e2e/cases/`):**
- Stand up a local OCI registry reachable from the kind cluster.
- Deploy an `auto-rescue` pod pointing at a tag that does **not** exist
  yet → assert the controller stubs it and the pod reaches `Running` on
  the dummy image.
- Push/import the "real" image under that tag → assert the controller
  reverts and the pod runs the original image again.
- Requires enabling the controller in the e2e `helm upgrade` and building
  a fourth image (`stubby-controller`). Gate the case on
  `controller.enabled`.

## Acceptance criteria

- With `controller.enabled=true`, a pod annotated
  `stubby.io/auto-rescue: "true"` whose image tag is missing is
  automatically stubbed and reaches `Running`.
- Once the tag exists in the registry, the same pod is reverted to its
  original image without manual action.
- The proactive webhook path is byte-for-byte unchanged.
- Sidecars are never rescued.
- Green: `cargo fmt --all`, `cargo clippy --workspace --all-targets
  --all-features -- -D warnings`, `cargo test --workspace`,
  `helm lint charts/stubby`, `helm unittest charts/stubby`,
  `bash test/e2e/run.sh`.
- Conventional commits; CHANGELOG updated; no chart version bump without
  asking.
