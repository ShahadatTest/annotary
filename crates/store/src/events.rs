//! The events log as a *read* surface (ticket 08): the page query behind `GET /api/v1/events`
//! (`wait`, the web live feed), the commit hook the server uses to wake parked long-polls, and
//! the small read that dresses a Comment event up with what it said.
//!
//! Writing is unchanged: every mutation still goes through `Store::tx` + `append_event`
//! (`store.rs`), which is why `seq` is Tenant-wide, strictly monotonic and atomic with the
//! mutation — a cursor (`seq`) is durable across a server restart because it *is* the row id.

use rusqlite::{OptionalExtension, params, types::Value as SqlValue};

use crate::error::Result;
use crate::model::Event;
use crate::store::Store;

/// One page of the log, as `GET /api/v1/events` asks for it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EventQuery {
    /// Exclusive lower bound: `seq > since`.
    pub since: i64,
    /// Inclusive upper bound: `seq <= until` — the head the caller read *before* querying, so the
    /// cursor it hands back never outruns what it scanned.
    pub until: i64,
    /// Only this Document's events.
    pub doc_id: Option<String>,
    /// Only these event types (`version.pushed`, …); `None` = every type.
    pub types: Option<Vec<String>>,
    /// At most this many rows, oldest first (ask for one more than you want to learn whether
    /// the page was cut).
    pub limit: usize,
    /// CMD-23: also this Member's own `setting.changed` / `device.*` events, whatever the
    /// Document and type narrowing (`&me=1` on the live feed).
    pub member_events: Option<String>,
}

/// CMD-23: the event types readable only by the Member in their `payload.member`.
pub fn is_member_event(kind: &str) -> bool {
    kind == "setting.changed" || kind.starts_with("device.")
}

/// One event plus the Vault its Document lives in — `None` for Tenant-level events (members,
/// tokens, vaults) and for a Document that has since been purged. What a rights check needs
/// without another read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EventRow {
    pub event: Event,
    pub vault_id: Option<String>,
}

/// A Comment as it is *now* — what an event about it quotes (`wait`'s excerpt).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentBrief {
    pub body: String,
    pub author: String,
    pub deleted: bool,
}

/// What the server hangs on the store to learn about commits.
pub type CommitHook = Box<dyn Fn() + Send + Sync + 'static>;

