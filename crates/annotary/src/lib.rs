//! annotary: one binary that is both the CLI and the local server.
//!
//! Module map:
//! - [`cli`] — clap definitions and verb dispatch (`serve`, `member …`).
//! - [`server`] — axum app: auth extractor, JSON errors, `/api/v1/*` handlers, `server.json`.
//! - [`client`] — the CLI's HTTP client + credential resolution (env / profile / default).
//! - [`config`] — `config.toml` profiles.
//! - [`output`] — TTY vs non-TTY tables, `--json [FIELDS]`, colour, notes/errors.
//! - [`exit`] — the exit-code table and `CliError`.
//! - [`paths`] — data dir resolution and `~` display.
//! - [`testing`] — in-process server for integration tests.
//! - [`assets`] — the push-time Asset scan (types, references, dimensions, derivatives).
//! - [`qr`] — QR codes for pairing a device (SVG + terminal quadrant blocks; ticket 18).
//! - [`update`] — the cached check for a newer release, for `-V` and `serve` start.

pub mod assets;
pub mod cli;
pub mod client;
pub mod config;
pub mod exit;
pub mod output;
pub mod paths;
pub mod qr;
pub mod server;
pub mod testing;
pub mod update;

/// The binary's version (also what `-V` prints and what the skill file compares against): the
/// `ANNOTARY_VERSION` the build ran with (the deploy's next tag, `deploy/build_release.sh`),
/// else the workspace's `0.0.0` placeholder.
pub const VERSION: &str = match option_env!("ANNOTARY_VERSION") {
    Some(v) if !v.is_empty() => v,
    _ => env!("CARGO_PKG_VERSION"),
};

/// Default server URL the CLI calls when neither env nor profile says otherwise.
pub const DEFAULT_SERVER: &str = "http://127.0.0.1:7377";

/// Default port for `serve`.
pub const DEFAULT_PORT: u16 = 7377;
