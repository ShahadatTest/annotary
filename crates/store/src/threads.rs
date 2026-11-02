//! Threads and Comments on the [`Store`] (ticket 04): new / reply / resolve / reopen / edit /
//! delete (tombstone), `@handle` mentions resolved to Member ids at post time, the per-Document
//! body cap, and the reads behind `thread list`, `thread show`, `show`, `status`, `mentions` and
//! `list --mentions-me`.
//!
//! Every mutation runs in one `Store::tx`, stamps `{by, at, version}` with the head Version at
//! that moment, bumps the Document's `last_activity_at` and appends its event before commit:
//! `thread.created {thread, subject, version, mentions}`, `thread.replied {thread, comment,
//! version, mentions}`, `thread.resolved|reopened {thread, version}`, `comment.edited|deleted
//! {thread, comment, version}` (mentions are Member ids). Ids are server-allocated per Document:
//! `t7` (`threads.num`), `t7.1` (`comments.num`; the root is `num` 0).
//!
//! Anchors: a span Thread stores a [`annotary_sidecar::Anchor`] (`original` frozen at
//! creation, `current` = original with `status: anchored`, sparse `history` with the one entry);
//! ticket 07 re-anchors `current`/`history` on push (`crate::anchors`) and adds asset Threads
//! (`subject = 'asset'`, anchored by the Asset's content hash from the head manifest, the
//! referencing line display-only). The read-only time-slice ([`Store::threads_at`]) rebuilds
//! every Thread as it stood on Version N: created ≤ N, Replies ≤ N, resolved per the last
//! resolve/reopen in `thread_events` with `version ≤ N`, placement per the sparse history.

use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};

use annotary_sidecar::anchor::{ANCHORED, REFINED};
use annotary_sidecar::range::{Range, extract};
use annotary_sidecar::{Anchor, Placement};

use crate::docs::bump_activity;
use crate::error::{Result, StoreError};
use crate::model::DocumentView;
use crate::store::{Store, append_event};
use crate::time;

/// Maximum total characters (Unicode code points) across all Comment bodies of one Document —
/// the server constant behind story 89.
pub const BODY_CAP: usize = 100_000;

/// What a Thread is about.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThreadSubject {
    Span,
    Document,
    /// Ticket 07.
    Asset,
}

impl ThreadSubject {
    pub fn as_str(self) -> &'static str {
        match self {
            ThreadSubject::Span => "span",
            ThreadSubject::Document => "document",
            ThreadSubject::Asset => "asset",
        }
    }

    pub fn parse(s: &str) -> Option<ThreadSubject> {
        match s {
            "span" => Some(ThreadSubject::Span),
            "document" => Some(ThreadSubject::Document),
            "asset" => Some(ThreadSubject::Asset),
            _ => None,
        }
    }
}

/// `deleted {at, by}` on a tombstoned Comment.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub at: String,
    /// Member id.
    pub by: String,
    pub by_handle: String,
}

/// `resolved {by, at, version}` on a resolved Thread.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    /// Member id.
    pub by: String,
    pub by_handle: String,
    pub at: String,
    pub version: u32,
}

/// One Comment (root or Reply) as read back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentView {
    /// `t7` for the root, `t7.1` for a Reply.
    pub id: String,
    /// 0 for the root, n for `t7.n`.
    pub num: u32,
    /// Member id of the author; `author_handle` is the current handle.
    pub author: String,
    pub author_handle: String,
    /// Markdown, verbatim; empty once tombstoned.
    pub body: String,
    /// Member ids mentioned (`@handle` resolved at post time); cleared on tombstone.
    pub mentions: Vec<String>,
    pub mention_handles: Vec<String>,
    pub at: String,
    /// Head Version when posted.
    pub version: u32,
    pub edited_at: Option<String>,
    pub deleted: Option<Tombstone>,
}

impl CommentView {
    pub fn is_deleted(&self) -> bool {
        self.deleted.is_some()
    }
}

/// A Thread: root Comment + ordered Replies, subject/anchor, resolution.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadView {
    pub doc_id: String,
    pub num: u32,
    /// `t7`.
    pub id: String,
    pub subject: ThreadSubject,
    /// Span (and, from ticket 07, asset) Threads only.
    pub anchor: Option<Anchor>,
    pub root: CommentView,
    /// Chronological.
    pub replies: Vec<CommentView>,
    pub resolved: Option<Resolution>,
    /// The latest of: any Comment posted / edited / deleted, resolve / reopen.
    pub updated_at: String,
}

impl ThreadView {
    pub fn is_open(&self) -> bool {
        self.resolved.is_none()
    }

    /// True when the root and every Reply are tombstoned — hidden from lists and counts.
    pub fn is_fully_deleted(&self) -> bool {
        self.root.is_deleted() && self.replies.iter().all(CommentView::is_deleted)
    }

    /// Member ids mentioned by any live Comment (root first, then Replies), deduplicated.
    pub fn mentioned(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in std::iter::once(&self.root).chain(self.replies.iter()) {
            for m in &c.mentions {
                if !out.contains(m) {
                    out.push(m.clone());
                }
            }
        }
        out
    }

    /// Handles of [`Self::mentioned`], same order.
    pub fn mentioned_handles(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for c in std::iter::once(&self.root).chain(self.replies.iter()) {
            for m in &c.mention_handles {
                if !out.contains(m) {
                    out.push(m.clone());
                }
            }
        }
        out
    }

    pub fn mentions(&self, member_id: &str) -> bool {
        self.root.mentions.iter().any(|m| m == member_id)
            || self.replies.iter().any(|r| r.mentions.iter().any(|m| m == member_id))
    }

    /// The Anchor's placement status (`anchored` for a Document-level Thread — it never moves).
    pub fn placement_status(&self) -> &str {
        self.anchor.as_ref().and_then(|a| a.current.status.as_deref()).unwrap_or(ANCHORED)
    }

    /// Document-order sort key: Document-level first, then span/asset by current position,
    /// orphaned last (by original position); ties by id.
    pub fn document_order_key(&self) -> (u8, u32, u32, u32) {
        match &self.anchor {
            None => (0, 0, 0, self.num),
            Some(a) => {
                let orphaned = a.current.status.as_deref() == Some("orphaned");
                let p = if orphaned { &a.original } else { &a.current };
                let (line, col) = position_of(p);
                (if orphaned { 2 } else { 1 }, line, col, self.num)
            }
        }
    }
}

/// `(line, col)` a placement starts at (`range` for spans, `line` for assets).
fn position_of(p: &Placement) -> (u32, u32) {
    if let Some(r) = p.range.as_deref().and_then(|r| Range::parse(r).ok()) {
        return (r.start_line, r.start_col);
    }
    (p.line.unwrap_or(0), 0)
}

/// Which Threads `threads()` returns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThreadStatusFilter {
    #[default]
    Open,
    Resolved,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum ThreadSort {
    /// Document order (the default).
    #[default]
    Document,
    /// Most recent activity first.
    Recent,
}

/// Filters for `threads()`.
#[derive(Clone, Debug, Default)]
pub struct ThreadFilter {
    pub status: ThreadStatusFilter,
    /// Only Threads a live Comment of which mentions this Member id.
    pub mentions: Option<String>,
    /// `updated_at >= since` (stored timestamp text).
    pub since: Option<String>,
    pub sort: ThreadSort,
    /// Include fully-tombstoned Threads (the Sidecar projection wants everything).
    pub include_hidden: bool,
    // ---- ticket 07 ----
    /// Only Threads whose placement status is one of these (`anchored`, `refined`, `orphaned`;
    /// a Document-level Thread counts as `anchored`). `None` = every status.
    pub anchor_status: Option<Vec<String>>,
    // ---- end ticket 07 ----
}

/// What `new_thread` takes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewThread {
    pub subject: ThreadSubject,
    /// `"L:C-L:C"` for a span Thread.
    pub range: Option<String>,
    pub body: String,
    /// The head Version the poster looked at: refused (`NotHead`) when the head is not that.
    pub expect_version: Option<u32>,
    // ---- ticket 07 ----
    /// The Asset `ref` (as in the head manifest) for an asset Thread.
    pub reference: Option<String>,
    /// The 1-based line of the head Markdown that references the Asset (display-only; the
    /// server handler computes it with the Asset scanner), `None` when not referenced directly.
    pub line: Option<u32>,
    // ---- end ticket 07 ----
}

