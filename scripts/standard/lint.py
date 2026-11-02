#!/usr/bin/env python3
"""Check a repo against the standard's file rules. Exit 1 on any finding.

    python3 scripts/standard/lint.py [repo-root]

Checks: the Makefile's pipefail shell line; no Python embedded in committed shell scripts; no
hand-set app versions; instruction and rule file sizes; documents at the root; the pre-commit
and pre-push hooks; no GitHub-hosted macOS runner in a workflow; no GitHub cache in a workflow or
composite action; broken links in active docs, missing paths in README.md, AGENTS.md and the
rules, and missing make targets in README.md and AGENTS.md (the rest of docs/ is the weekly
documentation sweep's; checking it here failed most repos on design notes that name planned files);
the recorded standard version is the latest repo-standard tag.
A public repo (`visibility: public` in .agents/standard.yml) runs on GitHub-hosted runners, so
the runner and cache checks are off there, its open-source documents may sit at the root, and a
version lookup that cannot read the private template only warns, in CI too. It records the server
host `generic`, whatever host it runs on: the repo is published, and an answers file edited by hand
reaches no Copier check until the next update.
Reads tracked and untracked (not ignored) files only; the docs check reads tracked docs only.
Stdlib only; runs on Python 3.9.
"""
import json
import os
import re
import subprocess
import sys
import tempfile
import time

PIPEFAIL_LINE = "SHELL := /bin/bash -o pipefail"
FILE_LIMIT = 24000          # bytes per instruction, rule or skill file (Antigravity's cap)
SESSION_LIMIT = 60000       # bytes loaded at session start, about 15k tokens of the 20k cap
ROOT_DOCS = {"AGENTS.md", "CLAUDE.md", "GEMINI.md", "DESIGN.md", "README.md", "CONTRIBUTING.md"}
PUBLIC_ROOT_DOCS = ROOT_DOCS | {"SECURITY.md", "CODE_OF_CONDUCT.md"}  # where GitHub's community profile looks
# `python3 -c`, `python3 <<EOF`, and `python3 - [args] <<EOF` (the script on stdin, arguments
# before the redirect), each after any flags; `python3 tool.py <<EOF` feeds a real file its input.
EMBEDDED_PYTHON = re.compile(r"\bpython[0-9.]*(?:\s+-[A-Za-z]+)*(?:\s+-c\b|\s+-(?:\s+[^\s<>|;&]+)*\s*<<|\s*<<)")
PLACEHOLDER_VERSIONS = {"0", "0.0", "0.0.0", "1"}


class NotARepo(Exception):
    pass


def is_public(root):
    """True when `.agents/standard.yml` records `visibility: public`; any other value, or none, is
    a private repo, which keeps every check."""
    path = os.path.join(root, ".agents", "standard.yml")
    if not os.path.isfile(path):
        return False
    return any(re.match(r"^visibility:\s*['\"]?public['\"]?\s*$", line) for line in read(root, ".agents/standard.yml").splitlines())


