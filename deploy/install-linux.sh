#!/usr/bin/env bash
# Installs or updates annotary on a Linux host under systemd, as root:
#
#   install-linux.sh vX.Y.Z [SETTINGS]                 the published release (what everyone runs)
#   install-linux.sh --binary FILE --src DIR [SETTINGS]  a binary built here, with DIR/deploy/ holding
#                                                      the unit and the health check (DEPLOY_SOURCE=build)
#
#   --url URL|none         the URL other devices use; required until /etc/annotary/env has one
#   --link-base URL|none   the link relay base (default none)
#   --accept-new-url       on a host with a store, let --url and --link-base differ from what the
#                          store holds when they fill in a key the env file lacks
#   --owner-handle NAME, --owner-name NAME    the owner first-run setup makes (default: the account)
#   --tailscale-service svc:NAME              serve this Tailscale Service to the server (default: none)
#
# vX.Y.Z downloads annotary-<target>.tar.gz and its .sha256 over HTTPS only, redirects too, from the
# GitHub release (ANNOTARY_RELEASE_BASE overrides where) and refuses to go on unless the SHA-256
# matches, every member of the tarball is in the release's exact layout (read before it is unpacked,
# no link, no path outside it) and the binary reports X.Y.Z; nothing on the host has changed by then.
# The tarball's owners and modes are not kept when it is unpacked. The new binary is never run as root
# before it is installed: its --version and --print-schema-version run as `nobody`, from a copy in
# a folder of its own under /tmp, because the file it came from may be somewhere only root and the
# build's user can read. Idempotent:
#
#   0. a key the env file lacks (or holds empty, which is the same to systemd) is filled in below; on
#      a host with a store the value must match what the running server's health answer says the
#      store holds, or the script refuses with nothing changed, so a typo in the settings cannot
#      replace a live URL. --accept-new-url, or editing the env file, is how to change it on purpose;
#   1. the system user `annotary`; /opt/annotary/app (root, the program),
#      /srv/annotary/data (the user's, 750: the store, snapshots), /etc/annotary/env
#      (root, 600: the unit's EnvironmentFile). --url and --link-base are added to the env file only
#      when it lacks them: a value already there is the host's and is never overwritten;
#   2. the installed binary is kept as annotary.prev, the installed unit as
#      /opt/annotary/app/annotary.service.prev and the health check's copy as healthcheck.sh.prev; the
#      new binary is renamed over annotary; the unit and the health check's copy are installed;
#   3. the service is stopped. When the new binary runs a newer store schema than the store's
#      (`serve --print-schema-version` against store/meta.json's `schema_version`, CMD-23), the
#      store is first exported, by the previous binary, to snapshots/pre-schema-N.tar (N = the
#      store's schema; 600, the user's; an older file of that name is kept as
#      pre-schema-N.<UTC stamp>.tar; never deleted here, never rotated by the server). Then
#      `serve --setup-only` runs as `annotary` (migrations; first-run setup on an empty store;
#      otherwise a no-op). Its stdout, which on the first run holds the owner Token, is discarded:
#      never shown, never in the journal;
#   4. the unit is enabled and started, and must answer /api/v1/health on 127.0.0.1:7377 with
#      the new version and the env file's URL within 30 s; otherwise annotary.prev, the previous
#      unit and the previous health check are put back and started, and the script exits 1 (so does
#      a failed setup or export,
#      which never starts the unit). When this run wrote pre-schema-N.tar, the previous binary
#      cannot open the migrated store (SchemaTooNew), so the rollback first moves store/ aside as
#      store.failed-<UTC stamp> (kept, never deleted) and untars the export into a fresh store/
#      (750, the user's) before it starts annotary.prev (docs/ops-health-check.md P7);
#   5. with --tailscale-service, that Service (https on 443) is pointed at 127.0.0.1:7377, once
#      this host is a tagged node; until then it says so and goes on.
#
# No firewall rule: nothing listens beyond loopback. deploy/test_install_linux.sh runs it with
# stubs: CD_ROOT prefixes every path, SYSTEMCTL, RUNUSER, CURL, TAR, TAILSCALE replace the commands,
# and CD_WAIT_TRIES (default 60, half a second each) bounds the wait for health.
set -euo pipefail

usage() {
  echo "usage: install-linux.sh vX.Y.Z | --binary FILE --src DIR  [--url URL|none] [--link-base URL|none]" >&2
  echo "       [--accept-new-url] [--owner-handle NAME] [--owner-name NAME] [--tailscale-service svc:NAME]" >&2
  exit 1
}
say() { echo "annotary: $*" >&2; }

