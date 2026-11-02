# annotary Phase 1 — launch plan

![architecture](img/architecture.png)

## Goal

Ship a local server + CLI + web viewer that humans and agents can use to comment on Markdown with equal standing.

## Order of work

1. Sidecar format and anchor engine — the core, testable in isolation.
2. Local server with content-addressed storage.
3. CLI (`push`, `pull`, `thread`, `act`, `wait`) and the skill file.
4. Web viewer and commenter.

We ship the CLI before the web UI so agents can start using it early.

## Risks

- Re-anchoring edge cases: mitigated by the fixture table from Google Docs and GitHub.
- Token cost of the sidecar: agents should use the CLI, not read the file whole.

## Rollout

### Internal

Dogfood on this repo for two weeks.

### External

Then invite three external developers and collect feedback for a week.

![CLI screenshot](img/cli-screenshot.png)

See [API notes](api-notes.md) for the endpoint list.
