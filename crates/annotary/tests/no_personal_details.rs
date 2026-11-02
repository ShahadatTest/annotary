//! The guard: everything git tracks or would track holds no tailnet hostname other than
//! `example.ts.net`, no email address outside an `example` domain, and no home path other than
//! `/home/linuxbrew`. Binary files are scanned as bytes, and a file with one of [`OPAQUE_SUFFIXES`],
//! whose bytes cannot be read that way, may not be committed unless [`OPAQUE_FILES`] names it.

use std::path::{Path, PathBuf};
use std::process::Command;

use regex::Regex;

/// Where an email address is meant to be read, by path: the Code of Conduct's contact and the
/// template's record of it.
const EMAIL_ALLOWED: &[&str] = &["CODE_OF_CONDUCT.md", ".agents/standard.yml"];

const NEVER_EXEMPT: &[&str] = &["tests", "fixtures", "crates"];

/// Files the patterns cannot read into: archives and databases.
const OPAQUE_SUFFIXES: &[&str] = &[
    ".tar", ".tgz", ".tbz2", ".gz", ".zip", ".7z", ".rar", ".bz2", ".xz", ".zst", ".lz4", ".sqlite", ".sqlite3", ".db",
    ".parquet",
];

/// The opaque files allowed, and why.
const OPAQUE_FILES: &[&str] = &[];

struct Patterns {
    tailnet: Regex,
    tailnet_ip: Regex,
    email: Regex,
    home: Regex,
}

impl Patterns {
    fn new() -> Patterns {
        Patterns {
            tailnet: Regex::new(r"(?i)(?:^|[^a-z0-9.-])((?:[a-z0-9-]+\.)+ts\.net)\b").unwrap(),
            tailnet_ip: Regex::new(r"100\.(\d{1,3})\.\d{1,3}\.\d{1,3}").unwrap(),
            email: Regex::new(r"(?i)(?:^|[^a-z0-9._%+-])[a-z0-9._%+-]+@((?:[a-z0-9-]+\.)+[a-z]{2,})\b").unwrap(),
            home: Regex::new(r"(?i)(?:^|[^A-Za-z0-9_.-])(/(?:users|home)/[A-Za-z0-9._-]+)").unwrap(),
        }
    }

    /// What is wrong with one line of text.
    fn findings(&self, line: &str) -> Vec<String> {
        let mut out = Vec::new();
        for c in self.tailnet_ip.captures_iter(line) {
            // 100.64.0.0/10 is the tailnet range; 100.64.x.x is what examples and tests use. The
            // edges are checked here, not in the pattern, so that two addresses can share one.
            let whole = c.get(0).unwrap();
            let before = line[..whole.start()].chars().next_back();
            let mut after = line[whole.end()..].chars();
            let inside_a_number = before.is_some_and(|c| c.is_ascii_digit() || c == '.')
                || match after.next() {
                    Some('.') => after.next().is_some_and(|c| c.is_ascii_digit()),
                    Some(c) => c.is_ascii_digit(),
                    None => false,
                };
            let second: u32 = c[1].parse().unwrap_or(0);
            if !inside_a_number && (65..=127).contains(&second) {
                out.push("tailnet address".to_string());
            }
        }
        for c in self.tailnet.captures_iter(line) {
            let host = c[1].to_ascii_lowercase();
            if host != "example.ts.net" && !host.ends_with(".example.ts.net") {
                out.push(format!("tailnet hostname {host}"));
            }
        }
        for c in self.email.captures_iter(line) {
            let domain = c[1].to_ascii_lowercase();
            if !is_example(&domain) {
                out.push(format!("email address at {domain}"));
            }
        }
        for c in self.home.captures_iter(line) {
            if !c[1].eq_ignore_ascii_case("/home/linuxbrew") {
                out.push(format!("home path {}", &c[1]));
            }
        }
        out
    }
}

/// `example.<tld>` or a name under it; `example.com.other.org` is not.
fn is_example(domain: &str) -> bool {
    let mut labels = domain.rsplit('.');
    labels.next();
    labels.next() == Some("example")
}

/// What is wrong with these files under `root`, as `file:line: finding`.
fn failures(root: &Path, files: &[String]) -> Vec<String> {
    let patterns = Patterns::new();
    let mut out = Vec::new();
    for file in files {
        let lower = file.to_ascii_lowercase();
        if OPAQUE_SUFFIXES.iter().any(|s| lower.ends_with(s)) && !OPAQUE_FILES.contains(&file.as_str()) {
            out.push(format!("{file}: an archive or a database cannot be scanned; commit what it is made from"));
        }
        // Bytes, not text: a binary's metadata can name a host. A directory link or a file deleted
        // in the working tree has nothing to read.
        let Ok(bytes) = std::fs::read(root.join(file)) else { continue };
        let text = String::from_utf8_lossy(&bytes);
        for (i, line) in text.lines().enumerate() {
            for finding in patterns.findings(line) {
                if finding.starts_with("email") && EMAIL_ALLOWED.contains(&file.as_str()) {
                    continue;
                }
                out.push(format!("{file}:{}: {finding}", i + 1));
            }
        }
    }
    out
}

/// Tracked files and new ones that are not ignored: what a commit could carry.
fn repo_files(root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files", "-z", "--cached", "--others", "--exclude-standard"])
        .current_dir(root)
        .output()
        .expect("the guard needs git to list the repo's files");
    assert!(out.status.success(), "git ls-files: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap().split('\0').filter(|f| !f.is_empty()).map(String::from).collect()
}

/// A hit for a test. The guard scans this file too, so the hit is made from a placeholder when the
/// test runs: `{ts}` is the tailnet suffix, `{at}` the at sign, `{users}` and `{home}` the home folders.
fn plant(template: &str) -> String {
    template.replace("{ts}", "ts.net").replace("{at}", "@").replace("{users}", "/Users/").replace("{home}", "/home/")
}

// spec: S32-07
#[test]
fn the_repo_holds_no_personal_details() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap();
    let found = failures(&root, &repo_files(&root));
    assert!(
        found.is_empty(),
        "personal details; use example.ts.net, example.com / example.org, or a path outside a home:\n{}",
        found.join("\n")
    );
}

