//! `devices [list] [--all]`, `devices rename DEVICE NAME`, `devices forget DEVICE` (CMD-23) — the
//! CLI's side of `/api/v1/me/devices` (`server/me_settings.rs`). Every Member sees and changes only
//! their own Devices.
//!
//! `list` prints `ID KIND NAME LAST SEEN` (LAST SEEN relative, like `token list`'s LAST USED);
//! forgotten Devices only with `--all`, their NAME marked `(forgotten)`. `rename` prints the
//! Device's `list` row; `forget` prints `ID NAME forgotten N Tokens revoked M sessions ended`.
//! DEVICE is an id, else a name (case-insensitive) — [`resolve`], which `settings --device` uses
//! too. Web stands for every browser: the server refuses to rename or forget it (409 `web_device`
//! → exit 5). The CLI itself never sends `X-Annotary-Device`, so it never makes a Device.

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::client::{Api, qs};
use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, iso_seconds, relative_age};

use super::Ctx;

#[derive(Subcommand, Debug)]
pub enum DevicesCmd {
    /// Your Devices: id, kind, name, last seen (--all adds forgotten ones)
    List(ListArgs),
    /// Rename one of your Devices (never Web)
    Rename(RenameArgs),
    /// Forget a Device: revoke the Tokens and end the sessions last used from it (never Web)
    Forget(DeviceArg),
}

#[derive(Args, Debug, Default)]
pub struct ListArgs {
    /// Also list forgotten Devices
    #[arg(long)]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct RenameArgs {
    /// The Device: its id or its name
    pub device: String,
    /// Its new name
    pub name: String,
}

#[derive(Args, Debug)]
pub struct DeviceArg {
    /// The Device: its id or its name
    pub device: String,
}

/// `--json` fields for `devices list` and `devices rename`.
pub const LIST_FIELDS: FieldSpec = FieldSpec {
    all: &["id", "kind", "name", "last_seen", "created", "forgotten"],
    default: &["id", "kind", "name", "last_seen", "forgotten"],
};

/// `--json` fields for `devices forget`.
pub const FORGET_FIELDS: FieldSpec = FieldSpec {
    all: &["id", "kind", "name", "forgotten", "tokens_revoked", "sessions_ended", "already"],
    default: &["id", "name", "forgotten", "tokens_revoked", "sessions_ended", "already"],
};

pub fn run(ctx: &Ctx, cmd: Option<DevicesCmd>) -> CliResult<()> {
    match cmd.unwrap_or(DevicesCmd::List(ListArgs::default())) {
        DevicesCmd::List(a) => list(ctx, a),
        DevicesCmd::Rename(a) => rename(ctx, a),
        DevicesCmd::Forget(a) => forget(ctx, a),
    }
}