tag=
new=
src=
want_url=
want_link=
owner_handle=
owner_name=
ts_service=
accept_new_url=0
while [ $# -gt 0 ]; do
  [ $# -ge 2 ] || case "$1" in v[0-9]* | --accept-new-url) ;; *) usage ;; esac
  case "$1" in
    v[0-9]*) tag=$1; shift ;;
    --accept-new-url) accept_new_url=1; shift ;;
    --binary) new=$2; shift 2 ;;
    --src) src=$2; shift 2 ;;
    --url) want_url=$2; shift 2 ;;
    --link-base) want_link=$2; shift 2 ;;
    --owner-handle) owner_handle=$2; shift 2 ;;
    --owner-name) owner_name=$2; shift 2 ;;
    --tailscale-service) ts_service=$2; shift 2 ;;
    *) usage ;;
  esac
done
if [ -n "$tag" ]; then
  [ -z "$new" ] && [ -z "$src" ] || usage
  [[ $tag =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || { say "'$tag' is not vX.Y.Z"; exit 1; }
else
  [ -n "$new" ] && [ -n "$src" ] || usage
fi

root=${CD_ROOT:-}
user=annotary
unit=annotary.service
port=7377
app=$root/opt/annotary/app
data=$root/srv/annotary/data
etc=$root/etc/annotary
unit_dir=$root/etc/systemd/system
bin=$app/annotary
systemctl=${SYSTEMCTL:-systemctl}
runuser=${RUNUSER:-runuser}
curl=${CURL:-curl}
tar=${TAR:-tar}
tailscale=${TAILSCALE:-tailscale}
env_file=$etc/env

# A URL as the server stores it: lower case, no trailing slash (normalize_base_url). `none` stays.
normal_url() { printf '%s' "$1" | tr '[:upper:]' '[:lower:]' | sed 's:/*$::'; }
# The value systemd takes for KEY: the file's last, without quotes or blanks (no URL holds either).
# A key with no value counts as missing: the unit would pass `--url ""`, which serve takes as "clear".
env_value() { sed -n "s/^$1=//p" "$env_file" 2>/dev/null | tail -n 1 | tr -d ' \t"'"'"; }
env_has() { [ -r "$env_file" ] && [ -n "$(env_value "$1")" ]; }
check_url() { # check_url <flag> <value>: what goes in a root-only file that systemd reads
  [ "$2" = none ] || [[ $2 =~ ^[Hh][Tt][Tt][Pp][Ss]?://[A-Za-z0-9._:-]+/?$ || $2 =~ ^[Hh][Tt][Tt][Pp][Ss]?://\[[0-9A-Fa-f:]+\](:[0-9]+)?/?$ ]] ||
    { say "$1 '$2' is not none or scheme://host[:port]"; exit 1; }
}
if [ -z "$root" ] && [ "$(id -u)" != 0 ]; then
  say "run as root (sudo)"
  exit 1
fi
[ -z "$want_url" ] || check_url --url "$want_url"
[ -z "$want_link" ] || check_url --link-base "$want_link"
[ -z "$ts_service" ] || [[ $ts_service =~ ^svc:[a-z0-9-]+$ ]] || { say "--tailscale-service '$ts_service' is not svc:NAME"; exit 1; }
[ -z "$owner_handle" ] || [[ $owner_handle =~ ^[a-z0-9][a-z0-9-]*$ ]] || { say "--owner-handle '$owner_handle' is not a handle"; exit 1; }
env_has ANNOTARY_URL || [ -n "$want_url" ] ||
  { say "$env_file has no ANNOTARY_URL: give --url with the URL other devices use, or --url none"; exit 1; }

sha256() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }

# vX.Y.Z: fetch and check the release before anything on the host changes. Sets new and the two
# files the tarball carries; a mismatch, a missing checksum or the wrong version stops here.
unit_src=
health_src=
work=
probe_dir=
# shellcheck disable=SC2329  # the trap calls it
cleanup() { rm -rf "$work" "$probe_dir"; }
trap cleanup EXIT

# What the new binary says about itself, asked without root. A copy, because --binary is usually under
# the build user's home (750) and the download under a 700 temp folder; /tmp by name, because TMPDIR
# can be private too. HOME and the working directory are ones `nobody` is not refused.
prepare_probe() {
  if ! probe_dir=$(mktemp -d /tmp/annotary-probe.XXXXXX) || ! chmod 755 "$probe_dir" ||
    ! install -m 755 "$new" "$probe_dir/annotary"; then
    say "could not copy $new under /tmp to check it; nothing was installed"
    exit 1
  fi
}
probe() { (cd / && HOME=/nonexistent $runuser -u nobody -- "$probe_dir/annotary" "$@"); }

# Root unpacks the tarball, so its members are read first, and anything but the layout dist writes
# stops here: an exact list refuses an absolute path, `..` and any other name, and the listing's type
# column refuses a link or a device under one of those names. docs/dev/releases.md has where the
# list comes from.
check_members() { # check_members <tarball> <top folder>
  local tarball=$1 top=$2 names types name kind
  # GNU tar lists a file that is no archive as an empty one, with no error.
  if ! names=$($tar -tzf "$tarball") || ! types=$($tar -tvzf "$tarball" | cut -c1) || [ -z "$names" ]; then
    say "could not read ${tarball##*/}; nothing was installed"
    return 1
  fi
  while IFS= read -r name; do
    case "$name" in
      "$top/" | "$top/annotary" | "$top/annotary.service" | "$top/healthcheck.sh" | \
        "$top/README.md" | "$top/LICENSE-MIT" | "$top/LICENSE-APACHE") ;;
      *) say "${tarball##*/} holds '$name', which is not in the layout of a release; nothing was installed"; return 1 ;;
    esac
  done <<< "$names"
  while IFS= read -r kind; do
    case "$kind" in
      - | d) ;;
      *) say "${tarball##*/} holds a link or a special file (type '$kind'); nothing was installed"; return 1 ;;
    esac
  done <<< "$types"
  if [ -n "$(sort <<< "$names" | uniq -d)" ]; then
    say "${tarball##*/} holds a member twice; nothing was installed"
    return 1
  fi
}

