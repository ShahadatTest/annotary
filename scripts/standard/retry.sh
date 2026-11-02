#!/usr/bin/env bash
# Runs a command until it succeeds or the time runs out, for a health check straight after a
# deploy: a new version reaches Cloudflare's edge, or a restarted service comes up, some seconds
# after the deploy command returns. A real outage still fails, once the time is up.
# Usage: retry.sh <seconds> <every> <command...>
set -uo pipefail

wait=$1 every=$2
shift 2
deadline=$((SECONDS + wait))
while true; do
  "$@" && exit 0
  code=$?
  if (( SECONDS + every > deadline )); then
    echo "retry: still failing after ${wait}s: $*" >&2
    exit "$code"
  fi
  echo "retry: not healthy yet (exit $code); trying again in ${every}s" >&2
  sleep "$every"
done
