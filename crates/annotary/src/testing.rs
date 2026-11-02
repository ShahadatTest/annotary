//! In-process server for integration tests: a temp data dir, a set-up store (owner `rowan`), the
//! app bound on `127.0.0.1:0`. Tests talk HTTP to `url` (with `ureq`) or read the `store`
//! directly to assert on state.
//!
//! ```no_run
//! let ts = annotary::testing::TestServer::start();
//! let resp = ureq::get(format!("{}/api/v1/members", ts.url))
//!     .header("Authorization", format!("Bearer {}", ts.owner_token))
//!     .call();
//! ```
//!
//! A test that counts events or Devices across a wait starts with [`TestServer::start_locked`]
//! (CMD-19, CMD-24). With no local password, any `GET /` from loopback signs a browser in as the
//! owner, which appends `session.started` and makes a Web Device; tools that probe freshly bound
//! loopback ports (T3 Code's preview port watcher) send exactly that. [`probe_root`] sends the
//! same request, so a test can make the probe happen on purpose.

use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use annotary_store::{SetupInput, Store};

use crate::paths::DataDir;
use crate::server;

/// A running in-process server. Dropping it shuts the server down and deletes the temp dir.
pub struct TestServer {
    pub url: String,
    pub store: Arc<Store>,
    pub data_dir: PathBuf,
    pub server_json: PathBuf,
    pub owner_id: String,
    pub owner_token: String,
    _tmp: tempfile::TempDir,
    rt: Option<tokio::runtime::Runtime>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<std::io::Result<()>>>,
}

impl TestServer {
    /// Start on `127.0.0.1:0` with a fresh, set-up store (owner `@rowan`, no password).
    pub fn start() -> TestServer {
        Self::try_start("127.0.0.1".parse().unwrap()).expect("test server starts")
    }

    /// Start like [`TestServer::start`], but the owner has the local password
    /// [`TEST_PASSWORD`]: a browser on loopback must log in, so a stray `GET /` changes nothing
    /// in the store. API calls with a Bearer Token behave the same.
    pub fn start_locked() -> TestServer {
        Self::try_start_with("127.0.0.1".parse().unwrap(), false, Some(TEST_PASSWORD)).expect("test server starts")
    }

    /// Start on `ip:0`; `Err` when the bind is refused (non-loopback) or fails.
    pub fn try_start(ip: IpAddr) -> Result<TestServer, server::BindError> {
        Self::try_start_ex(ip, false)
    }

    // ---- ticket 18 ----
    /// Start on `ip:0` with `serve --allow-remote` semantics: any IP may be bound (a LAN /
    /// Tailscale address, `0.0.0.0`).
    pub fn start_remote(ip: IpAddr) -> Result<TestServer, server::BindError> {
        Self::try_start_ex(ip, true)
    }
    // ---- end ticket 18 ----

    fn try_start_ex(ip: IpAddr, allow_remote: bool) -> Result<TestServer, server::BindError> {
        Self::try_start_with(ip, allow_remote, None)
    }

    fn try_start_with(ip: IpAddr, allow_remote: bool, password: Option<&str>) -> Result<TestServer, server::BindError> {
        let tmp = tempfile::tempdir().expect("tempdir");
        let dd = DataDir::new(tmp.path().join(".annotary"));
        dd.ensure().expect("data dir");
        let store = Arc::new(Store::open(&dd.store()).expect("store opens"));
        let out = store
            .setup(SetupInput {
                owner_handle: "rowan".into(),
                owner_name: "Rowan Vale".into(),
                password: password.map(str::to_string),
            })
            .expect("setup");
        let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
        let (listener, addr) = rt.block_on(server::bind(ip, 0, allow_remote))?;
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let sj = dd.server_json();
        let task = rt.spawn(server::run(store.clone(), listener, sj.clone(), dd.root.clone(), async move {
            let _ = rx.await;
        }));
        // Wait until the server answers.
        let url = server::url_for(addr);
        for _ in 0..200 {
            if ureq::get(format!("{url}/api/v1/health")).call().is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        Ok(TestServer {
            url,
            store,
            data_dir: dd.root.clone(),
            server_json: sj,
            owner_id: out.owner.id,
            owner_token: out.owner_token.plain,
            _tmp: tmp,
            rt: Some(rt),
            shutdown: Some(tx),
            task: Some(task),
        })
    }

    /// Create another Vault through the store (default Actions); returns its name.
    pub fn create_vault(&self, name: &str) -> String {
        self.store.create_vault(&self.owner_id, name).expect("create vault").name
    }

    /// Add a Member through the store and return its Token (plain text).
    pub fn add_member(&self, handle: &str) -> (String, String) {
        let (m, t) = self.store.add_member(&self.owner_id, handle, None).expect("add member");
        (m.id, t.plain)
    }

    /// GET `path` with `token`; returns (status, body JSON or string).
    pub fn get(&self, path: &str, token: Option<&str>) -> (u16, serde_json::Value) {
        let mut req = ureq::get(format!("{}{}", self.url, path));
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        finish(req.config().http_status_as_error(false).build().call())
    }

    /// POST `path` with a JSON body and `token`.
    pub fn post(&self, path: &str, token: Option<&str>, body: &serde_json::Value) -> (u16, serde_json::Value) {
        let mut req = ureq::post(format!("{}{}", self.url, path));
        if let Some(t) = token {
            req = req.header("Authorization", format!("Bearer {t}"));
        }
        finish(req.config().http_status_as_error(false).build().send_json(body))
    }

    /// Stop the server (graceful) and wait for `run` to return.
    pub fn shutdown(&mut self) {
        if let Some(tx) = self.shutdown.take() {
            let _ = tx.send(());
        }
        if let (Some(rt), Some(task)) = (self.rt.as_ref(), self.task.take()) {
            let _ = rt.block_on(task);
        }
    }
}

/// The local password [`TestServer::start_locked`] gives the owner.
pub const TEST_PASSWORD: &str = "test-password";

/// What a port watcher sends to a freshly bound loopback port: `GET /` with no credentials and
/// no redirects followed. Returns the status (0 when the request failed).
pub fn probe_root(url: &str) -> u16 {
    let res = ureq::get(format!("{url}/")).config().http_status_as_error(false).max_redirects(0).build().call();
    res.map(|r| r.status().as_u16()).unwrap_or(0)
}

fn finish(res: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> (u16, serde_json::Value) {
    match res {
        Ok(mut r) => {
            let status = r.status().as_u16();
            let text = r.body_mut().read_to_string().unwrap_or_default();
            let v = serde_json::from_str(&text).unwrap_or(serde_json::Value::String(text));
            (status, v)
        }
        Err(e) => (0, serde_json::Value::String(e.to_string())),
    }
}

impl Drop for TestServer {
    fn drop(&mut self) {
        self.shutdown();
        if let Some(rt) = self.rt.take() {
            rt.shutdown_timeout(std::time::Duration::from_secs(2));
        }
    }
}
