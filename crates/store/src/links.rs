//! Links other devices can open (ticket 18 — the iOS app's prerequisites R2/R3).
//!
//! Two server settings live in `settings` (no migration — the table is schema v1):
//!
//! - [`PUBLIC_URL_KEY`] `public_url` — the base URL *other devices* use to reach this server
//!   (e.g. `https://annotary.example.ts.net`); set by `serve --url URL` / `daemon install
//!   --url URL`, cleared by `--url none`; event `setting.public_url {url}`.
//! - [`LINK_BASE_KEY`] `link_base` — the base of the link relay (e.g. `https://go.annotary.dev`)
//!   that turns a Document link into a universal link for the app; `serve --link-base`; event
//!   `setting.link_base {url}`.
//!
//! Both are `scheme://host[:port]` — absolute http(s), no user info, path, query or fragment
//! (a trailing `/` is trimmed): [`normalize_base_url`].
//!
//! The pure helpers here are shared by the CLI (`link`, `show`/`push` on a TTY, `pair`) and the
//! server (`POST /api/v1/pairing`, the web page's "Open in Annotary" link): [`doc_link`],
//! [`authority`], [`pair_profile`], [`pair_url`], [`percent_encode`].

use rusqlite::params;

use crate::error::{Result, StoreError};
use crate::store::{Store, append_event};

/// `settings` key of the public URL.
pub const PUBLIC_URL_KEY: &str = "public_url";
/// `settings` key of the link relay base.
pub const LINK_BASE_KEY: &str = "link_base";

/// Validate and normalise a base URL: `http(s)://host[:port]`, lower-case scheme and host, no
/// trailing slash. `Err` carries the reason (without the URL).
pub fn normalize_base_url(text: &str) -> std::result::Result<String, String> {
    let t = text.trim();
    let Some((scheme, rest)) = t.split_once("://") else {
        return Err("use an absolute http:// or https:// URL".into());
    };
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return Err("use an absolute http:// or https:// URL".into());
    }
    let rest = rest.trim_end_matches('/');
    if rest.contains(['/', '?', '#']) {
        return Err("no path, query or fragment — just scheme://host[:port]".into());
    }
    if rest.contains('@') {
        return Err("no user info — just scheme://host[:port]".into());
    }
    if rest.is_empty() {
        return Err("the host is missing".into());
    }
    let (host, port) = if let Some(after) = rest.strip_prefix('[') {
        let Some((h, tail)) = after.split_once(']') else { return Err("unclosed [ in an IPv6 host".into()) };
        if h.parse::<std::net::Ipv6Addr>().is_err() {
            return Err(format!("[{h}] is not an IPv6 address"));
        }
        let port = match tail {
            "" => None,
            p => Some(p.strip_prefix(':').ok_or("junk after the IPv6 host")?),
        };
        (format!("[{}]", h.to_ascii_lowercase()), port)
    } else {
        match rest.rsplit_once(':') {
            Some((h, p)) => (h.to_ascii_lowercase(), Some(p)),
            None => (rest.to_ascii_lowercase(), None),
        }
    };
    if !host.starts_with('[')
        && (host.is_empty() || !host.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.'))
    {
        return Err(format!("{host:?} is not a host name or IP address"));
    }
    let port = match port {
        None => String::new(),
        Some(p) => match p.parse::<u16>() {
            Ok(n) if n > 0 && p.chars().all(|c| c.is_ascii_digit()) => format!(":{n}"),
            _ => return Err(format!("port {p:?} is not a number from 1 to 65535")),
        },
    };
    Ok(format!("{scheme}://{host}{port}"))
}

/// `host[:port]` of a normalised base URL (`http://h:7377` → `h:7377`).
pub fn authority(base: &str) -> Option<&str> {
    let (_, rest) = base.split_once("://")?;
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() { None } else { Some(rest) }
}

/// Whether an authority carries an explicit port (`h:7377`, `[::1]:7377`).
fn has_port(authority: &str) -> bool {
    match authority.rsplit_once(']') {
        Some((_, tail)) => tail.starts_with(':'),
        None => authority.contains(':'),
    }
}

