//! Guards on password sign-in and on browser mutations (CMD-4): since ticket 18 the server can
//! listen beyond loopback (`--allow-remote`), so password guessing and cross-site form posts
//! are reachable.
//!
//! **Failed-login throttle.** `POST /login` (the web form) and `POST /api/v1/login` share one
//! [`LoginThrottle`] in `AppState`. A failure is counted per client (the peer address; an IPv6
//! peer by its /64, which one host can rotate through) and, for non-loopback peers, against the
//! local password as a whole (the "account": the password is the only thing a sign-in names).
//! [`CLIENT_MAX_FAILURES`] failures from one client within [`WINDOW`], or
//! [`ACCOUNT_MAX_FAILURES`] from all non-loopback clients together, lock further attempts out
//! until the oldest of them is [`WINDOW`] old: 429 `too_many_attempts` with `Retry-After`, and
//! the password is not even checked (a right guess during a lockout fails too). A success clears
//! that client's failures. Loopback is neither counted against nor locked by the account limit,
//! so a remote attacker can never lock the owner out of the machine the server runs on. The
//! counters live in memory: a restart clears them.
//!
//! **Cross-site refusal** ([`refuse_cross_site`], a layer on the whole router). A request that
//! changes something (any method but GET / HEAD / OPTIONS) and carries no bearer Token — the
//! login form, or anything a browser sends with the `cd_session` cookie — is refused with 403
//! `cross_site` when the browser says it came from elsewhere: `Sec-Fetch-Site` other than
//! `same-origin` / `none`, or an `Origin` whose `host[:port]` is not the request's `Host`
//! (`Origin: null` included). A request with neither header (the CLI, curl, an old browser) is
//! let through; every current browser sends `Origin` on a cross-origin POST. A bearer request
//! with a Token that `Auth` will use is exempt (not the login form, which ignores it): a browser
//! never attaches a Token by itself, so it cannot be forged cross-site.
//! The `cd_session` cookie stays `SameSite=Lax` on top.
//!
//! **A reverse proxy on this host** (CMD-17: for example Tailscale Serve). Every request it
//! forwards arrives from a loopback peer, so [`client_ip`] is what loopback trust and the
//! throttle read instead of the bare peer address: a loopback peer whose request carries a
//! forwarding header ([`FORWARDED_HEADERS`]) is the proxy speaking for someone else, never
//! loopback. The client is then the last `X-Forwarded-For` entry (the one this host's proxy
//! added), else the unspecified address. The rule can only take trust away: a browser on this
//! machine sends none of these headers, and a remote peer's headers are never read.

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::error::ApiError;

/// How long a failure counts.
pub const WINDOW: Duration = Duration::from_secs(15 * 60);
/// Failures from one client within [`WINDOW`] that lock that client out.
pub const CLIENT_MAX_FAILURES: usize = 5;
/// Failures from all non-loopback clients within [`WINDOW`] that lock every non-loopback client out.
pub const ACCOUNT_MAX_FAILURES: usize = 20;

/// The failed-login counters (see the module docs).
#[derive(Default)]
pub struct LoginThrottle {
    inner: Mutex<Counters>,
}

#[derive(Default)]
struct Counters {
    clients: HashMap<IpAddr, VecDeque<Instant>>,
    account: VecDeque<Instant>,
}

/// Why a sign-in is refused before the password is checked, and for how long.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Locked {
    pub retry_after: Duration,
    /// True when the account-wide limit (not this client's own) is what locks it.
    pub account: bool,
}

/// The client key: an IPv4-mapped address as IPv4, an IPv6 address by its /64.
fn client_key(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => IpAddr::V4(v4),
            None => {
                let s = v6.segments();
                IpAddr::V6(std::net::Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0))
            }
        },
        v4 => v4,
    }
}

