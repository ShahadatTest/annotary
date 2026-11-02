//! The owner screens (ticket 12), server-composed like every page. CMD-23 moved them under the
//! Settings screen's Server group (`settings_page.rs` draws the frame: the top bar and the
//! sidebar): `GET /settings/server/members` (the Members screen — handle, display name, role,
//! Grants summary, auth method as a hint, every Token with `last_used_at`; rename / disable, mint /
//! revoke; role and Grants editing behind an "Advanced" disclosure; "Add agent" = one handle →
//! Member + Membership + `*` all-rights Grant + Token shown once with the `ANNOTARY_TOKEN=…`
//! line), `GET /settings/server/vaults` (Vaults — create / rename / delete + the default Actions
//! editor), `GET /settings/server/general` (the local password — set / change / remove; "purge
//! deleted after N days" and housekeeping), and the two card fragments the script re-draws after a
//! mutation (`GET /members/{h}`, `GET /settings/vaults/{name}` → JSON `{html, …}`) so card markup
//! has one renderer. Ticket 18 adds "Pair a device" to every active Member's card (`POST
//! /api/v1/pairing`; the QR panel `.cd-paironce` is drawn by `admin.js` from the response's
//! `qr_svg`). `GET /members` 308s to `/settings/server/members` (`settings_page.rs`).
//!
//! | route | who | what |
//! |---|---|---|
//! | `GET /settings/server/members` | owner (web auth) | the Members screen; a Member gets the 403 page |
//! | `GET /members/{handle}` | owner | `{handle, html}` — one Member's card |
//! | `GET /settings/server/vaults` | owner (web auth) | the Vaults and their default Actions; a Member gets the 403 page |
//! | `GET /settings/server/general` | owner (web auth) | the local password, auto-purge, housekeeping; a Member gets the 403 page |
//! | `GET /settings/vaults/{name}` | owner | `{name, id, html}` — one Vault's card |
//!
//! Every mutation the screens make goes through `/api/v1` with the session cookie
//! (`POST members`, `…/rename|disable|role|grants`, `POST/DELETE tokens`, `PUT settings/password`,
//! `POST vaults`, `…/rename`, `DELETE vaults/{name}`, `PUT vaults/{name}/actions`,
//! `PUT settings/auto-purge`, `POST housekeeping`) — nothing page-specific was added to the API.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use serde_json::{Value, json};

use annotary_store::{ActionDecl, MemberView, Right, TokenRow, Vault, VaultCount, time};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;
use super::ui::{self, MemberBrief, esc, time_tag};

// ---------------------------------------------------------------- pages

/// `GET /settings/server/{page}` (`members`, `vaults`, `general`); any other page is the 404 page.
pub async fn server_page(
    State(st): State<AppState>,
    auth: Auth,
    headers: HeaderMap,
    Path(page): Path<String>,
) -> Response {
    let r = match page.as_str() {
        "members" => render_members(&st, &auth, &headers),
        "vaults" => render_vaults(&st, &auth, &headers),
        "general" => render_general(&st, &auth, &headers),
        _ => Err(ApiError::not_found(format!("no settings page server/{page}"))),
    };
    match r {
        Ok(r) => r,
        Err(e) => ui::error_page_for(&st, &auth, &headers, e),
    }
}

/// `GET /members/{handle}` → `{handle, html}` (owner; JSON errors — the script fetches it).
pub async fn member_fragment(
    State(st): State<AppState>,
    auth: Auth,
    Path(handle): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("the Members screen")?;
    let handle = handle.trim().trim_start_matches('@');
    let m = st.store.member_by_handle(handle)?.ok_or_else(|| ApiError::not_found(format!("no Member @{handle}")))?;
    let tokens = st.store.list_tokens(Some(&m.id))?;
    let vaults = st.store.list_vaults()?;
    Ok(Json(json!({ "handle": m.handle, "html": member_card(&m, &tokens, &vaults, &auth.0.handle) })))
}

