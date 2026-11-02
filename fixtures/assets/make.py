#!/usr/bin/env python3
"""Regenerates the tiny media fixtures in this directory (ticket 03: dimensions, derivatives,
asset scan). Needs macOS `sips` (HEIC/AVIF/TIFF/BMP/GIF/JPEG) and `ffmpeg` (mp4/mov/webm/mp3).
Everything is 32x20 (images) or 48x32 (video); `rot.*` carry a 90-degree orientation so the
displayed size is swapped. `a.webp` is a hand-written VP8L header (no encoder here) — enough for
dimension probing, not a displayable image. Run from this directory: python3 make.py"""
import os, struct, subprocess, zlib

def png(w, h, rgb):
    raw = b''.join(b'\x00' + bytes(rgb) * w for _ in range(h))
    def chunk(t, d):
        return struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
    return (b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 8, 2, 0, 0, 0))
            + chunk(b'IDAT', zlib.compress(raw)) + chunk(b'IEND', b''))

open('a.png', 'wb').write(png(32, 20, (200, 30, 30)))
open('b.png', 'wb').write(png(16, 8, (30, 200, 30)))

def sips(fmt, src, dst, *extra):
    subprocess.run(['sips', '-s', 'format', fmt, *extra, src, '--out', dst], check=True, capture_output=True)

sips('jpeg', 'a.png', 'a.jpg')
sips('gif', 'a.png', 'a.gif')
sips('tiff', 'a.png', 'a.tiff')
sips('bmp', 'a.png', 'a.bmp')
sips('heic', 'a.png', 'a.heic')
sips('avif', 'a.png', 'a.avif')

# rot.jpg: a.jpg with an EXIF APP1 whose Orientation is 6 (rotate 90 CW) — displayed 20x32.
jpg = open('a.jpg', 'rb').read()
assert jpg[:2] == b'\xff\xd8'
# strip existing APPn segments right after SOI
i = 2
while jpg[i] == 0xFF and 0xE0 <= jpg[i + 1] <= 0xEF:
    seg = struct.unpack('>H', jpg[i + 2:i + 4])[0]
    i += 2 + seg
tiff = b'MM\x00\x2a\x00\x00\x00\x08' + struct.pack('>H', 1) + struct.pack('>HHIHH', 0x0112, 3, 1, 6, 0) + b'\x00\x00\x00\x00'
app1 = b'Exif\x00\x00' + tiff
open('rot.jpg', 'wb').write(jpg[:2] + b'\xff\xe1' + struct.pack('>H', len(app1) + 2) + app1 + jpg[i:])

# a.webp: RIFF/WEBP/VP8L header for 32x20 (14-bit width-1, 14-bit height-1), then padding.
vp8l = b'\x2f' + struct.pack('<I', (32 - 1) | ((20 - 1) << 14)) + b'\x00' * 8
chunk = b'VP8L' + struct.pack('<I', len(vp8l)) + vp8l
open('a.webp', 'wb').write(b'RIFF' + struct.pack('<I', 4 + len(chunk)) + b'WEBP' + chunk)

open('a.svg', 'w').write('<?xml version="1.0"?>\n<svg xmlns="http://www.w3.org/2000/svg" width="120" height="80"><rect width="120" height="80" fill="#c33"/></svg>\n')
open('viewbox.svg', 'w').write('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 300 150"><circle cx="150" cy="75" r="50"/></svg>\n')

# Smallest well-formed PDF (one blank page).
objs = [b'<< /Type /Catalog /Pages 2 0 R >>', b'<< /Type /Pages /Kids [3 0 R] /Count 1 >>',
        b'<< /Type /Page /Parent 2 0 R /MediaBox [0 0 72 72] >>']
out = b'%PDF-1.4\n'
offsets = []
for n, o in enumerate(objs, 1):
    offsets.append(len(out))
    out += f'{n} 0 obj\n'.encode() + o + b'\nendobj\n'
xref = len(out)
out += f'xref\n0 {len(objs)+1}\n0000000000 65535 f \n'.encode()
for off in offsets:
    out += f'{off:010d} 00000 n \n'.encode()
out += f'trailer\n<< /Size {len(objs)+1} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n'.encode()
open('a.pdf', 'wb').write(out)

def ff(*args):
    subprocess.run(['ffmpeg', '-loglevel', 'error', '-y', *args], check=True)

ff('-f', 'lavfi', '-i', 'color=c=red:s=48x32:d=0.04:r=25', '-frames:v', '1', '-pix_fmt', 'yuv420p',
   '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '51', 'a.mp4')
ff('-f', 'lavfi', '-i', 'color=c=green:s=48x32:d=0.04:r=25', '-frames:v', '1', '-pix_fmt', 'yuv420p',
   '-c:v', 'libx264', '-preset', 'ultrafast', '-crf', '51', '-f', 'mov', 'a.mov')
ff('-display_rotation', '90', '-i', 'a.mp4', '-c', 'copy', 'rot.mp4')
ff('-f', 'lavfi', '-i', 'color=c=blue:s=48x32:d=0.04:r=25', '-frames:v', '1', '-pix_fmt', 'yuv420p',
   '-c:v', 'libvpx', '-b:v', '10k', 'a.webm')
ff('-f', 'lavfi', '-i', 'sine=frequency=440:duration=0.05', '-c:a', 'libmp3lame', '-b:a', '8k', 'a.mp3')
print('ok:', sorted(f for f in os.listdir('.') if f != 'make.py'))
