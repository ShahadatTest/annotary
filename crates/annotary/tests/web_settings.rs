//! CMD-23's Settings screen over HTTP against the in-process server: the route table (every
//! catalog group for every Member, an unknown group the 404 page, the owner pages under
//! `/settings/server/*`), the sidebar (the Server group for owners only), and the first paint —
//! every control in its current state from the store (Mixed, the amber layers icon, the scope line
//! only on groups with device settings), Profile and Devices. The interactions are
//! `tests/web/settings.spec.ts`.

use annotary::testing::TestServer;
use annotary_store::settings::{GROUPS, new_device_id};
use serde_json::{Value, json};

/// A page as a browser would fetch it (`Accept: text/html`) with a bearer: (status, body).
fn page(ts: &TestServer, path: &str, token: &str) -> (u16, String) {
    let mut r = ureq::get(format!("{}{path}", ts.url))
        .header("Accept", "text/html")
        .header("Authorization", format!("Bearer {token}"))
        .config()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .call()
        .unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap_or_default())
}

/// A request from an iOS Device (the owner's Token + the device headers).
fn as_device(ts: &TestServer, id: &str, info: &str, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
    let url = format!("{}{path}", ts.url);
    let auth = format!("Bearer {}", ts.owner_token);
    let res = match method {
        "GET" => ureq::get(&url)
            .header("Authorization", &auth)
            .header("X-Annotary-Device", id)
            .header("X-Annotary-Device-Info", info)
            .config()
            .http_status_as_error(false)
            .build()
            .call(),
        _ => ureq::put(&url)
            .header("Authorization", &auth)
            .header("X-Annotary-Device", id)
            .header("X-Annotary-Device-Info", info)
            .config()
            .http_status_as_error(false)
            .build()
            .send_json(body.unwrap_or(json!({}))),
    };
    let mut r = res.unwrap();
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

/// One row of the page (`<div class="cd-set-row" id="cd-set-KEY" …>` up to the next row).
fn row<'a>(html: &'a str, key: &str) -> &'a str {
    let start = html.find(&format!("id=\"cd-set-{key}\" data-key")).unwrap_or_else(|| panic!("no row {key}"));
    let rest = &html[start..];
    let end = rest[10..].find("class=\"cd-set-row\"").map(|i| i + 10).unwrap_or(rest.len());
    &rest[..end]
}

// spec: S29-20
#[test]
fn every_group_has_a_url_for_every_member_and_an_unknown_group_is_the_404_page() {
    let ts = TestServer::start();
    let (_, agent) = ts.add_member("scribe");
    for token in [ts.owner_token.as_str(), agent.as_str()] {
        for g in GROUPS {
            let (status, html) = page(&ts, &format!("/settings/{}", g.slug), token);
            assert_eq!(status, 200, "/settings/{}: {html}", g.slug);
            assert!(html.contains(&format!("<title>{} — annotary</title>", g.label)), "{}", g.slug);
            assert!(html.contains(&format!("<h1 class=\"cd-set-title\">{}</h1>", g.label)));
            assert!(html.contains(&format!(
                "<a class=\"cd-set-gl cd-current\" href=\"/settings/{}\" data-group=\"{}\" aria-current=\"page\">",
                g.slug, g.slug
            )));
            assert!(html.contains("class=\"cd-set-layout cd-set-grouppage\""));
            assert!(html.contains("id=\"cd-set-data\""));
            // Only the Settings pages carry the Settings script; never the owner script.
            assert!(html.contains("cd-set-ready") && !html.contains("cd-addagent"));
        }
        // `/settings` is the Profile group (wide) and the group list (narrow): one page.
        let (status, html) = page(&ts, "/settings", token);
        assert_eq!(status, 200);
        assert!(html.contains("class=\"cd-set-layout cd-set-index\""));
        assert!(html.contains("<h1 class=\"cd-set-title\">Profile</h1>") && html.contains("cd-set-listtitle"));
        // An unknown group, or an unknown Server page, is the HTML 404 page.
        for path in ["/settings/nope", "/settings/server", "/settings/server/nope"] {
            let (status, html) = page(&ts, path, token);
            if path == "/settings/server/nope" && token == agent {
                assert_eq!(status, 404, "{path}: unknown before owner-only");
            } else {
                assert_eq!(status, 404, "{path}");
            }
            assert!(html.contains("<!doctype html>") && html.contains("404"), "{path}: {html}");
        }
    }
    // The owner pages answer the owner; the sidebar's Server group is the owner's only.
    for (slug, title) in [("members", "Members"), ("vaults", "Vaults"), ("general", "Server settings")] {
        let (status, html) = page(&ts, &format!("/settings/server/{slug}"), &ts.owner_token);
        assert_eq!(status, 200, "{slug}");
        assert!(html.contains(&format!("<title>{title} — annotary</title>")));
        assert!(html.contains("class=\"cd-app cd-settingspage cd-adminpage"), "admin.js finds its page");
        let (status, _) = page(&ts, &format!("/settings/server/{slug}"), &agent);
        assert_eq!(status, 403, "{slug}");
    }
    let (_, owner_html) = page(&ts, "/settings/appearance", &ts.owner_token);
    assert!(owner_html.contains("<div class=\"cd-set-sidehead\" id=\"cd-set-serverhead\">Server</div>"));
    for href in ["/settings/server/members", "/settings/server/vaults", "/settings/server/general"] {
        assert!(owner_html.contains(&format!("href=\"{href}\"")), "{href}");
    }
    let (_, member_html) = page(&ts, "/settings/appearance", &agent);
    assert!(!member_html.contains("id=\"cd-set-serverhead\"") && !member_html.contains("href=\"/settings/server/"));
    // The search index rides on the page (no request), the Server pages for owners only.
    assert!(owner_html.contains("\"group\":\"server/vaults\""));
    assert!(!member_html.contains("\"group\":\"server/vaults\""));
    assert!(member_html.contains("Where new Documents go when you don't name a Vault."));
}