/// Loopback, an IPv4-mapped loopback address included (a dual-stack `[::]` listener).
pub fn is_loopback(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V6(v6) => v6.is_loopback() || v6.to_ipv4_mapped().is_some_and(|v4| v4.is_loopback()),
        v4 => v4.is_loopback(),
    }
}

/// Headers a reverse proxy adds for the client it forwards (CMD-17). Tailscale Serve sets
/// `X-Forwarded-For` and, for a signed-in tailnet user, `Tailscale-User-Login`.
pub const FORWARDED_HEADERS: [&str; 4] = ["x-forwarded-for", "forwarded", "x-real-ip", "tailscale-user-login"];

/// The client a request comes from (CMD-17, module docs): `peer`, unless `peer` is loopback and
/// the request was forwarded; then the last `X-Forwarded-For` entry when it is a non-loopback
/// address, else `0.0.0.0`. Either way a forwarded request is never loopback.
pub fn client_ip(peer: IpAddr, headers: &HeaderMap) -> IpAddr {
    if !is_loopback(peer) || !FORWARDED_HEADERS.iter().any(|h| headers.contains_key(*h)) {
        return peer;
    }
    headers
        .get_all("x-forwarded-for")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .last()
        .and_then(|s| s.trim().parse::<IpAddr>().ok())
        .filter(|ip| !is_loopback(*ip))
        .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
}

/// Drop failures older than [`WINDOW`]; the time until the oldest left expires when `max` remain.
fn locked_for(q: &mut VecDeque<Instant>, max: usize, now: Instant) -> Option<Duration> {
    while q.front().is_some_and(|t| now.saturating_duration_since(*t) >= WINDOW) {
        q.pop_front();
    }
    (q.len() >= max).then(|| WINDOW - now.saturating_duration_since(q[q.len() - max]))
}

impl LoginThrottle {
    /// May `ip` try a password now? `Err` names how long it must wait.
    pub fn check(&self, ip: IpAddr, now: Instant) -> Result<(), Locked> {
        let mut c = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(q) = c.clients.get_mut(&client_key(ip))
            && let Some(d) = locked_for(q, CLIENT_MAX_FAILURES, now)
        {
            return Err(Locked { retry_after: d, account: false });
        }
        if !is_loopback(ip)
            && let Some(d) = locked_for(&mut c.account, ACCOUNT_MAX_FAILURES, now)
        {
            return Err(Locked { retry_after: d, account: true });
        }
        Ok(())
    }

    /// Count a wrong password from `ip`. Returns the lock it now triggers, if any (for the log).
    pub fn fail(&self, ip: IpAddr, now: Instant) -> Option<Locked> {
        let mut c = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        // Forget clients whose failures have all expired, so the map stays small.
        c.clients.retain(|_, q| q.back().is_some_and(|t| now.saturating_duration_since(*t) < WINDOW));
        let q = c.clients.entry(client_key(ip)).or_default();
        q.push_back(now);
        if let Some(d) = locked_for(q, CLIENT_MAX_FAILURES, now) {
            if !is_loopback(ip) {
                c.account.push_back(now);
            }
            return Some(Locked { retry_after: d, account: false });
        }
        if !is_loopback(ip) {
            c.account.push_back(now);
            if let Some(d) = locked_for(&mut c.account, ACCOUNT_MAX_FAILURES, now) {
                return Some(Locked { retry_after: d, account: true });
            }
        }
        None
    }

    /// A right password from `ip`: its own failures are forgiven (the account's are not).
    pub fn succeed(&self, ip: IpAddr) {
        let mut c = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        c.clients.remove(&client_key(ip));
    }
}

/// Count a wrong password from `ip` (`via` = `web` | `api`) and log it; a lock it triggers is
/// a warning (what an operator greps the server log for).
pub fn note_failure(st: &super::app::AppState, ip: IpAddr, via: &str) {
    match st.login_throttle.fail(ip, Instant::now()) {
        Some(l) if l.account => tracing::warn!(
            "wrong password ({via}) from {ip}: sign-ins from every non-loopback client paused for {} min",
            minutes(l.retry_after)
        ),
        Some(l) => tracing::warn!(
            "wrong password ({via}) from {ip}: sign-ins from it paused for {} min",
            minutes(l.retry_after)
        ),
        None => tracing::info!("wrong password ({via}) from {ip}"),
    }
}