/// `GET /settings/vaults/{name}` → `{name, id, html}` (owner).
pub async fn vault_fragment(
    State(st): State<AppState>,
    auth: Auth,
    Path(name): Path<String>,
) -> Result<Json<Value>, ApiError> {
    auth.require_owner("the Settings screen")?;
    let v = super::vaults::load_vault(&st, &name)?;
    let count = st.store.vault_count(&v.id)?;
    Ok(Json(json!({ "name": v.name, "id": v.id, "html": vault_card(&v, count) })))
}

/// An owner page in the Settings frame (the Server group of the sidebar is current).
#[allow(clippy::too_many_arguments)]
fn owner_page(
    st: &AppState,
    auth: &Auth,
    headers: &HeaderMap,
    what: &str,
    page: &str,
    title: &str,
    head_extra: &str,
    main: &str,
) -> Result<Response, ApiError> {
    use super::settings_page::{Active, Frame, render};
    auth.require_owner(what)?;
    let data = json!({ "group": format!("server/{page}") });
    let main = format!("<div class=\"cd-adminmain\">{main}</div>");
    render(
        st,
        auth,
        headers,
        Frame { active: Active::Server(page), index: false, title, head_extra, body: &main, admin: true, data },
    )
}

fn render_members(st: &AppState, auth: &Auth, headers: &HeaderMap) -> Result<Response, ApiError> {
    auth.require_owner("the Members screen")?;
    let members = st.store.list_members()?;
    let vaults = st.store.list_vaults()?;
    let mut by_member: HashMap<String, Vec<TokenRow>> = HashMap::new();
    for t in st.store.list_tokens(None)? {
        by_member.entry(t.info.member_id.clone()).or_default().push(t);
    }
    let none: Vec<TokenRow> = Vec::new();
    let cards: String = members
        .iter()
        .map(|m| member_card(m, by_member.get(&m.id).unwrap_or(&none), &vaults, &auth.0.handle))
        .collect();
    let active = members.iter().filter(|m| !m.is_disabled()).count();
    let head = format!(
        "<span class=\"cd-muted cd-small\" id=\"cd-memcount\">{active} active</span><span class=\"cd-grow\"></span>\
           <button type=\"button\" class=\"cd-btn cd-primary\" id=\"cd-addagent\">Add agent</button>"
    );
    let main = format!(
        "<form class=\"cd-addform cd-box\" id=\"cd-addform\" hidden autocomplete=\"off\">\
           <div class=\"cd-formrow\"><label>Handle <input type=\"text\" name=\"handle\" id=\"cd-add-handle\" placeholder=\"scribe\" required spellcheck=\"false\" autocapitalize=\"off\"></label>\
           <label>Name <input type=\"text\" name=\"name\" id=\"cd-add-name\" placeholder=\"optional\"></label>\
           <button type=\"submit\" class=\"cd-btn cd-primary\">Create</button> <button type=\"button\" class=\"cd-btn\" data-act=\"cancel\">Cancel</button></div>\
           <p class=\"cd-muted cd-small\">One step: the Member, its Membership, a <code>*</code> Grant with every Right (read, comment, action, write — Vaults created later included) and a Token shown once. Narrow the Grants under <i>Advanced</i> when you need to.</p>\
           <div class=\"cd-err\" hidden></div></form>\
         <div id=\"cd-addresult\"></div>\
         <div class=\"cd-memlist\" id=\"cd-memlist\">{cards}</div>"
    );
    owner_page(st, auth, headers, "the Members screen", "members", "Members", &head, &main)
}

