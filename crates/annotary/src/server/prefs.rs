//! The signed-in Member's settings as a web page applies them (CMD-23), read in the same server
//! response as the page so the first paint is final: every page opens its `.cd-app` with
//! [`WebPrefs::app_attrs`].
//!
//! - **Appearance**: `cd-theme-light` / `cd-theme-dark` / `cd-theme-system` on `.cd-app`;
//!   `app.css` carries both palettes, the dark one under `.cd-theme-dark` and, for
//!   `cd-theme-system`, under `@media (prefers-color-scheme: dark)`.
//! - **Highlight colors, text size**: CSS variables in `.cd-app`'s `style` for each palette
//!   (`--cd-hl-light`, `--cd-hl-light-sel`, `--cd-hl-dark`, `--cd-hl-dark-sel`, `--cd-text-scale`);
//!   `app.css` maps `--cd-hl` / `--cd-hl-sel` onto the light or dark pair.
//! - **Show resolved**: `cd-show-resolved` on `.cd-app` (the toggle stays client-side after that).
//! - **Times shown as / Clock**: `data-cd-times` / `data-cd-clock` on `.cd-app`; absolute times
//!   need the browser's time zone, which the script saves in the `cd_tz` cookie ([`tz_cookie`]),
//!   with its hour cycle in `cd_hc` ([`hc_cookie`], what Clock "Follow system" means here). With
//!   both, the page renders every `<time>` absolute on the server ([`WebPrefs::times`] →
//!   [`Times::tag`]) and adds `data-cd-tz`; until then times render relative, as before.
//! - **Keyboard shortcuts / Send key**: `data-cd-keys` / `data-cd-send` on `.cd-app`.
//!
//! Every browser is the Member's one Web Device, so two browsers resolve the same values.

use axum::http::HeaderMap;

use annotary_store::settings::{WEB_DEVICE, highlight_colors};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;
use super::ui::{self, esc};

/// The browser's IANA time zone, saved by the script on the first visit.
pub const TZ_COOKIE: &str = "cd_tz";
/// The browser's hour cycle (`12` / `24`), saved by the script beside `cd_tz`: what Clock
/// "Follow system" means when the server renders absolute times.
pub const HC_COOKIE: &str = "cd_hc";

/// The Web Device's resolved settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebPrefs {
    /// `system`, `light` or `dark`.
    pub appearance: String,
    pub hl_light: String,
    pub hl_light_sel: String,
    pub hl_dark: String,
    pub hl_dark_sel: String,
    /// Percent (100 = 15 px Document text).
    pub text_size: i64,
    pub show_resolved: bool,
    /// `relative` or `absolute`.
    pub times: String,
    /// `system`, `12` or `24`.
    pub clock: String,
    pub keyboard_shortcuts: bool,
    /// `enter` or `mod_enter`.
    pub send_key: String,
    /// The `cd_tz` cookie, when the browser has sent it.
    pub tz: Option<String>,
    /// The `cd_hc` cookie (`12` / `24`), when the browser has sent it.
    pub hc: Option<u8>,
}

impl Default for WebPrefs {
    fn default() -> Self {
        let (hl_light, hl_light_sel) = highlight_colors("yellow", false);
        let (hl_dark, hl_dark_sel) = highlight_colors("yellow", true);
        WebPrefs {
            appearance: "system".into(),
            hl_light,
            hl_light_sel,
            hl_dark,
            hl_dark_sel,
            text_size: 100,
            show_resolved: false,
            times: "relative".into(),
            clock: "system".into(),
            keyboard_shortcuts: true,
            send_key: "enter".into(),
            tz: None,
            hc: None,
        }
    }
}

/// A cookie's raw value.
fn cookie(headers: &HeaderMap, name: &str) -> Option<String> {
    for v in headers.get_all(axum::http::header::COOKIE) {
        let Ok(s) = v.to_str() else { continue };
        for pair in s.split(';') {
            if let Some(val) = pair.trim().strip_prefix(name).and_then(|r| r.strip_prefix('=')) {
                return Some(val.trim().to_string());
            }
        }
    }
    None
}

/// The `cd_hc` cookie: `12` or `24` (anything else is ignored).
pub fn hc_cookie(headers: &HeaderMap) -> Option<u8> {
    match cookie(headers, HC_COOKIE)?.as_str() {
        "12" => Some(12),
        "24" => Some(24),
        _ => None,
    }
}