// spec: S29-21
#[test]
fn the_first_paint_shows_each_control_in_its_state_mixed_included() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    // Nothing set: All devices, the defaults, no Mixed, gray icons.
    let (_, html) = page(&ts, "/settings/appearance", &owner);
    assert!(html.contains("id=\"cd-set-scopebtn\"") && html.contains(">All devices</span>"));
    assert!(html.contains("Copy from another device"));
    let ap = row(&html, "appearance");
    assert!(ap.contains("data-value=\"system\" aria-checked=\"true\""), "{ap}");
    assert!(ap.contains("<span class=\"cd-set-mixed\" aria-hidden=\"true\" hidden>Mixed</span>"));
    assert!(ap.contains("class=\"cd-set-layers\" data-act=\"chain\" aria-label=\"Appearance\""));
    let label = ap.split("class=\"cd-set-label\"").nth(1).unwrap().split("class=\"cd-set-ctl\"").next().unwrap();
    assert!(!label.contains("Light or dark pages"), "help lines are never shown under a name: {label}");
    let hl = row(&html, "highlight_dark");
    assert!(hl.contains("aria-label=\"Yellow\" aria-checked=\"true\""));
    assert!(hl.contains("cd-set-sample-dark") && hl.contains("background:#facc15"));
    let ts_row = row(&html, "text_size");
    assert!(ts_row.contains(">100%</span>") && ts_row.contains("font-size:15px\">See how the text size looks."));
    assert!(
        row(&html, "highlight_light")
            .contains("See <mark class=\"cd-set-mark\" style=\"background:#fff1a8\">what\u{2019}s highlighted</mark>")
    );

    // An iPhone with its own Dark: All devices is Mixed — no value selected, the pill, the amber
    // icon, "Mixed" in the group's name.
    let iphone = new_device_id();
    let info = "kind=iphone; name=iPhone 17 Pro";
    let (status, body) = as_device(
        &ts,
        &iphone,
        info,
        "PUT",
        "/api/v1/me/settings",
        Some(json!({ "key": "appearance", "value": "dark", "scope": "devices", "devices": [iphone] })),
    );
    assert_eq!(status, 200, "{body}");
    let (status, body) = as_device(
        &ts,
        &iphone,
        info,
        "PUT",
        "/api/v1/me/settings",
        Some(json!({ "key": "text_size", "value": 130, "scope": "all" })),
    );
    assert_eq!(status, 200, "{body}");
    let (_, html) = page(&ts, "/settings/appearance", &owner);
    let ap = row(&html, "appearance");
    assert!(!ap.contains("aria-checked=\"true\""), "{ap}");
    assert!(ap.contains("<span class=\"cd-set-mixed\" aria-hidden=\"true\">Mixed</span>"));
    assert!(ap.contains("class=\"cd-set-layers cd-mixed\""));
    assert!(ap.contains("aria-label=\"Appearance: Mixed\""));
    // Text size was set for All devices: 130% everywhere, not Mixed.
    let ts_row = row(&html, "text_size");
    assert!(ts_row.contains(">130%</span>") && ts_row.contains("font-size:20px"), "{ts_row}");
    assert!(ts_row.contains("class=\"cd-set-layers\""));
    // The page carries what the script needs: the Devices (current first is the script's), the matrix.
    assert!(html.contains("\"current\":\"web\""));
    assert!(html.contains(&format!("\"id\":\"{iphone}\"")) && html.contains("\"name\":\"iPhone 17 Pro\""));

    // Keyboard keys its Mixed on Web and iPad only (an iPhone has no keyboard setting).
    let (_, html) = page(&ts, "/settings/keyboard", &owner);
    assert!(html.contains("id=\"cd-set-scope\""));
    assert!(row(&html, "keyboard_shortcuts").contains("data-value=\"on\" aria-checked=\"true\""));
    // Groups of user settings only: no picker, no layers icon, the "yours" line.
    for g in ["threads", "documents", "date-and-time", "profile"] {
        let (_, html) = page(&ts, &format!("/settings/{g}"), &owner);
        assert!(!html.contains("id=\"cd-set-scope\""), "{g}");
        assert!(!html.contains("class=\"cd-set-layers"), "{g}");
        assert!(html.contains("These settings are yours on <b>127.0.0.1:"), "{g}");
    }
    // A user setting away from its default shows Reset; Default Vault lists Any + readable Vaults.
    ts.create_vault("drafts");
    let vault = ts.store.list_vaults().unwrap().into_iter().find(|v| v.name == "drafts").unwrap().id;
    let mut r = ureq::put(format!("{}/api/v1/me/settings", ts.url))
        .header("Authorization", format!("Bearer {owner}"))
        .config()
        .http_status_as_error(false)
        .build()
        .send_json(json!({ "key": "default_vault", "value": vault }))
        .unwrap();
    assert_eq!(r.status().as_u16(), 200, "{}", r.body_mut().read_to_string().unwrap_or_default());
    let (_, html) = page(&ts, "/settings/documents", &owner);
    let dv = row(&html, "default_vault");
    assert!(dv.contains("<option value=\"any\">Any</option>"), "{dv}");
    assert!(dv.contains(&format!("<option value=\"{vault}\" selected>drafts</option>")), "{dv}");
    assert!(dv.contains("data-act=\"reset\">Reset</button>"));
    let (_, html) = page(&ts, "/settings/threads", &owner);
    assert!(row(&html, "show_resolved").contains("data-act=\"reset\" hidden>Reset"));
}

