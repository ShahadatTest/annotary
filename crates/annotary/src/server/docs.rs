//! `/api/v1/docs…` — Documents and Versions.
//!
//! | route | right | what |
//! |---|---|---|
//! | `GET /api/v1/docs?vault=&name=&exact=&deleted=only\|any&since=&limit=&action=&mentions_me=` | read | list (`{docs, total, truncated}`), newest activity first; ticket 18: rows carry the display `title` |
//! | `POST /api/v1/docs {vault, name, md_hash, manifest?, message?, actions_available?}` | write | create Document + v1 |
//! | `GET /api/v1/docs/resolve?doc=&vault=&deleted=` | read | id → doc; Name → doc when unique (404 `ambiguous` + `candidates` otherwise) |
//! | `GET /api/v1/docs/{id}` | read | the Document with its head (deleted or not); ticket 18: + `title` |
//! | `DELETE /api/v1/docs/{id}` · `POST …/restore` | write | soft delete / undo |
//! | `POST /api/v1/docs/{id}/purge` | owner | permanent, with blob GC (ticket 13: + a GC sweep of the whole blob store, `gc_removed`) |
//! | `GET /api/v1/docs/{id}/versions[/{n}]` | read | Version list / one Version |
//! | `POST /api/v1/docs/{id}/versions {name, md_hash, manifest?, message?, base_version?, contains_head?, discard_head_changes?, placements?, placements_from?, renderer?, html_hash?, actions_available?}` | write | push (201 new / 200 `already`); ticket 17: with `base_version` behind the head, 409 `head_moved` unless `contains_head` == head (rechecked under `--verify-edge`) or `discard_head_changes` (response `discarded[]`, `head_moved` = what was accepted over); ticket 07: `placements[]` (computed from `placements_from` = the head the pusher saw) are applied to every Thread — spot-verified (400 `invalid_placements` on a mismatch), Threads left out placed by the server; the response carries `reanchored {total, anchored, refined, orphaned, changed, verified, computed}` |
//! | `GET /api/v1/docs/{id}/status` | read | `{name,id,version,action,open_threads,mentions_me,seq}` |
//! | `GET /api/v1/docs/{id}/sidecar?version=N` | read | the canonical Sidecar text |
//!
//! `open_threads`, `mentions_me` and `mentions_me=` are live (ticket 04, `server/threads.rs` has
//! the Thread routes). `action` (the last Action on head), `available_actions` /
//! `actions_inherited` (the effective declaration), the `action=` filter and `actions_available`
//! on commits are live (ticket 05, `server/actions.rs` has the Action routes).

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use annotary_sidecar::identity::is_document_id;
use annotary_store::{DeletedFilter, DocFilter, DocumentView, PushInput, Right, VersionView, time};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;
use super::vaults::{ProfileVaultHeader, Scope, scoped_vault, scoped_vault_ids};

/// Default page size of `list`.
pub const LIST_LIMIT: usize = 50;

pub fn version_json(v: &VersionView) -> Value {
    json!({
        "number": v.number,
        "name": v.name,
        "hash": v.hash,
        "md_hash": v.md_hash,
        "by": v.by_handle,
        "at": v.at,
        "message": v.message,
        "same_as": v.same_as,
        "manifest": v.manifest,
        "renderer": v.renderer,
        "html_hash": v.html_hash,
    })
}

/// A Document as the API shows it (head fields flattened; the Thread/Action fields are filled by
/// `doc_json_for` — bare `doc_json` leaves them at 0 / null).
pub fn doc_json(d: &DocumentView) -> Value {
    json!({
        "id": d.id,
        "name": d.head.name,
        "vault": d.vault_name,
        "version": d.head.number,
        "hash": d.head.hash,
        "md_hash": d.head.md_hash,
        "by": d.head.by_handle,
        "at": d.head.at,
        "message": d.head.message,
        "same_as": d.head.same_as,
        "created_at": d.created_at,
        "deleted_at": d.deleted_at,
        "updated_at": d.last_activity_at,
        "action": Value::Null,
        "open_threads": 0,
        "mentions_me": 0,
    })
}

