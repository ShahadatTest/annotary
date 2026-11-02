//! Re-anchoring on push (ticket 07): every Thread's placement on a new Version — computed at
//! the edge by the CLI ([`compute_placements`], the same function the server runs), submitted
//! with the commit as `placements [{thread, status, range?, line?, exact?}]`, applied here in
//! the push transaction (`current` rewritten, sparse `history` appended — the sidecar crate's
//! `Anchor::record`), and **spot-verified** by the server with the same pure function
//! (`Store::set_verify_edge`; on by default — a mismatch refuses the commit with
//! `InvalidPlacements`, since a wrong placement would be a corrupted Anchor forever). Threads a
//! client left out are placed by the server; a submission computed from a Version that is no
//! longer the head (`placements_from`) is ignored the same way. Orphaned is sticky, and
//! `orphaned ⊥ resolved`: resolved and tombstoned Threads are re-anchored like the rest.
//!
//! Document-level Threads have no Anchor and count as anchored in the summary. Asset Threads
//! anchor by content hash: the hash referenced anywhere in the new manifest ⇒ anchored (the
//! display line updated from `asset_lines`), gone ⇒ orphaned.

use std::collections::HashMap;

use rusqlite::{Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use annotary_diff::{Mapping, Span, Status, asset_status};
use annotary_sidecar::Anchor;
use annotary_sidecar::anchor::{ANCHORED, ORPHANED, REFINED};
use annotary_sidecar::range::{Range, extract};

use crate::error::{Result, StoreError};
use crate::threads::{parse_thread_id, thread_id};

/// One Thread's placement on the new Version — the wire shape inside a commit's `placements[]`
/// and what [`compute_placements`] produces. `range` (spans) / `line` (assets) when not
/// orphaned; `exact` = the text now under a refined span.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubmittedPlacement {
    pub thread: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact: Option<String>,
}

/// Summary of one re-anchoring pass (`threads: N re-anchored — a anchored, r refined, o orphaned`).
#[derive(Clone, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Reanchored {
    /// Every Thread of the Document (Document-level ones count as anchored).
    pub total: u32,
    pub anchored: u32,
    pub refined: u32,
    pub orphaned: u32,
    /// `(thread id, new status)` for every Thread whose status changed on this Version.
    pub changed: Vec<(String, String)>,
    /// How many submitted placements the server checked against its own computation.
    pub verified: u32,
    /// How many Threads the server placed itself (nothing submitted for them, or the submission
    /// was computed from a stale head).
    pub computed: u32,
}

impl Reanchored {
    /// `threads: N re-anchored — a anchored, r refined, o orphaned (+1 refined: t3; +2 orphaned: t4, t8)`
    /// / `… (unchanged)`.
    pub fn line(&self) -> String {
        let mut groups: Vec<String> = Vec::new();
        for st in [REFINED, ORPHANED, ANCHORED] {
            let ids: Vec<&str> = self.changed.iter().filter(|(_, s)| s == st).map(|(t, _)| t.as_str()).collect();
            if !ids.is_empty() {
                groups.push(format!("+{} {st}: {}", ids.len(), ids.join(", ")));
            }
        }
        let suffix = if groups.is_empty() { "unchanged".to_string() } else { groups.join("; ") };
        format!(
            "threads: {} re-anchored — {} anchored, {} refined, {} orphaned ({suffix})",
            self.total, self.anchored, self.refined, self.orphaned
        )
    }
}

/// Parse and shape-check a commit's `placements` (a JSON array; absent/null = none): every entry
/// names a Thread (`t7`), one of the three statuses, a `L:C-L:C` range when it has one, no
/// Thread twice.
pub fn parse_placements(value: &Value) -> Result<Vec<SubmittedPlacement>> {
    let bad = |reason: String| StoreError::InvalidPlacements(reason);
    if value.is_null() {
        return Ok(Vec::new());
    }
    let arr = value.as_array().ok_or_else(|| bad("must be an array".into()))?;
    let mut out: Vec<SubmittedPlacement> = Vec::with_capacity(arr.len());
    for (i, v) in arr.iter().enumerate() {
        let p: SubmittedPlacement = serde_json::from_value(v.clone())
            .map_err(|e| bad(format!("entry {i}: {e} (expected {{thread, status, range?, line?, exact?}})")))?;
        if parse_thread_id(&p.thread).is_none() {
            return Err(bad(format!("entry {i}: {:?} is not a Thread id (t7)", p.thread)));
        }
        if Status::parse(&p.status).is_none() {
            return Err(bad(format!("{}: status {:?} — use anchored|refined|orphaned", p.thread, p.status)));
        }
        if let Some(r) = &p.range {
            Range::parse(r).map_err(|e| bad(format!("{}: {e}", p.thread)))?;
        }
        if out.iter().any(|q| q.thread == p.thread) {
            return Err(bad(format!("{} is placed twice", p.thread)));
        }
        out.push(p);
    }
    Ok(out)
}

