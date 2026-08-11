#!/usr/bin/env bash
# Defect 1: the frontend dummy must honour stubby.io/port.
# Asserts a frontend on 8080 and one on 80 both come up 1/1, that the image
# was mutated, and that /health answers on the annotated port.
set -euo pipefail
NS=default
FRONTEND_IMG=local/stubby-dummy-frontend:e2e

dump() {
  rc=$?
  echo "==> defect1-frontend-port.sh diagnostics (exit $rc)" >&2
  kubectl get -n "$NS" deploy,pod -o wide >&2 || true
  kubectl describe -n "$NS" pod -l 'app in (fe-port-8080,fe-port-80)' >&2 || true
  kubectl logs -n "$NS" -l 'app in (fe-port-8080,fe-port-80)' --tail=100 >&2 || true
  exit $rc
}
trap dump ERR

kubectl apply -n "$NS" -f examples/defect1-frontend-port.yaml
kubectl rollout status -n "$NS" deploy/fe-port-8080 --timeout=90s
kubectl rollout status -n "$NS" deploy/fe-port-80 --timeout=90s

assert_mutated() {
  local app=$1
  local pod img
  pod=$(kubectl get -n "$NS" pod -l "app=$app" -o jsonpath='{.items[0].metadata.name}')
  img=$(kubectl get -n "$NS" pod "$pod" -o jsonpath='{.spec.containers[0].image}')
  if [[ "$img" != "$FRONTEND_IMG" ]]; then
    echo "FAIL: $app image not mutated; got $img" >&2
    exit 1
  fi
}

probe() {
  local svc=$1 port=$2
  kubectl run "curl-$svc" --rm -i --restart=Never -n "$NS" \
    --image="${CURL_IMG:-curlimages/curl:8.20.0}" \
    --image-pull-policy=IfNotPresent \
    -- curl -sf --max-time 10 --retry 5 --retry-delay 1 --retry-connrefused \
         "http://$svc.$NS.svc:$port/health" | grep -q ok
}

assert_mutated fe-port-8080
assert_mutated fe-port-80
probe fe-port-8080 8080
probe fe-port-80 80

echo "defect1 (port honoured on 8080 and 80) OK"