/// `doc_json` with the caller's live `open_threads` / `mentions_me` (ticket 04) and the head
/// `action` (ticket 05: `{id, by, at, version, label, intent}` — the last Action on head, or null).
pub fn doc_json_for(st: &AppState, member_id: &str, d: &DocumentView) -> Result<Value, ApiError> {
    let mut v = doc_json(d);
    let (open, mine) = st.store.thread_counts(&d.id, member_id)?;
    v["open_threads"] = json!(open);
    v["mentions_me"] = json!(mine);
    v["action"] =
        st.store.head_action(&d.id, d.head.number)?.map(|a| super::actions::action_json(&a)).unwrap_or(Value::Null);
    Ok(v)
}

/// `doc_json_for` plus `available_actions` (the effective Action declaration: the Document's own,
/// else the Vault defaults) and `actions_inherited` for single-Document responses.
fn doc_json_full(st: &AppState, member_id: &str, d: &DocumentView) -> Result<Value, ApiError> {
    let mut v = doc_json_for(st, member_id, d)?;
    let (available, inherited) = st.store.available_actions(d)?;
    v["available_actions"] = json!(available.iter().map(super::actions::decl_json).collect::<Vec<_>>());
    v["actions_inherited"] = json!(inherited);
    Ok(v)
}

/// The Document for a path id (deleted or not); 404 when the id is malformed or unknown.
pub fn load_doc(st: &AppState, id: &str) -> Result<DocumentView, ApiError> {
    if !is_document_id(id) {
        return Err(ApiError::not_found(format!("no Document {id}")));
    }
    st.store.document(id)?.ok_or_else(|| ApiError::not_found(format!("no Document {id}")))
}

fn parse_deleted(s: Option<&str>) -> Result<DeletedFilter, ApiError> {
    match s.map(str::trim) {
        None | Some("") | Some("no") | Some("false") | Some("0") => Ok(DeletedFilter::Exclude),
        Some("only") | Some("true") | Some("1") | Some("yes") => Ok(DeletedFilter::Only),
        Some("any") | Some("all") => Ok(DeletedFilter::Any),
        Some(other) => Err(ApiError::invalid(format!("deleted={other:?}: use only|any"))),
    }
}

// ---------------------------------------------------------------- list / resolve

#[derive(Debug, Deserialize, Default)]
pub struct ListQuery {
    pub vault: Option<String>,
    /// Substring of the Name (case-insensitive).
    pub name: Option<String>,
    /// Exact Name.
    pub exact: Option<String>,
    pub deleted: Option<String>,
    pub since: Option<String>,
    pub limit: Option<usize>,
    /// `none` = Documents whose head has no Action; an Action id = those whose head Action is it.
    pub action: Option<String>,
    /// `1` = only Documents with an open Thread mentioning me (ticket 04).
    pub mentions_me: Option<String>,
}