if [ -n "$tag" ]; then
  case "$(uname -m)" in
    x86_64 | amd64) triple=x86_64-unknown-linux-gnu ;;
    aarch64 | arm64) triple=aarch64-unknown-linux-gnu ;;
    *) say "no release for $(uname -m)"; exit 1 ;;
  esac
  base=${ANNOTARY_RELEASE_BASE:-https://github.com/josh-andrews-dev/annotary/releases/download}
  tarball=annotary-$triple.tar.gz
  work=$(mktemp -d)
  download=(--proto '=https' --proto-redir '=https' -fsSL --retry 2)
  $curl "${download[@]}" -o "$work/$tarball" "$base/$tag/$tarball" || { say "could not download $base/$tag/$tarball"; exit 1; }
  $curl "${download[@]}" -o "$work/$tarball.sha256" "$base/$tag/$tarball.sha256" ||
    { say "could not download $tarball.sha256; refusing to install what cannot be checked"; exit 1; }
  sum=$(cut -d' ' -f1 "$work/$tarball.sha256" | head -n 1)
  [[ $sum =~ ^[0-9a-f]{64}$ ]] || { say "$tarball.sha256 holds no SHA-256; refusing to install"; exit 1; }
  got=$(sha256 "$work/$tarball")
  [ "$got" = "$sum" ] || { say "$tarball does not match its SHA-256 ($got, expected $sum); nothing was installed"; exit 1; }
  check_members "$work/$tarball" "annotary-$triple" || exit 1
  $tar --no-same-owner --no-same-permissions -xzf "$work/$tarball" -C "$work" ||
    { say "could not unpack $tarball; nothing was installed"; exit 1; }
  from=$work/annotary-$triple
  new=$from/annotary
  unit_src=$from/annotary.service
  health_src=$from/healthcheck.sh
  for f in "$new" "$unit_src" "$health_src"; do
    [ -f "$f" ] || { say "$tarball has no $(basename "$f"); nothing was installed"; exit 1; }
  done
  prepare_probe
  want=$(probe --version) || { say "the binary in $tarball does not run on this host; nothing was installed"; exit 1; }
  [ "$want" = "annotary ${tag#v}" ] || { say "$tarball holds '$want', not annotary ${tag#v}; nothing was installed"; exit 1; }
else
  unit_src=$src/deploy/$unit
  health_src=$src/deploy/healthcheck.sh
  [ -f "$new" ] || { say "$new is not a file; nothing was installed"; exit 1; }
  prepare_probe
  want=$(probe --version) || { say "$new does not run on this host; nothing was installed"; exit 1; }
fi

# A value that will fill in a missing key must be what the store already holds: the unit stores
# whatever --url it gets, and the start check only compares the server with that same value, so a
# typo would otherwise replace the live URL and pass. No store: nothing to replace. `none` is the
# health answer's null.
check_new_key() { # check_new_key <key> <value> <health field>
  env_has "$1" && return 0
  [ -n "$2" ] || return 0
  local health stored
  health=$($curl -sS --max-time 5 "http://127.0.0.1:$port/api/v1/health" 2>/dev/null || true)
  if ! grep -q '"ok":true' <<< "$health"; then
    say "$env_file has no $1 and the server is not answering, so $2 cannot be compared with what the store holds; nothing was changed"
    say "start the server and run this again, put $1 in that file yourself, or give --accept-new-url"
    exit 1
  fi
  stored=$(grep -o "\"$3\":\"[^\"]*\"" <<< "$health" | cut -d'"' -f4 || true)
  stored=${stored:-none}
  if [ "$(normal_url "$stored")" != "$(normal_url "$2")" ]; then
    say "$env_file has no $1, and $2 is not what the store holds ($stored); nothing was changed"
    say "correct the value, or give --accept-new-url if this is a deliberate change"
    exit 1
  fi
}
if [ -f "$data/store/annotary.sqlite" ] && [ "$accept_new_url" = 0 ]; then
  check_new_key ANNOTARY_URL "$want_url" public_url
  check_new_key ANNOTARY_LINK_BASE "${want_link:-none}" link_base
fi

if [ -z "$root" ]; then
  id "$user" >/dev/null 2>&1 ||
    useradd --system --home-dir /srv/annotary --no-create-home --shell /usr/sbin/nologin "$user"
  owner=(-o root -g root)
  mine=(-o "$user" -g "$user")
else
  owner=()
  mine=()
fi
install -d -m 755 ${owner[@]+"${owner[@]}"} "$root/opt/annotary" "$app"
install -d -m 750 ${mine[@]+"${mine[@]}"} "$root/srv/annotary" "$data"
install -d -m 700 ${owner[@]+"${owner[@]}"} "$etc"

# The values given here, added only where the file lacks them; what is already there is the host's.
# Written whole and renamed, 600 whatever it was. A value that differs is reported, not changed. A
# line with no value is replaced, as `grep -v` drops it (and ends a last line that had no newline).
add_env() { # add_env <key> <value>
  [ -n "$2" ] || return 0
  if env_has "$1"; then
    [ "$(normal_url "$(env_value "$1")")" = "$(normal_url "$2")" ] ||
      say "warning: $env_file keeps $1=$(env_value "$1") (not $2); edit that file to change it"
    return 0
  fi
  { grep -v "^$1=" "$env_file" || true; printf '%s=%s\n' "$1" "$2"; } > "$env_file.new"
  chmod 600 "$env_file.new"
  mv -f "$env_file.new" "$env_file"
}
[ -f "$env_file" ] || install -m 600 ${owner[@]+"${owner[@]}"} /dev/null "$env_file"
chmod 600 "$env_file"
add_env ANNOTARY_URL "$want_url"
add_env ANNOTARY_LINK_BASE "${want_link:-none}"
install -d -m 755 "$unit_dir"

# CMD-23: does NEW_BINARY migrate the store? The store's schema comes from store/meta.json, which
# every open rewrites after its migrations, so nothing opens the store (or waits on its LOCK)
# while the old server still runs. No store yet (a first install): nothing to export.
store_schema=
export_pending=0
pre_tar=
if [ -f "$data/store/annotary.sqlite" ]; then
  store_schema=$(sed -n 's/.*"schema_version": *\([0-9][0-9]*\).*/\1/p' "$data/store/meta.json" 2>/dev/null | head -n 1 || true)
  new_schema=$(probe serve --print-schema-version 2>/dev/null || true)
  if [ -z "$store_schema" ]; then
    say "no schema_version in $data/store/meta.json; refusing to deploy without knowing whether $want migrates the store"
    exit 1
  fi
  case "$new_schema" in
    '' | *[!0-9]*) say "$want did not print its store schema (serve --print-schema-version); refusing"; exit 1 ;;
  esac
  if [ "$new_schema" -gt "$store_schema" ]; then
    export_pending=1
    say "$want migrates the store from schema $store_schema to $new_schema; it is exported first"
  fi
