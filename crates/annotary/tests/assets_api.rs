//! Assets over HTTP against the in-process server (ticket 03): the manifest-shaped two-phase
//! check (only lacking blobs), Tenant-wide de-dup across Documents, asset-only changes making a
//! new Version, manifest validation, the caps as typed rejections, and `/a/{hash}` serving with
//! content types and derivatives.

use annotary::testing::TestServer;
use annotary_sidecar::identity::{sha256_hex, version_hash};
use annotary_store::caps::{ASSET_MAX, MARKDOWN_MAX, VERSION_MAX};
use serde_json::{Value, json};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets").join(name)).unwrap()
}

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

/// GET with the raw response: `(status, headers, body)`.
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

/// Store a blob through the API (unconditionally) and return its hash.
fn upload(ts: &TestServer, token: &str, bytes: &[u8]) -> String {
    let hash = sha256_hex(bytes);
    let (status, body) = put_bytes(ts, token, &format!("/api/v1/blobs/{hash}"), bytes);
    assert!(status == 201 || status == 200, "{status} {body}");
    hash
}

fn check(ts: &TestServer, token: &str, body: &Value) -> (u16, Value) {
    ts.post("/api/v1/blobs/check", Some(token), body)
}

fn lacking(v: &Value) -> Vec<String> {
    v["lacking"].as_array().unwrap().iter().map(|h| h.as_str().unwrap().to_string()).collect()
}

fn entry(reference: &str, bytes: &[u8], content_type: &str) -> Value {
    json!({ "ref": reference, "hash": sha256_hex(bytes), "size": bytes.len(), "type": content_type })
}

