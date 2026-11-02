#!/usr/bin/env bash
# `make server-health` (project-health), run through deploy/with_settings.py: feeds
# deploy/healthcheck.sh to `ssh $SERVER_SSH` as root, with the URL the settings say the server has.
#
# The deploy gate passes on healthy (0) or warning (1): a warning (the public URL does not answer,
# no password yet, disk under 10 GB) says nothing about the binary just deployed and would leave
# every deploy untagged. A critical verdict (2) or no verdict fails. The installed copy keeps the
# script's 0/1/2. Exit 1 also means ssh or sudo failed, so a verdict line is required before 1
# counts as a warning.
set -uo pipefail
cd "$(dirname "$0")/.."
host=${SERVER_SSH:?SERVER_SSH is not set (docs/ops-health-check.md, Settings)}

expect=
[ -z "${ANNOTARY_URL:-}" ] || expect=$(printf ' ANNOTARY_EXPECT_URL=%q' "$ANNOTARY_URL")
# shellcheck disable=SC2029  # the setting is meant to expand here
out=$(ssh -o ConnectTimeout=10 -o BatchMode=yes "$host" "sudo -n env$expect bash -s" < deploy/healthcheck.sh)
rc=$?
echo "$out"
case "$out" in
  *'{"status":'*) [ "$rc" -le 1 ] ;;
  *)
    echo "server-health: no verdict from $host (exit $rc)" >&2
    exit 2
    ;;
esac
