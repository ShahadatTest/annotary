//! The Settings screen (CMD-23), server-composed for every Member like every page: a sidebar of
//! the catalog's groups (and, for owners, the Server group with the owner pages), one URL per
//! group, the "Applying to" scope picker on groups with device settings, and every control in its
//! current state in the first response (from `Store::settings_matrix` / `resolved_settings`), so
//! the first paint is final. `ui/settings.js` only wires the interactions (saving, the scope and
//! copy menus, the chain popover, search, the live feed) and updates the controls after a change.
//!
//! | route | who | what |
//! |---|---|---|
//! | `GET /settings` | every Member | Profile at 850px and wider; the group list when narrower (one page, container queries) |
//! | `GET /settings/{group}` | every Member | one catalog group (`profile`, `devices`, `appearance`, `threads`, `documents`, `keyboard`, `date-and-time`); an unknown slug is the 404 page |
//! | `GET /settings/server/{members,vaults,general}` | owner | the owner pages (`admin.rs`) in the same frame; a Member gets the 403 page |
//! | `GET /members` | — | 308 to `/settings/server/members` |
//!
//! Every change the screen makes goes through `/api/v1/me/…` (`me_settings.rs`) with the session
//! cookie; a browser is always the Member's one Web Device. `ui/settings.css` / `settings.js` are
//! inlined only on these pages (with `admin.css` / `admin.js` too on the Server pages).

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use serde_json::{Value, json};

use annotary_store::settings::{self, GROUPS, PRESETS, SettingDef, SettingKind, ValueType, WEB_DEVICE};
use annotary_store::{DeviceView, MatrixEntry};

use super::app::AppState;
use super::auth::Auth;
use super::error::ApiError;
use super::prefs::WebPrefs;
use super::ui::{self, esc};

pub const SETTINGS_CSS: &str = include_str!("ui/settings.css");
pub const SETTINGS_JS: &str = include_str!("ui/settings.js");

/// The owner pages of the Server group: slug, sidebar label, words the search also matches.
pub const SERVER_PAGES: &[(&str, &str, &str)] = &[
    ("members", "Members", "Tokens agents Grants roles pair a device"),
    ("vaults", "Vaults", "default Actions rename delete"),
    ("general", "Server settings", "local password sign in deleted Documents purge housekeeping"),
];

// ---------------------------------------------------------------- icons (round 3's board)

fn svg(d: &str, size: u32, sw: &str) -> String {
    format!(
        "<svg width=\"{size}\" height=\"{size}\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"{sw}\" stroke-linecap=\"round\" stroke-linejoin=\"round\" aria-hidden=\"true\" focusable=\"false\">{d}</svg>"
    )
}

/// The path data of a kind icon (`web`, `iphone`, `ipad`, `cli`).
pub fn kind_path(kind: &str) -> &'static str {
    match kind {
        "iphone" => "<rect x=\"7\" y=\"2.5\" width=\"10\" height=\"19\" rx=\"2.5\"/><path d=\"M11 18.5h2\"/>",
        "ipad" => "<rect x=\"4\" y=\"2.5\" width=\"16\" height=\"19\" rx=\"2.5\"/><path d=\"M11 18.5h2\"/>",
        "cli" => "<rect x=\"3\" y=\"4\" width=\"18\" height=\"16\" rx=\"2\"/><path d=\"M7 9l3 3-3 3M12 15h5\"/>",
        _ => "<rect x=\"3\" y=\"4\" width=\"18\" height=\"12\" rx=\"2\"/><path d=\"M8 20h8M12 16v4\"/>",
    }
}
const CHEV: &str = "<path d=\"M6 9l6 6 6-6\"/>";
const RIGHT: &str = "<path d=\"M9 6l6 6-6 6\"/>";
const SEARCH: &str = "<circle cx=\"11\" cy=\"11\" r=\"6.5\"/><path d=\"M16 16l4.5 4.5\"/>";
const LAYERS: &str = "<path d=\"M12 3l9 5-9 5-9-5z\"/><path d=\"M3 13l9 5 9-5\"/>";

