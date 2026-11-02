# Decisions

Smaller dated decisions: context, choice, the alternative rejected and why. Architecture decisions
are ADRs in [`adr/`](adr/). Entries use today's names; the project had another name until
2026-11-01. `CMD-n` is the project's old tracker ID: the items are not public, and the label stays
so that code comments and tests still point at the entry that explains them.

## 2026-10-21: `export` verifies its blobs instead of locking out GC (CMD-9)

An export racing a purge or GC could leave out a blob its database snapshot references. Chose to
check, after copying, that every blob the snapshot references is in the archive, and to fail the
attempt otherwise (the CLI retries on a fresh snapshot, up to 3 times, then exits 5 and leaves no
file); rejected a cross-process lock between `export` and the server's purge/GC because `export`
must work with no server and take no LOCK, and a lock file shared by two processes adds a stale
lock to recover from; rejected a grace period on blob deletion because purge removes its blobs at
once by design and a grace only narrows the window an export longer than it still loses.

## 2026-10-21: login throttle and cross-site refusal (CMD-4)

Chose an in-memory failed-login throttle per client (peer IP, IPv6 by /64) and account-wide for
non-loopback peers, shared by the web form and `POST /api/v1/login`; rejected a store table
because a restart clearing the counters is harmless and nothing else needs them. Loopback is never
locked by the account limit, so a remote guesser cannot lock the owner out of their own machine. Chose
`Origin` / `Sec-Fetch-Site` checks on every cookie or form mutation (bearer requests exempt) plus
the existing `SameSite=Lax`; rejected a CSRF token in the form because the pages are stateless and
every current browser sends `Origin` on a cross-origin POST, which the check refuses.
Rejected checking `/login` alone because `SameSite=Lax` still sends the cookie on a same-site,
cross-origin request (another port on the same host).

## 2026-10-21: the binary's version is stamped at build time (CMD-2)

Chose a build-time `ANNOTARY_VERSION` read by `option_env!`, with `0.0.0` in `Cargo.toml` and the
committed skill; rejected a `build.rs` running `git describe` because a dev build would then report
a stale or dirty tag and every crate rebuild would shell out to git. Dev and CI builds report
`0.0.0`; the goldens pin it.

## 2026-10-21: the server writes its own consistent snapshots for backups (CMD-1)

A backup that copies a data dir file by file can catch the live WAL database mid-write. Chose to
have the server write an `export_tar` of the store into `~/.annotary/snapshots/` at start and every
15 minutes (skipped when the event `seq` is unchanged, newest four kept); rejected a pre-backup hook
because the backup job is not this product's, and rejected a separate scheduled job because the
server already owns the store and a ticker beside housekeeping's needs no service manager. Chose
full tars (database plus blobs) over a database-only copy because a database copy paired with blob
files read at another moment could miss a blob a purge removed in between.

## 2026-10-21: no default profile; every CLI call names who it is

An agent replied on a Document as the owner: `config.toml` had a default owner profile next to an
`agent` profile, and the skill wrongly said `ANNOTARY_TOKEN` was already in the agent's environment,
so the CLI silently fell back to the owner. Chose to remove the default profile (`profile
default`, `login --default/--no-default`, `profile add --default`), so a call without `-p`,
`ANNOTARY_TOKEN` or `ANNOTARY_PROFILE` is exit 3 listing the profiles, and to make the `-p`
flag beat the environment so an agent naming its own profile is never overridden by a token or
profile the human exported in a shell the agent also loads; the skill tells agents to pass `-p`
on every command, find their profile with `profile list`, ask the human to create one when there
is none, and use the human's profile only when asked to post for them. Rejected detecting agent
harnesses from their environment variables (`CLAUDECODE`, `AI_AGENT`, `CODEX_*`) because the
list is incomplete and inherited by a human's shell started from a harness, and rejected a
person/agent kind on Members because neither is a security boundary on one machine (an agent can
read `config.toml`) and the goal is preventing mistakes. People typing commands pay one `-p`.

## 2026-10-23: the no-password login page names the loopback URL as text (CMD-14)

