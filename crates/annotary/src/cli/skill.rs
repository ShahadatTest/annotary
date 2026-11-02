//! `skill install [--dir DIR] [--force]` (ticket 14): write the binary's embedded agent skill.
//!
//! The skill (`crates/annotary/skill/SKILL.md`, ≤ 60 lines) is compiled into the binary, so
//! the CLI is the source of truth for what agents are told; its front matter carries
//! `metadata.version`, pinned here to this binary's version, and the skill itself tells the
//! agent to re-run `skill install` when `annotary --version` says something else. The file
//! lands in `DIR/annotary/SKILL.md` (one folder per skill is what every harness expects);
//! `DIR` defaults to `./.agents/skills`, the cross-harness location, and `--dir ~/.claude/skills`
//! & co. cover the rest. An identical file is left alone (`unchanged`), a differing one is
//! refused without `--force` (exit 5) — so an older install never silently wins, and a local
//! edit is never clobbered by accident. No `references/` file: `annotary <command> --help`
//! is the reference.

use std::path::{Path, PathBuf};

use clap::{Args, Subcommand};
use serde_json::{Map, json};

use crate::exit::{CliError, CliResult};
use crate::output::{FieldSpec, JsonMode};
use crate::paths;

use super::Ctx;

/// The skill file as it sits in the repo (`metadata.version` is the workspace's `0.0.0`
/// placeholder; `skill_md()` pins it at run time to the version the binary was stamped with).
pub const SKILL_SOURCE: &str = include_str!("../../skill/SKILL.md");
/// The folder the skill lives in under DIR (`DIR/annotary/SKILL.md`).
pub const SKILL_DIR_NAME: &str = "annotary";
/// The file name every harness looks for.
pub const SKILL_FILE_NAME: &str = "SKILL.md";
/// Default DIR: the cross-harness skills location, relative to the current directory.
pub const DEFAULT_SKILLS_DIR: &str = "./.agents/skills";
/// The spec's ceiling for the skill (story 147).
pub const MAX_LINES: usize = 60;

#[derive(Subcommand, Debug)]
pub enum SkillCmd {
    /// Write the embedded SKILL.md to DIR/annotary/SKILL.md (default DIR: ./.agents/skills)
    Install(InstallArgs),
}

#[derive(Args, Debug, Default)]
pub struct InstallArgs {
    /// Skills directory: the file lands in DIR/annotary/SKILL.md (e.g. ~/.claude/skills)
    #[arg(long, value_name = "DIR")]
    pub dir: Option<PathBuf>,
    /// Overwrite a SKILL.md that differs (an older version, a local edit)
    #[arg(long)]
    pub force: bool,
}

pub const FIELDS: FieldSpec =
    FieldSpec { all: &["path", "version", "written", "dir", "previous"], default: &["path", "version", "written"] };

/// The embedded skill with `metadata.version` equal to this binary's version.
pub fn skill_md() -> String {
    with_version(SKILL_SOURCE, crate::VERSION)
}

/// `metadata.version` of a SKILL.md text (the `version:` line of its YAML front matter), if any.
pub fn skill_version(text: &str) -> Option<&str> {
    let mut rest = text.strip_prefix("---\n")?;
    let end = rest.find("\n---\n")?;
    rest = &rest[..end];
    rest.lines().find_map(|l| {
        let t = l.trim_start();
        // Only the indented `version:` of `metadata:` — not a top-level key of the same name.
        if l.starts_with(char::is_whitespace) { t.strip_prefix("version:").map(|v| v.trim()) } else { None }
    })
}

/// Rewrite the front matter's `metadata.version` to `version`; anything else is left byte for byte.
fn with_version(text: &str, version: &str) -> String {
    let Some(body) = text.strip_prefix("---\n") else { return text.to_string() };
    let Some(end) = body.find("\n---\n") else { return text.to_string() };
    let (front, tail) = body.split_at(end);
    let mut out = String::with_capacity(text.len() + 8);
    out.push_str("---\n");
    for (i, line) in front.lines().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let t = line.trim_start();
        if line.starts_with(char::is_whitespace) && t.starts_with("version:") {
            let indent = &line[..line.len() - t.len()];
            out.push_str(indent);
            out.push_str("version: ");
            out.push_str(version);
        } else {
            out.push_str(line);
        }
    }
    out.push_str(tail);
    out
}

/// Where `skill install` writes: `DIR/annotary/SKILL.md`. A literal leading `~/` (a harness
/// that passes `--dir ~/.claude/skills` without a shell) means `$HOME`.
pub fn target_path(dir: Option<&Path>) -> PathBuf {
    let dir = dir.map(expand_tilde).unwrap_or_else(|| PathBuf::from(DEFAULT_SKILLS_DIR));
    dir.join(SKILL_DIR_NAME).join(SKILL_FILE_NAME)
}

fn expand_tilde(p: &Path) -> PathBuf {
    if let Ok(rest) = p.strip_prefix("~")
        && let Some(home) = paths::home_dir()
    {
        return home.join(rest);
    }
    p.to_path_buf()
}

pub fn run(ctx: &Ctx, cmd: SkillCmd) -> CliResult<()> {
    match cmd {
        SkillCmd::Install(a) => install(ctx, a),
    }
}

