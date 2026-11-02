//! `push FILE.md [-m MSG] [--doc ID] [--vault V] [--force] [--strict] [--discard-head-changes] [-q]`.
//!
//! Compute at the edge, then two-phase upload: hash the Markdown, scan it for Assets (manifest
//! with dimensions and web-safe derivatives — `crate::assets`), check the caps, ask the server
//! which blobs it lacks (`blobs/check` with the manifest), upload only those, commit (create the
//! Document, or a new Version bound by the Sidecar / `--doc`), then rewrite the Sidecar beside
//! FILE.md from the server's fresh projection so its `base_version` is the Version just pushed.
//! Progress and warnings on stderr (`warning: …` lines, then `assets: N referenced, M new
//! (SIZE), K missing (…)` when anything is referenced); exactly one result line on stdout:
//! `name id vN` | `name id vN (same content as vK)` | `name id already vN`. `--strict` turns
//! every warning (missing/unsupported asset, absolute path, depth, no derivative) into exit 5
//! before anything is uploaded. The render happens here too (ticket 09, ADR 0001):
//! the Markdown is rendered to viewer-agnostic HTML with the manifest, the HTML blob rides along
//! in the two-phase flow and the commit records `html_hash` + `renderer` — the server stores it
//! and never renders on push. Re-anchoring happens here as well (ticket 07): for an existing
//! Document whose content changed, the CLI fetches the head's Threads and Markdown, maps every
//! Thread from head to the new text with `annotary_diff` (`annotary_store::compute_placements`
//! — the same function the server verifies with), prints `threads: N re-anchored — a anchored,
//! r refined, o orphaned (…)` on stderr and submits `placements[]` + `placements_from` with the
//! commit. Mapping is incremental: the head's stored placements already carry every earlier
//! Version, so a moved head (someone pushed since your pull) needs no intermediate Versions.
//!
//! Concurrent edits (ticket 17, ADR 0002): when the Sidecar's `base_version` is behind the head
//! and the file differs from the head, the CLI fetches the base and head Markdown + manifests and
//! runs `annotary_store::check_head_changes` *before* re-anchoring and uploading. Contained
//! (the Member merged the head's changes with git or by hand) ⇒ a normal push from the head,
//! `contains_head: H` in the commit. Missing ⇒ exit 5 `head_moved` with the missing base→head
//! hunks on stderr (`--json`: `missing[]`, `assets[]`). `--discard-head-changes` skips the check
//! and pushes over them with a warning. A Sidecar left with git conflict markers is read side by
//! side (`Binding`: the lower `base_version`) and rewritten clean after the push.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use clap::Args;
use serde_json::{Map, Value, json};

use annotary_sidecar::identity::sha256_hex;
use annotary_store::caps::{ASSET_MAX, MARKDOWN_MAX, VERSION_MAX, human_size};

use crate::assets::{self, Scan, Warning};
use crate::exit::{CliError, CliResult, Exit};
use crate::output::{FieldSpec, JsonMode};

use super::Ctx;
use super::docref::{DeletedMode, name_of, read_binding, resolve, select_vault, sidecar_path, version_word};

#[derive(Args, Debug)]
pub struct PushArgs {
    /// The Markdown file (its stem becomes the Document's Name)
    pub file: PathBuf,
    /// Version message
    #[arg(short = 'm', long, value_name = "MSG")]
    pub message: Option<String>,
    /// Bind this file to an existing Document (writes the Sidecar); default: the Sidecar beside
    /// FILE.md, else create a new Document. DOC = id | unique Name | path to a pulled .md
    #[arg(long, value_name = "DOC")]
    pub doc: Option<String>,
    /// Vault for a new Document (default: Sidecar → --vault → profile → error if more than one)
    #[arg(long, value_name = "NAME")]
    pub vault: Option<String>,
    /// Warnings are errors: missing/unsupported assets, absolute paths, depth > 5, no web
    /// derivative
    #[arg(long)]
    pub strict: bool,
    /// Push even if the Sidecar is from another Document id (rebinds it to --doc)
    #[arg(long)]
    pub force: bool,
    /// Push over the changes the head made since your pull (they stay in the history; the
    /// event log records what was discarded). Without it a push whose file lacks them is refused
    #[arg(long)]
    pub discard_head_changes: bool,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &["name", "id", "version", "same_as", "already", "warnings", "assets", "threads", "vault", "hash", "url"],
    default: &["name", "id", "version", "same_as", "already", "warnings"],
};

