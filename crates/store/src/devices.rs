//! Devices and the stored Member settings (CMD-23). The catalog, stamps and colors are
//! `settings.rs`; the notes are `docs/dev/member-settings.md`.
//!
//! - **Devices** (`devices`, keyed by (member, id)): `web` is every browser, made the first time a
//!   Member signs in to a page; an iPhone / iPad row is made the first time a settings route sees
//!   an id the Member doesn't have ([`Store::ensure_device`]; ordinary API calls never make one),
//!   at most [`MAX_DEVICES`] that aren't forgotten. [`Store::touch_device`] stamps `last_seen_at`
//!   at most once a minute and the Token's `device_id`, outside the commit hook, like a Token's
//!   `last_used_at`. Forget revokes the Tokens and ends the sessions last used from the Device;
//!   the row and its values stay (Copy from another device still reads them), and its id is
//!   refused from then on.
//! - **Settings**: `member_settings` (user), `all_device_settings` (All devices),
//!   `device_settings` (a Device's own value; a NULL value is a tombstone). Every write goes
//!   through `Store::tx` + `append_event` and appends `setting.changed {member, key, scope,
//!   device?, stamp}` — never the value — or nothing when it changes nothing.

use std::collections::BTreeMap;

use rusqlite::{OptionalExtension, Transaction, params};
use serde_json::{Value, json};

use crate::error::{Result, StoreError};
use crate::settings::{
    self, MAX_AHEAD_MS, Refusal, SettingDef, SettingKind, Stamp, ValueType, WEB_DEVICE, is_device_id, wall_ms,
};
use crate::store::{Store, append_event, load_member};
use crate::time;

/// Most Devices a Member may have that aren't forgotten (Web included).
pub const MAX_DEVICES: usize = 20;
/// `last_seen_at` is written at most this often.
pub const SEEN_EVERY_SECS: i64 = 60;
/// Longest Device name.
pub const DEVICE_NAME_MAX: usize = 80;

/// One Device of a Member.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub created_at: String,
    pub last_seen_at: Option<String>,
    pub forgotten_at: Option<String>,
}

impl DeviceView {
    pub fn is_web(&self) -> bool {
        self.id == WEB_DEVICE
    }

    pub fn to_json(&self) -> Value {
        json!({
            "id": self.id, "name": self.name, "kind": self.kind, "created_at": self.created_at,
            "last_seen_at": self.last_seen_at, "forgotten": self.forgotten_at.is_some(),
        })
    }
}

/// What [`Store::touch_device`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceTouch {
    /// The Member has no Device with that id (nothing written).
    Unknown,
    Known,
    Forgotten,
}

/// One stored value with its stamp; `value: None` is a tombstone (Device rows only).
#[derive(Clone, Debug, PartialEq)]
pub struct StoredValue {
    pub value: Option<Value>,
    pub stamp: String,
}

impl StoredValue {
    pub fn to_json(&self) -> Value {
        json!({ "value": self.value, "stamp": self.stamp })
    }
}

/// Where a write applies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    User,
    All,
    Devices(Vec<String>),
}

impl Scope {
    pub fn word(&self) -> &'static str {
        match self {
            Scope::User => "user",
            Scope::All => "all",
            Scope::Devices(_) => "devices",
        }
    }
}

/// A `PUT /api/v1/me/settings`.
#[derive(Clone, Debug)]
pub struct PutSetting {
    pub key: String,
    /// `null` at scope `devices` writes a tombstone.
    pub value: Value,
    pub scope: Scope,
    /// A client's stamp (an iOS sync push), kept as given; `None` = the server stamps it.
    pub stamp: Option<String>,
}

/// What a write did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PutOutcome {
    pub changed: bool,
    /// The stamp written (`None` when nothing changed, or for a display name).
    pub stamp: Option<String>,
}

/// A setting resolved for one Device: `source` is `default`, `all`, `device` or `user`.
#[derive(Clone, Debug)]
pub struct Resolved {
    pub def: &'static SettingDef,
    pub value: Value,
    pub source: &'static str,
}