def files(root):
    result = subprocess.run(
        ["git", "-C", root, "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
        capture_output=True)
    if result.returncode != 0:
        raise NotARepo(root)
    out = result.stdout.decode()
    return sorted(p for p in out.split("\0") if p and os.path.isfile(os.path.join(root, p)))


def read(root, path):
    with open(os.path.join(root, path), encoding="utf-8", errors="replace") as f:
        return f.read()


def is_shell(root, path):
    if path.endswith((".sh", ".bash", ".zsh")) or path.startswith(".githooks/"):
        return True
    try:
        with open(os.path.join(root, path), "rb") as f:
            first = f.readline(200)
    except OSError:
        return False
    return first.startswith(b"#!") and re.search(rb"\b(ba|z)?sh\b", first) is not None


def check_makefile(root, paths, problems):
    if "Makefile" not in paths:
        return
    lines = [l.strip() for l in read(root, "Makefile").splitlines()]
    if PIPEFAIL_LINE not in lines:
        problems.append("Makefile: missing `%s` (a failing pipe would pass)" % PIPEFAIL_LINE)


def check_shell(root, paths, problems):
    for p in paths:
        if not is_shell(root, p):
            continue
        for n, line in enumerate(read(root, p).splitlines(), 1):
            if line.lstrip().startswith("#"):
                continue
            if EMBEDDED_PYTHON.search(line):
                problems.append("%s:%d: Python embedded in a shell script; move it to a sibling .py file" % (p, n))


def check_versions(root, paths, problems):
    def hand_set(value):
        value = value.strip().strip("\"'")
        return value and not value.startswith("$") and value not in PLACEHOLDER_VERSIONS

    for p in paths:
        name = os.path.basename(p)
        if name == "project.yml" or p.endswith(".xcconfig"):
            for n, line in enumerate(read(root, p).splitlines(), 1):
                m = re.match(r"\s*(MARKETING_VERSION|CURRENT_PROJECT_VERSION)\s*[:=]\s*(\S+)", line)
                if m and hand_set(m.group(2)):
                    problems.append("%s:%d: %s is set by hand (%s); the version comes from the git tag"
                                    % (p, n, m.group(1), m.group(2)))
        elif name == "Info.plist":
            text = read(root, p)
            for key in ("CFBundleShortVersionString", "CFBundleVersion"):
                m = re.search(r"<key>%s</key>\s*<string>([^<]*)</string>" % key, text)
                if m and hand_set(m.group(1)):
                    problems.append("%s: %s is set by hand (%s); the version comes from the git tag"
                                    % (p, key, m.group(1)))
        elif name == "package.json" and "node_modules/" not in p:
            try:
                data = json.loads(read(root, p))
            except ValueError:
                continue
            version = str(data.get("version", ""))
            if not data.get("private") and hand_set(version):
                problems.append("%s: version %s is set by hand on a published package; "
                                "the version comes from the git tag" % (p, version))
        elif name in ("build.gradle", "build.gradle.kts"):
            for n, line in enumerate(read(root, p).splitlines(), 1):
                m = re.match(r"\s*(versionName|versionCode)\s*=?\s*(\S+)", line)
                if m and hand_set(m.group(2)):
                    problems.append("%s:%d: %s is set by hand (%s); the version comes from the git tag"
                                    % (p, n, m.group(1), m.group(2)))


def check_sizes(root, paths, problems):
    at_start = [p for p in paths
                if p in ("AGENTS.md", "CLAUDE.md", "GEMINI.md")
                or (p.startswith(".agents/rules/") and p.endswith(".md"))]
    sized = at_start + [p for p in paths if p.startswith(".agents/skills/") and p.endswith(".md")]
    for p in sized:
        size = os.path.getsize(os.path.join(root, p))
        if size >= FILE_LIMIT:
            problems.append("%s: %d bytes; keep instruction files under %d" % (p, size, FILE_LIMIT))
    total = sum(os.path.getsize(os.path.join(root, p)) for p in at_start)
    if total >= SESSION_LIMIT:
        problems.append("instruction files loaded at session start total %d bytes; keep them under %d"
                        % (total, SESSION_LIMIT))


def check_root(root, paths, problems):
    allowed = PUBLIC_ROOT_DOCS if is_public(root) else ROOT_DOCS
    for p in paths:
        if "/" not in p and p.lower().endswith((".md", ".txt", ".pdf")) and p not in allowed:
            problems.append("%s: documents other than %s go in docs/" % (p, ", ".join(sorted(allowed))))


def check_public_host(root, paths, problems):
    if not is_public(root):
        return
    for line in read(root, ".agents/standard.yml").splitlines():
        host = re.match(r"^server_host:\s*['\"]?([^'\"\s]+)", line)
        if host and host.group(1) != "generic":
            problems.append(".agents/standard.yml: server_host is %s, but a public repo names no host; update "
                            "with -d server_host=generic and set SERVER_SSH outside the repo" % host.group(1))


HOOKS = {"pre-commit": "it runs gitleaks on staged changes",
         "pre-push": "it runs gitleaks on the commits each push sends"}


def check_hook(root, paths, problems):
    for name, purpose in HOOKS.items():
        path = ".githooks/" + name
        hook = os.path.join(root, ".githooks", name)
        if not os.path.isfile(hook):
            problems.append("%s: missing (%s)" % (path, purpose))
        elif not os.access(hook, os.X_OK):
            problems.append("%s: not executable" % path)
        elif "gitleaks" not in read(root, path):
            problems.append("%s: does not run gitleaks" % path)


def check_workflows(root, paths, problems):
    if is_public(root):
        return  # a public repo's jobs run on GitHub-hosted runners, never the home ones
    for p in paths:
        if p.startswith(".github/workflows/") and p.endswith((".yml", ".yaml")):
            for n, line in enumerate(read(root, p).splitlines(), 1):
                # Hosted labels are macos-<version>; a self-hosted macOS runner's label is macOS.
                if re.match(r"\s*runs-on:.*\bmacos-", line, re.I):
                    problems.append("%s:%d: GitHub-hosted macOS runner; private repos use the home runners" % (p, n))


# ── GitHub's cache (C9) ──────────────────────────────────────────────────────────────
# A home runner's disk persists, so GitHub's cache only adds a download and can restore an older
# toolchain over a newer one. A tool a job needs is on the host or in a per-runner folder.

CACHE_ACTION = re.compile(r"^(actions/cache|swatinem/rust-cache)(/|@|$)", re.I)
SETUP_ACTION = re.compile(r"^[\w.-]+/setup-[\w.-]+(/[^@]*)?(@|$)", re.I)
CACHE_INPUTS = ("cache", "package-manager-cache", "enable-cache")  # setup-node/python/go/java…, setup-uv
CACHE_OFF = {"false", ""}
SETUP_GO_CACHES = re.compile(r"^actions/setup-go@(?!v[1-3]\b)", re.I)  # v4 and later cache unless told not to
LIST_ITEM = re.compile(r"^(\s*)-(?:\s+|$)")
KEY_VALUE = re.compile(r"^([\w-]+):(?:\s+(.*))?$")


def scalar(value):
    value = re.sub(r"\s+#.*$", "", value or "").strip()
    return value[1:-1] if len(value) >= 2 and value[0] == value[-1] and value[0] in "'\"" else value


def yaml_items(text):
    """(line number, keys, `with:` inputs) for each block-list item: a line-based reading of block
    YAML, enough for the steps of a workflow or composite action (stdlib only, no PyYAML)."""
    lines = text.splitlines()
    for start, line in enumerate(lines):
        m = LIST_ITEM.match(line)
        if not m:
            continue
        body = [(m.end(), line[m.end():].strip())] if line[m.end():].strip() else []
        for later in lines[start + 1:]:
            if not later.strip() or later.lstrip().startswith("#"):
                continue
            col = len(later) - len(later.lstrip())
            if col <= len(m.group(1)):
                break
            body.append((col, later.strip()))
        if not body:
            continue
        key_col, with_col = body[0][0], None
        keys, inputs, in_with = {}, {}, False
        for col, entry in body:
            kv = KEY_VALUE.match(entry)
            if col == key_col:
                in_with, with_col = bool(kv) and kv.group(1) == "with", None
                if kv:
                    keys[kv.group(1)] = scalar(kv.group(2))
                if in_with and keys["with"].startswith("{"):
                    inputs.update((k, scalar(v)) for k, v in re.findall(r"([\w-]+)\s*:\s*([^,}]*)", keys["with"]))
            elif in_with and kv and col == (with_col or col):
                with_col = col
                inputs[kv.group(1)] = scalar(kv.group(2))
        yield start + 1, keys, inputs


def check_cache(root, paths, problems):
    if is_public(root):
        return  # a hosted runner starts empty; GitHub's cache is how a public repo's jobs keep tools
    for p in paths:
        workflow = p.startswith(".github/workflows/") and p.endswith((".yml", ".yaml"))
        action = p.startswith(".github/actions/") and os.path.basename(p) in ("action.yml", "action.yaml")
        if not (workflow or action):
            continue
        for n, keys, inputs in yaml_items(read(root, p)):
            uses = keys.get("uses", "")
            action_name = uses.split("@", 1)[0]
            why = None
            if CACHE_ACTION.match(uses):
                why = action_name
            elif SETUP_ACTION.match(uses):
                on = [k for k in CACHE_INPUTS if k in inputs and inputs[k].lower() not in CACHE_OFF]
                if on:
                    why = "`%s: %s` on %s" % (on[0], inputs[on[0]], action_name)
                elif SETUP_GO_CACHES.match(uses) and "cache" not in inputs:
                    why = "%s caches unless it has `cache: false`" % action_name
            if why:
                problems.append("%s:%d: step '%s' uses GitHub's cache (%s); on the home runners a tool is on the "
                                "host or in a per-runner folder (C9)" % (p, n, keys.get("name") or uses, why))


# ── docs ─────────────────────────────────────────────────────────────────────────────
# Ported from the docs sweep's scanner and calibrated against every repo on the standard after
# the first documentation sweep (2026-10-19). Each skip below stopped a false positive there.

NOT_ACTIVE = re.compile(r"^docs/(archive|design)/|(^|/)(node_modules|vendor|\.venv)/|\.jinja$|"
                        r"(^|/)(fixtures|seed|demo)/", re.I)
TEST_DIR = re.compile(r"^(?!docs/)(.*/)?(tests?|e2e)/[^/]+/")  # Markdown below a test folder is test input
TEST_SOURCE = (":(glob)**/tests/**", ":(glob)**/test/**", ":(glob)**/e2e/**", ":(glob)**/benches/**",
               ":(glob)**/*_test.*", ":(glob)**/*Tests.swift", ":(glob)**/*.test.*",
               ":(exclude)*.md")
READS_FILE = re.compile(r"include_str!|include_bytes!|read_to_string|read_dir|fs::read|read_text|open\(|"
                        r"Path\(|PathBuf|join\(|contentsOf|readFile|fixture", re.I)
LINK_TITLE = r"(?:\s+(?:\"[^\"]*\"|'[^']*'|\([^)]*\)))?"
INLINE_LINK = re.compile(r"!?\[[^\]]*\]\(\s*(?:<([^>\n]+)>|([^)\s]+))" + LINK_TITLE + r"\s*\)")
REF_LINK = re.compile(r"^\s*\[[^\]]+\]:\s*(?:<([^>\n]+)>|(\S+))")
CODE_SPAN = re.compile(r"`([^`\n]+)`")
EXTERNAL = re.compile(r"^[a-z][a-z0-9+.-]*:", re.I)  # http:, mailto:, t3-thread:, conversation: …
DATED_ROW = re.compile(r"^\s*(?:\|\s*|[-*+]\s+)\**\s*20\d\d-\d\d-\d\d\b")  # a dated table row or list item
BLOCK_START = re.compile(r"^\s*(?:[-*+]\s|\d+[.)]\s|\||#|>)")
# A sentence that names another repo ("in the other-project repo", "the parent repository", "the
# kit checkout's") is about that repo's paths and targets, not this one's.
OTHER_REPO = re.compile(r"(?i)(?<![\w`'-])`?([\w.-]+)`?\s+(?:repo|repository|checkout)\b")
NOT_A_REPO_NAME = {"this", "the", "a", "an", "our", "its", "each", "every", "your", "any", "one", "same", "own",
                   "new", "which", "whole", "main", "template", "local", "git", "github", "private", "public",
                   "fresh", "current", "clean", "my", "his", "her", "their"}
OTHER_REPO_WORD = re.compile(r"(?i)\b(?:parent|other|another|sibling|that)\s+(?:repo|repository|checkout)\b")


def names_other_repo(sentence, own_names):
    """True when a sentence is about another repo's paths: "in the other-project repo", "the parent
    repository", "the kit checkout's". The repo's own name and generic words do not count."""
    if OTHER_REPO_WORD.search(sentence):
        return True
    return any(m.group(1).lower() not in NOT_A_REPO_NAME | own_names for m in OTHER_REPO.finditer(sentence))


# `make a b`, `make deploy BUMP=minor`, `PERF=0 make check`: each target is checked. A span with
# anything else after `make` (a flag, a placeholder, `-C` into another folder) is left alone.
MAKE_REF = re.compile(r"^(?:[A-Za-z_][A-Za-z0-9_]*=\S*\s+)*make((?:\s+(?:[a-z][a-z0-9_-]*|[A-Za-z_][A-Za-z0-9_]*=\S*))+)$")
ASSIGNMENT = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*=")
MAKE_TARGET = re.compile(r"^([A-Za-z][\w.-]*(?:\s+[A-Za-z][\w.-]*)*)\s*::?(?!=)")


def tracked(root):
    """The tracked paths, plus the paths reachable through a tracked symlink to a folder."""
    out = subprocess.run(["git", "-C", root, "ls-files", "--cached", "-s", "-z"],
                         capture_output=True).stdout.decode("utf-8", "replace")
    files, links = set(), {}
    for entry in out.split("\0"):
        meta, _, path = entry.partition("\t")
        if not path:
            continue
        files.add(path)
        if meta.startswith("120000 ") and os.path.islink(os.path.join(root, path)):
            target = os.readlink(os.path.join(root, path))
            resolved = os.path.normpath(os.path.join(os.path.dirname(path), target))
            if not os.path.isabs(target) and not resolved.startswith(".."):
                links[path] = resolved
    for link, target in links.items():
        prefix = target + "/"
        files |= {link + "/" + f[len(prefix):] for f in list(files) if f.startswith(prefix)}
    return files


def dirs_of(files):
    dirs = set()
    for f in files:
        parts = f.split("/")[:-1]
        for i in range(1, len(parts) + 1):
            dirs.add("/".join(parts[:i]))
    return dirs


def answers(root):
    """(the starter files `.agents/standard.yml` says the repo deleted on purpose, the repo's names:
    its folder and its project_name, in lower case)."""
    names = {os.path.basename(os.path.abspath(root)).lower()}
    if not os.path.isfile(os.path.join(root, ".agents", "standard.yml")):
        return set(), names
    removed, inside = set(), False
    for line in read(root, ".agents/standard.yml").splitlines():
        m = re.match(r"^project_name:\s*['\"]?([^'\"\s]+)", line)
        if m:
            names.add(m.group(1).lower())
        if re.match(r"^removed_starter_files:", line):
            inside = True
        elif inside and re.match(r"^\s*-\s*", line):
            removed.add(re.sub(r"^\s*-\s*", "", line).strip().strip("'\""))
        elif inside and line.strip():
            inside = False
    return removed, names


def test_inputs(root, docs):
    """Markdown a test reads as data: below a test folder, or read by test code by path or as a
    folder (`include_str!("../docs/x/v1.md")`, `.join("docs/x")`). It is not documentation."""
    data = {d for d in docs if TEST_DIR.search(os.path.dirname(d) + "/")}
    nested = [d for d in docs if "/" in d and d not in data]
    if not nested:
        return data
    folders = {os.path.dirname(d) for d in nested if d.count("/") >= 2}
    result = subprocess.run(["git", "-C", root, "grep", "-h", "-I", "-F"]
                            + [a for p in sorted(set(nested) | folders) for a in ("-e", p)]
                            + ["--"] + list(TEST_SOURCE), capture_output=True)
    text = "\n".join(l for l in result.stdout.decode("utf-8", "replace").splitlines() if READS_FILE.search(l))
    read_folders = {f for f in folders if re.search(re.escape(f) + r"/?[\"'`)]", text)}
    return data | {d for d in nested if d in text or any(d.startswith(f + "/") for f in read_folders)}


def prose_lines(text):
    """(line number, line) outside fenced code blocks and HTML comments."""
    fence = None
    in_comment = False
    for n, line in enumerate(text.split("\n"), 1):
        stripped = line.lstrip()
        if in_comment:
            if "-->" not in line:
                continue
            in_comment = False
            line = line.split("-->", 1)[1]
            stripped = line.lstrip()
        if fence:
            # a fence closes on a run of its own character at least as long, with nothing after it
            if re.match(re.escape(fence[0]) + "{%d,}\\s*$" % len(fence), stripped):
                fence = None
            continue
        m = re.match(r"(`{3,}|~{3,})", stripped)
        if m:
            fence = m.group(1)
            continue
        line = re.sub(r"<!--.*?-->", "", line)
        if "<!--" in line:
            line = line.split("<!--", 1)[0]
            in_comment = True
        yield n, line


def spans(text):
    """(line number, inline code span, the sentence around it) in prose. A sentence may wrap lines;
    a blank line, list item, table row, heading or quote starts a new one."""
    blocks, block = [], []
    for n, line in prose_lines(text):
        if not line.strip() or BLOCK_START.match(line):
            if block:
                blocks.append(block)
            block = [(n, line)] if line.strip() else []
        else:
            block.append((n, line))
    if block:
        blocks.append(block)
    for block in blocks:
        joined, starts = "", []
        for n, line in block:
            starts.append((len(joined), n))
            joined += line.strip() + " "
        bounds = [0] + [m.end() for m in re.finditer(r"[.!?]\s+(?=[A-Z`*(\[])", joined)] + [len(joined)]
        for m in CODE_SPAN.finditer(joined):
            n = max(line_no for start, line_no in starts if start <= m.start())
            lo = max(b for b in bounds if b <= m.start())
            hi = min(b for b in bounds if b >= m.end())
            yield n, m.group(1), joined[lo:hi]


def ignored_paths(root, paths):
    """The paths the repo's .gitignore excludes: runtime files and build output a doc may name."""
    if not paths:
        return set()
    asked = [q for p in paths for q in (p, p + "/")]  # a `dir/` pattern only matches a path asked as a folder
    result = subprocess.run(["git", "-C", root, "check-ignore", "--no-index", "--stdin", "-z"],
                            input="\0".join(asked).encode(), capture_output=True)
    return {p.rstrip("/") for p in result.stdout.decode("utf-8", "replace").split("\0") if p}


def check_links(doc, text, files, dirs, problems):
    from urllib.parse import unquote
    for n, line in prose_lines(text):
        bare = CODE_SPAN.sub("", line)
        targets = [m.group(1) or m.group(2) for m in INLINE_LINK.finditer(bare)]
        m = REF_LINK.match(bare)
        if m:
            targets.append(m.group(1) or m.group(2))
        for target in targets:
            if target.lower().startswith("file://"):
                problems.append("%s:%d: local file:// link (%s) breaks across checkouts; use a repo-relative path"
                                % (doc, n, target))
                continue
            if target.startswith("#") or EXTERNAL.match(target):
                continue
            path = unquote(target.split("#", 1)[0].split("?", 1)[0])
            path = path.lstrip("/") if path.startswith("/") else os.path.join(os.path.dirname(doc), path)
            path = os.path.normpath(path)
            if path in (".", "") or path.startswith(".."):
                continue
            if path.startswith("docs/archive/"):
                problems.append("%s:%d: active doc links into docs/archive/ (%s); active docs must not depend "
                                "on archived files" % (doc, n, target))
            elif path not in files and path not in dirs:
                problems.append("%s:%d: broken link to %s" % (doc, n, target))


def documented_paths(text, top_dirs, own_names):
    """(line number, token) for each inline code span in this repo's prose that names a repo path:
    it starts with a top-level folder the repo has, and its sentence names no other repo."""
    lines = text.split("\n")
    for n, span, sentence in spans(text):
        if DATED_ROW.match(lines[n - 1]):
            continue  # a dated log row records the path as it was then
        tok = re.sub(r"((?:::\w+)+|(?::\d+)+(?:-\d+)?)$", "", span.strip()).split("#", 1)[0]
        if "/" not in tok or re.search(r"[\s*<>{}$…]", tok) or tok.startswith(("~", "/")) or EXTERNAL.match(tok):
            continue
        while tok.startswith("./"):
            tok = tok[2:]
        if tok.split("/", 1)[0] in top_dirs and not names_other_repo(sentence, own_names):
            yield n, tok


def check_paths(root, doc, text, files, dirs, removed, own_names, problems):
    known = files | dirs
    removed = removed | dirs_of(removed)
    suffixes = {}
    for p in known:
        suffixes.setdefault(p.rsplit("/", 1)[-1], []).append(p)
    missing = []
    for n, tok in documented_paths(text, {d for d in dirs if "/" not in d}, own_names):
        clean = tok.rstrip("/")
        if doc == ".agents/rules/standard.md" and (clean == "docs/decisions.md" or clean.startswith("docs/plans")):
            continue  # the standard names where these go before a repo has its first one
        if {clean, os.path.normpath(os.path.join(os.path.dirname(doc), clean))} & known or clean in removed:
            continue
        # shorthand for one longer tracked path (`server/error.rs` for `crates/x/src/server/error.rs`)
        if len([p for p in suffixes.get(clean.rsplit("/", 1)[-1], []) if p.endswith("/" + clean)]) == 1:
            continue
        missing.append((n, tok, clean))
    ignored = ignored_paths(root, sorted({c for _, _, c in missing}))
    for n, tok, clean in missing:
        if clean not in ignored:
            problems.append("%s:%d: documented path '%s' does not exist" % (doc, n, tok))


def make_targets(root, files):
    makefiles = [f for f in ("Makefile", "project.mk") if f in files]
    makefiles += sorted(f for f in files if re.match(r"^scripts/[^/]+/[^/]+\.mk$", f))
    targets, seen = set(), set()
    while makefiles:
        mf = makefiles.pop(0)
        if mf in seen or not os.path.isfile(os.path.join(root, mf)):
            continue
        seen.add(mf)
        in_define = False
        for line in read(root, mf).splitlines():
            inc = re.match(r"^-?include\s+(.+)$", line) or re.match(r"^sinclude\s+(.+)$", line)
            if inc and not in_define:
                # a literal included file, read from the repo root as make does; `$(…)` names are skipped
                makefiles += [os.path.normpath(f) for f in inc.group(1).split()
                              if "$" not in f and os.path.normpath(f) in files]
                continue
            if line.startswith("define "):
                in_define = True
            elif line.startswith("endef"):
                in_define = False
            if in_define or line.startswith("\t"):
                continue
            m = MAKE_TARGET.match(line)
            head = line.split(":")[0]
            if m and "%" not in head and "$(" not in head:
                targets.update(m.group(1).split())
    return targets


def make_refs(span):
    """The make targets a code span runs: [] when it is not a plain `make` line."""
    m = MAKE_REF.match(span.strip())
    return [w for w in m.group(1).split() if not ASSIGNMENT.match(w)] if m else []


def check_make(doc, text, targets, own_names, problems):
    for n, span, sentence in spans(text):
        missing = [t for t in make_refs(span) if t not in targets]
        if missing and not names_other_repo(sentence, own_names):
            for t in missing:
                problems.append("%s:%d: '%s' is documented, but target '%s' is not in Makefile or project.mk"
                                % (doc, n, span.strip(), t))


def check_docs(root, paths, problems):
    tracked_files = tracked(root)
    # The docs checked are tracked ones; what they name may also be a new file not yet staged (a
    # starter `copier update` just added), but never an ignored one.
    files = tracked_files | set(paths)
    dirs = dirs_of(files)
    docs = sorted(p for p in tracked_files if p.endswith(".md") and not NOT_ACTIVE.search(p)
                  and os.path.isfile(os.path.join(root, p)))
    data = test_inputs(root, docs)
    removed, own_names = answers(root)
    targets = make_targets(root, files)
    for doc in docs:
        if doc in data:
            continue
        text = read(root, doc)
        check_links(doc, text, files, dirs, problems)
        if doc in ("README.md", "AGENTS.md") or re.match(r"^\.agents/rules/[^/]+\.md$", doc):
            check_paths(root, doc, text, files, dirs, removed, own_names, problems)
        if doc in ("README.md", "AGENTS.md"):
            check_make(doc, text, targets, own_names, problems)


# ── standard version ─────────────────────────────────────────────────────────────────
# A repo upgrades to a new standard at its next touch, in its own pull request before other
# changes (L12), never by a sweep. The skills plugin's session-start hook tells the agent; this
# fails the lint, so the `ci` check, until the repo has upgraded.

SEMVER = re.compile(r"^v(\d+)\.(\d+)\.(\d+)$")
UPGRADE = "uvx copier update --trust --conflict inline --answers-file .agents/standard.yml"
LOOKUP_SECONDS = 10
REMOTE_SOURCE = re.compile(r"^(gh:|gl:|https?://|ssh://|git@)")  # a source outside this machine
LOOKUP_TRIES = 2
RETRY_SECONDS = 3
# In CI the lookup uses the runner user's own GitHub login, which reads the private template (T-208).
CI_CREDENTIAL = ("the lookup uses the runner user's own GitHub login; check it with `gh auth status` "
                 "as that user")


def version(tag):
    m = SEMVER.match(tag or "")
    return tuple(int(x) for x in m.groups()) if m else None


def source_url(src, root="."):
    """The URL `git ls-remote` reads: `gh:owner/repo` on GitHub, a relative local path from the repo."""
    if src.startswith("gh:"):
        return "https://github.com/" + src[3:] + ".git"
    if ":" not in src.split("/", 1)[0] and not os.path.isabs(src):
        return os.path.abspath(os.path.join(root, src))
    return src


# Variables that would point git back at the checkout from the temp folder.
REPO_VARIABLES = ("GIT_DIR", "GIT_WORK_TREE", "GIT_INDEX_FILE", "GIT_COMMON_DIR", "GIT_OBJECT_DIRECTORY")


def latest_tag(url):
    """(the highest vX.Y.Z tag at `url` or None, why the lookup failed or None)."""
    env = {k: v for k, v in os.environ.items() if k not in REPO_VARIABLES}
    env["GIT_TERMINAL_PROMPT"] = "0"
    why = None
    tries = LOOKUP_TRIES if os.environ.get("CI") else 1  # locally a failed lookup only warns
    for attempt in range(tries):
        if attempt:
            time.sleep(RETRY_SECONDS)  # one retry rides out a network blip
        try:
            # Outside the checkout: CI's checkout sets a token header for this repo alone, which
            # another private repo refuses; the machine's own git credentials apply instead.
            # errors="replace": a stray non-UTF-8 byte from git must not crash the whole lint.
            result = subprocess.run(["git", "ls-remote", "--tags", "--refs", url], capture_output=True, text=True,
                                    errors="replace", timeout=LOOKUP_SECONDS, cwd=tempfile.gettempdir(),
                                    stdin=subprocess.DEVNULL, env=env)
        except subprocess.TimeoutExpired:
            why = "`git ls-remote %s` timed out after %d s" % (url, LOOKUP_SECONDS)
            continue
        except OSError as e:
            return None, "`git ls-remote %s` failed: %s" % (url, e)
        if result.returncode != 0:
            lines = result.stderr.strip().splitlines()
            fatal = [l for l in lines if l.startswith("fatal:")]
            why = "`git ls-remote %s` failed: %s" % (
                url, (fatal or lines or ["exit %d" % result.returncode])[0 if fatal else -1])
            continue
        break
    else:
        return None, why + (" (%d tries)" % tries if tries > 1 else "")
    tags = [t for t in (l.rsplit("refs/tags/", 1)[-1] for l in result.stdout.splitlines()) if version(t)]
    if not tags:
        return None, "%s has no vX.Y.Z tag" % url
    return max(tags, key=version), None


def check_standard_version(root, paths, problems):
    if ".agents/standard.yml" not in paths:
        return
    found = {}
    for line in read(root, ".agents/standard.yml").splitlines():
        m = re.match(r"^(_commit|_src_path):\s*(.*?)\s*$", line)
        if m:
            found[m.group(1)] = m.group(2).strip("'\"")
    recorded = found.get("_commit", "")
    if "REPO_STANDARD_LATEST" in os.environ:
        # Tests and offline runs; the eval worlds set it to the commit they rendered from.
        latest, why = os.environ["REPO_STANDARD_LATEST"], None
        if latest == recorded:
            return
        if not version(latest):
            problems.append(".agents/standard.yml: REPO_STANDARD_LATEST is %r, not a vX.Y.Z tag" % latest)
            return
    elif not found.get("_src_path"):
        latest, why = None, "it records no _src_path"
    else:
        latest, why = latest_tag(source_url(found["_src_path"], root))
    if why:
        if is_public(root) and why.startswith("`git ls-remote") and version(recorded) and \
                REMOTE_SOURCE.match(found["_src_path"]):
            # repo-standard is private: a contributor's clone and the hosted runner cannot read it.
            # Only a released tag from a remote source passes so; a local path or an untagged
            # commit is still a finding in CI.
            print("standard lint: the standard version was not checked (%s); this public repo's template "
                  "is private, so only a clone that can read it checks the version" % why, file=sys.stderr)
            return
        if os.environ.get("CI"):
            hint = "; " + CI_CREDENTIAL if why.startswith("`git ls-remote") else ""
            problems.append(".agents/standard.yml: the standard version was not checked (%s); in CI this check "
                            "never passes unchecked%s" % (why, hint))
        else:
            print("standard lint: the standard version was not checked (%s); CI checks it" % why, file=sys.stderr)
        return
    if version(recorded) and version(recorded) == version(latest):
        return
    if version(recorded) and version(recorded) > version(latest):
        problems.append(".agents/standard.yml: records %s, which is ahead of the latest repo-standard tag %s; it "
                        "records a released tag, never one by hand" % (recorded, latest))
        return
    what = "records %s" % recorded if version(recorded) else \
        "records the untagged template commit %s" % (recorded or "(none)")
    problems.append(".agents/standard.yml: %s, and the latest repo-standard is %s. The standard is upgraded "
                    "before other changes (L12): in its own pull request, run `%s`" % (what, latest, UPGRADE))


CHECKS = [check_makefile, check_shell, check_versions, check_sizes, check_root, check_public_host, check_hook,
          check_workflows, check_cache, check_docs, check_standard_version]


def lint(root):
    paths = files(root)
    problems = []
    for check in CHECKS:
        check(root, paths, problems)
    return problems


def main(argv=None):
    argv = sys.argv[1:] if argv is None else argv
    root = argv[0] if argv else "."
    try:
        problems = lint(root)
    except NotARepo:
        print("standard lint: %s is not in a git repository; run it inside the repo" % os.path.abspath(root),
              file=sys.stderr)
        return 2
    for p in problems:
        print("❌ " + p)
    print("standard lint: %d problem(s)" % len(problems) if problems else "✅ standard lint: clean")
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main())