fi

# A UTC stamp for a name, made unique against what is already there.
stamped() { # stamped <prefix> <suffix>
  local p
  p="$1$(date -u +%Y%m%dT%H%M%SZ)"
  [ -e "$p$2" ] && p="$p-$$"
  echo "$p$2"
}

# CMD-23: snapshots/pre-schema-N.tar, written by the previous binary (the one that owns schema N;
# `export` opens the store read-only and never migrates it, so the new binary would do too) with
# the service stopped, so no write lands after it. Sets pre_tar once the file is complete.
export_store() {
  local snaps=$data/snapshots tar exp=$bin
  tar=$snaps/pre-schema-$store_schema.tar
  [ "$had_prev" = 1 ] && exp=$bin.prev
  [ -d "$snaps" ] || install -d -m 700 ${mine[@]+"${mine[@]}"} "$snaps" ||
    { say "cannot create $snaps"; return 1; }
  if [ -e "$tar" ]; then
    local old
    old=$(stamped "$snaps/pre-schema-$store_schema." .tar)
    mv "$tar" "$old" || { say "cannot keep the earlier $tar as $old"; return 1; }
    say "kept the earlier $tar as $old"
  fi
  $runuser -u "$user" -- "$exp" export --data-dir "$data" "$tar" < /dev/null >&2 ||
    { say "export of the schema-$store_schema store failed; nothing was migrated, the unit is not started"; return 1; }
  chmod 600 "$tar" || say "warning: could not make $tar mode 600"
  pre_tar=$tar
  say "exported the schema-$store_schema store to $tar"
}

