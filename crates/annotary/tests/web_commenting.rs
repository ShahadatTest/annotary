//! Ticket 11, server side, over HTTP against the in-process server: the Thread-card fragment
//! (`GET /d/{id}/t/{tid}?v=N`) the page re-draws cards from, the mutation controls the page
//! renders per viewer (reply / resolve for commenters, edit / delete for the author or an owner,
//! Action buttons head-only and by intent, none for `[]` or without the `action` right), the pill
//! / help markup on head, and the extra `#cd-data` fields the script needs. The browser behaviour
//! itself is `tests/web/commenting.spec.ts`, `keyboard.spec.ts`, `live.spec.ts`.

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use annotary_store::{Grant, Right};
use serde_json::{Value, json};

const V1: &str = "# Plan\n\nShip a local server + CLI + web viewer.\n\nWe ship the CLI before the web UI so agents can start using it early.\n";
const V2: &str = "# Plan\n\nShip a local server + CLI + web viewer.\n\nWe ship the CLI well before the web UI so agents can start using it early.\n";

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

fn create(ts: &TestServer, token: &str, name: &str, md: &str, extra: Value) -> String {
    let hash = upload(ts, token, md.as_bytes());
    let mut body = json!({ "vault": "notes", "name": name, "md_hash": hash });
    if let Some(o) = extra.as_object() {
        for (k, v) in o {
            body[k] = v.clone();
        }
    }
    let (status, doc) = ts.post("/api/v1/docs", Some(token), &body);
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

fn push(ts: &TestServer, token: &str, id: &str, name: &str, md: &str) -> u64 {
    let hash = upload(ts, token, md.as_bytes());
    let (status, v) =
        ts.post(&format!("/api/v1/docs/{id}/versions"), Some(token), &json!({ "name": name, "md_hash": hash }));
    assert_eq!(status, 201, "{v}");
    v["version"].as_u64().unwrap()
}

fn thread(ts: &TestServer, token: &str, id: &str, body: Value) -> String {
    let (status, t) = ts.post(&format!("/api/v1/docs/{id}/threads"), Some(token), &body);
    assert_eq!(status, 201, "{t}");
    t["id"].as_str().unwrap().to_string()
}

/// A browser-like GET (bearer, any Accept): (status, body).
fn get(ts: &TestServer, token: &str, path: &str) -> (u16, String) {
    let mut r = ureq::get(format!("{}{path}", ts.url))
        .header("Authorization", format!("Bearer {token}"))
        .header("Accept", "text/html")
        .config()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .call()
        .unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap_or_default())
}

fn cd_data(html: &str) -> Value {
    const OPEN: &str = "<script type=\"application/json\" id=\"cd-data\">";
    let start = html.find(OPEN).expect("cd-data") + OPEN.len();
    let end = html[start..].find("</script>").unwrap() + start;
    serde_json::from_str(html[start..end].replace("<\\/", "</").as_str()).expect("cd-data parses")
}

fn card<'a>(html: &'a str, tid: &str) -> &'a str {
    let open = "<article class=\"cd-thread cd-st-";
    let mut from = 0;
    while let Some(p) = html[from..].find(open) {
        let s = from + p;
        let e = s + html[s..].find("</article>").unwrap() + "</article>".len();
        let c = &html[s..e];
        if c.contains(&format!("data-t=\"{tid}\"")) {
            return c;
        }
        from = e;
    }
    panic!("no card {tid} in the page");
}

