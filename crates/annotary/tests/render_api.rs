//! Rendered HTML over HTTP against the in-process server (ticket 09): a commit carries
//! `html_hash` + `renderer` and the server stores them without rendering; the raw HTML endpoint
//! serves the blob; a Version without HTML (or with a too-old renderer) is rendered by the
//! server on first read and cached; `rerender` replaces HTML only; the html blob is GC-safe and
//! purged with its Document; rights.

use annotary::testing::TestServer;
use annotary_renderer::{Asset, MIN_RENDERER_VERSION, RENDERER_VERSION, render_html};
use annotary_sidecar::identity::sha256_hex;
use annotary_store::{Grant, Right};
use serde_json::{Value, json};

fn put_bytes(ts: &TestServer, token: &str, path: &str, bytes: &[u8]) -> (u16, Value) {
    let res = ureq::put(format!("{}{}", ts.url, path))
        .header("Authorization", format!("Bearer {token}"))
        .content_type("application/octet-stream")
        .config()
        .http_status_as_error(false)
        .build()
        .send(bytes);
    match res {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let text = r.body_mut().read_to_string().unwrap_or_default();
            (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
        }
        Err(e) => (0, Value::String(e.to_string())),
    }
}

fn get_raw(ts: &TestServer, token: Option<&str>, path: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut req = ureq::get(format!("{}{}", ts.url, path));
    if let Some(t) = token {
        req = req.header("Authorization", format!("Bearer {t}"));
    }
    let mut r = req.config().http_status_as_error(false).build().call().unwrap();
    let status = r.status().as_u16();
    let headers: Vec<(String, String)> = r
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_string()))
        .collect();
    (status, headers, r.body_mut().read_to_vec().unwrap())
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

fn upload(ts: &TestServer, token: &str, bytes: &[u8]) -> String {
    let hash = sha256_hex(bytes);
    let (status, body) = put_bytes(ts, token, &format!("/api/v1/blobs/{hash}"), bytes);
    assert!(status == 201 || status == 200, "{status} {body}");
    hash
}

const MD: &[u8] = b"# Plan\n\nA paragraph with **bold** and a [link](https://x.y).\n\n- one\n- two\n";

/// The client-side render of `MD` with no Assets — what `push` would upload.
fn client_html() -> String {
    render_html(std::str::from_utf8(MD).unwrap(), &[])
}

// spec: S09-08, S09-09
#[test]
fn commit_stores_the_clients_render_and_the_html_endpoint_serves_it() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let md_hash = upload(&ts, &tok, MD);
    let html = client_html();
    // Phase 1 lists the html blob as lacking; phase 2 uploads it.
    let (_, body) = ts.post(
        "/api/v1/blobs/check",
        Some(&tok),
        &json!({ "md_hash": md_hash, "assets": [], "html_hash": sha256_hex(html.as_bytes()) }),
    );
    let lacking: Vec<&str> = body["lacking"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(lacking, vec![sha256_hex(html.as_bytes())]);
    // A commit naming an html blob the store lacks is refused (409 blob_missing) …
    let (status, body) = ts.post(
        "/api/v1/docs",
        Some(&tok),
        &json!({ "vault": "notes", "name": "plan", "md_hash": md_hash, "html_hash": sha256_hex(html.as_bytes()), "renderer": RENDERER_VERSION }),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "blob_missing");
    // … a malformed html_hash or renderer is 400 …
    let (status, body) = ts.post(
        "/api/v1/docs",
        Some(&tok),
        &json!({ "vault": "notes", "name": "plan", "md_hash": md_hash, "html_hash": "nope", "renderer": RENDERER_VERSION }),
    );
    assert_eq!(status, 400, "{body}");
    let (status, body) = ts.post(
        "/api/v1/docs",
        Some(&tok),
        &json!({ "vault": "notes", "name": "plan", "md_hash": md_hash, "renderer": "not a version" }),
    );
    assert_eq!(status, 400, "{body}");
    // … and with the blob uploaded the commit stores exactly what the client sent.
    let html_hash = upload(&ts, &tok, html.as_bytes());
    let events_before = ts.store.last_seq().unwrap();
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&tok),
        &json!({ "vault": "notes", "name": "plan", "md_hash": md_hash, "html_hash": html_hash, "renderer": RENDERER_VERSION }),
    );
    assert_eq!(status, 201, "{doc}");
    let id = doc["id"].as_str().unwrap().to_string();
    let (_, v1) = ts.get(&format!("/api/v1/docs/{id}/versions/1"), Some(&tok));
    assert_eq!(v1["html_hash"], html_hash);
    assert_eq!(v1["renderer"], RENDERER_VERSION);
    let (_, list) = ts.get(&format!("/api/v1/docs/{id}/versions"), Some(&tok));
    assert_eq!(list["versions"][0]["html_hash"], html_hash);
    // The raw endpoint serves the blob as HTML, with the hash as ETag.
    let (status, headers, body) = get_raw(&ts, Some(&tok), &format!("/api/v1/docs/{id}/versions/1/html"));
    assert_eq!(status, 200);
    assert_eq!(header(&headers, "content-type"), Some("text/html; charset=utf-8"));
    assert_eq!(header(&headers, "etag"), Some(format!("\"{html_hash}\"").as_str()));
    assert_eq!(header(&headers, "x-annotary-renderer"), Some(RENDERER_VERSION));
    assert_eq!(body, html.as_bytes());
    // The server did not render anything on push or on this read: no version.rendered event.
    let events = ts.store.events_since(events_before).unwrap();
    assert!(
        events.iter().all(|e| e.kind != "version.rendered"),
        "{:?}",
        events.iter().map(|e| &e.kind).collect::<Vec<_>>()
    );
    // Unknown Version / Document, and no credential.
    let (status, _, _) = get_raw(&ts, Some(&tok), &format!("/api/v1/docs/{id}/versions/9/html"));
    assert_eq!(status, 404);
    let (status, _, _) = get_raw(&ts, Some(&tok), "/api/v1/docs/zzzzzzzz/versions/1/html");
    assert_eq!(status, 404);
    let (status, _, _) = get_raw(&ts, None, &format!("/api/v1/docs/{id}/versions/1/html"));
    assert_eq!(status, 401);
}

