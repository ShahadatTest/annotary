//! Ticket 06 over HTTP against the in-process server: the table-driven rights matrix (every
//! endpoint × every kind of caller), `*` covering a later Vault, owner-only operations, the last
//! owner, disable/rename/handle reuse, Tokens (mint/list/revoke/expiry/`last_used_at`), the local
//! password + `login`, browser sessions (loopback trust iff no password, login page, logout) and
//! the API's 401 with neither cookie nor bearer.

use std::sync::atomic::{AtomicUsize, Ordering};

use annotary::testing::TestServer;
use annotary_store::{Grant, NewThread, PushInput, Right};
use serde_json::{Value, json};

// ---------------------------------------------------------------- HTTP helpers

fn send(ts: &TestServer, method: &str, path: &str, token: Option<&str>, body: Option<&Value>) -> (u16, Value) {
    let url = format!("{}{}", ts.url, path);
    let auth = token.map(|t| format!("Bearer {t}"));
    let res = match method {
        "GET" => {
            let mut r = ureq::get(&url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.config().http_status_as_error(false).build().call()
        }
        "DELETE" => {
            let mut r = ureq::delete(&url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.config().http_status_as_error(false).build().call()
        }
        "POST" => {
            let mut r = ureq::post(&url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.config().http_status_as_error(false).build().send_json(body.cloned().unwrap_or(json!({})))
        }
        // CMD-23: `PATCH /api/v1/me/devices/{id}`
        "PATCH" => {
            let mut r = ureq::patch(&url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.config().http_status_as_error(false).build().send_json(body.cloned().unwrap_or(json!({})))
        }
        "PUT" => {
            let mut r = ureq::put(&url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.config().http_status_as_error(false).build().send_json(body.cloned().unwrap_or(json!({})))
        }
        "PUTBYTES" => {
            let mut r = ureq::put(&url).content_type("application/octet-stream");
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.config().http_status_as_error(false).build().send(b"# bytes\n".as_slice())
        }
        _ => unreachable!(),
    };
    match res {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let text = r.body_mut().read_to_string().unwrap_or_default();
            (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
        }
        Err(e) => (0, Value::String(e.to_string())),
    }
}

/// A raw GET with arbitrary headers, no redirect following; returns (status, headers, body).
fn raw_get(url: &str, headers: &[(&str, &str)]) -> (u16, Vec<(String, String)>, String) {
    let mut req = ureq::get(url);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut r = req.config().http_status_as_error(false).max_redirects(0).build().call().unwrap();
    let status = r.status().as_u16();
    let hs: Vec<(String, String)> = r
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_string()))
        .collect();
    (status, hs, r.body_mut().read_to_string().unwrap_or_default())
}

fn raw_post_form(url: &str, form: &str, headers: &[(&str, &str)]) -> (u16, Vec<(String, String)>, String) {
    let mut req = ureq::post(url).content_type("application/x-www-form-urlencoded");
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut r = req.config().http_status_as_error(false).max_redirects(0).build().send(form.as_bytes()).unwrap();
    let status = r.status().as_u16();
    let hs: Vec<(String, String)> = r
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_string()))
        .collect();
    (status, hs, r.body_mut().read_to_string().unwrap_or_default())
}

fn header<'a>(hs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn cookie_of(hs: &[(String, String)]) -> Option<String> {
    header(hs, "set-cookie").and_then(|c| c.split(';').next()).map(str::to_string)
}

// ---------------------------------------------------------------- store-side setup helpers

static COUNTER: AtomicUsize = AtomicUsize::new(0);

/// A fresh Document (v1, unique content) in `vault`, created through the store as the owner.
fn fresh_doc(ts: &TestServer, vault: &str) -> String {
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let md = format!("# doc {n}\n\nline two of {n}\n");
    let (hash, _) = ts.store.put_blob(md.as_bytes()).unwrap();
    let v = ts.store.vault_by_name(vault).unwrap().unwrap();
    ts.store.create_document(&ts.owner_id, &v.id, PushInput::new(format!("m{n}"), hash)).unwrap().id
}

fn deleted_doc(ts: &TestServer, vault: &str) -> String {
    let id = fresh_doc(ts, vault);
    ts.store.delete_document(&ts.owner_id, &id).unwrap();
    id
}

/// A Document with one Document-level Thread `t1` by `author` (a Member id).
fn doc_with_thread(ts: &TestServer, vault: &str, author: &str) -> String {
    let id = fresh_doc(ts, vault);
    ts.store.new_thread(author, &id, NewThread::document("hello")).unwrap();
    id
}

/// CMD-23: a fresh iPad Device of the caller, made through the store.
fn own_device(ts: &TestServer, c: &Caller) -> String {
    let id = annotary_store::settings::new_device_id();
    ts.store.ensure_device(&c.id, &id, Some(("ipad", "iPad"))).unwrap();
    id
}

fn member_with(ts: &TestServer, handle: &str, grants: Vec<Grant>) -> (String, String) {
    let (id, tok) = ts.add_member(handle);
    ts.store.set_grants(&ts.owner_id, &id, grants).unwrap();
    (id, tok)
}

// ---------------------------------------------------------------- the matrix

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Need {
    /// Any authenticated Member.
    Any,
    /// Owner-only.
    Owner,
    /// A Right on the Document's/named Vault.
    Right(Right),
    /// `comment` on the Vault and (author or owner) — edit/delete a Comment.
    CommentAuthor,
    /// A Right on *some* Vault (blob transport, `/a/{hash}`).
    AnyVault(Right),
    /// Nothing: answered before sign-in, whoever asks (M3b, the icons and the manifest).
    Public,
}

struct Caller {
    name: &'static str,
    handle: String,
    id: String,
    token: String,
    /// Effective rights on Vault `notes` / on a later Vault, as expected by the table.
    on_notes: Vec<Right>,
    on_later: Vec<Right>,
    owner: bool,
    disabled: bool,
}

type Call = Box<dyn Fn(&TestServer, &Caller, &str) -> (u16, Value)>;

/// One endpoint: how to call it as `caller` against a Document/Vault named `vault`.
struct Endpoint {
    name: &'static str,
    need: Need,
    call: Call,
}

fn ep(name: &'static str, need: Need, call: impl Fn(&TestServer, &Caller, &str) -> (u16, Value) + 'static) -> Endpoint {
    Endpoint { name, need, call: Box::new(call) }
}