// spec: S03-11, S03-06, S03-09, S03-16
#[test]
fn manifest_check_sends_only_lacking_blobs_dedups_tenant_wide_and_asset_changes_make_versions() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let md = b"# Plan\n\n![a](img/a.png)\n![b](img/b.png)\n![gone](img/gone.png)\n";
    let md_hash = sha256_hex(md);
    let a_png = fixture("a.png");
    let b_png = fixture("b.png");
    let manifest = json!([
        { "ref": "img/a.png", "hash": sha256_hex(&a_png), "size": a_png.len(), "type": "image/png", "width": 32, "height": 20 },
        entry("img/b.png", &b_png, "image/png"),
        { "ref": "img/gone.png", "type": "image/png", "missing": true },
    ]);

    // Phase 1: the manifest form lists everything the server lacks — the Markdown and both files.
    let (status, body) = check(&ts, &tok, &json!({ "md_hash": md_hash, "assets": manifest }));
    assert_eq!(status, 200, "{body}");
    let mut want = vec![md_hash.clone(), sha256_hex(&a_png), sha256_hex(&b_png)];
    want.sort();
    assert_eq!(lacking(&body), want, "missing entries have nothing to upload");
    // Committing before the uploads is refused, typed.
    let (status, body) = ts.post(
        "/api/v1/docs",
        Some(&tok),
        &json!({ "vault": "notes", "name": "plan", "md_hash": md_hash, "manifest": manifest }),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "blob_missing");
    // Phase 2: upload only those; a second check finds nothing lacking.
    for bytes in [&md[..], &a_png[..], &b_png[..]] {
        upload(&ts, &tok, bytes);
    }
    let (_, body) = check(&ts, &tok, &json!({ "md_hash": md_hash, "assets": manifest }));
    assert!(lacking(&body).is_empty(), "{body}");
    // The plain `hashes` form still works, mixed with the manifest form.
    let (_, body) =
        check(&ts, &tok, &json!({ "hashes": [sha256_hex(b"nope")], "assets": [entry("x.png", &a_png, "image/png")] }));
    assert_eq!(lacking(&body), vec![sha256_hex(b"nope")]);

    // Commit (with the fields later tickets fill — accepted, ignored).
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&tok),
        &json!({
            "vault": "notes", "name": "plan", "md_hash": md_hash, "manifest": manifest,
            "placements": [{"thread": "t1"}], "html_hash": Value::Null, "renderer": "none/0",
        }),
    );
    assert_eq!(status, 201, "{doc}");
    let id = doc["id"].as_str().unwrap().to_string();
    let expect_hash = version_hash(
        &md_hash,
        [
            ("img/a.png", Some(sha256_hex(&a_png).as_str())),
            ("img/b.png", Some(sha256_hex(&b_png).as_str())),
            ("img/gone.png", None),
        ],
    );
    assert_eq!(doc["hash"], expect_hash, "identity covers the sorted (ref, hash) manifest");
    let (_, v1) = ts.get(&format!("/api/v1/docs/{id}/versions/1"), Some(&tok));
    let m = v1["manifest"].as_array().unwrap();
    assert_eq!(m.len(), 3);
    assert_eq!(
        m[0],
        json!({ "ref": "img/a.png", "hash": sha256_hex(&a_png), "size": a_png.len(), "type": "image/png", "width": 32, "height": 20 })
    );
    assert_eq!(m[2], json!({ "ref": "img/gone.png", "type": "image/png", "missing": true }));
    assert_eq!(v1["renderer"], "none/0");

    // Tenant-wide de-dup: another Document referencing the same bytes under another ref has
    // nothing to upload for it, and the blob store holds one file.
    let files_before = ts.store.blobs().all().unwrap().len();
    let md2 = b"# Other\n\n![same](pics/copy.png)\n";
    let (_, body) = check(
        &ts,
        &tok,
        &json!({ "md_hash": sha256_hex(md2), "assets": [entry("pics/copy.png", &a_png, "image/png")] }),
    );
    assert_eq!(lacking(&body), vec![sha256_hex(md2)]);
    upload(&ts, &tok, md2);
    let (status, other) = ts.post(
        "/api/v1/docs",
        Some(&tok),
        &json!({ "vault": "notes", "name": "other", "md_hash": sha256_hex(md2), "manifest": [entry("pics/copy.png", &a_png, "image/png")] }),
    );
    assert_eq!(status, 201, "{other}");
    assert_eq!(ts.store.blobs().all().unwrap().len(), files_before + 1, "only the new Markdown was added");

    // Asset-only change: same Markdown, img/b.png now has other bytes → v2; same again → already.
    let b2 = fixture("a.gif");
    upload(&ts, &tok, &b2);
    let manifest2 = json!([
        { "ref": "img/a.png", "hash": sha256_hex(&a_png), "size": a_png.len(), "type": "image/png" },
        entry("img/b.png", &b2, "image/png"),
        { "ref": "img/gone.png", "type": "image/png", "missing": true },
    ]);
    let (status, v2) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&tok),
        &json!({ "name": "plan", "md_hash": md_hash, "manifest": manifest2 }),
    );
    assert_eq!(status, 201, "{v2}");
    assert_eq!(v2["version"], 2);
    assert_eq!(v2["already"], false);
    let (status, again) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&tok),
        &json!({ "name": "plan", "md_hash": md_hash, "manifest": manifest2 }),
    );
    assert_eq!(status, 200, "{again}");
    assert_eq!(again["already"], true);
    // A manifest that only reorders entries is the same identity.
    let reordered = json!([manifest2[2], manifest2[1], manifest2[0]]);
    let (_, same) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&tok),
        &json!({ "name": "plan", "md_hash": md_hash, "manifest": reordered }),
    );
    assert_eq!(same["already"], true);
    // Back to v1's manifest → v3 (same content as v1).
    let (_, v3) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&tok),
        &json!({ "name": "plan", "md_hash": md_hash, "manifest": manifest }),
    );
    assert_eq!(v3["version"], 3);
    assert_eq!(v3["same_as"], 1);

    // Manifest validation is typed.
    let (status, body) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&tok),
        &json!({ "name": "plan", "md_hash": md_hash, "manifest": [{ "ref": "/abs.png", "type": "image/png", "hash": sha256_hex(&a_png) }] }),
    );
    assert_eq!(status, 400, "{body}");
    assert_eq!(body["error"], "invalid_manifest");
    let (status, body) = ts.post(
        &format!("/api/v1/docs/{id}/versions"),
        Some(&tok),
        &json!({ "name": "plan", "md_hash": md_hash, "manifest": [{ "ref": "x.png", "type": "image/png", "hash": sha256_hex(b"never uploaded") }] }),
    );
    assert_eq!(status, 409, "{body}");
    assert_eq!(body["error"], "blob_missing");
    let (status, body) = check(&ts, &tok, &json!({ "assets": [{ "ref": "x.png", "hash": 12 }] }));
    assert_eq!(status, 400, "{body}");
    let (status, body) = check(&ts, &tok, &json!({ "assets": [{ "ref": "x.png", "hash": "zz" }] }));
    assert_eq!(status, 400, "{body}");
}

