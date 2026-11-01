## Summary

<!-- What changes and why? One paragraph. -->

## Issue

<!-- Closes #N -->

## Evidence

<!-- Paste the failing-test output (before) and the green gate run (after):
     cargo fmt --all -- --check && cargo clippy -p ium-core --all-targets -- -D warnings && cargo test -p ium-core -->

## Checklist (DoD — docs/05_Engineering_Process.md)

- [ ] Failing test written first, output pasted above
- [ ] Minimal fix — no unrelated changes
- [ ] fmt + clippy `-D warnings` + full suite green locally
- [ ] Journal invariants respected (state read from journal, transitions journaled first)
- [ ] Verify-before-declare not weakened
- [ ] STATUS.md session log updated
- [ ] CI green on all 3 OS
