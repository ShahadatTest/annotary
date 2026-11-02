# This repo's own recipes, included by the Makefile. The repo owns this file; a standard
# upgrade never changes it. The Makefile's `make <name>` runs `project-<name>` here.
#
# annotary's entry points. `make check` is the definition of done (docs/PHASE1-DONE.md):
# everything this machine can run, perf included; `make check-ci` is the same without perf (for a
# machine without the release toolchain or Chromium, or `PERF=0 make check`). Works from a fresh
# shell with only ~/.cargo/bin on PATH.

CARGO ?= cargo
export PATH := $(HOME)/.cargo/bin:$(HOME)/.local/bin:$(PATH)
# ---- ticket 16: `PERF=0 make check` skips the perf suite (no release toolchain / Chromium) ----
PERF ?= 1
# Where cargo writes: CARGO_TARGET_DIR when set (the checks workflow keeps it outside the checkout).
TARGET_DIR := $(or $(CARGO_TARGET_DIR),$(CURDIR)/target)

.PHONY: project-build project-test project-lint project-ci project-deploy project-health deploy-test trim-test brand \
	e2e web goldens fmt fmt-check clippy spec-check wasm-check launchd-test token-report perf \
	check check-ci clean trim

project-build: trim
	$(CARGO) build

# Crate unit tests + in-process server integration tests (< 60 s), the spec-id checker included.
project-test: trim deploy-test trim-test
	$(CARGO) test --workspace

trim-test: ## Test scripts/trim-target.sh (TRIM_TARGET_DIR, the refused folders) on a temp copy
	@bash scripts/test_trim_target.sh

deploy-test: ## Test the deploy scripts (install, swap and rollback, build path and version, health, the settings)
	@bash deploy/test_install_linux.sh
	@bash deploy/test_build_release.sh
	@bash deploy/test_healthcheck.sh
	@bash deploy/test_deploy.sh
	@python3 deploy/test_release_tools.py -q

brand: ## Render brand/ from its masters (scripts/brand/render.py); commit the result
	uv run --no-project scripts/brand/render.py

project-lint: fmt-check clippy

# What the template's `ci` job runs besides the standard lint, gitleaks and the PR title: nothing.
# The Rust checks run in .github/workflows/checks.yml, which installs its own toolchain.
project-ci:
	@echo "Rust checks run in .github/workflows/checks.yml; nothing more for ci"

# The server runs under systemd as annotary.service (deploy/annotary.service). Where it runs is not
# in this repo: deploy/with_settings.py reads ~/.config/annotary/deploy.env (the keys are in
# docs/ops-health-check.md, Settings), and each target below refuses before it does anything when a
# key it needs is missing. `make deploy` runs from a clean checkout of origin/main: deploy/deploy.sh
# has the steps, and deploy/server_health.sh then runs deploy/healthcheck.sh on the host as root.
DEPLOY_KEYS := SERVER_SSH DEPLOY_SOURCE ANNOTARY_URL ANNOTARY_LINK_BASE
.PHONY: deploy-settings health-settings verify-settings
deploy-settings:
	@python3 deploy/with_settings.py --check $(DEPLOY_KEYS) --
health-settings:
	@python3 deploy/with_settings.py --check SERVER_SSH --
verify-settings:
	@python3 deploy/with_settings.py --check VERIFY_SNAPSHOT_GLOB VERIFY_UNIT --
deploy: deploy-settings
server-health: health-settings

project-deploy:
	@BUMP=$(BUMP) python3 deploy/with_settings.py $(DEPLOY_KEYS) -- deploy/deploy.sh

project-health:
	@python3 deploy/with_settings.py SERVER_SSH -- deploy/server_health.sh

e2e: trim ## trycmd goldens against a real spawned `annotary serve`, then the Playwright suite
	$(CARGO) test -p annotary --test e2e -- --ignored --nocapture
	@# CMD-22: verify_live's config-guard case needs node_modules, which only this job has in CI.
	$(CARGO) test -p annotary --test verify_live

goldens: ## Regenerate the trycmd goldens (TRYCMD=overwrite); review the diff
	TRYCMD=overwrite $(CARGO) test -p annotary --test e2e -- --ignored --nocapture

fmt: ## rustfmt the workspace
	$(CARGO) fmt --all

fmt-check: ## Check the workspace is rustfmt-clean
	$(CARGO) fmt --all -- --check

clippy: trim ## clippy with warnings denied
	$(CARGO) clippy --workspace --all-targets -- -D warnings