// spec: S09-10, S09-07
#[test]
fn a_version_without_html_or_with_an_old_renderer_is_rendered_by_the_server_and_cached() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let md_hash = upload(&ts, &tok, MD);
    // An old client: no render fields at all.
    let (status, doc) =
        ts.post("/api/v1/docs", Some(&tok), &json!({ "vault": "notes", "name": "plan", "md_hash": md_hash }));
    assert_eq!(status, 201, "{doc}");
    let id = doc["id"].as_str().unwrap().to_string();
    let (_, v1) = ts.get(&format!("/api/v1/docs/{id}/versions/1"), Some(&tok));
    assert!(v1["html_hash"].is_null() && v1["renderer"].is_null());
    let seq0 = ts.store.last_seq().unwrap();
    // First read: the server renders with the same crate, stores the blob, records it.
    let (status, headers, body) = get_raw(&ts, Some(&tok), &format!("/api/v1/docs/{id}/versions/1/html"));
    assert_eq!(status, 200);
    let expected = client_html();
    assert_eq!(String::from_utf8_lossy(&body), expected, "server fallback render == CLI render");
    assert_eq!(header(&headers, "x-annotary-renderer"), Some(RENDERER_VERSION));
    let (_, v1) = ts.get(&format!("/api/v1/docs/{id}/versions/1"), Some(&tok));
    assert_eq!(v1["html_hash"], sha256_hex(expected.as_bytes()));
    assert_eq!(v1["renderer"], RENDERER_VERSION);
    let events = ts.store.events_since(seq0).unwrap();
    assert_eq!(events.iter().filter(|e| e.kind == "version.rendered").count(), 1);
    // Second read: served from the cache, no new event, same bytes.
    let (_, _, again) = get_raw(&ts, Some(&tok), &format!("/api/v1/docs/{id}/versions/1/html"));
    assert_eq!(again, body);
    assert_eq!(ts.store.last_seq().unwrap(), events.last().unwrap().seq);

    // A too-old renderer: the client's HTML is replaced on read; a current one is served as-is.
    let stale = upload(&ts, &tok, b"<p data-l=\"1-1\">stale render</p>\n");
    let (status, _) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&tok),
        &json!({ "name": "plan", "md_hash": md_hash, "manifest": [{"ref": "x.png", "type": "image/png", "missing": true}], "html_hash": stale, "renderer": "0.0.1" }),
    );
    assert_eq!(status, 201);
    let (_, _, body) = get_raw(&ts, Some(&tok), &format!("/api/v1/docs/{id}/versions/2/html"));
    assert_ne!(body, b"<p data-l=\"1-1\">stale render</p>\n");
    let (_, v2) = ts.get(&format!("/api/v1/docs/{id}/versions/2"), Some(&tok));
    assert_eq!(v2["renderer"], RENDERER_VERSION);
    assert!(annotary_renderer::parse_version("0.0.1") < annotary_renderer::parse_version(MIN_RENDERER_VERSION));
    let newer = upload(&ts, &tok, b"<p data-l=\"1-1\">from a newer client</p>\n");
    let (status, _) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&tok),
        &json!({ "name": "plan", "md_hash": md_hash, "manifest": [{"ref": "y.png", "type": "image/png", "missing": true}], "html_hash": newer, "renderer": "99.0.0" }),
    );
    assert_eq!(status, 201);
    let (_, _, body) = get_raw(&ts, Some(&tok), &format!("/api/v1/docs/{id}/versions/3/html"));
    assert_eq!(body, b"<p data-l=\"1-1\">from a newer client</p>\n", "never downgraded on read");
}

