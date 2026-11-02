#!/usr/bin/env bash
# Tests deploy/install-linux.sh under a temp CD_ROOT with stub commands: the binaries are shell
# scripts that answer `--version`, `--health` (what the curl stub asks), `serve
# --print-schema-version`, `serve --setup-only` (which prints a fake owner Token, to prove it is
# discarded, and "migrates" the store: it refuses a store/meta.json newer than its schema, as
# SchemaTooNew does, and otherwise writes its schema there and notes a migration in
# store/content) and `export` (a tar of store/); systemctl and tailscale log their calls, runuser
# refuses what `nobody` could not reach and tells the binary who it runs as, and curl serves the
# fake release in $CD_RELEASES or asks the installed binary for its health. The health answer's URL
# is the env file's, or with none there the store's: the files stored_url and stored_link under
# CD_ROOT. A file server_down makes curl fail until the next `systemctl start`.
# Runs anywhere; touches only a temp dir. `make deploy-test` runs it, and so does
# crates/annotary/tests/deploy_scripts.rs (spec S29-40).
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0

check() { # check <description> <command...>
  if "${@:2}"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi
}

# stub_bin <path> <X.Y.Z> <up: 1 answers health, 0 never does> [<setup exit code, default 0>]
#   [<store schema, default 1>]
stub_bin() {
  cat > "$1" <<STUB
#!/bin/sh
schema=${5:-1}
probed() { [ -z "\${CD_LOG:-}" ] || echo "probe \$1 as \${CD_AS:-root}" >> "\$CD_LOG"; }
case "\$1" in
  --version) probed version; echo "annotary $2" ;;
  --health)
    [ $3 = 1 ] || exit 7
    # As the unit: the public URL is the env file's, ${6:-} forces another one.
    url=\$(sed -n 's/^ANNOTARY_URL=//p' "\$CD_ROOT/etc/annotary/env" 2>/dev/null | tail -n 1)
    [ -n "\$url" ] || url=\$(cat "\$CD_ROOT/stored_url" 2>/dev/null)
    [ -z "${6:-}" ] || url=${6:-}
    url=\$(printf '%s' "\$url" | tr '[:upper:]' '[:lower:]' | sed 's:/*\$::') # the server normalizes
    link=\$(cat "\$CD_ROOT/stored_link" 2>/dev/null)
    echo "{\"ok\":true,\"version\":\"$2\",\"public_url\":\"\$url\"\${link:+,\"link_base\":\"\$link\"}}" ;;
  serve)
    [ "\$2" = --print-schema-version ] && { probed schema; echo "\$schema"; exit 0; }
    echo "setup \$*" >> "\$CD_LOG"; echo "ANNOTARY_TOKEN=ant_fake_owner_token"
    [ ${4:-0} = 0 ] || exit ${4:-0}
    store="\$3/store"
    have=\$(sed -n 's/.*"schema_version": *\([0-9]*\).*/\1/p' "\$store/meta.json" 2>/dev/null)
    [ -n "\$have" ] && [ "\$have" -gt "\$schema" ] && { echo "SchemaTooNew \$have > \$schema" >&2; exit 1; }
    mkdir -p "\$store"
    [ -n "\$have" ] && [ "\$have" -lt "\$schema" ] && echo "migrated to \$schema by $2" >> "\$store/content"
    : >> "\$store/annotary.sqlite"
    printf '{\n  "schema_version": %s\n}\n' "\$schema" > "\$store/meta.json" ;;
  export) echo "export $2 \$*" >> "\$CD_LOG"; tar -cf "\$4" -C "\$3" store ;;
esac
STUB
  chmod 755 "$1"
}

mkdir -p "$tmp/bin" "$tmp/src/deploy"
cp "$here/annotary.service" "$here/healthcheck.sh" "$tmp/src/deploy/"
# shellcheck disable=SC2016  # the stubs' own variables
{
  cat > "$tmp/bin/systemctl" <<'SYSTEMCTL'
#!/bin/sh
echo "systemctl $*" >> "$CD_LOG"
if [ "$1" = start ]; then
  rm -f "$CD_ROOT/server_down"
  # Stands for a file that loses its value between the installer's check and the start.
  [ -z "${STUB_BLANK_AT_START:-}" ] || printf 'ANNOTARY_URL=\nANNOTARY_LINK_BASE=none\n' > "$CD_ROOT/etc/annotary/env"
fi
exit 0
SYSTEMCTL
  cat > "$tmp/bin/runuser" <<'RUNUSER'
#!/bin/sh
# runuser -u WHO -- PROGRAM ARGS. As `nobody`, PROGRAM must be world-executable in folders it can
# enter, as it must be on a real host: this is what a 700 temp folder or a 750 home would fail.
who=$2
while [ "$1" != -- ]; do shift; done; shift
if [ "$who" = nobody ]; then
  echo "nobody ran $1" >> "$CD_LOG"
  perm() { stat -L -c %a "$1" 2>/dev/null || stat -L -f %Lp "$1"; }
  p=$1
  [ $(( $(perm "$p") % 2 )) = 1 ] || { echo "runuser: nobody cannot run $p" >&2; exit 126; }
  while p=$(dirname "$p") && [ "$p" != / ]; do
    [ $(( $(perm "$p") % 2 )) = 1 ] || { echo "runuser: nobody cannot enter $p" >&2; exit 126; }
  done
fi
CD_AS=$who exec "$@"
RUNUSER
  cat > "$tmp/bin/curl" <<'CURL'
#!/bin/sh
out=
url=
all="$*"
while [ $# -gt 0 ]; do
  case "$1" in
    -o) out=$2; shift 2 ;;
    --max-time | --retry | --proto | --proto-redir) shift 2 ;;
    -*) shift ;;
    *) url=$1; shift ;;
  esac
