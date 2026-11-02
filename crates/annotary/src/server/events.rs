//! `GET /api/v1/events` — the events log behind `wait` and the web UI's live feed / History
//! (ticket 08).
//!
//! | route | right | what |
//! |---|---|---|
//! | `GET /api/v1/events?since=SEQ&wait=SECS&doc=ID&kinds=a,b&action=ID&limit=N` | `read` on the Document's Vault (Tenant-level events: owner only) | every event with `seq > since` the caller may read, oldest first, narrowed by `doc` / `kinds` / `action`; parks up to `wait` s (≤ 30) until one matches; `{events[], seq}` — `seq` is the cursor to continue from (everything ≤ it has been considered, even when `events` is empty) |
//! | the same with `Accept: text/event-stream` | same | SSE: a `ready` message (`{seq}`), then one message per event (`id: seq`, `event: TYPE`, `data: {…}`), keep-alive comments while idle; `Last-Event-ID` resumes a reconnect; `wait` is ignored |
//!
//! `since` absent ⇒ from the head (only what happens from now on). `kinds` are `wait`'s words —
//! `version` (`version.pushed`), `action` (`action.taken`, `action=ID` narrows it), `mention`
//! (`thread.created` / `thread.replied` whose `mentions` name the caller), `thread`, `reply`,
//! `resolve`, `reopen` — or a raw event type (`version.rendered`); absent ⇒ every readable event
//! (what a page's live feed wants). Each event carries the raw `type` and `payload`, the
//! classification `kind` (`mention` when that is what the caller asked for), `mention`, `by` as
//! a handle, the Document's id / current `name` / `vault`, `mentions` as handles and, for Comment
//! events, an `excerpt` of the body (centred on the caller's `@handle` when it mentions them).
//!
//! The long-poll parks on the store's commit hook (`AppState.events`, a `watch` channel bumped
//! after every committed transaction) and re-reads — so it can never miss an event that landed
//! between its read and its sleep, and a cursor is exactly a `seq`. Both forms also watch
//! `AppState.shutdown`: when the server is asked to stop, a parked long-poll answers at once
//! (empty, with its cursor) and an SSE stream ends, so shutdown never waits on them.
//!
//! Both forms re-check access as they go (CMD-8): whenever the commit counter has moved since
//! the last check — a Grant change, a revoked Token, an ended session or a disabled Member is
//! itself a commit — they re-resolve the Credential ([`Store::reauthenticate`], read-only) and
//! recompute the readable Vaults before they send anything more. A dead Credential ends an SSE
//! stream and answers a parked long-poll with 401; a revoked Grant stops that Vault's events
//! while the others keep flowing. One check per wake, never per event.
//!
//! [`Store::reauthenticate`]: annotary_store::Store::reauthenticate

use std::collections::{HashMap, HashSet, VecDeque};
use std::convert::Infallible;
use std::time::{Duration, Instant};

use axum::Json;
use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use annotary_store::{AuthVia, DocumentView, Event, EventQuery, EventRow, Right, parse_comment_id, parse_thread_id};

use super::app::AppState;
use super::auth::Auth;
use super::docs::load_doc;
use super::error::ApiError;
use super::vaults::readable_vaults;

/// Longest a long-poll parks (seconds); larger `wait=` values are clamped.
pub const MAX_WAIT_SECS: u64 = 30;
/// Raw rows scanned per page when `limit` is absent.
pub const DEFAULT_LIMIT: usize = 500;
/// Largest `limit=` accepted.
pub const MAX_LIMIT: usize = 1000;
/// Longest `excerpt` (characters, before the `…` marks).
pub const EXCERPT_CHARS: usize = 72;

/// The kinds `wait --on` knows, each the name of one or two raw event types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Version,
    Action,
    Mention,
    Thread,
    Reply,
    Resolve,
    Reopen,
}

impl Kind {
    pub const ALL: [Kind; 7] =
        [Kind::Version, Kind::Action, Kind::Mention, Kind::Thread, Kind::Reply, Kind::Resolve, Kind::Reopen];