# CMD-23: the rollback across a migration. The new binary has (or may have) migrated store/, which
# the previous binary refuses, so store/ goes aside (kept) and pre_tar becomes a fresh store/.
# The export is untarred first, so a bad archive leaves store/ as it was.
restore_store() {
  local work=$data/.restore-$$ aside="(none)"
  $systemctl stop "$unit" 2>/dev/null || true
  install -d -m 750 ${mine[@]+"${mine[@]}"} "$work" || { say "cannot create $work"; return 1; }
  if ! $runuser -u "$user" -- tar -xf "$pre_tar" -C "$work" || [ ! -f "$work/store/annotary.sqlite" ]; then
    say "could not unpack $pre_tar; store/ is left as it is"
    rm -rf "$work"
    return 1
  fi
  if [ -e "$data/store" ]; then
    aside=$(stamped "$data/store.failed-" "")
    mv "$data/store" "$aside" || { say "cannot move store/ aside"; rm -rf "$work"; return 1; }
    say "kept the migrated store as $aside"
  fi
  mv "$work/store" "$data/store" || { say "cannot put the restored store in place (the migrated one is $aside)"; return 1; }
  rmdir "$work" 2>/dev/null || true
  chmod 750 "$data/store"
  say "restored store/ from $pre_tar"
}