A browser on another device that opens `/login` while no password is set cannot be signed in, so
the page says how to get in. Chose to name the server's own loopback URL (`st.url`) in `<code>`;
rejected a link because on the remote device it would open that device's own loopback, and
rejected hard-coding `127.0.0.1:7377` because the port and bind vary. Chose 200 for the page;
rejected 401/403 because S18-02 defines `/login` as never 401 and the page is guidance, not an error.

## 2026-10-29: a request a local reverse proxy forwards is not loopback (CMD-17)

A reverse proxy on the same machine (Tailscale Serve, say) forwards every request from `127.0.0.1`,
so loopback trust would sign any of its clients in as the owner while no password is set, and the
login throttle would see one loopback client. Chose a header rule in `login_guard::client_ip`: a
loopback peer whose request carries `X-Forwarded-For`, `Forwarded`, `X-Real-IP` or
`Tailscale-User-Login` is remote, keyed by the last `X-Forwarded-For` entry (else `0.0.0.0`). It can
only take trust away: a browser on the machine sends none of them, and a remote peer's headers are
never read. Rejected a second listener for the proxy, because it changes `serve`'s flags for the
same result, and rejected a `--behind-proxy` flag that turns loopback trust off, because the owner's
first password needs a loopback browser (an SSH tunnel), which the header rule keeps.

## 2026-10-29: the skill's link rule and review flow

The skill says which link to give (only what `annotary link` prints; never loopback, `.local`, a LAN
address or a hand-built URL) and how a document written for the human is reviewed: the repo file is
the source, pushed as a copy, the approved Version committed in the PR.

- Chose to push a copy from `~/.cache/annotary/review/`; rejected pushing the repo file with
  `*.annotary` git-ignored, because every repo would need a `.gitignore` change and the Name
  would be the bare stem, colliding across repos (every repo has a `plan.md`).
- Chose the Name `<repo>:<path>` with `/` as `:` (`my-repo:docs:plans:x`); rejected allowing
  `/` in Names, because `pull` writes `<Name>.md` as one file and a Name is a file stem.
- Chose to fit it in the 60-line, ±20%-token skill by folding the exit codes and the keep-current
  hook into the commands section and shortening the profile rule; rejected raising the token
  band, which is the token report's contract with the prototype.

## 2026-10-29: the CLI speaks https with rustls and the bundled roots (CMD-18)

A server behind an https proxy answers only on https, and ureq was built with no TLS provider, so
every https server URL failed with "TLS required, but transport is unsecured". Chose ureq's `rustls`
feature: rustls with the ring provider and the bundled Mozilla roots (`webpki-roots`), pure Rust and
the same on macOS and Linux. Rejected `platform-verifier` (the OS trust store), because it adds
per-OS code and the servers the CLI meets have public certificates; rejected `native-tls`, because
it links OpenSSL on Linux. The roots are fixed at build time, so a release rebuild refreshes them. A
TLS failure now says "TLS with URL failed: …" (exit 6, kind `tls`) instead of "not reachable".

## 2026-10-29: Vault verbs, and profiles and Sidecars that follow a renamed Vault (CMD-5)

The CLI gains `vault list | create | rename | delete` over the existing `/api/v1/vaults` routes;
`vault rename` / `delete` also fix this machine's profiles on that server. A Vault renamed
elsewhere (the web Settings screen, another machine) is followed without anyone editing
`config.toml` or a Sidecar.

- Chose to have the CLI send the profile's Vault on every request (`X-Annotary-Profile-Vault`,
  plus `X-Annotary-Profile-Vault-Id` once known; names percent-encoded because header values
  are ASCII) and the Vault-scoped routes resolve it and answer `X-Annotary-Resolved-Vault` /
  `-Id` (or `X-Annotary-Deleted-Vault`) in the same round trip; the CLI then rewrites the
  profile. Rejected a separate "where is my Vault" request before each command (one more round
  trip on every call), and rejected the server writing `config.toml` (it never does, AGENTS.md).
- Chose to keep sending the old `vault=` query / body field beside the headers, the profile's
  rule applying when it equals the header; rejected sending only the headers, because an older
  server would then silently list every Vault instead of the profile's.