// ---- ticket 10: the list query is shared by the API and the web Document list (`listing.rs`) ----
/// The rows of `GET /api/v1/docs` for `q`: `(rows, total, truncated)` — `rows` are `doc_json`
/// objects with live `open_threads` / `mentions_me` / `action`, newest activity first.
/// CMD-5: `pv` is the CLI profile's Vault (`vaults::scoped_vault`; the web page passes none).
pub fn list_rows(
    st: &AppState,
    auth: &Auth,
    pv: &ProfileVaultHeader,
    q: &ListQuery,
) -> Result<(Vec<Value>, usize, bool), ApiError> {
    let (vault_ids, _) = scoped_vault_ids(st, auth, pv, q.vault.as_deref())?;
    let since = match q.since.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(s) => Some(
            time::parse(s)
                .map(time::format)
                .ok_or_else(|| ApiError::invalid(format!("since={s:?} is not an ISO-8601 timestamp")))?,
        ),
        None => None,
    };
    let limit = match q.limit {
        Some(0) => None,
        Some(n) => Some(n),
        None => Some(LIST_LIMIT),
    };
    let filter = DocFilter {
        vault_ids: Some(vault_ids),
        name_contains: q.name.clone().filter(|s| !s.is_empty()),
        name_exact: q.exact.clone(),
        deleted: parse_deleted(q.deleted.as_deref())?,
        since,
        limit: None,
    };
    let (mut docs, _) = st.store.list_documents(&filter)?;
    // Head Actions in one pass (ticket 05): a Document's last Action counts iff it is on the
    // current head. `action=none` keeps Documents without one; an id keeps those whose head
    // Action is that id.
    let last_actions = st.store.last_actions_all()?;
    let head_action = |d: &DocumentView| last_actions.get(&d.id).filter(|a| a.version == d.head.number);
    match q.action.as_deref().map(str::trim) {
        None | Some("") => {}
        Some("none") => docs.retain(|d| head_action(d).is_none()),
        Some(id) => docs.retain(|d| head_action(d).is_some_and(|a| a.id == id)),
    }
    // Thread counts for the caller in one pass; `mentions_me=1` keeps Documents with an open
    // Thread mentioning them.
    let counts = st.store.thread_counts_all(&auth.0.member_id)?;
    if matches!(q.mentions_me.as_deref(), Some("1") | Some("true") | Some("yes")) {
        docs.retain(|d| counts.get(&d.id).is_some_and(|(_, m)| *m > 0));
    }
    let total = docs.len();
    if let Some(l) = limit {
        docs.truncate(l);
    }
    let mut rows: Vec<Value> = Vec::with_capacity(docs.len());
    for d in &docs {
        let mut v = doc_json(d);
        let (open, mine) = counts.get(&d.id).copied().unwrap_or((0, 0));
        v["open_threads"] = json!(open);
        v["mentions_me"] = json!(mine);
        v["action"] = head_action(d).map(super::actions::action_json).unwrap_or(Value::Null);
        // ticket 18 (R5): the display title (front matter `title:` else the first H1; null when
        // none) — cached per Markdown blob hash, the same value the web list shows.
        v["title"] = json!(super::ui::title_for(st, &d.head.md_hash)?);
        rows.push(v);
    }
    Ok((rows, total, docs.len() < total))
}
// ---- end ticket 10 ----

pub async fn list(
    State(st): State<AppState>,
    auth: Auth,
    pv: ProfileVaultHeader,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let (rows, total, truncated) = list_rows(&st, &auth, &pv, &q)?;
    Ok(Json(json!({
        "docs": rows,
        "total": total,
        "truncated": truncated,
    })))
}

#[derive(Debug, Deserialize)]
pub struct ResolveQuery {
    pub doc: String,
    pub vault: Option<String>,
    pub deleted: Option<String>,
}

/// `DOC` → Document: an id (any Vault, deleted or not), else a Name unique among the scoped
/// Vaults' Documents (`deleted` selects which). Ambiguous ⇒ 404 `ambiguous` with `candidates`.
pub async fn resolve(
    State(st): State<AppState>,
    auth: Auth,
    pv: ProfileVaultHeader,
    Query(q): Query<ResolveQuery>,
) -> Result<Json<Value>, ApiError> {
    let doc = q.doc.trim();
    if doc.is_empty() {
        return Err(ApiError::invalid("empty DOC"));
    }
    if is_document_id(doc)
        && let Some(d) = st.store.document(doc)?
    {
        auth.require_right(&d.vault_name, Right::Read)?;
        return Ok(Json(doc_json_full(&st, &auth.0.member_id, &d)?));
    }
    let (vault_ids, scoped) = scoped_vault_ids(&st, &auth, &pv, q.vault.as_deref())?;
    let filter = DocFilter {
        vault_ids: Some(vault_ids),
        name_exact: Some(doc.to_string()),
        deleted: parse_deleted(q.deleted.as_deref())?,
        ..Default::default()
    };
    let (found, _) = st.store.list_documents(&filter)?;
    let scope = match scoped {
        Some(v) => format!(" in Vault {v}"),
        None => String::new(),
    };
    match found.len() {
        0 => Err(ApiError::not_found(format!("no Document {doc:?}{scope}")).hint(
            "DOC is an id, a Name unique in the Vault, or the path to a pulled .md; `annotary list` shows Documents",
        )),
        1 => Ok(Json(doc_json_full(&st, &auth.0.member_id, &found[0])?)),
        n => {
            let candidates: Vec<Value> = found
                .iter()
                .map(|d| json!({ "id": d.id, "name": d.head.name, "version": d.head.number, "updated_at": d.last_activity_at }))
                .collect();
            let mut err = ApiError::new(
                StatusCode::NOT_FOUND,
                "ambiguous",
                format!("{n} Documents named {doc:?}{scope} — use the id:"),
            );
            err.extra = Some(Box::new(json!({ "candidates": candidates })));
            Err(err)
        }
    }
}