/// `hash → refs` of the manifest entries that have content (missing ones excluded), in manifest order.
fn hashes_of(manifest: &Value) -> HashMap<String, Vec<String>> {
    let mut out: HashMap<String, Vec<String>> = HashMap::new();
    if let Some(entries) = manifest.as_array() {
        for e in entries {
            if let (Some(r), Some(h)) = (e.get("ref").and_then(Value::as_str), e.get("hash").and_then(Value::as_str)) {
                out.entry(h.to_string()).or_default().push(r.to_string());
            }
        }
    }
    out
}

/// The pure computation the CLI and the server share: the placement of every anchored Thread
/// (`(id, anchor)`; Document-level Threads carry `None` and are skipped — they never move) from
/// text N to text N+1, given the new Version's manifest and its `(ref, line)` reference lines.
/// Errors only for a corrupt stored range (not on text N).
pub fn compute_placements(
    threads: &[(String, Option<Anchor>)],
    text_n: &str,
    text_n1: &str,
    manifest_n1: &Value,
    asset_lines: &[(String, u32)],
) -> Result<Vec<SubmittedPlacement>> {
    let mapping = Mapping::new(text_n, text_n1);
    let hashes = hashes_of(manifest_n1);
    let line_of = |r: &str| asset_lines.iter().find(|(x, _)| x == r).map(|(_, l)| *l);
    let mut out = Vec::new();
    for (id, anchor) in threads {
        let Some(a) = anchor else { continue };
        if a.is_asset() {
            let hash = a.original.hash.as_deref().unwrap_or("");
            let refs = hashes.get(hash);
            let status = asset_status(a.is_orphaned(), refs.is_some());
            let line = match (status, refs) {
                (Status::Anchored, Some(refs)) => {
                    let original_ref = a.original.reference.as_deref().unwrap_or("");
                    let r = refs.iter().find(|r| *r == original_ref).or(refs.first());
                    r.and_then(|r| line_of(r))
                }
                _ => None,
            };
            out.push(SubmittedPlacement {
                thread: id.clone(),
                status: status.as_str().into(),
                range: None,
                line,
                exact: None,
            });
            continue;
        }
        let range = a
            .current
            .range
            .as_deref()
            .and_then(|r| Range::parse(r).ok())
            .ok_or_else(|| StoreError::InvalidPlacements(format!("{id}: stored anchor has no range")))?;
        let span = Span { range, original: a.original.exact.clone().unwrap_or_default(), orphaned: a.is_orphaned() };
        let p = mapping.place(&span).map_err(|e| StoreError::InvalidPlacements(format!("{id}: stored anchor: {e}")))?;
        out.push(SubmittedPlacement {
            thread: id.clone(),
            status: p.status.as_str().into(),
            range: p.range.map(|r| r.to_string()),
            line: None,
            exact: p.exact,
        });
    }
    Ok(out)
}

/// The summary for `threads` (every Thread, Document-level included) and their placements
/// (`before` = each Thread's status before this Version, from its Anchor).
pub fn summarize(threads: &[(String, Option<Anchor>)], placements: &[SubmittedPlacement]) -> Reanchored {
    let mut r = Reanchored { total: threads.len() as u32, ..Default::default() };
    for (id, anchor) in threads {
        let (before, after) = match anchor {
            None => (ANCHORED, ANCHORED),
            Some(a) => {
                let before = a.current.status.as_deref().unwrap_or(ANCHORED);
                let after = placements.iter().find(|p| &p.thread == id).map(|p| p.status.as_str()).unwrap_or(before);
                (before, after)
            }
        };
        match after {
            REFINED => r.refined += 1,
            ORPHANED => r.orphaned += 1,
            _ => r.anchored += 1,
        }
        if before != after {
            r.changed.push((id.clone(), after.to_string()));
        }
    }
    r
}