// ---------------------------------------------------------------- handlers

/// `GET /settings`: Profile when wide, the group list when narrow.
pub async fn index(State(st): State<AppState>, auth: Auth, headers: HeaderMap) -> Response {
    match render_group(&st, &auth, &headers, "profile", true) {
        Ok(r) => r,
        Err(e) => ui::error_page_for(&st, &auth, &headers, e),
    }
}

/// `GET /settings/{group}`.
pub async fn group(State(st): State<AppState>, auth: Auth, headers: HeaderMap, Path(slug): Path<String>) -> Response {
    if !GROUPS.iter().any(|g| g.slug == slug) {
        return ui::error_page_for(&st, &auth, &headers, ApiError::not_found(format!("no settings group {slug:?}")));
    }
    match render_group(&st, &auth, &headers, &slug, false) {
        Ok(r) => r,
        Err(e) => ui::error_page_for(&st, &auth, &headers, e),
    }
}

/// `GET /members` → 308 `/settings/server/members` (the owner page moved under Settings).
pub async fn members_redirect() -> Response {
    Redirect::permanent("/settings/server/members").into_response()
}

// ---------------------------------------------------------------- the frame

/// Which sidebar entry is current.
#[derive(Clone, Copy, Debug)]
pub enum Active<'a> {
    Group(&'a str),
    Server(&'a str),
}

/// One Settings page: the sidebar, the title row and the content.
pub struct Frame<'a> {
    pub active: Active<'a>,
    /// `GET /settings`: the group list stands alone when narrow.
    pub index: bool,
    /// Plain text (escaped here): the `<title>` and the page's heading.
    pub title: &'a str,
    /// Trusted HTML after the heading, on its row (the Members page's count and Add agent).
    pub head_extra: &'a str,
    /// Trusted HTML.
    pub body: &'a str,
    /// The Server pages carry `admin.css` / `admin.js` too.
    pub admin: bool,
    /// What `settings.js` reads (`#cd-set-data`); `search` is added here.
    pub data: Value,
}

/// JSON safe inside `<script type="application/json">`.
fn json_script(v: &Value) -> String {
    v.to_string().replace('<', "\\u003c")
}

/// The page around a Settings group or a Server page (`admin.rs` uses it for the owner pages).
pub fn render(st: &AppState, auth: &Auth, headers: &HeaderMap, f: Frame<'_>) -> Result<Response, ApiError> {
    let prefs = WebPrefs::load(st, auth, headers)?;
    let attrs = prefs.app_attrs(if f.admin { "cd-settingspage cd-adminpage" } else { "cd-settingspage" });
    let header = ui::top_nav(st, auth, "/settings")?;
    let side = sidebar(auth, f.active);
    let mode = if f.index { "cd-set-index" } else { "cd-set-grouppage" };
    let mut data = f.data;
    data["search"] = search_index(auth);
    let body = format!(
        "<div {attrs} id=\"cd-app\">{header}<div class=\"cd-set-layout {mode}\">{side}\
         <main class=\"cd-set-main\" id=\"cd-set-main\"><div class=\"cd-set-content\">\
         <a class=\"cd-set-back\" href=\"/settings\">\u{2039} Settings</a>\
         <div class=\"cd-set-head\"><h1 class=\"cd-set-title\">{title}</h1>{extra}</div>{body}</div></main></div>\
         <script type=\"application/json\" id=\"cd-set-data\">{json}</script></div>",
        title = esc(f.title),
        extra = f.head_extra,
        body = f.body,
        json = json_script(&data),
    );
    let (css, js) = if f.admin {
        (format!("{SETTINGS_CSS}{}", ui::ADMIN_CSS), format!("{SETTINGS_JS}\n{}", ui::ADMIN_JS))
    } else {
        (SETTINGS_CSS.to_string(), SETTINGS_JS.to_string())
    };
    Ok(ui::page_ex(&format!("{} — annotary", f.title), &body, &css, &js))
}