impl Store {
    /// The rows of one page: `seq > since AND seq <= until`, filtered by Document / type, oldest
    /// first, at most `limit` (0 = none). Purely a read; rights are the caller's business
    /// (`EventRow.vault_id` is there for that).
    pub fn events_page(&self, q: &EventQuery) -> Result<Vec<EventRow>> {
        if q.limit == 0 || q.until <= q.since {
            return Ok(Vec::new());
        }
        let mut sql = String::from(
            "SELECT e.seq, e.at, e.by, e.type, e.doc_id, e.payload, d.vault_id
             FROM events e LEFT JOIN documents d ON d.id = e.doc_id
             WHERE e.seq > ?1 AND e.seq <= ?2",
        );
        let mut args: Vec<SqlValue> = vec![SqlValue::Integer(q.since), SqlValue::Integer(q.until)];
        let mut narrow: Vec<String> = Vec::new();
        if let Some(doc) = &q.doc_id {
            args.push(SqlValue::Text(doc.clone()));
            narrow.push(format!("e.doc_id = ?{}", args.len()));
        }
        if let Some(types) = &q.types {
            if types.is_empty() && q.member_events.is_none() {
                return Ok(Vec::new());
            }
            let mut marks = Vec::with_capacity(types.len());
            for t in types {
                args.push(SqlValue::Text(t.clone()));
                marks.push(format!("?{}", args.len()));
            }
            narrow.push(if marks.is_empty() { "0".into() } else { format!("e.type IN ({})", marks.join(", ")) });
        }
        let narrowed = if narrow.is_empty() { None } else { Some(narrow.join(" AND ")) };
        match (&q.member_events, narrowed) {
            (Some(member), Some(n)) => {
                args.push(SqlValue::Text(member.clone()));
                sql.push_str(&format!(
                    " AND (({n}) OR (e.doc_id IS NULL AND (e.type = 'setting.changed' OR e.type LIKE 'device.%')
                       AND json_extract(e.payload, '$.member') = ?{}))",
                    args.len()
                ));
            }
            (_, Some(n)) => sql.push_str(&format!(" AND {n}")),
            (_, None) => {}
        }
        args.push(SqlValue::Integer(q.limit as i64));
        sql.push_str(&format!(" ORDER BY e.seq LIMIT ?{}", args.len()));
        let conn = self.conn();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(rusqlite::params_from_iter(args.iter()), |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, Option<String>>(4)?,
                r.get::<_, String>(5)?,
                r.get::<_, Option<String>>(6)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (seq, at, by, kind, doc_id, payload, vault_id) = r?;
            out.push(EventRow {
                event: Event { seq, at, by, kind, doc_id, payload: serde_json::from_str(&payload)? },
                vault_id,
            });
        }
        Ok(out)
    }

    /// The current body/author/tombstone state of one Comment (`num` 0 = the root).
    pub fn comment_brief(&self, doc_id: &str, thread_num: u32, num: u32) -> Result<Option<CommentBrief>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                "SELECT body, author, deleted_at IS NOT NULL FROM comments
                 WHERE doc_id = ?1 AND thread_num = ?2 AND num = ?3",
                params![doc_id, thread_num as i64, num as i64],
                |r| Ok(CommentBrief { body: r.get(0)?, author: r.get(1)?, deleted: r.get(2)? }),
            )
            .optional()?)
    }

    /// Install (or clear) the hook run after every committed transaction — the server wakes its
    /// parked `GET /api/v1/events` long-polls with it. Called outside the store mutex, so it may
    /// not touch the store synchronously in a way that re-enters (it needn't: a wake is a signal,
    /// the waiter re-reads).
    pub fn set_commit_hook(&self, hook: Option<CommitHook>) {
        *self.commit_hook.lock().unwrap_or_else(|e| e.into_inner()) = hook.map(std::sync::Arc::from);
    }

    /// Run the commit hook, if any.
    pub(crate) fn fire_commit_hook(&self) {
        let hook = self.commit_hook.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if let Some(h) = hook {
            h();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PushInput, SetupInput};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn fresh() -> (tempfile::TempDir, Store, String) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan".into(), password: None })
            .unwrap();
        (dir, store, out.owner.id)
    }

    fn push(store: &Store, by: &str, vault: &str, name: &str, text: &str) -> String {
        let (hash, _) = store.put_blob(text.as_bytes()).unwrap();
        store.create_document(by, vault, PushInput::new(name, &hash)).unwrap().id
    }

    // spec: S08-01
    #[test]
    fn page_query_filters_by_doc_and_type_and_reports_the_vault() {
        let (_d, store, owner) = fresh();
        let vault = store.list_vaults().unwrap()[0].id.clone();
        let a = push(&store, &owner, &vault, "a", "# A\n");
        let b = push(&store, &owner, &vault, "b", "# B\n");
        let head = store.last_seq().unwrap();
        let all = store
            .events_page(&EventQuery {
                since: 0,
                until: head,
                doc_id: None,
                types: None,
                limit: 100,
                member_events: None,
            })
            .unwrap();
        assert_eq!(all.len() as i64, head);
        assert!(all.iter().filter(|r| r.event.doc_id.is_none()).all(|r| r.vault_id.is_none()));
        assert!(all.iter().filter(|r| r.event.doc_id.is_some()).all(|r| r.vault_id.as_deref() == Some(&*vault)));
        let only_b = store
            .events_page(&EventQuery {
                since: 0,
                until: head,
                doc_id: Some(b.clone()),
                types: None,
                limit: 100,
                member_events: None,
            })
            .unwrap();
        assert_eq!(
            only_b.iter().map(|r| r.event.kind.as_str()).collect::<Vec<_>>(),
            ["document.created", "version.pushed"]
        );
        let pushes = store
            .events_page(&EventQuery {
                since: 0,
                until: head,
                doc_id: None,
                types: Some(vec!["version.pushed".into()]),
                limit: 100,
                member_events: None,
            })
            .unwrap();
        assert_eq!(pushes.len(), 2);
        assert_eq!(pushes[0].event.doc_id.as_deref(), Some(a.as_str()));
        // Bounds are honoured: `until` caps the page, `limit` cuts it, `since` excludes itself.
        let cut = store
            .events_page(&EventQuery {
                since: 0,
                until: head,
                doc_id: None,
                types: None,
                limit: 3,
                member_events: None,
            })
            .unwrap();
        assert_eq!(cut.iter().map(|r| r.event.seq).collect::<Vec<_>>(), [1, 2, 3]);
        let capped = store
            .events_page(&EventQuery { since: 2, until: 4, doc_id: None, types: None, limit: 100, member_events: None })
            .unwrap();
        assert_eq!(capped.iter().map(|r| r.event.seq).collect::<Vec<_>>(), [3, 4]);
        assert!(
            store.events_page(&EventQuery { since: 5, until: 5, limit: 10, ..Default::default() }).unwrap().is_empty()
        );
        assert!(
            store
                .events_page(&EventQuery {
                    since: 0,
                    until: head,
                    types: Some(vec![]),
                    limit: 10,
                    ..Default::default()
                })
                .unwrap()
                .is_empty()
        );
    }

    // spec: S08-02
    #[test]
    fn commit_hook_fires_once_per_committed_transaction() {
        let (_d, store, owner) = fresh();
        let hits = Arc::new(AtomicUsize::new(0));
        let h = hits.clone();
        store.set_commit_hook(Some(Box::new(move || {
            h.fetch_add(1, Ordering::SeqCst);
        })));
        let vault = store.list_vaults().unwrap()[0].id.clone();
        push(&store, &owner, &vault, "a", "# A\n");
        assert_eq!(hits.load(Ordering::SeqCst), 1, "one transaction, one wake");
        assert!(store.add_member(&owner, "rowan", None).is_err(), "handle taken");
        assert_eq!(hits.load(Ordering::SeqCst), 1, "a rolled-back transaction wakes nobody");
        store.set_commit_hook(None);
        push(&store, &owner, &vault, "b", "# B\n");
        assert_eq!(hits.load(Ordering::SeqCst), 1);
    }
}
