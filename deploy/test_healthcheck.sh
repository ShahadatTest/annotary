#!/usr/bin/env bash
# Tests deploy/healthcheck.sh on a temp data dir (ANNOTARY_DATA) and env file (ANNOTARY_ENV_FILE):
# the snapshot check (CMD-1: no snapshot, a fresh one, one over an hour old and one over a day old)
# and the URL check, with a stub curl as the server. Only those problems are asserted; the other
# checks read systemd and the disk and say what they say. `make deploy-test` runs it.
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0

check() { # check <description> <command...>
  if "${@:2}"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi
}
snapshot_problem() { # the snapshot problem's severity, or "none"
  local verdict sev
  verdict=$(ANNOTARY_DATA=$tmp/data ANNOTARY_ENV_FILE=$tmp/env bash "$here/healthcheck.sh" 2>/dev/null) # exits 1 or 2 on a problem
  sev=$(grep -o '"severity":"[a-z]*","check":"snapshot"' <<< "$verdict" | cut -d'"' -f4)
  echo "${sev:-none}"
}

mkdir -p "$tmp/data/store"
check "no snapshot is a warning" test "$(snapshot_problem)" = warning
mkdir -p "$tmp/data/snapshots"
f=$tmp/data/snapshots/annotary-20261021T000000Z-s1.tar
: > "$f"
check "a fresh snapshot is fine" test "$(snapshot_problem)" = none
touch -t "$(date -v-2H +%Y%m%d%H%M 2>/dev/null || date -d '2 hours ago' +%Y%m%d%H%M)" "$f"
check "a snapshot over an hour old is a warning" test "$(snapshot_problem)" = warning
: > "$tmp/data/snapshots/annotary-20261021T001500Z-s2.tar"
check "the newest snapshot counts, not the oldest" test "$(snapshot_problem)" = none
rm "$tmp/data/snapshots/annotary-20261021T001500Z-s2.tar"
touch -t "$(date -v-2d +%Y%m%d%H%M 2>/dev/null || date -d '2 days ago' +%Y%m%d%H%M)" "$f"
check "a snapshot over a day old is critical" test "$(snapshot_problem)" = critical
: > "$tmp/data/snapshots/.annotary-20261021T003000Z-s3.tar.tmp"
check "a half-written temp file does not count" test "$(snapshot_problem)" = critical
rm -rf "$tmp/data/store"
check "no store, no snapshot check" test "$(snapshot_problem)" = none

# The URL. The stub answers as a server whose public URL is $STUB_URL; its own address answers ok.
mkdir -p "$tmp/bin" "$tmp/data"
cat > "$tmp/bin/curl" <<'CURL'
#!/bin/sh
for a in "$@"; do last=$a; done
case "$last" in
  http://127.0.0.1:*) [ -n "${STUB_DOWN:-}" ] || printf '{"ok":true,"set_up":true,"password_set":true%s}' "${STUB_URL:+,\"public_url\":\"$STUB_URL\"}" ;;
  *) [ -n "${STUB_FAR_DOWN:-}" ] || printf '{"ok":true}' ;;
esac
CURL
chmod 755 "$tmp/bin/curl"
U=https://annotary.example.ts.net
url_problem() { # url_problem [ENV=value...]: the "public" problem's severity, or "none"
  local verdict sev
  verdict=$(env "$@" PATH="$tmp/bin:$PATH" ANNOTARY_DATA="$tmp/data" ANNOTARY_ENV_FILE="$tmp/env" bash "$here/healthcheck.sh" 2>/dev/null)
  sev=$(grep -o '"severity":"[a-z]*","check":"public"' <<< "$verdict" | head -n 1 | cut -d'"' -f4)
  echo "${sev:-none}"
}
rm -f "$tmp/env"
check "no env file is critical" test "$(url_problem STUB_URL=$U)" = critical
: > "$tmp/env"
check "an env file without the URL is critical, though the store remembers one" test "$(url_problem STUB_URL=$U)" = critical
echo "ANNOTARY_URL=$U" > "$tmp/env"
check "the file and the server agreeing is fine" test "$(url_problem STUB_URL=$U)" = none
check "a trailing slash and case do not matter" test "$(url_problem STUB_URL=https://ANNOTARY.example.ts.net/)" = none
check "a server with no URL, with one in the file, is critical" test "$(url_problem STUB_URL=)" = critical
check "a server with another URL is critical" test "$(url_problem STUB_URL=https://other.example.com)" = critical
check "settings agreeing with the file are fine" test "$(url_problem STUB_URL=$U ANNOTARY_EXPECT_URL=$U)" = none
check "settings disagreeing with the file are critical" test "$(url_problem STUB_URL=$U ANNOTARY_EXPECT_URL=https://other.example.com)" = critical
check "a URL that does not answer from outside is a warning" test "$(url_problem STUB_URL=$U STUB_FAR_DOWN=1)" = warning
echo "ANNOTARY_URL=none" > "$tmp/env"
check "none says no URL on purpose" test "$(url_problem STUB_URL=)" = none
for empty in 'ANNOTARY_URL=' 'ANNOTARY_URL=""' $'ANNOTARY_URL=https://a.example.com\nANNOTARY_URL='; do
  printf '%s\n' "$empty" > "$tmp/env"
  check "an empty ANNOTARY_URL is critical, though the store remembers one" test "$(url_problem STUB_URL=$U)" = critical
  check "an empty ANNOTARY_URL is critical, with no URL on the server" test "$(url_problem STUB_URL=)" = critical
done
echo 'ANNOTARY_URL="'$U'"' > "$tmp/env"
check "a quoted URL is the URL" test "$(url_problem STUB_URL=$U)" = none
echo "ANNOTARY_URL=none" > "$tmp/env"
check "a server that does not answer is the API's problem, not this one" test "$(url_problem STUB_DOWN=1)" = none

if [ "$fails" -gt 0 ]; then echo "$fails failed"; exit 1; fi
echo "all passed"
