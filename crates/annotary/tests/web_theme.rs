//! CMD-23: the web pages read the signed-in Member's Web settings in the same server response, so
//! the first paint is final — the theme class (and, for Follow system, both palettes in the CSS),
//! the highlight colors and text size as `.cd-app` variables, Show resolved, absolute times once
//! the browser's zone is known (`cd_tz`), and the same values for every browser (Web is one
//! Device). The browser side (colors drawn, live changes) is `tests/web/theme.spec.ts`.

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use serde_json::{Value, json};

const MD: &str = "# Theme\n\nAlpha paragraph to anchor a Thread on.\n\nBeta paragraph.\n";

fn upload(ts: &TestServer, token: &str, md: &[u8]) -> String {
    let hash = sha256_hex(md);
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(md)
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    hash
}

/// A Document with one open and one resolved Thread.
fn seed(ts: &TestServer) -> String {
    let tok = &ts.owner_token;
    let hash = upload(ts, tok, MD.as_bytes());
    let (status, doc) =
        ts.post("/api/v1/docs", Some(tok), &json!({ "vault": "notes", "name": "theme", "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    let id = doc["id"].as_str().unwrap().to_string();
    for (range, body) in [("3:0-3:5", "open one"), ("5:0-5:4", "resolved one")] {
        let (status, t) = ts.post(
            &format!("/api/v1/docs/{id}/threads"),
            Some(tok),
            &json!({ "subject": "span", "range": range, "body": body }),
        );
        assert_eq!(status, 201, "{t}");
    }
    let (status, r) = ts.post(&format!("/api/v1/docs/{id}/threads/t2/resolve"), Some(tok), &json!({}));
    assert!(status == 200 || status == 201, "{status} {r}");
    id
}

/// `PUT /api/v1/me/settings` as the owner: a device setting for All devices (Web included), a
/// user setting for the Member.
fn set(ts: &TestServer, key: &str, value: Value) {
    let user = annotary_store::settings::setting(key).expect("a catalog key").kind
        == annotary_store::settings::SettingKind::User;
    let scope = if user { "user" } else { "all" };
    let res = ureq::put(format!("{}/api/v1/me/settings", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .config()
        .http_status_as_error(false)
        .build()
        .send_json(json!({ "key": key, "value": value, "scope": scope }))
        .unwrap();
    assert_eq!(res.status().as_u16(), 200, "PUT {key}");
}

/// A browser page request (loopback = the owner): (status, set-cookie, body).
fn page_with(ts: &TestServer, path: &str, cookie: Option<&str>) -> (u16, Option<String>, String) {
    let mut req = ureq::get(format!("{}{path}", ts.url)).header("Accept", "text/html");
    if let Some(c) = cookie {
        req = req.header("Cookie", c);
    }
    let mut r = req.config().http_status_as_error(false).max_redirects(0).build().call().unwrap();
    let set_cookie =
        r.headers().get("set-cookie").and_then(|v| v.to_str().ok()).map(|v| v.split(';').next().unwrap().to_string());
    (r.status().as_u16(), set_cookie, r.body_mut().read_to_string().unwrap_or_default())
}

fn page(ts: &TestServer, path: &str) -> String {
    let (status, _, body) = page_with(ts, path, None);
    assert_eq!(status, 200, "{path}: {body}");
    body
}

/// The opening tag of `.cd-app`.
fn app_tag(html: &str) -> &str {
    let at = html.find("<div class=\"cd-app").expect("a .cd-app");
    &html[at..at + html[at..].find('>').unwrap() + 1]
}

// spec: S29-24
#[test]
fn follow_system_carries_both_palettes_and_every_page_opens_cd_app_with_the_settings() {
    let ts = TestServer::start();
    let id = seed(&ts);
    for path in [format!("/d/{id}"), format!("/d/{id}/v/1"), "/".to_string()] {
        let html = page(&ts, &path);
        let tag = app_tag(&html);
        assert!(tag.contains(" cd-theme-system"), "{path}: {tag}");
        assert!(tag.contains("--cd-hl-light:#fff1a8;--cd-hl-light-sel:#ffd54a;--cd-hl-dark:#facc15;"), "{tag}");
        assert!(tag.contains("--cd-text-scale:1\""), "{tag}");
        assert!(!tag.contains("cd-show-resolved"), "{tag}");
        // Both palettes are in the page: light on :root, dark for .cd-theme-dark and, for Follow
        // system, under the media query.
        assert!(html.contains(":root{color-scheme:light;"));
        assert!(html.contains(".cd-app.cd-theme-dark{color-scheme:dark;"));
        let media = html.find("@media (prefers-color-scheme: dark){").expect("the dark media query");
        assert!(
            html[media..]
                .starts_with("@media (prefers-color-scheme: dark){\n.cd-app.cd-theme-system{color-scheme:dark;")
        );
    }
    // The pages nobody is signed in to follow the system too: the error page and the login page.
    let html = page_with(&ts, "/d/nope0000", None).2;
    assert!(app_tag(&html).contains("cd-theme-system"), "{}", app_tag(&html));
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some("hunter2")).unwrap();
    let html = page(&ts, "/login");
    let tag = app_tag(&html);
    assert!(tag.contains("cd-loginpage") && tag.contains("cd-theme-system"), "{tag}");
}

// spec: S29-24
#[test]
fn light_dark_highlights_text_size_and_show_resolved_are_in_the_first_response() {
    let ts = TestServer::start();
    let id = seed(&ts);
    set(&ts, "appearance", json!("dark"));
    let html = page(&ts, &format!("/d/{id}"));
    assert!(app_tag(&html).contains(" cd-theme-dark"), "{}", app_tag(&html));
    set(&ts, "appearance", json!("light"));
    set(&ts, "highlight_light", json!("green"));
    set(&ts, "highlight_dark", json!("#FF00FF"));
    set(&ts, "text_size", json!(130));
    set(&ts, "show_resolved", json!("shown"));
    for path in [format!("/d/{id}"), "/".to_string()] {
        let html = page(&ts, &path);
        let tag = app_tag(&html);
        assert!(tag.contains(" cd-theme-light"), "{path}: {tag}");
        // Green's light pair, the custom dark color (lowercased) with its derived selected shade.
        assert!(
            tag.contains(
                "--cd-hl-light:#c8f2d0;--cd-hl-light-sel:#86e29b;--cd-hl-dark:#ff00ff;--cd-hl-dark-sel:#ff4dff;"
            ),
            "{tag}"
        );
        assert!(tag.contains("--cd-text-scale:1.3\""), "{tag}");
        assert!(tag.contains(" cd-show-resolved"), "{tag}");
    }
    // Show resolved: the toggle starts checked, and the resolved card is in the page.
    let html = page(&ts, &format!("/d/{id}"));
    assert!(html.contains("id=\"cd-resolved\" checked>"), "the toggle is checked");
    assert!(html.contains("class=\"cd-thread cd-st-anchored cd-resolved"), "the resolved card is there");
    // The keyboard settings ride along as data attributes.
    set(&ts, "keyboard_shortcuts", json!("off"));
    set(&ts, "send_key", json!("mod_enter"));
    let html = page(&ts, &format!("/d/{id}"));
    let tag = app_tag(&html);
    assert!(tag.contains("data-cd-keys=\"off\" data-cd-send=\"mod_enter\""), "{tag}");
    assert!(html.contains("<kbd id=\"cd-send-key\">\u{2318}\u{21A9}</kbd>"), "the help names the Send key's chord");
}

// spec: S29-24
#[test]
fn a_signed_in_members_error_pages_wear_their_appearance() {
    let ts = TestServer::start();
    set(&ts, "appearance", json!("dark"));
    for path in ["/d/nope0000", "/settings/nope", "/settings/server/nope"] {
        let (status, _, html) = page_with(&ts, path, None);
        assert_eq!(status, 404, "{path}");
        assert!(app_tag(&html).contains("cd-theme-dark"), "{path}: {}", app_tag(&html));
    }
}

// spec: S29-24
#[test]
fn absolute_times_render_on_the_server_once_the_zone_cookie_is_known() {
    let ts = TestServer::start();
    let id = seed(&ts);
    set(&ts, "times", json!("absolute"));
    set(&ts, "clock", json!("24"));
    // No zone yet: relative ages as before, nothing absolute.
    let html = page(&ts, &format!("/d/{id}"));
    assert!(html.contains("<time class=\"cd-rel\" datetime=\"20"));
    assert!(!html.contains("<time class=\"cd-abs"));
    assert!(!app_tag(&html).contains("data-cd-tz"));
    // With the browser's zone: every <time> is absolute, in that zone, 24-hour.
    let (status, _, html) = page_with(&ts, &format!("/d/{id}"), Some("cd_tz=Asia/Tokyo; cd_hc=12"));
    assert_eq!(status, 200);
    assert!(app_tag(&html).contains("data-cd-times=\"absolute\" data-cd-clock=\"24\""), "{}", app_tag(&html));
    assert!(app_tag(&html).contains("data-cd-tz=\"Asia/Tokyo\""));
    assert!(!html.contains("<time class=\"cd-rel\" datetime=\"20"), "no relative age left");
    let now = jiff::Timestamp::now().to_zoned(jiff::tz::TimeZone::get("Asia/Tokyo").unwrap());
    let day = now.strftime("%b %-d, ").to_string();
    let abs = html.matches("<time class=\"cd-abs cd-localized\"").count();
    assert!(abs >= 4, "cards, comments and the resolve line: {abs}");
    assert!(html.contains(&format!("JST\">{day}")), "Tokyo's date: {day}");
    assert!(!html.contains(" AM</time>") && !html.contains(" PM</time>"), "24-hour");
    // Clock: Follow system takes the browser's hour cycle (`cd_hc`).
    set(&ts, "clock", json!("system"));
    let (_, _, html) = page_with(&ts, "/", Some("cd_tz=Asia/Tokyo; cd_hc=12"));
    assert!(html.contains(" AM</time>") || html.contains(" PM</time>"), "12-hour from cd_hc");
    let (_, _, html) = page_with(&ts, "/", Some("cd_tz=Asia/Tokyo; cd_hc=24"));
    assert!(html.contains("<time class=\"cd-abs") && !html.contains(" AM</time>") && !html.contains(" PM</time>"));
    // The card the page swaps in after a mutation renders its times the same way.
    let res = ureq::get(format!("{}/d/{id}/t/t1", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .header("Cookie", "cd_tz=Asia/Tokyo; cd_hc=24")
        .call()
        .unwrap()
        .body_mut()
        .read_json::<Value>()
        .unwrap();
    assert!(res["html"].as_str().unwrap().contains("cd-abs cd-localized"), "{res}");
    // A cookie that is not a zone is ignored.
    let (_, _, html) = page_with(&ts, "/", Some("cd_tz=Not/AZone"));
    assert!(!html.contains("<time class=\"cd-abs"));
}

// spec: S29-24
#[test]
fn two_browsers_resolve_the_same_web_values() {
    let ts = TestServer::start();
    let id = seed(&ts);
    set(&ts, "appearance", json!("dark"));
    set(&ts, "highlight_dark", json!("blue"));
    set(&ts, "text_size", json!(150));
    // Two browsers: each gets its own session on its first page.
    let (_, a, _) = page_with(&ts, &format!("/d/{id}"), None);
    let (_, b, _) = page_with(&ts, &format!("/d/{id}"), None);
    let (a, b) = (a.expect("a session"), b.expect("another session"));
    assert!(a.starts_with("cd_session=") && b.starts_with("cd_session=") && a != b);
    let (_, _, pa) = page_with(&ts, &format!("/d/{id}"), Some(&a));
    let (_, _, pb) = page_with(&ts, &format!("/d/{id}"), Some(&b));
    assert_eq!(app_tag(&pa), app_tag(&pb));
    let tag = app_tag(&pa);
    assert!(
        tag.contains("cd-theme-dark") && tag.contains("--cd-hl-dark:#60a5fa;") && tag.contains("--cd-text-scale:1.5\""),
        "{tag}"
    );
    // A change for Web alone (scope devices) reaches both browsers too.
    let res = ureq::put(format!("{}/api/v1/me/settings", ts.url))
        .header("Cookie", &a)
        .config()
        .http_status_as_error(false)
        .build()
        .send_json(json!({ "key": "appearance", "value": "light", "scope": "devices", "devices": ["web"] }))
        .unwrap();
    assert_eq!(res.status().as_u16(), 200);
    let (_, _, pb) = page_with(&ts, &format!("/d/{id}"), Some(&b));
    assert!(app_tag(&pb).contains("cd-theme-light"), "{}", app_tag(&pb));
}