/// One device setting across a Member's Devices (what the web picker and Mixed read).
#[derive(Clone, Debug)]
pub struct MatrixEntry {
    pub def: &'static SettingDef,
    pub all: Option<StoredValue>,
    /// Each non-forgotten Device's own row (tombstones included), by id.
    pub own: BTreeMap<String, StoredValue>,
    /// Each non-forgotten Device's resolved value and source, by id.
    pub resolved: BTreeMap<String, (Value, &'static str)>,
    /// The Devices the setting applies to resolve to more than one value.
    pub mixed: bool,
}

impl MatrixEntry {
    pub fn to_json(&self) -> Value {
        let own: serde_json::Map<String, Value> = self.own.iter().map(|(k, v)| (k.clone(), v.to_json())).collect();
        let resolved: serde_json::Map<String, Value> =
            self.resolved.iter().map(|(k, (v, src))| (k.clone(), json!({ "value": v, "source": src }))).collect();
        json!({
            "key": self.def.key,
            "all": self.all.as_ref().map(StoredValue::to_json),
            "default": self.def.default_value(),
            "devices": own,
            "resolved": resolved,
            "mixed": self.mixed,
        })
    }
}

fn refuse(r: Refusal) -> StoreError {
    StoreError::Setting(r)
}

fn clean_device_name(name: &str) -> Result<String> {
    let n = name.trim();
    if n.is_empty() || n.chars().any(char::is_control) {
        return Err(StoreError::Invalid {
            what: "Device name",
            reason: "must not be empty or hold control characters".into(),
        });
    }
    if n.chars().count() > DEVICE_NAME_MAX {
        return Err(StoreError::Invalid {
            what: "Device name",
            reason: format!("at most {DEVICE_NAME_MAX} characters"),
        });
    }
    Ok(n.to_string())
}

fn default_name(kind: &str) -> &'static str {
    match kind {
        "ipad" => "iPad",
        "cli" => "CLI",
        "web" => "Web",
        _ => "iPhone",
    }
}

fn row_to_device(r: &rusqlite::Row<'_>) -> rusqlite::Result<DeviceView> {
    Ok(DeviceView {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        created_at: r.get(3)?,
        last_seen_at: r.get(4)?,
        forgotten_at: r.get(5)?,
    })
}

const DEVICE_COLS: &str = "id, name, kind, created_at, last_seen_at, forgotten_at";

fn load_device(tx: &Transaction, member_id: &str, id: &str) -> Result<Option<DeviceView>> {
    Ok(tx
        .query_row(
            &format!("SELECT {DEVICE_COLS} FROM devices WHERE member_id = ?1 AND id = ?2"),
            params![member_id, id],
            row_to_device,
        )
        .optional()?)
}

/// Insert the Web Device for `member_id` unless it is there (no event: it exists for every
/// Member who signs in to a page). Called by `open_session` and [`Store::ensure_device`].
pub(crate) fn ensure_web(tx: &Transaction, member_id: &str) -> Result<()> {
    let now = time::now();
    tx.execute(
        "INSERT OR IGNORE INTO devices(member_id, id, name, kind, created_at, last_seen_at)
         VALUES (?1, 'web', 'Web', 'web', ?2, ?2)",
        params![member_id, now],
    )?;
    Ok(())
}

fn seen_cutoff() -> String {
    let t = jiff::Timestamp::now()
        .checked_sub(jiff::SignedDuration::from_secs(SEEN_EVERY_SECS))
        .unwrap_or(jiff::Timestamp::UNIX_EPOCH);
    time::format(t)
}

/// Stamp the Web Device seen (at most once a minute) inside the caller's (hook-free) transaction.
pub(crate) fn touch_web(tx: &Transaction, member_id: &str) -> Result<()> {
    tx.execute(
        "UPDATE devices SET last_seen_at = ?1
         WHERE member_id = ?2 AND id = 'web' AND (last_seen_at IS NULL OR last_seen_at < ?3)",
        params![time::now(), member_id, seen_cutoff()],
    )?;
    Ok(())
}

// ---------------------------------------------------------------- the server's clock

fn server_writer(tx: &Transaction) -> Result<String> {
    let tenant: Option<String> =
        tx.query_row("SELECT value FROM settings WHERE key = 'tenant_id'", [], |r| r.get(0)).optional()?;
    Ok(format!("s{}", tenant.unwrap_or_else(|| "local".into())))
}

fn hlc_last(tx: &Transaction) -> Result<Option<Stamp>> {
    let s: Option<String> =
        tx.query_row("SELECT value FROM settings WHERE key = 'hlc'", [], |r| r.get(0)).optional()?;
    Ok(s.as_deref().and_then(Stamp::parse))
}

fn hlc_store(tx: &Transaction, s: &Stamp) -> Result<()> {
    tx.execute(
        "INSERT INTO settings(key, value) VALUES ('hlc', ?1) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        [s.to_string()],
    )?;
    Ok(())
}

/// The server's next stamp: later than every stamp it issued or accepted.
fn hlc_next(tx: &Transaction) -> Result<Stamp> {
    let writer = server_writer(tx)?;
    let wall = wall_ms();
    let (ms, counter) = match hlc_last(tx)? {
        Some(l) if l.ms >= wall => (l.ms, l.counter.saturating_add(1)),
        _ => (wall, 0),
    };
    let s = Stamp { ms, counter, writer };
    hlc_store(tx, &s)?;
    Ok(s)
}

/// Move the server's clock past a stamp it accepts.
fn hlc_observe(tx: &Transaction, remote: &Stamp) -> Result<()> {
    let last = hlc_last(tx)?;
    if last.as_ref().is_none_or(|l| remote.clock_after(l)) {
        let writer = server_writer(tx)?;
        hlc_store(tx, &Stamp { ms: remote.ms, counter: remote.counter, writer })?;
    }
    Ok(())
}

