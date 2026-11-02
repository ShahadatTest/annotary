# The perf suite (`make perf`, ticket 15)

Budgets as tests: `make perf` builds the **release** binaries, spawns a real `annotary serve`
on a temp HOME, seeds it with the generated fixture through the real binary, measures the web
UI with Playwright + CDP (cold browser cache per run, median of 5) and fails on any budget miss.
The budgets — FCP < 100 ms, interactive < 200 ms, Version switch < 150 ms, Document list
< 100 ms, CLS 0 after first paint, optimistic reply < 50 ms perceived — live in
`BUDGETS.md` **and nowhere else** (the runner parses that table); server TTFB is
reported separately. Absolute values, no CI multiplier.

## What is in here

| file | what |
|---|---|
| `BUDGETS.md` | the budgets (the single source), how each is measured, the budget-raise rule, the record of every raise and the optimisation log |
| `budgets.ts` | reads the table of `BUDGETS.md` for the runner |
| `playwright.config.ts` | one worker, no retries, no parallelism; `global-setup.ts` / `global-teardown.ts` |
| `global-setup.ts` | spawns the release server (`tests/web/harness.ts`'s `spawnServer`) and runs `perfgen seed` — the generated Document (≥ 5,000 lines / ~50k words, 5 Versions, ≥ 500 Threads with mixed placements, Replies, mentions, Actions on two Versions) plus 60 small Documents for the list; state in `target/perf/harness.json` |
| `perf.spec.ts` | the runner: Document view (FCP, interactive, CLS, TTFB; layout A at 1280 px, layout C at 600 px reported), Version switch, Document list, optimistic reply; prints the table, writes `target/perf/results.{md,json}`, asserts the budgets |
| `../crates/perfgen` | the deterministic generator (`Plan::generate(scale, seed)`) and the seeder (`seed()`); `perfgen stats|write|seed` |

The generator is checked in; its output is not — every run regenerates the fixture from the
seed (`PERF_SEED`, default 1). The smoke variant (`crates/annotary/tests/perf_smoke.rs`) runs
the generator at `Scale::SMOKE` through the real binary under plain `cargo test`, so the
generator and the seeder cannot rot between perf runs.

## Running it

```
make perf                      # release build → server + fixture → Playwright → table
PERF_RUNS=3 make perf          # fewer runs while iterating (the record is 5)
PERF_SCALE=smoke make perf     # the small fixture (the numbers are not the budgets' numbers)
CD_PERF_KEEP=1 make perf       # keep the seeded temp store / HOME after the run
npx playwright test -c perf/playwright.config.ts   # after `cargo build --release -p annotary -p annotary-perfgen`
```

A run takes about 25 s on a recent Mac (≈ 2 s of it seeding). The table `make perf` prints is
the record of the run; `target/perf/results.md` keeps it.

`make perf` is part of `make check` (ticket 16) — the definition of done
(`docs/PHASE1-DONE.md`) — so it runs before anything merges from a developer machine; `PERF=0
make check` / `make check-ci` leave it out where the release toolchain / Chromium is not. It is not
part of this repo's CI, so run `make perf` on your own machine before merging anything that touches the web view (`server/view.rs`, `splice.rs`, `ui.rs`,
`ui/app.js`, `ui/app.css`), the renderer, the read path of the store or the perf suite itself.

## Budgets and the budget-raise rule

Budgets may change only by the rule in `BUDGETS.md`: at least three genuine, measured
optimisation attempts at the missed budget first; then that budget alone may be raised to one
absolute value with the attempts and their numbers recorded there; a raised budget is still
asserted. Today no budget is raised — the original values hold on the fixture (see the
optimisation log in `BUDGETS.md` for what it took).

## Where it runs

**Never on a GitHub-hosted runner**: the budgets are absolute and the numbers of a shared, noisy
runner mean nothing, so no workflow in this repo runs `make perf`. Nor does a pull request from a fork
run it anywhere: this repo has no self-hosted runner, because one would run a stranger's code on
someone's machine.

Instead a dedicated Mac the maintainer keeps runs it nightly against `main`, from a job outside this
repo, and files an issue here when a budget fails. A regression is therefore caught within a day
rather than on its pull request. That Mac also does other work, so a run can be slower than on a
developer's Mac; check that first when a budget fails there.

What the job needs and does, which is all you need to run it on your own Mac: rustup stable and Node
22 for the user, nothing system-wide; then, from a clean checkout of `main`:

```sh
npm ci --no-fund --no-audit
npx playwright install chromium
make perf
```

`make perf` builds what it needs (`cargo build --release -p annotary -p annotary-perfgen`), spawns its
own server on an ephemeral port and cleans up after itself; nothing persists between runs except the
rustup, cargo, npm and Playwright caches. Only one run at a time on a machine: a concurrent run would
skew the numbers.

## Reading a failure

The last test prints the table, then asserts. A `FAIL` row names the metric, its median and the
budget; `target/perf/results.json` has every sample. First check the machine (other load, a
debug build — `make perf` always builds release, but `ANNOTARY_BIN` can override), then the
change: the optimisation log in `BUDGETS.md` lists what each budget depends on. Raising a budget
is the last resort and follows the rule.