# Answers /api/v1/health with `"version":"<want's X.Y.Z>"` within 30 s, and with `url` (the env
# file's) as its public URL when one is given: a unit that lost its URL has not come up right. An
# empty one given is that too: the unit has passed `--url ""`, which clears the store's.
comes_up() { # comes_up <version> [url]
  local v=${1#annotary } url=${2:-} body
  if [ $# -ge 2 ] && [ -z "$url" ]; then
    say "$env_file has no value for ANNOTARY_URL"
    return 1
  fi
  for _ in $(seq 1 "${CD_WAIT_TRIES:-60}"); do
    body=$($curl -sS --max-time 2 "http://127.0.0.1:$port/api/v1/health" 2>/dev/null || true)
    if grep -q '"ok":true' <<< "$body" && grep -q "\"version\":\"$v\"" <<< "$body" &&
      { [ $# -lt 2 ] || [ "$url" = none ] || grep -qF "\"public_url\":\"$url\"" <<< "$body"; }; then
      return 0
    fi
    sleep 0.5
  done
  return 1
}

# 3 and 4. Setup (stdout discarded: the first run prints the owner Token), then start. It runs
# inside `if`, where bash ignores `set -e`, so every step is checked: if setup fails, the unit
# is never started (its own `serve` would run setup; the unit sends stdout, where the Token goes,
# to /dev/null as well).
start() {
  $systemctl stop "$unit" 2>/dev/null || true
  if [ "$export_pending" = 1 ]; then
    export_pending=0
    export_store || return 1
  fi
  local who=()
  [ -z "$owner_handle" ] || who+=("ANNOTARY_SETUP_HANDLE=$owner_handle")
  [ -z "$owner_name" ] || who+=("ANNOTARY_SETUP_NAME=$owner_name")
  $runuser -u "$user" -- env ${who[@]+"${who[@]}"} \
    "$bin" serve --data-dir "$data" --setup-only < /dev/null > /dev/null ||
    { say "serve --setup-only failed; the unit is not started"; return 1; }
  $systemctl enable --quiet "$unit" || { say "systemctl enable $unit failed"; return 1; }
  $systemctl start "$unit" || { say "systemctl start $unit failed"; return 1; }
}

# What the installed binary says it is. It has been running as this user, so it is asked as this user.
installed_version() { (cd / && HOME=/nonexistent $runuser -u "$user" -- "$bin" --version); }

# Puts annotary.prev (and the unit and health check it ran with) back and starts it, then exits 1. Runs when
# the new binary does not come up, and on an interrupt from before anything is replaced, so a
# stopped deploy never leaves the new binary installed with the good one only in .prev.
rollback() {
  trap '' INT TERM HUP
  rm -f "$bin.new"
  $systemctl status "$unit" --no-pager -n 30 >&2 || true
  say "$want did not come up, or the deploy was interrupted; rolling back"
  if [ "$had_unit" = 1 ]; then
    install -m 644 ${owner[@]+"${owner[@]}"} "$app/$unit.prev" "$unit_dir/$unit" ||
      say "could not put the previous unit back; restoring the binary anyway"
    $systemctl daemon-reload || true
  fi
  if [ "$had_health" = 1 ]; then
    install -m 755 ${owner[@]+"${owner[@]}"} "$app/healthcheck.sh.prev" "$app/healthcheck.sh" ||
      say "could not put the previous health check back; restoring the binary anyway"
  fi
  export_pending=0
  if [ -n "$pre_tar" ] && ! restore_store; then
    say "rollback across the migration failed; the previous binary is not started (docs/ops-health-check.md P7)"
    exit 1
  fi
  if [ "$had_prev" = 1 ]; then
    cp -p "$bin.prev" "$bin.new"
    mv -f "$bin.new" "$bin"
    if start && comes_up "$(installed_version)"; then
      say "rolled back to $(installed_version); it is running"
    else
      say "rolled back to the previous binary, but it did not come up either"
    fi
  else
    say "no previous binary to roll back to"
  fi
  exit 1
}

# 2. Keep the old binary, unit and check, rename the new binary into place, install the unit and the
# check.
had_prev=0
had_unit=0
had_health=0
if [ -f "$bin" ]; then
  cp -p "$bin" "$bin.prev.new"
  mv -f "$bin.prev.new" "$bin.prev"
  had_prev=1
fi
if [ -f "$unit_dir/$unit" ]; then
  install -m 644 ${owner[@]+"${owner[@]}"} "$unit_dir/$unit" "$app/$unit.prev"
  had_unit=1
fi
if [ -f "$app/healthcheck.sh" ]; then
  install -m 755 ${owner[@]+"${owner[@]}"} "$app/healthcheck.sh" "$app/healthcheck.sh.prev"
  had_health=1
fi
trap rollback INT TERM HUP
install -m 755 ${owner[@]+"${owner[@]}"} "$new" "$bin.new"
mv -f "$bin.new" "$bin"
install -m 755 ${owner[@]+"${owner[@]}"} "$health_src" "$app/healthcheck.sh" || rollback
install -m 644 ${owner[@]+"${owner[@]}"} "$unit_src" "$unit_dir/$unit" || rollback
$systemctl daemon-reload || rollback

if ! { start && comes_up "$want" "$(normal_url "$(env_value ANNOTARY_URL)")"; }; then
  rollback
fi
trap - INT TERM HUP
say "$want is running ($unit, 127.0.0.1:$port)"

# 5. The Tailscale Service. A tagged node is what Tailscale needs to host a Service.
[ -n "$ts_service" ] || exit 0
if $tailscale status --peers=false --json 2>/dev/null | grep -q '"Tags": *\['; then
  if $tailscale serve --service="$ts_service" --https=443 --bg --yes "http://127.0.0.1:$port" >&2; then
    say "Tailscale Service $ts_service serves 127.0.0.1:$port"
  else
    say "warning: tailscale serve --service=$ts_service failed (docs/ops-health-check.md P3)"
  fi
else
  say "warning: this host is not a tagged Tailscale node, so $ts_service is not served yet (docs/ops-health-check.md P3)"
fi
