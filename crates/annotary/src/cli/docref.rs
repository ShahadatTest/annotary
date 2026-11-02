//! Shared by every Document verb: `DOC` resolution (id | unique Name | path to a pulled `.md`),
//! Vault selection, the local Sidecar beside a `.md`, Name rules on the file name, `--since`
//! parsing and the web URL.

use std::path::{Path, PathBuf};

use serde_json::Value;

use annotary_sidecar::identity::is_document_id;
use annotary_sidecar::{Binding, ParseError};

use crate::client::{Api, qs};
use crate::exit::{CliError, CliResult, Exit};

use super::Ctx;

/// Which Documents a Name may resolve to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeletedMode {
    /// Live Documents only (default).
    Exclude,
    /// Deleted only (`restore`, `purge`).
    Only,
    /// Either (`pull`, `versions`).
    Any,
}

impl DeletedMode {
    fn param(self) -> &'static str {
        match self {
            DeletedMode::Exclude => "",
            DeletedMode::Only => "only",
            DeletedMode::Any => "any",
        }
    }
}

/// True when `doc` names a file rather than an id/Name: it ends in `.md` or contains a separator.
pub fn looks_like_path(doc: &str) -> bool {
    doc.ends_with(".md") || doc.contains('/') || doc.contains(std::path::MAIN_SEPARATOR)
}

/// The Sidecar beside a `.md`: `<dir>/<stem>.annotary`.
pub fn sidecar_path(md: &Path) -> PathBuf {
    let stem = md.file_name().and_then(|f| f.to_str()).unwrap_or("").strip_suffix(".md").unwrap_or("").to_string();
    md.with_file_name(annotary_sidecar::file_name(&stem))
}

/// The Document Name a file would push as: the file name minus `.md`, validated (typed errors
/// for a missing `.md` extension and every rejected stem).
pub fn name_of(md: &Path) -> CliResult<String> {
    let file = md.file_name().and_then(|f| f.to_str()).ok_or_else(|| {
        CliError::new(Exit::Usage, "invalid_name", format!("{} has no usable file name", md.display()))
    })?;
    let Some(stem) = file.strip_suffix(".md") else {
        return Err(CliError::new(Exit::Usage, "not_markdown", format!("{file} is not a .md file"))
            .hint("annotary pushes Markdown files: FILE.md (the stem becomes the Document's Name)"));
    };
    annotary_store::validate_name(stem).map_err(|e| {
        CliError::new(Exit::Usage, "invalid_name", e.to_string())
            .hint("Names are the filename stem: not empty, no path separators or control characters, no leading '.'")
    })?;
    Ok(stem.to_string())
}

/// Read and parse the Sidecar at `path`; `Ok(None)` when there is none.
pub fn read_binding(path: &Path) -> CliResult<Option<Binding>> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(CliError::usage(format!("cannot read {}: {e}", path.display()))),
    };
    Binding::from_text(&text).map(Some).map_err(|e| sidecar_error(path, e))
}

/// A malformed Sidecar is a typed usage error naming what is wrong.
pub fn sidecar_error(path: &Path, e: ParseError) -> CliError {
    let file = path.file_name().and_then(|f| f.to_str()).unwrap_or("Sidecar");
    // Ticket 17: a git conflict between two different Documents (or Vaults) — exit 5.
    if let ParseError::Conflict { what } = e {
        return CliError::new(Exit::Refused, "sidecar_conflict", format!("{file} has conflicting {what}"))
            .hint("keep one side");
    }
    let kind = match e {
        ParseError::Minified => "sidecar_minified",
        ParseError::MultipleValues => "sidecar_jsonl",
        ParseError::NotJson { looks_like: Some(_), .. } => "sidecar_yaml",
        _ => "sidecar_invalid",
    };
    CliError::new(Exit::Usage, kind, format!("{file} is not a Sidecar: {e}")).hint(
        "`annotary pull DOC` regenerates it (only id, vault, base_version and actions.available are read from it)",
    )
}