/// Whole minutes to wait, rounded up (at least 1), for a human-readable message.
pub fn minutes(d: Duration) -> u64 {
    d.as_secs().div_ceil(60).max(1)
}

/// The 429 a locked-out `POST /api/v1/login` answers with.
pub fn too_many_attempts(l: Locked) -> Response {
    let who = if l.account { "this server" } else { "this client" };
    let mut resp = ApiError::new(
        StatusCode::TOO_MANY_REQUESTS,
        "too_many_attempts",
        format!("too many failed sign-ins from {who}; try again in {} min", minutes(l.retry_after)),
    )
    .hint("sign-ins are paused after repeated wrong passwords; a Token keeps working meanwhile")
    .into_response();
    retry_after_header(&mut resp, l);
    resp
}

/// Set `Retry-After` (whole seconds, rounded up).
pub fn retry_after_header(resp: &mut Response, l: Locked) {
    let secs = l.retry_after.as_secs() + u64::from(l.retry_after.subsec_nanos() > 0);
    resp.headers_mut().insert(header::RETRY_AFTER, secs.max(1).into());
}

// ---------------------------------------------------------------- cross-site refusal

/// Why `headers` look like a cross-site browser request, or `None`.
fn cross_site(headers: &HeaderMap) -> Option<&'static str> {
    let get = |h: &str| headers.get(h).and_then(|v| v.to_str().ok()).map(str::trim);
    if let Some(site) = get("sec-fetch-site") {
        return (!site.eq_ignore_ascii_case("same-origin") && !site.eq_ignore_ascii_case("none"))
            .then_some("Sec-Fetch-Site says the request came from another site");
    }
    let origin = get("origin")?;
    let authority = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
        .filter(|a| !a.is_empty() && !a.contains('/'));
    let host = get("host");
    match (authority, host) {
        (Some(a), Some(h)) if a.eq_ignore_ascii_case(h) => None,
        _ => Some("its Origin is not this server"),
    }
}

