//! Media probing at the edge: image/video dimensions (so the web UI can reserve boxes — CLS 0)
//! and web-safe derivatives for the formats browsers do not all display (HEIC/HEIF/AVIF →
//! JPEG through the OS or ImageMagick, TIFF/BMP → PNG in-process). Everything here is
//! best-effort: a probe that fails records nothing; a derivative that cannot be produced is a
//! push warning, never an error.

use std::io::Cursor;
use std::process::Command;

use super::types::SupportedType;

/// `(width, height)` in CSS pixels as the browser will display it (EXIF orientation applied),
/// for images and videos; `None` for anything else or on any parse failure.
pub fn dimensions(bytes: &[u8], content_type: &str) -> Option<(u32, u32)> {
    match content_type {
        "image/svg+xml" => svg_dimensions(bytes),
        ct if ct.starts_with("image/") => raster_dimensions(bytes),
        "video/mp4" | "video/quicktime" | "video/x-m4v" => mp4_dimensions(bytes),
        "video/webm" => webm_dimensions(bytes),
        _ => None,
    }
}

fn raster_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let size = imagesize::blob_size(bytes).ok()?;
    let (w, h) = (u32::try_from(size.width).ok()?, u32::try_from(size.height).ok()?);
    // A top-down BMP stores a negative height; the probe hands it back two's-complemented.
    let h = if h >= 0x8000_0000 { (h as i32).unsigned_abs() } else { h };
    if w == 0 || h == 0 {
        return None;
    }
    // EXIF orientations 5–8 are rotated by 90°: browsers display them swapped.
    let swapped = exif::Reader::new()
        .read_from_container(&mut Cursor::new(bytes))
        .ok()
        .and_then(|e| e.get_field(exif::Tag::Orientation, exif::In::PRIMARY).and_then(|f| f.value.get_uint(0)))
        .is_some_and(|o| (5..=8).contains(&o));
    Some(if swapped { (h, w) } else { (w, h) })
}

/// `width`/`height` attributes of the root `<svg>` (unitless or `px`), else the `viewBox`.
fn svg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let text = std::str::from_utf8(&bytes[..bytes.len().min(64 * 1024)]).ok()?;
    let start = text.find("<svg")?;
    let rest = &text[start + 4..];
    let end = rest.find('>')?;
    let tag = &rest[..end];
    let attr = |name: &str| -> Option<String> {
        let mut search = tag;
        while let Some(pos) = search.find(name) {
            let before_ok = pos == 0 || !search.as_bytes()[pos - 1].is_ascii_alphanumeric();
            let after = &search[pos + name.len()..];
            let after = after.trim_start();
            if before_ok && let Some(v) = after.strip_prefix('=') {
                let v = v.trim_start();
                let q = v.chars().next()?;
                if q == '"' || q == '\'' {
                    let v = &v[1..];
                    return Some(v[..v.find(q)?].to_string());
                }
                return Some(v.split(|c: char| c.is_whitespace() || c == '/' || c == '>').next()?.to_string());
            }
            search = &search[pos + name.len()..];
        }
        None
    };
    let px = |v: &str| -> Option<u32> {
        let v = v.trim();
        let v = v.strip_suffix("px").unwrap_or(v).trim();
        let f: f64 = v.parse().ok()?;
        if f > 0.0 && f < 1e7 { Some(f.round() as u32) } else { None }
    };
    if let (Some(w), Some(h)) = (attr("width").and_then(|v| px(&v)), attr("height").and_then(|v| px(&v))) {
        return Some((w, h));
    }
    let vb = attr("viewBox")?;
    let nums: Vec<f64> = vb.split(|c: char| c.is_whitespace() || c == ',').filter_map(|n| n.parse().ok()).collect();
    if nums.len() == 4 && nums[2] > 0.0 && nums[3] > 0.0 {
        return Some((nums[2].round() as u32, nums[3].round() as u32));
    }
    None
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
}

fn be64(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|s| u64::from_be_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]]))
}

