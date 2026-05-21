# Installation

## Prerequisites

- Kubernetes 1.29 or newer.
- Helm 3.13 or newer.
- Optional: [cert-manager](https://cert-manager.io/) if you set
  `tls.mode=cert-manager` (default is `self-signed`).

## Install via Helm

```bash
helm repo add stubby https://kauemendes.github.io/stubby
helm install stubby stubby/stubby \
  --namespace stubby-system \
  --create-namespace
```

## TLS modes

The chart ships two TLS bootstrapping strategies, selected via
`values.tls.mode`:

| Mode | When to use |
|------|-------------|
| `self-signed` (default) | No external dependencies. A pre-install Job generates a self-signed CA, writes the Secret, and patches the `caBundle` field on the `MutatingWebhookConfiguration`. |
| `cert-manager` | Use if cert-manager is already installed. The chart creates a self-signed `Issuer` + `Certificate`; cert-manager handles rotation. |

Switching modes:

```bash
helm upgrade --install stubby stubby/stubby \
  --namespace stubby-system \
  --set tls.mode=cert-manager
```

## Air-gapped / private registry

Override the image repositories and add pull secrets:

```bash
helm upgrade --install stubby ./stubby \
  --set image.repository=registry.internal/stubby-webhook \
  --set dummyImages.backend=registry.internal/stubby-dummy-backend:0.1.0 \
  --set dummyImages.frontend=registry.internal/stubby-dummy-frontend:0.1.0 \
  --set 'imagePullSecrets[0].name=my-pull-secret'
```

## Uninstall

```bash
helm uninstall stubby --namespace stubby-system
kubectl delete mutatingwebhookconfiguration stubby
kubectl delete namespace stubby-system
```

The `MutatingWebhookConfiguration` is a cluster-scoped resource and is
not always cleaned up by `helm uninstall` — delete it explicitly to be
sure.
