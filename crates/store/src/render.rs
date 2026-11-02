//! Rendered HTML on a Version (ticket 09): the commit's `html_hash` + `renderer` are validated
//! here, and [`Store::set_version_html`] records a (re-)render — the server's fallback on read
//! and `annotary rerender` — without touching anything else about the Version. Identity,
//! number, `by`/`at`/`message`, Threads and Actions never change; only the derived
//! `html_hash` / `renderer` columns do, with a `version.rendered` event.

use rusqlite::{OptionalExtension, params};

use crate::error::{Result, StoreError};
use crate::model::PushInput;
use crate::store::{Store, append_event};

/// Longest `renderer` string a commit may record.
pub const RENDERER_MAX_LEN: usize = 64;

/// A `renderer` value is a short printable ASCII token (`1.0.0`, `1.2.0-beta`), no whitespace.
pub fn check_renderer(renderer: &str) -> Result<()> {
    let bad = |reason: String| StoreError::Invalid { what: "renderer", reason };
    if renderer.is_empty() {
        return Err(bad("must not be empty".into()));
    }
    if renderer.len() > RENDERER_MAX_LEN {
        return Err(bad(format!("longer than {RENDERER_MAX_LEN} bytes")));
    }
    if !renderer.bytes().all(|b| b.is_ascii_graphic()) {
        return Err(bad(format!("{renderer:?} must be printable ASCII without spaces")));
    }
    Ok(())
}

impl Store {
    /// A commit's render fields: `html_hash` names a blob already in the store (409
    /// `blob_missing` otherwise; 400 when it is not a hash) and `renderer` says which renderer
    /// produced it (a short token, 400 otherwise). Both are stored as sent, either alone is fine
    /// (an `html_hash` without a `renderer`, or with one older than the minimum, is served
    /// through the fallback — re-rendered on first read); neither ⇒ the server renders on demand.
    pub(crate) fn check_render_fields(&self, input: &mut PushInput) -> Result<()> {
        input.renderer = input.renderer.as_deref().map(str::trim).filter(|r| !r.is_empty()).map(str::to_string);
        input.html_hash = input.html_hash.as_deref().map(str::trim).filter(|h| !h.is_empty()).map(str::to_string);
        if let Some(r) = &input.renderer {
            check_renderer(r)?;
        }
        if let Some(h) = &input.html_hash {
            if !crate::blobs::is_blob_hash(h) {
                return Err(StoreError::Invalid {
                    what: "html_hash",
                    reason: format!("{h:?} is not a sha256 hex hash"),
                });
            }
            if !self.blobs.has(h) {
                return Err(StoreError::BlobMissing(h.clone()));
            }
        }
        Ok(())
    }

