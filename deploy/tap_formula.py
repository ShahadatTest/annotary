#!/usr/bin/env python3
"""Add the `test do` block to the Homebrew formula cargo-dist generated.

    python3 deploy/tap_formula.py path/to/annotary.rb

dist writes the formula with no test and with an alias table `brew audit --strict` refuses, so the
tap's publish job (.github/workflows/publish-tap.yml) runs this before the audit. It refuses a formula
with a test already (one is all a formula may have), with no closing `end` to put it before, or with
a download that has no checksum.
"""
import re
import sys

# dist's table of binary aliases, written so that `brew audit --strict` refuses it.
ALIASES = re.compile(r"^  BINARY_ALIASES = \{\n(?:    \"[^\"]+\": \{\},?\n)+  \}\n", re.M)

TEST_BLOCK = """
  test do
    assert_match version.to_s, shell_output("#{bin}/annotary --version")
    ENV["ANNOTARY_SETUP_HANDLE"] = "brewtest"
    shell_output("#{bin}/annotary serve --setup-only --data-dir #{testpath}/data < /dev/null")
    assert_path_exists testpath/"data/store/annotary.sqlite"
  end
"""


def aligned_aliases(match):
    """The table as Homebrew's style wants it: aligned, a trailing comma, frozen."""
    names = re.findall(r'"([^"]+)": \{\}', match.group(0))
    width = max(len(n) for n in names) + 3
    rows = "".join('    %s {},\n' % ('"%s":' % n).ljust(width) for n in names)
    return "  BINARY_ALIASES = {\n" + rows + "  }.freeze\n"


def with_test(formula):
    """`formula` with the test added; ValueError if it should not be published."""
    if re.search(r"^\s*test do\b", formula, re.M):
        raise ValueError("the formula already has a test do block")
    urls = len(re.findall(r'^\s*url "', formula, re.M))
    sums = len(re.findall(r'^\s*sha256 "[0-9a-f]{64}"', formula, re.M))
    if not urls or urls != sums:
        raise ValueError("the formula has %d url lines and %d sha256 lines; every download needs a checksum" % (urls, sums))
    formula = ALIASES.sub(aligned_aliases, formula)
    # `brew audit --strict`: the version is already in the url.
    formula = re.sub(r'^  version "[^"]+"\n', "", formula, count=1, flags=re.M)
    stripped = formula.rstrip("\n")
    if not stripped.endswith("\nend"):
        raise ValueError("the formula does not end with the class's end")
    return stripped[: -len("end")].rstrip("\n") + "\n" + TEST_BLOCK + "end\n"


def main(argv):
    if len(argv) != 1:
        sys.exit(__doc__)
    with open(argv[0], encoding="utf-8") as f:
        formula = f.read()
    try:
        updated = with_test(formula)
    except ValueError as error:
        sys.exit("tap_formula: %s" % error)
    with open(argv[0], "w", encoding="utf-8") as f:
        f.write(updated)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