pub fn install(ctx: &Ctx, a: InstallArgs) -> CliResult<()> {
    if ctx.out.json.as_ref() == Some(&JsonMode::ListFields) {
        return ctx.out.json_records(&FIELDS, Vec::new(), true);
    }
    let content = skill_md();
    let path = target_path(a.dir.as_deref());
    let shown = paths::display(&path);
    let existing = match std::fs::read_to_string(&path) {
        Ok(s) => Some(s),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) if path.is_dir() => {
            return Err(CliError::refused(format!("{shown} is a directory")).hint(format!("{e}")));
        }
        Err(e) => return Err(CliError::refused(format!("cannot read {shown}: {e}"))),
    };
    let previous = existing.as_deref().and_then(skill_version).map(str::to_string);
    let written = match &existing {
        Some(cur) if cur == &content => false,
        Some(_) if !a.force => {
            let what = match previous.as_deref() {
                Some(v) if v == crate::VERSION => format!("differs from the embedded v{v} (edited?)"),
                Some(v) => format!("has metadata.version {v} (this CLI is {})", crate::VERSION),
                None => "is not a annotary skill (no metadata.version)".to_string(),
            };
            return Err(CliError::refused(format!("{shown} exists and {what}")).hint("`--force` overwrites it"));
        }
        _ => {
            write_atomic(&path, &content)
                .map_err(|e| CliError::refused(format!("cannot write {shown}: {e}")).hint("check the directory"))?;
            true
        }
    };
    if ctx.out.is_json() {
        let mut m = Map::new();
        m.insert("path".into(), json!(path.to_string_lossy()));
        m.insert("version".into(), json!(crate::VERSION));
        m.insert("written".into(), json!(written));
        m.insert("dir".into(), json!(a.dir.as_deref().unwrap_or(Path::new(DEFAULT_SKILLS_DIR)).to_string_lossy()));
        m.insert("previous".into(), json!(previous));
        return ctx.out.json_records(&FIELDS, vec![m], true);
    }
    let verb = if written { "wrote" } else { "unchanged" };
    ctx.out.line(&format!("{verb} {shown} (v{})", crate::VERSION));
    if a.dir.is_none() {
        ctx.out.note("also: --dir ~/.claude/skills (any skills dir — the file lands in DIR/annotary/SKILL.md)");
    }
    Ok(())
}

/// Temp file + rename in the target directory (created if missing), so a reader never sees a
/// half-written skill.
fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("."));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.{}.tmp", SKILL_FILE_NAME, std::process::id()));
    let result = std::fs::write(&tmp, content).and_then(|()| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_skill_carries_the_placeholder_and_fits_the_spec() {
        // The repo copy carries the Cargo.toml placeholder; the real number is stamped at build
        // time (CMD-2), so the source never has to be edited for a release.
        assert_eq!(skill_version(SKILL_SOURCE), Some(env!("CARGO_PKG_VERSION")));
        assert_eq!(skill_version(&skill_md()), Some(crate::VERSION));
        assert!(SKILL_SOURCE.lines().count() <= MAX_LINES, "SKILL.md must stay ≤ {MAX_LINES} lines");
        assert!(SKILL_SOURCE.starts_with("---\nname: annotary\n"));
        assert!(SKILL_SOURCE.contains("metadata:\n  version: "));
        assert!(SKILL_SOURCE.contains("  cli: annotary\n"));
        assert!(SKILL_SOURCE.contains("annotary skill install"));
        assert!(SKILL_SOURCE.ends_with('\n'));
    }

    #[test]
    fn skill_metadata_version_agrees_with_dash_dash_version() {
        // CMD-2: the skill tells agents to re-install when `annotary --version` differs from
        // its metadata.version, so the two must be the same string in every build, stamped or not.
        use clap::CommandFactory;
        let cmd = crate::cli::Cli::command();
        let dash_dash_version = cmd.get_version().expect("clap has a version");
        assert_eq!(skill_version(&skill_md()), Some(dash_dash_version));
        assert_eq!(cmd.render_version().trim_end(), format!("annotary {}", skill_version(&skill_md()).unwrap()));
    }

    #[test]
    fn with_version_rewrites_only_the_metadata_line() {
        let src = "---\nname: x\nversion: top\nmetadata:\n  version: 0.0.1\n  cli: x\n---\n\nbody version: 0.0.1\n";
        let out = with_version(src, "9.9.9");
        assert_eq!(
            out,
            "---\nname: x\nversion: top\nmetadata:\n  version: 9.9.9\n  cli: x\n---\n\nbody version: 0.0.1\n"
        );
        assert_eq!(skill_version(&out), Some("9.9.9"));
        assert_eq!(skill_version("no front matter"), None);
        assert_eq!(with_version("no front matter", "1"), "no front matter");
    }

    #[test]
    fn target_path_joins_dir_annotary_skill_md() {
        assert_eq!(target_path(None), PathBuf::from("./.agents/skills/annotary/SKILL.md"));
        assert_eq!(target_path(Some(Path::new("/tmp/x"))), PathBuf::from("/tmp/x/annotary/SKILL.md"));
        if let Some(home) = paths::home_dir() {
            assert_eq!(target_path(Some(Path::new("~/.claude/skills"))), home.join(".claude/skills/annotary/SKILL.md"));
        }
        assert_eq!(target_path(Some(Path::new("~x"))), PathBuf::from("~x/annotary/SKILL.md"));
    }
}
