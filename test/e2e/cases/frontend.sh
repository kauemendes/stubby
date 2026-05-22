#!/usr/bin/env bash
# Frontend case: apply storefront Deployment with stubby.io/type=frontend,
# verify the webhook swapped the container image to stubby-dummy-frontend,
# and curl / through the Service to assert the rendered HTML contains
# the configured stubby.io/app-name.
set -euo pipefail
NS=default

dump() {
  rc=$?
  echo "==> frontend.sh diagnostics (exit $rc)" >&2
  kubectl get -n "$NS" all -o wide >&2 || true
  kubectl describe -n "$NS" pod -l app=storefront >&2 || true
  kubectl logs    -n "$NS" -l app=storefront --tail=200 >&2 || true
  exit $rc
}
trap dump ERR

kubectl apply -n "$NS" -f examples/frontend.yaml
kubectl rollout status -n "$NS" deploy/storefront --timeout=90s

POD=$(kubectl get -n "$NS" pod -l app=storefront -o jsonpath='{.items[0].metadata.name}')
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')

if [[ "$IMG" != "local/stubby-dummy-frontend:e2e" ]]; then
  echo "FAIL: frontend image not mutated; got $IMG" >&2
  exit 1
fi

# Capture the response body so a grep miss reveals what was rendered.
BODY=$(kubectl run curl-fe --rm -i --restart=Never -n "$NS" \
  --image="${CURL_IMG:-curlimages/curl:8.20.0}" \
  --image-pull-policy=IfNotPresent \
  -- curl -sf --max-time 10 --retry 5 --retry-delay 1 --retry-connrefused \
       http://storefront.default.svc:80/)

if ! grep -q 'Storefront' <<<"$BODY"; then
  echo "FAIL: response from storefront did not contain 'Storefront'" >&2
  echo "---- body ----" >&2
  printf '%s\n' "$BODY" >&2
  echo "---- /body ----" >&2
  exit 1
fi

echo "frontend OK ($IMG)"
