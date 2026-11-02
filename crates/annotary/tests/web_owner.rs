//! Ticket 12 over HTTP against the in-process server: the owner screens, since CMD-23 the Server
//! group of the Settings screen (`/settings/server/members|vaults|general`; server-composed; what
//! they show; owner-only — a Member gets the 403 page and no Server group), their card fragments, the Vault rename / delete API (Grants follow a rename and are
//! dropped on delete; non-empty and last Vault refused), and the restyled login page on the shared
//! shell. The browser-side flows (Add agent → a Token the CLI accepts, password → login page →
//! session, the default Actions editor → `pull`, auto-purge, hidden nav) are `tests/web/owner.spec.ts`.

use annotary::testing::TestServer;
use annotary_store::{Grant, Right};
use serde_json::{Value, json};

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
        "PUT" => {
            let mut r = ureq::put(&url);
            if let Some(a) = &auth {
                r = r.header("Authorization", a);
            }
            r.config().http_status_as_error(false).build().send_json(body.cloned().unwrap_or(json!({})))
        }
        other => panic!("method {other}"),
    };
    match res {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let text = r.body_mut().read_to_string().unwrap_or_default();
            (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
        }
        Err(e) => panic!("{e}"),
    }
}

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

fn create_doc(ts: &TestServer, token: &str, vault: &str, name: &str, md: &str) -> String {
    let hash = annotary_sidecar::identity::sha256_hex(md.as_bytes());
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(md.as_bytes())
        .unwrap();
    assert!(res.status().is_success());
    let (status, doc) = ts.post("/api/v1/docs", Some(token), &json!({ "vault": vault, "name": name, "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

// spec: S12-01, S12-02, S12-05
#[test]
fn members_screen_shows_handle_name_role_grants_auth_hint_and_tokens_with_last_used() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    ts.create_vault("drafts");
    let (_, agent) = ts.add_member("scribe");
    ts.store.rename_member(&ts.owner_id, &ts.store.member_by_handle("scribe").unwrap().unwrap().id, "Scribe").unwrap();
    // The agent's Token gets used once → `last_used_at`; a second, named Token with an expiry.
    assert_eq!(send(&ts, "GET", "/api/v1/me", Some(&agent), None).0, 200);
    let (status, t2) = send(
        &ts,
        "POST",
        "/api/v1/tokens",
        Some(&owner),
        Some(&json!({ "member": "scribe", "name": "laptop", "expires_at": "2030-01-01T00:00:00Z" })),
    );
    assert_eq!(status, 201, "{t2}");
    // A Grant narrowed on one Vault and a disabled Member.
    let linter_id = ts.add_member("linter").0;
    ts.store.grant(&ts.owner_id, &linter_id, "drafts", &[Right::Read, Right::Comment]).unwrap();
    ts.store.grant(&ts.owner_id, &linter_id, "*", &[]).unwrap();
    let old_id = ts.add_member("old-bot").0;
    ts.store.disable_member(&ts.owner_id, &old_id).unwrap();

    let (status, html) = page(&ts, "/settings/server/members", &owner);
    assert_eq!(status, 200);
    // The shared shell, Settings current in the top bar, Members current in the sidebar's Server
    // group (CMD-23 moved the page there).
    assert!(html.contains("<title>Members — annotary</title>"), "{html}");
    assert!(html.contains("id=\"cd-nav-settings\" href=\"/settings\" aria-current=\"page\""));
    assert!(html.contains(
        "<a class=\"cd-set-gl cd-current\" href=\"/settings/server/members\" data-group=\"server/members\" aria-current=\"page\">"
    ));
    assert!(html.contains("id=\"cd-addagent\""));
    // Every Member: display name, handle, role chip, auth hint (never "bot"), Grants summary.
    assert!(html.contains("<b class=\"cd-member\" title=\"@rowan\">Rowan Vale</b>"));
    assert!(html.contains("<b class=\"cd-member\" title=\"@scribe\">Scribe</b>"));
    assert!(html.contains("<span class=\"cd-chip cd-role cd-role-owner\">owner</span>"));
    assert!(html.contains("<span class=\"cd-chip cd-role cd-role-member\">member</span>"));
    assert!(html.contains("authenticates with a Token") && html.contains("cd-auth-token"));
    assert!(!html.to_lowercase().contains("bot badge") && !html.contains("class=\"cd-bot"));
    let linter_card = html.split("data-handle=\"linter\"").nth(1).unwrap().split("</article>").next().unwrap();
    let linter_summary =
        linter_card.split("cd-grants-summary\">").nth(1).unwrap().split("</span></div>").next().unwrap();
    assert_eq!(linter_summary, "<span class=\"cd-grant\"><b>drafts</b>: read, comment</span>", "{linter_card}");
    let old_card = html.split("data-handle=\"old-bot\"").nth(1).unwrap().split("</article>").next().unwrap();
    assert!(old_card.contains("cd-disabled") && !old_card.contains("cd-advanced"));
    // Tokens with `last_used_at`, names, expiry, ids; the role / Grants editor behind Advanced.
    let cc = html.split("data-handle=\"scribe\"").nth(1).unwrap().split("</article>").next().unwrap();
    assert!(cc.contains("<details class=\"cd-advanced\">"), "{cc}");
    assert_eq!(cc.matches("<tr class=\"cd-tok\"").count(), 2, "{cc}");
    assert!(cc.contains("<td class=\"cd-lastused\"><time class=\"cd-rel\""), "last_used_at of the used Token: {cc}");
    assert!(cc.contains("<td class=\"cd-lastused\"><span class=\"cd-muted\">never</span>"), "{cc}");
    assert!(cc.contains("<td class=\"cd-tokname\">laptop</td>"));
    assert!(cc.contains(&format!("data-token=\"{}\"", t2["id"].as_str().unwrap())));
    assert!(cc.contains("datetime=\"2030-01-01T00:00:00.000Z\""));
    assert!(cc.contains("data-right=\"write\" checked") && cc.contains("data-vault=\"drafts\""));
    assert!(!cc.contains("ant_"), "plain Tokens never appear on the page");
    // The fragment is the very card the page carries (one renderer).
    let (status, frag) = send(&ts, "GET", "/members/scribe", Some(&owner), None);
    assert_eq!(status, 200, "{frag}");
    assert_eq!(frag["handle"], "scribe");
    let start = html.find("<article class=\"cd-mem\" id=\"cd-mem-scribe\"").unwrap();
    let end = html[start..].find("</article>").unwrap() + start + "</article>".len();
    assert_eq!(frag["html"].as_str().unwrap(), &html[start..end]);
    assert_eq!(send(&ts, "GET", "/members/nobody", Some(&owner), None).0, 404);
}

// spec: S12-03, S12-05
#[test]
fn settings_screen_shows_password_state_vaults_default_actions_and_auto_purge() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    ts.create_vault("drafts");
    create_doc(&ts, &owner, "drafts", "plan", "# Plan\n");
    // CMD-23: today's owner Settings is two pages of the Server group, Vaults and Server settings.
    let (status, html) = page(&ts, "/settings/server/general", &owner);
    assert_eq!(status, 200);
    assert!(html.contains("<title>Server settings — annotary</title>"));
    assert!(html.contains("id=\"cd-nav-settings\" href=\"/settings\" aria-current=\"page\""));
    assert!(html.contains("data-group=\"server/general\" aria-current=\"page\""));
    assert!(!html.contains("id=\"cd-vaults\""), "the Vaults have their own page");
    // Password: none set yet — the state says so; the form sets one.
    assert!(html.contains("data-password-set=\"false\""));
    assert!(html.contains("No local password is set"));
    assert!(html.contains("id=\"cd-pwform\"") && html.contains("name=\"password\""));
    assert!(!html.contains("id=\"cd-pw-remove\""));
    // Auto-purge off by default.
    assert!(html.contains("id=\"cd-set-purge\" data-days=\"0\""));
    assert!(html.contains("id=\"cd-purge-on\">") && !html.contains("id=\"cd-purge-on\" checked"));
    // Vaults with counts and the default Actions rows (id / label / intent), create form.
    let (status, html) = page(&ts, "/settings/server/vaults", &owner);
    assert_eq!(status, 200);
    assert!(html.contains("<title>Vaults — annotary</title>"));
    assert!(html.contains("data-group=\"server/vaults\" aria-current=\"page\""));
    assert!(!html.contains("id=\"cd-pwform\""), "the password is on Server settings");
    assert!(html.contains("data-vault=\"notes\"") && html.contains("data-vault=\"drafts\""));
    let drafts = html.split("data-vault=\"drafts\"").nth(1).unwrap().split("</article>").next().unwrap();
    assert!(drafts.contains("1 Document"), "{drafts}");
    assert!(drafts.contains("value=\"approve\"") && drafts.contains("value=\"Approve\""));
    assert!(drafts.contains("<option value=\"positive\" selected>"));
    assert!(drafts.contains("value=\"send-back\"") && drafts.contains("<option value=\"neutral\" selected>"));
    assert!(drafts.contains("data-act=\"save-actions\"") && drafts.contains("data-act=\"addrow\""));
    assert!(drafts.contains("data-act=\"rename\"") && drafts.contains("data-act=\"delete\""));
    assert!(html.contains("id=\"cd-vaultnew\""));
    // With a password and the setting on, the page reflects both.
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some("hunter2")).unwrap();
    ts.store.set_auto_purge_days(&ts.owner_id, Some(14)).unwrap();
    let (_, html) = page(&ts, "/settings/server/general", &owner);
    assert!(html.contains("data-password-set=\"true\" data-mine=\"true\""));
    assert!(html.contains("id=\"cd-pw-remove\"") && html.contains("Change password"));
    assert!(
        html.contains("data-days=\"14\"")
            && html.contains("id=\"cd-purge-on\" checked")
            && html.contains("value=\"14\"")
    );
    assert!(html.contains("Sign out"), "a password set ⇒ the top bar offers Sign out");
    // The Vault fragment.
    let (status, frag) = send(&ts, "GET", "/settings/vaults/drafts", Some(&owner), None);
    assert_eq!(status, 200, "{frag}");
    assert_eq!(frag["name"], "drafts");
    assert!(frag["html"].as_str().unwrap().contains("data-vault=\"drafts\""));
    assert_eq!(send(&ts, "GET", "/settings/vaults/nope", Some(&owner), None).0, 404);
}

// CMD-23 changed this test on purpose (it was
// `owner_screens_are_403_pages_for_a_member_and_the_nav_never_links_them`): every Member has the
// Settings screen and its top-bar link; the owner pages moved under `/settings/server/*`, which
// a Member gets as the 403 page (`/members` 308s there); the fragments are unchanged.
// spec: S12-05, S29-20
#[test]
fn settings_is_every_members_and_the_owner_pages_are_403_pages_for_a_member() {
    let ts = TestServer::start();
    let (_, agent) = ts.add_member("scribe");
    // Settings: 200 for a Member, with the Settings link current and no Server group.
    let (status, html) = page(&ts, "/settings", &agent);
    assert_eq!(status, 200, "{html}");
    assert!(html.contains("id=\"cd-nav-settings\" href=\"/settings\" aria-current=\"page\""));
    assert!(html.contains("class=\"cd-app cd-settingspage cd-theme-"), "opens with the Member's settings");
    assert!(!html.contains("id=\"cd-set-serverhead\"") && !html.contains("href=\"/settings/server/"));
    // The owner pages: the 403 page.
    for path in ["/settings/server/members", "/settings/server/vaults", "/settings/server/general"] {
        let (status, html) = page(&ts, path, &agent);
        assert_eq!(status, 403, "{path}");
        assert!(html.contains("<!doctype html>") && html.contains("owner-only"), "{path}: {html}");
        assert!(
            !html.contains("id=\"cd-addagent\"")
                && !html.contains("id=\"cd-pwform\"")
                && !html.contains("id=\"cd-vaults\"")
        );
    }
    // `/members` 308s to the Members page, for anyone (a Member then gets the 403 page there).
    for token in [&agent, &ts.owner_token] {
        let r = ureq::get(format!("{}/members", ts.url))
            .header("Accept", "text/html")
            .header("Authorization", format!("Bearer {token}"))
            .config()
            .http_status_as_error(false)
            .max_redirects(0)
            .build()
            .call()
            .unwrap();
        assert_eq!(r.status().as_u16(), 308);
        assert_eq!(r.headers().get("location").unwrap(), "/settings/server/members");
    }
    // The card fragments: unchanged, never redirected, 403 JSON for a Member.
    for path in ["/members/rowan", "/settings/vaults/notes"] {
        let (status, body) = send(&ts, "GET", path, Some(&agent), None);
        assert_eq!(status, 403, "{path}");
        assert_eq!(body["error"], "forbidden");
        assert!(body["message"].as_str().unwrap().contains("owner-only"));
    }
    // The list page: Documents and Settings for a Member and for the owner; no Members link.
    let (status, html) = page(&ts, "/", &agent);
    assert_eq!(status, 200);
    assert!(html.contains("id=\"cd-nav-docs\"") && html.contains("id=\"cd-nav-settings\" href=\"/settings\""));
    assert!(!html.contains("cd-nav-members"));
    let (_, html) = page(&ts, "/", &ts.owner_token);
    assert!(html.contains("id=\"cd-nav-settings\" href=\"/settings\"") && !html.contains("cd-nav-members"));
    // The owner-only mutations behind the screens stay 403 for a Member too.
    assert_eq!(send(&ts, "POST", "/api/v1/vaults/notes/rename", Some(&agent), Some(&json!({ "name": "x" }))).0, 403);
    assert_eq!(send(&ts, "DELETE", "/api/v1/vaults/notes", Some(&agent), None).0, 403);
    assert_eq!(send(&ts, "PUT", "/api/v1/settings/auto-purge", Some(&agent), Some(&json!({ "days": 3 }))).0, 403);
    assert_eq!(
        send(&ts, "PUT", "/api/v1/vaults/notes/actions", Some(&agent), Some(&json!({ "available": [] }))).0,
        403
    );
}

// spec: S12-04
#[test]
fn vault_rename_follows_grants_and_delete_refuses_non_empty_or_last() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (status, v) = send(&ts, "POST", "/api/v1/vaults", Some(&owner), Some(&json!({ "name": "drafts" })));
    assert_eq!(status, 201, "{v}");
    // A duplicate / `*` / empty name on create or rename.
    assert_eq!(send(&ts, "POST", "/api/v1/vaults", Some(&owner), Some(&json!({ "name": "drafts" }))).0, 409);
    assert_eq!(send(&ts, "POST", "/api/v1/vaults", Some(&owner), Some(&json!({ "name": "*" }))).0, 400);
    let (linter_id, linter) = ts.add_member("linter");
    ts.store.grant(&ts.owner_id, &linter_id, "drafts", &[Right::Write]).unwrap();
    ts.store.grant(&ts.owner_id, &linter_id, "*", &[]).unwrap();
    let doc = create_doc(&ts, &owner, "drafts", "plan", "# Plan\n");
    // linter can read it through the `drafts` Grant …
    assert_eq!(send(&ts, "GET", &format!("/api/v1/docs/{doc}"), Some(&linter), None).0, 200);
    // … rename: the Grant follows, the Document is reachable under the new name, old name gone.
    let (status, r) =
        send(&ts, "POST", "/api/v1/vaults/drafts/rename", Some(&owner), Some(&json!({ "name": "reviews" })));
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["name"], "reviews");
    assert_eq!(r["from"], "drafts");
    assert_eq!(r["documents"], 1);
    assert_eq!(send(&ts, "GET", &format!("/api/v1/docs/{doc}"), Some(&linter), None).1["vault"], "reviews");
    let m = ts.store.member_by_id(&linter_id).unwrap().unwrap();
    assert_eq!(m.grants, vec![Grant { vault: "reviews".into(), rights: vec![Right::Write] }]);
    assert_eq!(send(&ts, "POST", "/api/v1/vaults/drafts/rename", Some(&owner), Some(&json!({ "name": "x" }))).0, 404);
    let (status, r) =
        send(&ts, "POST", "/api/v1/vaults/reviews/rename", Some(&owner), Some(&json!({ "name": "notes" })));
    assert_eq!(status, 409, "{r}");
    assert_eq!(r["error"], "conflict");
    let (status, r) = send(&ts, "POST", "/api/v1/vaults/reviews/rename", Some(&owner), Some(&json!({ "name": "" })));
    assert_eq!(status, 400, "{r}");
    // Delete: non-empty (also while merely deleted) ⇒ 409 vault_not_empty; after purge it goes
    // and linter's Grant with it; the last Vault is refused.
    let (status, r) = send(&ts, "DELETE", "/api/v1/vaults/reviews", Some(&owner), None);
    assert_eq!(status, 409, "{r}");
    assert_eq!(r["error"], "vault_not_empty");
    assert_eq!(send(&ts, "DELETE", &format!("/api/v1/docs/{doc}"), Some(&owner), None).0, 200);
    let (status, r) = send(&ts, "DELETE", "/api/v1/vaults/reviews", Some(&owner), None);
    assert_eq!(status, 409, "{r}");
    assert!(r["message"].as_str().unwrap().contains("1 deleted"), "{r}");
    assert_eq!(send(&ts, "POST", &format!("/api/v1/docs/{doc}/purge"), Some(&owner), None).0, 200);
    let (status, r) = send(&ts, "DELETE", "/api/v1/vaults/reviews", Some(&owner), None);
    assert_eq!(status, 200, "{r}");
    assert_eq!(r["deleted"], true);
    assert!(ts.store.vault_by_name("reviews").unwrap().is_none());
    assert!(ts.store.member_by_id(&linter_id).unwrap().unwrap().grants.is_empty());
    assert_eq!(send(&ts, "DELETE", "/api/v1/vaults/reviews", Some(&owner), None).0, 404);
    let (status, r) = send(&ts, "DELETE", "/api/v1/vaults/notes", Some(&owner), None);
    assert_eq!(status, 409, "{r}");
    assert_eq!(r["error"], "last_vault");
    // Events recorded both.
    let kinds: Vec<String> = ts.store.events_since(0).unwrap().into_iter().map(|e| e.kind).collect();
    assert!(kinds.contains(&"vault.renamed".to_string()) && kinds.contains(&"vault.deleted".to_string()));
}

