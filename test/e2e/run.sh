#!/usr/bin/env bash
# End-to-end smoke for stubby:
#   1. spins up (or reuses) a kind cluster
#   2. builds + loads the 3 images
#   3. helm-installs the chart in self-signed TLS mode
#   4. runs every case script under test/e2e/cases/
#
# Set KEEP=1 to leave the cluster running after the run.
set -euo pipefail

CLUSTER=${CLUSTER:-stubby-e2e}
NS=stubby-system
WEBHOOK_IMG=local/stubby-webhook:e2e
BACKEND_IMG=local/stubby-dummy-backend:e2e
FRONTEND_IMG=local/stubby-dummy-frontend:e2e

ROOT=$(cd "$(dirname "$0")/../.." && pwd)
cd "$ROOT"

echo "==> ensuring kind cluster $CLUSTER"
if ! kind get clusters | grep -qx "$CLUSTER"; then
  kind create cluster --name "$CLUSTER" --config test/kind-config.yaml
fi

echo "==> building images"
docker build -f docker/webhook.Dockerfile        -t "$WEBHOOK_IMG"  .
docker build -f docker/dummy-backend.Dockerfile  -t "$BACKEND_IMG"  .
docker build -f docker/dummy-frontend.Dockerfile -t "$FRONTEND_IMG" .

echo "==> loading images into kind"
kind load docker-image "$WEBHOOK_IMG"  --name "$CLUSTER"
kind load docker-image "$BACKEND_IMG"  --name "$CLUSTER"
kind load docker-image "$FRONTEND_IMG" --name "$CLUSTER"

echo "==> installing chart"
kubectl get ns "$NS" >/dev/null 2>&1 || kubectl create namespace "$NS"

dump_diagnostics() {
  echo "==> helm install failed — dumping diagnostics"
  echo "---- get all"
  kubectl get all,jobs -n "$NS" -o wide || true
  echo "---- describe pods"
  kubectl describe pods -n "$NS" || true
  echo "---- pod logs (any container, --previous and current)"
  for pod in $(kubectl get pods -n "$NS" -o name 2>/dev/null); do
    echo "++ logs $pod (current)"
    kubectl logs -n "$NS" "$pod" --all-containers --tail=200 || true
    echo "++ logs $pod (previous)"
    kubectl logs -n "$NS" "$pod" --all-containers --previous --tail=200 || true
  done
  echo "---- recent events"
  kubectl get events -n "$NS" --sort-by=.lastTimestamp | tail -40 || true
}

# Stream bootstrap-pod logs in the background so the failure mode is
# visible even if Job/pod cleanup happens before dump_diagnostics runs.
(
  while sleep 2; do
    kubectl logs -n "$NS" -l job-name=stubby-tls-bootstrap \
      --all-containers --tail=-1 --prefix=true 2>/dev/null || true
  done
) &
LOG_TAIL_PID=$!
trap 'kill $LOG_TAIL_PID 2>/dev/null || true' EXIT

if ! helm upgrade --install stubby ./charts/stubby \
      --namespace "$NS" \
      --set image.repository=local/stubby-webhook \
      --set image.tag=e2e \
      --set image.pullPolicy=Never \
      --set dummyImages.backend="$BACKEND_IMG" \
      --set dummyImages.frontend="$FRONTEND_IMG" \
      --set tls.mode=self-signed \
      --wait --timeout=3m; then
  dump_diagnostics
  exit 1
fi
kill $LOG_TAIL_PID 2>/dev/null || true
trap - EXIT

echo "==> running case scripts"
status=0
for case in test/e2e/cases/*.sh; do
  echo "---- $case"
  if ! bash "$case"; then
    status=1
  fi
done

if [[ "${KEEP:-0}" != "1" ]]; then
  echo "==> deleting kind cluster (set KEEP=1 to keep)"
  kind delete cluster --name "$CLUSTER"
fi

exit $status
