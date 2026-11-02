//! annotary renderer: Markdown → viewer-agnostic HTML (ticket 09; ADR 0001 "compute at the
//! edge"). The pushing CLI renders and uploads the HTML as a content-addressed blob; the server
//! keeps this same crate as the fallback (`rerender`, missing / too-old HTML); Phase 2 compiles
//! it to WASM for the browser. It therefore builds for `wasm32-unknown-unknown` (std is fine
//! there; `make wasm-check`).
//!
//! One superset, no flavour detection: GFM (tables, task lists, strikethrough, extended
//! autolinks), footnotes, fenced code (`class="language-x"`, no baked highlighting), Mermaid
//! fences as `<pre class="mermaid">` for viewer JS, math (`$…$`, `$$…$$`, ```` ```math ````) as
//! MathML, YAML front matter parsed out (its `title:` is display-only). Output guardrails:
//! semantic elements only, `data-l="L-L"` (1-based, inclusive source lines) on every block-level
//! element the renderer opens, no `<style>` / `<script>` / chrome (raw HTML is sanitized), local
//! Asset references rewritten to `/a/{hash}` from the manifest (`?derivative=1` for
//! HEIC/HEIF/AVIF/TIFF/BMP, `width`/`height` from the manifest, SVG via `<img>`), `missing`
//! Assets as `<span class="asset-missing" data-ref="…">`, Asset links as plain `<a href="/a/…">`.
//! Byte-stable: the same (Markdown, manifest, [`RENDERER_VERSION`]) always gives the same bytes.
//!
//! - [`render`] / [`render_html`] — the renderer; [`title`] — the display title only.
//! - [`Asset::from_manifest`] — the manifest JSON (commit body / stored Version) → what the
//!   renderer needs; the CLI and the server both go through it, so their renders agree.
//! - [`refs`] — `MARKDOWN_OPTIONS`, `classify`, `normalize`: shared with the push-time scan.
//! - [`RENDERER_VERSION`] is recorded on every Version; [`MIN_RENDERER_VERSION`] is the oldest
//!   the server still serves as-is ([`is_current`]) — bump it when a rendering fix should be
//!   applied retroactively on read.

pub mod assets;
pub mod autolink;
pub mod escape;
pub mod html;
pub mod refs;
pub mod sanitize;

pub use assets::{Asset, Kind};
pub use html::{Options, Rendered, render};

/// Renderer version recorded on every Version (`renderer` field). Bump on any output change.
pub const RENDERER_VERSION: &str = "1.0.0";

/// The oldest recorded renderer whose HTML the server serves as-is; anything older (or missing,
/// or unparsable) is re-rendered by the server with this crate on first read and cached.
pub const MIN_RENDERER_VERSION: &str = "1.0.0";

/// `render` with a manifest and embeds on — the Document render.
pub fn render_html(md: &str, assets: &[Asset]) -> String {
    render(md, &Options { assets, embeds: true }).html
}

/// The display title of a Markdown text: front matter `title:` else the first `# H1`.
pub fn title(md: &str) -> Option<String> {
    render(md, &Options::default()).title
}

/// Parse `a.b.c[-…]` into a comparable triple (missing parts are 0; garbage is `None`).
pub fn parse_version(v: &str) -> Option<(u64, u64, u64)> {
    let core = v.trim().split(['-', '+']).next()?;
    let mut it = core.split('.');
    let major = it.next()?.parse().ok()?;
    let minor = it.next().unwrap_or("0").parse().ok()?;
    let patch = it.next().unwrap_or("0").parse().ok()?;
    if it.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Whether HTML rendered by `recorded` (a Version's `renderer` field) is still current, i.e.
/// at least [`MIN_RENDERER_VERSION`]. `None`/garbage ⇒ not current (the server re-renders).
pub fn is_current(recorded: Option<&str>) -> bool {
    match (recorded.and_then(parse_version), parse_version(MIN_RENDERER_VERSION)) {
        (Some(r), Some(m)) => r >= m,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_semver_triples() {
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("1.2.3-beta+x"), Some((1, 2, 3)));
        assert_eq!(parse_version("2"), Some((2, 0, 0)));
        assert_eq!(parse_version("x.y"), None);
        assert_eq!(parse_version("1.2.3.4"), None);
        assert!(is_current(Some(RENDERER_VERSION)));
        assert!(is_current(Some("99.0.0")), "a newer client's render is never downgraded");
        assert!(!is_current(Some("0.0.0")));
        assert!(!is_current(Some("garbage")));
        assert!(!is_current(None));
        assert!(parse_version(RENDERER_VERSION) >= parse_version(MIN_RENDERER_VERSION));
    }
}
