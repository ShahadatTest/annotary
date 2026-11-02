# Spec: Annotary Phase 1 — local server + CLI + web viewer/commenter

Label: ready-for-agent
Status: ready (2026-08-18)
Source: [CONTEXT.md](../CONTEXT.md) (glossary — use its terms), [ADR 0001 compute at the edge](../../docs/adr/0001-compute-at-the-edge.md)
Prototypes: this spec was written after prototypes of the Sidecar, the CLI and the web UI; what they measured is recorded where it is used.
Fixtures referenced: [fixtures/anchoring/TABLE.md](../../fixtures/anchoring/TABLE.md) (Docs D1–D12, GitHub G0–G14)

Everything in this spec was decided during charting and in the fifteen resolved tickets. It records Phase 1 as built; later changes are in `docs/decisions.md` and `docs/dev/`.

---

## Problem Statement

A developer works on Markdown documents — plans, specs, notes — with several coding agents (Claude Code, Codex, Antigravity, Hermes) and moves work between them. Feedback on those documents is currently lost: agent harnesses (e.g. Antigravity artifacts + `agy` comments) let you drop a comment on a line but there are no replies, no resolution tracking, comments don't survive edits, and the next agent session cannot see what the last one was told. Google Docs and GitHub PR review get comments-under-edits right but are for humans and for code respectively, and neither is something an agent can read in a few dozen tokens.

The developer wants one place where humans and agents, with equal standing, can share a Markdown Document, comment on spans of it, reply, resolve, take a decision (Approve / Send back), upload a new Version, have every comment follow the text across Versions (or be honestly marked orphaned), and let an agent block cheaply until something needs it — all extremely fast, extremely token-efficient, with the data as plain portable files, and running entirely on the developer's own machine in Phase 1.

## Solution

`annotary`: a single Rust binary that is both the CLI and a local web server.

