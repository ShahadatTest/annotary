# annotary Phase 1 — launch plan

![architecture](img/architecture.png)

## Goal

Ship a local server + CLI + web viewer that humans and agents can use to comment on Markdown with equal standing.

## Order of work

1. Sidecar format and anchor engine — the core, testable in isolation.
2. Local server with content-addressed storage.
3. CLI (`push`, `pull`, `thread`, `act`, `wait`) and the skill file.
4. Web viewer and commenter.

We ship the CLI well before the web UI so agents can start using it early.

## Rollout

Dogfood on this repo for two weeks, then invite three external developers.

![CLI screenshot](img/cli-screenshot.png)

See [API notes](api-notes.md) for the endpoint list.