/// ISO BMFF boxes of `data`: `(type, header_len, total_len)`.
fn boxes(data: &[u8]) -> impl Iterator<Item = (&[u8], usize, usize)> {
    let mut pos = 0usize;
    std::iter::from_fn(move || {
        if pos + 8 > data.len() {
            return None;
        }
        let size32 = be32(data, pos)? as usize;
        let kind = &data[pos + 4..pos + 8];
        let (header, total) = match size32 {
            0 => (8, data.len() - pos),
            1 => (16, usize::try_from(be64(data, pos + 8)?).ok()?),
            n => (8, n),
        };
        if total < header || pos + total > data.len() {
            return None;
        }
        let item = (kind, pos + header, pos + total);
        pos += total;
        Some(item)
    })
}

/// The first video track's `tkhd` width/height (16.16 fixed point), swapped when its matrix
/// rotates by 90°/270° (phones do that) — the size the browser displays.
fn mp4_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    let (moov_start, moov_end) = boxes(bytes).find(|(k, _, _)| *k == b"moov").map(|(_, s, e)| (s, e))?;
    for (kind, ts, te) in boxes(&bytes[moov_start..moov_end]) {
        if kind != b"trak" {
            continue;
        }
        let trak = &bytes[moov_start + ts..moov_start + te];
        for (k2, hs, he) in boxes(trak) {
            if k2 != b"tkhd" {
                continue;
            }
            let h = &trak[hs..he];
            let version = *h.first()?;
            let after_times = if version == 1 { 4 + 8 + 8 + 4 + 4 + 8 } else { 4 + 4 + 4 + 4 + 4 + 4 };
            let matrix_at = after_times + 8 + 2 + 2 + 2 + 2;
            let width = be32(h, matrix_at + 36)? >> 16;
            let height = be32(h, matrix_at + 40)? >> 16;
            if width == 0 || height == 0 {
                continue;
            }
            let a = be32(h, matrix_at)? as i32;
            let d = be32(h, matrix_at + 16)? as i32;
            let rotated = a == 0 && d == 0;
            return Some(if rotated { (height, width) } else { (width, height) });
        }
    }
    None
}

/// EBML variable-length integer at `pos`: `(value, length, all_ones)`; ids keep their marker.
fn vint(b: &[u8], pos: usize, keep_marker: bool) -> Option<(u64, usize, bool)> {
    let first = *b.get(pos)?;
    if first == 0 {
        return None;
    }
    let len = first.leading_zeros() as usize + 1;
    if len > 8 || pos + len > b.len() {
        return None;
    }
    let mut value: u64 = if keep_marker { first as u64 } else { (first as u64) & ((1u64 << (8 - len)) - 1) };
    let mut all_ones = value == (1u64 << (8 - len)) - 1;
    for i in 1..len {
        let byte = b[pos + i];
        value = (value << 8) | byte as u64;
        all_ones &= byte == 0xFF;
    }
    Some((value, len, all_ones && !keep_marker))
}

/// WebM (Matroska): Segment → Tracks → TrackEntry → Video → PixelWidth (0xB0) / PixelHeight (0xBA).
fn webm_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const SEGMENT: u64 = 0x1853_8067;
    const TRACKS: u64 = 0x1654_AE6B;
    const TRACK_ENTRY: u64 = 0xAE;
    const VIDEO: u64 = 0xE0;
    const PIXEL_WIDTH: u64 = 0xB0;
    const PIXEL_HEIGHT: u64 = 0xBA;
    const CLUSTER: u64 = 0x1F43_B675;
    fn walk(b: &[u8], mut pos: usize, end: usize, path: &[u64], depth: usize) -> Option<(u32, u32)> {
        let mut w = None;
        let mut h = None;
        while pos < end {
            let (id, idl, _) = vint(b, pos, true)?;
            let (size, sl, unknown) = vint(b, pos + idl, false)?;
            let start = pos + idl + sl;
            let stop = if unknown { end } else { end.min(start + usize::try_from(size).ok()?) };
            if depth < path.len() {
                if id == path[depth] {
                    if let Some(found) = walk(b, start, stop, path, depth + 1) {
                        return Some(found);
                    }
                } else if id == CLUSTER {
                    return None;
                }
            } else {
                let uint = || {
                    b.get(start..stop)
                        .filter(|s| s.len() <= 8)
                        .map(|s| s.iter().fold(0u64, |a, x| (a << 8) | *x as u64))
                };
                if id == PIXEL_WIDTH {
                    w = uint().and_then(|v| u32::try_from(v).ok());
                } else if id == PIXEL_HEIGHT {
                    h = uint().and_then(|v| u32::try_from(v).ok());
                }
                if let (Some(w), Some(h)) = (w, h)
                    && w > 0
                    && h > 0
                {
                    return Some((w, h));
                }
            }
            if unknown {
                // Unknown-sized element: its children run to our end; we already descended.
                return None;
            }
            pos = stop;
        }
        None
    }
    walk(bytes, 0, bytes.len(), &[SEGMENT, TRACKS, TRACK_ENTRY, VIDEO], 0)
}

