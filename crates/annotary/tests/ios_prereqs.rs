//! Ticket 18 — the iOS app's prerequisites, against the in-process server and the real binary:
//! the `public_url` / `link_base` settings (store, `serve --url/--link-base`, `health`,
//! `settings`), `daemon install`'s refusals, `POST /api/v1/pairing`, the `link` and `pair` verbs,
//! `show` / `push` on a TTY, titles in the list / Document API, and the page-side pieces (the
//! hidden "Open in Annotary" chip, the Members screen's "Pair a device").

use std::path::Path;
use std::process::{Command, Output, Stdio};

use annotary::testing::TestServer;
use annotary_store::{NewThread, PushInput, Store};
use serde_json::{Value, json};

const PUBLIC: &str = "http://studio.example.ts.net:7377";
const RELAY: &str = "https://cd.example.dev";

fn run_env(ts: &TestServer, home: &Path, token: &str, tty: bool, cwd: Option<&Path>, args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_annotary"));
    c.args(args)
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env("ANNOTARY_TOKEN", token)
        .env("ANNOTARY_SERVER", &ts.url)
        .env("ANNOTARY_TTY", if tty { "1" } else { "0" })
        .env("NO_COLOR", "1")
        .env_remove("ANNOTARY_PROFILE")
        .stdin(Stdio::null());
    if let Some(d) = cwd {
        c.current_dir(d);
    }
    c.output().unwrap()
}

fn run(ts: &TestServer, home: &Path, token: &str, args: &[&str]) -> Output {
    run_env(ts, home, token, false, None, args)
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

/// A Document (v1) named `name` with `md`, created through the store as the owner.
fn doc(ts: &TestServer, name: &str, md: &str) -> String {
    let (hash, _) = ts.store.put_blob(md.as_bytes()).unwrap();
    let v = ts.store.vault_by_name("notes").unwrap().unwrap();
    ts.store.create_document(&ts.owner_id, &v.id, PushInput::new(name, hash)).unwrap().id
}

fn events_of(ts: &TestServer, kind: &str) -> Vec<Value> {
    let (_, body) = ts.get("/api/v1/events?since=0&limit=1000", Some(&ts.owner_token));
    body["events"].as_array().unwrap().iter().filter(|e| e["type"] == kind).cloned().collect()
}

// ---------------------------------------------------------------- settings

// spec: S18-04
#[test]
fn public_url_and_link_base_are_settings_with_events_health_and_settings() {
    let ts = TestServer::start();
    let (_, h) = ts.get("/api/v1/health", None);
    assert!(h["public_url"].is_null() && h["link_base"].is_null(), "{h}");
    assert!(h.as_object().unwrap().contains_key("public_url"), "the field is there, null");
    // Set (normalised), unchanged = no event, cleared.
    assert!(ts.store.set_public_url(None, Some("HTTP://Studio.example.ts.net:7377/")).unwrap());
    assert!(!ts.store.set_public_url(None, Some(PUBLIC)).unwrap());
    assert!(ts.store.set_link_base(Some(&ts.owner_id), Some(RELAY)).unwrap());
    assert_eq!(ts.store.public_url().unwrap().as_deref(), Some(PUBLIC));
    let (_, h) = ts.get("/api/v1/health", None);
    assert_eq!(h["public_url"], PUBLIC);
    assert_eq!(h["link_base"], RELAY);
    let (status, s) = ts.get("/api/v1/settings", Some(&ts.owner_token));
    assert_eq!(status, 200);
    assert_eq!(s["public_url"], PUBLIC);
    assert_eq!(s["link_base"], RELAY);
    let ev = events_of(&ts, "setting.public_url");
    assert_eq!(ev.len(), 1, "one event per change: {ev:?}");
    assert_eq!(ev[0]["payload"]["url"], PUBLIC);
    assert!(ev[0]["by"].is_null());
    assert_eq!(events_of(&ts, "setting.link_base")[0]["by"], "rowan");
    // Bad values are refused (nothing changes); None clears.
    for bad in ["my-host:7377", "http://h/x", "http://h?x", "ftp://h"] {
        assert!(ts.store.set_public_url(None, Some(bad)).is_err(), "{bad}");
    }
    assert!(ts.store.set_public_url(None, None).unwrap());
    let (_, h) = ts.get("/api/v1/health", None);
    assert!(h["public_url"].is_null());
    assert_eq!(events_of(&ts, "setting.public_url").len(), 2);
}

fn serve_setup(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_annotary"))
        .arg("serve")
        .arg("--setup-only")
        .args(args)
        .env("ANNOTARY_HOME", home)
        .env("ANNOTARY_SETUP_HANDLE", "rowan")
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn stored(home: &Path) -> (Option<String>, Option<String>) {
    let s = Store::open(&home.join("store")).unwrap();
    (s.public_url().unwrap(), s.link_base().unwrap())
}

// spec: S18-04
#[test]
fn serve_url_and_link_base_flags_persist_are_kept_and_cleared_with_none() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("cd");
    let out = serve_setup(&home, &["--url", "http://studio.example.ts.net:7377/", "--link-base", RELAY]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(stored(&home), (Some(PUBLIC.into()), Some(RELAY.into())));
    // A later serve without the flags keeps them.
    assert!(serve_setup(&home, &[]).status.success());
    assert_eq!(stored(&home), (Some(PUBLIC.into()), Some(RELAY.into())));
    // A bad URL is refused (exit 1) before the store is touched.
    let out = serve_setup(&home, &["--url", "my-host:7377"]);
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--url \"my-host:7377\"") && err.contains("http://"), "{err}");
    let out = serve_setup(&home, &["--link-base", "https://cd.example.dev/path"]);
    assert_eq!(out.status.code(), Some(1));
    assert_eq!(stored(&home), (Some(PUBLIC.into()), Some(RELAY.into())));
    // `none` clears.
    assert!(serve_setup(&home, &["--link-base", "none"]).status.success());
    assert_eq!(stored(&home), (Some(PUBLIC.into()), None));
    assert!(serve_setup(&home, &["--url", "none"]).status.success());
    assert_eq!(stored(&home), (None, None));
}

