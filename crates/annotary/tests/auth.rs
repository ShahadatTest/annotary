//! Bearer auth, owner-only, tokens at rest, `last_used_at`, handles.

use annotary::testing::TestServer;
use annotary_store::{Role, token};
use serde_json::json;

fn members_body(ts: &TestServer, tok: Option<&str>) -> (u16, serde_json::Value) {
    ts.get("/api/v1/members", tok)
}

// spec: S01-06
#[test]
fn missing_invalid_revoked_or_disabled_credentials_are_401() {
    let ts = TestServer::start();
    let (status, body) = members_body(&ts, None);
    assert_eq!(status, 401);
    assert_eq!(body["error"], "unauthorized");
    assert!(body["message"].as_str().unwrap().contains("no credential"));
    assert!(body["hint"].as_str().is_some());

    let (status, body) = members_body(&ts, Some("ant_notAToken00000000000000000000000000000000"));
    assert_eq!(status, 401);
    assert_eq!(body["error"], "unauthorized");
    let (status, _) = members_body(&ts, Some("garbage"));
    assert_eq!(status, 401);

    let (status, _) = members_body(&ts, Some(&ts.owner_token));
    assert_eq!(status, 200);

    // Revoked token ⇒ 401.
    let (agent_id, agent_tok) = ts.add_member("agent");
    let (status, _) = members_body(&ts, Some(&agent_tok));
    assert_eq!(status, 200);
    let tokens = ts.store.tokens_for_member(&agent_id).unwrap();
    ts.store.revoke_token(&ts.owner_id, &tokens[0].id).unwrap();
    let (status, body) = members_body(&ts, Some(&agent_tok));
    assert_eq!(status, 401, "{body}");

    // Disabled member ⇒ 401 even with a fresh token; history intact.
    let (other_id, other_tok) = ts.add_member("other");
    ts.store.disable_member(&ts.owner_id, &other_id).unwrap();
    let (status, _) = members_body(&ts, Some(&other_tok));
    assert_eq!(status, 401);
    let (status, body) = members_body(&ts, Some(&ts.owner_token));
    assert_eq!(status, 200);
    let listed: Vec<&str> = body["members"].as_array().unwrap().iter().map(|m| m["handle"].as_str().unwrap()).collect();
    assert_eq!(listed, vec!["rowan", "agent", "other"], "disabled members are kept in the list");
    let other = body["members"].as_array().unwrap().iter().find(|m| m["handle"] == "other").unwrap();
    assert!(!other["disabled_at"].is_null());

    // Bad Authorization scheme ⇒ 401.
    let resp = ureq::get(format!("{}/api/v1/members", ts.url))
        .header("Authorization", format!("Basic {}", ts.owner_token))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 401);
}

// spec: S01-07
#[test]
fn owner_only_endpoints_are_403_for_members() {
    let ts = TestServer::start();
    let (_id, agent_tok) = ts.add_member("agent");
    let (status, body) = ts.post("/api/v1/members", Some(&agent_tok), &json!({ "handle": "nope" }));
    assert_eq!(status, 403);
    assert_eq!(body["error"], "forbidden");
    assert!(body["message"].as_str().unwrap().contains("owner-only"), "{body}");
    assert!(body["hint"].as_str().is_some());
    // Listing is allowed for any Member (handles are needed for @mentions).
    let (status, _) = ts.get("/api/v1/members", Some(&agent_tok));
    assert_eq!(status, 200);
    let (status, body) = ts.get("/api/v1/me", Some(&agent_tok));
    assert_eq!(status, 200);
    assert_eq!(body["handle"], "agent");
    assert_eq!(body["role"], "member");
    // Owner may add.
    let (status, body) =
        ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "handle": "nope", "name": "Nope" }));
    assert_eq!(status, 201, "{body}");
    assert_eq!(body["name"], "Nope");
}

// spec: S01-08
#[test]
fn last_used_at_updates_on_every_use() {
    let ts = TestServer::start();
    let (agent_id, agent_tok) = ts.add_member("agent");
    let t0 = ts.store.tokens_for_member(&agent_id).unwrap().remove(0);
    assert!(t0.last_used_at.is_none(), "unused token has no last_used_at");
    let (status, _) = ts.get("/api/v1/me", Some(&agent_tok));
    assert_eq!(status, 200);
    let t1 = ts.store.tokens_for_member(&agent_id).unwrap().remove(0);
    let first = t1.last_used_at.expect("stamped on use");
    std::thread::sleep(std::time::Duration::from_millis(15));
    let (status, _) = ts.get("/api/v1/me", Some(&agent_tok));
    assert_eq!(status, 200);
    let t2 = ts.store.tokens_for_member(&agent_id).unwrap().remove(0);
    let second = t2.last_used_at.expect("stamped again");
    assert!(second > first, "{second} > {first}");
    // A rejected token does not stamp anything.
    let owner_before = ts.store.tokens_for_member(&ts.owner_id).unwrap().remove(0).last_used_at;
    let _ = ts.get("/api/v1/me", Some("ant_bogus"));
    assert_eq!(ts.store.tokens_for_member(&ts.owner_id).unwrap().remove(0).last_used_at, owner_before);
}

