//! CMD-23 over HTTP against the in-process server: the catalog route and its refusals, the
//! device headers (made by settings routes only, malformed and forgotten ids refused), the sync
//! route and a stamped push, renaming oneself, the live feed's per-Member rule and that no
//! Member reaches another's settings or Devices. The store side is `settings_store.rs`; the
//! rights matrix rows are in `access_api.rs`.

use annotary::testing::TestServer;
use annotary_store::settings;
use serde_json::{Value, json};

/// A request with a Token and extra headers; returns (status, JSON body).
fn call(
    ts: &TestServer,
    method: &str,
    path: &str,
    token: &str,
    headers: &[(&str, &str)],
    body: Option<Value>,
) -> (u16, Value) {
    let url = format!("{}{}", ts.url, path);
    let auth = format!("Bearer {token}");
    let res = match method {
        "GET" | "DELETE" => {
            let mut r = if method == "GET" { ureq::get(&url) } else { ureq::delete(&url) };
            r = r.header("Authorization", &auth);
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.config().http_status_as_error(false).build().call()
        }
        _ => {
            let mut r = match method {
                "POST" => ureq::post(&url),
                "PUT" => ureq::put(&url),
                "PATCH" => ureq::patch(&url),
                _ => unreachable!(),
            };
            r = r.header("Authorization", &auth);
            for (k, v) in headers {
                r = r.header(*k, *v);
            }
            r.config().http_status_as_error(false).build().send_json(body.unwrap_or(json!({})))
        }
    };
    let mut r = res.unwrap();
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
}

fn phone_headers(id: &str) -> [(&'static str, String); 2] {
    [("X-Annotary-Device", id.to_string()), ("X-Annotary-Device-Info", "kind=iphone; name=iPhone 17 Pro".into())]
}

fn hs<'a>(h: &'a [(&'static str, String); 2]) -> Vec<(&'static str, &'a str)> {
    h.iter().map(|(k, v)| (*k, v.as_str())).collect()
}

// spec: S29-02
#[test]
fn the_catalog_is_served_and_unknown_keys_and_values_are_400() {
    let ts = TestServer::start();
    let t = ts.owner_token.clone();
    let (s, cat) = call(&ts, "GET", "/api/v1/me/settings/catalog", &t, &[], None);
    assert_eq!(s, 200);
    assert_eq!(cat["groups"][0]["slug"], "profile");
    assert!(cat["settings"].as_array().unwrap().iter().any(|v| v["key"] == "appearance" && v["kind"] == "device"));
    assert_eq!(cat["presets"].as_array().unwrap().len(), settings::PRESETS.len());
    let (s, e) = call(&ts, "PUT", "/api/v1/me/settings", &t, &[], Some(json!({ "key": "font", "value": "serif" })));
    assert_eq!((s, e["error"].as_str()), (400, Some("unknown_setting")));
    let (s, e) = call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &t,
        &[],
        Some(json!({ "key": "appearance", "value": "sepia", "scope": "all" })),
    );
    assert_eq!((s, e["error"].as_str()), (400, Some("invalid_setting")));
    let (s, e) =
        call(&ts, "PUT", "/api/v1/me/settings", &t, &[], Some(json!({ "key": "appearance", "value": "dark" })));
    assert_eq!(s, 400, "a device setting needs a scope: {e}");
    let (s, e) = call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &t,
        &[],
        Some(json!({ "key": "highlight_dark", "value": "#222222", "scope": "all" })),
    );
    assert_eq!((s, e["error"].as_str()), (400, Some("low_contrast")));
    assert!(e["ratio"].as_f64().unwrap() < 4.5);
    let (s, e) = call(&ts, "DELETE", "/api/v1/me/settings/nope", &t, &[], None);
    assert_eq!((s, e["error"].as_str()), (400, Some("unknown_setting")));
}

