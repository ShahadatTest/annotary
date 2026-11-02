# CLI end-to-end goldens (trycmd)

Run with `make e2e` (= `cargo test -p annotary --test e2e -- --ignored`); regenerate with
`TRYCMD=overwrite make e2e`. Harness: `crates/annotary/tests/e2e.rs`.

- Groups `NN-name/` run in lexical order; cases in a group run in parallel and must not depend on
  one another (mutating cases use handles nobody else uses). Put a dependent sequence in one
  `.trycmd` file (steps run in order; stdout+stderr are merged there — use `.toml` when the
  stream split matters).
- The harness gives every case `HOME=<temp>` with `~/.annotary/config.toml` holding profiles
  `rowan` (owner), `agent` (member @scribe, vault notes) and `down` (server nobody listens on),
  no default; the seeded store has @rowan, @scribe, @linter. `[URL]` substitutes the server URL.
- Cases in `00-setup/` point `ANNOTARY_HOME` at a fresh dir inside their sandbox.
- Every case names its spec ids: `# spec: S01-11` (toml) or `spec: S01-10` in `.trycmd` prose.
- Files: `<case>.in/` is copied into the sandbox cwd (inputs for `push`); `<case>.out/` holds
  files to compare after the run (only the files listed there; `[..]` wildcards work inside
  them — see `40-docs/pull.out/`). A `.trycmd` needs `<case>.out/.keep` to get a sandbox at all.
- Documents: `30-docs-seed/seed.trycmd` creates `ro-plan` (v1 → v2 → v3 = v1's content), two
  `dup-plan`, a deleted `ro-del` and `lst-a/b/c`; the read-only `.toml` cases in `40-docs/` read
  those. Mutating `.trycmd` sequences in `40-docs/` use Names nobody else uses; a `list` golden
  always filters by a Name substring because every case's Documents share the one store.
- `.trycmd` exit codes: `? N` on the line right after the `$` command line.
- `45-assets/` (ticket 03): `push-assets.trycmd` is the ticket's scenario (PNG + linked `.md`
  with a `via` SVG + missing screenshot → warnings, `assets:` line, `pull --assets` skip/refuse/
  `--force`, the pulled tree pushes back as `already`); `asset-change.trycmd` (asset-only change
  = new Version), `derivative.trycmd` (BMP → PNG derivative, cross-platform), `push-strict*.toml`
  (exit 5), `push-unsupported.trycmd` (warnings, no manifest entries). Binary inputs are copies of
  `fixtures/assets/`; `.out/` goldens compare only text files (Sidecar, `.md`, `.svg`) — binary
  round-trips are proven by the re-push being `already vN`. HEIC/AVIF derivatives need macOS
  `sips` (or ImageMagick) and are covered by unit/integration tests that skip without one.
- Threads (ticket 04): `45-threads-seed/seed.trycmd` seeds `th-plan` with Threads t1–t6 (see its prose);
  `50-threads/*.toml` are read-only views of them (`thread list|show`, `show`, `status`, `mentions`,
  `pull` with `pull-threads.out/`), `55-threads-mut/lifecycle.trycmd` mutates its own `life-a` (a later group: `mentions` is
  cross-Document, so nothing may add mentions while `50-threads` runs). Non-TTY
  `thread list` rows omit the empty REPLIES field (as the CLI prototype does); TTY shows a blank cell.
- Corpus (M5): `94-corpus/seed.trycmd` pushes every Document of `fixtures/corpus/` and replays the
  RFC's history as `hist-rfc`; `copy.sh` copies them from `$ANNOTARY_E2E_FIXTURES` (set by the harness).
