//! `token mint [--name NAME] [--expires DUR|DATE] [--member HANDLE]`, `token list [--member
//! HANDLE|*] [--all]`, `token revoke ID` (ticket 06).
//!
//! Tokens belong to a Member: a Member manages their own, an owner anyone's. `mint` prints the
//! Token once as `ANNOTARY_TOKEN=…` (like `member add`); `list` shows `MEMBER NAME CREATED
//! LAST-USED EXPIRES ID` (revoked ones only with `--all`, marked `revoked` in EXPIRES); `revoke`
//! prints `ID @member revoked`.

use clap::{Args, Subcommand};
use serde_json::{Map, Value, json};

use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, iso_seconds, relative_age};

use super::Ctx;
use super::member::clean_handle;

#[derive(Subcommand, Debug)]
pub enum TokenCmd {
    /// Mint a token (yours; --member HANDLE for another Member, owner only), printed once
    Mint(MintArgs),
    /// Tokens with created / last used / expiry (yours; owner: --member HANDLE or everyone's)
    List(ListArgs),
    /// Revoke a token by id — it stops working at once (yours, or an owner)
    Revoke(RevokeArgs),
}

#[derive(Args, Debug)]
pub struct MintArgs {
    /// A label for the token (e.g. "ci", "laptop")
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
    /// Expiry: a duration from now (30d, 12h, 2w) or a date / ISO-8601 time; default: never
    #[arg(long, value_name = "WHEN")]
    pub expires: Option<String>,
    /// Mint for another Member (owner only); default: yourself
    #[arg(long, value_name = "HANDLE")]
    pub member: Option<String>,
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Another Member's tokens, or `*` for every Member's (owner only); default: yours
    /// (an owner without --member sees everyone's)
    #[arg(long, value_name = "HANDLE")]
    pub member: Option<String>,
    /// Include revoked tokens
    #[arg(long)]
    pub all: bool,
}

#[derive(Args, Debug)]
pub struct RevokeArgs {
    /// The token id (`token list` shows it)
    pub id: String,
}

pub const MINT_FIELDS: FieldSpec = FieldSpec {
    all: &["token", "id", "member", "name", "expires", "created"],
    default: &["token", "id", "member", "name", "expires"],
};

pub const LIST_FIELDS: FieldSpec = FieldSpec {
    all: &["id", "member", "name", "created", "last_used", "expires", "status", "revoked"],
    default: &["id", "member", "name", "created", "last_used", "expires", "status"],
};

pub const REVOKE_FIELDS: FieldSpec =
    FieldSpec { all: &["id", "member", "revoked", "name"], default: &["id", "member", "revoked"] };

pub fn run(ctx: &Ctx, cmd: TokenCmd) -> CliResult<()> {
    match cmd {
        TokenCmd::Mint(a) => mint(ctx, a),
        TokenCmd::List(a) => list(ctx, a),
        TokenCmd::Revoke(a) => revoke(ctx, a),
    }
}

/// `--expires`: a duration from `now` (`30d`, `12h`, `2w`, `90m`), a date (`2026-12-31`, local
/// midnight) or an ISO-8601 time → the stored timestamp text (must be in the future).
pub fn parse_expiry(text: &str, now: jiff::Timestamp) -> CliResult<String> {
    let t = text.trim();
    let bad = || {
        CliError::usage(format!(
            "--expires {t:?}: use a duration (30d, 12h, 2w), a date (2026-12-31) or an ISO-8601 time"
        ))
    };
    if t.is_empty() {
        return Err(bad());
    }
    let ts = if let Some((num, unit)) = t.split_at_checked(t.len() - 1)
        && let Ok(n) = num.parse::<i64>()
        && n >= 0
    {
        let secs = match unit {
            "s" => n,
            "m" => n * 60,
            "h" => n * 3600,
            "d" => n * 86_400,
            "w" => n * 7 * 86_400,
            _ => return Err(bad()),
        };
        now.checked_add(jiff::SignedDuration::from_secs(secs)).map_err(|_| bad())?
    } else if let Ok(date) = t.parse::<jiff::civil::Date>() {
        date.to_zoned(jiff::tz::TimeZone::system()).map_err(|_| bad())?.timestamp()
    } else if let Some(ts) = annotary_store::time::parse(t) {
        ts
    } else if let Ok(dt) = t.parse::<jiff::civil::DateTime>() {
        dt.to_zoned(jiff::tz::TimeZone::system()).map_err(|_| bad())?.timestamp()
    } else {
        return Err(bad());
    };
    if ts <= now {
        return Err(CliError::usage(format!("--expires {t:?} is not in the future")));
    }
    Ok(annotary_store::time::format(ts))
}

/// A token API object → the flat record for text and JSON.
fn record(t: &Value) -> Map<String, Value> {
    let s = |k: &str| t.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut r = Map::new();
    r.insert("id".into(), json!(s("id")));
    r.insert("member".into(), json!(s("member")));
    r.insert("name".into(), json!(s("name")));
    r.insert("created".into(), json!(iso_seconds(&s("created_at"))));
    r.insert("last_used".into(), json!(iso_seconds(&s("last_used_at"))));
    r.insert("expires".into(), json!(iso_seconds(&s("expires_at"))));
    r.insert("status".into(), json!(s("status")));
    r.insert("revoked".into(), json!(iso_seconds(&s("revoked_at"))));
    r.insert("created_raw".into(), json!(s("created_at")));
    r.insert("last_used_raw".into(), json!(s("last_used_at")));
    r.insert("expires_raw".into(), json!(s("expires_at")));
    if let Some(tok) = t.get("token") {
        r.insert("token".into(), tok.clone());
    }
    r
}