// spec: S12-06
#[test]
fn login_page_wears_the_shared_shell_and_keeps_its_contract() {
    let ts = TestServer::start();
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some("hunter2")).unwrap();
    let mut r = ureq::get(format!("{}/login?next=%2Fsettings", ts.url))
        .header("Accept", "text/html")
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    let html = r.body_mut().read_to_string().unwrap();
    assert!(html.contains("<title>Sign in — annotary</title>"), "{html}");
    assert!(html.contains("class=\"cd-app cd-loginpage cd-theme-system\""), "the shared shell: {html}");
    assert!(html.contains(".cd-loginform{"), "admin.css inlined");
    assert!(html.contains("<form method=\"post\" action=\"/login\" class=\"cd-loginform\" id=\"cd-loginform\">"));
    assert!(html.contains("<input type=\"hidden\" name=\"next\" value=\"/settings\">"));
    assert!(html.contains("name=\"password\"") && html.contains("class=\"cd-btn cd-primary\">Sign in</button>"));
    assert!(!html.contains("role=\"alert\""), "no error on a fresh form");
    // Wrong password: 401 + the form with the error line.
    let mut r = ureq::post(format!("{}/login", ts.url))
        .header("Content-Type", "application/x-www-form-urlencoded")
        .config()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .send("password=nope&next=%2F")
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);
    let html = r.body_mut().read_to_string().unwrap();
    assert!(html.contains("<p class=\"cd-err\" role=\"alert\">Wrong password.</p>"), "{html}");
    // Owner pages without a session redirect to the login page with `next` (the middleware).
    let r = ureq::get(format!("{}/members", ts.url))
        .header("Accept", "text/html")
        .config()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .call()
        .unwrap();
    assert_eq!(r.status().as_u16(), 303);
    assert_eq!(r.headers().get("location").unwrap().to_str().unwrap(), "/login?next=%2Fmembers");
}