// spec: S11-09, S11-03
#[test]
fn thread_card_fragment_renders_the_card_with_the_viewer_s_controls_and_the_time_slice_placement() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let (reader_id, reader) = ts.add_member("reader");
    ts.store
        .set_grants(&ts.owner_id, &reader_id, vec![Grant { vault: "notes".into(), rights: vec![Right::Read] }])
        .unwrap();
    let id = create(&ts, &owner, "plan", V1, json!({}));
    let t1 = thread(&ts, &agent, &id, json!({ "subject": "span", "range": "5:0-5:33", "body": "Order? @rowan" }));
    let t2 = thread(&ts, &owner, &id, json!({ "subject": "document", "body": "whole" }));
    let (status, _) =
        ts.post(&format!("/api/v1/docs/{id}/threads/{t1}/replies"), Some(&owner), &json!({ "body": "yes" }));
    assert_eq!(status, 201);

    // ---- the fragment for the owner: live card, reply / resolve, edit / delete on every Comment ----
    let (status, body) = get(&ts, &owner, &format!("/d/{id}/t/{t1}"));
    assert_eq!(status, 200, "{body}");
    let frag: Value = serde_json::from_str(&body).unwrap();
    let html = frag["html"].as_str().unwrap();
    assert!(html.starts_with("<article class=\"cd-thread cd-st-anchored"), "{html}");
    assert!(html.contains(&format!("data-t=\"{t1}\"")));
    assert!(html.contains("data-act=\"reply\"") && html.contains("data-act=\"resolve\""), "{html}");
    assert_eq!(html.matches("data-act=\"edit\"").count(), 2, "owner edits any Comment: {html}");
    assert_eq!(html.matches("data-act=\"delete\"").count(), 2);
    assert!(html.contains("<div class=\"cd-c\" data-c=\"t1.1\">"));
    let item = &frag["item"];
    assert_eq!(item["id"], t1);
    assert_eq!(item["subject"], "span");
    assert_eq!(item["status"], "anchored");
    assert_eq!(item["line"], 5);
    assert_eq!(item["quote"], "We ship the CLI before the web UI");
    assert_eq!(item["resolved"], false);
    assert_eq!(item["mine"], true, "t1 mentions @rowan");
    assert_eq!(frag["hidden"], false);
    assert_eq!(frag["head"], 1);
    assert_eq!(frag["version"], 1);

    // ---- the agent: author of the root only → edit / delete on the root, not on the owner's reply ----
    let (status, body) = get(&ts, &agent, &format!("/d/{id}/t/{t1}"));
    assert_eq!(status, 200);
    let html = serde_json::from_str::<Value>(&body).unwrap()["html"].as_str().unwrap().to_string();
    assert_eq!(html.matches("data-act=\"edit\"").count(), 1, "{html}");
    let root_end = html.find("data-c=\"t1.1\"").unwrap();
    assert!(html[..root_end].contains("data-act=\"edit\""), "the root carries the tools");
    assert!(html.contains("data-act=\"reply\""));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["item"]["mine"], false);

    // ---- a read-only Member: no controls at all ----
    let (status, body) = get(&ts, &reader, &format!("/d/{id}/t/{t1}"));
    assert_eq!(status, 200);
    let html = serde_json::from_str::<Value>(&body).unwrap()["html"].as_str().unwrap().to_string();
    assert!(!html.contains("data-act="), "{html}");

    // ---- resolved state / document-level ----
    let (status, _) = ts.post(&format!("/api/v1/docs/{id}/threads/{t2}/resolve"), Some(&owner), &json!({}));
    assert_eq!(status, 200);
    let (_, body) = get(&ts, &owner, &format!("/d/{id}/t/{t2}"));
    let frag: Value = serde_json::from_str(&body).unwrap();
    assert!(frag["html"].as_str().unwrap().contains("cd-resolved"));
    assert!(frag["html"].as_str().unwrap().contains("data-act=\"reopen\">Reopen</button>"));
    assert_eq!(frag["item"]["subject"], "document");
    assert_eq!(frag["item"]["resolved"], true);
    assert!(frag["item"]["line"].is_null());

    // ---- v=N: the placement as of N, the conversation live ----
    assert_eq!(push(&ts, &owner, &id, "plan", V2), 2);
    let (status, _) =
        ts.post(&format!("/api/v1/docs/{id}/threads/{t1}/replies"), Some(&agent), &json!({ "body": "later" }));
    assert_eq!(status, 201);
    let (_, live) = get(&ts, &owner, &format!("/d/{id}/t/{t1}"));
    let live: Value = serde_json::from_str(&live).unwrap();
    assert_eq!(live["item"]["status"], "refined");
    assert_eq!(live["item"]["now"], "We ship the CLI well before the web UI");
    assert!(live["html"].as_str().unwrap().contains("cd-now"));
    assert!(live["html"].as_str().unwrap().contains("data-c=\"t1.2\""));
    assert_eq!(live["head"], 2);
    let (_, v1) = get(&ts, &owner, &format!("/d/{id}/t/{t1}?v=1"));
    let v1: Value = serde_json::from_str(&v1).unwrap();
    assert_eq!(v1["item"]["status"], "anchored");
    assert!(v1["item"]["now"].is_null());
    assert!(!v1["html"].as_str().unwrap().contains("cd-now"));
    assert!(
        v1["html"].as_str().unwrap().contains("data-c=\"t1.2\""),
        "the conversation stays live on a time-slice card"
    );
    assert_eq!(v1["version"], 1);
    assert_eq!(v1["head"], 2);

    // ---- hidden when fully tombstoned; 404s; 403 ----
    let t3 = thread(&ts, &owner, &id, json!({ "subject": "span", "range": "3:0-3:4", "body": "gone" }));
    let r = ureq::delete(format!("{}/api/v1/docs/{id}/threads/{t3}", ts.url))
        .header("Authorization", format!("Bearer {owner}"))
        .call()
        .unwrap();
    assert!(r.status().is_success());
    let (_, body) = get(&ts, &owner, &format!("/d/{id}/t/{t3}"));
    assert_eq!(serde_json::from_str::<Value>(&body).unwrap()["hidden"], true);
    let (status, body) = get(&ts, &owner, &format!("/d/{id}/t/t99"));
    assert_eq!(status, 404, "{body}");
    assert!(body.contains("no Thread t99"));
    let (status, body) = get(&ts, &owner, &format!("/d/{id}/t/{t1}?v=9"));
    assert_eq!(status, 404, "{body}");
    let (nobody_id, nobody) = ts.add_member("nobody");
    ts.store.set_grants(&ts.owner_id, &nobody_id, vec![]).unwrap();
    let (status, _) = get(&ts, &nobody, &format!("/d/{id}/t/{t1}"));
    assert_eq!(status, 403);
}