impl NewThread {
    pub fn span(range: impl Into<String>, body: impl Into<String>) -> NewThread {
        NewThread {
            subject: ThreadSubject::Span,
            range: Some(range.into()),
            body: body.into(),
            expect_version: None,
            reference: None,
            line: None,
        }
    }

    pub fn document(body: impl Into<String>) -> NewThread {
        NewThread {
            subject: ThreadSubject::Document,
            range: None,
            body: body.into(),
            expect_version: None,
            reference: None,
            line: None,
        }
    }

    pub fn asset(reference: impl Into<String>, line: Option<u32>, body: impl Into<String>) -> NewThread {
        NewThread {
            subject: ThreadSubject::Asset,
            range: None,
            body: body.into(),
            expect_version: None,
            reference: Some(reference.into()),
            line,
        }
    }
}

/// One row of `mentions`: an open Thread mentioning a Member, with its Document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MentionHit {
    pub doc: DocumentView,
    pub thread: ThreadView,
}

// ---------------------------------------------------------------- mentions (pure)

/// The `@handle` candidates in a Markdown body, in order of first appearance, deduplicated:
/// kebab-case handles (`[a-z0-9]+(-[a-z0-9]+)*`) preceded by nothing or a non-word character and
/// followed by nothing or a non-word character; never inside fenced code blocks or code spans.
/// Whether a candidate is a Member is decided at post time — unknown handles stay plain text.
pub fn mention_candidates(body: &str) -> Vec<String> {
    let text = strip_fenced_blocks(body);
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '`' {
            // A code span: a run of N backticks closed by the next run of exactly N.
            let mut n = 0;
            while i + n < chars.len() && chars[i + n] == '`' {
                n += 1;
            }
            let mut j = i + n;
            let mut closed = None;
            while j < chars.len() {
                if chars[j] == '`' {
                    let mut m = 0;
                    while j + m < chars.len() && chars[j + m] == '`' {
                        m += 1;
                    }
                    if m == n {
                        closed = Some(j + m);
                        break;
                    }
                    j += m;
                } else {
                    j += 1;
                }
            }
            match closed {
                Some(end) => i = end,
                None => i += n,
            }
            continue;
        }
        if c == '@' {
            let before_ok = i == 0 || !is_word(chars[i - 1]);
            let mut j = i + 1;
            while j < chars.len() && (chars[j].is_ascii_lowercase() || chars[j].is_ascii_digit() || chars[j] == '-') {
                j += 1;
            }
            // Trim to the kebab-case shape: no leading/trailing/double dashes.
            let mut end = j;
            while end > i + 1 && chars[end - 1] == '-' {
                end -= 1;
            }
            let candidate: String = chars[i + 1..end].iter().collect();
            let after_ok = end >= chars.len() || !is_word(chars[end]) || chars[end] == '-';
            if before_ok
                && after_ok
                && !candidate.is_empty()
                && !candidate.starts_with('-')
                && !candidate.contains("--")
                && crate::model::validate_handle(&candidate).is_ok()
                && !out.contains(&candidate)
            {
                out.push(candidate);
            }
            i = end.max(i + 1);
            continue;
        }
        i += 1;
    }
    out
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Drop fenced code blocks (``` or ~~~ fences, up to 3 spaces of indent, closed by a fence of the
/// same character at least as long; an unclosed fence runs to the end).
fn strip_fenced_blocks(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut fence: Option<(char, usize)> = None;
    for line in body.split_inclusive('\n') {
        let bare = line.trim_end_matches(['\n', '\r']);
        let trimmed = bare.trim_start_matches(' ');
        let indent = bare.len() - trimmed.len();
        let run = |ch: char| trimmed.chars().take_while(|c| *c == ch).count();
        match fence {
            Some((ch, n)) => {
                let m = run(ch);
                if indent <= 3 && m >= n && trimmed[m..].trim().is_empty() {
                    fence = None;
                }
            }
            None => {
                let (bt, tl) = (run('`'), run('~'));
                if indent <= 3 && (bt >= 3 || tl >= 3) {
                    fence = Some(if bt >= 3 { ('`', bt) } else { ('~', tl) });
                } else {
                    out.push_str(line);
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------- SQL helpers

fn handles(tx: &rusqlite::Connection) -> Result<HashMap<String, String>> {
    let mut stmt = tx.prepare("SELECT member_id, handle FROM memberships")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = HashMap::new();
    for r in rows {
        let (id, h) = r?;
        out.insert(id, h);
    }
    Ok(out)
}

fn handle_of(map: &HashMap<String, String>, id: &str) -> String {
    map.get(id).cloned().unwrap_or_else(|| id.to_string())
}

/// Resolve `@handle` candidates in `body` to Member ids (unknown handles are dropped).
fn resolve_mentions(tx: &rusqlite::Connection, body: &str) -> Result<Vec<String>> {
    let cands = mention_candidates(body);
    let mut out = Vec::new();
    for h in cands {
        if let Some(id) = tx
            .query_row("SELECT member_id FROM memberships WHERE handle = ?1", [&h], |r| r.get::<_, String>(0))
            .optional()?
        {
            out.push(id);
        }
    }
    Ok(out)
}

fn doc_head(tx: &Transaction, doc_id: &str) -> Result<(u32, String)> {
    let deleted: Option<Option<String>> =
        tx.query_row("SELECT deleted_at FROM documents WHERE id = ?1", [doc_id], |r| r.get(0)).optional()?;
    match deleted {
        None => return Err(StoreError::DocumentNotFound(doc_id.to_string())),
        Some(Some(_)) => return Err(StoreError::DocumentDeleted(doc_id.to_string())),
        Some(None) => {}
    }
    Ok(tx.query_row(
        "SELECT number, md_hash FROM versions WHERE doc_id = ?1 ORDER BY number DESC LIMIT 1",
        [doc_id],
        |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)),
    )?)
}

fn body_total(tx: &rusqlite::Connection, doc_id: &str) -> Result<usize> {
    Ok(tx.query_row("SELECT COALESCE(SUM(LENGTH(body)), 0) FROM comments WHERE doc_id = ?1", [doc_id], |r| {
        r.get::<_, i64>(0)
    })? as usize)
}

/// Refuse a body that would push the Document past [`BODY_CAP`] (`replacing` = the characters
/// of the body being edited away, if any).
fn check_cap(tx: &rusqlite::Connection, doc_id: &str, adding: usize, replacing: usize) -> Result<()> {
    let current = body_total(tx, doc_id)?;
    let after = current.saturating_sub(replacing) + adding;
    if after > BODY_CAP {
        return Err(StoreError::BodyCap { cap: BODY_CAP, current, adding });
    }
    Ok(())
}

fn validate_body(body: &str) -> Result<()> {
    if body.trim().is_empty() {
        return Err(StoreError::Invalid { what: "Comment body", reason: "must not be empty".into() });
    }
    Ok(())
}

/// `t7` → 7.
pub fn parse_thread_id(id: &str) -> Option<u32> {
    let n = id.strip_prefix('t')?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) || n.starts_with('0') {
        return None;
    }
    n.parse().ok()
}

/// `t7` → (7, 0); `t7.3` → (7, 3).
pub fn parse_comment_id(id: &str) -> Option<(u32, u32)> {
    match id.split_once('.') {
        None => Some((parse_thread_id(id)?, 0)),
        Some((t, n)) => {
            let t = parse_thread_id(t)?;
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) || n.starts_with('0') {
                return None;
            }
            Some((t, n.parse().ok()?))
        }
    }
}

pub fn thread_id(num: u32) -> String {
    format!("t{num}")
}

pub fn comment_id(thread_num: u32, num: u32) -> String {
    if num == 0 { thread_id(thread_num) } else { format!("t{thread_num}.{num}") }
}

