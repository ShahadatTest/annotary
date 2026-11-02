# Spec ids — traceability index

"Comprehensive" means every spec id below has at least one test naming it, not line coverage.

**Scheme.** `STT-nn`: `TT` = the build ticket that owns the behaviour (`01`…`16`), `nn` = a
running number inside that ticket. Each line is `- STT-nn — one-phrase description (spec ref)`.
The ids come from the "From [ticket]" bullets of the test-strategy ticket
(the spec's Testing section) and the spec's decisions; the
`## Stories` section at the end maps every numbered story of the spec (1–167) to the ids that
cover it, and `## Deferred` lists, with reasons, what Phase 1 does not automate.

**How a test names its ids.** Put a marker line next to the test — the checker greps for the
literal `spec:` followed by ids:

- Rust: `// spec: S01-06, S01-08` (a comment line above the `#[test]` fn, or inside it)
- trycmd `.toml`: `# spec: S01-11`
- trycmd `.trycmd` / Markdown: `spec: S01-10` in the prose outside the fenced blocks
- TypeScript (Playwright): `// spec: S10-01`

The checker (`crates/annotary/tests/spec_ids.rs`, part of `cargo test` and `make check`)
fails when an id listed here has zero markers, when a marker names an id that is not listed here
(it reads `crates/`, `tests/` and `perf/`), when an id sits both in a ticket section and under
`## Deferred`, or when the `## Stories` map misses a story number of the spec or names an unknown
id. Ids under `## Deferred` need no marker (a gated test may still carry one). Do not put `spec:`
markers in implementation code — only in tests.

## Ticket 01 — walking skeleton

- S01-01 — `serve` binds loopback only; any non-loopback bind is refused with a clear error (story 7; ticket 18: unless `--allow-remote`, S18-01)
- S01-02 — a second `serve` on the same store is refused via `store/LOCK` (story 9)
- S01-03 — `server.json` `{url,pid,started_at,data_dir}` written on start, removed on clean exit; stale pid reads as not running (story 10)
- S01-04 — schema carries `PRAGMA user_version`; forward-only migrations run on `serve` start; a newer store is refused (story 13)
- S01-05 — `events.seq` Tenant-wide strictly monotonic and written in the same transaction as every mutation (failpoint between ⇒ neither persists) (Storage)
- S01-06 — bearer auth: valid Token → Member; missing/invalid/revoked Token or disabled Member → 401 with `{error,message,hint}` (API surface)
- S01-07 — owner-only / missing Right → 403 naming what is missing (API surface, story 29)
- S01-08 — Token `last_used_at` updated on every use (story 21)
- S01-09 — Tokens are opaque random ≥ 32 bytes base62 with the `ant_` prefix, sha256 at rest, shown once (story 22, Storage)
- S01-10 — first `serve` on an empty store runs first-run setup inline: owner handle + display name (defaults from `$USER`), optional argon2id password, Tenant + owner Member/Membership (role owner, `*` grant), Vault `notes`; ends by saying how the CLI gets a credential (stories 4, 5)
- S01-11 — `member add HANDLE [--name]` is one step (Member + Membership + `*` grant with all four Rights + Token) and prints `ANNOTARY_TOKEN=…` once; the pasted Token works (story 15)
- S01-12 — `member list` prints handle, name, role, grants; TTY (aligned, dim header) and non-TTY (tab-separated, no header) show the same fields in the same order (stories 17, 150)
- S01-13 — handles are kebab-case and unique per Tenant forever (including disabled Members) (story 19)
- S01-14 — credential resolution: who the caller is is always named, never assumed — the `-p/--profile` flag > `ANNOTARY_TOKEN` > `ANNOTARY_PROFILE` (the flag beats the environment, so an agent's `-p` is never overridden by what the human exported); `ANNOTARY_SERVER` overrides the server; profiles in `config.toml` (`server`, `token`, optional `vault`); no default profile — an older config's `default =` is read only for the error message and never written back (story 27; 2026-10-21)
- S01-15 — exit 3 message: "say who you are: pass -p NAME (`annotary profile list` shows whose each one is), or set ANNOTARY_TOKEN", then the existing profiles (and, when an older config names one, that its `default` is no longer used) (story 28)
- S01-16 — exit 6 message names the URL and the `serve` / `daemon install` hint; the CLI never hangs on a stopped server (story 11)
- S01-17 — `--json` bare = compact default set, `--json a,b` narrows, `--json ?` lists fields; minified; empty/null omitted; unknown field is a usage error (story 151)
- S01-18 — exit codes 0–7 defined centrally; errors are one line `error: MSG — HINT` on stderr, `{error,message,hint}` under `--json` (story 153)
- S01-19 — data dir (`~/.annotary`, `ANNOTARY_HOME`, `--data-dir`) holds `config.toml`, `server.json`, `logs/`, `store/` with `LOCK` and `meta.json` (schema version + tenant id) (story 12)
- S01-20 — `serve` runs in the foreground, logs to stderr, `--port` overrides 7377 (stories 1, 8)
- S01-21 — the server never writes a profile / `config.toml` (Access model)

## Ticket 02 — Documents & Versions core

- S02-01 — `push FILE.md` without a Sidecar creates a new Document (v1) and prints one line `name id v1` on stdout (stories 35, 59)
- S02-02 — `push` of a file with a Sidecar binds by Document id and creates the next Version (story 36)
- S02-03 — `push --doc DOC` binds an unbound file and writes the Sidecar; a Sidecar bound to another id is refused (exit 5) unless `--force` rebinds (stories 37, 38)
- S02-04 — Name = filename stem verbatim (Unicode/case), recorded per Version, the head's Name shown; renaming = pushing under a new filename; nested paths contribute only the stem (story 39)
- S02-05 — Name rules: empty / path separators / control chars / leading `.` rejected; `.md` extension required — typed errors (story 40)
- S02-06 — duplicate Names in a Vault are allowed; `list` always shows the id (story 41)
- S02-07 — DOC = id | Name unique in the Vault | path to a pulled `.md` (id from the sibling Sidecar); ambiguous Name ⇒ exit 2 listing `id name vN updated`; unknown ⇒ exit 2 (story 42)
- S02-08 — Vault selection: Sidecar → `--vault` → profile `vault` → typed error listing Vaults when there is more than one; single-Vault Tenants are never asked (story 43)
- S02-09 — push identical to head ⇒ no-op `already vN` (no Version, no event) (story 44)
- S02-10 — push identical to an older Version ⇒ new Version `vN (same content as vK)`; `same_as` derived from stored hashes, never stored (story 45)
- S02-11 — Version identity = sha256 over the Markdown blob hash + sorted `(ref, hash)` manifest; numbers sequential per Document; Versions immutable, never deleted; each records `{number, name, hash, by, at, message?}` (story 47, Implementation → Documents/Versions/Assets)
- S02-12 — head moved since the Sidecar's `base_version` ⇒ the commit is refused (409 `head_moved`, exit 5) unless it carries `contains_head` == head or `discard_head_changes` — never warn-and-overwrite (story 57; reworded by ticket 17, which owns the details: S17-02…S17-09)
- S02-13 — push prints progress/warnings on stderr and exactly one result line on stdout; `-q` silences progress; `--json` shape `{name,id,version,same_as?,already?,warnings[]}` (story 59)
- S02-14 — `pull DOC [--out DIR]` writes exactly `<name>.md` (byte-identical) and `<name>.annotary`; stdout `name.md name.annotary vN`, summary on stderr (stories 61, 65)
- S02-15 — `pull --version N` writes N's Name and a Sidecar with `base_version: N` (story 62)
- S02-16 — pull refuses (exit 5) over a local `.md` whose hash differs from the Sidecar's recorded base hash unless `--force` (story 63)
- S02-17 — `delete DOC` is soft (`write` right; Versions retained; hidden from `list`/`show`/`status` unless `--deleted`), `restore DOC` undoes it (story 66)
- S02-18 — `purge DOC` (owner-only) permanently removes a deleted Document and GCs blobs no other Version references (story 67)
- S02-19 — `versions DOC` lists `number by at message same_as` (story 68)
- S02-20 — `list [NAME]` prints `id name vN action|- open updated`, most recent activity first, limit 50 with a truncation note; positional Name substring (story 70)
- S02-21 — `list` filters `--vault`, `--deleted`, `--mentions-me`, `--action ID|none`, `--since T`, `--limit N` parse and apply (story 71)
- S02-22 — Sidecar is JSON `.annotary`, leaf-objects-inline / containers 2-space, fixed key order `format, id, vault, base_version, members, versions[], assets[], actions, threads[]`, `format: 1`, empty/null omitted; deterministic writer (projection → emit → parse → emit byte-identical) (stories 134, 135)
- S02-23 — Sidecar carries handles + a `members {handle: {id, name}}` table (story 136)
- S02-24 — push reads only `id`, `vault`, `base_version` (+ its hash) and `actions.available` when `inherited` is absent; hand edits elsewhere are ignored, not errors (story 137)
- S02-25 — YAML / JSONL / minified Sidecar variants are rejected on push with a typed error (story 140)
- S02-26 — Storage: `documents` / `versions` tables per the schema; Markdown stored as a content-addressed blob under `store/blobs/sha256/ab/<hash>`; Document ids are 8 random base32 chars without `0/1/o/l`, Tenant-unique (never sequential — ids stay stable across a later sync, no collision) (Storage)
- S02-27 — every Document/Version mutation appends an event in the same transaction (`document.created`, `version.pushed`, `document.deleted|restored|purged`) (Storage)
- S02-28 — `show DOC` / `status DOC` header lines (`name id vault vN by @h at "msg" action:- open:0 actions:…` / `name id vN action:- open:0 mentions-me:0 seq:N`) and their `--json` shapes; TTY appends the web URL for `show`/`push` (stories 88, 108, 154)
- S02-29 — timestamps: server clock, ISO-8601 UTC in `--json`, relative in text (story 133)
- S02-30 — TTY/non-TTY parity for `list` / `push` / `pull` / `versions` / `show` (story 150)
- S02-31 — rights: push/delete/restore need `write`, pull/list/show/status/versions need `read`, purge is owner-only; each refusal names the Right (stories 30, 32)
- S02-32 — two-phase blob upload: check lacking hashes → upload only those → commit; the server verifies uploaded bytes against their hash and refuses a commit whose Markdown blob is missing (story 54 hook)

## Ticket 03 — Assets

- S03-01 — push scans the Markdown for Markdown images `![](p)`, links `[](p)`, reference-style `[x]: p` and HTML `<img>/<video>/<audio>/<source>/<a>` `src|href`, resolved relative to the `.md`'s directory (`../` allowed); `http(s)://`, `#frag`, `mailto:`, `data:` ignored silently; absolute filesystem paths and `file://` warned and ignored (stories 48, 49)
- S03-02 — the supported types are one constant (text `.md .markdown .txt`; image `.png .jpg .jpeg .gif .webp .svg .heic .heif .avif .bmp .tif .tiff`; video `.mp4 .mov .m4v .webm`; audio `.mp3 .m4a .aac .wav .ogg .flac`; `.pdf`); anything else referenced locally warns, is not uploaded and the link is left as written (stories 48, 52)
- S03-03 — a linked `.md` is an Asset, not a Document: scanned recursively relative to its own directory, cycles ignored via a seen-hash set, depth capped at 5 (warning beyond); recursive entries carry `via` = the referencing file (story 50)
- S03-04 — a missing or unreadable referenced file warns and still pushes, with `missing: true` (no hash/size) in the manifest (story 51)
- S03-05 — `--strict` turns every push warning (missing/unsupported asset, absolute path, depth > 5, no derivative) into exit 5, before anything is uploaded (stories 51, 58; ticket 17 took "head moved" off the list — it is refused without `--strict` now)
- S03-06 — manifest entries are `{ref, hash, size, type, missing?, via?, width?, height?, derivative?}`, `ref` relative to the root Document's directory (Implementation → Documents/Versions/Assets)
- S03-07 — image/video dimensions are recorded in the manifest at push time (story 55)
- S03-08 — HEIC/HEIF/AVIF/TIFF/BMP get a web-safe derivative: a content-addressed second blob linked via `derivative`, the original bytes stored untouched (story 56)
- S03-09 — Version identity covers the sorted `(ref, hash)` manifest: an asset-only change (same Markdown) makes a new Version (story 46)
- S03-10 — caps of 2 MB Markdown / 100 MB per Asset / 500 MB per Version are server constants enforced with typed rejections (413 `too_large` → exit 5); the CLI checks the same constants before uploading (story 53)
- S03-11 — two-phase upload: the client sends the manifest (`{md_hash, assets[]}`) to `blobs/check`, the server lists the lacking hashes, the client uploads only those, then commits; blobs are content-addressed and de-duplicated Tenant-wide (across Documents); the server validates the manifest at commit (every blob present, shape) (story 54)
- S03-12 — the server serves every referenced blob at `/a/{hash}` with the manifest's content type (immutable caching; `?derivative=1` selects the web-safe derivative when there is one) (Implementation → API surface)
- S03-13 — `pull --assets` writes each Asset (recursive `.md` Assets and theirs included) to its referenced relative path, creating directories; a same-hash file is skipped, a differing file refuses the pull (exit 5) unless `--force`, missing Assets are skipped with a note; the stderr summary is `N threads, M assets (K missing: …)` (stories 64, 65)
- S03-14 — the Sidecar `assets[]` lists `base_version`'s manifest as `{ref, hash, size, type, missing?, via?}` and round-trips through pull → parse → emit (story 135)
- S03-15 — push prints `warning:` lines and `assets: N referenced, M new (SIZE), K missing (…)` on stderr (nothing when nothing is referenced); `--json` carries `warnings[]` and `assets` (story 59)
- S03-16 — the commit body accepts `placements[]`, `html_hash`, `renderer` (ignored until tickets 07/09) so later tickets extend the same call (Implementation → API surface)
## Ticket 04 — Threads on head

- S04-01 — Threads/Comments are append-only events stamped `{by, at, version}` (the head Version at that moment); ids per Document, sequential, server-allocated (`t7`, replies `t7.1`); a Comment stores `author` and `mentions` as Member ids; every Thread event is appended to `events` and bumps the Document's `last_activity_at` (Storage, story 84)
- S04-02 — `thread new DOC "L:C-L:C" | --document -m MSG|-` (`comment` right): a span anchor stores `original {version, range, exact}` (frozen), `current` (= original, `status: anchored`) and `history [{version, range, status}]`; new Threads only on head; empty spans refused (stories 74, 76, 90)
- S04-03 — `thread reply DOC ID -m MSG|- [--resolve]` (`-` = stdin), `thread resolve | reopen DOC ID`: bare events by anyone with `comment`, stamped `{by, at, version}`, no reason field; idempotent (stories 78, 79)
- S04-04 — `thread new --version N` ⇒ exit 5 when head ≠ N; the CLI verifies the quoted range exists on head before posting and refuses (exit 5) when it does not (story 75)
- S04-05 — coordinates: lines 1-based, columns 0-based Unicode code points, half-open, CRLF tolerated; `"L:C-L:C"` range strings everywhere (story 99, Anchor model)
- S04-06 — `@handle` mentions (kebab-case, unique per Tenant) resolved to Member ids at post time; not inside code spans / fenced blocks; unknown handles stay plain text; display-name renames never break history (stories 82, 19)
- S04-07 — `thread edit DOC ID -m MSG|-`: author or owner; sets `edited_at`; no history; a tombstoned Comment cannot be edited (story 80)
- S04-08 — `thread delete DOC ID` = tombstone: body and mentions cleared, `deleted {at, by}` kept, id kept; a tombstoned root keeps the Thread and its Replies; fully-tombstoned Threads hidden from lists and counts (story 81)
- S04-09 — 100,000-character total across all Comment bodies of one Document (server constant); posting/editing past it ⇒ exit 5 naming the cap and the current count (story 89)
- S04-10 — `mentions [--since T]` lists open Threads mentioning me across every Document I can read; `list --mentions-me` = Documents with ≥ 1 open Thread mentioning me (stories 83, 71)
- S04-11 — `thread list DOC [--all | --resolved] [--mentions-me] [--since T] [--sort recent]`: default open only, document order (Document-level first, then span by current position), Replies chronological; columns `ID AUTHOR ANCHOR BODY REPLIES(+N) AGE`; body = the root's first 120 chars, newlines collapsed, `(deleted)` for a tombstone; TTY footer hint (stories 85, 86)
- S04-12 — anchor column `vN:L "exact"` / `document`; `thread show DOC ID...` prints the full Thread with `vN:L:C-L:C "exact"`, created Version, mentions and every Reply (stories 87, 152)
- S04-13 — `show DOC` = header line + the open Threads as `thread list` rows (the ≈245-token read path); `status DOC` reports `open:N mentions-me:N` (stories 88, 108)
- S04-14 — Sidecar `threads[]` in the fixed layout `{id, subject, anchor?, author, created{at,version}, body, mentions?, edited_at?, deleted?, resolved?, replies?}` with handles not ids, ranges as `"L:C-L:C"`, empty/null omitted (`resolved` present ⇒ resolved); `members {handle: {id, name}}` covers every handle referenced; regenerated whole on pull; hand edits ignored on push (stories 135, 136, 137)
- S04-15 — `--json` for the Thread verbs: `thread list` default set `id author anchor body mentions replies resolved deleted created updated version`, `thread show` = the full Thread, `--json ?` lists fields; `show --json` carries `threads` (story 151)
- S04-16 — TTY/non-TTY parity for `thread list|show|new|reply`, `show`, `mentions` — same fields, same order (story 150)
- S04-17 — rights: `thread new|reply|resolve|reopen` need `comment`; `edit`/`delete` the author or an owner (exit 4 naming the author); reads need `read` (stories 30, 80; Access)
- S04-18 — unknown Thread/Comment ⇒ exit 2 `no Thread t99 in NAME (ID)`; a malformed or composite id (`plan/t7`) ⇒ exit 1; Thread mutations on a deleted Document are refused (stories 42, 153; CLI surface)

<!-- ---- ticket 05 ---- -->
## Ticket 05 — Actions & status

- S05-01 — a declaration is `{id, label, intent ∈ positive|neutral|negative}`; every Vault stores defaults `approve`/positive and `send-back`/neutral; an owner-only endpoint replaces them; changing the defaults affects only Documents without their own declaration, and only future Actions (story 102, Actions model)
- S05-02 — `act DOC ACTION [--version N]` (`action` right) records `{id, by, at, version, label, intent}` on head only; `--version N` ≠ head ⇒ exit 5 (`not_head`); the event is appended to `events` (`action.taken`) and bumps the Document's activity (story 104)
- S05-03 — an Action id not in the Document's effective declaration is refused with a typed error naming the available ids (409 `undeclared_action` → exit 5); Actions on a deleted Document are refused (story 104)
- S05-04 — Actions are append-only (no note, no retract); several Members / repeated Actions on one Version are all recorded; the last event on a Version wins for reporting (story 105)
- S05-05 — a new Version starts with no Action — no stale flag, nothing carried forward; older Versions keep theirs in the log (story 106)
- S05-06 — a per-Document `actions.available` (Sidecar, `inherited` absent) is read on push (`write` right) and replaces the Vault set entirely — `[]` yields no available Actions; label/intent snapshotted in the log survive a redeclaration (stories 103, 107)
- S05-07 — pull writes the effective set into `actions.available`, with `inherited: true` when it comes from the Vault; `inherited` present ⇒ push sends no declaration, absent ⇒ it does — editing `available` and dropping the flag is how a Document declares (stories 137, 138)
- S05-08 — `actions DOC` prints the log `id by at version label`, newest first, with the snapshotted label even after the declaration changed; `--json [{id,by,at,version,label,intent}]` (story 107)
- S05-09 — `status DOC` = `name id vN action:ID@vN by @h|- open:N mentions-me:N seq:N`, `--json {name,id,version,action{id,by,at}|null,open_threads,mentions_me,seq}`; TTY parity (story 108)
- S05-10 — `list` `action` column and `--action ID|none` filter are live; `show` header carries `action:ID@vN by @h|-` and `actions:a,b` (stories 70, 71, 88)
- S05-11 — no orphaned count anywhere by default: `status`, `list`, `show`, push output (story 109)
- S05-12 — Sidecar `actions {available[], inherited?, log[]}` in the fixed layout (`log` entries `{id, by, at, version, label, intent}`, handles, seconds), round-tripping through pull → parse → emit → push (story 135)
- S05-13 — rights: `act` needs `action` (exit 4 naming the Right); declaring needs `write`; Vault defaults are owner-only (exit 4); reads need `read`; purge drops the Action log (Access model, stories 30, 67)
- S05-14 — `act` prints `NAME vN ACTION by @h` (TTY parity), `--json {name,id,version,action,by,at}`; `--json ?` lists the fields of `act` / `actions` (stories 150, 151)
- S05-15 — an invalid declaration (bad shape, id or intent) is refused with a typed error — by the CLI before anything is uploaded (exit 1 `invalid_actions`) and by the server at commit (400 `invalid_actions`) (story 103)
- S05-16 — Action ids are kebab-case (≤ 39 chars), unique within a declaration and never `none`; `label` defaults to the id and `intent` to `neutral` when absent (Actions model)
<!-- ---- end ticket 05 ---- -->

<!-- ---- ticket 06 ---- -->
## Ticket 06 — Access model completion

- S06-01 — Rights are exactly `read / comment / action / write` per Vault, `read` implied by any other; a Membership holds Grants `{vault | "*", rights[]}` and the Grant that applies to a Vault is the Vault's own entry, else `*` (a Vault-specific Grant overrides `*` for that Vault); every endpoint checks the Right for the Document's Vault (stories 29, 30, 31; Access model)
- S06-02 — a `*` Grant covers Vaults created later; `member grant HANDLE VAULT|* RIGHTS` sets one Vault's Grant to exactly RIGHTS (`none` removes it), an unknown Vault or right is a typed error (stories 17, 31)
- S06-03 — owner-only, never grantable: purge, Vault create, Member/Membership management (add, rename, disable, role, grants), Vault default Actions, server settings incl. the local password, editing/deleting any Comment — role member gets 403 / exit 4 naming what is owner-only (story 32)
- S06-04 — role ∈ owner|member via `member role HANDLE owner|member`; the last owner cannot be demoted or disabled (409 `last_owner` → exit 5) (story 33)
- S06-05 — disable, never delete: a disabled Member's Tokens and sessions ⇒ 401, attribution/history intact, the handle stays taken forever; `member rename` changes the display name only (mentions keep resolving) (stories 18, 19)
- S06-06 — CLI `member rename|disable|grant|role` print the Member's updated row (TTY parity, `--json`), unknown Member ⇒ exit 2 (story 17)
- S06-07 — `token mint [--name] [--expires] [--member]` prints the Token once as `ANNOTARY_TOKEN=…` (hashed at rest, never listed); `token list` shows `MEMBER NAME CREATED LAST-USED EXPIRES ID` with `last_used_at` updating on use; `token revoke ID` ⇒ 401 afterwards, idempotent; a Member manages their own Tokens, an owner anyone's (stories 20, 21, 22)
- S06-08 — Token expiry: `--expires DUR|DATE` in the future; an expired Token ⇒ 401 and `expired` in `token list` (story 21)
- S06-09 — the local password lives on the owner Member (argon2id), set/changed/removed by an owner via `PUT /api/v1/settings/password`; `POST /api/v1/login {password}` exchanges it for a Token; wrong ⇒ 401 `wrong_password`, none set ⇒ 409 `no_password` (story 23; Access model)
- S06-10 — browser sessions: with no password a loopback request without credentials is signed in as the owner with an auto-issued `cd_session` cookie; with a password set the login page (`/login`) issues the cookie, remembered across requests; `/logout` ends it; a disabled Member's session dies (stories 24, 131)
- S06-11 — the API with neither cookie nor bearer ⇒ 401 (never auto-issued); the CLI with no credential never becomes the owner (Access model, story 25)
- S06-12 — `annotary login` prompts for server + local password, refuses (exit 5) with a hint when the server has no password, saves a profile (`-p NAME` or `ANNOTARY_PROFILE`, default the handle) and notes `use it with -p NAME` — there is no default profile; wrong password ⇒ exit 3 (stories 25, 26)
- S06-13 — `logout` revokes the profile's Token on the server and removes it from the profile; `profile list|add|remove` manage `config.toml` without ever printing a Token; `list` shows each profile's `@member`, so an agent can find its own (stories 26, 27)
- S06-14 — the exit-4 message names the missing Right and suggests `annotary member grant HANDLE VAULT RIGHT` (story 29)
- S06-15 — table-driven rights matrix: {read, comment, action, write} × every endpoint × {owner, member with `*`, member with a Vault Grant, member with none, disabled} → 200/401/403 (Testing seam 2)
<!-- ---- end ticket 06 ---- -->

<!-- ---- ticket 09 ---- -->
## Ticket 09 — Renderer & edge render

- S09-01 — one Markdown superset with no flavour detection — GFM (tables, task lists, strikethrough, extended autolinks), footnotes, fenced code, Mermaid fences, math, YAML front matter — parsed with the same options as the Asset scan (`refs::MARKDOWN_OPTIONS`) (story 146, Rendering 142)
- S09-02 — output is semantic HTML only: no `<style>` / `<script>` / `style=` / event handlers / chrome; raw HTML in the source is sanitized to an allow-list and `javascript:` URLs are dropped (story 142; ADR 0001)
- S09-03 — every block-level element the renderer opens carries `data-l="L-L"` (1-based, inclusive source lines of its content) so a rendered selection maps back to source coordinates; soft breaks stay newlines inside a block (story 142; Web UI decision)
- S09-04 — fenced code → `<pre><code class="language-x">` (no baked highlighting); ```` ```mermaid ```` → `<pre class="mermaid">` for viewer JS; math (`$…$`, `$$…$$`, ```` ```math ````) → MathML, a failed parse rendered as `<merror>` with the source (story 142)
- S09-05 — front matter is parsed out and never rendered; its `title:` (else the first H1's text) is the display-only title (stories 72, 146)
- S09-06 — local Asset references are rewritten from the manifest to `/a/{hash}`: `<img>` for images incl. SVG, `?derivative=1` and manifest `width`/`height` for HEIC/HEIF/AVIF/TIFF/BMP, `<video>`/`<audio>`/`<object>` for media/PDF referenced with image syntax; `missing` Assets become the marked placeholder `<span class="asset-missing" data-ref>`; links to Assets stay plain `<a href="/a/{hash}">`; unsupported / external / absolute references are left as written (stories 121, 129, 142)
- S09-07 — output is byte-stable for a given (Markdown, manifest, renderer version); the renderer version is recorded on each Version (story 142)
- S09-08 — `push` renders locally and uploads the HTML as a content-addressed blob in the two-phase flow (`blobs/check` learns `html_hash`); the commit carries `html_hash` + `renderer`; the server validates (blob present, `renderer` shape) and stores, never renders on push (stories 142, 144; ADR 0001)
- S09-09 — `GET /api/v1/docs/{id}/versions/{n}` reports `renderer` and `html_hash`; `GET …/versions/{n}/html` serves the rendered HTML as `text/html` (`read`) (story 143; API surface)
- S09-10 — a Version with no HTML, or a renderer older than `MIN_RENDERER_VERSION`, is rendered by the server with the same crate on first read, the blob stored and recorded on the Version (`version.rendered`); the server's fallback render equals the CLI's render for the corpus (story 143; Testing seam 2)
- S09-11 — `annotary rerender DOC | --all` re-renders every Version's HTML in place with the server's renderer (`write` on the Vault; `--all` owner-only, deleted Documents included); Version identity, numbers, Threads untouched; rows `name id versions changed`, summary on stderr, `--json` (story 143)
- S09-12 — the renderer (and sidecar) crate builds for `wasm32-unknown-unknown` (`make wasm-check`) (story 145)
- S09-13 — the HTML blob is referenced by its Version for blob GC and purge: a rerender leaves the old blob unreferenced, purge removes it (Storage)
<!-- ---- end ticket 09 ---- -->

<!-- ---- ticket 07 ---- -->
## Ticket 07 — Re-anchoring across Versions

- S07-01 — re-anchoring N → N+1 is a deterministic pure function of (text N, text N+1, anchor): a histogram line diff maps the span's lines, a Myers diff of the words inside changed lines maps the columns; sticky inside / non-sticky at the edges; identical in the CLI, the server and the WASM build (story 91; Anchor model)
- S07-02 — exactly three placement statuses: anchored (text equals the frozen quote, may have shifted), refined (grew / shrank / changed — the current text kept), orphaned (nothing survives — the last valid placement kept, stamped with the last Version it was valid on) (story 92)
- S07-03 — partial deletion / insertion inside a span shrinks / grows it (refined, D2–D5, G6); a whitespace-only line change keeps it anchored (G12 divergence); type-over, a find-and-replace that swallows the span (D10' divergence), a moved block and lines gone orphan it; boundary insertions fall outside (D6/D7) (stories 93, 94, 95)
- S07-04 — the empirical fixture table is pinned: one directory per row (D1–D12, G0–G14) plus the ticket-05 additions under `fixtures/anchoring/<id>/{v1.md, v2.md, …, threads.json, expected.json}` with declared expectations; a checker fails when a research row has no directory (story 101; Testing seam 3)
- S07-05 — orphaned is sticky (a later Version restoring the quote never revives it, G8); the function is deterministic and order-independent (story 95)
- S07-06 — `push` runs the re-anchoring for every Thread from the head onto the new text (incremental — the head's stored placements already carry every earlier Version, so a moved head needs no intermediate Versions), submits `placements[]` + `placements_from` with the commit and prints `threads: N re-anchored — a anchored, r refined, o orphaned (…)` on stderr (`--json` `threads`); Document-level Threads count as anchored (Implementation → CLI surface; ADR 0001)
- S07-07 — the server accepts placements from a `write` Member and applies them in the push transaction (`current` rewritten, sparse `history` appended only where the placement changed), places Threads left out itself, ignores a submission computed from a stale head, and spot-verifies with the same crate (`serve --verify-placements on|off`, on locally): a mismatch refuses the commit with 400 `invalid_placements` (ADR 0001; Testing seam 2)
- S07-08 — sparse history: an entry only where range/line/status changed; the placement at Version N is the last entry ≤ N (an orphaned entry ⇒ the last valid range stamped with the last Version it was valid on); the rule lives in the sidecar crate and is shared by CLI and server; sparse and full history agree at every N (the prototype oracle) (stories 100, 139)
- S07-09 — asset Threads: `thread new DOC --asset REF -m MSG` anchors by the Asset's content hash (+ the referencing line, display-only); on push the same hash referenced anywhere in the new manifest ⇒ anchored (line updated), gone ⇒ orphaned; anchor column `asset REF`; an unknown or missing ref ⇒ exit 5 (stories 74, 98)
- S07-10 — orphaned ⊥ resolved: orphaned Threads stay open, listed, replyable and resolvable; resolved and tombstoned Threads are re-anchored like the rest; Document-level Threads have no Anchor and never orphan (stories 96, 97)
- S07-11 — refined ⇒ `current.exact` stored (omitted in the Sidecar when equal to the original); Sidecar `anchor {original, current, history[]}` with `"L:C-L:C"` range strings round-trips (Sidecar format, story 135)
- S07-12 — `thread list DOC --version N` is a read-only time-slice: Threads created ≤ N, Replies ≤ N, resolved-state per the last resolve/reopen ≤ N, placement per the sparse history at N (a refined span's current text from Version N); unknown N ⇒ exit 2 (story 86; Thread model)
- S07-13 — `thread list --status anchored,refined,orphaned` filters by placement status (a Document-level Thread counts as anchored); default order = document order by current position, orphaned last (by original position) and shown with `orphaned` in the anchor column (its last valid placement `vN:L`); the TTY footer hints `--status anchored,refined`; every printed line number is stamped `vN:` (stories 85, 86)
- S07-14 — `thread show` prints the frozen quote and, when refined, `now: "<current text>"`; the anchor column reads `vN:L refined "quote"` (story 87)
- S07-15 — the diff crate builds for `wasm32-unknown-unknown` (`make wasm-check`) (Runtime decision)
- S07-16 — server spot-verification equals the CLI's placements over the whole fixture table (Asset lines included); mapping stays incremental across a Version the pusher never pulled (Testing seam 2)
- S07-17 — TTY/non-TTY parity for the ticket's outputs: `push` threads line, `thread list --status` / `--version`, `thread show` `now:`, asset Threads (story 150)
<!-- ---- end ticket 07 ---- -->

<!-- ---- ticket 08 ---- -->
## Ticket 08 — Events, `wait`, live feed

- S08-01 — every mutation appends to `events (seq, at, by, type, doc_id, payload)` in the same transaction; the Version (`version.pushed {version, name, message}`), Thread (`thread.created|replied {thread, comment?, version, mentions}`, `thread.resolved|reopened {thread}`, `comment.edited|deleted`), Action (`action.taken {action, version, label, intent}`) and Document lifecycle events carry the fields `wait` prints; `seq` is Tenant-wide and strictly monotonic; the events read surface (`Store::events_page`) filters by Document / type and reports the Document's Vault (Storage; story 113)
- S08-02 — a failpoint between the mutation and the event insert ⇒ neither persists; a rolled-back transaction wakes no waiter, a committed one fires the commit hook exactly once (Storage; Testing seam 2)
- S08-03 — `GET /api/v1/events?since=SEQ&wait=SECS[&doc=&kinds=&action=&limit=]` returns every event with `seq > since` the caller may read (`read` on the Document's Vault; Tenant-level events owner-only), oldest first, as `{events[], seq}` where `seq` is the cursor to continue from (it advances past non-matching events too); parks up to `wait` s (≤ 30) until a matching event lands and wakes on each kind; `since` absent = from the head (API surface; story 110)
- S08-04 — filters: `doc=ID` (404 unknown, 403 unreadable), `kinds=` (`wait`'s words `version,action,mention,thread,reply,resolve,reopen` or raw types; unknown ⇒ 400), `action=ID` narrows `action`; `mention` = the caller's Member id in `mentions` of `thread.created` / `thread.replied` (reported as `mention` when asked for, as the thread/reply it is otherwise); events carry `by` / `mentions` as handles, the Document's current `name` / `vault`, the raw `payload`, and Comment events an `excerpt` (tombstone ⇒ `(deleted)`); a cut page reports the last scanned `seq` and the next call continues without loss (API surface)
- S08-05 — `wait [DOC] [--on KINDS] [--action ID] [--cursor SEQ] [--timeout DUR]`: default `--on version,action,mention`; `thread,reply,resolve,reopen` opt-in; an unknown kind or a malformed `--timeout` ⇒ exit 1 before the server is asked; unknown / deleted DOC ⇒ exit 2 (stories 110, 111)
- S08-06 — `wait` prints every event that arrived together as `SEQ KIND DOC DETAIL` one per line and exits 0 — DETAIL `v5 by @rowan "msg"` / `approve v5 by @rowan` / `t2.2 by @rowan "…@you…"` / `t9 by @rowan "…"` / `t2 by @rowan`; TTY parity (two-space fields, a note on stderr); `--json` = one object per event line (`{seq,kind,doc,name,action,version,thread,comment,message,excerpt,by,at}`), `--json ?` lists the fields (stories 110, 150, 151)
- S08-07 — the cursor is a durable Tenant-wide integer: `--cursor SEQ` resumes; without it the first cursor is `status`'s `seq` (DOC given) or the head; `status` reports `seq:N`; `wait` exits 6 when the server goes away mid-wait (naming the cursor to resume from) and the same `--cursor` on the restarted server returns exactly what happened since — no gap, no repeat (SIGKILL `serve` + restart) (stories 108, 113)
- S08-08 — timeout: `wait: nothing after DUR (seq N)` on stderr, exit 7, nothing on stdout; `--timeout 0` = forever; `--json` ⇒ `{error: "wait_timeout", message, seq}` (story 112)
- S08-09 — `wait` without DOC covers every Document the Member can read; a Member without `read` on a Vault never sees its events; a purged Document's events stay readable to owners only (story 114; Access model)
- S08-11 — shutdown is bounded: on SIGTERM/SIGINT the server flips a shutdown signal that makes every parked long-poll answer at once (empty page, its cursor) and ends every SSE stream, then caps the graceful drain (`server::DRAIN_MAX`, 3 s) as a backstop — the process exits and `server.json` is removed within seconds however many `wait`/`EventSource` clients are attached (story 10; Runtime decision)
- S08-10 — SSE on the same resource (`Accept: text/event-stream`): a `ready` message naming the cursor, then `id: seq` / `event: type` / `data: {…}` per event as it happens, keep-alive while idle; `Last-Event-ID` resumes a reconnect; the web "what's new" / live feed read this same log (stories 115, 116)
<!-- ---- end ticket 08 ---- -->

<!-- ---- ticket 10 ---- -->
## Ticket 10 — Web: Document view + list, layouts A/C, time-slice, Playwright harness

- S10-01 — server-composed pages `/` (the Document list), `/d/{id}` (head) and `/d/{id}/v/{n}`: each a single HTML response that already carries the rendered Version (`render::ensure_html`), every visible Thread with its placement and the embedded placement data (`#cd-data`) — nothing fetches Threads after load (CSS/JS inlined); "what's new" reads `GET /api/v1/events` for the Document on demand (stories 117, 128; API surface; Web UI decision)
- S10-02 — a loopback browser with no local password is the owner: the first page issues the session cookie and `/login` sends it on; with a password set pages redirect to `/login?next=…` (only then), the form signs in, the session is remembered, Sign out ends it (stories 24, 131)
- S10-03 — layout A when the container is ≥ 850 px: rendered Markdown at ≤ 72ch + a right margin with Thread cards at their anchor line, pushed apart, Document-level cards pinned at the top; layout C below: one column, Threads inline as one-line rows (status dot · frozen quote · author · snippet · replies) directly under their block, expanding in place, the Document-level strip under the title; ultra-wide stays centred at the max widths — no third column; the same Thread element everywhere (story 117; Web UI decision)
- S10-04 — header: wide = name, Version switcher, status, resolved toggle, what's new, Action chip; narrow = one persistent line `name · vN · n open · n @you` + Version arrows, tap opens an overlay with the rest, dismissed on an outside click / Escape; no bottom bar (story 118)
- S10-05 — Thread rendering: anchored spans highlighted in the Document; refined = frozen `original.exact` label + `now: current.exact` + a dotted underline; orphaned grouped at the end behind a `▸ Orphaned (n)` disclosure, hidden by default; fully-tombstoned Threads hidden; tombstoned Comments show `(deleted)`; Members by display name + auth-method glyph (never a bot badge; harness/model secondary text when present); resolved Threads and their highlights behind the toggle; an Asset Thread highlights its inline Asset (stories 122, 123, 130; Thread model)
- S10-06 — Version time-slice: `<select>` rows `vN · by · at · same as vK · Action`, arrows; older-Version banner "Viewing vN · by · at · message — Threads as they were; replies/resolve work, new comments only on head" + Go to head; Threads as they stood on N (`Store::threads_at`); each Version shows its own Action chip (snapshotted label/intent, coloured by intent); no diff view (stories 124, 125)
- S10-07 — media inline from `/a/{hash}` with boxes reserved from the manifest dimensions (SVG via `<img>`, derivatives, video/audio/PDF), `missing` ⇒ the broken-asset placeholder, Asset links plain text opening in a new tab, "same content as vN", every timestamp localized client-side (stories 121, 129, 133)
- S10-08 — the Document list page is `annotary list`: columns `id name vN action open updated`, activity sort, filters (Name substring, Vault, deleted, mentions-me, Action id | none, since), H1 / front-matter title display-only, no unread markers, 50 rows + "more" (stories 70–73, 132)
- S10-09 — Comment bodies are rendered with the Document superset minus embeds (images / files as plain links, never asset-scanned), no `data-l` (story 77)
- S10-10 — the Playwright (TypeScript) harness runs against a real spawned `annotary serve` on an ephemeral port with a store seeded through the real CLI (the sidecar-prototype scenario + list Documents) and a second password-protected server; it is part of `make e2e` / `make check`; `// spec:` markers in `.ts` count (Testing seam 4)
- S10-11 — a span Thread's highlight (on load, after a reload, on a live re-draw) lands on the occurrence the Thread is anchored to when its quote occurs more than once in the searched blocks (two items of one list, twice on one line): each hit's source line = its innermost `data-l` block's first line + the soft breaks before it, and `#cd-data`'s `occ` (earlier starts of the text on that source line) picks the hit on the Thread's line; the first hit only when the position cannot be used (story 117; tracker CMD-7)
<!-- ---- end ticket 10 ---- -->

<!-- ---- ticket 11 ---- -->
## Ticket 11 — Web: commenting, Actions & live updates

- S11-01 — selecting text on the head Version shows a floating pill (`c`); pressing it opens a composer in the margin card / inline strip where the Thread will live; the selection is mapped from the rendered DOM to source coordinates via `data-l` + text offsets (`L:C-L:C`, 1-based lines, 0-based code points, half-open), verified against the head Markdown (fetched once) and posted with the head guard; no per-block `+`; no pill on an older Version (stories 119, 126; Anchor model coordinates; Web UI decision)
- S11-02 — clicking or selecting an inline image / video / audio / PDF shows the same pill and creates a whole-Asset Thread by content hash (the manifest `ref`); Asset links stay plain text — selecting their text makes a span Thread, clicking opens the Asset in a new tab (stories 120, 121)
- S11-03 — reply (with optional resolve), resolve / reopen, edit own Comment (owner: any), delete (tombstone with an inline confirm) work from any Version view and are stamped with the head Version; new Threads only on head; the 100k body-cap error is shown inline on the card; the card of an older-Version view keeps the time-slice placement while the conversation is live (stories 76, 78–81, 89; Thread model)
- S11-04 — Action buttons on the head view only, one per available Action coloured by intent, none when the declaration is `[]` or the caller lacks `action`; taking one posts with the head Version guard and shows the new chip at once (status line and Version select follow); older-Version views show their chip and no buttons (stories 104, 125)
- S11-05 — every mutation (new Thread, reply, resolve / reopen, edit, delete, Action) is applied to the page before the request (< 50 ms perceived), confirmed by the server's answer (the card re-drawn by the server's renderer), and rolled back with a visible error on a 4xx / 5xx / network failure — the composer / editor comes back with the text (story 127)
- S11-06 — keyboard: `j` / `k` next / previous visible Thread, `r` reply, `e` resolve / reopen, `c` comment on the selection, `[` / `]` older / newer Version, `↩` posts a reply, `⌘↩` (Ctrl↩) posts a comment, `esc` closes / deselects, `?` the help overlay — and nothing more; keys inside a field are text; modifier chords are not bindings (story 126)
- S11-07 — live updates: the page subscribes to `GET /api/v1/events?doc=` as SSE once ready; Thread / Comment events re-draw the card in place (new Threads at their block with their highlight), Action events update the chip, a new Version shows a banner offering to go to head (no auto-navigation; head-only controls go away), "what's new" lists recent events and grows while open; the page's own mutations are not applied twice (stories 115, 116; API surface)
- S11-08 — the selection → source mapping agrees with the CLI: the Thread posted from a browser selection has the same `L:C-L:C` range and frozen quote `thread show` prints for that text, including selections across inline syntax (bold, links) (story 99; Anchor model)
- S11-09 — `GET /d/{id}/t/{tid}?v=N` (web auth + `read`) returns JSON `{html, item, hidden, head, version}` — the Thread's card as the page renders it (mutation buttons only for a caller with `comment`, edit / delete only for the author or an owner), its `#cd-data` entry, `hidden` for a fully tombstoned Thread; `v=N` shows the placement as of N with the live conversation; 404 for an unknown Thread / Version; the head page carries the pill, the help overlay, the Action buttons and the extra `#cd-data` fields (`me`, rights, `md_hash`, `assets`, `actions`, `action`) (Implementation: Web UI)
- S11-10 — `esc` in an editor that holds unsaved text (the composer or a reply box with any text, an edit box whose text differs from the Comment's body) does not discard it: an inline `.cd-discard` confirm asks, "keep" (or `esc` again) keeps the text and the editor open, "discard" closes it; an empty / unchanged editor closes at once; Cancel still closes without asking (stories 126, 127; tracker CMD-6)
<!-- ---- end ticket 11 ---- -->

<!-- ---- ticket 13 ---- -->
## Ticket 13 — Daemon, export & housekeeping ticker

- S13-01 — `daemon install [--port] [--data-dir]` (macOS) writes `~/Library/LaunchAgents/com.annotary.server.plist` — `ProgramArguments [<binary>, serve, --data-dir DIR, --port PORT]`, `RunAtLoad`, `KeepAlive`, stdout/err → `DIR/logs/server.log` — then `launchctl bootstrap` + `kickstart` and prints `Installed PLIST → running, pid N, URL`; the plist text is a golden with the paths substituted and round-trips through the parser (story 2; Runtime and process)
- S13-02 — `daemon status` reads `server.json`: `running  pid N  since YYYY-MM-DD HH:MM  URL  store DIR/store  log DIR/logs/server.log  (vX)` or `not running  store …  log …`, a stderr note on whether the LaunchAgent is installed; a stale pid (process gone) or no file reads as not running; `--json {state, pid, since, url, port, store, log, plist, installed, …}`; TTY/non-TTY parity (stories 3, 10, 150)
- S13-03 — off macOS every `daemon` verb exits 1 with `daemon is unsupported here (OS) — run `annotary serve` under your service manager …` and touches nothing; `serve` is unchanged there (story 6)
- S13-04 — `daemon uninstall` (bootout + remove the plist; data dir kept), `start` (bootstrap), `stop` (bootout — installed stays, `start` or the next login brings it back), `restart` (`kickstart -k`; upgrade = replace the binary + `restart`); `start`/`restart` without a plist exit 5, `stop`/`uninstall` are idempotent; the live `launchctl` round trip runs only with `ANNOTARY_LAUNCHD_TESTS=1` (temp HOME + data dir, private label) — `launchctl` itself is not in CI (story 3; Testing: deferred)
- S13-05 — `daemon install` on an empty store runs first-run setup inline (the same prompts/env answers as `serve`) and ends with the owner Token and how the CLI gets a credential; a store held by a foreground `serve` is refused by the LOCK (exit 5) before anything is written; re-running `install` stops, rewrites and restarts (stories 4, 5)
- S13-06 — `export FILE.tar` = a consistent tar of `store/` (`meta.json`, a SQLite online-backup snapshot of the database, every blob; no LOCK/-wal/temp files) that works beside a running server and restores — untar into a fresh data dir — to a working `serve`; result `FILE  N blobs  SIZE  schema vK` (`--json`); refuses to overwrite without `--force` (exit 5), wants a `.tar` name (exit 1), no store ⇒ exit 5 (story 12; Data dir)
- S13-07 — the "purge deleted after N days" setting is off by default and owner-only: `PUT /api/v1/settings/auto-purge {days}` (`null`/`0` = off, invalid ⇒ 400, Member ⇒ 403), `GET /api/v1/settings` reports `auto_purge_days`, `setting.auto_purge` event — ticket 12's settings screen drives it (stories 32, 67)
- S13-08 — auto-purge purges a deleted Document only when the setting is on and `deleted_at` is ≥ N days ago, through the purge path (rows, `document.purged {name, auto: true}` with no actor, blobs no other Version references); younger or restored Documents are left alone; running twice is a no-op (story 67; Runtime: ticker)
- S13-09 — the hourly in-process ticker: `serve` runs auto-purge → blob GC → derived-blob cleanup every interval (1 h; `ANNOTARY_HOUSEKEEPING_INTERVAL` seconds is the test seam) and logs one line; `POST /api/v1/housekeeping` (owner) runs one tick now and returns the report; no external cron (story 14; Runtime and process)
- S13-10 — blob GC never drops a referenced blob: every blob any Version references (Markdown, Assets, derivatives, rendered HTML — including blobs shared across Documents) survives; unreferenced blobs older than the grace period go, younger ones wait; an explicit `purge` runs the GC right after (`gc_removed`) (story 67; Testing seam 2)
- S13-11 — derived-blob cleanup: rendered HTML whose recorded `renderer` is older than `MIN_RENDERER_VERSION` is un-recorded (`html_hash`/`renderer` null, one `housekeeping.cleaned` event) and its blob removed unless another Version references it; current renders stay; leftover temp files of interrupted blob writes older than the grace period are swept (Runtime: derived-blob cleanup)
<!-- ---- end ticket 13 ---- -->

<!-- ---- ticket 12 ---- -->
## Ticket 12 — Web: owner screens & login page

- S12-01 — the Members screen (`/members`, owner-only) lists every Member with handle, display name, role, a Grants summary, the auth method as a hint (password / Token glyph + words — never a "bot" badge) and every live Token with name, created, `last_used_at`, expiry and id; rename and disable are one click away, role and Grants editing sit behind an "Advanced" disclosure (story 34; Web UI: owner screens)
- S12-02 — "Add agent": one handle (+ optional name) creates the Member + Membership + `*` all-rights Grant + a Token in one step and shows the Token once as the `ANNOTARY_TOKEN=…` line with a Copy button; the Token works from the CLI at once; Tokens can be minted (name / expiry) and revoked per Member from the screen (stories 15, 16, 21)
- S12-03 — the Settings screen (`/settings`, owner-only): set / change / remove the local password through `PUT /api/v1/settings/password` (setting one makes the login page appear on the next request without a session while the current session stays); edit each Vault's default Actions as `{id, label, intent}` rows (add / remove / reorder, saved whole) — a Document pushed afterwards without its own declaration pulls them as `actions.available` with `inherited: true`; the "purge deleted after N days" toggle + N persists through `PUT /api/v1/settings/auto-purge` (+ run housekeeping now); Vault create (stories 23, 67, 102; Actions model)
- S12-04 — Vault rename and delete (owner-only): `POST /api/v1/vaults/{name}/rename {name}` renames and the Grants naming the Vault follow (a taken name is 409); `DELETE /api/v1/vaults/{name}` removes an empty Vault and drops the Grants naming it, refuses a Vault that still holds Documents — deleted ones included — (409 `vault_not_empty`) and the last Vault (409 `last_vault`) (story 32; Access model)
- S12-05 — (changed on purpose by CMD-23, see S29-20) every Member gets the Settings screen at `/settings` (200) and a Settings link in the top bar; a Member (role member) gets the 403 page for the owner pages under `/settings/server/*` (`members`, `vaults`, `general`; `/members` 308s there) and 403 JSON for their card fragments (`/members/{handle}`, `/settings/vaults/{name}`, unchanged and never redirected); the top bar and the sidebar's Server group link the owner pages only for owners (story 32; Web UI)
- S12-06 — the login page wears the shared shell (same stylesheet, `.cd-loginpage`): shown only when a password is set, the form signs in and the cookie session is remembered, Sign out ends it; wrong password ⇒ 401 + the form with the error (story 131)
<!-- ---- end ticket 12 ---- -->

<!-- ---- ticket 14 ---- -->
## Ticket 14 — Skill file & agent golden flow

- S14-01 — SKILL.md is embedded in the binary: Agent-Skills front matter (`name: annotary`, a trigger-word `description`, `metadata.version` = the crate version, `metadata.cli: annotary`), body = the rules (read via CLI never the file; cite id + quote never line numbers; pull → edit outside → push; reply then `--resolve`; `--version N` guards; wait don't poll; `--json` when parsing), the command table, the exit codes and the keep-current hook; ≤ 60 lines, ≈ 900 tokens; no `references/` file — `annotary <command> --help` is the reference (stories 147, 149; CLI surface: skill file)
- S14-02 — `skill install [--dir DIR] [--force]` writes the embedded SKILL.md byte for byte to `DIR/annotary/SKILL.md` (DIR defaults to `./.agents/skills`; dirs created) and prints the path; `metadata.version` in the written file equals `annotary --version`; an identical file is left alone (`unchanged`), a differing one (older version, local edit) is refused without `--force` (exit 5); `--json {path, version, written}`; nothing is written anywhere else (story 148)
- S14-03 — `--help` is the reference (≈ 700 tokens on demand): the top-level help (`-h` = `--help`; `-V` the version) and the `push`, `pull`, `thread`, `wait`, `status`, `act`, `member`, `daemon` helps match the CLI prototype's `help.txt`, adjusted only where a later ticket changed a flag; every flag a verb accepts is named in its help; usage errors keep clap's `Usage:` line (story 149; CLI surface)
- S14-04 — the agent golden flow: the prototype's `session-agent.txt` runs for real against a spawned `serve` seeded with the prototype's scenario — `mentions` → `show` → `thread show` → `pull` → edit outside → `push` (the `assets:` / `threads:` summary lines) → `thread reply --resolve` → `status` → `wait` woken by an `act` from a second profile → the `--json` variants (`status`, `thread list --json id,anchor,body`, `--json ?`, the `wait` record) → `thread new … --version N` success and the exit-5 guard → the typical failures (exit 2 unknown Thread / ambiguous Name, 3 no credential, 4 missing right, 6 unreachable, 7 timeout) (stories 147, 150–153; Testing seam 1)
- S14-05 — the human transcript (`session-human.txt`) in TTY mode (`ANNOTARY_TTY=1`): the same verbs print the same fields in the same order as non-TTY, aligned with a dim header, relative ages, the web URL after `push`/`show`; the safety rails (pull over local edits, a push over a moved head refused with the missing hunk and re-pushed once merged — ticket 17, `pull --version N` note), `login`/`logout`, `member add`, `list`, `thread list` with its footers, `act`, `profile list`, `skill install`, `token list` (stories 150, 154; CLI surface)
- S14-06 — TTY / non-TTY parity across the flow: `show`, `status`, `thread list`, `thread show`, `mentions`, `wait` print identical fields in identical order in both modes — only separators, header and alignment differ (story 150)
- S14-07 — token-count guard: the o200k counts of `status`, a `wait` event line, `mentions` (one hit), `show` (header + five open Threads), `thread show`, the top-level `--help` and SKILL.md, measured from real output of the real binary on the prototype's scenario, stay within ±20% of the prototype's numbers (24 · 12 · 58 · 245 · 117 · 688 · 905; `status --json` 61, `thread list --json id,anchor,body` 188); the regenerated report is `tests/token-report.md` (Further Notes: measured targets; story 147)
<!-- ---- end ticket 14 ---- -->

<!-- ---- ticket 15 ---- -->
## Ticket 15 — Perf suite & large-Document generator

- S15-01 — the deterministic seeded generator (`crates/perfgen`, checked in; its output is not): `(Scale, seed)` → ≥ 5,000 lines / ~50k words of realistic Markdown (headings, paragraphs, lists, tables, code, quotes, images with real dimensions), ≥ 500 Threads across ≥ 4 Versions whose edits leave the placements mixed (anchored / refined / orphaned), Replies, @mentions, resolves / reopens, tombstones, Actions on two Versions; the same seed gives a byte-identical plan; every planned span is on the Version it is planned for; seeded through the real `push` (render + re-anchor at the edge) and the real `/api/v1` (story 128; Testing seam 5)
- S15-02 — `make perf`: a release build, a real spawned `serve` seeded with the fixture, Playwright + CDP on localhost, the browser cache disabled and cleared per run, median of 5 runs, the absolute budgets asserted — Document view FCP < 100 ms, interactive < 200 ms, Version switch < 150 ms, Document list < 100 ms, CLS 0 after first paint, optimistic reply < 50 ms perceived (the DOM updated and painted before the server answers) — a table printed, any miss fails the suite (story 128; Testing seam 5: "absolute budgets, no CI multiplier")
- S15-03 — server TTFB per page (Document view, Version page, list) is measured and reported separately, not budgeted; DOMContentLoaded, page bytes and the layout-C numbers are reported too (Testing seam 5)
- S15-04 — the budgets live in `perf/BUDGETS.md` and nowhere else: the runner reads them from that table; a budget may be raised only by the budget-raise rule (≥ 3 genuine measured attempts, one budget at a time, the record in that file) and a raised budget is still asserted (ticket 15 rule)
- S15-05 — the smoke variant: the generator at its small scale seeded through the real binary against the in-process server under plain `cargo test` (`make e2e` / `make check` run it too) — counts, the mixed statuses, Actions on two Versions, mentions — so the generator cannot rot between perf runs (Testing: Commands and budget)
- S15-06 — on the generated Document the first HTML response still carries every Thread with its placement (nothing fetches Threads after load), media boxes come from the manifest dimensions, and the viewport-first rendering (`content-visibility: auto` from the 61st top-level child on, never a strip's block) is CSS on the same one-response page; the inline scripts are deferred modules so the page paints before they run (story 128: "Threads/placements in the first HTML; media boxes from manifest dimensions; no font/CSS swap"; Implementation → Web UI)
- S15-07 — `make perf` is its own target (ticket 16 wired it into `make check`; `PERF=0` / `make check-ci` leave it out), documented for a dedicated Mac (what to install, how it is invoked), and never run on a GitHub-hosted runner (Testing seam 5)
<!-- ---- end ticket 15 ---- -->

<!-- ---- ticket 16 ---- -->
## Ticket 16 — Traceability closure & `make check`

- S16-01 — the rights matrix covers every route added after ticket 06 — the events log (`GET /api/v1/events`, `?doc=`), rendered HTML (`GET …/versions/{n}/html`), `POST …/rerender` / `POST /api/v1/rerender`, housekeeping + the auto-purge setting, the web pages (`/`, `/d/{id}`, `/d/{id}/v/{n}`, `/members`, `/settings` — HTML 403 pages) and their fragments (`/d/{id}/t/{tid}`, `/members/{h}`, `/settings/vaults/{v}` — JSON), both logouts — × every kind of caller → 200/401/403 with the Right or "owner-only" named (Testing seam 2; Access model)
- S16-02 — the Sidecar round trip carries every field the final format defines: `members` (id + name per handle), `versions[]` with `message`, `assets[]` with `via` and `missing` (a web derivative stays on the Version record — identity is `(ref, hash)` pairs), `actions {available, inherited, log[]}` across two Versions, Threads of every subject with `anchor {original, current (+ `exact` when refined), history[]}` (refined with a two-entry history, orphaned frozen at the last valid placement, asset by hash + ref + line), `mentions`, `edited_at` (root and Reply), tombstoned Reply and root, `resolved`, `replies[]` — built for real through the binary and the server: projection → parse → emit byte-identical, `pull` writes the same bytes, the pulled tree pushes back as `already`; the proptest covers the Action log and the root's `edited_at` / `deleted` too (stories 134–139; Testing seam 3)
- S16-03 — `push` refuses a moved head whose changes the file lacks (`head_moved`, exit 5) before the re-anchoring line and before anything is uploaded, `--strict` or not — the CLI checks the head it already read for the re-anchoring, the server still refuses at commit (stories 57, 58; S03-05's "before anything is uploaded"; ticket 17 made it the default, it was `--strict` only)
- S16-04 — CLI parity sweep: every verb has a TTY and a non-TTY golden printing the same fields in the same order (`thread resolve | reopen | edit | delete`, `delete | restore | purge`, `member rename | disable`, `token revoke`, `profile add | default | remove`, `daemon start | restart | stop | uninstall` joined the rest; `serve` is the foreground server — its first-run prompts are stdin-scripted in `00-setup/` and `cli_login.rs`; `daemon install` itself is Deferred S13-12), and `--json ?` of every verb is pinned in one golden (`10-readonly/json-fields.trycmd`) (stories 150, 151, 153)
- S16-05 — the spec-id checker fails on an id with zero tests that is not Deferred, on a marker naming an unknown id, on an id that is both listed and Deferred, and on a story of the spec missing from the `## Stories` map (or mapped to an unknown id); it runs in `cargo test` and `make check` (Testing Decisions: "Comprehensive")
- S16-06 — `make check` = `fmt-check` + `clippy` + `test` + `e2e` (trycmd + Playwright) + `spec-check` + `wasm-check` + `perf` (`PERF=0 make check` / `make check-ci` leave `perf` out where the release toolchain or Chromium is absent); CI (`.github/workflows/checks.yml`) runs on pull requests and pushes to `main`: `cargo test` on Linux and macOS, `make e2e` on Linux; `make perf` runs on one dedicated Mac only, never on a GitHub-hosted runner, and only where `.github/workflows/` and `perf/README.md` say (Testing: Commands and budget)
- S16-07 — the Phase 1 definition of done is recorded in `docs/PHASE1-DONE.md` (all 16 tickets closed, `make check` green incl. perf, every budget met on the fixture — any raise listed under "Raised budgets" in `perf/BUDGETS.md`, currently none) and linked from AGENTS.md ("Read first")
<!-- ---- end ticket 16 ---- -->

<!-- ---- ticket 17 ---- -->
## Ticket 17 — Concurrent edits: `push` checks content, not version numbers

- S17-01 — `contains_changes(base, head, local)`: base→head hunks present in local ⇒ contained; untouched ⇒ missing; overlapping edits resolved at word-token level; both appending at one point ⇒ contained when the head's insertion is a run of local's; pure function (CRLF / final newline ignored, oversized hunks bounded, deterministic); property tests: a stale file never contains a change, and with A's and B's edits separated by an unchanged unique line, `A∪B` contains A while B alone does not (stories 155, 156)
- S17-02 — a push whose file contains the head's changes over a moved head is a normal push from the head: no warning, re-anchored head → local, the commit carries `contains_head` (story 155)
- S17-03 — missing changes ⇒ refused before re-anchoring and before any upload, exit 5, the message names head / author(s) / base / file (`v5 by @x changed plan.md since your v4 — those changes are not in your file — put them into plan.md and push again`; `v5–v7 by @a, @b` for several), the missing base→head hunks as a unified diff on stderr (stories 156, 157)
- S17-04 — `--json` refusal: `{error: "head_moved", message, hint, base, head, by[], missing[{hunk, lines[]}], assets[{ref, change}]}` (story 157)
- S17-05 — a stale, unedited file (local == base) is refused, never reverts the head (story 160)
- S17-06 — an Asset the head changed (same `ref` new hash; added; removed) counts as a head change (`asset REF changed in vN`); local may change refs the head did not touch (story 161)
- S17-07 — a git-conflicted Sidecar (two-way or diff3 markers, around any section): same id ⇒ the lower `base_version` is used and push / pull work, the Sidecar is rewritten clean; differing ids ⇒ exit 5 `keep one side` (CMD-5: differing Vault names with the same id are a Vault rename, S26-04); differing `actions.available` ⇒ none sent + a note (story 158)
- S17-08 — `--discard-head-changes` pushes over the head with a stderr warning naming what was discarded; `version.pushed` carries `discarded: [versions]` (the response's `discarded[]`, `wait`'s `discarding vN` / `--json discarded`) (story 159)
- S17-09 — server: `contains_head` ≠ head ⇒ 409 `head_moved` "head moved again"; neither `contains_head` nor discard over a moved head ⇒ 409 `head_moved` (older CLIs are refused); `--verify-edge on` (default; `--verify-placements` is its alias) recomputes containment and refuses a mismatch, `off` trusts the edge (stories 155, 156)
- S17-10 — the git worktree flow end to end: A pushes, B merges A's branch in git (the conflicted `.annotary` committed with its markers), B pushes, the head contains both edits, Threads from both sides stay anchored (stories 155, 158)
<!-- ---- end ticket 17 ---- -->

<!-- ---- ticket 18 ---- -->
## Ticket 18 — iOS app prerequisites: remote bind, public links, pairing, titles, "Open in Annotary"

- S18-01 — `serve --bind ADDR` accepts any IP only with `--allow-remote`: without it a non-loopback bind is refused before the store is touched (exit 1, the hint names `--allow-remote`); with it the server listens there, `server.json` / `health` name a URL this machine can reach (an unspecified bind ⇒ its loopback address) and one warning line says the API is reachable from the network and a local password is needed for web access (stories 7, 162)
- S18-02 — loopback trust stays tied to the *peer*: a request from a non-loopback peer with no password set is never signed in (a page ⇒ `303 /login`, anything else ⇒ 401; `/login` signs nobody in), while a bearer Token works over a non-loopback listener (stories 24, 162)
- S18-03 — `daemon install --bind ADDR --allow-remote --url URL --link-base URL` writes them into the plist's `serve` arguments (render ⇄ parse round trip, `--allow-remote` valueless); a non-loopback `--bind` without `--allow-remote` or a bad URL is refused (exit 1) before anything is written (story 162)
- S18-04 — the `public_url` / `link_base` settings: `serve --url URL` / `--link-base URL` validate (absolute http(s), no user info / path / query / fragment, trailing `/` trimmed) and persist them; a later `serve` without the flags keeps them; `none` clears; each change appends `setting.public_url` / `setting.link_base {url}` (none when unchanged); `GET /api/v1/health` (unauthenticated) and `GET /api/v1/settings` carry both (null when unset) (story 163)
- S18-05 — `doc_link(public_url, link_base, id, thread)`: none without a public URL; `{public_url}/d/{id}[#tN]` with it; `{link_base}/{host[:port]}/d/{id}[#tN]` with both — only when the relay rule (port ⇒ `http://`, no port ⇒ `https://`) reconstructs the public URL, never for an IPv6 host (the plain public link instead) (story 163)
- S18-06 — `link DOC [tN]` prints the link on stdout (TTY and non-TTY alike); no public URL ⇒ the CLI's own `…/d/{id}[#tN]` + the stderr note naming `serve --url`; an unknown Thread ⇒ exit 2 (`no Thread tN in NAME (ID)`), `plan/t7` or a malformed id ⇒ exit 1, a deleted / unknown Document ⇒ exit 2; `--json` `{url, name, id}` (+ `thread`, `public`); the web page selects the Thread of a `#tN` fragment; SKILL.md tells agents to hand people this link (stories 163, 164)
- S18-07 — on a TTY `show` / `push` append the public link when the server has a public URL (else `[server]/d/[id]` as before); their non-TTY output does not change (stories 154, 163)
- S18-08 — `POST /api/v1/pairing {member?, name?}` → 201 `{pair_url, server, profile, member, token, token_id, name, qr_svg}`: a Token (default name `device`, no expiry) through the ordinary mint path that works at once and is listed / revocable; `profile` = `handle@first-DNS-label` (`handle@ip` for an IP); `pair_url` = `annotary://pair?v=1&server=…&token=…&profile=…` percent-encoded; `qr_svg` an inline black-on-white SVG without external references; no public URL ⇒ 409 `no_public_url` with the `serve --url` hint and nothing minted; any Member for themselves, owner-only for another (403), unknown Member 404, 401 without a credential (story 165)
- S18-09 — `pair [--member H] [--name N]`: on a TTY a header (`Pairing a device for @h on SERVER`), the QR code in Unicode quadrant blocks (light modules drawn, so it scans on a dark terminal), `profile …`, the pair URL, and `Shown once. Revoke: annotary token revoke ID` on stderr; non-TTY one line `PAIR_URL MEMBER TOKEN_ID NAME`; `--json` `{pair_url, member, token_id, name}` (+ `server, profile, token`); exits 5 (`no_public_url`), 4 (another Member, not an owner), 2 (unknown Member) (story 165)
- S18-10 — the owner Members screen: every active Member's card has **Pair a device** (none on a disabled one); it POSTs `/api/v1/pairing {member, name: "device"}` and shows `.cd-paironce[data-handle]` with the inline QR, the "Scan with the iPhone or iPad camera, or Annotary…" sentence, Copy link (the pair URL) and Dismiss, the card's Tokens table re-drawn with the new `device` Token; `no_public_url` shows its message + hint in the card's `.cd-memerr` (story 165)
- S18-11 — `GET /api/v1/docs` rows and `GET /api/v1/docs/{id}` carry the display `title` (front matter `title:` else the first H1, null when none — the web list's value, cached per Markdown blob); CLI output is unchanged (story 166)
- S18-12 — the Document page renders a hidden `#cd-openapp` "Open in Annotary" chip, `href="annotary://open?server=<public URL or the request's origin>&doc=<id>"`; the script shows it only for an iPhone / iPad user agent (or a Mac one with touch points) and appends `&thread=tN` from `#tN`; other browsers never see it (no layout change) (story 167)
<!-- ---- end ticket 18 ---- -->

<!-- ---- CMD-4 ---- -->
## CMD-4 — Failed-login throttle and cross-site refusal

- S19-01 — wrong passwords at `POST /login` and `POST /api/v1/login` count together per client (the peer address; IPv6 by its /64, an IPv4-mapped address as IPv4): the 5th within 15 min locks that client out until the oldest of them is 15 min old — 429 (`too_many_attempts` JSON from the API, the form with "Too many wrong passwords" from the page) with `Retry-After`, the password not checked; other clients are unaffected, a success forgives the client's own failures, a Token keeps working (story 23)
- S19-02 — 20 wrong passwords within 15 min from non-loopback clients together lock every non-loopback client out the same way (the account limit); loopback is never counted against it nor locked by it (story 23)
- S19-03 — a request that is not GET / HEAD / OPTIONS and carries no bearer Token is refused 403 `cross_site` when `Sec-Fetch-Site` is present and not `same-origin` / `none`, or else when `Origin` is present and its `host[:port]` is not the request's `Host` (`null` included): the login form, Sign out and cookie API mutations alike; same-origin, bearer and header-less requests pass (an empty `Bearer` or one sent to `/login` buys no exemption); the session cookie stays `SameSite=Lax` (stories 24, 131)
<!-- ---- end CMD-4 ---- -->

<!-- ---- CMD-8 ---- -->
## CMD-8 — the SSE stream re-checks access

- S20-01 — an open SSE stream on `GET /api/v1/events` re-checks access whenever the commit counter moves, before it sends more: once an owner revokes a Member's Grant on a Vault, no further event of that Vault reaches the stream while events of Vaults still readable keep flowing; a Grant given back resumes them (stories 114, 115)
- S20-02 — an open SSE stream ends as soon as its Credential stops authenticating — the Token revoked, the browser session ended, or the Member disabled — instead of running until the connection drops (story 115; Access model)
- S20-03 — a parked long-poll (`wait=`) re-checks access when a commit wakes it, before it rescans: a Grant revoked mid-park drops that Vault's events from the answer while a Vault still readable answers; a Credential that dies mid-park answers 401 (stories 110, 114)
<!-- ---- end CMD-8 ---- -->

<!-- ---- CMD-9 ---- -->
## CMD-9 — export is complete or fails loudly

- S21-01 — `export_tar` reads the blob set its database snapshot references (`referenced_hashes`, the GC's definition) and, when a purge / GC / derived-blob cleanup between the snapshot and the blob copy removed one of them, fails with `ExportIncomplete {missing}` instead of writing an archive without it; a fresh export after the purge is complete (story 12)
- S21-02 — `export FILE.tar` retries an incomplete export on a fresh snapshot (up to 3 attempts, a stderr note per retry), then exits 5 `export incomplete: N blobs the database snapshot references are not in the store (after 3 attempts) — nothing was written …` with a `missing blob HASH` line each (at most 10); neither FILE nor its temp file is left (story 12)
<!-- ---- end CMD-9 ---- -->

<!-- ---- CMD-14 ---- -->
## CMD-14 — the login page tells a remote browser how to get in

- S22-01 — `GET /login` from a non-loopback peer while no password is set: 200, no cookie, and instead of a claim that it is signed in automatically, the steps to get in — open the server's loopback URL (`server.json`'s, e.g. `http://127.0.0.1:7377`) on its machine and set a password in Settings, or run `annotary pair` for the iPhone / iPad app; a server bound to one non-loopback address names that address and the `--bind 0.0.0.0` restart instead; a loopback peer is still signed in and sent on (stories 131, 162)
<!-- ---- end CMD-14 ---- -->

<!-- ---- CMD-17 ---- -->
## CMD-17 — a request a local reverse proxy forwards is not loopback

- S23-01 — a loopback peer whose request carries `X-Forwarded-For`, `Forwarded`, `X-Real-IP` or `Tailscale-User-Login` (Tailscale Serve in front of the server) is not loopback while no password is set: a web page redirects to `/login` with no cookie, and `GET /login` gives the no-password steps; a loopback request without them is still signed in (stories 24, 162)
- S23-02 — wrong passwords forwarded by a local proxy count per forwarded client (the last `X-Forwarded-For` entry; a missing, unparsable or loopback entry is `0.0.0.0`) and against the account limit, so one tailnet client cannot lock out another and the account limit holds behind the proxy (story 23)
<!-- ---- end CMD-17 ---- -->

<!-- ---- CMD-18 ---- -->
## CMD-18 — the CLI speaks https

- S24-01 — the CLI's client completes a request to an `https://` server URL over TLS (rustls), so a profile can point at the live server's https address (story 8)
- S24-02 — the CLI's own client checks certificates against the bundled public roots: an untrusted certificate fails with `TLS with URL failed: …`, exit 6, kind `tls`, never "transport is unsecured" (story 8)
<!-- ---- end CMD-18 ---- -->

<!-- ---- CMD-13 ---- -->
## CMD-13 — an open editor survives a live card refresh

- S25-01 — a live event that re-draws a non-tombstoned Thread card (another Member's reply over SSE) while a reply or edit editor is open on it, empty or not, keeps the editor attached and focused and marks the card `data-stale`; Cancel or posting then re-draws the card with the update, while a tombstoned (`hidden`) Thread is still removed at once (stories 115, 127; tracker CMD-13)
<!-- ---- end CMD-13 ---- -->

<!-- ---- CMD-5 ---- -->
## CMD-5 — Vault verbs, and profiles and Sidecars that follow a renamed Vault

- S26-01 — `vault list` (any Member): the Vaults the caller can read, oldest first, as `NAME DOCUMENTS DELETED CREATED` (DELETED = deleted Documents still in it, CREATED relative) — TTY aligned with a header, non-TTY tab-separated without one, `--json` `{name, documents, deleted, created, id}` (+ `default_actions`), `--json ?`; `GET /api/v1/vaults` rows carry `documents` and `deleted` (story 32)
- S26-02 — `vault create NAME` / `vault rename OLD NEW` print the Vault's `list` row (`rename --json` adds `from` and `profiles`), `vault delete NAME` prints `NAME deleted`; all three owner-only (exit 4); unknown Vault exit 2; a taken name, a Vault still holding Documents (deleted ones until purged) and the last Vault exit 5; an empty name exit 1 (story 32)
- S26-03 — `vault rename` points every profile in the local `config.toml` on the same server whose Vault it was (its `vault_id`, or its `vault` name when it has no id) at the new name and id, `vault delete` clears `vault` and `vault_id` from such profiles; a note names each profile; profiles on other servers or other Vaults are untouched (stories 27, 32)
- S26-04 — a git-conflicted Sidecar whose two sides have the same Document `id` and different `vault` names is one Document across a Vault rename, not two: it reads without a human (the newer side's `vault`, the lower side's `base_version` and base hash) and `push` / `pull` rewrite it clean; different ids are still exit 5 `keep one side` (stories 137, 158)
- S26-05 — `Store::resolve_profile_vault(name, id)`: an id that ever carried the name (events `vault.created` / `vault.renamed` / `vault.deleted`) follows that Vault through any chain of renames, even after a new Vault takes the old name, and is `Deleted` once it is gone; without an id, or with one that never carried the name (a hand-edited `vault`), the live Vault of that name, else the one live Vault that ever carried it, else `Deleted` when every Vault that carried it is gone, else not found (ambiguous included) (story 27)
- S26-06 — the CLI sends the profile's Vault as `X-Annotary-Profile-Vault` (+ `X-Annotary-Profile-Vault-Id` once known); `GET /api/v1/docs`, `…/docs/resolve`, `POST /api/v1/docs` and `GET /api/v1/mentions` resolve it through renames in the same request and answer `X-Annotary-Resolved-Vault` / `-Id` (`GET /api/v1/vaults` and `/me` only answer them); the CLI rewrites that profile's `vault` and `vault_id` — the first `list`, `push` of a new Document, `show NAME`, `mentions` or `profile list` after a rename made elsewhere works and heals it (a note when the name changed; a missing `vault_id` is backfilled silently), and a `vault` edited by hand to a name that `vault_id` never carried wins over the stale `vault_id` (delete `vault_id` to point at a new Vault reusing a formerly carried name) (story 27)
- S26-07 — a profile whose Vault was deleted: the response carries `X-Annotary-Deleted-Vault`, the CLI clears `vault` and `vault_id` from the profile with a note, `list` / `resolve` / `mentions` cover every readable Vault, and `push` of a new Document (404 `vault_deleted`) picks the Vault again as if the profile named none (stories 27, 43)
- S26-08 — Vault names travel percent-encoded in those headers (an HTTP header value is ASCII): a non-ASCII Vault name (`Übersicht`) is renamed, followed and healed end to end (story 27)
- S26-09 — rights stay as before: a profile Vault resolved to a Vault the caller lacks the right on is 403 naming the profile's name, with nothing echoed; an explicit `--vault OLD` nobody has is 404, its hint `Vault OLD was renamed to NEW` only for a caller who may read NEW (stories 30, 32)
<!-- ---- end CMD-5 ---- -->

<!-- ---- CMD-12 ---- -->
## CMD-12 — opening another composer never discards typed text

- S27-01 — asking for a composer (the pill, `c`, an inline Asset) while the open composer holds typed text keeps that composer and its text and shows its inline `.cd-discard` confirm instead: "keep" (or `esc`) removes the confirm and puts the cursor back, "discard" closes the first composer with its pending highlight and opens the new one, focused, on the new target (also when the confirm was already showing from `esc`); Cancel or posting the first composer opens nothing; an empty composer is still replaced at once (stories 119, 120, 126, 127; tracker CMD-12)
<!-- ---- end CMD-12 ---- -->

<!-- ---- CMD-20 ---- -->
## CMD-20 — a hidden resolved Thread's text is not clickable

- S28-01 — with "Show resolved" off, clicking text whose only highlight is a resolved Thread's selects nothing and leaves the toggle off; a resolved highlight inside an open Thread's text opens the open Thread; with the toggle on, it opens the resolved Thread as before (story 117; tracker CMD-20)
<!-- ---- end CMD-20 ---- -->

<!-- ---- CMD-23 ---- -->
## CMD-23 — Member settings and Devices

Notes: `docs/dev/member-settings.md`, `docs/dev/web-settings.md`.

- S29-01 — migration v5 adds `devices` (keyed by member and id), `tokens.device_id` / `sessions.device_id`, `member_settings`, `all_device_settings` and `device_settings` (a NULL value is a tombstone); a v4 store with Members, sessions, Documents and Threads migrates with everything intact, and every Member who had a session has the Web Device (tracker CMD-23)
- S29-02 — one catalog (`annotary_store::settings::CATALOG`): every setting has key, kind (user / device), label, help line, group and order, type and allowed values, default and the Device kinds it applies to; `GET /api/v1/me/settings/catalog` serves it with the groups (whose slugs are never `server` or `vaults`) and the highlight presets; an unknown key is 400 `unknown_setting`, a value outside the catalog 400 `invalid_setting` (tracker CMD-23)
- S29-03 — highlight colors: each preset's light and dark value is fixed in the catalog and keeps the highlighted text at 4.5:1 or more (dark text `#000` on the bright dark presets) and a set ΔE from the refined color of its appearance; a custom `#rrggbb` whose text falls below 4.5:1 is 400 `low_contrast` with the ratio (tracker CMD-23)
- S29-04 — resolution: a device setting resolves on a Device to its own value, else the All devices value, else the default, with `source` `device` / `all` / `default`; a new Device has no values of its own and gets every All devices value at once and the default for what was set only on other Devices; a tombstone follows All devices; user settings are one value per Member (`source` `user` / `default`) (tracker CMD-23)
- S29-05 — a write at scope `all` sets the All devices value and tombstones every non-forgotten Device's own value with one server stamp later than every stamp it replaces; at scope `devices` it sets those Devices' own values (`null` = a tombstone); the matrix shows each Device's own and resolved value and `mixed` when the Devices the setting applies to disagree (tracker CMD-23)
- S29-06 — stamps: the server stamps every write that carries no stamp and keeps a client's stamp; a stamp more than five minutes ahead of the server's clock is 409 `clock_skew` with the server's time; a stamp equal to or older than the stored one is 409 `stale` with the stored value and stamp (the same value is a no-op); the server's clock (`settings.hlc`) survives a restart (tracker CMD-23)
- S29-07 — a write that changes nothing appends no event; a real change appends `setting.changed {member, key, scope, device?, stamp}` and never the value (tracker CMD-23)
- S29-08 — Devices: an id is `web` or 26 characters of Crockford base32, anything else 400 `bad_device`; a settings route makes the request's Device on first use (named from `X-Annotary-Device-Info`), ordinary API calls never make one; at most 20 Devices that aren't forgotten (409 `too_many_devices`); `last_seen_at` is written at most once a minute and wakes no `wait`; every browser is the one Web Device, made at sign-in, never renamed or forgotten (409 `web_device`) (tracker CMD-23)
- S29-09 — Forget revokes the Tokens and ends the sessions last used from that Device and marks it forgotten; a request naming a forgotten id is 409 `device_forgotten`; its row and values stay, so Copy from another device can still read them (tracker CMD-23)
- S29-10 — Copy from another device copies every device setting of one Device onto another (its own values, and tombstones where it follows All devices) or onto All devices (what it resolves to becomes everyone's), as new changes stamped now (tracker CMD-23)
- S29-11 — `GET /api/v1/me/devices/{id}/sync` gives each device setting's own row and All devices row with their stamps (absent when there is none); an iOS push (`PUT` with `stamp`) of an own value or a tombstone is kept as stamped (tracker CMD-23)
- S29-12 — `POST /api/v1/me/rename {name}` lets every Member rename themselves with the owner route's rules (`PUT` of `display_name` does the same); the owner route stays owner-only (tracker CMD-23)
- S29-13 — live feed: `setting.changed` and `device.*` events reach their own Member only, on a feed that adds `&me=1` (alone or beside `doc=`); another Member and an owner never receive them; their payloads carry no values (tracker CMD-23)
- S29-14 — Default Vault is stored as a Vault id: it survives a Vault rename and falls back to Any when the Vault is deleted; an unknown id is refused (tracker CMD-23)
- S29-15 — every `/api/v1/me/settings*` and `/api/v1/me/devices*` route is open to every Member for themselves only: one Member can neither read nor change another's settings or Devices (tracker CMD-23)
- S29-20 — the Settings screen: `/settings` and `/settings/{group}` (one URL per catalog group) for every Member, with the sidebar of groups and a search field, the owner pages under the Server group (`/settings/server/members|vaults|general`, 403 page for a Member; `/members` 308s to `/settings/server/members`); at 850px and wider the sidebar is on the left, narrower the group list is its own page (story 32; tracker CMD-23)
- S29-21 — a group with device settings shows "Applying to" (All devices, or one or more Devices; a menu of `menuitemcheckbox` items usable from the keyboard) and "Copy from another device"; changes save at once; with All devices chosen and the Devices disagreeing the control shows Mixed (no value selected, "Mixed" to a screen reader) and the layers icon turns amber; the layers icon opens the chain (All devices, then each Device with its value, own or following, and Reset on an own value) (tracker CMD-23)
- S29-22 — Copy from another device asks first ("Copy NAME's settings to NAME?", Copy / Cancel, Escape closes it with nothing copied) (tracker CMD-23)
- S29-23 — the Profile page renames the Member themselves; the Devices page lists each Device's kind icon, name and last seen, with Rename and Forget for iOS Devices and neither for Web (tracker CMD-23)
- S29-24 — a dark theme for every page: Light or Dark puts that palette on `.cd-app` in the first response, Follow system carries both (the dark one under `prefers-color-scheme: dark`); the highlight colors and the text size are CSS variables on `.cd-app` for each palette, in the first response; Show resolved and absolute times (once the `cd_tz` cookie is known) are in the first paint; two different browsers resolve the same Web values (tracker CMD-23)
- S29-25 — a change made in one browser context shows in another without a reload (`&me=1` on the live feed) (tracker CMD-23)
- S29-26 — the Settings search filters settings by name and help line across groups with no request (tracker CMD-23)
- S29-30 — `annotary settings [list|get|set|reset] KEY [VALUE] [--all-devices|--device NAME]`: list and get show each setting's value and source (device settings for All devices unless `--device` names one), set and reset write it (a device setting needs `--all-devices` or `--device`), TTY and non-TTY, `--json`, unknown key / bad value exit 1, unknown Device exit 2 (tracker CMD-23)
- S29-31 — `annotary devices [list|rename|forget]`: the caller's Devices (`ID KIND NAME LAST SEEN`), rename and forget by name or id; Web can't be renamed or forgotten (exit 5); TTY and non-TTY, `--json` (tracker CMD-23)
- S29-40 — `deploy/install-linux.sh` writes `snapshots/pre-schema-N.tar` (an `export` of the old store) before it starts a binary with a newer schema, and its rollback restores that export when the new binary does not come up (tracker CMD-23)
<!-- ---- end CMD-23 ---- -->

<!-- ---- CMD-22 ---- -->
## CMD-22 — `password` changes a local password in a stopped data dir

- S30-01 — `password --clear` on a stopped data dir removes every active Member's local password (`--member HANDLE`: that one only), one `MEMBER PASSWORD` record per Member changed (`@handle removed`; TTY aligned with a header, non-TTY tab-separated, `--json` `{member, password, id, store}`); once no password is left a note says a browser on the machine is trusted as the owner; nothing to remove is a note (naming the Member with `--member`) and exit 0 (stories 23, 24; tracker CMD-22)
- S30-02 — `password --set [--member HANDLE]` (default the owner) reads the new password from the prompt — no echo on a TTY, a stdin line otherwise, `ANNOTARY_PASSWORD` answers it — never an argument, and never prints it; an empty answer is exit 1 and changes nothing (story 23; tracker CMD-22)
- S30-03 — `password` is refused (exit 5) while a server holds the store's `LOCK`, changing nothing; no store in the data dir is exit 5 and creates none; an unknown Member is exit 2 and a disabled one exit 5 (`--clear` without `--member` skips disabled Members); an unknown `--json` field is exit 1 before anything changes; neither or both of `--clear` / `--set` is exit 1 (stories 12, 23; tracker CMD-22)
- S30-04 — `deploy/verify_live.py` (`make verify-live`) extracts a snapshot into a fresh private folder, clears the copy's passwords (never the snapshot's), serves it with the build under test on a free loopback port that is not 7377 and does not answer on the host's outward address, mints a Token in the copy for a private CLI home whose only profile `copy` points at it, and prints one JSON verdict (`status`, `checks`, `documents`, `deleted`, `production`: the unit not restarted, the copied snapshot's sha256 unchanged, and on live runs the live server refusing the copy's Token); the folder is gone afterwards (story 12; tracker CMD-22)
- S30-05 — a failed run still deletes its copy (`deleted: true`, exit 1); a run first sweeps `cd-verify-live-*` folders of this user whose run is certainly over (a readable pid that is gone, or no pid and over an hour old; never a live run's), stopping the server and Playwright groups a hard-killed run left (only groups whose leader names that copy; a group that will not die keeps its folder) before deleting it (story 12; tracker CMD-22)
- S30-06 — `verify/playwright.config.ts` refuses to run unless `VERIFY_URL` is `127.0.0.1` with a port that is not 7377, `VERIFY_TMP` is a `cd-verify-live-*` folder and `VERIFY_CLI_HOME` lies inside it, so the scenarios (which change data) can never reach the live server (story 12; tracker CMD-22)
<!-- ---- end CMD-22 ---- -->

<!-- ---- M3b ---- -->
## M3b — the Annotary brand in the web app

- S31-01 — `scripts/brand/render.py --check` passes on the committed `brand/`, and fails naming the file when a rendered file differs from a fresh render (a changed pixel in a PNG or an `.ico` layer, a changed byte in a generated SVG), is missing, or when `brand/` holds a file the script does not know; the check writes nothing
- S31-02 — what the script renders: `icon.svg` and `favicon.ico` (a 32 and a 16 layer) from the small master; `icon-192.png` and `icon-512.png` from the full master with transparent rounded corners; `apple-touch-icon.png` (180) and `icon-mask.png` (512) opaque full squares, the maskable mark inside the 40% safe circle; `logo.svg` and `logo-dark.svg` carry the mark and the wordmark; the masters use only `DESIGN.md`'s ink, page and pink
- S31-03 — `/favicon.ico`, `/icon.svg`, `/apple-touch-icon.png`, `/manifest.webmanifest` and the manifest's `/icon-192.png`, `/icon-512.png`, `/icon-mask.png` answer 200 before sign-in (no credential, a password set, a request a proxy forwarded, while `/` redirects to `/login`), with the committed bytes, their content type, `nosniff`, a cache lifetime and no cookie; `icon.svg` carries the no-scripts CSP; the manifest names Annotary and each icon it lists answers (story 131)
- S31-04 — every page's head links the icons and the manifest: the list, a Document, Settings, the login page and an error page (story 131)
- S31-05 — the 18 px icon is the first thing in `.cd-hdr` and links to the Document list: followed by the wordmark on the list and the Settings pages, by the Vault crumb and no wordmark on a Document page, at 18 × 18 CSS px in light and dark and at a narrow width, without changing what the narrow header's overlay does (stories 117, 118)
<!-- ---- end M3b ---- -->
<!-- ---- M5 ---- -->
## M5 — a made-up test corpus, and no personal details in the test data

- S32-01 — the three corpus Documents with a Version history (`fixtures/corpus/`: v1 to v4, a paragraph rewritten, a section moved, a section deleted, typo fixes, in Latin, CJK and right-to-left text) replay through `annotary_diff::reanchor` and reproduce their declared `expected.json`, including every status, and each span's placement does not depend on the others (story 91)
- S32-02 — the same histories pushed through the server give the same placements whether the edge computes them or the server does, and match `expected.json` (story 91)
- S32-03 — the corpus pushes through the real CLI: every Document, with its Assets, a missing one warned about, Threads on it, a Version replayed, and a pull that pushes back as `already` (stories 91, 146)
- S32-04 — the corpus renders in the web viewer: headings, long tables, code, footnotes, relative links, media, CJK and right-to-left text, MathML and mermaid fences, raw HTML with its scripts inert, and no page wider than a phone or a desktop window (story 146)
- S32-05 — a Thread made by selecting text on a corpus Document lands on the source range the CLI reports, and its re-anchored state shows on the next Version (story 91)
- S32-06 — selecting right-to-left, CJK and combining-mark text, by a DOM range and by double-clicking a word, makes a Thread whose quote and code-point range match the source (story 146)
- S32-07 — a guard fails when test data, code or the docs hold a tailnet hostname other than `example.ts.net`, an email address outside an `example` domain, or a home path other than `/home/linuxbrew`, or an archive or a database it cannot read; a hit planted in a file fails it; nothing under `tests`, `fixtures` or `crates` is exempt
- S32-08 — what is checked before publishing passes on the committed tree; an email address is allowed only in the files the guard lists by path (the Code of Conduct's contact and the template's record of it)
<!-- ---- end M5 ---- -->
<!-- ---- M4 ---- -->
## M4 — a server installs from a release, and the repo names no host

- S33-01 — `deploy/install-linux.sh vX.Y.Z` downloads the release tarball and its `.sha256`, and installs only when the SHA-256 matches and the binary reports X.Y.Z: a tarball that differs, a missing or empty checksum, a missing tarball or another version each refuse with nothing on the host changed; it keeps `annotary.prev`, and `--binary FILE --src DIR` stays for a build on the host
- S33-02 — the installer adds `--url` and `--link-base` to the root-only `/etc/annotary/env` only where the file lacks them: an empty file is filled in, a value already there is kept and a different one reported, other lines are kept, the mode ends 600; with no URL given or recorded it refuses before changing anything, `--url none` is a choice, and a value that is not `none` or scheme://host[:port] is refused
- S33-03 — the installer's start check needs the env file's URL in the health answer (or it rolls back to `annotary.prev`); a Tailscale Service is served only with `--tailscale-service`; first-run owner settings come from flags, with no name built in
- S33-04 — the unit takes `--url` and `--link-base` from the env file, required: with no file, or no key, it does not start
- S33-05 — the health check reads the env file, because the store remembers the last URL: no file, no `ANNOTARY_URL`, a server answering with another URL or none, or deploy settings that disagree with the file are each critical, which fails the deploy gate; `ANNOTARY_URL=none` is no URL on purpose
- S33-06 — `make deploy` and `make server-health` take the host and what they install from settings (`~/.config/annotary/deploy.env`, the environment wins, a key it does not know is refused); `DEPLOY_SOURCE=release` installs HEAD's published tag and refuses with no tag, `DEPLOY_SOURCE=build` builds on the host; both pass the URL and link base to the installer
- S33-07 — with a setting missing, `make deploy`, `make server-health` and `make verify-live` refuse before doing anything, naming the keys and the file
- S33-08 — with `SERVER_SSH` set, `make verify-live` copies the host's newest snapshot over ssh, deletes the local tar, checks the unit and the snapshot there, and has the host's server refuse the copy's Token, which goes over stdin
- S33-09 — an empty `ANNOTARY_URL` (or link base) in the env file counts as missing, because the unit would pass an empty `--url`, which serve takes as "clear": the installer refuses it with no `--url` and replaces it with one, its start check fails on it, and the health check is critical for it; `none` and a keyless file given `--url` are unaffected
- S33-10 — filling in a key the env file lacks, on a host with a store, refuses with nothing changed unless the value matches what the running server's health answer says the store holds, `none` matching no value; no answer also refuses; `--accept-new-url` or editing the file is the deliberate change, and a first install has nothing to compare with
- S33-11 — the new binary runs its `--version` and `--print-schema-version` as `nobody` from a world-readable copy, never as root, on both the release and the `--binary` path, and a rollback asks the previous binary as `annotary`
- S33-12 — a deploy keeps the installed health check as `healthcheck.sh.prev` and a rollback restores it with the unit; a first install has no previous copy to keep or restore
<!-- ---- end M4 ---- -->
<!-- ---- M3 ---- -->
## M3 — releases and the update notice

- S34-01 — with a newer release, `-V` on a terminal prints the notice on stderr, naming both versions and the upgrade doc, and stdout stays `annotary X.Y.Z`; the same or an older release prints nothing
- S34-02 — the answer is cached in the data dir: a second `-V` that day asks nothing and still prints the notice; after a day, or with the clock gone backwards, it asks again
- S34-03 — off with `update_check = false` in `config.toml` or `ANNOTARY_UPDATE_CHECK=off`: no request, no cache file, no notice even from an old cache; `update_check` survives the CLI rewriting `config.toml`
- S34-04 — it fails silently: a 404, a 500, an answer that is not a release, or no server at all each exit 0 with nothing on stderr, and the attempt counts for the day
- S34-05 — it stays off the hot path: `-V` with no terminal never asks (it prints what the cache knows), other verbs neither ask nor print, and a build with no release version never asks
- S34-06 — `serve` logs the cached notice at start and asks on a thread of its own: a release endpoint that never answers does not hold up the server, and what it learns is logged
- S34-07 — the release pipeline's own code: the stamp writes the tag's version into the workspace manifest and lockfile for the workspace's crates only, and refuses what is not a plain version or a manifest with no placeholder; the tap's formula gets a `test do` that runs `--version` and `serve --setup-only` in a temp data dir, once, and a formula it cannot place it in is refused; in the published workflows every action is pinned to a full SHA, only the job that makes the release can write, the dist installer is checked before it runs, and the tap's deploy key is passed by name to a push job that has no third-party code and runs only after the formula has passed
<!-- ---- end M3 ---- -->

## Stories

Every numbered story of the spec (`docs/phase1/spec.md`, "User Stories" 1–167)
and the ids whose tests cover it — the checker demands every number and only known ids; a story
whose coverage Phase 1 does not automate points at a Deferred id.

- 1 — `serve` in the foreground, no cloud → S01-20
- 2 — `daemon install` writes + starts the LaunchAgent → S13-01
- 3 — `daemon uninstall|status|start|stop|restart` → S13-02, S13-04
- 4 — first-run setup inline (owner, password, Vault `notes`) → S01-10, S13-05
- 5 — setup ends by saying how the CLI gets a credential → S01-10, S13-05
- 6 — `daemon` unsupported off macOS, `serve` works → S13-03
- 7 — loopback-only bind (unless `--allow-remote`, ticket 18) → S01-01, S18-01
- 8 — fixed port 7377 / `--port`; the CLI calls the profile's URL → S01-20, S24-01, S24-02
- 9 — second `serve` refused by the LOCK → S01-02
- 10 — `server.json` on start, removed on exit, stale pid = not running → S01-03, S08-11, S13-02
- 11 — exit 6 with the unreachable message, never a hang → S01-16
- 12 — one data dir; `export FILE.tar` → S01-19, S13-06, S21-01, S21-02, S30-03, S30-04, S30-05, S30-06
- 13 — schema version + forward-only migrations → S01-04
- 14 — hourly in-process ticker (auto-purge, GC, derivative cleanup) → S13-09, S13-10, S13-11
- 15 — `member add` one step, `ANNOTARY_TOKEN=` once → S01-11, S12-02
- 16 — web Add agent (Token once, env line) → S12-02
- 17 — `member list|rename|disable|grant|role` → S01-12, S06-02, S06-06
- 18 — disable, never delete → S06-05
- 19 — handles unique forever, kebab-case → S01-13, S04-06, S06-05
- 20 — Token belongs to the Member → S06-07
- 21 — `token mint|list|revoke`, `last_used_at`, name/expiry → S01-08, S06-07, S06-08, S12-02
- 22 — Tokens shown once, hashed, no expiry by default → S01-09, S06-07
- 23 — optional local password (argon2id) → S06-09, S12-03, S19-01, S19-02, S23-02, S30-01, S30-02, S30-03
- 24 — loopback browser = owner with an auto cookie → S06-10, S10-02, S19-02, S23-01, S30-01
- 25 — CLI always needs a credential; `login` refuses without a password → S06-11, S06-12
- 26 — `login` saves a profile (no default profile since 2026-10-21) → S06-12, S06-13
- 27 — profiles in config.toml, `-p` / env overrides → S01-14, S06-13, S26-03, S26-05, S26-06, S26-07, S26-08
- 28 — exit 3 message (env var, `--profile`, `login`, profiles) → S01-15
- 29 — exit 4 names the Right + `member grant` hint → S01-07, S06-01, S06-14
- 30 — Rights = read/comment/action/write per Vault → S02-31, S04-17, S05-13, S06-01, S26-09
- 31 — Grant `{vault | *, rights[]}`, `*` covers later Vaults → S06-01, S06-02
- 32 — owner-only operations, never grantable → S02-31, S06-03, S12-04, S12-05, S29-20, S13-07, S26-01, S26-02, S26-03, S26-09
- 33 — last owner un-demotable → S06-04
- 34 — Members screen (auth hint, grants behind Advanced, `last_used_at`) → S12-01
- 35 — `push` without a Sidecar creates v1 → S02-01
- 36 — `push` with a Sidecar binds by id → next Version → S02-02
- 37 — `push --doc ID` binds an unbound file → S02-03
- 38 — `push --force` rebinds another id's Sidecar → S02-03
- 39 — Name = filename stem per Version, head's Name shown → S02-04
- 40 — Name rules (verbatim stem, rejections, `.md` required) → S02-05
- 41 — duplicate Names allowed, id is the handle → S02-06
- 42 — DOC = id | unique Name | path; ambiguous ⇒ exit 2 listing → S02-07, S04-18
- 43 — Vault selection order, typed error when > 1 → S02-08, S26-07
- 44 — identical to head ⇒ `already vN` → S02-09
- 45 — identical to older ⇒ new Version `same content as vK` → S02-10
- 46 — asset-only change ⇒ new Version → S03-09
- 47 — Version record, immutable, never deleted → S02-11
- 48 — asset scan (syntaxes, supported types) → S03-01, S03-02
- 49 — external/fragment/mailto/data ignored; absolute/`file://` warned → S03-01
- 50 — linked `.md` = recursive Asset, cycles, depth 5 → S03-03
- 51 — missing Asset warns, `missing: true`, `--strict` fails → S03-04, S03-05
- 52 — unsupported types warned, left as written → S03-02
- 53 — caps 2 MB / 100 MB / 500 MB → S03-10
- 54 — two-phase upload, Tenant-wide de-dup → S02-32, S03-11
- 55 — dimensions in the manifest → S03-07
- 56 — web-safe derivatives (HEIC/HEIF/AVIF/TIFF/BMP) → S03-08
- 57 — head moved ⇒ refused unless the file contains the head's changes (ticket 17) → S02-12, S16-03, S17-02, S17-03
- 58 — `--strict` turns every warning into exit 5 → S03-05, S16-03
- 59 — progress/warnings on stderr, one result line on stdout → S02-01, S02-13, S03-15
- 60 — progress on large pushes (the stderr lines; pushes stay sub-second on the 5k-line fixture) → S02-13, S03-15, S07-06, S15-01
- 61 — `pull` writes `<name>.md` + `<name>.annotary` → S02-14
- 62 — `pull --version N` writes N's Name, `base_version: N` → S02-15
- 63 — `pull` refuses over local edits unless `--force` → S02-16
- 64 — `--assets` writes Assets, skips same hash, refuses differing → S03-13
- 65 — `pull` stdout line + stderr summary → S02-14, S03-13
- 66 — `delete` soft, `restore` → S02-17
- 67 — `purge` (owner) + GC; auto-purge setting off by default → S02-18, S05-13, S12-03, S13-07, S13-08, S13-10
- 68 — `versions DOC` → S02-19
- 69 — no diff view in Phase 1 → S10-06
- 70 — `list [NAME]` columns, activity sort, limit 50 → S02-20, S05-10, S10-08
- 71 — `list` filters → S02-21, S04-10, S05-10, S10-08
- 72 — web list = the CLI list (title display-only) → S09-05, S10-08
- 73 — no unread markers / folders / tags / archive → S10-08
- 74 — `thread new` span | `--document` | `--asset` → S04-02, S07-09
- 75 — `thread new --version N` guard + quote verification → S04-04
- 76 — new Threads on head only; replies/resolve from any Version → S04-02, S11-03
- 77 — Comment bodies rendered minus embeds → S10-09
- 78 — `thread reply -m MSG|-` with `--resolve` → S04-03, S11-03
- 79 — `thread resolve | reopen` bare events by `comment` → S04-03, S11-03
- 80 — edit own Comment (owner any), `edited_at` → S04-07, S04-17, S11-03
- 81 — delete = tombstone; fully tombstoned hidden → S04-08, S11-03
- 82 — `@handle` mentions resolved at post time → S04-06
- 83 — `mentions [--since T]` → S04-10
- 84 — Thread ids `t7`, `t7.1` → S04-01
- 85 — `thread list` default open, document order, orphaned last → S04-11, S07-13
- 86 — `thread list` filters incl. `--version N` → S04-11, S07-12, S07-13
- 87 — `thread show` full range, quote, `now:` → S04-12, S07-14
- 88 — `show DOC` = header + open Threads → S02-28, S04-13, S05-10
- 89 — 100k body cap per Document, exit 5 naming it → S04-09, S11-03
- 90 — original placement frozen = the label → S04-02
- 91 — deterministic re-anchoring on every push → S07-01, S07-06, S07-16, S32-01, S32-02, S32-03, S32-05
- 92 — exactly three placement statuses → S07-02
- 93 — inside edits refine, not orphan → S07-03
- 94 — whitespace-only changes stay anchored → S07-03
- 95 — type-over / swallow / moved block orphan; orphaned sticky → S07-03, S07-05
- 96 — orphaned stays open, replyable, resolvable → S07-10
- 97 — Document-level Threads never orphan → S07-10
- 98 — Asset Threads by content hash → S07-09
- 99 — source coordinates `L:C-L:C` → S04-05, S11-08
- 100 — sparse per-Version placement history → S07-08
- 101 — the empirical fixture table is binding → S07-04
- 102 — Vault default Actions editable → S05-01, S12-03
- 103 — per-Document `actions.available` replaces the Vault set → S05-06, S05-15
- 104 — `act DOC ACTION [--version N]` on head only → S05-02, S05-03, S11-04
- 105 — Actions append-only, last on a Version wins → S05-04
- 106 — a new Version starts with no Action → S05-05
- 107 — `actions DOC` with snapshotted label/intent → S05-06, S05-08
- 108 — `status DOC` ≈ 24 tokens incl. `seq` → S02-28, S04-13, S05-09, S08-07
- 109 — no orphaned count by default → S05-11
- 110 — `wait` blocks, prints `SEQ KIND DOC DETAIL`, exit 0 → S08-03, S08-05, S08-06
- 111 — `--on thread,reply,resolve` opt-in → S08-05
- 112 — `wait` timeout exit 7 `(seq N)` → S08-08
- 113 — durable Tenant-wide cursor → S08-01, S08-07
- 114 — `wait` without DOC covers every readable Document → S08-09
- 115 — SSE on the events endpoint → S08-10, S11-07, S25-01
- 116 — in-app what's new → S08-10, S11-07
- 117 — Document view: margin cards ≥ 850 px, inline below → S10-01, S10-03, S10-11, S28-01, S31-05
- 118 — narrow header line + overlay, no bottom bar → S10-04, S31-05
- 119 — select → pill → composer (head only) → S11-01, S27-01
- 120 — media → pill → whole-Asset Thread → S11-02, S27-01
- 121 — Asset links are plain text, open in a new tab → S09-06, S10-07, S11-02
- 122 — refined: frozen quote + `now:` + dotted underline → S10-05
- 123 — orphaned behind a disclosure, still replyable → S10-05
- 124 — Version `<select>` + older-Version banner → S10-06
- 125 — Action buttons head-only, chips per Version → S10-06, S11-04
- 126 — keyboard map and nothing more → S11-01, S11-06, S11-10, S27-01
- 127 — optimistic mutations < 50 ms with rollback → S11-05, S11-10, S15-02, S25-01, S27-01
- 128 — speed budgets incl. the ≥ 5,000-line fixture → S10-01, S15-01, S15-02, S15-06
- 129 — inline media, derivatives, missing placeholder, same-content note → S03-08, S09-06, S10-07
- 130 — Members by name + auth glyph, never a bot badge → S10-05, S12-01
- 131 — login page only when a password is set → S06-10, S10-02, S12-06, S19-03, S22-01, S31-03, S31-04
- 132 — Deleted filter, no unread state → S10-08
- 133 — timestamps localized (web, TTY), ISO in `--json` → S02-29, S10-07
- 134 — Sidecar JSON, leaf-inline, fixed key order → S02-22, S16-02
- 135 — Sidecar = the full projection → S02-22, S03-14, S04-14, S05-12, S07-11, S16-02
- 136 — handles + `members` table in the Sidecar → S02-23, S04-14
- 137 — push reads only id/vault/base_version/actions.available → S02-24, S04-14, S05-07, S26-04
- 138 — `inherited: true` marks copied-in defaults → S05-07
- 139 — sparse history rule shared by CLI and server → S07-08
- 140 — YAML/JSONL/minified rejected → S02-25
- 141 — W3C Web Annotation export = a backlog mapping → S02-33
- 142 — edge render → viewer-agnostic HTML blob + renderer version → S09-02, S09-03, S09-04, S09-06, S09-07, S09-08
- 143 — server fallback renderer + `rerender` → S09-09, S09-10, S09-11
- 144 — CLI submits `placements[]`, server spot-verifies → S07-06, S07-07, S07-16, S09-08
- 145 — renderer/diff/sidecar crates build to WASM → S07-15, S09-12
- 146 — one Markdown superset, no flavour detection → S09-01, S09-05, S32-03, S32-04, S32-06
- 147 — SKILL.md ≤ 60 lines / ≈ 900 tokens → S14-01, S14-04, S14-07
- 148 — `skill install [--dir]` + keep-current hook → S14-02
- 149 — `--help` is the reference → S14-01, S14-03
- 150 — TTY / non-TTY parity → S01-12, S02-30, S04-16, S05-14, S07-17, S08-06, S13-02, S14-04, S14-05, S14-06, S16-04
- 151 — `--json` bare / `a,b` / `?` → S01-17, S04-15, S05-14, S08-06, S14-04, S16-04
- 152 — anchor column stamped with its Version → S04-12, S14-04
- 153 — exit codes 0–7, one-line errors / JSON errors → S01-18, S04-18, S14-04, S16-04
- 154 — `show`/`push` on a TTY append the web URL → S02-28, S14-05
- 155 — a merged file pushes over a moved head without extra steps → S17-01, S17-02, S17-09, S17-10
- 156 — a file lacking the head's changes is refused with what is missing → S17-01, S17-03, S17-09
- 157 — the refusal's unified diff on stderr and in `--json` → S17-03, S17-04
- 158 — a git-conflicted Sidecar needs no resolving → S17-07, S17-10, S26-04
- 159 — `--discard-head-changes` is the explicit overwrite → S17-08
- 160 — a stale, unedited file never reverts the head → S17-05
- 161 — an Asset the head changed counts as a head change → S17-06
- 162 — `serve` / `daemon install` reachable from a phone with an explicit `--allow-remote`; loopback trust stays for real loopback peers → S18-01, S18-02, S18-03, S22-01, S23-01
- 163 — the server knows the URL other devices use; links printed for people go through it (and the link relay) → S18-04, S18-05, S18-06, S18-07
- 164 — `link DOC [tN]` + the skill line: agents hand people a link that opens on their phone → S18-06
- 165 — pairing a phone: `pair` (QR on the terminal), `POST /api/v1/pairing`, **Pair a device** on the Members screen → S18-08, S18-09, S18-10
- 166 — the list / Document API carries the display title → S18-11
- 167 — an iPhone / iPad browser offers "Open in Annotary" → S18-12

## Deferred

Not automated in Phase 1 (spec "Testing Decisions: Deferred"). These ids need no marker; a
gated test may still name one. Each names its owning ticket and the reason.

- S13-12 — `launchctl` itself (ticket 13): the live `daemon install → status → restart → stop → start → uninstall` round trip needs a logged-in macOS GUI session and touches launchd, so it runs only by hand — `make launchd-test` (`ANNOTARY_LAUNCHD_TESTS=1`, temp HOME + data dir, private label); the plist text is unit-tested (S13-01) and every other `daemon` path is in `cargo test` / e2e (S13-02…05)
- S03-17 — HEIC/HEIF/AVIF decode on Linux CI (ticket 03): the derivatives need macOS `sips` or ImageMagick; the tests skip without a converter (`media::converter()`), so they run on a Mac only — TIFF/BMP derivatives are in-process and covered everywhere (S03-08)
- S03-18 — video dimensions beyond the fixtures (ticket 03): one mp4 (+ a rotated mp4, one mov, one WebM header) pin the `tkhd` / EBML readers; other encoders, fragmented files and multi-track containers are untested
- S09-14 — parser fuzzing (tickets 09 / 02): the Markdown renderer and the Sidecar reader are not fuzzed (backlog); the renderer corpus + `insta` snapshots and the Sidecar proptest are the coverage
- S15-08 — load / soak (ticket 15): the perf suite is single-user, cold-cache, median of 5; sustained load and long-running soak are backlog
- S13-13 — systemd unit / Windows service (ticket 13): out of scope — `daemon` exits 1 off macOS (S13-03) and `serve` runs under any service manager
- S02-33 — W3C Web Annotation export (ticket 02, story 141): a backlog mapping tool, not native storage — the native shape it would map from is S02-22
