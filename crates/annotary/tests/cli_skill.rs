//! `skill install` with the real binary (ticket 14): the written file is the embedded SKILL.md
//! byte for byte, its `metadata.version` is what `annotary --version` prints, `--dir` and
//! `--force` behave, and nothing is ever written outside the temp dirs the tests hand it (never
//! the repo's `.agents/`, never the developer's home).

use std::path::Path;
use std::process::{Command, Output};

fn run(cwd: &Path, home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env_remove("ANNOTARY_TOKEN")
        .env_remove("ANNOTARY_SERVER")
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_TTY")
        .output()
        .unwrap()
}

fn text(o: &Output) -> (String, String) {
    (String::from_utf8_lossy(&o.stdout).to_string(), String::from_utf8_lossy(&o.stderr).to_string())
}

// spec: S14-02, S14-01
#[test]
fn install_writes_the_embedded_skill_byte_for_byte_with_the_binary_version() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let out = run(work.path(), home.path(), &["skill", "install"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    let version = String::from_utf8_lossy(&run(work.path(), home.path(), &["--version"]).stdout)
        .trim()
        .strip_prefix("annotary ")
        .unwrap()
        .to_string();
    assert_eq!(so, format!("wrote ./.agents/skills/annotary/SKILL.md (v{version})\n"));
    assert!(se.contains("--dir ~/.claude/skills"), "the hint names the other location: {se}");

    let path = work.path().join(".agents/skills/annotary/SKILL.md");
    let written = std::fs::read_to_string(&path).unwrap();
    // Byte for byte the embedded skill…
    assert_eq!(written, annotary::cli::skill::skill_md());
    // …whose metadata.version is the binary's version (the keep-current hook compares the two).
    assert_eq!(annotary::cli::skill::skill_version(&written), Some(version.as_str()));
    assert!(written.contains(&format!("  version: {version}\n")));
    // Agent-Skills front matter and the pieces the spec asks for.
    assert!(written.starts_with("---\nname: annotary\ndescription: "));
    assert!(written.contains("metadata:\n  version: "));
    assert!(written.contains("  cli: annotary\n---\n"));
    for needle in [
        "## Do this",
        "## A document for the human's review",
        "## Commands",
        "Exit: 0 ok",
        "annotary skill install",
        "only the URL `annotary link DOC [t7]` prints",
        "`preview_open`",
    ] {
        assert!(written.contains(needle), "SKILL.md lacks {needle:?}");
    }
    assert!(written.lines().count() <= 60);
    // No references file — only SKILL.md was written.
    let entries: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().flatten().collect();
    assert_eq!(entries.len(), 1, "only SKILL.md: {entries:?}");
    // Nothing landed in HOME.
    assert!(!home.path().join(".agents").exists());
    assert!(!home.path().join(".claude").exists());

    // Second run: identical file, nothing rewritten, still exit 0.
    let before = std::fs::metadata(&path).unwrap().modified().unwrap();
    let out = run(work.path(), home.path(), &["skill", "install"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(text(&out).0, format!("unchanged ./.agents/skills/annotary/SKILL.md (v{version})\n"));
    assert_eq!(std::fs::metadata(&path).unwrap().modified().unwrap(), before);
}

// spec: S14-02
#[test]
fn dir_force_and_json() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let skills = home.path().join(".claude").join("skills");
    let dir = skills.to_string_lossy().to_string();
    // --dir DIR → DIR/annotary/SKILL.md, dirs created; the path prints with ~ for $HOME.
    let out = run(work.path(), home.path(), &["skill", "install", "--dir", &dir]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    let target = skills.join("annotary").join("SKILL.md");
    assert!(target.is_file());
    assert_eq!(so, format!("wrote ~/.claude/skills/annotary/SKILL.md (v{})\n", annotary::VERSION));
    assert!(se.is_empty(), "no location hint when --dir was given: {se}");
    assert!(!work.path().join(".agents").exists(), "the default location is untouched");

    // A differing file: refused without --force (exit 5), kept as it was.
    let mut edited = std::fs::read_to_string(&target).unwrap();
    edited.push_str("\nlocal note\n");
    std::fs::write(&target, &edited).unwrap();
    let out = run(work.path(), home.path(), &["skill", "install", "--dir", &dir]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(5));
    assert!(so.is_empty());
    assert_eq!(
        se.trim(),
        format!(
            "error: ~/.claude/skills/annotary/SKILL.md exists and differs from the embedded v{} (edited?) — `--force` overwrites it",
            annotary::VERSION
        )
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), edited);
    // An older version is named as such.
    let older = edited.replace(&format!("  version: {}\n", annotary::VERSION), "  version: 0.0.1\n");
    std::fs::write(&target, &older).unwrap();
    let out = run(work.path(), home.path(), &["skill", "install", "--dir", &dir]);
    assert_eq!(out.status.code(), Some(5));
    assert!(text(&out).1.contains("has metadata.version 0.0.1 (this CLI is"), "{}", text(&out).1);
    // --force overwrites; --json reports it (`previous` = the version it replaced).
    let out = run(work.path(), home.path(), &["skill", "install", "--dir", &dir, "--force", "--json"]);
    let (so, se) = text(&out);
    assert_eq!(out.status.code(), Some(0), "{se}");
    let v: serde_json::Value = serde_json::from_str(so.trim()).unwrap();
    assert_eq!(v["path"], serde_json::json!(target.to_string_lossy()));
    assert_eq!(v["version"], serde_json::json!(annotary::VERSION));
    assert_eq!(v["written"], serde_json::json!(true));
    assert!(v.get("previous").is_none(), "not in the default set: {v}");
    assert_eq!(std::fs::read_to_string(&target).unwrap(), annotary::cli::skill::skill_md());
    // `--json ?` lists the fields; a narrowed set; `written` is omitted when false.
    let out = run(work.path(), home.path(), &["skill", "install", "--json", "?"]);
    assert_eq!(text(&out).0, "path version written dir previous\n");
    assert!(!work.path().join(".agents").exists(), "`--json ?` writes nothing");
    let out = run(work.path(), home.path(), &["skill", "install", "--dir", &dir, "--json", "written,previous"]);
    assert_eq!(
        text(&out).0.trim(),
        format!("{{\"previous\":\"{}\"}}", annotary::VERSION),
        "unchanged ⇒ written false (omitted); previous = the version that was on disk"
    );
}

// spec: S14-02
#[test]
fn help_and_usage() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let out = run(work.path(), home.path(), &["skill", "--help"]);
    assert_eq!(out.status.code(), Some(0));
    let so = text(&out).0;
    assert!(so.starts_with("Usage: annotary skill install [--dir DIR] [--force]\n"), "{so}");
    assert!(so.contains("DIR/annotary/SKILL.md"));
    // `skill` alone is a usage error (exit 1), like every verb group.
    let out = run(work.path(), home.path(), &["skill"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(!work.path().join(".agents").exists());
}
