//! Shared chrome of the server-composed web pages (ticket 10): the page shell with the inlined
//! stylesheet and script (`ui/app.css`, `ui/app.js` — one response, no CSS/JS swap), HTML
//! escaping, `<time>` tags the browser localizes, Member display (display name + auth-method
//! glyph — never a "bot" badge), Action-intent classes, HTML error pages, and the display-title
//! cache behind the Document list. The Document view is `view.rs`, the list `listing.rs`.

use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};

use annotary_store::MemberView;

use super::app::AppState;
use super::brand::HEAD_LINKS;
use super::error::ApiError;

pub const CSS: &str = include_str!("ui/app.css");
pub const JS: &str = include_str!("ui/app.js");
// ---- ticket 12: the owner screens and the login page carry their own small stylesheet / script,
// inlined only there (the Document pages stay as light as ticket 10/11 left them) ----
pub const ADMIN_CSS: &str = include_str!("ui/admin.css");
pub const ADMIN_JS: &str = include_str!("ui/admin.js");
// ---- end ticket 12 ----

/// Escape text for HTML (text content and double-quoted attributes).
pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// The page shell: `<!doctype html>` … inlined CSS … `body` … inlined JS. `body` is trusted HTML.
pub fn shell(title: &str, body: &str) -> String {
    shell_ex(title, body, "", "")
}

// ---- ticket 12 ----
/// The shell with an extra inlined stylesheet / script after the shared ones (the owner screens
/// add `ADMIN_CSS` / `ADMIN_JS`; the login page `ADMIN_CSS` only).
pub fn shell_ex(title: &str, body: &str, extra_css: &str, extra_js: &str) -> String {
    // ---- ticket 15: the scripts are inline *module* scripts — deferred by definition, so the
    // browser paints the server-composed page before running them (a parser-blocking script at
    // the end of a 1.8 MB body held the first paint back until it had run; modules keep their
    // order, so admin.js still finds `window.cd`). Everything is still one response. ----
    let js2 = if extra_js.is_empty() { String::new() } else { format!("<script type=\"module\">{extra_js}</script>") };
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\
         <title>{}</title>{HEAD_LINKS}<style>{CSS}{extra_css}</style></head><body>{body}<script type=\"module\">{JS}</script>{js2}</body></html>\n",
        esc(title)
    )
}

/// A full page response built with [`shell_ex`] (`text/html`, never cached).
pub fn page_ex(title: &str, body: &str, extra_css: &str, extra_js: &str) -> Response {
    let mut resp = Html(shell_ex(title, body, extra_css, extra_js)).into_response();
    resp.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("private, no-cache"));
    resp
}

/// The shared top bar of the list page and the Settings pages: the icon, `annotary` + the nav
/// (`Documents` and `Settings`, for every Member — CMD-23: the owner pages are the Settings
/// sidebar's Server group, shown to owners only), the signed-in Member, and Sign out when a
/// password is set. `current` marks the active link (`"/"`, `"/settings"`).
pub fn top_nav(st: &AppState, auth: &super::auth::Auth, current: &str) -> Result<String, ApiError> {
    let password_set = st.store.password_set()?;
    let me = match st.store.member_by_id(&auth.0.member_id)? {
        Some(m) => MemberBrief::from_view(&m),
        None => MemberBrief::unknown(&auth.0.handle),
    };
    let link = |href: &str, id: &str, label: &str| {
        format!(
            "<a class=\"cd-navlink{}\" id=\"{id}\" href=\"{href}\"{}>{label}</a>",
            if href == current { " cd-current" } else { "" },
            if href == current { " aria-current=\"page\"" } else { "" }
        )
    };
    let mut nav = link("/", "cd-nav-docs", "Documents");
    nav.push_str(&link("/settings", "cd-nav-settings", "Settings"));
    let signout = if password_set {
        "<form method=\"post\" action=\"/logout\" class=\"cd-logout\"><button type=\"submit\" class=\"cd-btn\">Sign out</button></form>"
    } else {
        ""
    };
    Ok(format!(
        "<header class=\"cd-hdr cd-hdr-nav\"><div class=\"cd-line\">{}<b class=\"cd-name\">annotary</b><nav class=\"cd-nav\" id=\"cd-nav\">{nav}</nav></div>\
         <span class=\"cd-grow\"></span><span class=\"cd-status cd-me\">{} <span class=\"cd-muted\">@{} · {}</span></span>{signout}</header>",
        super::brand::header_icon(),
        me.html(),
        esc(&auth.0.handle),
        auth.0.role
    ))
}
// ---- end ticket 12 ----

