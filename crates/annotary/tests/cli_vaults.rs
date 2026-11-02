//! CMD-5 with the real binary against the in-process server: the `vault` verbs and their exit
//! paths, the local profiles `vault rename` / `delete` update, and profiles that heal themselves
//! after a Vault was renamed or deleted elsewhere (the web Settings screen, iOS, another machine)
//! on the very first call — chained renames, a reused old name, a legacy profile without a
//! `vault_id`, a hand-edited `vault`, a non-ASCII name — plus a Sidecar git left conflicted
//! across a rename. The trycmd goldens are `tests/e2e/86-vaults/`; the HTTP side is
//! `tests/vaults_api.rs`.

use std::path::Path;
use std::process::{Command, Output};

use annotary::config::{Config, Profile};
use annotary::testing::TestServer;
use annotary_store::PushInput;
use serde_json::{Value, json};

struct Home {
    _tmp: tempfile::TempDir,
    root: std::path::PathBuf,
}

impl Home {
    fn new() -> Home {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().to_path_buf();
        Home { _tmp: tmp, root }
    }
    fn config_path(&self) -> std::path::PathBuf {
        self.root.join(".annotary/config.toml")
    }
    fn config(&self) -> Config {
        Config::load(&self.config_path()).unwrap()
    }
    fn profile(&self, name: &str) -> Profile {
        self.config().profiles.get(name).cloned().unwrap_or_else(|| panic!("no profile {name}"))
    }
    fn set(&self, name: &str, p: Profile) {
        let mut c = self.config();
        c.profiles.insert(name.into(), p);
        c.save(&self.config_path()).unwrap();
    }
}

fn prof(server: &str, token: &str, vault: Option<&str>, vault_id: Option<&str>) -> Profile {
    Profile {
        server: Some(server.into()),
        token: Some(token.into()),
        vault: vault.map(str::to_string),
        vault_id: vault_id.map(str::to_string),
    }
}

