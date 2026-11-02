//! CMD-23 with the real binary against the in-process server: `settings list | get | set | reset`
//! and `devices list | rename | forget`, their exit paths, and that the CLI never makes a Device
//! (it sends no `X-Annotary-Device`). iPhones and iPads are simulated as API clients that send
//! their Device id, as the iOS app does. The trycmd goldens are `tests/e2e/87-settings/`; the HTTP
//! side is `server/me_settings.rs` and the store's tests.

use std::process::{Command, Output};

use annotary::config::{Config, Profile};
use annotary::testing::{TestServer, probe_root};
use serde_json::{Value, json};

const PHONE: &str = "01K9PH0NE00000000000000001";
const PAD: &str = "01K9PAD0000000000000000002";

struct Home {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
}

impl Home {
    fn new(profiles: &[(&str, &str, &str)]) -> Home {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        let mut c = Config::default();
        for (name, server, token) in profiles {
            c.profiles.insert(
                name.to_string(),
                Profile { server: Some(server.to_string()), token: Some(token.to_string()), ..Profile::default() },
            );
        }
        c.save(&root.join(".annotary/config.toml")).unwrap();
        Home { _tmp: tmp, root }
    }
}

fn run_env(home: &Home, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_annotary"));
    c.args(args)
        .current_dir(&home.root)
        .env("HOME", &home.root)
        .env("ANNOTARY_HOME", home.root.join(".annotary"))
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_TTY");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().unwrap()
}

fn run(home: &Home, args: &[&str]) -> Output {
    run_env(home, &[], args)
}

fn tty(home: &Home, args: &[&str]) -> Output {
    run_env(home, &[("ANNOTARY_TTY", "1"), ("NO_COLOR", "1")], args)
}

#[track_caller]
fn ok(o: Output) -> (String, String) {
    let so = String::from_utf8_lossy(&o.stdout).to_string();
    let se = String::from_utf8_lossy(&o.stderr).to_string();
    assert_eq!(o.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    (so, se)
}

#[track_caller]
fn exits(o: Output, code: i32) -> String {
    let so = String::from_utf8_lossy(&o.stdout).to_string();
    let se = String::from_utf8_lossy(&o.stderr).to_string();
    assert_eq!(o.status.code(), Some(code), "stdout: {so}\nstderr: {se}");
    assert!(so.is_empty(), "{so}");
    se
}

/// A request from an iOS Device: `X-Annotary-Device` (+ the info header naming it).
fn from_device(ts: &TestServer, token: &str, device: &str, info: &str, method: &str, path: &str, body: Option<Value>) {
    let url = format!("{}{}", ts.url, path);
    let auth = format!("Bearer {token}");
    let res = match method {
        "PUT" => ureq::put(&url)
            .header("Authorization", &auth)
            .header("X-Annotary-Device", device)
            .header("X-Annotary-Device-Info", info)
            .send_json(body.unwrap()),
        _ => ureq::get(&url)
            .header("Authorization", &auth)
            .header("X-Annotary-Device", device)
            .header("X-Annotary-Device-Info", info)
            .call(),
    };
    let status = res.map(|r| r.status().as_u16()).unwrap_or_else(|e| panic!("{method} {path}: {e}"));
    assert_eq!(status, 200);
}

fn field(line: &str, i: usize) -> &str {
    line.split('\t').nth(i).unwrap_or("")
}

fn line_of<'a>(out: &'a str, key: &str) -> &'a str {
    out.lines().find(|l| l.split(['\t', ' ']).next() == Some(key)).unwrap_or_else(|| panic!("no {key} in {out}"))
}

