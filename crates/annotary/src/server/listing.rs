//! The Document list page (ticket 10): `GET /?name=&vault=&deleted=1&mentions_me=1&action=&since=&limit=`
//! — the same list as `annotary list` (same query: `docs::list_rows`; same sort — newest
//! activity first; same columns `id name vN action open updated`; same filters: Name substring,
//! Vault, deleted, mentions-me, Action id | `none`, since), plus the display-only H1 /
//! front-matter title under the Name (story 72). No unread markers. 50 rows by default with a
//! "more" link (story 70). Web auth via the `web_pages` router (loopback trust / login page).

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde::Deserialize;
use serde_json::Value;

use super::app::AppState;
use super::auth::Auth;
use super::docs::{LIST_LIMIT, ListQuery, list_rows};
use super::error::ApiError;
use super::prefs::WebPrefs;
use super::ui::{self, esc};
use super::vaults::readable_vaults;

#[derive(Debug, Deserialize, Default)]
pub struct ListPageQuery {
    pub name: Option<String>,
    pub vault: Option<String>,
    pub deleted: Option<String>,
    pub mentions_me: Option<String>,
    pub action: Option<String>,
    pub since: Option<String>,
    pub limit: Option<usize>,
}

fn flag(v: Option<&str>) -> bool {
    matches!(v.map(str::trim), Some("1") | Some("on") | Some("true") | Some("yes"))
}

pub async fn index(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Query(q): Query<ListPageQuery>,
) -> Response {
    match render(&st, &auth, &headers, &q) {
        Ok(r) => r,
        Err(e) => ui::error_page_for(&st, &auth, &headers, e),
    }
}

