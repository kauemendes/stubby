#!/usr/bin/env bash
# Auto-rescue STUB phase: a pod that opts in (stubby.io/auto-rescue=true) and
# points at a non-existent image goes ImagePullBackOff; the controller swaps
# its image to the dummy backend IN PLACE and it reaches Running.
#
# reactive-api lives in the `reactive` namespace, which is labelled
# stubby.io/exclude=true so the admission webhook does NOT mutate it at CREATE
# (the chart's namespaceSelector excludes that label). Otherwise the webhook
# would swap the image up front and the pod would never fail to pull — the
# controller, not the webhook, must be the one that rescues it here.
#
# The REVERT path (controller restores the original image once it appears in
# the registry) is covered by unit tests (reconcile::plan_action revert branch
# and registry::is_available) rather than here: exercising it in kind needs a
# registry reachable from both the node's containerd and the controller pod,
# which is disproportionate plumbing for a smoke test.
set -euo pipefail
NS=reactive
BACKEND_IMG=local/stubby-dummy-backend:e2e

dump() {
  rc=$?
  echo "==> autorescue.sh diagnostics (exit $rc)" >&2
  kubectl get -n "$NS" deploy,pod -o wide >&2 || true
  kubectl describe -n "$NS" pod -l app=reactive-api >&2 || true
  kubectl get deploy -A -l app.kubernetes.io/component=controller >&2 || true
  kubectl logs -n stubby-system -l app.kubernetes.io/component=controller --tail=120 >&2 || true
  exit $rc
}
trap dump ERR

kubectl apply -f examples/autorescue.yaml

# The controller reconciles on a ~10s interval; give it several cycles to
# observe ImagePullBackOff and patch the image.
echo "==> waiting for controller to stub reactive-api"
for i in $(seq 1 30); do
  IMG=$(kubectl get -n "$NS" pod -l app=reactive-api -o jsonpath='{.items[0].spec.containers[0].image}' 2>/dev/null || true)
  [[ "$IMG" == "$BACKEND_IMG" ]] && break
  sleep 5
done

kubectl rollout status -n "$NS" deploy/reactive-api --timeout=120s
POD=$(kubectl get -n "$NS" pod -l app=reactive-api -o jsonpath='{.items[0].metadata.name}')
IMG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.spec.containers[0].image}')
if [[ "$IMG" != "$BACKEND_IMG" ]]; then
  echo "FAIL: controller did not stub the pod; image=$IMG" >&2
  exit 1
fi

# The original image must be recorded so a future revert can restore it.
ORIG=$(kubectl get -n "$NS" pod "$POD" -o jsonpath='{.metadata.annotations.stubby\.io/original-image}')
if ! grep -q 'stubby-nonexistent' <<<"$ORIG"; then
  echo "FAIL: original-image annotation not recorded; got '$ORIG'" >&2
  exit 1
fi

# The stubbed dummy actually serves.
kubectl run curl-autorescue --rm -i --restart=Never -n "$NS" \
  --image="${CURL_IMG:-curlimages/curl:8.20.0}" \
  --image-pull-policy=IfNotPresent \
  -- curl -sf --max-time 10 --retry 5 --retry-delay 1 --retry-connrefused \
       http://reactive-api.reactive.svc:8080/health | grep -q ok

echo "autorescue (reactive stub) OK ($IMG)"
