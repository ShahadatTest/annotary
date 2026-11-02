//! `settings [list] [--device NAME]`, `settings get KEY`, `settings set KEY VALUE`, `settings
//! reset KEY` (CMD-23) — the CLI's side of `/api/v1/me/settings` (`server/me_settings.rs`). Every
//! Member reads and changes only their own settings.
//!
//! One record per setting: `KEY VALUE SOURCE KIND`. VALUE is the plain value (`system`, `100`,
//! `yellow`; Default Vault as the Vault's name, or `any`); SOURCE is `default | all | device |
//! user`; KIND is `user | device`. Without `--device`, a device setting shows its All devices value
//! (source `all` or `default`): the CLI names no Device of its own (it sends no
//! `X-Annotary-Device`), so it never makes one. `--device NAME` (an id or a name, see
//! `devices::resolve`) resolves for that Device.
//!
//! `set` and `reset`: a user setting takes no flag; a device setting takes exactly one of
//! `--all-devices` (scope `all`: every Device follows) or `--device NAME` (that Device's own
//! value). `text_size` takes `130` or `130%`; `default_vault` a Vault name (or id) or `any`;
//! `display_name` renames you. Both print the setting's record afterwards (for the Device named,
//! else All devices), with a note on stderr when nothing changed. Server refusals map as usual:
//! `unknown_setting` / `invalid_setting` / `low_contrast` → 1, an unknown Device → 2, a forgotten
//! one → 5.

use annotary_store::settings::{self, SettingKind, ValueType};
use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::client::{Api, qs};
use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode};

use super::{Ctx, devices};

#[derive(Subcommand, Debug)]
pub enum SettingsCmd {
    /// Every setting: key, value, source, kind (device settings for All devices unless --device)
    List(ListArgs),
    /// One setting's record
    Get(GetArgs),
    /// Change a setting (a device setting needs --all-devices or --device NAME)
    Set(SetArgs),
    /// Back to the default (a Device's own value: back to following All devices)
    Reset(ResetArgs),
}

#[derive(Args, Debug, Default)]
pub struct ListArgs {
    /// Resolve device settings for this Device (id or name) instead of All devices
    #[arg(long, value_name = "NAME")]
    pub device: Option<String>,
}

#[derive(Args, Debug)]
pub struct GetArgs {
    /// The setting's key (`annotary settings list` shows them)
    pub key: String,
    /// Resolve for this Device (id or name) instead of All devices
    #[arg(long, value_name = "NAME")]
    pub device: Option<String>,
}

#[derive(Args, Debug)]
pub struct Where {
    /// A device setting: change All devices (every Device follows)
    #[arg(long, conflicts_with = "device")]
    pub all_devices: bool,
    /// A device setting: change this Device's own value (id or name)
    #[arg(long, value_name = "NAME")]
    pub device: Option<String>,
}

#[derive(Args, Debug)]
pub struct SetArgs {
    /// The setting's key
    pub key: String,
    /// Its new value
    pub value: String,
    #[command(flatten)]
    pub at: Where,
}

#[derive(Args, Debug)]
pub struct ResetArgs {
    /// The setting's key
    pub key: String,
    #[command(flatten)]
    pub at: Where,
}

/// `--json` fields for `settings list` and `settings get`.
pub const LIST_FIELDS: FieldSpec = FieldSpec {
    all: &["key", "value", "source", "kind", "label", "group", "device"],
    default: &["key", "value", "source", "kind"],
};

/// `--json` fields for `settings set` and `settings reset`.
pub const WRITE_FIELDS: FieldSpec = FieldSpec {
    all: &["key", "value", "source", "kind", "label", "group", "device", "changed"],
    default: &["key", "value", "source", "kind", "device", "changed"],
};

pub fn run(ctx: &Ctx, cmd: Option<SettingsCmd>) -> CliResult<()> {
    match cmd.unwrap_or(SettingsCmd::List(ListArgs::default())) {
        SettingsCmd::List(a) => list(ctx, a),
        SettingsCmd::Get(a) => get(ctx, a),
        SettingsCmd::Set(a) => set(ctx, a),
        SettingsCmd::Reset(a) => reset(ctx, a),
    }
}

/// `--device NAME` → the Device's id (exit 2 for an unknown or ambiguous one).
fn device_id(api: &Api, device: Option<&str>) -> CliResult<Option<String>> {
    match device {
        None => Ok(None),
        Some(d) => Ok(Some(devices::resolve(api, d)?["id"].as_str().unwrap_or("").to_string())),
    }
}