// ---------------------------------------------------------------- create / push

#[derive(Debug, Deserialize)]
pub struct CreateBody {
    pub vault: String,
    pub name: String,
    pub md_hash: String,
    #[serde(default)]
    pub manifest: Option<Value>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub placements: Option<Value>,
    #[serde(default)]
    pub renderer: Option<String>,
    #[serde(default)]
    pub html_hash: Option<String>,
    /// Per-Document Action declaration from the Sidecar (ticket 05 stores it).
    #[serde(default)]
    pub actions_available: Option<Value>,
}

fn push_input(
    name: &str,
    md_hash: &str,
    manifest: Option<Value>,
    message: Option<String>,
) -> Result<PushInput, ApiError> {
    let manifest = manifest.unwrap_or_else(|| Value::Array(Vec::new()));
    if !manifest.is_array() {
        return Err(ApiError::invalid("manifest must be an array"));
    }
    if !annotary_store::blobs::is_blob_hash(md_hash) {
        return Err(ApiError::invalid(format!("md_hash {md_hash:?} is not a sha256 hex hash")));
    }
    let mut input = PushInput::new(name.trim_end_matches('\n'), md_hash);
    input.manifest = manifest;
    input.message = message.map(|m| m.trim().to_string()).filter(|m| !m.is_empty());
    Ok(input)
}

pub async fn create(
    State(st): State<AppState>,
    auth: Auth,
    pv: ProfileVaultHeader,
    body: Result<Json<CreateBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let Json(body) = body?;
    // CMD-5: the profile's Vault follows a rename; a deleted one is refused (the CLI clears it
    // from the profile on `X-Annotary-Deleted-Vault` and picks again).
    let vault = match scoped_vault(&st, &auth, &pv, Some(&body.vault), Right::Write)? {
        Scope::Vault(v) => v,
        Scope::Deleted(name) => {
            return Err(ApiError::new(StatusCode::NOT_FOUND, "vault_deleted", format!("Vault {name} was deleted"))
                .hint("pass --vault NAME (`annotary vault list` shows them)"));
        }
        Scope::Every => return Err(ApiError::not_found("no Vault \"\"")),
    };
    let mut input = push_input(&body.name, &body.md_hash, body.manifest, body.message)?;
    input.placements = body.placements.unwrap_or_else(|| Value::Array(Vec::new()));
    input.renderer = body.renderer;
    input.html_hash = body.html_hash;
    input.actions_available = body.actions_available; // ticket 05
    let doc = st.store.create_document(&auth.0.member_id, &vault.id, input)?;
    tracing::info!("document {} ({}) created in {} by @{}", doc.id, doc.head.name, vault.name, auth.0.handle);
    let mut v = doc_json(&doc);
    v["already"] = json!(false);
    v["head_moved"] = Value::Null;
    Ok((StatusCode::CREATED, Json(v)))
}