/// The EXPIRES cell: `-` (never), the local date, `expired`, or `revoked`.
fn expires_cell(rec: &Map<String, Value>) -> String {
    let s = |k: &str| rec.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    match s("status").as_str() {
        "revoked" => "revoked".to_string(),
        "expired" => "expired".to_string(),
        _ => {
            let raw = s("expires_raw");
            match annotary_store::time::parse(&raw) {
                Some(t) => t.to_zoned(jiff::tz::TimeZone::system()).strftime("%Y-%m-%d").to_string(),
                None => "-".to_string(),
            }
        }
    }
}

fn mint(ctx: &Ctx, a: MintArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&MINT_FIELDS, Vec::new(), true);
    }
    let mut body = json!({});
    if let Some(n) = a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        body["name"] = json!(n);
    }
    if let Some(e) = &a.expires {
        body["expires_at"] = json!(parse_expiry(e, jiff::Timestamp::now())?);
    }
    if let Some(m) = &a.member {
        body["member"] = json!(clean_handle(m));
    }
    let api = ctx.api()?;
    let created = api.post("/api/v1/tokens", &body)?;
    let rec = record(&created);
    let s = |k: &str| rec.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let token = created.get("token").and_then(Value::as_str).unwrap_or("").to_string();
    let name = if s("name").is_empty() { String::new() } else { format!(" \"{}\"", s("name")) };
    let exp = if s("expires").is_empty() { String::new() } else { format!(", expires {}", expires_cell(&rec)) };
    ctx.out.note(&format!("token minted for @{}{name} (id {}{exp})", s("member"), s("id")));
    ctx.out.note("Token (shown once):");
    if ctx.out.is_json() {
        return ctx.out.json_records(&MINT_FIELDS, vec![rec], true);
    }
    if ctx.out.tty {
        ctx.out.line("");
        ctx.out.line(&format!("  ANNOTARY_TOKEN={token}   # put this in the agent's environment"));
        ctx.out.line("");
    } else {
        ctx.out.line(&format!("ANNOTARY_TOKEN={token}"));
    }
    Ok(())
}

fn list(ctx: &Ctx, a: ListArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&LIST_FIELDS, Vec::new(), false);
    }
    let api = ctx.api()?;
    let path = match a.member.as_deref().map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => format!("/api/v1/tokens?member={}", crate::client::qs(&clean_handle(m))),
        None => "/api/v1/tokens".to_string(),
    };
    let body = api.get(&path)?;
    let tokens = body["tokens"].as_array().cloned().unwrap_or_default();
    let records: Vec<Map<String, Value>> = tokens
        .iter()
        .map(record)
        .filter(|r| a.all || r.get("status").and_then(Value::as_str) != Some("revoked"))
        .collect();
    if ctx.out.is_json() {
        return ctx.out.json_records(&LIST_FIELDS, records, false);
    }
    let now = jiff::Timestamp::now();
    let rows: Vec<Vec<String>> = records
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let name = if s("name").is_empty() { "-".to_string() } else { s("name") };
            let last =
                if s("last_used_raw").is_empty() { "-".to_string() } else { relative_age(&s("last_used_raw"), now) };
            vec![
                format!("@{}", s("member")),
                name,
                relative_age(&s("created_raw"), now),
                last,
                expires_cell(r),
                s("id"),
            ]
        })
        .collect();
    if ctx.out.tty && rows.is_empty() {
        ctx.out.note("no tokens");
        return Ok(());
    }
    let table = ctx.out.table(&["MEMBER", "NAME", "CREATED", "LAST USED", "EXPIRES", "ID"], &rows, &[]);
    print!("{table}");
    Ok(())
}

fn revoke(ctx: &Ctx, a: RevokeArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&REVOKE_FIELDS, Vec::new(), true);
    }
    let id = a.id.trim();
    if id.is_empty() {
        return Err(CliError::usage("ID must not be empty").hint("`annotary token list` shows token ids"));
    }
    let api = ctx.api()?;
    let r = api.delete(&format!("/api/v1/tokens/{}", crate::client::qs(id)))?;
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let revoked_now = r["revoked"].as_bool().unwrap_or(false);
    if !revoked_now {
        ctx.out.note(&format!("token {} was already revoked", s("id")));
    }
    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("id".into(), json!(s("id")));
        rec.insert("member".into(), json!(s("member")));
        rec.insert("revoked".into(), json!(true));
        rec.insert("name".into(), json!(s("name")));
        return ctx.out.json_records(&REVOKE_FIELDS, vec![rec], true);
    }
    let fields = [s("id"), format!("@{}", s("member")), "revoked".to_string()];
    let sep = if ctx.out.tty { "  " } else { "\t" };
    ctx.out.line(&fields.join(sep));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: S06-08
    #[test]
    fn expiry_parses_durations_dates_and_iso() {
        let now = annotary_store::time::parse("2026-08-18T10:00:00.000Z").unwrap();
        assert_eq!(parse_expiry("30d", now).unwrap(), "2026-09-17T10:00:00.000Z");
        assert_eq!(parse_expiry("12h", now).unwrap(), "2026-08-18T22:00:00.000Z");
        assert_eq!(parse_expiry("2w", now).unwrap(), "2026-09-01T10:00:00.000Z");
        assert_eq!(parse_expiry("2027-01-01T00:00:00Z", now).unwrap(), "2027-01-01T00:00:00.000Z");
        assert!(
            parse_expiry("2999-01-01", now).unwrap().starts_with("2998-12-31")
                || parse_expiry("2999-01-01", now).unwrap().starts_with("2999-01-01")
        );
        assert!(parse_expiry("0s", now).is_err(), "not in the future");
        assert!(parse_expiry("2020-01-01", now).is_err());
        assert!(parse_expiry("soon", now).is_err());
        assert!(parse_expiry("", now).is_err());
    }
}