fn thread_exists(tx: &rusqlite::Connection, doc_id: &str, num: u32) -> Result<bool> {
    Ok(tx
        .query_row("SELECT 1 FROM threads WHERE doc_id = ?1 AND num = ?2", params![doc_id, num as i64], |_| Ok(()))
        .optional()?
        .is_some())
}

fn resolved_of(tx: &rusqlite::Connection, doc_id: &str, num: u32) -> Result<Option<(String, String, u32)>> {
    Ok(tx
        .query_row(
            "SELECT resolved_by, resolved_at, resolved_version FROM threads WHERE doc_id = ?1 AND num = ?2",
            params![doc_id, num as i64],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, Option<String>>(1)?, r.get::<_, Option<i64>>(2)?)),
        )
        .optional()?
        .and_then(|(by, at, v)| Some((by?, at?, v? as u32))))
}

/// Append a resolve/reopen row + mirror it onto the Thread row + the Tenant event.
fn set_resolution(
    tx: &Transaction,
    by: &str,
    doc_id: &str,
    num: u32,
    resolve: bool,
    now: &str,
    head: u32,
) -> Result<()> {
    tx.execute(
        "INSERT INTO thread_events(doc_id, thread_num, kind, by, at, version) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![doc_id, num as i64, if resolve { "resolve" } else { "reopen" }, by, now, head as i64],
    )?;
    if resolve {
        tx.execute(
            "UPDATE threads SET resolved_by = ?1, resolved_at = ?2, resolved_version = ?3 WHERE doc_id = ?4 AND num = ?5",
            params![by, now, head as i64, doc_id, num as i64],
        )?;
    } else {
        tx.execute(
            "UPDATE threads SET resolved_by = NULL, resolved_at = NULL, resolved_version = NULL WHERE doc_id = ?1 AND num = ?2",
            params![doc_id, num as i64],
        )?;
    }
    append_event(
        tx,
        Some(by),
        if resolve { "thread.resolved" } else { "thread.reopened" },
        Some(doc_id),
        serde_json::json!({ "thread": thread_id(num), "version": head }),
    )?;
    Ok(())
}

// ---------------------------------------------------------------- reads (shared)

const COMMENT_SELECT: &str = "SELECT thread_num, num, author, body, mentions, created_at, created_version, edited_at,
        deleted_at, deleted_by FROM comments WHERE doc_id = ?1";

fn row_to_comment(r: &rusqlite::Row<'_>, hs: &HashMap<String, String>) -> rusqlite::Result<(u32, CommentView)> {
    let thread_num = r.get::<_, i64>(0)? as u32;
    let num = r.get::<_, i64>(1)? as u32;
    let author: String = r.get(2)?;
    let mentions: String = r.get(4)?;
    let mentions: Vec<String> = serde_json::from_str(&mentions).unwrap_or_default();
    let deleted_at: Option<String> = r.get(8)?;
    let deleted_by: Option<String> = r.get(9)?;
    Ok((
        thread_num,
        CommentView {
            id: comment_id(thread_num, num),
            num,
            author_handle: handle_of(hs, &author),
            author,
            body: r.get(3)?,
            mention_handles: mentions.iter().map(|m| handle_of(hs, m)).collect(),
            mentions,
            at: r.get(5)?,
            version: r.get::<_, i64>(6)? as u32,
            edited_at: r.get(7)?,
            deleted: match (deleted_at, deleted_by) {
                (Some(at), Some(by)) => Some(Tombstone { at, by_handle: handle_of(hs, &by), by }),
                _ => None,
            },
        },
    ))
}

/// Every Thread of a Document (unfiltered, by `num`), with Replies chronological.
fn load_threads(conn: &rusqlite::Connection, doc_id: &str, only: Option<u32>) -> Result<Vec<ThreadView>> {
    let hs = handles(conn)?;
    let mut comments: BTreeMap<u32, Vec<CommentView>> = BTreeMap::new();
    {
        let mut stmt = conn.prepare(&format!("{COMMENT_SELECT} ORDER BY thread_num, num"))?;
        for row in stmt.query_map([doc_id], |r| row_to_comment(r, &hs))? {
            let (t, c) = row?;
            comments.entry(t).or_default().push(c);
        }
    }
    let mut last_events: HashMap<u32, String> = HashMap::new();
    {
        let mut stmt =
            conn.prepare("SELECT thread_num, MAX(at) FROM thread_events WHERE doc_id = ?1 GROUP BY thread_num")?;
        for row in stmt.query_map([doc_id], |r| Ok((r.get::<_, i64>(0)? as u32, r.get::<_, String>(1)?)))? {
            let (t, at) = row?;
            last_events.insert(t, at);
        }
    }
    let mut sql = String::from(
        "SELECT num, subject, anchor, created_by, created_at, created_version, resolved_by, resolved_at, resolved_version
         FROM threads WHERE doc_id = ?1",
    );
    if only.is_some() {
        sql.push_str(" AND num = ?2");
    }
    sql.push_str(" ORDER BY num");
    let mut stmt = conn.prepare(&sql)?;
    let mut out = Vec::new();
    /// `(num, subject, anchor JSON, resolved (by, at, version))`.
    type ThreadRow = (u32, String, Option<String>, Option<(String, String, u32)>);
    let map_row = |r: &rusqlite::Row<'_>| -> rusqlite::Result<ThreadRow> {
        let num = r.get::<_, i64>(0)? as u32;
        let subject: String = r.get(1)?;
        let anchor: Option<String> = r.get(2)?;
        let rb: Option<String> = r.get(6)?;
        let ra: Option<String> = r.get(7)?;
        let rv: Option<i64> = r.get(8)?;
        let resolved = match (rb, ra, rv) {
            (Some(b), Some(a), Some(v)) => Some((b, a, v as u32)),
            _ => None,
        };
        Ok((num, subject, anchor, resolved))
    };
    let rows: Vec<_> = match only {
        Some(n) => stmt.query_map(params![doc_id, n as i64], map_row)?.collect::<rusqlite::Result<_>>()?,
        None => stmt.query_map([doc_id], map_row)?.collect::<rusqlite::Result<_>>()?,
    };
    for (num, subject, anchor, resolved) in rows {
        let mut cs = comments.remove(&num).unwrap_or_default();
        if cs.is_empty() || cs[0].num != 0 {
            continue; // never happens: a Thread always has its root
        }
        let root = cs.remove(0);
        let mut updated_at = root.at.clone();
        for c in std::iter::once(&root).chain(cs.iter()) {
            for t in [Some(&c.at), c.edited_at.as_ref(), c.deleted.as_ref().map(|d| &d.at)].into_iter().flatten() {
                if *t > updated_at {
                    updated_at = t.clone();
                }
            }
        }
        if let Some(t) = last_events.get(&num)
            && *t > updated_at
        {
            updated_at = t.clone();
        }
        out.push(ThreadView {
            doc_id: doc_id.to_string(),
            num,
            id: thread_id(num),
            subject: ThreadSubject::parse(&subject).unwrap_or(ThreadSubject::Document),
            anchor: anchor.and_then(|a| serde_json::from_str(&a).ok()),
            root,
            replies: cs,
            resolved: resolved.map(|(by, at, version)| Resolution { by_handle: handle_of(&hs, &by), by, at, version }),
            updated_at,
        });
    }
    Ok(out)
}

fn apply_filter(mut all: Vec<ThreadView>, filter: &ThreadFilter) -> Vec<ThreadView> {
    all.retain(|t| {
        (filter.include_hidden || !t.is_fully_deleted())
            && match filter.status {
                ThreadStatusFilter::Open => t.is_open(),
                ThreadStatusFilter::Resolved => !t.is_open(),
                ThreadStatusFilter::All => true,
            }
            && filter.mentions.as_deref().is_none_or(|m| t.mentions(m))
            && filter.since.as_deref().is_none_or(|s| t.updated_at.as_str() >= s)
            && filter.anchor_status.as_ref().is_none_or(|ss| ss.iter().any(|s| s == t.placement_status()))
    });
    match filter.sort {
        ThreadSort::Document => all.sort_by_key(ThreadView::document_order_key),
        ThreadSort::Recent => all.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(b.num.cmp(&a.num))),
    }
    all
}