// spec: S29-23
#[test]
fn profile_and_devices_pages() {
    let ts = TestServer::start();
    let (_, agent) = ts.add_member("scribe");
    let (_, html) = page(&ts, "/settings/profile", &agent);
    assert!(html.contains("id=\"cd-set-name-in\" value=\""));
    assert!(html.contains("<span class=\"cd-set-handle\">@scribe</span>"));
    assert!(html.contains("cd-set-ro\">Member</div>"));
    let (_, html) = page(&ts, "/settings/profile", &ts.owner_token);
    assert!(html.contains("cd-set-ro\">Owner</div>"));
    // A Member renames themselves (the route the Profile page calls).
    let (status, m) = ts.post("/api/v1/me/rename", Some(&agent), &json!({ "name": "Scribe" }));
    assert_eq!(status, 200, "{m}");
    let (_, html) = page(&ts, "/settings/profile", &agent);
    assert!(html.contains("id=\"cd-set-name-in\" value=\"Scribe\""));

    // Devices: Web (this device, no Rename / Forget), an iPad with both.
    let ipad = new_device_id();
    let (status, _) = as_device(&ts, &ipad, "kind=ipad; name=iPad Pro 13%E2%80%B3", "GET", "/api/v1/me/settings", None);
    assert_eq!(status, 200);
    let (_, html) = page(&ts, "/settings/devices", &ts.owner_token);
    let web = html.split("data-device=\"web\"").nth(1).unwrap().split("</article>").next().unwrap();
    assert!(
        web.contains("<span class=\"cd-dev-name\">Web</span><span class=\"cd-dev-this\">this device</span>"),
        "{web}"
    );
    assert!(web.contains("Last seen now") && !web.contains("data-act"));
    let pad = html.split(&format!("data-device=\"{ipad}\"")).nth(1).unwrap().split("</article>").next().unwrap();
    assert!(pad.contains("<span class=\"cd-dev-name\">iPad Pro 13\u{2033}</span>"), "{pad}");
    assert!(pad.contains("data-act=\"rename\">Rename</button>") && pad.contains("data-act=\"forget\">Forget</button>"));
    assert!(!pad.contains("this device"));
    assert!(html.find("data-device=\"web\"").unwrap() < html.find(&format!("data-device=\"{ipad}\"")).unwrap());
}
