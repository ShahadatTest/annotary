#!/usr/bin/env bash
# deploy/build_release.sh VERSION: builds the release binary stamped with VERSION and prints its
# path. deploy/deploy.sh runs it from the repo root.
#
# - The binary lands where cargo puts it: $CARGO_TARGET_DIR/release when that is set, else
#   target/release (CMD-3; the same rule as project.mk's TARGET_DIR), never a stale copy elsewhere.
# - ANNOTARY_VERSION is the version the binary reports (`option_env!` in src/lib.rs), so
#   `--version`, /api/v1/health and the skill's metadata.version say the tag the deploy is about
#   to get (CMD-2). The build is refused if the binary reports anything else.
#
# CARGO overrides the cargo command (deploy/test_build_release.sh uses a stub).
set -euo pipefail

version=${1:?usage: deploy/build_release.sh VERSION}
[[ $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "deploy: '$version' is not X.Y.Z" >&2; exit 1; }

ANNOTARY_VERSION=$version ${CARGO:-cargo} build --release --locked -p annotary >&2
new=${CARGO_TARGET_DIR:-target}/release/annotary
[ -x "$new" ] || { echo "deploy: cargo built no binary at $new" >&2; exit 1; }
said=$("$new" --version)
[ "$said" = "annotary $version" ] || { echo "deploy: $new says '$said', not 'annotary $version'" >&2; exit 1; }
echo "$new"
