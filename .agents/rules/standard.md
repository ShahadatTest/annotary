# Repository standard

This file is rendered from the repo-standard template and is never edited in this repo; a change
to it arrives through a standard upgrade. The repo's own rules are in `AGENTS.md`.

## Commands

- The `Makefile` is the command surface; never type a raw build or test line from memory. Its
  targets: `make build`, `make test`, `make lint`, `make deploy`, `make server-health`; `make help` lists them all.
- The project's own recipes go in `project.mk`. Never edit `Makefile`, `scripts/standard/` or
  this file; they change only through a standard upgrade.
- `make lint` fails while `.agents/standard.yml` records a version behind the latest
  repo-standard tag: the upgrade comes first, in its own pull request, before other changes.
  The template repo is private, so where it cannot be read (a contributor's clone, the hosted
  CI runner) the lint only warns that the version was not checked.

## Merge gate

A pull request merges when:
- `make lint test` passes (a change only to docs or agent instructions needs only `make lint`);
- the `ci` check has passed on the pull request;
- for a server change, you have run `make deploy` and `make server-health` passes.

A change touching only docs or agent instructions, made after your last local checks passed, may
rely on the `ci` check's lint instead of running `make lint` again.

A code change is reviewed before it merges by following the `review-gate` skill, which names the
reviewer (no other model reviews, and with no reviewer available the change waits), when it
touches a server, authentication, secrets handling, money, a data migration, CI or a runner, a
hook or guard, or more than about 200 changed lines; for any other code change the author decides
and logs it in the PR body.

## Repository layout

- The root holds only `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `DESIGN.md`, `README.md`,
  `CONTRIBUTING.md`, `SECURITY.md`, `CODE_OF_CONDUCT.md`, `LICENSE-MIT`, `LICENSE-APACHE`,
  `Makefile`, `project.mk`, the package manifest and dotfiles. Durable docs
  (why, decisions in `docs/decisions.md`, external contracts, ops playbooks) go in `docs/`.
  Working plans go in `docs/plans/` while active and move to `docs/archive/` in the PR that
  finishes the work; active docs do not link into `docs/archive/`, and you never read or follow
  `docs/archive/` unless asked about history.
- Keep each instruction, rule or skill file under 24,000 bytes. Long procedures go in a skill
  under `.agents/skills/`, history in `docs/`.
- Committed shell scripts never embed Python (`python3 -c`, heredocs); put it in a sibling `.py`
  file. Python the repo needs is pinned with `uv` (`.python-version`, `uv run`).
- The shipped version comes from the git tag at build time. Never write a version number into a
  file by hand; `0.0.0` placeholders are fine.
- This repo is public, so its CI runs on GitHub-hosted runners (`ubuntu-latest`,
  `macos-latest`). Never register a self-hosted runner to it or target one: a fork's pull
  request could run its code there.
- A job may use GitHub's cache (`actions/cache`, a setup action's `cache:` option); a hosted
  runner starts empty.

## Commits

- The commit-msg hook in `.githooks/` refuses a subject not in Conventional Commits form, and the `ci` check refuses such a pull request title, which becomes the
  squash-merge subject. Reword it; never bypass the hook.

## Secrets

- The pre-commit hook in `.githooks/` runs gitleaks on staged changes, and the pre-push hook on
  the commits each push sends. If one refuses, take the secret out of the change (rewrite the
  branch if it is already committed); never bypass the hooks.
- No secret inside a systemd unit: settings and secrets go in the root-only
  `/etc/annotary/env`, which the unit reads with `EnvironmentFile=`.

## Monitoring

- Adding, renaming or removing anything that runs (service, LaunchAgent, systemd unit, runner,
  public hostname, scheduled job) updates the maintainer's monitoring in the same change.

## Tracker

"Bug", "ticket" and "issue" all mean an issue in this repo's GitHub Issues, whether a defect or
a feature; do not ask which. Outside reports arrive there too, through the forms in
`.github/ISSUE_TEMPLATE/`; questions go to Discussions, and security reports to private
vulnerability reporting (`SECURITY.md`), never a public issue. File and close issues with
`gh issue`; a pull request that fixes one says "Fixes #<number>" in its body.

## Server

- The server runs under systemd on the host `SERVER_SSH` names: the unit `annotary.service`,
  installed with sudo. A merged change deploys with `make deploy`, which refuses a dirty tree or a
  commit other than `origin/main`, checks health and tags the release.
- The service runs as its own system user `annotary`: no login shell, no sudo, not in the
  `docker` group. It runs as a login user only when its job is to read that user's own files,
  with the same hardening and an explicit `ReadWritePaths=` list, and the reason in
  `docs/decisions.md`.
- It logs to journald (`journalctl -u annotary.service`). A log file only for an HTTP access log,
  under `/var/log/annotary/` with a logrotate rule.
- A firewall rule only when something outside the box must reach the service: from `10.0.0.0/24`
  or on `tailscale0`, added by the install step after it checks that ufw is active. A service behind
  a Cloudflare Tunnel needs none.
- `deploy/healthcheck.sh` is read-only and prints one JSON verdict (exit 0, 1 or 2, with
  `problems[]`). Each playbook in `docs/ops-health-check.md` ends at the word **stop**.
- A pull request that changes how production behaves or is watched says "Monitoring changes:"
  and what changed, or "none", in its body.
