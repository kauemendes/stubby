# Security policy

`stubby` is a learning-grade lab project but it runs in the admission
path of a Kubernetes cluster, so I take vulnerabilities seriously.
Thanks for taking the time to report one.

## Reporting a vulnerability

**Do not** open a public issue.

Use GitHub's private vulnerability reporting:

1. Go to the [Security tab](https://github.com/kauemendes/stubby/security)
   of this repository.
2. Click **Report a vulnerability**.
3. Fill in the form — repro, impact, suggested fix if you have one.

Or, if you cannot use GitHub Security Advisories, email
**kaue.mendes@gmail.com** with the subject line `[stubby security]`.
You should receive an acknowledgement within **3 business days**.

If you do not hear back within a week, please re-send — your message
may have ended up in spam.

## Disclosure timeline

- **Day 0** — report received.
- **Day 0–3** — acknowledgement + initial triage.
- **Day 3–14** — fix developed in a private branch; coordinated
  disclosure date agreed with the reporter.
- **Day ≤30** — patch released, advisory published, CVE requested if
  applicable.

Credit is given in the advisory unless the reporter prefers to remain
anonymous.

## Supported versions

Security fixes target the latest minor release. Older minor releases
receive fixes only for critical issues.

| Version | Supported          |
|---------|--------------------|
| 0.1.x   | :white_check_mark: |
| < 0.1   | :x:                |

## Threat model

`stubby` sits in the mutating-admission path of every `Pod CREATE`
that matches its `namespaceSelector`. The risks the design takes
seriously:

| Risk | Mitigation |
|------|------------|
| A malicious AdmissionReview body crashes the webhook → API server stops accepting pods. | `failurePolicy: Ignore` by default — API server skips the webhook when it errors out. Body is bounded to 8 MiB. Malformed bodies still return HTTP 200 with an `AdmissionResponse.status` so the contract holds. |
| Stolen webhook TLS material lets an attacker forge AdmissionReviews. | TLS-only ingress, cert hot-reloaded every 60s without restart so rotation is cheap. Self-signed mode generates a fresh CA per cluster; cert-manager mode delegates to a proper PKI. |
| Excessive privilege on the webhook ServiceAccount. | Webhook runs with **no** Kubernetes RBAC verbs — it never reads or writes other resources. Only the API server talks to it; it talks back only via the admission response. |
| Container escape via a known CVE. | Distroless base, `runAsNonRoot`, `readOnlyRootFilesystem`, `drop ALL` capabilities, seccomp `RuntimeDefault`. Multi-arch images signed with `cosign` keyless and shipped with SBOM + SLSA provenance. |
| Supply-chain tampering. | All images are signed with `cosign` keyless against the GitHub Actions OIDC issuer; verify with `cosign verify`. SBOMs are attached as OCI attestations. |

Out of scope:

- Denial of service against the webhook by an attacker who already has
  the ability to create pods at high rate — that's a cluster-level
  problem, not a stubby problem.
- The dummy images. They are intentionally trivial and not meant for
  production traffic.

## Hardening recommendations for operators

- Tighten `webhook.namespaceSelector` so the webhook only watches the
  namespaces you actually want.
- Run `cosign verify` against image pulls (e.g., via Sigstore policy
  controller).
- Use `tls.mode: cert-manager` and a real issuer in production.
- Set a `PodDisruptionBudget` if you cannot tolerate the webhook
  being down during voluntary disruptions.
- Scrape `/metrics` and alert on `stubby_admissions_total{decision="error"}`.
