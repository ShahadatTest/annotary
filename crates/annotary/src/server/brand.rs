//! The brand files before sign-in (M3b, `DESIGN.md` Brand): the favicon set, the touch and manifest
//! icons and the web manifest, built into the binary from `brand/` (which `scripts/brand/render.py`
//! renders from its masters), plus the page head's links and the title bar's inlined icon. A
//! browser asks for these without a cookie, even on the login page, so they sit outside
//! `web::require_web_auth`; nothing here reads the store.

use std::sync::LazyLock;

use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use axum::{Router, routing::get};

use super::app::AppState;

const ICON_SVG: &str = include_str!("../../../../brand/icon.svg");

/// The web manifest. Its icons are routes below, because a browser fetches them before sign-in too.
const MANIFEST: &str = r##"{
  "name": "Annotary",
  "short_name": "Annotary",
  "start_url": "/",
  "scope": "/",
  "display": "standalone",
  "background_color": "#1c1c1e",
  "theme_color": "#1c1c1e",
  "icons": [
    { "src": "/icon-192.png", "sizes": "192x192", "type": "image/png" },
    { "src": "/icon-512.png", "sizes": "512x512", "type": "image/png" },
    { "src": "/icon-mask.png", "sizes": "512x512", "type": "image/png", "purpose": "maskable" }
  ]
}
"##;

/// The file names are unversioned, so a browser may keep them a day and no longer.
const CACHE: &str = "public, max-age=86400";
/// The icon is same-origin with the app, so it gets what a served SVG Asset gets (`assets.rs`).
const SVG_CSP: &str = "default-src 'none'; style-src 'unsafe-inline'";

/// Route, content type and bytes of every public brand file.
const FILES: [(&str, &str, &[u8]); 7] = [
    ("/favicon.ico", "image/x-icon", include_bytes!("../../../../brand/favicon.ico")),
    ("/icon.svg", "image/svg+xml", ICON_SVG.as_bytes()),
    ("/apple-touch-icon.png", "image/png", include_bytes!("../../../../brand/apple-touch-icon.png")),
    ("/icon-192.png", "image/png", include_bytes!("../../../../brand/icon-192.png")),
    ("/icon-512.png", "image/png", include_bytes!("../../../../brand/icon-512.png")),
    ("/icon-mask.png", "image/png", include_bytes!("../../../../brand/icon-mask.png")),
    ("/manifest.webmanifest", "application/manifest+json", MANIFEST.as_bytes()),
];

/// What every page's `<head>` links (`ui::shell_ex`).
pub const HEAD_LINKS: &str = "<link rel=\"icon\" href=\"/favicon.ico\" sizes=\"32x32\">\
<link rel=\"icon\" href=\"/icon.svg\" type=\"image/svg+xml\">\
<link rel=\"apple-touch-icon\" href=\"/apple-touch-icon.png\">\
<link rel=\"manifest\" href=\"/manifest.webmanifest\">";

/// The title bar's 18 px icon, linking to the Document list. The small master inlined, not an
/// `<img>`: an image would arrive after the first paint, and the first paint is final.
pub fn header_icon() -> &'static str {
    static HTML: LazyLock<String> = LazyLock::new(|| {
        let svg = ICON_SVG.trim().replacen(
            "<svg ",
            "<svg class=\"cd-icon\" width=\"18\" height=\"18\" aria-hidden=\"true\" ",
            1,
        );
        format!("<a class=\"cd-brand\" href=\"/\" aria-label=\"Documents\">{svg}</a>")
    });
    &HTML
}

/// The public routes, merged beside (never into) the signed-in web pages.
pub fn routes() -> Router<AppState> {
    FILES.iter().fold(Router::new(), |router, &(path, content_type, bytes)| {
        router.route(path, get(move || async move { file(content_type, bytes) }))
    })
}

fn file(content_type: &'static str, bytes: &'static [u8]) -> Response {
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static(CACHE));
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if content_type == "image/svg+xml" {
        h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(SVG_CSP));
    }
    resp
}
