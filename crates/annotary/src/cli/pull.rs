//! `pull DOC [--version N] [--out DIR] [--assets] [--force]`: writes exactly `<name of N>.md`
//! (byte-identical to the pushed Version) and `<name of N>.annotary` (the server's full
//! projection with `base_version: N`). Refuses (exit 5) to overwrite a local `.md` whose content
//! differs from what its Sidecar recorded — or that has no Sidecar and differs from what would be
//! written — unless `--force`. `--assets` also writes every Asset of Version N (recursive `.md`
//! Assets and theirs included) to its `ref` relative to the `.md`, creating directories: a file
//! with the same hash is left alone, a differing file refuses the whole pull (exit 5) unless
//! `--force`, missing Assets are skipped with a note. stdout: `name.md name.annotary vN`; the
//! summary (`pull: N threads, M assets (K missing: …); W written, U unchanged`) goes to stderr.

use std::path::{Path, PathBuf};

use clap::Args;
use serde_json::{Map, json};

use annotary_sidecar::identity::sha256_hex;

use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode, relative_age};

use super::Ctx;
use super::docref::{DeletedMode, read_binding, resolve, sidecar_path};
use super::push::write_sidecar;

#[derive(Args, Debug)]
pub struct PullArgs {
    /// Document id, unique Name, or path to a pulled .md
    pub doc: String,
    /// Pull Version N (writes N's Name; Threads as they were on N)
    #[arg(long, value_name = "N")]
    pub version: Option<u32>,
    /// Directory to write into (default: .)
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
    /// Also write the Assets to their relative paths (off by default)
    #[arg(long)]
    pub assets: bool,
    /// Overwrite local edits / differing asset files
    #[arg(long)]
    pub force: bool,
}

pub const FIELDS: FieldSpec = FieldSpec {
    all: &["md", "sidecar", "version", "name", "id", "vault", "threads", "assets", "written"],
    default: &["md", "sidecar", "version"],
};

/// One Asset file to write (or leave alone).
struct Planned {
    reference: String,
    hash: String,
    path: PathBuf,
    /// A file with the same content is already there.
    unchanged: bool,
}

