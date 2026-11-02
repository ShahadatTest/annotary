//! `pair [--member HANDLE] [--name NAME]` (ticket 18): pair a phone or tablet with this server —
//! `POST /api/v1/pairing` mints a Token (named `device` by default; yours, or another Member's
//! for an owner) and returns the `annotary://pair` URL the app scans.
//!
//! - TTY: `Pairing a device for @h on SERVER`, the QR code in Unicode quadrant blocks
//!   (`crate::qr::terminal`), `profile NAME`, the pair URL on its own line; the "shown once" note
//!   (with the `token revoke` command) on stderr.
//! - non-TTY: one line `PAIR_URL  MEMBER  TOKEN_ID  NAME` (tab-separated), no QR.
//! - `--json` → `{pair_url, member, token_id, name}` (+ `server`, `profile`, `token`).
//!
//! Exits: 5 when the server has no public URL (`serve --url`), 4 for another Member without being
//! an owner, 2 for an unknown Member.

use clap::Args;
use serde_json::{Map, Value, json};

use crate::exit::CliResult;
use crate::output::{FieldSpec, JsonMode};

use super::Ctx;
use super::member::clean_handle;

#[derive(Args, Debug)]
pub struct PairArgs {
    /// Pair a device for another Member (owner only); default: yourself
    #[arg(long, value_name = "HANDLE")]
    pub member: Option<String>,
    /// The new Token's name (default: device)
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &["pair_url", "member", "token_id", "name", "server", "profile", "token"],
    default: &["pair_url", "member", "token_id", "name"],
};

pub fn run(ctx: &Ctx, a: PairArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let mut body = Map::new();
    if let Some(m) = &a.member {
        body.insert("member".into(), json!(clean_handle(m)));
    }
    if let Some(n) = a.name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        body.insert("name".into(), json!(n));
    }
    let r = api.post("/api/v1/pairing", &Value::Object(body))?;
    let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    if ctx.out.is_json() {
        let mut rec = Map::new();
        for k in FIELDS.all {
            rec.insert((*k).into(), json!(s(k)));
        }
        return ctx.out.json_records(&FIELDS, vec![rec], true);
    }
    if ctx.out.tty {
        ctx.out.line(&format!("Pairing a device for @{} on {}", s("member"), s("server")));
        ctx.out.line("");
        ctx.out.line(&crate::qr::terminal(&s("pair_url")));
        ctx.out.line("");
        ctx.out.line(&format!("profile {}", s("profile")));
        ctx.out.line(&s("pair_url"));
        ctx.out.note(&format!("Shown once. Revoke: annotary token revoke {}", s("token_id")));
    } else {
        ctx.out.line(&[s("pair_url"), s("member"), s("token_id"), s("name")].join("\t"));
    }
    Ok(())
}