/// The `cd_tz` cookie (an IANA name such as `Europe/Berlin`; anything else is ignored).
pub fn tz_cookie(headers: &HeaderMap) -> Option<String> {
    let val = cookie(headers, TZ_COOKIE)?.replace("%2F", "/").replace("%2f", "/");
    (!val.is_empty()
        && val.len() <= 64
        && val.bytes().all(|b| b.is_ascii_alphanumeric() || b"/_-+".contains(&b))
        && jiff::tz::TimeZone::get(&val).is_ok())
    .then_some(val)
}

/// How a page renders a timestamp (CMD-23 Times shown as / Clock). `abs` = the browser's zone and
/// whether the clock is 12-hour, set only for Times shown as Absolute with the zone known.
#[derive(Clone, Debug, Default)]
pub struct Times {
    abs: Option<(jiff::tz::TimeZone, bool)>,
}

impl Times {
    /// Relative ages where the page asks for them; the script localizes the rest (pre-CMD-23).
    pub fn relative() -> Times {
        Times::default()
    }

    /// Absolute in `tz` (tests, and [`WebPrefs::times`]).
    pub fn absolute(tz: jiff::tz::TimeZone, h12: bool) -> Times {
        Times { abs: Some((tz, h12)) }
    }

    /// A `<time>` tag: [`ui::time_tag`] when relative; else the time in the browser's zone,
    /// rendered here so the script changes nothing (`cd-localized`), in the shape of the script's
    /// `fmtAbs` (`Oct 30, 2:05 PM`; the year when it is not this year's).
    pub fn tag(&self, iso: &str, rel: bool) -> String {
        let Some((tz, h12)) = &self.abs else { return ui::time_tag(iso, rel) };
        let Some(t) = annotary_store::time::parse(iso) else { return ui::time_tag(iso, rel) };
        let z = t.to_zoned(tz.clone());
        let this_year = jiff::Timestamp::now().to_zoned(tz.clone()).year() == z.year();
        let clock = if *h12 { "%-I:%M %p" } else { "%H:%M" };
        let shape = if this_year { format!("%b %-d, {clock}") } else { format!("%b %-d, %Y, {clock}") };
        format!(
            "<time class=\"cd-abs cd-localized\" datetime=\"{}\" title=\"{}\">{}</time>",
            esc(iso),
            esc(&z.strftime("%Y-%m-%d %H:%M:%S %Z").to_string()),
            esc(&z.strftime(&shape).to_string())
        )
    }
}

impl WebPrefs {
    /// The caller's Web settings (the Web Device is made on first use).
    pub fn load(st: &AppState, auth: &Auth, headers: &HeaderMap) -> Result<WebPrefs, ApiError> {
        let device = st.store.ensure_device(&auth.0.member_id, WEB_DEVICE, None)?;
        let rows = st.store.resolved_settings(&auth.0.member_id, Some(&device.id))?;
        let mut p = WebPrefs { tz: tz_cookie(headers), hc: hc_cookie(headers), ..WebPrefs::default() };
        for r in rows {
            let s = r.value.as_str().unwrap_or_default().to_string();
            match r.def.key {
                "appearance" => p.appearance = s,
                "highlight_light" => (p.hl_light, p.hl_light_sel) = highlight_colors(&s, false),
                "highlight_dark" => (p.hl_dark, p.hl_dark_sel) = highlight_colors(&s, true),
                "text_size" => p.text_size = r.value.as_i64().unwrap_or(100),
                "show_resolved" => p.show_resolved = s == "shown",
                "times" => p.times = s,
                "clock" => p.clock = s,
                "keyboard_shortcuts" => p.keyboard_shortcuts = s != "off",
                "send_key" => p.send_key = s,
                _ => {}
            }
        }
        Ok(p)
    }

    /// The `class` words for `.cd-app` (after `cd-app` itself).
    pub fn classes(&self) -> String {
        let mut c = format!("cd-theme-{}", self.appearance);
        if self.show_resolved {
            c.push_str(" cd-show-resolved");
        }
        c
    }

    /// The inline `style` for `.cd-app`: each palette's highlight pair and the text scale.
    pub fn style(&self) -> String {
        format!(
            "--cd-hl-light:{};--cd-hl-light-sel:{};--cd-hl-dark:{};--cd-hl-dark-sel:{};--cd-text-scale:{}",
            self.hl_light,
            self.hl_light_sel,
            self.hl_dark,
            self.hl_dark_sel,
            self.text_size as f64 / 100.0
        )
    }