    pub fn parse(word: &str) -> Option<Kind> {
        Some(match word {
            "version" => Kind::Version,
            "action" => Kind::Action,
            "mention" => Kind::Mention,
            "thread" => Kind::Thread,
            "reply" => Kind::Reply,
            "resolve" => Kind::Resolve,
            "reopen" => Kind::Reopen,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Version => "version",
            Kind::Action => "action",
            Kind::Mention => "mention",
            Kind::Thread => "thread",
            Kind::Reply => "reply",
            Kind::Resolve => "resolve",
            Kind::Reopen => "reopen",
        }
    }

    /// The raw event types this kind is read from.
    pub fn types(self) -> &'static [&'static str] {
        match self {
            Kind::Version => &["version.pushed"],
            Kind::Action => &["action.taken"],
            Kind::Mention => &["thread.created", "thread.replied"],
            Kind::Thread => &["thread.created"],
            Kind::Reply => &["thread.replied"],
            Kind::Resolve => &["thread.resolved"],
            Kind::Reopen => &["thread.reopened"],
        }
    }

    /// The words, comma-joined — for error messages and help.
    pub fn words() -> String {
        Kind::ALL.iter().map(|k| k.as_str()).collect::<Vec<_>>().join(",")
    }
}

/// An event's own classification: the kind it primarily is (None for events `wait` has no word
/// for) and whether it mentions `member_id`.
pub fn classify(e: &Event, member_id: &str) -> (Option<Kind>, bool) {
    let mentions_me =
        || e.payload["mentions"].as_array().is_some_and(|m| m.iter().any(|x| x.as_str() == Some(member_id)));
    match e.kind.as_str() {
        "version.pushed" => (Some(Kind::Version), false),
        "action.taken" => (Some(Kind::Action), false),
        "thread.created" => (Some(Kind::Thread), mentions_me()),
        "thread.replied" => (Some(Kind::Reply), mentions_me()),
        "thread.resolved" => (Some(Kind::Resolve), false),
        "thread.reopened" => (Some(Kind::Reopen), false),
        _ => (None, false),
    }
}

/// What one request asked for, resolved against the caller's rights.
#[derive(Clone, Debug)]
pub struct Filter {
    pub member_id: String,
    pub handle: String,
    pub is_owner: bool,
    /// Vault ids the caller may read.
    pub readable: HashSet<String>,
    pub doc_id: Option<String>,
    /// `kinds=` words; empty together with `types` = everything readable.
    pub kinds: HashSet<Kind>,
    /// Raw event types asked for by name.
    pub types: HashSet<String>,
    pub action: Option<String>,
    /// CMD-23 (`&me=1`): also the caller's own `setting.changed` / `device.*` events.
    pub me: bool,
}