// spec: S09-11, S09-13
#[test]
fn rerender_replaces_html_only_and_needs_write_or_owner() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let (reader_id, reader_tok) = ts.add_member("reader");
    ts.store
        .set_grants(&ts.owner_id, &reader_id, vec![Grant { vault: "*".into(), rights: vec![Right::Read] }])
        .unwrap();
    let (writer_id, writer_tok) = ts.add_member("writer");
    ts.store
        .set_grants(
            &ts.owner_id,
            &writer_id,
            vec![Grant { vault: "*".into(), rights: vec![Right::Read, Right::Write] }],
        )
        .unwrap();
    let md_hash = upload(&ts, &tok, MD);
    let stale = upload(&ts, &tok, b"<p>stale</p>\n");
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&tok),
        &json!({ "vault": "notes", "name": "plan", "md_hash": md_hash, "html_hash": stale, "renderer": RENDERER_VERSION }),
    );
    assert_eq!(status, 201, "{doc}");
    let id = doc["id"].as_str().unwrap().to_string();
    let md2 = upload(&ts, &tok, b"# Plan v2\n\nchanged\n");
    let (status, _) =
        ts.post(&format!("/api/v1/docs/{id}/versions"), Some(&tok), &json!({ "name": "plan", "md_hash": md2 }));
    assert_eq!(status, 201);
    // A Thread on head, to prove it survives untouched.
    let (status, thread) = ts.post(
        &format!("/api/v1/docs/{id}/threads"),
        Some(&tok),
        &json!({ "subject": "span", "range": "1:2-1:6", "body": "keep me", "version": 2 }),
    );
    assert_eq!(status, 201, "{thread}");
    let before_versions = ts.store.versions(&id).unwrap();
    let before_threads = ts.store.threads(&id, &Default::default()).unwrap();

    // Rights: read-only ⇒ 403; write ⇒ ok; --all owner-only.
    let (status, body) = ts.post(&format!("/api/v1/docs/{id}/rerender"), Some(&reader_tok), &json!({}));
    assert_eq!(status, 403, "{body}");
    let (status, body) = ts.post("/api/v1/rerender", Some(&writer_tok), &json!({}));
    assert_eq!(status, 403, "{body}");
    let (status, body) = ts.post(&format!("/api/v1/docs/{id}/rerender"), Some(&writer_tok), &json!({}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["id"], id);
    assert_eq!(body["name"], "plan");
    assert_eq!(body["versions"], 2);
    assert_eq!(body["changed"], 2, "v1's stale HTML replaced, v2 rendered for the first time");
    assert_eq!(body["renderer"], RENDERER_VERSION);

    // HTML now equals the crate's render; everything else about the Versions and the Thread is untouched.
    let after_versions = ts.store.versions(&id).unwrap();
    for (b, a) in before_versions.iter().zip(&after_versions) {
        assert_eq!(
            (b.number, &b.hash, &b.md_hash, &b.by, &b.at, &b.name, &b.manifest),
            (a.number, &a.hash, &a.md_hash, &a.by, &a.at, &a.name, &a.manifest)
        );
        assert_eq!(a.renderer.as_deref(), Some(RENDERER_VERSION));
    }
    assert_eq!(after_versions[0].html_hash.as_deref(), Some(sha256_hex(client_html().as_bytes()).as_str()));
    let after_threads = ts.store.threads(&id, &Default::default()).unwrap();
    assert_eq!(before_threads.len(), 1);
    assert_eq!(before_threads[0].id, after_threads[0].id);
    assert_eq!(before_threads[0].anchor, after_threads[0].anchor);
    let (_, _, body) = get_raw(&ts, Some(&tok), &format!("/api/v1/docs/{id}/versions/1/html"));
    assert_eq!(String::from_utf8_lossy(&body), client_html());

    // A second rerender changes nothing (idempotent); the old stale blob is now unreferenced.
    let (_, body) = ts.post(&format!("/api/v1/docs/{id}/rerender"), Some(&tok), &json!({}));
    assert_eq!(body["changed"], 0);
    let removed = ts.store.gc_blobs(std::time::Duration::ZERO).unwrap();
    assert!(removed.contains(&stale), "{removed:?}");
    assert!(
        !removed.iter().any(|h| after_versions.iter().any(|v| v.html_hash.as_deref() == Some(h))),
        "live html blobs survive GC"
    );

    // --all covers every Document, deleted ones too.
    let md3 = upload(&ts, &tok, b"# Other\n");
    let (status, other) =
        ts.post("/api/v1/docs", Some(&tok), &json!({ "vault": "notes", "name": "other", "md_hash": md3 }));
    assert_eq!(status, 201);
    let other_id = other["id"].as_str().unwrap().to_string();
    let (status, _) = ts.post(&format!("/api/v1/docs/{other_id}/rerender"), Some(&tok), &json!({}));
    assert_eq!(status, 200);
    let res = ureq::delete(format!("{}/api/v1/docs/{other_id}", ts.url))
        .header("Authorization", format!("Bearer {tok}"))
        .call();
    assert!(res.is_ok());
    let (status, body) = ts.post("/api/v1/rerender", Some(&tok), &json!({}));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["documents"], 2);
    assert_eq!(body["versions"], 3);
    assert_eq!(body["changed"], 0);
    assert_eq!(body["docs"].as_array().unwrap().len(), 2);
    // Purge removes the html blobs with the Document.
    let html_of_other = ts.store.versions(&other_id).unwrap()[0].html_hash.clone().unwrap();
    let (status, body) = ts.post(&format!("/api/v1/docs/{other_id}/purge"), Some(&tok), &json!({}));
    assert_eq!(status, 200, "{body}");
    assert!(!ts.store.has_blob(&html_of_other));
}

