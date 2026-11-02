//! M3b, the Annotary brand in the web app: `brand/` is what `scripts/brand/render.py` renders, the
//! server hands those files out before sign-in, and every page links them and wears the icon. The
//! icon's size and appearance in a browser are `tests/web/brand.spec.ts`.

use std::path::{Path, PathBuf};
use std::process::Command;

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;
use serde_json::{Value, json};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

// spec: S31-01, S31-02
#[test]
fn committed_brand_files_are_a_fresh_render() {
    // python3, not uv: the script is standard library only, and a contributor's `make test` should
    // need nothing more. `make brand` keeps the pinned Python.
    let out = Command::new("python3")
        .arg("scripts/brand/test_render.py")
        .current_dir(repo_root())
        .output()
        .expect("run scripts/brand/test_render.py");
    assert!(out.status.success(), "{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
}

/// Headers a Tailscale Serve request carries: never loopback-trusted (CMD-17).
const FORWARDED: (&str, &str) = ("X-Forwarded-For", "100.64.0.9");

/// A GET with no credential, forwarded by a local proxy, following no redirect.
fn anon(ts: &TestServer, path: &str) -> (u16, Vec<(String, String)>, Vec<u8>) {
    let mut r = ureq::get(format!("{}{path}", ts.url))
        .header(FORWARDED.0, FORWARDED.1)
        .header("Accept", "text/html")
        .config()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .call()
        .unwrap();
    let hs = r
        .headers()
        .iter()
        .map(|(k, v)| (k.as_str().to_ascii_lowercase(), v.to_str().unwrap_or("").to_string()))
        .collect();
    (r.status().as_u16(), hs, r.body_mut().read_to_vec().unwrap())
}

fn header<'a>(hs: &'a [(String, String)], name: &str) -> Option<&'a str> {
    hs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
}

// spec: S31-03
#[test]
fn the_icons_and_the_manifest_answer_before_sign_in() {
    let ts = TestServer::start();
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some("hunter2")).unwrap();
    // The request really is signed out: a page sends it to the login page.
    let (status, hs, _) = anon(&ts, "/");
    assert_eq!(status, 303);
    assert_eq!(header(&hs, "location"), Some("/login?next=%2F"));

    let root = repo_root().join("brand");
    for (path, file, content_type) in [
        ("/favicon.ico", "favicon.ico", "image/x-icon"),
        ("/icon.svg", "icon.svg", "image/svg+xml"),
        ("/apple-touch-icon.png", "apple-touch-icon.png", "image/png"),
        ("/icon-192.png", "icon-192.png", "image/png"),
        ("/icon-512.png", "icon-512.png", "image/png"),
        ("/icon-mask.png", "icon-mask.png", "image/png"),
    ] {
        let (status, hs, body) = anon(&ts, path);
        assert_eq!(status, 200, "{path}");
        assert_eq!(sha256_hex(&body), sha256_hex(&std::fs::read(root.join(file)).unwrap()), "{path}");
        assert_eq!(header(&hs, "content-type"), Some(content_type), "{path}");
        assert_eq!(header(&hs, "x-content-type-options"), Some("nosniff"), "{path}");
        assert_eq!(header(&hs, "cache-control"), Some("public, max-age=86400"), "{path}");
        assert_eq!(header(&hs, "set-cookie"), None, "{path}");
        let csp = header(&hs, "content-security-policy");
        if content_type == "image/svg+xml" {
            assert_eq!(csp, Some("default-src 'none'; style-src 'unsafe-inline'"));
        } else {
            assert_eq!(csp, None, "{path}");
        }
    }

    let (status, hs, body) = anon(&ts, "/manifest.webmanifest");
    assert_eq!(status, 200);
    assert_eq!(header(&hs, "content-type"), Some("application/manifest+json"));
    assert_eq!(header(&hs, "set-cookie"), None);
    let manifest: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(manifest["name"], "Annotary");
    assert_eq!(manifest["start_url"], "/");
    let icons = manifest["icons"].as_array().unwrap();
    assert_eq!(icons.len(), 3);
    assert_eq!(icons[2]["purpose"], "maskable");
    for icon in icons {
        let (status, hs, _) = anon(&ts, icon["src"].as_str().unwrap());
        assert_eq!(status, 200, "{icon}");
        assert_eq!(Some(icon["type"].as_str().unwrap()), header(&hs, "content-type"), "{icon}");
    }

    // No credential and no proxy header either, and HEAD as well as GET.
    let r = ureq::head(format!("{}/favicon.ico", ts.url)).call().unwrap();
    assert_eq!(r.status().as_u16(), 200);
}

