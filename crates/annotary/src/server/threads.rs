//! `/api/v1/docs/{id}/threads…` — Threads and Comments (ticket 04) and `/api/v1/mentions`.
//!
//! | route | right | what |
//! |---|---|---|
//! | `GET /api/v1/docs/{id}/threads?status=open\|resolved\|all&mentions_me=1&since=T&sort=recent&hidden=1&anchor=anchored,refined,orphaned&version=N` | read | `{threads:[…]}` — default open, document order; `anchor=` filters by placement status; `version=N` = the read-only time-slice (Threads as they stood on N; 404 for an unknown N) (ticket 07) |
//! | `POST /api/v1/docs/{id}/threads {subject: span\|document\|asset, range?, ref?, body, version?}` | comment | new Thread on head (201); `version` ≠ head ⇒ 409 `not_head`; range not on head ⇒ 409 `range_not_on_head`; asset `ref` not in the head manifest ⇒ 409 `asset_not_on_head`, missing ⇒ 409 `asset_missing` (ticket 07) |
//! | `GET /api/v1/docs/{id}/threads/{tid}` | read | one Thread with its Replies (`tid` = `t7`) |
//! | `POST /api/v1/docs/{id}/threads/{tid}/replies {body, resolve?}` | comment | Reply (201) `{thread, comment}`; `resolve` also resolves |
//! | `POST /api/v1/docs/{id}/threads/{tid}/resolve` · `…/reopen` | comment | bare events; idempotent (`changed`) |
//! | `PUT /api/v1/docs/{id}/threads/{cid} {body}` | comment (author or owner) | edit a Comment (`cid` = `t7` or `t7.1`) |
//! | `DELETE /api/v1/docs/{id}/threads/{cid}` | comment (author or owner) | tombstone a Comment |
//! | `GET /api/v1/mentions?since=T&vault=` | read | open Threads mentioning the caller across readable Vaults |
//!
//! Errors: `body_cap` (409) names the cap and the current count; unknown Thread/Comment ⇒ 404
//! `no Thread t99 in NAME (ID)`; not the author ⇒ 403; a Comment already tombstoned cannot be
//! edited (409 `deleted`). Mutations on a deleted Document ⇒ 409 `deleted`.

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_store::{
    CommentView, DocumentView, NewThread, Right, StoreError, ThreadFilter, ThreadSort, ThreadStatusFilter,
    ThreadSubject, ThreadView, parse_thread_id, time,
};

use super::app::AppState;
use super::auth::Auth;
use super::docs::{doc_json, load_doc};
use super::error::ApiError;

/// A Comment as the API shows it (handles, not Member ids).
pub fn comment_json(c: &CommentView) -> Value {
    json!({
        "id": c.id,
        "author": c.author_handle,
        "created": { "at": c.at, "version": c.version },
        "body": c.body,
        "mentions": c.mention_handles,
        "edited_at": c.edited_at,
        "deleted": c.deleted.as_ref().map(|d| json!({ "at": d.at, "by": d.by_handle })),
    })
}

/// A Thread as the API shows it: the Sidecar's `threads[]` shape plus `updated_at`,
/// `reply_count` and `hidden` (fully tombstoned).
pub fn thread_json(t: &ThreadView) -> Value {
    let mut v = comment_json(&t.root);
    let obj = v.as_object_mut().expect("object");
    obj.insert("subject".into(), json!(t.subject.as_str()));
    obj.insert("anchor".into(), t.anchor.as_ref().map(|a| json!(a)).unwrap_or(Value::Null));
    obj.insert(
        "resolved".into(),
        t.resolved
            .as_ref()
            .map(|r| json!({ "by": r.by_handle, "at": r.at, "version": r.version }))
            .unwrap_or(Value::Null),
    );
    obj.insert("replies".into(), json!(t.replies.iter().map(comment_json).collect::<Vec<_>>()));
    obj.insert("reply_count".into(), json!(t.replies.len()));
    obj.insert("mentioned".into(), json!(t.mentioned_handles()));
    obj.insert("updated_at".into(), json!(t.updated_at));
    obj.insert("hidden".into(), json!(t.is_fully_deleted()));
    v
}

