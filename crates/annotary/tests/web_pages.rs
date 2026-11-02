//! The web pages of ticket 10 over HTTP against the in-process server: `/` (the Document list),
//! `/d/{id}` and `/d/{id}/v/{n}` are single server-composed responses that carry the rendered
//! Version, every visible Thread with its placement and the embedded placement data; the
//! time-slice, the banner, Action chips per Version, Comment bodies without embeds, the list's
//! columns / filters / title / "more", and the login redirect when a password is set. The
//! browser-side behaviour (layouts, overlay, highlights) is the Playwright suite in `tests/web/`.

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use serde_json::{Value, json};

const V1: &str = "# Launch plan\n\nShip a local server + CLI + web viewer that humans and agents can use.\n\n## Order of work\n\n1. Sidecar format and anchor engine — the core, testable in isolation.\n2. Local server with content-addressed storage.\n\nWe ship the CLI before the web UI so agents can start using it early.\n\n## Risks\n\n- Token cost of the sidecar: agents should use the CLI, not read the file whole.\n\nSee [API notes](api-notes.md) for the endpoint list.\n";
/// v2: the span of t2 is edited underneath (refined), the Risks bullet is gone (orphans t4).
const V2: &str = "# Launch plan\n\nShip a local server + CLI + web viewer that humans and agents can use.\n\n## Order of work\n\n1. Sidecar format and anchor engine — the core, testable in isolation.\n2. Local server with content-addressed storage.\n\nWe ship the CLI well before the web UI so agents can start using it early.\n\nSee [API notes](api-notes.md) for the endpoint list.\n";

