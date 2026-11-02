//! `daemon install | uninstall | status | start | stop | restart` (ticket 13) — the macOS
//! LaunchAgent around `annotary serve`.
//!
//! - `install [--port] [--bind ADDR --allow-remote] [--url URL] [--link-base URL]` writes
//!   `~/Library/LaunchAgents/com.annotary.server.plist` (`ProgramArguments: [<this binary>,
//!   serve, --data-dir DIR, --port PORT]` + the ticket-18 flags as given, `RunAtLoad`,
//!   `KeepAlive`, stdout/stderr → `DIR/logs/server.log`), runs first-run setup inline when the
//!   store is empty (the same flow as `serve`, `cli::serve::first_run`), then `launchctl
//!   bootstrap` + `kickstart` and prints `Installed PLIST → running, pid N, URL`; after a first
//!   run it ends with the owner Token and how the CLI gets a credential. Re-running it on an
//!   installed daemon stops it, rewrites the plist and starts it again (the data dir stays).
//! - `status` reads `server.json` (a stale pid = not running) and prints one line:
//!   `running  pid N  since YYYY-MM-DD HH:MM  URL  store DIR/store  log DIR/logs/server.log  (vX)`
//!   or `not running  store …  log …`; a note on stderr says whether the LaunchAgent is installed.
//! - `start` loads the agent (`bootstrap`), `stop` unloads it (`bootout` — `start` or the next
//!   login brings it back), `restart` = `kickstart -k` (upgrade = replace the binary, then
//!   `restart`), `uninstall` = `bootout` + remove the plist. Each prints the status line.
//! - Anywhere but macOS every subcommand exits 1: `daemon is unsupported here (OS) — run
//!   `annotary serve` under your service manager`.
//!
//! Test seams (documented in docs/dev/daemon-export-housekeeping.md): `ANNOTARY_DAEMON_OS`
//! overrides the detected OS (`linux` forces the unsupported path on a Mac; `macos` lets `status`
//! run anywhere — it needs no `launchctl`), `ANNOTARY_LAUNCHD_LABEL` overrides the label (and
//! so the plist name) so a live test never touches the real `com.annotary.server`. `HOME`
//! decides where `Library/LaunchAgents` is.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use annotary_store::Store;

use crate::client::Api;
use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode};
use crate::paths::{self, DataDir};
use crate::server::{ServerJson, pid_alive};

use super::Ctx;

/// The LaunchAgent label (and the plist's file stem).
pub const LABEL: &str = "com.annotary.server";

// ticket 14: the verb's `--help` is `cli::help::DAEMON` (the prototype's text).
/// The daemon's log file inside `DIR/logs/`.
pub const LOG_FILE: &str = "server.log";
/// Test seam: pretend to be this OS (`linux`, `macos`).
pub const OS_ENV: &str = "ANNOTARY_DAEMON_OS";
/// Test seam: use this label instead of [`LABEL`] (a live launchd test stays out of the way).
pub const LABEL_ENV: &str = "ANNOTARY_LAUNCHD_LABEL";
/// How long `install`/`start`/`restart` wait for the server to come up.
pub const START_TIMEOUT: Duration = Duration::from_secs(15);
/// How long `stop`/`uninstall` wait for the server to go away.
pub const STOP_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Subcommand, Debug)]
pub enum DaemonCmd {
    /// Write the LaunchAgent (RunAtLoad, KeepAlive, log → logs/server.log), start it, print the URL; first-run setup if the store is empty
    Install(InstallArgs),
    /// Stop the LaunchAgent and remove its plist (the data dir stays)
    Uninstall,
    /// running / pid / since / URL / store / log — from server.json (a stale pid reads as not running)
    Status,
    /// Load the LaunchAgent (it starts now and at every login)
    Start,
    /// Unload the LaunchAgent (it stays installed; `start` or the next login brings it back)
    Stop,
    /// Restart the running server (upgrade = replace the binary, then `daemon restart`)
    Restart,
}

