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
- [Implementation plan](docs/superpowers/plans/2026-05-20-stubby-implementation.md)

## How it works

`stubby` is a [Mutating Admission Webhook](https://kubernetes.io/docs/reference/access-authn-authz/extensible-admission-controllers/).
On every Pod CREATE the API server forwards the AdmissionReview to a TLS
endpoint exposed by stubby. If the pod's annotations declare a
`stubby.io/type`, stubby returns a JSONPatch that rewrites:

- `image` (override > type default)
- `ports` (single `http` port matching `stubby.io/port`)
- `livenessProbe` / `readinessProbe` (httpGet on `/healthz` and `/readyz`)
- `env` (appends `STUBBY_APP_NAME`)
- `resources` (defaults applied only if absent)

Containers whose name starts with a known sidecar prefix (`istio-`,
`linkerd-`, `vault-`, `cilium-`) or matches `stubby.io/skip-containers`
are left untouched.

`failurePolicy: Ignore` is the chart default — a stubby outage doesn't
block pod creation.

## License

MIT
