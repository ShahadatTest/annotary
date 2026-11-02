# Contributing to Annotary

Thanks for helping. Bugs and feature requests go to
[Issues](https://github.com/josh-andrews-dev/annotary/issues) (the forms ask for what is
needed), questions to [Discussions](https://github.com/josh-andrews-dev/annotary/discussions),
and security problems to [private vulnerability reporting](https://github.com/josh-andrews-dev/annotary/security/policy),
never a public issue. Everyone taking part follows the
[Code of Conduct](https://github.com/josh-andrews-dev/annotary/blob/main/CODE_OF_CONDUCT.md).

## Set up

You need:

- [rustup](https://rustup.rs) with the stable toolchain, and the `wasm32-unknown-unknown` target
  for `make wasm-check` (`rustup target add wasm32-unknown-unknown`);
- [Node](https://nodejs.org) LTS for the web tests (`make e2e` runs `npm ci`; then
  `npx playwright install chromium` once);
- [uv](https://docs.astral.sh/uv/) for `make test` (it checks the icons with `scripts/brand/render.py`);
- [gitleaks](https://github.com/gitleaks/gitleaks) for the git hooks.

macOS and Linux are supported; Windows is not.

`make help` lists the commands. Turn on the repo's git hooks once in each clone:

    git config core.hooksPath .githooks

The pre-commit and pre-push hooks scan for secrets with gitleaks.

## Find your way around

- [`docs/CONTEXT.md`](docs/CONTEXT.md) is the glossary. Use its words (Document, Version, Vault, Thread,
  Comment, Action, Member, Sidecar, Asset, Anchor) in code, docs and issues.
- [`docs/dev/README.md`](docs/dev/README.md) has implementation notes per area. Read the file for the
  area you are changing, and update it in the same pull request.
- [`docs/adr/`](docs/adr/) and [`docs/decisions.md`](docs/decisions.md) say why things are as they are.
- [`AGENTS.md`](AGENTS.md) has the full rules. Agents read it, and it is worth a read before a large
  change. If you work with a coding agent, point it there.
- `CMD-n` and "ticket N" in comments are labels from the project's old tracker and its build history;
  the items are not public.

## The checks

- `make lint test` before you open a pull request: formatting, clippy with warnings denied, and the
  unit and in-process server tests (under a minute).
- `make check` is the definition of done ([`docs/PHASE1-DONE.md`](docs/PHASE1-DONE.md)). `PERF=0 make
  check` leaves out the perf suite, which needs a release build and a quiet machine; CI runs the rest
  of it on your pull request. Run `make perf` if you touch the web view, the renderer or the store's
  read path. Budgets live in [`perf/BUDGETS.md`](perf/BUDGETS.md) and change only by the rule there.

## The rules that catch people out

- **Goldens.** `make goldens` regenerates the CLI's expected output in `tests/e2e/`. Read the diff: never
  regenerate a golden just to make a test pass. `tests/e2e/95-agent-flow/agent.trycmd` is the end-to-end
  acceptance test, so say in the pull request why a line of it changed.
- **Spec ids.** A behaviour change adds ids to [`tests/SPEC.md`](tests/SPEC.md) first (with the story numbers
  in its `## Stories` section), then tests that name them in a `// spec: S12-03` comment. `cargo test -p
  annotary --test spec_ids` checks it. Never put a marker in implementation code.
- **The agent skill.** `crates/annotary/skill/SKILL.md` is what agents are told, at most 60 lines. Changing a
  verb's output or a flag it names changes the skill, the goldens and `tests/token-report.md` (`make
  token-report`) together.
- **A new CLI verb** needs a TTY and a non-TTY case, a line in `tests/e2e/10-readonly/json-fields.trycmd`, every
  exit path, and an entry in `cli/help.rs`. **A new route** needs a row in `tests/access_api.rs`'s
  `endpoints()`. [`docs/dev/cli-conventions.md`](docs/dev/cli-conventions.md) has the details.
- **The web pages.** Every id and class starts with `cd-`, Thread cards have one renderer (Rust, on the server),
  and there is no `position: fixed`. [`DESIGN.md`](DESIGN.md) has the rest.
- **No personal details in tests or docs.** Use `fixtures/corpus/` for realistic Documents, and `example.com`,
  `example.ts.net` and `@rowan` in examples; a test refuses real email addresses, tailnet names and home paths.

## Pull requests

- Branch from `main`, one change per pull request.
- Commit messages and the pull request title follow
  [Conventional Commits](https://www.conventionalcommits.org/) (`fix: …`, `feat(cli): …`); the
  `commit-msg` hook and CI check them. A pull request is squash-merged with its title as the
  subject.
- A bug fix comes with a test that fails without the fix.
- A visible change has a screenshot in the pull request.
- The README's screenshot comes from the made-up corpus: `node scripts/docs_screenshots.mjs` (after `make build` and `npm ci`) writes it again. Never use real Documents.

## License

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in
this project by you, as defined in the Apache-2.0 license, shall be dual licensed under the
[MIT license](LICENSE-MIT) and the [Apache License, Version 2.0](LICENSE-APACHE), without any
additional terms or conditions.