impl Filter {
    pub fn build(st: &AppState, auth: &Auth, q: &EventsQuery) -> Result<Filter, ApiError> {
        let doc_id = match q.doc.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
            Some(id) => {
                let doc = load_doc(st, id)?;
                auth.require_right(&doc.vault_name, Right::Read)?;
                Some(doc.id)
            }
            None => None,
        };
        let mut kinds = HashSet::new();
        let mut types = HashSet::new();
        if let Some(raw) = q.kinds.as_deref() {
            for w in raw.split(',').map(str::trim).filter(|w| !w.is_empty()) {
                match Kind::parse(w) {
                    Some(k) => {
                        kinds.insert(k);
                    }
                    None if w.contains('.') => {
                        types.insert(w.to_string());
                    }
                    None => {
                        return Err(ApiError::invalid(format!(
                            "kinds={w:?}: use {} (or an event type such as version.pushed)",
                            Kind::words()
                        )));
                    }
                }
            }
        }
        let readable = readable_vaults(st, auth)?.into_iter().map(|v| v.id).collect();
        Ok(Filter {
            member_id: auth.0.member_id.clone(),
            handle: auth.0.handle.clone(),
            is_owner: auth.0.is_owner(),
            readable,
            doc_id,
            kinds,
            types,
            action: q.action.as_deref().map(str::trim).filter(|a| !a.is_empty()).map(str::to_string),
            me: q.me.as_deref().is_some_and(|v| v == "1" || v == "true"),
        })
    }

    /// CMD-8: re-resolve the stream's Credential and refresh what it may read (the readable
    /// Vaults, owner-ness for Tenant-level events, the handle for excerpts). `Ok(false)` = the
    /// Credential no longer authenticates (Token revoked or expired, session ended, Member
    /// disabled): the stream must end.
    fn refresh(&mut self, st: &AppState, via: &AuthVia) -> Result<bool, ApiError> {
        let Some(m) = st.store.reauthenticate(via)? else { return Ok(false) };
        let auth = Auth(m);
        self.readable = readable_vaults(st, &auth)?.into_iter().map(|v| v.id).collect();
        self.is_owner = auth.0.is_owner();
        self.handle = auth.0.handle;
        Ok(true)
    }

    /// Everything readable, no kind narrowing.
    fn any(&self) -> bool {
        self.kinds.is_empty() && self.types.is_empty()
    }

    /// The SQL `type IN (…)` list (None = no type filter).
    fn sql_types(&self) -> Option<Vec<String>> {
        if self.any() {
            return None;
        }
        let mut out: Vec<String> = self.types.iter().cloned().collect();
        for k in &self.kinds {
            for t in k.types() {
                if !out.iter().any(|x| x == t) {
                    out.push((*t).to_string());
                }
            }
        }
        out.sort();
        Some(out)
    }

    /// May the caller read this row? Document events need `read` on the Vault; events without
    /// a Document (members, tokens, vaults — or a purged Document) are the owner's.
    fn readable(&self, row: &EventRow) -> bool {
        // CMD-23: a Member's settings and Devices are theirs alone, owners included, and only
        // on a feed that asked for them (`&me=1`).
        if annotary_store::is_member_event(&row.event.kind) {
            return self.me && row.event.payload["member"].as_str() == Some(self.member_id.as_str());
        }
        match &row.vault_id {
            Some(v) => self.readable.contains(v),
            None => self.is_owner,
        }
    }

    /// Does the row pass the kind / action narrowing? `Some(kind)` = yes, with the kind to report
    /// (`mention` when the caller asked for mentions and it is one); `None` = skip.
    fn admit(&self, e: &Event) -> Option<(Option<Kind>, bool)> {
        if annotary_store::is_member_event(&e.kind) {
            return Some((None, false));
        }
        let (primary, mention) = classify(e, &self.member_id);
        let action_ok = primary != Some(Kind::Action)
            || self.action.as_deref().is_none_or(|a| e.payload["action"].as_str() == Some(a));
        if self.any() {
            return action_ok.then_some((primary, mention));
        }
        if mention && self.kinds.contains(&Kind::Mention) {
            return Some((Some(Kind::Mention), true));
        }
        if primary.is_some_and(|p| self.kinds.contains(&p)) && action_ok {
            return Some((primary, mention));
        }
        if self.types.contains(&e.kind) && action_ok {
            return Some((primary, mention));
        }
        None
    }
}

/// Per-response caches for the lookups that dress an event up (handles, Documents).
struct Enricher {
    st: AppState,
    handles: HashMap<String, Option<String>>,
    docs: HashMap<String, Option<DocumentView>>,
}

impl Enricher {
    fn new(st: AppState) -> Enricher {
        Enricher { st, handles: HashMap::new(), docs: HashMap::new() }
    }

    fn handle(&mut self, member_id: &str) -> Result<Option<String>, ApiError> {
        if let Some(h) = self.handles.get(member_id) {
            return Ok(h.clone());
        }
        let h = self.st.store.member_by_id(member_id)?.map(|m| m.handle);
        self.handles.insert(member_id.to_string(), h.clone());
        Ok(h)
    }

    fn doc(&mut self, id: &str) -> Result<Option<DocumentView>, ApiError> {
        if let Some(d) = self.docs.get(id) {
            return Ok(d.clone());
        }
        let d = self.st.store.document(id)?;
        self.docs.insert(id.to_string(), d.clone());
        Ok(d)
    }

    /// The wire shape of one event.
    fn enrich(&mut self, f: &Filter, e: &Event, kind: Option<Kind>, mention: bool) -> Result<Value, ApiError> {
        let mut m = Map::new();
        m.insert("seq".into(), json!(e.seq));
        m.insert("at".into(), json!(e.at));
        let by = match &e.by {
            Some(id) => self.handle(id)?,
            None => None,
        };
        m.insert("by".into(), json!(by));
        m.insert("type".into(), json!(e.kind));
        m.insert("kind".into(), json!(kind.map(Kind::as_str)));
        m.insert("mention".into(), json!(mention));
        let doc = match &e.doc_id {
            Some(id) => self.doc(id)?,
            None => None,
        };
        m.insert("doc".into(), json!(e.doc_id));
        m.insert("name".into(), json!(doc.as_ref().map(|d| d.head.name.clone())));
        m.insert("vault".into(), json!(doc.as_ref().map(|d| d.vault_name.clone())));
        m.insert("payload".into(), e.payload.clone());
        if let Some(ids) = e.payload["mentions"].as_array() {
            let mut handles = Vec::new();
            for id in ids.iter().filter_map(Value::as_str) {
                if let Some(h) = self.handle(id)? {
                    handles.push(h);
                }
            }
            m.insert("mentions".into(), json!(handles));
        }
        if let Some(excerpt) = self.excerpt(f, e, mention)? {
            m.insert("excerpt".into(), json!(excerpt));
        }
        Ok(Value::Object(m))
    }