/// The Document JSON the API returns for `DOC`: a path (id from the sibling Sidecar), an id, or a
/// Name unique among the Vaults in scope (`vault`, else the profile's Vault, else all readable).
/// Not found / ambiguous ⇒ exit 2 (ambiguous lists `id name vN updated`).
pub fn resolve(ctx: &Ctx, api: &Api, doc: &str, vault: Option<&str>, deleted: DeletedMode) -> CliResult<Value> {
    let doc = doc.trim();
    if doc.is_empty() {
        return Err(CliError::usage("DOC must not be empty").hint("DOC = id | Name | path to a pulled .md"));
    }
    if looks_like_path(doc) {
        let md = Path::new(doc);
        let sc = sidecar_path(md);
        let Some(binding) = read_binding(&sc)? else {
            return Err(CliError::not_found(format!(
                "no Sidecar beside {} ({} missing)",
                md.display(),
                sc.file_name().and_then(|f| f.to_str()).unwrap_or("?")
            ))
            .hint("`annotary pull DOC` writes it; or pass the Document id or Name"));
        };
        return api.get(&format!("/api/v1/docs/{}", binding.id)).map_err(|e| {
            if e.exit == Exit::NotFound {
                CliError::not_found(format!("no Document {} (from {})", binding.id, sc.display())).hint(
                    "was it purged? `annotary push --doc ID` rebinds; delete the Sidecar to create a new Document",
                )
            } else {
                e
            }
        });
    }
    let scope = vault.map(str::to_string).or_else(|| ctx.profile_vault());
    let mut path = format!("/api/v1/docs/resolve?doc={}", qs(doc));
    if let Some(v) = &scope {
        path.push_str(&format!("&vault={}", qs(v)));
    }
    if deleted != DeletedMode::Exclude {
        path.push_str(&format!("&deleted={}", deleted.param()));
    }
    api.get(&path)
}

