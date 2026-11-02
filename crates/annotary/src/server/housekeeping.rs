//! The hourly housekeeping ticker (ticket 13) and its API.
//!
//! | route | who | what |
//! |---|---|---|
//! | `POST /api/v1/housekeeping` | owner | run one tick now → the [`Report`] as JSON (`{purged[], gc_removed, gc_bytes, html_dropped, derived_removed, derived_bytes, temp_removed, took_ms}`) |
//! | `PUT /api/v1/settings/auto-purge {days}` | owner | `days` ≥ 1 turns "purge deleted after N days" on, `null`/`0` off → `{auto_purge_days, changed}` (the web settings screen, ticket 12) |
//!
//! `GET /api/v1/settings` (ticket 06, `access.rs`) reports `auto_purge_days` too.
//!
//! A tick = [`Store::auto_purge`] (when the setting is on) → [`Store::gc_blobs`] (every blob no
//! Version references, older than [`GC_GRACE`]) → [`Store::clean_derived`] (rendered HTML whose
//! `renderer` is below `MIN_RENDERER_VERSION`, its now-unreferenced blobs, leftover temp files).
//! `server::run` spawns [`ticker`] next to the listener: the first tick fires one interval after
//! start, then every interval ([`DEFAULT_INTERVAL`] = 1 h; `ANNOTARY_HOUSEKEEPING_INTERVAL`
//! = seconds, a test seam). An explicit `purge` (`docs.rs`) also runs the blob GC right after.
//! The tick runs on a blocking thread and never holds the store across an await.

use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_renderer::is_current;
use annotary_store::{DerivedCleanup, PurgeOutcome, Store, StoreError};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;

/// How often the ticker runs (an hour — the spec's "hourly in-process ticker").
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(3600);

/// Env var (seconds) that shortens the interval — tests only; `0`/unparsable ⇒ default.
pub const INTERVAL_ENV: &str = "ANNOTARY_HOUSEKEEPING_INTERVAL";

/// Blobs younger than this are never swept: an upload whose commit is still on its way (a
/// push uploads every lacking blob, then commits).
pub const GC_GRACE: Duration = Duration::from_secs(3600);

/// What one tick did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub purged: Vec<PurgeOutcome>,
    pub gc_removed: Vec<String>,
    pub gc_bytes: u64,
    pub derived: DerivedCleanup,
    pub took: Duration,
}

impl Report {
    /// True when the tick changed nothing (the log line is then `debug`, not `info`).
    pub fn is_noop(&self) -> bool {
        self.purged.is_empty()
            && self.gc_removed.is_empty()
            && self.derived.html_dropped == 0
            && self.derived.blobs_removed.is_empty()
            && self.derived.temp_removed == 0
    }

    /// One log line: `housekeeping: purged 2 Documents, GC removed 5 blobs (1.2 MB), cleaned 3
    /// stale renders (3 blobs, 12 KB), 1 temp file (41 ms)`.
    pub fn summary(&self) -> String {
        use annotary_store::caps::human_size;
        let plural = |n: usize, w: &str| if n == 1 { format!("{n} {w}") } else { format!("{n} {w}s") };
        format!(
            "housekeeping: purged {}, GC removed {} ({}), cleaned {} ({}, {}), {} ({} ms)",
            plural(self.purged.len(), "Document"),
            plural(self.gc_removed.len(), "blob"),
            human_size(self.gc_bytes),
            plural(self.derived.html_dropped, "stale render"),
            plural(self.derived.blobs_removed.len(), "blob"),
            human_size(self.derived.bytes),
            plural(self.derived.temp_removed, "temp file"),
            self.took.as_millis()
        )
    }

    pub fn json(&self) -> Value {
        json!({
            "purged": self.purged.iter().map(|p| json!({ "id": p.id, "name": p.name, "blobs_removed": p.blobs_removed.len() })).collect::<Vec<_>>(),
            "gc_removed": self.gc_removed.len(),
            "gc_bytes": self.gc_bytes,
            "html_dropped": self.derived.html_dropped,
            "derived_removed": self.derived.blobs_removed.len(),
            "derived_bytes": self.derived.bytes,
            "temp_removed": self.derived.temp_removed,
            "took_ms": self.took.as_millis() as u64,
        })
    }
}

/// Stale = the read path would re-render it anyway (`ensure_html`, ticket 09).
pub fn is_stale_renderer(recorded: Option<&str>) -> bool {
    !is_current(recorded)
}