    /// The quoted body for Comment events (`thread.created` = the root, `thread.replied` /
    /// `comment.edited` = the Comment named), as it is now.
    fn excerpt(&mut self, f: &Filter, e: &Event, mention: bool) -> Result<Option<String>, ApiError> {
        let Some(doc_id) = e.doc_id.as_deref() else { return Ok(None) };
        let (tn, cn) = match e.kind.as_str() {
            "thread.created" => match e.payload["thread"].as_str().and_then(parse_thread_id) {
                Some(t) => (t, 0),
                None => return Ok(None),
            },
            "thread.replied" | "comment.edited" => match e.payload["comment"].as_str().and_then(parse_comment_id) {
                Some(c) => c,
                None => return Ok(None),
            },
            _ => return Ok(None),
        };
        let Some(c) = self.st.store.comment_brief(doc_id, tn, cn)? else { return Ok(None) };
        if c.deleted {
            return Ok(Some("(deleted)".to_string()));
        }
        let focus = if mention { Some(format!("@{}", f.handle)) } else { None };
        Ok(Some(excerpt(&c.body, focus.as_deref())))
    }
}

/// One line of at most [`EXCERPT_CHARS`] characters out of a Comment body: whitespace collapsed;
/// when `focus` (an `@handle`) occurs and the body is longer, the window is moved to include it;
/// `…` marks a cut at either end.
pub fn excerpt(body: &str, focus: Option<&str>) -> String {
    let collapsed: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = collapsed.chars().collect();
    if chars.len() <= EXCERPT_CHARS {
        return collapsed;
    }
    let mut start = 0usize;
    if let Some(f) = focus
        && let Some(byte_idx) = collapsed.find(f)
    {
        let idx = collapsed[..byte_idx].chars().count();
        start = idx.saturating_sub(EXCERPT_CHARS / 3);
        // Never leave a dangling tail shorter than the window when the head could hold it.
        start = start.min(chars.len() - EXCERPT_CHARS);
    }
    let end = (start + EXCERPT_CHARS).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

#[derive(Debug, Deserialize, Default)]
pub struct EventsQuery {
    pub since: Option<i64>,
    pub wait: Option<u64>,
    pub doc: Option<String>,
    pub kinds: Option<String>,
    pub action: Option<String>,
    pub limit: Option<usize>,
    /// CMD-23: `me=1` adds the caller's own settings / Devices events.
    pub me: Option<String>,
}

/// One scan from `since` up to `head`: the admitted, enriched events; the cursor the scan reached
/// (the last raw row when the page was cut, else `head`); whether it was cut.
fn scan(
    st: &AppState,
    f: &Filter,
    en: &mut Enricher,
    since: i64,
    head: i64,
    limit: usize,
) -> Result<(Vec<Value>, i64, bool), ApiError> {
    let rows = st.store.events_page(&EventQuery {
        since,
        until: head,
        doc_id: f.doc_id.clone(),
        types: f.sql_types(),
        limit: limit + 1,
        member_events: f.me.then(|| f.member_id.clone()),
    })?;
    let cut = rows.len() > limit;
    let rows = &rows[..rows.len().min(limit)];
    let reached = if cut { rows.last().map(|r| r.event.seq).unwrap_or(head) } else { head };
    let mut out = Vec::new();
    for r in rows {
        if !f.readable(r) {
            continue;
        }
        let Some((kind, mention)) = f.admit(&r.event) else { continue };
        out.push(en.enrich(f, &r.event, kind, mention)?);
    }
    Ok((out, reached, cut))
}

fn wants_sse(headers: &HeaderMap) -> bool {
    headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|a| a.split(',').any(|p| p.trim().starts_with("text/event-stream")))
}

