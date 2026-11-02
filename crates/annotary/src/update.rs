//! The update notice: tells a person when a newer release exists (`docs/install.md`, Upgrade).
//!
//! It asks GitHub's Releases API at most once a day and remembers the answer in the data dir's
//! `update-check.json`; nothing but that request and a `User-Agent` of this version leaves the
//! machine. It never runs on the hot path: only `-V` on a terminal waits for it (bounded by
//! [`TIMEOUT`]), `serve` does it on a thread of its own after it starts listening, and no other
//! verb reads it. It fails silently: offline, a 404 (a repo not public yet), a bad answer or a
//! data dir it cannot write each mean no notice, and the attempt still counts for the day.
//!
//! Off with `update_check = false` in `config.toml`, or `ANNOTARY_UPDATE_CHECK=off`. A build with
//! no release version (`0.0.0`, from a checkout) has nothing to compare, so it never asks.
//! `ANNOTARY_UPDATE_URL` points the check elsewhere (the tests' fake) and makes `0.0.0` count.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::Config;

const RELEASES_URL: &str = "https://api.github.com/repos/josh-andrews-dev/annotary/releases/latest";
const UPGRADE_DOC: &str = "https://github.com/josh-andrews-dev/annotary/blob/main/docs/install.md#upgrade";
const CACHE_FILE: &str = "update-check.json";
const CHECK_EVERY_SECS: u64 = 24 * 60 * 60;
const TIMEOUT: Duration = Duration::from_secs(3);

/// What the last check learned. A check that learned nothing keeps the last `latest`.
#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
struct Cache {
    checked_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    latest: Option<String>,
}

/// `X.Y.Z` (a leading `v` allowed) as numbers; anything else, a prerelease included, is `None`.
fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.strip_prefix('v').unwrap_or(text).split('.');
    let mut number = || parts.next()?.parse::<u64>().ok();
    let version = (number()?, number()?, number()?);
    parts.next().is_none().then_some(version)
}

fn is_newer(latest: &str, current: &str) -> bool {
    matches!((parse_version(latest), parse_version(current)), (Some(l), Some(c)) if l > c)
}