done
if [ -n "$out" ]; then
  echo "curl $url" >> "$CD_LOG"
  echo "curl-args $all" >> "$CD_LOG"
  cp "$CD_RELEASES/${url##*/}" "$out" 2>/dev/null || exit 22
  exit 0
fi
[ ! -e "$CD_ROOT/server_down" ] || exit 7
exec "$CD_ROOT/opt/annotary/app/annotary" --health
CURL
  cat > "$tmp/bin/tar" <<'TAR'
#!/bin/sh
echo "tar $*" >> "$CD_LOG"
exec "$CD_REAL_TAR" "$@"
TAR
  printf '#!/bin/sh\necho "tailscale $*" >> "$CD_LOG"\n[ "$1" = status ] && { [ "$TS_TAGGED" = 1 ] && echo "{\\"Self\\": {\\"Tags\\": [\\"tag:server\\"]}}" || echo "{\\"Self\\": {}}"; }\nexit 0\n' > "$tmp/bin/tailscale"
}
chmod 755 "$tmp/bin/"*

URL=https://annotary.example.ts.net
# install <root> <installer args...>: the installer's stderr and stdout, its exit code in $rc.
install_with() {
  local r=$1
  shift
  out=$(CD_ROOT=$r CD_LOG=$r/calls.log CD_WAIT_TRIES=1 TS_TAGGED=${TS_TAGGED:-0} CD_RELEASES=$tmp/releases \
    ANNOTARY_RELEASE_BASE=https://releases.example.com CD_REAL_TAR=$(command -v tar) TAR=$tmp/bin/tar \
    SYSTEMCTL=$tmp/bin/systemctl RUNUSER=$tmp/bin/runuser CURL=$tmp/bin/curl TAILSCALE=$tmp/bin/tailscale \
    bash "$here/install-linux.sh" "$@" 2>&1)
  rc=$?
}
# run <root> <new binary> [TS_TAGGED]: a build-path install with the usual settings.
run() {
  TS_TAGGED=${3:-0} install_with "$1" --binary "$2" --src "$tmp/src" --url "$URL" --tailscale-service svc:annotary
}
version() { "$1" --version 2>/dev/null || echo missing; }
app=opt/annotary/app

# 1. A first install on an untagged node.
r=$tmp/first; mkdir -p "$r"
stub_bin "$tmp/v1" 0.1.0 1
run "$r" "$tmp/v1"
check "first install exits 0" test "$rc" -eq 0
check "first install installs the binary" test "$(version "$r/$app/annotary")" = "annotary 0.1.0"
check "first install makes no .prev" test ! -e "$r/$app/annotary.prev"
check "the unit is installed" cmp -s "$here/annotary.service" "$r/etc/systemd/system/annotary.service"
check "the health check's copy is installed" cmp -s "$here/healthcheck.sh" "$r/$app/healthcheck.sh"
check "the data dir and the env file exist" test -d "$r/srv/annotary/data" -a -f "$r/etc/annotary/env"
check "the env file holds the URL it was given" grep -qx "ANNOTARY_URL=$URL" "$r/etc/annotary/env"
check "the env file's link base defaults to none" grep -qx "ANNOTARY_LINK_BASE=none" "$r/etc/annotary/env"
check "setup runs on the data dir" grep -q "^setup serve --data-dir $r/srv/annotary/data --setup-only" "$r/calls.log"
check "setup runs before the unit starts" \
  test "$(grep -n -E '^setup |^systemctl start' "$r/calls.log" | head -1 | cut -d: -f2 | cut -c1-6)" = "setup "
check "the owner Token setup prints is never shown" test -z "$(grep ant_fake <<< "$out")"
check "the unit is enabled and started" grep -q '^systemctl start annotary.service' "$r/calls.log"
check "untagged: says the Service waits" grep -q 'not a tagged Tailscale node' <<< "$out"
check "untagged: no tailscale serve" test -z "$(grep 'tailscale serve' "$r/calls.log")"

check "the unit sends stdout (the Token) to /dev/null" grep -q '^StandardOutput=null$' "$r/etc/systemd/system/annotary.service"
check "the unit keeps stderr (the log) in the journal" grep -q '^StandardError=journal$' "$r/etc/systemd/system/annotary.service"

# 2. An upgrade on a tagged node keeps the old binary and serves svc:annotary.
stub_bin "$tmp/v2" 0.2.0 1
run "$r" "$tmp/v2" 1
check "upgrade exits 0" test "$rc" -eq 0
check "upgrade installs the new binary" test "$(version "$r/$app/annotary")" = "annotary 0.2.0"
check "upgrade keeps the old one as .prev" test "$(version "$r/$app/annotary.prev")" = "annotary 0.1.0"
check "tagged: serves the Service to loopback" \
  grep -q '^tailscale serve --service=svc:annotary --https=443 --bg --yes http://127.0.0.1:7377$' "$r/calls.log"

# 3. A binary that never answers health is rolled back to .prev.
stub_bin "$tmp/bad" 0.3.0 0
run "$r" "$tmp/bad" 1
check "failed upgrade exits 1" test "$rc" -eq 1
check "failed upgrade puts the previous binary back" test "$(version "$r/$app/annotary")" = "annotary 0.2.0"
check "failed upgrade keeps .prev the previous binary" test "$(version "$r/$app/annotary.prev")" = "annotary 0.2.0"
check "failed upgrade says it is running the old one" grep -q 'rolled back to annotary 0.2.0; it is running' <<< "$out"