fn endpoints() -> Vec<Endpoint> {
    let tok = |c: &Caller| Some(c.token.clone());
    vec![
        // members / vaults / settings
        ep("GET members", Need::Any, move |ts, c, _| send(ts, "GET", "/api/v1/members", tok(c).as_deref(), None)),
        ep("GET me", Need::Any, move |ts, c, _| send(ts, "GET", "/api/v1/me", tok(c).as_deref(), None)),
        // ---- CMD-23: Member settings and Devices — every Member, for themselves only ----
        ep("GET me/settings", Need::Any, move |ts, c, _| {
            send(ts, "GET", "/api/v1/me/settings", tok(c).as_deref(), None)
        }),
        ep("GET me/settings/catalog", Need::Any, move |ts, c, _| {
            send(ts, "GET", "/api/v1/me/settings/catalog", tok(c).as_deref(), None)
        }),
        ep("GET me/settings/matrix", Need::Any, move |ts, c, _| {
            send(ts, "GET", "/api/v1/me/settings/matrix", tok(c).as_deref(), None)
        }),
        ep("PUT me/settings", Need::Any, move |ts, c, _| {
            let body = json!({ "key": "appearance", "value": "dark", "scope": "all" });
            send(ts, "PUT", "/api/v1/me/settings", tok(c).as_deref(), Some(&body))
        }),
        ep("DELETE me/settings/{key}", Need::Any, move |ts, c, _| {
            send(ts, "DELETE", "/api/v1/me/settings/times?scope=user", tok(c).as_deref(), None)
        }),
        ep("GET me/devices", Need::Any, move |ts, c, _| send(ts, "GET", "/api/v1/me/devices", tok(c).as_deref(), None)),
        ep("PATCH me/devices/{id}", Need::Any, move |ts, c, _| {
            let id = own_device(ts, c);
            send(ts, "PATCH", &format!("/api/v1/me/devices/{id}"), tok(c).as_deref(), Some(&json!({ "name": "Pad" })))
        }),
        ep("POST me/devices/{id}/forget", Need::Any, move |ts, c, _| {
            let id = own_device(ts, c);
            send(ts, "POST", &format!("/api/v1/me/devices/{id}/forget"), tok(c).as_deref(), None)
        }),
        ep("POST me/devices/{id}/copy-from", Need::Any, move |ts, c, _| {
            let id = own_device(ts, c);
            send(ts, "POST", "/api/v1/me/devices/all/copy-from", tok(c).as_deref(), Some(&json!({ "from": id })))
        }),
        ep("GET me/devices/{id}/sync", Need::Any, move |ts, c, _| {
            let id = own_device(ts, c);
            send(ts, "GET", &format!("/api/v1/me/devices/{id}/sync"), tok(c).as_deref(), None)
        }),
        ep("POST me/rename", Need::Any, move |ts, c, _| {
            send(
                ts,
                "POST",
                "/api/v1/me/rename",
                tok(c).as_deref(),
                Some(&json!({ "name": format!("{} again", c.handle) })),
            )
        }),
        // ---- end CMD-23 ----
        ep("GET vaults", Need::Any, move |ts, c, _| send(ts, "GET", "/api/v1/vaults", tok(c).as_deref(), None)),
        ep("GET members/{h}", Need::Any, move |ts, c, _| {
            send(ts, "GET", "/api/v1/members/rowan", tok(c).as_deref(), None)
        }),
        ep("POST members", Need::Owner, move |ts, c, _| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            send(ts, "POST", "/api/v1/members", tok(c).as_deref(), Some(&json!({ "handle": format!("new{n}") })))
        }),
        ep("POST members/{h}/rename", Need::Owner, move |ts, c, _| {
            send(ts, "POST", "/api/v1/members/rowan/rename", tok(c).as_deref(), Some(&json!({ "name": "Rowan A" })))
        }),
        ep("POST members/{h}/disable", Need::Owner, move |ts, c, _| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let (_, _) = ts.add_member(&format!("victim{n}"));
            send(ts, "POST", &format!("/api/v1/members/victim{n}/disable"), tok(c).as_deref(), None)
        }),
        ep("POST members/{h}/role", Need::Owner, move |ts, c, _| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let (_, _) = ts.add_member(&format!("promo{n}"));
            send(
                ts,
                "POST",
                &format!("/api/v1/members/promo{n}/role"),
                tok(c).as_deref(),
                Some(&json!({ "role": "owner" })),
            )
        }),
        ep("POST members/{h}/grants", Need::Owner, move |ts, c, _| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let (_, _) = ts.add_member(&format!("grantee{n}"));
            send(
                ts,
                "POST",
                &format!("/api/v1/members/grantee{n}/grants"),
                tok(c).as_deref(),
                Some(&json!({ "vault": "notes", "rights": ["read"] })),
            )
        }),
        ep("PUT members/{h}/grants", Need::Owner, move |ts, c, _| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let (_, _) = ts.add_member(&format!("grantee{n}"));
            send(
                ts,
                "PUT",
                &format!("/api/v1/members/grantee{n}/grants"),
                tok(c).as_deref(),
                Some(&json!({ "grants": [{ "vault": "*", "rights": ["read"] }] })),
            )
        }),
        ep("GET tokens (own)", Need::Any, move |ts, c, _| send(ts, "GET", "/api/v1/tokens", tok(c).as_deref(), None)),
        ep("POST tokens (own)", Need::Any, move |ts, c, _| {
            send(ts, "POST", "/api/v1/tokens", tok(c).as_deref(), Some(&json!({ "name": "mine" })))
        }),
        ep("GET tokens?member=rowan", Need::Owner, move |ts, c, _| {
            send(ts, "GET", "/api/v1/tokens?member=rowan", tok(c).as_deref(), None)
        }),
        ep("POST tokens {member: other}", Need::Owner, move |ts, c, _| {
            send(ts, "POST", "/api/v1/tokens", tok(c).as_deref(), Some(&json!({ "member": "rowan" })))
        }),
        ep("DELETE tokens/{id of other}", Need::Owner, move |ts, c, _| {
            let t = ts.store.mint_token_expiring(&ts.owner_id, &ts.owner_id, Some("victim"), None).unwrap();
            send(ts, "DELETE", &format!("/api/v1/tokens/{}", t.id), tok(c).as_deref(), None)
        }),
        // ---- ticket 18: pairing — any Member for themselves, owner-only for another ----
        ep("POST pairing (own)", Need::Any, move |ts, c, _| {
            ts.store.set_public_url(None, Some("http://studio.example.ts.net:7377")).unwrap();
            send(ts, "POST", "/api/v1/pairing", tok(c).as_deref(), Some(&json!({ "name": "phone" })))
        }),
        ep("POST pairing {member: other}", Need::Owner, move |ts, c, _| {
            ts.store.set_public_url(None, Some("http://studio.example.ts.net:7377")).unwrap();
            send(ts, "POST", "/api/v1/pairing", tok(c).as_deref(), Some(&json!({ "member": "rowan" })))
        }),
        // ---- end ticket 18 ----
        ep("GET settings", Need::Owner, move |ts, c, _| send(ts, "GET", "/api/v1/settings", tok(c).as_deref(), None)),
        ep("PUT settings/password", Need::Owner, move |ts, c, _| {
            send(ts, "PUT", "/api/v1/settings/password", tok(c).as_deref(), Some(&json!({ "password": null })))
        }),
        ep("POST vaults", Need::Owner, move |ts, c, _| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            send(ts, "POST", "/api/v1/vaults", tok(c).as_deref(), Some(&json!({ "name": format!("v{n}") })))
        }),
        ep("GET vaults/{v}/actions", Need::Right(Right::Read), move |ts, c, v| {
            send(ts, "GET", &format!("/api/v1/vaults/{v}/actions"), tok(c).as_deref(), None)
        }),
        ep("PUT vaults/{v}/actions", Need::Owner, move |ts, c, v| {
            send(
                ts,
                "PUT",
                &format!("/api/v1/vaults/{v}/actions"),
                tok(c).as_deref(),
                Some(&json!({ "available": [{ "id": "approve" }] })),
            )
        }),
        // ---- ticket 12: Vault rename / delete (each call on a throwaway Vault of its own) ----
        ep("POST vaults/{v}/rename", Need::Owner, move |ts, c, _| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            ts.create_vault(&format!("rn{n}"));
            send(
                ts,
                "POST",
                &format!("/api/v1/vaults/rn{n}/rename"),
                tok(c).as_deref(),
                Some(&json!({ "name": format!("rn{n}-b") })),
            )
        }),
        ep("DELETE vaults/{v}", Need::Owner, move |ts, c, _| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            ts.create_vault(&format!("del{n}"));
            send(ts, "DELETE", &format!("/api/v1/vaults/del{n}"), tok(c).as_deref(), None)
        }),
        // ---- end ticket 12 ----
        // blob transport (a Right on some Vault)
        ep("POST blobs/check", Need::AnyVault(Right::Write), move |ts, c, _| {
            send(ts, "POST", "/api/v1/blobs/check", tok(c).as_deref(), Some(&json!({ "hashes": [] })))
        }),
        ep("PUT blobs/{hash}", Need::AnyVault(Right::Write), move |ts, c, _| {
            let hash = annotary_sidecar::identity::sha256_hex(b"# bytes\n");
            send(ts, "PUTBYTES", &format!("/api/v1/blobs/{hash}"), tok(c).as_deref(), None)
        }),
        ep("GET blobs/{hash}", Need::AnyVault(Right::Read), move |ts, c, _| {
            let (hash, _) = ts.store.put_blob(b"# blob\n").unwrap();
            send(ts, "GET", &format!("/api/v1/blobs/{hash}"), tok(c).as_deref(), None)
        }),
        // documents
        ep("GET docs", Need::Any, move |ts, c, _| send(ts, "GET", "/api/v1/docs", tok(c).as_deref(), None)),
        ep("POST docs", Need::Right(Right::Write), move |ts, c, v| {
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let (hash, _) = ts.store.put_blob(format!("# new {n}\n").as_bytes()).unwrap();
            send(
                ts,
                "POST",
                "/api/v1/docs",
                tok(c).as_deref(),
                Some(&json!({ "vault": v, "name": format!("new{n}"), "md_hash": hash })),
            )
        }),
        ep("GET docs/resolve", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/docs/resolve?doc={id}"), tok(c).as_deref(), None)
        }),
        ep("GET docs/{id}", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/docs/{id}"), tok(c).as_deref(), None)
        }),
        ep("DELETE docs/{id}", Need::Right(Right::Write), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "DELETE", &format!("/api/v1/docs/{id}"), tok(c).as_deref(), None)
        }),
        ep("POST docs/{id}/restore", Need::Right(Right::Write), move |ts, c, v| {
            let id = deleted_doc(ts, v);
            send(ts, "POST", &format!("/api/v1/docs/{id}/restore"), tok(c).as_deref(), None)
        }),
        ep("POST docs/{id}/purge", Need::Owner, move |ts, c, v| {
            let id = deleted_doc(ts, v);
            send(ts, "POST", &format!("/api/v1/docs/{id}/purge"), tok(c).as_deref(), None)
        }),
        ep("GET docs/{id}/versions", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/docs/{id}/versions"), tok(c).as_deref(), None)
        }),
        ep("POST docs/{id}/versions", Need::Right(Right::Write), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let (hash, _) = ts.store.put_blob(format!("# v2 {n}\n").as_bytes()).unwrap();
            send(
                ts,
                "POST",
                &format!("/api/v1/docs/{id}/versions"),
                tok(c).as_deref(),
                Some(&json!({ "name": "m", "md_hash": hash })),
            )
        }),
        ep("GET docs/{id}/versions/1", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/docs/{id}/versions/1"), tok(c).as_deref(), None)
        }),
        ep("GET docs/{id}/status", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/docs/{id}/status"), tok(c).as_deref(), None)
        }),
        ep("GET docs/{id}/sidecar", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/docs/{id}/sidecar"), tok(c).as_deref(), None)
        }),
        // threads
        ep("GET docs/{id}/threads", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/docs/{id}/threads"), tok(c).as_deref(), None)
        }),
        ep("POST docs/{id}/threads", Need::Right(Right::Comment), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(
                ts,
                "POST",
                &format!("/api/v1/docs/{id}/threads"),
                tok(c).as_deref(),
                Some(&json!({ "subject": "document", "body": "hi" })),
            )
        }),
        ep("GET docs/{id}/threads/t1", Need::Right(Right::Read), move |ts, c, v| {
            let id = doc_with_thread(ts, v, &ts.owner_id);
            send(ts, "GET", &format!("/api/v1/docs/{id}/threads/t1"), tok(c).as_deref(), None)
        }),
        ep("POST threads/t1/replies", Need::Right(Right::Comment), move |ts, c, v| {
            let id = doc_with_thread(ts, v, &ts.owner_id);
            send(
                ts,
                "POST",
                &format!("/api/v1/docs/{id}/threads/t1/replies"),
                tok(c).as_deref(),
                Some(&json!({ "body": "re" })),
            )
        }),
        ep("POST threads/t1/resolve", Need::Right(Right::Comment), move |ts, c, v| {
            let id = doc_with_thread(ts, v, &ts.owner_id);
            send(ts, "POST", &format!("/api/v1/docs/{id}/threads/t1/resolve"), tok(c).as_deref(), None)
        }),
        ep("POST threads/t1/reopen", Need::Right(Right::Comment), move |ts, c, v| {
            let id = doc_with_thread(ts, v, &ts.owner_id);
            send(ts, "POST", &format!("/api/v1/docs/{id}/threads/t1/reopen"), tok(c).as_deref(), None)
        }),
        ep("PUT threads/t1 (edit)", Need::CommentAuthor, move |ts, c, v| {
            // The caller authors the Thread when they may comment (else the owner does, and the
            // call fails on the missing Right, not on authorship).
            let author = if c.on_notes.contains(&Right::Comment) && v == "notes"
                || c.on_later.contains(&Right::Comment) && v != "notes"
            {
                c.id.clone()
            } else {
                ts.owner_id.clone()
            };
            let id = doc_with_thread(ts, v, &author);
            send(
                ts,
                "PUT",
                &format!("/api/v1/docs/{id}/threads/t1"),
                tok(c).as_deref(),
                Some(&json!({ "body": "edited" })),
            )
        }),
        ep("DELETE threads/t1", Need::CommentAuthor, move |ts, c, v| {
            let author = if c.on_notes.contains(&Right::Comment) && v == "notes"
                || c.on_later.contains(&Right::Comment) && v != "notes"
            {
                c.id.clone()
            } else {
                ts.owner_id.clone()
            };
            let id = doc_with_thread(ts, v, &author);
            send(ts, "DELETE", &format!("/api/v1/docs/{id}/threads/t1"), tok(c).as_deref(), None)
        }),
        ep("GET mentions", Need::Any, move |ts, c, _| send(ts, "GET", "/api/v1/mentions", tok(c).as_deref(), None)),
        // actions
        ep("GET docs/{id}/actions", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/docs/{id}/actions"), tok(c).as_deref(), None)
        }),
        ep("POST docs/{id}/actions", Need::Right(Right::Action), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(
                ts,
                "POST",
                &format!("/api/v1/docs/{id}/actions"),
                tok(c).as_deref(),
                Some(&json!({ "action": "approve", "version": 1 })),
            )
        }),
        // assets: a Right on some Vault (loopback trust never applies with a bearer present)
        ep("GET /a/{hash}", Need::AnyVault(Right::Read), move |ts, c, _| {
            let (hash, _) = ts.store.put_blob(b"png-ish").unwrap();
            let id = fresh_doc(ts, "notes");
            // Reference the blob from a manifest so it is an Asset (else 404 regardless).
            let n = COUNTER.fetch_add(1, Ordering::SeqCst);
            let (md, _) = ts.store.put_blob(format!("# a {n}\n![](x.png)\n").as_bytes()).unwrap();
            let mut input = PushInput::new(format!("a{n}"), md);
            input.manifest = json!([{ "ref": "x.png", "hash": hash, "size": 7, "type": "image/png" }]);
            ts.store.push_version(&ts.owner_id, &id, input).unwrap();
            send(ts, "GET", &format!("/a/{hash}"), tok(c).as_deref(), None)
        }),
        // <!-- ---- ticket 16 ---- --> every route added after ticket 06 (the sweep): the events
        // log (08), rendered HTML + rerender (09), the web pages and fragments (10/11/12 — HTML
        // 403 pages for a Member, JSON for the fragments), housekeeping + the auto-purge setting
        // (13), the two logouts.
        ep("GET events", Need::Any, move |ts, c, _| send(ts, "GET", "/api/v1/events?since=0", tok(c).as_deref(), None)),
        ep("GET events?doc=", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/events?since=0&doc={id}"), tok(c).as_deref(), None)
        }),
        ep("GET docs/{id}/versions/1/html", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/api/v1/docs/{id}/versions/1/html"), tok(c).as_deref(), None)
        }),
        ep("POST docs/{id}/rerender", Need::Right(Right::Write), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "POST", &format!("/api/v1/docs/{id}/rerender"), tok(c).as_deref(), None)
        }),
        ep("POST rerender (all)", Need::Owner, move |ts, c, _| {
            send(ts, "POST", "/api/v1/rerender", tok(c).as_deref(), None)
        }),
        ep("POST housekeeping", Need::Owner, move |ts, c, _| {
            send(ts, "POST", "/api/v1/housekeeping", tok(c).as_deref(), None)
        }),
        ep("PUT settings/auto-purge", Need::Owner, move |ts, c, _| {
            send(ts, "PUT", "/api/v1/settings/auto-purge", tok(c).as_deref(), Some(&json!({ "days": null })))
        }),
        ep("GET / (list page)", Need::Any, move |ts, c, _| send(ts, "GET", "/", tok(c).as_deref(), None)),
        ep("GET /d/{id} (page)", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/d/{id}"), tok(c).as_deref(), None)
        }),
        ep("GET /d/{id}/v/1 (page)", Need::Right(Right::Read), move |ts, c, v| {
            let id = fresh_doc(ts, v);
            send(ts, "GET", &format!("/d/{id}/v/1"), tok(c).as_deref(), None)
        }),
        ep("GET /d/{id}/t/t1 (fragment)", Need::Right(Right::Read), move |ts, c, v| {
            let id = doc_with_thread(ts, v, &ts.owner_id);
            send(ts, "GET", &format!("/d/{id}/t/t1"), tok(c).as_deref(), None)
        }),
        // CMD-23: the owner pages moved under the Settings screen's Server group; `GET /members`
        // 308s to the first (its redirect is `web_owner.rs`'s — ureq drops the bearer on a
        // redirect, so the matrix calls the target).
        ep("GET /settings/server/members (page)", Need::Owner, move |ts, c, _| {
            send(ts, "GET", "/settings/server/members", tok(c).as_deref(), None)
        }),
        ep("GET /settings/server/vaults (page)", Need::Owner, move |ts, c, _| {
            send(ts, "GET", "/settings/server/vaults", tok(c).as_deref(), None)
        }),
        ep("GET /settings/server/general (page)", Need::Owner, move |ts, c, _| {
            send(ts, "GET", "/settings/server/general", tok(c).as_deref(), None)
        }),
        ep("GET /members/{h} (fragment)", Need::Owner, move |ts, c, _| {
            send(ts, "GET", "/members/rowan", tok(c).as_deref(), None)
        }),
        // CMD-23: the Settings screen is every Member's.
        ep("GET /settings (page)", Need::Any, move |ts, c, _| send(ts, "GET", "/settings", tok(c).as_deref(), None)),
        ep("GET /settings/{group} (page)", Need::Any, move |ts, c, _| {
            send(ts, "GET", "/settings/appearance", tok(c).as_deref(), None)
        }),
        ep("GET /settings/vaults/{v} (fragment)", Need::Owner, move |ts, c, _| {
            send(ts, "GET", "/settings/vaults/notes", tok(c).as_deref(), None)
        }),
        // `POST /logout` (web): with a bearer there is no session to end — a 303 to `/` (followed).
        ep("POST /logout (web)", Need::Any, move |ts, c, _| send(ts, "POST", "/logout", tok(c).as_deref(), None)),
        // `POST /api/v1/logout` revokes the bearer it is called with — a throwaway Token per call,
        // so the caller's own Token survives for the rest of the matrix (a disabled Member cannot
        // mint one; their dead Token is the point there).
        ep("POST logout (api)", Need::Any, move |ts, c, _| {
            let t = if c.disabled {
                c.token.clone()
            } else {
                ts.store.mint_token_expiring(&ts.owner_id, &c.id, Some("throwaway"), None).unwrap().plain
            };
            send(ts, "POST", "/api/v1/logout", Some(&t), None)
        }),
        // <!-- ---- end ticket 16 ---- -->
        // ---- M3b: the icons and the manifest, with the caller's Token (a dead one too) and, in
        // `brand.rs`, with none ----
        ep("GET /favicon.ico", Need::Public, move |ts, c, _| send(ts, "GET", "/favicon.ico", tok(c).as_deref(), None)),
        ep("GET /icon.svg", Need::Public, move |ts, c, _| send(ts, "GET", "/icon.svg", tok(c).as_deref(), None)),
        ep("GET /apple-touch-icon.png", Need::Public, move |ts, c, _| {
            send(ts, "GET", "/apple-touch-icon.png", tok(c).as_deref(), None)
        }),
        ep("GET /manifest.webmanifest", Need::Public, move |ts, c, _| {
            send(ts, "GET", "/manifest.webmanifest", tok(c).as_deref(), None)
        }),
        ep("GET /icon-192.png", Need::Public, move |ts, c, _| {
            send(ts, "GET", "/icon-192.png", tok(c).as_deref(), None)
        }),
        ep("GET /icon-512.png", Need::Public, move |ts, c, _| {
            send(ts, "GET", "/icon-512.png", tok(c).as_deref(), None)
        }),
        ep("GET /icon-mask.png", Need::Public, move |ts, c, _| {
            send(ts, "GET", "/icon-mask.png", tok(c).as_deref(), None)
        }),
        // ---- end M3b ----
    ]
}