pub fn run(ctx: &Ctx, a: PushArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    let name = name_of(&a.file)?;
    let bytes =
        std::fs::read(&a.file).map_err(|e| CliError::usage(format!("cannot read {}: {e}", a.file.display())))?;
    if bytes.len() as u64 > MARKDOWN_MAX {
        return Err(too_large(&format!("{}", a.file.display()), bytes.len() as u64, MARKDOWN_MAX));
    }
    let md_hash = sha256_hex(&bytes);
    let sc_path = sidecar_path(&a.file);
    let binding = read_binding(&sc_path)?;
    let api = ctx.api()?;

    // What are we pushing onto? `--doc` accepts any DOC form and is resolved to an id first.
    let bind_to: Option<String> = match &a.doc {
        Some(doc) => Some(
            resolve(ctx, &api, doc, a.vault.as_deref(), DeletedMode::Any)?["id"].as_str().unwrap_or("").to_string(),
        ),
        None => None,
    };
    // Every warning, for `--json`; `late` = the ones printed after the commit (rebinding); asset
    // warnings (and ticket 17's discard warning) are printed before anything is uploaded.
    let mut warnings: Vec<String> = Vec::new();
    let mut late: Vec<String> = Vec::new();
    let target: Option<String> = match (&bind_to, &binding) {
        (Some(id), Some(b)) if &b.id != id => {
            if !a.force {
                return Err(CliError::refused(format!(
                    "{} is bound to {} ({}) — pass --force to rebind it to {id}",
                    a.file.display(),
                    b.id,
                    sc_path.file_name().and_then(|f| f.to_str()).unwrap_or("Sidecar"),
                ))
                .hint("or delete the Sidecar to create a new Document"));
            }
            late.push(format!("rebinding {} from {} to {id}", a.file.display(), b.id));
            Some(id.clone())
        }
        (Some(id), _) => Some(id.clone()),
        (None, Some(b)) => Some(b.id.clone()),
        (None, None) => None,
    };
    if let (Some(v), Some(_)) = (&a.vault, &target)
        && a.doc.is_none()
    {
        ctx.out.note(&format!("note: --vault {v} ignored — the Document's Vault is fixed"));
    }
    // A base_version only means something for the Document it was pulled from.
    let base_version = match (&binding, &target) {
        (Some(b), Some(t)) if &b.id == t => Some(b.base_version),
        _ => None,
    };
    let actions_available: Option<Value> = match (&binding, &target) {
        (Some(b), Some(t)) if &b.id == t => b.actions_available.as_ref().map(|d| json!(d)),
        _ => None,
    };
    // A per-Document declaration (Sidecar `actions.available` without `inherited`) is checked
    // with the server's rules before anything is uploaded (ticket 05).
    if let Some(av) = &actions_available {
        annotary_store::validate_declaration(av).map_err(|e| {
            CliError::new(
                Exit::Usage,
                "invalid_actions",
                format!("{}: {e}", sc_path.file_name().and_then(|f| f.to_str()).unwrap_or("Sidecar")),
            )
            .hint(
                "actions.available is a list of {id, label, intent} — kebab-case ids, intent positive|neutral|negative",
            )
        })?;
    }

    // Ticket 17: a Sidecar git left in conflict is read side by side (lower base_version).
    if let Some(b) = binding.as_ref().filter(|b| b.conflicted) {
        let file = sc_path.file_name().and_then(|f| f.to_str()).unwrap_or("Sidecar");
        ctx.out.note(&format!(
            "note: {file} has git conflict markers — using v{} (the older side) as your base; the push rewrites it",
            b.base_version
        ));
        if b.actions_disagree {
            ctx.out.note(&format!(
                "note: the two sides of {file} declare different actions.available — none sent, the Document keeps its set"
            ));
        }
    }

    // Scan for Assets (compute at the edge): manifest entries, blobs to upload, warnings.
    let scan = assets::scan_document(&a.file, &bytes);
    if a.strict && !scan.warnings.is_empty() {
        return Err(strict_error(&scan.warnings));
    }
    for w in &scan.warnings {
        ctx.out.note(&format!("warning: {}", w.text));
        warnings.push(w.text.clone());
    }
    for e in &scan.entries {
        if let Some(size) = e.size
            && size > ASSET_MAX
        {
            return Err(too_large(&format!("Asset {}", e.reference), size, ASSET_MAX));
        }
    }
    if scan.total_size() > VERSION_MAX {
        return Err(too_large(
            &format!("the Version's {} Assets together", scan.entries.iter().filter(|e| !e.missing).count()),
            scan.total_size(),
            VERSION_MAX,
        ));
    }
    let manifest = scan.manifest_json();

    // Compute at the edge (ticket 09): render to viewer-agnostic HTML with the manifest.
    let html = {
        let text = String::from_utf8_lossy(&bytes);
        let assets = annotary_renderer::Asset::from_manifest(&manifest);
        annotary_renderer::render_html(&text, &assets)
    };
    let html_hash = sha256_hex(html.as_bytes());

    // Ticket 17: the head moved since the base ⇒ the file must already contain the head's
    // changes (checked here, before re-anchoring and before any upload), or the push says it
    // discards them. Then re-anchor every Thread from the head onto this text (ticket 07).
    let local_text = String::from_utf8_lossy(&bytes).into_owned();
    let mut contains_head: Option<u64> = None;
    let reanchor = match &target {
        Some(id) => match head_of(&api, id)? {
            Some(doc) => {
                let head = doc["version"].as_u64().unwrap_or(0);
                let identical = annotary_store::identity_of(&md_hash, &manifest) == doc["hash"].as_str().unwrap_or("");
                let mut head_text: Option<String> = None;
                if let Some(base) = base_version.map(u64::from).filter(|b| *b < head && !identical) {
                    let history = api.get(&format!("/api/v1/docs/{id}/versions"))?;
                    let version = |n: u64| -> CliResult<&Value> {
                        history["versions"]
                            .as_array()
                            .and_then(|vs| vs.iter().find(|v| v["number"].as_u64() == Some(n)))
                            .ok_or_else(|| CliError::internal(format!("the server listed no v{n} of {id}")))
                    };
                    let mut by: Vec<String> = Vec::new();
                    for n in base + 1..=head {
                        if let Some(h) = version(n)?["by"].as_str().filter(|h| !by.iter().any(|b| b == h)) {
                            by.push(h.to_string());
                        }
                    }
                    let (base32, head32) = (base as u32, head as u32);
                    let changes = format!(
                        "{} by {}",
                        annotary_store::versions_word(base32, head32),
                        annotary_store::handles_word(&by)
                    );
                    if a.discard_head_changes {
                        let w = format!("discarding {changes} since your v{base}");
                        ctx.out.note(&format!("warning: {w}"));
                        warnings.push(w);
                    } else {
                        let (base_v, head_v) = (version(base)?, version(head)?);
                        let base_text = fetch_markdown(&api, base_v["md_hash"].as_str().unwrap_or(""))?;
                        let ht = fetch_markdown(&api, head_v["md_hash"].as_str().unwrap_or(""))?;
                        let check = annotary_store::check_head_changes(
                            &base_text,
                            &ht,
                            &local_text,
                            &base_v["manifest"],
                            &head_v["manifest"],
                            &manifest,
                        );
                        if !check.is_contained() {
                            return Err(head_moved_error(ctx, &a.file, base, head, &by, &changes, &check));
                        }
                        contains_head = Some(head);
                        head_text = Some(ht);
                    }
                }
                reanchor_for(&api, id, &doc, &md_hash, &manifest, &local_text, head_text)?
            }
            None => None,
        },
        None => None,
    };

    // Phase 1: which blobs does the server lack? Phase 2: upload only those.
    let lacking =
        api.post("/api/v1/blobs/check", &json!({ "md_hash": md_hash, "assets": manifest, "html_hash": html_hash }))?;
    let lacking: HashSet<String> = lacking["lacking"]
        .as_array()
        .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let assets_summary = summarize_assets(&scan, &lacking);
    if let Some(line) = &assets_summary.line {
        ctx.out.note(line);
    }
    if let Some(r) = &reanchor
        && r.summary.total > 0
    {
        ctx.out.note(&r.summary.line());
    }
    if lacking.contains(&md_hash) {
        api.put_bytes(&format!("/api/v1/blobs/{md_hash}"), &bytes)?;
    }
    for blob in scan.blobs.iter().filter(|b| lacking.contains(&b.hash)) {
        let data = blob.read().map_err(CliError::refused)?;
        api.put_bytes(&format!("/api/v1/blobs/{}", blob.hash), &data)?;
    }
    if lacking.contains(&html_hash) {
        api.put_bytes(&format!("/api/v1/blobs/{html_hash}"), html.as_bytes())?;
    }

    // Commit (with the render: `html_hash` + the renderer version that produced it).
    let mut body = json!({
        "name": name, "md_hash": md_hash, "manifest": manifest, "message": a.message,
        "html_hash": html_hash, "renderer": annotary_renderer::RENDERER_VERSION,
    });
    if let Some(av) = actions_available {
        body["actions_available"] = av;
    }
    let result = match &target {
        Some(id) => {
            body["base_version"] = json!(base_version);
            if let Some(h) = contains_head {
                body["contains_head"] = json!(h); // ticket 17
            }
            if a.discard_head_changes {
                body["discard_head_changes"] = json!(true);
            }
            if let Some(r) = &reanchor {
                body["placements"] = json!(r.placements);
                body["placements_from"] = json!(r.from);
            }
            api.post(&format!("/api/v1/docs/{id}/versions"), &body).map_err(|e| {
                if e.exit == Exit::NotFound && binding.as_ref().is_some_and(|b| &b.id == id) && a.doc.is_none() {
                    CliError::not_found(format!("no Document {id} (from {})", sc_path.display()))
                        .hint("was it purged? `push --doc ID` rebinds; delete the Sidecar to create a new Document")
                } else {
                    e
                }
            })?
        }
        None => {
            body["vault"] = json!(select_vault(ctx, &api, a.vault.as_deref(), &a.file)?);
            match api.post("/api/v1/docs", &body) {
                // CMD-5: the profile's Vault was deleted — the client has cleared it from the
                // profile; pick again as if the profile never named one.
                Err(e) if e.kind == "vault_deleted" && a.vault.is_none() => {
                    body["vault"] = json!(select_vault(ctx, &api, None, &a.file)?);
                    api.post("/api/v1/docs", &body)?
                }
                r => r?,
            }
        }
    };
    let id = result["id"].as_str().unwrap_or("").to_string();
    let version = result["version"].as_u64().unwrap_or(0);
    let same_as = result["same_as"].as_u64();
    let already = result["already"].as_bool().unwrap_or(false);
    let out_name = result["name"].as_str().unwrap_or(&name).to_string();

    warnings.extend(late.iter().cloned());

    // Refresh the Sidecar so base_version is what is now on disk.
    let sc_text = api.get_bytes(&format!("/api/v1/docs/{id}/sidecar?version={version}"))?;
    write_sidecar(&sc_path, &sc_text)?;

    for w in &late {
        ctx.out.note(&format!("warning: {w}"));
    }
    let word = version_word(version, same_as, already);
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("name".into(), json!(out_name));
        rec.insert("id".into(), json!(id));
        rec.insert("version".into(), json!(version));
        rec.insert("same_as".into(), json!(same_as));
        rec.insert("already".into(), json!(already));
        rec.insert("warnings".into(), json!(warnings));
        rec.insert("assets".into(), assets_summary.json());
        rec.insert(
            "threads".into(),
            reanchor.as_ref().filter(|r| r.summary.total > 0).map(|r| r.json()).unwrap_or(Value::Null),
        );
        rec.insert("vault".into(), result["vault"].clone());
        rec.insert("hash".into(), result["hash"].clone());
        // ticket 18: the public link when the server has one; `url` is in `all` only, so `health`
        // is asked only when `--json …,url` names it.
        if matches!(&ctx.out.json, Some(crate::output::JsonMode::Fields(f)) if f.iter().any(|x| x == "url")) {
            rec.insert("url".into(), json!(api.display_link(&id)));
        }
        return ctx.out.json_records(&FIELDS, vec![rec], true);
    }
    if ctx.out.tty {
        let line = format!("{out_name}  {id}  {word}");
        ctx.out.line(&format!("{line:<48}{}", api.display_link(&id))); // ticket 18: public link
    } else {
        ctx.out.line(&format!("{out_name}\t{id}\t{word}"));
    }
    Ok(())
}

