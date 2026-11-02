//! The Document view (ticket 10): `GET /d/{id}` (head) and `GET /d/{id}/v/{n}` (time-slice) —
//! one server-composed HTML response that already contains the rendered Version (ADR 0001:
//! `render::ensure_html`, never rendered here unless the stored HTML is missing/too old), every
//! visible Thread with its placement, the Version `<select>`, the older-Version banner and the
//! Action chip of the viewed Version. Nothing is fetched after load to show a Thread (the only
//! later requests are the on-demand History panel (#cd-whatsnew), `GET /api/v1/events`, and — ticket 11 —
//! the SSE subscription on the same URL, the mutations through `/api/v1`, and the card fragment
//! below that redraws one Thread after a confirmed mutation or a live event).
//!
//! | route | right | what |
//! |---|---|---|
//! | `GET /d/{id}` | web auth + `read` | the head Version with its live Threads |
//! | `GET /d/{id}/v/{n}` | web auth + `read` | Version n with the Threads as they stood on n (`Store::threads_at`); 404 for an unknown n |
//! | `GET /d/{id}/t/{tid}?v=N` | web auth + `read` | ticket 11: JSON `{html, item, hidden, head, version}` — one Thread's card exactly as the page would render it now (`html`) plus its `#cd-data` entry (`item`); `v=N` (N ≠ head) keeps the conversation live but shows the placement as of N (what the time-slice page shows); `hidden` = fully tombstoned (the page drops the card); 404 for an unknown Thread / Version |
//!
//! Threads are spliced into the rendered HTML at the block they anchor to (`splice.rs`): the
//! Document-level strip under the title, one strip per anchor line, the orphaned group last
//! behind a `<details>` — CSS turns those strips into margin cards (layout A, container ≥ 850px)
//! or leaves them inline in the flow (layout C). The embedded `#cd-data` JSON carries what the
//! script needs to highlight spans/Assets and (ticket 11) to comment: the caller's handle and
//! rights, the head Markdown hash (fetched once for selection → source mapping), the head
//! manifest's `(ref, hash)` pairs (an inline image → its Asset Thread `ref`), the available
//! Actions. Errors on the pages come back as HTML; on the fragment as JSON.
//!
//! Ticket 18 (X5): the header carries a hidden `#cd-openapp` chip ("Open in Annotary",
//! `annotary://open?server=<public URL or this request's origin>&doc=<id>`) that the script
//! shows on iPhone / iPad only.

use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

use annotary_sidecar::range::Range;
use annotary_store::{
    ActionDecl, ActionView, CommentView, Right, ThreadFilter, ThreadStatusFilter, ThreadSubject, ThreadView,
    parse_thread_id,
};

use super::app::AppState;
use super::auth::Auth;
use super::docs::load_doc;
use super::error::ApiError;
use super::prefs::{Times, WebPrefs};
use super::render::ensure_html;
use super::splice;
use super::ui::{self, MemberBrief, esc};

pub async fn document(State(st): State<AppState>, auth: Auth, headers: HeaderMap, Path(id): Path<String>) -> Response {
    match render_page(&st, &auth, &id, None, &headers) {
        Ok(r) => r,
        Err(e) => ui::error_page_for(&st, &auth, &headers, e),
    }
}

pub async fn document_version(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((id, n)): Path<(String, u32)>,
) -> Response {
    match render_page(&st, &auth, &id, Some(n), &headers) {
        Ok(r) => r,
        Err(e) => ui::error_page_for(&st, &auth, &headers, e),
    }
}

// ---- ticket 18 (X5) ----
/// The origin the browser used to reach this page (`http://` + `Host`), else the server's own
/// URL — what "Open in Annotary" hands the app when no public URL is set.
fn origin_of(st: &AppState, headers: &HeaderMap) -> String {
    headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::trim)
        .filter(|h| !h.is_empty() && h.chars().all(|c| c.is_ascii_alphanumeric() || "-.:[]".contains(c)))
        .map(|h| format!("http://{h}"))
        .unwrap_or_else(|| st.url.clone())
}

/// The hidden "Open in Annotary" chip: `annotary://open?server=<pct>&doc=<id>` (the
/// script shows it on iPhone / iPad only and appends `&thread=tN` from the URL's `#tN`).
fn open_app_link(server: &str, doc_id: &str) -> String {
    let href = format!(
        "annotary://open?server={}&doc={}",
        annotary_store::percent_encode(server),
        annotary_store::percent_encode(doc_id)
    );
    format!("<a class=\"cd-chip cd-openapp\" id=\"cd-openapp\" href=\"{}\" hidden>Open in Annotary</a>", esc(&href))
}
// ---- end ticket 18 ----