#[derive(Args, Debug)]
pub struct InstallArgs {
    /// Port the daemon serves on
    #[arg(long, default_value_t = crate::DEFAULT_PORT, value_name = "PORT")]
    pub port: u16,
    // ---- ticket 18: passed through to `serve` in the plist ----
    /// Address to bind (default 127.0.0.1); any other IP needs --allow-remote
    #[arg(long, value_name = "ADDR")]
    pub bind: Option<String>,
    /// Allow a non-loopback --bind (a Tailscale / LAN address for your phone)
    #[arg(long)]
    pub allow_remote: bool,
    /// The URL other devices use to reach the server (http://HOST:PORT); `none` clears it
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    /// Base of the link relay (https://…); `none` clears it
    #[arg(long, value_name = "URL")]
    pub link_base: Option<String>,
    // ---- end ticket 18 ----
}

/// `--json` fields of the status line (`status`, `start`, `stop`, `restart`, `install`).
pub const FIELDS: FieldSpec = FieldSpec {
    all: &["state", "pid", "since", "url", "port", "store", "log", "plist", "installed", "loaded", "label", "version"],
    default: &["state", "pid", "url", "port", "log", "installed"],
};

/// `--json` fields of `uninstall`.
pub const UNINSTALL_FIELDS: FieldSpec =
    FieldSpec { all: &["plist", "removed", "stopped", "label"], default: &["plist", "removed", "stopped"] };

// ---------------------------------------------------------------- platform

/// The OS name (`std::env::consts::OS`, or the [`OS_ENV`] override).
pub fn os_name() -> String {
    std::env::var(OS_ENV)
        .ok()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| std::env::consts::OS.to_string())
}

/// The exit-1 error every `daemon` verb gives off macOS.
pub fn unsupported(os: &str) -> CliError {
    CliError::usage(format!("daemon is unsupported here ({os})"))
        .hint("run `annotary serve` under your service manager (systemd, a Windows service, …)")
}

fn require_macos() -> CliResult<()> {
    let os = os_name();
    if os == "macos" { Ok(()) } else { Err(unsupported(&os)) }
}

// ---------------------------------------------------------------- plist

/// The LaunchAgent label ([`LABEL`], or the [`LABEL_ENV`] override).
pub fn label() -> String {
    std::env::var(LABEL_ENV)
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| LABEL.to_string())
}

/// `~/Library/LaunchAgents/<label>.plist`.
pub fn plist_path() -> CliResult<PathBuf> {
    let home = paths::home_dir().ok_or_else(|| CliError::usage("cannot find your home directory (HOME is unset)"))?;
    Ok(home.join("Library").join("LaunchAgents").join(format!("{}.plist", label())))
}

/// The daemon's log file.
pub fn log_path(data: &DataDir) -> PathBuf {
    data.logs().join(LOG_FILE)
}

/// What the plist says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plist {
    pub label: String,
    pub binary: PathBuf,
    pub data_dir: PathBuf,
    pub port: u16,
    pub log: PathBuf,
    // ---- ticket 18 ----
    /// `serve --bind ADDR` (absent = serve's default, 127.0.0.1).
    pub bind: Option<String>,
    /// `serve --allow-remote`.
    pub allow_remote: bool,
    /// `serve --url URL|none`.
    pub url: Option<String>,
    /// `serve --link-base URL|none`.
    pub link_base: Option<String>,
    // ---- end ticket 18 ----
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