check "failed upgrade keeps the previous unit" test -f "$r/$app/annotary.service.prev"
echo "# the previous unit" >> "$r/etc/systemd/system/annotary.service"
run "$r" "$tmp/bad" 1
check "failed upgrade puts the previous unit back" grep -q '^# the previous unit$' "$r/etc/systemd/system/annotary.service"
run "$r" "$tmp/v2" 1

# 4. A binary that reports the old version (an old process still answering) does not pass.
stub_bin "$tmp/liar" 0.4.0 1
sed -i.bak 's/version\\":\\"0.4.0/version\\":\\"0.2.0/' "$tmp/liar" && rm -f "$tmp/liar.bak"
run "$r" "$tmp/liar"
check "a health answer from another version is not the new binary" test "$rc" -eq 1

# 5. Setup fails: the unit is never started, so its own serve never runs setup.
r=$tmp/setupfail; mkdir -p "$r"
stub_bin "$tmp/nosetup" 0.1.0 1 1
run "$r" "$tmp/nosetup"
check "failed setup exits 1" test "$rc" -eq 1
check "failed setup says so" grep -q 'setup-only failed; the unit is not started' <<< "$out"
check "failed setup never starts the unit" test -z "$(grep '^systemctl start' "$r/calls.log")"

# 6. Interrupted while the new binary starts: the previous one is put back.
r=$tmp/interrupted; mkdir -p "$r"
run "$r" "$tmp/v1"
stub_bin "$tmp/slow" 0.5.0 0
CD_ROOT=$r CD_LOG=$r/calls.log CD_WAIT_TRIES=100 TS_TAGGED=0 \
  SYSTEMCTL=$tmp/bin/systemctl RUNUSER=$tmp/bin/runuser CURL=$tmp/bin/curl TAILSCALE=$tmp/bin/tailscale \
  bash "$here/install-linux.sh" --binary "$tmp/slow" --src "$tmp/src" --url "$URL" > "$r/out" 2>&1 &
pid=$!
for _ in $(seq 1 50); do [ "$(grep -c '^systemctl start' "$r/calls.log")" = 2 ] && break; sleep 0.1; done
kill -TERM "$pid"
wait "$pid"; rc=$?
check "interrupted deploy exits 1" test "$rc" -eq 1
check "interrupted deploy puts the previous binary back" test "$(version "$r/$app/annotary")" = "annotary 0.1.0"
check "interrupted deploy keeps .prev the previous binary" test "$(version "$r/$app/annotary.prev")" = "annotary 0.1.0"

# 7. A first install that fails has nothing to roll back to.
r=$tmp/firstbad; mkdir -p "$r"
run "$r" "$tmp/bad"
check "failed first install exits 1" test "$rc" -eq 1
check "failed first install says there is nothing to roll back to" grep -q 'no previous binary' <<< "$out"

# 8. CMD-23: a newer store schema. The store is exported (by the previous binary, with the
# service stopped) to snapshots/pre-schema-N.tar before setup migrates it; no bump, no export.
mode() { stat -c %a "$1" 2>/dev/null || stat -f %Lp "$1"; }
first_of() { grep -E "$2" "$1" | head -n 1 | cut -d' ' -f1; }
d=srv/annotary/data
r=$tmp/migrate; mkdir -p "$r"
stub_bin "$tmp/s4a" 0.6.0 1 0 4
stub_bin "$tmp/s4b" 0.6.1 1 0 4
stub_bin "$tmp/s5" 0.7.0 1 0 5
run "$r" "$tmp/s4a"
echo before > "$r/$d/store/content"
rm -f "$r/calls.log"
run "$r" "$tmp/s4b"
check "no schema bump exits 0" test "$rc" -eq 0
check "no schema bump writes no pre-schema tar" test -z "$(find "$r/$d" -name 'pre-schema-*')"
check "no schema bump runs no export" test -z "$(grep '^export' "$r/calls.log")"
rm -f "$r/calls.log"
run "$r" "$tmp/s5"
check "a schema bump exits 0" test "$rc" -eq 0
check "a schema bump says it exports first" grep -q 'migrates the store from schema 4 to 5' <<< "$out"
check "a schema bump writes pre-schema-4.tar before setup" \
  test "$(first_of "$r/calls.log" '^(export|setup) ')" = export
check "a schema bump stops the service before the export" \
  test "$(first_of "$r/calls.log" '^(export|systemctl stop) ')" = systemctl
check "the export is written by the previous binary" grep -q "^export 0.6.1 export --data-dir $r/$d $r/$d/snapshots/pre-schema-4.tar$" "$r/calls.log"
check "pre-schema-4.tar holds the schema-4 store" \
  grep -q '"schema_version": 4' <(tar -xOf "$r/$d/snapshots/pre-schema-4.tar" store/meta.json)
check "pre-schema-4.tar is mode 600" test "$(mode "$r/$d/snapshots/pre-schema-4.tar")" = 600
check "the bump then migrates the store" grep -q 'migrated to 5 by 0.7.0' "$r/$d/store/content"

