#!/usr/bin/env bash
# `make deploy` (project-deploy), run through deploy/with_settings.py from a clean checkout of
# origin/main on a machine that reaches `ssh $SERVER_SSH` with passwordless sudo. The Makefile then
# checks health and tags the release. Where it goes and what it installs are settings outside the
# repo (docs/ops-health-check.md, Settings):
#
#   DEPLOY_SOURCE=release   HEAD must carry a tag vX.Y.Z whose release is published: the host runs
#                           deploy/install-linux.sh vX.Y.Z, which downloads that release and checks
#                           its SHA-256. The host runs exactly what other people download.
#   DEPLOY_SOURCE=build     no release yet, or a hotfix that cannot wait for one:
#     1. the version release.py will tag this commit with (deploy/next_version.py; BUMP from
#        `make deploy BUMP=...`), stamped into the binary;
#     2. `git archive HEAD` replaces ~/.cache/annotary-deploy/src on the host, so only committed
#        files reach it; the cargo target dir beside it stays, so a rebuild is incremental;
#     3. deploy/build_release.sh builds there as the ssh user (rustup in ~/.cargo/bin) and prints
#        where the binary is;
#     4. deploy/install-linux.sh --binary, as root, installs it.
#
# Either way the installer restarts the unit and rolls back to annotary.prev if the new binary does
# not come up (its header has the steps). ANNOTARY_URL and ANNOTARY_LINK_BASE go to it, and it adds
# them to the host's root-only /etc/annotary/env if that file lacks them.
set -euo pipefail
cd "$(dirname "$0")/.."
host=${SERVER_SSH:?SERVER_SSH is not set (docs/ops-health-check.md, Settings)}
source=${DEPLOY_SOURCE:?DEPLOY_SOURCE is not set: release or build (docs/ops-health-check.md, Settings)}
ssh_() { ssh -o ConnectTimeout=10 -o BatchMode=yes "$host" "$@"; }

args=(--url "${ANNOTARY_URL:?ANNOTARY_URL is not set}" --link-base "${ANNOTARY_LINK_BASE:?ANNOTARY_LINK_BASE is not set}")
[ -z "${ANNOTARY_OWNER_HANDLE:-}" ] || args+=(--owner-handle "$ANNOTARY_OWNER_HANDLE")
[ -z "${ANNOTARY_OWNER_NAME:-}" ] || args+=(--owner-name "$ANNOTARY_OWNER_NAME")
[ -z "${ANNOTARY_TAILSCALE_SERVICE:-}" ] || args+=(--tailscale-service "$ANNOTARY_TAILSCALE_SERVICE")
remote_args=$(printf ' %q' "${args[@]}")

case "$source" in
  release)
    tag=$(git tag --points-at HEAD | grep -E '^v[0-9]+\.[0-9]+\.[0-9]+$' | sort -V | tail -n 1 || true)
    if [ -z "$tag" ]; then
      echo "deploy: DEPLOY_SOURCE=release needs a release of this commit: HEAD has no tag vX.Y.Z." >&2
      echo "deploy: push the tag, wait for the Release workflow to publish it, then deploy again." >&2
      exit 1
    fi
    echo "deploy: installing the published $tag on $host"
    # The script goes over stdin, so the host needs no checkout; the installer checks the download.
    # shellcheck disable=SC2029  # the arguments are meant to expand here
    ssh_ "sudo -n bash -s -- $tag$remote_args" < deploy/install-linux.sh
    ;;
  build)
    work='~/.cache/annotary-deploy' # expanded by the host's shell
    version=$(python3 deploy/next_version.py --bump "${BUMP:-patch}")
    echo "deploy: shipping $(git rev-parse --short HEAD) to $host as $version, built on the host"
    ssh_ "rm -rf $work/src && mkdir -p $work/src"
    git archive --format=tar HEAD | ssh_ "tar -x -C $work/src"
    new=$(ssh_ "cd $work/src && PATH=\$HOME/.cargo/bin:\$PATH CARGO_TARGET_DIR=$work/target deploy/build_release.sh $version")
    echo "deploy: built $new on $host"
    # shellcheck disable=SC2029  # the arguments are meant to expand here
    ssh_ "sudo -n bash $work/src/deploy/install-linux.sh --binary $new --src $work/src$remote_args"
    ;;
  *)
    echo "deploy: DEPLOY_SOURCE is '$source'; use release or build" >&2
    exit 1
    ;;
esac