fn parse_stamp(s: &str) -> Result<Stamp> {
    Stamp::parse(s)
        .ok_or_else(|| StoreError::Invalid { what: "stamp", reason: format!("{s:?} is not MS-COUNTER-WRITER") })
}

// ---------------------------------------------------------------- rows

fn from_json(text: Option<String>) -> Result<Option<Value>> {
    Ok(match text {
        Some(t) => Some(serde_json::from_str(&t)?),
        None => None,
    })
}

fn user_row(tx: &Transaction, member: &str, key: &str) -> Result<Option<StoredValue>> {
    let r: Option<(String, String)> = tx
        .query_row(
            "SELECT value, stamp FROM member_settings WHERE member_id = ?1 AND key = ?2",
            params![member, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    r.map(|(v, s)| Ok(StoredValue { value: Some(serde_json::from_str(&v)?), stamp: s })).transpose()
}

fn all_row(tx: &Transaction, member: &str, key: &str) -> Result<Option<StoredValue>> {
    let r: Option<(String, String)> = tx
        .query_row(
            "SELECT value, stamp FROM all_device_settings WHERE member_id = ?1 AND key = ?2",
            params![member, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    r.map(|(v, s)| Ok(StoredValue { value: Some(serde_json::from_str(&v)?), stamp: s })).transpose()
}

fn device_row(tx: &Transaction, member: &str, device: &str, key: &str) -> Result<Option<StoredValue>> {
    let r: Option<(Option<String>, String)> = tx
        .query_row(
            "SELECT value, stamp FROM device_settings WHERE member_id = ?1 AND device_id = ?2 AND key = ?3",
            params![member, device, key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    r.map(|(v, s)| Ok(StoredValue { value: from_json(v)?, stamp: s })).transpose()
}

fn write_user(tx: &Transaction, member: &str, key: &str, value: &Value, stamp: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO member_settings(member_id, key, value, stamp) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(member_id, key) DO UPDATE SET value = excluded.value, stamp = excluded.stamp",
        params![member, key, value.to_string(), stamp],
    )?;
    Ok(())
}

fn write_all(tx: &Transaction, member: &str, key: &str, value: &Value, stamp: &str) -> Result<()> {
    tx.execute(
        "INSERT INTO all_device_settings(member_id, key, value, stamp) VALUES (?1, ?2, ?3, ?4)
         ON CONFLICT(member_id, key) DO UPDATE SET value = excluded.value, stamp = excluded.stamp",
        params![member, key, value.to_string(), stamp],
    )?;
    Ok(())
}

fn write_device(
    tx: &Transaction,
    member: &str,
    device: &str,
    key: &str,
    value: Option<&Value>,
    stamp: &str,
) -> Result<()> {
    tx.execute(
        "INSERT INTO device_settings(member_id, device_id, key, value, stamp) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(member_id, device_id, key) DO UPDATE SET value = excluded.value, stamp = excluded.stamp",
        params![member, device, key, value.map(Value::to_string), stamp],
    )?;
    Ok(())
}

fn changed_event(
    tx: &Transaction,
    member: &str,
    key: &str,
    scope: &str,
    device: Option<&str>,
    stamp: &str,
) -> Result<()> {
    let mut p = json!({ "member": member, "key": key, "scope": scope, "stamp": stamp });
    if let Some(d) = device {
        p["device"] = json!(d);
    }
    append_event(tx, Some(member), "setting.changed", None, p)?;
    Ok(())
}

fn live_devices(tx: &Transaction, member: &str) -> Result<Vec<DeviceView>> {
    let mut stmt = tx.prepare(&format!(
        "SELECT {DEVICE_COLS} FROM devices WHERE member_id = ?1 AND forgotten_at IS NULL ORDER BY created_at, id"
    ))?;
    let rows = stmt.query_map([member], row_to_device)?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

fn vault_exists(tx: &Transaction, id: &str) -> Result<bool> {
    Ok(tx.query_row("SELECT 1 FROM vaults WHERE id = ?1", [id], |_| Ok(())).optional()?.is_some())
}

/// A stored Default Vault that no longer exists falls back to the default (Any).
fn live_value(tx: &Transaction, def: &SettingDef, v: Value) -> Result<Option<Value>> {
    if matches!(def.ty, ValueType::Vault)
        && let Some(id) = v.as_str()
        && id != "any"
        && !vault_exists(tx, id)?
    {
        return Ok(None);
    }
    Ok(Some(v))
}

/// The stamp a client pushed, checked against the server's clock (409 `clock_skew`).
fn accept_stamp(tx: &Transaction, s: &str) -> Result<Stamp> {
    let st = parse_stamp(s)?;
    let wall = wall_ms();
    if st.ms > wall + MAX_AHEAD_MS {
        return Err(StoreError::ClockSkew { stamp: st.to_string(), server_ms: wall });
    }
    hlc_observe(tx, &st)?;
    Ok(st)
}

/// One device setting as iOS syncs it: the setting, the Device's own row, the All devices row.
pub type SyncRow = (&'static SettingDef, Option<StoredValue>, Option<StoredValue>);

/// A pushed stamp not newer than the stored one: a no-op when the value is the same, else 409
/// `stale` with what is stored.
fn stale_or_noop(key: &str, pushed: &Stamp, value: Option<&Value>, stored: &StoredValue) -> Result<bool> {
    let stored_stamp = parse_stamp(&stored.stamp)?;
    if *pushed > stored_stamp {
        return Ok(false);
    }
    if stored.value.as_ref() == value {
        return Ok(true);
    }
    Err(StoreError::Stale {
        key: key.to_string(),
        stamp: pushed.to_string(),
        stored_value: stored.value.clone().map(Box::new),
        stored_stamp: stored.stamp.clone(),
    })
}

impl Store {
    // ---------------------------------------------------------------- devices

    /// The Member's Devices, oldest first (forgotten ones only when asked).
    pub fn list_devices(&self, member_id: &str, include_forgotten: bool) -> Result<Vec<DeviceView>> {
        let conn = self.conn();
        let sql = format!(
            "SELECT {DEVICE_COLS} FROM devices WHERE member_id = ?1 {} ORDER BY created_at, id",
            if include_forgotten { "" } else { "AND forgotten_at IS NULL" }
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map([member_id], row_to_device)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// One Device of the Member, forgotten or not.
    pub fn device(&self, member_id: &str, id: &str) -> Result<Option<DeviceView>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                &format!("SELECT {DEVICE_COLS} FROM devices WHERE member_id = ?1 AND id = ?2"),
                params![member_id, id],
                row_to_device,
            )
            .optional()?)
    }

    /// The Device a settings route names, made on first use (an upsert inside one write
    /// transaction, so two first requests cannot race): `web` always exists once asked for; an
    /// unknown id gets a row named from `info` (`kind`, `name`; defaults to an iPhone), unless
    /// the Member already has [`MAX_DEVICES`] (`TooManyDevices`). A forgotten id is refused
    /// (`DeviceForgotten`), a malformed one too (`BadDevice`). Appends `device.added {member,
    /// device, kind}` when it made one.
    pub fn ensure_device(&self, member_id: &str, id: &str, info: Option<(&str, &str)>) -> Result<DeviceView> {
        if !is_device_id(id) {
            return Err(StoreError::BadDevice(id.to_string()));
        }
        if let Some(d) = self.device(member_id, id)? {
            return match d.forgotten_at {
                Some(_) => Err(StoreError::DeviceForgotten(id.to_string())),
                None => Ok(d),
            };
        }
        self.tx(|tx| {
            if let Some(d) = load_device(tx, member_id, id)? {
                return match d.forgotten_at {
                    Some(_) => Err(StoreError::DeviceForgotten(id.to_string())),
                    None => Ok(d),
                };
            }
            if id == WEB_DEVICE {
                ensure_web(tx, member_id)?;
            } else {
                let live: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM devices WHERE member_id = ?1 AND forgotten_at IS NULL",
                    [member_id],
                    |r| r.get(0),
                )?;
                if live as usize >= MAX_DEVICES {
                    return Err(StoreError::TooManyDevices(MAX_DEVICES));
                }
                let kind = info.map(|i| i.0).filter(|k| matches!(*k, "iphone" | "ipad" | "cli")).unwrap_or("iphone");
                let name = info.and_then(|i| clean_device_name(i.1).ok()).unwrap_or_else(|| default_name(kind).into());
                let now = time::now();
                tx.execute(
                    "INSERT INTO devices(member_id, id, name, kind, created_at, last_seen_at) VALUES (?1, ?2, ?3, ?4, ?5, ?5)
                     ON CONFLICT(member_id, id) DO NOTHING",
                    params![member_id, id, name, kind, now],
                )?;
            }
            let d = load_device(tx, member_id, id)?.ok_or_else(|| StoreError::DeviceNotFound(id.to_string()))?;
            append_event(
                tx,
                Some(member_id),
                "device.added",
                None,
                json!({ "member": member_id, "device": id, "kind": d.kind }),
            )?;
            Ok(d)
        })
    }

    /// A request named Device `id` (`X-Annotary-Device`): stamp its `last_seen_at` (at most
    /// once a minute) and, for a Token, the Token's `device_id`. Never makes a Device and never
    /// fires the commit hook, so it wakes neither `wait` nor the live feed.
    pub fn touch_device(&self, member_id: &str, id: &str, token_id: Option<&str>) -> Result<DeviceTouch> {
        let mut conn = self.conn();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(d) = load_device(&tx, member_id, id)? else { return Ok(DeviceTouch::Unknown) };
        if d.forgotten_at.is_some() {
            return Ok(DeviceTouch::Forgotten);
        }
        tx.execute(
            "UPDATE devices SET last_seen_at = ?1
             WHERE member_id = ?2 AND id = ?3 AND (last_seen_at IS NULL OR last_seen_at < ?4)",
            params![time::now(), member_id, id, seen_cutoff()],
        )?;
        if let Some(t) = token_id {
            tx.execute(
                "UPDATE tokens SET device_id = ?1 WHERE id = ?2 AND member_id = ?3 AND device_id IS NOT ?1",
                params![id, t, member_id],
            )?;
        }
        tx.commit()?;
        Ok(DeviceTouch::Known)
    }

    /// Rename one of the Member's Devices (`device.renamed {member, device}`). Web can't be
    /// renamed (`WebDevice`); a no-op rename appends nothing.
    pub fn rename_device(&self, member_id: &str, id: &str, name: &str) -> Result<DeviceView> {
        if id == WEB_DEVICE {
            return Err(StoreError::WebDevice);
        }
        let name = clean_device_name(name)?;
        self.tx(|tx| {
            let d = load_device(tx, member_id, id)?.ok_or_else(|| StoreError::DeviceNotFound(id.to_string()))?;
            if d.forgotten_at.is_some() {
                return Err(StoreError::DeviceForgotten(id.to_string()));
            }
            if d.name == name {
                return Ok(d);
            }
            tx.execute("UPDATE devices SET name = ?1 WHERE member_id = ?2 AND id = ?3", params![name, member_id, id])?;
            append_event(tx, Some(member_id), "device.renamed", None, json!({ "member": member_id, "device": id }))?;
            load_device(tx, member_id, id)?.ok_or_else(|| StoreError::DeviceNotFound(id.to_string()))
        })
    }

    /// Forget a Device: revoke the Member's Tokens and end the sessions last used from it, and
    /// mark it forgotten (`device.forgotten {member, device, tokens, sessions}`). Its row and
    /// values stay. Web can't be forgotten (`WebDevice`). `Ok(false)` when it already was.
    pub fn forget_device(&self, member_id: &str, id: &str) -> Result<(bool, usize, usize)> {
        if id == WEB_DEVICE {
            return Err(StoreError::WebDevice);
        }
        self.tx(|tx| {
            let d = load_device(tx, member_id, id)?.ok_or_else(|| StoreError::DeviceNotFound(id.to_string()))?;
            if d.forgotten_at.is_some() {
                return Ok((false, 0, 0));
            }
            let now = time::now();
            let tokens = tx.execute(
                "UPDATE tokens SET revoked_at = ?1 WHERE member_id = ?2 AND device_id = ?3 AND revoked_at IS NULL",
                params![now, member_id, id],
            )?;
            let sessions =
                tx.execute("DELETE FROM sessions WHERE member_id = ?1 AND device_id = ?2", params![member_id, id])?;
            tx.execute(
                "UPDATE devices SET forgotten_at = ?1 WHERE member_id = ?2 AND id = ?3",
                params![now, member_id, id],
            )?;
            append_event(
                tx,
                Some(member_id),
                "device.forgotten",
                None,
                json!({ "member": member_id, "device": id, "tokens": tokens, "sessions": sessions }),
            )?;
            Ok((true, tokens, sessions))
        })
    }

    // ---------------------------------------------------------------- reading settings

    /// Every setting resolved for `device` (`None`: no Device — device settings resolve from All
    /// devices, else the default): the Device's own value, else All devices, else the default.
    pub fn resolved_settings(&self, member_id: &str, device: Option<&str>) -> Result<Vec<Resolved>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let m = load_member(&tx, member_id)?.ok_or(StoreError::MemberNotFound)?;
        let mut out = Vec::new();
        for def in settings::CATALOG {
            out.push(resolve_one(&tx, member_id, &m.display_name, def, device)?);
        }
        Ok(out)
    }

    /// One setting resolved for `device`.
    pub fn resolved_setting(&self, member_id: &str, key: &str, device: Option<&str>) -> Result<Resolved> {
        let def = settings::setting(key).ok_or_else(|| refuse(Refusal::UnknownSetting(key.to_string())))?;
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let m = load_member(&tx, member_id)?.ok_or(StoreError::MemberNotFound)?;
        resolve_one(&tx, member_id, &m.display_name, def, device)
    }

    /// Each device setting's All devices value and every non-forgotten Device's own value, what
    /// each resolves to and whether they disagree.
    pub fn settings_matrix(&self, member_id: &str) -> Result<(Vec<DeviceView>, Vec<MatrixEntry>)> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let devices = live_devices(&tx, member_id)?;
        let mut out = Vec::new();
        for def in settings::device_settings() {
            out.push(matrix_entry(&tx, member_id, def, &devices)?);
        }
        Ok((devices, out))
    }

    /// One device setting's matrix entry.
    pub fn matrix_entry(&self, member_id: &str, key: &str) -> Result<Option<MatrixEntry>> {
        let Some(def) = settings::setting(key).filter(|d| d.kind == SettingKind::Device) else { return Ok(None) };
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let devices = live_devices(&tx, member_id)?;
        Ok(Some(matrix_entry(&tx, member_id, def, &devices)?))
    }

    /// What iOS syncs from: for each device setting, the Device's own row (or absent) and the
    /// All devices row (or absent).
    pub fn device_sync(&self, member_id: &str, device: &str) -> Result<Vec<SyncRow>> {
        let mut conn = self.conn();
        let tx = conn.transaction()?;
        let mut out = Vec::new();
        for def in settings::device_settings() {
            out.push((def, device_row(&tx, member_id, device, def.key)?, all_row(&tx, member_id, def.key)?));
        }
        Ok(out)
    }

    // ---------------------------------------------------------------- writing settings

    /// Write one setting (`PUT /api/v1/me/settings`). See the module docs and the plan: at scope
    /// `all` the All devices value is written and every non-forgotten Device's own value is
    /// tombstoned, with one server stamp later than every stamp it replaces; at scope `devices`
    /// each named Device gets its own value (`null` = a tombstone). Without `stamp` the server
    /// stamps the change; with one (an iOS push) it is kept, refused when more than five
    /// minutes ahead (`ClockSkew`) or not newer than the stored one (`Stale`; the same value is a
    /// no-op). A write that changes nothing appends no event.
    pub fn put_setting(&self, member_id: &str, put: &PutSetting) -> Result<PutOutcome> {
        let def = settings::setting(&put.key).ok_or_else(|| refuse(Refusal::UnknownSetting(put.key.clone())))?;
        let tombstone = put.value.is_null();
        match (def.kind, &put.scope) {
            (SettingKind::User, Scope::User) | (SettingKind::Device, Scope::All | Scope::Devices(_)) => {}
            (SettingKind::User, _) => {
                return Err(refuse(Refusal::Invalid {
                    key: def.key,
                    reason: "a user setting takes scope user".into(),
                }));
            }
            (SettingKind::Device, Scope::User) => {
                return Err(refuse(Refusal::Invalid {
                    key: def.key,
                    reason: "a device setting takes scope all or devices".into(),
                }));
            }
        }
        if tombstone && !matches!(put.scope, Scope::Devices(_)) {
            return Err(refuse(Refusal::Invalid { key: def.key, reason: "null is only a Device's tombstone".into() }));
        }
        let value = if tombstone { Value::Null } else { settings::validate(def, &put.value).map_err(refuse)? };
        if put.stamp.is_some() && matches!(put.scope, Scope::User) {
            // User settings are not synced by a device; a reset deletes the row and leaves no
            // stamp an older push could be compared with, so the server stamps every user write.
            return Err(refuse(Refusal::Invalid {
                key: def.key,
                reason: "a user setting takes no stamp (the server stamps it)".into(),
            }));
        }
        if matches!(def.ty, ValueType::Name) {
            let before = self.member_by_id(member_id)?.ok_or(StoreError::MemberNotFound)?;
            let after = self.rename_member(member_id, member_id, value.as_str().unwrap_or_default())?;
            return Ok(PutOutcome { changed: before.display_name != after.display_name, stamp: None });
        }
        self.tx(|tx| {
            if matches!(def.ty, ValueType::Vault)
                && let Some(id) = value.as_str()
                && id != "any"
                && !vault_exists(tx, id)?
            {
                return Err(StoreError::VaultNotFound(id.to_string()));
            }
            write_one(tx, member_id, def, &put.scope, &value, put.stamp.as_deref(), false)
        })
    }

    /// Reset (`DELETE /api/v1/me/settings/{key}`): a user setting back to its default (its row
    /// removed); All devices back to the default (written like any All devices change); a
    /// Device's own value tombstoned so it follows All devices. The server stamps it.
    pub fn reset_setting(&self, member_id: &str, key: &str, scope: &Scope) -> Result<PutOutcome> {
        let def = settings::setting(key).ok_or_else(|| refuse(Refusal::UnknownSetting(key.to_string())))?;
        if matches!(def.ty, ValueType::Name) {
            return Err(refuse(Refusal::Invalid { key: def.key, reason: "a display name has no default".into() }));
        }
        match (def.kind, scope) {
            (SettingKind::User, Scope::User) => self.tx(|tx| {
                if user_row(tx, member_id, key)?.is_none() {
                    return Ok(PutOutcome { changed: false, stamp: None });
                }
                tx.execute("DELETE FROM member_settings WHERE member_id = ?1 AND key = ?2", params![member_id, key])?;
                let s = hlc_next(tx)?.to_string();
                changed_event(tx, member_id, key, "user", None, &s)?;
                Ok(PutOutcome { changed: true, stamp: Some(s) })
            }),
            (SettingKind::Device, Scope::All) => {
                let put = PutSetting { key: key.into(), value: def.default_value(), scope: Scope::All, stamp: None };
                self.put_setting(member_id, &put)
            }
            (SettingKind::Device, Scope::Devices(_)) => {
                let put = PutSetting { key: key.into(), value: Value::Null, scope: scope.clone(), stamp: None };
                self.put_setting(member_id, &put)
            }
            _ => Err(refuse(Refusal::Invalid {
                key: def.key,
                reason: format!("{} setting: wrong scope {}", def.kind.as_str(), scope.word()),
            })),
        }
    }

    /// Copy from another device: every device setting of Device `from` (those its kind has) onto
    /// `to` as new changes, stamped now. Onto a Device: `from`'s own values, and a tombstone where `from` follows All
    /// devices. Onto All devices (`to` = `None`): what `from` resolves to becomes the All devices
    /// value (tombstoning every Device's own value). `from` may be forgotten; `to` may not.
    pub fn copy_device_settings(&self, member_id: &str, from: &str, to: Option<&str>) -> Result<usize> {
        if to == Some(from) {
            return Err(StoreError::Invalid { what: "copy", reason: "pick another Device to copy from".into() });
        }
        self.tx(|tx| {
            let source =
                load_device(tx, member_id, from)?.ok_or_else(|| StoreError::DeviceNotFound(from.to_string()))?;
            if let Some(t) = to {
                let d = load_device(tx, member_id, t)?.ok_or_else(|| StoreError::DeviceNotFound(t.to_string()))?;
                if d.forgotten_at.is_some() {
                    return Err(StoreError::DeviceForgotten(t.to_string()));
                }
            }
            let mut n = 0;
            for def in settings::device_settings() {
                // A setting the source kind doesn't have (an iPhone's keyboard settings) is left
                // as it is on the target.
                if !def.applies_to(&source.kind) {
                    continue;
                }
                let own = device_row(tx, member_id, from, def.key)?.and_then(|r| r.value);
                match to {
                    Some(t) => {
                        let v = own.unwrap_or(Value::Null);
                        write_one(tx, member_id, def, &Scope::Devices(vec![t.to_string()]), &v, None, true)?;
                    }
                    None => {
                        let v = match own {
                            Some(v) => v,
                            None => all_row(tx, member_id, def.key)?
                                .and_then(|r| r.value)
                                .unwrap_or_else(|| def.default_value()),
                        };
                        write_one(tx, member_id, def, &Scope::All, &v, None, true)?;
                    }
                }
                n += 1;
            }
            Ok(n)
        })
    }
}

fn resolve_one(
    tx: &Transaction,
    member: &str,
    display_name: &str,
    def: &'static SettingDef,
    device: Option<&str>,
) -> Result<Resolved> {
    if matches!(def.ty, ValueType::Name) {
        return Ok(Resolved { def, value: json!(display_name), source: "user" });
    }
    match def.kind {
        SettingKind::User => {
            if let Some(v) = user_row(tx, member, def.key)?.and_then(|r| r.value)
                && let Some(v) = live_value(tx, def, v)?
            {
                return Ok(Resolved { def, value: v, source: "user" });
            }
            Ok(Resolved { def, value: def.default_value(), source: "default" })
        }
        SettingKind::Device => {
            if let Some(d) = device
                && let Some(v) = device_row(tx, member, d, def.key)?.and_then(|r| r.value)
            {
                return Ok(Resolved { def, value: v, source: "device" });
            }
            if let Some(v) = all_row(tx, member, def.key)?.and_then(|r| r.value) {
                return Ok(Resolved { def, value: v, source: "all" });
            }
            Ok(Resolved { def, value: def.default_value(), source: "default" })
        }
    }
}

fn matrix_entry(
    tx: &Transaction,
    member: &str,
    def: &'static SettingDef,
    devices: &[DeviceView],
) -> Result<MatrixEntry> {
    let all = all_row(tx, member, def.key)?;
    let fallback: (Value, &'static str) = match all.as_ref().and_then(|a| a.value.clone()) {
        Some(v) => (v, "all"),
        None => (def.default_value(), "default"),
    };
    let mut own = BTreeMap::new();
    let mut resolved = BTreeMap::new();
    let mut seen: Vec<Value> = Vec::new();
    for d in devices {
        let row = device_row(tx, member, &d.id, def.key)?;
        let r = match row.as_ref().and_then(|r| r.value.clone()) {
            Some(v) => (v, "device"),
            None => fallback.clone(),
        };
        if def.applies_to(&d.kind) && !seen.contains(&r.0) {
            seen.push(r.0.clone());
        }
        if let Some(row) = row {
            own.insert(d.id.clone(), row);
        }
        resolved.insert(d.id.clone(), r);
    }
    Ok(MatrixEntry { def, all, own, resolved, mixed: seen.len() > 1 })
}

/// One write inside the caller's transaction (see [`Store::put_setting`]). `force` writes and
/// stamps even when the value is the same (Copy from another device: "as new changes").
fn write_one(
    tx: &Transaction,
    member: &str,
    def: &'static SettingDef,
    scope: &Scope,
    value: &Value,
    stamp: Option<&str>,
    force: bool,
) -> Result<PutOutcome> {
    let key = def.key;
    if stamp.is_some() && matches!(scope, Scope::User) {
        // User settings are not synced by a device; a reset deletes the row and leaves no stamp
        // an older push could be compared with, so the server stamps every user write.
        return Err(refuse(Refusal::Invalid {
            key,
            reason: "a user setting takes no stamp (the server stamps it)".into(),
        }));
    }
    let pushed = match stamp {
        Some(s) => Some(accept_stamp(tx, s)?),
        None => None,
    };
    match scope {
        Scope::User => {
            let row = user_row(tx, member, key)?;
            if let Some(p) = &pushed {
                if let Some(r) = &row
                    && stale_or_noop(key, p, Some(value), r)?
                {
                    return Ok(PutOutcome { changed: false, stamp: None });
                }
            } else if !force && row.as_ref().and_then(|r| r.value.as_ref()) == Some(value) {
                return Ok(PutOutcome { changed: false, stamp: None });
            }
            let s = match pushed {
                Some(p) => p.to_string(),
                None => hlc_next(tx)?.to_string(),
            };
            write_user(tx, member, key, value, &s)?;
            changed_event(tx, member, key, "user", None, &s)?;
            Ok(PutOutcome { changed: true, stamp: Some(s) })
        }
        Scope::All => {
            let row = all_row(tx, member, key)?;
            let devices = live_devices(tx, member)?;
            let mut overrides = Vec::new();
            for d in &devices {
                if let Some(r) = device_row(tx, member, &d.id, key)?
                    && r.value.is_some()
                {
                    overrides.push((d.id.clone(), r));
                }
            }
            if let Some(p) = &pushed
                && let Some(r) = &row
                && stale_or_noop(key, p, Some(value), r)?
            {
                // A pushed stamp not newer than the stored All value, with the same value: the
                // All row (and its newer stamp) stays; only own values older than the push are
                // tombstoned, with the pushed stamp.
                let mut changed = false;
                for (id, o) in &overrides {
                    if parse_stamp(&o.stamp).is_ok_and(|os| os < *p) {
                        write_device(tx, member, id, key, None, &p.to_string())?;
                        changed = true;
                    }
                }
                if !changed {
                    return Ok(PutOutcome { changed: false, stamp: None });
                }
                let s = p.to_string();
                changed_event(tx, member, key, "all", None, &s)?;
                return Ok(PutOutcome { changed: true, stamp: Some(s) });
            }
            if pushed.is_none()
                && !force
                && row.as_ref().and_then(|r| r.value.as_ref()) == Some(value)
                && overrides.is_empty()
            {
                return Ok(PutOutcome { changed: false, stamp: None });
            }
            let st = match pushed {
                Some(p) => p,
                None => hlc_next(tx)?,
            };
            let s = st.to_string();
            write_all(tx, member, key, value, &s)?;
            for (id, o) in &overrides {
                if parse_stamp(&o.stamp).is_ok_and(|os| os < st) {
                    write_device(tx, member, id, key, None, &s)?;
                }
            }
            changed_event(tx, member, key, "all", None, &s)?;
            Ok(PutOutcome { changed: true, stamp: Some(s) })
        }
        Scope::Devices(ids) => {
            if ids.is_empty() {
                return Err(refuse(Refusal::Invalid { key, reason: "scope devices names at least one Device".into() }));
            }
            let v = if value.is_null() { None } else { Some(value) };
            let mut plan = Vec::new();
            for id in ids {
                let d = load_device(tx, member, id)?.ok_or_else(|| StoreError::DeviceNotFound(id.clone()))?;
                if d.forgotten_at.is_some() {
                    return Err(StoreError::DeviceForgotten(id.clone()));
                }
                let row = device_row(tx, member, id, key)?;
                if let Some(p) = &pushed {
                    if let Some(r) = &row
                        && stale_or_noop(key, p, v, r)?
                    {
                        continue;
                    }
                } else if !force && row.as_ref().map(|r| r.value.as_ref()).unwrap_or(None) == v {
                    continue;
                }
                plan.push(id.clone());
            }
            if plan.is_empty() {
                return Ok(PutOutcome { changed: false, stamp: None });
            }
            let s = match pushed {
                Some(p) => p.to_string(),
                None => hlc_next(tx)?.to_string(),
            };
            for id in &plan {
                write_device(tx, member, id, key, v, &s)?;
                changed_event(tx, member, key, "devices", Some(id), &s)?;
            }
            Ok(PutOutcome { changed: true, stamp: Some(s) })
        }
    }
}
