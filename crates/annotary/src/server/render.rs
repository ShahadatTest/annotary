//! Rendered HTML of a Version (ticket 09; ADR 0001).
//!
//! | route | right | what |
//! |---|---|---|
//! | `GET /api/v1/docs/{id}/versions/{n}/html` | read | the Version's rendered HTML (`text/html`); a Version with no HTML or a renderer older than [`annotary_renderer::MIN_RENDERER_VERSION`] is rendered here with the same crate, the blob stored and recorded on the Version (the fallback the ADR allows — never on push) |
//! | `POST /api/v1/docs/{id}/rerender` | write | re-render every Version of the Document with this server's renderer, in place → `{id, name, versions, changed, renderer}` |
//! | `POST /api/v1/rerender` | owner | the same for every Document (deleted ones too) → `{documents, versions, changed, renderer, docs[]}` |
//!
//! [`ensure_html`] is what the web view (ticket 10) composes its page from. Nothing here touches
//! Version identity, numbers, Threads or Actions — only the derived `html_hash` / `renderer`.

use axum::Json;
use axum::extract::{Path, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use annotary_renderer::{Asset, RENDERER_VERSION, is_current, render_html};
use annotary_store::{DeletedFilter, DocFilter, DocumentView, Right, VersionView};

use super::app::AppState;
use super::auth::Auth;
use super::docs::load_doc;
use super::error::ApiError;

/// Render a Version with this server's crate (the Markdown blob + its manifest).
fn render_version(st: &AppState, v: &VersionView) -> Result<String, ApiError> {
    let md = st
        .store
        .read_blob(&v.md_hash)?
        .ok_or_else(|| ApiError::internal(format!("Markdown blob {} of v{} is missing", v.md_hash, v.number)))?;
    let text = String::from_utf8_lossy(&md);
    let assets = Asset::from_manifest(&v.manifest);
    Ok(render_html(&text, &assets))
}

/// The rendered HTML of a Version: the stored blob when it is current, else a fresh server
/// render, stored and recorded (`version.rendered`). Returns `(html, html_hash, renderer)`.
pub fn ensure_html(st: &AppState, member_id: &str, v: &VersionView) -> Result<(Vec<u8>, String, String), ApiError> {
    if let (Some(hash), true) = (&v.html_hash, is_current(v.renderer.as_deref()))
        && let Some(bytes) = st.store.read_blob(hash)?
    {
        return Ok((bytes, hash.clone(), v.renderer.clone().unwrap_or_default()));
    }
    let html = render_version(st, v)?;
    let (hash, _) = st.store.put_blob(html.as_bytes())?;
    st.store.set_version_html(member_id, &v.doc_id, v.number, &hash, RENDERER_VERSION)?;
    tracing::info!("rendered {} v{} on read (renderer {})", v.doc_id, v.number, RENDERER_VERSION);
    Ok((html.into_bytes(), hash, RENDERER_VERSION.to_string()))
}

/// Re-render one Version in place; true when its HTML (or recorded renderer) changed.
fn rerender_version(st: &AppState, member_id: &str, v: &VersionView) -> Result<bool, ApiError> {
    let html = render_version(st, v)?;
    let (hash, _) = st.store.put_blob(html.as_bytes())?;
    Ok(st.store.set_version_html(member_id, &v.doc_id, v.number, &hash, RENDERER_VERSION)?)
}

/// Every Version of a Document: `(versions, changed)`.
fn rerender_document(st: &AppState, member_id: &str, doc: &DocumentView) -> Result<(usize, usize), ApiError> {
    let versions = st.store.versions(&doc.id)?;
    let mut changed = 0;
    for v in &versions {
        if rerender_version(st, member_id, v)? {
            changed += 1;
        }
    }
    Ok((versions.len(), changed))
}

fn doc_result(doc: &DocumentView, versions: usize, changed: usize) -> Value {
    json!({ "id": doc.id, "name": doc.head.name, "vault": doc.vault_name, "versions": versions, "changed": changed })
}

pub async fn html(
    State(st): State<AppState>,
    auth: Auth,
    Path((id, n)): Path<(String, u32)>,
) -> Result<Response, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let v = st.store.version(&doc.id, n)?.ok_or_else(|| ApiError::not_found(format!("Document {id} has no v{n}")))?;
    let (bytes, hash, renderer) = ensure_html(&st, &auth.0.member_id, &v)?;
    let mut resp = bytes.into_response();
    let h = resp.headers_mut();
    let set = |h: &mut axum::http::HeaderMap, name: header::HeaderName, value: &str| {
        if let Ok(v) = header::HeaderValue::from_str(value) {
            h.insert(name, v);
        }
    };
    set(h, header::CONTENT_TYPE, "text/html; charset=utf-8");
    // A rerender can replace it: revalidate by ETag rather than cache forever.
    set(h, header::CACHE_CONTROL, "private, no-cache");
    set(h, header::ETAG, &format!("\"{hash}\""));
    set(h, header::X_CONTENT_TYPE_OPTIONS, "nosniff");
    set(h, header::HeaderName::from_static("x-annotary-renderer"), &renderer);
    Ok(resp)
}

pub async fn rerender_doc(
    State(st): State<AppState>,
    auth: Auth,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Write)?;
    let (versions, changed) = rerender_document(&st, &auth.0.member_id, &doc)?;
    tracing::info!("document {} rerendered by @{}: {versions} versions, {changed} changed", doc.id, auth.0.handle);
    let mut v = doc_result(&doc, versions, changed);
    v["renderer"] = json!(RENDERER_VERSION);
    Ok(Json(v))
}

pub async fn rerender_all(State(st): State<AppState>, auth: Auth) -> Result<Json<Value>, ApiError> {
    auth.require_owner("rerender --all")?;
    let filter = DocFilter { deleted: DeletedFilter::Any, ..Default::default() };
    let (docs, _) = st.store.list_documents(&filter)?;
    let mut rows = Vec::with_capacity(docs.len());
    let (mut total_versions, mut total_changed) = (0usize, 0usize);
    for doc in &docs {
        let (versions, changed) = rerender_document(&st, &auth.0.member_id, doc)?;
        total_versions += versions;
        total_changed += changed;
        rows.push(doc_result(doc, versions, changed));
    }
    tracing::info!(
        "rerender --all by @{}: {} documents, {total_versions} versions, {total_changed} changed",
        auth.0.handle,
        docs.len()
    );
    Ok(Json(json!({
        "documents": docs.len(),
        "versions": total_versions,
        "changed": total_changed,
        "renderer": RENDERER_VERSION,
        "docs": rows,
    })))
}