<!-- ---- ticket 05 ---- -->
- Actions (ticket 05): `60-actions-seed/seed.trycmd` seeds `act-plan` (approve by @rowan, send-back
  by @scribe on v1; a v2 that starts clean; approve by @scribe on v2) and `act-none` (no
  Action); `65-actions/*.toml` are read-only views (`status` incl. TTY/JSON, `show`, `actions`
  newest first, `list --action ID|none`, `pull` with `pull-actions.out/` = the Sidecar `actions`
  block golden, the exit-1/2/5 paths of `act`); `70-actions-mut/lifecycle.trycmd` mutates its own
  `act-life` (guards, repeated/two-Member Actions, per-Version clearing, pulled Sidecar pushed
  back as `already`, deleted). The Sidecar-edit flow (drop `inherited`, edit `available`, `[]`,
  invalid declaration) lives in `crates/annotary/tests/cli_actions.rs` (files edited between
  steps).
<!-- ---- end ticket 05 ---- -->
<!-- ---- ticket 06 ---- -->
- Access model (ticket 06): the harness config gains profiles `limited` (Member with `notes:
  read,comment` only — the exit-4 goldens) and `pw` (a **second** spawned server whose owner
  `@ana` has the local password `hunter2`; the profile carries only the server — `login` puts
  the token in and `logout` takes it out again). `75-access/*.toml` are independent read-only
  cases (missing-Right exit 4 with the `member grant` hint, owner-only exit 4, last-owner exit 5,
  bad right / unknown Vault / unknown Member, `token list` views incl. TTY, `login` refusal on the
  passwordless server, `profile list` incl. TTY/JSON, `json-help.trycmd` = `--json ?` of every new
  verb); `80-access-mut/` holds `members.trycmd` (`granted`: grant upsert / `*` drop / rename /
  role / disable and the refusals), `tokens.trycmd` (mint for another Member with name + expiry,
  own tokens, lists; `token-mint-tty.toml`), `login.trycmd` (login → works → wrong password →
  logout → `--default` → `--all` list → restore no default) and `reply-allowed.trycmd` (the
  ticket's scenario: reply ok, `act` exit 4, grant `action`, `act` ok, `write` refused);
  `85-profiles/profiles.trycmd` runs in a group of its own because it sets and clears the
  default profile. Token ids cannot be captured by trycmd, so mint → revoke → 401 is
  `crates/annotary/tests/cli_access.rs`, and the interactive `login` prompts (stdin) are
  `crates/annotary/tests/cli_login.rs`.
<!-- ---- end ticket 06 ---- -->
<!-- ---- CMD-5 ---- -->
- Vaults (CMD-5): `86-vaults/vaults.trycmd` runs in a group of its own because it creates Vaults
  on the shared store (a parallel `push` without `--vault` would see two Vaults and stop) — it
  deletes them again, so later groups see only `notes`. It walks `vault create | rename | list |
  delete` (non-TTY, TTY, `--json`) with exits 1/2/4/5, and `sh add-profile.sh` (`vaults.in/`)
  adds a profile on the harness server (the golden cannot spell its URL) that `vault rename`
  follows and `vault delete` clears. Renames made elsewhere healing a profile on the next call,
  chained renames, a reused name, hand edits and the last-Vault refusal are
  `crates/annotary/tests/cli_vaults.rs`; the headers themselves `tests/vaults_api.rs`.
<!-- ---- end CMD-5 ---- -->
<!-- ---- CMD-23 ---- -->
- Settings and Devices (CMD-23): `87-settings/settings.trycmd` runs for its own Member @kim:
  `sh setup.sh` (`settings.in/`) adds @kim, a profile `kim` on the harness server (removed again at
  the end, so no later `profile list` sees it) and Kim's Web (`?device=web`), iPhone and iPad (curl
  with `X-Annotary-Device`, each from a Token of its own used twice so `devices forget` has one
  to revoke). It walks `settings list | get | set | reset` (non-TTY, TTY, `--json`, `--device` by
  id and by name) and `devices list | rename | forget` with exits 1/2/5/6. The in-process twin with
  more cases (ambiguous names, the CLI never making a Device, forgotten Devices readable but not
  writable) is `crates/annotary/tests/cli_settings.rs`.