impl Plist {
    /// The plist text: `ProgramArguments [binary, serve, --data-dir DIR, --port PORT]` (+
    /// `--bind ADDR`, `--allow-remote`, `--url URL`, `--link-base URL` when set — ticket 18),
    /// `RunAtLoad`, `KeepAlive`, stdout/stderr → `log`.
    pub fn render(&self) -> String {
        let mut args = vec![
            self.binary.to_string_lossy().to_string(),
            "serve".to_string(),
            "--data-dir".to_string(),
            self.data_dir.to_string_lossy().to_string(),
            "--port".to_string(),
            self.port.to_string(),
        ];
        // ---- ticket 18 ----
        if let Some(b) = &self.bind {
            args.extend(["--bind".to_string(), b.clone()]);
        }
        if self.allow_remote {
            args.push("--allow-remote".to_string());
        }
        if let Some(u) = &self.url {
            args.extend(["--url".to_string(), u.clone()]);
        }
        if let Some(l) = &self.link_base {
            args.extend(["--link-base".to_string(), l.clone()]);
        }
        // ---- end ticket 18 ----
        let mut s = String::new();
        s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        s.push_str(
            "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        );
        s.push_str("<plist version=\"1.0\">\n<dict>\n");
        s.push_str(&format!("\t<key>Label</key>\n\t<string>{}</string>\n", xml_escape(&self.label)));
        s.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
        for a in &args {
            s.push_str(&format!("\t\t<string>{}</string>\n", xml_escape(a)));
        }
        s.push_str("\t</array>\n");
        s.push_str("\t<key>RunAtLoad</key>\n\t<true/>\n");
        s.push_str("\t<key>KeepAlive</key>\n\t<true/>\n");
        let log = xml_escape(&self.log.to_string_lossy());
        s.push_str(&format!("\t<key>StandardOutPath</key>\n\t<string>{log}</string>\n"));
        s.push_str(&format!("\t<key>StandardErrorPath</key>\n\t<string>{log}</string>\n"));
        s.push_str("</dict>\n</plist>\n");
        s
    }

    /// Read back a plist this binary wrote (`None` for anything else).
    pub fn parse(text: &str) -> Option<Plist> {
        fn unescape(s: &str) -> String {
            s.replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
        }
        fn strings_after(text: &str, key: &str) -> Option<Vec<String>> {
            let marker = format!("<key>{key}</key>");
            let rest = &text[text.find(&marker)? + marker.len()..];
            // Up to the next <key> (or the end): every <string>…</string>.
            let end = rest.find("<key>").unwrap_or(rest.len());
            let mut out = Vec::new();
            let mut r = &rest[..end];
            while let Some(i) = r.find("<string>") {
                let after = &r[i + "<string>".len()..];
                let j = after.find("</string>")?;
                out.push(unescape(&after[..j]));
                r = &after[j + "</string>".len()..];
            }
            Some(out)
        }
        let label = strings_after(text, "Label")?.into_iter().next()?;
        let args = strings_after(text, "ProgramArguments")?;
        let binary = PathBuf::from(args.first()?);
        if args.get(1).map(String::as_str) != Some("serve") {
            return None;
        }
        let mut data_dir = None;
        let mut port = None;
        let (mut bind, mut allow_remote, mut url, mut link_base) = (None, false, None, None);
        let mut i = 2;
        while i < args.len() {
            // `--allow-remote` is the one flag without a value (ticket 18).
            if args[i] == "--allow-remote" {
                allow_remote = true;
                i += 1;
                continue;
            }
            let value = args.get(i + 1).cloned();
            match args[i].as_str() {
                "--data-dir" => data_dir = value.map(PathBuf::from),
                "--port" => port = value.and_then(|v| v.parse::<u16>().ok()),
                "--bind" => bind = value,
                "--url" => url = value,
                "--link-base" => link_base = value,
                _ => {}
            }
            i += 2;
        }
        let log = PathBuf::from(strings_after(text, "StandardOutPath")?.into_iter().next()?);
        Some(Plist { label, binary, data_dir: data_dir?, port: port?, log, bind, allow_remote, url, link_base })
    }

    /// The plist currently installed at `path`, if it parses.
    pub fn read(path: &Path) -> Option<Plist> {
        Plist::parse(&std::fs::read_to_string(path).ok()?)
    }
}

// ---------------------------------------------------------------- launchctl

fn domain() -> String {
    // SAFETY: getuid has no preconditions.
    format!("gui/{}", unsafe { libc::getuid() })
}

fn service() -> String {
    format!("{}/{}", domain(), label())
}

fn launchctl(args: &[&str]) -> CliResult<std::process::Output> {
    Command::new("launchctl").args(args).output().map_err(|e| {
        CliError::internal(format!("cannot run launchctl: {e}")).hint("is this a Mac? `daemon` drives launchd")
    })
}

fn launchctl_ok(args: &[&str], what: &str) -> CliResult<()> {
    let out = launchctl(args)?;
    if out.status.success() {
        return Ok(());
    }
    let msg = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let msg = if msg.is_empty() { String::from_utf8_lossy(&out.stdout).trim().to_string() } else { msg };
    Err(CliError::internal(format!("launchctl {what} failed: {}", if msg.is_empty() { "no details" } else { &msg }))
        .hint(format!("launchctl {}", args.join(" "))))
}

/// Whether launchd has the service loaded (`launchctl print gui/UID/label`). False when
/// `launchctl` does not exist (a non-Mac pretending, tests).
pub fn is_loaded() -> bool {
    Command::new("launchctl")
        .args(["print", &service()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn bootstrap(plist: &Path) -> CliResult<()> {
    launchctl_ok(&["bootstrap", &domain(), &plist.to_string_lossy()], "bootstrap")
}

/// Unload the service; `Ok(false)` when it was not loaded.
fn bootout() -> CliResult<bool> {
    if !is_loaded() {
        return Ok(false);
    }
    launchctl_ok(&["bootout", &service()], "bootout")?;
    Ok(true)
}

fn kickstart(kill: bool) -> CliResult<()> {
    if kill {
        launchctl_ok(&["kickstart", "-k", &service()], "kickstart -k")
    } else {
        launchctl_ok(&["kickstart", &service()], "kickstart")
    }
}

// ---------------------------------------------------------------- status

/// What `status` knows, from `server.json` (+ `health` for the version) and the plist.
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub running: Option<ServerJson>,
    pub version: Option<String>,
    pub store: PathBuf,
    pub log: PathBuf,
    pub plist: PathBuf,
    pub installed: bool,
    pub loaded: bool,
}

/// The running server per `server.json` (pid alive) — `None` for a missing file or a stale pid.
pub fn running(data: &DataDir) -> Option<ServerJson> {
    crate::server::running_server(&data.server_json())
}

/// Ask a running server for its version (`/api/v1/health`), quickly and quietly.
fn server_version(url: &str) -> Option<String> {
    Api::anonymous(url).health().ok()?.get("version")?.as_str().map(str::to_string)
}

/// Gather the status (reads `server.json`, the plist, launchd's opinion and `health`).
pub fn status(ctx: &Ctx, check_launchd: bool) -> CliResult<Status> {
    let plist = plist_path()?;
    let running = running(&ctx.data);
    let version = running.as_ref().and_then(|r| server_version(&r.url));
    Ok(Status {
        running,
        version,
        store: ctx.data.store(),
        log: log_path(&ctx.data),
        installed: plist.is_file(),
        loaded: check_launchd && is_loaded(),
        plist,
    })
}

/// `YYYY-MM-DD HH:MM` in the local time zone (the stored `started_at` is UTC millis).
pub fn local_minute(ts: &str) -> String {
    match annotary_store::time::parse(ts) {
        Some(t) => t.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d %H:%M").to_string(),
        None => ts.to_string(),
    }
}

fn port_of(url: &str) -> Option<u16> {
    url.rsplit(':').next()?.trim_end_matches('/').parse().ok()
}

impl Status {
    /// The result line's fields, in order (joined with `\t` non-TTY, two spaces on a TTY).
    pub fn fields(&self) -> Vec<String> {
        let mut f = Vec::new();
        match &self.running {
            Some(r) => {
                f.push("running".to_string());
                f.push(format!("pid {}", r.pid));
                f.push(format!("since {}", local_minute(&r.started_at)));
                f.push(r.url.clone());
            }
            None => f.push("not running".to_string()),
        }
        f.push(format!("store {}", paths::display(&self.store)));
        f.push(format!("log {}", paths::display(&self.log)));
        if let Some(v) = &self.version {
            f.push(format!("(v{v})"));
        }
        f
    }

    /// The `--json` record.
    pub fn record(&self) -> Map<String, Value> {
        let mut m = Map::new();
        m.insert("state".into(), json!(if self.running.is_some() { "running" } else { "not running" }));
        if let Some(r) = &self.running {
            m.insert("pid".into(), json!(r.pid));
            m.insert("since".into(), json!(crate::output::iso_seconds(&r.started_at)));
            m.insert("url".into(), json!(r.url));
            m.insert("port".into(), json!(port_of(&r.url)));
        }
        m.insert("store".into(), json!(self.store.to_string_lossy()));
        m.insert("log".into(), json!(self.log.to_string_lossy()));
        m.insert("plist".into(), json!(self.plist.to_string_lossy()));
        m.insert("installed".into(), json!(self.installed));
        m.insert("loaded".into(), json!(self.loaded));
        m.insert("label".into(), json!(label()));
        m.insert("version".into(), json!(self.version));
        m
    }

    /// The stderr note about the LaunchAgent.
    pub fn agent_note(&self) -> String {
        if self.installed {
            format!("LaunchAgent {} installed ({})", label(), paths::display(&self.plist))
        } else {
            "no LaunchAgent installed — `annotary daemon install` keeps the server up after login".to_string()
        }
    }
}

fn print_status(ctx: &Ctx, st: &Status) -> CliResult<()> {
    if ctx.out.is_json() {
        return ctx.out.json_records(&FIELDS, vec![st.record()], true);
    }
    ctx.out.note(&st.agent_note());
    let f = st.fields();
    ctx.out.line(&if ctx.out.tty { f.join("  ") } else { f.join("\t") });
    Ok(())
}

// ---------------------------------------------------------------- waiting

fn wait_until(timeout: Duration, mut pred: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if pred() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Wait for a server with a pid other than `not_pid` to write `server.json` and answer `health`.
fn wait_running(data: &DataDir, not_pid: Option<u32>) -> Option<ServerJson> {
    let mut found = None;
    wait_until(START_TIMEOUT, || {
        if let Some(r) = running(data)
            && Some(r.pid) != not_pid
            && server_version(&r.url).is_some()
        {
            found = Some(r);
            return true;
        }
        false
    });
    found
}

/// Wait for the server recorded in `server.json` (if any) to be gone.
fn wait_stopped(data: &DataDir, pid: Option<u32>) -> bool {
    wait_until(STOP_TIMEOUT, || match pid {
        Some(p) => !pid_alive(p) || running(data).is_none_or(|r| r.pid != p),
        None => running(data).is_none(),
    })
}

// ---------------------------------------------------------------- verbs

pub fn run(ctx: &Ctx, cmd: DaemonCmd) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return match cmd {
            DaemonCmd::Uninstall => ctx.out.json_records(&UNINSTALL_FIELDS, Vec::new(), true),
            _ => ctx.out.json_records(&FIELDS, Vec::new(), true),
        };
    }
    require_macos()?;
    match cmd {
        DaemonCmd::Install(a) => install(ctx, a),
        DaemonCmd::Uninstall => uninstall(ctx),
        DaemonCmd::Status => print_status(ctx, &status(ctx, true)?),
        DaemonCmd::Start => start(ctx),
        DaemonCmd::Stop => stop(ctx),
        DaemonCmd::Restart => restart(ctx),
    }
}

fn install(ctx: &Ctx, a: InstallArgs) -> CliResult<()> {
    // ---- ticket 18: the same checks `serve` makes, before anything is written ----
    if let Some(b) = &a.bind {
        crate::server::parse_bind_ex(b, a.allow_remote)
            .map_err(|m| CliError::usage(m).hint(super::serve::LOOPBACK_HINT))?;
    }
    let url = super::serve::parse_url_flag("--url", a.url.as_deref())?;
    let link_base = super::serve::parse_url_flag("--link-base", a.link_base.as_deref())?;
    let as_arg = |f: Option<Option<String>>| f.map(|v| v.unwrap_or_else(|| "none".to_string()));
    // ---- end ticket 18 ----
    ctx.data.ensure()?;
    let plist_file = plist_path()?;
    let binary = std::env::current_exe()?;
    let binary = binary.canonicalize().unwrap_or(binary);
    let plist = Plist {
        label: label(),
        binary,
        data_dir: ctx.data.root.clone(),
        port: a.port,
        log: log_path(&ctx.data),
        bind: a.bind.clone(),
        allow_remote: a.allow_remote,
        url: as_arg(url),
        link_base: as_arg(link_base),
    };

    // A daemon already installed: stop it first (the store must be free for setup/migrations,
    // and the new plist must be the one launchd runs).
    let was = running(&ctx.data).map(|r| r.pid);
    if bootout()? {
        ctx.out.note(&format!("stopping the installed LaunchAgent {} first", label()));
        wait_stopped(&ctx.data, was);
    }

    // First-run setup inline when the store is empty (a foreground `serve` holding the store
    // is refused here by the LOCK, exit 5, before anything is written).
    let store_dir = ctx.data.store();
    let setup = {
        let store = Arc::new(Store::open(&store_dir)?);
        if store.is_set_up()? { None } else { Some(super::serve::first_run(&store, &store_dir)?) }
    }; // the store (and its LOCK) is dropped here, before launchd starts `serve`

    if let Some(dir) = plist_file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&plist_file, plist.render())?;
    bootstrap(&plist_file)?;
    kickstart(false)?;
    let Some(r) = wait_running(&ctx.data, None) else {
        return Err(CliError::internal(format!(
            "installed {} but the server did not start within {} s",
            paths::display(&plist_file),
            START_TIMEOUT.as_secs()
        ))
        .hint(format!("see {} and `annotary daemon status`", paths::display(&plist.log))));
    };
    if ctx.out.is_json() {
        let mut rec = status(ctx, true)?.record();
        rec.insert("setup".into(), json!(setup.is_some()));
        return ctx.out.json_records(&FIELDS, vec![rec], true);
    }
    let line = format!("Installed {} → running, pid {}, {}", paths::display(&plist_file), r.pid, r.url);
    ctx.out.line(&line);
    if let Some(outcome) = setup {
        super::serve::print_credential_note(&outcome, &ctx.data.config_toml());
    }
    Ok(())
}

fn uninstall(ctx: &Ctx) -> CliResult<()> {
    let plist_file = plist_path()?;
    let was = running(&ctx.data).map(|r| r.pid);
    let stopped = bootout()?;
    if stopped {
        wait_stopped(&ctx.data, was);
    }
    let removed = match std::fs::remove_file(&plist_file) {
        Ok(()) => true,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(e) => return Err(e.into()),
    };
    if ctx.out.is_json() {
        let mut m = Map::new();
        m.insert("plist".into(), json!(plist_file.to_string_lossy()));
        m.insert("removed".into(), json!(removed));
        m.insert("stopped".into(), json!(stopped));
        m.insert("label".into(), json!(label()));
        return ctx.out.json_records(&UNINSTALL_FIELDS, vec![m], true);
    }
    if !removed && !stopped {
        ctx.out.note(&format!("no LaunchAgent {} installed — nothing to do", label()));
    }
    let state = match (stopped, removed) {
        (true, true) => "stopped and removed",
        (false, true) => "removed",
        (true, false) => "stopped",
        (false, false) => "not installed",
    };
    let line = format!("{}\t{state}\t(data dir {} kept)", paths::display(&plist_file), paths::display(&ctx.data.root));
    ctx.out.line(&if ctx.out.tty { line.replace('\t', "  ") } else { line });
    Ok(())
}

fn not_installed(plist_file: &Path) -> CliError {
    CliError::refused(format!("no LaunchAgent installed ({} is missing)", paths::display(plist_file)))
        .hint("`annotary daemon install` writes it and starts the server")
}

fn start(ctx: &Ctx) -> CliResult<()> {
    let plist_file = plist_path()?;
    if !plist_file.is_file() {
        return Err(not_installed(&plist_file));
    }
    if let Some(r) = running(&ctx.data)
        && is_loaded()
    {
        ctx.out.note(&format!("already running (pid {})", r.pid));
        return print_status(ctx, &status(ctx, true)?);
    }
    if !is_loaded() {
        bootstrap(&plist_file)?;
    }
    kickstart(false)?;
    if wait_running(&ctx.data, None).is_none() {
        return Err(CliError::internal(format!(
            "the LaunchAgent is loaded but the server did not start within {} s",
            START_TIMEOUT.as_secs()
        ))
        .hint(format!("see {}", paths::display(&log_path(&ctx.data)))));
    }
    print_status(ctx, &status(ctx, true)?)
}

fn stop(ctx: &Ctx) -> CliResult<()> {
    let was = running(&ctx.data).map(|r| r.pid);
    if bootout()? {
        wait_stopped(&ctx.data, was);
        ctx.out.note(&format!(
            "stopped LaunchAgent {} — `annotary daemon start` or the next login brings it back",
            label()
        ));
    } else {
        ctx.out.note(&format!("LaunchAgent {} is not loaded — nothing to stop", label()));
    }
    print_status(ctx, &status(ctx, true)?)
}

fn restart(ctx: &Ctx) -> CliResult<()> {
    let plist_file = plist_path()?;
    if !plist_file.is_file() {
        return Err(not_installed(&plist_file));
    }
    let was = running(&ctx.data).map(|r| r.pid);
    if is_loaded() {
        kickstart(true)?;
    } else {
        bootstrap(&plist_file)?;
        kickstart(false)?;
    }
    if wait_running(&ctx.data, was).is_none() {
        return Err(CliError::internal(format!("the server did not come back within {} s", START_TIMEOUT.as_secs()))
            .hint(format!("see {}", paths::display(&log_path(&ctx.data)))));
    }
    print_status(ctx, &status(ctx, true)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: S13-01
    #[test]
    fn plist_golden_with_substituted_paths_and_round_trip() {
        let p = Plist {
            label: LABEL.to_string(),
            binary: PathBuf::from("/usr/local/bin/annotary"),
            data_dir: PathBuf::from("/opt/rowan/.annotary"),
            port: 7377,
            log: PathBuf::from("/opt/rowan/.annotary/logs/server.log"),
            bind: None,
            allow_remote: false,
            url: None,
            link_base: None,
        };
        let expected = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\">\n\
<dict>\n\
\t<key>Label</key>\n\
\t<string>com.annotary.server</string>\n\
\t<key>ProgramArguments</key>\n\
\t<array>\n\
\t\t<string>/usr/local/bin/annotary</string>\n\
\t\t<string>serve</string>\n\
\t\t<string>--data-dir</string>\n\
\t\t<string>/opt/rowan/.annotary</string>\n\
\t\t<string>--port</string>\n\
\t\t<string>7377</string>\n\
\t</array>\n\
\t<key>RunAtLoad</key>\n\
\t<true/>\n\
\t<key>KeepAlive</key>\n\
\t<true/>\n\
\t<key>StandardOutPath</key>\n\
\t<string>/opt/rowan/.annotary/logs/server.log</string>\n\
\t<key>StandardErrorPath</key>\n\
\t<string>/opt/rowan/.annotary/logs/server.log</string>\n\
</dict>\n\
</plist>\n";
        assert_eq!(p.render(), expected);
        assert_eq!(Plist::parse(&p.render()), Some(p.clone()));
        // Paths with XML-special characters survive the round trip.
        let odd = Plist { data_dir: PathBuf::from("/tmp/a&b <c>"), port: 8080, ..p };
        assert!(odd.render().contains("<string>/tmp/a&amp;b &lt;c&gt;</string>"));
        assert_eq!(Plist::parse(&odd.render()), Some(odd));
        assert_eq!(Plist::parse("<plist/>"), None);
    }

    // spec: S18-03
    #[test]
    fn plist_carries_bind_allow_remote_url_and_link_base() {
        let p = Plist {
            label: LABEL.to_string(),
            binary: PathBuf::from("/usr/local/bin/annotary"),
            data_dir: PathBuf::from("/opt/rowan/.annotary"),
            port: 7377,
            log: PathBuf::from("/opt/rowan/.annotary/logs/server.log"),
            bind: Some("100.64.0.7".into()),
            allow_remote: true,
            url: Some("http://studio.example.ts.net:7377".into()),
            link_base: Some("https://cd.example.dev".into()),
        };
        let text = p.render();
        assert!(text.contains(
            "\t\t<string>--port</string>\n\t\t<string>7377</string>\n\t\t<string>--bind</string>\n\t\t<string>100.64.0.7</string>\n\t\t<string>--allow-remote</string>\n\t\t<string>--url</string>\n\t\t<string>http://studio.example.ts.net:7377</string>\n\t\t<string>--link-base</string>\n\t\t<string>https://cd.example.dev</string>\n\t</array>"
        ), "{text}");
        assert_eq!(Plist::parse(&text), Some(p.clone()));
        // `--allow-remote` alone (no value) between two valued flags parses too.
        let q = Plist { bind: Some("0.0.0.0".into()), url: None, link_base: Some("none".into()), ..p };
        assert_eq!(Plist::parse(&q.render()), Some(q));
    }

    // spec: S13-03
    #[test]
    fn unsupported_message_and_exit_code() {
        let e = unsupported("linux");
        assert_eq!(e.exit, crate::exit::Exit::Usage);
        assert_eq!(
            e.text_line(),
            "error: daemon is unsupported here (linux) — run `annotary serve` under your service manager (systemd, a Windows service, …)"
        );
        assert_eq!(e.json()["error"], "usage");
    }

    // spec: S13-02
    #[test]
    fn status_line_and_record_shapes() {
        let dd = DataDir::new(PathBuf::from("/opt/rowan/.annotary"));
        let mut st = Status {
            running: None,
            version: None,
            store: dd.store(),
            log: log_path(&dd),
            plist: PathBuf::from("/opt/rowan/Library/LaunchAgents/com.annotary.server.plist"),
            installed: false,
            loaded: false,
        };
        assert_eq!(
            st.fields(),
            vec!["not running", "store /opt/rowan/.annotary/store", "log /opt/rowan/.annotary/logs/server.log"]
        );
        assert!(st.agent_note().starts_with("no LaunchAgent installed"));
        let rec = st.record();
        assert_eq!(rec["state"], "not running");
        assert_eq!(rec["installed"], false);
        assert!(rec.get("pid").is_none());
        st.running = Some(ServerJson {
            url: "http://127.0.0.1:7377".into(),
            pid: 4242,
            started_at: "2026-08-18T08:02:11.123Z".into(),
            data_dir: dd.root.clone(),
        });
        st.version = Some("0.1.0".into());
        st.installed = true;
        let f = st.fields();
        assert_eq!(f[0], "running");
        assert_eq!(f[1], "pid 4242");
        assert!(f[2].starts_with("since 2026-08-1"), "{}", f[2]);
        assert_eq!(f[3], "http://127.0.0.1:7377");
        assert_eq!(f[6], "(v0.1.0)");
        let rec = st.record();
        assert_eq!(rec["port"], 7377);
        assert_eq!(rec["since"], "2026-08-18T08:02:11Z");
        assert_eq!(rec["version"], "0.1.0");
        assert!(st.agent_note().starts_with("LaunchAgent com.annotary.server installed"));
        assert_eq!(log_path(&dd), PathBuf::from("/opt/rowan/.annotary/logs/server.log"));
    }
}
