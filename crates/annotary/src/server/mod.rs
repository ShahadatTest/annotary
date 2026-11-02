//! The local server: an axum app over the store, loopback by default.
//!
//! - [`bind`] refuses any non-loopback address unless `allow_remote` (ticket 18: `serve --bind
//!   ADDR --allow-remote`, so a phone reaches the server over Tailscale / the LAN). Loopback trust
//!   (`web.rs`) is decided by the *peer* address, never by the bind, so a remote listener changes
//!   nothing for browsers on other machines: they sign in with the local password.
//! - [`ServerJson`] is `server.json` (`{url, pid, started_at, data_dir}`): written after a
//!   successful bind, removed on clean exit; a stale pid reads as "not running".
//! - [`run`] serves until the shutdown future resolves (and runs the housekeeping ticker
//!   meanwhile — ticket 13, `housekeeping.rs`).

pub mod access;
pub mod actions;
pub mod app;
pub mod assets;
pub mod auth;
pub mod blobs;
pub mod docs;
pub mod error;
// ---- ticket 08 ----
pub mod events;
// ---- end ticket 08 ----
// ---- ticket 13 ----
pub mod housekeeping;
// ---- end ticket 13 ----
pub mod me_settings;
pub mod members;
// ---- ticket 18 ----
pub mod pairing;
pub mod prefs;
// ---- end ticket 18 ----
// ---- ticket 09 ----
pub mod render;
// ---- end ticket 09 ----
// CMD-23: the Settings screen (sidebar of groups, scope picker, the owner pages' frame).
pub mod settings_page;
pub mod sidecar;
// CMD-1: consistent store snapshots under DATA_DIR/snapshots/ for file-by-file backups.
pub mod snapshots;
pub mod threads;
pub mod vaults;
pub mod web;
// CMD-4: the failed-login throttle and the cross-site refusal
pub mod login_guard;
// ---- ticket 10 ----
pub mod listing;
pub mod splice;
pub mod ui;
pub mod view;
// ---- end ticket 10 ----
// ---- ticket 12 ----
pub mod admin;
// ---- end ticket 12 ----
// M3b: the icons and the manifest, public
pub mod brand;

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;

use annotary_store::Store;

pub use app::{AppState, router};

/// Reject anything that is not loopback. Returns the reason on refusal.
pub fn check_loopback(ip: IpAddr) -> Result<(), String> {
    if ip.is_loopback() { Ok(()) } else { Err(format!("refusing to bind {ip}")) }
}

/// Parse `--bind` text into an IP, refusing non-loopback addresses.
pub fn parse_bind(text: &str) -> Result<IpAddr, String> {
    parse_bind_ex(text, false)
}

// ---- ticket 18 ----
/// Parse `--bind` text into an IP; a non-loopback address only with `allow_remote`
/// (`serve --allow-remote`).
pub fn parse_bind_ex(text: &str, allow_remote: bool) -> Result<IpAddr, String> {
    let ip: IpAddr = text
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .parse()
        .map_err(|_| format!("invalid bind address {text:?} (use an IP address, e.g. 127.0.0.1)"))?;
    if !allow_remote {
        check_loopback(ip)?;
    }
    Ok(ip)
}
// ---- end ticket 18 ----

/// Bind a listener on `ip:port` (`port` 0 = ephemeral), refusing a non-loopback `ip` unless
/// `allow_remote` (ticket 18).
pub async fn bind(ip: IpAddr, port: u16, allow_remote: bool) -> Result<(TcpListener, SocketAddr), BindError> {
    if !allow_remote {
        check_loopback(ip).map_err(BindError::NotLoopback)?;
    }
    let listener =
        TcpListener::bind(SocketAddr::new(ip, port)).await.map_err(|e| BindError::Io(SocketAddr::new(ip, port), e))?;
    let addr = listener.local_addr().map_err(|e| BindError::Io(SocketAddr::new(ip, port), e))?;
    Ok((listener, addr))
}

#[derive(Debug, thiserror::Error)]
pub enum BindError {
    #[error("{0}")]
    NotLoopback(String),
    #[error("cannot bind {0}: {1}")]
    Io(SocketAddr, std::io::Error),
}