/// Map the store's Thread errors onto API errors that name the Document.
fn map_err(doc: &DocumentView, e: StoreError) -> ApiError {
    match e {
        StoreError::ThreadNotFound { thread, .. } => {
            ApiError::not_found(format!("no Thread {thread} in {} ({})", doc.head.name, doc.id))
        }
        StoreError::CommentNotFound { comment, .. } => {
            ApiError::not_found(format!("no Comment {comment} in {} ({})", doc.head.name, doc.id))
                .hint("Comment ids are t7 (root) or t7.1 (reply)")
        }
        StoreError::NotHead { expected, head } => {
            ApiError::new(StatusCode::CONFLICT, "not_head", format!("head is v{head}, not v{expected}"))
                .hint(format!("run `annotary pull {}` and re-read before commenting", doc.head.name))
        }
        other => other.into(),
    }
}

fn parse_tid(doc: &DocumentView, tid: &str) -> Result<u32, ApiError> {
    parse_thread_id(tid).ok_or_else(|| {
        ApiError::not_found(format!("no Thread {tid} in {} ({})", doc.head.name, doc.id))
            .hint("Thread ids look like t7; Comment ids like t7.1")
    })
}

#[derive(Debug, Deserialize, Default)]
pub struct ThreadListQuery {
    /// `open` (default) | `resolved` | `all`.
    pub status: Option<String>,
    pub mentions_me: Option<String>,
    pub since: Option<String>,
    /// `recent` for most recent activity first (default: document order).
    pub sort: Option<String>,
    /// `1` = include fully-tombstoned Threads.
    pub hidden: Option<String>,
    /// Ticket 07: read-only time-slice — Threads as they stood on Version N.
    pub version: Option<u32>,
    /// Ticket 07: comma list of `anchored|refined|orphaned` (placement status).
    pub anchor: Option<String>,
}

fn flag(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some("1") | Some("true") | Some("yes"))
}

pub fn filter_from(q: &ThreadListQuery, member_id: &str) -> Result<ThreadFilter, ApiError> {
    let status = match q.status.as_deref().map(str::trim) {
        None | Some("") | Some("open") => ThreadStatusFilter::Open,
        Some("resolved") => ThreadStatusFilter::Resolved,
        Some("all") => ThreadStatusFilter::All,
        Some(other) => return Err(ApiError::invalid(format!("status={other:?}: use open|resolved|all"))),
    };
    let sort = match q.sort.as_deref().map(str::trim) {
        None | Some("") | Some("document") => ThreadSort::Document,
        Some("recent") => ThreadSort::Recent,
        Some(other) => return Err(ApiError::invalid(format!("sort={other:?}: use recent"))),
    };
    let since = match q.since.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(
            time::parse(s)
                .map(time::format)
                .ok_or_else(|| ApiError::invalid(format!("since={s:?} is not an ISO-8601 timestamp")))?,
        ),
        None => None,
    };
    // ---- ticket 07: placement-status filter ----
    let anchor_status = match q.anchor.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        None => None,
        Some(list) => {
            let mut out = Vec::new();
            for s in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                if annotary_diff::Status::parse(s).is_none() {
                    return Err(ApiError::invalid(format!("anchor={s:?}: use anchored|refined|orphaned (comma list)")));
                }
                if !out.iter().any(|x: &String| x == s) {
                    out.push(s.to_string());
                }
            }
            Some(out)
        }
    };
    // ---- end ticket 07 ----
    Ok(ThreadFilter {
        status,
        mentions: flag(q.mentions_me.as_deref()).then(|| member_id.to_string()),
        since,
        sort,
        include_hidden: flag(q.hidden.as_deref()),
        anchor_status,
    })
}