/// Why no derivative could be produced.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DerivativeError {
    /// No decoder for this format on this machine (HEIC/HEIF/AVIF need macOS `sips` or ImageMagick).
    NoConverter,
    /// The converter ran and failed (corrupt file, unsupported sub-format, …).
    Failed(String),
}

impl std::fmt::Display for DerivativeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DerivativeError::NoConverter => {
                f.write_str("no converter on this machine (needs macOS `sips` or ImageMagick `magick`)")
            }
            DerivativeError::Failed(why) => write!(f, "conversion failed: {why}"),
        }
    }
}

/// A web-safe derivative of `bytes` (type `ty`, which must have `ty.derivative`): PNG for
/// TIFF/BMP (decoded in-process), JPEG for HEIC/HEIF/AVIF (macOS `sips`, else ImageMagick).
pub fn derivative(bytes: &[u8], ty: &SupportedType) -> Result<Vec<u8>, DerivativeError> {
    match ty.ext {
        "bmp" | "tif" | "tiff" => {
            let format = if ty.ext == "bmp" { image::ImageFormat::Bmp } else { image::ImageFormat::Tiff };
            let img = image::load_from_memory_with_format(bytes, format)
                .map_err(|e| DerivativeError::Failed(e.to_string()))?;
            let mut out = Cursor::new(Vec::new());
            let result = if img.color().has_alpha() {
                image::DynamicImage::ImageRgba8(img.to_rgba8()).write_to(&mut out, image::ImageFormat::Png)
            } else {
                image::DynamicImage::ImageRgb8(img.to_rgb8()).write_to(&mut out, image::ImageFormat::Png)
            };
            result.map_err(|e| DerivativeError::Failed(e.to_string()))?;
            Ok(out.into_inner())
        }
        "heic" | "heif" | "avif" => external_jpeg(bytes, ty.ext),
        _ => Err(DerivativeError::Failed(format!("{} needs no derivative", ty.ext))),
    }
}

/// Which external converter this machine has, if any.
pub fn converter() -> Option<&'static str> {
    if cfg!(target_os = "macos") && std::path::Path::new("/usr/bin/sips").is_file() {
        return Some("sips");
    }
    for c in ["magick", "convert", "heif-convert"] {
        if let Ok(path) = std::env::var("PATH")
            && std::env::split_paths(&path).any(|d| d.join(c).is_file())
        {
            return Some(c);
        }
    }
    None
}

