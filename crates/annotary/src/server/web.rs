//! Browser auth (ticket 06): the loopback-trust middleware, the login page and logout. Ticket 10
//! replaced the placeholder `/` with the Document list (`listing.rs`) and added `/d/{id}`
//! (`view.rs`) in the same `web_pages` router; ticket 12 restyled the login page onto the shared
//! shell (`ui.rs`) and added the owner screens `/members`, `/settings` (`admin.rs`).
//!
//! | route | who | what |
//! |---|---|---|
//! | `GET /` | web auth | the Document list (ticket 10, `listing.rs`) |
//! | `GET /login?next=` | anyone | the login page — only when a password is set; a loopback browser with no password is signed in and sent on; an already signed-in browser is sent on |
//! | `POST /login` (`password`, `next`) | anyone | verify the local password → session cookie → redirect to `next` (default `/`); wrong ⇒ 401 with the page again; throttled after repeated failures ⇒ 429 with the page (CMD-4, `login_guard.rs`) |
//! | `POST /logout` | web auth | end the session, clear the cookie, redirect to `/login` (or `/` when no password is set) |
//!
//! [`require_web_auth`] guards every web page (`/`, `/a/{hash}`, tickets 10/12): a bearer Token
//! or the `cd_session` cookie signs the request in; with neither, a **loopback** request while
//! **no** local password is set is signed in as the founding owner and gets a session cookie
//! automatically (story 24); otherwise a page request is redirected to `/login` and anything
//! else (an `<img>` fetch, `fetch()`) gets 401 JSON. API routes never do any of this: neither
//! cookie nor bearer ⇒ 401 (`Auth`), so the CLI with no credential never becomes the owner.

use std::net::SocketAddr;

