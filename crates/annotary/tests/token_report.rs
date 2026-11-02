//! Token-count guard (ticket 14): the reads the skill file sends an agent to — `status`, a `wait`
//! event line, `mentions`, `show`, `thread show` — plus `--help` and SKILL.md itself, measured
//! with the CLI prototype's tokenizer (o200k) from **real output** of the real binary against an
//! in-process server seeded with the prototype's scenario, must stay within ±20% of the
//! prototype's numbers (spec "Further Notes": 24 · 12 · 58 · 245 · 117 · 688 · 905).
//!
//! The measured table is `tests/token-report.md` at the repo root: random parts of the output
//! (the Document id, ages, the cursor) are normalised to the prototype's values first, so the
//! counts are stable and the committed report must match — `TOKEN_REPORT=overwrite cargo test
//! -p annotary --test token_report` (or `make token-report`) regenerates it after a deliberate
//! output change.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use annotary::testing::TestServer;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

struct Cli {
    server: String,
    home: PathBuf,
    cwd: PathBuf,
}

impl Cli {
    fn run(&self, token: &str, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_annotary"))
            .args(args)
            .current_dir(&self.cwd)
            .env("HOME", &self.home)
            .env("ANNOTARY_HOME", self.home.join(".annotary"))
            .env("ANNOTARY_TOKEN", token)
            .env("ANNOTARY_SERVER", &self.server)
            .env_remove("ANNOTARY_PROFILE")
            .env_remove("ANNOTARY_TTY")
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    }

    /// Run and return stdout; panics (with stderr) on a non-zero exit.
    fn ok(&self, token: &str, args: &[&str]) -> String {
        let out = self.run(token, args);
        assert!(
            out.status.success(),
            "annotary {:?} failed ({:?}):\n{}",
            args,
            out.status.code(),
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).to_string()
    }
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        let t = to.join(e.file_name());
        if p.is_dir() {
            copy_dir(&p, &t);
        } else {
            std::fs::copy(&p, &t).unwrap();
        }
    }
}

/// The prototype's numbers (o200k), as in the spec's "Measured targets from the prototypes".
const PROTOTYPE: &[(&str, usize)] = &[
    ("SKILL.md (whole)", 905),
    ("annotary --help", 688),
    ("$ annotary mentions", 58),
    ("$ annotary show plan", 245),
    ("$ annotary thread show plan t2", 117),
    ("$ annotary status plan", 24),
    ("$ annotary wait plan --on action,mention --timeout 30m", 12),
    ("$ annotary status plan --json", 61),
    ("$ annotary thread list plan --json id,anchor,body", 188),
];

/// Replace the run-specific parts of a CLI output with the prototype's values so the token
/// count is reproducible: the Document id, the event cursor, relative ages and ISO timestamps.
fn normalise(text: &str, id: &str) -> String {
    let text = text.replace(id, "k7q2m9xd");
    let is_age = |w: &str| {
        w.is_ascii()
            && w.len() >= 2
            && w[..w.len() - 1].chars().all(|c| c.is_ascii_digit())
            && w.ends_with(['s', 'm', 'h', 'd', 'w'])
    };
    let is_iso = |w: &str| {
        w.is_ascii()
            && w.len() == 20
            && w.as_bytes()[10] == b'T'
            && w.ends_with('Z')
            && w[..10].chars().all(|c| c.is_ascii_digit() || c == '-')
            && w[11..19].chars().all(|c| c.is_ascii_digit() || c == ':')
    };
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let mut first = true;
        // Words and their separators, in order.
        let mut rest = line;
        while !rest.is_empty() {
            let word_end = rest.find([' ', '\t', '\n']).unwrap_or(rest.len());
            let (word, tail) = rest.split_at(word_end);
            let sep_end = tail.find(|c| c != ' ' && c != '\t' && c != '\n').unwrap_or(tail.len());
            let (sep, after) = tail.split_at(sep_end);
            let mapped: String =
                if first && word.chars().all(|c| c.is_ascii_digit()) && !word.is_empty() && sep.starts_with('\t') {
                    "814".into() // a `wait` event line: SEQ first
                } else if is_age(word) {
                    "4d".into()
                } else if word.strip_prefix("seq:").is_some_and(|n| n.chars().all(|c| c.is_ascii_digit())) {
                    "seq:812".into()
                } else {
                    // JSON: `"seq":NNN` and ISO timestamps inside one "word".
                    let mut w = word.to_string();
                    if let Some(i) = w.find("\"seq\":") {
                        let digits: String = w[i + 6..].chars().take_while(|c| c.is_ascii_digit()).collect();
                        w.replace_range(i + 6..i + 6 + digits.len(), "814");
                    }
                    if let Some(i) = w.find("\"at\":\"") {
                        let start = i + 6;
                        if w.len() >= start + 20 && w.is_char_boundary(start + 20) && is_iso(&w[start..start + 20]) {
                            w.replace_range(start..start + 20, "2026-08-18T09:41:02Z");
                        }
                    }
                    w
                };
            out.push_str(&mapped);
            out.push_str(sep);
            rest = after;
            first = false;
        }
    }
    out
}