# 9. A failed start after a bump: the migrated store goes aside, the export becomes store/, and
# annotary.prev (which refuses the migrated store) starts on it.
r=$tmp/migfail; mkdir -p "$r"
stub_bin "$tmp/bad5" 0.8.0 0 0 5
run "$r" "$tmp/s4a"
echo before > "$r/$d/store/content"
run "$r" "$tmp/bad5"
check "a failed start after a bump exits 1" test "$rc" -eq 1
check "a failed start after a bump restores the export" test "$(cat "$r/$d/store/content")" = before
check "the restored store is schema 4" grep -q '"schema_version": 4' "$r/$d/store/meta.json"
check "the restored store is mode 750" test "$(mode "$r/$d/store")" = 750
aside=$(find "$r/$d" -maxdepth 1 -name 'store.failed-*' | head -n 1)
check "a failed start after a bump keeps the migrated store aside" grep -q 'migrated to 5 by 0.8.0' "$aside/content"
check "a failed start after a bump starts annotary.prev" grep -q 'rolled back to annotary 0.6.0; it is running' <<< "$out"
check "the rollback keeps pre-schema-4.tar" test -f "$r/$d/snapshots/pre-schema-4.tar"
check "no restore scratch dir left" test -z "$(find "$r/$d" -maxdepth 1 -name '.restore-*')"
run "$r" "$tmp/bad5"
check "a second try keeps the earlier pre-schema-4.tar" \
  test "$(find "$r/$d/snapshots" -name 'pre-schema-4.*.tar' | wc -l | tr -d ' ')" = 1
check "a second try keeps both migrated stores" \
  test "$(find "$r/$d" -maxdepth 1 -name 'store.failed-*' | wc -l | tr -d ' ')" = 2
check "a second try rolls back again" grep -q 'rolled back to annotary 0.6.0; it is running' <<< "$out"

# 10. Without the restore the previous binary would not come up: the stub's SchemaTooNew is real.
r=$tmp/toonew; mkdir -p "$r"
run "$r" "$tmp/s5"
stub_bin "$tmp/s4c" 0.6.2 1 0 4
run "$r" "$tmp/s4c"
check "an older binary on a migrated store fails setup and rolls back" test "$rc" -eq 1

# 11. A store with no readable schema refuses before anything is replaced.
r=$tmp/nometa; mkdir -p "$r"
run "$r" "$tmp/s4a"
rm -f "$r/$d/store/meta.json"
run "$r" "$tmp/s5"
check "a store without meta.json refuses the deploy" test "$rc" -eq 1
check "a store without meta.json says why" grep -q 'no schema_version in' <<< "$out"
check "a store without meta.json keeps the installed binary" test "$(version "$r/$app/annotary")" = "annotary 0.6.0"

# 12. The env file: the live host's case first, a file that exists but is empty. The URL still
# goes in, or the next start would have no --url.
r=$tmp/envfile; mkdir -p "$r/etc/annotary"
: > "$r/etc/annotary/env"
chmod 644 "$r/etc/annotary/env"
install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url "$URL" --link-base https://go.example.com
check "an empty env file is filled in" test "$rc" -eq 0
check "an empty env file gets the URL" grep -qx "ANNOTARY_URL=$URL" "$r/etc/annotary/env"
check "an empty env file gets the link base" grep -qx "ANNOTARY_LINK_BASE=https://go.example.com" "$r/etc/annotary/env"
check "the env file is mode 600 whatever it was" test "$(mode "$r/etc/annotary/env")" = 600
printf 'ANNOTARY_URL=https://mine.example.com\nOTHER=kept\n' > "$r/etc/annotary/env"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url "$URL" --link-base none
check "what the host's file has is kept" grep -qx "ANNOTARY_URL=https://mine.example.com" "$r/etc/annotary/env"
check "a differing value is reported, not written" grep -q "keeps ANNOTARY_URL=https://mine.example.com" <<< "$out"
check "the file's other lines are kept" grep -qx "OTHER=kept" "$r/etc/annotary/env"
check "a missing key is still added" grep -qx "ANNOTARY_LINK_BASE=none" "$r/etc/annotary/env"
check "the running URL is the file's" test "$rc" -eq 0

r=$tmp/nourl; mkdir -p "$r"
install_with "$r" --binary "$tmp/v1" --src "$tmp/src"
check "no URL anywhere refuses" test "$rc" -eq 1
check "no URL says what to give" grep -q 'has no ANNOTARY_URL: give --url' <<< "$out"
check "no URL changes nothing" test ! -e "$r/opt/annotary/app/annotary"
install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url none
check "--url none is a choice" test "$rc" -eq 0
check "--url none is recorded" grep -qx "ANNOTARY_URL=none" "$r/etc/annotary/env"
for bad in 'https://x.example.com/path' $'https://x.example.com\nEVIL=1' 'ftp://x.example.com' 'x'; do
  r=$tmp/badurl; rm -rf "$r"; mkdir -p "$r"
  install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url "$bad"
  check "a bad URL is refused" test "$rc" -eq 1
  check "a bad URL writes no env file" test ! -e "$r/etc/annotary/env"
done
install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url "$URL" --link-base 'https://a.example.com/path'
check "a bad link base is refused" test "$rc" -eq 1

# 13. A unit that comes up without the file's URL has not come up: it rolls back.
r=$tmp/losturl; mkdir -p "$r"
run "$r" "$tmp/v1"
stub_bin "$tmp/wrongurl" 0.9.0 1 0 1 https://other.example.com
run "$r" "$tmp/wrongurl"
check "a server answering with another URL rolls back" test "$rc" -eq 1
check "after a lost URL the previous binary runs" test "$(version "$r/$app/annotary")" = "annotary 0.1.0"

# 14. Only with --tailscale-service does a tagged node serve a Service: a stranger's host is left alone.
r=$tmp/noservice; mkdir -p "$r"
TS_TAGGED=1 install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url "$URL"
check "no --tailscale-service: nothing is served" test -z "$(grep '^tailscale' "$r/calls.log")"

