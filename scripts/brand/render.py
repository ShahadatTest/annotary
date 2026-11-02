#!/usr/bin/env python3
"""Render everything in brand/ from its two masters (DESIGN.md, Brand).

    uv run --no-project scripts/brand/render.py           write the files (`make brand`)
    uv run --no-project scripts/brand/render.py --check   fail when a committed file differs

Standard library only, on purpose: the check runs on Linux and macOS and compares pixels exactly,
and a real SVG library's antialiasing may differ between platforms by a level or two. A rounded
box is all the masters need, since a rect, a horizontal line and a circle are all rounded boxes;
anything else in a master is refused rather than drawn wrong. Only IEEE arithmetic and sqrt are
used, so a pixel is the same on every machine. PNG compression depends on the build's zlib, so
the check compares a PNG's header and scanlines, not its compressed bytes.
"""

import argparse
import functools
import math
import struct
import sys
import xml.etree.ElementTree as ET
import zlib
from pathlib import Path

BRAND = Path(__file__).resolve().parents[2] / "brand"
SVG_NS = "http://www.w3.org/2000/svg"
MASTERS = ("mark.svg", "mark-small.svg")
# Edge pixels are measured on a grid this fine; 8 x 8 is 64 coverage levels.
SAMPLES = 8
# A pixel whose center is further than this from an edge (half its diagonal) is all in or all out.
HALF_DIAGONAL = 0.7072
# The maskable icon's mark is scaled to the 409 px of 512 that its 40% safe circle guarantees.
MASK_SCALE = 409 / 512
WORDMARK_FONT = "-apple-system, BlinkMacSystemFont, 'Segoe UI', Inter, sans-serif"


# What the rasterizer draws, per element. Anything else in a master is refused.
ATTRIBUTES = {
    "rect": {"x", "y", "width", "height", "rx", "fill"},
    "circle": {"cx", "cy", "r", "fill"},
    "line": {"x1", "y1", "x2", "y2", "stroke", "stroke-opacity", "stroke-width", "stroke-linecap"},
}


class Box:
    """A filled rounded box in the master's 100-unit grid."""

    def __init__(self, group, x0, y0, x1, y1, radius, color, opacity, source):
        self.group, self.x0, self.y0, self.x1, self.y1 = group, x0, y0, x1, y1
        self.radius, self.opacity, self.source = radius, opacity, source
        self.color = tuple(int(color[i : i + 2], 16) / 255 for i in (1, 3, 5))


def _tag(el):
    return el.tag.removeprefix("{%s}" % SVG_NS)


def _num(el, name, default=None):
    value = el.get(name)
    if value is None and default is None:
        raise ValueError("<%s> needs %s" % (_tag(el), name))
    return float(value) if value is not None else default


def _box(group, el):
    tag = _tag(el)
    if tag in ATTRIBUTES and set(el.attrib) - ATTRIBUTES[tag]:
        # An attribute the rasterizer ignores would stay in the SVGs and not in the PNGs.
        raise ValueError("a master's <%s> cannot have %s" % (tag, sorted(set(el.attrib) - ATTRIBUTES[tag])))
    attrs = " ".join('%s="%s"' % item for item in el.attrib.items())
    source = "<%s %s/>" % (tag, attrs)
    if tag == "rect":
        x, y = _num(el, "x", 0.0), _num(el, "y", 0.0)
        w, h = _num(el, "width"), _num(el, "height")
        return Box(group, x, y, x + w, y + h, _num(el, "rx", 0.0), el.get("fill"), 1.0, source)
    if tag == "circle":
        cx, cy, r = _num(el, "cx"), _num(el, "cy"), _num(el, "r")
        return Box(group, cx - r, cy - r, cx + r, cy + r, r, el.get("fill"), 1.0, source)
    if tag == "line":
        x1, x2, y = _num(el, "x1"), _num(el, "x2"), _num(el, "y1")
        if _num(el, "y2") != y:
            raise ValueError("a master's <line> must be horizontal: %s" % source)
        if el.get("stroke-linecap", "butt") not in ("round", "butt"):
            raise ValueError("a master's <line> caps are round or butt: %s" % source)
        half = _num(el, "stroke-width") / 2
        cap = half if el.get("stroke-linecap") == "round" else 0.0
        return Box(group, x1 - cap, y - half, x2 + cap, y + half, cap, el.get("stroke"),
                   _num(el, "stroke-opacity", 1.0), source)
    raise ValueError("a master holds only rect, line and circle, not <%s>" % tag)


