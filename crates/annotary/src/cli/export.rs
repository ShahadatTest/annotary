//! `export FILE.tar [--force]` (ticket 13): a consistent tarball of the data dir's `store/`.
//!
//! Works with or without a running server: the database goes in as one snapshot taken with
//! SQLite's online backup API (`annotary_store::backup`), then `meta.json` and every blob
//! file. Nothing else of the data dir travels (`config.toml` holds your profiles, `server.json`
//! and `logs/` are the running server's). Restore = untar into a fresh data dir and `serve`
//! there. Result line: `FILE  N blobs  SIZE  schema vK`; refuses to overwrite without `--force`.
//!
//! This verb reads `store/` directly (read-only) rather than going through the server — a
//! backup must work when no server runs, and the snapshot API is safe beside a live one.
//!
//! CMD-9: an export is complete or it fails. A blob the snapshot references that a purge / GC
//! removed before it was copied makes the store refuse the attempt; the temp file is removed and
//! a fresh snapshot tried (up to [`ATTEMPTS`]), then exit 5 with the missing hashes.

use std::path::PathBuf;

use clap::Args;
use serde_json::{Map, json};

use annotary_store::StoreError;
use annotary_store::caps::human_size;

use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode};
use crate::paths;

use super::Ctx;

pub const AFTER_HELP: &str =
    "Writes store/meta.json, a consistent snapshot of store/annotary.sqlite (SQLite online backup — a
running server is fine) and every blob; LOCK, -wal and temp files stay out. Restore: untar into a
fresh data dir and `annotary serve --data-dir` there. Prints `FILE  N blobs  SIZE  schema vK`.";

#[derive(Args, Debug)]
pub struct ExportArgs {
    /// Where to write the archive (must end in .tar)
    #[arg(value_name = "FILE.tar")]
    pub file: PathBuf,
    /// Overwrite FILE.tar when it exists
    #[arg(long)]
    pub force: bool,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &["file", "blobs", "bytes", "size", "schema_version", "tenant_id", "store", "live"],
    default: &["file", "blobs", "bytes", "schema_version"],
};

/// How many snapshots `export` tries before it gives up on blobs vanishing under it (CMD-9).
pub const ATTEMPTS: u32 = 3;

/// One export attempt's failure: the store's (an incomplete export is retried) or the CLI's own.
pub enum ExportFailure {
    Store(StoreError),
    Cli(CliError),
}

impl From<StoreError> for ExportFailure {
    fn from(e: StoreError) -> Self {
        ExportFailure::Store(e)
    }
}

/// Run `attempt` up to `attempts` times while it fails with `ExportIncomplete` (`on_retry` gets
/// the missing hashes before each retry); map the final failure to a `CliError` — an export
/// still incomplete after the last attempt is refused (exit 5) with the hashes listed.
pub fn with_retries<T>(
    attempts: u32,
    mut attempt: impl FnMut() -> Result<T, ExportFailure>,
    mut on_retry: impl FnMut(&[String]),
) -> CliResult<T> {
    let mut tried = 0;
    loop {
        tried += 1;
        match attempt() {
            Ok(v) => return Ok(v),
            Err(ExportFailure::Cli(e)) => return Err(e),
            Err(ExportFailure::Store(StoreError::ExportIncomplete { missing })) if tried < attempts => {
                on_retry(&missing)
            }
            Err(ExportFailure::Store(StoreError::ExportIncomplete { missing })) => {
                let message = StoreError::ExportIncomplete { missing: missing.clone() }.to_string();
                let mut err = CliError::refused(format!("{message} (after {tried} attempts)")).hint(
                    "nothing was written — a purge or GC removed them during the export; re-run export. \
                     If it keeps failing, these blobs are missing from the store",
                );
                const SHOWN: usize = 10;
                for h in missing.iter().take(SHOWN) {
                    err = err.detail(format!("missing blob {h}"));
                }
                if missing.len() > SHOWN {
                    err = err.detail(format!("… and {} more", missing.len() - SHOWN));
                }
                return Err(err);
            }
            Err(ExportFailure::Store(StoreError::Invalid { what: "store", reason })) => {
                return Err(CliError::refused(reason));
            }
            Err(ExportFailure::Store(other)) => return Err(CliError::internal(format!("export failed: {other}"))),
        }
    }
}

