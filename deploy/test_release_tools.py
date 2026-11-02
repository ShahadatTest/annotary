#!/usr/bin/env python3
"""deploy/stamp_version.py and deploy/tap_formula.py; `make deploy-test` runs this."""
import importlib.util
import os
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name):
    spec = importlib.util.spec_from_file_location(name, os.path.join(HERE, name + ".py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


stamp_version = load("stamp_version")
tap_formula = load("tap_formula")

MANIFEST = """[workspace]
members = ["crates/*"]

[workspace.package]
# A placeholder: the comment stays.
version = "0.0.0"
edition = "2024"

[workspace.dependencies]
other = { version = "0.0.0" }
"""
LOCK = """version = 4

[[package]]
name = "annotary"
version = "0.0.0"
dependencies = [
 "annotary-store",
]

[[package]]
name = "annotary-store"
version = "0.0.0"

[[package]]
name = "annotation-thing"
version = "0.0.0"

[[package]]
name = "annotary-store-extra-thing"
version = "0.0.0"
"""
FORMULA = """class Annotary < Formula
  desc "Share Markdown"
  url "https://example.com/a.tar.gz"
  sha256 "%s"

  def install
    bin.install "annotary"
  end
end
""" % ("a" * 64)


class Stamp(unittest.TestCase):
    def setUp(self):
        self.dir = tempfile.TemporaryDirectory()
        self.addCleanup(self.dir.cleanup)
        self.write("Cargo.toml", MANIFEST)
        self.write("Cargo.lock", LOCK)

    def write(self, name, text):
        with open(os.path.join(self.dir.name, name), "w", encoding="utf-8") as f:
            f.write(text)

    def read(self, name):
        with open(os.path.join(self.dir.name, name), encoding="utf-8") as f:
            return f.read()

    def test_it_stamps_the_workspace_version_and_its_crates_and_nothing_else(self):
        self.assertEqual(stamp_version.stamp(self.dir.name, "v1.2.3"), 2)
        manifest = self.read("Cargo.toml")
        self.assertIn('[workspace.package]\n# A placeholder: the comment stays.\nversion = "1.2.3"\n', manifest)
        self.assertIn('other = { version = "0.0.0" }', manifest)
        lock = self.read("Cargo.lock")
        self.assertIn('name = "annotary"\nversion = "1.2.3"', lock)
        self.assertIn('name = "annotary-store"\nversion = "1.2.3"', lock)
        self.assertIn('name = "annotation-thing"\nversion = "0.0.0"', lock)
        self.assertIn('name = "annotary-store-extra-thing"\nversion = "0.0.0"', lock)

    def test_it_refuses_what_is_not_a_plain_version_and_writes_nothing(self):
        for bad in ("1.2.3", "v1.2", "v1.2.3-rc.1", "annotary-v1.2.3", ""):
            with self.assertRaises(ValueError, msg=bad):
                stamp_version.stamp(self.dir.name, bad)
        self.assertEqual(self.read("Cargo.toml"), MANIFEST)

    def test_it_refuses_a_manifest_with_no_placeholder_and_a_lock_with_no_crate(self):
        self.write("Cargo.toml", MANIFEST.replace('version = "0.0.0"\nedition', 'version = "9.9.9"\nedition'))
        with self.assertRaises(ValueError):
            stamp_version.stamp(self.dir.name, "v1.2.3")
        self.write("Cargo.toml", MANIFEST)
        self.write("Cargo.lock", "version = 4\n")
        with self.assertRaises(ValueError):
            stamp_version.stamp(self.dir.name, "v1.2.3")
        self.assertEqual(self.read("Cargo.toml"), MANIFEST)


class Formula(unittest.TestCase):
    def test_the_test_block_goes_inside_the_class(self):
        out = tap_formula.with_test(FORMULA)
        self.assertTrue(out.endswith('  end\nend\n'), out)
        self.assertIn("  end\n\n  test do\n", out)
        for needle in ("annotary --version", "serve --setup-only", "#{testpath}/data"):
            self.assertIn(needle, out)
        self.assertEqual(out.count("test do"), 1)

    def test_it_refuses_a_formula_that_should_not_go_out(self):
        for bad in (
            tap_formula.with_test(FORMULA),  # a second test
            FORMULA.replace("a" * 64, "not-a-checksum"),
            FORMULA.replace('  sha256 "%s"\n' % ("a" * 64), ""),
            FORMULA.replace('  url "https://example.com/a.tar.gz"\n', "").replace('  sha256 "%s"\n' % ("a" * 64), ""),
            FORMULA[: -len("end\n")],
        ):
            with self.assertRaises(ValueError, msg=bad):
                tap_formula.with_test(bad)


if __name__ == "__main__":
    unittest.main()
