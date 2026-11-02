//! `annotary serve [--port 7377] [--bind 127.0.0.1] [--allow-remote] [--url URL|none]
//! [--link-base URL|none] [--setup-only] [--verify-edge on|off]`.
//!
//! Foreground, logs to stderr, loopback unless `--allow-remote` (ticket 18: `--bind` a Tailscale /
//! LAN address so a phone reaches the API; a warning is logged). `--url` / `--link-base` store
//! the public URL other devices use and the link relay base (`annotary_store::links`); a later
//! `serve` without them keeps the stored values, `none` clears. On an empty store it runs first-run setup inline
//! (owner handle + display name defaulting from `$USER`, optional local password, Vault
//! `notes`), prints the owner's first Token once, and says how the CLI gets a credential.
//!
//! Scriptable setup (tests, orchestration, `daemon install` — which reuses [`first_run`] and
//! [`print_credential_note`], ticket 13): when stdin is not a terminal each
//! prompt reads one line from stdin (EOF/blank = default; the effective answer is echoed);
//! `ANNOTARY_SETUP_HANDLE`, `ANNOTARY_SETUP_NAME`, `ANNOTARY_SETUP_PASSWORD` answer a
//! prompt without asking (the prompt machinery is `cli::prompt`, shared with `login`).
//! `--setup-only` exits after setup instead of serving.
//!
//! CMD-23: the hidden `--print-schema-version` prints the store schema this binary runs and exits
//! before the data dir is touched (no LOCK, no migration); the deploy uses it to decide whether
//! it must export the store before the new binary migrates it.

use std::io::IsTerminal;
use std::sync::Arc;

use clap::Args;

use annotary_store::{SetupInput, SetupOutcome, Store};

use crate::exit::{CliError, CliResult};
use crate::paths;
use crate::server;

use super::Ctx;
use super::prompt::{Prompter, Sink};

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// Port to listen on (0 = pick a free port; the URL is logged and written to server.json)
    #[arg(long, default_value_t = crate::DEFAULT_PORT, value_name = "PORT")]
    pub port: u16,
    /// Address to bind (127.0.0.1); any other IP — a Tailscale / LAN address, 0.0.0.0 — needs --allow-remote
    #[arg(long, default_value = "127.0.0.1", value_name = "ADDR")]
    pub bind: String,
    // ---- ticket 18 ----
    /// Allow a non-loopback --bind: the API is reachable from the network (every request needs a
    /// Token); browsers on other machines sign in with the local password
    #[arg(long)]
    pub allow_remote: bool,
    /// The URL other devices use to reach this server (http://HOST:PORT), stored; `none` clears it
    #[arg(long, value_name = "URL")]
    pub url: Option<String>,
    /// Base of the link relay (https://…) that makes Document links open the app; stored; `none` clears it
    #[arg(long, value_name = "URL")]
    pub link_base: Option<String>,
    // ---- end ticket 18 ----
    /// Run first-run setup if the store is empty, then exit without serving
    #[arg(long)]
    pub setup_only: bool,
    /// Recompute what every push computed at the edge — the Thread placements (ticket 07) and
    /// that the file contains the head's changes (ticket 17) — and refuse a commit that disagrees
    /// (on locally; a hosted server may trust the edge with `off`). `--verify-placements` is the
    /// ticket-07 name, kept as an alias
    #[arg(long, alias = "verify-placements", default_value = "on", value_name = "on|off", value_parser = ["on", "off"])]
    pub verify_edge: String,
    // ---- CMD-23 ----
    /// Print the store schema this binary runs (`SCHEMA_VERSION`) and exit without touching the
    /// data dir; `deploy/install-linux.sh` compares it with `store/meta.json` before a deploy
    #[arg(long, hide = true)]
    pub print_schema_version: bool,
    // ---- end CMD-23 ----
}

/// The hint of a refused non-loopback bind (ticket 18: `--allow-remote` is the opt-in).
pub const LOOPBACK_HINT: &str = "annotary serves loopback only; add --allow-remote to serve on a Tailscale / LAN address (then set a local password for web access), or use an SSH tunnel";