- Chose to record the Vault's id beside the name (`vault_id` in the profile, written only by the
  CLI) and to resolve with the events log (`vault.created` / `vault.renamed` / `vault.deleted`):
  an id that ever carried the profile's name follows that Vault through any chain of renames,
  even after a new Vault takes the old name; an id that never carried it means a human edited
  `vault`, and the name wins. Rejected resolving by name alone (a reused name would misroute the
  profile to a different Vault) and by id alone (a hand edit would be undone). No migration: the
  events log already holds the lineage, and the lookup only runs when the fast path (id and name
  agree, or the name is live) misses.
- Chose, for a deleted profile Vault, to clear it from the profile and fall back to every
  readable Vault for `list` / `resolve` / `mentions`, while `POST /api/v1/docs` refuses
  (`vault_deleted`) and `push` picks the Vault again as if the profile named none; rejected
  creating the new Document in some other Vault on the server's own initiative.
- Chose to echo a resolved Vault only when the caller holds the route's right on it (else 403
  naming the profile's name), and to put `Vault OLD was renamed to NEW` in an explicit
  `--vault OLD`'s 404 only for a caller who may read NEW; rejected echoing regardless, which would
  tell a Member a Vault's new name they cannot read.
- Chose to treat a git-conflicted Sidecar whose sides share the Document `id` but differ in
  `vault` as one Document across a rename (the newer side's name, the lower side's base);
  rejected keeping ticket 17's "conflicting Vaults — keep one side" (S17-07), because a Document
  id is unique in the Tenant and a Document never changes Vault, so a human had nothing to decide.
  Sidecars are never rewritten by scanning directories or on read-only verbs; push and pull
  binding by id and rewriting the file is enough.