/// A full page response: `text/html`, never cached (Threads and counts are live).
pub fn page(title: &str, body: &str) -> Response {
    page_ex(title, body, "", "")
}

/// A small centred page (errors). No signed-in Member to read settings for: it follows the
/// system's appearance (CMD-23).
pub fn simple(status: StatusCode, title: &str, inner: &str) -> Response {
    simple_with(&super::prefs::WebPrefs::default(), status, title, inner)
}

/// A small centred page wearing the given settings (CMD-23: a signed-in Member's error pages).
pub fn simple_with(prefs: &super::prefs::WebPrefs, status: StatusCode, title: &str, inner: &str) -> Response {
    let body = format!("<div {}><main class=\"cd-simple\">{inner}</main></div>", prefs.app_attrs(""));
    let mut resp = page(title, &body);
    *resp.status_mut() = status;
    resp
}

/// An [`ApiError`] as an HTML page (web routes answer browsers, not the CLI).
pub fn error_page(e: ApiError) -> Response {
    error_page_with(&super::prefs::WebPrefs::default(), e)
}

/// An error page for a signed-in Member, in their appearance (CMD-23); the system's when their
/// settings can't be read.
pub fn error_page_for(
    st: &AppState,
    auth: &super::auth::Auth,
    headers: &axum::http::HeaderMap,
    e: ApiError,
) -> Response {
    let prefs = super::prefs::WebPrefs::load(st, auth, headers).unwrap_or_default();
    error_page_with(&prefs, e)
}

fn error_page_with(prefs: &super::prefs::WebPrefs, e: ApiError) -> Response {
    let hint = e.hint.as_deref().map(|h| format!("<p class=\"cd-muted\">{}</p>", esc(h))).unwrap_or_default();
    simple_with(
        prefs,
        e.status,
        &format!("annotary — {}", e.status.as_u16()),
        &format!(
            "<h1>annotary</h1><p class=\"cd-err\">{} {}</p>{hint}<p><a href=\"/\">← Documents</a></p>",
            e.status.as_u16(),
            esc(&e.message)
        ),
    )
}

/// `<time datetime="ISO">fallback</time>` — the script localizes it; `rel` asks for a relative age.
/// <!-- ---- ticket 15 ---- --> A relative tag is rendered with the age the script would compute
/// (`rel_age`, the same rule as `fmtRel` in app.js, `title` = the absolute fallback): the
/// localization then changes nothing visible, so a row or banner painted before the script ran
/// does not shift (CLS 0 — a `2026-08-19 12:30Z` → `2h` rewrite moved the text after it).
/// <!-- ---- end ticket 15 ---- -->
pub fn time_tag(iso: &str, rel: bool) -> String {
    let fallback = fallback_time(iso);
    if rel {
        format!(
            "<time class=\"cd-rel\" datetime=\"{}\" title=\"{}\">{}</time>",
            esc(iso),
            esc(&fallback),
            esc(&rel_age(iso, jiff::Timestamp::now()))
        )
    } else {
        format!("<time datetime=\"{}\">{}</time>", esc(iso), esc(&fallback))
    }
}

// ---- ticket 15 ----
/// The web UI's relative age — **the same rule as `fmtRel` in `ui/app.js`** (keep them equal):
/// `now` under a minute, then `Nm` / `Nh` (under 48 h) / `Nd` (under 30 d) / `Nmo` / `Ny`, each
/// rounded to the nearest unit.
pub fn rel_age(iso: &str, now: jiff::Timestamp) -> String {
    let Some(t) = annotary_store::time::parse(iso) else {
        return iso.to_string();
    };
    let secs = (now.as_millisecond() - t.as_millisecond()).max(0) as f64 / 1000.0;
    if secs < 60.0 {
        return "now".into();
    }
    let m = secs / 60.0;
    if m < 60.0 {
        return format!("{}m", m.round());
    }
    let h = m / 60.0;
    if h < 48.0 {
        return format!("{}h", h.round());
    }
    let d = h / 24.0;
    if d < 30.0 {
        return format!("{}d", d.round());
    }
    if d < 365.0 {
        return format!("{}mo", (d / 30.0).round());
    }
    format!("{}y", (d / 365.0).round())
}
// ---- end ticket 15 ----

/// The pre-localization text of a timestamp: `2026-08-14 09:12Z` (UTC, minutes).
pub fn fallback_time(iso: &str) -> String {
    match annotary_store::time::parse(iso) {
        Some(t) => t.strftime("%Y-%m-%d %H:%MZ").to_string(),
        None => iso.to_string(),
    }
}