fn sidebar(auth: &Auth, active: Active<'_>) -> String {
    let item = |href: &str, key: &str, label: &str, on: bool| {
        format!(
            "<li><a class=\"cd-set-gl{}\" href=\"{href}\" data-group=\"{key}\"{}><span>{}</span>{}</a></li>",
            if on { " cd-current" } else { "" },
            if on { " aria-current=\"page\"" } else { "" },
            esc(label),
            svg(RIGHT, 16, "2").replace("<svg ", "<svg class=\"cd-set-chev\" "),
        )
    };
    let groups: String = GROUPS
        .iter()
        .map(|g| {
            item(&format!("/settings/{}", g.slug), g.slug, g.label, matches!(active, Active::Group(s) if s == g.slug))
        })
        .collect();
    let server = if auth.0.is_owner() {
        let items: String = SERVER_PAGES
            .iter()
            .map(|(slug, label, _)| {
                item(
                    &format!("/settings/server/{slug}"),
                    &format!("server/{slug}"),
                    label,
                    matches!(active, Active::Server(s) if s == *slug),
                )
            })
            .collect();
        format!(
            "<div class=\"cd-set-sidehead\" id=\"cd-set-serverhead\">Server</div><ul class=\"cd-set-groups cd-set-server\" aria-labelledby=\"cd-set-serverhead\">{items}</ul>"
        )
    } else {
        String::new()
    };
    format!(
        "<nav class=\"cd-set-side\" id=\"cd-set-side\" aria-label=\"Settings\"><h1 class=\"cd-set-listtitle\">Settings</h1>\
         <div class=\"cd-set-search\">{}<input type=\"search\" id=\"cd-set-q\" placeholder=\"Search settings\" aria-label=\"Search settings\" autocomplete=\"off\" spellcheck=\"false\"></div>\
         <ul class=\"cd-set-groups\" id=\"cd-set-groups\">{groups}</ul>{server}\
         <p class=\"cd-set-nomatch\" id=\"cd-set-nomatch\" hidden>No settings match.</p></nav>",
        svg(SEARCH, 14, "2"),
    )
}

/// What the search matches, for every page (no request): each group's label, each setting's
/// label and help line, the Profile and Devices rows that are not catalog settings, the Server
/// pages for owners.
fn search_index(auth: &Auth) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for g in GROUPS {
        out.push(json!({ "group": g.slug, "text": g.label }));
    }
    for d in settings::CATALOG {
        out.push(json!({ "group": d.group, "key": d.key, "text": format!("{} {}", d.label, d.help) }));
    }
    out.push(json!({ "group": "profile", "text": "Handle Role" }));
    out.push(json!({ "group": "devices", "text": "Device names last seen Rename Forget" }));
    if auth.0.is_owner() {
        for (slug, label, words) in SERVER_PAGES {
            out.push(json!({ "group": format!("server/{slug}"), "text": format!("{label} {words}") }));
        }
    }
    Value::Array(out)
}

// ---------------------------------------------------------------- the groups

/// What a group page reads from the store, once.
struct Ctx {
    current: String,
    devices: Vec<DeviceView>,
    matrix: BTreeMap<&'static str, MatrixEntry>,
    user: BTreeMap<&'static str, Value>,
    vaults: Vec<(String, String)>,
}

fn load_ctx(st: &AppState, auth: &Auth) -> Result<Ctx, ApiError> {
    // A browser is always the Member's Web Device (made on first use).
    let web = st.store.ensure_device(&auth.0.member_id, WEB_DEVICE, None)?;
    let (devices, entries) = st.store.settings_matrix(&auth.0.member_id)?;
    let matrix = entries.into_iter().map(|e| (e.def.key, e)).collect();
    let user = st
        .store
        .resolved_settings(&auth.0.member_id, Some(&web.id))?
        .into_iter()
        .filter(|r| r.def.kind == SettingKind::User)
        .map(|r| (r.def.key, r.value))
        .collect();
    let vaults = super::vaults::readable_vaults(st, auth)?.into_iter().map(|v| (v.id, v.name)).collect();
    Ok(Ctx { current: web.id, devices, matrix, user, vaults })
}

