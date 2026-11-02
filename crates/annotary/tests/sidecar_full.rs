//! Ticket 16's sweep: the Sidecar round trip with every field the final Sidecar can carry, built
//! for real through the binary and the server — three Members, two Versions with messages, Assets
//! with `via` / `missing` / a web derivative, the Vault's Action declaration copied in with
//! `inherited: true` plus a three-row Action log on two Versions, and Threads of every kind
//! (document-level with a mention and a resolution, a span refined by v2 with a `now:` text and a
//! two-entry history, a span orphaned by v2 frozen at its last valid placement, a whole-Asset
//! Thread, a tombstoned Reply, a tombstoned root with a live Reply, `edited_at` on a root and on a
//! Reply). The server's projection parses and re-emits byte-identically, `pull` writes the same
//! bytes, and the pulled tree pushes back as `already`.

use std::path::Path;
use std::process::{Command, Output};

use annotary::testing::TestServer;
use annotary_sidecar::identity::sha256_hex;

fn run(ts: &TestServer, home: &Path, cwd: &Path, token: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_annotary"))
        .args(args)
        .current_dir(cwd)
        .env("HOME", home)
        .env("ANNOTARY_HOME", home.join(".annotary"))
        .env("ANNOTARY_TOKEN", token)
        .env("ANNOTARY_SERVER", &ts.url)
        .env_remove("ANNOTARY_PROFILE")
        .env_remove("ANNOTARY_TTY")
        .output()
        .unwrap()
}

fn ok(out: &Output) -> String {
    assert_eq!(out.status.code(), Some(0), "stderr: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/assets").join(name)).unwrap()
}

fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, bytes).unwrap();
}

const V1: &str = "# Plan\n\
\n\
The skill file ships with the CLI.\n\
\n\
This line will be deleted in v2.\n\
\n\
![arch](img/a.png)\n\
\n\
![bitmap](img/a.bmp)\n\
\n\
![gone](img/missing.png)\n\
\n\
See the [notes](notes.md) for the details.\n\
\n\
The last paragraph stays put.\n";

// v2: a word inserted inside t2's span (refined), t3's line deleted (orphaned), the rest moves.
const V2: &str = "# Plan\n\
\n\
The skill file always ships with the CLI.\n\
\n\
![arch](img/a.png)\n\
\n\
![bitmap](img/a.bmp)\n\
\n\
![gone](img/missing.png)\n\
\n\
See the [notes](notes.md) for the details.\n\
\n\
The last paragraph stays put.\n";