fn run_in(home: &Home, cwd: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_annotary"));
    c.args(args)
        .current_dir(cwd)
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
    run_in(home, &home.root, &[], args)
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

#[track_caller]
fn ok(o: Output) -> (String, String) {
    let (so, se) = text(&o);
    assert_eq!(o.status.code(), Some(0), "stdout: {so}\nstderr: {se}");
    (so, se)
}

#[track_caller]
fn exits(o: Output, code: i32) -> String {
    let (so, se) = text(&o);
    assert_eq!(o.status.code(), Some(code), "stdout: {so}\nstderr: {se}");
    assert!(so.is_empty(), "{so}");
    se
}

fn doc_in(ts: &TestServer, vault: &str, name: &str) -> String {
    let v = ts.store.vault_by_name(vault).unwrap().unwrap();
    let (hash, _) = ts.store.put_blob(format!("# {name}\n").as_bytes()).unwrap();
    ts.store.create_document(&ts.owner_id, &v.id, PushInput::new(name, &hash)).unwrap().id
}

fn vault_id(ts: &TestServer, name: &str) -> String {
    ts.store.vault_by_name(name).unwrap().unwrap().id
}

/// Rename a Vault the way the web Settings screen or the iOS app does (HTTP, no CLI).
fn rename_elsewhere(ts: &TestServer, from: &str, to: &str) {
    let (status, body) = ts.post(
        &format!("/api/v1/vaults/{}/rename", annotary::client::qs(from)),
        Some(&ts.owner_token),
        &json!({ "name": to }),
    );
    assert_eq!(status, 200, "{body}");
}

// spec: S26-01, S26-02, S26-03
#[test]
fn vault_verbs_print_rows_refuse_with_their_exit_codes_and_update_local_profiles() {
    let ts = TestServer::start();
    let (_, member) = ts.add_member("agent");
    let home = Home::new();
    home.set("rowan", prof(&ts.url, &ts.owner_token, None, None));
    home.set("agent", prof(&ts.url, &member, Some("notes"), None));
    let in_notes = doc_in(&ts, "notes", "a");

    // list: non-TTY rows, no header; TTY aligned with a header; --json; --json ? offline.
    let (so, _) = ok(run(&home, &["-p", "rowan", "vault", "list"]));
    assert!(so.starts_with("notes\t1\t0\t") && so.lines().count() == 1, "{so}");
    let (so, _) =
        ok(run_in(&home, &home.root, &[("ANNOTARY_TTY", "1"), ("NO_COLOR", "1")], &["-p", "rowan", "vault", "list"]));
    let lines: Vec<&str> = so.lines().collect();
    assert_eq!(lines[0].split_whitespace().collect::<Vec<_>>(), ["NAME", "DOCUMENTS", "DELETED", "CREATED"], "{so}");
    assert!(lines[1].starts_with("notes  "), "{so}");
    let (so, _) = ok(run(&home, &["-p", "agent", "vault", "list", "--json"]));
    let v: Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!((v[0]["name"].clone(), v[0]["documents"].clone()), (json!("notes"), json!(1)));
    assert_eq!(v[0]["deleted"], 0);
    assert!(v[0]["created"].as_str().unwrap().ends_with('Z') && v[0]["id"].is_string());
    let (so, _) = ok(run(&home, &["-p", "rowan", "vault", "list", "--json", "?"]));
    assert_eq!(so, "name documents deleted created id default_actions\n");

    // create: the row; owner only (4); taken (5); empty (1).
    let (so, _) = ok(run(&home, &["-p", "rowan", "vault", "create", "drafts"]));
    assert!(so.starts_with("drafts\t0\t0\t") && so.ends_with("s\n"), "{so}");
    let se = exits(run(&home, &["-p", "rowan", "vault", "create", "drafts"]), 5);
    assert_eq!(se, "error: Vault drafts already exists\n");
    let se = exits(run(&home, &["-p", "agent", "vault", "create", "mine"]), 4);
    assert!(se.starts_with("error: vault create is owner-only"), "{se}");
    exits(run(&home, &["-p", "rowan", "vault", "create", " "]), 1);
    let drafts = vault_id(&ts, "drafts");

    // Profiles naming `drafts`: by name (no id yet), by id (a stale name), another server, another Vault.
    home.set("byname", prof(&ts.url, &ts.owner_token, Some("drafts"), None));
    home.set("byid", prof(&ts.url, &ts.owner_token, Some("drafts-old"), Some(&drafts)));
    home.set("elsewhere", prof("http://127.0.0.1:1", "ant_x", Some("drafts"), None));
    home.set("notes", prof(&ts.url, &ts.owner_token, Some("notes"), None));

    // rename: the row; profiles follow; --json carries from + profiles.
    // Even with ANNOTARY_SERVER exported, another server's name-only profile stays untouched.
    let (so, se) = ok(run_in(
        &home,
        &home.root,
        &[("ANNOTARY_SERVER", &ts.url)],
        &["-p", "rowan", "vault", "rename", "drafts", "reviews"],
    ));
    assert!(so.starts_with("reviews\t0\t0\t"), "{so}");
    assert_eq!(
        se, "profile \"byid\" now uses Vault \"reviews\"\nprofile \"byname\" now uses Vault \"reviews\"\n",
        "{se}"
    );
    for p in ["byname", "byid"] {
        let p = home.profile(p);
        assert_eq!((p.vault.as_deref(), p.vault_id.as_deref()), (Some("reviews"), Some(drafts.as_str())));
    }
    assert_eq!(home.profile("elsewhere").vault.as_deref(), Some("drafts"), "another server");
    assert_eq!(home.profile("notes").vault.as_deref(), Some("notes"));
    let (so, _) = ok(run(&home, &["-p", "rowan", "vault", "rename", "reviews", "Übersicht", "--json"]));
    let v: Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!((v["name"].as_str(), v["from"].as_str()), (Some("Übersicht"), Some("reviews")));
    assert_eq!(v["profiles"], json!(["byid", "byname"]));
    assert_eq!(home.profile("byname").vault.as_deref(), Some("Übersicht"));
    let (_, se) = ok(run(&home, &["-p", "rowan", "vault", "rename", "Übersicht", "Übersicht"]));
    assert!(se.contains("already has that name"), "{se}");
    exits(run(&home, &["-p", "rowan", "vault", "rename", "nowhere", "x"]), 2);
    let se = exits(run(&home, &["-p", "rowan", "vault", "rename", "Übersicht", "notes"]), 5);
    assert_eq!(se, "error: Vault notes already exists\n");
    exits(run(&home, &["-p", "agent", "vault", "rename", "Übersicht", "x"]), 4);
    exits(run(&home, &["-p", "rowan", "vault", "rename", "Übersicht", " "]), 1);
    let (so, _) = ok(run(&home, &["-p", "rowan", "vault", "rename", "x", "y", "--json", "?"]));
    assert_eq!(so, "name from documents deleted created id default_actions profiles\n");

    // delete: refused while it holds a Document (deleted ones until purged), then deleted, the
    // profiles cleared; the last Vault refused; unknown 2; owner only 4.
    let doc = doc_in(&ts, "Übersicht", "d");
    let se = exits(run(&home, &["-p", "rowan", "vault", "delete", "Übersicht"]), 5);
    assert!(se.starts_with("error: Vault Übersicht still holds 1 Document — "), "{se}");
    ts.store.delete_document(&ts.owner_id, &doc).unwrap();
    let se = exits(run(&home, &["-p", "rowan", "vault", "delete", "Übersicht"]), 5);
    assert!(se.contains("(1 deleted — purge them first)"), "{se}");
    exits(run(&home, &["-p", "agent", "vault", "delete", "Übersicht"]), 4);
    ts.store.purge_document(&ts.owner_id, &doc).unwrap();
    let (so, se) = ok(run(&home, &["-p", "rowan", "vault", "delete", "Übersicht"]));
    assert_eq!(so, "Übersicht\tdeleted\n");
    assert!(se.contains("profile \"byname\" no longer names a Vault (\"Übersicht\" was deleted)"), "{se}");
    for p in ["byname", "byid"] {
        let p = home.profile(p);
        assert_eq!((p.vault, p.vault_id), (None, None));
    }
    assert_eq!(home.profile("notes").vault.as_deref(), Some("notes"));
    ts.store.delete_document(&ts.owner_id, &in_notes).unwrap();
    ts.store.purge_document(&ts.owner_id, &in_notes).unwrap();
    let se = exits(run(&home, &["-p", "rowan", "vault", "delete", "notes"]), 5);
    assert!(se.starts_with("error: notes is the last Vault"), "{se}");
    exits(run(&home, &["-p", "rowan", "vault", "delete", "Übersicht"]), 2);
    let (so, _) = ok(run(&home, &["-p", "rowan", "vault", "delete", "x", "--json", "?"]));
    assert_eq!(so, "name id deleted created profiles\n");
    ts.create_vault("tmp");
    let (so, _) = ok(run(&home, &["-p", "rowan", "vault", "delete", "tmp", "--json"]));
    let v: Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!((v["name"].as_str(), v["deleted"].as_bool()), (Some("tmp"), Some(true)));
    // No credential / unreachable as for every verb.
    exits(run(&home, &["vault", "list"]), 3);
    exits(run(&home, &["-p", "elsewhere", "vault", "list"]), 6);
}

// spec: S26-06, S26-08
#[test]
fn a_rename_made_elsewhere_heals_the_profile_on_the_very_first_call() {
    let ts = TestServer::start();
    ts.create_vault("Übersicht");
    doc_in(&ts, "Übersicht", "plan");
    doc_in(&ts, "notes", "plan");
    let id = vault_id(&ts, "Übersicht");
    let home = Home::new();
    home.set("p", prof(&ts.url, &ts.owner_token, Some("Übersicht"), None));
    let cwd = tempfile::tempdir().unwrap();

    // The first call backfills the id silently.
    let (so, se) = ok(run(&home, &["-p", "p", "list"]));
    assert_eq!(so.lines().count(), 1, "scoped to the profile's Vault: {so}");
    assert_eq!(se, "");
    assert_eq!(home.profile("p").vault_id.as_deref(), Some(id.as_str()));

    // Each verb, the first call after a rename elsewhere: works, heals, says so once.
    let note = |from: &str, to: &str| format!("note: Vault \"{from}\" is now \"{to}\" — updated profile \"p\"\n");
    rename_elsewhere(&ts, "Übersicht", "Überblick");
    let (so, se) = ok(run(&home, &["-p", "p", "list"]));
    assert!(so.contains("\tplan\t") && so.lines().count() == 1, "{so}");
    assert_eq!(se, note("Übersicht", "Überblick"));
    let p = home.profile("p");
    assert_eq!((p.vault.as_deref(), p.vault_id.as_deref()), (Some("Überblick"), Some(id.as_str())));
    let (_, se) = ok(run(&home, &["-p", "p", "list"]));
    assert_eq!(se, "", "healed: nothing more to say");

    rename_elsewhere(&ts, "Überblick", "b2");
    let (so, se) = ok(run(&home, &["-p", "p", "show", "plan"]));
    assert!(so.starts_with("plan\t"), "the Name is unique in the profile's Vault: {so}");
    assert_eq!(se, note("Überblick", "b2"));

    rename_elsewhere(&ts, "b2", "b3");
    std::fs::write(cwd.path().join("fresh.md"), "# fresh\n").unwrap();
    let (_, se) = ok(run_in(&home, cwd.path(), &[], &["-p", "p", "push", "fresh.md"]));
    assert!(se.contains(&note("b2", "b3")), "{se}");
    let (_, docs) = ts.get("/api/v1/docs?vault=b3", Some(&ts.owner_token));
    assert!(docs["docs"].as_array().unwrap().iter().any(|d| d["name"] == "fresh"), "pushed into b3: {docs}");
    let sc = std::fs::read_to_string(cwd.path().join("fresh.annotary")).unwrap();
    assert!(sc.contains("\"vault\": \"b3\""), "{sc}");

    rename_elsewhere(&ts, "b3", "b4");
    let (_, se) = ok(run(&home, &["-p", "p", "mentions"]));
    assert_eq!(se, note("b3", "b4"));

    rename_elsewhere(&ts, "b4", "b5");
    let (so, se) = ok(run(&home, &["-p", "p", "profile", "list", "--json", "name,vault"]));
    assert_eq!(so.trim(), r#"[{"name":"p","vault":"b5"}]"#);
    assert_eq!(se, note("b4", "b5"));

    // -q: healed all the same, nothing said.
    rename_elsewhere(&ts, "b5", "b6");
    let (_, se) = ok(run(&home, &["-p", "p", "-q", "list"]));
    assert_eq!(se, "");
    assert_eq!(home.profile("p").vault.as_deref(), Some("b6"));

    // ANNOTARY_TOKEN + ANNOTARY_PROFILE: the env profile scopes the Vault and is the one healed.
    rename_elsewhere(&ts, "b6", "b7");
    let env = [("ANNOTARY_TOKEN", ts.owner_token.as_str()), ("ANNOTARY_PROFILE", "p")];
    let (_, se) = ok(run_in(&home, &home.root, &env, &["list"]));
    assert_eq!(se, note("b6", "b7"));
}

// spec: S26-05, S26-06
#[test]
fn chained_renames_a_reused_name_a_legacy_profile_and_a_hand_edit() {
    let ts = TestServer::start();
    ts.create_vault("a");
    doc_in(&ts, "a", "in-a");
    let x = vault_id(&ts, "a");
    let home = Home::new();

    // (c) a -> b -> c, then a brand-new Vault takes `a`: the profile that knew the id goes to `c`.
    home.set("p", prof(&ts.url, &ts.owner_token, Some("a"), None));
    ok(run(&home, &["-p", "p", "list"]));
    rename_elsewhere(&ts, "a", "b");
    rename_elsewhere(&ts, "b", "c");
    ts.create_vault("a");
    let (so, se) = ok(run(&home, &["-p", "p", "list"]));
    assert!(so.contains("\tin-a\t"), "never the new, empty `a`: {so}");
    assert_eq!(se, "note: Vault \"a\" is now \"c\" — updated profile \"p\"\n");
    assert_eq!(home.profile("p").vault_id.as_deref(), Some(x.as_str()));

    // (d) a legacy profile (no vault_id) on `c` across c -> d.
    home.set("legacy", prof(&ts.url, &ts.owner_token, Some("c"), None));
    rename_elsewhere(&ts, "c", "d");
    let (so, _) = ok(run(&home, &["-p", "legacy", "list"]));
    assert!(so.contains("\tin-a\t"), "{so}");
    let p = home.profile("legacy");
    assert_eq!((p.vault.as_deref(), p.vault_id.as_deref()), (Some("d"), Some(x.as_str())));

    // …but a legacy profile on the reused name means the Vault named so today.
    home.set("plain", prof(&ts.url, &ts.owner_token, Some("a"), None));
    let (so, se) = ok(run(&home, &["-p", "plain", "list"]));
    assert_eq!((so.as_str(), se.as_str()), ("", ""));
    assert_eq!(home.profile("plain").vault_id, Some(vault_id(&ts, "a")));

    // (e) a hand edit to `vault = "other"` with the stale id left in the file: the edit wins.
    ts.create_vault("other");
    doc_in(&ts, "other", "in-other");
    let mut p = home.profile("p");
    p.vault = Some("other".into());
    home.set("p", p);
    let (so, se) = ok(run(&home, &["-p", "p", "list"]));
    assert!(so.contains("\tin-other\t") && !so.contains("in-a"), "{so}");
    assert_eq!(se, "", "the name did not change: nothing to say");
    let p = home.profile("p");
    assert_eq!((p.vault.as_deref(), p.vault_id), (Some("other"), Some(vault_id(&ts, "other"))));
}

// spec: S26-07
#[test]
fn a_deleted_profile_vault_is_cleared_and_a_new_document_picks_its_vault_again() {
    let ts = TestServer::start();
    doc_in(&ts, "notes", "kept");
    let home = Home::new();
    let cwd = tempfile::tempdir().unwrap();
    for (vault, step) in [("temp", "list"), ("temp2", "push")] {
        ts.create_vault(vault);
        home.set("p", prof(&ts.url, &ts.owner_token, Some(vault), None));
        ok(run(&home, &["-p", "p", "list"])); // learns the id
        let (status, _) = ts.get("/api/v1/vaults", Some(&ts.owner_token));
        assert_eq!(status, 200);
        let r = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .build()
            .new_agent()
            .delete(format!("{}/api/v1/vaults/{vault}", ts.url))
            .header("Authorization", format!("Bearer {}", ts.owner_token))
            .call()
            .unwrap();
        assert_eq!(r.status().as_u16(), 200);
        let note = format!("note: Vault \"{vault}\" was deleted — cleared it from profile \"p\"\n");
        if step == "list" {
            let (so, se) = ok(run(&home, &["-p", "p", "list"]));
            assert!(so.contains("\tkept\t"), "every readable Vault: {so}");
            assert_eq!(se, note);
        } else {
            std::fs::write(cwd.path().join("new.md"), "# new\n").unwrap();
            let (so, se) = ok(run_in(&home, cwd.path(), &[], &["-p", "p", "push", "new.md"]));
            assert!(se.starts_with(&note), "{se}");
            assert!(so.ends_with("\tv1\n"), "{so}");
            let sc = std::fs::read_to_string(cwd.path().join("new.annotary")).unwrap();
            assert!(sc.contains("\"vault\": \"notes\""), "the one Vault left: {sc}");
        }
        let p = home.profile("p");
        assert_eq!((p.vault, p.vault_id), (None, None));
    }
}

// spec: S26-09
#[test]
fn an_explicit_vault_flag_for_a_renamed_vault_names_the_new_one() {
    let ts = TestServer::start();
    ts.create_vault("old");
    rename_elsewhere(&ts, "old", "new");
    let home = Home::new();
    home.set("rowan", prof(&ts.url, &ts.owner_token, None, None));
    let se = exits(run(&home, &["-p", "rowan", "list", "--vault", "old"]), 2);
    assert_eq!(se, "error: no Vault old — Vault old was renamed to new\n");
    let se = exits(run(&home, &["-p", "rowan", "mentions", "--vault", "old"]), 2);
    assert_eq!(se, "error: no Vault old — Vault old was renamed to new\n");
}

/// Wrap `from` (a line of `text`) in git conflict markers with `to` on the other side.
fn conflict(text: &str, from: &str, to: &str) -> String {
    assert!(text.contains(from), "{from}");
    text.replacen(from, &format!("<<<<<<< HEAD\n{from}\n=======\n{to}\n>>>>>>> other"), 1)
}

// spec: S26-04
#[test]
fn a_sidecar_git_conflicted_across_a_vault_rename_pushes_and_pulls_clean() {
    let ts = TestServer::start();
    ts.create_vault("team");
    let home = Home::new();
    home.set("rowan", prof(&ts.url, &ts.owner_token, None, None));
    let (a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (a.path(), b.path());
    std::fs::write(a.join("plan.md"), "# Plan\n\nOne.\n").unwrap();
    ok(run_in(&home, a, &[], &["-p", "rowan", "push", "plan.md", "--vault", "team"]));
    let before = std::fs::read_to_string(a.join("plan.annotary")).unwrap();
    assert!(before.contains("\"vault\": \"team\","), "{before}");
    // Another branch pulls after the rename: its Sidecar names the new Vault.
    rename_elsewhere(&ts, "team", "crew");
    let id = ts.get("/api/v1/docs", Some(&ts.owner_token)).1["docs"][0]["id"].as_str().unwrap().to_string();
    ok(run_in(&home, b, &[], &["-p", "rowan", "pull", &id]));
    // git merges the two: only the `vault` line differs.
    let merged = conflict(&before, "  \"vault\": \"team\",", "  \"vault\": \"crew\",");
    std::fs::write(a.join("plan.annotary"), &merged).unwrap();
    // Read-only verbs read it and leave the file alone.
    ok(run_in(&home, a, &[], &["-p", "rowan", "show", "plan.md"]));
    ok(run_in(&home, a, &[], &["-p", "rowan", "status", "plan.md"]));
    assert_eq!(std::fs::read_to_string(a.join("plan.annotary")).unwrap(), merged);
    // push: no human needed; the Sidecar is rewritten clean with the Vault's name now.
    std::fs::write(a.join("plan.md"), "# Plan\n\nTwo.\n").unwrap();
    let (so, se) = ok(run_in(&home, a, &[], &["-p", "rowan", "push", "plan.md"]));
    assert!(so.ends_with("\tv2\n"), "{so}");
    assert!(se.contains("git conflict markers"), "{se}");
    let clean = std::fs::read_to_string(a.join("plan.annotary")).unwrap();
    assert!(!clean.contains("<<<<<<<") && clean.contains("\"vault\": \"crew\","), "{clean}");
    assert!(annotary_sidecar::parse(&clean).is_ok());
    // pull as well.
    let merged = conflict(&clean, "  \"vault\": \"crew\",", "  \"vault\": \"team\",");
    std::fs::write(a.join("plan.annotary"), &merged).unwrap();
    ok(run_in(&home, a, &[], &["-p", "rowan", "pull", &id]));
    let clean = std::fs::read_to_string(a.join("plan.annotary")).unwrap();
    assert!(!clean.contains("<<<<<<<") && clean.contains("\"vault\": \"crew\","), "{clean}");
    // Two different Documents are still a human's call (exit 5).
    let other = conflict(&clean, &format!("  \"id\": \"{id}\","), "  \"id\": \"p3x8w2ka\",");
    std::fs::write(a.join("plan.annotary"), &other).unwrap();
    let se = exits(run_in(&home, a, &[], &["-p", "rowan", "push", "plan.md"]), 5);
    assert_eq!(se, "error: plan.annotary has conflicting Document ids — keep one side\n");
}
