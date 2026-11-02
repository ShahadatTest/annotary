//! Loopback-only binding — and, since ticket 18, the explicit `--allow-remote` opt-in, with
//! loopback trust still decided by the peer.

mod common;

use std::net::IpAddr;

use annotary::server::{BindError, check_loopback, parse_bind};
use annotary::testing::TestServer;
use common::serve::ServeGuard;

// spec: S01-01
#[test]
fn non_loopback_bind_is_refused_before_touching_the_store() {
    for bad in ["0.0.0.0", "192.168.1.10", "10.0.0.1", "::", "2001:db8::1"] {
        let ip: IpAddr = bad.parse().unwrap();
        assert!(check_loopback(ip).is_err(), "{bad} must be refused");
        assert!(parse_bind(bad).is_err(), "{bad} must be refused");
    }
    for ok in ["127.0.0.1", "127.0.0.2", "::1", "[::1]"] {
        assert!(parse_bind(ok).is_ok(), "{ok} is loopback");
    }
    assert!(parse_bind("not-an-ip").is_err());
    match TestServer::try_start("0.0.0.0".parse().unwrap()) {
        Err(BindError::NotLoopback(msg)) => assert!(msg.contains("0.0.0.0")),
        Err(other) => panic!("wrong error: {other}"),
        Ok(_) => panic!("server must not start on 0.0.0.0"),
    }
}

// spec: S01-01, S01-20
#[test]
fn loopback_bind_works_and_url_is_loopback() {
    let ts = TestServer::start();
    assert!(ts.url.starts_with("http://127.0.0.1:"), "{}", ts.url);
    let (status, body) = ts.get("/api/v1/health", None);
    assert_eq!(status, 200);
    assert_eq!(body["ok"], true);
    assert_eq!(body["url"], ts.url);
}

// ---------------------------------------------------------------- ticket 18: --allow-remote

/// A local non-loopback IPv4 address of this machine (the interface a packet to a public address
/// would leave by — no packet is sent), if there is one.
fn lan_ip() -> Option<IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("192.0.2.1:9").ok()?;
    let ip = sock.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() { None } else { Some(ip) }
}

// spec: S18-01
#[test]
fn allow_remote_accepts_any_ip_and_names_a_reachable_url() {
    use annotary::server::{parse_bind_ex, url_for};
    for ip in ["0.0.0.0", "192.168.1.10", "100.64.0.7", "::", "[fd7a::1]"] {
        assert!(parse_bind_ex(ip, false).is_err(), "{ip} needs --allow-remote");
        assert!(parse_bind_ex(ip, true).is_ok(), "{ip} with --allow-remote");
    }
    assert!(parse_bind_ex("studio.local", true).is_err(), "a host name is not an IP");
    assert_eq!(url_for("0.0.0.0:7377".parse().unwrap()), "http://127.0.0.1:7377");
    assert_eq!(url_for("[::]:7377".parse().unwrap()), "http://[::1]:7377");
    assert_eq!(url_for("100.64.0.7:7377".parse().unwrap()), "http://100.64.0.7:7377");
    // A real listener on 0.0.0.0: health answers on loopback and names the loopback URL.
    let ts = TestServer::start_remote("0.0.0.0".parse().unwrap()).expect("0.0.0.0 with allow_remote");
    assert!(ts.url.starts_with("http://127.0.0.1:"), "{}", ts.url);
    let (status, body) = ts.get("/api/v1/health", None);
    assert_eq!(status, 200);
    assert_eq!(body["url"], ts.url);
}