/// The Devices a device setting counts (non-forgotten, of a kind it applies to) — `ids` narrows
/// to the chosen ones (`None` = All devices).
fn counted<'a>(def: &SettingDef, devices: &'a [DeviceView], ids: Option<&[&str]>) -> Vec<&'a DeviceView> {
    devices.iter().filter(|d| def.applies_to(&d.kind) && ids.is_none_or(|ids| ids.contains(&d.id.as_str()))).collect()
}

/// What a device setting's control shows for All devices: the value every counted Device
/// resolves to, or `Mixed` (`None`, true) when they disagree.
fn shown(entry: &MatrixEntry, devices: &[DeviceView]) -> (Value, bool) {
    let mut seen: Vec<&Value> = Vec::new();
    for d in counted(entry.def, devices, None) {
        if let Some((v, _)) = entry.resolved.get(&d.id)
            && !seen.contains(&v)
        {
            seen.push(v);
        }
    }
    match seen.as_slice() {
        [] => (entry.all.as_ref().and_then(|a| a.value.clone()).unwrap_or_else(|| entry.def.default_value()), false),
        [one] => ((*one).clone(), false),
        _ => (entry.all.as_ref().and_then(|a| a.value.clone()).unwrap_or_else(|| entry.def.default_value()), true),
    }
}

fn host_of(st: &AppState) -> String {
    let url = st.store.public_url().ok().flatten().unwrap_or_else(|| st.url.clone());
    let rest = url.split_once("://").map(|(_, r)| r).unwrap_or(&url);
    rest.split('/').next().unwrap_or(rest).to_string()
}

fn render_group(
    st: &AppState,
    auth: &Auth,
    headers: &HeaderMap,
    slug: &str,
    index: bool,
) -> Result<Response, ApiError> {
    let ctx = load_ctx(st, auth)?;
    let label = GROUPS.iter().find(|g| g.slug == slug).map(|g| g.label).unwrap_or("Settings");
    let defs: Vec<&SettingDef> = {
        let mut v: Vec<&SettingDef> = settings::CATALOG.iter().filter(|d| d.group == slug).collect();
        v.sort_by_key(|d| d.order);
        v
    };
    let has_device = defs.iter().any(|d| d.kind == SettingKind::Device);
    let yours = format!(
        "<p class=\"cd-set-yours\">These settings are yours on <b>{}</b>, on every device.</p>",
        esc(&host_of(st))
    );
    let body = match slug {
        "profile" => profile_body(st, auth, &yours)?,
        "devices" => devices_body(&ctx),
        _ if has_device => {
            let mut b = scope_line();
            for d in defs.iter().filter(|d| d.kind == SettingKind::Device) {
                b.push_str(&row(d, &ctx));
            }
            let users: String = defs.iter().filter(|d| d.kind == SettingKind::User).map(|d| row(d, &ctx)).collect();
            if !users.is_empty() {
                b.push_str(&format!(
                    "<section class=\"cd-set-foryou\"><h2 class=\"cd-set-sub\">For you on every device</h2>{users}</section>"
                ));
            }
            b
        }
        _ => {
            let rows: String = defs.iter().map(|d| row(d, &ctx)).collect();
            format!("{yours}{rows}")
        }
    };
    let now = jiff::Timestamp::now();
    let devices: Vec<Value> = ctx
        .devices
        .iter()
        .map(|d| {
            let mut j = d.to_json();
            j["seen"] = json!(seen_words(d, &ctx.current, now));
            j
        })
        .collect();
    let matrix: serde_json::Map<String, Value> =
        ctx.matrix.iter().map(|(k, e)| ((*k).to_string(), e.to_json())).collect();
    let data = json!({
        "group": slug,
        "current": ctx.current,
        "devices": devices,
        "matrix": matrix,
        "settings": defs.iter().map(|d| settings::setting_json(d)).collect::<Vec<_>>(),
        "user": ctx.user,
        "presets": settings::catalog_json()["presets"],
        "seq": st.store.last_seq()?,
    });
    render(
        st,
        auth,
        headers,
        Frame { active: Active::Group(slug), index, title: label, head_extra: "", body: &body, admin: false, data },
    )
}