// spec: S29-30
#[test]
fn settings_verbs_read_and_write_with_their_scopes_and_exit_codes() {
    // Locked: a loopback browser sign-in would make a Web Device (CMD-24).
    let ts = TestServer::start_locked();
    let home = Home::new(&[("rowan", &ts.url, &ts.owner_token), ("down", "http://127.0.0.1:1", "ant_x")]);
    // An iPhone of @rowan's (its Token is a second one, so the CLI's never names a Device).
    let phone_tok = ts.store.mint_token(&ts.owner_id, &ts.owner_id, Some("iphone")).unwrap().plain;
    from_device(&ts, &phone_tok, PHONE, "kind=iphone; name=iPhone 17", "GET", "/api/v1/me/settings", None);

    // list: every catalog key, defaults; non-TTY tab-separated without a header; TTY aligned.
    let (so, _) = ok(run(&home, &["-p", "rowan", "settings", "list"]));
    let n = annotary_store::settings::CATALOG.len();
    assert_eq!(so.lines().count(), n, "{so}");
    assert_eq!(line_of(&so, "appearance"), "appearance\tsystem\tdefault\tdevice");
    assert_eq!(line_of(&so, "text_size"), "text_size\t100\tdefault\tdevice");
    assert_eq!(line_of(&so, "default_vault"), "default_vault\tany\tdefault\tuser");
    assert_eq!(field(line_of(&so, "display_name"), 2), "user");
    let (so, _) = ok(tty(&home, &["-p", "rowan", "settings"]));
    let lines: Vec<&str> = so.lines().collect();
    assert_eq!(lines[0].split_whitespace().collect::<Vec<_>>(), ["KEY", "VALUE", "SOURCE", "KIND"]);
    assert_eq!(lines.len(), n + 1);
    assert!(line_of(&so, "appearance").split_whitespace().eq(["appearance", "system", "default", "device"]));

    // set: a device setting needs exactly one flag; a user setting takes none.
    let se = exits(run(&home, &["-p", "rowan", "settings", "set", "appearance", "dark"]), 1);
    assert!(se.starts_with("error: appearance is a device setting: say --all-devices or --device NAME"), "{se}");
    let se = exits(run(&home, &["-p", "rowan", "settings", "set", "times", "absolute", "--all-devices"]), 1);
    assert!(se.starts_with("error: times is a user setting, the same on every Device: drop --all-devices"), "{se}");
    exits(run(&home, &["-p", "rowan", "settings", "set", "appearance", "dark", "--all-devices", "--device", "x"]), 1);
    let se = exits(run(&home, &["-p", "rowan", "settings", "set", "nope", "1"]), 1);
    assert!(se.starts_with("error: no setting \"nope\""), "{se}");
    let se = exits(run(&home, &["-p", "rowan", "settings", "set", "appearance", "purple", "--all-devices"]), 1);
    assert!(se.contains("\"purple\" is not one of system, light, dark"), "{se}");
    let se = exits(run(&home, &["-p", "rowan", "settings", "set", "highlight_light", "#333333", "--all-devices"]), 1);
    assert!(se.contains("contrast"), "{se}");
    let se = exits(run(&home, &["-p", "rowan", "settings", "set", "text_size", "120%", "--all-devices"]), 1);
    assert!(se.contains("120 is not one of"), "{se}");
    let se = exits(run(&home, &["-p", "rowan", "settings", "set", "appearance", "dark", "--device", "toaster"]), 2);
    assert!(se.starts_with("error: no Device toaster"), "{se}");

    // All devices: the record afterwards; the iPhone follows it.
    let (so, _) = ok(run(&home, &["-p", "rowan", "settings", "set", "text_size", "130%", "--all-devices"]));
    assert_eq!(so, "text_size\t130\tall\tdevice\n");
    let (so, se) = ok(run(&home, &["-p", "rowan", "settings", "set", "text_size", "130", "--all-devices"]));
    assert_eq!((so.as_str(), se.as_str()), ("text_size\t130\tall\tdevice\n", "text_size unchanged\n"));
    // One Device's own value, by name (case-insensitive) or id.
    let (so, _) = ok(run(&home, &["-p", "rowan", "settings", "set", "appearance", "dark", "--device", "iphone 17"]));
    assert_eq!(so, "appearance\tdark\tdevice\tdevice\n");
    let (so, _) = ok(run(&home, &["-p", "rowan", "settings", "get", "appearance"]));
    assert_eq!(so, "appearance\tsystem\tdefault\tdevice\n", "without --device: All devices");
    let (so, _) = ok(run(&home, &["-p", "rowan", "settings", "get", "appearance", "--device", PHONE, "--json"]));
    let v: Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!(v, json!({"key": "appearance", "value": "dark", "source": "device", "kind": "device"}));
    let (so, _) =
        ok(run(&home, &["-p", "rowan", "settings", "list", "--device", "iPhone 17", "--json", "key,value,device"]));
    let v: Value = serde_json::from_str(so.trim()).unwrap();
    let app = v.as_array().unwrap().iter().find(|r| r["key"] == "appearance").unwrap();
    assert_eq!(app, &json!({"key": "appearance", "value": "dark", "device": PHONE}));
    let ts_row = v.as_array().unwrap().iter().find(|r| r["key"] == "text_size").unwrap();
    assert_eq!(ts_row["value"], 130);
    let (so, _) = ok(tty(&home, &["-p", "rowan", "settings", "get", "appearance", "--device", "iPhone 17"]));
    assert_eq!(so, "appearance  dark  device  device\n");
    exits(run(&home, &["-p", "rowan", "settings", "get", "nope"]), 1);

    // User settings: Default Vault by name, shown by name; an unknown Vault is exit 2.
    let (so, _) = ok(run(&home, &["-p", "rowan", "settings", "set", "default_vault", "notes"]));
    assert_eq!(so, "default_vault\tnotes\tuser\tuser\n");
    let se = exits(run(&home, &["-p", "rowan", "settings", "set", "default_vault", "nowhere"]), 2);
    assert!(se.starts_with("error: no Vault nowhere"), "{se}");
    let (so, _) = ok(run(&home, &["-p", "rowan", "settings", "set", "display_name", "Rowan A.", "--json"]));
    let v: Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!((v["value"].as_str(), v["changed"].as_bool()), (Some("Rowan A."), Some(true)));
    assert_eq!(ts.store.member_by_handle("rowan").unwrap().unwrap().display_name, "Rowan A.");

    // reset: same flag rules; a Device's own value goes back to following All devices.
    exits(run(&home, &["-p", "rowan", "settings", "reset", "appearance"]), 1);
    let (so, _) = ok(run(&home, &["-p", "rowan", "settings", "reset", "appearance", "--device", "iPhone 17"]));
    assert_eq!(so, "appearance\tsystem\tdefault\tdevice\n");
    let (so, _) = ok(run(&home, &["-p", "rowan", "settings", "reset", "default_vault"]));
    assert_eq!(so, "default_vault\tany\tdefault\tuser\n");
    let (_, se) = ok(run(&home, &["-p", "rowan", "settings", "reset", "default_vault"]));
    assert_eq!(se, "default_vault unchanged\n");

    // --json ? needs no server; an unreachable server is exit 6.
    let (so, _) = ok(run(&home, &["-p", "down", "settings", "list", "--json", "?"]));
    assert_eq!(so, "key value source kind label group device\n");
    let (so, _) = ok(run(&home, &["-p", "down", "settings", "set", "k", "v", "--json", "?"]));
    assert_eq!(so, "key value source kind label group device changed\n");
    exits(run(&home, &["-p", "down", "settings", "list"]), 6);

    // A port watcher probing the server (CMD-24) makes no Device either.
    probe_root(&ts.url);
    // The CLI made no Device: only the iPhone exists.
    let devices = ts.store.list_devices(&ts.owner_id, true).unwrap();
    assert_eq!(devices.iter().map(|d| d.id.as_str()).collect::<Vec<_>>(), [PHONE]);
}

