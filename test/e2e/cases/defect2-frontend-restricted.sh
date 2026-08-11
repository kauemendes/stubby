#!/usr/bin/env bash
# Defect 2: a mutated frontend must survive Pod Security "restricted"
# (readOnlyRootFilesystem, runAsUser 1000, drop ALL). The old nginx image
# died writing the rendered page to a read-only rootfs; the Rust dummy
# renders in memory and comes up 1/1.
set -euo pipefail
NS=default
FRONTEND_IMG=local/stubby-dummy-frontend:e2e

dump() {
  rc=$?
  echo "==> defect2-frontend-restricted.sh diagnostics (exit $rc)" >&2
  kubectl get -n "$NS" deploy,pod -o wide >&2 || true
  kubectl describe -n "$NS" pod -l app=fe-restricted >&2 || true
  kubectl logs -n "$NS" -l app=fe-restricted --tail=100 >&2 || true
  exit $rc
}
trap dump ERR

kubectl apply -n "$NS" -f examples/defect2-frontend-restricted.yaml
kubectl rollout status -n "$NS" deploy/fe-restricted --timeout=90s

POD=$(kubectl get -n "$NS" pod -l app=fe-restricted -o jsonpath='{.items[0].metadata.name}')
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')
if [[ "$IMG" != "$FRONTEND_IMG" ]]; then
  echo "FAIL: fe-restricted image not mutated; got $IMG" >&2
  exit 1
fi

# Prove the restricted context actually stuck (not silently dropped).
ROFS=$(kubectl get -n "$NS" pod "$POD" \
  -o jsonpath='{.spec.containers[0].securityContext.readOnlyRootFilesystem}')
if [[ "$ROFS" != "true" ]]; then
  echo "FAIL: expected readOnlyRootFilesystem=true, got '$ROFS'" >&2
  exit 1
fi

BODY=$(kubectl run curl-fe-restricted --rm -i --restart=Never -n "$NS" \
  --image="${CURL_IMG:-curlimages/curl:8.20.0}" \
  --image-pull-policy=IfNotPresent \
  -- curl -sf --max-time 10 --retry 5 --retry-delay 1 --retry-connrefused \
       http://fe-restricted.default.svc:80/)

if ! grep -q 'Restricted' <<<"$BODY"; then
  echo "FAIL: rendered page did not contain the app name 'Restricted'" >&2
  printf '%s\n' "$BODY" >&2
  exit 1
fi

echo "defect2 (restricted + readOnlyRootFilesystem) OK ($IMG)"