<!-- ---- end CMD-23 ---- -->
<!-- ---- ticket 09 ---- -->
- Rendering (ticket 09): `47-render/rerender.trycmd` pushes its own `rr-plan` (unique SVG Asset so
  the `assets:` line is deterministic) and re-renders it (`name id versions changed`, `--json`);
  `rerender-tty.toml` re-renders the seeded `ro-plan` (idempotent, so it can run in parallel);
  `rerender-usage|-not-found|-all-forbidden|-json-help.toml` cover exits 1/2/4 and `--json ?`.
  Since ticket 09 every Version references its rendered HTML blob, so `40-docs/lifecycle.trycmd`'s
  purge removes two blobs (Markdown + HTML). The rendered HTML itself is covered by the renderer's
  `insta` corpus (`fixtures/render/`), not by trycmd.
<!-- ---- end ticket 09 ---- -->
<!-- ---- ticket 07 ---- -->
- Re-anchoring (ticket 07): `48-reanchor-seed/seed.trycmd` seeds `ra-plan` — the sidecar
  prototype's scenario pushed for real (v1 → v2 → v3 → v4 = v1's text → v5 with one word changed
  under t2): t1 document (resolved), t2 span (refined on v5), t3 span (shifted → refined → anchored
  again; resolved), t4 whole-Asset Thread on `img/architecture.png`, t5 span, t6/t7 (v2) and t8
  (v3) orphaned by v4 and never revived; every push prints `threads: N re-anchored — …`. No
  @mentions (the `mentions` cases are cross-Document). `50-threads/reanchor-*.toml` read it:
  `thread list` (orphaned last, `vN:L orphaned "quote"` = last valid placement; TTY footers),
  `--status`, `--version N` (time-slice, JSON, TTY note, unknown N = exit 2), `thread show`
  refined `now:` / orphaned / `--json` (sparse history), `thread new --asset` exit paths, and
  `pull-reanchor.out/ra-plan.annotary` = the Sidecar golden with every Anchor.
  `55-threads-mut/reanchor.trycmd` mutates its own `ra-life`: an asset Thread and a span Thread
  orphaned by one push, restored bytes stay orphaned (`(unchanged)`), orphaned Threads still
  replyable/resolvable, `push --json threads`. The fixture table itself is replayed by
  `crates/diff/tests/fixtures.rs` and, through the server, `crates/annotary/tests/reanchor_api.rs`.
<!-- ---- end ticket 07 ---- -->
<!-- ---- ticket 08 ---- -->
- Events / `wait` (ticket 08): `72-wait/wait.trycmd` pushes its own `wt-plan` and walks every
  kind from `--cursor 0` (version, thread → `mention` for the agent it names, reply, resolve,
  reopen, action), the default `--on` for the agent vs the owner, `--on`/`--action` narrowing,
  `--json` (bare = one object per event line; narrowed), the TTY shape (two-space fields + the
  `wait: watching …` note), and exit-7 timeouts (`wait: nothing after 1s (seq N)`); then `wt-a` /
  `wt-b` whose Threads mention `@limited` prove `wait` without DOC spans Documents (`-p limited`
  — no other case may mention `@limited`). `wait-timeout*.toml` (stderr split, `--json`),
  `wait-json-help.toml`, `wait-on-bad.toml` / `wait-timeout-bad.toml` (exit 1), `wait-not-found.toml`
  (exit 2), `wait-unreachable.toml` (exit 6 with the `resume with --cursor` line) read `ro-plan` or
  profile `down`. Real blocking/waking and the SIGKILL-then-restart resume are
  `crates/annotary/tests/cli_wait.rs`; the API (incl. SSE) is `tests/events_api.rs`.