// spec: S29-31
#[test]
fn devices_verbs_list_rename_and_forget_and_refuse_web() {
    let ts = TestServer::start();
    let home = Home::new(&[("rowan", &ts.url, &ts.owner_token)]);

    // No Devices yet: nothing on stdout, a note on a TTY.
    let (so, _) = ok(run(&home, &["-p", "rowan", "devices"]));
    assert_eq!(so, "");
    let (so, se) = ok(tty(&home, &["-p", "rowan", "devices", "list"]));
    assert_eq!(so, "");
    assert!(se.starts_with("no Devices yet"), "{se}");

    // Web (made by a browser sign-in; here by naming it), an iPhone and an iPad, each with its
    // own Token used from it twice (the second request stamps the Token's Device).
    from_device(&ts, &ts.owner_token, "web", "", "GET", "/api/v1/me/settings?device=web", None);
    let phone_tok = ts.store.mint_token(&ts.owner_id, &ts.owner_id, Some("iphone")).unwrap().plain;
    let pad_tok = ts.store.mint_token(&ts.owner_id, &ts.owner_id, Some("ipad")).unwrap().plain;
    for _ in 0..2 {
        from_device(&ts, &phone_tok, PHONE, "kind=iphone; name=iPhone 17", "GET", "/api/v1/me/settings", None);
        from_device(&ts, &pad_tok, PAD, "kind=ipad; name=iPad Pro", "GET", "/api/v1/me/settings", None);
    }

    let (so, _) = ok(run(&home, &["-p", "rowan", "devices", "list"]));
    let lines: Vec<&str> = so.lines().collect();
    assert_eq!(lines.len(), 3, "{so}");
    assert!(lines[0].starts_with("web\tweb\tWeb\t"), "{so}");
    assert!(lines[1].starts_with(&format!("{PHONE}\tiphone\tiPhone 17\t")), "{so}");
    assert!(lines[2].starts_with(&format!("{PAD}\tipad\tiPad Pro\t")), "{so}");
    assert!(lines[1].ends_with('s'), "LAST SEEN is relative: {so}");
    let (so, _) = ok(tty(&home, &["-p", "rowan", "devices", "list"]));
    assert_eq!(
        so.lines().next().unwrap().split_whitespace().collect::<Vec<_>>(),
        ["ID", "KIND", "NAME", "LAST", "SEEN"]
    );
    let (so, _) = ok(run(&home, &["-p", "rowan", "devices", "list", "--json", "id,name"]));
    assert_eq!(
        serde_json::from_str::<Value>(so.trim()).unwrap(),
        json!([{"id": "web", "name": "Web"}, {"id": PHONE, "name": "iPhone 17"}, {"id": PAD, "name": "iPad Pro"}])
    );

    // rename by name (case-insensitive): the row; Web refused (5); unknown (2); empty (1).
    let (so, _) = ok(run(&home, &["-p", "rowan", "devices", "rename", "ipad pro", "Rowan's iPad"]));
    assert!(so.starts_with(&format!("{PAD}\tipad\tRowan's iPad\t")), "{so}");
    let se = exits(run(&home, &["-p", "rowan", "devices", "rename", "Web", "Browser"]), 5);
    assert!(se.starts_with("error: Web stands for every browser"), "{se}");
    exits(run(&home, &["-p", "rowan", "devices", "rename", "toaster", "x"]), 2);
    exits(run(&home, &["-p", "rowan", "devices", "rename", PAD, " "]), 1);
    // Two Devices with one name: ambiguous (2), the id still works.
    ok(run(&home, &["-p", "rowan", "devices", "rename", PHONE, "Rowan's iPad"]));
    let se = exits(run(&home, &["-p", "rowan", "devices", "rename", "rowan's ipad", "x"]), 2);
    assert!(se.starts_with("error: 2 Devices named \"rowan's ipad\" — use the id:"), "{se}");
    assert!(se.contains(PHONE) && se.contains(PAD), "{se}");
    ok(run(&home, &["-p", "rowan", "devices", "rename", PHONE, "iPhone 17"]));

    // forget: revokes the iPad's Token; Web refused (5); forgotten only with --all.
    let se = exits(run(&home, &["-p", "rowan", "devices", "forget", "web"]), 5);
    assert!(se.starts_with("error: Web stands for every browser"), "{se}");
    let (so, _) = ok(run(&home, &["-p", "rowan", "devices", "forget", "Rowan's iPad"]));
    assert_eq!(so, format!("{PAD}\tRowan's iPad\tforgotten\t1 Token revoked\t0 sessions ended\n"));
    let (st, _) = ts.get("/api/v1/me/devices", Some(&pad_tok));
    assert_eq!(st, 401, "the iPad's Token is revoked");
    let (so, se) = ok(run(&home, &["-p", "rowan", "devices", "forget", PAD, "--json"]));
    assert_eq!(se, format!("Device {PAD} was already forgotten\n"));
    let v: Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!((v["forgotten"].as_bool(), v["already"].as_bool()), (Some(true), Some(true)));
    let (so, _) = ok(run(&home, &["-p", "rowan", "devices", "list"]));
    assert_eq!(so.lines().count(), 2);
    let (so, _) = ok(run(&home, &["-p", "rowan", "devices", "list", "--all"]));
    assert!(so.contains("\tRowan's iPad (forgotten)\t"), "{so}");
    let (so, _) = ok(run(&home, &["-p", "rowan", "devices", "list", "--all", "--json", "id,forgotten"]));
    assert!(so.contains(&format!("{{\"id\":\"{PAD}\",\"forgotten\":true}}")), "{so}");
    // A forgotten Device is still readable, never writable (5).
    ok(run(&home, &["-p", "rowan", "settings", "get", "appearance", "--device", PAD]));
    let se = exits(run(&home, &["-p", "rowan", "settings", "set", "appearance", "dark", "--device", PAD]), 5);
    assert!(se.contains("was forgotten"), "{se}");
    let (so, _) = ok(run(&home, &["-p", "rowan", "devices", "forget", "x", "--json", "?"]));
    assert_eq!(so, "id kind name forgotten tokens_revoked sessions_ended already\n");
}