/// `GET /api/v1/events` — JSON long-poll, or SSE when the client accepts `text/event-stream`.
pub async fn list(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Query(q): Query<EventsQuery>,
) -> Result<Response, ApiError> {
    // CMD-8: the commit generation access was checked at (the request's own authentication).
    let mut checked = *st.events.borrow();
    let mut filter = Filter::build(&st, &auth, &q)?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    if wants_sse(&headers) {
        // A reconnecting `EventSource` says where it was.
        let last_id = headers.get("last-event-id").and_then(|v| v.to_str().ok()).and_then(|v| v.trim().parse().ok());
        return Ok(stream(st, filter, auth.0.via, q.since.or(last_id), limit));
    }
    let wait = q.wait.unwrap_or(0).min(MAX_WAIT_SECS);
    let deadline = Instant::now() + Duration::from_secs(wait);
    let mut rx = st.events.clone();
    let mut shutdown = st.shutdown.clone();
    let mut en = Enricher::new(st.clone());
    let mut cursor: Option<i64> = q.since;
    loop {
        // Mark the current generation seen *before* reading, so a commit that lands between the
        // read and the sleep wakes us.
        let generation = *rx.borrow_and_update();
        // CMD-8: a commit since the last check may have changed access — re-check before the
        // rescan (so never on a first pass nothing has committed under).
        if generation != checked {
            checked = generation;
            if !filter.refresh(&st, &auth.0.via)? {
                return Err(ApiError::unauthorized());
            }
        }
        let head = st.store.last_seq()?;
        let since = cursor.unwrap_or(head).min(head);
        let (events, reached, cut) = scan(&st, &filter, &mut en, since, head, limit)?;
        cursor = Some(reached);
        if !events.is_empty() {
            return Ok(Json(json!({ "events": events, "seq": reached })).into_response());
        }
        if cut {
            continue;
        }
        let now = Instant::now();
        // The server is stopping: answer now with the cursor (the client resumes from it).
        if now >= deadline || *shutdown.borrow() {
            break;
        }
        tokio::select! {
            r = tokio::time::timeout(deadline - now, rx.changed()) => match r {
                Ok(Ok(())) => continue,
                // Elapsed, or the store (and its hook) is gone: answer with what we know.
                _ => break,
            },
            _ = shutdown.changed() => break,
        }
    }
    Ok(Json(json!({ "events": [], "seq": cursor.unwrap_or(0) })).into_response())
}