/// A Thread dressed for the page.
struct Item<'a> {
    t: &'a ThreadView,
    status: &'a str,
    /// First / last source line of the current placement (None for Document-level / unplaced).
    line: Option<u32>,
    end: Option<u32>,
    resolved: bool,
    mine: bool,
    /// CMD-7: how many times the placement's text starts earlier on its start line in the viewed
    /// Version's source (`occ_before`), so the script highlights that occurrence, not the first.
    occ: Option<u32>,
}

impl Item<'_> {
    fn orphaned(&self) -> bool {
        self.status == "orphaned"
    }
    fn doc_level(&self) -> bool {
        self.t.subject == ThreadSubject::Document
    }
}

fn item_of<'a>(t: &'a ThreadView, member_id: &str) -> Item<'a> {
    let status = t.placement_status();
    let (line, end) = match &t.anchor {
        None => (None, None),
        Some(a) => {
            let cur = &a.current;
            match cur.range.as_deref().and_then(|r| Range::parse(r).ok()) {
                Some(r) => (Some(r.start_line), Some(r.end_line)),
                None => (cur.line, cur.line),
            }
        }
    };
    Item { t, status, line, end, resolved: !t.is_open(), mine: t.mentions(member_id), occ: None }
}

fn items<'a>(threads: &'a [ThreadView], member_id: &str) -> Vec<Item<'a>> {
    threads.iter().map(|t| item_of(t, member_id)).collect()
}

/// CMD-7: fill `occ` for the span Threads placed in the Version whose Markdown is `md_hash` (read
/// only when one of them has a range).
fn set_occ(st: &AppState, items: &mut [Item], md_hash: &str) -> Result<(), ApiError> {
    let placed = |i: &Item| i.t.subject == ThreadSubject::Span && !i.orphaned() && span_of(i).is_some();
    if !items.iter().any(placed) {
        return Ok(());
    }
    let Some(md) = st.store.read_blob(md_hash)? else { return Ok(()) };
    let md = String::from_utf8_lossy(&md);
    let starts = line_starts(&md);
    for i in items.iter_mut().filter(|i| placed(i)) {
        if let Some((r, exact)) = span_of(i) {
            i.occ = occ_before(&md, &starts, &r, &exact);
        }
    }
    Ok(())
}

/// The current range of a span Thread and the text the script searches for it (`now`, else the
/// frozen quote — the script's `t.now || t.quote`).
fn span_of(i: &Item) -> Option<(Range, String)> {
    let a = i.t.anchor.as_ref()?;
    let r = Range::parse(a.current.range.as_deref()?).ok()?;
    let exact = a.current.exact.clone().or_else(|| a.original.exact.clone())?;
    Some((r, exact))
}

/// Byte offset of the start of each line (index 0 = line 1).
fn line_starts(md: &str) -> Vec<usize> {
    std::iter::once(0).chain(md.match_indices('\n').map(|(k, _)| k + 1)).collect()
}

/// How many occurrences of `exact` start on `r.start_line` before `r.start_col` (code points) in
/// `md`; None when the line is past the end or `exact` is empty.
fn occ_before(md: &str, starts: &[usize], r: &Range, exact: &str) -> Option<u32> {
    if exact.is_empty() {
        return None;
    }
    let rest = &md[*starts.get(r.start_line.checked_sub(1)? as usize)?..];
    let mut n = 0;
    for (col, (k, c)) in rest.char_indices().enumerate() {
        if col as u32 >= r.start_col || c == '\n' {
            break;
        }
        if rest[k..].starts_with(exact) {
            n += 1;
        }
    }
    Some(n)
}

/// The `#cd-data` entry of a Thread (what the script needs to highlight and place it).
fn item_json(i: &Item) -> Value {
    let a = i.t.anchor.as_ref();
    json!({
        "id": i.t.id,
        "subject": i.t.subject.as_str(),
        "status": i.status,
        "line": i.line,
        "end": i.end,
        "quote": a.and_then(|a| a.original.exact.clone()),
        "now": a.and_then(|a| a.current.exact.clone()),
        "hash": a.and_then(|a| a.original.hash.clone()),
        "ref": a.and_then(|a| a.original.reference.clone()),
        "resolved": i.resolved,
        "mine": i.mine,
        "occ": i.occ,
    })
}

