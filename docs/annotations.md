# Annotation reference

All annotations live on the **Pod** (or `spec.template.metadata.annotations`
of the Deployment / StatefulSet / Job / etc.).

| Annotation | Type | Default | Meaning |
|---|---|---|---|
| `stubby.io/type` | `backend` \| `frontend` \| `off` | absent → skip | Selects dummy image. `off` (or absent) disables injection. |
| `stubby.io/app-name` | string | `metadata.name` | Display name in OpenAPI title and HTML page. |
| `stubby.io/port` | u16 | `8080` (backend), `80` (frontend) | Container port that the dummy listens on. |
| `stubby.io/image-override` | `registry/image:tag` | (none) | Use your own image instead of the official dummy. |
| `stubby.io/skip-containers` | CSV | (none) | Container names within the pod to leave untouched. |

## Skipped sidecars

Containers whose `name` starts with any of these prefixes are always
skipped:

- `istio-`
- `linkerd-`
- `vault-`
- `cilium-`

This list is fixed in v1; if you run a sidecar with a different prefix,
add it to `stubby.io/skip-containers`.

## Examples

Backend with port override:

```yaml
metadata:
  annotations:
    stubby.io/type: backend
    stubby.io/app-name: "Orders API"
    stubby.io/port: "9000"
```

Use a custom dummy image:

```yaml
metadata:
  annotations:
    stubby.io/type: backend
    stubby.io/image-override: ghcr.io/me/my-custom-dummy:dev
```

Skip an additional sidecar by name:

```yaml
metadata:
  annotations:
    stubby.io/type: backend
    stubby.io/skip-containers: telemetry,audit
```
