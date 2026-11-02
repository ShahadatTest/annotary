#!/usr/bin/env python3
"""Tag a shipped commit with the next version in its series and create the GitHub Release.

    python3 scripts/standard/release.py --prefix v|server-v|web-v|ios-v|mac-v [--bump patch|minor|major]
        [--asset <file>]...

Runs after a deploy whose health check passed, a TestFlight upload, or a notarised Mac build,
whose zipped app goes onto the release as an --asset. The next version
is the highest `<prefix>X.Y.Z` tag plus one patch (or minor, or major). If HEAD already carries
a tag in the series (a re-deploy or a rebuild), nothing new is tagged: the tag is pushed and its
release made if an earlier run stopped before either, and with --asset the assets replace the
old ones. A failed push or `gh` step exits non-zero, so a re-run finishes the job. The release
notes are GitHub's, generated from the previous tag in the same series. Stdlib only; runs on
Python 3.9.
"""
import argparse
import os
import re
import subprocess
import sys


def git(*args):
    return subprocess.run(["git", *args], check=True, capture_output=True, text=True).stdout.strip()


def series_tags(prefix):
    pattern = re.compile(r"^" + re.escape(prefix) + r"(\d+)\.(\d+)\.(\d+)$")
    tags = []
    for name in git("tag", "--list").splitlines():
        m = pattern.match(name)
        if m:
            tags.append((tuple(int(x) for x in m.groups()), name))
    return sorted(tags)


def next_version(current, bump):
    major, minor, patch = current
    if bump == "major":
        return (major + 1, 0, 0)
    if bump == "minor":
        return (major, minor + 1, 0)
    return (major, minor, patch + 1)


def create_release(tag, previous, assets):
    """The GitHub release of `tag`, its notes generated from `previous` in the same series."""
    cmd = ["gh", "release", "create", tag, "--title", tag, "--generate-notes", "--verify-tag"]
    if previous:
        cmd += ["--notes-start-tag", previous]
    gh(cmd + assets, "make the release of %s" % tag)


def push_tag(remote, tag):
    """Push `tag`; on failure (a pre-push hook refusing it, say) stop under git's own message with
    how to finish, so the deploy fails cleanly and a re-run pushes the tag already made."""
    code = subprocess.run(["git", "push", "--quiet", remote, "refs/tags/" + tag]).returncode
    if code != 0:
        sys.exit("❌ could not push %s to %s (git exit %d); run this again to finish" % (tag, remote, code))


def gh(cmd, what):
    """Run `gh`; on failure stop with what it was doing, so the deploy fails and a re-run finishes it."""
    code = subprocess.run(cmd).returncode
    if code != 0:
        sys.exit("❌ could not %s (gh exit %d); the tag is pushed, so run this again to finish" % (what, code))


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--prefix", required=True, help="tag series prefix, e.g. v or server-v")
    parser.add_argument("--bump", default="patch", choices=["patch", "minor", "major"])
    parser.add_argument("--remote", default="origin")
    parser.add_argument("--asset", action="append", default=[],
                        help="a file to attach to the release (repeatable)")
    args = parser.parse_args(argv)
    if not re.match(r"^([a-z]+-)?v$", args.prefix):
        parser.error("prefix must be v or <name>-v")
    for path in args.asset:
        if not os.path.isfile(path):
            parser.error("no asset at %s; nothing tagged" % path)

    git("fetch", "--quiet", "--tags", args.remote)
    tags = series_tags(args.prefix)
    on_head = set(git("tag", "--points-at", "HEAD").splitlines())
    for i, (_, name) in enumerate(tags):
        if name in on_head:
            # An earlier run may have tagged and stopped before the push or the release.
            push_tag(args.remote, name)
            done = []
            if subprocess.run(["gh", "release", "view", name], capture_output=True).returncode != 0:
                create_release(name, tags[i - 1][1] if i else None, [])
                done.append("made its missing release")
            if args.asset:
                gh(["gh", "release", "upload", name, "--clobber"] + args.asset, "attach the assets to %s" % name)
                done.append("attached %s" % ", ".join(args.asset))
            print("HEAD is already released as %s; %s" % (name, " and ".join(done) or "nothing to tag"))
            return 0

    previous = tags[-1][1] if tags else None
    version = next_version(tags[-1][0] if tags else (0, 0, 0), args.bump)
    tag = args.prefix + ".".join(str(x) for x in version)
    git("tag", "-a", tag, "-m", tag)
    push_tag(args.remote, tag)
    create_release(tag, previous, args.asset)
    print("released %s%s" % (tag, " (after %s)" % previous if previous else ""))
    return 0


if __name__ == "__main__":
    sys.exit(main())