/// `GET /api/v1/me/settings[?device=ID]` → `(settings[], vaults[])`.
fn fetch(api: &Api, device: Option<&str>) -> CliResult<(Vec<Value>, Vec<Value>)> {
    let path = match device {
        Some(id) => format!("/api/v1/me/settings?device={}", qs(id)),
        None => "/api/v1/me/settings".to_string(),
    };
    let body = api.get(&path)?;
    let list = |k: &str| body[k].as_array().cloned().unwrap_or_default();
    Ok((list("settings"), list("vaults")))
}

/// A setting's value for text and JSON: Default Vault by the Vault's name (`any` stays `any`; an
/// id the caller can't read stays the id).
fn shown_value(key: &str, value: &Value, vaults: &[Value]) -> Value {
    if key == "default_vault"
        && let Some(id) = value.as_str()
        && let Some(v) = vaults.iter().find(|v| v["id"].as_str() == Some(id))
    {
        return v["name"].clone();
    }
    value.clone()
}

fn record(row: &Value, vaults: &[Value], device: Option<&str>) -> Map<String, Value> {
    let key = row["key"].as_str().unwrap_or("").to_string();
    let def = settings::setting(&key);
    let mut r = Map::new();
    r.insert("key".into(), json!(key));
    r.insert("value".into(), shown_value(&key, &row["value"], vaults));
    r.insert("source".into(), row["source"].clone());
    r.insert("kind".into(), row["kind"].clone());
    r.insert("label".into(), json!(def.map(|d| d.label).unwrap_or("")));
    r.insert("group".into(), json!(def.map(|d| d.group).unwrap_or("")));
    let is_device = row["kind"].as_str() == Some("device");
    r.insert("device".into(), json!(if is_device { device.unwrap_or("") } else { "" }));
    r
}

fn plain(v: &Value) -> String {
    match v {
        Value::String(s) if s.is_empty() => "-".to_string(),
        Value::String(s) => s.clone(),
        Value::Null => "-".to_string(),
        other => other.to_string(),
    }
}

fn row(r: &Map<String, Value>) -> Vec<String> {
    let g = |k: &str| r.get(k).map(plain).unwrap_or_default();
    vec![g("key"), g("value"), g("source"), g("kind")]
}

fn unknown(key: &str) -> CliError {
    CliError::usage(format!("no setting {key:?}")).hint("`annotary settings list` shows them")
}

/// The record of `key` from a fresh read (for the Device named, else All devices).
fn one(api: &Api, key: &str, device: Option<&str>) -> CliResult<Map<String, Value>> {
    let (rows, vaults) = fetch(api, device)?;
    let row = rows.iter().find(|r| r["key"].as_str() == Some(key)).ok_or_else(|| unknown(key))?;
    Ok(record(row, &vaults, device))
}

fn print_one(ctx: &Ctx, r: &Map<String, Value>) {
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&row(r).join(sep));
}

fn list(ctx: &Ctx, a: ListArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let device = device_id(&api, a.device.as_deref())?;
    let (rows, vaults) = fetch(&api, device.as_deref())?;
    let records: Vec<Map<String, Value>> = rows.iter().map(|r| record(r, &vaults, device.as_deref())).collect();
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, records, false);
    }
    let rows: Vec<Vec<String>> = records.iter().map(row).collect();
    print!("{}", ctx.out.table(&["KEY", "VALUE", "SOURCE", "KIND"], &rows, &[]));
    Ok(())
}

fn get(ctx: &Ctx, a: GetArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), true);
    }
    let key = a.key.trim();
    let api = ctx.api()?;
    let device = device_id(&api, a.device.as_deref())?;
    let rec = one(&api, key, device.as_deref())?;
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, vec![rec], true);
    }
    print_one(ctx, &rec);
    Ok(())
}

/// Where a write applies, checked against the catalog this CLI knows: a user setting takes no
/// flag, a device setting exactly one. A key this CLI doesn't know goes to the server as typed
/// (a newer server may have it; an older one answers `unknown_setting`).
enum Target {
    User,
    All,
    Device(String),
    /// Unknown to this CLI and no flag: let the server pick its default scope.
    Unsaid,
}