/// Apply the placements to every Thread of `doc_id` for the new Version `number` (called
/// inside the push transaction, after the Version row): threads with anchors are loaded, the
/// submitted placement (when the client mapped from the current head) is checked — cheap
/// on-text checks always, full recomputation when `verify` — else computed here; then
/// `Anchor::record` and the row is rewritten.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply_on_push(
    tx: &Transaction,
    doc_id: &str,
    head: u32,
    number: u32,
    text_n: &str,
    text_n1: &str,
    manifest_n1: &Value,
    asset_lines: &[(String, u32)],
    submitted: &Value,
    placements_from: Option<u32>,
    verify: bool,
) -> Result<Reanchored> {
    // Every Thread (Document-level ones too, for the summary), by number.
    let mut threads: Vec<(String, Option<Anchor>)> = Vec::new();
    {
        let mut stmt = tx.prepare("SELECT num, anchor FROM threads WHERE doc_id = ?1 ORDER BY num")?;
        for row in stmt.query_map([doc_id], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, Option<String>>(1)?)))? {
            let (num, anchor) = row?;
            let anchor = anchor.map(|a| serde_json::from_str::<Anchor>(&a)).transpose()?;
            threads.push((thread_id(num), anchor));
        }
    }
    if threads.is_empty() {
        return Ok(Reanchored::default());
    }
    let mut submitted = parse_placements(submitted)?;
    for p in &submitted {
        if !threads.iter().any(|(id, _)| id == &p.thread) {
            return Err(StoreError::InvalidPlacements(format!("no Thread {} in this Document", p.thread)));
        }
        if threads.iter().any(|(id, a)| id == &p.thread && a.is_none()) {
            return Err(StoreError::InvalidPlacements(format!("{} is a Document-level Thread (no Anchor)", p.thread)));
        }
    }
    // A submission mapped from a Version that is no longer the head is not about this diff.
    if placements_from.is_some_and(|v| v != head) {
        submitted.clear();
    }
    let computed = compute_placements(&threads, text_n, text_n1, manifest_n1, asset_lines)?;
    let mut summary = summarize(&threads, &computed);
    let mut applied: Vec<SubmittedPlacement> = Vec::with_capacity(computed.len());
    for c in &computed {
        let (_, anchor) = threads.iter().find(|(id, _)| id == &c.thread).expect("computed from threads");
        let anchor = anchor.as_ref().expect("anchored Thread");
        let chosen = match submitted.iter().find(|s| s.thread == c.thread) {
            None => {
                summary.computed += 1;
                c.clone()
            }
            Some(s) => {
                check_submission(s, anchor, text_n1)?;
                if verify {
                    summary.verified += 1;
                    if s != c {
                        return Err(StoreError::InvalidPlacements(format!(
                            "{}: the server computes {} but the commit says {}",
                            c.thread,
                            describe(c),
                            describe(s)
                        )));
                    }
                }
                s.clone()
            }
        };
        applied.push(chosen);
    }
    if !verify {
        // Trusted submissions may differ from what we would compute: the summary follows them.
        summary = summarize(&threads, &applied);
        summary.computed = applied.iter().filter(|a| !submitted.iter().any(|s| s.thread == a.thread)).count() as u32;
    }
    for p in &applied {
        let (num, anchor) = threads
            .iter_mut()
            .find(|(id, _)| id == &p.thread)
            .map(|(id, a)| (parse_thread_id(id).expect("valid id"), a.as_mut().expect("anchored")))
            .expect("known");
        anchor.record(number, &p.status, p.range.clone(), p.line, p.exact.clone());
        tx.execute(
            "UPDATE threads SET anchor = ?1 WHERE doc_id = ?2 AND num = ?3",
            params![serde_json::to_string(anchor)?, doc_id, num as i64],
        )?;
    }
    Ok(summary)
}

fn describe(p: &SubmittedPlacement) -> String {
    let mut s = p.status.clone();
    if let Some(r) = &p.range {
        s.push_str(&format!(" {r}"));
    }
    if let Some(l) = p.line {
        s.push_str(&format!(" line {l}"));
    }
    if let Some(e) = &p.exact {
        s.push_str(&format!(" {e:?}"));
    }
    s
}

