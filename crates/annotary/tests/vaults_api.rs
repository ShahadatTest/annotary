//! CMD-5 over HTTP against the in-process server: `GET /api/v1/vaults` with Document counts, and
//! the profile-Vault headers — the Vault-scoped routes resolve a CLI profile's Vault through
//! renames in the same request and say so in the response headers, a deleted one falls back (or
//! is refused for a new Document), names travel percent-encoded, and rights stay as they were.
//! The CLI that sends and heals from these headers is `tests/cli_vaults.rs`.

use annotary::client::qs;
use annotary::testing::TestServer;
use annotary_store::{PushInput, Right};
use serde_json::{Value, json};

/// (status, response headers we care about, body)
struct Resp {
    status: u16,
    resolved: Option<String>,
    resolved_id: Option<String>,
    deleted: Option<String>,
    body: Value,
}

/// `method path` with `token` and the profile-Vault headers (`vault` = raw name, encoded here).
fn send(
    ts: &TestServer,
    method: &str,
    path: &str,
    token: &str,
    vault: Option<(&str, Option<&str>)>,
    body: Option<Value>,
) -> Resp {
    let url = format!("{}{}", ts.url, path);
    let agent = ureq::Agent::config_builder().http_status_as_error(false).build().new_agent();
    macro_rules! hdrs {
        ($r:expr) => {{
            let mut r = $r.header("Authorization", format!("Bearer {token}"));
            if let Some((name, id)) = vault {
                r = r.header("X-Annotary-Profile-Vault", qs(name));
                if let Some(id) = id {
                    r = r.header("X-Annotary-Profile-Vault-Id", id);
                }
            }
            r
        }};
    }
    let res = match method {
        "GET" => hdrs!(agent.get(&url)).call(),
        "POST" => hdrs!(agent.post(&url)).send_json(body.unwrap_or(json!({}))),
        other => panic!("{other}"),
    };
    let mut r = res.expect("request");
    let h = |k: &str| r.headers().get(k).and_then(|v| v.to_str().ok()).map(|v| annotary::client::unqs(v).unwrap());
    let (resolved, resolved_id, deleted) =
        (h("x-annotary-resolved-vault"), h("x-annotary-resolved-vault-id"), h("x-annotary-deleted-vault"));
    let status = r.status().as_u16();
    let text = r.body_mut().read_to_string().unwrap_or_default();
    Resp { status, resolved, resolved_id, deleted, body: serde_json::from_str(&text).unwrap_or(Value::String(text)) }
}

fn doc_in(ts: &TestServer, vault: &str, name: &str) -> String {
    let v = ts.store.vault_by_name(vault).unwrap().unwrap();
    let (hash, _) = ts.store.put_blob(format!("# {name}\n").as_bytes()).unwrap();
    ts.store.create_document(&ts.owner_id, &v.id, PushInput::new(name, &hash)).unwrap().id
}

// spec: S26-01
#[test]
fn the_vault_list_carries_document_counts() {
    let ts = TestServer::start();
    ts.create_vault("drafts");
    doc_in(&ts, "notes", "a");
    let gone = doc_in(&ts, "notes", "b");
    ts.store.delete_document(&ts.owner_id, &gone).unwrap();
    let r = send(&ts, "GET", "/api/v1/vaults", &ts.owner_token, None, None);
    assert_eq!(r.status, 200);
    let rows = r.body["vaults"].as_array().unwrap();
    let cell = |i: usize, k: &str| rows[i][k].clone();
    assert_eq!((cell(0, "name"), cell(0, "documents"), cell(0, "deleted")), (json!("notes"), json!(2), json!(1)));
    assert_eq!((cell(1, "name"), cell(1, "documents"), cell(1, "deleted")), (json!("drafts"), json!(0), json!(0)));
    assert!(rows[0]["id"].is_string() && rows[0]["created_at"].is_string() && rows[0]["default_actions"].is_array());
    // Nothing about a profile Vault without the header.
    assert_eq!((r.resolved, r.deleted), (None, None));
}

