# Implementation notes

What each area's code is, where it lives and the decisions to keep, written by the agents that
built it, ticket by ticket. These were the root `CLAUDE.md` until 2026-10-18; they moved here so a
session loads only `AGENTS.md` and reads the file for the area it is about to touch. Each file is
"extend, don't rewrite": a change to an area updates its file in the same pull request.

| File | Area |
|---|---|
| [workspace-map.md](workspace-map.md) | the crates and what lives where, file by file, with what each ticket added |
| [commands.md](commands.md) | every `make` target and test command in detail |
| [testing.md](testing.md) | adding tests: in-process server, trycmd groups and seeds, Playwright, spec ids, perf, fixtures |
| [cli-conventions.md](cli-conventions.md) | exit codes, errors, output (TTY / non-TTY / `--json`), DOC arguments, global flags, adding a verb |
| [server-conventions.md](server-conventions.md) | routes, auth, status → exit mapping, long-lived routes, `serve` |
| [documents-versions.md](documents-versions.md) | ticket 02: Documents, Versions, blobs, the Sidecar, the push flow |
| [assets.md](assets.md) | ticket 03: Asset types, caps, manifest, scan rules, media, `/a/{hash}`, `pull --assets` |
| [threads.md](threads.md) | ticket 04: Threads and Comments |
| [actions.md](actions.md) | ticket 05: Actions and status |
| [access-model.md](access-model.md) | ticket 06: Rights, Grants, Tokens, password, sessions, web auth |
| [reanchoring.md](reanchoring.md) | ticket 07: re-anchoring across Versions |
| [events-wait-live.md](events-wait-live.md) | ticket 08: the events log, `wait`, the live feed, bounded shutdown |
| [rendering.md](rendering.md) | ticket 09: the renderer and its HTML contract |
| [web-document-view.md](web-document-view.md) | ticket 10 (+15): the Document view and list pages |
| [web-commenting.md](web-commenting.md) | ticket 11: commenting, Actions, keyboard, live updates in the browser |
| [web-owner-screens.md](web-owner-screens.md) | ticket 12: the Members and Settings screens, login (since CMD-23 the Settings screen's Server group) |
| [member-settings.md](member-settings.md) | CMD-23: Devices, the settings catalog, stamps and the server clock, the `/api/v1/me/settings` and `/api/v1/me/devices` routes, the per-Member live-feed rule |
| [web-settings.md](web-settings.md) | CMD-23: the Settings screen for every Member (sidebar, Applying to, Mixed, the chain, Copy from another device, Profile, Devices) |
| [daemon-export-housekeeping.md](daemon-export-housekeeping.md) | ticket 13: `daemon`, `export`, housekeeping |
| [skill-help-golden-flows.md](skill-help-golden-flows.md) | ticket 14: the agent skill, `--help`, the golden flows |
| [perf-suite.md](perf-suite.md) | ticket 15: the perf suite and budgets |
| [phase1-closure.md](phase1-closure.md) | ticket 16: the definition of done, traceability, CI |
| [concurrent-edits.md](concurrent-edits.md) | ticket 17: concurrent edits by content |
| [ios-prerequisites.md](ios-prerequisites.md) | ticket 18: remote bind, links, pairing, titles |
| [releases.md](releases.md) | the release pipeline (cargo-dist and its hand edits), the Homebrew tap, the update notice |

The spec is [`../phase1/spec.md`](../phase1/spec.md). "Ticket N" and `CMD-n` in these notes are labels
from the project's build history and its old tracker; the items themselves are not public. New work is
filed in GitHub Issues.