// spec: S32-07
#[test]
fn a_planted_hit_in_a_file_fails_the_guard() {
    let dir = tempfile::tempdir().unwrap();
    let root: PathBuf = dir.path().to_path_buf();
    for (name, body) in [
        ("tests/host.md", "see box.tail1234.{ts} today"),
        ("fixtures/mail.md", "write to someone{at}mail.test today"),
        ("crates/path.rs", "let p = \"{users}someone/.ssh/config\";"),
    ] {
        std::fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
        std::fs::write(root.join(name), plant(body)).unwrap();
        let found = failures(&root, &[name.to_string()]);
        assert_eq!(found.len(), 1, "{name}: {found:?}");
        assert!(found[0].starts_with(&format!("{name}:1: ")), "{found:?}");
    }
}

// spec: S32-07
#[test]
fn an_opaque_file_fails_the_guard_and_a_binary_is_read_as_bytes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("data.tar.gz"), b"").unwrap();
    assert_eq!(failures(dir.path(), &["data.tar.gz".to_string()]).len(), 1);
    let mut bytes = vec![0xff, 0xfe, 0x00];
    bytes.extend_from_slice(plant(" box.tail1234.{ts} ").as_bytes());
    std::fs::write(dir.path().join("logo.png"), bytes).unwrap();
    assert_eq!(failures(dir.path(), &["logo.png".to_string()]).len(), 1);
}

// spec: S32-08
#[test]
fn an_email_address_is_allowed_by_path_and_nothing_else_is() {
    let dir = tempfile::tempdir().unwrap();
    let address = plant("reach us at someone{at}mail.test\n");
    for allowed in EMAIL_ALLOWED {
        std::fs::create_dir_all(dir.path().join(allowed).parent().unwrap()).unwrap();
        std::fs::write(dir.path().join(allowed), &address).unwrap();
        assert_eq!(failures(dir.path(), &[allowed.to_string()]), Vec::<String>::new(), "{allowed}");
    }
    std::fs::write(dir.path().join("README.md"), &address).unwrap();
    assert_eq!(failures(dir.path(), &["README.md".to_string()]).len(), 1, "any other file");
    // Only the address: a host in an allowed file still fails.
    std::fs::write(dir.path().join(EMAIL_ALLOWED[0]), plant("box.tail1234.{ts}\n")).unwrap();
    assert_eq!(failures(dir.path(), &[EMAIL_ALLOWED[0].to_string()]).len(), 1, "a host in an allowed file");
}

// spec: S32-07
#[test]
fn nothing_under_tests_fixtures_or_crates_is_exempt() {
    for path in EMAIL_ALLOWED {
        let top = path.split('/').next().unwrap();
        assert!(!NEVER_EXEMPT.contains(&top), "{path} may not be exempt");
    }
}

// spec: S32-07
#[test]
fn the_patterns_catch_what_they_should_and_nothing_else() {
    let p = Patterns::new();
    for bad in [
        "https://box.tail1234.{ts}:7377/d/x",
        "box.tailnet.{ts}",
        "box.{ts}",
        "BOX.TAIL1234.{ts}",
        "write to someone{at}mail.test today",
        "<first.last+tag{at}mail.company.co.uk>",
        "someone{at}example.com.other.org",
        "{users}someone/.ssh/config",
        "HOME={home}someone",
        "(file://{users}someone)",
    ] {
        let bad = plant(bad);
        assert!(!p.findings(&bad).is_empty(), "should be caught: {bad}");
    }
    // Built at run time: this file is scanned too. 100.64.x.x is the range examples use.
    for second in [65, 99, 127] {
        let bad = format!("peer 100.{second}.88.77 joined");
        assert!(!p.findings(&bad).is_empty(), "should be caught: {bad}");
    }
    for second in [63, 64, 128] {
        let good = format!("peer 100.{second}.88.77 joined");
        assert_eq!(p.findings(&good), Vec::<String>::new(), "should pass: {good}");
    }
    // After an allowed address, at a sentence's end; and not inside a longer number.
    let bad_ip = format!("100.{}.1.1", 99);
    for bad in [format!("100.64.0.1 {bad_ip}"), format!("100.64.0.1,{bad_ip}"), format!("it was {bad_ip}.")] {
        assert!(!p.findings(&bad).is_empty(), "should be caught: {bad}");
    }
    for good in [format!("1.{bad_ip}"), format!("{bad_ip}234"), format!("{bad_ip}.5")] {
        assert_eq!(p.findings(&good), Vec::<String>::new(), "should pass: {good}");
    }
    for good in [
        "https://annotary.example.{ts}/d/x",
        "example.{ts}",
        "someone{at}example.com",
        "someone{at}tiles.example.org",
        "{home}linuxbrew",
        "{home}linuxbrew/.linuxbrew/bin/annotary",
        "a@b",
        "mermaid@11.4.1",
        "@scribe and @types/node",
        "/srv/annotary/data and /opt/rowan/.annotary",
        "<tailnet>.ts.net and /Users/<name>",
    ] {
        let good = plant(good);
        assert_eq!(p.findings(&good), Vec::<String>::new(), "should pass: {good}");
    }
}