/// The link to a Document (and optionally one of its Threads) that another device can open.
///
/// - no `public_url` → `None` (the caller falls back to its own server URL and says so);
/// - `public_url` only → `{public_url}/d/{id}` (+ `#tN`);
/// - both → `{link_base}/{authority}/d/{id}` (+ `#tN`), `authority` = `host[:port]` of
///   `public_url` — a universal link for the app on a device that has it.
///
/// **The relay rule** (the Cloudflare Worker at `link_base`, built separately): a browser that
/// opens `{link_base}/{authority}/{rest}` is redirected to `http://{authority}/{rest}` when the
/// authority has a port and to `https://{authority}/{rest}` when it has none. So a relay link is
/// only made when that rule reconstructs `public_url` exactly — `http://` *with* a port or
/// `https://` *without* one — and never for an IPv6 host (`[…]` does not survive a path
/// segment everywhere); otherwise the plain public link is returned.
pub fn doc_link(
    public_url: Option<&str>,
    link_base: Option<&str>,
    doc_id: &str,
    thread: Option<&str>,
) -> Option<String> {
    let public = public_url.map(|u| u.trim().trim_end_matches('/')).filter(|u| !u.is_empty())?;
    let fragment = thread.map(|t| format!("#{}", t.trim().trim_start_matches('#'))).unwrap_or_default();
    let plain = format!("{public}/d/{doc_id}{fragment}");
    let Some(base) = link_base.map(|u| u.trim().trim_end_matches('/')).filter(|u| !u.is_empty()) else {
        return Some(plain);
    };
    let Some(auth) = authority(public) else { return Some(plain) };
    if auth.starts_with('[') {
        return Some(plain);
    }
    let relay_ok =
        if public.starts_with("http://") { has_port(auth) } else { public.starts_with("https://") && !has_port(auth) };
    if !relay_ok {
        return Some(plain);
    }
    Some(format!("{base}/{auth}/d/{doc_id}{fragment}"))
}

/// Percent-encode everything but the URI unreserved characters (`A-Z a-z 0-9 - . _ ~`).
pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// The profile name a paired device saves: `<handle>@<first DNS label of the host>`
/// (`rowan@annotary` for `https://annotary.example.ts.net`), or `<handle>@<ip>` for an IP
/// host (IPv6 without brackets).
pub fn pair_profile(handle: &str, server: &str) -> String {
    let auth = authority(server).unwrap_or(server);
    let host = match auth.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or(rest).to_string(),
        None => auth.rsplit_once(':').map(|(h, _)| h).unwrap_or(auth).to_string(),
    };
    let is_ip = host.parse::<std::net::IpAddr>().is_ok();
    let label = if is_ip { host.as_str() } else { host.split('.').next().unwrap_or(&host) };
    format!("{handle}@{label}")
}

/// The pairing URL the app's QR scanner reads:
/// `annotary://pair?v=1&server=<pct>&token=<pct>&profile=<pct>`.
pub fn pair_url(server: &str, token: &str, profile: &str) -> String {
    format!(
        "annotary://pair?v=1&server={}&token={}&profile={}",
        percent_encode(server),
        percent_encode(token),
        percent_encode(profile)
    )
}

impl Store {
    /// The public URL (`serve --url`), if set.
    pub fn public_url(&self) -> Result<Option<String>> {
        Ok(self.setting(PUBLIC_URL_KEY)?.filter(|v| !v.trim().is_empty()))
    }

    /// The link relay base (`serve --link-base`), if set.
    pub fn link_base(&self) -> Result<Option<String>> {
        Ok(self.setting(LINK_BASE_KEY)?.filter(|v| !v.trim().is_empty()))
    }

    /// Set (`Some`, validated + normalised) or clear (`None`) the public URL. Returns whether it
    /// changed — an unchanged value writes nothing and appends no event. `by` is `None` when
    /// `serve` / `daemon install` set it (data-dir verbs, no Member).
    pub fn set_public_url(&self, by: Option<&str>, url: Option<&str>) -> Result<bool> {
        self.set_url_setting(PUBLIC_URL_KEY, "setting.public_url", "public_url", by, url)
    }

    /// Set or clear the link relay base; see [`Store::set_public_url`].
    pub fn set_link_base(&self, by: Option<&str>, url: Option<&str>) -> Result<bool> {
        self.set_url_setting(LINK_BASE_KEY, "setting.link_base", "link_base", by, url)
    }