// spec: S18-03
#[test]
fn daemon_install_refuses_a_remote_bind_without_allow_remote_and_bad_urls_before_writing() {
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let plist = home.join("Library/LaunchAgents/com.annotary.t18.plist");
    for (args, needle) in [
        (vec!["daemon", "install", "--bind", "0.0.0.0"], "--allow-remote"),
        (vec!["daemon", "install", "--bind", "100.64.0.7", "--allow-remote", "--url", "ftp://x"], "--url"),
        (vec!["daemon", "install", "--link-base", "https://x/y"], "--link-base"),
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_annotary"))
            .args(&args)
            .env("HOME", home)
            .env("ANNOTARY_HOME", home.join(".annotary"))
            .env("ANNOTARY_DAEMON_OS", "macos")
            .env("ANNOTARY_LAUNCHD_LABEL", "com.annotary.t18")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1), "{args:?}");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(needle), "{args:?}: {err}");
        assert!(!plist.exists(), "nothing written");
        assert!(!home.join(".annotary").exists(), "no data dir");
    }
}

// ---------------------------------------------------------------- pairing

// spec: S18-08
#[test]
fn pairing_mints_a_device_token_and_returns_the_pair_url_and_qr() {
    let ts = TestServer::start();
    let (agent_id, agent) = ts.add_member("scribe");
    let tokens = |m: &str| ts.store.list_tokens(Some(m)).unwrap().len();
    let before = tokens(&ts.owner_id);
    // No public URL: 409 with the hint, nothing minted.
    let (status, body) = ts.post("/api/v1/pairing", Some(&ts.owner_token), &json!({}));
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "no_public_url");
    assert_eq!(body["hint"], "annotary serve --url http://HOST:PORT");
    assert_eq!(tokens(&ts.owner_id), before);
    // With one: 201 and every field.
    ts.store.set_public_url(None, Some(PUBLIC)).unwrap();
    let (status, p) = ts.post("/api/v1/pairing", Some(&ts.owner_token), &json!({}));
    assert_eq!(status, 201, "{p}");
    assert_eq!(p["server"], PUBLIC);
    assert_eq!(p["member"], "rowan");
    assert_eq!(p["name"], "device");
    assert_eq!(p["profile"], "rowan@studio");
    let token = p["token"].as_str().unwrap();
    assert!(token.starts_with("ant_"));
    assert_eq!(
        p["pair_url"].as_str().unwrap(),
        format!(
            "annotary://pair?v=1&server=http%3A%2F%2Fstudio.example.ts.net%3A7377&token={}&profile=rowan%40studio",
            annotary_store::percent_encode(token)
        )
    );
    let svg = p["qr_svg"].as_str().unwrap();
    assert!(svg.starts_with("<svg") && svg.contains("#000000") && !svg.contains("href"), "{svg}");
    // The Token works at once, is listed (no expiry) and revocable.
    let (status, me) = ts.get("/api/v1/me", Some(token));
    assert_eq!(status, 200);
    assert_eq!(me["handle"], "rowan");
    let row = ts.store.token_row(p["token_id"].as_str().unwrap()).unwrap().unwrap();
    assert_eq!(row.info.name.as_deref(), Some("device"));
    assert!(row.info.expires_at.is_none());
    ts.store.revoke_token(&ts.owner_id, &row.info.id).unwrap();
    assert_eq!(ts.get("/api/v1/me", Some(token)).0, 401);
    // A Member pairs a device for themselves (own name); not for someone else.
    let (status, p) = ts.post("/api/v1/pairing", Some(&agent), &json!({ "name": "iPhone" }));
    assert_eq!(status, 201, "{p}");
    assert_eq!((p["member"].as_str(), p["name"].as_str()), (Some("scribe"), Some("iPhone")));
    assert_eq!(p["profile"], "scribe@studio");
    let (status, body) = ts.post("/api/v1/pairing", Some(&agent), &json!({ "member": "rowan" }));
    assert_eq!(status, 403, "{body}");
    assert!(body["message"].as_str().unwrap().contains("owner-only"));
    // The owner pairs one for another Member; an unknown Member is 404; no credential is 401.
    let n = tokens(&agent_id);
    let (status, p) = ts.post("/api/v1/pairing", Some(&ts.owner_token), &json!({ "member": "@scribe" }));
    assert_eq!(status, 201, "{p}");
    assert_eq!(p["member"], "scribe");
    assert_eq!(tokens(&agent_id), n + 1);
    assert_eq!(ts.post("/api/v1/pairing", Some(&ts.owner_token), &json!({ "member": "nobody" })).0, 404);
    assert_eq!(ts.post("/api/v1/pairing", None, &json!({})).0, 401);
    // An IP public URL names the profile by the IP.
    ts.store.set_public_url(None, Some("http://100.64.0.7:7377")).unwrap();
    let (_, p) = ts.post("/api/v1/pairing", Some(&ts.owner_token), &json!({}));
    assert_eq!(p["profile"], "rowan@100.64.0.7");
}