// spec: S18-01
#[test]
fn serve_refuses_a_remote_bind_without_allow_remote_and_warns_with_it() {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};
    let tmp = tempfile::tempdir().unwrap();
    let home = tmp.path().join("cd");
    let out = Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(["serve", "--bind", "0.0.0.0", "--port", "0"])
        .env("ANNOTARY_HOME", &home)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("refusing to bind 0.0.0.0") && err.contains("--allow-remote"), "{err}");
    assert!(!home.exists(), "nothing created");
    // With it: setup, then a listener on 0.0.0.0 and one warning line.
    let child = Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(["serve", "--bind", "0.0.0.0", "--allow-remote", "--port", "0"])
        .env("ANNOTARY_HOME", &home)
        .env("ANNOTARY_SETUP_HANDLE", "rowan")
        .env("RUST_LOG", "info")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut serve = ServeGuard::new(child);
    let mut warned = String::new();
    for line in BufReader::new(serve.take_stderr().unwrap()).lines() {
        let line = line.unwrap();
        if line.contains("WARN") {
            warned = line;
            break;
        }
    }
    serve.kill();
    assert!(warned.contains("not loopback") && warned.contains("reachable from the network"), "{warned}");
    assert!(warned.contains("local password"), "{warned}");
}

/// A web request through the router with `peer` as the connection's address (what
/// `into_make_service_with_connect_info` gives a real listener).
fn via_peer(ts: &TestServer, peer: &str, path: &str, headers: &[(&str, &str)]) -> (u16, Vec<(String, String)>, String) {
    via_peer_at(ts, &ts.url, peer, path, headers)
}

/// [`via_peer`] on a server whose own URL (`server.json`'s, what `url_for` names) is `server_url`.
fn via_peer_at(
    ts: &TestServer,
    server_url: &str,
    peer: &str,
    path: &str,
    headers: &[(&str, &str)],
) -> (u16, Vec<(String, String)>, String) {
    use axum::extract::ConnectInfo;
    use tower::ServiceExt;
    let app = annotary::server::router(annotary::server::AppState::new(
        ts.store.clone(),
        server_url.to_string(),
        "2026-10-08T00:00:00.000Z".into(),
    ));
    let mut req = axum::http::Request::builder().uri(path);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut req = req.body(axum::body::Body::empty()).unwrap();
    req.extensions_mut().insert(ConnectInfo::<std::net::SocketAddr>(peer.parse().unwrap()));
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async move {
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status().as_u16();
        let hs = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let bytes = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap().to_bytes();
        (status, hs, String::from_utf8_lossy(&bytes).to_string())
    })
}

// spec: S18-02
#[test]
fn a_non_loopback_peer_is_never_signed_in_by_loopback_trust() {
    let ts = TestServer::start();
    assert!(!ts.store.password_set().unwrap());
    let html = [("accept", "text/html")];
    // The loopback peer: signed in as the owner, cookie set (story 24 unchanged).
    let (status, hs, _) = via_peer(&ts, "127.0.0.1:50000", "/", &html);
    assert_eq!(status, 200);
    assert!(hs.iter().any(|(k, v)| k == "set-cookie" && v.starts_with("cd_session=")));
    // A Tailscale / LAN peer with no password set: a page ⇒ the login redirect, no cookie.
    for peer in ["100.64.0.9:50000", "192.168.1.20:50000", "[fd7a::9]:50000"] {
        let (status, hs, _) = via_peer(&ts, peer, "/", &html);
        assert_eq!(status, 303, "{peer}");
        assert!(hs.iter().any(|(k, v)| k == "location" && v.starts_with("/login")), "{peer}: {hs:?}");
        assert!(!hs.iter().any(|(k, _)| k == "set-cookie"), "{peer}: no session");
        // Anything else (an asset fetch, a fragment) ⇒ 401 JSON.
        let (status, _, body) = via_peer(&ts, peer, "/members", &[]);
        assert_eq!(status, 401, "{peer}: {body}");
        // The login page itself signs nobody in from there.
        let (status, hs, body) = via_peer(&ts, peer, "/login", &html);
        assert_eq!(status, 200, "{peer}");
        assert!(!hs.iter().any(|(k, _)| k == "set-cookie"), "{peer}: no session from /login");
        assert!(body.contains("annotary pair"), "{peer}: the way in, not a dead end: {body}");
    }
    // A bearer Token from the same remote peer works on pages and the API.
    let bearer = format!("Bearer {}", ts.owner_token);
    let (status, _, _) = via_peer(&ts, "100.64.0.9:50000", "/", &[("accept", "text/html"), ("authorization", &bearer)]);
    assert_eq!(status, 200);
    let (status, _, body) = via_peer(&ts, "100.64.0.9:50000", "/api/v1/me", &[("authorization", &bearer)]);
    assert_eq!(status, 200, "{body}");
}

