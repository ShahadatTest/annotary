#!/usr/bin/env bash
# Refuses to go on unless the working tree is clean (untracked files count) and HEAD is the
# commit at origin/main. Run before every deploy and TestFlight upload, so only merged code ships.
set -euo pipefail

if ! git fetch --quiet origin main; then
  echo "❌ could not fetch origin/main; refusing to ship without knowing what is merged" >&2
  exit 1
fi
dirty=$(git status --porcelain)
if [ -n "$dirty" ]; then
  echo "❌ the working tree is not clean; commit or remove these first:" >&2
  echo "$dirty" >&2
  exit 1
fi
head=$(git rev-parse HEAD)
main=$(git rev-parse origin/main)
if [ "$head" != "$main" ]; then
  echo "❌ HEAD ($head) is not origin/main ($main); ship only merged code: git switch --detach origin/main" >&2
  exit 1
fi
echo "✅ clean tree at origin/main ($(git rev-parse --short HEAD))"