use axum::Form;
use axum::extract::{ConnectInfo, Query, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::middleware::Next;
use axum::response::{Html, IntoResponse, Redirect, Response};
use serde::Deserialize;

use annotary_store::SESSION_TTL_DAYS;

use super::app::AppState;
use super::auth::{Auth, Presented, SESSION_COOKIE, presented, resolve};
use super::error::ApiError;
use super::ui::esc;

/// `Set-Cookie` value for a fresh session (HttpOnly, SameSite=Lax, one year; no `Secure` — the
/// server is plain http on loopback).
pub fn session_cookie_header(plain: &str) -> HeaderValue {
    let max_age = SESSION_TTL_DAYS * 24 * 3600;
    HeaderValue::from_str(&format!("{SESSION_COOKIE}={plain}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}"))
        .expect("cookie header")
}

/// `Set-Cookie` value that clears the session cookie.
pub fn clear_cookie_header() -> HeaderValue {
    HeaderValue::from_static("cd_session=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0")
}

/// A browser on this machine: a loopback peer, and not a request a local proxy forwarded (CMD-17).
fn is_loopback(req: &Request) -> bool {
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .is_some_and(|c| super::login_guard::is_loopback(super::login_guard::client_ip(c.0.ip(), req.headers())))
}

fn wants_html(req: &Request) -> bool {
    req.headers().get(header::ACCEPT).and_then(|v| v.to_str().ok()).is_some_and(|a| a.contains("text/html"))
}

/// A safe `next` target: a same-origin absolute path (never `//host` or a full URL).
fn safe_next(next: Option<&str>) -> String {
    match next.map(str::trim) {
        Some(n) if n.starts_with('/') && !n.starts_with("//") && !n.contains("\\") && n != "/login" => n.to_string(),
        _ => "/".to_string(),
    }
}

fn login_redirect(uri: &Uri) -> Response {
    let next = uri.path_and_query().map(|p| p.as_str()).unwrap_or("/");
    Redirect::to(&format!("/login?next={}", crate::client::qs(next))).into_response()
}

/// Sign a loopback browser in as the founding owner (no password set): open a session and
/// return the `Auth` plus the cookie to set.
fn loopback_sign_in(st: &AppState) -> Result<(Auth, HeaderValue), ApiError> {
    let owner = st.store.founding_owner()?.ok_or_else(|| ApiError::internal("no owner"))?;
    let session = st.store.open_session(&owner.id, "loopback")?;
    let auth = st
        .store
        .authenticate_session(&session.plain)?
        .ok_or_else(|| ApiError::internal("fresh session does not authenticate"))?;
    Ok((Auth(auth), session_cookie_header(&session.plain)))
}

/// Middleware for web pages: see the module docs.
pub async fn require_web_auth(State(st): State<AppState>, mut req: Request, next: Next) -> Response {
    match resolve(&st, req.headers()) {
        Ok(Some(auth)) => {
            req.extensions_mut().insert(auth);
            next.run(req).await
        }
        Ok(None) => {
            let password_set = match st.store.password_set() {
                Ok(b) => b,
                Err(e) => return ApiError::from(e).into_response(),
            };
            if !password_set && is_loopback(&req) {
                let (auth, cookie) = match loopback_sign_in(&st) {
                    Ok(x) => x,
                    Err(e) => return e.into_response(),
                };
                req.extensions_mut().insert(auth);
                let mut resp = next.run(req).await;
                resp.headers_mut().append(header::SET_COOKIE, cookie);
                resp
            } else if wants_html(&req) {
                login_redirect(req.uri())
            } else {
                ApiError::missing_credential().into_response()
            }
        }
        Err(e) => {
            // A dead cookie: clear it and start over (page ⇒ login, else 401).
            let mut resp = if wants_html(&req) && matches!(presented(req.headers()), Presented::Cookie(_)) {
                login_redirect(req.uri())
            } else {
                e.into_response()
            };
            if matches!(presented(req.headers()), Presented::Cookie(_)) {
                resp.headers_mut().append(header::SET_COOKIE, clear_cookie_header());
            }
            resp
        }
    }
}

// ---------------------------------------------------------------- pages

// ---- ticket 12: the login page wears the shared shell (`ui::page_ex` + `ADMIN_CSS`) ----
fn simple_page(title: &str, inner: &str) -> Html<String> {
    Html(super::ui::shell_ex(
        title,
        &format!(
            "<div {} id=\"cd-app\"><main class=\"cd-simple cd-login\">{inner}</main></div>",
            // CMD-23: nobody is signed in yet, so the page follows the system's appearance
            super::prefs::WebPrefs::default().app_attrs("cd-loginpage")
        ),
        super::ui::ADMIN_CSS,
        "",
    ))
}

fn login_form(next: &str, error: Option<&str>) -> Html<String> {
    let err = error.map(|e| format!("<p class=\"cd-err\" role=\"alert\">{}</p>", esc(e))).unwrap_or_default();
    simple_page(
        "Sign in — annotary",
        &format!(
            "<h1 class=\"cd-logo\">annotary</h1><p class=\"cd-muted\">This server has a local password. Sign in to continue.</p>{err}\
             <form method=\"post\" action=\"/login\" class=\"cd-loginform\" id=\"cd-loginform\"><input type=\"hidden\" name=\"next\" value=\"{}\">\
             <label>Password <input type=\"password\" name=\"password\" autofocus autocomplete=\"current-password\" required></label> \
             <button type=\"submit\" class=\"cd-btn cd-primary\">Sign in</button></form>\
             <p class=\"cd-muted cd-small\">Agents and the CLI use a Token instead (<code>annotary login</code> turns this password into one).</p>",
            esc(next)
        ),
    )
}
// ---- end ticket 12 ----

#[derive(Debug, Deserialize, Default)]
pub struct LoginQuery {
    pub next: Option<String>,
}

/// `GET /login`.
pub async fn login_page(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<LoginQuery>,
) -> Result<Response, ApiError> {
    let next = safe_next(q.next.as_deref());
    // Already signed in (a valid cookie or bearer): straight on. A dead cookie is cleared and
    // the form shown — the login page itself must never 401.
    let mut stale_cookie = false;
    match resolve(&st, &headers) {
        Ok(Some(_)) => return Ok(Redirect::to(&next).into_response()),
        Ok(None) => {}
        Err(_) => stale_cookie = true,
    }
    let clear = |mut resp: Response| {
        if stale_cookie {
            resp.headers_mut().append(header::SET_COOKIE, clear_cookie_header());
        }
        resp
    };
    if !st.store.password_set()? {
        if super::login_guard::is_loopback(super::login_guard::client_ip(addr.ip(), &headers)) {
            let (_, cookie) = loopback_sign_in(&st)?;
            let mut resp = Redirect::to(&next).into_response();
            resp.headers_mut().append(header::SET_COOKIE, cookie);
            return Ok(resp);
        }
        return Ok(clear(no_password_page(&st.url).into_response()));
    }
    Ok(clear(login_form(&next, None).into_response()))
}

// ---- CMD-14: the no-password page for a browser loopback trust does not cover ----
/// `GET /login` from a non-loopback peer while no password is set: nothing here can sign it in,
/// so say how to get in instead — set a password from a browser on the server's machine (where
/// loopback trust signs it in), or pair the iPhone / iPad app with `annotary pair`.
fn no_password_page(server_url: &str) -> Html<String> {
    let set_password = match loopback_url(server_url) {
        Some(url) => format!(
            "On the machine running the server, open <code>{}</code> (a browser there is signed in \
             automatically) and set a password in Settings. Then sign in here with it.",
            esc(&url)
        ),
        None => format!(
            "This server listens only on <code>{}</code>, so no browser is signed in automatically: \
             restart it with <code>--bind 0.0.0.0 --allow-remote</code>, open \
             <code>http://127.0.0.1:PORT</code> on its machine and set a password in Settings. Then \
             sign in here with it.",
            esc(server_url)
        ),
    };
    simple_page(
        "Sign in — annotary",
        &format!(
            "<h1 class=\"cd-logo\">annotary</h1><p>This server has no local password yet, so a \
             browser on another device cannot sign in.</p><ol class=\"cd-loginsteps\" id=\"cd-loginsteps\">\
             <li>{set_password}</li><li>For the iPhone or iPad app, run <code>annotary pair</code> on \
             that machine and scan the code it shows.</li></ol>"
        ),
    )
}

/// The server's own URL when a browser on its machine reaches it over loopback (`url_for` names
/// an unspecified bind by its loopback address), else `None` (bound to one non-loopback address).
fn loopback_url(server_url: &str) -> Option<String> {
    let host = server_url.strip_prefix("http://")?.rsplit_once(':')?.0;
    let ip: std::net::IpAddr = host.trim_start_matches('[').trim_end_matches(']').parse().ok()?;
    super::login_guard::is_loopback(ip).then(|| server_url.to_string())
}
// ---- end CMD-14 ----

#[derive(Debug, Deserialize, Default)]
pub struct LoginForm {
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub next: Option<String>,
}

/// `POST /login`. CMD-4: a client or account locked out by the failed-login throttle gets 429
/// and the form with how long to wait, before the password is checked (`login_guard.rs`).
pub async fn login_post(
    State(st): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(f): Form<LoginForm>,
) -> Result<Response, ApiError> {
    let next = safe_next(f.next.as_deref());
    let ip = super::login_guard::client_ip(addr.ip(), &headers);
    if let Err(l) = st.login_throttle.check(ip, std::time::Instant::now()) {
        let msg = format!("Too many wrong passwords. Try again in {} min.", super::login_guard::minutes(l.retry_after));
        let mut resp = (StatusCode::TOO_MANY_REQUESTS, login_form(&next, Some(&msg))).into_response();
        super::login_guard::retry_after_header(&mut resp, l);
        return Ok(resp);
    }
    match st.store.check_password(&f.password) {
        Ok(m) => {
            st.login_throttle.succeed(ip);
            let session = st.store.open_session(&m.id, "login")?;
            tracing::info!("browser session opened for @{}", m.handle);
            let mut resp = Redirect::to(&next).into_response();
            resp.headers_mut().append(header::SET_COOKIE, session_cookie_header(&session.plain));
            Ok(resp)
        }
        Err(annotary_store::StoreError::WrongPassword) => {
            super::login_guard::note_failure(&st, ip, "web");
            Ok((StatusCode::UNAUTHORIZED, login_form(&next, Some("Wrong password."))).into_response())
        }
        Err(annotary_store::StoreError::PasswordNotSet) => Ok(Redirect::to(&next).into_response()),
        Err(e) => Err(e.into()),
    }
}

/// `POST /logout` (web): end the cookie session and send the browser to the login page (or `/`
/// when no password is set — where loopback trust signs it in again).
pub async fn logout(State(st): State<AppState>, auth: Auth) -> Result<Response, ApiError> {
    if let annotary_store::AuthVia::Session { id } = &auth.0.via {
        st.store.end_session(id)?;
    }
    let target = if st.store.password_set()? { "/login" } else { "/" };
    let mut resp = Redirect::to(target).into_response();
    resp.headers_mut().append(header::SET_COOKIE, clear_cookie_header());
    Ok(resp)
}
