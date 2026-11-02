//! The axum router and shared state. Later tickets add routes here under `/api/v1/…`.

use std::sync::Arc;

use axum::Router;
use axum::extract::{DefaultBodyLimit, State};
use axum::routing::{get, patch, post, put};
use serde_json::json;

use annotary_store::Store;

use super::error::ApiError;

/// Shared state: the store plus what `health` reports.
#[derive(Clone)]
pub struct AppState {
    pub store: Arc<Store>,
    pub url: String,
    pub started_at: String,
    // ---- ticket 08 ----
    /// Bumped after every committed store transaction (the store's commit hook) — what
    /// `GET /api/v1/events` long-polls and SSE streams park on (`events.rs`).
    pub events: tokio::sync::watch::Receiver<u64>,
    /// Flipped to `true` by [`AppState::begin_shutdown`] when the server is asked to stop:
    /// every parked long-poll answers with what it has and every SSE stream ends, so graceful
    /// shutdown never waits on a client that would otherwise stay attached for hours.
    pub shutdown: tokio::sync::watch::Receiver<bool>,
    shutdown_tx: Arc<tokio::sync::watch::Sender<bool>>,
    // ---- end ticket 08 ----
    // ---- ticket 10 ----
    /// Display titles (front matter `title:` / first H1) by Markdown blob hash — the Document
    /// list shows them without rendering every Document per request (`ui::title_for`).
    pub titles: Arc<std::sync::Mutex<std::collections::HashMap<String, Option<String>>>>,
    // ---- end ticket 10 ----
    // ---- CMD-4 ----
    /// Failed password sign-ins per client and for the account (`login_guard.rs`), shared by
    /// `POST /login` and `POST /api/v1/login`.
    pub login_throttle: Arc<super::login_guard::LoginThrottle>,
    // ---- end CMD-4 ----
}

impl AppState {
    pub fn new(store: Arc<Store>, url: String, started_at: String) -> AppState {
        // ---- ticket 08: wake parked long-polls on every commit ----
        let (tx, rx) = tokio::sync::watch::channel(0u64);
        store.set_commit_hook(Some(Box::new(move || {
            tx.send_modify(|g| *g = g.wrapping_add(1));
        })));
        let (shutdown_tx, shutdown) = tokio::sync::watch::channel(false);
        // ---- end ticket 08 ----
        AppState {
            store,
            url,
            started_at,
            events: rx,
            shutdown,
            shutdown_tx: Arc::new(shutdown_tx),
            titles: Arc::default(),
            login_throttle: Arc::default(),
        }
    }

    // ---- ticket 08 ----
    /// Tell every long-lived request (`GET /api/v1/events` long-polls and SSE streams) to let
    /// go now. `server::run` calls it the moment the shutdown signal fires.
    pub fn begin_shutdown(&self) {
        self.shutdown_tx.send_replace(true);
    }

    /// True once [`AppState::begin_shutdown`] ran.
    pub fn is_shutting_down(&self) -> bool {
        *self.shutdown.borrow()
    }
    // ---- end ticket 08 ----
}

