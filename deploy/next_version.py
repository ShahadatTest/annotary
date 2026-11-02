#!/usr/bin/env python3
"""Print the version `make deploy` is about to tag HEAD with, without the prefix (e.g. 0.1.3).

    python3 deploy/next_version.py [--bump patch|minor|major] [--remote origin]

deploy/deploy.sh builds the binary with this as ANNOTARY_VERSION, so the binary reports the
tag scripts/standard/release.py gives it after the health check (CMD-2). It uses release.py's own
series_tags() and next_version(), so the two can never compute different numbers: HEAD's tag in
the `v` series if it has one (a re-deploy), otherwise the highest `vX.Y.Z` plus the bump. Stdlib
only; runs on Python 3.9.
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "scripts", "standard"))
import release  # noqa: E402  (the template's tagger; never edited in this repo)

PREFIX = "v"


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bump", default="patch", choices=["patch", "minor", "major"])
    parser.add_argument("--remote", default="origin")
    args = parser.parse_args(argv)

    release.git("fetch", "--quiet", "--tags", args.remote)
    tags = release.series_tags(PREFIX)
    on_head = set(release.git("tag", "--points-at", "HEAD").splitlines())
    for numbers, name in tags:
        if name in on_head:
            print(".".join(str(x) for x in numbers))
            return 0
    version = release.next_version(tags[-1][0] if tags else (0, 0, 0), args.bump)
    print(".".join(str(x) for x in version))
    return 0


if __name__ == "__main__":
    sys.exit(main())