/// The CSS class for an Action intent.
pub fn intent_class(intent: &str) -> &'static str {
    match intent {
        "positive" => "cd-pos",
        "negative" => "cd-neg",
        _ => "cd-neu",
    }
}

/// What a page needs to show a Member: display name + how they authenticate. People and agents
/// read alike — the glyph says *how they sign in*, never *what they are*.
#[derive(Clone, Debug)]
pub struct MemberBrief {
    pub handle: String,
    pub name: String,
    /// `password` when the Member has a local password, else `token`.
    pub auth: &'static str,
    /// Agent harness / model as secondary text — nothing in the Phase 1 model carries it, so
    /// it is `None` today; the markup hook (`.cd-sec`) is in place.
    pub secondary: Option<String>,
}

impl MemberBrief {
    pub fn from_view(m: &MemberView) -> MemberBrief {
        MemberBrief {
            handle: m.handle.clone(),
            name: m.display_name.clone(),
            auth: if m.has_password { "password" } else { "token" },
            secondary: None,
        }
    }

    /// An unknown handle (nothing in `list_members` — cannot happen for stored rows, but never
    /// panic in a page).
    pub fn unknown(handle: &str) -> MemberBrief {
        MemberBrief { handle: handle.to_string(), name: format!("@{handle}"), auth: "token", secondary: None }
    }

    /// `<b title="@handle">Name<span class="cd-sec"> · harness</span></b><span class="cd-auth" title="…">⚷</span>`.
    pub fn html(&self) -> String {
        let (glyph, title) = match self.auth {
            "password" => ("\u{25CF}", "signs in with the local password"),
            _ => ("\u{26B7}", "authenticates with a Token"),
        };
        let sec = self
            .secondary
            .as_deref()
            .map(|s| format!("<span class=\"cd-sec\"> · {}</span>", esc(s)))
            .unwrap_or_default();
        format!(
            "<b class=\"cd-member\" title=\"@{}\">{}{sec}</b><span class=\"cd-auth cd-auth-{}\" title=\"{title}\">{glyph}</span>",
            esc(&self.handle),
            esc(&self.name),
            self.auth
        )
    }
}

/// Every Member by handle, for the pages (one query per page).
pub fn members_by_handle(st: &AppState) -> Result<std::collections::HashMap<String, MemberBrief>, ApiError> {
    Ok(st.store.list_members()?.iter().map(|m| (m.handle.clone(), MemberBrief::from_view(m))).collect())
}

/// The display title of a Version's Markdown (front matter `title:` else the first H1), cached
/// per blob hash on the server: the list shows it display-only (story 72) and must not render
/// 50 Documents per request.
pub fn title_for(st: &AppState, md_hash: &str) -> Result<Option<String>, ApiError> {
    if let Ok(cache) = st.titles.lock()
        && let Some(t) = cache.get(md_hash)
    {
        return Ok(t.clone());
    }
    let title = match st.store.read_blob(md_hash)? {
        Some(bytes) => annotary_renderer::title(&String::from_utf8_lossy(&bytes)),
        None => None,
    };
    if let Ok(mut cache) = st.titles.lock() {
        if cache.len() > 10_000 {
            cache.clear();
        }
        cache.insert(md_hash.to_string(), title.clone());
    }
    Ok(title)
}