// spec: S29-08, S29-09
#[test]
fn settings_routes_make_the_requests_device_and_nothing_else_does() {
    let ts = TestServer::start();
    let t = ts.owner_token.clone();
    let id = settings::new_device_id();
    let h = phone_headers(&id);
    // An ordinary API call naming an unknown Device makes nothing.
    let (s, _) = call(&ts, "GET", "/api/v1/docs", &t, &hs(&h), None);
    assert_eq!(s, 200);
    assert!(ts.store.device(&ts.owner_id, &id).unwrap().is_none());
    // A malformed id is refused on any route.
    let (s, e) = call(&ts, "GET", "/api/v1/docs", &t, &[("X-Annotary-Device", "phone-1")], None);
    assert_eq!((s, e["error"].as_str()), (400, Some("bad_device")));
    // A settings route makes it, named from the info header.
    let (s, v) = call(&ts, "GET", "/api/v1/me/settings", &t, &hs(&h), None);
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["device"]["id"], json!(id));
    assert_eq!((v["device"]["kind"].as_str(), v["device"]["name"].as_str()), (Some("iphone"), Some("iPhone 17 Pro")));
    assert_eq!(
        v["settings"].as_array().unwrap().iter().find(|r| r["key"] == "appearance").unwrap()["source"],
        "default"
    );
    let (_, list) = call(&ts, "GET", "/api/v1/me/devices", &t, &hs(&h), None);
    assert_eq!(list["current"], json!(id));
    // The Token is now last used from that Device; forgetting it revokes the Token.
    let (other_tok, _) = (ts.store.mint_token(&ts.owner_id, &ts.owner_id, Some("other")).unwrap().plain, ());
    let (s, f) = call(&ts, "POST", &format!("/api/v1/me/devices/{id}/forget"), &other_tok, &[], None);
    assert_eq!(s, 200, "{f}");
    assert_eq!(f["tokens_revoked"], 1);
    let (s, _) = call(&ts, "GET", "/api/v1/me", &t, &[], None);
    assert_eq!(s, 401, "the Token used from the forgotten Device is revoked");
    let (s, e) = call(&ts, "GET", "/api/v1/docs", &other_tok, &hs(&h), None);
    assert_eq!((s, e["error"].as_str()), (409, Some("device_forgotten")));
    // Web can't be renamed or forgotten.
    let (s, e) = call(&ts, "PATCH", "/api/v1/me/devices/web", &other_tok, &[], Some(json!({ "name": "Safari" })));
    assert_eq!((s, e["error"].as_str()), (409, Some("web_device")));
    let (s, e) = call(&ts, "POST", "/api/v1/me/devices/web/forget", &other_tok, &[], None);
    assert_eq!((s, e["error"].as_str()), (409, Some("web_device")));
}