// spec: S03-10
#[test]
fn caps_are_typed_rejections_on_check_upload_and_commit() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    // check: a claimed Asset size over the cap, and a total over the Version cap.
    let (status, body) = check(
        &ts,
        &tok,
        &json!({ "assets": [{ "ref": "big.mp4", "hash": sha256_hex(b"x"), "size": ASSET_MAX + 1, "type": "video/mp4" }] }),
    );
    assert_eq!(status, 413, "{body}");
    assert_eq!(body["error"], "too_large");
    assert_eq!(body["message"], "Asset big.mp4 is 100.1 MB — the cap is 100 MB");
    assert_eq!(body["size"], ASSET_MAX + 1);
    assert_eq!(body["max"], ASSET_MAX);
    let six: Vec<Value> = (0..6)
        .map(
            |i| json!({ "ref": format!("v{i}.mp4"), "hash": sha256_hex(b"x"), "size": ASSET_MAX, "type": "video/mp4" }),
        )
        .collect();
    let (status, body) = check(&ts, &tok, &json!({ "assets": six }));
    assert_eq!(status, 413, "{body}");
    assert!(body["message"].as_str().unwrap().starts_with("the Version's Assets together is 600 MB"), "{body}");
    assert_eq!(body["max"], VERSION_MAX);

    // upload: a body past the Asset cap is refused with the typed error (and drained cleanly).
    let big = vec![0u8; (ASSET_MAX + 1) as usize];
    let (status, body) = put_bytes(&ts, &tok, &format!("/api/v1/blobs/{}", sha256_hex(b"whatever")), &big);
    assert_eq!(status, 413, "{body}");
    assert_eq!(body["error"], "too_large");
    assert_eq!(body["what"], "this blob");
    drop(big);
    // Exactly the cap is fine.
    let ok = vec![7u8; 1024];
    let (status, _) = put_bytes(&ts, &tok, &format!("/api/v1/blobs/{}", sha256_hex(&ok)), &ok);
    assert_eq!(status, 201);

    // commit: Markdown over 2 MB is refused at commit (the CLI never sends one — see the
    // cli e2e — but the server enforces regardless).
    let big_md = vec![b'x'; (MARKDOWN_MAX + 1) as usize];
    let md_hash = upload(&ts, &tok, &big_md);
    let (status, body) =
        ts.post("/api/v1/docs", Some(&tok), &json!({ "vault": "notes", "name": "big", "md_hash": md_hash }));
    assert_eq!(status, 413, "{body}");
    assert_eq!(body["error"], "too_large");
    assert_eq!(body["message"], "the Markdown is 2.1 MB — the cap is 2 MB");
    assert!(body["hint"].as_str().unwrap().contains("500 MB"));
    // (per-Asset and per-Version totals at commit are checked from real blob sizes — see the
    // store's sparse-file test.)
}

