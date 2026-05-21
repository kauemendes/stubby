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
