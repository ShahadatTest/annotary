//! CMD-4 through the router: repeated wrong passwords are throttled per client and for the
//! account (web form and API alike), and a cross-site form or cookie POST is refused
//! (`server/login_guard.rs`). Requests go through `router().oneshot` with the peer address set,
//! as a real listener's `into_make_service_with_connect_info` does.

use annotary::testing::TestServer;
use serde_json::{Value, json};

struct Resp {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or(Value::Null)
    }
}

/// One request through a router that shares `app`'s state, from `peer`.
fn call(
    app: &axum::Router,
    peer: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: impl Into<axum::body::Body>,
) -> Resp {
    use axum::extract::ConnectInfo;
    use tower::ServiceExt;
    let mut req = axum::http::Request::builder().method(method).uri(path);
    for (k, v) in headers {
        req = req.header(*k, *v);
    }
    let mut req = req.body(body.into()).unwrap();
    req.extensions_mut().insert(ConnectInfo::<std::net::SocketAddr>(peer.parse().unwrap()));
    let app = app.clone();
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
    rt.block_on(async move {
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status().as_u16();
        let headers = resp
            .headers()
            .iter()
            .map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or("").to_string()))
            .collect();
        let bytes = http_body_util::BodyExt::collect(resp.into_body()).await.unwrap().to_bytes();
        Resp { status, headers, body: String::from_utf8_lossy(&bytes).to_string() }
    })
}

fn app_with_password(ts: &TestServer, password: &str) -> axum::Router {
    ts.store.set_password(&ts.owner_id, &ts.owner_id, Some(password)).unwrap();
    annotary::server::router(annotary::server::AppState::new(
        ts.store.clone(),
        ts.url.clone(),
        "2026-10-21T00:00:00.000Z".into(),
    ))
}

const FORM: (&str, &str) = ("content-type", "application/x-www-form-urlencoded");
const JSON: (&str, &str) = ("content-type", "application/json");

fn web_login(app: &axum::Router, peer: &str, password: &str) -> Resp {
    call(app, peer, "POST", "/login", &[FORM, ("accept", "text/html")], format!("password={password}&next=%2F"))
}

fn api_login(app: &axum::Router, peer: &str, password: &str) -> Resp {
    call(app, peer, "POST", "/api/v1/login", &[JSON], json!({ "password": password }).to_string())
}

// spec: S19-01
#[test]
fn repeated_wrong_passwords_lock_that_client_out_of_the_form_and_the_api() {
    let ts = TestServer::start();
    let app = app_with_password(&ts, "hunter2");
    let a = "100.64.0.9:50000";
    // Five wrong guesses, the web form and the API counting together.
    for i in 0..5 {
        let r = if i % 2 == 0 { web_login(&app, a, "nope") } else { api_login(&app, a, "nope") };
        assert_eq!(r.status, 401, "guess {i}: {}", r.body);
    }
    // Locked: even the right password is refused, before it is checked.
    let r = web_login(&app, a, "hunter2");
    assert_eq!(r.status, 429, "{}", r.body);
    assert!(r.body.contains("Too many wrong passwords") && r.body.contains("<form"), "{}", r.body);
    assert!(r.header("set-cookie").is_none());
    let secs: u64 = r.header("retry-after").unwrap().parse().unwrap();
    assert!(secs > 14 * 60 && secs <= 15 * 60, "{secs}");
    let r = api_login(&app, a, "hunter2");
    assert_eq!(r.status, 429);
    assert_eq!(r.json()["error"], "too_many_attempts");
    assert!(r.header("retry-after").is_some());
    // Another address from the same IPv4 host is the same client; another host is not.
    assert_eq!(web_login(&app, "100.64.0.9:50001", "hunter2").status, 429);
    let b = "100.64.0.10:50000";
    assert_eq!(web_login(&app, b, "nope").status, 401);
    let r = web_login(&app, b, "hunter2");
    assert_eq!(r.status, 303, "{}", r.body);
    assert!(r.header("set-cookie").is_some_and(|c| c.starts_with("cd_session=")));
    // A success forgives that client's earlier failures: four more wrong ones stay 401.
    for _ in 0..4 {
        assert_eq!(api_login(&app, b, "nope").status, 401);
    }
    assert_eq!(api_login(&app, b, "hunter2").status, 200);
    // A Token keeps working for the locked-out client.
    let r = call(&app, a, "GET", "/api/v1/me", &[("authorization", &format!("Bearer {}", ts.owner_token))], "");
    assert_eq!(r.status, 200, "{}", r.body);
}

