<!--
Thanks for the PR! Please fill in the sections below.

If this is a draft you're sharing for early feedback, mark the PR as
"Draft" and only fill in the Summary section — the rest can come later.
-->

## Summary

<!-- 1–3 bullets describing what this PR changes and why. Focus on the why. -->

-
-

## Related issues

<!-- Use "Closes #123" to auto-close when the PR is merged, or just reference. -->

Closes #

## Type of change

<!-- Tick all that apply. -->

- [ ] `feat` — new behaviour
- [ ] `fix` — bug fix
- [ ] `refactor` — code restructure, no behaviour change
- [ ] `test` — test-only change
- [ ] `docs` — documentation
- [ ] `ci` — workflows / release plumbing
- [ ] `build` — chart, Dockerfile, dependency
- [ ] `chore` — anything else

## Breaking changes?

<!-- "No" or a short description of the break + migration steps. -->

No.

## Test plan

<!-- How to verify the change works as intended. -->

- [ ] `cargo fmt --check`
- [ ] `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `helm lint     charts/stubby`
- [ ] `helm unittest charts/stubby`
- [ ] `bash test/e2e/run.sh` (if the change touches the webhook or chart)

## Checklist

- [ ] I've added or updated tests for the behaviour I'm changing.
- [ ] I've updated `CHANGELOG.md` under `## [Unreleased]`.
- [ ] My commits follow [Conventional Commits](https://www.conventionalcommits.org/).
- [ ] I've read [CONTRIBUTING.md](../CONTRIBUTING.md) and [CODE_OF_CONDUCT.md](../CODE_OF_CONDUCT.md).