// ---------------------------------------------------------------- titles

// spec: S18-11
#[test]
fn list_rows_and_the_document_carry_the_display_title() {
    let ts = TestServer::start();
    let fm = doc(&ts, "t18-fm", "---\ntitle: Front matter wins\n---\n# Heading\n\ntext\n");
    let h1 = doc(&ts, "t18-h1", "intro\n\n# The plan\n\ntext\n");
    let none = doc(&ts, "t18-none", "no heading here\n");
    let (status, body) = ts.get("/api/v1/docs?name=t18-", Some(&ts.owner_token));
    assert_eq!(status, 200);
    let rows = body["docs"].as_array().unwrap();
    let title = |id: &str| rows.iter().find(|r| r["id"] == id).unwrap()["title"].clone();
    assert_eq!(title(&fm), "Front matter wins");
    assert_eq!(title(&h1), "The plan");
    assert!(title(&none).is_null());
    assert!(rows.iter().all(|r| r.as_object().unwrap().contains_key("title")));
    let (_, d) = ts.get(&format!("/api/v1/docs/{h1}"), Some(&ts.owner_token));
    assert_eq!(d["title"], "The plan");
    let (_, d) = ts.get(&format!("/api/v1/docs/{none}"), Some(&ts.owner_token));
    assert!(d.as_object().unwrap().contains_key("title") && d["title"].is_null());
}