/// Seed the prototype scenario (as `tests/e2e/95-agent-flow/agent.trycmd` does) and measure.
/// Returns `(label, lines, tokens, text)` rows in report order.
fn measure() -> Vec<(&'static str, usize, usize, String)> {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    // The seed files are copied: `push` writes a Sidecar beside the pushed file.
    copy_dir(&repo_root().join("tests/e2e/95-agent-flow/agent.in/seed"), &work.path().join("seed"));
    let cli = Cli { server: ts.url.clone(), home: home.path().to_path_buf(), cwd: work.path().to_path_buf() };
    let rowan = ts.owner_token.clone();
    let (_, agent) = ts.add_member("scribe");
    let (_, linter) = ts.add_member("linter");
    let j = rowan.as_str();
    let a = agent.as_str();
    let c = linter.as_str();

    let v1 = cli.ok(j, &["push", "seed/plan.md", "-m", "first draft"]);
    let id = v1.split('\t').nth(1).unwrap().trim().to_string();
    let steps: &[(&str, &[&str])] = &[
        (
            j,
            &[
                "thread",
                "new",
                "plan",
                "--document",
                "-m",
                "Needs a **Risks** section before I can approve. @scribe can you add one?",
            ],
        ),
        (
            j,
            &[
                "thread",
                "new",
                "plan",
                "16:0-16:33",
                "-m",
                "Agreed — but the skill file must land *with* the CLI, not after. @scribe make sure the order list says so.",
            ],
        ),
        (a, &["thread", "new", "plan", "20:41-20:73", "-m", "Three feels low for useful signal. Five?"]),
        (
            j,
            &[
                "thread",
                "new",
                "plan",
                "--asset",
                "img/architecture.png",
                "-m",
                "Diagram is out of date — the daemon box is missing.",
            ],
        ),
        (
            a,
            &[
                "thread",
                "new",
                "plan",
                "11:48-11:69",
                "-m",
                "Should the anchor engine be its own package so the CLI and server share one implementation? @rowan @linter",
            ],
        ),
        (j, &["act", "plan", "send-back"]),
        (a, &["push", "seed/v2/plan.md", "--doc", "plan", "-m", "address review: add Risks, split rollout"]),
        (a, &["thread", "reply", "plan", "t1", "--resolve", "-m", "Added §Risks in v2."]),
        (
            a,
            &[
                "thread",
                "reply",
                "plan",
                "t2",
                "-m",
                "Item 3 already lists the skill file with the CLI. @rowan do you want it as its own item?",
            ],
        ),
        (a, &["thread", "reply", "plan", "t4", "-m", "(a reply, deleted below)"]),
        (a, &["thread", "delete", "plan", "t4.1"]),
        (
            j,
            &[
                "thread",
                "new",
                "plan",
                "21:29-21:79",
                "-m",
                "This is exactly what the sidecar-format ticket has to answer — link it here.",
            ],
        ),
        (c, &["thread", "new", "plan", "20:27-20:57", "-m", "(a question, deleted below)"]),
        (
            j,
            &[
                "thread",
                "reply",
                "plan",
                "t7",
                "-m",
                "(Replying to the deleted question: yes — the fixture table is also the seed of the test suite.)",
            ],
        ),
        (c, &["thread", "delete", "plan", "t7"]),
        (j, &["push", "seed/v3/plan.md", "--doc", "plan"]),
        (
            c,
            &[
                "thread",
                "new",
                "plan",
                "31:41-31:57",
                "-m",
                "How — survey or interviews? Worth deciding before the invites go out.",
            ],
        ),
        (c, &["thread", "reply", "plan", "t5", "-m", "Yes — one package, both import it. Determinism depends on it."]),
        (j, &["thread", "reply", "plan", "t5", "-m", "Agreed."]),
        (c, &["thread", "resolve", "plan", "t5"]),
        (j, &["thread", "reply", "plan", "t3", "--resolve", "-m", "Done — five, and two weeks of feedback."]),
        (j, &["act", "plan", "approve"]),
        (j, &["push", "seed/v4/plan.md", "--doc", "plan", "-m", "back to the first-draft wording for comparison"]),
    ];
    for (tok, args) in steps {
        cli.ok(tok, args);
    }

    let mut rows = Vec::new();
    let skill = annotary::cli::skill::skill_md();
    rows.push(("SKILL.md (whole)", skill.lines().count(), 0, skill));
    let help = cli.ok(j, &["--help"]);
    rows.push(("annotary --help", help.lines().count(), 0, help));
    // The agent's reads at head v4 (the prototype's transcript).
    let mentions = cli.ok(a, &["mentions"]);
    rows.push(("$ annotary mentions", 0, 0, mentions));
    let show = cli.ok(a, &["show", "plan"]);
    rows.push(("$ annotary show plan", 0, 0, show));
    let tshow = cli.ok(a, &["thread", "show", "plan", "t2"]);
    rows.push(("$ annotary thread show plan t2", 0, 0, tshow));
    // …then the agent's push (v5) + reply --resolve, and `status` with no Action yet.
    cli.ok(a, &["pull", "plan", "--assets"]);
    let md = work.path().join("plan.md");
    let text = std::fs::read_to_string(&md).unwrap();
    std::fs::write(&md, text.replace("and the skill file.", "and its skill file — shipped together.")).unwrap();
    cli.ok(a, &["push", "plan.md", "-m", "make the skill file explicit in the order of work"]);
    cli.ok(
        a,
        &[
            "thread",
            "reply",
            "plan",
            "t2",
            "--resolve",
            "-m",
            "Done in v5 — item 3 now reads 'CLI … and its skill file'. @rowan",
        ],
    );
    let status = cli.ok(a, &["status", "plan"]);
    rows.push(("$ annotary status plan", 0, 0, status.clone()));
    // The owner approves; the agent's `wait` (from the cursor `status` gave) prints the event.
    let seq: String = status.rsplit("seq:").next().unwrap().trim().to_string();
    cli.ok(j, &["act", "plan", "approve"]);
    let wait = cli.ok(a, &["wait", "plan", "--on", "action,mention", "--cursor", &seq]);
    rows.push(("$ annotary wait plan --on action,mention --timeout 30m", 0, 0, wait));
    let status_json = cli.ok(a, &["status", "plan", "--json"]);
    rows.push(("$ annotary status plan --json", 0, 0, status_json));
    let list_json = cli.ok(a, &["thread", "list", "plan", "--json", "id,anchor,body"]);
    rows.push(("$ annotary thread list plan --json id,anchor,body", 0, 0, list_json));

    let bpe = tiktoken_rs::o200k_base().unwrap();
    rows.into_iter()
        .map(|(label, lines, _, text)| {
            let text = normalise(&text, &id);
            let lines = if lines == 0 { text.lines().count() } else { lines };
            let tokens = bpe.encode_ordinary(&text).len();
            (label, lines, tokens, text)
        })
        .collect()
}