<!-- ---- end ticket 08 ---- -->
<!-- ---- ticket 10 ---- -->
- Web UI (ticket 10): the browser tests are **Playwright**, not trycmd — `tests/web/*.spec.ts`,
  run by `make web` (part of `make e2e`; `playwright.config.ts` at the repo root).
  `tests/web/global-setup.ts` spawns its own `annotary serve --port 0` (temp HOME) and seeds it
  with the real CLI: the sidecar-prototype scenario of `48-reanchor-seed/seed.in` as `ra-plan`
  (v1 → v5, t1–t8 incl. refined/orphaned/tombstoned, a fully tombstoned t9, Actions on v1 and v3,
  @rowan mentions) and `lst-*` Documents for the list page, plus a second server whose owner `@ana`
  has the password `hunter2` (the login page); `harness.ts` hands the URLs/ids to the specs.
  `document.spec.ts` = layouts A/C across 850px, the narrow header overlay, Thread rendering, the
  Version time-slice + banner + Action chips, media, what's new, "every Thread is in the first
  response"; `list.spec.ts` = the list page's columns/filters/title/more; `login.spec.ts` = login
  page iff a password. The HTML-level assertions (strings, redirects, 404/403 pages) are
  `crates/annotary/tests/web_pages.rs` under plain `cargo test`.
<!-- ---- end ticket 10 ---- -->
<!-- ---- ticket 11 ---- -->
- Web commenting / live (ticket 11): `commenting.spec.ts` (selection pill → composer → the card
  before the server answers, the posted range == `thread show`'s incl. selections over bold /
  link text, rollback on an injected 500 and a network failure, a whole-Asset Thread from an
  image, reply / resolve / reopen / edit / delete from an older-Version view, the 100k cap inline,
  Action buttons head-only / by intent / `[]` / rollback), `keyboard.spec.ts` (every binding and
  "nothing more"), `live.spec.ts` (CLI reply / Thread / resolve / `act` / `push` reaching the open
  page over SSE, what's new, the live conversation on an older-Version view). Every one of these
  tests pushes **its own Document** through `harness.ts`'s `cli()` / `seedDoc()` /
  `pushVersion()` (the shared `ra-plan` seed is never mutated — the ticket-10 specs read it);
  `actions.ts` has `selectText()` (a real DOM Range over the rendered text nodes). The server side
  (the card fragment `GET /d/{id}/t/{tid}?v=N`, the per-viewer controls, the `#cd-data` fields) is
  `crates/annotary/tests/web_commenting.rs`. Live-update waits are condition waits
  (`expect(...)` auto-retries), never fixed sleeps.
<!-- ---- end ticket 11 ---- -->
<!-- ---- ticket 12 ---- -->
- Web owner screens / login (ticket 12): `owner.spec.ts` — the Members screen (Add agent → the
  `ANNOTARY_TOKEN=` line, copied, and the Token driving `annotary status` at once; per-Token
  last used on the next draw; mint / revoke / rename / Advanced role + Grants / disable, each
  checked through the CLI's exit code), the Settings screen (Vault create → the default Actions
  editor → `push` into that Vault + `pull` → the Sidecar's `actions.available` with
  `inherited: true`, the Document view's buttons, rename, delete refused while it holds a
  Document), and a Member's hidden nav + 403 pages — all on the shared main server (unique
  handles / Vault names per run, nothing Tenant-wide changed). The two tests that change
  Tenant-wide state — the local password (set ⇒ the login page for a session-less browser ⇒
  signed-in session ⇒ remove) and the auto-purge setting — each spawn a **private server** with
  `harness.ts`'s `spawnServer()` (first-run setup + `serve --port 0` on a temp HOME; `stop()` at
  the end), so the parallel specs never see a password appear. The HTML-level assertions (what
  the pages carry, the card fragments, 403 for a Member, the Vault rename / delete API, the
  restyled login page) are `crates/annotary/tests/web_owner.rs` under plain `cargo test`; the
  Vault endpoints also sit in `access_api.rs`'s rights matrix.