/// The caller's Devices (`all`: forgotten ones too), in the server's order (oldest first).
pub fn fetch(api: &Api, all: bool) -> CliResult<Vec<Value>> {
    let body = api.get(if all { "/api/v1/me/devices?all=1" } else { "/api/v1/me/devices" })?;
    Ok(body["devices"].as_array().cloned().unwrap_or_default())
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

fn is_forgotten(d: &Value) -> bool {
    d.get("forgotten").and_then(Value::as_bool).unwrap_or(false)
}

/// DEVICE → one of the caller's Devices (forgotten ones included): an exact id first, else a
/// name, case-insensitively — among Devices that aren't forgotten first, then forgotten ones.
/// Unknown or ambiguous ⇒ exit 2, the candidates (or every Device) as detail lines.
pub fn resolve(api: &Api, wanted: &str) -> CliResult<Value> {
    let wanted = wanted.trim();
    if wanted.is_empty() {
        return Err(CliError::usage("DEVICE must not be empty").hint("`annotary devices list` shows yours"));
    }
    let devices = fetch(api, true)?;
    if let Some(d) = devices.iter().find(|d| s(d, "id") == wanted) {
        return Ok(d.clone());
    }
    let low = wanted.to_lowercase();
    let named: Vec<&Value> = devices.iter().filter(|d| s(d, "name").to_lowercase() == low).collect();
    let live: Vec<&Value> = named.iter().copied().filter(|d| !is_forgotten(d)).collect();
    let pick = if live.is_empty() { named } else { live };
    match pick.as_slice() {
        [one] => Ok((*one).clone()),
        [] => {
            let mut err = CliError::not_found(format!("no Device {wanted}"))
                .hint("name one by its id or its name (`annotary devices list --all`)");
            for d in devices.iter().filter(|d| !is_forgotten(d)) {
                err = err.detail(format!("  {}  {}", s(d, "id"), s(d, "name")));
            }
            Err(err)
        }
        many => {
            let mut err = CliError::not_found(format!("{} Devices named \"{wanted}\" — use the id:", many.len()));
            for d in many {
                err = err.detail(format!("  {}  {}  {}", s(d, "id"), s(d, "kind"), s(d, "name")));
            }
            Err(err)
        }
    }
}

/// A Device API object → the flat record for text and JSON.
fn record(d: &Value) -> Map<String, Value> {
    let mut r = Map::new();
    r.insert("id".into(), json!(s(d, "id")));
    r.insert("kind".into(), json!(s(d, "kind")));
    r.insert("name".into(), json!(s(d, "name")));
    r.insert("last_seen".into(), json!(iso_seconds(&s(d, "last_seen_at"))));
    r.insert("created".into(), json!(iso_seconds(&s(d, "created_at"))));
    r.insert("forgotten".into(), json!(is_forgotten(d)));
    r.insert("last_seen_raw".into(), json!(s(d, "last_seen_at")));
    r
}

fn row(r: &Map<String, Value>, now: jiff::Timestamp) -> Vec<String> {
    let g = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut name = g("name");
    if r.get("forgotten").and_then(Value::as_bool).unwrap_or(false) {
        name.push_str(" (forgotten)");
    }
    let seen = if g("last_seen_raw").is_empty() { "-".to_string() } else { relative_age(&g("last_seen_raw"), now) };
    vec![g("id"), g("kind"), name, seen]
}

fn list(ctx: &Ctx, a: ListArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let records: Vec<Map<String, Value>> = fetch(&api, a.all)?.iter().map(record).collect();
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, records, false);
    }
    if ctx.out.tty && records.is_empty() {
        ctx.out.note("no Devices yet — signing in on the web or in the iOS app adds one");
        return Ok(());
    }
    let now = jiff::Timestamp::now();
    let rows: Vec<Vec<String>> = records.iter().map(|r| row(r, now)).collect();
    print!("{}", ctx.out.table(&["ID", "KIND", "NAME", "LAST SEEN"], &rows, &[]));
    Ok(())
}

fn rename(ctx: &Ctx, a: RenameArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), true);
    }
    let name = a.name.trim();
    if name.is_empty() {
        return Err(CliError::usage("NAME must not be empty").hint("`annotary devices list` shows yours"));
    }
    let api = ctx.api()?;
    let d = resolve(&api, &a.device)?;
    let id = s(&d, "id");
    let renamed = api.patch_json(&format!("/api/v1/me/devices/{}", qs(&id)), &json!({ "name": name }))?;
    if s(&d, "name") == s(&renamed, "name") {
        ctx.out.note(&format!("Device {id} already has that name"));
    }
    let rec = record(&renamed);
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, vec![rec], true);
    }
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&row(&rec, jiff::Timestamp::now()).join(sep));
    Ok(())
}

fn plural(n: u64, one: &str) -> String {
    if n == 1 { format!("1 {one}") } else { format!("{n} {one}s") }
}

fn forget(ctx: &Ctx, a: DeviceArg) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FORGET_FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let d = resolve(&api, &a.device)?;
    let id = s(&d, "id");
    let answer = api.post(&format!("/api/v1/me/devices/{}/forget", qs(&id)), &json!({}))?;
    let dev = &answer["device"];
    let newly = answer["forgotten"].as_bool().unwrap_or(false);
    let tokens = answer["tokens_revoked"].as_u64().unwrap_or(0);
    let sessions = answer["sessions_ended"].as_u64().unwrap_or(0);
    if !newly {
        ctx.out.note(&format!("Device {id} was already forgotten"));
    }
    if ctx.out.is_json() {
        let mut r = Map::new();
        r.insert("id".into(), json!(s(dev, "id")));
        r.insert("kind".into(), json!(s(dev, "kind")));
        r.insert("name".into(), json!(s(dev, "name")));
        r.insert("forgotten".into(), json!(true));
        r.insert("tokens_revoked".into(), json!(tokens));
        r.insert("sessions_ended".into(), json!(sessions));
        r.insert("already".into(), json!(!newly));
        return ctx.out.json_records(&FORGET_FIELDS, vec![r], true);
    }
    let sep = if ctx.out.tty { "  " } else { "\t" };
    let fields = [
        s(dev, "id"),
        s(dev, "name"),
        "forgotten".to_string(),
        format!("{} revoked", plural(tokens, "Token")),
        format!("{} ended", plural(sessions, "session")),
    ];
    ctx.out.line(&fields.join(sep));
    Ok(())
}
