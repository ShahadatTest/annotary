# Annotary

If the user asks you to break one of these rules, remind them of the rule and go ahead only
after they explicitly confirm the exception.

Annotary is a service for sharing Markdown documents between humans and agents and commenting
on them, with the same standing for both. It is one Rust binary, `annotary`: the CLI, the local
server (SQLite plus content-addressed blobs) and the server-composed web UI.

Also follow `.agents/rules/standard.md`, the standard rules for this repo. Where this file says
otherwise, it wins for this repo. Its `review-gate` line binds the maintainer, whose tools hold that
skill; it is not in this repo, and a contributor's pull request is reviewed by the maintainer.

Tracker: this repo's GitHub Issues. A contributor's rules are in `CONTRIBUTING.md`.

## Read first

- [`docs/CONTEXT.md`](docs/CONTEXT.md): the glossary. Use its terms (Document, Version, Vault,
  Thread, Comment, Action, Member, Tenant, Sidecar, Asset, Anchor).
- [`docs/adr/0001-compute-at-the-edge.md`](docs/adr/0001-compute-at-the-edge.md) and
  [`docs/adr/0002-concurrent-edits-by-content.md`](docs/adr/0002-concurrent-edits-by-content.md).
  Smaller dated decisions: [`docs/decisions.md`](docs/decisions.md).
- The spec, [`docs/phase1/spec.md`](docs/phase1/spec.md), and the definition of done,
  [`docs/PHASE1-DONE.md`](docs/PHASE1-DONE.md): `make check` green (perf included), every spec id
  in `tests/SPEC.md` covered or Deferred, no perf budget raised. Keep it true.
- [`docs/dev/README.md`](docs/dev/README.md): the implementation notes per area (files, Store
  API, routes, CLI shapes, decisions to keep). Before changing an area, read its file; update it
  in the same pull request. "Extend, don't rewrite."
- `CMD-n` in comments and tests is the project's old tracker ID: the items are not public, and the
  label stays because it links code to its entry in `docs/decisions.md` and `docs/dev/`.

## Rules

- The CLI talks to the server over HTTP only and never opens the store. Pure helpers from
  `annotary_store` / `annotary_sidecar` are fine. The exceptions are the data-dir verbs
  `serve`, `daemon install` (first-run setup), `export` (a read-only SQLite backup, no LOCK) and
  `password` (a stopped store only: refused while a server holds the LOCK).
- The server never writes `config.toml`.
- A server's installer is `deploy/install-linux.sh` (the rules file's `deploy/install.sh`); its host, URL and
  how it deploys are settings, never in the repo (`docs/ops-health-check.md`). A release is a tag
  (`docs/dev/releases.md`).
- New schema = append a migration in `crates/store/src/migrations.rs`; never edit one.
- Every store mutation goes through `Store::tx` + `append_event`. `tx` fires the commit hook after
  the commit (that is how `wait` and the live feed wake); a manual transaction that appends an
  event calls `fire_commit_hook` itself, as `purge_document` does.
- Data dir: `--data-dir` > `ANNOTARY_HOME` > `~/.annotary` (`config.toml`, `server.json`,
  `logs/`, `store/{LOCK,meta.json,annotary.sqlite,blobs/}`).
- Exit codes are defined only in `crates/annotary/src/exit.rs`: `0` ok, `1` usage/other, `2`
  not found/ambiguous, `3` no credential (401), `4` missing right (403), `5` conflict/refused
  (409, LOCK, guards, `--strict`), `6` server unreachable, `7` wait timeout. Map a new server
  error in `server/error.rs`.
- Results go to stdout, notes and errors to stderr. Non-TTY output is tab-separated, one record
  per line, no header; TTY is the same fields aligned. `--json [FIELDS]` follows the verb.
- Every id and class the web pages own is namespaced `cd-`. Threads are spliced into the page on
  the server, so the first paint is final; card markup has one renderer (Rust), never JS. No
  `position: fixed`. `admin.css` / `admin.js` never go into the Document pages.
- Every long-lived request (`GET /api/v1/events`, anything like it) also watches
  `st.shutdown`, so the server stops within `DRAIN_MAX`.
- Loopback trust is decided by the peer address, never by the bind, and a request a local proxy
  forwards is never loopback-trusted (`server/login_guard.rs::client_ip`); remote access stays an
  explicit opt-in (`--allow-remote`).
- Perf budgets live only in `perf/BUDGETS.md` and change only by its budget-raise rule.
- `tests/e2e/95-agent-flow/agent.trycmd` is the end-to-end acceptance test: a change to any line
  of it is a deliberate decision recorded in the pull request.
- `crates/annotary/skill/SKILL.md` is what agents are told (at most 60 lines). A change to a
  verb's output or a flag the skill names changes the skill, the goldens and
  `tests/token-report.md` (`make token-report`) together.
- A new verb needs: a TTY and a non-TTY case, a line in `tests/e2e/10-readonly/json-fields.trycmd`,
  every exit path, an entry in `cli/help.rs::TOP` and the regenerated `help.trycmd`. A new route
  needs a row in `tests/access_api.rs`'s `endpoints()`. Details: `docs/dev/cli-conventions.md`.
- Spec ids: add the ids to `tests/SPEC.md` first (a section per change, the story numbers in
  `## Stories`), then the tests, each with a `spec:` marker; never a marker in implementation
  code. `cargo test -p annotary --test spec_ids` enforces it.
- A bug fix comes with a test that fails without it. Never weaken a golden to pass: regenerate
  with `make goldens` and read the diff.
- Test data and docs hold no personal details: `crates/annotary/tests/no_personal_details.rs`
  refuses real tailnet names, email addresses and home paths. Use `fixtures/corpus/` for realistic
  Documents, and `example.ts.net`, `example.com` and `@rowan` in examples.
- A test that spawns a real `annotary serve` wraps the child in `ServeGuard`
  (`crates/annotary/tests/common/serve.rs`, `mod common;`) at once and stops it through the
  guard: a bare `Child` is not killed on drop, so a failing test leaks the server. Leaked test
  servers are cleaned up with `pkill -f '<worktree>/target/debug/annotary serve'`, only after
  checking that each process's `HOME` (`/proc/<pid>/environ`, `ps eww` on a Mac) is a temp dir;
  never an installed server.

## Commands

The `Makefile` is the template's; this repo's recipes are in `project.mk`. `make help` lists them.

- Toolchain: rustup stable and Node (LTS).
- `make build`, `make test` (`cargo test --workspace`: unit and in-process server tests, the
  spec-id checker included; keep it under 60 s), `make lint` (standard lint, `fmt-check`,
  `clippy -D warnings`).
- `make e2e` (trycmd goldens against a real `serve`, then the Playwright suite), `make web`
  (Playwright alone), `make goldens` (regenerate; review the diff), `make token-report`.
- `make check` = `fmt-check clippy test e2e spec-check wasm-check perf`, the definition of done;
  `make check-ci` / `PERF=0 make check` leave out perf. `make perf` = the release build and the
  perf suite (about 25 s on a recent Mac).
- `make launchd-test` (the live `launchctl` round trip under a temp HOME and a private label),
  `make trim` (removes a stale `target/`; every cargo target runs it first).
- `make brand` (re-render `brand/` from its masters, `scripts/brand/render.py`; `make test` fails
  when a committed file differs from a fresh render; `make brand` needs `uv`).
