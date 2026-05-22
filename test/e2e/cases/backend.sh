#!/usr/bin/env bash
# Backend case: apply orders-api Deployment with stubby.io/type=backend,
# verify the webhook swapped the container image to stubby-dummy-backend,
# and curl /health through the Service.
set -euo pipefail
NS=default

kubectl apply -n "$NS" -f examples/backend.yaml
kubectl rollout status -n "$NS" deploy/orders-api --timeout=90s

POD=$(kubectl get -n "$NS" pod -l app=orders-api -o jsonpath='{.items[0].metadata.name}')
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')

if [[ "$IMG" != "local/stubby-dummy-backend:e2e" ]]; then
  echo "FAIL: backend image not mutated; got $IMG" >&2
  exit 1
fi

kubectl run curl-be --rm -i --restart=Never -n "$NS" \
  --image="${CURL_IMG:-curlimages/curl:8.20.0}" \
  --image-pull-policy=IfNotPresent \
  -- curl -sf --max-time 10 --retry 5 --retry-delay 1 --retry-connrefused \
       http://orders-api.default.svc:8080/health \
  | grep -q ok

echo "backend OK ($IMG)"