    fn set_url_setting(
        &self,
        key: &'static str,
        event: &'static str,
        what: &'static str,
        by: Option<&str>,
        url: Option<&str>,
    ) -> Result<bool> {
        let value = match url.map(str::trim).filter(|u| !u.is_empty()) {
            Some(u) => Some(normalize_base_url(u).map_err(|reason| StoreError::Invalid { what, reason })?),
            None => None,
        };
        if self.setting(key)?.filter(|v| !v.trim().is_empty()) == value {
            return Ok(false);
        }
        self.tx(|tx| {
            match &value {
                Some(v) => {
                    tx.execute(
                        "INSERT INTO settings(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                        params![key, v],
                    )?;
                }
                None => {
                    tx.execute("DELETE FROM settings WHERE key = ?1", [key])?;
                }
            }
            append_event(tx, by, event, None, serde_json::json!({ "url": value }))?;
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: S18-04
    #[test]
    fn base_urls_are_validated_and_normalised() {
        assert_eq!(
            normalize_base_url("http://Studio.example.ts.net:7377/").unwrap(),
            "http://studio.example.ts.net:7377"
        );
        assert_eq!(normalize_base_url(" HTTPS://cd.example.dev ").unwrap(), "https://cd.example.dev");
        assert_eq!(normalize_base_url("http://100.64.0.7:7377").unwrap(), "http://100.64.0.7:7377");
        assert_eq!(normalize_base_url("http://[fd7a::1]:7377").unwrap(), "http://[fd7a::1]:7377");
        for bad in [
            "my-host:7377",
            "ftp://x",
            "http://",
            "http://x/path",
            "http://x?q=1",
            "http://x#f",
            "http://u@x",
            "http://x:0",
            "http://x:99999",
            "http://x:7a",
            "http://bad_host",
            "http://[nope]:1",
        ] {
            assert!(normalize_base_url(bad).is_err(), "{bad} must be refused");
        }
    }

    // spec: S18-05
    #[test]
    fn doc_links_plain_relay_and_the_relay_rule() {
        let id = "k7q2m9xd";
        assert_eq!(doc_link(None, Some("https://cd.example.dev"), id, None), None);
        assert_eq!(
            doc_link(Some("http://studio.example.ts.net:7377"), None, id, Some("t7")).unwrap(),
            "http://studio.example.ts.net:7377/d/k7q2m9xd#t7"
        );
        // http with a port: the relay sends browsers to http://authority/…
        assert_eq!(
            doc_link(Some("http://studio.example.ts.net:7377"), Some("https://cd.example.dev"), id, None).unwrap(),
            "https://cd.example.dev/studio.example.ts.net:7377/d/k7q2m9xd"
        );
        // https without a port: the relay sends browsers to https://authority/…
        assert_eq!(
            doc_link(Some("https://cd-home.example.net"), Some("https://cd.example.dev/"), id, Some("t2")).unwrap(),
            "https://cd.example.dev/cd-home.example.net/d/k7q2m9xd#t2"
        );
        // Shapes the relay rule cannot reconstruct get the plain public link.
        assert_eq!(
            doc_link(Some("http://my-host.lan"), Some("https://cd.example.dev"), id, None).unwrap(),
            "http://my-host.lan/d/k7q2m9xd"
        );
        assert_eq!(
            doc_link(Some("https://h.example.net:8443"), Some("https://cd.example.dev"), id, None).unwrap(),
            "https://h.example.net:8443/d/k7q2m9xd"
        );
        assert_eq!(
            doc_link(Some("http://[fd7a::1]:7377"), Some("https://cd.example.dev"), id, Some("t1")).unwrap(),
            "http://[fd7a::1]:7377/d/k7q2m9xd#t1"
        );
    }

    // spec: S18-08
    #[test]
    fn pairing_profile_and_url() {
        assert_eq!(pair_profile("rowan", "http://studio.example.ts.net:7377"), "rowan@studio");
        assert_eq!(pair_profile("rowan", "http://100.64.0.7:7377"), "rowan@100.64.0.7");
        assert_eq!(pair_profile("rowan", "https://cd.example.dev"), "rowan@cd");
        assert_eq!(pair_profile("ana", "http://[fd7a::1]:7377"), "ana@fd7a::1");
        assert_eq!(
            pair_url("http://studio.example.ts.net:7377", "ant_ab+/=", "rowan@studio"),
            "annotary://pair?v=1&server=http%3A%2F%2Fstudio.example.ts.net%3A7377&token=ant_ab%2B%2F%3D&profile=rowan%40studio"
        );
        assert_eq!(percent_encode("a-b._~ é"), "a-b._~%20%C3%A9");
    }
}