<!-- ---- end ticket 12 ---- -->
<!-- ---- ticket 13 ---- -->
- Daemon / export (ticket 13): `90-daemon-export/` reads the harness's own running `serve`
  through `~/.annotary/server.json` — `daemon-status*.toml` (non-TTY, TTY, `--json` bare and
  narrowed; `ANNOTARY_DAEMON_OS=macos` is the seam that lets `status` run on any platform, it
  needs no launchctl), `daemon-unsupported*.toml` (`ANNOTARY_DAEMON_OS=linux` ⇒ exit 1, text
  and `--json`), `daemon-start|restart-not-installed.toml` (exit 5 — HOME is the temp dir, so no
  plist exists), `daemon-help.toml`, `json-help.trycmd` (`--json ?` of every new verb);
  `export.trycmd` exports the live harness store into its sandbox (`[..]` for counts/sizes; the
  "server running" note, exit 5 on an existing file, `--force`, `--json`, non-`.tar` exit 1,
  missing FILE usage error) and `export-no-store.toml` (fresh `ANNOTARY_HOME` ⇒ exit 5). The
  harness runs every case with `ANNOTARY_LAUNCHD_LABEL=com.annotary.e2e`, so nothing here
  can ever touch a developer's real `com.annotary.server`; the live launchctl round trip is
  `crates/annotary/tests/cli_daemon.rs` behind `ANNOTARY_LAUNCHD_TESTS=1` (`make
  launchd-test`), and the ticker / auto-purge / GC are `tests/housekeeping_api.rs`.
