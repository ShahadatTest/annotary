#!/usr/bin/env bash
# Tests the settings (deploy/with_settings.py), deploy/deploy.sh and deploy/server_health.sh with a
# stub ssh that logs what would run on the host; `make deploy-test` runs it. Runs anywhere, in a temp
# dir; no host is contacted.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
repo=$(dirname "$here")
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0

check() { # check <description> <command...>
  if "${@:2}"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi
}
grep_q() { grep -q -- "$1" <<< "$2"; }

mkdir -p "$tmp/bin"
# The stub logs the remote command and the size of what came over stdin, so every call gives it a
# stdin of its own: an inherited one that stays open would hang it. A verdict for the health
# check comes from STUB_VERDICT; build_release answers with where the binary is.
cat > "$tmp/bin/ssh" <<'SSH'
#!/bin/bash
while [ "$1" = -o ]; do shift 2; done
host=$1
shift
in=$(cat)
printf 'ssh %s: %s [stdin %s bytes]\n' "$host" "$*" "${#in}" >> "$SSH_LOG"
[ -n "${STUB_FAIL:-}" ] && exit 255
case "$*" in
  *healthcheck* | *'bash -s') [ "${STUB_HEALTH:-0}" = 1 ] && { echo "${STUB_VERDICT:-}"; exit "${STUB_RC:-0}"; } ;;
  *build_release*) echo /built/annotary ;;
esac
exit 0
SSH
chmod 755 "$tmp/bin/ssh"
export SSH_LOG=$tmp/ssh.log

# ---- with_settings.py
ws() { # ws <env assignments...> -- <args>: its stdout and stderr, exit code in $rc
  local envs=()
  while [ "$1" != -- ]; do envs+=("$1"); shift; done
  shift
  out=$(env -i PATH="$PATH" HOME="$tmp/home" "${envs[@]+"${envs[@]}"}" python3 "$here/with_settings.py" "$@" 2>&1)
  rc=$?
}
mkdir -p "$tmp/home/.config/annotary"
file=$tmp/home/.config/annotary/deploy.env

ws -- SERVER_SSH -- /bin/echo ran
check "no settings file: a required key refuses" test "$rc" -eq 2
check "no settings file: it names the key" grep_q 'SERVER_SSH not set' "$out"
check "no settings file: it names the file" grep_q '~/.config/annotary/deploy.env' "$out"
check "no settings file: the command never ran" test "${out#*ran}" = "$out"

printf '# a comment\n\nSERVER_SSH=from-file\nANNOTARY_URL=https://x.example.com\n' > "$file"
ws -- SERVER_SSH -- /usr/bin/printenv SERVER_SSH
check "the file gives a required key" test "$rc:$out" = "0:from-file"
ws SERVER_SSH=from-env -- SERVER_SSH -- /usr/bin/printenv SERVER_SSH
check "the environment beats the file" test "$out" = from-env
ws SERVER_SSH= -- SERVER_SSH -- /usr/bin/printenv SERVER_SSH
check "an empty value is not set" test "$rc" -eq 2
ws -- SERVER_SSH DEPLOY_SOURCE -- /usr/bin/true
check "a missing key among others is named" grep_q '^annotary: DEPLOY_SOURCE not set' "$out"
ws -- --check SERVER_SSH --
check "--check with the key passes" test "$rc" -eq 0
ws -- --check DEPLOY_SOURCE --
check "--check without the key fails" test "$rc" -eq 2
printf 'SERVER_SHH=typo\n' > "$file"
ws -- -- /usr/bin/true
check "an unknown key is refused" test "$rc" -eq 2
check "an unknown key is named with its line" grep_q 'deploy.env:1: unknown setting SERVER_SHH' "$out"
printf 'export SERVER_SSH=x\n' > "$file"
ws -- -- /usr/bin/true
check "a line that is not KEY=value is refused" test "$rc" -eq 2
ws ANNOTARY_DEPLOY_ENV="$tmp/other.env" -- --check SERVER_SSH --
check "ANNOTARY_DEPLOY_ENV names another file" grep_q "$tmp/other.env" "$out"
rm -f "$file"

# ---- a small repo for deploy.sh: its script, HEAD and a tag or none
r=$tmp/repo
mkdir -p "$r/deploy" "$r/scripts/standard"
cp "$here/deploy.sh" "$here/install-linux.sh" "$here/next_version.py" "$here/server_health.sh" "$here/healthcheck.sh" "$r/deploy/"
cp "$repo/scripts/standard/release.py" "$r/scripts/standard/"
g() { git -C "$r" -c user.name=t -c user.email=t@example.com "$@"; }
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_SYSTEM=/dev/null
g init -q
g add .
g commit -qm first
git init -q --bare "$tmp/origin.git"
g remote add origin "$tmp/origin.git"
g push -q origin HEAD 2>/dev/null

