#!/usr/bin/env bash
# Frontend case: apply storefront Deployment with stubby.io/type=frontend,
# verify the webhook swapped the container image to stubby-dummy-frontend,
# and curl / through the Service to assert the rendered HTML contains
# the configured stubby.io/app-name.
set -euo pipefail
NS=default

kubectl apply -n "$NS" -f examples/frontend.yaml
kubectl rollout status -n "$NS" deploy/storefront --timeout=90s

POD=$(kubectl get -n "$NS" pod -l app=storefront -o jsonpath='{.items[0].metadata.name}')
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')

if [[ "$IMG" != "local/stubby-dummy-frontend:e2e" ]]; then
  echo "FAIL: frontend image not mutated; got $IMG" >&2
  exit 1
fi

kubectl run curl-fe --rm -i --restart=Never -n "$NS" \
  --image="${CURL_IMG:-curlimages/curl:8.20.0}" \
  --image-pull-policy=IfNotPresent \
  -- curl -sf --max-time 10 http://storefront.default.svc:80/ \
  | grep -q 'Storefront'

echo "frontend OK ($IMG)"