# 15. Owner settings reach first-run setup, and no name is built in.
r=$tmp/owner; mkdir -p "$r"
cat > "$tmp/v-who" <<'STUB'
#!/bin/sh
case "$1" in
  --version) echo "annotary 0.1.0" ;;
  --health) echo '{"ok":true,"version":"0.1.0","public_url":"none"}' ;;
  serve) [ "$2" = --print-schema-version ] && { echo 1; exit 0; }
    echo "who=[$ANNOTARY_SETUP_HANDLE|$ANNOTARY_SETUP_NAME]" >> "$CD_LOG"; mkdir -p "$3/store"; : > "$3/store/annotary.sqlite"
    printf '{"schema_version": 1}\n' > "$3/store/meta.json" ;;
esac
STUB
chmod 755 "$tmp/v-who"
install_with "$r" --binary "$tmp/v-who" --src "$tmp/src" --url none --owner-handle rowan --owner-name "Rowan Vale"
check "the owner handle and name reach setup" grep -qx 'who=\[rowan|Rowan Vale\]' "$r/calls.log"
r=$tmp/noowner; mkdir -p "$r"
install_with "$r" --binary "$tmp/v-who" --src "$tmp/src" --url none
check "no owner given: setup takes its own defaults" grep -qx 'who=\[|\]' "$r/calls.log"

# 16. The release path: vX.Y.Z downloads the tarball, and cannot install without a matching SHA-256.
case "$(uname -m)" in
  x86_64 | amd64) triple=x86_64-unknown-linux-gnu ;;
  *) triple=aarch64-unknown-linux-gnu ;;
