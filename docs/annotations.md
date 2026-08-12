# Annotation reference

All annotations live on the **Pod** (or `spec.template.metadata.annotations`
of the Deployment / StatefulSet / Job / etc.).

| Annotation | Type | Default | Meaning |
|---|---|---|---|
| `stubby.io/type` | `backend` \| `frontend` \| `off` | absent → skip | Selects dummy image. `off` (or absent) disables injection. |
| `stubby.io/app-name` | string | `metadata.name` | Display name in OpenAPI title and HTML page. |
| `stubby.io/port` | u16 | `8080` (backend), `80` (frontend) | Container port the dummy listens on. Injected as `containerPort`, the probe port, and `STUBBY_PORT` (which the dummy binary binds), so all three always agree. |
| `stubby.io/image-override` | `registry/image:tag` | (none) | Use your own image instead of the official dummy. |
| `stubby.io/skip-containers` | CSV | (none) | Container names within the pod to leave untouched. |
| `stubby.io/keep-env-from` | `true` \| `false` | `false` | Keep the container's `envFrom` instead of stripping it (see below). |
| `stubby.io/keep-volumes` | `true` \| `false` | `false` | Keep `volumeMounts` and orphaned `volumes` instead of pruning them (see below). |
| `stubby.io/auto-rescue` | `true` \| `false` | `false` | Experimental. Requires the controller (`controller.enabled`). When the container is stuck in `ImagePullBackOff`/`ErrImagePull`, swap it to a dummy in place and revert once the real image is available. `stubby.io/type`/`port` act as hints. |

## Skipped sidecars

Containers whose `name` starts with any of these prefixes are always
skipped:

- `istio-`
- `linkerd-`
- `vault-`
- `cilium-`

This list is fixed in v1; if you run a sidecar with a different prefix,
add it to `stubby.io/skip-containers`.

## Stripping orphaned config (`envFrom`, volumes)

The point of stubby is that **every pod boots green**. A pod that
references config which doesn't exist yet — the normal state before the
real app is provisioned — would otherwise leave `ImagePullBackOff` only
to land in `CreateContainerConfigError` (missing `envFrom` source) or a
stuck `ContainerCreating` (missing `secret`/`configMap` volume). The
dummy needs none of that config, so by default stubby removes it:

- **`envFrom`** on each mutated container is dropped. Opt out with
  `stubby.io/keep-env-from: "true"` (e.g. when the referenced Secret
  *does* exist and you want the dummy to see it).
- **`volumeMounts`** on each mutated container are dropped, and any
  pod-level `secret` / `configMap` / `projected` volume that is no
  longer referenced by a kept container (a skipped sidecar or an init
  container) is pruned. `emptyDir`, PVCs and other volume types are left
  alone. Opt out with `stubby.io/keep-volumes: "true"`.

Inline `env` entries are **not** stripped — only `envFrom`. Stubby still
appends `STUBBY_APP_NAME` and `STUBBY_PORT`.

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

Keep the original `envFrom` and volumes (the referenced objects exist and
you want the dummy to see them):

```yaml
metadata:
  annotations:
    stubby.io/type: backend
    stubby.io/keep-env-from: "true"
    stubby.io/keep-volumes: "true"
```