/// A page as the owner, as HTML.
fn html(ts: &TestServer, path: &str) -> String {
    let mut r = ureq::get(format!("{}{path}", ts.url))
        .header("Authorization", format!("Bearer {}", ts.owner_token))
        .config()
        .http_status_as_error(false)
        .build()
        .call()
        .unwrap();
    r.body_mut().read_to_string().unwrap()
}

fn seeded_doc(ts: &TestServer) -> String {
    let md = "# A plan\n\nOne line.\n";
    let (hash, _) = ts.store.put_blob(md.as_bytes()).unwrap();
    let (status, doc) =
        ts.post("/api/v1/docs", Some(&ts.owner_token), &json!({ "vault": "notes", "name": "a-plan", "md_hash": hash }));
    assert_eq!(status, 201, "{doc}");
    doc["id"].as_str().unwrap().to_string()
}

// spec: S31-04
#[test]
fn every_page_links_the_icons_and_the_manifest() {
    let ts = TestServer::start();
    let id = seeded_doc(&ts);
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some("hunter2")).unwrap();
    let login = String::from_utf8(anon(&ts, "/login").2).unwrap();
    let not_found = html(&ts, "/d/nosuchdoc");
    assert!(not_found.contains("cd-simple"), "{not_found}");
    let pages = [
        ("list", html(&ts, "/")),
        ("document", html(&ts, &format!("/d/{id}"))),
        ("settings", html(&ts, "/settings")),
        ("owner settings", html(&ts, "/settings/server/members")),
        ("login", login),
        ("error", not_found),
    ];
    for (name, page) in &pages {
        let head = &page[..page.find("</head>").expect(name)];
        for link in [
            "<link rel=\"icon\" href=\"/favicon.ico\" sizes=\"32x32\">",
            "<link rel=\"icon\" href=\"/icon.svg\" type=\"image/svg+xml\">",
            "<link rel=\"apple-touch-icon\" href=\"/apple-touch-icon.png\">",
            "<link rel=\"manifest\" href=\"/manifest.webmanifest\">",
        ] {
            assert!(head.contains(link), "{name} lacks {link}");
        }
    }
}

/// What follows the header's icon on a page.
fn after_icon(page: &str) -> &str {
    let start = page.find("<header class=\"cd-hdr").expect("a header");
    let header = &page[start..];
    let line = header.find("<div class=\"cd-line\"").expect("a line");
    let line_open = line + header[line..].find('>').unwrap() + 1;
    let rest = &header[line_open..];
    assert!(rest.starts_with("<a class=\"cd-brand\" href=\"/\" aria-label=\"Documents\"><svg class=\"cd-icon\" width=\"18\" height=\"18\" aria-hidden=\"true\" "), "{rest:.200}");
    let close = rest.find("</svg></a>").unwrap();
    &rest[close + "</svg></a>".len()..]
}

// spec: S31-05
#[test]
fn the_header_starts_with_the_icon_then_the_wordmark_or_the_crumb() {
    let ts = TestServer::start();
    let id = seeded_doc(&ts);
    for path in ["/", "/settings", "/settings/appearance"] {
        let page = html(&ts, path);
        assert!(after_icon(&page).starts_with("<b class=\"cd-name\">annotary</b>"), "{path}");
    }
    let page = html(&ts, &format!("/d/{id}"));
    let rest = after_icon(&page);
    assert!(rest.starts_with("<a class=\"cd-crumb\" href=\"/\">notes</a>"), "{rest:.120}");
    let header = &rest[..rest.find("</header>").unwrap()];
    assert!(!header.contains(">annotary<"), "no wordmark on a Document page: {header:.200}");
    // The small master's dot, not the full one's.
    assert!(page.contains("r=\"12\""));
}