// ---- ticket 18 ----
/// A `--url` / `--link-base` value: `None` = flag absent (keep the stored value), `Some(None)` =
/// `none` (clear), `Some(Some(url))` = set (validated and normalised here, before the store is
/// touched).
pub type UrlFlag = Option<Option<String>>;

/// Parse `--url` / `--link-base` (`flag` names it in the error, exit 1).
pub fn parse_url_flag(flag: &str, value: Option<&str>) -> CliResult<UrlFlag> {
    let Some(v) = value.map(str::trim) else { return Ok(None) };
    if v.eq_ignore_ascii_case("none") || v.is_empty() {
        return Ok(Some(None));
    }
    annotary_store::normalize_base_url(v).map(|u| Some(Some(u))).map_err(|reason| {
        CliError::usage(format!("{flag} {v:?}: {reason}"))
            .hint(format!("e.g. {flag} http://studio.example.ts.net:7377, or {flag} none to clear it"))
    })
}

/// Store the `--url` / `--link-base` values that were given (no event when unchanged).
pub fn apply_url_flags(store: &Store, url: &UrlFlag, link_base: &UrlFlag) -> CliResult<()> {
    if let Some(u) = url {
        store.set_public_url(None, u.as_deref())?;
    }
    if let Some(l) = link_base {
        store.set_link_base(None, l.as_deref())?;
    }
    Ok(())
}
// ---- end ticket 18 ----

pub fn run(ctx: &Ctx, args: ServeArgs) -> CliResult<()> {
    // CMD-23: before anything else, so it never creates, locks or migrates a store.
    if args.print_schema_version {
        println!("{}", annotary_store::SCHEMA_VERSION);
        return Ok(());
    }
    // Cheap argument checks first: a refused bind or a bad URL must not touch the store.
    let ip =
        server::parse_bind_ex(&args.bind, args.allow_remote).map_err(|m| CliError::usage(m).hint(LOOPBACK_HINT))?;
    let url_flag = parse_url_flag("--url", args.url.as_deref())?;
    let link_flag = parse_url_flag("--link-base", args.link_base.as_deref())?;

    ctx.data.ensure()?;
    let store_dir = ctx.data.store();
    let store = Arc::new(Store::open(&store_dir)?);
    store.set_verify_edge(args.verify_edge == "on"); // tickets 07 / 17

    if args.setup_only {
        if !store.is_set_up()? {
            let outcome = first_run(&store, &store_dir)?;
            apply_url_flags(&store, &url_flag, &link_flag)?; // ticket 18
            print_credential_note(&outcome, &ctx.data.config_toml());
        } else {
            apply_url_flags(&store, &url_flag, &link_flag)?; // ticket 18
            let owner = store.list_members()?.into_iter().find(|m| m.role == annotary_store::Role::Owner);
            println!(
                "store {} is already set up{}",
                paths::display(&store_dir),
                owner.map(|o| format!(" (owner @{})", o.handle)).unwrap_or_default()
            );
        }
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| CliError::internal(format!("cannot start runtime: {e}")))?;
    rt.block_on(async {
        // Bind before first-run setup so a busy port fails before anything is created; the
        // backlog simply holds early connections until setup is done and serving starts.
        let (listener, addr) = server::bind(ip, args.port, args.allow_remote).await.map_err(|e| match e {
            server::BindError::NotLoopback(m) => CliError::usage(m).hint(LOOPBACK_HINT),
            server::BindError::Io(a, e) => CliError::refused(format!("cannot bind {a}: {e}"))
                .hint("is another annotary serve running? try --port"),
        })?;
        if !store.is_set_up()? {
            let outcome = first_run(&store, &store_dir)?;
            print_credential_note(&outcome, &ctx.data.config_toml());
        }
        apply_url_flags(&store, &url_flag, &link_flag)?; // ticket 18
        init_logging();
        let url = server::url_for(addr);
        tracing::info!(
            "annotary {} serving {url} (store {}, pid {})",
            crate::VERSION,
            paths::display(&store_dir),
            std::process::id()
        );
        // ---- ticket 18 ----
        if !ip.is_loopback() {
            tracing::warn!(
                "bound to {} (not loopback): the API is reachable from the network — every request needs a Token; \
                 set a local password (`annotary login` needs one; web Settings) so browsers on other devices can sign in \
                 (loopback trust covers this machine only)",
                addr
            );
        }
        match (store.public_url()?, store.link_base()?) {
            (Some(p), Some(l)) => tracing::info!("public URL {p}; Document links go through {l}"),
            (Some(p), None) => tracing::info!("public URL {p}"),
            _ => {}
        }
        // ---- end ticket 18 ----
        crate::update::at_serve_start(&ctx.data.root);
        tracing::info!("press Ctrl-C to stop");
        server::run(store.clone(), listener, ctx.data.server_json(), ctx.data.root.clone(), server::shutdown_signal())
            .await
            .map_err(|e| CliError::internal(format!("server error: {e}")))?;
        tracing::info!("stopped");
        Ok::<(), CliError>(())
    })
}