/// The cheap checks every submitted placement gets, verified or not: the status fits the
/// Thread's kind and stickiness, an anchored/refined span has a range that is on the new text
/// (and, for refined, `exact` is what stands there; for anchored, that text is the original
/// quote), an asset placement has no range.
fn check_submission(s: &SubmittedPlacement, anchor: &Anchor, text_n1: &str) -> Result<()> {
    let bad = |reason: String| StoreError::InvalidPlacements(format!("{}: {reason}", s.thread));
    if anchor.is_orphaned() && s.status != ORPHANED {
        return Err(bad("orphaned is sticky".into()));
    }
    if anchor.is_asset() {
        if s.range.is_some() || s.exact.is_some() {
            return Err(bad("an asset Thread has a line, not a range".into()));
        }
        if s.status == REFINED {
            return Err(bad("an asset Thread is anchored or orphaned, never refined".into()));
        }
        return Ok(());
    }
    if s.line.is_some() {
        return Err(bad("a span Thread has a range, not a line".into()));
    }
    match s.status.as_str() {
        ORPHANED => {
            if s.range.is_some() || s.exact.is_some() {
                return Err(bad("an orphaned placement carries no range".into()));
            }
        }
        st => {
            let r = s.range.as_deref().ok_or_else(|| bad(format!("{st} needs a range")))?;
            let r = Range::parse(r).map_err(|e| bad(e.to_string()))?;
            let text = extract(text_n1, &r).map_err(|e| bad(format!("range {r} is not on the new Version: {e}")))?;
            let original = anchor.original.exact.as_deref().unwrap_or("");
            if st == ANCHORED && text != original {
                return Err(bad(format!("anchored, but the text at {r} is {text:?}, not the quote {original:?}")));
            }
            if st == REFINED {
                if text == original {
                    return Err(bad(format!("refined, but the text at {r} equals the quote (that is anchored)")));
                }
                if s.exact.as_deref().is_some_and(|e| e != text) {
                    return Err(bad(format!("refined exact {:?} is not the text at {r} ({text:?})", s.exact)));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DocumentView, PushInput};
    use crate::store::{SetupInput, SetupOutcome, Store};
    use crate::threads::{NewThread, ThreadFilter, ThreadStatusFilter};

    fn fresh() -> (tempfile::TempDir, Store, SetupOutcome) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan Vale".into(), password: None })
            .unwrap();
        (dir, store, out)
    }

    fn input(store: &Store, md: &str, manifest: Value) -> PushInput {
        let (h, _) = store.put_blob(md.as_bytes()).unwrap();
        let mut i = PushInput::new("plan", h);
        i.manifest = manifest;
        i
    }

    fn manifest(store: &Store, reference: &str, bytes: &[u8]) -> Value {
        let (h, _) = store.put_blob(bytes).unwrap();
        serde_json::json!([{ "ref": reference, "hash": h, "size": bytes.len(), "type": "image/png" }])
    }

    const V1: &str = "# Plan\n\n![a](img/a.png)\n\nWe ship the CLI before the web UI so agents can start.\n\nDogfood for two weeks, then invite three external developers.\n";
    const V2: &str = "# Plan\n\nIntro added.\n\n![a](img/a.png)\n\nWe ship the CLI before the web UI so agents can start.\n\nDogfood for two weeks, then invite five external developers.\n";
    const V3: &str = "# Plan\n\nIntro added.\n\nWe ship the CLI before the web UI so agents can start.\n\nSomething else entirely now.\n";

    /// Every Thread, by id (not document order).
    fn all(store: &Store, doc: &DocumentView) -> Vec<crate::threads::ThreadView> {
        let mut ts = store
            .threads(
                &doc.id,
                &ThreadFilter { status: ThreadStatusFilter::All, include_hidden: true, ..Default::default() },
            )
            .unwrap();
        ts.sort_by_key(|t| t.num);
        ts
    }

    // spec: S07-06, S07-07, S07-08, S07-09, S07-10, S07-13
    #[test]
    fn push_reanchors_every_thread_and_verifies_submissions() {
        let (_d, store, out) = fresh();
        let m1 = manifest(&store, "img/a.png", b"png-bytes-1");
        let doc = store.create_document(&out.owner.id, &out.vault.id, input(&store, V1, m1.clone())).unwrap();
        // t1 document, t2 span (anchored through), t3 span (refined then orphaned), t4 asset, t5 resolved span.
        store.new_thread(&out.owner.id, &doc.id, NewThread::document("whole")).unwrap();
        store.new_thread(&out.owner.id, &doc.id, NewThread::span("5:0-5:33", "cli first")).unwrap();
        store.new_thread(&out.owner.id, &doc.id, NewThread::span("7:28-7:60", "three?")).unwrap();
        store.new_thread(&out.owner.id, &doc.id, NewThread::asset("img/a.png", Some(3), "diagram")).unwrap();
        store.new_thread(&out.owner.id, &doc.id, NewThread::span("1:2-1:6", "title")).unwrap();
        store.set_thread_resolved(&out.owner.id, &doc.id, 5, true).unwrap();
        let t4 = store.thread(&doc.id, 4).unwrap().unwrap();
        let a4 = t4.anchor.as_ref().unwrap();
        assert!(a4.is_asset());
        assert_eq!(a4.original.reference.as_deref(), Some("img/a.png"));
        assert_eq!(a4.original.line, Some(3));
        assert_eq!(a4.current.status.as_deref(), Some(ANCHORED));
        // Unknown / missing Assets are refused.
        assert!(matches!(
            store.new_thread(&out.owner.id, &doc.id, NewThread::asset("img/nope.png", None, "x")),
            Err(StoreError::AssetNotOnHead { available, .. }) if available == vec!["img/a.png".to_string()]
        ));

        // v2: the client computes placements exactly like the server would.
        let threads: Vec<(String, Option<Anchor>)> = all(&store, &doc).into_iter().map(|t| (t.id, t.anchor)).collect();
        let lines2 = vec![("img/a.png".to_string(), 5u32)];
        let computed = compute_placements(&threads, V1, V2, &m1, &lines2).unwrap();
        let mut i2 = input(&store, V2, m1.clone());
        i2.placements = serde_json::to_value(&computed).unwrap();
        i2.placements_from = Some(1);
        i2.asset_lines = lines2.clone();
        let out2 = store.push_version(&out.owner.id, &doc.id, i2).unwrap();
        let r = out2.reanchored.unwrap();
        assert_eq!((r.total, r.anchored, r.refined, r.orphaned, r.verified, r.computed), (5, 4, 1, 0, 4, 0));
        assert_eq!(r.changed, vec![("t3".to_string(), REFINED.to_string())]);
        assert_eq!(r.line(), "threads: 5 re-anchored — 4 anchored, 1 refined, 0 orphaned (+1 refined: t3)");
        let ts = all(&store, &doc);
        let a2 = ts[1].anchor.as_ref().unwrap();
        assert_eq!(a2.current.to_string(), "v2:7:0-7:33 anchored");
        assert_eq!(a2.history.len(), 2, "shifted ⇒ a new entry");
        let a3 = ts[2].anchor.as_ref().unwrap();
        assert_eq!(a3.current.to_string(), "v2:9:28-9:59 refined");
        assert_eq!(a3.current.exact.as_deref(), Some("invite five external developers"));
        let a4 = ts[3].anchor.as_ref().unwrap();
        assert_eq!(a4.current.to_string(), "v2:5 anchored", "the asset moved to line 5");
        assert_eq!(
            ts[4].anchor.as_ref().unwrap().current.status.as_deref(),
            Some(ANCHORED),
            "resolved Threads move too"
        );

        // v3 with no placements: the server computes them all (fallback); the asset is gone.
        let m3 = serde_json::json!([]);
        let mut i3 = input(&store, V3, m3);
        i3.asset_lines = vec![];
        let out3 = store.push_version(&out.owner.id, &doc.id, i3).unwrap();
        let r = out3.reanchored.unwrap();
        assert_eq!((r.total, r.anchored, r.refined, r.orphaned, r.verified, r.computed), (5, 3, 0, 2, 0, 4));
        assert_eq!(r.line(), "threads: 5 re-anchored — 3 anchored, 0 refined, 2 orphaned (+2 orphaned: t3, t4)");
        let ts = all(&store, &doc);
        let a3 = ts[2].anchor.as_ref().unwrap();
        assert_eq!(a3.current.to_string(), "v2:9:28-9:59 orphaned", "the last valid placement, stamped v2");
        assert_eq!(a3.history.last().unwrap().to_string(), "v3 orphaned");
        assert_eq!(ts[3].anchor.as_ref().unwrap().current.to_string(), "v2:5 orphaned");

        // v4 = v1 again: orphaned is sticky, even though the quote is back; the summary says so.
        let mut i4 = input(&store, V1, m1.clone());
        i4.asset_lines = vec![("img/a.png".into(), 3)];
        let r = store.push_version(&out.owner.id, &doc.id, i4).unwrap().reanchored.unwrap();
        assert_eq!((r.anchored, r.orphaned), (3, 2));
        assert!(r.changed.is_empty());
        assert_eq!(r.line(), "threads: 5 re-anchored — 3 anchored, 0 refined, 2 orphaned (unchanged)");
        let ts = all(&store, &doc);
        assert!(ts[2].anchor.as_ref().unwrap().is_orphaned());
        assert!(ts[3].anchor.as_ref().unwrap().is_orphaned());
        assert_eq!(ts[1].anchor.as_ref().unwrap().current.to_string(), "v4:5:0-5:33 anchored");

        // A wrong submission is refused (verification on) — nothing written; a submission
        // computed from a stale head is ignored (server computes); with verification off a
        // plausible-but-different placement is stored as sent.
        let head = store.document(&doc.id).unwrap().unwrap().head.number;
        let seq = store.last_seq().unwrap();
        let mut bad = input(&store, V2, m1.clone());
        bad.placements = serde_json::json!([{ "thread": "t2", "status": "anchored", "range": "7:0-7:33" }, { "thread": "t5", "status": "orphaned" }]);
        bad.placements_from = Some(head);
        match store.push_version(&out.owner.id, &doc.id, bad) {
            Err(StoreError::InvalidPlacements(m)) => assert!(m.starts_with("t5: the server computes"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(store.last_seq().unwrap(), seq, "refused commit writes nothing");
        assert_eq!(store.document(&doc.id).unwrap().unwrap().head.number, head);
        let mut stale = input(&store, V2, m1.clone());
        stale.placements = serde_json::json!([{ "thread": "t5", "status": "orphaned" }]);
        stale.placements_from = Some(1);
        stale.asset_lines = vec![("img/a.png".into(), 5)];
        let r = store.push_version(&out.owner.id, &doc.id, stale).unwrap().reanchored.unwrap();
        assert_eq!((r.verified, r.computed), (0, 4), "stale submissions are ignored, not refused");
        assert!(!all(&store, &doc)[4].anchor.as_ref().unwrap().is_orphaned());
        store.set_verify_edge(false);
        assert!(!store.verify_edge());
        let mut trusted = input(&store, V3, serde_json::json!([]));
        trusted.placements = serde_json::json!([{ "thread": "t5", "status": "orphaned" }]);
        trusted.placements_from = Some(head + 1);
        let r = store.push_version(&out.owner.id, &doc.id, trusted).unwrap().reanchored.unwrap();
        assert_eq!((r.verified, r.computed), (0, 3));
        assert!(all(&store, &doc)[4].anchor.as_ref().unwrap().is_orphaned(), "trusted as sent");
        // Shape/on-text checks still apply without verification.
        let mut nonsense = input(&store, V1, m1.clone());
        nonsense.placements = serde_json::json!([{ "thread": "t2", "status": "anchored", "range": "1:0-1:3" }]);
        nonsense.placements_from = Some(head + 2);
        assert!(matches!(store.push_version(&out.owner.id, &doc.id, nonsense), Err(StoreError::InvalidPlacements(_))));
        let mut unknown = input(&store, V1, m1.clone());
        unknown.placements = serde_json::json!([{ "thread": "t99", "status": "anchored" }]);
        assert!(matches!(store.push_version(&out.owner.id, &doc.id, unknown), Err(StoreError::InvalidPlacements(_))));
    }

    // spec: S07-12
    #[test]
    fn time_slice_reconstructs_threads_as_of_a_version() {
        let (_d, store, out) = fresh();
        let m1 = manifest(&store, "img/a.png", b"png-bytes-1");
        let doc = store.create_document(&out.owner.id, &out.vault.id, input(&store, V1, m1.clone())).unwrap();
        store.new_thread(&out.owner.id, &doc.id, NewThread::span("7:28-7:60", "three?")).unwrap(); // t1
        store.new_thread(&out.owner.id, &doc.id, NewThread::document("whole")).unwrap(); // t2
        let mut i2 = input(&store, V2, m1.clone());
        i2.asset_lines = vec![("img/a.png".into(), 5)];
        store.push_version(&out.owner.id, &doc.id, i2).unwrap(); // t1 refined on v2
        store.reply(&out.owner.id, &doc.id, 1, "reply on v2", false).unwrap();
        store.set_thread_resolved(&out.owner.id, &doc.id, 1, true).unwrap(); // resolved at v2
        store.new_thread(&out.owner.id, &doc.id, NewThread::span("3:0-3:5", "intro")).unwrap(); // t3 created v2
        store.push_version(&out.owner.id, &doc.id, input(&store, V3, serde_json::json!([]))).unwrap(); // t1 orphaned on v3
        store.set_thread_resolved(&out.owner.id, &doc.id, 1, false).unwrap(); // reopened at v3
        store.reply(&out.owner.id, &doc.id, 1, "reply on v3", false).unwrap();

        let all_f = ThreadFilter { status: ThreadStatusFilter::All, ..Default::default() };
        // As of v1: only t1 (anchored, no replies, open) and t2.
        let v1 = store.threads_at(&doc.id, 1, &all_f).unwrap();
        assert_eq!(v1.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["t2", "t1"]);
        let t1 = &v1[1];
        assert!(t1.replies.is_empty() && t1.is_open());
        assert_eq!(t1.anchor.as_ref().unwrap().current.to_string(), "v1:7:28-7:60 anchored");
        assert_eq!(t1.anchor.as_ref().unwrap().history.len(), 1);
        // As of v2: t1 refined with the text of v2, one reply, resolved; t3 exists.
        let v2 = store.threads_at(&doc.id, 2, &all_f).unwrap();
        assert_eq!(v2.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["t2", "t3", "t1"]);
        let t1 = &v2[2];
        assert_eq!(t1.replies.len(), 1);
        assert!(!t1.is_open());
        assert_eq!(t1.resolved.as_ref().unwrap().version, 2);
        let a = t1.anchor.as_ref().unwrap();
        assert_eq!(a.current.to_string(), "v2:9:28-9:59 refined");
        assert_eq!(a.current.exact.as_deref(), Some("invite five external developers"));
        // As of v3 (= now): t1 orphaned (listed last), reopened, two replies.
        let v3 = store.threads_at(&doc.id, 3, &all_f).unwrap();
        assert_eq!(v3.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["t2", "t3", "t1"]);
        let t1 = &v3[2];
        assert!(t1.is_open() && t1.replies.len() == 2);
        assert_eq!(t1.anchor.as_ref().unwrap().current.to_string(), "v2:9:28-9:59 orphaned");
        assert_eq!(store.threads(&doc.id, &all_f).unwrap()[2].anchor, t1.anchor, "the head slice is the live view");
        // Filters apply to the slice: open only, anchor status.
        assert_eq!(store.threads_at(&doc.id, 2, &ThreadFilter::default()).unwrap().len(), 2);
        let orphaned = ThreadFilter { anchor_status: Some(vec![ORPHANED.into()]), ..all_f.clone() };
        assert!(store.threads_at(&doc.id, 2, &orphaned).unwrap().is_empty());
        assert_eq!(store.threads_at(&doc.id, 3, &orphaned).unwrap().len(), 1);
        let not_orphaned = ThreadFilter { anchor_status: Some(vec![ANCHORED.into(), REFINED.into()]), ..all_f.clone() };
        assert_eq!(store.threads(&doc.id, &not_orphaned).unwrap().len(), 2, "document-level counts as anchored");
        assert!(matches!(store.threads_at(&doc.id, 9, &all_f), Err(StoreError::VersionNotFound { number: 9, .. })));
    }

    // spec: S07-06, S07-13
    #[test]
    fn placements_parse_compute_and_summarize() {
        let v1 = "# T\n\nOne two three.\n\n![a](img/a.png)\n";
        let v2 = "# T\n\nZero one two three.\n\ntext\n\n![a](img/a.png)\n";
        let threads = vec![
            ("t1".to_string(), None),
            ("t2".to_string(), Some(Anchor::new_span(1, "3:4-3:13", "two three"))),
            ("t3".to_string(), Some(Anchor::new_asset(1, "aaaa", "img/a.png", Some(5)))),
            ("t4".to_string(), Some(Anchor::new_span(1, "3:0-3:3", "One"))),
        ];
        let manifest = serde_json::json!([{ "ref": "img/a.png", "hash": "aaaa" }]);
        let lines = vec![("img/a.png".to_string(), 7u32)];
        let ps = compute_placements(&threads, v1, v2, &manifest, &lines).unwrap();
        assert_eq!(ps.len(), 3, "Document-level Threads are not placed");
        assert_eq!(
            ps[0],
            SubmittedPlacement {
                thread: "t2".into(),
                status: "anchored".into(),
                range: Some("3:9-3:18".into()),
                line: None,
                exact: None
            }
        );
        assert_eq!(
            ps[1],
            SubmittedPlacement {
                thread: "t3".into(),
                status: "anchored".into(),
                range: None,
                line: Some(7),
                exact: None
            }
        );
        assert_eq!(ps[2].status, "orphaned");
        let s = summarize(&threads, &ps);
        assert_eq!((s.total, s.anchored, s.refined, s.orphaned), (4, 3, 0, 1));
        assert_eq!(s.changed, vec![("t4".to_string(), "orphaned".to_string())]);
        assert_eq!(s.line(), "threads: 4 re-anchored — 3 anchored, 0 refined, 1 orphaned (+1 orphaned: t4)");
        assert_eq!(
            Reanchored { total: 8, anchored: 5, orphaned: 3, ..Default::default() }.line(),
            "threads: 8 re-anchored — 5 anchored, 0 refined, 3 orphaned (unchanged)"
        );
        // The wire shape round-trips and is shape-checked.
        let json = serde_json::to_value(&ps).unwrap();
        assert_eq!(parse_placements(&json).unwrap(), ps);
        assert!(parse_placements(&Value::Null).unwrap().is_empty());
        for bad in [
            serde_json::json!({}),
            serde_json::json!([{ "thread": "x", "status": "anchored" }]),
            serde_json::json!([{ "thread": "t1", "status": "moved" }]),
            serde_json::json!([{ "thread": "t1", "status": "anchored", "range": "1-2" }]),
            serde_json::json!([{ "thread": "t1", "status": "anchored" }, { "thread": "t1", "status": "orphaned" }]),
        ] {
            assert!(matches!(parse_placements(&bad), Err(StoreError::InvalidPlacements(_))), "{bad}");
        }
        // Cheap checks on a submission.
        let a = threads[1].1.as_ref().unwrap();
        let ok = SubmittedPlacement {
            thread: "t2".into(),
            status: "anchored".into(),
            range: Some("3:9-3:18".into()),
            line: None,
            exact: None,
        };
        check_submission(&ok, a, v2).unwrap();
        let wrong = SubmittedPlacement { range: Some("3:0-3:4".into()), ..ok.clone() };
        assert!(check_submission(&wrong, a, v2).is_err(), "anchored must sit on the quote");
        let refined = SubmittedPlacement {
            status: "refined".into(),
            range: Some("3:5-3:18".into()),
            exact: Some("one two three".into()),
            ..ok.clone()
        };
        check_submission(&refined, a, v2).unwrap();
        let lying = SubmittedPlacement { exact: Some("something else".into()), ..refined.clone() };
        assert!(check_submission(&lying, a, v2).is_err());
        let off = SubmittedPlacement { range: Some("9:0-9:1".into()), ..ok.clone() };
        assert!(check_submission(&off, a, v2).is_err(), "not on the new text");
        let asset = threads[2].1.as_ref().unwrap();
        assert!(check_submission(&ok, asset, v2).is_err(), "assets take a line, not a range");
        check_submission(
            &SubmittedPlacement {
                thread: "t3".into(),
                status: "anchored".into(),
                range: None,
                line: Some(7),
                exact: None,
            },
            asset,
            v2,
        )
        .unwrap();
    }
}