/// What the table expects for `caller` on an endpoint scoped to `vault`.
#[derive(Debug, PartialEq, Eq)]
enum Expect {
    Ok,
    Forbidden,
    Unauthorized,
}

fn expected(need: Need, c: &Caller, vault: &str) -> Expect {
    if need == Need::Public {
        return Expect::Ok;
    }
    if c.disabled {
        return Expect::Unauthorized;
    }
    let rights = if vault == "notes" { &c.on_notes } else { &c.on_later };
    let has = |r: Right| match r {
        Right::Read => !rights.is_empty(),
        r => rights.contains(&r),
    };
    let ok = match need {
        Need::Any | Need::Public => true,
        Need::Owner => c.owner,
        Need::Right(r) => has(r),
        Need::CommentAuthor => has(Right::Comment),
        Need::AnyVault(r) => {
            let any = |rs: &Vec<Right>| match r {
                Right::Read => !rs.is_empty(),
                r => rs.contains(&r),
            };
            any(&c.on_notes) || any(&c.on_later)
        }
    };
    if ok { Expect::Ok } else { Expect::Forbidden }
}

// spec: S06-01, S06-02, S06-03, S06-11, S06-14, S06-15, S16-01, S18-08, S29-15
#[test]
fn rights_matrix_every_endpoint_times_every_caller() {
    let ts = TestServer::start();
    let all = Right::ALL.to_vec();
    // The later Vault exists before the Grants below are written — what matters is that no
    // Grant names it: `*` covers it, a `notes` Grant does not.
    ts.create_vault("later");
    let mut callers: Vec<Caller> = Vec::new();
    callers.push(Caller {
        name: "owner",
        handle: "rowan".into(),
        id: ts.owner_id.clone(),
        token: ts.owner_token.clone(),
        on_notes: all.clone(),
        on_later: all.clone(),
        owner: true,
        disabled: false,
    });
    let (id, tok) = member_with(&ts, "star", vec![Grant::all()]);
    callers.push(Caller {
        name: "member *",
        handle: "star".into(),
        id,
        token: tok,
        on_notes: all.clone(),
        on_later: all.clone(),
        owner: false,
        disabled: false,
    });
    let (id, tok) = member_with(&ts, "vaultee", vec![Grant { vault: "notes".into(), rights: all.clone() }]);
    callers.push(Caller {
        name: "member notes:*",
        handle: "vaultee".into(),
        id,
        token: tok,
        on_notes: all.clone(),
        on_later: vec![],
        owner: false,
        disabled: false,
    });
    let (id, tok) = member_with(&ts, "nobody", vec![]);
    callers.push(Caller {
        name: "member none",
        handle: "nobody".into(),
        id,
        token: tok,
        on_notes: vec![],
        on_later: vec![],
        owner: false,
        disabled: false,
    });
    let (id, tok) = member_with(&ts, "gone", vec![Grant::all()]);
    ts.store.disable_member(&ts.owner_id, &id).unwrap();
    callers.push(Caller {
        name: "disabled",
        handle: "gone".into(),
        id,
        token: tok,
        on_notes: all.clone(),
        on_later: all.clone(),
        owner: false,
        disabled: true,
    });
    // Per-Right precision on `notes`: exactly that Right (success), and everything but it (403
    // naming it) — `read` is implied so "all but read" is "nothing".
    for r in Right::ALL {
        let (id, tok) = member_with(&ts, &format!("only-{r}"), vec![Grant { vault: "notes".into(), rights: vec![r] }]);
        callers.push(Caller {
            name: Box::leak(format!("member notes:{r}").into_boxed_str()),
            handle: format!("only-{r}"),
            id,
            token: tok,
            on_notes: vec![r],
            on_later: vec![],
            owner: false,
            disabled: false,
        });
        if r != Right::Read {
            let rest: Vec<Right> = Right::ALL.into_iter().filter(|x| *x != r).collect();
            let (id, tok) =
                member_with(&ts, &format!("all-but-{r}"), vec![Grant { vault: "notes".into(), rights: rest.clone() }]);
            callers.push(Caller {
                name: Box::leak(format!("member notes:all-but-{r}").into_boxed_str()),
                handle: format!("all-but-{r}"),
                id,
                token: tok,
                on_notes: rest,
                on_later: vec![],
                owner: false,
                disabled: false,
            });
        }
    }
    // A `*` Grant restricted on `notes` only: the override rule — comment on notes, everything elsewhere.
    let (id, tok) = member_with(
        &ts,
        "star-but-notes",
        vec![Grant::all(), Grant { vault: "notes".into(), rights: vec![Right::Comment] }],
    );
    callers.push(Caller {
        name: "member *:all + notes:comment",
        handle: "star-but-notes".into(),
        id,
        token: tok,
        on_notes: vec![Right::Comment],
        on_later: all.clone(),
        owner: false,
        disabled: false,
    });

    let mut checked = 0;
    let mut failures = Vec::new();
    for e in endpoints() {
        for vault in ["notes", "later"] {
            // Endpoints without a Vault scope are exercised once.
            if vault == "later" && !matches!(e.need, Need::Right(_) | Need::CommentAuthor) {
                continue;
            }
            for c in &callers {
                let want = expected(e.need, c, vault);
                let (status, body) = (e.call)(&ts, c, vault);
                let got = match status {
                    401 => Expect::Unauthorized,
                    403 => Expect::Forbidden,
                    200..=299 => Expect::Ok,
                    other => {
                        failures.push(format!("{} [{vault}] as {}: HTTP {other} {body}", e.name, c.name));
                        continue;
                    }
                };
                checked += 1;
                if got != want {
                    failures.push(format!(
                        "{} [{vault}] as {}: expected {want:?}, got {got:?} ({status} {body})",
                        e.name, c.name
                    ));
                    continue;
                }
                // The 403 names what is missing (story 29): the Right + a `member grant` hint, or "owner-only".
                // (ticket 16: a page route answers with the HTML 403 page — the same message and
                // hint, escaped into the page text.)
                if status == 403 {
                    let page = body.as_str().unwrap_or("");
                    let msg = body["message"].as_str().unwrap_or(page);
                    let hint = body["hint"].as_str().unwrap_or(page);
                    match e.need {
                        Need::Owner => assert!(msg.contains("owner-only"), "{}: {body}", e.name),
                        Need::Right(r) => {
                            assert!(
                                msg.contains(&format!("missing right `{r}` on Vault {vault} (you are @")),
                                "{}: {body}",
                                e.name
                            );
                            assert!(
                                hint.contains(&format!("annotary member grant {} {vault} {r}", c.handle)),
                                "{}: {body}",
                                e.name
                            );
                        }
                        Need::CommentAuthor => assert!(
                            msg.contains("missing right `comment`") || msg.contains("only the author"),
                            "{}: {body}",
                            e.name
                        ),
                        Need::AnyVault(r) => assert!(msg.contains(&format!("`{r}`")), "{}: {body}", e.name),
                        Need::Any | Need::Public => unreachable!(),
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
    assert!(checked > 600, "matrix ran ({checked} checks)");
    // Neither cookie nor bearer ⇒ 401 on the API — the credential-less loopback CLI/curl never
    // becomes the owner (S06-11).
    let (status, body) = send(&ts, "GET", "/api/v1/docs", None, None);
    assert_eq!(status, 401, "{body}");
    let (status, _) =
        send(&ts, "POST", "/api/v1/docs", None, Some(&json!({ "vault": "notes", "name": "x", "md_hash": "0" })));
    assert_eq!(status, 401);
    // ticket 18: pairing without a credential mints nothing.
    let (status, _) = send(&ts, "POST", "/api/v1/pairing", None, Some(&json!({})));
    assert_eq!(status, 401);
}

// spec: S06-02
#[test]
fn a_star_grant_covers_a_vault_created_later_and_a_vault_grant_does_not() {
    let ts = TestServer::start();
    let (_, star) = member_with(&ts, "star", vec![Grant::all()]);
    let (_, notes_only) =
        member_with(&ts, "notes-only", vec![Grant { vault: "notes".into(), rights: Right::ALL.to_vec() }]);
    // Created *after* the Grants were written.
    let (status, v) = send(&ts, "POST", "/api/v1/vaults", Some(&ts.owner_token), Some(&json!({ "name": "drafts" })));
    assert_eq!(status, 201, "{v}");
    assert_eq!(v["name"], "drafts");
    let (status, _) = send(&ts, "POST", "/api/v1/vaults", Some(&ts.owner_token), Some(&json!({ "name": "drafts" })));
    assert_eq!(status, 409, "duplicate Vault name");
    let (status, _) = send(&ts, "POST", "/api/v1/vaults", Some(&star), Some(&json!({ "name": "nope" })));
    assert_eq!(status, 403, "Vault create is owner-only");
    let id = fresh_doc(&ts, "drafts");
    let (status, _) = send(&ts, "GET", &format!("/api/v1/docs/{id}"), Some(&star), None);
    assert_eq!(status, 200);
    let (status, body) = send(&ts, "GET", &format!("/api/v1/docs/{id}"), Some(&notes_only), None);
    assert_eq!(status, 403, "{body}");
    assert!(body["message"].as_str().unwrap().contains("missing right `read` on Vault drafts"));
    // The `vaults` list is scoped by `read`.
    let (_, vs) = send(&ts, "GET", "/api/v1/vaults", Some(&notes_only), None);
    assert_eq!(vs["vaults"].as_array().unwrap().len(), 1);
    let (_, vs) = send(&ts, "GET", "/api/v1/vaults", Some(&star), None);
    assert_eq!(vs["vaults"].as_array().unwrap().len(), 2);
    // `member grant` over HTTP: upsert one Vault, `*` removable, unknown Vault / right typed.
    let (status, m) = send(
        &ts,
        "POST",
        "/api/v1/members/star/grants",
        Some(&ts.owner_token),
        Some(&json!({ "vault": "drafts", "rights": ["comment"] })),
    );
    assert_eq!(status, 200, "{m}");
    assert_eq!(
        m["grants"],
        json!([{ "vault": "*", "rights": ["read", "comment", "action", "write"] }, { "vault": "drafts", "rights": ["comment"] }])
    );
    let (status, body) = send(&ts, "GET", &format!("/api/v1/docs/{id}"), Some(&star), None);
    assert_eq!(status, 200, "read implied by comment: {body}");
    let (status, body) = send(&ts, "DELETE", &format!("/api/v1/docs/{id}"), Some(&star), None);
    assert_eq!(status, 403, "the drafts Grant overrides * there: {body}");
    let (status, body) = send(
        &ts,
        "POST",
        "/api/v1/members/star/grants",
        Some(&ts.owner_token),
        Some(&json!({ "vault": "zzz", "rights": ["read"] })),
    );
    assert_eq!(status, 404, "{body}");
    let (status, body) = send(
        &ts,
        "POST",
        "/api/v1/members/star/grants",
        Some(&ts.owner_token),
        Some(&json!({ "vault": "notes", "rights": ["admin"] })),
    );
    assert_eq!(status, 400, "{body}");
    let (status, m) = send(
        &ts,
        "POST",
        "/api/v1/members/star/grants",
        Some(&ts.owner_token),
        Some(&json!({ "vault": "*", "rights": [] })),
    );
    assert_eq!(status, 200, "{m}");
    assert_eq!(m["grants"], json!([{ "vault": "drafts", "rights": ["comment"] }]));
    let (status, body) = send(
        &ts,
        "PUT",
        "/api/v1/members/star/grants",
        Some(&ts.owner_token),
        Some(&json!({ "grants": [{ "vault": "*", "rights": [] }] })),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "invalid_grant");
}

// spec: S06-03, S06-04, S06-05
#[test]
fn owner_only_member_management_last_owner_disable_rename_and_handle_reuse() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (linter_id, linter) = ts.add_member("linter");
    // Every management call is owner-only for a member — even on themselves.
    for (m, path, body) in [
        ("POST", "/api/v1/members/linter/rename", json!({ "name": "Me" })),
        ("POST", "/api/v1/members/linter/disable", json!({})),
        ("POST", "/api/v1/members/linter/role", json!({ "role": "owner" })),
        ("POST", "/api/v1/members/linter/grants", json!({ "vault": "notes", "rights": ["read"] })),
        ("PUT", "/api/v1/settings/password", json!({ "password": "x" })),
        ("PUT", "/api/v1/vaults/notes/actions", json!({ "available": [] })),
    ] {
        let (status, b) = send(&ts, m, path, Some(&linter), Some(&body));
        assert_eq!(status, 403, "{m} {path}: {b}");
        assert!(b["message"].as_str().unwrap().contains("owner-only"), "{b}");
    }
    // Rename: display name only; the handle and mentions stay.
    let (status, m) =
        send(&ts, "POST", "/api/v1/members/linter/rename", Some(&owner), Some(&json!({ "name": "Linter CLI" })));
    assert_eq!(status, 200, "{m}");
    assert_eq!((m["handle"].as_str().unwrap(), m["name"].as_str().unwrap()), ("linter", "Linter CLI"));
    let id = fresh_doc(&ts, "notes");
    let (_, t) = send(
        &ts,
        "POST",
        &format!("/api/v1/docs/{id}/threads"),
        Some(&owner),
        Some(&json!({ "subject": "document", "body": "hey @linter" })),
    );
    assert_eq!(t["mentions"], json!(["linter"]));
    let (status, _) = send(&ts, "POST", "/api/v1/members/linter/rename", Some(&owner), Some(&json!({ "name": "  " })));
    assert_eq!(status, 400);
    let (status, _) = send(&ts, "POST", "/api/v1/members/nobody/rename", Some(&owner), Some(&json!({ "name": "x" })));
    assert_eq!(status, 404);
    // The last owner cannot be demoted or disabled.
    let (status, b) = send(&ts, "POST", "/api/v1/members/rowan/role", Some(&owner), Some(&json!({ "role": "member" })));
    assert_eq!(status, 409, "{b}");
    assert_eq!(b["error"], "last_owner");
    let (status, b) = send(&ts, "POST", "/api/v1/members/rowan/disable", Some(&owner), None);
    assert_eq!(status, 409, "{b}");
    assert_eq!(b["error"], "last_owner");
    // Promote linter; now rowan may step down and linter is the last owner.
    let (status, m) = send(&ts, "POST", "/api/v1/members/linter/role", Some(&owner), Some(&json!({ "role": "owner" })));
    assert_eq!(status, 200, "{m}");
    assert_eq!(m["role"], "owner");
    assert_eq!(m["changed"], true);
    let (_, m) = send(&ts, "POST", "/api/v1/members/linter/role", Some(&owner), Some(&json!({ "role": "owner" })));
    assert_eq!(m["changed"], false, "idempotent");
    let (status, _) = send(&ts, "POST", "/api/v1/members/linter/role", Some(&owner), Some(&json!({ "role": "admin" })));
    assert_eq!(status, 400);
    let (status, m) =
        send(&ts, "POST", "/api/v1/members/rowan/role", Some(&linter), Some(&json!({ "role": "member" })));
    assert_eq!(status, 200, "{m}");
    assert_eq!(m["role"], "member");
    let (status, _) =
        send(&ts, "POST", "/api/v1/members/linter/role", Some(&linter), Some(&json!({ "role": "member" })));
    assert_eq!(status, 409, "linter is now the last owner");
    // Disable rowan (a member now): 401 for their Token, history intact, handle never reused,
    // idempotent, and a disabled Member cannot be granted / promoted / given a token.
    let (status, m) = send(&ts, "POST", "/api/v1/members/rowan/disable", Some(&linter), None);
    assert_eq!(status, 200, "{m}");
    assert_eq!(m["changed"], true);
    assert!(!m["disabled_at"].is_null());
    let (status, _) = send(&ts, "GET", "/api/v1/me", Some(&owner), None);
    assert_eq!(status, 401);
    let (_, m) = send(&ts, "POST", "/api/v1/members/rowan/disable", Some(&linter), None);
    assert_eq!(m["changed"], false);
    let (_, list) = send(&ts, "GET", "/api/v1/members", Some(&linter), None);
    assert!(list["members"].as_array().unwrap().iter().any(|m| m["handle"] == "rowan"), "kept in the list");
    let (_, t1) = send(&ts, "GET", &format!("/api/v1/docs/{id}/threads/t1"), Some(&linter), None);
    assert_eq!(t1["author"], "rowan", "attribution stays");
    let (status, _) = send(&ts, "POST", "/api/v1/members", Some(&linter), Some(&json!({ "handle": "rowan" })));
    assert_eq!(status, 409, "handle reuse after disable rejected");
    for (path, body) in [
        ("/api/v1/members/rowan/role", json!({ "role": "owner" })),
        ("/api/v1/members/rowan/grants", json!({ "vault": "notes", "rights": ["read"] })),
        ("/api/v1/tokens", json!({ "member": "rowan" })),
    ] {
        let (status, b) = send(&ts, "POST", path, Some(&linter), Some(&body));
        assert_eq!(status, 409, "{path}: {b}");
        assert_eq!(b["error"], "disabled");
    }
    let _ = linter_id;
}

// spec: S06-07, S06-08, S06-13
#[test]
fn tokens_mint_list_revoke_expire_and_belong_to_the_member() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, linter) = ts.add_member("linter");
    // A Member mints their own (shown once), sees only their own, revokes their own.
    let (status, t) = send(&ts, "POST", "/api/v1/tokens", Some(&linter), Some(&json!({ "name": "laptop" })));
    assert_eq!(status, 201, "{t}");
    let plain = t["token"].as_str().unwrap().to_string();
    assert!(plain.starts_with("ant_"));
    assert_eq!(t["member"], "linter");
    assert_eq!(t["name"], "laptop");
    assert_eq!(t["status"], "active");
    let tid = t["id"].as_str().unwrap().to_string();
    let (status, me) = send(&ts, "GET", "/api/v1/me", Some(&plain), None);
    assert_eq!(status, 200, "{me}");
    let (_, list) = send(&ts, "GET", "/api/v1/tokens", Some(&linter), None);
    let toks = list["tokens"].as_array().unwrap();
    assert_eq!(toks.len(), 2, "member add + laptop");
    assert!(toks.iter().all(|t| t["member"] == "linter" && t.get("token").is_none() && t.get("hash").is_none()));
    let used = toks.iter().find(|t| t["id"] == tid).unwrap();
    assert!(!used["last_used_at"].is_null(), "last_used_at stamped by the /me call");
    // The owner sees everyone's, one Member's, and may mint/revoke for anyone.
    let (_, all) = send(&ts, "GET", "/api/v1/tokens", Some(&owner), None);
    assert_eq!(all["tokens"].as_array().unwrap().len(), 3);
    let (_, one) = send(&ts, "GET", "/api/v1/tokens?member=linter", Some(&owner), None);
    assert_eq!(one["tokens"].as_array().unwrap().len(), 2);
    let (status, _) = send(&ts, "GET", "/api/v1/tokens?member=rowan", Some(&linter), None);
    assert_eq!(status, 403);
    let (status, _) = send(&ts, "GET", "/api/v1/tokens?member=*", Some(&linter), None);
    assert_eq!(status, 403);
    let (status, _) = send(&ts, "GET", "/api/v1/tokens?member=ghost", Some(&owner), None);
    assert_eq!(status, 404);
    // Expiry: a past date is refused; a future one is stored; expired ⇒ 401 + `expired`.
    let (status, b) = send(
        &ts,
        "POST",
        "/api/v1/tokens",
        Some(&owner),
        Some(&json!({ "member": "linter", "expires_at": "2020-01-01T00:00:00Z" })),
    );
    assert_eq!(status, 400, "{b}");
    let (status, b) = send(
        &ts,
        "POST",
        "/api/v1/tokens",
        Some(&owner),
        Some(&json!({ "member": "linter", "expires_at": "tomorrow" })),
    );
    assert_eq!(status, 400, "{b}");
    let (status, exp) = send(
        &ts,
        "POST",
        "/api/v1/tokens",
        Some(&owner),
        Some(&json!({ "member": "linter", "name": "ci", "expires_at": "2999-01-01T00:00:00Z" })),
    );
    assert_eq!(status, 201, "{exp}");
    assert_eq!(exp["expires_at"], "2999-01-01T00:00:00.000Z");
    let exp_plain = exp["token"].as_str().unwrap().to_string();
    let exp_id = exp["id"].as_str().unwrap().to_string();
    assert_eq!(send(&ts, "GET", "/api/v1/me", Some(&exp_plain), None).0, 200);
    let conn = rusqlite::Connection::open(ts.data_dir.join("store/annotary.sqlite")).unwrap();
    conn.execute("UPDATE tokens SET expires_at = '2020-01-01T00:00:00.000Z' WHERE id = ?1", [&exp_id]).unwrap();
    drop(conn);
    assert_eq!(send(&ts, "GET", "/api/v1/me", Some(&exp_plain), None).0, 401);
    let (_, one) = send(&ts, "GET", "/api/v1/tokens?member=linter", Some(&owner), None);
    assert_eq!(one["tokens"].as_array().unwrap().iter().find(|t| t["id"] == exp_id).unwrap()["status"], "expired");
    // Revoke: 401 afterwards, idempotent, only own or owner, unknown 404.
    let (status, r) = send(&ts, "DELETE", &format!("/api/v1/tokens/{tid}"), Some(&linter), None);
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["revoked"], true);
    assert_eq!(send(&ts, "GET", "/api/v1/me", Some(&plain), None).0, 401);
    let (_, r) = send(&ts, "DELETE", &format!("/api/v1/tokens/{tid}"), Some(&linter), None);
    assert_eq!(r["revoked"], false);
    assert_eq!(r["status"], "revoked");
    let owner_tid = send(&ts, "GET", "/api/v1/tokens?member=rowan", Some(&owner), None).1["tokens"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let (status, _) = send(&ts, "DELETE", &format!("/api/v1/tokens/{owner_tid}"), Some(&linter), None);
    assert_eq!(status, 403);
    let (status, _) = send(&ts, "DELETE", "/api/v1/tokens/nope", Some(&owner), None);
    assert_eq!(status, 404);
    // `logout` with a bearer revokes that very Token.
    let (status, r) = send(&ts, "POST", "/api/v1/logout", Some(&exp_plain), None);
    assert_eq!(status, 401, "expired already: {r}");
    let (_, fresh) = send(&ts, "POST", "/api/v1/tokens", Some(&linter), None);
    let fresh_plain = fresh["token"].as_str().unwrap().to_string();
    let (status, r) = send(&ts, "POST", "/api/v1/logout", Some(&fresh_plain), None);
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["ended"], "token");
    assert_eq!(send(&ts, "GET", "/api/v1/me", Some(&fresh_plain), None).0, 401);
    // `GET members/{h}` carries `tokens[]` for the owner / self only.
    let (_, m) = send(&ts, "GET", "/api/v1/members/linter", Some(&owner), None);
    assert!(m["tokens"].as_array().is_some_and(|t| t.len() == 4));
    let (_, m) = send(&ts, "GET", "/api/v1/members/linter", Some(&linter), None);
    assert!(m["tokens"].is_array());
    let (_, other) = ts.add_member("other");
    let (_, m) = send(&ts, "GET", "/api/v1/members/linter", Some(&other), None);
    assert!(m.get("tokens").is_none());
}

// spec: S06-09, S06-11, S06-12
#[test]
fn local_password_and_login_exchange_it_for_a_token() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, linter) = ts.add_member("linter");
    // No password: health says so and login is a typed refusal — never a silent owner.
    let (_, h) = send(&ts, "GET", "/api/v1/health", None, None);
    assert_eq!(h["password_set"], false);
    let (status, b) = send(&ts, "POST", "/api/v1/login", None, Some(&json!({ "password": "anything" })));
    assert_eq!(status, 409, "{b}");
    assert_eq!(b["error"], "no_password");
    assert!(b["hint"].as_str().unwrap().contains("member add"));
    // Setting it is owner-only and sets the caller's own; argon2id at rest; never in events.
    let (status, _) = send(&ts, "PUT", "/api/v1/settings/password", Some(&linter), Some(&json!({ "password": "x" })));
    assert_eq!(status, 403);
    let (status, b) =
        send(&ts, "PUT", "/api/v1/settings/password", Some(&owner), Some(&json!({ "password": "hunter2" })));
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["password_set"], true);
    let conn = rusqlite::Connection::open(ts.data_dir.join("store/annotary.sqlite")).unwrap();
    let phc: String =
        conn.query_row("SELECT password_hash FROM members WHERE id = ?1", [&ts.owner_id], |r| r.get(0)).unwrap();
    assert!(phc.starts_with("$argon2id$"));
    let payloads: Vec<String> = conn
        .prepare("SELECT payload FROM events")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(payloads.iter().all(|p| !p.contains("hunter2") && !p.contains("argon2")));
    drop(conn);
    let (_, s) = send(&ts, "GET", "/api/v1/settings", Some(&owner), None);
    assert_eq!(s["password_set"], true);
    let (_, h) = send(&ts, "GET", "/api/v1/health", None, None);
    assert_eq!(h["password_set"], true);
    // Login: wrong ⇒ 401 wrong_password; right ⇒ a Token of the owner, shown once, that works.
    let (status, b) = send(&ts, "POST", "/api/v1/login", None, Some(&json!({ "password": "hunter3" })));
    assert_eq!(status, 401, "{b}");
    assert_eq!(b["error"], "wrong_password");
    let (status, b) = send(&ts, "POST", "/api/v1/login", None, Some(&json!({ "password": "hunter2" })));
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["handle"], "rowan");
    assert_eq!(b["role"], "owner");
    let plain = b["token"].as_str().unwrap().to_string();
    let (status, me) = send(&ts, "GET", "/api/v1/me", Some(&plain), None);
    assert_eq!(status, 200);
    assert_eq!(me["handle"], "rowan");
    let (_, toks) = send(&ts, "GET", "/api/v1/tokens?member=rowan", Some(&owner), None);
    assert!(toks["tokens"].as_array().unwrap().iter().any(|t| t["name"] == "login"));
    // Change, then remove: the old password stops working; removing brings loopback trust back.
    send(&ts, "PUT", "/api/v1/settings/password", Some(&owner), Some(&json!({ "password": "changed" })));
    assert_eq!(send(&ts, "POST", "/api/v1/login", None, Some(&json!({ "password": "hunter2" }))).0, 401);
    assert_eq!(send(&ts, "POST", "/api/v1/login", None, Some(&json!({ "password": "changed" }))).0, 200);
    let (status, b) = send(&ts, "PUT", "/api/v1/settings/password", Some(&owner), Some(&json!({ "password": null })));
    assert_eq!(status, 200, "{b}");
    assert_eq!(b["password_set"], false);
    assert_eq!(send(&ts, "POST", "/api/v1/login", None, Some(&json!({ "password": "changed" }))).0, 409);
    // The server never writes a profile.
    assert!(!ts.data_dir.join("config.toml").exists());
}

