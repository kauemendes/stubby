#!/usr/bin/env bash
# Defect 3: a pod referencing a not-yet-created envFrom Secret (and a Secret
# volume) must still boot green. stubby strips the orphaned envFrom,
# volumeMounts, and volume, so the pod reaches 1/1 instead of
# CreateContainerConfigError / ContainerCreating.
set -euo pipefail
NS=default
BACKEND_IMG=local/stubby-dummy-backend:e2e

dump() {
  rc=$?
  echo "==> defect3-backend-envfrom.sh diagnostics (exit $rc)" >&2
  kubectl get -n "$NS" deploy,pod -o wide >&2 || true
  kubectl describe -n "$NS" pod -l app=api-envfrom >&2 || true
  kubectl logs -n "$NS" -l app=api-envfrom --tail=100 >&2 || true
  exit $rc
}
trap dump ERR

kubectl apply -n "$NS" -f examples/defect3-backend-envfrom.yaml
kubectl rollout status -n "$NS" deploy/api-envfrom --timeout=90s

POD=$(kubectl get -n "$NS" pod -l app=api-envfrom -o jsonpath='{.items[0].metadata.name}')
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')
if [[ "$IMG" != "$BACKEND_IMG" ]]; then
  echo "FAIL: api-envfrom image not mutated; got $IMG" >&2
  exit 1
fi

# The orphaned envFrom and Secret volume must be gone from the running spec.
ENVFROM=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].envFrom}')
if [[ -n "$ENVFROM" ]]; then
  echo "FAIL: expected envFrom stripped, got '$ENVFROM'" >&2
  exit 1
fi
VOLS=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.volumes[*].name}')
if grep -q 'app-config' <<<"$VOLS"; then
  echo "FAIL: expected orphan secret volume 'app-config' pruned, got '$VOLS'" >&2
  exit 1
fi

kubectl run curl-api-envfrom --rm -i --restart=Never -n "$NS" \
  --image="${CURL_IMG:-curlimages/curl:8.20.0}" \
  --image-pull-policy=IfNotPresent \
  -- curl -sf --max-time 10 --retry 5 --retry-delay 1 --retry-connrefused \
       http://api-envfrom.default.svc:8080/health | grep -q ok

echo "defect3 (orphan envFrom/volume stripped) OK ($IMG)"
