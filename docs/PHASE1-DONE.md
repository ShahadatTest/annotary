# Definition of done

Phase 1 (the local server + CLI + web viewer/commenter of
[`docs/phase1/spec.md`](phase1/spec.md)) closed when its 16 build tickets did, and every later change
keeps these true:

1. **`make check` is green** — `fmt-check` + `clippy -D warnings` + `cargo test --workspace`
   (crate unit tests + in-process server integration tests + the spec-id checker, < 60 s) +
   `make e2e` (trycmd goldens against a real spawned `serve` + the Playwright suite) +
   `spec-check` + `wasm-check` (renderer / diff / sidecar for `wasm32-unknown-unknown`) +
   `make perf`. `PERF=0 make check` / `make check-ci` is the same without the perf suite, for a
   machine without the release toolchain or Chromium.
   [`.github/workflows/checks.yml`](../.github/workflows/checks.yml) (beside the template's `ci.yml`)
   runs them in CI ([`perf/README.md`](../perf/README.md) has where the perf suite runs).
2. **Every perf budget is met on the generated fixture** (`make perf`: release build, ≥ 5,000
   lines / ≥ 500 Threads, median of 5 cold runs) — the budgets live only in
   [`perf/BUDGETS.md`](../perf/BUDGETS.md); a budget may be raised only by that file's
   budget-raise rule (≥ 3 genuine measured attempts, one budget at a time, the record in the
   file) and every raise is listed under its **"Raised budgets"** heading. At the close of
   Phase 1 that list reads **"None. Every budget is the original value."**

## Traceability

"Comprehensive" means spec-id traceability, not line coverage
([`tests/SPEC.md`](../tests/SPEC.md)): every id has at least one test naming it (or sits under
`## Deferred` with a reason), every numbered story of the spec maps to the ids that cover it, and
`cargo test -p annotary --test spec_ids` (in `cargo test` and `make check`) fails on an id
with zero tests, a marker naming an unknown id, or a story missing from the map. Deferred in
Phase 1, with reasons in `SPEC.md`: `launchctl` itself (plist unit-tested; the live round trip is
`make launchd-test`), HEIC/AVIF decode on Linux CI, video dimensions beyond the fixtures, parser
fuzzing, load/soak, systemd/Windows, the W3C Web Annotation export.

## Measured at the close (Apple M4 Pro, 2026-08-19)

`make perf`: FCP 80 ms (budget 100) · interactive 125 ms (200) · Version switch 81 ms (150)
· list 36 ms (100) · CLS 0 (0) · optimistic reply 16 ms (50) — `perf/BUDGETS.md`, "Measured".