// spec: S19-02
#[test]
fn failures_across_many_remote_clients_lock_the_account_but_never_loopback() {
    let ts = TestServer::start();
    let app = app_with_password(&ts, "hunter2");
    // 20 wrong guesses, 4 each from 5 clients: no client reaches its own limit.
    for c in 0..5 {
        for _ in 0..4 {
            assert_eq!(api_login(&app, &format!("192.168.1.{}:40000", 20 + c), "nope").status, 401);
        }
    }
    // A fresh remote client with the right password is locked out by the account limit.
    let r = web_login(&app, "100.64.0.77:50000", "hunter2");
    assert_eq!(r.status, 429, "{}", r.body);
    let r = api_login(&app, "[fd7a:115c:a1e0::5]:50000", "hunter2");
    assert_eq!(r.status, 429);
    assert!(r.json()["message"].as_str().unwrap().contains("this server"), "{}", r.body);
    // The machine the server runs on can still sign in.
    assert_eq!(web_login(&app, "127.0.0.1:50000", "hunter2").status, 303);
    assert_eq!(api_login(&app, "[::1]:50000", "hunter2").status, 200);
}

// spec: S19-03
#[test]
fn a_cross_site_login_or_cookie_post_is_refused_and_same_origin_bearer_and_headerless_pass() {
    let ts = TestServer::start();
    let app = app_with_password(&ts, "hunter2");
    let peer = "100.64.0.9:50000";
    let host = ("host", "studio.example:7377");
    let form = "password=hunter2&next=%2F";
    // The login form posted from another site: refused, no session.
    for headers in [
        vec![FORM, host, ("origin", "https://evil.example")],
        vec![FORM, host, ("origin", "null")],
        vec![FORM, host, ("origin", "http://studio.example:8080")],
        vec![FORM, host, ("sec-fetch-site", "cross-site"), ("origin", "https://evil.example")],
        vec![FORM, host, ("sec-fetch-site", "same-site")],
    ] {
        let r = call(&app, peer, "POST", "/login", &headers, form);
        assert_eq!(r.status, 403, "{headers:?}: {}", r.body);
        assert_eq!(r.json()["error"], "cross_site");
        assert!(r.header("set-cookie").is_none());
    }
    // From the server's own page: signed in.
    let r = call(
        &app,
        peer,
        "POST",
        "/login",
        &[FORM, host, ("origin", "http://studio.example:7377"), ("sec-fetch-site", "same-origin")],
        form,
    );
    assert_eq!(r.status, 303, "{}", r.body);
    let cookie = r.header("set-cookie").unwrap().split(';').next().unwrap().to_string();
    // A cookie-carrying mutation from another site is refused too (here: Sign out, a Thread).
    let r = call(&app, peer, "POST", "/logout", &[host, ("cookie", &cookie), ("origin", "https://evil.example")], "");
    assert_eq!(r.status, 403);
    let r = call(
        &app,
        peer,
        "POST",
        "/api/v1/vaults",
        &[JSON, host, ("cookie", &cookie), ("sec-fetch-site", "cross-site")],
        json!({ "name": "evil" }).to_string(),
    );
    assert_eq!(r.status, 403);
    assert!(ts.store.list_vaults().unwrap().iter().all(|v| v.name != "evil"));
    // The session still works from its own origin, and a GET is never refused.
    let r = call(&app, peer, "GET", "/api/v1/me", &[host, ("cookie", &cookie), ("sec-fetch-site", "cross-site")], "");
    assert_eq!(r.status, 200);
    // A bearer request is exempt whatever its Origin (a browser never attaches a Token itself).
    let bearer = format!("Bearer {}", ts.owner_token);
    let r = call(
        &app,
        peer,
        "POST",
        "/api/v1/vaults",
        &[JSON, host, ("authorization", &bearer), ("origin", "https://evil.example")],
        json!({ "name": "fine" }).to_string(),
    );
    assert_eq!(r.status, 201, "{}", r.body);
    // An empty or ignored `Authorization` buys no exemption: an empty Bearer falls back to the
    // cookie, and the login form never reads the header.
    let r = call(
        &app,
        peer,
        "POST",
        "/api/v1/vaults",
        &[JSON, host, ("cookie", &cookie), ("authorization", "Bearer   "), ("origin", "https://evil.example")],
        json!({ "name": "evil2" }).to_string(),
    );
    assert_eq!(r.status, 403, "{}", r.body);
    let r = call(
        &app,
        peer,
        "POST",
        "/login",
        &[FORM, host, ("authorization", &bearer), ("origin", "https://evil.example")],
        form,
    );
    assert_eq!(r.status, 403, "{}", r.body);
    // No Origin and no Sec-Fetch-Site (the CLI, curl): let through.
    assert_eq!(call(&app, peer, "POST", "/login", &[FORM, host], form).status, 303);
}