fn render_vaults(st: &AppState, auth: &Auth, headers: &HeaderMap) -> Result<Response, ApiError> {
    auth.require_owner("the Vaults screen")?;
    let vaults = st.store.list_vaults()?;
    let mut vcards = String::new();
    for v in &vaults {
        vcards.push_str(&vault_card(v, st.store.vault_count(&v.id)?));
    }
    let main = format!(
        "<section class=\"cd-set\" id=\"cd-set-vaults\">\
           <p class=\"cd-muted cd-small\">Documents live in one Vault; Grants are given per Vault (or <code>*</code>). The default Actions apply to Documents without their own <code>actions.available</code> declaration — and only to future Actions.</p>\
           <div id=\"cd-vaults\">{vcards}</div>\
           <form id=\"cd-vaultnew\" class=\"cd-formrow cd-box\" autocomplete=\"off\"><label>New Vault <input type=\"text\" name=\"name\" id=\"cd-vault-name\" placeholder=\"name\" required spellcheck=\"false\"></label>\
             <button type=\"submit\" class=\"cd-btn cd-primary\">Create Vault</button><div class=\"cd-err\" hidden></div></form>\
         </section>"
    );
    owner_page(st, auth, headers, "the Vaults screen", "vaults", "Vaults", "", &main)
}

fn render_general(st: &AppState, auth: &Auth, headers: &HeaderMap) -> Result<Response, ApiError> {
    auth.require_owner("the Server settings screen")?;
    let password_set = st.store.password_set()?;
    let mine = st.store.member_by_id(&auth.0.member_id)?.is_some_and(|m| m.has_password);
    let holders: Vec<String> = st
        .store
        .list_members()?
        .iter()
        .filter(|m| m.has_password && !m.is_disabled())
        .map(|m| format!("@{}", m.handle))
        .collect();
    let days = st.store.auto_purge_days()?;

    let pw_state = if !password_set {
        format!(
            "No local password is set — a browser on this machine is signed in as <b>@{}</b> automatically. Set one and the login page appears on the next request without a session.",
            esc(&st.store.founding_owner()?.map(|m| m.handle).unwrap_or_default())
        )
    } else if mine {
        "A local password is set (yours). Browsers sign in at <code>/login</code>; <code>annotary login</code> turns it into a Token for the CLI.".to_string()
    } else {
        format!(
            "A local password is set by {}. Setting one here sets <b>your</b> password (@{}); removing clears yours.",
            esc(&holders.join(", ")),
            esc(&auth.0.handle)
        )
    };
    let remove_btn = if mine {
        "<button type=\"button\" class=\"cd-btn\" data-act=\"remove-password\" id=\"cd-pw-remove\">Remove password</button>"
    } else {
        ""
    };
    let set_label = if mine { "Change password" } else { "Set password" };
    let password = format!(
        "<section class=\"cd-set\" id=\"cd-set-password\" data-password-set=\"{password_set}\" data-mine=\"{mine}\">\
           <h2>Local password</h2><p class=\"cd-muted\" id=\"cd-pw-state\">{pw_state}</p>\
           <form id=\"cd-pwform\" class=\"cd-formrow\" autocomplete=\"off\">\
             <label>New password <input type=\"password\" name=\"password\" id=\"cd-pw-new\" autocomplete=\"new-password\" required minlength=\"1\"></label>\
             <label>Again <input type=\"password\" name=\"confirm\" id=\"cd-pw-again\" autocomplete=\"new-password\" required></label>\
             <button type=\"submit\" class=\"cd-btn cd-primary\" id=\"cd-pw-set\">{set_label}</button> {remove_btn}\
             <span class=\"cd-ok\" hidden></span></form>\
           <div class=\"cd-err\" hidden></div>\
         </section>"
    );

    let purge = format!(
        "<section class=\"cd-set\" id=\"cd-set-purge\" data-days=\"{}\"><h2>Deleted Documents</h2>\
           <p class=\"cd-muted cd-small\">Deleted Documents stay (restorable) until purged. Off by default: with this on, housekeeping purges a deleted Document — its Versions, Threads, Actions and the blobs nothing else references — once it has been deleted for N days.</p>\
           <form id=\"cd-purgeform\" class=\"cd-formrow\">\
             <label class=\"cd-chk\"><input type=\"checkbox\" name=\"on\" id=\"cd-purge-on\"{}> purge deleted Documents after</label>\
             <input type=\"number\" name=\"days\" id=\"cd-purge-days\" min=\"1\" max=\"{}\" value=\"{}\" size=\"4\"> days\
             <button type=\"submit\" class=\"cd-btn cd-primary\" id=\"cd-purge-save\">Save</button><span class=\"cd-ok\" hidden></span></form>\
           <div class=\"cd-err\" hidden></div>\
           <p class=\"cd-muted cd-small\">Housekeeping (auto-purge, blob GC, stale renders) runs hourly in the server; <button type=\"button\" class=\"cd-link\" id=\"cd-runhk\">run it now</button><span id=\"cd-hkreport\" class=\"cd-small\"></span></p>\
         </section>",
        days.unwrap_or(0),
        if days.is_some() { " checked" } else { "" },
        annotary_store::AUTO_PURGE_MAX_DAYS,
        days.unwrap_or(30),
    );

    let main = format!("{password}{purge}");
    owner_page(st, auth, headers, "the Server settings screen", "general", "Server settings", "", &main)
}