// spec: S11-09, S11-04, S11-01
#[test]
fn head_page_carries_pill_help_action_buttons_and_the_script_s_data() {
    let ts = TestServer::start();
    let owner = ts.owner_token.clone();
    let (commenter_id, commenter) = ts.add_member("scribe");
    let (reader_id, reader) = ts.add_member("reader");
    ts.store
        .set_grants(&ts.owner_id, &commenter_id, vec![Grant { vault: "notes".into(), rights: vec![Right::Comment] }])
        .unwrap();
    ts.store
        .set_grants(&ts.owner_id, &reader_id, vec![Grant { vault: "notes".into(), rights: vec![Right::Read] }])
        .unwrap();
    let id = create(&ts, &owner, "plan", V1, json!({}));
    thread(&ts, &owner, &id, json!({ "subject": "span", "range": "5:0-5:33", "body": "root" }));

    // ---- the owner on head: pill, help, two Action buttons by intent, every control ----
    let (status, html) = get(&ts, &owner, &format!("/d/{id}"));
    assert_eq!(status, 200);
    assert!(
        html.contains("<button type=\"button\" class=\"cd-pill\" id=\"cd-pill\" hidden>Comment <kbd>c</kbd></button>")
    );
    assert!(html.contains("id=\"cd-help\" hidden>"));
    for k in [
        "<kbd>j</kbd>",
        "<kbd>r</kbd>",
        "<kbd>e</kbd>",
        "<kbd>c</kbd>",
        "<kbd>[</kbd>",
        "<kbd>esc</kbd>",
        "<kbd>?</kbd>",
    ] {
        assert!(html.contains(k), "{k}");
    }
    assert!(html.contains("id=\"cd-helpbtn\""));
    assert!(html.contains("<span class=\"cd-actbtns\" id=\"cd-actbtns\"><button type=\"button\" class=\"cd-btn cd-actbtn cd-pos\" data-action=\"approve\" data-label=\"Approve\" data-intent=\"positive\" data-version=\"1\">Approve</button><button type=\"button\" class=\"cd-btn cd-actbtn cd-neu\" data-action=\"send-back\" data-label=\"Send back\" data-intent=\"neutral\" data-version=\"1\">Send back</button></span>"), "{html}");
    let c = card(&html, "t1");
    assert!(c.contains("<div class=\"cd-acts\"><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"reply\">Reply</button><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"resolve\">Resolve</button></div>"), "{c}");
    assert!(c.contains("<span class=\"cd-ctools\"><button type=\"button\" class=\"cd-link\" data-act=\"edit\">edit</button><button type=\"button\" class=\"cd-link\" data-act=\"delete\">delete</button></span>"));
    let data = cd_data(&html);
    assert_eq!(data["me"], "rowan");
    assert_eq!(data["me_name"], "Rowan Vale");
    assert_eq!(data["owner"], true);
    assert_eq!(data["can_comment"], true);
    assert_eq!(data["can_act"], true);
    assert_eq!(data["deleted"], false);
    assert_eq!(data["md_hash"], sha256_hex(V1.as_bytes()));
    assert_eq!(data["assets"], json!([]));
    assert_eq!(data["actions"][0]["id"], "approve");
    assert_eq!(data["actions"][1]["intent"], "neutral");
    assert!(data["action"].is_null());
    // The status line and the resolved count keep ticket 10's shape (the script rebuilds them).
    assert!(html.contains("<span class=\"cd-status\"><b>v1</b> · no action yet · <b>1</b> open</span>"));
    assert!(html.contains("<span>resolved (<span class=\"cd-res-n\">0</span>)</span>"));

    // ---- a commenter (no `action`): pill and Thread controls, but no Action buttons; not the owner ⇒ no tools on the owner's Comment ----
    let (_, html) = get(&ts, &commenter, &format!("/d/{id}"));
    assert!(html.contains("id=\"cd-pill\""));
    assert!(!html.contains("id=\"cd-actbtns\""), "no action right ⇒ no buttons");
    let c = card(&html, "t1");
    assert!(c.contains("data-act=\"reply\""));
    assert!(!c.contains("data-act=\"edit\""), "{c}");
    let data = cd_data(&html);
    assert_eq!(data["can_comment"], true);
    assert_eq!(data["can_act"], false);
    assert_eq!(data["owner"], false);

    // ---- a reader: nothing to press ----
    let (_, html) = get(&ts, &reader, &format!("/d/{id}"));
    assert!(!html.contains("id=\"cd-pill\""));
    assert!(!html.contains("id=\"cd-actbtns\""));
    assert!(!card(&html, "t1").contains("data-act="));
    assert_eq!(cd_data(&html)["can_comment"], false);

    // ---- after an Action and a push: the chip, and an older-Version view without pill / buttons ----
    let (status, _) = ts.post(&format!("/api/v1/docs/{id}/actions"), Some(&owner), &json!({ "action": "approve" }));
    assert_eq!(status, 201);
    let (_, html) = get(&ts, &owner, &format!("/d/{id}"));
    let data = cd_data(&html);
    assert_eq!(data["action"]["id"], "approve");
    assert_eq!(data["action"]["by"], "rowan");
    assert_eq!(data["action"]["version"], 1);
    assert_eq!(push(&ts, &owner, &id, "plan", V2), 2);
    let (_, v1) = get(&ts, &owner, &format!("/d/{id}/v/1"));
    assert!(!v1.contains("id=\"cd-pill\""), "no pill on an older Version");
    assert!(!v1.contains("id=\"cd-actbtns\""), "no Action buttons on an older Version");
    assert!(v1.contains("data-action=\"approve\" data-version=\"1\">Approve"), "its own chip stays");
    assert!(card(&v1, "t1").contains("data-act=\"reply\""), "replies work from any Version view");
    let d1 = cd_data(&v1);
    assert_eq!(d1["version"], 1);
    assert_eq!(d1["head"], 2);
    assert_eq!(d1["md_hash"], sha256_hex(V2.as_bytes()), "the head's Markdown (the selection maps against head)");

    // ---- `[]` declaration ⇒ no buttons; a deleted Document ⇒ no controls ----
    let none = create(&ts, &owner, "none", V1, json!({ "actions_available": [] }));
    let (_, html) = get(&ts, &owner, &format!("/d/{none}"));
    assert!(!html.contains("id=\"cd-actbtns\""));
    assert_eq!(cd_data(&html)["actions"], json!([]));
    let r = ureq::delete(format!("{}/api/v1/docs/{none}", ts.url))
        .header("Authorization", format!("Bearer {owner}"))
        .call()
        .unwrap();
    assert!(r.status().is_success());
    let (_, html) = get(&ts, &owner, &format!("/d/{none}"));
    assert!(!html.contains("id=\"cd-pill\""));
    assert_eq!(cd_data(&html)["can_comment"], false);
    assert_eq!(cd_data(&html)["deleted"], true);

    // ---- an Asset in the head manifest is listed for the media pill ----
    let png = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/e2e/48-reanchor-seed/seed.in/img/architecture.png"
    ))
    .unwrap();
    let ph = upload(&ts, &owner, &png);
    let md = "# Img\n\n![a](img/architecture.png)\n";
    let mh = upload(&ts, &owner, md.as_bytes());
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&owner),
        &json!({ "vault": "notes", "name": "img", "md_hash": mh, "manifest": [{ "ref": "img/architecture.png", "hash": ph, "size": png.len(), "type": "image/png" }] }),
    );
    assert_eq!(status, 201, "{doc}");
    let (_, html) = get(&ts, &owner, &format!("/d/{}", doc["id"].as_str().unwrap()));
    let assets = cd_data(&html)["assets"].clone();
    assert_eq!(assets[0]["ref"], "img/architecture.png");
    assert_eq!(assets[0]["hash"], ph);
}