pub fn run(ctx: &Ctx, a: ExportArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    let is_tar = a.file.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("tar"));
    if !is_tar {
        return Err(CliError::usage(format!("{} does not end in .tar", a.file.display()))
            .hint("export writes an uncompressed tar archive — name it FILE.tar"));
    }
    let store_dir = ctx.data.store();
    if !annotary_store::backup::has_store(&store_dir) {
        return Err(CliError::refused(format!("no store at {}", paths::display(&store_dir))).hint(
            "nothing to export yet — `annotary serve` creates it; --data-dir / ANNOTARY_HOME pick another data dir",
        ));
    }
    if a.file.exists() && !a.force {
        return Err(CliError::refused(format!("{} exists", a.file.display())).hint("--force overwrites it"));
    }
    let live = crate::server::running_server(&ctx.data.server_json());
    if let Some(r) = &live {
        ctx.out.note(&format!(
            "export: server running (pid {}) — the database goes in as a consistent snapshot (SQLite online backup)",
            r.pid
        ));
    }
    // Write to a sibling temp file and rename, so an interrupted export never leaves a half tar
    // under the final name.
    let tmp = a.file.with_file_name(format!(
        ".{}.{}.tmp",
        a.file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "export".into()),
        std::process::id()
    ));
    // CMD-9: a purge / GC in the server can remove a blob the snapshot references; the store
    // refuses such an export, the temp file goes and a fresh snapshot is tried.
    let report = with_retries(
        ATTEMPTS,
        || {
            let attempt = (|| -> Result<annotary_store::ExportReport, ExportFailure> {
                let file = std::fs::File::create(&tmp).map_err(|e| ExportFailure::Cli(e.into()))?;
                let report = annotary_store::export_tar(&store_dir, std::io::BufWriter::new(file))?;
                std::fs::rename(&tmp, &a.file).map_err(|e| ExportFailure::Cli(e.into()))?;
                Ok(report)
            })();
            if attempt.is_err() {
                let _ = std::fs::remove_file(&tmp);
            }
            attempt
        },
        |missing| {
            ctx.out.note(&format!(
                "export: {} referenced blob{} removed during the export (a purge or GC ran) — retrying with a fresh snapshot",
                missing.len(),
                if missing.len() == 1 { " was" } else { "s were" }
            ))
        },
    )?;
    let size = std::fs::metadata(&a.file).map(|m| m.len()).unwrap_or(report.bytes);
    if ctx.out.is_json() {
        let mut m = Map::new();
        m.insert("file".into(), json!(a.file.to_string_lossy()));
        m.insert("blobs".into(), json!(report.blobs));
        m.insert("bytes".into(), json!(size));
        m.insert("size".into(), json!(human_size(size)));
        m.insert("schema_version".into(), json!(report.schema_version));
        m.insert("tenant_id".into(), json!(report.tenant_id));
        m.insert("store".into(), json!(store_dir.to_string_lossy()));
        m.insert("live".into(), json!(live.is_some()));
        return ctx.out.json_records(&FIELDS, vec![m], true);
    }
    let fields = [
        a.file.display().to_string(),
        format!("{} blob{}", report.blobs, if report.blobs == 1 { "" } else { "s" }),
        human_size(size),
        format!("schema v{}", report.schema_version),
    ];
    ctx.out.line(&if ctx.out.tty { fields.join("  ") } else { fields.join("\t") });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exit::Exit;

    fn incomplete(n: usize) -> ExportFailure {
        ExportFailure::Store(StoreError::ExportIncomplete { missing: (0..n).map(|i| format!("{i:064x}")).collect() })
    }

    // spec: S21-02
    #[test]
    fn an_incomplete_export_is_retried_then_refused_with_the_missing_blobs() {
        // Vanished once (a purge ran), complete on the fresh snapshot.
        let (mut calls, mut retries) = (0, Vec::new());
        let v = with_retries(
            ATTEMPTS,
            || {
                calls += 1;
                if calls == 1 { Err(incomplete(1)) } else { Ok(7) }
            },
            |m| retries.push(m.len()),
        )
        .unwrap();
        assert_eq!((v, calls, retries), (7, 2, vec![1]));

        // Still incomplete after the last attempt: refused (exit 5), the hashes listed.
        let mut calls = 0;
        let e = with_retries::<()>(
            ATTEMPTS,
            || {
                calls += 1;
                Err(incomplete(12))
            },
            |_| {},
        )
        .unwrap_err();
        assert_eq!(calls, ATTEMPTS);
        assert_eq!(e.exit, Exit::Refused);
        assert_eq!(
            e.message,
            "export incomplete: 12 blobs the database snapshot references are not in the store (after 3 attempts)"
        );
        assert!(e.hint.as_deref().unwrap().contains("re-run export"));
        assert_eq!(e.detail.len(), 11);
        assert_eq!(e.detail[0], format!("missing blob {:064x}", 0));
        assert_eq!(e.detail[10], "… and 2 more");

        // Other failures are not retried.
        let mut calls = 0;
        let e = with_retries::<()>(
            ATTEMPTS,
            || {
                calls += 1;
                Err(ExportFailure::Store(StoreError::Invalid { what: "store", reason: "no store at x".into() }))
            },
            |_| {},
        )
        .unwrap_err();
        assert_eq!((calls, e.exit, e.message.as_str()), (1, Exit::Refused, "no store at x"));
    }
}