// spec: S29-11, S29-05
#[test]
fn sync_gives_own_and_all_rows_and_a_stamped_push_is_kept() {
    let ts = TestServer::start();
    let t = ts.owner_token.clone();
    let id = settings::new_device_id();
    let h = phone_headers(&id);
    let (s, v) = call(&ts, "GET", &format!("/api/v1/me/devices/{id}/sync"), &t, &hs(&h), None);
    assert_eq!(s, 200, "{v}");
    assert_eq!(v["settings"]["appearance"], json!({}), "nothing stored: both absent");
    let (s, w) = call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &t,
        &[],
        Some(json!({ "key": "appearance", "value": "dark", "scope": "all" })),
    );
    assert_eq!(s, 200, "{w}");
    assert_eq!(w["matrix"]["all"]["value"], "dark");
    let stamp = format!("{}-3-{id}", settings::wall_ms());
    let (s, p) = call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &t,
        &hs(&h),
        Some(json!({ "key": "appearance", "value": "light", "scope": "devices", "devices": [id], "stamp": stamp })),
    );
    assert_eq!(s, 200, "{p}");
    assert_eq!((p["changed"].as_bool(), p["stamp"].as_str()), (Some(true), Some(stamp.as_str())));
    assert_eq!((p["value"].as_str(), p["source"].as_str()), (Some("light"), Some("device")));
    let (_, v) = call(&ts, "GET", &format!("/api/v1/me/devices/{id}/sync"), &t, &hs(&h), None);
    assert_eq!(v["settings"]["appearance"]["own"], json!({ "value": "light", "stamp": stamp }));
    assert_eq!(v["settings"]["appearance"]["all"]["value"], "dark");
    // The same push again changes nothing; another value with that stamp is stale.
    let (s, p) = call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &t,
        &hs(&h),
        Some(json!({ "key": "appearance", "value": "light", "scope": "devices", "devices": [id], "stamp": stamp })),
    );
    assert_eq!((s, p["changed"].as_bool()), (200, Some(false)));
    let (s, e) = call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &t,
        &hs(&h),
        Some(json!({ "key": "appearance", "value": "system", "scope": "devices", "devices": [id], "stamp": stamp })),
    );
    assert_eq!((s, e["error"].as_str()), (409, Some("stale")));
    assert_eq!(e["stored"], json!({ "value": "light", "stamp": stamp }));
    let ahead = format!("{}-0-{id}", settings::wall_ms() + 10 * 60_000);
    let (s, e) = call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &t,
        &hs(&h),
        Some(json!({ "key": "text_size", "value": 130, "scope": "devices", "devices": [id], "stamp": ahead })),
    );
    assert_eq!((s, e["error"].as_str()), (409, Some("clock_skew")));
    assert!(e["server_time_ms"].as_u64().is_some());
    // The matrix shows the phone's own value and Mixed against All devices' Web.
    let (_, m) = call(&ts, "GET", "/api/v1/me/settings/matrix", &t, &[], None);
    assert_eq!(m["settings"]["appearance"]["devices"][&id]["value"], "light");
    assert_eq!(m["settings"]["appearance"]["resolved"][&id]["source"], "device");
    // Reset puts the phone back on All devices.
    let (s, r) =
        call(&ts, "DELETE", &format!("/api/v1/me/settings/appearance?scope=device&device={id}"), &t, &hs(&h), None);
    assert_eq!(s, 200, "{r}");
    assert_eq!((r["value"].as_str(), r["source"].as_str()), (Some("dark"), Some("all")));
}

// spec: S29-12
#[test]
fn a_member_renames_themselves_and_the_owner_route_stays_owner_only() {
    let ts = TestServer::start();
    let (_, kim) = ts.add_member("kim");
    let (s, m) = call(&ts, "POST", "/api/v1/me/rename", &kim, &[], Some(json!({ "name": "Kim Park" })));
    assert_eq!((s, m["name"].as_str(), m["handle"].as_str()), (200, Some("Kim Park"), Some("kim")));
    let (s, _) = call(&ts, "POST", "/api/v1/me/rename", &kim, &[], Some(json!({ "name": "  " })));
    assert_eq!(s, 400);
    let (s, w) =
        call(&ts, "PUT", "/api/v1/me/settings", &kim, &[], Some(json!({ "key": "display_name", "value": "Kim P." })));
    assert_eq!((s, w["value"].as_str()), (200, Some("Kim P.")));
    let (s, _) = call(&ts, "POST", "/api/v1/members/rowan/rename", &kim, &[], Some(json!({ "name": "nope" })));
    assert_eq!(s, 403);
}

fn feed(ts: &TestServer, token: &str, query: &str) -> Vec<Value> {
    let (s, v) = call(ts, "GET", &format!("/api/v1/events?since=0{query}"), token, &[], None);
    assert_eq!(s, 200, "{v}");
    v["events"].as_array().unwrap().clone()
}

fn types(evs: &[Value]) -> Vec<String> {
    evs.iter().map(|e| e["type"].as_str().unwrap().to_string()).collect()
}

