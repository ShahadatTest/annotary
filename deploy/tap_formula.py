#!/usr/bin/env python3
"""Add the `test do` block to the Homebrew formula cargo-dist generated.

    python3 deploy/tap_formula.py path/to/annotary.rb

dist writes the formula but gives it no test, and `brew audit --strict` wants one, so the tap's
publish job (.github/workflows/publish-tap.yml) runs this before `brew test`. It refuses a formula
with a test already (one is all a formula may have), with no closing `end` to put it before, or with
a download that has no checksum.
"""
import re
import sys

TEST_BLOCK = """
  test do
    assert_match version.to_s, shell_output("#{bin}/annotary --version")
    ENV["ANNOTARY_SETUP_HANDLE"] = "brewtest"
    shell_output("#{bin}/annotary serve --setup-only --data-dir #{testpath}/data < /dev/null")
    assert_path_exists testpath/"data/store/annotary.sqlite"
  end
"""


def with_test(formula):
    """`formula` with the test added; ValueError if it should not be published."""
    if re.search(r"^\s*test do\b", formula, re.M):
        raise ValueError("the formula already has a test do block")
    urls = len(re.findall(r'^\s*url "', formula, re.M))
    sums = len(re.findall(r'^\s*sha256 "[0-9a-f]{64}"', formula, re.M))
    if not urls or urls != sums:
        raise ValueError("the formula has %d url lines and %d sha256 lines; every download needs a checksum" % (urls, sums))
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
