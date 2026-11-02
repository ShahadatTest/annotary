#!/usr/bin/env bash
# Tests deploy/build_release.sh (where the built binary is read from, and the version it is
# stamped with) with a stub cargo, and deploy/next_version.py against a throwaway git repo with
# a bare origin. Runs anywhere; touches only a temp dir. `make deploy-test` runs it (and `make test`).
set -uo pipefail
here=$(cd "$(dirname "$0")" && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
fails=0

check() { # check <description> <command...>
  if "${@:2}"; then echo "ok   $1"; else echo "FAIL $1"; fails=$((fails + 1)); fi
}
binary() { # binary <path> <version>: a stand-in annotary that answers --version
  mkdir -p "$(dirname "$1")"
  printf '#!/bin/sh\necho "annotary %s"\n' "$2" > "$1"
  chmod 755 "$1"
}

# A stub cargo that "builds" where real cargo would: $CARGO_TARGET_DIR/release, else target/release,
# a binary reporting ANNOTARY_VERSION (or 0.0.0, the Cargo.toml placeholder, when unset).
cat > "$tmp/cargo" <<'STUB'
#!/bin/sh
out=${CARGO_TARGET_DIR:-target}/release/annotary
mkdir -p "$(dirname "$out")"
printf '#!/bin/sh\necho "annotary %s"\n' "${ANNOTARY_VERSION:-0.0.0}" > "$out"
chmod 755 "$out"
STUB
chmod 755 "$tmp/cargo"
# A cargo that ignores ANNOTARY_VERSION, as a build without the option_env! stamp would.
cat > "$tmp/cargo-unstamped" <<'STUB'
#!/bin/sh
mkdir -p target/release
printf '#!/bin/sh\necho "annotary 0.0.0"\n' > target/release/annotary
chmod 755 target/release/annotary
STUB
chmod 755 "$tmp/cargo-unstamped"

build() { # build <dir> <version> [VAR=value...]: run build_release.sh in <dir>, print its stdout
  local dir=$1 version=$2; shift 2
  (cd "$dir" && env -u CARGO_TARGET_DIR CARGO="$tmp/cargo" "$@" "$here/build_release.sh" "$version" 2>/dev/null)
}

# 1. CARGO_TARGET_DIR unset: the binary is target/release/annotary, stamped with the version.
d=$tmp/plain; mkdir -p "$d"
out=$(build "$d" 1.2.3); rc=$?
check "unset target dir exits 0" test "$rc" -eq 0
check "unset target dir prints target/release/annotary" test "$out" = target/release/annotary
check "unset target dir binary reports the version" test "$("$d/$out" --version)" = "annotary 1.2.3"

# 2. CARGO_TARGET_DIR set, a stale binary in target/release (CMD-3): the fresh one is used.
d=$tmp/elsewhere; mkdir -p "$d"
binary "$d/target/release/annotary" 0.0.1
out=$(build "$d" 1.2.4 CARGO_TARGET_DIR="$tmp/cargo-target"); rc=$?
check "CARGO_TARGET_DIR exits 0" test "$rc" -eq 0
check "CARGO_TARGET_DIR prints the binary cargo built" test "$out" = "$tmp/cargo-target/release/annotary"
check "CARGO_TARGET_DIR binary is the new one" test "$("$out" --version)" = "annotary 1.2.4"
check "the stale target/release binary is left alone" test "$("$d/target/release/annotary" --version)" = "annotary 0.0.1"

# 3. A relative CARGO_TARGET_DIR is relative to where cargo ran (the repo root).
d=$tmp/relative; mkdir -p "$d"
out=$(build "$d" 1.2.5 CARGO_TARGET_DIR=build-out); rc=$?
check "relative CARGO_TARGET_DIR prints build-out/release/annotary" test "$rc/$out" = 0/build-out/release/annotary

# 4. A binary that does not report the version it was asked for is refused (CMD-2).
d=$tmp/unstamped; mkdir -p "$d"
err=$(cd "$d" && env -u CARGO_TARGET_DIR CARGO="$tmp/cargo-unstamped" "$here/build_release.sh" 1.2.6 2>&1 >/dev/null); rc=$?
check "unstamped binary exits 1" test "$rc" -eq 1
check "unstamped binary says what it reported" grep -q "says 'annotary 0.0.0', not 'annotary 1.2.6'" <<< "$err"

# 5. Nothing at the expected path (cargo wrote elsewhere) is refused, not a silent stale install.
cat > "$tmp/cargo-nowhere" <<'STUB'
#!/bin/sh
exit 0
STUB
chmod 755 "$tmp/cargo-nowhere"
d=$tmp/nowhere; mkdir -p "$d"
err=$(cd "$d" && env -u CARGO_TARGET_DIR CARGO="$tmp/cargo-nowhere" "$here/build_release.sh" 1.2.7 2>&1 >/dev/null); rc=$?
check "missing binary exits 1" test "$rc" -eq 1
check "missing binary names the path" grep -q "no binary at target/release/annotary" <<< "$err"

# 6. A version that is not X.Y.Z is refused before anything is built.
d=$tmp/badversion; mkdir -p "$d"
(cd "$d" && CARGO="$tmp/cargo" "$here/build_release.sh" v1.2 2>/dev/null); rc=$?
check "bad version exits 1" test "$rc" -eq 1
check "bad version builds nothing" test ! -e "$d/target"

# 7. next_version.py: the tag release.py will give HEAD, without the prefix.
g() { git -C "$tmp/repo" -c user.name=t -c user.email=t@t "$@" >/dev/null 2>&1; }
next() { (cd "$tmp/repo" && python3 "$here/next_version.py" "$@" 2>/dev/null); }
git init -q --bare "$tmp/origin.git"
git init -q "$tmp/repo"
g remote add origin "$tmp/origin.git"
g commit --allow-empty -m one
check "no tags: patch gives 0.0.1" test "$(next)" = 0.0.1
check "no tags: minor gives 0.1.0" test "$(next --bump minor)" = 0.1.0
g tag -a v0.1.0 -m v0.1.0
g tag -a v0.1.2 -m v0.1.2
g tag -a server-v9.0.0 -m other-series
g commit --allow-empty -m two
check "highest v tag plus a patch" test "$(next)" = 0.1.3
check "highest v tag plus a minor" test "$(next --bump minor)" = 0.2.0
check "highest v tag plus a major" test "$(next --bump major)" = 1.0.0
g tag -a v0.1.3 -m v0.1.3
check "HEAD already tagged (a re-deploy) keeps its version" test "$(next)" = 0.1.3
g commit --allow-empty -m three
g push -q origin HEAD:refs/heads/main
git -C "$tmp/origin.git" tag v0.4.0 refs/heads/main >/dev/null 2>&1
check "tags only on origin are fetched first" test "$(next)" = 0.4.0

if [ "$fails" -gt 0 ]; then echo "$fails failed"; exit 1; fi
echo "all passed"