// spec: S18-02
#[test]
fn a_bearer_token_works_over_a_non_loopback_listener() {
    let Some(ip) = lan_ip() else {
        eprintln!("no non-loopback interface here — the router-level test above covers the peer rule");
        return;
    };
    let ts = TestServer::start_remote(ip).expect("bind the LAN address with allow_remote");
    assert!(ts.url.contains(&ip.to_string()), "{}", ts.url);
    // From this machine to its LAN address the peer is that (non-loopback) address.
    let (status, _) = ts.get("/api/v1/me", None);
    assert_eq!(status, 401, "no credential, no owner");
    let (status, body) = ts.get("/api/v1/me", Some(&ts.owner_token));
    assert_eq!(status, 200, "{body}");
    assert_eq!(body["handle"], "rowan");
    // A page without a credential is not signed in by loopback trust.
    let resp = ureq::get(format!("{}/", ts.url))
        .header("Accept", "text/html")
        .config()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .call()
        .unwrap();
    assert_eq!(resp.status().as_u16(), 303);
    assert!(resp.headers().get("set-cookie").is_none());
}

// ---- CMD-14 ----
// spec: S22-01
#[test]
fn the_no_password_login_page_tells_a_remote_browser_how_to_get_in() {
    let ts = TestServer::start();
    assert!(!ts.store.password_set().unwrap());
    let html = [("accept", "text/html")];
    let old_claim = "a browser on this machine is signed in automatically";
    // Loopback stays as it was: signed in as the owner and sent on, no page at all.
    let (status, hs, _) = via_peer(&ts, "127.0.0.1:50000", "/login?next=/d/abc", &html);
    assert_eq!(status, 303);
    assert!(hs.iter().any(|(k, v)| k == "location" && v == "/d/abc"), "{hs:?}");
    assert!(hs.iter().any(|(k, v)| k == "set-cookie" && v.starts_with("cd_session=")));
    // A Tailscale / LAN peer (the relay's link lands here): the two ways in, no false claim.
    assert!(ts.url.starts_with("http://127.0.0.1:"), "{}", ts.url);
    for peer in ["100.64.0.2:50000", "192.168.1.20:50000", "[fd7a::9]:50000"] {
        let (status, hs, body) = via_peer(&ts, peer, "/login?next=/d/abc", &html);
        assert_eq!(status, 200, "{peer}");
        assert!(!hs.iter().any(|(k, _)| k == "set-cookie"), "{peer}: no session");
        assert!(!body.contains(old_claim), "{peer}: {body}");
        assert!(body.contains("This server has no local password yet"), "{peer}: {body}");
        assert!(body.contains(&format!("open <code>{}</code>", ts.url)), "{peer}: the loopback URL: {body}");
        assert!(body.contains("set a password in Settings"), "{peer}: {body}");
        assert!(body.contains("run <code>annotary pair</code>"), "{peer}: {body}");
        assert!(body.contains("class=\"cd-app cd-loginpage cd-theme-system\""), "{peer}: the shared shell");
        assert!(!body.contains("<form"), "{peer}: no password form while none is set");
    }
    // A server bound to one non-loopback address: there is no loopback URL to name.
    let (status, _, body) = via_peer_at(&ts, "http://100.64.0.7:7377", "100.64.0.2:50000", "/login", &html);
    assert_eq!(status, 200);
    assert!(body.contains("listens only on <code>http://100.64.0.7:7377</code>"), "{body}");
    assert!(body.contains("<code>--bind 0.0.0.0 --allow-remote</code>"), "{body}");
    assert!(!body.contains("open <code>http://100.64.0.7:7377</code>"), "{body}");
    assert!(body.contains("run <code>annotary pair</code>") && !body.contains(old_claim), "{body}");
    // An unspecified IPv6 bind is named by `[::1]`, a loopback URL too.
    let (_, _, body) = via_peer_at(&ts, "http://[::1]:7377", "[fd7a::9]:50000", "/login", &html);
    assert!(body.contains("open <code>http://[::1]:7377</code>"), "{body}");
}
// ---- end CMD-14 ----