fn scope_line() -> String {
    format!(
        "<div class=\"cd-set-scope\" id=\"cd-set-scope\"><span class=\"cd-set-scopelbl\" id=\"cd-set-scopelbl\">Applying to</span>\
         <button type=\"button\" class=\"cd-set-menubtn\" id=\"cd-set-scopebtn\" aria-haspopup=\"menu\" aria-expanded=\"false\" aria-labelledby=\"cd-set-scopelbl cd-set-scopebtn\"><span class=\"cd-set-scopename\" id=\"cd-set-scopename\">All devices</span>{chev}</button>\
         <span class=\"cd-grow\"></span>\
         <button type=\"button\" class=\"cd-set-copybtn\" id=\"cd-set-copybtn\" aria-haspopup=\"menu\" aria-expanded=\"false\"><span>Copy from another device</span>{chev13}</button></div>",
        chev = svg(CHEV, 14, "2"),
        chev13 = svg(CHEV, 13, "2"),
    )
}

/// One setting's row: the label (+ the layers icon for a device setting) and the control in its
/// current state (All devices for a device setting).
fn row(def: &SettingDef, ctx: &Ctx) -> String {
    let (value, mixed) = match def.kind {
        SettingKind::Device => match ctx.matrix.get(def.key) {
            Some(e) => shown(e, &ctx.devices),
            None => (def.default_value(), false),
        },
        SettingKind::User => (ctx.user.get(def.key).cloned().unwrap_or_else(|| def.default_value()), false),
    };
    let key = def.key;
    let label = esc(def.label);
    let layers = if def.kind == SettingKind::Device {
        format!(
            "<button type=\"button\" class=\"cd-set-layers{}\" data-act=\"chain\" aria-label=\"{label}\" aria-haspopup=\"dialog\" aria-expanded=\"false\">{}</button>",
            if mixed { " cd-mixed" } else { "" },
            svg(LAYERS, 15, "1.8"),
        )
    } else {
        String::new()
    };
    let reset = if def.kind == SettingKind::User {
        format!(
            "<button type=\"button\" class=\"cd-set-reset\" data-act=\"reset\"{}>Reset</button>",
            if value == def.default_value() { " hidden" } else { "" }
        )
    } else {
        String::new()
    };
    format!(
        "<div class=\"cd-set-row\" id=\"cd-set-{key}\" data-key=\"{key}\" data-kind=\"{kind}\" data-type=\"{ty}\" data-search=\"{search}\">\
           <div class=\"cd-set-label\"><span class=\"cd-set-name\" id=\"cd-set-lbl-{key}\">{label}</span>{layers}</div>\
           <div class=\"cd-set-ctl\">{ctl}{reset}<div class=\"cd-set-state\" aria-live=\"polite\"></div></div>\
         </div>",
        kind = def.kind.as_str(),
        ty = def.ty.as_str(),
        search = esc(&format!("{} {}", def.label, def.help)),
        ctl = control(def, &value, mixed, ctx),
    )
}

fn mixed_pill(mixed: bool) -> String {
    format!("<span class=\"cd-set-mixed\" aria-hidden=\"true\"{}>Mixed</span>", if mixed { "" } else { " hidden" })
}

fn group_label(def: &SettingDef, mixed: bool) -> String {
    if mixed { format!("{}: Mixed", esc(def.label)) } else { esc(def.label) }
}