/// `http://127.0.0.1:7377` for a bound address (IPv6 gets brackets). An unspecified bind
/// (`0.0.0.0`, `::` — ticket 18) is named by its loopback address, which is what a client on this
/// machine (the CLI, `daemon status`, `server.json`) can connect to.
pub fn url_for(addr: SocketAddr) -> String {
    let ip = match addr.ip() {
        IpAddr::V4(ip) if ip.is_unspecified() => IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
        IpAddr::V6(ip) if ip.is_unspecified() => IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
        ip => ip,
    };
    match ip {
        IpAddr::V4(ip) => format!("http://{ip}:{}", addr.port()),
        IpAddr::V6(ip) => format!("http://[{ip}]:{}", addr.port()),
    }
}

/// `server.json` — informational: what is (or was) running.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerJson {
    pub url: String,
    pub pid: u32,
    pub started_at: String,
    pub data_dir: PathBuf,
}

impl ServerJson {
    pub fn read(path: &Path) -> Option<ServerJson> {
        let s = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&s).ok()
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let mut s = serde_json::to_string_pretty(self).expect("serialises");
        s.push('\n');
        std::fs::write(path, s)
    }

    pub fn remove(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    /// Whether the recorded pid is alive. A stale pid (process gone) reads as not running.
    pub fn is_running(&self) -> bool {
        pid_alive(self.pid)
    }
}

/// `kill(pid, 0)`: true when the process exists (even if owned by someone else).
pub fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }
    // SAFETY: signal 0 performs error checking only; no signal is sent.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

/// Read `server.json` and report whether that server is running.
pub fn running_server(path: &Path) -> Option<ServerJson> {
    ServerJson::read(path).filter(ServerJson::is_running)
}

/// Serve `store` on an already-bound `listener` until `shutdown` resolves. Writes `server.json`
/// at `server_json_path` first and removes it after a clean exit.
pub async fn run(
    store: Arc<Store>,
    listener: TcpListener,
    server_json_path: PathBuf,
    data_dir: PathBuf,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> std::io::Result<()> {
    let addr = listener.local_addr()?;
    let url = url_for(addr);
    let started_at = annotary_store::time::now();
    let info = ServerJson { url: url.clone(), pid: std::process::id(), started_at: started_at.clone(), data_dir };
    info.write(&server_json_path)?;
    // CMD-1: a snapshot now, then every 15 min, for the backup to copy whole.
    let snapshotter =
        tokio::spawn(snapshots::ticker(store.clone(), info.data_dir.clone(), snapshots::interval_from_env()));
    let state = AppState::new(store, url.clone(), started_at);
    // ---- ticket 13: the hourly housekeeping ticker lives and dies with the listener ----
    let ticker = tokio::spawn(housekeeping::ticker(state.clone(), housekeeping::interval_from_env()));
    // ---- end ticket 13 ----
    let app = router(state.clone());
    // ---- ticket 08: a bounded shutdown. The moment the signal fires, every parked
    // `GET /api/v1/events` long-poll answers and every SSE stream ends (`begin_shutdown`), so
    // the graceful drain has nothing long-lived to wait for; should a connection still linger
    // (a client mid-upload, a proxy holding the socket), `DRAIN_MAX` drops it — the listener is
    // already closed by then and `server.json` is removed either way.
    let (fired_tx, fired_rx) = tokio::sync::oneshot::channel::<()>();
    let on_signal = state.clone();
    let graceful = async move {
        shutdown.await;
        on_signal.begin_shutdown();
        let _ = fired_tx.send(());
    };
    // ---- end ticket 08 ----
    // Peer addresses reach the handlers (`ConnectInfo`): the loopback-trust middleware in
    // `web.rs` signs a loopback browser in as the owner while no password is set.
    let serve = std::future::IntoFuture::into_future(
        axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(graceful),
    );
    tokio::pin!(serve);
    let result = tokio::select! {
        r = &mut serve => r,
        _ = async {
            let _ = fired_rx.await;
            tokio::time::sleep(DRAIN_MAX).await;
        } => {
            tracing::warn!("shutdown: connections still open after {:?}; dropping them", DRAIN_MAX);
            Ok(())
        }
    };
    ticker.abort(); // ticket 13
    snapshotter.abort(); // CMD-1
    ServerJson::remove(&server_json_path);
    result
}

/// Longest the graceful drain waits after the shutdown signal before remaining connections are
/// dropped (ticket 08). Long-polls and SSE streams end at the signal itself; this is the backstop.
pub const DRAIN_MAX: std::time::Duration = std::time::Duration::from_secs(3);

/// Resolves on SIGINT (Ctrl-C) or SIGTERM.
pub async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let term = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {},
        _ = term => {},
    }
}