// ---- CMD-17 ----
/// A request from `127.0.0.1` with extra headers: what Tailscale Serve on the same host sends.
fn get_from(app: &axum::Router, peer: &str, path: &str, headers: &[(&str, &str)]) -> Resp {
    call(app, peer, "GET", path, headers, "")
}

fn no_password_app(ts: &TestServer) -> axum::Router {
    annotary::server::router(annotary::server::AppState::new(
        ts.store.clone(),
        ts.url.clone(),
        "2026-10-29T00:00:00.000Z".into(),
    ))
}

// spec: S23-01
#[test]
fn a_request_a_local_proxy_forwards_is_not_signed_in_as_the_owner() {
    let ts = TestServer::start();
    assert!(!ts.store.password_set().unwrap());
    let app = no_password_app(&ts);
    let html = ("accept", "text/html");
    let local = "127.0.0.1:50000";
    // A browser on this machine is still the owner.
    let r = get_from(&app, local, "/", &[html]);
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.header("set-cookie").is_some_and(|c| c.starts_with("cd_session=")));
    let r = get_from(&app, "[::1]:50000", "/login?next=/d/abc", &[html]);
    assert_eq!(r.status, 303);
    // The same peer speaking for someone else, by each header a proxy adds.
    for fwd in [
        ("x-forwarded-for", "100.64.0.2"),
        ("x-forwarded-for", "127.0.0.1"),
        ("forwarded", "for=100.64.0.2;proto=https"),
        ("x-real-ip", "100.64.0.2"),
        ("tailscale-user-login", "someone@example.com"),
    ] {
        let r = get_from(&app, local, "/", &[html, fwd]);
        assert_eq!(r.status, 303, "{fwd:?}: {}", r.body);
        assert!(r.header("location").is_some_and(|l| l.starts_with("/login")), "{fwd:?}");
        assert!(r.header("set-cookie").is_none(), "{fwd:?}: no session");
        let r = get_from(&app, local, "/api/v1/me", &[fwd]);
        assert_eq!(r.status, 401, "{fwd:?}: the API never signs anyone in");
        let r = get_from(&app, local, "/login?next=/d/abc", &[html, fwd]);
        assert_eq!(r.status, 200, "{fwd:?}");
        assert!(r.header("set-cookie").is_none(), "{fwd:?}");
        assert!(r.body.contains("This server has no local password yet"), "{fwd:?}: {}", r.body);
    }
    // A remote peer's own forwarding headers change nothing (still not loopback).
    let r = get_from(&app, "100.64.0.2:50000", "/", &[html, ("x-forwarded-for", "127.0.0.1")]);
    assert_eq!(r.status, 303);
    assert!(r.header("set-cookie").is_none());
}

// spec: S23-02
#[test]
fn wrong_passwords_a_local_proxy_forwards_count_per_client_and_for_the_account() {
    let ts = TestServer::start();
    let app = app_with_password(&ts, "hunter2");
    let proxy = "127.0.0.1:50000";
    let via = |client: &'static str, password: &str| {
        call(
            &app,
            proxy,
            "POST",
            "/login",
            &[FORM, ("accept", "text/html"), ("x-forwarded-for", client)],
            format!("password={password}&next=%2F"),
        )
    };
    // Five wrong guesses from one tailnet client lock that client, not another one.
    for _ in 0..5 {
        assert_eq!(via("100.64.0.9", "nope").status, 401);
    }
    assert_eq!(via("100.64.0.9", "hunter2").status, 429);
    // The last entry is the client this host's proxy saw; a spoofed first entry is ignored.
    assert_eq!(via("10.9.9.9, 100.64.0.9", "hunter2").status, 429);
    assert_eq!(via("100.64.0.9, 100.64.0.10", "hunter2").status, 303);
    // Fifteen more spread over clients reach the account limit of 20, which loopback is
    // exempt from but forwarded clients are not.
    for c in 0..5 {
        for _ in 0..3 {
            let r = api_login_via(&app, proxy, &format!("100.64.1.{c}"), "nope");
            assert_eq!(r.status, 401, "{}", r.body);
        }
    }
    assert_eq!(via("100.64.2.1", "hunter2").status, 429, "the account limit holds behind the proxy");
    // A forwarded request with no usable client address is one client, 0.0.0.0, and still remote.
    assert_eq!(via("garbage", "hunter2").status, 429);
    // The machine itself, not forwarded, still signs in.
    assert_eq!(web_login(&app, proxy, "hunter2").status, 303);
}

fn api_login_via(app: &axum::Router, peer: &str, client: &str, password: &str) -> Resp {
    call(
        app,
        peer,
        "POST",
        "/api/v1/login",
        &[JSON, ("x-forwarded-for", client)],
        json!({ "password": password }).to_string(),
    )
}
// ---- end CMD-17 ----
