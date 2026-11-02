#!/usr/bin/env bash
# CI's gitleaks, pinned by GITLEAKS_VERSION, GITLEAKS_PLATFORM (the release's tarball for the
# runner: linux_x64 on a Linux runner, the default, or darwin_arm64 on a Mac) and GITLEAKS_SHA256
# (that tarball's sha256). The runner's disk persists between jobs, so the tarball is fetched
# once per runner and version into $RUNNER_TOOL_CACHE/gitleaks/<version>/ and checked against the
# pin every time it is used. A new version is a new folder, so a bump always fetches and
# verifies the new tarball.
# Usage: gitleaks.sh check   (writes hit=true or hit=false to $GITHUB_OUTPUT)
#        gitleaks.sh fetch   (download, verify, then one rename into the cache)
#        gitleaks.sh scan    (verify a copy of the cached tarball, fetching it once more if it
#                             is missing or does not match, extract it, put it on the job's
#                             PATH for the later steps, scan the history)
set -euo pipefail

: "${GITLEAKS_VERSION:?}" "${GITLEAKS_SHA256:?}" "${RUNNER_TOOL_CACHE:?}" "${RUNNER_TEMP:?}"
name="gitleaks_${GITLEAKS_VERSION}_${GITLEAKS_PLATFORM:-linux_x64}.tar.gz"
url="https://github.com/gitleaks/gitleaks/releases/download/v${GITLEAKS_VERSION}/$name"
dir="$RUNNER_TOOL_CACHE/gitleaks/$GITLEAKS_VERSION"
cached="$dir/$name"

sha256() {
  if command -v sha256sum >/dev/null; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1
}

verified() {
  [ -f "$1" ] && [ "$(sha256 "$1")" = "$GITLEAKS_SHA256" ]
}

# Several jobs may fetch at once: each writes its own temporary file in the cache folder (the same
# filesystem), and the rename replaces the cached file whole, never part-written.
fetch() {
  mkdir -p "$dir"
  # A job killed mid-download (SIGKILL skips the trap) leaves its temporary file; one older than
  # a day is no live download's.
  find "$dir" -maxdepth 1 -type f -name ".$name.*" -mmin +1440 -delete
  tmp=$(mktemp "$dir/.$name.XXXXXX")
  trap 'rm -f "$tmp"' EXIT
  curl -sSfL -o "$tmp" "$url"
  if ! verified "$tmp"; then
    echo "gitleaks: $url does not match GITLEAKS_SHA256" >&2
    exit 1
  fi
  mv -f "$tmp" "$cached"
  # The name is free again, and another job's mktemp may take it.
  trap - EXIT
  echo "gitleaks $GITLEAKS_VERSION: fetched and verified into $dir"
}

case "${1:-}" in
  check)
    hit=false
    if verified "$cached"; then hit=true; fi
    echo "gitleaks $GITLEAKS_VERSION in $dir: $([ "$hit" = true ] && echo "verified, reused" || echo "missing or not matching, fetching")"
    echo "hit=$hit" >> "${GITHUB_OUTPUT:-/dev/null}"
    ;;
  fetch)
    fetch
    ;;
  scan)
    # Verify the copy that is extracted, so nothing can change it between the check and tar.
    # A cached tarball removed or changed since check is fetched once more, against the same pin.
    copy="$RUNNER_TEMP/$name"
    if ! { [ -f "$cached" ] && cp "$cached" "$copy" && verified "$copy"; }; then
      echo "gitleaks: $cached is missing or does not match GITLEAKS_SHA256; fetching it again" >&2
      fetch
      cp "$cached" "$copy"
      if ! verified "$copy"; then
        echo "gitleaks: $cached does not match GITLEAKS_SHA256; run gitleaks.sh fetch" >&2
        exit 1
      fi
    fi
    bin="$RUNNER_TEMP/gitleaks-bin"
    mkdir -p "$bin"
    tar -xzf "$copy" -C "$bin" gitleaks
    rm -f "$copy"
    # The later steps (a hook test that runs `gitleaks`) find this one by name.
    echo "$bin" >> "${GITHUB_PATH:-/dev/null}"
    "$bin/gitleaks" git --redact --no-banner
    ;;
  *)
    echo "usage: gitleaks.sh check|fetch|scan" >&2
    exit 2
    ;;
esac
