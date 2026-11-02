//! The real binary against the in-process server, for Asset scenarios that need generated files
//! or the machine's HEIC decoder: the client-side Markdown cap, `../` references pulled back to
//! their place, `pull --assets --version N`, and a HEIC derivative end to end (macOS only).

use std::path::Path;
use std::process::{Command, Output};

use annotary::assets::media::converter;
use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use annotary_store::caps::MARKDOWN_MAX;

fn run(ts: &TestServer, home: &Path, cwd: &Path, token: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(args)
        .current_dir(cwd)
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

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets").join(name)).unwrap()
}

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

// spec: S03-10
#[test]
fn markdown_over_the_cap_is_refused_before_anything_is_uploaded() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let mut big = b"# big\n".to_vec();
    big.resize(MARKDOWN_MAX as usize + 1, b'x');
    write(work.path(), "big.md", &big);
    let out = run(&ts, home.path(), work.path(), &ts.owner_token, &["push", "big.md"]);
    assert_eq!(out.status.code(), Some(5), "{}", text(&out).1);
    let (so, se) = text(&out);
    assert!(so.is_empty());
    assert_eq!(
        se.trim(),
        "error: big.md is 2.1 MB — the cap is 2 MB — caps: Markdown 2 MB, each Asset 100 MB, all Assets of a Version 500 MB"
    );
    assert!(!ts.store.has_blob(&sha256_hex(&big)), "nothing was uploaded");
    let out = run(&ts, home.path(), work.path(), &ts.owner_token, &["push", "big.md", "--json"]);
    assert_eq!(out.status.code(), Some(5));
    assert!(text(&out).1.starts_with("{\"error\":\"too_large\""), "{}", text(&out).1);
    // Exactly at the cap is fine.
    big.truncate(MARKDOWN_MAX as usize);
    write(work.path(), "big.md", &big);
    let out = run(&ts, home.path(), work.path(), &ts.owner_token, &["push", "big.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
}

// spec: S03-01, S03-13, S03-03
#[test]
fn parent_relative_references_round_trip_and_pull_assets_of_an_older_version() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "shared/logo.png", &fixture("a.png"));
    write(root, "docs/plan.md", b"# Plan\n\n![logo](../shared/logo.png)\n\n[notes](sub/notes.md)\n");
    write(root, "docs/sub/notes.md", b"# Notes\n\n![deep](../../shared/logo.png) ![own](pic.gif)\n");
    write(root, "docs/sub/pic.gif", &fixture("a.gif"));
    let out = run(&ts, home.path(), &root.join("docs"), &ts.owner_token, &["push", "plan.md"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let (so, se) = text(&out);
    assert!(so.starts_with("plan\t"), "{so}");
    assert_eq!(se.trim(), "assets: 3 referenced, 3 new (215 B)");
    let sc = std::fs::read_to_string(root.join("docs/plan.annotary")).unwrap();
    let sc = annotary_sidecar::parse(&sc).unwrap();
    let refs: Vec<(&str, Option<&str>)> = sc.assets.iter().map(|a| (a.reference.as_str(), a.via.as_deref())).collect();
    assert_eq!(
        refs,
        vec![("../shared/logo.png", None), ("sub/notes.md", None), ("sub/pic.gif", Some("sub/notes.md"))],
        "the shared logo is one entry (referenced twice, once through notes.md)"
    );

    // v2: only the gif changes (asset-only change).
    write(root, "docs/sub/pic.gif", &fixture("b.png"));
    let out = run(&ts, home.path(), &root.join("docs"), &ts.owner_token, &["push", "plan.md", "-m", "new gif"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(text(&out).0.contains("\tv2"), "{}", text(&out).0);

    // Pull v2 with assets into a fresh tree: `../shared/logo.png` lands beside `dl/docs`.
    let dl = root.join("dl/docs");
    std::fs::create_dir_all(&dl).unwrap();
    let out = run(&ts, home.path(), root, &ts.owner_token, &["pull", "plan", "--out", "dl/docs", "--assets"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(text(&out).1.trim(), "pull: 0 threads, 3 assets; 3 written, 0 unchanged");
    assert_eq!(std::fs::read(root.join("dl/shared/logo.png")).unwrap(), fixture("a.png"));
    assert_eq!(std::fs::read(root.join("dl/docs/sub/pic.gif")).unwrap(), fixture("b.png"));
    assert!(root.join("dl/docs/sub/notes.md").is_file());
    // The pulled tree pushes back as `already v2`.
    let out = run(&ts, home.path(), &dl, &ts.owner_token, &["push", "plan.md"]);
    assert!(text(&out).0.contains("already v2"), "{}", text(&out).0);

    // `--version 1 --assets`: v1's gif differs from what is on disk → refused, then --force.
    let out = run(
        &ts,
        home.path(),
        root,
        &ts.owner_token,
        &["pull", "plan", "--version", "1", "--out", "dl/docs", "--assets"],
    );
    assert_eq!(out.status.code(), Some(5), "{}", text(&out).1);
    assert!(text(&out).1.contains("1 asset file here differs from v1 (sub/pic.gif)"), "{}", text(&out).1);
    assert_eq!(std::fs::read(root.join("dl/docs/sub/pic.gif")).unwrap(), fixture("b.png"), "nothing written");
    let out = run(
        &ts,
        home.path(),
        root,
        &ts.owner_token,
        &["pull", "plan", "--version", "1", "--out", "dl/docs", "--assets", "--force"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert!(text(&out).1.contains("3 assets; 1 written, 2 unchanged"), "{}", text(&out).1);
    assert_eq!(std::fs::read(root.join("dl/docs/sub/pic.gif")).unwrap(), fixture("a.gif"));
    let sc = std::fs::read_to_string(root.join("dl/docs/plan.annotary")).unwrap();
    assert!(sc.contains("\"base_version\": 1"));
}

// spec: S03-08, S03-07, S03-12 — needs a HEIC decoder (macOS `sips` / ImageMagick); skipped elsewhere.
#[test]
fn heic_gets_a_jpeg_derivative_end_to_end() {
    if converter().is_none() {
        eprintln!("skipped: no HEIC converter on this machine");
        return;
    }
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let root = work.path();
    write(root, "photo.heic", &fixture("a.heic"));
    write(root, "trip.md", b"# Trip\n\n![photo](photo.heic)\n");
    let out = run(&ts, home.path(), root, &ts.owner_token, &["push", "trip.md", "--json", "assets,warnings"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    let (so, _) = text(&out);
    let j: serde_json::Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!(j["assets"]["referenced"], 1);
    assert_eq!(j["assets"]["new"], 1);
    assert!(
        j["assets"]["new_bytes"].as_u64().unwrap() > fixture("a.heic").len() as u64,
        "original + derivative uploaded"
    );
    assert!(j["warnings"].is_null(), "{j}");
    // The stored manifest: original hash, dimensions, derivative — both blobs in the store.
    let sc = std::fs::read_to_string(root.join("trip.annotary")).unwrap();
    let id = annotary_sidecar::Binding::from_text(&sc).unwrap().id;
    let doc = ts.store.document(&id).unwrap().unwrap();
    let e = &doc.head.manifest[0];
    assert_eq!(e["ref"], "photo.heic");
    assert_eq!(e["type"], "image/heic");
    assert_eq!(e["hash"], sha256_hex(&fixture("a.heic")));
    assert_eq!((e["width"].as_u64(), e["height"].as_u64()), (Some(32), Some(20)));
    let deriv = e["derivative"].as_str().expect("derivative hash").to_string();
    let jpeg = ts.store.read_blob(&deriv).unwrap().expect("derivative blob stored");
    assert!(jpeg.starts_with(&[0xFF, 0xD8]));
    assert_eq!(
        ts.store.read_blob(&sha256_hex(&fixture("a.heic"))).unwrap().unwrap(),
        fixture("a.heic"),
        "original untouched"
    );
    // Served: the original as image/heic, the derivative on request as image/jpeg.
    let (status, body) = ts.get(&format!("/a/{}", sha256_hex(&fixture("a.heic"))), Some(&ts.owner_token));
    assert_eq!(status, 200, "{body}");
    let mut r = ureq::get(format!("{}/a/{}?derivative=1", ts.url, sha256_hex(&fixture("a.heic"))))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .call()
        .unwrap();
    assert_eq!(r.headers().get("content-type").unwrap().to_str().unwrap(), "image/jpeg");
    assert_eq!(r.body_mut().read_to_vec().unwrap(), jpeg);
    // A pull with --assets writes the original only (derivatives are the server's business).
    let out = run(&ts, home.path(), root, &ts.owner_token, &["pull", "trip", "--out", "back", "--assets"]);
    assert_eq!(out.status.code(), Some(0), "{}", text(&out).1);
    assert_eq!(std::fs::read(root.join("back/photo.heic")).unwrap(), fixture("a.heic"));
    assert_eq!(std::fs::read_dir(root.join("back")).unwrap().count(), 3, "trip.md, trip.annotary, photo.heic");
}