#[derive(Debug, Deserialize)]
pub struct PushBody {
    pub name: String,
    pub md_hash: String,
    #[serde(default)]
    pub manifest: Option<Value>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub base_version: Option<u32>,
    /// Ticket 17: the head the pusher checked its file contains the changes of (since
    /// `base_version`); needed — or `discard_head_changes` — when the head has moved.
    #[serde(default)]
    pub contains_head: Option<u32>,
    #[serde(default)]
    pub discard_head_changes: bool,
    #[serde(default)]
    pub placements: Option<Value>,
    /// Ticket 07: the head Version the placements were computed from.
    #[serde(default)]
    pub placements_from: Option<u32>,
    #[serde(default)]
    pub renderer: Option<String>,
    #[serde(default)]
    pub html_hash: Option<String>,
    #[serde(default)]
    pub actions_available: Option<Value>,
}

pub async fn push(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    body: Result<Json<PushBody>, JsonRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Write)?;
    let Json(body) = body?;
    let mut input = push_input(&body.name, &body.md_hash, body.manifest, body.message)?;
    input.base_version = body.base_version;
    input.contains_head = body.contains_head; // ticket 17
    input.discard_head_changes = body.discard_head_changes;
    input.placements = body.placements.unwrap_or_else(|| Value::Array(Vec::new()));
    input.placements_from = body.placements_from; // ticket 07
    input.renderer = body.renderer;
    input.html_hash = body.html_hash;
    input.actions_available = body.actions_available; // ticket 05
    // ---- ticket 07: the referencing lines of the new Markdown, for Asset Threads (the blob is
    // already in the store — the commit is refused otherwise).
    if let Some(md) = st.store.read_blob(&input.md_hash)? {
        input.asset_lines = crate::assets::scan::reference_lines(&String::from_utf8_lossy(&md));
    }
    // ---- end ticket 07 ----
    let out = st.store.push_version(&auth.0.member_id, &doc.id, input)?;
    let doc = load_doc(&st, &id)?;
    if !out.already {
        tracing::info!("document {} v{} pushed by @{}", doc.id, out.version.number, auth.0.handle);
        if !out.discarded.is_empty() {
            tracing::info!("document {} v{}: discarded the changes of {:?}", doc.id, out.version.number, out.discarded);
        }
        if let Some(r) = &out.reanchored
            && r.total > 0
        {
            tracing::info!(
                "document {} v{}: {} ({} verified, {} placed by the server)",
                doc.id,
                out.version.number,
                r.line(),
                r.verified,
                r.computed
            );
        }
    }
    let mut v = doc_json_for(&st, &auth.0.member_id, &doc)?;
    v["version"] = json!(out.version.number);
    v["hash"] = json!(out.version.hash);
    v["same_as"] = json!(out.version.same_as);
    v["already"] = json!(out.already);
    v["head_moved"] = out.head_moved.as_ref().map(|h| json!(h)).unwrap_or(Value::Null);
    v["discarded"] = json!(out.discarded); // ticket 17
    v["reanchored"] = out.reanchored.as_ref().map(|r| json!(r)).unwrap_or(Value::Null); // ticket 07
    let status = if out.already { StatusCode::OK } else { StatusCode::CREATED };
    Ok((status, Json(v)))
}

// ---------------------------------------------------------------- one Document

pub async fn get(State(st): State<AppState>, auth: Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let mut v = doc_json_full(&st, &auth.0.member_id, &doc)?;
    // ticket 18 (R5): the display title, as the list rows carry it.
    v["title"] = json!(super::ui::title_for(&st, &doc.head.md_hash)?);
    Ok(Json(v))
}

pub async fn versions(State(st): State<AppState>, auth: Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let versions = st.store.versions(&doc.id)?;
    Ok(Json(json!({
        "id": doc.id,
        "name": doc.head.name,
        "vault": doc.vault_name,
        "version": doc.head.number,
        "versions": versions.iter().map(version_json).collect::<Vec<_>>(),
    })))
}