esac
tarball=annotary-$triple.tar.gz
sha() { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
seal_release() { printf '%s *%s\n' "$(sha "$tmp/releases/$tarball")" "$tarball" > "$tmp/releases/$tarball.sha256"; }
pack_release() { tar -czf "$tmp/releases/$tarball" -C "$tmp/pkg" "annotary-$triple" && seal_release; }
lay_out_release() { # lay_out_release <version>: the folder as dist lays it out (dist plan's assets)
  local d=$tmp/pkg/annotary-$triple
  rm -rf "$tmp/pkg" "$tmp/releases"
  mkdir -p "$d" "$tmp/releases"
  stub_bin "$d/annotary" "$1" 1
  cp "$here/annotary.service" "$here/healthcheck.sh" "$d/"
  echo readme > "$d/README.md"
  echo mit > "$d/LICENSE-MIT"
  echo apache > "$d/LICENSE-APACHE"
}
make_release() { # make_release <version>: that tarball, and its checksum
  lay_out_release "$1"
  pack_release
}
r=$tmp/release; mkdir -p "$r"
make_release 1.2.3
install_with "$r" v1.2.3 --url "$URL"
check "a release installs" test "$rc" -eq 0
check "a release installs its binary" test "$(version "$r/$app/annotary")" = "annotary 1.2.3"
check "a release downloads from the release base" grep -q "^curl https://releases.example.com/v1.2.3/$tarball$" "$r/calls.log"
check "a release installs the tarball's unit" cmp -s "$here/annotary.service" "$r/etc/systemd/system/annotary.service"
check "a release installs the tarball's health check" cmp -s "$here/healthcheck.sh" "$r/$app/healthcheck.sh"
make_release 1.2.4
install_with "$r" v1.2.4 --url "$URL"
check "a release upgrade keeps .prev" test "$(version "$r/$app/annotary.prev")" = "annotary 1.2.3"
check "a release upgrade runs" test "$(version "$r/$app/annotary")" = "annotary 1.2.4"

# Each refusal: nothing on the host has changed.
refuses() { # refuses <what> <grep for the message>
  install_with "$r" v1.2.5 --url "$URL"
  check "$1 refuses" test "$rc" -eq 1
  check "$1 says why" grep -q "$2" <<< "$out"
  check "$1 leaves the installed binary" test "$(version "$r/$app/annotary")" = "annotary 1.2.4"
  check "$1 leaves .prev" test "$(version "$r/$app/annotary.prev")" = "annotary 1.2.3"
}
make_release 1.2.5
printf 'tampered' >> "$tmp/releases/$tarball"
refuses "a tarball that does not match its SHA-256" 'does not match its SHA-256'
make_release 1.2.5
rm "$tmp/releases/$tarball.sha256"
refuses "a missing checksum" 'refusing to install what cannot be checked'
make_release 1.2.5
echo 'not a hash' > "$tmp/releases/$tarball.sha256"
refuses "a checksum file with no hash" 'holds no SHA-256'
make_release 1.2.5
rm "$tmp/releases/$tarball"
refuses "a missing tarball" 'could not download'
make_release 1.2.6
refuses "a tarball holding another version" "not annotary 1.2.5"
install_with "$r" 1.2.5 --url "$URL"
check "a version without its v is refused" test "$rc" -eq 1
install_with "$r" v1.2 --url "$URL"
check "a short version is refused" test "$rc" -eq 1
make_release 1.2.7
install_with "$r" v1.2.7
check "a later install needs no --url: the file has it" test "$rc" -eq 0
make_release 1.2.8
: > "$r/calls.log"
install_with "$r" v1.2.8
check "every download is HTTPS only, redirects too" \
  test "$(grep -c '^curl-args --proto =https --proto-redir =https ' "$r/calls.log"):$(grep -c '^curl-args ' "$r/calls.log")" = 2:2
check "the tarball is unpacked keeping no owner and no mode" \
  grep -q '^tar --no-same-owner --no-same-permissions -xzf ' "$r/calls.log"
check "the tarball is read before it is unpacked" \
  test "$(grep '^tar ' "$r/calls.log" | cut -d' ' -f2 | tr '\n' ' ')" = "-tzf -tvzf --no-same-owner "
installed=1.2.8

# Each bad tarball carries a valid checksum, so only the member check stands in the way. A tar that
# got as far as root would leave `marker` behind; the temp folder is ours to look in.
scratch=$tmp/installer-tmp
refuses_members() { # refuses_members <what> <grep for the message>
  seal_release
  rm -rf "$scratch"; mkdir -p "$scratch"
  : > "$r/calls.log"
  TMPDIR=$scratch install_with "$r" v1.2.9 --url "$URL"
  check "$1 refuses" test "$rc" -eq 1
  check "$1 says why" grep -q "$2" <<< "$out"
  check "$1 is never unpacked" test -z "$(grep -- '^tar .*-xzf' "$r/calls.log")"
  check "$1 leaves the installed binary" test "$(version "$r/$app/annotary")" = "annotary $installed"
  check "$1 runs nothing" test -z "$(grep '^nobody ran' "$r/calls.log")"
  check "$1 writes nothing outside the temp folder" test -z "$(find "$scratch" -name marker)"
}
top=annotary-$triple
lay_out_release 1.2.9
echo marker > "$tmp/pkg/marker"
(cd "$tmp/pkg" && tar -czPf "$tmp/releases/$tarball" "$top" "$top/../marker")
check "the test's tarball really holds a .. member" grep -q '\.\./marker' <(tar -tzf "$tmp/releases/$tarball")
refuses_members "a member with .." "holds '$top/../marker'"
lay_out_release 1.2.9
outside=$tmp/outside-marker; echo before > "$outside"
tar -czPf "$tmp/releases/$tarball" -C "$tmp/pkg" "$top" "$outside"
check "the test's tarball really holds an absolute member" grep -q '^/' <(tar -tzf "$tmp/releases/$tarball")
refuses_members "an absolute member" "holds '$outside'"
check "an absolute member overwrites nothing" test "$(cat "$outside")" = before
lay_out_release 1.2.9
echo x > "$tmp/pkg/$top/extra"
pack_release
refuses_members "a file outside the layout" "holds '$top/extra'"
lay_out_release 1.2.9
mkdir "$tmp/pkg/$top/sub"; echo x > "$tmp/pkg/$top/sub/annotary"
pack_release
refuses_members "a subfolder" "holds '$top/sub/'"
lay_out_release 1.2.9
mkdir "$tmp/pkg/annotary-other"; cp "$tmp/pkg/$top/annotary" "$tmp/pkg/annotary-other/"
tar -czf "$tmp/releases/$tarball" -C "$tmp/pkg" "$top" annotary-other
refuses_members "a second top folder" "holds 'annotary-other/'"
lay_out_release 1.2.9
tar -czf "$tmp/releases/$tarball" -C "$tmp/pkg/$top" .
refuses_members "a tarball with no top folder" "holds '\./"
lay_out_release 1.2.9
rm "$tmp/pkg/$top/README.md"
ln -s /etc/passwd "$tmp/pkg/$top/README.md"
pack_release
refuses_members "a symbolic link under an allowed name" "a link or a special file (type 'l')"
lay_out_release 1.2.9
rm "$tmp/pkg/$top/annotary"
ln -s annotary.service "$tmp/pkg/$top/annotary"
pack_release
refuses_members "the binary as a link" "a link or a special file (type 'l')"
lay_out_release 1.2.9
rm "$tmp/pkg/$top/LICENSE-MIT"
ln "$tmp/pkg/$top/LICENSE-APACHE" "$tmp/pkg/$top/LICENSE-MIT"
pack_release
refuses_members "a hard link under an allowed name" "a link or a special file (type 'h')"
lay_out_release 1.2.9
# GNU tar would store the second copy as a hard link; BSD tar has no such flag and stores it whole.
tar --hard-dereference -czf "$tmp/releases/$tarball" -C "$tmp/pkg" "$top" "$top/annotary" 2>/dev/null ||
  tar -czf "$tmp/releases/$tarball" -C "$tmp/pkg" "$top" "$top/annotary"
refuses_members "a member twice" "holds a member twice"
lay_out_release 1.2.9
printf 'not a tar' | gzip > "$tmp/releases/$tarball"
refuses_members "a tarball tar cannot read" "could not read"
lay_out_release 1.2.9
rm "$tmp/pkg/$top/healthcheck.sh"
pack_release
: > "$r/calls.log"
install_with "$r" v1.2.9 --url "$URL"
check "a tarball without a required file refuses" test "$rc" -eq 1
check "a tarball without a required file says which" grep -q 'has no healthcheck.sh' <<< "$out"
check "a tarball without a required file leaves the binary" test "$(version "$r/$app/annotary")" = "annotary $installed"
lay_out_release 1.2.9
rm "$tmp/pkg/$top/README.md" "$tmp/pkg/$top/LICENSE-MIT" "$tmp/pkg/$top/LICENSE-APACHE"
pack_release
install_with "$r" v1.2.9 --url "$URL"
check "the three required files alone are enough" test "$rc" -eq 0
check "that installs the binary" test "$(version "$r/$app/annotary")" = "annotary 1.2.9"

# 17. An empty value is a missing key: the unit would pass `--url ""`, which serve takes as "clear",
# so it neither satisfies the required URL nor survives as what the file "has".
env_of() { printf '%b' "$2" > "$1/etc/annotary/env"; }
for empty in 'ANNOTARY_URL=\n' 'ANNOTARY_URL=""\n' 'ANNOTARY_URL=https://a.example.com\nANNOTARY_URL=\n'; do
  r=$tmp/emptyval; rm -rf "$r"; mkdir -p "$r/etc/annotary"
  env_of "$r" "$empty"
  install_with "$r" --binary "$tmp/v1" --src "$tmp/src"
  check "an empty ANNOTARY_URL with no --url refuses" test "$rc" -eq 1
  check "an empty ANNOTARY_URL says what to give" grep -q 'has no ANNOTARY_URL: give --url' <<< "$out"
  check "an empty ANNOTARY_URL installs nothing" test ! -e "$r/$app/annotary"
done
r=$tmp/emptygiven; mkdir -p "$r/etc/annotary"
env_of "$r" 'ANNOTARY_URL=\nANNOTARY_LINK_BASE=\nOTHER=kept'
install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url "$URL"
check "an empty ANNOTARY_URL given --url installs" test "$rc" -eq 0
check "an empty ANNOTARY_URL given --url holds it" test "$(grep -c '^ANNOTARY_URL=' "$r/etc/annotary/env"):$(grep '^ANNOTARY_URL=' "$r/etc/annotary/env")" = "1:ANNOTARY_URL=$URL"
check "an empty link base is filled in" test "$(grep '^ANNOTARY_LINK_BASE=' "$r/etc/annotary/env")" = ANNOTARY_LINK_BASE=none
check "a last line with no newline stays its own line" grep -qx 'OTHER=kept' "$r/etc/annotary/env"
r=$tmp/urlnone; mkdir -p "$r/etc/annotary"
env_of "$r" 'ANNOTARY_URL=none\n'
install_with "$r" --binary "$tmp/v1" --src "$tmp/src"
check "ANNOTARY_URL=none needs no --url" test "$rc" -eq 0
check "ANNOTARY_URL=none is kept" grep -qx 'ANNOTARY_URL=none' "$r/etc/annotary/env"
r=$tmp/keyless; mkdir -p "$r/etc/annotary"
env_of "$r" 'OTHER=kept\n'
install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url "$URL"
check "a keyless file given --url installs" test "$rc:$(grep -c "^ANNOTARY_URL=$URL$" "$r/etc/annotary/env")" = 0:1
r=$tmp/blanked; mkdir -p "$r"
run "$r" "$tmp/v1"
STUB_BLANK_AT_START=1 install_with "$r" --binary "$tmp/v2" --src "$tmp/src"
check "a URL that goes empty before the start check rolls back" test "$rc" -eq 1
check "the start check says the value is gone" grep -q 'has no value for ANNOTARY_URL' <<< "$out"
check "after that the previous binary runs" test "$(version "$r/$app/annotary")" = "annotary 0.1.0"

# 18. Filling in a key on a host with a store: the value must be what the store holds. This is the
# live host's next deploy: an empty env file, a running server, and settings holding its values.
LINK=https://go.example.com
live_host() { # live_host <root>: a store holding URL and LINK, its env file empty
  rm -rf "$1"; mkdir -p "$1"
  install_with "$1" --binary "$tmp/v1" --src "$tmp/src" --url "$URL" --link-base "$LINK"
  : > "$1/etc/annotary/env"
  printf '%s' "$URL" > "$1/stored_url"
  printf '%s' "$LINK" > "$1/stored_link"
}
refused_new() { # refused_new <what> <grep for the message>
  check "$1 refuses" test "$rc" -eq 1
  check "$1 says why" grep -q "$2" <<< "$out"
  check "$1 leaves the env file" test ! -s "$r/etc/annotary/env"
  check "$1 leaves the binary" test "$(version "$r/$app/annotary")" = "annotary 0.1.0"
  check "$1 does not stop the server" test -z "$(grep -c '^systemctl stop' "$r/calls.log" | grep -v '^1$')"
}
r=$tmp/livehost
live_host "$r"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url "$URL" --link-base "$LINK"
check "the store's own values fill an empty file with no override" test "$rc" -eq 0
check "the store's own URL goes in" grep -qx "ANNOTARY_URL=$URL" "$r/etc/annotary/env"
check "the store's own link base goes in" grep -qx "ANNOTARY_LINK_BASE=$LINK" "$r/etc/annotary/env"
check "the deploy then runs the new binary" test "$(version "$r/$app/annotary")" = "annotary 0.2.0"
live_host "$r"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url "https://ANNOTARY.example.ts.net/" --link-base "$LINK/"
check "case and a trailing slash are the same URL" test "$rc" -eq 0
live_host "$r"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url https://annotary.example.com --link-base "$LINK"
refused_new "a mistyped URL" 'is not what the store holds (https://annotary.example.ts.net)'
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url "$URL" --link-base https://go.example.org
refused_new "a mistyped link base" 'is not what the store holds (https://go.example.com)'
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url "$URL"
refused_new "a link base left to its default of none" 'none is not what the store holds'
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url none --link-base "$LINK"
refused_new "--url none over a stored URL" 'none is not what the store holds'
touch "$r/server_down"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url "$URL" --link-base "$LINK"
refused_new "a store with no answering server" 'is not answering'
rm -f "$r/server_down"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url https://new.example.com --link-base https://go2.example.com --accept-new-url
check "--accept-new-url changes the URL on purpose" test "$rc:$(grep -c '^ANNOTARY_URL=https://new.example.com$' "$r/etc/annotary/env")" = 0:1
check "--accept-new-url changes the link base on purpose" grep -qx 'ANNOTARY_LINK_BASE=https://go2.example.com' "$r/etc/annotary/env"
live_host "$r"
touch "$r/server_down"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url "$URL" --link-base "$LINK" --accept-new-url
check "--accept-new-url needs no answering server" test "$rc" -eq 0
live_host "$r"
env_of "$r" "ANNOTARY_URL=https://edited.example.com\nANNOTARY_LINK_BASE=$LINK\n"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url "$URL" --link-base "$LINK"
check "a value edited into the file is a deliberate change" test "$rc" -eq 0
check "the edited value stays" grep -qx 'ANNOTARY_URL=https://edited.example.com' "$r/etc/annotary/env"
live_host "$r"
env_of "$r" "ANNOTARY_URL=\nANNOTARY_LINK_BASE=$LINK\n"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url https://annotary.example.com
check "an empty value is checked like a missing key" test "$rc" -eq 1
r=$tmp/storednone; mkdir -p "$r"
install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url none
: > "$r/etc/annotary/env"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url none
check "none matches a store with no URL and no link base" test "$rc" -eq 0
check "none is recorded for both" test "$(grep -c '=none$' "$r/etc/annotary/env")" = 2
r=$tmp/storednourl; mkdir -p "$r"
install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url none
: > "$r/etc/annotary/env"
install_with "$r" --binary "$tmp/v2" --src "$tmp/src" --url "$URL"
check "a URL over a store with none is a change, so it refuses" test "$rc" -eq 1
r=$tmp/brandnew; mkdir -p "$r/etc/annotary"
: > "$r/etc/annotary/env"
touch "$r/server_down"
install_with "$r" --binary "$tmp/v1" --src "$tmp/src" --url "$URL" --link-base "$LINK"
check "a first install has nothing to compare with" test "$rc" -eq 0

# 19. The new binary is never run as root before it is installed, on either path, from wherever it
# sits; the stub runuser refuses what `nobody` cannot reach, and $tmp itself is mode 700.
as_root() { grep 'as root$' "$1/calls.log"; }
r=$tmp/probes; mkdir -p "$r" "$tmp/private"
chmod 700 "$tmp/private"
stub_bin "$tmp/private/s1" 0.1.0 1 0 1
stub_bin "$tmp/private/s2" 0.2.0 1 0 2
install_with "$r" --binary "$tmp/private/s1" --src "$tmp/src" --url "$URL"
check "a build in a folder only its owner can read installs" test "$rc" -eq 0
check "the first install asks its version as nobody" grep -qx 'probe version as nobody' "$r/calls.log"
check "the first install runs nothing as root" test -z "$(as_root "$r")"
: > "$r/calls.log"
install_with "$r" --binary "$tmp/private/s2" --src "$tmp/src"
check "an upgrade asks the store schema as nobody" grep -qx 'probe schema as nobody' "$r/calls.log"
check "an upgrade runs nothing as root" test -z "$(as_root "$r")"
: > "$r/calls.log"
stub_bin "$tmp/private/bad" 0.3.0 0 0 2
install_with "$r" --binary "$tmp/private/bad" --src "$tmp/src"
check "a rollback exits 1" test "$rc" -eq 1
check "a rollback asks the old binary's version as its user" grep -qx 'probe version as annotary' "$r/calls.log"
check "a rollback runs nothing as root" test -z "$(as_root "$r")"
check "a rollback still says what it restored" grep -q 'rolled back to annotary 0.2.0; it is running' <<< "$out"
r=$tmp/probesrel; mkdir -p "$r"
make_release 1.3.0
TMPDIR=$tmp/private install_with "$r" v1.3.0 --url "$URL"
check "a release in a private TMPDIR installs" test "$rc" -eq 0
check "a release asks its version as nobody" grep -qx 'probe version as nobody' "$r/calls.log"
check "a release runs nothing as root" test -z "$(as_root "$r")"
probe_path=$(sed -n 's/^nobody ran //p' "$r/calls.log" | head -n 1)
check "a probe's copy is outside the temp folders" grep -q '^/tmp/annotary-probe\.' <<< "$probe_path"
check "no probe folder is left" test ! -e "$(dirname "$probe_path")"

# 20. A rollback puts the previous health check back with the unit, and a host with no previous
# copy has nothing to restore and no complaint about it.
r=$tmp/healthback; mkdir -p "$r"
run "$r" "$tmp/v1"
echo "# the previous check" >> "$r/$app/healthcheck.sh"
run "$r" "$tmp/v2"
check "a good upgrade installs the new check" cmp -s "$here/healthcheck.sh" "$r/$app/healthcheck.sh"
check "a good upgrade keeps the previous check" grep -q '^# the previous check$' "$r/$app/healthcheck.sh.prev"
echo "# the check v2 ran with" >> "$r/$app/healthcheck.sh"
run "$r" "$tmp/bad"
check "a rollback exits 1 with a check to restore" test "$rc" -eq 1
check "a rollback puts the previous check back" grep -q '^# the check v2 ran with$' "$r/$app/healthcheck.sh"
check "the check it puts back is executable" test -x "$r/$app/healthcheck.sh"
r=$tmp/healthfirst; mkdir -p "$r"
run "$r" "$tmp/bad"
check "a failed first install keeps no previous check" test ! -e "$r/$app/healthcheck.sh.prev"
check "a failed first install has no complaint about the check" test -z "$(grep 'previous health check' <<< "$out")"
check "a failed first install still says there is nothing to roll back to" grep -q 'no previous binary' <<< "$out"

check "no .new files left" test -z "$(find "$tmp" -name '*.new')"

if [ "$fails" -gt 0 ]; then echo "$fails failed"; exit 1; fi
echo "all passed"