- Chose to leave `crates/annotary/skill/SKILL.md` alone: Vault lifecycle is owner setup, not
  the agent's review loop (and the skill is at its token ceiling). The top-level `--help` gains
  one line (757 → 771 tokens, within the token report's ±20% of the prototype).

## 2026-10-29: the `daemon` verb stays; plain links in the skill

Chose to keep the `daemon` verb and the CLI's `127.0.0.1:7377` default: a local server on a Mac is a
product feature, whether or not any given install uses it. Chose, in the skill, "a plain Markdown
link (in code it can't be clicked)"; paid for by cutting asides (the `-p` precedence note, the
`*.annotary` token cost, `show`'s gloss, which the commands list keeps, the `wait` timeout default,
which the exit-code line keeps, and "one stderr line").

## 2026-10-30: the CLI's Device id is one top-level `device` line in `config.toml` (CMD-23)

Chose a top-level `device = "…"` (`Config::device`): one id for the install, used for every
profile and server, as iOS keeps one Keychain id across servers; Device rows are keyed by Member
and id, so one id never touches another Member's row. Rejected a `device` per profile because two
profiles on one machine are the same place, and a per-profile id would split one machine into
several Devices. Nothing writes it yet (no starter setting applies to kind `cli`); every rewrite
of `config.toml` keeps it. The `settings` / `devices` verbs send no `X-Annotary-Device`, so
the CLI makes no Device.

## 2026-10-30: `password` is a real data-dir verb (CMD-22)

Chose a shipped `annotary password --clear | --set` on a stopped data dir: it is the owner's way back
in after a forgotten password. Rejected a test-only binary because it would duplicate the store
wiring for one caller, and anyone who can run the verb on a data dir can already read its files, so
it opens nothing. Chose to refuse while a server holds the `LOCK` (it never changes a running store)
and to read a new password only from the prompt, never an argument, where shell history would keep it.

## 2026-10-31: tests that count events lock their server; loopback sign-in is unchanged (CMD-19, CMD-24)

A port watcher (an IDE's preview, say) sends `GET /` to a freshly bound loopback port, which on a
passwordless server signs a browser in as the owner, appends `session.started` and makes a Web
Device, so tests counting events or Devices across a wait flaked. Chose a password on those test
servers (`TestServer::start_locked`, `ANNOTARY_SETUP_PASSWORD` for a spawned `serve`); rejected
making `loopback_sign_in` skip the event or the Device, because loopback trust is the designed
behaviour and that sign-in is a real session worth recording; rejected locking every `TestServer`
by default, because about 40 test files and the web tests rely on loopback trust.

## 2026-10-31: spawned test servers stop through a guard in the tests, not a watchdog in `serve` (CMD-28)

Chose `ServeGuard` in `crates/annotary/tests/common/serve.rs`; rejected `annotary::testing`,
because it is compiled into the shipped library. Rejected an `ANNOTARY_EXIT_WITH_PARENT` watchdog
in `serve`, because it adds test-only code to the server and helps only when the test binary itself
is SIGKILLed, and Ctrl-C already reaches the whole process group. If leaked servers still show up,
file it.

## 2026-11-01: renamed to Annotary

The product's first name was taken: another project already had that name and idea. The product,
binary, crates, data dir (`~/.annotary`), env vars (`ANNOTARY_*`), URL scheme (`annotary://`),
Sidecar suffix (`.annotary`), skill, database file (`annotary.sqlite`), snapshot names
(`annotary-*.tar`), Token prefix (`ant_`), LaunchAgent label (`com.annotary.server`) and request
headers (`X-Annotary-*`) all took the new name. Chose no compatibility code for old names; rejected
aliases, because nobody outside the project had installed it and a clean setup is simpler. The `cd-`
prefix of the web pages' ids and classes and the `cd_session` cookie are internal and keep their
names.

## 2026-11-01: the brand renderer and the icon files

Chose a standard-library renderer for `brand/` (`scripts/brand/render.py`: a rounded-box rasterizer
and a PNG writer); rejected a pinned SVG library (resvg, cairosvg) because the freshness test runs
on Linux and macOS and pixels must match exactly, which IEEE arithmetic gives and a library's
platform-specific antialiasing does not promise. The script refuses any shape it cannot draw, so
a master that outgrows it fails loudly. The check compares PNG scanlines rather than compressed
bytes, because zlib differs by Python build.

Chose the small master for `icon.svg`; rejected the full master because a browser draws `icon.svg`
in the tab at 16 px, where the design round found the full master's dot disappears. Chose to serve
the manifest's three PNGs as well as the four routes the design names, because a manifest whose
icons are not public is broken. Chose `display: standalone`, for the installed web app.

## 2026-11-02: the release pipeline and the update notice (M3)

Chose cargo-dist (0.33.1) for the release workflow, tarballs and shell installer; rejected a hand-written
workflow because dist generates what we would otherwise keep up by hand. Three edits to what it generates are
kept (`docs/dev/releases.md`), and each is tested.

Chose to stamp the tag's version into the workspace in each release job; rejected committing the version at
each release, because the shipped version comes from the tag and the repo keeps `0.0.0`, and rejected a tag
that does not match, which dist refuses.

Chose our own job for the Homebrew tap, with a deploy key; rejected dist's `homebrew` job because it needs a
personal access token with `repo` scope and writes no `test do`, so its formula fails `brew audit --strict`.
dist still generates the formula, and our job adds the test, then audits, installs and tests before it pushes.

Chose `.tar.gz`; rejected dist's default `.tar.xz` because a minimal Linux container has no `xz`.

Chose `~/.local/bin` for the shell installer; rejected dist's default of Cargo's folder because the installer
is for people with no Rust.

Chose to ask for the latest release only from `-V` on a terminal and on a thread at `serve` start; rejected a
check on every verb because a CLI an agent calls hundreds of times must not touch the network, and rejected a
check on `-V` with no terminal because scripts call it. A build at `0.0.0` never asks, which keeps the tests
off the network without a setting in each harness. The setting is a line in `config.toml` or an environment
variable; rejected a new verb or flag because nobody needs it twice.

## 2026-11-02: a server's URL lives on the host (M4)

Chose `/etc/annotary/env` for `--url` and `--link-base`, with the unit requiring both; rejected optional
values, because `--url` is stored and the store would quietly keep an old one after the setting was lost.
`none` says "no URL" on purpose. For the same reason the health check reads that file: the health answer
alone stays right after the setting is gone.

Chose to add a missing key to an existing env file and never to change one that is there; rejected "create only
if missing", because an earlier installer created the file empty, and rejected overwriting, because the file is
the host's.