- The developer runs `annotary serve` (or installs it as a macOS LaunchAgent with `annotary daemon install`). First run creates the local Tenant, the owner Member and a Vault. The server binds to loopback on port 7377 and stores everything under `~/.annotary/`.
- The developer opens `http://127.0.0.1:7377` in a browser and is the owner (no login unless they set a local password). They add agents with one command (`annotary member add claude-code`) which prints a Token line to paste into the agent's environment.
- Anyone — human via CLI or web, agent via CLI — pushes a `.md` file, which becomes a Document with Version 1. Pull writes the `.md` back byte-identical plus a `<name>.annotary` JSON Sidecar (the server's full projection). Editing happens outside; pushing again creates Version 2 and re-anchors every Thread.
- The web UI renders the Markdown (GFM superset + Mermaid + math) with Thread cards in the margin (wide) or inline (narrow); select text → comment; reply, resolve, take Actions on head; switch to older Versions to see Threads as they were.
- Agents work through the CLI and a ≤ 60-line skill file: `show` (≈245 tokens), `status` (≈24 tokens), `thread reply --resolve`, `act --version N`, `wait` (blocking long-poll on new Version / Action / @mention). Agents never read the Sidecar.
- All per-Version computation (hash, asset scan, dimensions, image derivatives, diff + re-anchoring, Markdown → HTML) happens in the pushing client; the server stores, authorizes, serializes and serves — so views are a SQLite read plus static content-addressed blobs and hit hard speed budgets (FCP < 100 ms, CLS 0).

## User Stories

### Running the local server

1. As a developer, I want to run `annotary serve` in the foreground on my Mac, so that I have a local Annotary server without any cloud dependency.
2. As a developer, I want `annotary daemon install` to write a user LaunchAgent (RunAtLoad, KeepAlive, logs to `~/.annotary/logs/`) and start it, so that the server is always up after login without me thinking about it.
3. As a developer, I want `daemon uninstall|status|start|stop|restart`, so that I can manage the LaunchAgent without hand-editing plists or remembering `launchctl` syntax.
4. As a developer, I want the first `serve` (or `daemon install`) on an empty store to walk me through first-run setup inline — owner handle and display name (defaulting from `$USER`), an optional local password, and a first Vault named `notes` — so that the server is usable immediately.
5. As a developer, I want first-run setup to end by telling me the CLI still needs a credential (and how to get one), so that I am not surprised by "no credential" on my first CLI call.
6. As a developer on Linux, I want `serve` to work and `daemon` to fail with a clear "unsupported here — run `serve` under your service manager", so that I know exactly what to do.
7. As a developer, I want the server to bind to 127.0.0.1 only and refuse any other bind address unless I opt in with `--allow-remote` (ticket 18), so that nothing on my LAN can reach it by accident.
8. As a developer, I want the server to use a fixed default port (7377, `--port` to override) and the CLI to call the profile's server URL rather than discover it, so that behaviour is predictable and pointing the CLI at a cloud server later changes nothing.
9. As a developer, I want a second `serve` on the same store to be refused via a lock file, so that two processes never write the same data.
10. As a developer, I want the server to write `server.json` (url, pid, started_at, data_dir) on start and remove it on clean exit, so that `daemon status` and I can see what is running; a stale pid must read as "not running".
11. As an agent, I want the CLI to fail fast with a typed exit code (6) and the message "server at URL not reachable — run `annotary serve` or `annotary daemon install`" when the server is down, so that I never hang and never auto-start something behind the user's back.
12. As a developer, I want everything under one data dir (`~/.annotary/`, overridable via `ANNOTARY_HOME` / `--data-dir`) containing `config.toml`, `server.json`, `logs/`, `store/`, so that backup is "copy `store/`" and `annotary export FILE.tar` gives me a tarball.
13. As a developer, I want the store's schema to carry its own version and forward-only migrations to run on `serve` start, so that upgrading the binary never loses data.
14. As a developer, I want an in-process hourly ticker for auto-purge, blob GC and derivative cleanup, so that no external cron is needed.

### Members, credentials, rights

15. As an owner, I want `annotary member add HANDLE [--name]` to create a Member + Membership + `*` grant with all four Rights + a Token in one step and print the Token once as `ANNOTARY_TOKEN=…`, so that adding an agent takes one command and one paste.
16. As an owner, I want the same one-click "Add agent" in the web UI (mint, show Token once, show the env-var line), so that I can do it without the CLI.
17. As an owner, I want `member list|rename|disable|grant HANDLE VAULT|* RIGHTS|role HANDLE owner|member`, so that finer control exists when I ask for it, and stays out of the way otherwise.
18. As an owner, I want Members to be disabled, never deleted, so that attribution and history stay intact.
19. As an owner, I want Handles to be unique per Tenant forever (no reuse after disable), kebab-case, so that `@mentions` are unambiguous and history is rename-safe.
20. As a Member, I want my Token to belong to me (the Member), not to a Tenant or a Vault, so that one Token gets me into every Tenant I belong to with each Tenant's Grants applying.
21. As an owner, I want `token mint|list|revoke` with `last_used_at` and optional name/expiry, so that I can rotate and audit agent credentials.
22. As an owner, I want Tokens shown once, stored hashed, with no expiry by default, so that they are simple to operate and safe at rest.
23. As an owner, I want to set or change an optional local password (stored argon2id), so that a browser on my machine must log in if I choose.
24. As a developer with no password set, I want a browser hit on loopback to be treated as the owner with an auto-issued session cookie, so that bookmarks just work.
25. As a developer, I want the CLI to always require a credential — a Token via profile/env or `annotary login` (which asks for the local password and refuses if none is set) — so that an agent never accidentally acts as me.
26. As a developer, I want `login` to save a profile, so that I can use it with `-p`. (Changed 2026-10-21: there is no default profile — an agent fell back to the owner's and commented as them; see `docs/decisions.md`.)
27. As a developer, I want profiles in `~/.annotary/config.toml` (`server`, `token`, optional `vault`) selectable by `-p/--profile` or `ANNOTARY_PROFILE`, with `ANNOTARY_TOKEN` / `ANNOTARY_SERVER` env overrides, so that each agent process can carry its own identity.
28. As an agent with no credential, I want exit 3 and a message that says to pass `-p NAME` (or set the env var) and lists the profiles that exist, so that the fix is obvious.
29. As a Member lacking a Right, I want exit 4 with the missing Right named and a `member grant` hint, so that I can tell the owner exactly what to grant.
30. As an owner, I want Rights to be exactly `read`, `comment`, `action`, `write` per Vault (`read` implied by the others), with `write` covering create, push, delete/restore, and the per-Document Action declaration, so that the model is minimal.
31. As an owner, I want a Grant to be `{vault | "*", rights[]}` on a Membership, where `*` covers Vaults created later, so that "all Vaults" needs no extra concept.
32. As an owner, I want purge, the auto-purge setting, Vault CRUD, Member/Membership management, Vault default Actions, server settings and the local password to be owner-only and never grantable, so that the Tenant has a clear administrator.
33. As an owner, I want the last owner to be un-demotable, so that a Tenant always has one.
34. As an owner, I want the Members screen to show handle, display name, role, grants, auth method (as a hint, never a "bot" badge) and per-Token `last_used_at`, with grants/roles behind an "advanced" disclosure, so that the single-developer case sees nothing it doesn't need.

### Documents, Versions, assets

35. As a Member with `write`, I want `annotary push FILE.md [-m MSG]` to create a new Document (Version 1) when the file has no Sidecar, so that starting is one command.
36. As a Member, I want `push` of a file with a Sidecar to bind by Document id and create the next Version, so that the Sidecar is the binding.
37. As a Member, I want `push --doc ID` to bind an unbound file to an existing Document (writing the Sidecar), so that I recover from a lost Sidecar without a clobbering pull.
38. As a Member, I want `push --force` to rebind a Sidecar from another Document id, so that a mistaken binding is recoverable.
39. As a Member, I want the Document's Name to be the filename stem I pushed, recorded per Version, with the head's Name shown, so that renaming is just pushing under a new filename (no `rename` verb).
40. As a Member, I want the stem taken verbatim (Unicode, case) with rejection of empty / separators / control chars / leading `.` and a required `.md` extension (typed errors), so that names are predictable and portable.
41. As a Member, I want duplicate Names within a Vault to be allowed, with the 8-char id as the real handle, so that daily use never fights a uniqueness rule.
42. As a Member, I want any `DOC` argument to accept an id, a Name that is unique in the Vault, or a path to a pulled `.md` (the CLI reads the sibling Sidecar), so that I can refer to a Document the way that is handy — and get exit 2 listing `id name vN updated` when a Name is ambiguous.
43. As a Member, I want the Vault chosen from Sidecar → `--vault` → profile `vault` → typed error listing Vaults when there is more than one, so that single-Vault Tenants never see the question.
44. As a Member, I want a push identical to head to be a no-op ("already vN"), so that re-pushing is safe.
45. As a Member, I want a push identical to an older Version to create a new Version reported as "vN (same content as vK)", so that history is linear and honest (never a revert).
46. As a Member, I want an asset-only change (same Markdown) to create a new Version, so that Version identity covers the whole Document.
47. As a Member, I want each Version to record `{number, hash, by, at, message?, name}`, be immutable and never deleted, so that Threads and Actions always have a stable Version to pin to.
48. As a Member, I want `push` to scan my Markdown for local relative references (Markdown images/links/reference-style, HTML `img/video/audio/source/a`) of supported types (md/markdown/txt; png/jpg/jpeg/gif/webp/svg/heic/heif/avif/bmp/tif/tiff; mp4/mov/m4v/webm; mp3/m4a/aac/wav/ogg/flac; pdf) and upload them as Assets, so that a Document travels with what it references.
49. As a Member, I want `http(s)://`, `#frag`, `mailto:`, `data:` references ignored silently and absolute filesystem / `file://` paths warned and ignored, so that only portable references become Assets.
50. As a Member, I want a linked `.md` to be an Asset (not a Document), scanned recursively relative to its own directory, with cycles ignored and depth capped at 5 (warning beyond), so that a small doc set rides along.
51. As a Member, I want a missing or unreadable referenced file to warn and still push, with `missing: true` in the manifest, so that a broken link never blocks review — and `--strict` to fail instead.
52. As a Member, I want unsupported referenced types to warn and be left as written, so that nothing surprising is uploaded.
53. As a Member, I want caps of 2 MB Markdown / 100 MB per Asset / 500 MB per Version enforced with typed rejections, so that the local store stays sane.
54. As a Member, I want push to be two-phase (manifest → server lists lacking hashes → upload only those → commit), with content-addressed Tenant-wide de-dup, so that unchanged Assets are never re-uploaded.
55. As a Member, I want `push` to record image/video dimensions in the manifest at push time, so that the web UI can reserve boxes and reach CLS 0.
56. As a Member, I want `push` to produce web-safe derivatives for HEIC/HEIF/AVIF/TIFF/BMP (content-addressed second blob linked in the manifest, originals untouched), so that every image displays in every browser.
57. As a Member, I want `push` over a head that moved since my pull (the Sidecar's `base_version`) to be refused unless my file already contains the head's changes since that base — never warned about and overwritten — so that concurrent edits all end up in the Document while merging stays with my own tools. *(Changed by ticket 17, ADR 0002; was: warn by default, refuse under `--strict`.)*
58. As a Member, I want `--strict` to turn every push warning (missing/unsupported asset, absolute path, depth > 5, no web derivative) into exit 5, so that CI-style pushes can be strict. *(Ticket 17: a moved head is no longer a warning.)*
59. As a Member, I want `push` to print progress and warnings on stderr and exactly one result line on stdout (`name id vN`, `vN (same content as vK)`, or `already vN`), with `-q` silencing progress, so that scripts and agents parse one line.
60. As a Member, I want `push` to show progress on large Documents, so that the deliberately heavy push (render + re-anchor + derivatives) doesn't look hung.
61. As a Member with `read`, I want `pull DOC [--version N] [--out DIR]` to write exactly `<name>.md` (byte-identical to the pushed Version) and `<name>.annotary`, so that data is plain files.
62. As a Member, I want `pull --version N` to write N's Name and the Sidecar to carry `base_version: N`, so that tools pick the right placement.
63. As a Member, I want `pull` to refuse (exit 5) over a local `.md` whose hash differs from the last pulled/pushed hash in the Sidecar unless `--force`, so that local edits are never clobbered.
64. As a Member, I want Assets off by default and `--assets` to write each to its referenced relative path (creating dirs), skipping same-hash files and refusing differing files without `--force`, so that pulls are fast and safe.
65. As a Member, I want `pull` to print `name.md name.annotary vN` on stdout and the "8 threads, 4 assets (1 missing)" summary on stderr, so that output is parseable.
66. As a Member with `write`, I want `delete DOC` to be soft (Document hidden by default, Threads/Versions/Actions retained) and `restore DOC` to undo it, so that nothing is lost by accident.
67. As an owner, I want `purge DOC` to permanently remove a deleted Document and GC blobs no other Version references, plus an off-by-default "purge deleted after N days" setting, so that space can be reclaimed deliberately.
68. As a Member, I want `versions DOC` to list Versions with `number by at message same_as`, so that I can see history without a diff tool.
69. As a Member, I want no diff view in Phase 1 (CLI or web), so that the product stays focused; I use my own Markdown diff tools.

### Listing and finding Documents

70. As a Member, I want `list [NAME]` to print `id name vN action|- open_threads updated`, sorted by last activity (any Version/Thread/Action event) desc, limit 50 with a truncation note, so that "what's going on" is one cheap call.
71. As a Member, I want filters `--vault`, `--deleted`, `--mentions-me` (≥ 1 open Thread mentioning me), `--action ID|none`, `--since T`, and a positional Name substring, so that I can narrow the list without full-text search.
72. As a Member, I want the web Document list to be the same list (same sort, columns, filters; H1/front-matter title display-only), so that humans and agents see the same thing.
73. As a Member, I want no unread markers, folders, tags or archive, so that the model stays as simple as Vaults + Documents.

### Threads and Comments

74. As a Member with `comment`, I want to start a Thread on a span of the Markdown source (`thread new DOC "L:C-L:C" -m MSG`), on the whole Document (`--document`), or on an inline Asset (`--asset REF`), so that I can comment on exactly what I mean.
75. As an agent, I want `thread new --version N` to refuse (exit 5) if head ≠ N and to verify the quoted text at that range on head before posting, so that I never comment on text I haven't seen.
76. As a Member, I want new Threads to be allowed only on the head Version, while replies/resolve/reopen work from any Version view, so that comments always attach to current content.
77. As a Member, I want Comment bodies in Markdown rendered with the Document superset minus embeds (images/files render as plain links; comments are never asset-scanned), so that comments are expressive but light.
78. As a Member, I want `thread reply DOC ID -m MSG|-` (`-` reads stdin) with `--resolve` to post and resolve in one step, so that "done in v5" is one command.
79. As a Member, I want `thread resolve | reopen DOC ID` as bare events by anyone with `comment`, stamped `{by, at, version}`, so that tracking (who/when) rather than control is the solution.
80. As a Member, I want to edit my own Comment (owner can edit any), setting `edited_at` with no history, so that typos are fixable.
81. As a Member, I want delete to leave a tombstone (body cleared, `deleted {at, by}`, id kept; root tombstoned keeps the Thread and replies alive; fully-tombstoned Threads hidden), so that ids and the event log stay coherent.
82. As a Member, I want to `@handle`-mention Members (resolved to Member ids at post time; not inside code spans/blocks; unknown handles stay plain text), so that I can call someone in and it survives renames.
83. As a Member, I want `mentions [--since T]` to list open Threads mentioning me across all Documents, so that "what needs me" is one call.
84. As a Member, I want Thread ids per Document, sequential and short (`t7`, replies `t7.1`), so that they are cheap to cite in prose.
85. As a Member, I want `thread list DOC` to default to open Threads on head, in document order (Document-level first, then by current position), orphaned last and shown with `orphaned` in the anchor column, so that agents don't miss open questions.
86. As a Member, I want `thread list` filters `--all`, `--resolved`, `--status anchored,refined,orphaned`, `--mentions-me`, `--since T`, `--sort recent`, `--version N` (read-only time-slice), so that I can slice the list without reading everything.
87. As a Member, I want `thread show DOC ID...` to print the full Thread with the full `vN:L:C-L:C` range, the frozen quote and (when refined) the current text, so that I can see precisely what a Thread points at.
88. As a Member, I want `show DOC` to print a header line (`name id vault vN-by-at action:… open:N actions:a,b`) followed by the open Threads, so that one ≈245-token call is the read path.
89. As a Member, I want the total of all Comment bodies in one Document capped at 100,000 characters (server constant), with a clear exit-5 error naming the cap and current count, so that Sidecars stay bounded.

### Anchors across Versions

90. As a Member, I want a span Thread's original placement (Version, range, exact quote) frozen forever and used as its label, so that citations by id + quote never go stale.
91. As a Member, I want every Thread re-anchored at each Version upload by a deterministic pure function (histogram line diff, then character diff within changed lines; sticky inside, non-sticky at edges), so that the same inputs give the same placement in the CLI, the server and later WASM.
92. As a Member, I want exactly three placement statuses — anchored (text unchanged, may have shifted), refined (text under the span grew/shrank/changed; current text kept), orphaned (nothing survives; last valid placement kept, stamped with its Version) — so that state is honest and simple.
93. As a Member, I want partial deletion or insertion inside a span to shrink/grow it (refined, Google-Docs behaviour) rather than orphan it, so that small edits keep the conversation attached.
94. As a Member, I want whitespace-only line changes to keep a Thread anchored, so that reformatting doesn't orphan review.
95. As a Member, I want type-over, find-and-replace that swallows the span, and moved blocks to orphan the Thread, and orphaned to be sticky (a later Version restoring the text never revives it), so that placement never silently guesses.
96. As a Member, I want an orphaned Thread to remain open, listed, replyable and resolvable, so that orphaned is a placement state, not a status.
97. As a Member, I want Document-level Threads to have no Anchor and never orphan, so that "about the whole doc" is always safe.
98. As a Member, I want an Asset Thread anchored by content hash (referencing line display-only): same hash referenced anywhere in the new Version ⇒ anchored, hash no longer referenced ⇒ orphaned — so that image comments follow the image.
99. As a Member, I want positions in source coordinates (1-based lines, 0-based code-point columns, half-open, CRLF tolerated) as `L:C-L:C` range strings everywhere, so that CLI, Sidecar and web agree.
100. As a Member, I want each Version's placement recorded (sparse history: entries only where placement changed), so that viewing an older Version is a lookup, not a recomputation.
101. As a Member, I want the anchoring behaviour pinned by the empirical fixture table (Docs D1–D12, GitHub G0–G14 plus the ticket-05 additions) with documented divergences, so that behaviour is testable and explainable.

### Actions and status

102. As an owner, I want Vault default Actions `approve` (positive) / `send-back` (neutral) editable in the web UI/CLI, so that decisions are named the way my team names them.
103. As a Member with `write`, I want a per-Document `actions.available` declaration in the Sidecar (`{id, label, intent}` list, `[]` allowed) that replaces the Vault set entirely on push, so that a Document can have its own buttons or none.
104. As a Member with `action`, I want `act DOC ACTION [--version N]` to record `{id, by, at, version, label, intent}` on head only (exit 5 if `N ≠ head`; undeclared id rejected), so that I never approve content I haven't seen.
105. As a Member, I want Actions to be append-only, with no note, no retract, and last event on a Version winning for reporting, so that Actions are a decision record, not a conversation.
106. As a Member, I want a new Version to start with no Action (no "stale" concept), so that nothing needs recomputing.
107. As a Member, I want `actions DOC` to list Action history with snapshotted label/intent, so that history reads sensibly after the declaration changes.
108. As an agent, I want `status DOC` (≈24 tokens) to return `version`, `action {id, by, at} | null` (last on head), `open_threads`, `mentions_me`, `seq`, so that one call answers "what needs me and where do I resume from".
109. As a Member, I want no orphaned count in `status`, `list` or push output by default (only `thread list --status orphaned`), so that agents aren't nagged.

### Waiting and events

110. As an agent, I want `wait [DOC] [--on version,action,mention] [--action ID] [--cursor SEQ] [--timeout 30m]` to block until a matching event, print every co-arriving event as `SEQ KIND DOC DETAIL` one per line, and exit 0, so that I don't poll.
111. As an agent, I want `--on thread,reply,resolve` opt-in, so that the default wake set stays small.
112. As an agent, I want `wait` to exit 7 with `wait: nothing after DUR (seq N)` on timeout (`--timeout 0` = forever), so that I can loop deterministically.
113. As an agent, I want the `--cursor` to be a durable Tenant-wide integer from `status`/previous `wait`, so that a wait resumes correctly across a server restart or my own restart.
114. As an agent, I want `wait` without DOC to cover every Document I can read, so that one process can babysit a Vault.
115. As a web user, I want live updates via SSE on the same events endpoint, so that the page reflects others' replies without reload.
116. As a Member, I want an in-app "what's new" (from the events log) instead of notifications, so that Phase 1 stays simple.

### Web UI

117. As a human, I want a Document view that renders the Markdown (≤ 72ch column) with Thread cards in a right margin at their anchor line when the container is ≳ 850px, and a single column with Threads inline under their block when narrower, so that it works from a 20% strip to 3000px (no third column ever).
118. As a human, I want a one-line persistent narrow header (`name · vN · n open · n @you` + Version arrows) that opens an overlay with full status, Version list, Actions, resolved toggle and what's new, with no bottom bar, so that small viewports keep the doc on screen.
119. As a human, I want to select text → floating pill (`c`) → composer where the Thread will live (head only; no per-block `+`), so that commenting is one gesture.
120. As a human, I want to click/select an inline image/video/audio/PDF → same pill → whole-Asset Thread, so that media is commentable.
121. As a human, I want links to Assets (e.g. `[API notes](api-notes.md)`) to be ordinary text (comment on the link text; click opens the Asset in a new tab served by the local server), so that there is no lightbox to learn.
122. As a human, I want refined Threads to show the frozen original quote plus a `now:` line and a dotted underline in the doc, so that I see what changed under a comment.
123. As a human, I want orphaned Threads grouped at the end behind a `▸ Orphaned (n)` disclosure, hidden by default but still replyable/resolvable, so that they don't clutter but aren't lost.
124. As a human, I want a Version `<select>` (rows: by · at · same as vN · Action) and an older-Version banner ("Viewing v2 … Threads as they were; replies/resolve work, new comments only on head" + Go to head), so that time-slice is explicit.
125. As a human, I want Action buttons (coloured by intent) only on the head view, each Version showing its own Action chip, `[]` ⇒ no buttons, so that Actions are per-Version.
126. As a human, I want keyboard `j k r e c [ ] ↩ ⌘↩ esc ?` and nothing more, so that the UI is fast without being a modal editor.
127. As a human, I want every local action (reply, resolve/reopen, Action, new Thread) applied optimistically in < 50 ms, confirmed by the server, rolled back with a visible error on failure, so that it feels instant.
128. As a human, I want FCP < 100 ms, interactive < 200 ms, Version switch < 150 ms, Document list < 100 ms and CLS 0 (Threads/placements in the first HTML; media boxes from manifest dimensions; no font/CSS swap), on localhost, mid-range laptop, cold cache — including on a generated ≥ 5,000-line / ≥ 500-Thread Document — so that "extremely fast" is measurable.
129. As a human, I want inline rendering of images/video/audio/PDF/text (SVG via `<img>`; HEIC/HEIF/AVIF/TIFF/BMP via derivatives), a broken-asset placeholder for `missing`, and a "same content as vN" note, so that every Version is faithful.
130. As a human, I want Members shown by name + a glyph for auth method (never a "bot" badge) with agent harness/model as secondary text if present, so that people and agents read as equals.
131. As a human, I want a login page only when a password is set, so that loopback use has zero friction.
132. As a human, I want a Deleted filter in the list and no unread state, so that the list is quiet.
133. As a human, I want all timestamps localized to my time zone in the web UI and TTY CLI (ISO-8601 UTC in `--json`), so that times are readable and parseable.

### Sidecar and portability

134. As a developer, I want the Sidecar to be JSON (extension `.annotary`) with leaf objects inline and containers 2-space nested, fixed key order, not minified, so that it is git-diffable and reliably parseable.
135. As a developer, I want the Sidecar to carry `format, id, vault, base_version, members, versions[], assets[] (+via), actions{available, inherited?, log}, threads[]` — the server's full projection regardless of which Version was pulled — so that the data is complete plain files.
136. As a developer, I want handles (not Member ids) in the Sidecar plus a `members {handle: {id, name}}` table, so that it is human-readable and lossless.
137. As a developer, I want push to read only `id`, `vault`, `base_version` (+ hash) and `actions.available` when `inherited` is absent, ignoring hand edits elsewhere, so that the Sidecar is authoritative for exactly the right things.
138. As a developer, I want `actions.inherited: true` to mark copied-in Vault defaults so that editing `available` and dropping the flag is how a per-Document declaration is made.
139. As a developer, I want the sparse `history` rule (entry only on change; placement at N = last entry ≤ N) shared by CLI and server, so that files stay small and lossless.
140. As a developer, I want YAML/JSONL/minified variants rejected on push, so that there is one canonical writer.
141. As a developer, I want a W3C Web Annotation export to be a mapping (backlog tool), not native storage, so that the native shape stays compact.

### Rendering (compute at the edge)

142. As a developer, I want the pushing CLI to render Markdown → viewer-agnostic HTML (semantic markup only, `data-l="L-L"` on every block, no CSS/JS/chrome, MathML for math, code highlighting may be baked, Mermaid left to viewer JS) and upload it as a derived blob with the renderer version recorded on the Version, so that views are static and rendering fixes ship as a CLI release.
143. As a developer, I want the server to keep the same renderer crate as a fallback (missing/too-old HTML) and `annotary rerender DOC|--all` to re-render in place without touching Versions/Threads, so that a renderer fix can be applied retroactively.
144. As a developer, I want the CLI to submit re-anchoring `placements[]` with the commit and the server to accept them from a `write` Member (optionally spot-verifying with the same pure function), so that the server never computes on push.
145. As a developer, I want the renderer, diff/re-anchor and sidecar code as standalone crates that also build to WASM, so that Phase 2 browser upload reuses them.
146. As a developer, I want the renderer to handle one superset (GFM + tables/task lists/footnotes + fenced code + Mermaid + math + front matter) with no flavour detection, so that behaviour is predictable.

### Agent skill file

147. As an agent, I want a ≤ 60-line / ≈900-token SKILL.md (Agent-Skills frontmatter with `name`, trigger `description`, `metadata.version`, `metadata.cli`) telling me: read via CLI never the file; cite id + quote never line numbers; pull → edit outside → push; reply then `--resolve`; `--version N` guards; wait don't poll; `--json` when parsing — plus a command table and exit codes, so that I can work Annotary correctly with minimal context.
148. As an agent, I want `annotary skill install [--dir DIR]` to write the binary's embedded SKILL.md (default `./.agents/skills/annotary/`), and the skill to tell me to re-install when `annotary --version` ≠ `metadata.version`, so that the skill stays current.
149. As an agent, I want `--help` to be the full reference (≈700 tokens on demand) with no separate references file, so that context stays small.

### CLI output and errors

150. As an agent, I want non-TTY output to be one record per line, tab-separated, no header, no colour, and TTY output aligned with a dim header, relative times and colour — same fields, same order — so that parsing is trivial and humans still get a nice view.
151. As an agent, I want `--json` bare to return a documented compact default set, `--json a,b` to narrow, `--json ?` to list fields, minified with empty/null omitted, so that token cost is under my control.
152. As an agent, I want the anchor column as `vN:L "exact"` / `vN:L refined "exact"` / `vN:L orphaned "exact"` / `document` / `asset REF`, so that every line number is stamped with its Version.
153. As an agent, I want exit codes `0 ok · 1 usage · 2 not found/ambiguous · 3 no credential · 4 missing right · 5 conflict/refused · 6 unreachable · 7 wait timeout` and errors as one line `error: MSG — HINT` on stderr (or `{error,message,hint}` under `--json`), so that I can branch on failures.
154. As a human, I want `show`/`push` on a TTY to append the web URL, so that I can jump to the browser.

### Concurrent edits (ticket 17, ADR 0002 — after Phase 1)

155. As an agent working in a git worktree, I want `push` to accept my file when it already contains the edits another worktree pushed since my pull, so that after a normal git merge I can push without extra steps or warnings.
156. As a Member, I want `push` to refuse — and show me exactly which changes are missing — when the head has edits my file does not contain, so that a concurrent edit is never silently dropped from the head.
157. As an agent, I want the refusal to print the missing changes as a unified diff on stderr (and in `--json`), so that I can put them into my file and push again without another tool.
158. As a Member, I want a git-conflicted `.annotary` Sidecar to need no resolving, so that merging a branch only asks me to resolve the Markdown.
159. As a Member, I want an explicit, deliberately long flag to push over the head's changes (`--discard-head-changes`), so that dropping someone's edits is always a visible decision.
160. As a Member, I want pushing a stale, unedited file to be refused rather than reverting the head, so that an old checkout cannot undo newer work.
161. As a Member, I want an Asset the head changed (same `ref`, new bytes) to count as one of the head's changes, so that pushing an old copy of the image cannot revert it.

### iOS app prerequisites (ticket 18 — after Phase 1; the app's `docs/requirements.md` §R)

162. As an owner, I want `serve --bind ADDR --allow-remote` (and the same on `daemon install`) to serve on a Tailscale or LAN address, with loopback trust still limited to real loopback peers, so that my phone reaches the server directly and nobody on the network becomes the owner.
163. As an owner, I want to tell the server the URL other devices use (`serve --url`, plus an optional link relay `--link-base`), so that every link the CLI prints for a person opens on their phone — in the app where it is installed.
164. As an agent, I want `annotary link DOC [tN]` to print that link, so that I can point a person at a Document or a Thread in one line.
165. As a Member, I want to pair a phone by scanning a QR code (`annotary pair` in the terminal, **Pair a device** on the web Members screen), so that the app gets its own revocable Token without copying secrets by hand.
166. As an app, I want the Document list and Document API to carry the display title, so that I show what the web shows without rendering Markdown.
167. As an iPhone user opening a Document page in Safari, I want an "Open in Annotary" link, so that I can continue in the app.

## Implementation Decisions

### Runtime and process
- **Rust**, one binary `annotary` (no short alias) that is both CLI and server. Rationale: millisecond cold start (agents call it constantly), single-file distribution, off-the-shelf histogram line diff (`imara-diff`) and character diff (`similar`). Renderer, diff/re-anchor, sidecar and store logic are separate crates; renderer/diff/sidecar also compile to WASM.
- `serve [--port 7377] [--data-dir]` runs in the foreground, logs to stderr; no self-forking. `daemon install|uninstall|status|start|stop|restart` (macOS only) writes a user LaunchAgent (`com.annotary.server`, ProgramArguments `[binary, serve, --data-dir, --port]`, RunAtLoad, KeepAlive, logs to the data dir) and drives it with `launchctl bootstrap/bootout/kickstart`; upgrade = replace binary + `daemon restart`. Elsewhere `daemon` exits 1 "unsupported".
- First `serve` / `daemon install` on an empty store runs first-run setup inline: owner handle + display name (defaults from `$USER`), optional local password, Vault `notes`; ends by pointing at how to obtain a CLI credential.
- Discovery: none. Fixed default port 7377; the CLI calls the profile's `server` URL (default `http://127.0.0.1:7377`). No auto-start. `server.json` is informational.
- Data dir `~/.annotary/` (`ANNOTARY_HOME`, `--data-dir`): `config.toml` (profiles), `server.json`, `logs/`, `store/` (with `LOCK`; `meta.json` with store schema version + tenant id). `export FILE.tar` = tar of `store/`.
- Hourly in-process ticker: auto-purge (if enabled), blob GC after purge, derived-blob cleanup.

### Storage (SQLite is the truth + events table + blobs on disk)
- SQLite (WAL) is authoritative; `PRAGMA user_version` + forward-only migrations on `serve` start.
- Tables: members, memberships (handle, role, grants), tokens (sha256 hash, name, expires_at?, last_used_at, revoked), vaults (name, default actions), documents (id, vault, deleted_at, last_activity_at), versions (number, name, hash, by, at, message, manifest incl. dimensions/derivatives/via/missing, placements, renderer, html_hash), threads / comments / actions (append-only events stamped `{by, at, version}`), settings, sessions, and **events (seq, at, by, type, doc_id, payload)** — append-only, Tenant-wide monotonic `seq`, written in the same transaction as every mutation.
- Blobs: `store/blobs/sha256/ab/<hash>` content-addressed plain files — Markdown, Assets, rendered HTML, image derivatives — Tenant-wide de-dup. `pull` is the portability boundary (Markdown + Assets + full-projection Sidecar).
- Single writer: only the server touches the store; CLI always over HTTP. Per-Document in-process lock around id allocation + event append; one Tenant lock for member/vault writes.
- Document ids: 8 lowercase base32 chars (alphabet without `0/1/o/l`), Tenant-unique, random. Member ids: ULID. Thread ids per Document (`t7`, `t7.1`), server-allocated.
- Password argon2id; tokens sha256 at rest; tokens opaque random ≥ 32 bytes base62 with a recognisable prefix (`ant_`).

### API surface (shared by CLI and web)
- JSON REST under `/api/v1/…`: `docs` (list/create), `docs/{id}` (get/delete/restore/purge), `docs/{id}/versions/{n}`, `docs/{id}/threads[/{tid}]` (new/reply/edit/delete/resolve/reopen), `docs/{id}/actions` (list/act), `blobs` (two-phase: manifest → lacking hashes → upload → commit with `{manifest, derivatives[], placements[], html_hash, renderer, message?, base_version?}`), `events?since=<seq>&wait=30[&filter]` (long-poll; SSE for web), `members`, `tokens`, `login`, `settings`. Web pages `/`, `/d/{id}[/v/{n}]`, `/a/{hash}` are server-composed HTML with Threads + placements in the first response.
- Auth middleware: `Authorization: Bearer` (CLI) or cookie (browser; auto-issued to loopback iff no password) → Member or 401; missing Right → 403. Tenant implicit locally; Phase 2 adds a tenant prefix/header, resource shapes unchanged.
- Time-slice queries (Threads at Version N) are one cheap read of stored placements + event stamps; `wait` parks ≤ 30 s per request.

### Access model
- Member `{id, display_name, credentials[], created, disabled?}` — no kind field. Membership `{member, tenant, handle, role ∈ owner|member, grants[], created, disabled?}`. Grant `{vault | "*", rights[]}`, rights ∈ read/comment/action/write, read implied. Owner-only ops as listed in the stories; last owner cannot be demoted; disable never delete; handles unique per Tenant forever.
- Local browser: loopback = owner unless password set; then login page + cookie. CLI: credential always required; `login` exchanges the local password for a Token stored in a profile (no default profile: every call names one); server never auto-writes an owner profile.

### Documents, Versions, Assets
- Version identity = sha256 over Markdown blob hash + sorted `(ref, asset-hash)` manifest; sequential number per Document; `same_as` derived from hashes, never stored. Push identical-to-head = no-op; identical-to-older = new Version "same content as vK"; asset-only change = new Version. Versions immutable, never deleted.
- Delete soft (`deleted_at`), restore, purge (owner) with blob GC; auto-purge setting off by default.
- Asset scan rules, supported-type list, caps, recursion depth 5 with cycle set, missing ⇒ warn + `missing:true`, absolute/`file://` warned + ignored — as listed in the stories; the type list and caps are single constants.
- Manifest entries `{ref, hash, size, type, missing?, via?, width?, height?, derivative?}`.
- Names: filename stem verbatim per Version; rules as stated; not unique; `list` always shows id.

### Anchor model
- Subject ∈ span | document | asset. Span anchor = `original {version, range, exact}` (frozen) + `current {version, range, status, exact?}` + sparse `history [{version, range, status}]`. Asset anchor = content hash (+ referencing line display-only). Statuses exactly anchored / refined / orphaned; orphaned sticky; orphaned ⊥ resolved.
- Re-anchoring N → N+1 = histogram line diff, then character diff within changed lines; sticky-inside / non-sticky-edges; surviving text empty or whitespace-only ⇒ orphaned; equals `original.exact` ⇒ anchored; else refined. Pure function of (text N, text N+1, anchor); identical in CLI, server and WASM. Fixture outcomes and divergences (Docs D2–D4 shrink; D5/G6 grow; G5 divergence; G8 no revival; G11 moved ⇒ orphaned; G12 divergence — whitespace stays anchored; D8/D10' ⇒ orphaned; D10 overlap ⇒ refined; D9a single-range) are binding.
- Coordinates: lines 1-based, cols 0-based Unicode code points, half-open, source coordinates only; rendered-selection → source mapping is the web UI's job via `data-l` and text offsets. Zero-width spans valid but not produced by Phase 1 UI/CLI (Phase 3 hook).
- Agent guarantees: stable Thread id; immutable `original`; `current` changes only at Version upload; orphaned sticky; deterministic; every printed line number stamped with its Version.

### Thread model
- Thread = root Comment + ordered Replies; body Markdown minus embeds; 100k-char total cap per Document (server constant); edit by author/owner sets `edited_at`; delete = tombstone; resolve/reopen bare events by anyone with `comment`, stamped `{by, at, version}`; every Thread event stamped with head Version → time-slice; new Threads head-only; mentions `@handle` resolved to Member ids at post time (not in code spans/blocks; unknown = plain text); ordering document order, Document-level first, orphaned last (by original position), replies chronological; `--sort recent`; default open-only.
- Time-slice at Version N: Threads created ≤ N, replies ≤ N, resolved-state per last resolve/reopen ≤ N, placement per sparse history at N.

### Actions model
- Declaration `{id, label, intent ∈ positive|neutral|negative}`; Vault defaults `approve`/positive, `send-back`/neutral (owner-editable, affects only Documents without their own declaration, only future Actions); per-Document `actions.available` replaces the Vault set entirely (`[]` allowed); server copies the effective set into the Sidecar on pull with `inherited: true`.
- Event `{id, by, at, version, label, intent}` snapshotted; head-only; append-only; no note/retract; last event on a Version wins; undeclared id rejected; one Document-level `action` Right.
- `status` = `version`, `action {id, by, at} | null`, `open_threads`, `mentions_me`, `seq`. No orphaned count anywhere by default.

### Sidecar format
- JSON, `.annotary`, leaf-inline layout, fixed key order, not minified; `format: 1`. From the sidecar prototype, the decided top-level shape (order fixed):

  ```
  format         1
  id             8-char base32 Document id
  vault          Vault name
  base_version   the Version the sibling .md is
  members        {handle: {id, name}} for every handle referenced
  versions[]     {number, name, hash, by, at, message?}
  assets[]       {ref, hash, size, type, missing?, via?}   for base_version
  actions        {available[{id,label,intent}], inherited?, log[{id,by,at,version,label,intent}]}
  threads[]      {id, subject, anchor?, author, created{at,version}, body, mentions?, edited_at?, deleted?, resolved?, replies?}
  anchor         {original{version, range|hash+ref+line, exact?}, current{version, range|line, status, exact?}, history[{version, range|line, status}]}
  ```
  Ranges as `"L:C-L:C"`; handles not ids; sparse history; omit empty/null (`resolved` present ⇒ resolved; `current.exact` omitted when equal to original).
- Push reads only `id`, `vault`, `base_version` (+ hash from `versions[]`), and `actions.available` when `inherited` is absent. Everything else is projection, rewritten whole on pull; hand edits ignored. Agents never read the file (skill says so).

### CLI surface
- Verbs, flags, defaults, output formats, anchor column, `--json` semantics, `wait` semantics, guards, exit codes and error format exactly as in the stories and the CLI prototype (`help.txt`, `session-agent.txt`, `session-human.txt` are the goldens). `DOC` = id | unique Name | path to pulled `.md`; thread verbs take `DOC ID`; composite `plan/t7` rejected. Non-TTY vs TTY parity (same fields/order). Progress/warnings on stderr, one result line on stdout for `push`/`pull`.
- Compute at the edge in `push`: sha256, asset scan (recursive `.md`, depth 5), manifest with dimensions, HEIC/AVIF/TIFF/BMP derivatives, histogram + char diff re-anchoring of every Thread (`placements[]`), Markdown → HTML render (`html_hash`, `renderer`), all submitted with the commit; the server validates and stores.
- Skill file: the SKILL.md prototype is the content (frontmatter `name`, `description`, `metadata.version`, `metadata.cli`); embedded in the binary; `skill install [--dir]`; keep-current hook by version comparison; no references file.

### Web UI
- Server-rendered HTML + a small script (no SPA required; the budgets are what's binding). Layout A (margin) ≳ 850px container / C (inline in flow) below; narrow header line + overlay; selection pill; whole-Asset Threads for inline media only; Asset links plain text opening in a new tab; refined = frozen quote + `now:`; orphaned disclosure hidden by default; Version `<select>` + time-slice banner; Action buttons head-only, coloured by intent; keyboard map; optimistic mutations with rollback; owner screens (Members, Add agent, password, Vault default Actions, settings) with grants/roles behind "advanced"; login page only when a password is set. Renderer output must preserve source positions (`data-l`) so selection maps to source coordinates.

## Testing Decisions

**What makes a good test here:** it exercises external behaviour — a CLI invocation against a real server, an HTTP request, a Markdown-in / placements-out pure function, a browser interaction — and asserts on outputs users or agents see (stdout/stderr/exit code, JSON, HTML, timings), never on internal structure. Fixtures and goldens exist before implementation of each area; unit tests are written alongside (not strict red-green everywhere).

**Seams (highest first, fewest possible):**
1. **CLI end-to-end against a real spawned `serve`** (`trycmd`) — the primary seam. Because the CLI always goes over HTTP, this single seam covers CLI parsing/formatting, the API, auth, store, events, and the edge computation in `push`. Goldens seeded from the CLI prototype's agent and human transcripts; one case per verb path and per exit code 1–7; TTY parity via `ANNOTARY_TTY=1` env override (no pty); `--json` bare / `a,b` / `?`; stdout/stderr separation asserted; `wait --cursor` resume across a SIGKILL + restart of `serve`.
2. **Server integration over HTTP** (in-process server + temp store) — for what the CLI can't naturally reach: the rights matrix (table-driven: {read, comment, action, write} × every endpoint × {owner, member, `*` grant, vault grant, none, disabled} → 200/401/403), `events.seq` monotonic + same-transaction atomicity (`fail`-crate failpoints between mutation and event insert ⇒ neither), blobs/GC never dropping a referenced blob, time-slice queries, forward-only migrations from every prior schema version, `LOCK`, `server.json` stale pid, non-loopback bind refused, server fallback render == CLI render, spot-verification of submitted placements.
3. **Crate unit/property tests** for the pure functions that also build to WASM: re-anchoring driven by `fixtures/anchoring/<id>/{v1.md, v2.md, …, threads.json, expected.json}` (one dir per row of the research fixture table plus the ticket-05 additions; a checker fails if any row id lacks a directory); renderer `insta` HTML snapshots over a Markdown corpus (each syntax feature + each asset type), plus assertions of no `<style>`/`<script>` and `data-l` on every block, byte-stable per (Markdown, renderer version); sidecar `proptest` projection → emit → parse → emit byte-identical, key-order stability, sparse-history-vs-full oracle ported from the sidecar prototype's build script; asset scan rules; name validation.
4. **Browser tests** (Playwright/TypeScript — the only non-Rust foothold, justified because FCP/CLS/interactive come from CDP) against the real server with a seeded store: both layouts, narrow header overlay, selection pill → Thread, keyboard map, Version time-slice, optimistic-action rollback on server error, orphaned disclosure, login page iff password.
5. **Perf suite** (separate `make perf`): Playwright + CDP on the deterministic seeded generator (≥ 5,000 lines / ~50k words / ≥ 500 Threads; generator checked in, output not); FCP < 100 ms, interactive < 200 ms, Version switch < 150 ms, list < 100 ms, CLS 0, optimistic < 50 ms; server TTFB measured separately; median of 5 cold-cache runs; absolute budgets, no CI multiplier; a dedicated runner, `make perf` on a developer's machine blocks merge by convention until then.

**"Comprehensive" = spec-id traceability, not line coverage.** Every "From [ticket]" bullet in the test-strategy ticket becomes a spec id (`S05-03`, `S09-12`, …) in `tests/SPEC.md`; tests name the ids they cover; a checker fails CI when any id has zero tests.

**Commands and budget:** `cargo test` = layers 2+3 in < 60 s; `make e2e` = trycmd + Playwright; `make perf`; `make check` = everything the machine can run.

**Deferred (not automated in Phase 1):** `launchctl` itself (plist text is unit-tested), HEIC/AVIF decode on Linux CI (macOS only), video dimensions beyond one mp4 fixture, parser fuzzing, load/soak, systemd/Windows.

**Prior art:** none in-repo yet (greenfield). Patterns to follow: `trycmd` goldens for CLI, `insta` for snapshots, `proptest` for round-trips, `fail` for failpoints, table-driven rights matrix, Playwright + CDP for web vitals.

## Out of Scope

- Cloud / multi-tenant hosting (Google Cloud), sync from local to cloud, human sign-in (Google/passkey/device-flow), iOS/Android apps, notifications (mention settings, browser, email) — Phase 2 / backlog.
- Harness plug-ins for Claude Code / Codex that update the CLI and skill — Phase 2 (Phase 1 hook: `skill install` + `metadata.version`).
- Suggestions mode / in-app editing of any kind — Phase 3; content is edited outside and pushed.
- MCP server; human collaborators/sharing; algorithmic auto-merge of concurrent uploads (superseded by ADR 0002: `push` checks the file contains the head's changes, merging stays with the Member's tools); multi-Markdown packages; manual re-anchor of orphaned Threads and quote-search revival / moved-block detection; Version pruning/deletion; user-facing Version diff (CLI or web); cross-Document links as real Documents; full-text search; `list --open`; systemd unit / Windows service; LAN/Tailscale escape hatch; parser fuzzing; load/soak; comment edit history; W3C Web Annotation export tool — backlog.
- Commenting on regions within an image or PDF page — ruled out entirely (not even backlog); Asset Threads are whole-asset by content hash.
- Folders, tags, archive, unread/read state, reactions, comment attachments/embeds, per-Action rights, Action notes/retract, group mentions — deliberately not built and not backlogged.
- Quotas, rate limits, abuse controls, commercial model — public-service concerns, Phase 2.

## Further Notes

**Principles that decide ties:** (1) everything feels extremely fast; (2) extremely token-efficient for agents; (3) data completely portable — plain files in and out; (4) compute at the edge (ADR 0001).

**Measured targets from the prototypes (o200k tokens):** `status` 24 · `wait` event 12 · `mentions` (1 hit) 58 · `show DOC` (header + 5 open Threads) 245 · `thread show` 117 · `--help` 688 · SKILL.md 905 · sparse Sidecar of the same Document 3,146 (vs 5,326 verbose). These are the reference points, not hard budgets, except that the skill must stay ≤ 60 lines and agents must never need the Sidecar.

**Behavioural references:** Google Docs for span comments under edits (grow/shrink/refined, frozen quote label); GitHub PR review for line comments across pushes (orphaned ≙ outdated, freeze at last valid, never revive, map incrementally); Hypothesis for "quote is truth, orphans visible never silent"; W3C Web Annotation for semantics only.

**Phase roadmap (finalised at hand-off):**
- **Phase 1 — local (this spec):** Rust binary; CLI-started local web server (launchd-runnable) + CLI + web view/comment; Documents/Versions/Assets, Threads/Anchors, Actions, Vaults/Members/Rights, `status`/`wait`, skill file, five-layer test suite with perf budgets. Same-machine only.
- **Phase 2 — cloud:** hosting on Google Cloud (multi-tenant, sync from local — the events table with Tenant-wide `seq`, stable random ids, Member/Membership and Token-on-Member make this a transport problem, not a re-model), human sign-in (Google/passkey), CLI device-flow login, profiles gain `tenant`; iOS app with notifications (mention settings; browser + in-app on web); harness plug-ins for Claude Code and Codex where CLI and skill self-update via `skill install`; browser-side WASM push using the same crates; quotas/rate limits/commercial model.
- **Phase 3 — suggestions:** the mini-editor — an agent proposes a replacement for a span (a span Thread carrying a proposed replacement, applicable iff anchored), the human accepts/rejects → new Version.
- **Backlog:** Android, email notifications, human collaborators/sharing, MCP server, auto-merge of concurrent uploads (superseded by ADR 0002 — never merged by Annotary), multi-Markdown packages, manual re-anchor + quote-search revival/moved-block detection, Version pruning, user-facing diff, cross-Document links as Documents, full-text search, `list --open`, systemd/Windows service, LAN/Tailscale escape hatch, parser fuzzing, load/soak, comment edit history, W3C export tool.

**Handoff:** ticketed 2026-08-18 (16 tickets, ready-for-agent). Suggested cut lines for tickets: crates (renderer / diff+re-anchor / sidecar / store), server core (auth, docs, versions+blobs, threads, actions, events+wait), CLI verbs by group, web UI (document view, list, owner screens), daemon/first-run, skill file, test infrastructure (fixtures, goldens, spec-id checker, perf generator + runner).