/// The control of one setting, in `value`'s state (no value selected when `mixed`).
fn control(def: &SettingDef, value: &Value, mixed: bool, ctx: &Ctx) -> String {
    match def.ty {
        ValueType::Choice(opts) => {
            let any_on = !mixed && opts.iter().any(|(v, _)| value.as_str() == Some(*v));
            let buttons: String = opts
                .iter()
                .enumerate()
                .map(|(i, (v, l))| {
                    let on = !mixed && value.as_str() == Some(*v);
                    format!(
                        "<button type=\"button\" role=\"radio\" class=\"cd-seg-o{}\" data-value=\"{}\" aria-checked=\"{on}\" tabindex=\"{}\">{}</button>",
                        if on { " cd-on" } else { "" },
                        esc(v),
                        if on || (!any_on && i == 0) { "0" } else { "-1" },
                        esc(l)
                    )
                })
                .collect();
            format!(
                "<div class=\"cd-set-line\"><div class=\"cd-seg\" role=\"radiogroup\" aria-label=\"{}\">{buttons}</div>{}</div>",
                group_label(def, mixed),
                mixed_pill(mixed)
            )
        }
        ValueType::Steps(_) => {
            let n = value.as_i64().unwrap_or(100);
            format!(
                "<div class=\"cd-set-line cd-tsize\" role=\"group\" aria-label=\"{}\">\
                   <button type=\"button\" class=\"cd-seg cd-seg-one\" data-act=\"smaller\" aria-label=\"Smaller\">A\u{2212}</button>\
                   <span class=\"cd-tsize-v\"{}>{n}%</span>{}\
                   <button type=\"button\" class=\"cd-seg cd-seg-one\" data-act=\"larger\" aria-label=\"Larger\">A+</button></div>\
                 <div><div class=\"cd-set-sample cd-set-sample-size\" style=\"font-size:{}px\">See how the text size looks.</div></div>",
                group_label(def, mixed),
                if mixed { " hidden" } else { "" },
                mixed_pill(mixed),
                (15.0 * n as f64 / 100.0).round() as i64,
            )
        }
        ValueType::Color { dark } => {
            let v = value.as_str().unwrap_or("yellow");
            let custom = settings::parse_hex(v).is_some();
            let swatches: String = PRESETS
                .iter()
                .map(|p| {
                    let on = !mixed && p.key == v;
                    format!(
                        "<button type=\"button\" role=\"radio\" class=\"cd-sw{}\" data-value=\"{}\" aria-label=\"{}\" aria-checked=\"{on}\" tabindex=\"{}\" style=\"--cd-sw:{}\"></button>",
                        if on { " cd-on" } else { "" },
                        p.key,
                        p.name,
                        if on || ((mixed || custom) && p.key == "yellow") { "0" } else { "-1" },
                        if dark { p.dark } else { p.light },
                    )
                })
                .collect();
            let (color, _) = settings::highlight_colors(v, dark);
            let custom_sw = format!(
                "<button type=\"button\" role=\"radio\" class=\"cd-sw cd-sw-custom{on}\" data-value=\"{c}\" aria-label=\"Custom {c}\" aria-checked=\"{checked}\" tabindex=\"-1\" style=\"--cd-sw:{c}\"{hidden}></button>",
                on = if custom && !mixed { " cd-on" } else { "" },
                c = if custom { esc(v) } else { String::new() },
                checked = custom && !mixed,
                hidden = if custom { "" } else { " hidden" },
            );
            let which = if dark { "dark" } else { "light" };
            format!(
                "<div class=\"cd-set-line\"><div class=\"cd-swatches\" role=\"radiogroup\" aria-label=\"{gl}\">{swatches}{custom_sw}</div>\
                   <button type=\"button\" class=\"cd-set-custom\" data-act=\"custom\" aria-expanded=\"false\">Custom\u{2026}</button>{pill}</div>\
                 <div class=\"cd-set-customrow\" hidden><input type=\"color\" class=\"cd-set-colorpick\" value=\"{color}\" aria-label=\"Custom color, {label}\">\
                   <input type=\"text\" class=\"cd-set-hex\" value=\"{color}\" maxlength=\"7\" placeholder=\"#rrggbb\" spellcheck=\"false\" autocomplete=\"off\" aria-label=\"Custom color as #rrggbb, {label}\"></div>\
                 <div><div class=\"cd-set-sample cd-set-sample-{which}\">See <mark class=\"cd-set-mark\" style=\"background:{color}\">what\u{2019}s highlighted</mark></div></div>",
                gl = group_label(def, mixed),
                pill = mixed_pill(mixed),
                label = esc(def.label),
            )
        }
        ValueType::Vault => {
            let v = value.as_str().unwrap_or("any");
            let mut opts = format!("<option value=\"any\"{}>Any</option>", if v == "any" { " selected" } else { "" });
            for (id, name) in &ctx.vaults {
                opts.push_str(&format!(
                    "<option value=\"{}\"{}>{}</option>",
                    esc(id),
                    if id == v { " selected" } else { "" },
                    esc(name)
                ));
            }
            format!(
                "<div class=\"cd-set-line\"><select class=\"cd-set-select\" aria-labelledby=\"cd-set-lbl-{}\">{opts}</select></div>",
                def.key
            )
        }
        ValueType::Name => format!(
            "<div class=\"cd-set-line\"><input type=\"text\" class=\"cd-set-text\" value=\"{}\" aria-labelledby=\"cd-set-lbl-{}\" autocomplete=\"name\" spellcheck=\"false\"></div>",
            esc(value.as_str().unwrap_or_default()),
            def.key
        ),
    }
}