fn init_logging() {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::stderr().is_terminal())
        .with_target(false)
        .compact()
        .try_init();
}

// ---------------------------------------------------------------- first-run setup

/// The setup prompter: transcript on stdout, two-space indent; see `cli::prompt`.
fn prompter() -> Prompter {
    let p = Prompter::new(Sink::Stdout, "  ");
    if p.stdin_idle() {
        eprintln!(
            "(stdin is idle — first-run setup takes the defaults; ANNOTARY_SETUP_HANDLE/NAME/PASSWORD answer prompts)"
        );
    }
    p
}

/// The `$USER` name and, when the OS knows it, the account's full name.
fn whoami() -> (String, String) {
    let user = std::env::var("USER").ok().filter(|u| !u.trim().is_empty()).unwrap_or_else(|| "owner".to_string());
    let full = full_name().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| user.clone());
    (user, full)
}

/// GECOS full name of the current uid (macOS/Linux), first field before a comma.
fn full_name() -> Option<String> {
    // SAFETY: getpwuid returns a pointer to a static (thread-unsafe) struct; we copy the string
    // out immediately and never hold the pointer across another libc call.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() {
            return None;
        }
        let gecos = (*pw).pw_gecos;
        if gecos.is_null() {
            return None;
        }
        let s = std::ffi::CStr::from_ptr(gecos).to_string_lossy().into_owned();
        Some(s.split(',').next().unwrap_or("").trim().to_string())
    }
}

/// Run first-run setup with prompts, printing the transcript on stdout.
pub fn first_run(store: &Store, store_dir: &std::path::Path) -> CliResult<SetupOutcome> {
    println!("Tenant created at {}", paths::display(store_dir));
    let p = prompter();
    let (user, full) = whoami();
    let default_handle = annotary_store::suggest_handle(&user).unwrap_or_else(|| "owner".to_string());
    let handle = loop {
        let h = p.ask("owner handle", &default_handle, Some("ANNOTARY_SETUP_HANDLE"));
        match annotary_store::validate_handle(&h) {
            Ok(()) => break h,
            Err(e) => {
                println!("  {e}");
                if std::env::var_os("ANNOTARY_SETUP_HANDLE").is_some() || !p.stdin_tty {
                    return Err(CliError::usage(e.to_string()));
                }
            }
        }
    };
    let name = p.ask("display name", &full, Some("ANNOTARY_SETUP_NAME"));
    let password =
        p.ask_password("local password (blank = loopback trust, browser is you)", Some("ANNOTARY_SETUP_PASSWORD"));
    let outcome = store.setup(SetupInput { owner_handle: handle, owner_name: name, password })?;
    println!("Vault \"{}\" created.", outcome.vault.name);
    Ok(outcome)
}

/// The end of first-run setup: the owner Token (shown once) and how the CLI gets a credential.
/// `daemon install` (ticket 13) prints it too, after its `Installed …` line.
pub fn print_credential_note(outcome: &SetupOutcome, config_path: &std::path::Path) {
    println!(
        "Owner token for the CLI (shown once) — put it in your shell or a profile in {}:",
        paths::display(config_path)
    );
    println!();
    println!("  ANNOTARY_TOKEN={}", outcome.owner_token.plain);
    println!();
    println!(
        "No CLI credential is saved: export the token above, or run `annotary login` (needs the local password); \
         `annotary member add <handle>` mints a token for an agent."
    );
}
