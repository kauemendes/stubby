# Troubleshooting

## The webhook isn't mutating

1. **Webhook pod healthy?**
   ```bash
   kubectl get pods -n stubby-system
   kubectl logs -n stubby-system deploy/stubby
   ```
2. **`MutatingWebhookConfiguration` has a non-empty `caBundle`?**
   ```bash
   kubectl get mutatingwebhookconfiguration stubby \
     -o jsonpath='{.webhooks[0].clientConfig.caBundle}' | head -c 80
   ```
   An empty value means the self-signed TLS Job hasn't run yet (or
   failed). Check `kubectl get jobs -n stubby-system`.
3. **Namespace excluded by `webhook.namespaceSelector`?** The chart's
   default selector excludes `kube-system` and any namespace labeled
   `stubby.io/exclude=true`. Either remove the label or override the
   selector in `values.yaml`.
4. **Pod has the right annotation?** Annotations belong to
   `spec.template.metadata.annotations` on the Deployment (so they
   propagate to Pods), not to the Deployment's own metadata.

## `x509: certificate signed by unknown authority` in API server logs

The `caBundle` and the TLS Secret are out of sync. Re-run the bootstrap
Job:

```bash
kubectl delete job -n stubby-system stubby-tls-bootstrap || true
helm upgrade stubby stubby/stubby --reuse-values
```

## Decode errors

The webhook always returns HTTP 200 to the API server. If a pod arrives
that stubby can't decode, the response carries
`response.status.message: stubby: ...` and the counter
`stubby_admissions_total{decision="error"}` increments. Look in the
webhook's JSON logs for the `status.message` line.

## Metrics

The webhook exposes Prometheus metrics on port 443 path `/metrics`:

```bash
kubectl port-forward -n stubby-system svc/stubby 9443:443
curl -k https://localhost:9443/metrics
```

Series:

- `stubby_admissions_total{type, decision}` — `decision` is `inject`,
  `skip`, or `error`.

## `/docs` (swagger-ui) doesn't load

The dummy-backend's `/docs` page loads swagger-ui assets from
`cdn.jsdelivr.net` at runtime. In air-gapped clusters those assets are
unreachable. Consume the raw OpenAPI document at `/openapi.json`
instead, or replace the dummy image via `stubby.io/image-override` with
a custom build that vendors swagger-ui.

## Pod stuck `Pending` with `Insufficient memory`

The chart's defaults (`resources.requests`) are intentionally small
(50m CPU, 64Mi memory). If your cluster runs many tiny namespaces and
the webhook gets evicted, bump `resources.requests` upwards in your
values overlay.
