//! QR codes for pairing a device (ticket 18): one encoder (`qrcode`), two renderings — an SVG
//! (error correction M) for the web Members screen / the API (`POST /api/v1/pairing` `qr_svg`)
//! and Unicode quadrant blocks (error correction L) for `annotary pair` on a terminal.

use qrcode::render::svg;
use qrcode::{Color, EcLevel, QrCode};

/// An inline SVG of a QR code of `text`: black modules on white, a 4-module quiet zone, no XML
/// declaration and no external references (safe to put straight into a page).
pub fn svg(text: &str) -> String {
    let Ok(code) = QrCode::new(text.as_bytes()) else { return String::new() };
    let image = code
        .render::<svg::Color>()
        .min_dimensions(232, 232)
        .quiet_zone(true)
        .dark_color(svg::Color("#000000"))
        .light_color(svg::Color("#ffffff"))
        .build();
    match image.find("<svg") {
        Some(i) => image[i..].to_string(),
        None => image,
    }
}

/// Light modules around the terminal code: half the spec's 4 — VisionKit's scanner and the
/// iOS Camera read it, and the code stays small.
const TERMINAL_QUIET: usize = 2;

/// A QR code of `text` drawn with Unicode quadrant blocks (2×2 modules per character) for a
/// terminal: about half the width and two thirds the height of half blocks at error correction
/// M. A screen is never smudged, so it takes level L (fewer modules). Terminals are mostly dark,
/// so the light modules (and the quiet zone) are the drawn ones — the convention of
/// `qrencode -t UTF8`; the code then scans as black-on-white.
pub fn terminal(text: &str) -> String {
    let Ok(code) = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::L) else { return String::new() };
    let n = code.width();
    let colors = code.to_colors();
    let size = n + 2 * TERMINAL_QUIET;
    // Outside the code (the quiet zone, and the padding row/column of an odd size) is light.
    let light = |x: usize, y: usize| {
        let (Some(cx), Some(cy)) = (x.checked_sub(TERMINAL_QUIET), y.checked_sub(TERMINAL_QUIET)) else { return true };
        cx >= n || cy >= n || colors[cy * n + cx] == Color::Light
    };
    // Indexed by the drawn (light) quadrants: top-left 1, top-right 2, bottom-left 4, bottom-right 8.
    const GLYPHS: [char; 16] = [' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█'];
    let mut out = String::new();
    for y in (0..size).step_by(2) {
        if y > 0 {
            out.push('\n');
        }
        for x in (0..size).step_by(2) {
            let bits = usize::from(light(x, y))
                | usize::from(light(x + 1, y)) << 1
                | usize::from(light(x, y + 1)) << 2
                | usize::from(light(x + 1, y + 1)) << 3;
            out.push(GLYPHS[bits]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    // spec: S18-08
    #[test]
    fn svg_is_inline_black_on_white_without_external_refs() {
        let s = super::svg("annotary://pair?v=1&server=http%3A%2F%2Fh%3A7377&token=ant_x&profile=rowan%40h");
        assert!(s.starts_with("<svg"), "{s}");
        assert!(s.contains("#000000") && s.contains("#ffffff"));
        assert!(!s.contains("<?xml") && !s.contains("href") && !s.contains("<image"));
        assert!(s.trim_end().ends_with("</svg>"));
    }

    // spec: S18-09
    #[test]
    fn terminal_rendering_uses_quadrant_blocks_with_a_drawn_quiet_zone() {
        let t = super::terminal("annotary://pair?v=1");
        let lines: Vec<&str> = t.lines().collect();
        assert!(lines.len() > 10);
        // The quiet zone is light, i.e. drawn: the first row is all full blocks.
        assert!(lines[0].chars().all(|c| c == '\u{2588}'), "{}", lines[0]);
        // Square: as many characters across as rows down.
        assert!(lines.iter().all(|l| l.chars().count() == lines.len()), "{t}");
    }

    // spec: S18-09
    #[test]
    fn terminal_code_of_a_real_pair_url_is_under_half_the_width_of_half_blocks() {
        // The shape of the live server's pair URL (151 bytes): half blocks at level M drew it in
        // 61 columns × 31 rows, which filled most of a terminal.
        let url = format!(
            "annotary://pair?v=1&server=http%3A%2F%2Fstudio.example.ts.net%3A7377&token=ant_{}&profile=rowan%40studio",
            "x".repeat(49)
        );
        assert_eq!(url.len(), 151);
        let t = super::terminal(&url);
        let rows = t.lines().count();
        let cols = t.lines().map(|l| l.chars().count()).max().unwrap_or(0);
        assert!(cols <= 30 && rows <= 27, "{cols} columns × {rows} rows");
    }

    // spec: S18-09
    #[test]
    fn terminal_quadrants_round_trip_to_the_module_grid() {
        use qrcode::{Color, EcLevel, QrCode};
        let text = "annotary://pair?v=1&token=ant_abc";
        let code = QrCode::with_error_correction_level(text.as_bytes(), EcLevel::L).unwrap();
        let (n, colors) = (code.width(), code.to_colors());
        let q = super::TERMINAL_QUIET;
        let grid: Vec<Vec<char>> = super::terminal(text).lines().map(|l| l.chars().collect()).collect();
        for y in 0..n {
            for x in 0..n {
                let (gx, gy) = (x + q, y + q);
                let glyph = grid[gy / 2][gx / 2];
                let bit = 1 << ((gx % 2) + 2 * (gy % 2));
                let drawn = "\u{2598}\u{259d}\u{2580}\u{2596}\u{258c}\u{259e}\u{259b}\u{2597}\u{259a}\u{2590}\u{259c}\u{2584}\u{2599}\u{259f}\u{2588}"
                    .chars()
                    .position(|c| c == glyph)
                    .map_or(0, |i| i + 1);
                assert_eq!(drawn & bit != 0, colors[y * n + x] == Color::Light, "module ({x},{y})");
            }
        }
    }
}
