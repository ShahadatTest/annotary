#!/usr/bin/env python3
"""Tests of scripts/brand/render.py, run by crates/annotary/tests/brand.rs (`make test`)."""

import math
import re
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
import zlib
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
import render  # noqa: E402

BRAND = render.BRAND
SCRIPT = Path(render.__file__).resolve()


def check(directory):
    return subprocess.run([sys.executable, str(SCRIPT), "--check", "--dir", str(directory)],
                          capture_output=True, text=True)


def rewrite_first_pixel(data):
    """The same PNG with one scanline byte changed, recompressed."""
    ihdr, raw = render.decode_png(data)
    raw = raw[:1] + bytes([raw[1] ^ 1]) + raw[2:]
    return b"\x89PNG\r\n\x1a\n" + render._chunk(b"IHDR", ihdr) + render._chunk(b"IDAT", zlib.compress(raw)) \
        + render._chunk(b"IEND", b"")


class Check(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.dir = Path(tmp.name) / "brand"
        shutil.copytree(BRAND, self.dir)

    def assertFlags(self, name, why):
        r = check(self.dir)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("brand: %s: %s" % (name, why), r.stderr)

    def test_the_committed_files_match_a_fresh_render(self):
        r = check(BRAND)
        self.assertEqual(r.returncode, 0, r.stdout + r.stderr)

    def test_a_changed_png_pixel_fails(self):
        path = self.dir / "icon-192.png"
        path.write_bytes(rewrite_first_pixel(path.read_bytes()))
        self.assertFlags("icon-192.png", "differs from a fresh render")

    def test_a_changed_ico_layer_fails(self):
        path = self.dir / "favicon.ico"
        layers = render.decode_ico(path.read_bytes())
        self.assertEqual(len(layers), 2)
        data = path.read_bytes()
        _, _, _, _, _, _, length, offset = struct.unpack("<BBBBHHII", data[22:38])
        changed = rewrite_first_pixel(data[offset : offset + length])
        first = struct.unpack("<BBBBHHII", data[6:22])
        rebuilt = render.ico([(32, data[first[7] : first[7] + first[6]]), (16, changed)])
        path.write_bytes(rebuilt)
        self.assertFlags("favicon.ico", "differs from a fresh render")

    def test_a_changed_generated_svg_fails(self):
        path = self.dir / "icon.svg"
        path.write_text(path.read_text().replace("#f472b6", "#f472b7", 1))
        self.assertFlags("icon.svg", "differs from a fresh render")

    def test_a_missing_file_fails(self):
        (self.dir / "apple-touch-icon.png").unlink()
        self.assertFlags("apple-touch-icon.png", "missing")

    def test_a_truncated_file_fails(self):
        (self.dir / "icon-512.png").write_bytes(b"\x89PNG")
        r = check(self.dir)
        self.assertEqual(r.returncode, 1, r.stdout + r.stderr)
        self.assertIn("icon-512.png", r.stderr)

    def test_an_unknown_file_fails(self):
        (self.dir / "hand-exported.png").write_bytes(b"x")
        self.assertFlags("hand-exported.png", "not a master or a file this script renders")

    def test_the_check_writes_nothing(self):
        path = self.dir / "icon.svg"
        path.write_text("changed")
        check(self.dir)
        self.assertEqual(path.read_text(), "changed")


class Rendered(unittest.TestCase):
    def rgba(self, name, data=None):
        ihdr, raw = render.decode_png(data if data is not None else (BRAND / name).read_bytes())
        width, height, depth, kind = struct.unpack(">IIBB", ihdr[:10])
        self.assertEqual(depth, 8)
        channels = {2: 3, 6: 4}[kind]
        stride = width * channels + 1

        def at(x, y):
            p = raw[y * stride + 1 + x * channels : y * stride + 1 + (x + 1) * channels]
            return tuple(p) + ((255,) if channels == 3 else ())

        return width, height, kind, at

    def test_favicon_layers_come_from_the_small_master(self):
        layers = render.decode_ico((BRAND / "favicon.ico").read_bytes())
        self.assertEqual([(w, h) for w, h, _ in layers], [(32, 32), (16, 16)])
        self.assertEqual(layers[1][2], render.decode_png(render.png(render.rasterize("mark-small.svg", 16), 16)))
        self.assertIn('r="12"', (BRAND / "icon.svg").read_text())
        self.assertNotIn("stroke-opacity", (BRAND / "icon.svg").read_text())

    def test_the_small_master_keeps_a_visible_dot_at_16_px(self):
        layers = render.rasterize("mark-small.svg", 16)
        pink = [p for p in layers if p[0] > 0.8 and p[1] < 0.6 and p[3] > 0.9]
        self.assertGreaterEqual(len(pink), 20)

    def test_the_favicon_svg_is_one_inlineable_line(self):
        text = (BRAND / "icon.svg").read_text()
        self.assertEqual(text.count("\n"), 1)
        self.assertTrue(text.startswith('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><rect'))
        self.assertNotRegex(text, r"\b(id|class)=")

    def test_rounded_icons_have_transparent_corners(self):
        for name, size in (("icon-192.png", 192), ("icon-512.png", 512)):
            width, height, _, at = self.rgba(name)
            self.assertEqual((width, height), (size, size))
            self.assertEqual(at(0, 0)[3], 0, name)
            self.assertEqual(at(size // 2, 2), (0x1c, 0x1c, 0x1e, 255), name)

    def test_touch_and_mask_icons_are_opaque_squares(self):
        for name, size in (("apple-touch-icon.png", 180), ("icon-mask.png", 512)):
            width, height, kind, at = self.rgba(name)
            self.assertEqual((width, height, kind), (size, size, 2), name)
            self.assertEqual(at(0, 0), (0x1c, 0x1c, 0x1e, 255), name)
            self.assertEqual(at(size - 1, size - 1), (0x1c, 0x1c, 0x1e, 255), name)

    def test_the_touch_icons_mark_is_in_the_middle_140(self):
        _, _, _, at = self.rgba("apple-touch-icon.png")
        ground = (0x1c, 0x1c, 0x1e, 255)
        for i in range(180):
            for x, y in ((i, 19), (i, 160), (19, i), (160, i)):
                self.assertEqual(at(x, y), ground, (x, y))

    def test_the_maskable_mark_is_inside_the_safe_circle(self):
        _, _, _, at = self.rgba("icon-mask.png")
        ground = (0x1c, 0x1c, 0x1e, 255)
        radius, center, drawn = 512 * 0.40, 256, 0
        for y in range(512):
            for x in range(512):
                if at(x, y) != ground:
                    drawn += 1
                    self.assertLessEqual(math.hypot(x + 0.5 - center, y + 0.5 - center), radius, (x, y))
        self.assertGreater(drawn, 20000)

    def test_masters_use_only_the_design_colors(self):
        for master in render.MASTERS:
            colors = set(re.findall(r'(?:fill|stroke)="(#[0-9a-f]{6})"', (BRAND / master).read_text()))
            self.assertEqual(colors, {"#1c1c1e", "#f2f2f7", "#f472b6"}, master)

    def test_logos_carry_the_mark_and_the_wordmark(self):
        for name, ink in (("logo.svg", "#1c1c1e"), ("logo-dark.svg", "#f2f2f7")):
            text = (BRAND / name).read_text()
            self.assertIn(render.flat_svg("mark.svg"), text)
            self.assertIn('font-weight="600" fill="%s"' % ink, text)
            self.assertIn(">annotary</text>", text)

    def test_a_master_with_an_unsupported_shape_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            bad = Path(tmp) / "bad.svg"
            bad.write_text('<svg xmlns="%s" viewBox="0 0 100 100"><g id="ground"><path d="M0 0"/></g></svg>' % render.SVG_NS)
            original = render.BRAND
            render.BRAND = Path(tmp)
            try:
                with self.assertRaisesRegex(ValueError, "only rect, line and circle"):
                    render.load_master("bad.svg")
            finally:
                render.BRAND = original
                render.load_master.cache_clear()

    def test_a_master_with_an_attribute_the_rasterizer_ignores_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            original = render.BRAND
            render.BRAND = Path(tmp)
            try:
                for name, body, message in (
                    ("rect.svg", '<g id="ground"><rect width="100" height="100" fill="#1c1c1e" fill-opacity=".25"/></g>', "fill-opacity"),
                    ("move.svg", '<g id="ground"><rect width="100" height="100" fill="#1c1c1e" transform="translate(5 5)"/></g>', "transform"),
                    ("group.svg", '<g id="ground" opacity=".5"><rect width="100" height="100" fill="#1c1c1e"/></g>', "named groups"),
                    ("cap.svg", '<g id="ground"><line x1="0" y1="5" x2="9" y2="5" stroke="#1c1c1e" stroke-width="2" stroke-linecap="square"/></g>', "round or butt"),
                ):
                    (Path(tmp) / name).write_text('<svg xmlns="%s" viewBox="0 0 100 100">%s</svg>' % (render.SVG_NS, body))
                    with self.assertRaisesRegex(ValueError, message):
                        render.load_master(name)
            finally:
                render.BRAND = original
                render.load_master.cache_clear()


if __name__ == "__main__":
    unittest.main()