/// Strip the renderer's ` data-l="A-B"` attributes (Comment bodies must not look like Document
/// blocks to the highlighter) — a plain scan over our own output, where the attribute only ever
/// appears inside a tag.
pub fn strip_data_l(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(pos) = rest.find(" data-l=\"") {
        out.push_str(&rest[..pos]);
        let after = &rest[pos + 9..];
        match after.find('"') {
            Some(q) => rest = &after[q + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

/// Asset links open in a new tab: `<a href="/a/…">` → `<a target="_blank" rel="noopener" href="/a/…">`.
pub fn asset_links_blank(html: &str) -> String {
    html.replace("<a href=\"/a/", "<a target=\"_blank\" rel=\"noopener\" href=\"/a/")
}

/// A Comment body as HTML: the Document superset minus embeds (images / media / files become
/// plain links), Asset links opening in a new tab, no `data-l`.
pub fn comment_html(body: &str) -> String {
    let r = annotary_renderer::render(body, &annotary_renderer::Options { assets: &[], embeds: false });
    asset_links_blank(&strip_data_l(&r.html))
}

/// Serialize a JSON value for an inline `<script type="application/json">` (no `</script>` escape).
pub fn json_for_script(v: &serde_json::Value) -> String {
    serde_json::to_string(v).unwrap_or_else(|_| "null".into()).replace("</", "<\\/")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_data_l_and_blanks_asset_links() {
        let html = "<p data-l=\"1-2\">x <a href=\"/a/abc\">y</a> <a href=\"https://x\">z</a></p>\n<ul data-l=\"4-5\"><li data-l=\"4-4\">a</li></ul>";
        assert_eq!(
            strip_data_l(html),
            "<p>x <a href=\"/a/abc\">y</a> <a href=\"https://x\">z</a></p>\n<ul><li>a</li></ul>"
        );
        assert_eq!(
            asset_links_blank("<a href=\"/a/abc\">y</a> <a href=\"/b\">"),
            "<a target=\"_blank\" rel=\"noopener\" href=\"/a/abc\">y</a> <a href=\"/b\">"
        );
        let c = comment_html("see ![img](pic.png) and `x` **b**");
        assert!(!c.contains("<img"), "{c}");
        assert!(!c.contains("data-l"), "{c}");
        assert!(c.contains("<strong>b</strong>"));
    }

    #[test]
    fn script_json_and_times() {
        assert_eq!(json_for_script(&serde_json::json!({"a": "</script><b>"})), "{\"a\":\"<\\/script><b>\"}");
        assert_eq!(fallback_time("2026-08-14T09:12:04.123Z"), "2026-08-14 09:12Z");
        assert!(
            time_tag("2026-08-14T09:12:04.123Z", true).starts_with(
                "<time class=\"cd-rel\" datetime=\"2026-08-14T09:12:04.123Z\" title=\"2026-08-14 09:12Z\">"
            )
        );
        // ticket 15: the server's relative age follows app.js's fmtRel exactly.
        let t0 = annotary_store::time::parse("2026-08-14T09:12:04.123Z").unwrap();
        let at = |secs: i64| t0.checked_add(jiff::SignedDuration::from_secs(secs)).unwrap();
        assert_eq!(rel_age("2026-08-14T09:12:04.123Z", at(30)), "now");
        assert_eq!(rel_age("2026-08-14T09:12:04.123Z", at(90)), "2m");
        assert_eq!(rel_age("2026-08-14T09:12:04.123Z", at(3600 * 3 + 1700)), "3h");
        assert_eq!(rel_age("2026-08-14T09:12:04.123Z", at(3600 * 47)), "47h");
        assert_eq!(rel_age("2026-08-14T09:12:04.123Z", at(86_400 * 2)), "2d");
        assert_eq!(rel_age("2026-08-14T09:12:04.123Z", at(86_400 * 29)), "29d");
        assert_eq!(rel_age("2026-08-14T09:12:04.123Z", at(86_400 * 45)), "2mo");
        assert_eq!(rel_age("2026-08-14T09:12:04.123Z", at(86_400 * 800)), "2y");
        assert_eq!(esc("a<b>&\"'"), "a&lt;b&gt;&amp;&quot;&#39;");
    }

    /// CMD-23: the dark palette is written twice in app.css (`.cd-theme-dark`, and
    /// `.cd-theme-system` under `prefers-color-scheme: dark`); the two must never drift.
    #[test]
    fn the_two_dark_blocks_of_app_css_are_identical() {
        let body = |sel: &str| {
            let at = CSS.find(&format!("{sel}{{")).unwrap_or_else(|| panic!("no {sel} block"));
            let rest = &CSS[at + sel.len()..];
            rest[..=rest.find('}').unwrap()].to_string()
        };
        let dark = body(".cd-app.cd-theme-dark");
        assert_eq!(dark, body(".cd-app.cd-theme-system"));
        assert!(dark.contains("color-scheme:dark") && dark.contains("--cd-bg:#000;"), "{dark}");
        let media = CSS.find("@media (prefers-color-scheme: dark){").unwrap();
        assert!(CSS[media..].contains(".cd-app.cd-theme-system{"));
        // Every token the light palette defines, the dark one redefines (but the font stack).
        let root = body(":root");
        for decl in root.split(';') {
            let Some((name, _)) = decl.trim_start_matches('{').trim().split_once(':') else { continue };
            if name.starts_with("--cd-") && name != "--cd-mono" {
                assert!(dark.contains(&format!("{name}:")), "dark palette misses {name}");
            }
        }
    }
}