/// What the card renderer needs to know about the viewer (ticket 11: which buttons to draw).
struct Viewer<'a> {
    members: &'a std::collections::HashMap<String, MemberBrief>,
    me: &'a str,
    is_owner: bool,
    /// `comment` on the Vault and the Document is live.
    can_comment: bool,
    /// CMD-23: how the viewer's settings render timestamps.
    times: &'a Times,
}

impl Viewer<'_> {
    fn member(&self, handle: &str) -> MemberBrief {
        self.members.get(handle).cloned().unwrap_or_else(|| MemberBrief::unknown(handle))
    }
    /// Edit / delete: the author or an owner (the server's rule), on a live Comment.
    fn may_edit(&self, c: &CommentView) -> bool {
        self.can_comment && !c.is_deleted() && (c.author_handle == self.me || self.is_owner)
    }
}

fn render_page(
    st: &AppState,
    auth: &Auth,
    id: &str,
    n: Option<u32>,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let origin = &origin_of(st, headers);
    let doc = load_doc(st, id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let versions = st.store.versions(&doc.id)?;
    let head = doc.head.number;
    let number = n.unwrap_or(head);
    let v = versions
        .iter()
        .find(|x| x.number == number)
        .ok_or_else(|| ApiError::not_found(format!("Document {} has no v{number}", doc.id)))?;
    let (html, _, _) = ensure_html(st, &auth.0.member_id, v)?;
    let html = String::from_utf8_lossy(&html).into_owned();
    let filter = ThreadFilter { status: ThreadStatusFilter::All, ..Default::default() };
    let threads = if number == head {
        st.store.threads(&doc.id, &filter)?
    } else {
        st.store.threads_at(&doc.id, number, &filter).map_err(|e| match e {
            annotary_store::StoreError::VersionNotFound { .. } => {
                ApiError::not_found(format!("Document {} has no v{number}", doc.id))
            }
            other => other.into(),
        })?
    };
    let log = st.store.actions(&doc.id)?;
    let members = ui::members_by_handle(st)?;
    let seq = st.store.last_seq()?;
    let mut items = items(&threads, &auth.0.member_id);
    set_occ(st, &mut items, &v.md_hash)?; // CMD-7
    // ---- ticket 11: what the viewer may do ----
    let live = !doc.is_deleted();
    let can_comment = live && auth.0.has_right(&doc.vault_name, Right::Comment);
    let can_act = live && auth.0.has_right(&doc.vault_name, Right::Action);
    let (available, _inherited) = st.store.available_actions(&doc)?;
    // ---- CMD-23: the viewer's Web settings, read in the same response (the first paint is final) ----
    let prefs = WebPrefs::load(st, auth, headers)?;
    let times = prefs.times();
    let viewer =
        Viewer { members: &members, me: &auth.0.handle, is_owner: auth.0.is_owner(), can_comment, times: &times };
    // ---- end ticket 11 ----

    let member = |handle: &str| viewer.member(handle);
    let action_on = |n: u32| log.iter().rfind(|a| a.version == n);

    // ---- counts ----
    let open = items.iter().filter(|i| !i.resolved).count();
    let mine = items.iter().filter(|i| !i.resolved && i.mine).count();
    let orphaned_open = items.iter().filter(|i| !i.resolved && i.orphaned()).count();
    let resolved = items.iter().filter(|i| i.resolved).count();

    // ---- strips ----
    let card = |it: &Item| thread_card(it, &viewer, number);
    let doc_items: Vec<&Item> = items.iter().filter(|i| i.doc_level()).collect();
    let head_strip = (!doc_items.is_empty()).then(|| {
        let all_resolved = doc_items.iter().all(|i| i.resolved);
        format!(
            "<div class=\"cd-strip cd-strip-doc{}\"><div class=\"cd-strip-h\">Whole document</div>{}</div>",
            if all_resolved { " cd-all-resolved" } else { "" },
            doc_items.iter().map(|i| card(i)).collect::<String>()
        )
    });
    let mut strips: Vec<(u32, String)> = Vec::new();
    {
        // One strip per anchor line, Threads in document order (consecutive in the list).
        let mut groups: Vec<(u32, Vec<&Item>)> = Vec::new();
        for it in items.iter().filter(|i| !i.doc_level() && !i.orphaned()) {
            let line = it.line.unwrap_or(u32::MAX);
            match groups.last_mut() {
                Some((l, g)) if *l == line => g.push(it),
                _ => groups.push((line, vec![it])),
            }
        }
        for (line, g) in groups {
            let all_resolved = g.iter().all(|i| i.resolved);
            strips.push((
                line,
                format!(
                    "<div class=\"cd-strip{}\" data-line=\"{line}\">{}</div>",
                    if all_resolved { " cd-all-resolved" } else { "" },
                    g.iter().map(|i| card(i)).collect::<String>()
                ),
            ));
        }
    }
    let orphaned_items: Vec<&Item> = items.iter().filter(|i| i.orphaned()).collect();
    // CMD-23: with Show resolved, the count is of every orphaned Thread (as the toggle's script counts)
    let orphaned_shown = if prefs.show_resolved { orphaned_items.len() } else { orphaned_open };
    let tail = (!orphaned_items.is_empty()).then(|| {
        format!(
            "<div class=\"cd-strip cd-strip-orphaned\"><details class=\"cd-orphaned\"><summary>Orphaned (<span class=\"cd-orph-n\">{orphaned_shown}</span>) <span class=\"cd-muted\">— text no longer in v{number}</span></summary>{}</details></div>",
            orphaned_items.iter().map(|i| card(i)).collect::<String>()
        )
    });
    let html = ui::asset_links_blank(&html);
    let body_html = splice::splice(&html, head_strip.as_deref(), &strips, tail.as_deref());

    // ---- header pieces ----
    let same_chip =
        v.same_as.map(|k| format!("<span class=\"cd-chip cd-same\">same content as v{k}</span>")).unwrap_or_default();
    let status_html = {
        let act = match action_on(number) {
            Some(a) => format!("{} by {}", esc(&a.label.to_lowercase()), esc(&member(&a.by_handle).name)),
            None => "no action yet".to_string(),
        };
        let orph = if orphaned_open > 0 { format!(" ({orphaned_open} orphaned)") } else { String::new() };
        let me = if mine > 0 { format!(" · <b class=\"cd-mention\">{mine} @you</b>") } else { String::new() };
        format!("<span class=\"cd-status\"><b>v{number}</b> · {act} · <b>{open}</b> open{orph}{me}</span>")
    };
    let nline = {
        let me = if mine > 0 { format!(" · <b class=\"cd-mention\">{mine} @you</b>") } else { String::new() };
        format!("<span class=\"cd-nline\">· <b>v{number}</b> · <b>{open}</b> open{me}</span>")
    };
    let href = |k: u32| if k == head { format!("/d/{}", doc.id) } else { format!("/d/{}/v/{k}", doc.id) };
    let older = if number > 1 {
        format!("<a class=\"cd-arrow cd-older\" href=\"{}\" title=\"older Version\">\u{2039}</a>", href(number - 1))
    } else {
        "<span class=\"cd-arrow cd-older cd-off\">\u{2039}</span>".to_string()
    };
    let newer = if number < head {
        format!("<a class=\"cd-arrow cd-newer\" href=\"{}\" title=\"newer Version\">\u{203A}</a>", href(number + 1))
    } else {
        "<span class=\"cd-arrow cd-newer cd-off\">\u{203A}</span>".to_string()
    };
    let select = {
        let mut s = String::from("<select id=\"cd-vsel\" aria-label=\"Version\">");
        for x in versions.iter().rev() {
            let by = member(&x.by_handle).name;
            let pre = format!("v{}{} · {} · ", x.number, if x.number == head { " (head)" } else { "" }, by);
            let mut post = String::new();
            if let Some(k) = x.same_as {
                post.push_str(&format!(" · same as v{k}"));
            }
            if let Some(a) = action_on(x.number) {
                post.push_str(&format!(" · {}", a.label));
            }
            s.push_str(&format!(
                "<option value=\"{}\" data-href=\"{}\" data-at=\"{}\" data-pre=\"{}\" data-post=\"{}\"{}>{}{}{}</option>",
                x.number,
                href(x.number),
                esc(&x.at),
                esc(&pre),
                esc(&post),
                if x.number == number { " selected" } else { "" },
                esc(&pre),
                esc(&ui::fallback_time(&x.at)),
                esc(&post)
            ));
        }
        s.push_str("</select>");
        s
    };
    let action_area = match action_on(number) {
        Some(a) => action_chip(a, &member, &times),
        None if number == head => "<span class=\"cd-chip cd-none\">no action yet</span>".to_string(),
        None => format!("<span class=\"cd-chip cd-none\">no action on v{number}</span>"),
    };
    // ---- ticket 11: the Action buttons — head only, coloured by intent, none for `[]` ----
    let action_buttons = if number == head && can_act && !available.is_empty() {
        format!("<span class=\"cd-actbtns\" id=\"cd-actbtns\">{}</span>", action_buttons_html(&available, head))
    } else {
        String::new()
    };
    // ---- end ticket 11 ----
    let banner = if number != head {
        let msg = v.message.as_deref().map(|m| format!(" · \u{201C}{}\u{201D}", esc(m))).unwrap_or_default();
        format!(
            "<div class=\"cd-banner\" id=\"cd-banner\"><span>Viewing <b>v{number}</b> · {} · {}{msg} — Threads as they were; replies/resolve work, new comments only on head.</span><span class=\"cd-grow\"></span><a class=\"cd-btn\" id=\"cd-gohead\" href=\"{}\">Go to head v{head}</a></div>",
            esc(&member(&v.by_handle).name),
            times.tag(&v.at, true), // ticket 15: relative (rendered with its age), so the localization shifts nothing
            href(head)
        )
    } else {
        String::new()
    };
    let deleted_chip = if doc.is_deleted() { "<span class=\"cd-chip cd-warn\">deleted</span>" } else { "" };

    let header = format!(
        "<header class=\"cd-hdr\">\
           <div class=\"cd-line\" id=\"cd-line\">{icon}<a class=\"cd-crumb\" href=\"/\">{vault}</a><span class=\"cd-crumb\">/</span><b class=\"cd-name\">{name}</b><code class=\"cd-docid\">{id}</code>{deleted_chip}{nline}\
             {open_app}<span class=\"cd-arrows\">{older}{newer}</span></div>\
           <div class=\"cd-rest\" id=\"cd-rest\">\
             <span class=\"cd-vsw\">{older}{select}{newer}</span>{same_chip}\
             <span class=\"cd-grow\"></span>{status_html}\
             <label class=\"cd-chip\"><input type=\"checkbox\" id=\"cd-resolved\"{resolved_checked}><span>resolved (<span class=\"cd-res-n\">{resolved}</span>)</span></label>\
             <span class=\"cd-tools\"><button type=\"button\" class=\"cd-btn\" id=\"cd-whatsnew\">History</button>\
             <button type=\"button\" class=\"cd-btn cd-helpbtn\" id=\"cd-helpbtn\" title=\"keyboard shortcuts (?)\">?</button></span>\
             <span class=\"cd-actions\" id=\"cd-actions\">{action_area}{action_buttons}</span>\
           </div>\
         </header>{banner}<div class=\"cd-panel\" id=\"cd-news\" hidden></div>{help}{toast}{pill}",
        icon = super::brand::header_icon(),
        vault = esc(&doc.vault_name),
        name = esc(&v.name),
        id = esc(&doc.id),
        help = help_html(&prefs.send_key),
        resolved_checked = if prefs.show_resolved { " checked" } else { "" },
        open_app = open_app_link(&st.store.public_url()?.unwrap_or_else(|| origin.to_string()), &doc.id), // ticket 18
        toast = "<div class=\"cd-toast\" id=\"cd-toast\" hidden></div>",
        pill = if number == head && can_comment {
            "<button type=\"button\" class=\"cd-pill\" id=\"cd-pill\" hidden>Comment <kbd>c</kbd></button>"
        } else {
            ""
        },
    );

    // ---- embedded data for the script ----
    let assets: Vec<Value> = doc
        .head
        .manifest
        .as_array()
        .map(|m| {
            m.iter()
                .filter(|e| !e["missing"].as_bool().unwrap_or(false))
                .filter_map(|e| {
                    Some(json!({ "ref": e["ref"].as_str()?, "hash": e["hash"].as_str()?, "type": e["type"].as_str() }))
                })
                .collect()
        })
        .unwrap_or_default();
    let data = json!({
        "id": doc.id,
        "name": v.name,
        "vault": doc.vault_name,
        "head": head,
        "version": number,
        "seq": seq,
        "me": auth.0.handle,
        "me_name": member(&auth.0.handle).name,
        "owner": auth.0.is_owner(),
        "can_comment": can_comment,
        "can_act": can_act,
        "deleted": doc.is_deleted(),
        "md_hash": doc.head.md_hash,
        "assets": assets,
        "actions": available.iter().map(|d| json!({ "id": d.id, "label": d.label, "intent": d.intent })).collect::<Vec<_>>(),
        "action": action_on(number).map(|a| json!({ "id": a.id, "label": a.label, "intent": a.intent, "by": a.by_handle, "at": a.at, "version": a.version })),
        "members": members.iter().map(|(h, m)| (h.clone(), json!(m.name))).collect::<serde_json::Map<_, _>>(),
        "threads": items.iter().map(item_json).collect::<Vec<_>>(),
    });
    let body = format!(
        "<div {} id=\"cd-app\" data-doc=\"{}\" data-version=\"{number}\" data-head=\"{head}\">{header}\
         <main class=\"cd-layout\"><article class=\"cd-doc\" id=\"cd-doc\">{body_html}</article></main></div>\
         <script type=\"application/json\" id=\"cd-data\">{}</script>",
        prefs.app_attrs("cd-docpage"),
        esc(&doc.id),
        ui::json_for_script(&data)
    );
    let title =
        if number == head { format!("{} — annotary", v.name) } else { format!("{} v{number} — annotary", v.name) };
    Ok(ui::page(&title, &body))
}

/// The keyboard help overlay (`?`): the map of story 126 and nothing more. CMD-23: the
/// single-key rows (`cd-k1`) hide when Keyboard shortcuts is Off (app.css keys them on
/// `data-cd-keys`, so a live change needs no redraw), and the reply row's `#cd-send-key` names
/// the Send key's chord (`help_html`; the script rewrites it on a live change).
const HELP_HTML: &str = "<div class=\"cd-panel cd-help\" id=\"cd-help\" hidden><h3>Keyboard</h3><table>\
<tr class=\"cd-k1\"><td><kbd>j</kbd> <kbd>k</kbd></td><td>next / previous Thread</td></tr>\
<tr class=\"cd-k1\"><td><kbd>r</kbd></td><td>reply to the selected Thread</td></tr>\
<tr class=\"cd-k1\"><td><kbd>e</kbd></td><td>resolve / reopen the selected Thread</td></tr>\
<tr class=\"cd-k1\"><td><kbd>c</kbd></td><td>comment on the selection (head only)</td></tr>\
<tr class=\"cd-k1\"><td><kbd>[</kbd> <kbd>]</kbd></td><td>older / newer Version</td></tr>\
<tr><td><kbd id=\"cd-send-key\">{SEND}</kbd></td><td>post the reply</td></tr>\
<tr><td><kbd>\u{2318}\u{21A9}</kbd></td><td>post the comment</td></tr>\
<tr><td><kbd>esc</kbd></td><td>close / deselect</td></tr>\
<tr class=\"cd-k1\"><td><kbd>?</kbd></td><td>this help</td></tr>\
</table></div>";

/// [`HELP_HTML`] with the Send key's chord for a reply.
fn help_html(send_key: &str) -> String {
    HELP_HTML.replace("{SEND}", if send_key == "mod_enter" { "\u{2318}\u{21A9}" } else { "\u{21A9}" })
}

/// One button per available Action, coloured by intent (head only).
fn action_buttons_html(available: &[ActionDecl], head: u32) -> String {
    available
        .iter()
        .map(|d| {
            format!(
                "<button type=\"button\" class=\"cd-btn cd-actbtn {}\" data-action=\"{}\" data-label=\"{}\" data-intent=\"{}\" data-version=\"{head}\">{}</button>",
                ui::intent_class(&d.intent),
                esc(&d.id),
                esc(&d.label),
                esc(&d.intent),
                esc(&d.label)
            )
        })
        .collect()
}

fn action_chip(a: &ActionView, member: &dyn Fn(&str) -> MemberBrief, times: &Times) -> String {
    format!(
        "<span class=\"cd-chip cd-action {}\" data-action=\"{}\" data-version=\"{}\">{} · {} · {}</span>",
        ui::intent_class(&a.intent),
        esc(&a.id),
        a.version,
        esc(&a.label),
        esc(&member(&a.by_handle).name),
        times.tag(&a.at, true)
    )
}

/// The quote / subject label of a Thread (card label and row quote alike).
fn label_html(it: &Item, version: u32) -> String {
    let t = it.t;
    match (t.subject, &t.anchor) {
        (ThreadSubject::Document, _) => "<span class=\"cd-kind\">whole document</span>".to_string(),
        (ThreadSubject::Asset, Some(a)) => {
            let r = a.original.reference.as_deref().unwrap_or("");
            let out = if it.orphaned() {
                format!(" <span class=\"cd-out\">· orphaned — not in v{version}</span>")
            } else {
                String::new()
            };
            format!("<span class=\"cd-kind\">asset</span><q>{}</q>{out}", esc(r))
        }
        (_, Some(a)) => {
            let mut s = format!("<q>{}</q>", esc(a.original.exact.as_deref().unwrap_or("")));
            if it.status == "refined" {
                s.push_str(&format!(
                    "<span class=\"cd-now\">now: <q>{}</q></span>",
                    esc(a.current.exact.as_deref().unwrap_or(""))
                ));
            }
            if it.orphaned() {
                s.push_str(&format!(" <span class=\"cd-out\">· orphaned — text no longer in v{version}</span>"));
            }
            s
        }
        (_, None) => String::new(),
    }
}

fn comment_html(c: &CommentView, viewer: &Viewer) -> String {
    let who = viewer.member(&c.author_handle).html();
    let edited = if c.edited_at.is_some() { "<span>· edited</span>" } else { "" };
    let body =
        if c.is_deleted() { "<span class=\"cd-tomb\">(deleted)</span>".to_string() } else { ui::comment_html(&c.body) };
    // ---- ticket 11: edit / delete for the author or an owner ----
    let tools = if viewer.may_edit(c) {
        "<span class=\"cd-ctools\"><button type=\"button\" class=\"cd-link\" data-act=\"edit\">edit</button><button type=\"button\" class=\"cd-link\" data-act=\"delete\">delete</button></span>"
    } else {
        ""
    };
    // ---- end ticket 11 ----
    format!(
        "<div class=\"cd-c\" data-c=\"{}\"><div class=\"cd-who\">{who}<span>{}</span>{edited}<span class=\"cd-mono\">v{}</span>{tools}</div><div class=\"cd-cbody\">{body}</div></div>",
        esc(&c.id),
        viewer.times.tag(&c.at, false),
        c.version
    )
}

/// A short plain-text snippet of a body for the narrow row.
fn snippet(body: &str) -> String {
    let flat: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(120).collect();
    if flat.chars().count() > 120 {
        out.push('\u{2026}');
    }
    out
}

/// The Thread element — a card in layout A, a one-line row (expanding in place) in layout C.
fn thread_card(it: &Item, viewer: &Viewer, version: u32) -> String {
    let t = it.t;
    let mut classes = format!("cd-thread cd-st-{}", it.status);
    if it.resolved {
        classes.push_str(" cd-resolved");
    }
    if it.mine {
        classes.push_str(" cd-mine");
    }
    match t.subject {
        ThreadSubject::Document => classes.push_str(" cd-doclevel"),
        ThreadSubject::Asset => classes.push_str(" cd-asset"),
        ThreadSubject::Span => {}
    }
    let root_snip = if t.root.is_deleted() { "(deleted)".to_string() } else { snippet(&t.root.body) };
    let replies = t.replies.len();
    let last_at = t.replies.last().map(|r| r.at.as_str()).unwrap_or(t.root.at.as_str());
    let meta = format!(
        "{}{} · {}{}",
        esc(&t.id),
        if replies > 0 {
            format!(" · {replies} {}", if replies == 1 { "reply" } else { "replies" })
        } else {
            String::new()
        },
        viewer.times.tag(last_at, true),
        if it.mine && !it.resolved { " · <span class=\"cd-mention\">@you</span>" } else { "" }
    );
    let mut body = comment_html(&t.root, viewer);
    for r in &t.replies {
        body.push_str(&comment_html(r, viewer));
    }
    if let Some(r) = &t.resolved {
        body.push_str(&format!(
            "<div class=\"cd-resline\">\u{2713} resolved by {} · {} · v{}</div>",
            esc(&viewer.member(&r.by_handle).name),
            viewer.times.tag(&r.at, false),
            r.version
        ));
    }
    // ---- ticket 11: reply / resolve / reopen (anyone with `comment`, from any Version view) ----
    if viewer.can_comment {
        body.push_str(&format!(
            "<div class=\"cd-acts\"><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"reply\">Reply</button><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"{}\">{}</button></div>",
            if it.resolved { "reopen" } else { "resolve" },
            if it.resolved { "Reopen" } else { "Resolve" }
        ));
    }
    // ---- end ticket 11 ----
    format!(
        "<article class=\"{classes}\" data-t=\"{id}\" id=\"cd-{id}\"{line}>\
           <div class=\"cd-row\"><span class=\"cd-dot\" title=\"{state}\"></span><div class=\"cd-main\">\
             <div class=\"cd-quote\">{label}</div>\
             <div class=\"cd-snip\"><b>{author}</b> {snip}</div>\
             <div class=\"cd-meta\">{meta}</div></div><span class=\"cd-tid\">{id}</span></div>\
           <div class=\"cd-body\">{body}</div>\
         </article>",
        id = esc(&t.id),
        line = it.line.map(|l| format!(" data-line=\"{l}\"")).unwrap_or_default(),
        state = if it.resolved { "resolved" } else { "open" },
        label = label_html(it, version),
        author = esc(&viewer.member(&t.root.author_handle).name),
        snip = esc(&root_snip),
    )
}

// ---- ticket 11: the card fragment ----

#[derive(Debug, Deserialize, Default)]
pub struct FragmentQuery {
    /// The Version the page shows (placement as of it); absent / head = live.
    pub v: Option<u32>,
}

/// `GET /d/{id}/t/{tid}?v=N` — one Thread's card as the page renders it now (JSON `{html, item,
/// hidden, head, version}`): the script swaps it in after a confirmed mutation or a live event,
/// so the page and the live updates share one renderer.
pub async fn fragment(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path((id, tid)): Path<(String, String)>,
    Query(q): Query<FragmentQuery>,
) -> Result<Json<Value>, ApiError> {
    let doc = load_doc(&st, &id)?;
    auth.require_right(&doc.vault_name, Right::Read)?;
    let num = parse_thread_id(&tid)
        .ok_or_else(|| ApiError::not_found(format!("no Thread {tid} in {} ({})", doc.head.name, doc.id)))?;
    let mut t = st
        .store
        .thread(&doc.id, num)?
        .ok_or_else(|| ApiError::not_found(format!("no Thread {tid} in {} ({})", doc.head.name, doc.id)))?;
    let head = doc.head.number;
    let number = q.v.unwrap_or(head);
    let mut md_hash = doc.head.md_hash.clone(); // CMD-7: the viewed Version's source, for `occ`
    if number != head {
        // The conversation stays live (replies / resolve work from any Version view); the
        // placement is the one the time-slice page shows.
        let v = st
            .store
            .version(&doc.id, number)?
            .ok_or_else(|| ApiError::not_found(format!("Document {} has no v{number}", doc.id)))?;
        if let Some(a) = &t.anchor {
            let mut at = a.at(number).unwrap_or_else(|| a.clone());
            if at.current.status.as_deref() == Some(annotary_sidecar::anchor::REFINED)
                && let Some(r) = at.current.range.as_deref().and_then(|r| Range::parse(r).ok())
                && let Some(md) = st.store.read_blob(&v.md_hash)?
            {
                at.current.exact = annotary_sidecar::range::extract(&String::from_utf8_lossy(&md), &r).ok();
            }
            t.anchor = Some(at);
        }
        md_hash = v.md_hash.clone();
    }
    let members = ui::members_by_handle(&st)?;
    let live = !doc.is_deleted();
    // CMD-23: the card the page swaps in renders its times as the page did
    let times = WebPrefs::load(&st, &auth, &headers)?.times();
    let viewer = Viewer {
        members: &members,
        me: &auth.0.handle,
        is_owner: auth.0.is_owner(),
        can_comment: live && auth.0.has_right(&doc.vault_name, Right::Comment),
        times: &times,
    };
    let mut item = [item_of(&t, &auth.0.member_id)];
    set_occ(&st, &mut item, &md_hash)?;
    let [item] = item;
    Ok(Json(json!({
        "html": thread_card(&item, &viewer, number),
        "item": item_json(&item),
        "hidden": t.is_fully_deleted(),
        "head": head,
        "version": number,
    })))
}
// ---- end ticket 11 ----

#[cfg(test)]
mod tests {
    use super::*;

    fn occ(md: &str, range: &str, exact: &str) -> Option<u32> {
        occ_before(md, &line_starts(md), &Range::parse(range).unwrap(), exact)
    }

    // CMD-7: the script picks the occurrence on the Thread's line by this count.
    #[test]
    fn occ_counts_earlier_starts_on_the_same_line_only() {
        let md = "# T\n\nthe plan says ship it, the plan says wait\n\nthe plan again\n";
        assert_eq!(occ(md, "3:0-3:8", "the plan"), Some(0));
        assert_eq!(occ(md, "3:23-3:31", "the plan"), Some(1));
        // An occurrence on another line never counts.
        assert_eq!(occ(md, "5:0-5:8", "the plan"), Some(0));
        // Code points, not bytes; overlapping starts count.
        assert_eq!(occ("é aa é aa\n", "1:5-1:9", "é aa"), Some(1));
        assert_eq!(occ("aaa\n", "1:1-1:3", "aa"), Some(1));
        // A line past the end or an empty quote: unknown.
        assert_eq!(occ(md, "9:0-9:1", "x"), None);
        assert_eq!(occ(md, "3:0-3:0", ""), None);
    }
}