fn upload(ts: &TestServer, token: &str, md: &[u8]) -> String {
    let hash = sha256_hex(md);
    let res = ureq::put(format!("{}/api/v1/blobs/{hash}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(md)
        .unwrap();
    assert!(res.status().is_success(), "{}", res.status());
    hash
}

fn create(ts: &TestServer, token: &str, name: &str, md: &str) -> String {
    let hash = upload(ts, token, md.as_bytes());
    let (status, doc) =
        ts.post("/api/v1/docs", Some(token), &json!({ "vault": "notes", "name": name, "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

fn push(ts: &TestServer, token: &str, id: &str, name: &str, md: &str, message: &str) -> u64 {
    let hash = upload(ts, token, md.as_bytes());
    let (status, v) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(token),
        &json!({ "name": name, "md_hash": hash, "message": message }),
    );
    assert_eq!(status, 201, "{v}");
    v["version"].as_u64().unwrap()
}

fn thread(ts: &TestServer, token: &str, id: &str, body: Value) -> String {
    let (status, t) = ts.post(&format!("/api/v1/docs/{id}/threads"), Some(token), &body);
    assert_eq!(status, 201, "{t}");
    t["id"].as_str().unwrap().to_string()
}

/// A raw GET with headers, no redirect following: (status, headers, body).
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

fn header<'a>(hs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

/// A browser page request (loopback, `Accept: text/html`, no credential): the page and the
/// session cookie the middleware issued.
fn page(ts: &TestServer, path: &str) -> (u16, String) {
    let (status, _, body) = raw_get(&format!("{}{path}", ts.url), &[("Accept", "text/html")]);
    (status, body)
}

/// The embedded `#cd-data` JSON of a Document page.
fn cd_data(html: &str) -> Value {
    const OPEN: &str = "<script type=\"application/json\" id=\"cd-data\">";
    let start = html.find(OPEN).expect("cd-data") + OPEN.len();
    let end = html[start..].find("</script>").unwrap() + start;
    serde_json::from_str(html[start..end].replace("<\\/", "</").as_str()).expect("cd-data parses")
}

fn count(hay: &str, needle: &str) -> usize {
    hay.matches(needle).count()
}

// spec: S10-01, S10-05, S10-06, S10-07, S10-09
#[test]
fn document_pages_are_one_response_with_every_thread_placement_and_version() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let id = create(&ts, &owner, "plan", V1);
    // t1 document, t2 span (refined on v2), t3 span (anchored), t4 span (orphaned on v2),
    // t5 fully tombstoned (hidden), t6 with a tombstoned reply and an embed in a Comment.
    let t1 = thread(&ts, &owner, &id, json!({"subject":"document","body":"Needs a **Risks** section. @scribe?"}));
    let t2 = thread(
        &ts,
        &agent,
        &id,
        json!({"subject":"span","range":"10:0-10:33","body":"Agreed — the skill file lands *with* the CLI. @rowan"}),
    );
    let _t3 = thread(&ts, &owner, &id, json!({"subject":"span","range":"7:48-7:69","body":"Own package?"}));
    let t4 = thread(
        &ts,
        &agent,
        &id,
        json!({"subject":"span","range":"14:29-14:79","body":"Link the sidecar ticket here."}),
    );
    let t5 = thread(&ts, &owner, &id, json!({"subject":"span","range":"3:0-3:4","body":"gone"}));
    let t6 = thread(
        &ts,
        &owner,
        &id,
        json!({"subject":"span","range":"3:5-3:6","body":"see ![arch](img/architecture.png) and [notes](api-notes.md)"}),
    );
    let (status, _) = ts.post(&format!("/api/v1/docs/{id}/threads/{t6}/replies"), Some(&agent), &json!({"body":"bye"}));
    assert_eq!(status, 201);
    let (status, _) = ts.post(
        &format!("/api/v1/docs/{id}/threads/{t1}/replies"),
        Some(&agent),
        &json!({"body":"Added in v2.","resolve":true}),
    );
    assert_eq!(status, 201);
    for cid in [t5.as_str(), &format!("{t6}.1")] {
        let r = ureq::delete(format!("{}/api/v1/docs/{id}/threads/{cid}", ts.url))
            .header("Authorization", format!("Bearer {owner}"))
            .call()
            .unwrap();
        assert!(r.status().is_success());
    }
    let (status, _) = ts.post(&format!("/api/v1/docs/{id}/actions"), Some(&owner), &json!({"action":"send-back"}));
    assert_eq!(status, 201);
    assert_eq!(push(&ts, &owner, &id, "plan", V2, "address review"), 2);
    let (status, _) = ts.post(&format!("/api/v1/docs/{id}/actions"), Some(&agent), &json!({"action":"approve"}));
    assert_eq!(status, 201);

    // ---- head page ----
    let (status, html) = page(&ts, &format!("/d/{id}"));
    assert_eq!(status, 200, "{html}");
    assert!(html.starts_with("<!doctype html>"));
    // ticket 15: the script is an inline *module* (deferred, so the page paints before it runs).
    assert!(html.contains("<style>") && html.contains("<script type=\"module\">"), "CSS and JS are inlined");
    // Every visible Thread is in the first response, at its block (strips inside the doc).
    for t in [&t1, &t2, "t3", &t4, &t6] {
        assert!(html.contains("<article class=\"cd-thread cd-st-"), "{t}");
        assert!(html.contains(&format!("data-t=\"{t}\"")), "Thread {t} missing from the first response");
    }
    assert!(!html.contains(&format!("data-t=\"{t5}\"")), "a fully-tombstoned Thread is hidden");
    let doc_start = html.find("<article class=\"cd-doc\"").unwrap();
    let doc_end = html.find("</article></main>").unwrap();
    let doc_html = &html[doc_start..doc_end];
    assert!(
        doc_html.contains(
            "<div class=\"cd-strip cd-strip-doc cd-all-resolved\"><div class=\"cd-strip-h\">Whole document</div>"
        ),
        "Document-level strip (all resolved) under the title"
    );
    assert!(
        doc_html.contains("<h1 data-l=\"1-1\" id=\"launch-plan\">Launch plan</h1><div class=\"cd-strip cd-strip-doc"),
        "doc strip right after the title"
    );
    // Refined: frozen quote + now: line; orphaned grouped behind the disclosure (open, hidden by default).
    assert!(doc_html.contains("<q>We ship the CLI before the web UI</q><span class=\"cd-now\">now: <q>We ship the CLI well before the web UI</q></span>"), "{doc_html}");
    let orph = doc_html
        .find("<details class=\"cd-orphaned\"><summary>Orphaned (<span class=\"cd-orph-n\">1</span>)")
        .expect("orphaned disclosure");
    assert!(doc_html[orph..].contains(&format!("data-t=\"{t4}\"")));
    assert!(!doc_html.contains("<details class=\"cd-orphaned\" open"), "hidden by default");
    assert!(doc_html.contains("— text no longer in v2"));
    // Tombstoned reply shows (deleted); Members by display name + auth glyph; time tags.
    assert!(doc_html.contains("<span class=\"cd-tomb\">(deleted)</span>"));
    assert!(
        doc_html
            .contains("<b class=\"cd-member\" title=\"@rowan\">Rowan Vale</b><span class=\"cd-auth cd-auth-token\"")
    );
    assert!(doc_html.contains("<time datetime=\"20"));
    assert!(!doc_html.contains("bot"));
    // Comment bodies: the superset minus embeds — the image is a link, no <img> in any card.
    let t6_card = doc_html.find(&format!("data-t=\"{t6}\"")).unwrap();
    let card = &doc_html[t6_card..doc_html[t6_card..].find("</article>").unwrap() + t6_card];
    assert!(card.contains("<strong>") || card.contains("<a "), "{card}");
    assert!(!card.contains("<img"), "{card}");
    assert!(!card.contains("data-l="), "no data-l inside Comment bodies: {card}");
    // Asset links open in a new tab (rendered Document and Comments alike).
    assert!(doc_html.contains("<a target=\"_blank\" rel=\"noopener\" href=\"/a/") || !doc_html.contains("href=\"/a/"));
    // The embedded placement data: every visible Thread with status / line / quote / now.
    let data = cd_data(&html);
    assert_eq!(data["id"], id);
    assert_eq!(data["head"], 2);
    assert_eq!(data["version"], 2);
    let ths = data["threads"].as_array().unwrap();
    assert_eq!(ths.len(), 5, "{ths:?}");
    let by_id = |tid: &str| ths.iter().find(|t| t["id"] == tid).unwrap().clone();
    assert_eq!(by_id(&t2)["status"], "refined");
    assert_eq!(by_id(&t2)["line"], 10);
    assert_eq!(by_id(&t2)["quote"], "We ship the CLI before the web UI");
    assert_eq!(by_id(&t2)["now"], "We ship the CLI well before the web UI");
    assert_eq!(by_id(&t4)["status"], "orphaned");
    assert_eq!(by_id(&t1)["subject"], "document");
    assert_eq!(by_id(&t1)["resolved"], true);
    assert_eq!(by_id(&t2)["mine"], true, "t2 mentions @rowan");
    // Header: Version select rows (by · at · Action), the head's Action chip, counts.
    assert!(html.contains("<select id=\"cd-vsel\""));
    assert!(html.contains("data-pre=\"v2 (head) · Rowan Vale · \"") && html.contains("data-post=\" · Approve\""));
    assert!(html.contains("data-pre=\"v1 · Rowan Vale · \"") && html.contains("data-post=\" · Send back\""));
    assert!(html.contains("<span class=\"cd-chip cd-action cd-pos\" data-action=\"approve\" data-version=\"2\">Approve · scribe · <time class=\"cd-rel\""), "{html}");
    assert!(
        html.contains(
            "<b>v2</b> · approve by scribe · <b>4</b> open (1 orphaned) · <b class=\"cd-mention\">1 @you</b>"
        ),
        "{html}"
    );
    assert!(!html.contains("id=\"cd-banner\""), "no banner on head");
    // Nothing on the page fetches Threads to show them: the script never lists them (ticket 11's
    // mutations go to `…/threads` and read one card back through `/d/{id}/t/{tid}` — the
    // Playwright suite asserts no Thread request happens on load).
    assert!(!html.contains("/threads?"), "no Thread list fetch from the page");
    assert!(!html.contains("threads?status="), "no Thread list fetch from the page");

    // ---- time-slice: v1 ----
    let (status, v1) = page(&ts, &format!("/d/{id}/v/1"));
    assert_eq!(status, 200, "{v1}");
    assert!(
        v1.contains("id=\"cd-banner\"")
            && v1.contains("Viewing <b>v1</b>")
            && v1.contains("Threads as they were; replies/resolve work, new comments only on head")
    );
    assert!(v1.contains(&format!("<a class=\"cd-btn\" id=\"cd-gohead\" href=\"/d/{id}\">Go to head v2</a>")));
    // v1's own Action chip (snapshotted label / neutral intent), not the head's.
    assert!(v1.contains(
        "<span class=\"cd-chip cd-action cd-neu\" data-action=\"send-back\" data-version=\"1\">Send back · Rowan Vale"
    ));
    assert!(!v1.contains("data-action=\"approve\""));
    let d1 = cd_data(&v1);
    assert_eq!(d1["version"], 1);
    let ths1 = d1["threads"].as_array().unwrap();
    // On v1 t2 was anchored (not yet refined) and t4 anchored (not yet orphaned); t1 was open.
    let t2v1 = ths1.iter().find(|t| t["id"] == t2).unwrap();
    assert_eq!(t2v1["status"], "anchored");
    assert!(t2v1["now"].is_null());
    assert_eq!(ths1.iter().find(|t| t["id"] == t4).unwrap()["status"], "anchored");
    assert!(!v1.contains("<details class=\"cd-orphaned\""), "nothing orphaned on v1");
    assert!(v1.contains("data-t=\"t1\""));
    assert!(v1.contains("<title>plan v1 — annotary</title>"));
    // Unknown Version / Document ⇒ HTML 404 pages.
    let (status, body) = page(&ts, &format!("/d/{id}/v/9"));
    assert_eq!(status, 404);
    assert!(body.contains("has no v9") && body.starts_with("<!doctype html>"), "{body}");
    let (status, body) = page(&ts, "/d/zzzzzzzz");
    assert_eq!(status, 404);
    assert!(body.contains("no Document") && body.starts_with("<!doctype html>"), "{body}");
    // A Member without `read` on the Vault gets a 403 page (bearer auth works on pages too).
    let (mid, nobody) = ts.add_member("nobody");
    ts.store.set_grants(&ts.owner_id, &mid, vec![]).unwrap();
    let (status, _, body) = raw_get(
        &format!("{}/d/{id}", ts.url),
        &[("Accept", "text/html"), ("Authorization", &format!("Bearer {nobody}"))],
    );
    assert_eq!(status, 403, "{body}");
    assert!(body.contains("read"));
    // Counts stay per page: the list shows the same open count as the head page.
    let (_, list) = page(&ts, "/");
    assert!(list.contains(&format!("data-id=\"{id}\"")));
    assert_eq!(count(&list, "<tr class=\"cd-docrow\""), 1);
}

// spec: S10-08
#[test]
fn list_page_is_the_cli_list_with_title_filters_and_more() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let a = create(&ts, &owner, "lst-alpha", "---\ntitle: Alpha from front matter\n---\n\n# Not this\n\nbody\n");
    let b = create(&ts, &owner, "lst-beta", "# Beta heading\n\nbody\n");
    let c = create(&ts, &owner, "lst-gamma", "no title here\n");
    let d = create(&ts, &owner, "other-delta", "# Delta\n\nbody\n");
    // Activity: approve c (most recent), mention rowan on b, delete d.
    let (status, _) = ts.post(
        &format!("/api/v1/docs/{b}/threads"),
        Some(&agent),
        &json!({"subject":"document","body":"@rowan look"}),
    );
    assert_eq!(status, 201);
    let (status, _) = ts.post(&format!("/api/v1/docs/{c}/actions"), Some(&owner), &json!({"action":"approve"}));
    assert_eq!(status, 201);
    let r = ureq::delete(format!("{}/api/v1/docs/{d}", ts.url))
        .header("Authorization", format!("Bearer {owner}"))
        .call()
        .unwrap();
    assert!(r.status().is_success());

    let (status, html) = page(&ts, "/");
    assert_eq!(status, 200);
    // Columns as the CLI prints them; activity order (c, b, a); the deleted one hidden.
    assert!(html.contains("<th class=\"cd-col-id\">ID</th><th>NAME</th><th class=\"cd-col-ver\">VER</th><th class=\"cd-col-action\">ACTION</th><th class=\"cd-num cd-col-open\">OPEN</th><th class=\"cd-col-updated\">UPDATED</th>"));
    let pos = |id: &str| html.find(&format!("data-id=\"{id}\"")).unwrap_or(usize::MAX);
    assert!(pos(&c) < pos(&b) && pos(&b) < pos(&a), "activity order");
    assert_eq!(pos(&d), usize::MAX, "deleted Documents hidden by default");
    // Title display-only: front matter wins over the H1; the H1 else; none when there is none.
    assert!(html.contains("<div class=\"cd-title\">Alpha from front matter</div>"));
    assert!(html.contains("<div class=\"cd-title\">Beta heading</div>"));
    let c_row = &html[pos(&c)..html[pos(&c)..].find("</tr>").unwrap() + pos(&c)];
    assert!(!c_row.contains("cd-title"), "{c_row}");
    assert!(c_row.contains("<span class=\"cd-chip cd-pos\">approve</span>"));
    assert!(c_row.contains("<td class=\"cd-col-ver\">v1</td>"));
    let b_row = &html[pos(&b)..html[pos(&b)..].find("</tr>").unwrap() + pos(&b)];
    assert!(
        b_row.contains("<td class=\"cd-num cd-col-open\">1 <span class=\"cd-mention cd-small\">1 @you</span></td>"),
        "{b_row}"
    );
    assert!(b_row.contains("<time class=\"cd-rel\" datetime=\"20"));
    assert!(!html.contains("unread"), "no unread markers");
    // Filters mirror the CLI.
    let (_, h) = page(&ts, "/?name=lst-");
    assert_eq!(count(&h, "<tr class=\"cd-docrow\""), 3);
    let (_, h) = page(&ts, "/?name=beta");
    assert_eq!(count(&h, "<tr class=\"cd-docrow\""), 1);
    assert!(h.contains(&format!("data-id=\"{b}\"")));
    let (_, h) = page(&ts, "/?deleted=1");
    assert_eq!(count(&h, "<tr class=\"cd-docrow\""), 1);
    assert!(h.contains(&format!("data-id=\"{d}\"")) && h.contains("<span class=\"cd-deleted\">deleted</span>"));
    let (_, h) = page(&ts, "/?mentions_me=1");
    assert_eq!(count(&h, "<tr class=\"cd-docrow\""), 1);
    assert!(h.contains(&format!("data-id=\"{b}\"")));
    let (_, h) = page(&ts, "/?action=approve");
    assert_eq!(count(&h, "<tr class=\"cd-docrow\""), 1);
    assert!(h.contains(&format!("data-id=\"{c}\"")));
    let (_, h) = page(&ts, "/?action=none");
    assert_eq!(count(&h, "<tr class=\"cd-docrow\""), 2);
    let (_, h) = page(&ts, "/?vault=notes&since=1h");
    assert_eq!(count(&h, "<tr class=\"cd-docrow\""), 3);
    let (status, h) = page(&ts, "/?since=garbage");
    assert_eq!(status, 400, "{h}");
    let (_, h) = page(&ts, "/?name=zzz");
    assert!(h.contains("id=\"cd-empty\""));
    // limit + "more" keeps the filters.
    let (_, h) = page(&ts, "/?name=lst-&limit=2");
    assert_eq!(count(&h, "<tr class=\"cd-docrow\""), 2);
    assert!(h.contains("showing 2 of 3 — <a id=\"cd-more\" href=\"/?name=lst-&limit=52\">more</a>"), "{h}");
    let (_, h) = page(&ts, "/?name=lst-&limit=52");
    assert_eq!(count(&h, "<tr class=\"cd-docrow\""), 3);
    assert!(!h.contains("id=\"cd-more\""));
    // The filter form carries the current values.
    let (_, h) = page(&ts, "/?name=lst-&deleted=1&mentions_me=1&action=none");
    assert!(h.contains("name=\"name\" placeholder=\"Name contains…\" value=\"lst-\""));
    assert!(
        h.contains("name=\"deleted\" value=\"1\" checked") && h.contains("name=\"mentions_me\" value=\"1\" checked")
    );
}

// spec: S10-02
#[test]
fn pages_sign_a_loopback_browser_in_without_a_password_and_redirect_to_login_with_one() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let id = create(&ts, &owner, "plan", V1);
    // No password: the page is served and a session cookie issued.
    let (status, hs, body) = raw_get(&format!("{}/d/{id}", ts.url), &[("Accept", "text/html")]);
    assert_eq!(status, 200, "{body}");
    let cookie = header(&hs, "set-cookie").unwrap().split(';').next().unwrap().to_string();
    assert!(cookie.starts_with("cd_session=cds_"));
    assert!(body.contains("data-t=\"") || body.contains("cd-doc"));
    // The list page names no sign-out without a password.
    let (_, _, list) = raw_get(&format!("{}/", ts.url), &[("Accept", "text/html")]);
    assert!(!list.contains("Sign out"));
    // A password set: a credential-less page request is sent to /login?next=<the page>.
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some("hunter2")).unwrap();
    let (status, hs, _) = raw_get(&format!("{}/d/{id}", ts.url), &[("Accept", "text/html")]);
    assert_eq!(status, 303);
    assert_eq!(header(&hs, "location"), Some(format!("/login?next=%2Fd%2F{id}").as_str()));
    let (status, hs, _) = raw_get(&format!("{}/d/{id}/v/1", ts.url), &[("Accept", "text/html")]);
    assert_eq!(status, 303);
    assert!(header(&hs, "location").unwrap().starts_with("/login?next="));
    // …while the remembered session (and a bearer) still get the page, which now offers Sign out.
    let (status, _, body) = raw_get(&format!("{}/d/{id}", ts.url), &[("Accept", "text/html"), ("Cookie", &cookie)]);
    assert_eq!(status, 200, "{body}");
    let (status, _, list) = raw_get(&format!("{}/", ts.url), &[("Accept", "text/html"), ("Cookie", &cookie)]);
    assert_eq!(status, 200);
    assert!(list.contains("Sign out"));
    let (status, _, _) =
        raw_get(&format!("{}/", ts.url), &[("Accept", "text/html"), ("Authorization", &format!("Bearer {owner}"))]);
    assert_eq!(status, 200);
    // The login page itself renders the form now.
    let (status, _, body) = raw_get(&format!("{}/login", ts.url), &[("Accept", "text/html")]);
    assert_eq!(status, 200);
    assert!(body.contains("name=\"password\""));
}
