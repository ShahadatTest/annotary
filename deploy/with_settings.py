#!/usr/bin/env python3
"""Run a command with the maintainer's deploy settings in its environment.

    python3 deploy/with_settings.py [--check] REQUIRED_KEY... -- COMMAND [ARG...]

The settings say where this copy of Annotary is deployed, so they stay out of the repo: one
`KEY=value` per line in ~/.config/annotary/deploy.env (ANNOTARY_DEPLOY_ENV names another file).
A variable already in the environment wins over the file, so one run can override a value. A key
this script does not know, or a line that is not KEY=value, is refused: a typo must not leave a
setting quietly unset. docs/ops-health-check.md lists the keys.

With a REQUIRED_KEY missing or empty it prints what to do and exits 2, before COMMAND runs.
--check stops there, so `make` can refuse before it builds or fetches anything.
"""
import os
import re
import sys

KNOWN = {
    "SERVER_SSH",
    "DEPLOY_SOURCE",
    "ANNOTARY_URL",
    "ANNOTARY_LINK_BASE",
    "ANNOTARY_OWNER_HANDLE",
    "ANNOTARY_OWNER_NAME",
    "ANNOTARY_TAILSCALE_SERVICE",
    "VERIFY_SNAPSHOT_GLOB",
    "VERIFY_UNIT",
}
DEFAULT_PATH = "~/.config/annotary/deploy.env"
LINE = re.compile(r"^([A-Z][A-Z0-9_]*)=(.*)$")


def read_settings(path):
    """The file's settings; {} when there is no file. Raises ValueError on a bad line."""
    try:
        with open(os.path.expanduser(path), encoding="utf-8") as f:
            text = f.read()
    except FileNotFoundError:
        return {}
    settings = {}
    for number, line in enumerate(text.splitlines(), 1):
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        match = LINE.match(line)
        if not match:
            raise ValueError("%s:%d: not KEY=value" % (path, number))
        if match.group(1) not in KNOWN:
            raise ValueError("%s:%d: unknown setting %s" % (path, number, match.group(1)))
        settings[match.group(1)] = match.group(2)
    return settings


def main(argv):
    check = "--check" in argv[:1]
    argv = argv[1:] if check else argv
    if "--" not in argv:
        sys.exit(__doc__)
    split = argv.index("--")
    required, command = argv[:split], argv[split + 1:]
    if not command and not check:
        sys.exit(__doc__)
    path = os.environ.get("ANNOTARY_DEPLOY_ENV") or DEFAULT_PATH
    try:
        settings = read_settings(path)
    except ValueError as error:
        print("annotary: %s" % error, file=sys.stderr)
        return 2
    env = {**settings, **os.environ}
    missing = [key for key in required if not env.get(key)]
    if missing:
        print("annotary: %s not set. Put %s in %s or the environment; docs/ops-health-check.md lists the settings."
              % (" and ".join(missing), "them" if len(missing) > 1 else "it", path), file=sys.stderr)
        return 2
    if check:
        return 0
    os.execvpe(command[0], command, env)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