spec-check: ## The spec-id traceability checker (tests/SPEC.md vs `spec:` markers)
	$(CARGO) test -p annotary --test spec_ids

# ---- ticket 09: the crates that also ship to the browser must build for wasm32 (07 adds diff) ----
wasm-check: ## renderer + sidecar + diff build for wasm32-unknown-unknown
	$(CARGO) test -p annotary-renderer --test wasm -- --ignored --nocapture
# ---- end ticket 09 ----

# ---- ticket 13: drives the real launchctl under a temp HOME + private label; never part of check ----
launchd-test: ## The live launchctl round trip of `daemon` (macOS; temp HOME, private label)
	ANNOTARY_LAUNCHD_TESTS=1 $(CARGO) test -p annotary --test cli_daemon launchd_round_trip -- --nocapture
# ---- end ticket 13 ----

# ---- ticket 14: the token-count guard runs under `cargo test` (within ±20% of the prototype) and
# pins tests/token-report.md; this rewrites the report after a deliberate output change.
token-report: ## Regenerate tests/token-report.md; review the diff
	TOKEN_REPORT=overwrite $(CARGO) test -p annotary --test token_report
# ---- end ticket 14 ----

# ---- ticket 15: the perf suite. Always a *release* build (the budgets are about the product);
# `perf/global-setup.ts` spawns it, `perfgen seed` generates + seeds the fixture through the real
# binary, `perf/perf.spec.ts` measures with Playwright + CDP and asserts perf/BUDGETS.md. Meant for
# a dedicated Mac (perf/README.md), never a GitHub-hosted runner;
# part of `make check` since ticket 16 (`PERF=0` / `check-ci` leave it out). PERF_RUNS / PERF_SCALE /
# PERF_SEED tune a local run.
perf: trim node_modules/.stamp ## The perf suite against perf/BUDGETS.md (release build, median of 5 cold runs)
	$(CARGO) build --release -p annotary -p annotary-perfgen
	ANNOTARY_BIN=$(TARGET_DIR)/release/annotary PERFGEN_BIN=$(TARGET_DIR)/release/perfgen npx playwright test -c perf/playwright.config.ts
# ---- end ticket 15 ----

# ---- ticket 16: `check` = everything this machine can run (the definition of done), `check-ci`
# the same without the perf suite. ----
check-ci: trim fmt-check clippy test e2e spec-check wasm-check ## check minus the budgets suite, for a machine without the release toolchain or Chromium

check: ## The definition of done: fmt-check, clippy, test, e2e, spec-check, wasm-check, perf (PERF=0 skips perf)
ifeq ($(PERF),0)
check: check-ci
else
check: check-ci perf
endif
# ---- end ticket 16 ----

clean: ## cargo clean
	$(CARGO) clean

# Stale build output: cargo never deletes old artifacts (toolchain upgrades, dependency bumps) and a
# project left alone keeps target/ forever. scripts/trim-target.sh says when it removes it.
trim: ## Remove target/ when the toolchain changed, nothing was built for 14 days, or it passed 15 GB
	@scripts/trim-target.sh --quiet

# ---- ticket 10: Playwright (tests/web/) — browser tests against a real spawned `serve` with a seeded
# store. `npm ci` installs the pinned @playwright/test (its Chromium is the cached build, no
# download); `make e2e` runs this too (added as a prerequisite — the trycmd recipe above stays).
node_modules/.stamp: package.json package-lock.json
	npm ci --no-fund --no-audit
	@touch $@

web: node_modules/.stamp build ## The Playwright suite alone (tests/web/)
	npx playwright test

e2e: web
# ---- end ticket 10 ----

# ---- CMD-22: verify a build on a throwaway copy of the live store ----
# Copies the newest live snapshot (VERIFY_SNAPSHOT_GLOB, over ssh when SERVER_SSH is set), clears its
# passwords, serves it with this checkout's build on a loopback port, runs verify/scenarios with
# Playwright, deletes the copy and prints one JSON verdict. Production is never written.
# ARGS="--no-scenarios" or a scenario name (ARGS="-- cmd-20") narrow it; screenshots for the PR land
# in target/verify-live/.
.PHONY: verify-live
verify-live: verify-settings build node_modules/.stamp ## Test this build end to end on a throwaway copy of the live data
	@python3 deploy/with_settings.py VERIFY_SNAPSHOT_GLOB VERIFY_UNIT -- python3 deploy/verify_live.py $(ARGS)