pub async fn version(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, n)): Path<(String, u32)>,
) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let v = st.store.version(&doc.id, n)?.ok_or_else(|| ApiError::not_found(format!("Document {id} has no v{n}")))?;
    let mut j = version_json(&v);
    j["id"] = json!(doc.id);
    j["vault"] = json!(doc.vault_name);
    j["head"] = json!(doc.head.number);
    Ok(Json(j))
}

/// `status DOC`: the ≈24-token record.
pub async fn status(State(st): State<AppState>, auth: Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let (open, mine) = st.store.thread_counts(&doc.id, &auth.0.member_id)?;
    let action = st.store.head_action(&doc.id, doc.head.number)?; // ticket 05
    let mut m = Map::new();
    m.insert("name".into(), json!(doc.head.name));
    m.insert("id".into(), json!(doc.id));
    m.insert("vault".into(), json!(doc.vault_name));
    m.insert("version".into(), json!(doc.head.number));
    m.insert("action".into(), action.as_ref().map(super::actions::action_json).unwrap_or(Value::Null));
    m.insert("open_threads".into(), json!(open));
    m.insert("mentions_me".into(), json!(mine));
    m.insert("seq".into(), json!(st.store.last_seq()?));
    m.insert("deleted_at".into(), json!(doc.deleted_at));
    Ok(Json(Value::Object(m)))
}

#[derive(Debug, Deserialize, Default)]
pub struct SidecarQuery {
    pub version: Option<u32>,
}

/// The canonical Sidecar text for `version` (default head), served as JSON.
pub async fn sidecar(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
    Query(q): Query<SidecarQuery>,
) -> Result<Response, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let base = q.version.unwrap_or(doc.head.number);
    let sc = super::sidecar::build(&st.store, &doc, base)?;
    let text = annotary_sidecar::emit(&sc);
    Ok(([(header::CONTENT_TYPE, "application/json; charset=utf-8")], text).into_response())
}

// ---------------------------------------------------------------- delete / restore / purge

fn lifecycle_json(st: &AppState, member_id: &str, d: &DocumentView, state: &str) -> Result<Value, ApiError> {
    let mut v = doc_json_for(st, member_id, d)?;
    v["state"] = json!(state);
    Ok(v)
}

pub async fn delete(State(st): State<AppState>, auth: Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Write)?;
    let d = st.store.delete_document(&auth.0.member_id, &doc.id)?;
    tracing::info!("document {} deleted by @{}", d.id, auth.0.handle);
    Ok(Json(lifecycle_json(&st, &auth.0.member_id, &d, "deleted")?))
}

pub async fn restore(State(st): State<AppState>, auth: Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Write)?;
    let d = st.store.restore_document(&auth.0.member_id, &doc.id)?;
    tracing::info!("document {} restored by @{}", d.id, auth.0.handle);
    Ok(Json(lifecycle_json(&st, &auth.0.member_id, &d, "restored")?))
}

pub async fn purge(State(st): State<AppState>, auth: Auth, Path(id): Path<String>) -> Result<Json<Value>, ApiError> {
    auth.require_owner("purge")?;
    let doc = load_doc(&st, &id)?;
    let out = st.store.purge_document(&auth.0.member_id, &doc.id)?;
    tracing::info!("document {} purged by @{} ({} blobs removed)", out.id, auth.0.handle, out.blobs_removed.len());
    // ---- ticket 13: an explicit purge also runs the blob GC (anything else unreferenced and
    // older than the grace period goes with it; `blobs_removed` stays the purge's own count).
    let (gc_removed, gc_bytes) = st.store.gc_sweep(super::housekeeping::GC_GRACE)?;
    if !gc_removed.is_empty() {
        tracing::info!("GC after purge removed {} blobs ({} bytes)", gc_removed.len(), gc_bytes);
    }
    // ---- end ticket 13 ----
    let mut v = doc_json(&doc);
    v["state"] = json!("purged");
    v["blobs_removed"] = json!(out.blobs_removed.len());
    v["gc_removed"] = json!(gc_removed.len()); // ticket 13
    Ok(Json(v))
}