/// Build the router. `/api/v1/health`, `POST /api/v1/login` and the `/login` page are the only
/// unauthenticated routes; everything else takes `Auth` (web pages via the middleware in `web.rs`). Resources: `members`, `vaults`, `blobs` (two-phase push transport), `docs` (see
/// `docs.rs` for the table), `docs/{id}/threads` + `mentions` (`threads.rs`), `docs/{id}/actions` +
/// `vaults/{name}/actions` (`actions.rs`), `/a/{hash}` (Assets with content types, `assets.rs`),
/// `docs/{id}/versions/{n}/html` + `rerender` (rendered HTML, `render.rs`), `events` (the log
/// behind `wait` and the live feed — long-poll + SSE, `events.rs`), `housekeeping` +
/// `settings/auto-purge` (the ticker, `housekeeping.rs`), `vaults/{name}/rename` + `DELETE
/// vaults/{name}` (ticket 12, `vaults.rs`); the Settings screen `/settings`, `/settings/{group}`
/// (CMD-23, `settings_page.rs`) with the owner pages under `/settings/server/…` (`admin.rs`).
pub fn router(state: AppState) -> Router {
    use super::{
        access, actions, admin, assets, blobs, docs, events, housekeeping, listing, me_settings, members, pairing,
        render, settings_page, threads, vaults, view, web,
    };
    // ---- ticket 06: web pages sit behind the loopback-trust / login-redirect middleware ----
    let web_pages = Router::new()
        // ---- ticket 10: the Document list and the Document view (`listing.rs`, `view.rs`) ----
        .route("/", get(listing::index))
        .route("/d/{id}", get(view::document))
        .route("/d/{id}/v/{n}", get(view::document_version))
        // ---- end ticket 10 ----
        // ---- ticket 11: one Thread's card as JSON (`view::fragment`) for live updates / confirmations ----
        .route("/d/{id}/t/{tid}", get(view::fragment))
        // ---- end ticket 11 ----
        .route("/logout", post(web::logout))
        // ---- ticket 12: the owner screens' card fragments (`admin.rs`) ----
        .route("/members/{handle}", get(admin::member_fragment))
        .route("/settings/vaults/{name}", get(admin::vault_fragment))
        // ---- end ticket 12 ----
        // ---- CMD-23: the Settings screen for every Member (`settings_page.rs`); the owner pages
        // moved under its Server group, and `/members` 308s there ----
        .route("/settings", get(settings_page::index))
        .route("/settings/{group}", get(settings_page::group))
        .route("/settings/server/{page}", get(admin::server_page))
        .route("/members", get(settings_page::members_redirect))
        // ---- end CMD-23 ----
        // ticket 03: Assets at a stable URL (`assets.rs`); a browser reaches them with its cookie
        .route("/a/{hash}", get(assets::get))
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), web::require_web_auth));
    // ---- end ticket 06 ----
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/me", get(members::me))
        // ---- CMD-23: Member settings and Devices (`me_settings.rs` has the table) ----
        .route("/api/v1/me/settings", get(me_settings::get).put(me_settings::put))
        .route("/api/v1/me/settings/catalog", get(me_settings::catalog))
        .route("/api/v1/me/settings/matrix", get(me_settings::matrix))
        .route("/api/v1/me/settings/{key}", axum::routing::delete(me_settings::reset))
        .route("/api/v1/me/devices", get(me_settings::devices))
        .route("/api/v1/me/devices/{id}", patch(me_settings::rename_device))
        .route("/api/v1/me/devices/{id}/forget", post(me_settings::forget_device))
        .route("/api/v1/me/devices/{id}/copy-from", post(me_settings::copy_from))
        .route("/api/v1/me/devices/{id}/sync", get(me_settings::sync))
        .route("/api/v1/me/rename", post(me_settings::rename_self))
        // ---- end CMD-23 ----
        .route("/api/v1/members", get(members::list).post(members::add))
        .route("/api/v1/vaults", get(vaults::list).post(access::create_vault)) // POST: ticket 06
        .route("/api/v1/blobs/check", post(blobs::check))
        // The PUT handler reads its body as a capped stream (`caps::ASSET_MAX`, typed 413).
        .route("/api/v1/blobs/{hash}", get(blobs::get).put(blobs::put).layer(DefaultBodyLimit::disable()))
        .route("/api/v1/docs", get(docs::list).post(docs::create))
        .route("/api/v1/docs/resolve", get(docs::resolve))
        .route("/api/v1/docs/{id}", get(docs::get).delete(docs::delete))
        .route("/api/v1/docs/{id}/restore", post(docs::restore))
        .route("/api/v1/docs/{id}/purge", post(docs::purge))
        .route("/api/v1/docs/{id}/versions", get(docs::versions).post(docs::push))
        .route("/api/v1/docs/{id}/versions/{n}", get(docs::version))
        .route("/api/v1/docs/{id}/status", get(docs::status))
        .route("/api/v1/docs/{id}/sidecar", get(docs::sidecar))
        // ticket 04: Threads / Comments / mentions (`threads.rs` has the table)
        .route("/api/v1/docs/{id}/threads", get(threads::list).post(threads::create))
        .route("/api/v1/docs/{id}/threads/{tid}", get(threads::get).put(threads::edit).delete(threads::delete))
        .route("/api/v1/docs/{id}/threads/{tid}/replies", post(threads::reply))
        .route("/api/v1/docs/{id}/threads/{tid}/resolve", post(threads::resolve))
        .route("/api/v1/docs/{id}/threads/{tid}/reopen", post(threads::reopen))
        .route("/api/v1/mentions", get(threads::mentions))
        // ---- ticket 08: the events log — long-poll / SSE (`events.rs` has the table) ----
        .route("/api/v1/events", get(events::list))
        // ---- end ticket 08 ----
        // ---- ticket 09: rendered HTML + rerender (`render.rs` has the table) ----
        .route("/api/v1/docs/{id}/versions/{n}/html", get(render::html))
        .route("/api/v1/docs/{id}/rerender", post(render::rerender_doc))
        .route("/api/v1/rerender", post(render::rerender_all))
        // ---- end ticket 09 ----
        // ---- ticket 05: Actions (`actions.rs` has the table) ----
        .route("/api/v1/docs/{id}/actions", get(actions::list).post(actions::act))
        .route("/api/v1/vaults/{name}/actions", get(actions::vault_get).put(actions::vault_set))
        // ---- end ticket 05 ----
        // ---- ticket 12: Vault rename / delete (`vaults.rs` has the table) ----
        .route("/api/v1/vaults/{name}/rename", post(vaults::rename))
        .route("/api/v1/vaults/{name}", axum::routing::delete(vaults::delete))
        // ---- end ticket 12 ----
        // ---- ticket 06: access model (`access.rs` has the table; `web.rs` the login page) ----
        .route("/api/v1/members/{handle}", get(access::get_member))
        .route("/api/v1/members/{handle}/rename", post(access::rename))
        .route("/api/v1/members/{handle}/disable", post(access::disable))
        .route("/api/v1/members/{handle}/role", post(access::set_role))
        .route("/api/v1/members/{handle}/grants", post(access::grant).put(access::set_grants))
        .route("/api/v1/tokens", get(access::list_tokens).post(access::mint_token))
        .route("/api/v1/tokens/{id}", axum::routing::delete(access::revoke_token))
        .route("/api/v1/login", post(access::login))
        .route("/api/v1/logout", post(access::logout))
        .route("/api/v1/settings", get(access::settings))
        .route("/api/v1/settings/password", put(access::set_password))
        // ---- ticket 18: pair a device (a Token + the `annotary://pair` URL + its QR) ----
        .route("/api/v1/pairing", post(pairing::pair))
        // ---- end ticket 18 ----
        .route("/login", get(web::login_page).post(web::login_post))
        .merge(web_pages)
        // M3b: the icons and the manifest, before sign-in (`brand.rs`).
        .merge(super::brand::routes())
        // ---- end ticket 06 ----
        // ---- ticket 13: housekeeping (`housekeeping.rs` has the table) ----
        .route("/api/v1/housekeeping", post(housekeeping::run_now))
        .route("/api/v1/settings/auto-purge", put(housekeeping::set_auto_purge))
        // ---- end ticket 13 ----
        .fallback(not_found)
        // CMD-4: a cookie or form mutation from another site is refused (`login_guard.rs`).
        .layer(axum::middleware::from_fn(super::login_guard::refuse_cross_site))
        // CMD-5: a request with the CLI profile's Vault gets the resolved Vault back (`vaults.rs`).
        .layer(axum::middleware::from_fn(vaults::echo_layer))
        .with_state(state)
}

async fn health(State(st): State<AppState>) -> axum::Json<serde_json::Value> {
    axum::Json(json!({
        "ok": true,
        "version": crate::VERSION,
        "url": st.url,
        "started_at": st.started_at,
        "set_up": st.store.is_set_up().unwrap_or(false),
        // ticket 06: `login` checks this before asking for a password.
        "password_set": st.store.password_set().unwrap_or(false),
        // ---- ticket 18: where other devices reach this server, and the link relay (null = unset) ----
        "public_url": st.store.public_url().ok().flatten(),
        "link_base": st.store.link_base().ok().flatten(),
        // ---- end ticket 18 ----
    }))
}

async fn not_found(uri: axum::http::Uri) -> ApiError {
    ApiError::not_found(format!("no route {}", uri.path()))
}