fn target(api: &Api, verb: &str, key: &str, at: &Where) -> CliResult<Target> {
    let kind = settings::setting(key).map(|d| d.kind);
    match (kind, at.all_devices, at.device.as_deref()) {
        (Some(SettingKind::User), false, None) => Ok(Target::User),
        (Some(SettingKind::User), _, _) => Err(CliError::usage(format!(
            "{key} is a user setting, the same on every Device: drop {}",
            if at.all_devices { "--all-devices" } else { "--device" }
        ))
        .hint(format!("annotary settings {verb} {key}{}", if verb == "set" { " VALUE" } else { "" }))),
        (Some(SettingKind::Device), false, None) => {
            Err(CliError::usage(format!("{key} is a device setting: say --all-devices or --device NAME"))
                .hint("`annotary devices list` shows your Devices"))
        }
        (_, true, _) => Ok(Target::All),
        (_, false, Some(d)) => Ok(Target::Device(device_id(api, Some(d))?.unwrap_or_default())),
        (None, false, None) => Ok(Target::Unsaid),
    }
}

/// VALUE as typed → the JSON the server takes: Text size as a number (`130` or `130%`), Default
/// Vault as the id of the Vault named (or `any`), everything else as the string.
fn json_value(api: &Api, key: &str, raw: &str) -> CliResult<Value> {
    let raw = raw.trim();
    match settings::setting(key).map(|d| d.ty) {
        Some(ValueType::Steps(_)) => {
            Ok(raw.trim_end_matches('%').trim().parse::<i64>().map(|n| json!(n)).unwrap_or_else(|_| json!(raw)))
        }
        Some(ValueType::Vault) if !raw.eq_ignore_ascii_case("any") && !raw.is_empty() => {
            let body = api.get("/api/v1/vaults")?;
            let vaults = body["vaults"].as_array().cloned().unwrap_or_default();
            let by = |k: &str| vaults.iter().find(|v| v[k].as_str() == Some(raw));
            let v = by("name").or_else(|| by("id")).ok_or_else(|| {
                let names: Vec<&str> = vaults.iter().filter_map(|v| v["name"].as_str()).collect();
                CliError::not_found(format!("no Vault {raw}"))
                    .hint(format!("name one you can read ({}) or `any`", names.join(", ")))
            })?;
            Ok(v["id"].clone())
        }
        Some(ValueType::Vault) => Ok(json!(raw.to_lowercase())),
        _ => Ok(json!(raw)),
    }
}

/// After a write: the setting's fresh record (for the Device written, else All devices) plus
/// `changed`, a note when nothing changed.
fn answer(ctx: &Ctx, api: &Api, key: &str, t: &Target, changed: bool) -> CliResult<()> {
    let device = match t {
        Target::Device(id) => Some(id.as_str()),
        _ => None,
    };
    if !changed {
        ctx.out.note(&format!("{key} unchanged"));
    }
    let mut rec = one(api, key, device)?;
    rec.insert("changed".into(), json!(changed));
    if ctx.out.is_json() {
        return ctx.out.json_records(&WRITE_FIELDS, vec![rec], true);
    }
    print_one(ctx, &rec);
    Ok(())
}

fn set(ctx: &Ctx, a: SetArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&WRITE_FIELDS, Vec::new(), true);
    }
    let key = a.key.trim();
    let api = ctx.api()?;
    let t = target(&api, "set", key, &a.at)?;
    let value = json_value(&api, key, &a.value)?;
    let mut body = json!({ "key": key, "value": value });
    match &t {
        Target::User => body["scope"] = json!("user"),
        Target::All => body["scope"] = json!("all"),
        Target::Device(id) => {
            body["scope"] = json!("devices");
            body["devices"] = json!([id]);
        }
        Target::Unsaid => {}
    }
    let out = api.put_json("/api/v1/me/settings", &body)?;
    answer(ctx, &api, key, &t, out["changed"].as_bool().unwrap_or(true))
}

fn reset(ctx: &Ctx, a: ResetArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&WRITE_FIELDS, Vec::new(), true);
    }
    let key = a.key.trim();
    let api = ctx.api()?;
    let t = target(&api, "reset", key, &a.at)?;
    let query = match &t {
        Target::User => "?scope=user".to_string(),
        Target::All => "?scope=all".to_string(),
        Target::Device(id) => format!("?scope=device&device={}", qs(id)),
        Target::Unsaid => String::new(),
    };
    let out = api.delete(&format!("/api/v1/me/settings/{}{query}", qs(key)))?;
    answer(ctx, &api, key, &t, out["changed"].as_bool().unwrap_or(true))
}