// spec: S06-10, S06-11
#[test]
fn browser_sessions_loopback_trust_iff_no_password_login_page_and_logout() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let root = format!("{}/", ts.url);
    // No password: a loopback page request without credentials is the owner and gets a cookie.
    let (status, hs, body) = raw_get(&root, &[("Accept", "text/html")]);
    assert_eq!(status, 200, "{body}");
    let cookie = cookie_of(&hs).expect("session cookie issued");
    assert!(cookie.starts_with("cd_session=cds_"), "{cookie}");
    assert!(header(&hs, "set-cookie").unwrap().contains("HttpOnly"));
    assert!(body.contains("@rowan"), "{body}");
    // The cookie is a credential for the API too (browser JS), remembered across requests.
    let mut r = ureq::get(format!("{}/api/v1/me", ts.url))
        .header("Cookie", &cookie)
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let me: Value = serde_json::from_str(&r.body_mut().read_to_string().unwrap()).unwrap();
    assert_eq!(me["handle"], "rowan");
    // …but the API never issues one: neither cookie nor bearer ⇒ 401 (the CLI never becomes owner).
    let (status, _) = send(&ts, "GET", "/api/v1/me", None, None);
    assert_eq!(status, 401);
    let (status, _) = send(&ts, "GET", "/api/v1/docs", None, None);
    assert_eq!(status, 401);
    // The login page while no password is set: signs a loopback browser in and sends it on.
    let (status, hs, _) = raw_get(&format!("{}/login?next=/", ts.url), &[("Accept", "text/html")]);
    assert_eq!(status, 303);
    assert_eq!(header(&hs, "location"), Some("/"));
    assert!(cookie_of(&hs).is_some());
    // A dead cookie is cleared and (no password) replaced by a fresh loopback session.
    let (status, hs, _) = raw_get(&root, &[("Accept", "text/html"), ("Cookie", "cd_session=cds_deadbeefdeadbeef")]);
    assert_eq!(status, 303, "dead cookie ⇒ start over via /login");
    assert!(header(&hs, "set-cookie").unwrap().contains("Max-Age=0"));

    // Password set ⇒ login required: page ⇒ 303 to /login?next=…; non-page ⇒ 401 JSON.
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some("hunter2")).unwrap();
    let (status, hs, _) = raw_get(&root, &[("Accept", "text/html")]);
    assert_eq!(status, 303);
    assert_eq!(header(&hs, "location"), Some("/login?next=%2F"));
    assert!(cookie_of(&hs).is_none(), "no cookie issued once a password is set");
    let (status, _, body) = raw_get(&root, &[("Accept", "*/*")]);
    assert_eq!(status, 401, "{body}");
    let (status, _, body) = raw_get(&format!("{}/login", ts.url), &[("Accept", "text/html")]);
    assert_eq!(status, 200);
    assert!(body.contains("<form") && body.contains("name=\"password\""), "{body}");
    // The earlier (loopback) cookie still works — sessions are remembered.
    let (status, _, body) = raw_get(&root, &[("Accept", "text/html"), ("Cookie", &cookie)]);
    assert_eq!(status, 200, "{body}");
    assert!(body.contains("Sign out"));
    // Wrong password ⇒ 401 + the form again; right ⇒ cookie + redirect to `next` (same-origin only).
    let (status, _, body) = raw_post_form(&format!("{}/login", ts.url), "password=nope&next=%2F", &[]);
    assert_eq!(status, 401);
    assert!(body.contains("Wrong password"));
    let (status, hs, _) = raw_post_form(&format!("{}/login", ts.url), "password=hunter2&next=%2Fa%2Fx", &[]);
    assert_eq!(status, 303);
    assert_eq!(header(&hs, "location"), Some("/a/x"));
    let session = cookie_of(&hs).expect("cookie from the login page");
    let (status, hs, _) =
        raw_post_form(&format!("{}/login", ts.url), "password=hunter2&next=https%3A%2F%2Fevil.example", &[]);
    assert_eq!(status, 303);
    assert_eq!(header(&hs, "location"), Some("/"), "open redirect refused");
    let (status, _, body) = raw_get(&root, &[("Accept", "text/html"), ("Cookie", &session)]);
    assert_eq!(status, 200, "{body}");
    // A bearer works on web routes too (bearer wins over the cookie).
    let (status, _, _) = raw_get(&root, &[("Accept", "text/html"), ("Authorization", &format!("Bearer {owner}"))]);
    assert_eq!(status, 200);
    // Logout ends the session and clears the cookie; the cookie is dead afterwards.
    let (status, hs, _) = raw_post_form(&format!("{}/logout", ts.url), "", &[("Cookie", &session)]);
    assert_eq!(status, 303);
    assert_eq!(header(&hs, "location"), Some("/login"));
    assert!(header(&hs, "set-cookie").unwrap().contains("Max-Age=0"));
    let (status, _, _) = raw_get(&root, &[("Accept", "text/html"), ("Cookie", &session)]);
    assert_eq!(status, 303);
    // `POST /api/v1/logout` with a cookie ends that session too.
    let (_, hs, _) = raw_post_form(&format!("{}/login", ts.url), "password=hunter2", &[]);
    let s2 = cookie_of(&hs).unwrap();
    let mut r = ureq::post(format!("{}/api/v1/logout", ts.url))
        .header("Cookie", &s2)
        .config()
        .http_status_as_error(false)
        .build()
        .send_json(json!({}))
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let b: Value = serde_json::from_str(&r.body_mut().read_to_string().unwrap()).unwrap();
    assert_eq!(b["ended"], "session");
    let r = ureq::get(format!("{}/api/v1/me", ts.url))
        .header("Cookie", &s2)
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    // A disabled Member's session dies with them.
    let (cid, _) = ts.add_member("linter");
    let s = ts.store.open_session(&cid, "login").unwrap();
    let c = format!("cd_session={}", s.plain);
    let (status, _, _) = raw_get(&root, &[("Accept", "text/html"), ("Cookie", &c)]);
    assert_eq!(status, 200);
    ts.store.disable_member(&ts.owner_id, &cid).unwrap();
    let (status, _, _) = raw_get(&root, &[("Accept", "text/html"), ("Cookie", &c)]);
    assert_eq!(status, 303);
    // `/a/{hash}` from an `<img>` (no text/html): 401 without a cookie, 200 with one.
    let (hash, _) = ts.store.put_blob(b"png-ish").unwrap();
    let id = fresh_doc(&ts, "notes");
    let (md, _) = ts.store.put_blob(b"# a\n![](x.png)\n").unwrap();
    let mut input = PushInput::new("a", md);
    input.manifest = json!([{ "ref": "x.png", "hash": hash, "size": 7, "type": "image/png" }]);
    ts.store.push_version(&ts.owner_id, &id, input).unwrap();
    let (status, _, _) = raw_get(&format!("{}/a/{hash}", ts.url), &[("Accept", "image/avif,image/webp,*/*")]);
    assert_eq!(status, 401);
    let (_, hs, _) = raw_post_form(&format!("{}/login", ts.url), "password=hunter2", &[]);
    let s3 = cookie_of(&hs).unwrap();
    let (status, _, _) =
        raw_get(&format!("{}/a/{hash}", ts.url), &[("Accept", "image/avif,image/webp,*/*"), ("Cookie", &s3)]);
    assert_eq!(status, 200);
}