fn report(rows: &[(&str, usize, usize, String)]) -> String {
    let mut s = String::from(
        "# Token counts — the agent's reads (o200k), measured from real output\n\n\
         Generated by `crates/annotary/tests/token_report.rs` (`make token-report` regenerates; the test\n\
         asserts every row within ±20% of the CLI prototype's number). The scenario is the prototype's\n\
         (`tests/e2e/95-agent-flow/`); ids, ages and cursors are normalised to the prototype's values so the\n\
         numbers are stable.\n\n\
         | what | lines | tokens | prototype | Δ |\n|---|---:|---:|---:|---:|\n",
    );
    for (label, lines, tokens, _) in rows {
        let proto = PROTOTYPE.iter().find(|(l, _)| l == label).map(|(_, n)| *n).unwrap_or(0);
        let delta = *tokens as i64 - proto as i64;
        s.push_str(&format!("| {label} | {lines} | {tokens} | {proto} | {delta:+} |\n"));
    }
    s
}

// spec: S14-07, S14-01
#[test]
fn agent_reads_stay_within_twenty_percent_of_the_prototype_token_counts() {
    let rows = measure();
    let mut failures = Vec::new();
    for (label, _, tokens, text) in &rows {
        let proto = PROTOTYPE.iter().find(|(l, _)| l == label).map(|(_, n)| *n).expect("a prototype number");
        let lo = (proto as f64 * 0.8).floor() as usize;
        let hi = (proto as f64 * 1.2).ceil() as usize;
        if *tokens < lo || *tokens > hi {
            failures.push(format!("{label}: {tokens} tokens, prototype {proto} (allowed {lo}..={hi})\n{text}"));
        }
    }
    assert!(failures.is_empty(), "token counts drifted beyond ±20% of the prototype:\n{}", failures.join("\n---\n"));
    // The skill's own ceiling (story 147).
    let skill = rows.iter().find(|r| r.0 == "SKILL.md (whole)").unwrap();
    assert!(skill.1 <= 60, "SKILL.md is {} lines (≤ 60)", skill.1);

    // The committed report is the golden of these numbers.
    let path = repo_root().join("tests/token-report.md");
    let want = report(&rows);
    if std::env::var("TOKEN_REPORT").as_deref() == Ok("overwrite") {
        std::fs::write(&path, &want).unwrap();
        return;
    }
    let have = std::fs::read_to_string(&path).unwrap_or_default();
    assert_eq!(
        have, want,
        "tests/token-report.md is stale — regenerate with `make token-report` (TOKEN_REPORT=overwrite) and review the diff"
    );
}
