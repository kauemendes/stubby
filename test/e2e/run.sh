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
helm upgrade --install stubby ./charts/stubby \
  --namespace "$NS" \
  --set image.repository=local/stubby-webhook \
  --set image.tag=e2e \
  --set image.pullPolicy=Never \
  --set dummyImages.backend="$BACKEND_IMG" \
  --set dummyImages.frontend="$FRONTEND_IMG" \
  --set tls.mode=self-signed \
  --wait --timeout=3m

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