pub async fn list(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<ThreadListQuery>,
) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let filter = filter_from(&q, &auth.0.member_id)?;
    let all_filter = ThreadFilter { status: ThreadStatusFilter::All, ..Default::default() };
    // ---- ticket 07: `version=N` reads the time-slice instead of the live Threads ----
    let (threads, all, version) = match q.version {
        Some(n) => {
            let map = |e: annotary_store::StoreError| match e {
                annotary_store::StoreError::VersionNotFound { .. } => {
                    ApiError::not_found(format!("Document {} has no v{n}", doc.id))
                }
                other => other.into(),
            };
            (
                st.store.threads_at(&doc.id, n, &filter).map_err(map)?,
                st.store.threads_at(&doc.id, n, &all_filter).map_err(map)?,
                n,
            )
        }
        None => (st.store.threads(&doc.id, &filter)?, st.store.threads(&doc.id, &all_filter)?, doc.head.number),
    };
    // ---- end ticket 07 ----
    let resolved_hidden = match filter.status {
        ThreadStatusFilter::Open => all.iter().filter(|t| !t.is_open()).count(),
        _ => 0,
    };
    Ok(Json(json!({
        "id": doc.id,
        "name": doc.head.name,
        "version": version,
        "head": doc.head.number,
        "threads": threads.iter().map(thread_json).collect::<Vec<_>>(),
        "resolved_hidden": resolved_hidden,
    })))
}

#[derive(Debug, Deserialize)]
pub struct NewThreadBody {
    /// `span` | `document` | `asset` (ticket 07).
    pub subject: String,
    #[serde(default)]
    pub range: Option<String>,
    /// The Asset `ref` (as the manifest lists it) for `subject: asset`.
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    pub body: String,
    /// The head Version the poster looked at (refused when the head is not that).
    #[serde(default)]
    pub version: Option<u32>,
}

pub async fn create(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Result<Json<NewThreadBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Comment)?;
    let Json(body) = body?;
    let subject = match body.subject.trim() {
        "span" => ThreadSubject::Span,
        "document" => ThreadSubject::Document,
        "asset" => ThreadSubject::Asset,
        other => return Err(ApiError::invalid(format!("subject {other:?}: use span|document|asset"))),
    };
    if subject == ThreadSubject::Span && body.range.as_deref().map(str::trim).unwrap_or("").is_empty() {
        return Err(ApiError::invalid("a span Thread needs a range \"L:C-L:C\"").hint("or pass subject: document"));
    }
    // ---- ticket 07: an asset Thread names a `ref`; its referencing line comes from the head text.
    let mut line = None;
    let reference = match subject {
        ThreadSubject::Asset => {
            let r = body.reference.as_deref().map(str::trim).filter(|r| !r.is_empty()).ok_or_else(|| {
                ApiError::invalid("an asset Thread needs a ref (the Asset's path as the manifest lists it)")
                    .hint("`annotary pull DOC` — the Sidecar's assets[] lists the refs")
            })?;
            let r = annotary_renderer::refs::normalize("", r);
            if let Some(md) = st.store.read_blob(&doc.head.md_hash)? {
                let text = String::from_utf8_lossy(&md);
                line = crate::assets::scan::reference_lines(&text).into_iter().find(|(x, _)| *x == r).map(|(_, l)| l);
            }
            Some(r)
        }
        _ => None,
    };
    // ---- end ticket 07 ----
    let input =
        NewThread { subject, range: body.range, body: body.body, expect_version: body.version, reference, line };
    let t = st.store.new_thread(&auth.0.member_id, &doc.id, input).map_err(|e| map_err(&doc, e))?;
    tracing::info!("thread {} on {} ({}) by @{}", t.id, doc.head.name, doc.id, auth.0.handle);
    let mut v = thread_json(&t);
    v["doc"] = json!(doc.id);
    v["name"] = json!(doc.head.name);
    Ok((StatusCode::CREATED, Json(v)))
}

pub async fn get(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, tid)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let num = parse_tid(&doc, &tid)?;
    let t = st
        .store
        .thread(&doc.id, num)?
        .ok_or_else(|| ApiError::not_found(format!("no Thread {tid} in {} ({})", doc.head.name, doc.id)))?;
    let mut v = thread_json(&t);
    v["doc"] = json!(doc.id);
    v["name"] = json!(doc.head.name);
    Ok(Json(v))
}

#[derive(Debug, Deserialize)]
pub struct ReplyBody {
    pub body: String,
    #[serde(default)]
    pub resolve: bool,
}