fn profile_body(st: &AppState, auth: &Auth, yours: &str) -> Result<String, ApiError> {
    let m = st.store.member_by_id(&auth.0.member_id)?.ok_or_else(|| ApiError::not_found("no such Member"))?;
    let name = settings::setting("display_name").expect("display_name is in the catalog");
    let ro = |label: &str, value: &str| {
        format!(
            "<div class=\"cd-set-row\" data-search=\"{l}\"><div class=\"cd-set-label\"><span class=\"cd-set-name\">{l}</span></div><div class=\"cd-set-ctl cd-set-ro\">{value}</div></div>",
            l = esc(label)
        )
    };
    let role = if auth.0.is_owner() { "Owner" } else { "Member" };
    Ok(format!(
        "{yours}<div class=\"cd-set-row\" id=\"cd-set-display_name\" data-key=\"display_name\" data-kind=\"user\" data-type=\"name\" data-search=\"{search}\">\
           <div class=\"cd-set-label\"><label class=\"cd-set-name\" id=\"cd-set-lbl-display_name\" for=\"cd-set-name-in\">{label}</label></div>\
           <div class=\"cd-set-ctl\"><input type=\"text\" class=\"cd-set-text\" id=\"cd-set-name-in\" value=\"{value}\" autocomplete=\"name\" spellcheck=\"false\"><div class=\"cd-set-state\" aria-live=\"polite\"></div></div></div>\
         {handle}{role}",
        search = esc(&format!("{} {}", name.label, name.help)),
        label = esc(name.label),
        value = esc(&m.display_name),
        handle = ro("Handle", &format!("<span class=\"cd-set-handle\">@{}</span>", esc(&m.handle))),
        role = ro("Role", role),
    ))
}

