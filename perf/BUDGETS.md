# Perf budgets (ticket 15)

The budgets of spec story 128 / Testing seam 5, asserted by `make perf` (`perf/perf.spec.ts`)
against the generated fixture (`crates/perfgen`: ≥ 5,000 lines / ~50k words / ≥ 500 Threads
across 5 Versions, 60 list Documents) on a **release** build, localhost, Chromium (Playwright),
cold browser cache per run, **median of 5 runs**. Absolute values — no CI multiplier.

**This file is the only place a budget lives.** The runner parses the table below (the `id`,
`budget` and `unit` columns); nothing in the code repeats a number. A budget may change only by
the budget-raise rule at the end of this file.

## The budgets

| id | metric | budget | unit | how it is measured |
|---|---|---|---|---|
| fcp | Document view FCP | 100 | ms | `first-contentful-paint` of `GET /d/{id}` (head Version, 1280×900, layout A) |
| interactive | Document view interactive | 200 | ms | from navigation start until the page takes input: max(DOMContentLoaded end, the page's `cd:ready` mark — inline script run, handlers bound, margin cards positioned), extended to the end of any long task (≥ 50 ms) still running at that moment |
| switch | Version switch | 150 | ms | from the in-page click on "older Version" (`.cd-vsw a.cd-older`, `Date.now()` just before `click()`) to the FCP of `/d/{id}/v/{n−1}` |
| list | Document list | 100 | ms | `first-contentful-paint` of `GET /` with a full page of 50 rows (+ "more") |
| cls | CLS after first paint | 0 | score | sum of `layout-shift` values without recent input with `startTime ≥ FCP`, over the Document view's load until 500 ms after `cd:ready` |
| optimistic | Optimistic reply (perceived) | 50 | ms | from the in-page click on the reply editor's Post to the next painted frame (rAF, then a macrotask) after the pending Comment (`.cd-c.cd-pending`) is in the DOM — the server's answer held back by a route until afterwards, so the DOM update is measured, not the round trip |

Reported, not budgeted: server TTFB (`responseStart − requestStart`) of the Document view, the
Version page and the list; DOMContentLoaded; the page's byte size; the same metrics of the
Document view in layout C (600 px wide).

## Measured (dev Mac, Apple M4 Pro, release build, 2026-08-19)

See `target/perf/results.md` after a run; the table `make perf` prints is the record of the last
run. The numbers at the time the suite was written (median of 5):

| metric | budget | measured (median of 5) | min – max |
|---|---|---|---|
| Document view FCP | 100 ms | 80 ms | 80 – 96 |
| Document view interactive | 200 ms | 125 ms | 122 – 126 |
| Version switch | 150 ms | 81 ms | 78 – 94 |
| Document list | 100 ms | 36 ms | 36 – 36 |
| CLS after first paint | 0 | 0 | 0 – 0 |
| Optimistic reply (perceived) | 50 ms | 16 ms | 8 – 27 (one frame of granularity) |

Reported: server TTFB 11 ms (Document view, 1.8 MB) · 21 ms (Version page, time-slice) · 4 ms
(list, 123 KB); DOMContentLoaded 87 ms; layout C (600 px): FCP 72 ms, interactive 117 ms, CLS 0;
optimistic reply DOM updated 2.7 ms. Fixture: 5,146 lines / 56,621 words / 524 Threads
(321 anchored, 94 refined, 93 orphaned, 16 Document-level), 661 Replies, seeded in ≈ 1.8 s
(pushes 19 / 39 / 54 / 64 / 74 ms — re-anchoring 500 Threads at the edge included).

## Budget-raise rule (authorised by the human, 2026-08-18)

The budgets above are the targets. A budget may be raised only after **at least three genuine
optimisation attempts** at the missed budget — each a real change (viewport-first rendering, a
smaller first response, fewer round-trips, …) with numbers recorded before and after. If the budget
is still missed after three attempts, **that budget only** may be raised to one absolute value
judged reasonable from the measurements (not a multiplier), and the raise must be recorded here:
the original value, the new value, the three attempts with their measurements, and why the
remaining gap is not cheaply closable. A raised budget is still asserted — the suite still fails on
a regression from the new value. Budgets change nowhere else.

### Raised budgets

None. Every budget is the original value.

### Optimisation log (what it took to meet the original budgets on the fixture)

Baseline before any change (release server, the generated Document, 1280×900, Playwright Chromium,
median of 3): server TTFB 11 ms, page 1.84 MB; FCP 136 ms (and sometimes ≈ 400 ms — the parser
did not always yield to paint before the end-of-body script), DOMContentLoaded 424 ms, a 160 ms
long task after `load`; CLS 0. The script's phases: localize 35 ms, highlight 29 ms, layout 167 ms.

1. `localizeTimes`: one cached `Intl.DateTimeFormat` per shape instead of `toLocaleString` per
   `<time>` (≈ 1,300 tags): 35 ms → ≈ 3 ms.
2. `layout()`: highlight elements indexed by Thread id as the script makes them (no
   `querySelectorAll` over a 60k-node Document per strip), no `getComputedStyle` per strip, no
   re-layout on `window.load` / sized-media `load`: 167 ms (+ 160 ms after load) → ≈ 20 ms, no
   post-load task.
3. Highlight: top-level blocks found by binary search on the line index instead of a scan over
   every block per Thread (29 ms → ≈ 10 ms).
4. The inline scripts are `type="module"` (deferred): the browser paints the server-composed
   page before running them — FCP no longer waits for the script and stops varying with the
   parser's yield points.
5. Viewport-first rendering (`content-visibility: auto`) on the Document's top-level blocks and
   the Thread cards from the 61st child on (the first 60 render in the first frame, so nothing in
   view is a placeholder — a placeholder that grows after the first paint is a layout shift; with
   the rule on every block CLS was 0.022), except blocks that hold or precede a strip (their
   highlight is what the margin card aligns with): style + layout of the 61k-node page
   ≈ 105 ms → ≈ 25 ms with JS off.

After 1–5 (median of 5, release server): FCP 80 ms, interactive (`cd:ready` = DOMContentLoaded)
127 ms, CLS 0, no long task.