@functools.lru_cache(maxsize=None)
def load_master(name):
    root = ET.parse(BRAND / name).getroot()
    if _tag(root) != "svg" or root.get("viewBox") != "0 0 100 100" or set(root.attrib) != {"viewBox"}:
        raise ValueError("%s: expected an <svg> on the 100-unit grid, with no other attribute" % name)
    boxes = []
    for group in root:
        if _tag(group) != "g" or not group.get("id") or set(group.attrib) != {"id"}:
            raise ValueError("%s: the top level holds only named groups" % name)
        boxes.extend(_box(group.get("id"), el) for el in group)
    if not boxes or boxes[0].group != "ground":
        raise ValueError("%s: the ground comes first" % name)
    return tuple(boxes)


def _signed_distance(px, py, cx, cy, hx, hy, radius):
    qx, qy = abs(px - cx) - hx + radius, abs(py - cy) - hy + radius
    ox, oy = max(qx, 0.0), max(qy, 0.0)
    return math.sqrt(ox * ox + oy * oy) + min(max(qx, qy), 0.0) - radius


def rasterize(master, size, square=False, scale=1.0):
    """Premultiplied RGBA floats, row by row. `square` drops the ground's corners; `scale` shrinks
    everything but the ground around the center."""
    unit = size / 100
    pixels = [[0.0, 0.0, 0.0, 0.0] for _ in range(size * size)]
    for box in load_master(master):
        ground = box.group == "ground"
        s = 1.0 if ground else scale
        x0, y0, x1, y1 = ((50 + (v - 50) * s) * unit for v in (box.x0, box.y0, box.x1, box.y1))
        radius = 0.0 if ground and square else box.radius * s * unit
        cx, cy, hx, hy = (x0 + x1) / 2, (y0 + y1) / 2, (x1 - x0) / 2, (y1 - y0) / 2
        r, g, b = box.color
        for py in range(max(int(y0) - 1, 0), min(int(y1) + 2, size)):
            row = py * size
            for px in range(max(int(x0) - 1, 0), min(int(x1) + 2, size)):
                d = _signed_distance(px + 0.5, py + 0.5, cx, cy, hx, hy, radius)
                if d >= HALF_DIAGONAL:
                    continue
                coverage = 1.0
                if d > -HALF_DIAGONAL:
                    inside = 0
                    for sy in range(SAMPLES):
                        for sx in range(SAMPLES):
                            sample = _signed_distance(px + (sx + 0.5) / SAMPLES, py + (sy + 0.5) / SAMPLES,
                                                      cx, cy, hx, hy, radius)
                            inside += sample <= 0
                    coverage = inside / (SAMPLES * SAMPLES)
                a = coverage * box.opacity
                p = pixels[row + px]
                keep = 1 - a
                p[0], p[1], p[2], p[3] = r * a + p[0] * keep, g * a + p[1] * keep, b * a + p[2] * keep, a + p[3] * keep
    return pixels


def _byte(v):
    return min(255, int(v * 255 + 0.5))


def _chunk(kind, data):
    return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))


def png(pixels, size):
    """A PNG: truecolor when every pixel is opaque (an iOS touch icon has no alpha), else RGBA."""
    opaque = all(p[3] > 0.999 for p in pixels)
    channels = 3 if opaque else 4
    raw = bytearray()
    for y in range(size):
        raw.append(0)
        for p in pixels[y * size : (y + 1) * size]:
            a = p[3]
            if opaque:
                raw += bytes((_byte(p[0]), _byte(p[1]), _byte(p[2])))
            elif a <= 0:
                raw += b"\0\0\0\0"
            else:
                raw += bytes((_byte(p[0] / a), _byte(p[1] / a), _byte(p[2] / a), _byte(a)))
    ihdr = struct.pack(">IIBBBBB", size, size, 8, 2 if channels == 3 else 6, 0, 0, 0)
    return (b"\x89PNG\r\n\x1a\n" + _chunk(b"IHDR", ihdr) + _chunk(b"IDAT", zlib.compress(bytes(raw), 9))
            + _chunk(b"IEND", b""))