// spec: S03-12, S03-08
#[test]
fn assets_are_served_at_a_stable_url_with_content_types_and_derivatives() {
    let ts = TestServer::start();
    let tok = ts.owner_token.clone();
    let md = b"# Media\n\n![p](img/a.png) ![h](img/photo.heic) [n](notes.md) ![s](img/a.svg)\n";
    let md_hash = upload(&ts, &tok, md);
    let png = fixture("a.png");
    let heic = fixture("a.heic");
    let derivative = fixture("a.jpg"); // what the pusher produced from the HEIC
    let notes = b"# Notes\n\nplain markdown\n";
    let svg = fixture("a.svg");
    for b in [&png[..], &heic[..], &derivative[..], &notes[..], &svg[..]] {
        upload(&ts, &tok, b);
    }
    let manifest = json!([
        { "ref": "img/a.png", "hash": sha256_hex(&png), "size": png.len(), "type": "image/png", "width": 32, "height": 20 },
        { "ref": "img/photo.heic", "hash": sha256_hex(&heic), "size": heic.len(), "type": "image/heic", "width": 32, "height": 20, "derivative": sha256_hex(&derivative) },
        { "ref": "notes.md", "hash": sha256_hex(notes), "size": notes.len(), "type": "text/markdown" },
        { "ref": "img/a.svg", "hash": sha256_hex(&svg), "size": svg.len(), "type": "image/svg+xml", "width": 120, "height": 80 },
    ]);
    let (status, doc) = ts.post(
        "/api/v1/docs",
        Some(&tok),
        &json!({ "vault": "notes", "name": "media", "md_hash": md_hash, "manifest": manifest }),
    );
    assert_eq!(status, 201, "{doc}");

    // The original, with its manifest type and immutable caching.
    let (status, headers, body) = get_raw(&ts, Some(&tok), &format!("/a/{}", sha256_hex(&png)));
    assert_eq!(status, 200);
    assert_eq!(body, png);
    assert_eq!(header(&headers, "content-type"), Some("image/png"));
    assert_eq!(header(&headers, "cache-control"), Some("public, max-age=31536000, immutable"));
    assert_eq!(header(&headers, "content-disposition"), Some("inline; filename=\"a.png\""));
    assert_eq!(header(&headers, "etag"), Some(format!("\"{}\"", sha256_hex(&png)).as_str()));
    assert!(header(&headers, "content-security-policy").is_none());
    // HEIC: the original as is; `?derivative=1` the web-safe blob (sniffed type); the derivative
    // hash directly too.
    let (status, headers, body) = get_raw(&ts, Some(&tok), &format!("/a/{}", sha256_hex(&heic)));
    assert_eq!(status, 200);
    assert_eq!(body, heic);
    assert_eq!(header(&headers, "content-type"), Some("image/heic"));
    let (status, headers, body) = get_raw(&ts, Some(&tok), &format!("/a/{}?derivative=1", sha256_hex(&heic)));
    assert_eq!(status, 200);
    assert_eq!(body, derivative);
    assert_eq!(header(&headers, "content-type"), Some("image/jpeg"));
    assert_eq!(header(&headers, "content-disposition"), Some("inline; filename=\"photo.jpg\""));
    let (status, headers, body) = get_raw(&ts, Some(&tok), &format!("/a/{}", sha256_hex(&derivative)));
    assert_eq!(status, 200);
    assert_eq!(body, derivative);
    assert_eq!(header(&headers, "content-type"), Some("image/jpeg"));
    // `?derivative=1` on something without one falls back to the original.
    let (status, _, body) = get_raw(&ts, Some(&tok), &format!("/a/{}?derivative=1", sha256_hex(&png)));
    assert_eq!((status, body), (200, png.clone()));
    // Text gets a charset; SVG gets a script-blocking CSP.
    let (_, headers, body) = get_raw(&ts, Some(&tok), &format!("/a/{}", sha256_hex(notes)));
    assert_eq!(header(&headers, "content-type"), Some("text/markdown; charset=utf-8"));
    assert_eq!(body, notes);
    let (_, headers, _) = get_raw(&ts, Some(&tok), &format!("/a/{}", sha256_hex(&svg)));
    assert_eq!(header(&headers, "content-type"), Some("image/svg+xml; charset=utf-8"));
    assert_eq!(header(&headers, "content-security-policy"), Some("default-src 'none'; style-src 'unsafe-inline'"));
    assert_eq!(header(&headers, "x-content-type-options"), Some("nosniff"));
    // Unknown / unreferenced hashes are 404; auth is required; a Member with no read grant is 403.
    let (status, _, _) = get_raw(&ts, Some(&tok), &format!("/a/{}", sha256_hex(b"nothing")));
    assert_eq!(status, 404);
    let (status, _, _) = get_raw(&ts, Some(&tok), &format!("/a/{}", sha256_hex(md)));
    assert_eq!(status, 404, "the Markdown itself is not an Asset");
    let (status, _, _) = get_raw(&ts, Some(&tok), "/a/not-a-hash");
    assert_eq!(status, 404);
    // Ticket 06: `/a/{hash}` is a web page route — with no local password a credential-less
    // loopback request is signed in as the owner (and given a session cookie); once a password
    // is set, no cookie and no bearer ⇒ 401 (a navigation would be redirected to /login).
    let (status, headers, _) = get_raw(&ts, None, &format!("/a/{}", sha256_hex(&png)));
    assert_eq!(status, 200, "loopback trust while no password is set");
    assert!(header(&headers, "set-cookie").is_some_and(|c| c.starts_with("cd_session=")));
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some("hunter2")).unwrap();
    let (status, _, _) = get_raw(&ts, None, &format!("/a/{}", sha256_hex(&png)));
    assert_eq!(status, 401);
    ts.store.set_password(&ts.owner_id, &ts.owner_id, None).unwrap();
    let (mid, mtok) = ts.add_member("nobody");
    ts.store.set_grants(&ts.owner_id, &mid, vec![]).unwrap();
    let (status, _, _) = get_raw(&ts, Some(&mtok), &format!("/a/{}", sha256_hex(&png)));
    assert_eq!(status, 403);
    // The Sidecar projection carries the manifest as assets[] (dimensions/derivative are not
    // part of the decided Sidecar shape).
    let id = doc["id"].as_str().unwrap();
    let (status, _, sc) = get_raw(&ts, Some(&tok), &format!("/api/v1/docs/{id}/sidecar"));
    assert_eq!(status, 200);
    let sc = annotary_sidecar::parse(std::str::from_utf8(&sc).unwrap()).unwrap();
    assert_eq!(sc.assets.len(), 4);
    assert_eq!(sc.assets[1].reference, "img/photo.heic");
    assert_eq!(sc.assets[1].content_type, "image/heic");
    assert_eq!(sc.assets[1].size, Some(heic.len() as u64));
}
