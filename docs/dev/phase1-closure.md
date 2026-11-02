# Phase 1 closure (ticket 16 — keep it true)

Implementation notes, moved from the root `CLAUDE.md` on 2026-10-18 (read on demand; the
index is [README.md](README.md)). Extend them with the code they describe.

- **Definition of done:** `docs/PHASE1-DONE.md` — all 16 tickets closed, `make check` green (perf included), every budget met on the fixture with no raise (`perf/BUDGETS.md` "Raised budgets: None"). `tests/phase1_done.rs` pins the plumbing (Makefile targets, the CI jobs and their runners, the DoD's links).
- **Traceability:** `tests/SPEC.md` = the ticket sections (S01…S16) + `## Stories` (1–154 → ids) + `## Deferred` (launchctl S13-12, HEIC/AVIF on Linux S03-17, video dims S03-18, fuzzing S09-14, load/soak S15-08, systemd/Windows S13-13, W3C export S02-33 — each with its reason); `cargo test -p annotary --test spec_ids` enforces all of it.
- **CI:** `.github/workflows/checks.yml` (beside the template's `ci.yml`), on pull requests and pushes to `main`: `test` (fmt, clippy, `make test`, wasm-check on Linux and macOS), `e2e` (`make e2e` on Linux after `npm ci` + `npx playwright install chromium`; the trycmd goldens and the Playwright suite pass on Linux), `perf` (`make perf` on one dedicated Mac, in its own concurrency group, skipped when the repository variable `PERF_RUNNER=off`). `cargo test (Linux)`, `cargo test (macOS)` and `make e2e (trycmd + Playwright, Linux)` are required checks; `perf` is not, because that machine can be busy.
- **Sweep conventions:** every exit code 1–7, every verb's TTY/non-TTY pair and `--json ?`, every route in the rights matrix, the anchoring fixture checker (`crates/diff/tests/fixtures.rs` demands a directory per research row) and the full-Sidecar round trip are tests — extend them with the feature, never after.
<!-- ---- end ticket 16 ---- -->