def ico(layers):
    """An .ico of (size, PNG bytes) layers, largest first."""
    out = struct.pack("<HHH", 0, 1, len(layers))
    offset = 6 + 16 * len(layers)
    for size, data in layers:
        out += struct.pack("<BBBBHHII", size % 256, size % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    return out + b"".join(data for _, data in layers)


def flat_svg(master):
    """A master as one line of SVG without groups or comments, ready to inline in a page."""
    return "".join(box.source for box in load_master(master))


def icon_svg():
    return '<svg xmlns="%s" viewBox="0 0 100 100">%s</svg>\n' % (SVG_NS, flat_svg("mark-small.svg"))


def logo_svg(ink):
    return ('<svg xmlns="%s" viewBox="0 0 372 100" role="img" aria-label="annotary">%s'
            '<text x="114" y="70" font-family="%s" font-size="58" font-weight="600" fill="%s" textLength="250" '
            'lengthAdjust="spacingAndGlyphs">annotary</text></svg>\n') % (SVG_NS, flat_svg("mark.svg"), WORDMARK_FONT, ink)


def _png_of(master, size, **kw):
    return png(rasterize(master, size, **kw), size)


def render():
    """Every generated file: {name: bytes}."""
    return {
        "icon.svg": icon_svg().encode(),
        "logo.svg": logo_svg("#1c1c1e").encode(),
        "logo-dark.svg": logo_svg("#f2f2f7").encode(),
        "favicon.ico": ico([(32, _png_of("mark-small.svg", 32)), (16, _png_of("mark-small.svg", 16))]),
        "apple-touch-icon.png": _png_of("mark.svg", 180, square=True),
        "icon-192.png": _png_of("mark.svg", 192),
        "icon-512.png": _png_of("mark.svg", 512),
        "icon-mask.png": _png_of("mark.svg", 512, square=True, scale=MASK_SCALE),
    }


def decode_png(data):
    """(IHDR, scanlines) of a PNG, whatever zlib compressed it."""
    if not data.startswith(b"\x89PNG\r\n\x1a\n"):
        raise ValueError("not a PNG")
    pos, ihdr, idat = 8, None, b""
    while pos < len(data):
        length, kind = struct.unpack(">I4s", data[pos : pos + 8])
        body = data[pos + 8 : pos + 8 + length]
        if kind == b"IHDR":
            ihdr = body
        elif kind == b"IDAT":
            idat += body
        pos += 12 + length
    return ihdr, zlib.decompress(idat)


def decode_ico(data):
    """[(width, height, decoded PNG)] of an .ico."""
    _, kind, count = struct.unpack("<HHH", data[:6])
    if kind != 1:
        raise ValueError("not an .ico")
    layers = []
    for i in range(count):
        w, h, _, _, _, _, length, offset = struct.unpack("<BBBBHHII", data[6 + 16 * i : 22 + 16 * i])
        layers.append((w, h, decode_png(data[offset : offset + length])))
    return layers


def comparable(name, data):
    """What must match: a PNG's pixels and an .ico's layers (zlib varies by build), else the bytes."""
    if name.endswith(".png"):
        return decode_png(data)
    if name.endswith(".ico"):
        return decode_ico(data)
    return data


def differences(directory, rendered):
    """Why `directory` is not what `render` makes: one line per file."""
    problems = []
    for name, want in rendered.items():
        path = directory / name
        if not path.exists():
            problems.append("%s: missing" % name)
            continue
        try:
            same = comparable(name, path.read_bytes()) == comparable(name, want)
        except (ValueError, struct.error, zlib.error) as e:
            problems.append("%s: unreadable (%s)" % (name, e))
            continue
        if not same:
            problems.append("%s: differs from a fresh render" % name)
    known = set(rendered) | set(MASTERS)
    for path in sorted(directory.iterdir()):
        if path.name not in known:
            problems.append("%s: not a master or a file this script renders" % path.name)
    return problems


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--check", action="store_true", help="write nothing; fail when a file differs")
    parser.add_argument("--dir", type=Path, default=BRAND, help="the rendered files' folder (default brand/)")
    args = parser.parse_args()
    rendered = render()
    if args.check:
        problems = differences(args.dir, rendered)
        for line in problems:
            print("brand: %s" % line, file=sys.stderr)
        if problems:
            print("brand: run `make brand` and commit the result", file=sys.stderr)
            return 1
        print("brand: %d files match a fresh render" % len(rendered))
        return 0
    for name, data in rendered.items():
        (args.dir / name).write_bytes(data)
    print("brand: wrote %d files" % len(rendered))
    return 0


if __name__ == "__main__":
    sys.exit(main())
