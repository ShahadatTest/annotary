# Rendered from the repo-standard template; never edited in this repo. The project's own
# recipes live in project.mk: each target here calls the project-<name> target there.
SHELL := /bin/bash -o pipefail
.DEFAULT_GOAL := help
BUMP ?= patch
# Every inner make in a recipe is $(SUBMAKE), never $(MAKE) by name: make runs a recipe line that
# names $(MAKE) even under -n, -t or -q, so `make -n testflight` would upload.
SUBMAKE = $(MAKE) --no-print-directory

include project.mk

# Tag series (V12): vX.Y.Z for a repo's single deliverable, otherwise one series for each. Set
# here, after project.mk, so the series always follow the repo's kinds.
DEPLOY_TAG_PREFIX := v

.PHONY: help build test lint ci standard-lint deploy server-health

help: ## List the commands
	@grep -hE '^[a-z][a-z0-9-]*:.*## ' $(MAKEFILE_LIST) | sort -u | awk -F':.*## ' '{printf "  %-18s %s\n", $$1, $$2}'

build: project-build ## Build the product (no tests)

test: project-test ## Run the unit tests

lint: standard-lint project-lint ## Run every lint check; fails on findings

ci: standard-lint project-ci ## What the CI job runs (no iOS parts)

standard-lint: ## Check the repo against the standard's file rules
	@python3 scripts/standard/lint.py

deploy: ## Deploy origin/main, check health, then tag and release it  [BUMP=minor|major]
	@scripts/standard/require_main.sh
	@$(SUBMAKE) project-deploy
	@$(SUBMAKE) server-health
	@python3 scripts/standard/release.py --prefix $(DEPLOY_TAG_PREFIX) --bump $(BUMP)

# A new deploy takes a while to reach Cloudflare's edge (or a restarted service to come up), so the
# health check is retried every HEALTH_EVERY seconds for up to HEALTH_WAIT before it fails.
HEALTH_WAIT ?= 60
HEALTH_EVERY ?= 5

server-health: ## Check the deployed service's health, retrying up to HEALTH_WAIT seconds (exit 0 healthy)
	@scripts/standard/retry.sh $(HEALTH_WAIT) $(HEALTH_EVERY) $(SUBMAKE) project-health
