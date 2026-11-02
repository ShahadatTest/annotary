//! `password --clear | --set [--member HANDLE]` (CMD-22): change a local password in a stopped
//! data dir's store, with no server and no credential.
//!
//! A data-dir verb like `export`: it opens `store/` itself, so it is refused (exit 5) while a
//! `serve` holds the store's `LOCK`. `--clear` without `--member` removes every Member's local
//! password, so a browser on the server's machine is trusted as the owner again (loopback trust
//! needs no password set anywhere) — the way back in after a forgotten password, and how
//! `make verify-live` opens a throwaway copy of the live store. `--set` reads the new password
//! from a prompt (no echo on a TTY; a stdin line otherwise; `ANNOTARY_PASSWORD` answers it),
//! never from an argument, for the owner unless `--member` names another Member.
//!
//! Result: one record per Member changed, `MEMBER PASSWORD` (`@handle`, `removed` | `set`).
//! Each change appends `password.removed` / `password.set` through `Store::set_password`, by the
//! founding owner (whoever holds the data dir stands for the owner).

use clap::Args;
use serde_json::{Map, json};

use annotary_store::{MemberView, Store};

use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode};
use crate::paths;

use super::Ctx;
use super::prompt::{Prompter, Sink};

pub const AFTER_HELP: &str =
    "Works on a stopped data dir (--data-dir / ANNOTARY_HOME): refused while a server holds its LOCK.
--clear removes every Member's local password (--member: one Member's), so a browser on this machine is
trusted as the owner again. --set prompts for the new password (a stdin line on a non-TTY;
ANNOTARY_PASSWORD answers it) — never an argument. Prints `MEMBER PASSWORD` per Member changed.";

#[derive(Args, Debug)]
#[command(group(clap::ArgGroup::new("change").required(true).args(["clear", "set"])))]
pub struct PasswordArgs {
    /// Remove the local password (every Member's, or --member's)
    #[arg(long)]
    pub clear: bool,
    /// Set a new local password (prompted; the owner's, or --member's)
    #[arg(long)]
    pub set: bool,
    /// The Member to change [default: --clear every Member, --set the owner]
    #[arg(long, value_name = "HANDLE")]
    pub member: Option<String>,
}

pub const FIELDS: FieldSpec =
    FieldSpec { all: &["member", "password", "id", "store"], default: &["member", "password"] };

pub fn run(ctx: &Ctx, a: PasswordArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    // An unknown --json field is a usage error before anything changes.
    if let Some(mode) = &ctx.out.json {
        FIELDS.select(mode)?;
    }
    let store_dir = ctx.data.store();
    if !annotary_store::backup::has_store(&store_dir) {
        return Err(CliError::refused(format!("no store at {}", paths::display(&store_dir)))
            .hint("--data-dir / ANNOTARY_HOME pick the data dir whose store to change"));
    }
    let store = Store::open(&store_dir)?;
    let owner = store.founding_owner()?.ok_or_else(|| CliError::refused("the store has no owner yet"))?;
    let named = match a.member.as_deref() {
        Some(h) => {
            let handle = h.trim_start_matches('@');
            let m =
                store.member_by_handle(handle)?.ok_or_else(|| CliError::not_found(format!("no Member @{handle}")))?;
            if m.is_disabled() {
                return Err(CliError::refused(format!("@{handle} is disabled")));
            }
            Some(m)
        }
        None => None,
    };

    let mut changed = Vec::new();
    if a.clear {
        let targets = match named {
            Some(m) => vec![m],
            None => store.list_members()?.into_iter().filter(|m| !m.is_disabled()).collect(),
        };
        for m in targets.into_iter().filter(|m| m.has_password) {
            store.set_password(&owner.id, &m.id, None)?;
            changed.push((m, "removed"));
        }
    } else {
        let m = named.unwrap_or(owner.clone());
        let p = Prompter::new(Sink::Stderr, "");
        let pw = p
            .ask_password(&format!("new local password for @{}", m.handle), Some("ANNOTARY_PASSWORD"))
            .ok_or_else(|| CliError::usage("empty password").hint("--clear removes a password"))?;
        store.set_password(&owner.id, &m.id, Some(&pw))?;
        changed.push((m, "set"));
    }

    // Printed after the records, so the result reads first.
    let note = match (a.clear, changed.is_empty(), a.member.as_deref()) {
        (true, true, Some(h)) => Some(format!("password: @{} has no local password", h.trim_start_matches('@'))),
        (true, true, None) => Some("password: no local password to remove".to_string()),
        (true, false, _) if !store.password_set()? => {
            Some("password: no local password is set — a browser on this machine is trusted as the owner".to_string())
        }
        _ => None,
    };
    drop(store);
    let result = print(ctx, &changed, &store_dir);
    if let Some(n) = note {
        ctx.out.note(&n);
    }
    result
}

fn print(ctx: &Ctx, changed: &[(MemberView, &str)], store_dir: &std::path::Path) -> CliResult<()> {
    if ctx.out.is_json() {
        let records = changed
            .iter()
            .map(|(m, what)| {
                let mut r = Map::new();
                r.insert("member".into(), json!(m.handle));
                r.insert("password".into(), json!(what));
                r.insert("id".into(), json!(m.id));
                r.insert("store".into(), json!(store_dir.to_string_lossy()));
                r
            })
            .collect();
        return ctx.out.json_records(&FIELDS, records, false);
    }
    if changed.is_empty() {
        return Ok(());
    }
    let rows: Vec<Vec<String>> =
        changed.iter().map(|(m, what)| vec![format!("@{}", m.handle), what.to_string()]).collect();
    print!("{}", ctx.out.table(&["MEMBER", "PASSWORD"], &rows, &[]));
    Ok(())
}