/// What the edge computed for the commit's `placements`.
struct Reanchor {
    /// The head Version the placements map from.
    from: u64,
    placements: Vec<annotary_store::SubmittedPlacement>,
    summary: annotary_store::Reanchored,
}

impl Reanchor {
    fn json(&self) -> Value {
        let s = &self.summary;
        let changed: Map<String, Value> = s.changed.iter().map(|(t, st)| (t.clone(), json!(st))).collect();
        json!({ "re_anchored": s.total, "anchored": s.anchored, "refined": s.refined, "orphaned": s.orphaned, "changed": changed })
    }
}

/// `GET /api/v1/docs/{id}` — the head the push lands on; `None` when the Document cannot be
/// read (the commit reports why).
fn head_of(api: &crate::client::Api, id: &str) -> CliResult<Option<Value>> {
    match api.get(&format!("/api/v1/docs/{id}")) {
        Ok(d) => Ok(Some(d)),
        Err(e) if e.exit == Exit::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Re-anchor every Thread of Document `id` from its head (`doc`, see [`head_of`]) onto the text
/// being pushed (ticket 07). `None` when the content is identical to the head (nothing moves —
/// the server says `already`) or when the Document has no Threads. `head_text` = the head's
/// Markdown when the containment check (ticket 17) already fetched it.
fn reanchor_for(
    api: &crate::client::Api,
    id: &str,
    doc: &Value,
    md_hash: &str,
    manifest: &Value,
    new_text: &str,
    head_text: Option<String>,
) -> CliResult<Option<Reanchor>> {
    let head = doc["version"].as_u64().unwrap_or(0);
    let head_hash = doc["hash"].as_str().unwrap_or("");
    let head_md = doc["md_hash"].as_str().unwrap_or("").to_string();
    if annotary_store::identity_of(md_hash, manifest) == head_hash {
        return Ok(None); // already vN
    }
    let listing = api.get(&format!("/api/v1/docs/{id}/threads?status=all&hidden=1"))?;
    let mut threads: Vec<(String, Option<annotary_sidecar::Anchor>)> = listing["threads"]
        .as_array()
        .map(|ts| {
            ts.iter()
                .map(|t| {
                    let anchor =
                        t.get("anchor").filter(|a| !a.is_null()).and_then(|a| serde_json::from_value(a.clone()).ok());
                    (t["id"].as_str().unwrap_or("").to_string(), anchor)
                })
                .collect()
        })
        .unwrap_or_default();
    if threads.is_empty() {
        return Ok(None);
    }
    // By id (the listing is in document order), so the summary names Threads in id order.
    threads.sort_by_key(|(id, _)| annotary_store::parse_thread_id(id).unwrap_or(0));
    let head_text = match head_text {
        Some(t) => t,
        None => fetch_markdown(api, &head_md)?,
    };
    let lines = assets::scan::reference_lines(new_text);
    let placements = annotary_store::compute_placements(&threads, &head_text, new_text, manifest, &lines)
        .map_err(|e| CliError::internal(format!("re-anchoring: {e}")))?;
    let summary = annotary_store::anchors::summarize(&threads, &placements);
    Ok(Some(Reanchor { from: head, placements, summary }))
}

/// A Markdown blob, verified against its hash.
fn fetch_markdown(api: &crate::client::Api, hash: &str) -> CliResult<String> {
    let bytes = api.get_bytes(&format!("/api/v1/blobs/{hash}"))?;
    if sha256_hex(&bytes) != hash {
        return Err(CliError::internal(format!("blob {hash} did not verify")));
    }
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// Ticket 17: the head moved since `base` and the file lacks some of its changes — exit 5 before
/// anything is uploaded. Text: `error: v5 by @x changed FILE since your v4 — those changes are
/// not in your file — put them into FILE and push again`, then the missing base→head hunks and
/// `asset REF changed in v5` lines on stderr. `--json`: `base`, `head`, `by`, `missing[{hunk,
/// lines}]`, `assets[{ref, change}]` next to `error`/`message`/`hint`.
fn head_moved_error(
    ctx: &Ctx,
    file: &Path,
    base: u64,
    head: u64,
    by: &[String],
    changes: &str,
    check: &annotary_store::HeadChanges,
) -> CliError {
    let file = file.display();
    let mut e = CliError::new(
        Exit::Refused,
        "head_moved",
        format!("{changes} changed {file} since your v{base} — those changes are not in your file"),
    )
    .hint(format!("put them into {file} and push again"));
    let versions = annotary_store::versions_word(base as u32, head as u32);
    if ctx.out.is_json() {
        let mut extra = Map::new();
        extra.insert("base".into(), json!(base));
        extra.insert("head".into(), json!(head));
        extra.insert("by".into(), json!(by));
        extra.insert(
            "missing".into(),
            json!(check.hunks.iter().map(|h| json!({ "hunk": h.header(), "lines": h.lines })).collect::<Vec<_>>()),
        );
        extra.insert(
            "assets".into(),
            json!(
                check
                    .assets
                    .iter()
                    .map(|c| json!({ "ref": c.reference, "change": c.kind.as_str() }))
                    .collect::<Vec<_>>()
            ),
        );
        e.extra = Some(Box::new(extra));
        return e;
    }
    for line in annotary_store::concurrent::format_unified(&check.hunks).lines() {
        e = e.detail(line);
    }
    for c in &check.assets {
        e = e.detail(format!("asset {} {} in {versions}", c.reference, c.kind.as_str()));
    }
    e
}

/// A cap was exceeded on the client side (same constants as the server): exit 5, `too_large`.
fn too_large(what: &str, size: u64, max: u64) -> CliError {
    CliError::new(
        Exit::Refused,
        "too_large",
        format!("{what} is {} — the cap is {}", human_size(size), human_size(max)),
    )
    .hint("caps: Markdown 2 MB, each Asset 100 MB, all Assets of a Version 500 MB")
}

/// `--strict`: the scan's warnings become one exit-5 error (nothing has been uploaded yet).
fn strict_error(ws: &[Warning]) -> CliError {
    let bare = |w: &Warning| w.text.split(" — ").next().unwrap_or(&w.text).to_string();
    let hint = "fix the references or push without --strict";
    if ws.len() == 1 {
        return CliError::new(Exit::Refused, "strict", format!("{} (--strict)", bare(&ws[0]))).hint(hint);
    }
    let mut e = CliError::new(Exit::Refused, "strict", format!("{} push warnings (--strict)", ws.len())).hint(hint);
    for w in ws {
        e = e.detail(format!("  {}", bare(w)));
    }
    e
}

/// The `assets:` progress line and the `--json` `assets` field.
struct AssetsSummary {
    referenced: usize,
    new: usize,
    new_bytes: u64,
    missing: Vec<String>,
    /// `None` when the Document references nothing (no line printed).
    line: Option<String>,
}

impl AssetsSummary {
    fn json(&self) -> Value {
        if self.referenced == 0 {
            return Value::Null;
        }
        json!({ "referenced": self.referenced, "new": self.new, "new_bytes": self.new_bytes, "missing": self.missing })
    }
}

/// `assets: 4 referenced, 2 new (57 KB), 1 missing (img/cli-screenshot.png)`.
fn summarize_assets(scan: &Scan, lacking: &HashSet<String>) -> AssetsSummary {
    let referenced = scan.entries.len();
    let new = scan.entries.iter().filter(|e| e.hash.as_ref().is_some_and(|h| lacking.contains(h))).count();
    let new_bytes: u64 = scan.blobs.iter().filter(|b| lacking.contains(&b.hash)).map(|b| b.size).sum();
    let missing: Vec<String> = scan.missing_refs().into_iter().map(str::to_string).collect();
    let line = (referenced > 0).then(|| {
        let mut l = format!("assets: {referenced} referenced, {new} new");
        if new > 0 {
            l.push_str(&format!(" ({})", human_size(new_bytes)));
        }
        if !missing.is_empty() {
            l.push_str(&format!(", {} missing ({})", missing.len(), missing.join(", ")));
        }
        l
    });
    AssetsSummary { referenced, new, new_bytes, missing, line }
}

/// Write the Sidecar text next to the `.md` (the server's canonical bytes; re-emitted through
/// the shared writer so the file is canonical whatever the wire form).
pub fn write_sidecar(path: &Path, text: &[u8]) -> CliResult<()> {
    let text = std::str::from_utf8(text).map_err(|_| CliError::internal("server sent a non-UTF-8 Sidecar"))?;
    let sc = annotary_sidecar::parse(text)
        .map_err(|e| CliError::internal(format!("server sent an invalid Sidecar: {e}")))?;
    std::fs::write(path, annotary_sidecar::emit(&sc))
        .map_err(|e| CliError::usage(format!("cannot write {}: {e}", path.display())))?;
    Ok(())
}