// spec: S16-02, S02-22, S03-14, S04-14, S05-12, S07-11
#[test]
fn the_full_sidecar_round_trips_through_the_server_and_pull() {
    let ts = TestServer::start();
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let owner = ts.owner_token.clone();
    // In handle order: `list_members` sorts by creation time, then handle, and both may land in one millisecond.
    let (_, linter) = ts.add_member("linter");
    let (_, scribe) = ts.add_member("scribe");
    let w = work.path();
    write(w, "plan.md", V1.as_bytes());
    write(w, "img/a.png", &fixture("a.png"));
    write(w, "img/a.bmp", &fixture("a.bmp"));
    write(w, "notes.md", b"# Notes\n\n![s](img/a.svg)\n");
    write(w, "img/a.svg", &fixture("a.svg"));

    // v1 (a missing Asset warns and pushes), then every kind of Thread / Comment event.
    let out = run(&ts, home.path(), w, &owner, &["push", "plan.md", "-m", "first"]);
    let so = ok(&out);
    assert!(so.starts_with("plan\t") && so.trim_end().ends_with("\tv1"), "{so}");
    assert!(String::from_utf8_lossy(&out.stderr).contains("1 missing"));
    // t1: document-level, mentions @scribe, resolved by the reply.
    ok(&run(
        &ts,
        home.path(),
        w,
        &owner,
        &["thread", "new", "plan", "--document", "-m", "Needs a Risks section. @scribe?"],
    ));
    ok(&run(&ts, home.path(), w, &scribe, &["thread", "reply", "plan", "t1", "-m", "Added in v2.", "--resolve"]));
    // t2: a span that v2 refines; its Reply gets edited.
    assert_eq!(
        ok(&run(&ts, home.path(), w, &owner, &["thread", "new", "plan", "3:4-3:20", "-m", "Order matters."])),
        "t2\tv1:3:4-3:20 \"skill file ships\"\n"
    );
    ok(&run(&ts, home.path(), w, &scribe, &["thread", "reply", "plan", "t2", "-m", "Agreed"]));
    ok(&run(&ts, home.path(), w, &scribe, &["thread", "edit", "plan", "t2.1", "-m", "Agreed — item 3 says so."]));
    // t3: a span on the line v2 deletes (orphaned); its root gets edited.
    ok(&run(&ts, home.path(), w, &linter, &["thread", "new", "plan", "5:0-5:9", "-m", "Why this line?"]));
    ok(&run(&ts, home.path(), w, &linter, &["thread", "edit", "plan", "t3", "-m", "Why this line at all?"]));
    // t4: a whole-Asset Thread (anchored by hash, stays).
    assert_eq!(
        ok(&run(
            &ts,
            home.path(),
            w,
            &owner,
            &["thread", "new", "plan", "--asset", "img/a.png", "-m", "Diagram is stale."]
        )),
        "t4\tasset img/a.png\n"
    );
    // t5: a span with a tombstoned Reply.
    ok(&run(&ts, home.path(), w, &owner, &["thread", "new", "plan", "15:4-15:18", "-m", "Keep?"]));
    ok(&run(&ts, home.path(), w, &scribe, &["thread", "reply", "plan", "t5", "-m", "oops"]));
    ok(&run(&ts, home.path(), w, &scribe, &["thread", "delete", "plan", "t5.1"]));
    // t6: a tombstoned root with a live Reply.
    ok(&run(&ts, home.path(), w, &linter, &["thread", "new", "plan", "1:2-1:6", "-m", "Title?"]));
    ok(&run(&ts, home.path(), w, &owner, &["thread", "reply", "plan", "t6", "-m", "Fine as is."]));
    ok(&run(&ts, home.path(), w, &linter, &["thread", "delete", "plan", "t6"]));
    // Actions on v1 by two Members (the Vault defaults `approve` / `send-back`).
    ok(&run(&ts, home.path(), w, &owner, &["act", "plan", "approve"]));
    ok(&run(&ts, home.path(), w, &scribe, &["act", "plan", "send-back"]));

    // v2 (re-anchors: t2 refined, t3 orphaned, t4/t5/t6 anchored, t1 counts as anchored), one more Action on v2.
    write(w, "plan.md", V2.as_bytes());
    let out = run(&ts, home.path(), w, &owner, &["push", "plan.md", "-m", "second"]);
    let so = ok(&out);
    assert!(so.trim_end().ends_with("\tv2"), "{so}");
    let se = String::from_utf8_lossy(&out.stderr).to_string();
    assert!(
        se.contains("threads: 6 re-anchored — 4 anchored, 1 refined, 1 orphaned (+1 refined: t2; +1 orphaned: t3)"),
        "{se}"
    );
    ok(&run(&ts, home.path(), w, &linter, &["act", "plan", "approve"]));

    // The server's projection is canonical: parse → emit is byte-identical and `pull` writes the
    // same bytes.
    let id = std::fs::read_to_string(w.join("plan.annotary"))
        .unwrap()
        .lines()
        .find_map(|l| l.trim().strip_prefix("\"id\": \""))
        .unwrap()
        .trim_end_matches("\",")
        .to_string();
    let text = {
        let mut r = ureq::get(format!("{}/api/v1/docs/{id}/sidecar", ts.url))
            .header("Authorization", format!("Bearer {owner}"))
            .call()
            .unwrap();
        r.body_mut().read_to_string().unwrap()
    };
    let sc = annotary_sidecar::parse(&text).unwrap();
    assert_eq!(annotary_sidecar::emit(&sc), text, "emit(parse(text)) == text");
    let pulled = tempfile::tempdir().unwrap();
    ok(&run(&ts, home.path(), pulled.path(), &owner, &["pull", "plan", "--assets"]));
    assert_eq!(std::fs::read_to_string(pulled.path().join("plan.annotary")).unwrap(), text);

    // ---- every section, every field ----
    assert_eq!((sc.format, sc.id.as_str(), sc.vault.as_str(), sc.base_version), (1, id.as_str(), "notes", 2));
    // members: every handle referenced, with id + name.
    assert_eq!(sc.members.0.iter().map(|(h, _)| h.as_str()).collect::<Vec<_>>(), vec!["rowan", "linter", "scribe"]);
    assert!(sc.members.0.iter().all(|(_, m)| m.id.len() == 26 && !m.name.is_empty()));
    // versions[] with messages.
    assert_eq!(
        sc.versions.iter().map(|v| (v.number, v.message.as_deref())).collect::<Vec<_>>(),
        vec![(1, Some("first")), (2, Some("second"))]
    );
    assert!(
        sc.versions.iter().all(|v| v.name == "plan" && v.by == "rowan" && v.hash.len() == 64 && v.at.ends_with('Z'))
    );
    // assets[] of base_version: `{ref, hash, size, type, missing?, via?}` — the BMP's entry
    // carries the ORIGINAL hash (its web derivative lives on the Version record, never in the
    // Sidecar: identity = (ref, hash) pairs only).
    let asset = |r: &str| sc.assets.iter().find(|a| a.reference == r).unwrap_or_else(|| panic!("asset {r}"));
    let png = asset("img/a.png");
    assert_eq!(
        (png.hash.as_deref(), png.content_type.as_str(), png.missing, png.via.as_deref()),
        (Some(sha256_hex(&fixture("a.png")).as_str()), "image/png", false, None)
    );
    assert_eq!(png.size, Some(fixture("a.png").len() as u64));
    let bmp = asset("img/a.bmp");
    assert_eq!(
        (bmp.hash.as_deref(), bmp.content_type.as_str()),
        (Some(sha256_hex(&fixture("a.bmp")).as_str()), "image/bmp")
    );
    assert!(!text.contains("derivative"), "derivatives are not in the Sidecar");
    let missing = asset("img/missing.png");
    assert_eq!(
        (missing.hash.is_none(), missing.size.is_none(), missing.missing, missing.content_type.as_str()),
        (true, true, true, "image/png")
    );
    let notes = asset("notes.md");
    assert_eq!((notes.content_type.as_str(), notes.via.as_deref()), ("text/markdown", None));
    let svg = asset("img/a.svg");
    assert_eq!((svg.content_type.as_str(), svg.via.as_deref()), ("image/svg+xml", Some("notes.md")));
    // The Version record carries what the Sidecar leaves out.
    let (_, v2) = ts.get(&format!("/api/v1/docs/{id}/versions/2"), Some(&owner));
    let manifest = v2["manifest"].as_array().unwrap();
    let entry = |r: &str| manifest.iter().find(|e| e["ref"] == r).unwrap();
    assert!(entry("img/a.bmp")["derivative"].as_str().is_some_and(|d| d.len() == 64));
    assert!(entry("img/a.png")["width"].as_u64().is_some() && entry("img/a.png")["height"].as_u64().is_some());
    // actions: the Vault's declaration copied in (`inherited: true`) + the log on two Versions.
    assert!(sc.actions.inherited);
    assert_eq!(
        sc.actions.available.iter().map(|d| (d.id.as_str(), d.label.as_str(), d.intent.as_str())).collect::<Vec<_>>(),
        vec![("approve", "Approve", "positive"), ("send-back", "Send back", "neutral")]
    );
    assert_eq!(
        sc.actions
            .log
            .iter()
            .map(|e| (e.id.as_str(), e.by.as_str(), e.version, e.label.as_str(), e.intent.as_str()))
            .collect::<Vec<_>>(),
        vec![
            ("approve", "rowan", 1, "Approve", "positive"),
            ("send-back", "scribe", 1, "Send back", "neutral"),
            ("approve", "linter", 2, "Approve", "positive")
        ]
    );
    // threads[]: every Thread by id, lossless.
    let t = |id: &str| sc.threads.iter().find(|t| t.id == id).unwrap_or_else(|| panic!("thread {id}"));
    assert_eq!(sc.threads.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), vec!["t1", "t2", "t3", "t4", "t5", "t6"]);
    let t1 = t("t1");
    assert_eq!(
        (t1.subject.as_str(), t1.anchor.is_none(), t1.author.as_str(), t1.created.version),
        ("document", true, "rowan", 1)
    );
    assert_eq!(t1.mentions, vec!["scribe"]);
    let res = t1.resolved.as_ref().unwrap();
    assert_eq!((res.by.as_str(), res.version), ("scribe", 1));
    assert_eq!(
        (t1.replies[0].id.as_str(), t1.replies[0].author.as_str(), t1.replies[0].body.as_str()),
        ("t1.1", "scribe", "Added in v2.")
    );
    let t2 = t("t2");
    let a = t2.anchor.as_ref().unwrap();
    assert_eq!(
        (a.original.version, a.original.range.as_deref(), a.original.exact.as_deref()),
        (1, Some("3:4-3:20"), Some("skill file ships"))
    );
    assert_eq!(
        (a.current.version, a.current.range.as_deref(), a.current.status.as_deref(), a.current.exact.as_deref()),
        (2, Some("3:4-3:27"), Some("refined"), Some("skill file always ships"))
    );
    assert_eq!(
        a.history.iter().map(|h| (h.version, h.range.as_deref(), h.status.as_deref())).collect::<Vec<_>>(),
        vec![(1, Some("3:4-3:20"), Some("anchored")), (2, Some("3:4-3:27"), Some("refined"))]
    );
    assert!(a.history.iter().all(|h| h.exact.is_none()), "history rows carry no text");
    assert!(t2.replies[0].edited_at.is_some() && t2.replies[0].body == "Agreed — item 3 says so.");
    let t3 = t("t3");
    let a = t3.anchor.as_ref().unwrap();
    assert_eq!(
        (a.current.version, a.current.range.as_deref(), a.current.status.as_deref(), a.current.exact.is_none()),
        (1, Some("5:0-5:9"), Some("orphaned"), true)
    );
    assert_eq!(
        a.history.iter().map(|h| (h.version, h.status.as_deref())).collect::<Vec<_>>(),
        vec![(1, Some("anchored")), (2, Some("orphaned"))]
    );
    assert!(t3.edited_at.is_some() && t3.body == "Why this line at all?" && t3.resolved.is_none());
    let t4 = t("t4");
    let a = t4.anchor.as_ref().unwrap();
    assert_eq!(t4.subject, "asset");
    assert_eq!(
        (a.original.version, a.original.hash.as_deref(), a.original.reference.as_deref(), a.original.line),
        (1, Some(sha256_hex(&fixture("a.png")).as_str()), Some("img/a.png"), Some(7))
    );
    assert_eq!(
        (a.current.version, a.current.line, a.current.status.as_deref(), a.current.range.is_none()),
        (2, Some(5), Some("anchored"), true)
    );
    assert_eq!(
        a.history.iter().map(|h| (h.version, h.line, h.status.as_deref())).collect::<Vec<_>>(),
        vec![(1, Some(7), Some("anchored")), (2, Some(5), Some("anchored"))]
    );
    let t5 = t("t5");
    assert_eq!(t5.anchor.as_ref().unwrap().current.status.as_deref(), Some("anchored"));
    let r = &t5.replies[0];
    assert_eq!(
        (r.id.as_str(), r.body.as_str(), r.deleted.as_ref().map(|d| d.by.as_str())),
        ("t5.1", "", Some("scribe"))
    );
    let t6 = t("t6");
    assert_eq!(
        (t6.body.as_str(), t6.deleted.as_ref().map(|d| d.by.as_str()), t6.replies[0].body.as_str()),
        ("", Some("linter"), "Fine as is.")
    );
    // The text itself: the decided top-level order and every key name the format defines.
    let keys = [
        "\"format\"",
        "\"id\"",
        "\"vault\"",
        "\"base_version\"",
        "\"members\"",
        "\"versions\"",
        "\"assets\"",
        "\"actions\"",
        "\"threads\"",
    ];
    let mut last = 0;
    for k in keys {
        let pos = text.find(k).unwrap_or_else(|| panic!("{k} in the Sidecar"));
        assert!(pos >= last, "{k} in order");
        last = pos;
    }
    for k in [
        "\"message\"",
        "\"via\"",
        "\"missing\": true",
        "\"inherited\": true",
        "\"log\"",
        "\"history\"",
        "\"exact\"",
        "\"hash\"",
        "\"ref\"",
        "\"line\"",
        "\"mentions\"",
        "\"edited_at\"",
        "\"deleted\"",
        "\"resolved\"",
        "\"replies\"",
    ] {
        assert!(text.contains(k), "{k} in the Sidecar");
    }
    // The pulled tree pushes back as `already v2` (Assets byte-identical, declaration inherited ⇒ nothing sent).
    let out = run(&ts, home.path(), pulled.path(), &owner, &["push", "plan.md"]);
    assert!(ok(&out).trim_end().ends_with("\talready v2"), "{}", String::from_utf8_lossy(&out.stdout));
}