/// Middleware on the whole router: see the module docs.
pub async fn refuse_cross_site(req: Request, next: Next) -> Response {
    let safe = matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    // Exempt only what `Auth` will really sign in by its Token (an empty `Bearer` falls back to
    // the cookie), and never the login form, which ignores `Authorization`.
    let bearer = matches!(super::auth::presented(req.headers()), super::auth::Presented::Bearer(_))
        && req.uri().path() != "/login";
    if !safe
        && !bearer
        && let Some(why) = cross_site(req.headers())
    {
        tracing::warn!("refused a cross-site {} {}: {why}", req.method(), req.uri().path());
        return ApiError::new(StatusCode::FORBIDDEN, "cross_site", format!("refused a cross-site request: {why}"))
            .hint("open the page from this server's own address, or send a bearer Token")
            .into_response();
    }
    next.run(req).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    // spec: S19-01
    #[test]
    fn a_client_is_locked_after_five_failures_until_the_oldest_is_a_window_old() {
        let t = LoginThrottle::default();
        let t0 = Instant::now();
        let a = ip("100.64.0.9");
        for i in 0..CLIENT_MAX_FAILURES as u64 {
            assert_eq!(t.check(a, t0 + Duration::from_secs(i)), Ok(()));
            t.fail(a, t0 + Duration::from_secs(i));
        }
        let l = t.check(a, t0 + Duration::from_secs(10)).unwrap_err();
        assert!(!l.account);
        assert_eq!(l.retry_after, WINDOW - Duration::from_secs(10));
        // Another client is not affected.
        assert_eq!(t.check(ip("100.64.0.10"), t0 + Duration::from_secs(10)), Ok(()));
        // Once the oldest failure is a window old, one more try is allowed.
        assert_eq!(t.check(a, t0 + WINDOW), Ok(()));
        // A success forgives the client's own failures.
        t.fail(a, t0 + WINDOW);
        assert!(t.check(a, t0 + WINDOW).is_err());
        t.succeed(a);
        assert_eq!(t.check(a, t0 + WINDOW), Ok(()));
    }

    // spec: S19-01
    #[test]
    fn an_ipv6_client_is_counted_by_its_64_and_a_mapped_v4_as_v4() {
        let t = LoginThrottle::default();
        let now = Instant::now();
        for i in 0..CLIENT_MAX_FAILURES {
            t.fail(ip(&format!("fd7a:115c:a1e0:ab12::{:x}", i + 1)), now);
        }
        assert!(t.check(ip("fd7a:115c:a1e0:ab12::ffff"), now).is_err());
        assert_eq!(t.check(ip("fd7a:115c:a1e0:ab13::1"), now), Ok(()));
        for _ in 0..CLIENT_MAX_FAILURES {
            t.fail(ip("::ffff:192.168.1.20"), now);
        }
        assert!(t.check(ip("192.168.1.20"), now).is_err());
    }

    // spec: S19-02
    #[test]
    fn the_account_locks_remote_clients_after_twenty_failures_but_never_loopback() {
        let t = LoginThrottle::default();
        let now = Instant::now();
        for i in 0..ACCOUNT_MAX_FAILURES {
            t.fail(ip(&format!("100.64.{}.1", i)), now);
        }
        let l = t.check(ip("100.64.200.1"), now).unwrap_err();
        assert!(l.account);
        assert_eq!(t.check(ip("127.0.0.1"), now), Ok(()));
        assert_eq!(t.check(ip("::1"), now), Ok(()));
        // Loopback failures never count against the account.
        let t = LoginThrottle::default();
        for _ in 0..ACCOUNT_MAX_FAILURES * 2 {
            t.fail(ip("127.0.0.1"), now);
        }
        assert_eq!(t.check(ip("100.64.0.1"), now), Ok(()));
    }

    // spec: S19-03
    #[test]
    fn cross_site_reads_sec_fetch_site_then_origin_against_host() {
        let h = |pairs: &[(&'static str, &'static str)]| {
            let mut m = HeaderMap::new();
            for (k, v) in pairs {
                m.insert(*k, v.parse().unwrap());
            }
            m
        };
        assert!(cross_site(&h(&[])).is_none());
        assert!(cross_site(&h(&[("host", "a:7377")])).is_none());
        assert!(cross_site(&h(&[("sec-fetch-site", "same-origin"), ("origin", "http://x")])).is_none());
        assert!(cross_site(&h(&[("sec-fetch-site", "none")])).is_none());
        assert!(cross_site(&h(&[("sec-fetch-site", "same-site")])).is_some());
        assert!(cross_site(&h(&[("sec-fetch-site", "cross-site")])).is_some());
        assert!(cross_site(&h(&[("host", "a:7377"), ("origin", "http://a:7377")])).is_none());
        assert!(cross_site(&h(&[("host", "A:7377"), ("origin", "https://a:7377")])).is_none());
        assert!(cross_site(&h(&[("host", "[::1]:7377"), ("origin", "http://[::1]:7377")])).is_none());
        assert!(cross_site(&h(&[("host", "a:7377"), ("origin", "http://a:7378")])).is_some());
        assert!(cross_site(&h(&[("host", "a:7377"), ("origin", "https://evil.example")])).is_some());
        assert!(cross_site(&h(&[("host", "a:7377"), ("origin", "null")])).is_some());
        assert!(cross_site(&h(&[("origin", "http://a:7377")])).is_some(), "no Host to match");
    }
}