/// "now", "5 minutes ago", "2 hours ago", "yesterday", "3 days ago", or the date — the same rule
/// as `seenWords` in `settings.js` (keep them equal). The current Device is always "now".
pub fn seen_words(d: &DeviceView, current: &str, now: jiff::Timestamp) -> String {
    if d.id == current {
        return "now".into();
    }
    let Some(t) = d.last_seen_at.as_deref().and_then(annotary_store::time::parse) else {
        return "never".into();
    };
    let mins = ((now.as_millisecond() - t.as_millisecond()).max(0) / 60_000) as f64;
    let plural = |n: f64, w: &str| format!("{} {w}{} ago", n, if n == 1.0 { "" } else { "s" });
    if mins < 1.0 {
        "now".into()
    } else if mins < 60.0 {
        plural(mins.floor(), "minute")
    } else if mins < 24.0 * 60.0 {
        plural((mins / 60.0).floor(), "hour")
    } else if mins < 48.0 * 60.0 {
        "yesterday".into()
    } else if mins < 30.0 * 24.0 * 60.0 {
        plural((mins / 1440.0).floor(), "day")
    } else {
        t.strftime("%Y-%m-%d").to_string()
    }
}

fn devices_body(ctx: &Ctx) -> String {
    let now = jiff::Timestamp::now();
    let mut list: Vec<&DeviceView> = ctx.devices.iter().filter(|d| d.id == ctx.current).collect();
    list.extend(ctx.devices.iter().filter(|d| d.id != ctx.current));
    let cards: String = list
        .iter()
        .map(|d| {
            let this = if d.id == ctx.current { "<span class=\"cd-dev-this\">this device</span>" } else { "" };
            let tools = if d.is_web() {
                String::new()
            } else {
                "<div class=\"cd-dev-tools\"><button type=\"button\" class=\"cd-dev-btn\" data-act=\"rename\">Rename</button><button type=\"button\" class=\"cd-dev-btn\" data-act=\"forget\">Forget</button></div>".to_string()
            };
            format!(
                "<article class=\"cd-dev\" data-device=\"{id}\" data-kind=\"{kind}\"><span class=\"cd-dev-icon\">{icon}</span>\
                   <div class=\"cd-dev-main\"><div class=\"cd-dev-namerow\"><span class=\"cd-dev-name\">{name}</span>{this}</div>\
                   <div class=\"cd-dev-seen\">Last seen {seen}</div><div class=\"cd-dev-inline\" hidden></div><div class=\"cd-dev-err\" role=\"alert\" hidden></div></div>{tools}</article>",
                id = esc(&d.id),
                kind = esc(&d.kind),
                icon = svg(kind_path(&d.kind), 22, "1.7"),
                name = esc(&d.name),
                seen = esc(&seen_words(d, &ctx.current, now)),
            )
        })
        .collect();
    format!("<div class=\"cd-devlist\" id=\"cd-devlist\">{cards}</div>")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, seen: Option<&str>) -> DeviceView {
        DeviceView {
            id: id.into(),
            name: "iPhone".into(),
            kind: "iphone".into(),
            created_at: "2026-10-01T00:00:00.000Z".into(),
            last_seen_at: seen.map(str::to_string),
            forgotten_at: None,
        }
    }

    #[test]
    fn last_seen_reads_like_the_board() {
        let now: jiff::Timestamp = "2026-10-30T12:00:00Z".parse().unwrap();
        let w = |s: &str| seen_words(&dev("X", Some(s)), "web", now);
        assert_eq!(seen_words(&dev("web", None), "web", now), "now");
        assert_eq!(w("2026-10-30T11:59:30.000Z"), "now");
        assert_eq!(w("2026-10-30T11:59:00.000Z"), "1 minute ago");
        assert_eq!(w("2026-10-30T10:00:00.000Z"), "2 hours ago");
        assert_eq!(w("2026-10-29T10:00:00.000Z"), "yesterday");
        assert_eq!(w("2026-10-25T10:00:00.000Z"), "5 days ago");
        assert_eq!(w("2026-08-01T10:00:00.000Z"), "2026-08-01");
        assert_eq!(seen_words(&dev("X", None), "web", now), "never");
    }

    #[test]
    fn json_in_a_script_tag_never_closes_it() {
        assert_eq!(json_script(&json!({ "a": "</script>" })), "{\"a\":\"\\u003c/script>\"}");
    }
}