pub async fn reply(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, tid)): Path<(String, String)>,
    body: Result<Json<ReplyBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Comment)?;
    let num = parse_tid(&doc, &tid)?;
    let Json(body) = body?;
    let (t, c) =
        st.store.reply(&auth.0.member_id, &doc.id, num, &body.body, body.resolve).map_err(|e| map_err(&doc, e))?;
    tracing::info!(
        "reply {} on {} ({}) by @{}{}",
        c.id,
        doc.head.name,
        doc.id,
        auth.0.handle,
        if body.resolve { " (resolved)" } else { "" }
    );
    Ok((
        StatusCode::CREATED,
        Json(json!({ "thread": thread_json(&t), "comment": comment_json(&c), "resolved": !t.is_open() })),
    ))
}

async fn set_resolved(st: &AppState, auth: &Auth, id: &str, tid: &str, resolve: bool) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(st, id)?;
    auth.require_right(&doc.vault_name, Right::Comment)?;
    let num = parse_tid(&doc, tid)?;
    let (t, changed) =
        st.store.set_thread_resolved(&auth.0.member_id, &doc.id, num, resolve).map_err(|e| map_err(&doc, e))?;
    if changed {
        tracing::info!(
            "thread {} on {} {} by @{}",
            t.id,
            doc.id,
            if resolve { "resolved" } else { "reopened" },
            auth.0.handle
        );
    }
    let mut v = thread_json(&t);
    v["changed"] = json!(changed);
    Ok(Json(v))
}

pub async fn resolve(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, tid)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    set_resolved(&st, &auth, &id, &tid, true).await
}

pub async fn reopen(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, tid)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    set_resolved(&st, &auth, &id, &tid, false).await
}

#[derive(Debug, Deserialize)]
pub struct EditBody {
    pub body: String,
}

pub async fn edit(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, cid)): Path<(String, String)>,
    body: Result<Json<EditBody>, JsonRejection>,
) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Comment)?;
    let Json(body) = body?;
    let c = st
        .store
        .edit_comment(&auth.0.member_id, auth.0.is_owner(), &doc.id, cid.trim(), &body.body)
        .map_err(|e| map_err(&doc, e))?;
    tracing::info!("comment {} on {} edited by @{}", c.id, doc.id, auth.0.handle);
    Ok(Json(comment_json(&c)))
}

pub async fn delete(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, cid)): Path<(String, String)>,
) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Comment)?;
    let (c, changed) = st
        .store
        .delete_comment(&auth.0.member_id, auth.0.is_owner(), &doc.id, cid.trim())
        .map_err(|e| map_err(&doc, e))?;
    if changed {
        tracing::info!("comment {} on {} deleted by @{}", c.id, doc.id, auth.0.handle);
    }
    let mut v = comment_json(&c);
    v["changed"] = json!(changed);
    Ok(Json(v))
}

#[derive(Debug, Deserialize, Default)]
pub struct MentionsQuery {
    pub since: Option<String>,
    pub vault: Option<String>,
}

/// `GET /api/v1/mentions` — open Threads mentioning the caller across the Documents they can
/// read (`{mentions:[{doc:{…}, thread:{…}}]}`), most recent activity first.
pub async fn mentions(
    State(st): State<AppState>,
    auth: Auth,
    pv: super::vaults::ProfileVaultHeader,
    Query(q): Query<MentionsQuery>,
) -> Result<Json<Value>, ApiError> {
    // CMD-5: the profile's Vault follows a rename (`vaults::scoped_vault`).
    let (vault_ids, _) = super::vaults::scoped_vault_ids(&st, &auth, &pv, q.vault.as_deref())?;
    let since = match q.since.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(
            time::parse(s)
                .map(time::format)
                .ok_or_else(|| ApiError::invalid(format!("since={s:?} is not an ISO-8601 timestamp")))?,
        ),
        None => None,
    };
    let hits = st.store.mentions_for(&auth.0.member_id, Some(&vault_ids), since.as_deref())?;
    Ok(Json(json!({
        "mentions": hits.iter().map(|h| json!({ "doc": doc_json(&h.doc), "thread": thread_json(&h.thread) })).collect::<Vec<_>>(),
    })))
}