pub fn run(ctx: &Ctx, a: PullArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    let api = ctx.api()?;
    let doc = resolve(ctx, &api, &a.doc, None, DeletedMode::Any)?;
    let id = doc["id"].as_str().unwrap_or("").to_string();
    let head = doc["version"].as_u64().unwrap_or(0) as u32;
    let n = a.version.unwrap_or(head);
    let version = api.get(&format!("/api/v1/docs/{id}/versions/{n}"))?;
    let name = version["name"].as_str().unwrap_or("").to_string();
    let md_hash = version["md_hash"].as_str().unwrap_or("").to_string();

    let bytes = api.get_bytes(&format!("/api/v1/blobs/{md_hash}"))?;
    if sha256_hex(&bytes) != md_hash {
        return Err(CliError::internal(format!("blob {md_hash} did not verify")));
    }
    let sc_text = api.get_bytes(&format!("/api/v1/docs/{id}/sidecar?version={n}"))?;

    let out_dir = a.out.clone().unwrap_or_else(|| PathBuf::from("."));
    std::fs::create_dir_all(&out_dir)
        .map_err(|e| CliError::usage(format!("cannot create {}: {e}", out_dir.display())))?;
    let md_path = out_dir.join(format!("{name}.md"));
    let sc_path = sidecar_path(&md_path);
    let md_file = format!("{name}.md");
    let sc_file = annotary_sidecar::file_name(&name);

    // Never clobber local edits.
    if !a.force
        && let Ok(local) = std::fs::read(&md_path)
    {
        match read_binding(&sc_path) {
            Ok(Some(b)) => {
                if b.id != id {
                    return Err(CliError::refused(format!("{md_file} here is bound to {} — not {id}", b.id))
                        .hint("`pull --force` overwrites it, or use --out DIR"));
                }
                let local_hash = sha256_hex(&local);
                match b.matches_base(&local_hash) {
                    Some(true) => {}
                    Some(false) => {
                        let pulled = std::fs::metadata(&sc_path)
                            .and_then(|m| m.modified())
                            .ok()
                            .map(|t| {
                                let ts = jiff::Timestamp::try_from(t).unwrap_or_default();
                                format!(
                                    " (pulled {} ago)",
                                    relative_age(&annotary_store::time::format(ts), jiff::Timestamp::now())
                                )
                            })
                            .unwrap_or_default();
                        return Err(CliError::refused(format!(
                            "{md_file} has local edits since v{}{pulled}",
                            b.base_version
                        ))
                        .hint("commit them with `push`, or `pull --force` to overwrite"));
                    }
                    None => {
                        return Err(CliError::refused(format!(
                            "cannot tell whether {md_file} has local edits ({sc_file} lists no v{})",
                            b.base_version
                        ))
                        .hint("`pull --force` overwrites it"));
                    }
                }
            }
            Ok(None) => {
                if local != bytes {
                    return Err(CliError::refused(format!(
                        "{md_file} exists here without a Sidecar and differs from v{n}"
                    ))
                    .hint("`pull --force` overwrites it, or use --out DIR"));
                }
            }
            Err(e) => {
                return Err(CliError::refused(format!("cannot tell whether {md_file} has local edits: {}", e.message))
                    .hint("`pull --force` overwrites it"));
            }
        }
    }

    let sc = annotary_sidecar::parse(std::str::from_utf8(&sc_text).unwrap_or("")).ok();
    let threads = sc.as_ref().map(|s| s.threads.len()).unwrap_or(0);
    let assets = sc.as_ref().map(|s| s.assets.len()).unwrap_or(0);
    let missing: Vec<String> = sc
        .as_ref()
        .map(|s| s.assets.iter().filter(|a| a.missing).map(|a| a.reference.clone()).collect())
        .unwrap_or_default();

    // `--assets`: plan every Asset file before writing anything, so a refusal writes nothing.
    let mut planned: Vec<Planned> = Vec::new();
    if a.assets {
        let mut differing: Vec<String> = Vec::new();
        for entry in sc.as_ref().map(|s| s.assets.as_slice()).unwrap_or(&[]) {
            let Some(hash) = entry.hash.as_deref().filter(|_| !entry.missing) else { continue };
            if annotary_store::manifest::check_ref(&entry.reference).is_err() {
                ctx.out.note(&format!("note: asset {} has an unusable path — not written", entry.reference));
                continue;
            }
            let path = out_dir.join(Path::new(&entry.reference));
            let unchanged = match std::fs::read(&path) {
                Ok(local) => {
                    if sha256_hex(&local) == hash {
                        true
                    } else {
                        differing.push(entry.reference.clone());
                        false
                    }
                }
                Err(_) => false,
            };
            planned.push(Planned { reference: entry.reference.clone(), hash: hash.to_string(), path, unchanged });
        }
        if !differing.is_empty() && !a.force {
            let count = differing.len();
            return Err(CliError::refused(format!(
                "{count} asset file{} here differ{} from v{n} ({})",
                plural(count),
                if count == 1 { "s" } else { "" },
                differing.join(", "),
            ))
            .hint("`pull --assets --force` overwrites them, or use --out DIR"));
        }
    }

    std::fs::write(&md_path, &bytes)
        .map_err(|e| CliError::usage(format!("cannot write {}: {e}", md_path.display())))?;
    write_sidecar(&sc_path, &sc_text)?;

    let mut written = 0usize;
    let mut unchanged = 0usize;
    for p in &planned {
        if p.unchanged {
            unchanged += 1;
            continue;
        }
        let data = api.get_bytes(&format!("/api/v1/blobs/{}", p.hash))?;
        if sha256_hex(&data) != p.hash {
            return Err(CliError::internal(format!("blob {} ({}) did not verify", p.hash, p.reference)));
        }
        if let Some(dir) = p.path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| CliError::usage(format!("cannot create {}: {e}", dir.display())))?;
        }
        std::fs::write(&p.path, &data)
            .map_err(|e| CliError::usage(format!("cannot write {}: {e}", p.path.display())))?;
        written += 1;
    }

    // Summary (stderr).
    let mut summary = format!("pull: {threads} thread{}, {assets} asset{}", plural(threads), plural(assets));
    if !missing.is_empty() {
        summary.push_str(&format!(" ({} missing: {})", missing.len(), missing.join(", ")));
    }
    if a.assets {
        if !planned.is_empty() {
            summary.push_str(&format!("; {written} written, {unchanged} unchanged"));
        }
    } else if assets > 0 {
        summary.push_str("; assets not written (--assets)");
    }
    ctx.out.note(&summary);
    if n != head {
        ctx.out.note(&format!("(name on v{n} was \"{name}\"; Threads as they were on v{n}; head is v{head})"));
    }

    if ctx.out.is_json() {
        let mut rec = Map::new();
        rec.insert("md".into(), json!(md_file));
        rec.insert("sidecar".into(), json!(sc_file));
        rec.insert("version".into(), json!(n));
        rec.insert("name".into(), json!(name));
        rec.insert("id".into(), json!(id));
        rec.insert("vault".into(), doc["vault"].clone());
        rec.insert("threads".into(), json!(threads));
        rec.insert("assets".into(), json!(assets));
        rec.insert("written".into(), json!(written));
        return ctx.out.json_records(&FIELDS, vec![rec], true);
    }
    if ctx.out.tty {
        ctx.out.line(&format!("{md_file}  {sc_file}  v{n}"));
    } else {
        ctx.out.line(&format!("{md_file}\t{sc_file}\tv{n}"));
    }
    Ok(())
}

fn plural(n: usize) -> &'static str {
    if n == 1 { "" } else { "s" }
}
