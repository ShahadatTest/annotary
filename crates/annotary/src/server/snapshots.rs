//! The snapshot ticker (CMD-1): a consistent copy of the store under `DATA_DIR/snapshots/`.
//!
//! A file-by-file backup of the data dir (an hourly restic run, say) can catch the live WAL
//! database mid-write; it always catches a finished tar whole. So `server::run` spawns [`ticker`]
//! next to the housekeeping one: one snapshot right after start (a restarted or freshly deployed
//! server has a current copy at once), then every [`DEFAULT_INTERVAL`] (15 min, so the copy a
//! backup takes is at most that old; `ANNOTARY_SNAPSHOT_INTERVAL` = seconds, a test seam).
//! Each tick is [`annotary_store::take_snapshot`]: an `export_tar` of `store/` unless nothing
//! changed since the newest snapshot (then only its mtime moves), keeping the newest [`KEEP`].
//! `deploy/healthcheck.sh` reads the newest snapshot's age. Restore = untar the newest
//! `snapshots/annotary-*.tar` into a fresh data dir and `serve --data-dir` there.
//!
//! The tick runs on a blocking thread; it reads the store through its own read-only SQLite
//! connection (`snapshot_db`), so it never holds the server's connection while copying.

use std::path::{Path, PathBuf};
use std::time::Duration;

use annotary_store::{SNAPSHOTS_DIR, SnapshotOutcome, Store, StoreError, take_snapshot};

/// How often a snapshot is taken (the backup it feeds runs hourly).
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(15 * 60);

/// Env var (seconds) that changes the interval — tests only; `0`/unparsable ⇒ default.
pub const INTERVAL_ENV: &str = "ANNOTARY_SNAPSHOT_INTERVAL";

/// Snapshots kept: an hour of changes at the default interval; restic keeps the history.
pub const KEEP: usize = 4;

/// `DATA_DIR/snapshots`.
pub fn snapshots_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(SNAPSHOTS_DIR)
}

/// Tries per tick: a purge racing the copy makes `export_tar` fail `ExportIncomplete` (CMD-9), and
/// a fresh try no longer references what the purge removed (as `annotary export` retries).
pub const ATTEMPTS: u32 = 3;

/// One tick at `now`: snapshot the store into `DATA_DIR/snapshots` if it changed.
pub fn tick(store: &Store, data_dir: &Path, now: jiff::Timestamp) -> Result<SnapshotOutcome, StoreError> {
    let mut tried = 0;
    loop {
        tried += 1;
        let seq = store.last_seq()?; // re-read: the racing purge appended an event
        match take_snapshot(store.dir(), &snapshots_dir(data_dir), seq, now, KEEP) {
            Err(StoreError::ExportIncomplete { missing }) if tried < ATTEMPTS => {
                tracing::debug!("snapshot: {} blobs went away during the copy; trying again", missing.len())
            }
            other => return other,
        }
    }
}

/// The interval from the environment (`ANNOTARY_SNAPSHOT_INTERVAL` seconds) or the default.
pub fn interval_from_env() -> Duration {
    std::env::var(INTERVAL_ENV)
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|s| *s > 0)
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_INTERVAL)
}

/// The ticker task: a tick now, then every `interval`, each on a blocking thread, logged. Runs
/// until aborted (`server::run` aborts it when the listener stops).
pub async fn ticker(store: std::sync::Arc<Store>, data_dir: PathBuf, interval: Duration) {
    let mut timer = tokio::time::interval(interval);
    timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        timer.tick().await; // the first tick completes at once: the snapshot after start
        let (store, dir) = (store.clone(), data_dir.clone());
        match tokio::task::spawn_blocking(move || tick(&store, &dir, jiff::Timestamp::now())).await {
            Ok(Ok(SnapshotOutcome::Written { file, report, removed })) => tracing::info!(
                "snapshot: wrote {} ({} blobs, {} bytes), removed {} old",
                file.path.display(),
                report.blobs,
                report.bytes,
                removed.len()
            ),
            Ok(Ok(SnapshotOutcome::Unchanged { file })) => {
                tracing::debug!("snapshot: unchanged since {}", file.path.display())
            }
            Ok(Err(e)) => tracing::warn!("snapshot failed: {e}"),
            Err(e) => tracing::warn!("snapshot task panicked: {e}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_env_parses_seconds_and_falls_back() {
        // Only the parse; the env itself is process-global, so no set_var here.
        assert_eq!(interval_from_env(), DEFAULT_INTERVAL);
        assert_eq!(snapshots_dir(Path::new("/d")), PathBuf::from("/d/snapshots"));
    }
}
