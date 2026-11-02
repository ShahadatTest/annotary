#!/usr/bin/env bash
# Tests scripts/trim-target.sh on a copy in a temp "repo" (the script finds the repo from its own
# path), so nothing outside the temp dir is touched. Folders are made stale with an old mtime,
# which trims them by the idle rule. `make trim-test` runs it (and `make test`).
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0

# The checks workflow exports these; each case sets only what it tests.
unset TRIM_TARGET_DIR CARGO_TARGET_DIR TRIM_MAX_GB TRIM_IDLE_DAYS
mkdir -p "$tmp/repo/scripts" "$tmp/home"
cp "$here/trim-target.sh" "$tmp/repo/scripts/"
trim() { HOME="$tmp/home" "$tmp/repo/scripts/trim-target.sh" "$@" 2>&1; }
stale() { mkdir -p "$1/debug" && touch -t 202001010000 "$1/debug/old"; }
check() { # check <description> <command...>
  if "${@:2}"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi
}
says() { grep -q -- "$2" <<<"$1"; }

# 1. A stale folder named by TRIM_TARGET_DIR is removed.
stale "$tmp/own"
TRIM_TARGET_DIR="$tmp/own" trim >/dev/null
check "TRIM_TARGET_DIR's stale folder is removed" test ! -e "$tmp/own"

# 2. CARGO_TARGET_DIR alone (a developer's shared folder) is never trimmed.
stale "$tmp/shared"
CARGO_TARGET_DIR="$tmp/shared" trim >/dev/null
check "CARGO_TARGET_DIR alone is left alone" test -e "$tmp/shared/debug/old"

# 3. A relative TRIM_TARGET_DIR is under the repo.
stale "$tmp/repo/target"
TRIM_TARGET_DIR=target trim >/dev/null
check "relative TRIM_TARGET_DIR resolves in the repo" test ! -e "$tmp/repo/target"

# 4. $HOME, the repo and a folder that holds the repo are refused.
stale "$tmp/home/x"
for dir in "$tmp/home" "$tmp/home/" "$tmp/home//" "$tmp/home/." "$tmp/home/x/.." "$tmp/repo" \
  "$tmp/repo/./" "$tmp" "$tmp/home/../repo/.."; do
  out=$(TRIM_TARGET_DIR="$dir" trim)
  check "refuses $dir" says "$out" refusing
done
check "refused folders are still there" test -e "$tmp/home/x/debug/old" -a -e "$tmp/repo/scripts/trim-target.sh"

# 5. Relative spellings of the repo, and /, are refused (dry runs, so a broken guard still
# removes nothing outside the temp dir).
for dir in . .. ./ / // /. /// //.; do
  out=$(TRIM_TARGET_DIR="$dir" trim --dry-run)
  check "refuses $dir" says "$out" refusing
done

# 6. A symlink to $HOME is refused.
ln -s "$tmp/home" "$tmp/link"
out=$(TRIM_TARGET_DIR="$tmp/link" trim)
check "refuses a symlink to \$HOME" says "$out" refusing

[ "$fails" -eq 0 ] && echo "trim-target: all ok" || { echo "trim-target: $fails failed"; exit 1; }