// spec: S29-13
#[test]
fn settings_events_reach_their_own_member_only_and_only_with_me() {
    let ts = TestServer::start();
    let (_, kim) = ts.add_member("kim");
    let id = settings::new_device_id();
    let h = phone_headers(&id);
    call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &kim,
        &hs(&h),
        Some(json!({ "key": "appearance", "value": "dark", "scope": "all" })),
    );
    call(&ts, "PUT", "/api/v1/me/settings", &kim, &[], Some(json!({ "key": "times", "value": "absolute" })));
    let mine = feed(&ts, &kim, "&me=1");
    assert_eq!(types(&mine).iter().filter(|t| *t == "setting.changed").count(), 2, "{mine:?}");
    assert!(types(&mine).contains(&"device.added".to_string()));
    for e in mine.iter().filter(|e| e["type"] == "setting.changed") {
        let p = e["payload"].as_object().unwrap();
        assert!(p.keys().all(|k| ["member", "key", "scope", "device", "stamp"].contains(&k.as_str())), "{e}");
        assert!(!e.to_string().contains("absolute") && !e.to_string().contains("\"dark\""), "no values: {e}");
    }
    // Without me=1, not even Kim; the owner never, with or without it.
    assert!(!types(&feed(&ts, &kim, "")).iter().any(|t| t == "setting.changed" || t.starts_with("device.")));
    for q in ["", "&me=1"] {
        let owner = feed(&ts, &ts.owner_token, q);
        assert!(!types(&owner).iter().any(|t| t == "setting.changed" || t.starts_with("device.")), "{q}: {owner:?}");
    }
    // Beside doc=, me=1 still adds them (the Document page's feed).
    let doc = {
        let (hash, _) = ts.store.put_blob(b"# d\n").unwrap();
        let v = ts.store.vault_by_name("notes").unwrap().unwrap();
        ts.store.create_document(&ts.owner_id, &v.id, annotary_store::PushInput::new("d", hash)).unwrap().id
    };
    let both = feed(&ts, &kim, &format!("&doc={doc}&me=1"));
    assert!(types(&both).contains(&"setting.changed".to_string()), "{both:?}");
    let doc_only = feed(&ts, &kim, &format!("&doc={doc}"));
    assert!(!types(&doc_only).contains(&"setting.changed".to_string()));
}

// spec: S29-15
#[test]
fn no_member_reaches_another_members_settings_or_devices() {
    let ts = TestServer::start();
    let (_, kim) = ts.add_member("kim");
    let id = settings::new_device_id();
    let h = phone_headers(&id);
    call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &kim,
        &hs(&h),
        Some(json!({ "key": "appearance", "value": "dark", "scope": "devices", "devices": [id] })),
    );
    call(&ts, "PUT", "/api/v1/me/settings", &kim, &[], Some(json!({ "key": "show_resolved", "value": "shown" })));
    let t = ts.owner_token.clone();
    let (_, mine) = call(&ts, "GET", "/api/v1/me/settings", &t, &[], None);
    let show = mine["settings"].as_array().unwrap().iter().find(|r| r["key"] == "show_resolved").unwrap().clone();
    assert_eq!(show["value"], "hidden", "the owner sees their own value, not Kim's");
    let (_, m) = call(&ts, "GET", "/api/v1/me/settings/matrix", &t, &[], None);
    assert!(m["devices"].as_array().unwrap().iter().all(|d| d["id"] != json!(id)));
    for (method, path, body) in [
        ("GET", format!("/api/v1/me/settings?device={id}"), None),
        ("GET", format!("/api/v1/me/devices/{id}/sync"), None),
        ("PATCH", format!("/api/v1/me/devices/{id}"), Some(json!({ "name": "mine now" }))),
        ("POST", format!("/api/v1/me/devices/{id}/forget"), None),
        ("POST", "/api/v1/me/devices/web/copy-from".to_string(), Some(json!({ "from": id }))),
        ("DELETE", format!("/api/v1/me/settings/appearance?scope=device&device={id}"), None),
    ] {
        let (s, v) = call(&ts, method, &path, &t, &[], body);
        assert_eq!(s, 404, "{method} {path}: {v}");
    }
    let (s, _) = call(
        &ts,
        "PUT",
        "/api/v1/me/settings",
        &t,
        &[],
        Some(json!({ "key": "appearance", "value": "light", "scope": "devices", "devices": [id] })),
    );
    assert_eq!(s, 404);
    let kim_dev = ts.store.list_devices(&ts.store.member_by_handle("kim").unwrap().unwrap().id, false).unwrap();
    assert_eq!(kim_dev.iter().find(|d| d.id == id).unwrap().name, "iPhone 17 Pro");
}