<!-- ---- end ticket 13 ---- -->
<!-- ---- ticket 14 ---- -->
- Skill file, help, the golden flows (ticket 14): `10-readonly/help.trycmd` + `help-short.toml`
  are the `--help` reference (the CLI prototype's text for the top level, `push`, `pull`,
  `thread`, `wait`, `status`, `act`, `member`, `daemon`, `skill`; `-h`, `-V`);
  `20-mutating/skill-install.trycmd` installs the skill into its sandbox (default location,
  `unchanged`, refused / `--force`, `--dir`, `--json`). `95-agent-flow/agent.trycmd` is the
  **agent golden flow**: it seeds the CLI prototype's scenario for real (`plan` = the sidecar
  prototype's text v1 → v4 = v1, eight Threads by @rowan / @scribe / @linter in the
  prototype's order with @mentions both ways, a tombstoned reply and root, Actions on v1 and v3)
  and then replays `session-agent.txt` as `-p agent`: `mentions` → `show` → `thread show` →
  `pull --assets` → `sh edit.sh` (the edit outside) → `push plan.md` → `thread reply --resolve`
  → `status` → `sh wake.sh` (a parked `wait` woken by @rowan's `act` from a second profile, synced
  on the TTY `wait: watching` note, then the same event read from `status`'s cursor as the
  non-TTY line and the `--json` record) → the `--json` reads → `thread new --version N` + the
  exit-5 guard → exits 4/2/7/6/3 → a second `plan` for the ambiguous-Name exit 2 (then deleted
  and purged so `plan` stays unique). `96-human-tty/human.trycmd` is `session-human.txt` in TTY
  mode (`ANNOTARY_TTY=1 NO_COLOR=1` per step) on the same `plan` plus its own `roadmap`
  (pull-over-local-edits, a push over the agent's v4 refused with the missing hunk and re-pushed
  after `sh merge.sh` — ticket 17, `pull --version N`), `login`/`logout` on the
  password server, `member add`, `list`, `profile list`, `skill install`, `token list`. The
  harness registers `sh` (`/bin/sh`) as a trycmd bin and puts the binary's directory on `PATH`
  for these scripts, and adds profile `linter` to `config.toml` right before `95-agent-flow`
  (not earlier: `75-access` asserts @linter's first Token was never used). `E2E_GROUPS=95-agent-flow`
  (comma list of group-name words) runs just those groups while iterating — goldens that read
  earlier seeds then fail; `make e2e` runs everything. The token-count guard that measures the
  agent's reads is `crates/annotary/tests/token_report.rs` (plain `cargo test`; `make
  token-report` regenerates `tests/token-report.md`).
<!-- ---- end ticket 14 ---- -->
<!-- ---- ticket 15 ---- -->
- Perf (ticket 15): not an e2e group — `make perf` is its own target (`perf/`: a release
  `serve` seeded by `perfgen seed` with the generated ≥ 5,000-line / ≥ 500-Thread Document,
  Playwright + CDP, median of 5 cold runs, the budgets of `perf/BUDGETS.md`; `perf/README.md`
  has the runner notes). What runs under plain `cargo test` is the smoke variant,
  `crates/annotary/tests/perf_smoke.rs`: the same generator at `Scale::SMOKE` seeded through
  the real binary against the in-process server, plus the page-level guarantees the budgets
  rest on and the suite's plumbing (the budgets table, the Makefile target, the runner docs).
<!-- ---- end ticket 15 ---- -->
<!-- ---- ticket 16 ---- -->
- The sweep (ticket 16): `10-readonly/json-fields.trycmd` pins `--json ?` of every verb in one
  golden (regenerate with `make goldens` after a deliberate field change — and review); `97-parity/`
  holds the TTY twins of the verbs no earlier golden ran on a TTY (`thread resolve|reopen|edit|
  delete`, `delete|restore|purge`, `member rename|disable`, `token revoke` via `revoke.sh`, `profile
  add|default|remove`) on its own `par-a` / `@par-agent` / profile `par-p` (no default profile left
  behind); `90-daemon-export/daemon-stop*.toml` / `daemon-uninstall*.toml` are the idempotent
  no-plist paths under the private label. A new verb needs a TTY + non-TTY case, a `json-fields`
  line and every exit path (tests/SPEC.md S16-04).
<!-- ---- end ticket 16 ---- -->
<!-- ---- ticket 17 ---- -->
- Concurrent edits (ticket 17): `98-concurrent/concurrent.trycmd` = two checkouts of its own
  `cc-plan` (Rowan's in the sandbox, @scribe's in `agent/`, edited by `agent-edit.sh` /
  `rowan-edit.sh` / `rowan-merge.sh`): the agent pushes v2, Rowan's push from v1 is refused with the
  missing hunk (non-TTY, TTY, `--json`), he merges it in and the same push is v3, the agent's
  stale unedited checkout is refused, then `--discard-head-changes` pushes over v3 and `wait
  --cursor 0` shows `discarding v3`. Seeds that used to push over a moved head on purpose
  (`30-docs-seed`, `40-docs/push-flow`, `45-assets/asset-change`) now show the refusal and then
  `--discard-head-changes`. The containment check itself, conflicted Sidecars, the Asset rule and
  the real-git worktree flow are `crates/annotary/tests/push_concurrent.rs`.
<!-- ---- end ticket 17 ---- -->
<!-- ---- ticket 18 ---- -->
- iOS app prerequisites (ticket 18): the **second** server (`pw`) is started with `--url
  http://pw-host.example.ts.net:7377 --link-base https://cd.example.dev`; profile `ios` (its owner
  @ana's first Token) joins `config.toml` right before `99-ios` only (any earlier `profile list`
  golden would list it). `99-ios/*.toml` are the main server's paths (no public URL: `link`'s note,
  `pair` exit 5, the exit-1/2/4 paths, TTY twins); `99-ios/ios.trycmd` pushes its own `ios-plan`
  on the second server: `link` through the relay, `show` on a TTY with the public link, `pair`
  non-TTY / TTY (the QR block is `...`, Tokens and ids `[..]`) / `--json`. `00-setup/
  serve-non-loopback.toml` pins the `--allow-remote` hint; the rest (remote binds, the peer rule,
  `serve --url` persistence, pairing API, titles) is `crates/annotary/tests/ios_prereqs.rs` and
  `server_bind.rs`.
<!-- ---- end ticket 18 ---- -->