/// The SSE form: the same scan, pushed as messages for as long as the client listens.
fn stream(st: AppState, filter: Filter, via: AuthVia, since: Option<i64>, limit: usize) -> Response {
    struct S {
        st: AppState,
        filter: Filter,
        // ---- CMD-8: re-check access per wake ----
        /// The Credential the stream opened with, re-resolved whenever the commit counter moves.
        via: AuthVia,
        /// The commit generation access was last checked at (None = not yet: check first).
        checked: Option<u64>,
        /// `seq` of the last event sent (or the `ready` cursor): where a rescan restarts when
        /// a re-check drops events queued under the old access.
        delivered: i64,
        // ---- end CMD-8 ----
        en: Enricher,
        rx: tokio::sync::watch::Receiver<u64>,
        shutdown: tokio::sync::watch::Receiver<bool>,
        cursor: Option<i64>,
        pending: VecDeque<Value>,
        ready_sent: bool,
    }
    let state = S {
        rx: st.events.clone(),
        shutdown: st.shutdown.clone(),
        en: Enricher::new(st.clone()),
        st,
        filter,
        via,
        checked: None,
        delivered: 0,
        cursor: since,
        pending: VecDeque::new(),
        ready_sent: false,
    };
    let events = futures_util::stream::unfold(state, move |mut s| async move {
        loop {
            // The server is stopping: end the stream (an `EventSource` reconnects with
            // `Last-Event-ID` once it is back).
            if *s.shutdown.borrow() {
                return None;
            }
            // CMD-8: every commit may change access (a Grant, a Token, a session, a disabled
            // Member). Mark the generation seen *before* the check and the scan below, so a
            // commit that lands after it is checked on the next turn — before anything it could
            // have changed is sent.
            let generation = *s.rx.borrow_and_update();
            if s.checked != Some(generation) {
                s.checked = Some(generation);
                match s.filter.refresh(&s.st, &s.via) {
                    Ok(true) => {}
                    // The Credential is dead (or the store failed): end the stream. An
                    // `EventSource` that reconnects gets the 401 and stops.
                    Ok(false) | Err(_) => return None,
                }
                // Queued under the old access: drop and rescan from the last event sent.
                if !s.pending.is_empty() {
                    s.pending.clear();
                    s.cursor = Some(s.delivered);
                }
            }
            if let Some(ev) = s.pending.pop_front() {
                s.delivered = ev["seq"].as_i64().unwrap_or(s.delivered);
                let id = ev["seq"].as_i64().unwrap_or(0).to_string();
                let kind = ev["type"].as_str().unwrap_or("event").to_string();
                let msg = SseEvent::default().id(id).event(kind).data(ev.to_string());
                return Some((Ok::<_, Infallible>(msg), s));
            }
            if !s.ready_sent {
                s.ready_sent = true;
                let Ok(head) = s.st.store.last_seq() else { return None };
                let start = s.cursor.unwrap_or(head).min(head);
                s.cursor = Some(start);
                s.delivered = start;
                let msg = SseEvent::default().event("ready").data(json!({ "seq": start }).to_string());
                return Some((Ok(msg), s));
            }
            let Ok(head) = s.st.store.last_seq() else { return None };
            let since = s.cursor.unwrap_or(head).min(head);
            let Ok((events, reached, cut)) = scan(&s.st, &s.filter, &mut s.en, since, head, limit) else {
                return None;
            };
            s.cursor = Some(reached);
            if !events.is_empty() {
                s.pending.extend(events);
                continue;
            }
            if cut {
                continue;
            }
            tokio::select! {
                r = s.rx.changed() => {
                    if r.is_err() {
                        return None;
                    }
                }
                _ = s.shutdown.changed() => return None,
            }
        }
    });
    Sse::new(events).keep_alive(KeepAlive::default()).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_round_trip_and_name_their_types() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
            assert!(!k.types().is_empty());
        }
        assert_eq!(Kind::parse("nope"), None);
        assert_eq!(Kind::words(), "version,action,mention,thread,reply,resolve,reopen");
    }

    #[test]
    fn classify_maps_types_and_mentions() {
        let ev = |kind: &str, payload: Value| Event {
            seq: 1,
            at: "2026-08-18T00:00:00.000Z".into(),
            by: None,
            kind: kind.into(),
            doc_id: Some("k7q2m9xd".into()),
            payload,
        };
        assert_eq!(classify(&ev("version.pushed", json!({})), "m1"), (Some(Kind::Version), false));
        assert_eq!(classify(&ev("action.taken", json!({})), "m1"), (Some(Kind::Action), false));
        assert_eq!(classify(&ev("thread.created", json!({"mentions": ["m1"]})), "m1"), (Some(Kind::Thread), true));
        assert_eq!(classify(&ev("thread.replied", json!({"mentions": ["m2"]})), "m1"), (Some(Kind::Reply), false));
        assert_eq!(classify(&ev("thread.resolved", json!({})), "m1"), (Some(Kind::Resolve), false));
        assert_eq!(classify(&ev("thread.reopened", json!({})), "m1"), (Some(Kind::Reopen), false));
        assert_eq!(classify(&ev("document.deleted", json!({})), "m1"), (None, false));
    }

    #[test]
    fn excerpt_collapses_and_centres_on_the_handle() {
        assert_eq!(excerpt("  hello\n\nworld ", None), "hello world");
        let long = format!("{} @scribe make sure the order list says so. {}", "word ".repeat(30), "tail ".repeat(30));
        let e = excerpt(&long, Some("@scribe"));
        assert!(e.contains("@scribe"), "{e}");
        assert!(e.starts_with('…') && e.ends_with('…'), "{e}");
        assert!(e.chars().count() <= EXCERPT_CHARS + 2, "{e}");
        let head = excerpt(&long, None);
        assert!(head.starts_with("word word") && head.ends_with('…') && !head.contains("@scribe"), "{head}");
        let short = excerpt("a @b c", Some("@b"));
        assert_eq!(short, "a @b c");
        // A mention near the end never leaves a short tail.
        let late = format!("{}@x", "y".repeat(100));
        let e = excerpt(&late, Some("@x"));
        assert!(e.ends_with("@x") && e.chars().count() == EXCERPT_CHARS + 1, "{e}");
    }
}
