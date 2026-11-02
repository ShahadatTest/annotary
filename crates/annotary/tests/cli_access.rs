//! The real binary for the token round trip trycmd cannot capture: `token mint --json` → the
//! new Token works → `token revoke ID` (own, then an owner's second revoke is a no-op) → 401 /
//! exit 3; plus `member disable` cutting a Member's Token off with history intact.

use std::path::Path;
use std::process::{Command, Output};

use annotary::testing::TestServer;

fn run(ts: &TestServer, home: &Path, token: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(args)
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env("ANNOTARY_TOKEN", token)
        .env("ANNOTARY_SERVER", &ts.url)
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_TTY")
        .output()
        .unwrap()
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

// spec: S06-07, S06-05
#[test]
fn token_mint_revoke_and_disable_cut_credentials_off() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let home = home.path();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("agent");

    // A Member mints their own Token; the plain text is printed once and works.
    let out = run(&ts, home, &agent, &["token", "mint", "--name", "ci", "--json", "id,token,member,name"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let v: serde_json::Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    assert_eq!(v["member"], "agent");
    assert_eq!(v["name"], "ci");
    let id = v["id"].as_str().unwrap().to_string();
    let plain = v["token"].as_str().unwrap().to_string();
    assert!(plain.starts_with("ant_"));
    let out = run(&ts, home, &plain, &["member", "list"]);
    assert_eq!(out.status.code(), Some(0));
    // It shows in the Member's list (never the plain text), with `last_used_at` set by that call.
    let out = run(&ts, home, &agent, &["token", "list", "--json", "id,name,last_used"]);
    let v: serde_json::Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    let row = v.as_array().unwrap().iter().find(|t| t["id"] == id).unwrap();
    assert!(row["last_used"].as_str().is_some_and(|s| s.ends_with('Z')));
    assert!(!text(&out).0.contains(&plain));

    // Revoke: the Member's own → 401 (exit 3) for the revoked token; a second revoke by the
    // owner is a no-op with a note; the row shows `revoked` (with --all).
    let out = run(&ts, home, &agent, &["token", "revoke", &id]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert_eq!(so, format!("{id}\t@agent\trevoked\n"));
    let out = run(&ts, home, &plain, &["member", "list"]);
    assert_eq!(out.status.code(), Some(3), "{}", text(&out).1);
    let out = run(&ts, home, &owner, &["token", "revoke", &id, "--json"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    assert!(se.contains("was already revoked"), "{se}");
    let v: serde_json::Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!(v["revoked"], true);
    let out = run(&ts, home, &agent, &["token", "list", "--all"]);
    assert!(text(&out).0.contains("@agent\tci\t") && text(&out).0.contains("\trevoked\t"), "{}", text(&out).0);
    let out = run(&ts, home, &agent, &["token", "list"]);
    assert!(!text(&out).0.contains("\tci\t"), "revoked hidden by default: {}", text(&out).0);
    // The owner revokes another Member's live Token; a Member cannot touch the owner's.
    let out = run(&ts, home, &owner, &["token", "list", "--member", "rowan", "--json", "id"]);
    let v: serde_json::Value = serde_json::from_str(text(&out).0.trim()).unwrap();
    let owner_tid = v[0]["id"].as_str().unwrap().to_string();
    let out = run(&ts, home, &agent, &["token", "revoke", &owner_tid]);
    assert_eq!(out.status.code(), Some(4), "{}", text(&out).1);

    // Disable: the Member's remaining Token stops working; their Thread stays attributed.
    let (hash, _) = ts.store.put_blob(b"# d\n").unwrap();
    let vault = ts.store.vault_by_name("notes").unwrap().unwrap();
    let doc = ts.store.create_document(&ts.owner_id, &vault.id, annotary_store::PushInput::new("d", hash)).unwrap();
    let out = run(&ts, home, &agent, &["thread", "new", &doc.id, "--document", "-m", "by the agent"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let out = run(&ts, home, &owner, &["member", "disable", "agent"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).0, "@agent\tagent\tmember (disabled)\t*:read,comment,action,write\n");
    let out = run(&ts, home, &agent, &["member", "list"]);
    assert_eq!(out.status.code(), Some(3));
    let out = run(&ts, home, &owner, &["thread", "show", &doc.id, "t1", "--json", "id,author"]);
    assert!(text(&out).0.contains("\"author\":\"agent\""), "{}", text(&out).0);
    let out = run(&ts, home, &owner, &["member", "add", "agent"]);
    assert_eq!(out.status.code(), Some(5), "handle never reused");
}