    /// Record the rendered HTML of a Version (blob `html_hash`, produced by `renderer`). Returns
    /// whether the row changed (same hash + renderer ⇒ nothing written, no event).
    pub fn set_version_html(
        &self,
        by: &str,
        doc_id: &str,
        number: u32,
        html_hash: &str,
        renderer: &str,
    ) -> Result<bool> {
        check_renderer(renderer)?;
        if !self.blobs.has(html_hash) {
            return Err(StoreError::BlobMissing(html_hash.to_string()));
        }
        self.tx(|tx| {
            let current: Option<(Option<String>, Option<String>)> = tx
                .query_row(
                    "SELECT html_hash, renderer FROM versions WHERE doc_id = ?1 AND number = ?2",
                    params![doc_id, number as i64],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((old_hash, old_renderer)) = current else {
                return Err(StoreError::VersionNotFound { doc: doc_id.to_string(), number });
            };
            if old_hash.as_deref() == Some(html_hash) && old_renderer.as_deref() == Some(renderer) {
                return Ok(false);
            }
            tx.execute(
                "UPDATE versions SET html_hash = ?1, renderer = ?2 WHERE doc_id = ?3 AND number = ?4",
                params![html_hash, renderer, doc_id, number as i64],
            )?;
            append_event(
                tx,
                Some(by),
                "version.rendered",
                Some(doc_id),
                serde_json::json!({ "version": number, "renderer": renderer, "html_hash": html_hash, "previous": old_hash }),
            )?;
            Ok(true)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{SetupInput, SetupOutcome};

    fn fresh() -> (tempfile::TempDir, Store, SetupOutcome) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan Vale".into(), password: None })
            .unwrap();
        (dir, store, out)
    }

    // spec: S09-08
    #[test]
    fn commit_render_fields_are_validated_as_a_pair() {
        let (_d, store, out) = fresh();
        let (md, _) = store.put_blob(b"# doc\n").unwrap();
        let (html, _) = store.put_blob(b"<h1 data-l=\"1-1\">doc</h1>\n").unwrap();
        let mut inp = PushInput::new("plan", md.clone());
        inp.html_hash = Some(html.clone());
        inp.renderer = Some("1.0.0".into());
        let doc = store.create_document(&out.owner.id, &out.vault.id, inp).unwrap();
        assert_eq!(doc.head.html_hash.as_deref(), Some(html.as_str()));
        assert_eq!(doc.head.renderer.as_deref(), Some("1.0.0"));
        assert!(crate::version_blob_refs(&md, &doc.head.manifest, doc.head.html_hash.as_deref()).contains(&html));

        let err = |f: &dyn Fn(&mut PushInput)| {
            let mut i = PushInput::new("plan", md.clone());
            f(&mut i);
            store.push_version(&out.owner.id, &doc.id, i).unwrap_err().to_string()
        };
        assert!(
            err(&|i| {
                i.html_hash = Some("f".repeat(64));
                i.renderer = Some("1.0.0".into());
            })
            .contains("not in the store"),
            "unknown blob"
        );
        assert!(
            err(&|i| {
                i.html_hash = Some("nope".into());
                i.renderer = Some("1.0.0".into());
            })
            .contains("not a sha256"),
        );
        assert!(err(&|i| i.renderer = Some("has space".into())).contains("printable ASCII"));
        // No render fields at all is fine (the server renders on demand later).
        let two = store.push_version(&out.owner.id, &doc.id, PushInput::new("plan", md.clone())).unwrap();
        assert!(two.already, "same Markdown, no manifest ⇒ identical to head");
        // Either half alone is stored as sent (and served through the fallback).
        let (md2, _) = store.put_blob(b"# doc two\n").unwrap();
        let mut i = PushInput::new("plan", md2.clone());
        i.renderer = Some("none/0".into());
        let v2 = store.push_version(&out.owner.id, &doc.id, i).unwrap().version;
        assert_eq!(v2.renderer.as_deref(), Some("none/0"));
        assert!(v2.html_hash.is_none());
        let (md3, _) = store.put_blob(b"# doc three\n").unwrap();
        let mut i = PushInput::new("plan", md3);
        i.html_hash = Some(html.clone());
        let v3 = store.push_version(&out.owner.id, &doc.id, i).unwrap().version;
        assert_eq!(v3.html_hash.as_deref(), Some(html.as_str()));
        assert!(v3.renderer.is_none());
    }

    // spec: S09-10, S09-11
    #[test]
    fn set_version_html_replaces_only_the_render_and_logs_an_event() {
        let (_d, store, out) = fresh();
        let (md, _) = store.put_blob(b"# doc\n").unwrap();
        let doc = store.create_document(&out.owner.id, &out.vault.id, PushInput::new("plan", md.clone())).unwrap();
        let before = store.version(&doc.id, 1).unwrap().unwrap();
        assert!(before.html_hash.is_none() && before.renderer.is_none());
        let seq0 = store.last_seq().unwrap();
        let (h1, _) = store.put_blob(b"<p data-l=\"1-1\">one</p>\n").unwrap();
        assert!(store.set_version_html(&out.owner.id, &doc.id, 1, &h1, "1.0.0").unwrap());
        assert!(!store.set_version_html(&out.owner.id, &doc.id, 1, &h1, "1.0.0").unwrap(), "idempotent");
        let after = store.version(&doc.id, 1).unwrap().unwrap();
        assert_eq!(after.html_hash.as_deref(), Some(h1.as_str()));
        assert_eq!(after.renderer.as_deref(), Some("1.0.0"));
        // Nothing else about the Version moved.
        assert_eq!(
            (after.number, &after.hash, &after.md_hash, &after.by, &after.at, &after.name),
            (before.number, &before.hash, &before.md_hash, &before.by, &before.at, &before.name)
        );
        let d = store.document(&doc.id).unwrap().unwrap();
        assert_eq!(d.last_activity_at, d.created_at, "a render is not activity");
        let events = store.events_since(seq0).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, "version.rendered");
        assert_eq!(events[0].payload["version"], 1);
        assert_eq!(events[0].payload["html_hash"], h1);
        // A newer render replaces the hash; the old blob is no longer referenced (GC-able).
        let (h2, _) = store.put_blob(b"<p data-l=\"1-1\">two</p>\n").unwrap();
        assert!(store.set_version_html(&out.owner.id, &doc.id, 1, &h2, "1.1.0").unwrap());
        let removed = store.gc_blobs(std::time::Duration::ZERO).unwrap();
        assert!(removed.contains(&h1) && !removed.contains(&h2), "{removed:?}");
        assert!(store.set_version_html(&out.owner.id, &doc.id, 9, &h2, "1.1.0").is_err(), "no such Version");
        assert!(store.set_version_html(&out.owner.id, &doc.id, 1, &"a".repeat(64), "1.1.0").is_err(), "unknown blob");
        // Purge removes the HTML blob with the Document.
        store.delete_document(&out.owner.id, &doc.id).unwrap();
        let purged = store.purge_document(&out.owner.id, &doc.id).unwrap();
        assert!(purged.blobs_removed.contains(&h2));
    }
}