// ---------------------------------------------------------------- Store API

impl Store {
    /// Start a Thread on the head Version (`comment` right at the API). Span Threads need a
    /// `range` on head — the exact quote is read from the head Markdown and frozen as
    /// `anchor.original`; empty spans are refused. `expect_version` ≠ head ⇒ `NotHead`.
    pub fn new_thread(&self, by: &str, doc_id: &str, input: NewThread) -> Result<ThreadView> {
        validate_body(&input.body)?;
        let range = match input.subject {
            ThreadSubject::Span => Some(
                Range::parse(input.range.as_deref().unwrap_or(""))
                    .map_err(|e| StoreError::InvalidRange(e.to_string()))?,
            ),
            ThreadSubject::Document | ThreadSubject::Asset => None,
        };
        let reference = match input.subject {
            ThreadSubject::Asset => Some(
                input
                    .reference
                    .as_deref()
                    .map(str::trim)
                    .filter(|r| !r.is_empty())
                    .ok_or_else(|| StoreError::Invalid { what: "asset Thread", reason: "needs an Asset ref".into() })?
                    .to_string(),
            ),
            _ => None,
        };
        let num = self.tx(|tx| {
            let (head, md_hash) = doc_head(tx, doc_id)?;
            if let Some(v) = input.expect_version
                && v != head
            {
                return Err(StoreError::NotHead { expected: v, head });
            }
            let anchor = match (range, &reference) {
                (Some(r), _) => {
                    let md = self.blobs.read(&md_hash)?.ok_or_else(|| StoreError::BlobMissing(md_hash.clone()))?;
                    let text = String::from_utf8_lossy(&md);
                    let exact = extract(&text, &r).map_err(|e| StoreError::RangeNotOnHead {
                        range: r.to_string(),
                        head,
                        reason: e.to_string(),
                    })?;
                    if exact.trim().is_empty() {
                        return Err(StoreError::RangeNotOnHead {
                            range: r.to_string(),
                            head,
                            reason: "the span is empty (zero-width or whitespace only)".into(),
                        });
                    }
                    Some(Anchor::new_span(head, &r.to_string(), &exact))
                }
                // ---- ticket 07: an asset Thread anchors by the Asset's content hash on head.
                (None, Some(reference)) => {
                    let manifest: String = tx.query_row(
                        "SELECT manifest FROM versions WHERE doc_id = ?1 AND number = ?2",
                        params![doc_id, head as i64],
                        |r| r.get(0),
                    )?;
                    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap_or_default();
                    let entries = manifest.as_array().cloned().unwrap_or_default();
                    let entry =
                        entries.iter().find(|e| e.get("ref").and_then(|v| v.as_str()) == Some(reference.as_str()));
                    let Some(entry) = entry else {
                        return Err(StoreError::AssetNotOnHead {
                            reference: reference.clone(),
                            head,
                            available: entries
                                .iter()
                                .filter_map(|e| e.get("ref").and_then(|v| v.as_str()).map(str::to_string))
                                .collect(),
                        });
                    };
                    let Some(hash) = entry.get("hash").and_then(|v| v.as_str()) else {
                        return Err(StoreError::AssetMissing { reference: reference.clone(), head });
                    };
                    Some(Anchor::new_asset(head, hash, reference, input.line))
                }
                // ---- end ticket 07
                (None, None) => None,
            };
            check_cap(tx, doc_id, input.body.chars().count(), 0)?;
            let mentions = resolve_mentions(tx, &input.body)?;
            let num =
                tx.query_row("SELECT COALESCE(MAX(num), 0) + 1 FROM threads WHERE doc_id = ?1", [doc_id], |r| {
                    r.get::<_, i64>(0)
                })? as u32;
            let now = time::now();
            tx.execute(
                "INSERT INTO threads(doc_id, num, subject, anchor, created_by, created_at, created_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    doc_id,
                    num as i64,
                    input.subject.as_str(),
                    anchor.as_ref().map(serde_json::to_string).transpose()?,
                    by,
                    now,
                    head as i64
                ],
            )?;
            tx.execute(
                "INSERT INTO comments(doc_id, thread_num, num, author, body, mentions, created_at, created_version)
                 VALUES (?1, ?2, 0, ?3, ?4, ?5, ?6, ?7)",
                params![doc_id, num as i64, by, input.body, serde_json::to_string(&mentions)?, now, head as i64],
            )?;
            bump_activity(tx, doc_id, &now)?;
            append_event(
                tx,
                Some(by),
                "thread.created",
                Some(doc_id),
                serde_json::json!({
                    "thread": thread_id(num), "subject": input.subject.as_str(), "version": head, "mentions": mentions,
                    "range": anchor.as_ref().and_then(|a| a.original.range.clone()),
                    "ref": anchor.as_ref().and_then(|a| a.original.reference.clone()),
                }),
            )?;
            Ok(num)
        })?;
        Ok(self.thread(doc_id, num)?.expect("just created"))
    }

    /// Reply to a Thread (`comment` right); `resolve` also resolves it in the same transaction.
    /// Returns the Thread and the new Reply.
    pub fn reply(
        &self,
        by: &str,
        doc_id: &str,
        thread_num: u32,
        body: &str,
        resolve: bool,
    ) -> Result<(ThreadView, CommentView)> {
        validate_body(body)?;
        let num = self.tx(|tx| {
            let (head, _) = doc_head(tx, doc_id)?;
            if !thread_exists(tx, doc_id, thread_num)? {
                return Err(StoreError::ThreadNotFound { doc: doc_id.into(), thread: thread_id(thread_num) });
            }
            check_cap(tx, doc_id, body.chars().count(), 0)?;
            let mentions = resolve_mentions(tx, body)?;
            let num = tx.query_row(
                "SELECT COALESCE(MAX(num), 0) + 1 FROM comments WHERE doc_id = ?1 AND thread_num = ?2",
                params![doc_id, thread_num as i64],
                |r| r.get::<_, i64>(0),
            )? as u32;
            let now = time::now();
            tx.execute(
                "INSERT INTO comments(doc_id, thread_num, num, author, body, mentions, created_at, created_version)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![doc_id, thread_num as i64, num as i64, by, body, serde_json::to_string(&mentions)?, now, head as i64],
            )?;
            bump_activity(tx, doc_id, &now)?;
            append_event(
                tx,
                Some(by),
                "thread.replied",
                Some(doc_id),
                serde_json::json!({
                    "thread": thread_id(thread_num), "comment": comment_id(thread_num, num), "version": head, "mentions": mentions,
                }),
            )?;
            if resolve && resolved_of(tx, doc_id, thread_num)?.is_none() {
                set_resolution(tx, by, doc_id, thread_num, true, &now, head)?;
            }
            Ok(num)
        })?;
        let t = self.thread(doc_id, thread_num)?.expect("exists");
        let c = t.replies.iter().find(|r| r.num == num).cloned().expect("just written");
        Ok((t, c))
    }

    /// Resolve (`resolve` = true) or reopen a Thread — a bare event by anyone with `comment`,
    /// stamped `{by, at, version}`. Idempotent: an already-resolved (or already-open) Thread
    /// gets no new event; returns `(thread, changed)`.
    pub fn set_thread_resolved(
        &self,
        by: &str,
        doc_id: &str,
        thread_num: u32,
        resolve: bool,
    ) -> Result<(ThreadView, bool)> {
        let changed = self.tx(|tx| {
            let (head, _) = doc_head(tx, doc_id)?;
            if !thread_exists(tx, doc_id, thread_num)? {
                return Err(StoreError::ThreadNotFound { doc: doc_id.into(), thread: thread_id(thread_num) });
            }
            let is_resolved = resolved_of(tx, doc_id, thread_num)?.is_some();
            if is_resolved == resolve {
                return Ok(false);
            }
            let now = time::now();
            set_resolution(tx, by, doc_id, thread_num, resolve, &now, head)?;
            bump_activity(tx, doc_id, &now)?;
            Ok(true)
        })?;
        Ok((self.thread(doc_id, thread_num)?.expect("exists"), changed))
    }

    /// Edit a Comment's body (author, or `is_owner`): sets `edited_at`, re-resolves mentions,
    /// no history. Tombstoned Comments cannot be edited.
    pub fn edit_comment(
        &self,
        by: &str,
        is_owner: bool,
        doc_id: &str,
        comment: &str,
        body: &str,
    ) -> Result<CommentView> {
        validate_body(body)?;
        let Some((tn, cn)) = parse_comment_id(comment) else {
            return Err(StoreError::CommentNotFound { doc: doc_id.into(), comment: comment.into() });
        };
        self.tx(|tx| {
            let (head, _) = doc_head(tx, doc_id)?;
            let row: Option<(String, String, Option<String>)> = tx
                .query_row(
                    "SELECT author, body, deleted_at FROM comments WHERE doc_id = ?1 AND thread_num = ?2 AND num = ?3",
                    params![doc_id, tn as i64, cn as i64],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((author, old, deleted_at)) = row else {
                return Err(if thread_exists(tx, doc_id, tn)? {
                    StoreError::CommentNotFound { doc: doc_id.into(), comment: comment.into() }
                } else {
                    StoreError::ThreadNotFound { doc: doc_id.into(), thread: thread_id(tn) }
                });
            };
            if deleted_at.is_some() {
                return Err(StoreError::CommentDeleted(comment_id(tn, cn)));
            }
            if author != by && !is_owner {
                let hs = handles(tx)?;
                return Err(StoreError::NotAuthor { comment: comment_id(tn, cn), author: handle_of(&hs, &author), what: "edit" });
            }
            check_cap(tx, doc_id, body.chars().count(), old.chars().count())?;
            let mentions = resolve_mentions(tx, body)?;
            let now = time::now();
            tx.execute(
                "UPDATE comments SET body = ?1, mentions = ?2, edited_at = ?3 WHERE doc_id = ?4 AND thread_num = ?5 AND num = ?6",
                params![body, serde_json::to_string(&mentions)?, now, doc_id, tn as i64, cn as i64],
            )?;
            bump_activity(tx, doc_id, &now)?;
            append_event(
                tx,
                Some(by),
                "comment.edited",
                Some(doc_id),
                serde_json::json!({ "thread": thread_id(tn), "comment": comment_id(tn, cn), "version": head, "mentions": mentions }),
            )?;
            Ok(())
        })?;
        let t = self.thread(doc_id, tn)?.expect("exists");
        Ok(if cn == 0 { t.root } else { t.replies.into_iter().find(|r| r.num == cn).expect("exists") })
    }

    /// Tombstone a Comment (author, or `is_owner`): body and mentions cleared, `deleted {at, by}`
    /// kept, id kept. A tombstoned root keeps the Thread and its Replies. Idempotent — returns
    /// `(comment, changed)`.
    pub fn delete_comment(&self, by: &str, is_owner: bool, doc_id: &str, comment: &str) -> Result<(CommentView, bool)> {
        let Some((tn, cn)) = parse_comment_id(comment) else {
            return Err(StoreError::CommentNotFound { doc: doc_id.into(), comment: comment.into() });
        };
        let changed = self.tx(|tx| {
            let (head, _) = doc_head(tx, doc_id)?;
            let row: Option<(String, Option<String>)> = tx
                .query_row(
                    "SELECT author, deleted_at FROM comments WHERE doc_id = ?1 AND thread_num = ?2 AND num = ?3",
                    params![doc_id, tn as i64, cn as i64],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((author, deleted_at)) = row else {
                return Err(if thread_exists(tx, doc_id, tn)? {
                    StoreError::CommentNotFound { doc: doc_id.into(), comment: comment.into() }
                } else {
                    StoreError::ThreadNotFound { doc: doc_id.into(), thread: thread_id(tn) }
                });
            };
            if author != by && !is_owner {
                let hs = handles(tx)?;
                return Err(StoreError::NotAuthor {
                    comment: comment_id(tn, cn),
                    author: handle_of(&hs, &author),
                    what: "delete",
                });
            }
            if deleted_at.is_some() {
                return Ok(false);
            }
            let now = time::now();
            tx.execute(
                "UPDATE comments SET body = '', mentions = '[]', deleted_at = ?1, deleted_by = ?2
                 WHERE doc_id = ?3 AND thread_num = ?4 AND num = ?5",
                params![now, by, doc_id, tn as i64, cn as i64],
            )?;
            bump_activity(tx, doc_id, &now)?;
            append_event(
                tx,
                Some(by),
                "comment.deleted",
                Some(doc_id),
                serde_json::json!({ "thread": thread_id(tn), "comment": comment_id(tn, cn), "version": head }),
            )?;
            Ok(true)
        })?;
        let t = self.thread(doc_id, tn)?.expect("exists");
        let c = if cn == 0 { t.root } else { t.replies.into_iter().find(|r| r.num == cn).expect("exists") };
        Ok((c, changed))
    }

    // ---------------------------------------------------------------- reads

    /// One Thread (hidden or not), with its Replies.
    pub fn thread(&self, doc_id: &str, num: u32) -> Result<Option<ThreadView>> {
        Ok(load_threads(&self.conn(), doc_id, Some(num))?.into_iter().next())
    }

    /// The Threads of a Document per `filter` (default: open, not fully tombstoned, document
    /// order, Replies chronological).
    pub fn threads(&self, doc_id: &str, filter: &ThreadFilter) -> Result<Vec<ThreadView>> {
        Ok(apply_filter(load_threads(&self.conn(), doc_id, None)?, filter))
    }

    // ---- ticket 07 ----
    /// The Threads of a Document **as they stood on Version `n`** (read-only time-slice): Threads
    /// created ≤ n, Replies ≤ n, resolved-state per the last resolve/reopen stamped ≤ n
    /// (`thread_events`), placement per the sparse history (`Anchor::at`; a refined span's
    /// current text is extracted from Version n). Edits and tombstones are not versioned and show
    /// as they are now. `filter` applies to the reconstructed Threads. `VersionNotFound` when the
    /// Document has no v`n`.
    pub fn threads_at(&self, doc_id: &str, n: u32, filter: &ThreadFilter) -> Result<Vec<ThreadView>> {
        let conn = self.conn();
        let md_hash: Option<String> = conn
            .query_row(
                "SELECT md_hash FROM versions WHERE doc_id = ?1 AND number = ?2",
                params![doc_id, n as i64],
                |r| r.get(0),
            )
            .optional()?;
        let Some(md_hash) = md_hash else {
            return Err(StoreError::VersionNotFound { doc: doc_id.to_string(), number: n });
        };
        let hs = handles(&conn)?;
        // The last resolve/reopen per Thread with version ≤ n.
        let mut resolution: HashMap<u32, Option<Resolution>> = HashMap::new();
        {
            let mut stmt = conn.prepare(
                "SELECT thread_num, kind, by, at, version FROM thread_events
                 WHERE doc_id = ?1 AND version <= ?2 ORDER BY id",
            )?;
            for row in stmt.query_map(params![doc_id, n as i64], |r| {
                Ok((
                    r.get::<_, i64>(0)? as u32,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)? as u32,
                ))
            })? {
                let (num, kind, by, at, version) = row?;
                let res = (kind == "resolve").then(|| Resolution { by_handle: handle_of(&hs, &by), by, at, version });
                resolution.insert(num, res);
            }
        }
        let mut text: Option<String> = None;
        let mut out = Vec::new();
        for mut t in load_threads(&conn, doc_id, None)? {
            if t.root.version > n {
                continue;
            }
            t.replies.retain(|r| r.version <= n);
            t.resolved = resolution.get(&t.num).cloned().flatten();
            if let Some(a) = &t.anchor {
                let Some(mut at) = a.at(n) else { continue };
                if at.current.status.as_deref() == Some(REFINED)
                    && let Some(r) = at.current.range.as_deref().and_then(|r| Range::parse(r).ok())
                {
                    if text.is_none() {
                        let md = self.blobs.read(&md_hash)?.ok_or_else(|| StoreError::BlobMissing(md_hash.clone()))?;
                        text = Some(String::from_utf8_lossy(&md).into_owned());
                    }
                    at.current.exact = extract(text.as_deref().unwrap_or(""), &r).ok();
                }
                t.anchor = Some(at);
            }
            // Activity as of n: the latest Comment / resolve / reopen still included.
            let mut updated = t.root.at.clone();
            for c in std::iter::once(&t.root).chain(t.replies.iter()) {
                if c.at > updated {
                    updated = c.at.clone();
                }
            }
            if let Some(r) = &t.resolved
                && r.at > updated
            {
                updated = r.at.clone();
            }
            t.updated_at = updated;
            out.push(t);
        }
        Ok(apply_filter(out, filter))
    }
    // ---- end ticket 07 ----

    /// `(open_threads, mentions_me)` for one Document — open, not fully tombstoned; the second
    /// counts those a live Comment of which mentions `member_id`.
    pub fn thread_counts(&self, doc_id: &str, member_id: &str) -> Result<(u32, u32)> {
        let conn = self.conn();
        let open = apply_filter(load_threads(&conn, doc_id, None)?, &ThreadFilter::default());
        let mentions = open.iter().filter(|t| t.mentions(member_id)).count() as u32;
        Ok((open.len() as u32, mentions))
    }

    /// `(open_threads, mentions_me)` for every Document that has an open Thread, keyed by
    /// Document id (Documents without one are simply absent). One pass for `list`.
    pub fn thread_counts_all(&self, member_id: &str) -> Result<HashMap<String, (u32, u32)>> {
        let conn = self.conn();
        let mut docs: HashSet<String> = HashSet::new();
        {
            let mut stmt = conn.prepare("SELECT DISTINCT doc_id FROM threads WHERE resolved_at IS NULL")?;
            for r in stmt.query_map([], |r| r.get::<_, String>(0))? {
                docs.insert(r?);
            }
        }
        let mut out = HashMap::new();
        for d in docs {
            let open = apply_filter(load_threads(&conn, &d, None)?, &ThreadFilter::default());
            if !open.is_empty() {
                let mentions = open.iter().filter(|t| t.mentions(member_id)).count() as u32;
                out.insert(d, (open.len() as u32, mentions));
            }
        }
        Ok(out)
    }

    /// Open Threads mentioning `member_id` across live Documents (in `vault_ids` when given),
    /// most recent activity first; `since` filters on the Thread's `updated_at`.
    pub fn mentions_for(
        &self,
        member_id: &str,
        vault_ids: Option<&[String]>,
        since: Option<&str>,
    ) -> Result<Vec<MentionHit>> {
        let conn = self.conn();
        let mut pairs: Vec<(String, u32)> = Vec::new();
        {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT c.doc_id, c.thread_num FROM comments c
                 JOIN threads t ON t.doc_id = c.doc_id AND t.num = c.thread_num
                 JOIN documents d ON d.id = c.doc_id
                 WHERE d.deleted_at IS NULL AND t.resolved_at IS NULL AND c.deleted_at IS NULL
                   AND c.mentions LIKE ?1
                 ORDER BY c.doc_id, c.thread_num",
            )?;
            let like = format!("%\"{member_id}\"%");
            for r in stmt.query_map([like], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)? as u32)))? {
                pairs.push(r?);
            }
        }
        let mut docs: HashMap<String, DocumentView> = HashMap::new();
        let mut hits = Vec::new();
        for (doc_id, num) in pairs {
            if !docs.contains_key(&doc_id) {
                let Some(d) = crate::docs::load_document(&conn, &doc_id)? else { continue };
                docs.insert(doc_id.clone(), d);
            }
            let d = &docs[&doc_id];
            if let Some(vs) = vault_ids
                && !vs.contains(&d.vault_id)
            {
                continue;
            }
            let Some(t) = load_threads(&conn, &doc_id, Some(num))?.into_iter().next() else { continue };
            if !t.mentions(member_id) || t.is_fully_deleted() || !t.is_open() {
                continue;
            }
            if since.is_some_and(|s| t.updated_at.as_str() < s) {
                continue;
            }
            hits.push(MentionHit { doc: d.clone(), thread: t });
        }
        hits.sort_by(|a, b| b.thread.updated_at.cmp(&a.thread.updated_at).then(a.doc.id.cmp(&b.doc.id)));
        Ok(hits)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::PushInput;
    use crate::store::{SetupInput, SetupOutcome};

    fn fresh() -> (tempfile::TempDir, Store, SetupOutcome) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan Vale".into(), password: None })
            .unwrap();
        (dir, store, out)
    }

    fn doc(store: &Store, out: &SetupOutcome, md: &str) -> DocumentView {
        let (h, _) = store.put_blob(md.as_bytes()).unwrap();
        store.create_document(&out.owner.id, &out.vault.id, PushInput::new("plan", h)).unwrap()
    }

    fn span(range: &str, body: &str) -> NewThread {
        NewThread::span(range, body)
    }

    fn document(body: &str) -> NewThread {
        NewThread::document(body)
    }

    // spec: S04-06
    #[test]
    fn mention_candidates_skip_code_and_keep_order() {
        assert_eq!(mention_candidates("hi @rowan and @scribe, @rowan again"), vec!["rowan", "scribe"]);
        assert_eq!(mention_candidates("mail me@example.com or @a1-b2."), vec!["a1-b2"]);
        assert_eq!(mention_candidates("`@rowan` and ``code @linter`` but @real"), vec!["real"]);
        assert_eq!(mention_candidates("```\n@rowan\n```\n@linter\n~~~md\n@nope\n~~~"), vec!["linter"]);
        assert_eq!(mention_candidates("an unclosed ` backtick then @rowan"), vec!["rowan"]);
        assert_eq!(mention_candidates("@Rowan @rowan_x @-x @a--b @"), Vec::<String>::new(), "not kebab-case handles");
        assert_eq!(mention_candidates("(@rowan) [@linter](x) @scribe's"), vec!["rowan", "linter", "scribe"]);
        assert_eq!(mention_candidates("@rowan-"), vec!["rowan"], "a trailing dash is punctuation");
        assert_eq!(mention_candidates("no mentions here"), Vec::<String>::new());
        assert_eq!(parse_comment_id("t7"), Some((7, 0)));
        assert_eq!(parse_comment_id("t7.3"), Some((7, 3)));
        for bad in ["t", "7", "t07", "t7.", "t7.0", "x7", "plan/t7", "t7.1.1"] {
            assert_eq!(parse_comment_id(bad), None, "{bad}");
        }
        assert_eq!(comment_id(7, 0), "t7");
        assert_eq!(comment_id(7, 2), "t7.2");
    }

    // spec: S04-01, S04-02, S04-03, S04-04
    #[test]
    fn threads_are_stamped_with_head_and_ids_are_sequential() {
        let (_d, store, out) = fresh();
        let (agent, _) = store.add_member(&out.owner.id, "scribe", None).unwrap();
        let d = doc(&store, &out, "# Plan\n\nWe ship the CLI before the web UI so agents can start.\n");
        let seq0 = store.last_seq().unwrap();
        let t1 = store.new_thread(&out.owner.id, &d.id, document("Needs a Risks section. @scribe")).unwrap();
        assert_eq!(t1.id, "t1");
        assert_eq!(t1.subject, ThreadSubject::Document);
        assert!(t1.anchor.is_none());
        assert_eq!(t1.root.version, 1);
        assert_eq!(t1.root.author_handle, "rowan");
        assert_eq!(t1.root.mentions, vec![agent.id.clone()]);
        assert_eq!(t1.root.mention_handles, vec!["scribe"]);
        let t2 =
            store.new_thread(&out.owner.id, &d.id, span("3:0-3:33", "Agreed — @scribe check the order list")).unwrap();
        assert_eq!(t2.id, "t2");
        let a = t2.anchor.as_ref().unwrap();
        assert_eq!(a.original.exact.as_deref(), Some("We ship the CLI before the web UI"));
        assert_eq!(a.original.range.as_deref(), Some("3:0-3:33"));
        assert_eq!(a.original.version, 1);
        assert_eq!(a.current.status.as_deref(), Some("anchored"));
        assert_eq!(a.history.len(), 1);
        // A push moves the head; later events are stamped with the new head.
        let (h2, _) = store.put_blob(b"# Plan v2\n").unwrap();
        store.push_version(&out.owner.id, &d.id, PushInput::new("plan", h2)).unwrap();
        let (t2, r) = store.reply(&agent.id, &d.id, 2, "Item 3 says so. @rowan", false).unwrap();
        assert_eq!(r.id, "t2.1");
        assert_eq!(r.version, 2, "reply stamped with the head at that moment");
        assert_eq!(r.mentions, vec![out.owner.id.clone()]);
        assert_eq!(t2.replies.len(), 1);
        assert_eq!(t2.updated_at, r.at);
        let (t2, changed) = store.set_thread_resolved(&agent.id, &d.id, 2, true).unwrap();
        assert!(changed);
        let res = t2.resolved.as_ref().unwrap();
        assert_eq!((res.by_handle.as_str(), res.version), ("scribe", 2));
        assert!(!store.set_thread_resolved(&agent.id, &d.id, 2, true).unwrap().1, "resolve is idempotent");
        let (t2, changed) = store.set_thread_resolved(&out.owner.id, &d.id, 2, false).unwrap();
        assert!(changed && t2.is_open());
        // Every mutation appended an event with the Document id and bumped activity.
        let events = store.events_since(seq0).unwrap();
        let kinds: Vec<&str> = events.iter().map(|e| e.kind.as_str()).collect();
        assert_eq!(
            kinds,
            vec![
                "thread.created",
                "thread.created",
                "version.pushed",
                "thread.replied",
                "thread.resolved",
                "thread.reopened"
            ]
        );
        assert!(events.iter().all(|e| e.doc_id.as_deref() == Some(d.id.as_str())));
        assert_eq!(events[0].payload["thread"], "t1");
        assert_eq!(events[0].payload["mentions"], serde_json::json!([agent.id]));
        assert_eq!(events[3].payload["comment"], "t2.1");
        assert_eq!(events[3].payload["version"], 2);
        assert_eq!(events[4].payload["version"], 2);
        assert!(store.document(&d.id).unwrap().unwrap().last_activity_at >= t2.updated_at);
        // New Threads only on head: `expect_version` must be the head.
        let mut stale = span("1:0-1:6", "x");
        stale.expect_version = Some(1);
        assert!(matches!(
            store.new_thread(&out.owner.id, &d.id, stale),
            Err(StoreError::NotHead { expected: 1, head: 2 })
        ));
        // Sequential ids continue after the last; replies count per Thread.
        let t3 = store.new_thread(&out.owner.id, &d.id, document("third")).unwrap();
        assert_eq!(t3.id, "t3");
        assert_eq!(store.reply(&out.owner.id, &d.id, 2, "again", false).unwrap().1.id, "t2.2");
        assert!(matches!(store.reply(&out.owner.id, &d.id, 99, "x", false), Err(StoreError::ThreadNotFound { .. })));
    }

    // spec: S04-02, S04-05
    #[test]
    fn span_ranges_are_verified_on_head() {
        let (_d, store, out) = fresh();
        let d = doc(&store, &out, "line one\nline two\n");
        assert!(matches!(store.new_thread(&out.owner.id, &d.id, span("nope", "x")), Err(StoreError::InvalidRange(_))));
        assert!(matches!(
            store.new_thread(&out.owner.id, &d.id, span("3:0-3:2", "x")),
            Err(StoreError::RangeNotOnHead { head: 1, .. })
        ));
        assert!(matches!(
            store.new_thread(&out.owner.id, &d.id, span("2:0-2:99", "x")),
            Err(StoreError::RangeNotOnHead { .. })
        ));
        assert!(
            matches!(
                store.new_thread(&out.owner.id, &d.id, span("2:1-2:1", "x")),
                Err(StoreError::RangeNotOnHead { .. })
            ),
            "zero-width spans are refused"
        );
        assert!(
            matches!(
                store.new_thread(&out.owner.id, &d.id, span("1:4-1:5", "x")),
                Err(StoreError::RangeNotOnHead { .. })
            ),
            "whitespace-only spans are refused"
        );
        assert!(matches!(
            store.new_thread(&out.owner.id, &d.id, span("1:0-1:4", "  ")),
            Err(StoreError::Invalid { .. })
        ));
        let t = store.new_thread(&out.owner.id, &d.id, span("1:5-2:4", "two lines")).unwrap();
        assert_eq!(t.anchor.unwrap().original.exact.as_deref(), Some("one\nline"));
        // Deleted Documents take no Threads.
        store.delete_document(&out.owner.id, &d.id).unwrap();
        assert!(matches!(store.new_thread(&out.owner.id, &d.id, document("x")), Err(StoreError::DocumentDeleted(_))));
        assert!(matches!(store.reply(&out.owner.id, &d.id, 1, "x", false), Err(StoreError::DocumentDeleted(_))));
    }

    // spec: S04-07, S04-08
    #[test]
    fn edit_and_tombstone_invariants() {
        let (_d, store, out) = fresh();
        let (agent, _) = store.add_member(&out.owner.id, "scribe", None).unwrap();
        let (linter, _) = store.add_member(&out.owner.id, "linter", None).unwrap();
        let d = doc(&store, &out, "text\n");
        let t1 = store.new_thread(&agent.id, &d.id, document("root by agent @linter")).unwrap();
        store.reply(&linter.id, &d.id, 1, "reply by linter", false).unwrap();
        // Edit: author or owner; sets edited_at; mentions re-resolved; no history.
        assert!(matches!(
            store.edit_comment(&linter.id, false, &d.id, "t1", "hijack"),
            Err(StoreError::NotAuthor { what: "edit", .. })
        ));
        let e = store.edit_comment(&agent.id, false, &d.id, "t1", "root edited @rowan").unwrap();
        assert!(e.edited_at.is_some());
        assert_eq!(e.body, "root edited @rowan");
        assert_eq!(e.mention_handles, vec!["rowan"]);
        let e = store.edit_comment(&out.owner.id, true, &d.id, "t1.1", "owner edits anyone's").unwrap();
        assert_eq!(e.body, "owner edits anyone's");
        assert!(matches!(
            store.edit_comment(&out.owner.id, true, &d.id, "t1.9", "x"),
            Err(StoreError::CommentNotFound { .. })
        ));
        assert!(matches!(
            store.edit_comment(&out.owner.id, true, &d.id, "t9", "x"),
            Err(StoreError::ThreadNotFound { .. })
        ));
        assert!(matches!(
            store.edit_comment(&out.owner.id, true, &d.id, "bogus", "x"),
            Err(StoreError::CommentNotFound { .. })
        ));
        // Delete = tombstone: body + mentions cleared, `deleted {at, by}` kept, id kept.
        assert!(matches!(
            store.delete_comment(&linter.id, false, &d.id, "t1"),
            Err(StoreError::NotAuthor { what: "delete", .. })
        ));
        let (c, changed) = store.delete_comment(&agent.id, false, &d.id, "t1").unwrap();
        assert!(changed && c.is_deleted());
        assert_eq!(c.body, "");
        assert!(c.mentions.is_empty());
        assert_eq!(c.deleted.as_ref().unwrap().by_handle, "scribe");
        assert_eq!(c.id, "t1");
        assert!(!store.delete_comment(&agent.id, false, &d.id, "t1").unwrap().1, "delete is idempotent");
        assert!(matches!(store.edit_comment(&agent.id, false, &d.id, "t1", "x"), Err(StoreError::CommentDeleted(_))));
        // A tombstoned root keeps the Thread and its Replies alive and listed.
        let t = store.thread(&d.id, 1).unwrap().unwrap();
        assert!(t.root.is_deleted() && t.replies.len() == 1 && !t.is_fully_deleted());
        assert_eq!(store.threads(&d.id, &ThreadFilter::default()).unwrap().len(), 1);
        assert_eq!(store.thread_counts(&d.id, &out.owner.id).unwrap(), (1, 0));
        // Fully tombstoned ⇒ hidden from lists and counts, still readable by id and with
        // `include_hidden`; a new Reply brings it back.
        store.delete_comment(&out.owner.id, true, &d.id, "t1.1").unwrap();
        assert!(store.thread(&d.id, 1).unwrap().unwrap().is_fully_deleted());
        assert!(store.threads(&d.id, &ThreadFilter::default()).unwrap().is_empty());
        assert_eq!(
            store.threads(&d.id, &ThreadFilter { include_hidden: true, ..Default::default() }).unwrap().len(),
            1
        );
        assert_eq!(store.thread_counts(&d.id, &out.owner.id).unwrap(), (0, 0));
        store.reply(&out.owner.id, &d.id, 1, "revived", false).unwrap();
        assert_eq!(store.threads(&d.id, &ThreadFilter::default()).unwrap().len(), 1);
        let _ = t1;
    }

    // spec: S04-09
    #[test]
    fn body_cap_is_enforced_with_the_count() {
        let (_d, store, out) = fresh();
        let d = doc(&store, &out, "text\n");
        let big = "ü".repeat(60_000); // 60k characters, 120k bytes: the cap counts characters
        store.new_thread(&out.owner.id, &d.id, document(&big)).unwrap();
        match store.new_thread(&out.owner.id, &d.id, document(&"x".repeat(40_001))) {
            Err(StoreError::BodyCap { cap, current, adding }) => {
                assert_eq!((cap, current, adding), (BODY_CAP, 60_000, 40_001));
            }
            other => panic!("{other:?}"),
        }
        store.new_thread(&out.owner.id, &d.id, document(&"x".repeat(40_000))).unwrap();
        assert!(matches!(
            store.reply(&out.owner.id, &d.id, 1, "1", false),
            Err(StoreError::BodyCap { current: 100_000, .. })
        ));
        // Editing counts the replaced body out; tombstoning frees its characters.
        store.edit_comment(&out.owner.id, true, &d.id, "t2", &"y".repeat(40_000)).unwrap();
        assert!(matches!(
            store.edit_comment(&out.owner.id, true, &d.id, "t2", &"y".repeat(40_001)),
            Err(StoreError::BodyCap { .. })
        ));
        store.delete_comment(&out.owner.id, true, &d.id, "t2").unwrap();
        store.reply(&out.owner.id, &d.id, 1, &"z".repeat(40_000), false).unwrap();
        let (_, agent) = store.add_member(&out.owner.id, "agent", None).unwrap();
        let _ = agent;
    }

    // spec: S04-06, S04-10
    #[test]
    fn mentions_resolve_at_post_time_and_survive_display_name_changes() {
        let (_d, store, out) = fresh();
        let (agent, _) = store.add_member(&out.owner.id, "scribe", Some("Scribe")).unwrap();
        let d = doc(&store, &out, "text\n");
        let t = store.new_thread(&out.owner.id, &d.id, document("@scribe @nobody `@rowan`")).unwrap();
        assert_eq!(t.root.mentions, vec![agent.id.clone()], "unknown handles stay plain text; code spans excluded");
        assert_eq!(t.root.body, "@scribe @nobody `@rowan`", "bodies are stored verbatim");
        // A display-name change does not touch the stored Member id or the handle.
        assert_eq!(store.thread(&d.id, 1).unwrap().unwrap().root.mention_handles, vec!["scribe"]);
        assert_eq!(store.thread_counts(&d.id, &agent.id).unwrap(), (1, 1));
        assert_eq!(store.thread_counts(&d.id, &out.owner.id).unwrap(), (1, 0));
        // `mentions` across Documents: open, live Comments only, most recent first.
        let d2 = doc(&store, &out, "other\n");
        std::thread::sleep(std::time::Duration::from_millis(3));
        store.new_thread(&out.owner.id, &d2.id, document("also @scribe")).unwrap();
        let hits = store.mentions_for(&agent.id, None, None).unwrap();
        assert_eq!(hits.iter().map(|h| h.doc.id.as_str()).collect::<Vec<_>>(), vec![d2.id.as_str(), d.id.as_str()]);
        assert!(store.mentions_for(&out.owner.id, None, None).unwrap().is_empty());
        let since = hits[0].thread.updated_at.clone();
        assert_eq!(store.mentions_for(&agent.id, None, Some(&since)).unwrap().len(), 1);
        assert!(store.mentions_for(&agent.id, Some(&[]), None).unwrap().is_empty());
        // Resolving, tombstoning the mentioning Comment, or deleting the Document drops the hit.
        store.set_thread_resolved(&out.owner.id, &d2.id, 1, true).unwrap();
        assert_eq!(store.mentions_for(&agent.id, None, None).unwrap().len(), 1);
        store.delete_comment(&out.owner.id, true, &d.id, "t1").unwrap();
        assert!(store.mentions_for(&agent.id, None, None).unwrap().is_empty());
        store.set_thread_resolved(&out.owner.id, &d2.id, 1, false).unwrap();
        assert_eq!(store.thread_counts_all(&agent.id).unwrap().get(&d2.id), Some(&(1, 1)));
        store.delete_document(&out.owner.id, &d2.id).unwrap();
        assert!(store.mentions_for(&agent.id, None, None).unwrap().is_empty());
    }

    // spec: S04-11
    #[test]
    fn ordering_and_filters() {
        let (_d, store, out) = fresh();
        let (agent, _) = store.add_member(&out.owner.id, "scribe", None).unwrap();
        let d = doc(&store, &out, "alpha beta\ngamma delta\nepsilon\n");
        store.new_thread(&out.owner.id, &d.id, span("2:6-2:11", "on delta")).unwrap(); // t1
        store.new_thread(&out.owner.id, &d.id, document("whole doc")).unwrap(); // t2
        store.new_thread(&out.owner.id, &d.id, span("1:0-1:5", "on alpha @scribe")).unwrap(); // t3
        store.new_thread(&out.owner.id, &d.id, span("2:0-2:5", "on gamma")).unwrap(); // t4
        std::thread::sleep(std::time::Duration::from_millis(3));
        store.reply(&agent.id, &d.id, 1, "reply", false).unwrap();
        let ids = |ts: &[ThreadView]| ts.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        // Document-level first, then span by current position.
        assert_eq!(ids(&store.threads(&d.id, &ThreadFilter::default()).unwrap()), vec!["t2", "t3", "t4", "t1"]);
        // Most recent activity first.
        let recent = store.threads(&d.id, &ThreadFilter { sort: ThreadSort::Recent, ..Default::default() }).unwrap();
        assert_eq!(recent[0].id, "t1");
        // Status filters.
        store.set_thread_resolved(&out.owner.id, &d.id, 4, true).unwrap();
        assert_eq!(ids(&store.threads(&d.id, &ThreadFilter::default()).unwrap()), vec!["t2", "t3", "t1"]);
        let resolved =
            store.threads(&d.id, &ThreadFilter { status: ThreadStatusFilter::Resolved, ..Default::default() }).unwrap();
        assert_eq!(ids(&resolved), vec!["t4"]);
        let all =
            store.threads(&d.id, &ThreadFilter { status: ThreadStatusFilter::All, ..Default::default() }).unwrap();
        assert_eq!(ids(&all), vec!["t2", "t3", "t4", "t1"]);
        // Mentions + since.
        let me =
            store.threads(&d.id, &ThreadFilter { mentions: Some(agent.id.clone()), ..Default::default() }).unwrap();
        assert_eq!(ids(&me), vec!["t3"]);
        let since = recent[0].updated_at.clone();
        let s = store.threads(&d.id, &ThreadFilter { since: Some(since), ..Default::default() }).unwrap();
        assert_eq!(ids(&s), vec!["t1"]);
        assert_eq!(store.thread_counts(&d.id, &agent.id).unwrap(), (3, 1));
        // Purge removes every row.
        store.delete_document(&out.owner.id, &d.id).unwrap();
        store.purge_document(&out.owner.id, &d.id).unwrap();
        assert!(store.thread(&d.id, 1).unwrap().is_none());
        assert!(store.threads(&d.id, &ThreadFilter { include_hidden: true, ..Default::default() }).unwrap().is_empty());
    }
}
