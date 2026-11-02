//! `rerender DOC | --all`: re-render every Version's HTML in place with the *server's* renderer
//! (after a renderer fix shipped in a new release) — Versions, Threads and Actions untouched.
//! `DOC` needs `write` on its Vault; `--all` is owner-only and covers deleted Documents too.
//! One row per Document: `name id versions changed`; a summary note on stderr.

use clap::Args;
use serde_json::{Map, Value, json};

use crate::exit::{CliError, CliResult};
use crate::output::{Align, FieldSpec, JsonMode};

use super::Ctx;
use super::docref::{DeletedMode, resolve};

#[derive(Args, Debug)]
pub struct RerenderArgs {
    /// Document id, unique Name, or path to a pulled .md
    #[arg(value_name = "DOC", required_unless_present = "all", conflicts_with = "all")]
    pub doc: Option<String>,
    /// Every Document in the Tenant, deleted ones included (owner)
    #[arg(long)]
    pub all: bool,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &["name", "id", "versions", "changed", "vault", "renderer"],
    default: &["name", "id", "versions", "changed"],
};

pub fn run(ctx: &Ctx, a: RerenderArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), !a.all);
    }
    let api = ctx.api()?;
    let (rows, renderer, summary): (Vec<Value>, String, String) = if a.all {
        let r = api.post("/api/v1/rerender", &json!({}))?;
        let renderer = r["renderer"].as_str().unwrap_or("").to_string();
        let n = |k: &str| r[k].as_u64().unwrap_or(0);
        let summary = format!(
            "rerender: {} {}, {} {}, {} changed (renderer {renderer})",
            n("documents"),
            plural(n("documents"), "Document"),
            n("versions"),
            plural(n("versions"), "Version"),
            n("changed"),
        );
        (r["docs"].as_array().cloned().unwrap_or_default(), renderer, summary)
    } else {
        let doc = a.doc.as_deref().ok_or_else(|| CliError::usage("rerender needs DOC or --all"))?;
        let d = resolve(ctx, &api, doc, None, DeletedMode::Any)?;
        let id = d["id"].as_str().unwrap_or("").to_string();
        let r = api.post(&format!("/api/v1/docs/{id}/rerender"), &json!({}))?;
        let renderer = r["renderer"].as_str().unwrap_or("").to_string();
        let n = |k: &str| r[k].as_u64().unwrap_or(0);
        let summary = format!(
            "rerender: {} {}, {} changed (renderer {renderer})",
            n("versions"),
            plural(n("versions"), "Version"),
            n("changed")
        );
        (vec![r], renderer, summary)
    };
    ctx.out.note(&summary);
    let records: Vec<Map<String, Value>> = rows
        .iter()
        .map(|r| {
            let mut m = Map::new();
            m.insert("name".into(), r["name"].clone());
            m.insert("id".into(), r["id"].clone());
            m.insert("versions".into(), r["versions"].clone());
            m.insert("changed".into(), r["changed"].clone());
            m.insert("vault".into(), r["vault"].clone());
            m.insert("renderer".into(), json!(renderer));
            m
        })
        .collect();
    if ctx.out.is_json() {
        return ctx.out.json_records(&FIELDS, records, !a.all);
    }
    let cells: Vec<Vec<String>> = records
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let n = |k: &str| r.get(k).and_then(Value::as_u64).unwrap_or(0).to_string();
            vec![s("name"), s("id"), n("versions"), n("changed")]
        })
        .collect();
    let table = ctx.out.table(
        &["NAME", "ID", "VERSIONS", "CHANGED"],
        &cells,
        &[Align::Left, Align::Left, Align::Right, Align::Right],
    );
    print!("{table}");
    Ok(())
}

fn plural(n: u64, word: &str) -> String {
    if n == 1 { word.to_string() } else { format!("{word}s") }
}