/// Run one tick at `now` (injectable for tests): auto-purge → blob GC → derived-blob cleanup.
pub fn tick(store: &Store, now: jiff::Timestamp) -> Result<Report, StoreError> {
    let started = Instant::now();
    let purged = store.auto_purge(now)?;
    let (gc_removed, gc_bytes) = store.gc_sweep(GC_GRACE)?;
    let derived = store.clean_derived(&is_stale_renderer, GC_GRACE)?;
    Ok(Report { purged, gc_removed, gc_bytes, derived, took: started.elapsed() })
}

/// The interval from the environment (`ANNOTARY_HOUSEKEEPING_INTERVAL` seconds) or the default.
pub fn interval_from_env() -> Duration {
    std::env::var(INTERVAL_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_INTERVAL)
}

/// The ticker task: every `interval`, one [`tick`] on a blocking thread, logged. Runs until
/// aborted (`server::run` aborts it when the listener stops).
pub async fn ticker(state: AppState, interval: Duration) {
    let mut timer = tokio::time::interval(interval);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    timer.tick().await; // the first tick of a tokio interval completes immediately — skip it
    loop {
        timer.tick().await;
        let store = state.store.clone();
        match tokio::task::spawn_blocking(move || tick(&store, jiff::Timestamp::now())).await {
            Ok(Ok(report)) => {
                if report.is_noop() {
                    tracing::debug!("{}", report.summary());
                } else {
                    tracing::info!("{}", report.summary());
                }
            }
            Ok(Err(e)) => tracing::warn!("housekeeping failed: {e}"),
            Err(e) => tracing::warn!("housekeeping task panicked: {e}"),
        }
    }
}

/// `POST /api/v1/housekeeping` (owner): run one tick now.
pub async fn run_now(State(st): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require_owner("housekeeping")?;
    let store = st.store.clone();
    let report = tokio::task::spawn_blocking(move || tick(&store, jiff::Timestamp::now()))
        .await
        .map_err(|e| ApiError::internal(format!("housekeeping task failed: {e}")))??;
    tracing::info!("{} (by @{})", report.summary(), auth.0.handle);
    Ok(Json(report.json()))
}

#[derive(Debug, Deserialize, Default)]
pub struct AutoPurgeBody {
    #[serde(default)]
    pub days: Option<u32>,
}

/// `PUT /api/v1/settings/auto-purge {days}` (owner): `days` ≥ 1 on, `null`/`0` off.
pub async fn set_auto_purge(
    State(st): State<AppState>,
    auth: Auth,
    body: Result<Json<AutoPurgeBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("the auto-purge setting")?;
    let Json(body) = body?;
    let changed = st.store.set_auto_purge_days(&auth.0.member_id, body.days)?;
    let days = st.store.auto_purge_days()?;
    if changed {
        match days {
            Some(d) => {
                tracing::info!("auto-purge on: deleted Documents are purged after {d} days (by @{})", auth.0.handle)
            }
            None => tracing::info!("auto-purge off (by @{})", auth.0.handle),
        }
    }
    Ok(Json(json!({ "auto_purge_days": days, "changed": changed })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_and_json_shape() {
        let r = Report::default();
        assert!(r.is_noop());
        assert_eq!(
            r.summary(),
            "housekeeping: purged 0 Documents, GC removed 0 blobs (0 B), cleaned 0 stale renders (0 blobs, 0 B), 0 temp files (0 ms)"
        );
        let r = Report {
            purged: vec![PurgeOutcome { id: "a".into(), name: "a".into(), blobs_removed: vec!["x".into()] }],
            gc_removed: vec!["y".into()],
            gc_bytes: 2048,
            derived: DerivedCleanup { html_dropped: 1, blobs_removed: vec!["z".into()], bytes: 10, temp_removed: 1 },
            took: Duration::from_millis(7),
        };
        assert!(!r.is_noop());
        assert_eq!(
            r.summary(),
            "housekeeping: purged 1 Document, GC removed 1 blob (2 KB), cleaned 1 stale render (1 blob, 10 B), 1 temp file (7 ms)"
        );
        let j = r.json();
        assert_eq!(j["purged"][0]["name"], "a");
        assert_eq!(j["gc_removed"], 1);
        assert_eq!(j["html_dropped"], 1);
        assert_eq!(j["took_ms"], 7);
    }

    #[test]
    fn interval_env_parses_seconds_and_falls_back() {
        // The env is process-global; only assert the default path here (the real-binary test
        // covers the override).
        assert_eq!(interval_from_env(), DEFAULT_INTERVAL);
        assert!(is_stale_renderer(None));
        assert!(is_stale_renderer(Some("0.0.1")));
        assert!(!is_stale_renderer(Some(annotary_renderer::RENDERER_VERSION)));
    }
}