/// Pick the Vault for a new Document: `--vault` → profile `vault` → the only Vault the caller
/// can read → a typed error listing them.
pub fn select_vault(ctx: &Ctx, api: &Api, flag: Option<&str>, file: &Path) -> CliResult<String> {
    if let Some(v) = flag.map(str::trim).filter(|v| !v.is_empty()) {
        return Ok(v.to_string());
    }
    if let Some(v) = ctx.profile_vault() {
        return Ok(v);
    }
    let body = api.get("/api/v1/vaults")?;
    let names: Vec<String> = body["vaults"]
        .as_array()
        .map(|a| a.iter().filter_map(|v| v["name"].as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    match names.len() {
        1 => Ok(names[0].clone()),
        0 => Err(CliError::missing_right("no Vault you can read").hint("ask an owner for a grant")),
        _ => Err(CliError::usage(format!(
            "no sidecar beside {} and {} Vaults — pass --vault: {}",
            file.display(),
            names.len(),
            names.join(", ")
        ))
        .hint("or set `vault` in your profile")),
    }
}

/// `--since T`: `2h` / `3d` / `45m` / `1w`, `2026-08-17` (local midnight), or an RFC 3339
/// timestamp → the stored form.
pub fn parse_since(text: &str, now: jiff::Timestamp) -> CliResult<String> {
    let t = text.trim();
    let bad = || {
        CliError::usage(format!("--since {t:?}: use a duration (2h, 3d, 1w), a date (2026-08-17) or an ISO-8601 time"))
    };
    if t.is_empty() {
        return Err(bad());
    }
    if let Some((num, unit)) = t.split_at_checked(t.len() - 1)
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
        let then = now.checked_sub(jiff::SignedDuration::from_secs(secs)).map_err(|_| bad())?;
        return Ok(annotary_store::time::format(then));
    }
    if let Ok(date) = t.parse::<jiff::civil::Date>() {
        let zoned = date.to_zoned(jiff::tz::TimeZone::system()).map_err(|_| bad())?;
        return Ok(annotary_store::time::format(zoned.timestamp()));
    }
    if let Some(ts) = annotary_store::time::parse(t) {
        return Ok(annotary_store::time::format(ts));
    }
    // A bare `2026-08-17T10:00` (no offset) is taken as local time.
    if let Ok(dt) = t.parse::<jiff::civil::DateTime>() {
        let zoned = dt.to_zoned(jiff::tz::TimeZone::system()).map_err(|_| bad())?;
        return Ok(annotary_store::time::format(zoned.timestamp()));
    }
    Err(bad())
}

/// `v3` / `v3 (same content as v1)` / `already v3`.
pub fn version_word(number: u64, same_as: Option<u64>, already: bool) -> String {
    match (already, same_as) {
        (true, _) => format!("already v{number}"),
        (false, Some(k)) => format!("v{number} (same content as v{k})"),
        (false, None) => format!("v{number}"),
    }
}

/// True when `s` could be an id (used only for messages; the server decides).
pub fn is_id(s: &str) -> bool {
    is_document_id(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: S02-05, S02-07
    #[test]
    fn names_and_paths() {
        assert_eq!(name_of(Path::new("docs/plan.md")).unwrap(), "plan");
        assert_eq!(name_of(Path::new("Übersicht 2026.md")).unwrap(), "Übersicht 2026");
        assert_eq!(name_of(Path::new("a.b.md")).unwrap(), "a.b");
        let e = name_of(Path::new("notes.txt")).unwrap_err();
        assert_eq!((e.exit, e.kind.as_str()), (Exit::Usage, "not_markdown"));
        let e = name_of(Path::new(".md")).unwrap_err();
        assert_eq!(e.kind, "invalid_name");
        let e = name_of(Path::new(".hidden.md")).unwrap_err();
        assert_eq!(e.kind, "invalid_name");
        assert_eq!(name_of(Path::new("plan.MD")).unwrap_err().kind, "not_markdown");
        assert_eq!(sidecar_path(Path::new("docs/plan.md")), PathBuf::from("docs/plan.annotary"));
        assert!(looks_like_path("./plan.md") && looks_like_path("plan.md") && looks_like_path("a/b"));
        assert!(!looks_like_path("plan") && !looks_like_path("k7q2m9xd"));
        assert!(is_id("k7q2m9xd") && !is_id("plan"));
        assert_eq!(version_word(3, None, false), "v3");
        assert_eq!(version_word(3, Some(1), false), "v3 (same content as v1)");
        assert_eq!(version_word(2, None, true), "already v2");
    }

    // spec: S02-21
    #[test]
    fn since_forms() {
        let now = annotary_store::time::parse("2026-08-18T10:00:00.000Z").unwrap();
        assert_eq!(parse_since("2h", now).unwrap(), "2026-08-18T08:00:00.000Z");
        assert_eq!(parse_since("3d", now).unwrap(), "2026-08-15T10:00:00.000Z");
        assert_eq!(parse_since("1w", now).unwrap(), "2026-08-11T10:00:00.000Z");
        assert_eq!(parse_since("30m", now).unwrap(), "2026-08-18T09:30:00.000Z");
        assert_eq!(parse_since("2026-08-17T10:00Z", now).unwrap(), "2026-08-17T10:00:00.000Z");
        assert!(parse_since("2026-08-17", now).unwrap().starts_with("2026-08-1"));
        for bad in ["", "yesterday", "2x", "-1h"] {
            assert_eq!(parse_since(bad, now).unwrap_err().exit, Exit::Usage, "{bad}");
        }
    }

    #[test]
    fn sidecar_errors_are_typed() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("plan.annotary");
        assert!(read_binding(&p).unwrap().is_none());
        std::fs::write(&p, "format: 1\nid: k7q2m9xd\n").unwrap();
        let e = read_binding(&p).unwrap_err();
        assert_eq!((e.exit, e.kind.as_str()), (Exit::Usage, "sidecar_yaml"));
        assert!(e.message.starts_with("plan.annotary is not a Sidecar: not JSON (looks like YAML)"), "{}", e.message);
        std::fs::write(&p, "{\"format\":1,\"id\":\"k7q2m9xd\",\"vault\":\"notes\",\"base_version\":1,\"versions\":[]}")
            .unwrap();
        assert_eq!(read_binding(&p).unwrap_err().kind, "sidecar_minified");
        std::fs::write(&p, "{\"type\":\"document\",\"format\":1}\n{\"type\":\"version\"}\n").unwrap();
        assert_eq!(read_binding(&p).unwrap_err().kind, "sidecar_jsonl");
    }
}