/// Whether to check at all, from what `Checker::for_data_dir` reads.
fn enabled(current: &str, has_override: bool, env_off: bool, config_off: bool) -> bool {
    !env_off && !config_off && (has_override || current != "0.0.0")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Due when never checked, a day has passed, or the clock has gone backwards past the last check.
fn is_due(cache: Option<&Cache>, now: u64) -> bool {
    match cache {
        None => true,
        Some(c) => now < c.checked_at || now - c.checked_at >= CHECK_EVERY_SECS,
    }
}

/// The notice for one data dir, or `None` from [`Checker::for_data_dir`] when it is off.
pub struct Checker {
    cache_path: PathBuf,
    url: String,
    current: &'static str,
}

impl Checker {
    pub fn for_data_dir(root: &Path) -> Option<Checker> {
        let url_override = std::env::var("ANNOTARY_UPDATE_URL").ok().filter(|u| !u.is_empty());
        let env_off = std::env::var("ANNOTARY_UPDATE_CHECK").is_ok_and(|v| v.eq_ignore_ascii_case("off"));
        let config = root.join("config.toml");
        let config_off = Config::load(&config).ok().and_then(|c| c.update_check) == Some(false);
        enabled(crate::VERSION, url_override.is_some(), env_off, config_off).then(|| Checker {
            cache_path: root.join(CACHE_FILE),
            url: url_override.unwrap_or_else(|| RELEASES_URL.to_string()),
            current: crate::VERSION,
        })
    }

    fn read_cache(&self) -> Option<Cache> {
        serde_json::from_slice(&std::fs::read(&self.cache_path).ok()?).ok()
    }

    fn write_cache(&self, cache: &Cache) -> bool {
        let tmp = self.cache_path.with_extension(format!("json.{}.tmp", std::process::id()));
        let wrote = self.cache_path.parent().is_some_and(|p| std::fs::create_dir_all(p).is_ok())
            && serde_json::to_vec(cache).is_ok_and(|bytes| std::fs::write(&tmp, bytes).is_ok())
            && std::fs::rename(&tmp, &self.cache_path).is_ok();
        if !wrote {
            let _ = std::fs::remove_file(&tmp);
        }
        wrote
    }

    /// The notice the last check supports, if any. Reads one small file.
    pub fn notice(&self) -> Option<String> {
        let latest = self.read_cache()?.latest?;
        is_newer(&latest, self.current).then(|| {
            format!(
                "annotary {latest} is available (you have {}); see {UPGRADE_DOC}, or set update_check = false in config.toml to stop this notice",
                self.current
            )
        })
    }

    /// Asks when a check is due; returns whether it asked. The day's attempt is written first, so
    /// a failure or a second process does not ask again, and a data dir it cannot write means no
    /// request at all (it could not keep to once a day).
    pub fn refresh_if_due(&self) -> bool {
        let cache = self.read_cache();
        if !is_due(cache.as_ref(), now()) {
            return false;
        }
        let latest = cache.and_then(|c| c.latest);
        if !self.write_cache(&Cache { checked_at: now(), latest }) {
            return false;
        }
        if let Some(tag) = self.fetch_latest() {
            self.write_cache(&Cache { checked_at: now(), latest: Some(tag) });
        }
        true
    }

    /// The newest release's `X.Y.Z`, or `None` on any failure.
    fn fetch_latest(&self) -> Option<String> {
        let config = ureq::Agent::config_builder().timeout_global(Some(TIMEOUT)).build();
        let agent = ureq::Agent::new_with_config(config);
        let mut response = agent
            .get(&self.url)
            .header("User-Agent", &format!("annotary/{}", self.current))
            .header("Accept", "application/vnd.github+json")
            .call()
            .ok()?;
        let body: serde_json::Value = response.body_mut().read_json().ok()?;
        let tag = body.get("tag_name")?.as_str()?;
        parse_version(tag)?;
        Some(tag.strip_prefix('v').unwrap_or(tag).to_string())
    }
}

/// After `-V`: on a terminal a due check runs first; anywhere the known notice goes to stderr, so
/// a script reading stdout sees only the version. Uses `ANNOTARY_HOME` or `~/.annotary`.
pub fn after_version() {
    let root = crate::paths::data_dir(None, std::env::var("ANNOTARY_HOME").ok().as_deref());
    let Some(check) = Checker::for_data_dir(&root) else { return };
    if crate::output::Out::resolve(None, false, false).tty {
        check.refresh_if_due();
    }
    if let Some(notice) = check.notice() {
        eprintln!("{notice}");
    }
}

/// At `serve` start: logs the known notice, then checks on a thread of its own, which never holds
/// up the server and does not outlive the process, and logs what it learns.
pub fn at_serve_start(root: &Path) {
    let Some(check) = Checker::for_data_dir(root) else { return };
    let known = check.notice();
    if let Some(notice) = &known {
        tracing::info!("{notice}");
    }
    std::thread::spawn(move || {
        if check.refresh_if_due() {
            match check.notice() {
                Some(notice) if Some(&notice) != known.as_ref() => tracing::info!("{notice}"),
                _ => {}
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_compare_as_numbers_and_only_plain_ones_count() {
        assert!(is_newer("0.10.0", "0.9.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.9.9", "0.9.9"));
        assert!(!is_newer("0.9.8", "0.9.9"));
        for bad in ["", "1.2", "1.2.3.4", "1.2.3-rc.1", "latest", "1.x.3"] {
            assert_eq!(parse_version(bad), None, "{bad}");
            assert!(!is_newer(bad, "0.0.1"), "{bad}");
        }
    }

    #[test]
    fn a_build_with_no_release_version_never_asks_unless_pointed_elsewhere() {
        assert!(enabled("0.3.0", false, false, false));
        assert!(!enabled("0.0.0", false, false, false));
        assert!(enabled("0.0.0", true, false, false));
        assert!(!enabled("0.3.0", false, true, false));
        assert!(!enabled("0.3.0", false, false, true));
        assert!(!enabled("0.0.0", true, true, false), "off beats the override");
    }

    #[test]
    fn a_check_is_due_after_a_day_or_when_the_clock_went_back() {
        let cache = |checked_at| Cache { checked_at, latest: None };
        assert!(is_due(None, 100));
        assert!(!is_due(Some(&cache(100)), 100));
        assert!(!is_due(Some(&cache(100)), 100 + CHECK_EVERY_SECS - 1));
        assert!(is_due(Some(&cache(100)), 100 + CHECK_EVERY_SECS));
        assert!(is_due(Some(&cache(100)), 99));
    }
}