// spec: S26-06, S26-08
#[test]
fn every_vault_scoped_route_follows_a_renamed_profile_vault_in_one_request() {
    let ts = TestServer::start();
    ts.create_vault("Übersicht");
    let id = doc_in(&ts, "Übersicht", "plan");
    let v = ts.store.vault_by_name("Übersicht").unwrap().unwrap();
    ts.store.rename_vault(&ts.owner_id, &v.id, "Überblick 2026").unwrap();
    let tok = ts.owner_token.clone();
    let pv = Some(("Übersicht", None));
    let check = |r: &Resp, what: &str| {
        assert_eq!(r.resolved.as_deref(), Some("Überblick 2026"), "{what}: {:?}", r.body);
        assert_eq!(r.resolved_id.as_deref(), Some(v.id.as_str()), "{what}");
        assert_eq!(r.deleted, None, "{what}");
    };
    // list: scoped to the renamed Vault (the query names the old one, as the CLI sends it).
    let r = send(&ts, "GET", &format!("/api/v1/docs?vault={}", qs("Übersicht")), &tok, pv, None);
    assert_eq!(r.status, 200, "{:?}", r.body);
    assert_eq!(r.body["docs"].as_array().unwrap().len(), 1);
    check(&r, "list");
    // resolve by Name, in that Vault.
    let r = send(&ts, "GET", &format!("/api/v1/docs/resolve?doc=plan&vault={}", qs("Übersicht")), &tok, pv, None);
    assert_eq!((r.status, r.body["id"].as_str()), (200, Some(id.as_str())));
    check(&r, "resolve");
    // mentions.
    let r = send(&ts, "GET", &format!("/api/v1/mentions?vault={}", qs("Übersicht")), &tok, pv, None);
    assert_eq!(r.status, 200);
    check(&r, "mentions");
    // a new Document lands in the renamed Vault.
    let (hash, _) = ts.store.put_blob(b"# new\n").unwrap();
    let body = json!({ "vault": "Übersicht", "name": "new", "md_hash": hash });
    let r = send(&ts, "POST", "/api/v1/docs", &tok, pv, Some(body));
    assert_eq!((r.status, r.body["vault"].as_str()), (201, Some("Überblick 2026")), "{:?}", r.body);
    check(&r, "create");
    // `vaults` and `me` only report it.
    check(&send(&ts, "GET", "/api/v1/vaults", &tok, pv, None), "vaults");
    let r = send(&ts, "GET", "/api/v1/me", &tok, pv, None);
    assert_eq!(r.body["handle"], "rowan");
    check(&r, "me");
    // A Vault-less route says nothing.
    let r = send(&ts, "GET", &format!("/api/v1/docs/{id}"), &tok, pv, None);
    assert_eq!((r.status, r.resolved), (200, None));
    // An unchanged profile Vault is echoed as it is (the CLI backfills its id from it).
    let r = send(&ts, "GET", "/api/v1/docs?vault=notes", &tok, Some(("notes", None)), None);
    assert_eq!(r.resolved.as_deref(), Some("notes"));
    // An explicit `vault=` that is not the profile's is taken as it is: no echo, 404 by name.
    let r = send(&ts, "GET", "/api/v1/docs?vault=notes", &tok, pv, None);
    assert_eq!((r.status, r.resolved), (200, None));
    // The id keeps it on course after the old name is taken again (chained a -> b -> c too).
    ts.store.rename_vault(&ts.owner_id, &v.id, "Ü3").unwrap();
    ts.create_vault("Übersicht");
    let r = send(&ts, "GET", "/api/v1/docs?vault=%C3%9Cbersicht", &tok, Some(("Übersicht", Some(&v.id))), None);
    assert_eq!(r.resolved.as_deref(), Some("Ü3"));
    assert_eq!(r.body["docs"].as_array().unwrap().len(), 2, "the old Vault's Documents, not the new empty one");
    // Without the id the live name wins (a legacy profile now means the new Vault).
    let r = send(&ts, "GET", "/api/v1/docs?vault=%C3%9Cbersicht", &tok, Some(("Übersicht", None)), None);
    assert_eq!(r.resolved.as_deref(), Some("Übersicht"));
    assert_eq!(r.body["docs"].as_array().unwrap().len(), 0);
}