// spec: S09-10
#[test]
fn server_fallback_render_equals_the_client_render_for_the_corpus_with_assets() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/render");
    let mut names: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md") && p.file_name().is_some_and(|n| n != "README.md"))
        .collect();
    names.sort();
    assert!(names.len() >= 15);
    for path in names {
        let md = std::fs::read(&path).unwrap();
        let text = String::from_utf8_lossy(&md).into_owned();
        // A manifest whose entries point at real (tiny) blobs, so the commit validates.
        let mut manifest = Vec::new();
        if let Ok(m) = std::fs::read_to_string(path.with_extension("manifest.json")) {
            let v: Value = serde_json::from_str(&m).unwrap();
            for e in v.as_array().unwrap() {
                let mut e = e.clone();
                if e.get("missing").and_then(Value::as_bool) != Some(true) {
                    let bytes = format!("blob for {}", e["ref"]).into_bytes();
                    e["hash"] = json!(upload(&ts, &tok, &bytes));
                    e["size"] = json!(bytes.len());
                    if e.get("derivative").is_some() {
                        e["derivative"] = json!(upload(&ts, &tok, format!("derivative for {}", e["ref"]).as_bytes()));
                    }
                }
                manifest.push(e);
            }
        }
        let name = path.file_stem().unwrap().to_string_lossy().to_string();
        let md_hash = upload(&ts, &tok, &md);
        // Push without a render (an old client) → the server's fallback …
        let (status, doc) = ts.post(
            "/api/v1/docs",
            Some(&tok),
            &json!({ "vault": "notes", "name": name, "md_hash": md_hash, "manifest": manifest }),
        );
        assert_eq!(status, 201, "{name}: {doc}");
        let id = doc["id"].as_str().unwrap();
        let (_, v1) = ts.get(&format!("/api/v1/docs/{id}/versions/1"), Some(&tok));
        // … must equal what the client renders from the *stored* (normalised) manifest, which
        // is what `push` computes from its own manifest: same fields, same output.
        let client = render_html(&text, &Asset::from_manifest(&v1["manifest"]));
        let client_from_sent = render_html(&text, &Asset::from_manifest(&Value::Array(manifest.clone())));
        assert_eq!(client, client_from_sent, "{name}: the sent and the normalised manifest render the same");
        let (status, _, body) = get_raw(&ts, Some(&tok), &format!("/api/v1/docs/{id}/versions/1/html"));
        assert_eq!(status, 200);
        assert_eq!(String::from_utf8_lossy(&body), client, "{name}: server fallback == client render");
    }
}