// ---------------------------------------------------------------- link / show / push

// spec: S18-06, S18-07
#[test]
fn link_prints_the_public_link_or_the_cli_server_with_a_note() {
    let ts = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let owner = ts.owner_token.clone();
    let id = doc(&ts, "t18-plan", "# Plan\n\nhello\n");
    ts.store.new_thread(&ts.owner_id, &id, NewThread::document("look")).unwrap();

    // No public URL: this CLI's server + the note; TTY prints the same.
    for tty in [false, true] {
        let out = run_env(&ts, home, &owner, tty, None, &["link", "t18-plan"]);
        let (so, se) = text(&out);
        assert_eq!(out.status.code(), Some(0), "{se}");
        assert_eq!(so, format!("{}/d/{id}\n", ts.url));
        assert_eq!(
            se,
            "note: no public URL is set — other devices cannot open this; run: annotary serve --url http://HOST:PORT\n"
        );
    }
    let out = run(&ts, home, &owner, &["link", &id, "t1"]);
    assert_eq!(text(&out).0, format!("{}/d/{id}#t1\n", ts.url));
    // The skill tells agents to hand people this link (one line).
    assert!(annotary::cli::skill::skill_md().contains("`annotary link DOC [t7]`"));
    // show / push (TTY) keep the server URL without a public URL.
    let out = run_env(&ts, home, &owner, true, None, &["show", "t18-plan"]);
    assert!(text(&out).0.lines().next().unwrap().ends_with(&format!("  {}/d/{id}", ts.url)), "{}", text(&out).0);

    // Exit paths: unknown Thread 2, composite / malformed 1, unknown / deleted Document 2.
    let out = run(&ts, home, &owner, &["link", "t18-plan", "t9"]);
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(text(&out).1, format!("error: no Thread t9 in t18-plan ({id})\n"));
    for args in [vec!["link", "t18-plan/t1"], vec!["link", "t18-plan", "x7"]] {
        assert_eq!(run(&ts, home, &owner, &args).status.code(), Some(1), "{args:?}");
    }
    assert_eq!(run(&ts, home, &owner, &["link", "no-such-doc"]).status.code(), Some(2));
    let gone = doc(&ts, "t18-gone", "# Gone\n");
    ts.store.delete_document(&ts.owner_id, &gone).unwrap();
    assert_eq!(run(&ts, home, &owner, &["link", &gone]).status.code(), Some(2));

    // A public URL: the plain public link, no note; then through the relay.
    ts.store.set_public_url(None, Some(PUBLIC)).unwrap();
    let out = run(&ts, home, &owner, &["link", "t18-plan", "t1"]);
    assert_eq!(text(&out), (format!("{PUBLIC}/d/{id}#t1\n"), String::new()));
    ts.store.set_link_base(None, Some(RELAY)).unwrap();
    let relay = format!("{RELAY}/studio.example.ts.net:7377/d/{id}");
    let out = run_env(&ts, home, &owner, true, None, &["link", "t18-plan"]);
    assert_eq!(text(&out).0, format!("{relay}\n"));
    // --json: default and all fields.
    let out = run(&ts, home, &owner, &["link", "t18-plan", "t1", "--json"]);
    let v: Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    assert_eq!(v, json!({ "url": format!("{relay}#t1"), "name": "t18-plan", "id": id }));
    let out = run(&ts, home, &owner, &["link", "t18-plan", "t1", "--json", "url,thread,public"]);
    let v: Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    assert_eq!(v, json!({ "url": format!("{relay}#t1"), "thread": "t1", "public": true }));

    // show on a TTY appends the public link; non-TTY output is unchanged.
    let out = run_env(&ts, home, &owner, true, None, &["show", "t18-plan"]);
    assert!(text(&out).0.lines().next().unwrap().ends_with(&format!("  {relay}")), "{}", text(&out).0);
    let out = run(&ts, home, &owner, &["show", "t18-plan"]);
    assert!(!text(&out).0.contains("http"), "{}", text(&out).0);
    let out = run(&ts, home, &owner, &["show", "t18-plan", "--json", "url"]);
    assert_eq!(text(&out).0.trim(), format!("{{\"url\":\"{relay}\"}}"));

    // push on a TTY too.
    let work = tempfile::tempdir().unwrap();
    std::fs::write(work.path().join("t18-new.md"), "# New\n\nbody\n").unwrap();
    let out = run_env(&ts, home, &owner, true, Some(work.path()), &["push", "t18-new.md"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert!(so.contains(&format!("{RELAY}/studio.example.ts.net:7377/d/")), "{so}");
    let out = run_env(&ts, home, &owner, false, Some(work.path()), &["push", "t18-new.md"]);
    assert!(!text(&out).0.contains("http"), "{}", text(&out).0);
}

// ---------------------------------------------------------------- pair

// spec: S18-09
#[test]
fn pair_prints_a_qr_on_a_tty_one_line_otherwise_and_maps_the_errors() {
    let ts = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");

    let out = run(&ts, home, &owner, &["pair"]);
    assert_eq!(out.status.code(), Some(5));
    assert_eq!(
        text(&out).1,
        "error: no public URL is set — a phone cannot reach this server's loopback address — annotary serve --url http://HOST:PORT\n"
    );
    ts.store.set_public_url(None, Some(PUBLIC)).unwrap();

    // Non-TTY: PAIR_URL  MEMBER  TOKEN_ID  NAME.
    let out = run(&ts, home, &owner, &["pair", "--name", "phone"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    let f: Vec<&str> = so.trim_end().split('\t').collect();
    assert_eq!(f.len(), 4, "{so}");
    assert!(f[0].starts_with("annotary://pair?v=1&server=http%3A%2F%2Fstudio.example.ts.net%3A7377&token=ant_"));
    assert!(f[0].ends_with("&profile=rowan%40studio"));
    assert_eq!((f[1], f[3]), ("rowan", "phone"));
    assert!(ts.store.token_row(f[2]).unwrap().is_some());
    assert!(se.is_empty(), "{se}");

    // TTY: header, QR, profile, pair URL; the shown-once note on stderr.
    let out = run_env(&ts, home, &owner, true, None, &["pair"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    let lines: Vec<&str> = so.lines().collect();
    assert_eq!(lines[0], format!("Pairing a device for @rowan on {PUBLIC}"));
    assert_eq!(lines[1], "");
    assert!(lines[2].chars().all(|c| c == '\u{2588}') && lines[2].len() > 20, "{}", lines[2]);
    let n = lines.len();
    assert_eq!(lines[n - 2], "profile rowan@studio");
    assert!(lines[n - 1].starts_with("annotary://pair?v=1&"));
    assert_eq!(lines[n - 3], "");
    // Quadrant blocks (2×2 modules a character), square.
    assert!(lines[3..n - 3].iter().all(|l| l.chars().all(|c| " ▘▝▀▖▌▞▛▗▚▐▜▄▙▟█".contains(c))));
    assert!(lines[2..n - 3].iter().all(|l| l.chars().count() == n - 5), "{so}");
    let id = ts.store.list_tokens(Some(&ts.owner_id)).unwrap().into_iter().map(|t| t.info.id).max().unwrap();
    assert_eq!(se, format!("Shown once. Revoke: annotary token revoke {id}\n"));

    // --json: the default set and every field.
    let out = run(&ts, home, &owner, &["pair", "--json"]);
    let v: Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    let keys: Vec<&String> = v.as_object().unwrap().keys().collect();
    assert_eq!(keys, ["pair_url", "member", "token_id", "name"]);
    let out = run(&ts, home, &owner, &["pair", "--json", "server,profile,token,name"]);
    let v: Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    assert_eq!(v["server"], PUBLIC);
    assert_eq!(v["profile"], "rowan@studio");
    assert_eq!(v["name"], "device");
    assert!(v["token"].as_str().unwrap().starts_with("ant_"));

    // Another Member: owner yes; a Member 4; unknown 2.
    let out = run(&ts, home, &owner, &["pair", "--member", "@scribe"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(text(&out).0.contains("\tscribe\t"));
    assert_eq!(run(&ts, home, &agent, &["pair", "--member", "rowan"]).status.code(), Some(4));
    assert_eq!(run(&ts, home, &owner, &["pair", "--member", "nobody"]).status.code(), Some(2));
    assert_eq!(run(&ts, home, &agent, &["pair"]).status.code(), Some(0));
}

// ---------------------------------------------------------------- pages

fn page(ts: &TestServer, path: &str, host: Option<&str>) -> (u16, String) {
    let mut req = ureq::get(format!("{}{}", ts.url, path))
        .header("Accept", "text/html")
        .header("Authorization", format!("Bearer {}", ts.owner_token));
    if let Some(h) = host {
        req = req.header("Host", h);
    }
    let mut r = req.config().http_status_as_error(false).build().call().unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap())
}

// spec: S18-12
#[test]
fn the_document_page_carries_a_hidden_open_in_annotary_link() {
    let ts = TestServer::start();
    let id = doc(&ts, "t18-page", "# Page\n\nhello\n");
    let (status, html) = page(&ts, &format!("/d/{id}"), None);
    assert_eq!(status, 200);
    let origin = ts.url.trim_start_matches("http://");
    let expected = format!(
        "<a class=\"cd-chip cd-openapp\" id=\"cd-openapp\" href=\"annotary://open?server=http%3A%2F%2F{}&amp;doc={id}\" hidden>Open in Annotary</a>",
        origin.replace(':', "%3A")
    );
    assert!(html.contains(&expected), "no open-app link in the header");
    assert!(html.contains(".cd-openapp[hidden]{display:none}"), "hidden stays hidden despite .cd-chip");
    // The request's own origin (a phone reaching the server by its Tailscale name) …
    let (_, html) = page(&ts, &format!("/d/{id}/v/1"), Some("studio.example.ts.net:7377"));
    assert!(html.contains("server=http%3A%2F%2Fstudio.example.ts.net%3A7377&amp;doc="), "origin from Host");
    // … and the public URL wins when one is set.
    ts.store.set_public_url(None, Some("http://100.64.0.7:7377")).unwrap();
    let (_, html) = page(&ts, &format!("/d/{id}"), Some("studio.example.ts.net:7377"));
    assert!(html.contains("server=http%3A%2F%2F100.64.0.7%3A7377&amp;doc="));
    // The script knows the rule (shown on iPhone / iPad only).
    assert!(html.contains("/iPhone|iPad/") && html.contains("maxTouchPoints"));
}

// spec: S18-10
#[test]
fn the_members_screen_offers_pair_a_device_on_active_members() {
    let ts = TestServer::start();
    let (id, _) = ts.add_member("gone");
    ts.store.disable_member(&ts.owner_id, &id).unwrap();
    let (status, html) = page(&ts, "/members", None);
    assert_eq!(status, 200);
    let card = |h: &str| {
        let start = html.find(&format!("data-handle=\"{h}\"")).unwrap();
        let end = html[start..].find("</article>").unwrap() + start;
        html[start..end].to_string()
    };
    assert!(card("rowan").contains("data-act=\"pair\">Pair a device</button>"));
    assert!(card("rowan").contains("class=\"cd-paironce-slot\""));
    assert!(!card("gone").contains("data-act=\"pair\""));
    assert!(html.contains("/api/v1/pairing") && html.contains("Scan with the iPhone or iPad camera, or Annotary."));
}