fn render(st: &AppState, auth: &Auth, headers: &HeaderMap, q: &ListPageQuery) -> Result<Response, ApiError> {
    // CMD-23: the Member's Web settings, read in the same response (the first paint is final).
    let prefs = WebPrefs::load(st, auth, headers)?;
    let times = prefs.times();
    let name = q.name.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let vault = q.vault.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let action = q.action.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let since_raw = q.since.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let since = match &since_raw {
        Some(s) => Some(
            crate::cli::docref::parse_since(s, jiff::Timestamp::now())
                .map_err(|e| ApiError::invalid(e.message.clone()))?,
        ),
        None => None,
    };
    let deleted = flag(q.deleted.as_deref());
    let mentions_me = flag(q.mentions_me.as_deref());
    let limit = q.limit.unwrap_or(LIST_LIMIT).max(1);
    let api_q = ListQuery {
        vault: vault.clone(),
        name: name.clone(),
        exact: None,
        deleted: deleted.then(|| "only".to_string()),
        since,
        limit: Some(limit),
        action: action.clone(),
        mentions_me: mentions_me.then(|| "1".to_string()),
    };
    let (rows, total, truncated) = list_rows(st, auth, &super::vaults::ProfileVaultHeader::none(), &api_q)?;
    let vaults = readable_vaults(st, auth)?;

    // ---- header: the shared top bar (ticket 12 — nav to the owner screens for owners) ----
    let header = ui::top_nav(st, auth, "/")?;

    // ---- filters ----
    let vault_opts: String = vaults
        .iter()
        .map(|v| {
            format!(
                "<option value=\"{}\"{}>{}</option>",
                esc(&v.name),
                if vault.as_deref() == Some(v.name.as_str()) { " selected" } else { "" },
                esc(&v.name)
            )
        })
        .collect();
    let filters = format!(
        "<form class=\"cd-filters\" id=\"cd-filters\" method=\"get\" action=\"/\">\
           <input type=\"search\" name=\"name\" placeholder=\"Name contains…\" value=\"{}\" aria-label=\"Name contains\">\
           <select name=\"vault\" aria-label=\"Vault\"><option value=\"\">every Vault</option>{vault_opts}</select>\
           <input type=\"text\" name=\"action\" placeholder=\"action id | none\" value=\"{}\" size=\"12\" aria-label=\"Action\">\
           <input type=\"text\" name=\"since\" placeholder=\"since: 2h, 3d, 2026-08-17\" value=\"{}\" size=\"16\" aria-label=\"Since\">\
           <label><input type=\"checkbox\" name=\"deleted\" value=\"1\"{}> deleted</label>\
           <label><input type=\"checkbox\" name=\"mentions_me\" value=\"1\"{}> mentions me</label>\
           <button type=\"submit\" class=\"cd-btn\">Filter</button> <a href=\"/\" class=\"cd-small\">clear</a>\
         </form>",
        esc(name.as_deref().unwrap_or("")),
        esc(action.as_deref().unwrap_or("")),
        esc(since_raw.as_deref().unwrap_or("")),
        if deleted { " checked" } else { "" },
        if mentions_me { " checked" } else { "" },
    );

    // ---- rows ----
    let mut body_rows = String::new();
    for d in &rows {
        let s = |k: &str| d.get(k).and_then(Value::as_str).unwrap_or("");
        let id = s("id");
        // `title` comes with the row (`docs::list_rows`, ticket 18).
        let title = d
            .get("title")
            .and_then(Value::as_str)
            .map(str::to_string)
            .filter(|t| t != s("name"))
            .map(|t| format!("<div class=\"cd-title\">{}</div>", esc(&t)))
            .unwrap_or_default();
        let action_cell = match d.get("action").filter(|a| !a.is_null()) {
            Some(a) => format!(
                "<span class=\"cd-chip {}\">{}</span>",
                ui::intent_class(a["intent"].as_str().unwrap_or("")),
                esc(a["id"].as_str().unwrap_or(""))
            ),
            None => "<span class=\"cd-muted\">-</span>".to_string(),
        };
        let deleted_tag = if d.get("deleted_at").is_some_and(|v| !v.is_null()) {
            "<span class=\"cd-deleted\">deleted</span>"
        } else {
            ""
        };
        let open = d["open_threads"].as_u64().unwrap_or(0);
        let mine = d["mentions_me"].as_u64().unwrap_or(0);
        let mine_tag =
            if mine > 0 { format!(" <span class=\"cd-mention cd-small\">{mine} @you</span>") } else { String::new() };
        body_rows.push_str(&format!(
            "<tr class=\"cd-docrow\" data-id=\"{id}\"><td class=\"cd-col-id cd-mono\"><a href=\"/d/{id}\">{id}</a></td>\
             <td><a class=\"cd-docname\" href=\"/d/{id}\">{name}</a>{deleted_tag}{title}</td>\
             <td class=\"cd-col-ver\">v{ver}</td><td class=\"cd-col-action\">{action_cell}</td>\
             <td class=\"cd-num cd-col-open\">{open}{mine_tag}</td><td class=\"cd-col-updated\">{updated}</td></tr>",
            id = esc(id),
            name = esc(s("name")),
            ver = d["version"].as_u64().unwrap_or(0),
            updated = times.tag(s("updated_at"), true),
        ));
    }
    let table = if rows.is_empty() {
        "<div class=\"cd-empty\" id=\"cd-empty\">No Documents match. <code>annotary push FILE.md</code> creates one.</div>".to_string()
    } else {
        format!(
            "<table class=\"cd-list\" id=\"cd-list\"><thead><tr><th class=\"cd-col-id\">ID</th><th>NAME</th><th class=\"cd-col-ver\">VER</th><th class=\"cd-col-action\">ACTION</th><th class=\"cd-num cd-col-open\">OPEN</th><th class=\"cd-col-updated\">UPDATED</th></tr></thead><tbody>{body_rows}</tbody></table>"
        )
    };
    let foot = if truncated {
        let mut qs: Vec<String> = Vec::new();
        if let Some(n) = &name {
            qs.push(format!("name={}", crate::client::qs(n)));
        }
        if let Some(v) = &vault {
            qs.push(format!("vault={}", crate::client::qs(v)));
        }
        if let Some(a) = &action {
            qs.push(format!("action={}", crate::client::qs(a)));
        }
        if let Some(s) = &since_raw {
            qs.push(format!("since={}", crate::client::qs(s)));
        }
        if deleted {
            qs.push("deleted=1".into());
        }
        if mentions_me {
            qs.push("mentions_me=1".into());
        }
        qs.push(format!("limit={}", limit + LIST_LIMIT));
        format!(
            "<p class=\"cd-listfoot\" id=\"cd-listfoot\">showing {} of {total} — <a id=\"cd-more\" href=\"/?{}\">more</a></p>",
            rows.len(),
            qs.join("&")
        )
    } else {
        format!(
            "<p class=\"cd-listfoot\" id=\"cd-listfoot\">{} {}</p>",
            rows.len(),
            if rows.len() == 1 { "Document" } else { "Documents" }
        )
    };
    let body = format!(
        "<div {} id=\"cd-app\">{header}<main class=\"cd-listmain\">{filters}{table}{foot}</main></div>",
        prefs.app_attrs("cd-listpage")
    );
    Ok(ui::page("Documents — annotary", &body))
}
