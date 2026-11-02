#!/usr/bin/env bash
#
# trim-target.sh — keep target/ from holding build output nothing will use again.
#
# Cargo never deletes anything from target/: a toolchain upgrade, a dependency bump or a feature
# change leaves the old artifacts next to the new ones, and a project left alone keeps its whole
# target/ forever. This removes it when one of these holds (checked in this order):
#
#   1. the toolchain changed since the last run (`rustc -vV` differs from target/.trim-rustc) —
#      every artifact in target/ was built by the old compiler and will be rebuilt anyway;
#   2. nothing under target/ was written for TRIM_IDLE_DAYS days (default 14) — the project
#      was left alone, so the next build starts fresh rather than on top of stale output;
#   3. target/ is larger than TRIM_MAX_GB (default 15) — stale artifacts have piled up.
#
# Otherwise it does nothing. It runs before every cargo-using `make` target and at the start and
# end of every Claude Code session (.claude/settings.json). Deleting target/ is always safe: the
# next build recreates it (`make build` takes a few minutes from scratch).
#
#   scripts/trim-target.sh            check and trim, say what happened
#   scripts/trim-target.sh --quiet    only speak when something was removed
#   scripts/trim-target.sh --dry-run  say what would happen, remove nothing
#
# "target/" is TRIM_TARGET_DIR when that is set: the checks workflow sets it to its per-runner
# CARGO_TARGET_DIR outside the checkout, and this is what bounds it (TRIM_MAX_GB is lower there).
# It is a separate variable on purpose: a CARGO_TARGET_DIR a developer shares between projects is
# never removed. It refuses /, and $HOME, the repo or any folder holding either, by canonical path.
#
# Always exits 0 — it must never block a build or a session.

set -uo pipefail

IDLE_DAYS="${TRIM_IDLE_DAYS:-14}"
MAX_GB="${TRIM_MAX_GB:-15}"
QUIET=0
DRY=0
for arg in "$@"; do
    case "$arg" in
        --quiet|-q) QUIET=1 ;;
        --dry-run|-n) DRY=1 ;;
        *) echo "trim-target: unknown argument $arg" >&2; exit 0 ;;
    esac
done

ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
TARGET="${TRIM_TARGET_DIR:-$ROOT/target}"
case "$TARGET" in /*) ;; *) TARGET="$ROOT/$TARGET" ;; esac

say() { [ "$QUIET" = 1 ] || echo "trim-target: $*"; }

[ -d "$TARGET" ] || { say "no target/ — nothing to do"; exit 0; }

# The refusals compare canonical paths, so no spelling (//, /., x/..) or symlink gets past them.
TARGET="$(cd "$TARGET" && pwd -P)" || exit 0
home="$(cd "$HOME" 2>/dev/null && pwd -P || echo "$HOME")"
refuse() { echo "trim-target: refusing to trim '$TARGET'$1" >&2; exit 0; }
case "$TARGET" in *[!/]*) ;; *) refuse "" ;; esac   # /, and // (which pwd -P keeps)
for keep in "$home" "$ROOT"; do
    case "$keep/" in "$TARGET"/*) refuse ", which is or holds $keep" ;; esac
done
STAMP="$TARGET/.trim-rustc"
export PATH="$HOME/.cargo/bin:$PATH"

# Never pull target/ out from under a running build (cargo holds `<profile>/.cargo-lock` open) or
# a running test binary / server started from target/.
if { command -v lsof >/dev/null && lsof -t "$TARGET"/*/.cargo-lock "$TARGET"/*/*/.cargo-lock >/dev/null 2>&1; } \
    || pgrep -f "$TARGET/" >/dev/null 2>&1; then
    say "a build or a binary from target/ is running — skipped"
    exit 0
fi

remove() {
    local why="$1" size
    size="$(du -sh "$TARGET" 2>/dev/null | cut -f1)"
    if [ "$DRY" = 1 ]; then
        echo "trim-target: would remove target/ ($size): $why"
        exit 0
    fi
    rm -rf "$TARGET"
    echo "trim-target: removed target/ ($size): $why"
    exit 0
}

# 1. Toolchain change.
current="$(cd "$ROOT" && rustc -vV 2>/dev/null)"
if [ -n "$current" ] && [ -f "$STAMP" ] && [ "$current" != "$(cat "$STAMP")" ]; then
    remove "toolchain changed ($(head -1 "$STAMP") → $(echo "$current" | head -1))"
fi

# 2. Idle. The stamp itself is skipped so the check never counts its own write.
recent="$(find "$TARGET" -type f -mtime "-$IDLE_DAYS" ! -name .trim-rustc -print -quit 2>/dev/null)"
if [ -z "$recent" ]; then
    remove "nothing built for $IDLE_DAYS days"
fi

# 3. Size.
kb="$(du -sk "$TARGET" 2>/dev/null | cut -f1)"
if [ -n "$kb" ] && [ "$kb" -gt $((MAX_GB * 1024 * 1024)) ]; then
    remove "larger than ${MAX_GB} GB"
fi

if [ -n "$current" ] && [ "$DRY" = 0 ]; then
    printf '%s\n' "$current" > "$STAMP"
fi
say "target/ kept ($((kb / 1024 / 1024)) GB, built in the last $IDLE_DAYS days, same toolchain)"
exit 0
