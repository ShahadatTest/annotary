#!/usr/bin/env python3
"""Write a release tag's version into the workspace, in a release job's checkout only.

    python3 deploy/stamp_version.py vX.Y.Z

The repo keeps `0.0.0` (the shipped version comes from the tag), but cargo-dist will only build a tag
that matches the package's version, so .github/workflows/release.yml runs this before each dist call.
Nothing it writes is ever committed. Stamping the manifest also stamps the binary: with no
ANNOTARY_VERSION, `crate::VERSION` is CARGO_PKG_VERSION.
"""
import os
import re
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
TAG = re.compile(r"^v(\d+\.\d+\.\d+)$")
PLACEHOLDER = '"0.0.0"'
MANIFEST_LINE = re.compile(r'(\[workspace\.package\]\n(?:[^\[\n][^\n]*\n|\n)*?version = )"0\.0\.0"')
LOCK_ENTRY = re.compile(r'(\[\[package\]\]\nname = "annotary(?:-[a-z]+)?"\nversion = )"0\.0\.0"')


def stamp(root, tag):
    """Stamp `root`'s Cargo.toml and Cargo.lock with `tag`'s version; ValueError if it cannot."""
    match = TAG.match(tag)
    if not match:
        raise ValueError("%r is not vX.Y.Z" % tag)
    version = '"%s"' % match.group(1)
    with open(os.path.join(root, "Cargo.toml"), encoding="utf-8") as f:
        manifest = f.read()
    manifest, manifests = MANIFEST_LINE.subn(lambda m: m.group(1) + version, manifest)
    if manifests != 1:
        raise ValueError("Cargo.toml has no [workspace.package] version = %s to stamp" % PLACEHOLDER)
    with open(os.path.join(root, "Cargo.lock"), encoding="utf-8") as f:
        lock = f.read()
    lock, locks = LOCK_ENTRY.subn(lambda m: m.group(1) + version, lock)
    if not locks:
        raise ValueError("Cargo.lock has no workspace crate at version %s to stamp" % PLACEHOLDER)
    with open(os.path.join(root, "Cargo.toml"), "w", encoding="utf-8") as f:
        f.write(manifest)
    with open(os.path.join(root, "Cargo.lock"), "w", encoding="utf-8") as f:
        f.write(lock)
    return locks


def main(argv):
    if len(argv) != 1:
        sys.exit(__doc__)
    try:
        count = stamp(ROOT, argv[0])
    except ValueError as error:
        sys.exit("stamp_version: %s" % error)
    print("stamped %s into Cargo.toml and %d crates in Cargo.lock" % (argv[0], count))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
