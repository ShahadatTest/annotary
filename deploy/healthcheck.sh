#!/usr/bin/env bash
# Read-only health check for the deployed server. Prints one JSON verdict and exits 0 (healthy),
# 1 (warning) or 2 (critical). It observes only: it never restarts, writes or deletes anything.
# It runs on the server's host as root (the store is the `annotary` user's, mode 750, and the env
# file root's): `make server-health` feeds it to `ssh "$SERVER_SSH" 'sudo -n bash -s'`, and the
# deploy installs a copy at /opt/annotary/app/healthcheck.sh. Each problem names the playbook in
# docs/ops-health-check.md that handles it. ANNOTARY_DATA and ANNOTARY_ENV_FILE override the data
# dir and the env file (the tests); ANNOTARY_EXPECT_URL is the URL the maintainer's settings say
# this server has (`make server-health` passes it).
set -uo pipefail

problems=()

# problem <warning|critical> <check> <message> <playbook>
problem() { problems+=("$1|$2|$3|$4"); }

json_escape() { local s=${1//\\/\\\\}; s=${s//\"/\\\"}; printf '%s' "$s"; }

# ── Checks ─────────────────────────────────────────────────────────────────────────────
# annotary runs as the systemd unit annotary.service (deploy/annotary.service): `serve --data-dir
# /srv/annotary/data --port 7377 --bind 127.0.0.1`, with its URL from the env file.
unit=annotary.service
port=7377
data=${ANNOTARY_DATA:-/srv/annotary/data}
env_file=${ANNOTARY_ENV_FILE:-/etc/annotary/env}

# The unit is running and comes back at boot.
if ! command -v systemctl >/dev/null 2>&1; then
  problem critical service "systemctl is not here: run this on the server's host" P1
else
  state=$(systemctl is-active "$unit" 2>/dev/null)
  [ "$state" = active ] || problem critical service "$unit is ${state:-unknown}, not active" P1
  enabled=$(systemctl is-enabled "$unit" 2>/dev/null)
  [ "$enabled" = enabled ] || problem warning service "$unit is ${enabled:-unknown}, not enabled: it will not start at boot" P1
fi

# The API answers on loopback.
health=$(curl -sS --max-time 5 "http://127.0.0.1:$port/api/v1/health" 2>/dev/null || true)
if ! grep -q '"ok":true' <<< "$health"; then
  problem critical api "http://127.0.0.1:$port/api/v1/health does not answer ok" P2
else
  # Setup ran (deploy/install-linux.sh does it before every start).
  grep -q '"set_up":true' <<< "$health" || problem critical api "the store is not set up" P2
  # Until the owner sets a password, no browser off this machine can sign in (CMD-17).
  grep -q '"password_set":true' <<< "$health" ||
    problem warning password "no local password is set: browsers on other devices cannot sign in" P6
fi

# The URL the server should have is in the env file: the key must be there (`none` says "no URL" on
# purpose). The store remembers the last --url, so the health answer alone cannot show a lost
# setting: that is why this reads the file. The key missing or empty (the unit would pass `--url ""`,
# which clears the store's), or the answer or the maintainer's settings disagreeing with it, is
# critical: the deploy gate fails on it.
norm_url() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | sed 's:/*$::'; }
public=$(grep -o '"public_url":"[^"]*"' <<< "$health" | cut -d'"' -f4 || true)
if [ ! -r "$env_file" ]; then
  problem critical public "$env_file is missing or not readable (run as root): the unit needs its URL from there" P3
elif ! grep -q '^ANNOTARY_URL=' "$env_file"; then
  problem critical public "$env_file has no ANNOTARY_URL: the server has lost its public URL" P3
elif setting=$(norm_url "$(sed -n 's/^ANNOTARY_URL=//p' "$env_file" | tail -n 1 | tr -d ' \t"'"'")") && [ -z "$setting" ]; then
  # The last line is the one systemd takes, without quotes or blanks.
  problem critical public "$env_file has an empty ANNOTARY_URL: a start clears the server's public URL" P3
else
  if [ -n "${ANNOTARY_EXPECT_URL:-}" ] && [ "$(norm_url "$ANNOTARY_EXPECT_URL")" != "$setting" ]; then
    problem critical public "$env_file says ANNOTARY_URL=$setting, the deploy settings say $ANNOTARY_EXPECT_URL" P3
  fi
  if [ -n "$health" ] && [ "$setting" != none ] && [ "$(norm_url "$public")" != "$setting" ]; then
    problem critical public "the server answers with public URL '${public}', $env_file says $setting: restart it" P3
  fi
fi

# That URL (what links and paired devices use) answers too.
if [ -n "$public" ] && ! curl -sS --max-time 10 "$public/api/v1/health" 2>/dev/null | grep -q '"ok":true'; then
  problem warning public "$public/api/v1/health does not answer ok" P3
fi

# Room for the store (Documents, blobs, rendered HTML) on its volume.
if [ -d "$data" ]; then
  free_kb=$(df -Pk "$data" | awk 'NR==2 {print $4}')
  if [ "${free_kb:-0}" -lt $((2 * 1024 * 1024)) ]; then
    problem critical disk "$((free_kb / 1024)) MB free for $data" P4
  elif [ "$free_kb" -lt $((10 * 1024 * 1024)) ]; then
    problem warning disk "$((free_kb / 1024 / 1024)) GB free for $data" P4
  fi
else
  problem critical disk "the data dir $data is missing (or not readable: run as root)" P2
fi

# The newest consistent snapshot of the store (CMD-1): the server writes one at start and every
# 15 minutes (only touching the newest when nothing changed), so a backup always copies a whole
# one. Its mtime is when the server last checked the store.
snapdir=$data/snapshots
if [ -d "$data/store" ]; then
  # shellcheck disable=SC2012  # names are annotary-<stamp>-s<seq>.tar, no odd characters
  newest=$(ls -t "$snapdir"/annotary-*.tar 2>/dev/null | head -1)
  if [ -z "$newest" ]; then
    problem warning snapshot "no store snapshot in $snapdir" P5
  else
    mtime=$(stat -c %Y "$newest" 2>/dev/null || stat -f %m "$newest") # GNU, else BSD (macOS)
    age=$(( $(date +%s) - mtime ))
    if [ "$age" -gt 86400 ]; then
      problem critical snapshot "the newest store snapshot is $((age / 3600)) h old ($newest)" P5
    elif [ "$age" -gt 3600 ]; then
      problem warning snapshot "the newest store snapshot is $((age / 60)) min old ($newest)" P5
    fi
  fi
fi

# ── Verdict ────────────────────────────────────────────────────────────────────────────
status=healthy; code=0
for p in "${problems[@]+"${problems[@]}"}"; do
  case "${p%%|*}" in
    critical) status=critical; code=2 ;;
    warning) [ "$code" -lt 1 ] && { status=warning; code=1; } ;;
  esac
done
printf '{"status":"%s","problems":[' "$status"
sep=""
for p in "${problems[@]+"${problems[@]}"}"; do
  IFS='|' read -r sev check msg playbook <<< "$p"
  printf '%s{"severity":"%s","check":"%s","message":"%s","playbook":"%s"}' "$sep" \
    "$sev" "$(json_escape "$check")" "$(json_escape "$msg")" "$(json_escape "$playbook")"
  sep=","
done
printf ']}\n'
exit "$code"