fn external_jpeg(bytes: &[u8], ext: &str) -> Result<Vec<u8>, DerivativeError> {
    let converter = converter().ok_or(DerivativeError::NoConverter)?;
    let dir = tempfile::Builder::new()
        .prefix("annotary-derivative-")
        .tempdir()
        .map_err(|e| DerivativeError::Failed(e.to_string()))?;
    let input = dir.path().join(format!("in.{ext}"));
    let output = dir.path().join("out.jpg");
    std::fs::write(&input, bytes).map_err(|e| DerivativeError::Failed(e.to_string()))?;
    let status = match converter {
        "sips" => Command::new("/usr/bin/sips")
            .args(["-s", "format", "jpeg", "-s", "formatOptions", "85"])
            .arg(&input)
            .arg("--out")
            .arg(&output)
            .output(),
        "heif-convert" => Command::new(converter).arg(&input).arg(&output).output(),
        _ => Command::new(converter).arg(&input).arg("-quality").arg("85").arg(&output).output(),
    }
    .map_err(|e| DerivativeError::Failed(format!("{converter}: {e}")))?;
    if !status.status.success() {
        let err = String::from_utf8_lossy(&status.stderr);
        let line = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
        return Err(DerivativeError::Failed(format!("{converter}: {line}")));
    }
    let out = std::fs::read(&output).map_err(|e| DerivativeError::Failed(format!("{converter} wrote nothing: {e}")))?;
    if !out.starts_with(&[0xFF, 0xD8]) {
        return Err(DerivativeError::Failed(format!("{converter} did not produce a JPEG")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::types::type_of;

    fn fixture(name: &str) -> Vec<u8> {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets").join(name);
        std::fs::read(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
    }

    // spec: S03-07
    #[test]
    fn dimensions_from_image_and_video_fixtures() {
        for (file, expect) in [
            ("a.png", (32, 20)),
            ("a.jpg", (32, 20)),
            ("a.gif", (32, 20)),
            ("a.webp", (32, 20)),
            ("a.bmp", (32, 20)),
            ("a.tiff", (32, 20)),
            ("a.heic", (32, 20)),
            ("a.avif", (32, 20)),
            ("a.svg", (120, 80)),
            ("viewbox.svg", (300, 150)),
            ("a.mp4", (48, 32)),
            ("a.mov", (48, 32)),
            ("rot.mp4", (32, 48)),
            ("a.webm", (48, 32)),
        ] {
            let ty = type_of(file).unwrap();
            assert_eq!(dimensions(&fixture(file), ty.content_type), Some(expect), "{file}");
        }
        assert_eq!(dimensions(&fixture("rot.jpg"), "image/jpeg"), Some((20, 32)), "EXIF orientation 6 swaps");
        assert_eq!(dimensions(&fixture("a.mp3"), "audio/mpeg"), None);
        assert_eq!(dimensions(&fixture("a.pdf"), "application/pdf"), None);
        assert_eq!(dimensions(b"not an image", "image/png"), None);
        assert_eq!(dimensions(b"", "video/mp4"), None);
        assert_eq!(dimensions(b"<svg width=\"0\">", "image/svg+xml"), None);
        assert_eq!(svg_dimensions(b"<svg xmlns='x' height='10em' width='5%'></svg>"), None);
    }

    // spec: S03-08
    #[test]
    fn tiff_and_bmp_derivatives_are_png() {
        for file in ["a.tiff", "a.bmp"] {
            let ty = type_of(file).unwrap();
            let png = derivative(&fixture(file), ty).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert!(png.starts_with(b"\x89PNG"), "{file}");
            assert_eq!(dimensions(&png, "image/png"), Some((32, 20)));
        }
        assert!(matches!(derivative(b"garbage", type_of("x.bmp").unwrap()), Err(DerivativeError::Failed(_))));
        assert!(matches!(derivative(b"x", type_of("x.png").unwrap()), Err(DerivativeError::Failed(_))));
    }

    // spec: S03-08 — HEIC/AVIF decoding needs the OS (macOS) or ImageMagick; skipped elsewhere.
    #[test]
    fn heic_and_avif_derivatives_are_jpeg() {
        if converter().is_none() {
            eprintln!("skipped: no HEIC/AVIF converter on this machine");
            return;
        }
        for file in ["a.heic", "a.avif"] {
            let ty = type_of(file).unwrap();
            let jpg = derivative(&fixture(file), ty).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert!(jpg.starts_with(&[0xFF, 0xD8]), "{file}");
            assert_eq!(dimensions(&jpg, "image/jpeg"), Some((32, 20)), "{file}");
        }
        assert!(matches!(derivative(b"garbage", type_of("x.heic").unwrap()), Err(DerivativeError::Failed(_))));
    }
}