    /// Absolute times are shown only once the browser's zone is known.
    pub fn absolute_times(&self) -> bool {
        self.times == "absolute" && self.tz.is_some()
    }

    /// How this page renders its `<time>` tags: absolute in the browser's zone once
    /// [`Self::absolute_times`], else as before (relative ages, the script localizes the rest).
    pub fn times(&self) -> Times {
        let tz = self.tz.as_deref().filter(|_| self.times == "absolute").and_then(|t| jiff::tz::TimeZone::get(t).ok());
        let h12 = match self.clock.as_str() {
            "12" => true,
            "24" => false,
            _ => self.hc != Some(24),
        };
        Times { abs: tz.map(|tz| (tz, h12)) }
    }

    /// `class="cd-app EXTRA …" style="…" data-…` — what every page's `.cd-app` opens with.
    /// `extra` are the page's own classes (`cd-adminpage`, …).
    pub fn app_attrs(&self, extra: &str) -> String {
        let extra = if extra.is_empty() { String::new() } else { format!(" {extra}") };
        format!(
            "class=\"cd-app{extra} {}\" style=\"{}\" data-cd-times=\"{}\" data-cd-clock=\"{}\" data-cd-keys=\"{}\" data-cd-send=\"{}\"{}",
            self.classes(),
            self.style(),
            esc(&self.times),
            esc(&self.clock),
            if self.keyboard_shortcuts { "on" } else { "off" },
            esc(&self.send_key),
            self.tz.as_deref().map(|t| format!(" data-cd-tz=\"{}\"", esc(t))).unwrap_or_default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_system_with_yellow() {
        let p = WebPrefs::default();
        let a = p.app_attrs("cd-docpage");
        assert!(a.starts_with("class=\"cd-app cd-docpage cd-theme-system\""), "{a}");
        assert!(a.contains("--cd-hl-light:#fff1a8;--cd-hl-light-sel:#ffd54a;--cd-hl-dark:#facc15"), "{a}");
        assert!(a.contains("--cd-text-scale:1"), "{a}");
        assert!(!a.contains("data-cd-tz"));
    }

    #[test]
    fn the_tz_cookie_must_be_a_real_zone() {
        let mut h = HeaderMap::new();
        h.insert(axum::http::header::COOKIE, "a=b; cd_tz=Europe%2FBerlin".parse().unwrap());
        assert_eq!(tz_cookie(&h).as_deref(), Some("Europe/Berlin"));
        h.insert(axum::http::header::COOKIE, "cd_tz=Not/AZone".parse().unwrap());
        assert_eq!(tz_cookie(&h), None);
        h.insert(axum::http::header::COOKIE, "cd_tz=America/New_York; cd_hc=24".parse().unwrap());
        assert_eq!(tz_cookie(&h).as_deref(), Some("America/New_York"));
        assert_eq!(hc_cookie(&h), Some(24));
    }

    #[test]
    fn absolute_times_need_the_zone_and_follow_the_clock() {
        let iso = "2024-03-05T17:04:09.000Z";
        let ny = jiff::tz::TimeZone::get("America/New_York").unwrap();
        // An earlier year carries the year, in the zone, as the script's fmtAbs shapes it.
        let t = Times::absolute(ny.clone(), true).tag(iso, true);
        assert_eq!(
            t,
            "<time class=\"cd-abs cd-localized\" datetime=\"2024-03-05T17:04:09.000Z\" title=\"2024-03-05 12:04:09 EST\">Mar 5, 2024, 12:04 PM</time>"
        );
        assert!(Times::absolute(ny, false).tag(iso, false).ends_with(">Mar 5, 2024, 12:04</time>"));
        // Relative (the default, or Absolute before the zone is known) is ui::time_tag.
        assert_eq!(Times::relative().tag(iso, true), ui::time_tag(iso, true));
        let mut p = WebPrefs { times: "absolute".into(), clock: "system".into(), ..WebPrefs::default() };
        assert_eq!(p.times().tag(iso, false), ui::time_tag(iso, false));
        p.tz = Some("Europe/Berlin".into());
        assert!(p.times().tag(iso, false).contains(">Mar 5, 2024, 6:04 PM<"), "no cd_hc: 12-hour");
        p.hc = Some(24);
        assert!(p.times().tag(iso, false).contains(">Mar 5, 2024, 18:04<"));
        p.clock = "12".into();
        assert!(p.times().tag(iso, false).contains(">Mar 5, 2024, 6:04 PM<"));
    }
}