deploy() { # deploy <ENV=value...>
  : > "$SSH_LOG"
  out=$(env PATH="$tmp/bin:$PATH" SERVER_SSH=the-host ANNOTARY_URL=https://x.example.com ANNOTARY_LINK_BASE=none "$@" bash "$r/deploy/deploy.sh" 2>&1 < /dev/null)
  rc=$?
  log=$(cat "$SSH_LOG")
}

deploy DEPLOY_SOURCE=release
check "release with no tag on HEAD refuses" test "$rc" -eq 1
check "release with no tag says what to do" grep_q 'push the tag' "$out"
check "release with no tag contacts nothing" test -z "$log"
g tag v1.2.3
deploy DEPLOY_SOURCE=release
check "release with a tag installs" test "$rc" -eq 0
check "release runs the installer over stdin as root with the tag" grep_q 'ssh the-host: sudo -n bash -s -- v1.2.3 --url https://x.example.com --link-base none' "$log"
check "release sends the installer" test "${log##*stdin }" != "0 bytes]"
check "release builds nothing" test "${log#*build_release}" = "$log"
g tag -d v1.2.3 >/dev/null

deploy DEPLOY_SOURCE=build
check "build installs" test "$rc" -eq 0
check "build builds on the host" grep_q 'deploy/build_release.sh' "$log"
check "build installs the built binary with the settings" \
  grep_q 'install-linux.sh --binary /built/annotary --src ~/.cache/annotary-deploy/src --url https://x.example.com --link-base none' "$log"

deploy DEPLOY_SOURCE=build "ANNOTARY_OWNER_NAME=Rowan Vale" ANNOTARY_OWNER_HANDLE=rowan ANNOTARY_TAILSCALE_SERVICE=svc:annotary
check "owner and Service settings reach the installer, quoted" \
  grep_q '--owner-handle rowan --owner-name Rowan\\ Vale --tailscale-service svc:annotary' "$log"
deploy DEPLOY_SOURCE=sideways
check "an unknown source refuses" test "$rc" -eq 1
deploy DEPLOY_SOURCE=release SERVER_SSH=
check "deploy.sh alone refuses without SERVER_SSH" test "$rc" -ne 0
deploy DEPLOY_SOURCE=release ANNOTARY_URL=
check "deploy.sh alone refuses without the URL" test "$rc" -ne 0
check "deploy.sh without the URL contacts nothing" test -z "$log"

# ---- server_health.sh: the verdict decides, and the settings' URL goes to the check
health() { # health <ENV=value...>
  : > "$SSH_LOG"
  out=$(env PATH="$tmp/bin:$PATH" STUB_HEALTH=1 SERVER_SSH=the-host "$@" bash "$r/deploy/server_health.sh" 2>&1 < /dev/null)
  rc=$?
  log=$(cat "$SSH_LOG")
}
health STUB_VERDICT='{"status":"healthy","problems":[]}' STUB_RC=0 ANNOTARY_URL=https://x.example.com
check "healthy passes" test "$rc" -eq 0
check "the settings' URL goes to the check" grep_q 'sudo -n env ANNOTARY_EXPECT_URL=https://x.example.com bash -s' "$log"
check "the check goes over stdin" test "${log##*stdin }" != "0 bytes]"
health STUB_VERDICT='{"status":"warning","problems":[]}' STUB_RC=1
check "a warning passes" test "$rc" -eq 0
health STUB_VERDICT='{"status":"critical","problems":[]}' STUB_RC=2
check "critical fails" test "$rc" -ne 0
health STUB_VERDICT= STUB_RC=1
check "no verdict fails, even with exit 1" test "$rc" -eq 2
check "no verdict says so" grep_q 'no verdict from the-host' "$out"

# ---- project.mk: each target refuses before it does anything, naming the key
makes() { # makes <target...>: from the repo, with no settings
  out=$(env -i PATH="$PATH" HOME="$tmp/home" make -C "$repo" --no-print-directory "$@" 2>&1)
  rc=$?
}
makes health-settings
check "make server-health's check refuses without settings" test "$rc" -ne 0
check "make server-health's check names the key" grep_q 'SERVER_SSH not set' "$out"
makes deploy-settings
check "make deploy's check names the keys" grep_q 'SERVER_SSH and DEPLOY_SOURCE and ANNOTARY_URL and ANNOTARY_LINK_BASE not set' "$out"
makes verify-settings
check "make verify-live's check names the keys" grep_q 'VERIFY_SNAPSHOT_GLOB and VERIFY_UNIT not set' "$out"

if [ "$fails" -gt 0 ]; then echo "$fails failed"; exit 1; fi
echo "all passed"
