//! `link DOC [THREAD]` (ticket 18): the URL a person opens to look at a Document — or at one of
//! its Threads (`#tN`) — on another device.
//!
//! The server's public URL (`serve --url`) and link relay (`serve --link-base`) come from
//! `GET /api/v1/health`; the URL is `annotary_store::doc_link` (the relay link opens the iOS
//! app where it is installed and the web page everywhere else). With no public URL set the CLI
//! prints its own server's `…/d/{id}[#tN]` and says on stderr that other devices cannot open it.
//! DOC is a live Document (`docref`); THREAD = `tN`, checked on the server (404 → exit 2 with the
//! usual `no Thread tN in NAME (ID)`); `plan/t7` is a usage error like the thread verbs'.
//! Output: the URL alone on stdout, TTY or not; `--json` → `{url, name, id}` (+ `thread`, `public`).

use clap::Args;
use serde_json::{Map, Value, json};

use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode};

use super::Ctx;
use super::docref::{DeletedMode, resolve};
use super::thread::{check_id, reject_composite};

#[derive(Args, Debug)]
pub struct LinkArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// A Thread of it (t7): the link opens at that Thread
    pub thread: Option<String>,
}

pub const FIELDS: FieldSpec =
    FieldSpec { all: &["url", "name", "id", "thread", "public"], default: &["url", "name", "id"] };

/// The stderr note when the server has no public URL.
pub const NO_PUBLIC_NOTE: &str =
    "note: no public URL is set — other devices cannot open this; run: annotary serve --url http://HOST:PORT";

pub fn run(ctx: &Ctx, a: LinkArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    reject_composite(&a.doc)?;
    let thread = a.thread.as_deref().map(|t| check_id(t, false)).transpose()?;
    let api = ctx.api()?;
    let d = resolve(ctx, &api, &a.doc, None, DeletedMode::Exclude)?;
    let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let id = s("id");
    if !d["deleted_at"].is_null() {
        return Err(CliError::not_found(format!("Document {} ({id}) is deleted", s("name")))
            .hint("`annotary restore DOC` brings it back"));
    }
    if let Some(t) = &thread {
        api.get(&format!("/api/v1/docs/{id}/threads/{t}"))?;
    }
    let (url, public) = api.doc_link(&id, thread.as_deref())?;
    if !public {
        ctx.out.note(NO_PUBLIC_NOTE);
    }
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("url".into(), json!(url));
        rec.insert("name".into(), json!(s("name")));
        rec.insert("id".into(), json!(id));
        rec.insert("thread".into(), json!(thread));
        rec.insert("public".into(), json!(public));
        return ctx.out.json_records(&FIELDS, vec![rec], true);
    }
    ctx.out.line(&url);
    Ok(())
}