// ---------------------------------------------------------------- cards

fn rights_words(rights: &[Right]) -> String {
    rights.iter().map(|r| r.as_str()).collect::<Vec<_>>().join(", ")
}

/// `every Vault: read, comment, action, write` · `notes: read, comment` — or `no access`.
fn grants_summary(m: &MemberView) -> String {
    if m.grants.is_empty() {
        return "<span class=\"cd-muted\">no access</span>".to_string();
    }
    m.grants
        .iter()
        .map(|g| {
            let name = if g.vault == "*" { "every Vault".to_string() } else { esc(&g.vault) };
            format!("<span class=\"cd-grant\"><b>{name}</b>: {}</span>", rights_words(&g.rights))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn token_row(t: &TokenRow, now: &str) -> String {
    let status = t.status(now);
    let name = t.info.name.as_deref().filter(|n| !n.is_empty()).map(esc).unwrap_or_else(|| "-".to_string());
    let last = match t.info.last_used_at.as_deref() {
        Some(at) => time_tag(at, true),
        None => "<span class=\"cd-muted\">never</span>".to_string(),
    };
    let expires = match (status, t.info.expires_at.as_deref()) {
        ("expired", Some(at)) => format!("<span class=\"cd-warn-text\">expired</span> {}", time_tag(at, false)),
        (_, Some(at)) => time_tag(at, false),
        _ => "<span class=\"cd-muted\">-</span>".to_string(),
    };
    format!(
        "<tr class=\"cd-tok\" data-token=\"{id}\" data-status=\"{status}\"><td class=\"cd-tokname\">{name}</td><td>{created}</td><td class=\"cd-lastused\">{last}</td><td>{expires}</td><td class=\"cd-mono cd-tokid\">{id}</td>\
         <td class=\"cd-tokact\"><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"revoke\" data-token=\"{id}\">Revoke</button></td></tr>",
        id = esc(&t.info.id),
        created = time_tag(&t.info.created_at, false),
    )
}

/// One Member's card (the Members screen and the `/members/{handle}` fragment share it).
pub fn member_card(m: &MemberView, tokens: &[TokenRow], vaults: &[Vault], me: &str) -> String {
    let now = time::now();
    let brief = MemberBrief::from_view(m);
    let disabled = m.is_disabled();
    let auth_hint = if m.has_password { "signs in with the local password" } else { "authenticates with a Token" };
    let chips = format!(
        "<span class=\"cd-chip cd-role cd-role-{role}\">{role}</span>{}",
        if disabled { "<span class=\"cd-chip cd-warn cd-disabled\">disabled</span>" } else { "" },
        role = m.role.as_str()
    );
    let tools = if disabled {
        "<button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"rename\">Rename</button>".to_string()
    } else {
        "<button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"rename\">Rename</button> <button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"disable\">Disable</button>".to_string()
    };
    let live: Vec<&TokenRow> = tokens.iter().filter(|t| t.info.revoked_at.is_none()).collect();
    let revoked = tokens.len() - live.len();
    let rows: String = live.iter().map(|t| token_row(t, &now)).collect();
    let table = if live.is_empty() {
        "<p class=\"cd-muted cd-small cd-notokens\">no Tokens</p>".to_string()
    } else {
        format!(
            "<table class=\"cd-toktable\"><thead><tr><th>NAME</th><th>CREATED</th><th>LAST USED</th><th>EXPIRES</th><th>ID</th><th></th></tr></thead><tbody>{rows}</tbody></table>"
        )
    };
    let revoked_note = if revoked > 0 {
        format!(" <span class=\"cd-muted cd-small\">· {revoked} revoked</span>")
    } else {
        String::new()
    };
    // ticket 18: "Pair a device" next to "Mint Token" (`POST /api/v1/pairing`, admin.js).
    let mint = if disabled {
        String::new()
    } else {
        "<button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"mint\">Mint Token</button> <button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"pair\">Pair a device</button>".to_string()
    };
    let disabled_note = if disabled {
        "<p class=\"cd-muted cd-small\">Disabled — history and attribution kept; its Tokens and sessions are refused; it cannot be granted, promoted or given Tokens (the handle stays taken).</p>"
    } else {
        ""
    };
    // ---- advanced: role + Grants ----
    let role_sel = format!(
        "<select data-field=\"role\"><option value=\"owner\"{}>owner</option><option value=\"member\"{}>member</option></select>",
        if m.role.as_str() == "owner" { " selected" } else { "" },
        if m.role.as_str() == "member" { " selected" } else { "" }
    );
    let mut grant_rows = String::new();
    let mut names: Vec<(String, String)> =
        vec![("*".to_string(), "every Vault <span class=\"cd-muted\">(incl. future ones)</span>".to_string())];
    for v in vaults {
        names.push((v.name.clone(), esc(&v.name)));
    }
    for (vault, label) in names {
        let have = m.grants.iter().find(|g| g.vault == vault);
        let cells: String = Right::ALL
            .iter()
            .map(|r| {
                let on = have.is_some_and(|g| g.rights.contains(r));
                format!(
                    "<td><label class=\"cd-chk\"><input type=\"checkbox\" data-right=\"{r}\"{}> {r}</label></td>",
                    if on { " checked" } else { "" },
                    r = r.as_str()
                )
            })
            .collect();
        grant_rows.push_str(&format!(
            "<tr class=\"cd-grantrow\" data-vault=\"{}\"><td class=\"cd-grantvault\">{label}</td>{cells}</tr>",
            esc(&vault)
        ));
    }
    let advanced = if disabled {
        String::new()
    } else {
        format!(
            "<details class=\"cd-advanced\"><summary>Advanced <span class=\"cd-muted\">role · Grants</span></summary>\
               <div class=\"cd-advrow\"><label>Role {role_sel}</label> <button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"role\">Save role</button> <span class=\"cd-muted cd-small\">owners manage Members, Vaults, default Actions, purge and settings; the last owner cannot be demoted</span></div>\
               <div class=\"cd-grantsed\"><table class=\"cd-granttable\"><tbody>{grant_rows}</tbody></table>\
               <div class=\"cd-advrow\"><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"grants\">Save Grants</button> <span class=\"cd-muted cd-small\">a Vault's own row overrides <code>*</code> for that Vault; <code>read</code> is implied by any other Right; no box = no Grant</span></div></div>\
             </details>"
        )
    };
    format!(
        "<article class=\"cd-mem{}{}\" id=\"cd-mem-{h}\" data-handle=\"{h}\" data-id=\"{id}\" data-role=\"{role}\">\
           <div class=\"cd-memhead\"><span class=\"cd-memwho\">{who}<span class=\"cd-handle cd-mono\">@{h}</span>{chips}</span><span class=\"cd-grow\"></span><span class=\"cd-memtools\">{tools}</span></div>\
           <div class=\"cd-memmeta cd-muted cd-small\"><span class=\"cd-authhint\">{auth_hint}</span> · Member since {since}</div>\
           <div class=\"cd-inline\" data-slot=\"rename\" hidden></div>\
           {disabled_note}\
           <div class=\"cd-memgrants\"><span class=\"cd-k\">Access</span> <span class=\"cd-grants-summary\">{grants}</span></div>\
           <div class=\"cd-memtokens\"><div class=\"cd-memtokhead\"><span class=\"cd-k\">Tokens</span>{revoked_note}<span class=\"cd-grow\"></span>{mint}</div>\
             <div class=\"cd-inline\" data-slot=\"mint\" hidden></div><div class=\"cd-tokonce-slot\"></div><div class=\"cd-paironce-slot\"></div>{table}</div>\
           {advanced}\
           <div class=\"cd-err cd-memerr\" hidden></div>\
         </article>",
        if disabled { " cd-mem-disabled" } else { "" },
        if m.handle == me { " cd-mem-me" } else { "" },
        h = esc(&m.handle),
        id = esc(&m.id),
        role = m.role.as_str(),
        who = brief.html(),
        since = time_tag(&m.created_at, false),
        grants = grants_summary(m),
    )
}

fn action_row(d: &ActionDecl) -> String {
    let opts: String = annotary_store::actions::INTENTS
        .iter()
        .map(|i| format!("<option value=\"{i}\"{}>{i}</option>", if *i == d.intent { " selected" } else { "" }))
        .collect();
    format!(
        "<tr class=\"cd-actrow\"><td><input type=\"text\" data-f=\"id\" value=\"{}\" placeholder=\"id (kebab-case)\" spellcheck=\"false\"></td>\
         <td><input type=\"text\" data-f=\"label\" value=\"{}\" placeholder=\"label\"></td>\
         <td><select data-f=\"intent\">{opts}</select></td>\
         <td class=\"cd-actrowtools\"><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"up\" title=\"move up\">\u{2191}</button><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"down\" title=\"move down\">\u{2193}</button><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"remove\" title=\"remove\">\u{00D7}</button></td></tr>",
        esc(&d.id),
        esc(&d.label)
    )
}

/// One Vault's card (the Settings screen and the `/settings/vaults/{name}` fragment share it):
/// name, counts, rename / delete, and the default Actions editor (`{id, label, intent}` rows —
/// add / remove / reorder, saved whole with `PUT /api/v1/vaults/{name}/actions`).
pub fn vault_card(v: &Vault, count: VaultCount) -> String {
    let decls: Vec<ActionDecl> = serde_json::from_value(v.default_actions.clone()).unwrap_or_default();
    let rows: String = decls.iter().map(action_row).collect();
    let docs = match (count.documents, count.deleted) {
        (0, _) => "empty".to_string(),
        (1, 0) => "1 Document".to_string(),
        (n, 0) => format!("{n} Documents"),
        (n, d) => format!("{n} Document{} ({d} deleted)", if n == 1 { "" } else { "s" }),
    };
    format!(
        "<article class=\"cd-vault cd-box\" id=\"cd-vault-{id}\" data-vault=\"{name}\" data-id=\"{id}\" data-documents=\"{docs_n}\">\
           <div class=\"cd-vaulthead\"><b class=\"cd-vaultname\">{name}</b><span class=\"cd-muted cd-small cd-vaultmeta\">{docs} · created {created}</span><span class=\"cd-grow\"></span>\
             <span class=\"cd-vaulttools\"><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"rename\">Rename</button> <button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"delete\">Delete</button></span></div>\
           <div class=\"cd-inline\" data-slot=\"rename\" hidden></div>\
           <div class=\"cd-vaultacts\"><div class=\"cd-k\">Default Actions <span class=\"cd-muted\">— the buttons on Documents without their own declaration; an empty list means no buttons</span></div>\
             <table class=\"cd-actsed\"><thead><tr><th>ID</th><th>LABEL</th><th>INTENT</th><th></th></tr></thead><tbody>{rows}</tbody></table>\
             <div class=\"cd-advrow\"><button type=\"button\" class=\"cd-btn cd-sm\" data-act=\"addrow\">Add Action</button> <button type=\"button\" class=\"cd-btn cd-sm cd-primary\" data-act=\"save-actions\">Save Actions</button> <span class=\"cd-ok\" hidden>saved</span></div></div>\
           <div class=\"cd-err\" hidden></div>\
         </article>",
        id = esc(&v.id),
        name = esc(&v.name),
        docs_n = count.documents,
        created = time_tag(&v.created_at, false),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use annotary_store::{Grant, Role};

    fn member(handle: &str, grants: Vec<Grant>) -> MemberView {
        MemberView {
            id: "01J".into(),
            handle: handle.into(),
            display_name: "Linter".into(),
            role: Role::Member,
            grants,
            created_at: "2026-08-14T09:12:04.123Z".into(),
            disabled_at: None,
            has_password: false,
        }
    }

    #[test]
    fn member_card_shows_handle_name_role_grants_hint_and_tokens() {
        let m = member("linter", vec![Grant::all(), Grant { vault: "notes".into(), rights: vec![Right::Comment] }]);
        let html = member_card(&m, &[], &[], "rowan");
        assert!(html.contains("data-handle=\"linter\""));
        assert!(html.contains("<b class=\"cd-member\" title=\"@linter\">Linter</b>"));
        assert!(html.contains("cd-auth-token"));
        assert!(html.contains("authenticates with a Token"));
        assert!(html.contains("<span class=\"cd-chip cd-role cd-role-member\">member</span>"));
        assert!(html.contains("<b>every Vault</b>: read, comment, action, write"));
        assert!(html.contains("<b>notes</b>: comment"));
        assert!(html.contains("no Tokens"));
        assert!(html.contains("<details class=\"cd-advanced\">"));
        assert!(html.contains("data-act=\"pair\">Pair a device</button>")); // ticket 18
        assert!(!html.contains("bot"));
        // No Grants at all reads as "no access"; a disabled Member has no Advanced / Disable / Mint.
        let mut d = member("old", vec![]);
        d.disabled_at = Some("2026-08-15T00:00:00.000Z".into());
        let html = member_card(&d, &[], &[], "rowan");
        assert!(html.contains("no access"));
        assert!(html.contains("cd-disabled"));
        assert!(
            !html.contains("cd-advanced")
                && !html.contains("data-act=\"disable\"")
                && !html.contains("data-act=\"mint\"")
                && !html.contains("data-act=\"pair\"")
        );
    }

    #[test]
    fn vault_card_has_action_rows_and_counts() {
        let v = Vault {
            id: "V1".into(),
            name: "notes".into(),
            default_actions: annotary_store::default_vault_actions(),
            created_at: "2026-08-14T09:12:04.123Z".into(),
        };
        let html = vault_card(&v, VaultCount { documents: 3, deleted: 1 });
        assert!(html.contains("data-vault=\"notes\""));
        assert!(html.contains("3 Documents (1 deleted)"));
        assert!(html.contains("value=\"approve\"") && html.contains("value=\"send-back\""));
        assert!(html.contains("<option value=\"positive\" selected>"));
        assert!(vault_card(&v, VaultCount::default()).contains("empty"));
    }
}