// spec: S26-07
#[test]
fn a_deleted_profile_vault_falls_back_or_is_refused_and_is_reported() {
    let ts = TestServer::start();
    ts.create_vault("gone");
    let v = ts.store.vault_by_name("gone").unwrap().unwrap();
    ts.store.delete_vault(&ts.owner_id, &v.id).unwrap();
    doc_in(&ts, "notes", "kept");
    let tok = ts.owner_token.clone();
    for pv in [("gone", Some(v.id.as_str())), ("gone", None)] {
        let r = send(&ts, "GET", "/api/v1/docs?vault=gone", &tok, Some(pv), None);
        assert_eq!(r.status, 200, "{:?}", r.body);
        assert_eq!(r.deleted.as_deref(), Some("gone"));
        assert_eq!(r.resolved, None);
        assert_eq!(r.body["docs"].as_array().unwrap().len(), 1, "every readable Vault");
        let r = send(&ts, "GET", "/api/v1/docs/resolve?doc=kept&vault=gone", &tok, Some(pv), None);
        assert_eq!((r.status, r.deleted.as_deref()), (200, Some("gone")));
        let r = send(&ts, "GET", "/api/v1/mentions?vault=gone", &tok, Some(pv), None);
        assert_eq!((r.status, r.deleted.as_deref()), (200, Some("gone")));
        let (hash, _) = ts.store.put_blob(b"# x\n").unwrap();
        let r = send(
            &ts,
            "POST",
            "/api/v1/docs",
            &tok,
            Some(pv),
            Some(json!({"vault": "gone", "name": "x", "md_hash": hash})),
        );
        assert_eq!((r.status, r.body["error"].as_str()), (404, Some("vault_deleted")), "{:?}", r.body);
        assert_eq!(r.body["message"], "Vault gone was deleted");
        assert_eq!(r.deleted.as_deref(), Some("gone"), "the refusal carries it too");
        assert_eq!(send(&ts, "GET", "/api/v1/vaults", &tok, Some(pv), None).deleted.as_deref(), Some("gone"));
    }
    // Without the header an unknown Vault is the 404 it always was.
    let r = send(&ts, "GET", "/api/v1/docs?vault=gone", &tok, None, None);
    assert_eq!((r.status, r.body["message"].as_str()), (404, Some("no Vault gone")));
}

// spec: S26-09
#[test]
fn rights_hold_for_a_resolved_profile_vault_and_the_rename_hint() {
    let ts = TestServer::start();
    ts.create_vault("secret");
    let v = ts.store.vault_by_name("secret").unwrap().unwrap();
    let (m, tok) = ts.add_member("reader");
    // The Member may read `notes` only (no `*`).
    ts.store.grant(&ts.owner_id, &m, "*", &[]).unwrap();
    ts.store.grant(&ts.owner_id, &m, "notes", &[Right::Read]).unwrap();
    ts.store.rename_vault(&ts.owner_id, &v.id, "hidden").unwrap();
    // Their profile names `secret`: 403 naming `secret`, the new name never echoed.
    let r = send(&ts, "GET", "/api/v1/docs?vault=secret", &tok, Some(("secret", None)), None);
    assert_eq!(r.status, 403, "{:?}", r.body);
    assert!(r.body["message"].as_str().unwrap().contains("Vault secret"), "{:?}", r.body);
    assert!(!r.body.to_string().contains("hidden"));
    assert_eq!((r.resolved, r.resolved_id), (None, None));
    assert_eq!(send(&ts, "GET", "/api/v1/vaults", &tok, Some(("secret", None)), None).resolved, None);
    assert_eq!(send(&ts, "GET", "/api/v1/me", &tok, Some(("secret", None)), None).resolved, None);
    // Read but not write on the renamed Vault: a new Document there is refused `write`, naming
    // the profile's name, nothing echoed.
    let n = ts.store.vault_by_name("notes").unwrap().unwrap();
    ts.store.rename_vault(&ts.owner_id, &n.id, "notes2").unwrap();
    let (hash, _) = ts.store.put_blob(b"# y\n").unwrap();
    let r = send(
        &ts,
        "POST",
        "/api/v1/docs",
        &tok,
        Some(("notes", None)),
        Some(json!({"vault": "notes", "name": "y", "md_hash": hash})),
    );
    assert_eq!(r.status, 403, "{:?}", r.body);
    assert!(r.body["message"].as_str().unwrap().contains("`write` on Vault notes"), "{:?}", r.body);
    assert_eq!(r.resolved, None);
    // …while a read follows it (their Grant followed the rename).
    let r = send(&ts, "GET", "/api/v1/docs?vault=notes", &tok, Some(("notes", None)), None);
    assert_eq!((r.status, r.resolved.as_deref()), (200, Some("notes2")));
    // An explicit `vault=OLD` (no profile header): 404, the hint only for who may read the new one.
    let r = send(&ts, "GET", "/api/v1/docs?vault=notes", &tok, None, None);
    assert_eq!((r.status, r.body["hint"].as_str()), (404, Some("Vault notes was renamed to notes2")));
    let r = send(&ts, "GET", "/api/v1/docs?vault=secret", &tok, None, None);
    assert_eq!((r.status, r.body.get("hint")), (404, None), "{:?}", r.body);
    let r = send(&ts, "GET", "/api/v1/docs?vault=secret", &ts.owner_token, None, None);
    assert_eq!(r.body["hint"], "Vault secret was renamed to hidden");
    let r = send(&ts, "GET", "/api/v1/docs/resolve?doc=x&vault=secret", &ts.owner_token, None, None);
    assert_eq!((r.status, r.body["hint"].as_str()), (404, Some("Vault secret was renamed to hidden")));
}