// spec: S01-09, S01-11
#[test]
fn tokens_are_ant_prefixed_base62_and_hashed_at_rest() {
    let ts = TestServer::start();
    let (status, body) =
        ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "handle": "scribe", "name": "Scribe" }));
    assert_eq!(status, 201, "{body}");
    let plain = body["token"].as_str().expect("token shown once in the add response");
    assert!(plain.starts_with("ant_"));
    assert_eq!(plain.len(), 4 + 43, "≥ 32 bytes of entropy as base62");
    assert!(token::looks_like_token(plain));
    // One step: member + membership (role member) + `*` grant with all four rights + token.
    assert_eq!(body["role"], "member");
    assert_eq!(body["grants"], json!([{"vault":"*","rights":["read","comment","action","write"]}]));
    let m = ts.store.member_by_handle("scribe").unwrap().unwrap();
    assert_eq!(m.role, Role::Member);
    let toks = ts.store.tokens_for_member(&m.id).unwrap();
    assert_eq!(toks.len(), 1);
    assert_eq!(toks[0].hash, token::hash(plain), "sha256 at rest");
    assert_ne!(toks[0].hash, plain);
    // The plain text is nowhere in the database file.
    let db = std::fs::read(ts.data_dir.join("store").join(annotary_store::DB_FILE)).unwrap();
    let wal =
        std::fs::read(ts.data_dir.join("store").join(format!("{}-wal", annotary_store::DB_FILE))).unwrap_or_default();
    let needle = plain.as_bytes();
    assert!(!db.windows(needle.len()).any(|w| w == needle), "plain token must not be in the db");
    assert!(!wal.windows(needle.len()).any(|w| w == needle), "plain token must not be in the wal");
    // Listing never returns the token.
    let (_, list) = ts.get("/api/v1/members", Some(&ts.owner_token));
    assert!(list["members"].as_array().unwrap().iter().all(|m| m.get("token").is_none()));
    // The pasted token works as that member.
    let (status, me) = ts.get("/api/v1/me", Some(plain));
    assert_eq!(status, 200);
    assert_eq!(me["handle"], "scribe");
    assert_eq!(me["name"], "Scribe");
}

// spec: S01-13
#[test]
fn handles_are_kebab_case_and_unique_forever() {
    let ts = TestServer::start();
    for bad in ["Scribe", "scribe_bot", "-x", "x-", "a--b", "", "a b", "é"] {
        let (status, body) = ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "handle": bad }));
        assert_eq!(status, 400, "{bad:?}: {body}");
        assert_eq!(body["error"], "invalid");
    }
    let (status, _) = ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "handle": "linter" }));
    assert_eq!(status, 201);
    let (status, body) = ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "handle": "linter" }));
    assert_eq!(status, 409);
    assert_eq!(body["error"], "conflict");
    assert!(body["message"].as_str().unwrap().contains("unique per Tenant forever"));
    // The owner's handle is taken too.
    let (status, _) = ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "handle": "rowan" }));
    assert_eq!(status, 409);
    // Disabled ⇒ still taken.
    let linter = ts.store.member_by_handle("linter").unwrap().unwrap();
    ts.store.disable_member(&ts.owner_id, &linter.id).unwrap();
    let (status, _) = ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "handle": "linter" }));
    assert_eq!(status, 409);
    // Malformed body ⇒ 400 JSON error, not axum's plain text.
    let (status, body) = ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "nope": 1 }));
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "invalid");
    // Unknown route ⇒ 404 JSON error.
    let (status, body) = ts.get("/api/v1/nothing", Some(&ts.owner_token));
    assert_eq!(status, 404);
    assert_eq!(body["error"], "not_found");
}

// spec: S01-21
#[test]
fn the_server_never_writes_a_profile() {
    let ts = TestServer::start();
    let _ = ts.post("/api/v1/members", Some(&ts.owner_token), &json!({ "handle": "agent" }));
    assert!(!ts.data_dir.join("config.toml").exists(), "server must not write config.toml");
}
