//! Spec-id traceability checker: every id in `tests/SPEC.md` (outside `## Deferred`) must be
//! named by at least one `spec:` marker somewhere under `crates/`, `tests/` or `perf/`, every
//! marker must name a listed (or Deferred) id, no id may be both listed and Deferred, and the
//! `## Stories` map must name every story of the spec and only known ids (ticket 16). See
//! `tests/SPEC.md` for the scheme.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// Extract `Sтт-nn` ids from a string starting at `from`.
fn ids_in(s: &str) -> Vec<String> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i + 6 <= b.len() {
        let is_start = i == 0 || !(b[i - 1].is_ascii_alphanumeric());
        if is_start
            && b[i] == b'S'
            && b[i + 1].is_ascii_digit()
            && b[i + 2].is_ascii_digit()
            && b[i + 3] == b'-'
            && b[i + 4].is_ascii_digit()
            && b[i + 5].is_ascii_digit()
            && (i + 6 == b.len() || !b[i + 6].is_ascii_alphanumeric())
        {
            out.push(s[i..i + 6].to_string());
            i += 6;
        } else {
            i += 1;
        }
    }
    out
}

/// What SPEC.md says.
#[derive(Default)]
struct Spec {
    /// Ids under the ticket sections (need a marker).
    listed: BTreeSet<String>,
    /// Ids under `## Deferred` (need none).
    deferred: BTreeSet<String>,
    /// `## Stories`: story number → the ids named on its line (ticket 16).
    stories: BTreeMap<u32, Vec<String>>,
}

/// Parse SPEC.md: the listed ids, the deferred ids and the story map.
fn parse_spec(text: &str) -> Spec {
    let mut spec = Spec::default();
    let mut section = String::new();
    for line in text.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            section = h.to_lowercase();
            continue;
        }
        let Some(rest) = line.strip_prefix("- ") else { continue };
        if section.starts_with("stories") {
            // `- 7 — loopback-only bind → S01-01`
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(n) = num.parse::<u32>() {
                spec.stories.insert(n, ids_in(rest));
            }
            continue;
        }
        let Some(id) = ids_in(rest).into_iter().next() else { continue };
        if !rest.starts_with(&id) {
            continue;
        }
        if section.contains("deferred") {
            spec.deferred.insert(id);
        } else {
            spec.listed.insert(id);
        }
    }
    spec
}

/// The story numbers of the spec's "User Stories" (`N. As a …`), when the spec is in the repo;
/// else every number up to the largest one SPEC.md maps (so the map must at least be gap-free).
fn story_numbers(root: &Path, mapped: &BTreeMap<u32, Vec<String>>) -> BTreeSet<u32> {
    let spec = root.join("docs/phase1/spec.md");
    if let Ok(text) = std::fs::read_to_string(&spec) {
        let mut out = BTreeSet::new();
        let mut in_stories = false;
        for line in text.lines() {
            if let Some(h) = line.strip_prefix("## ") {
                in_stories = h.trim() == "User Stories";
                continue;
            }
            if !in_stories {
                continue;
            }
            let num: String = line.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !num.is_empty() && line[num.len()..].starts_with(". As ") {
                out.insert(num.parse().unwrap());
            }
        }
        assert!(out.len() >= 150, "the spec lists {} stories in {}", out.len(), spec.display());
        return out;
    }
    (1..=mapped.keys().next_back().copied().unwrap_or(0)).collect()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_string();
        if p.is_dir() {
            if matches!(name.as_str(), "target" | "node_modules" | ".git" | "dump" | "dist") {
                continue;
            }
            walk(&p, out);
        } else if matches!(
            p.extension().and_then(|e| e.to_str()),
            Some("rs" | "toml" | "trycmd" | "md" | "ts" | "js" | "py" | "sh")
        ) {
            out.push(p);
        }
    }
}

/// Collect `spec:` markers → id → files.
fn collect_markers(root: &Path) -> BTreeMap<String, BTreeSet<String>> {
    let mut files = Vec::new();
    walk(&root.join("crates"), &mut files);
    walk(&root.join("tests"), &mut files);
    walk(&root.join("perf"), &mut files); // ticket 15: the perf runner's `// spec:` markers
    let spec_md = root.join("tests/SPEC.md");
    let mut found: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for f in files {
        if f == spec_md {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        for line in text.lines() {
            let mut rest = line;
            while let Some(pos) = rest.find("spec:") {
                let after = &rest[pos + 5..];
                let trimmed = after.trim_start();
                // Only a marker when an id follows immediately (avoids prose like "the spec: says").
                if ids_in(trimmed).first().is_some_and(|id| trimmed.starts_with(id.as_str())) {
                    for id in ids_in(after) {
                        found.entry(id).or_default().insert(f.strip_prefix(root).unwrap().display().to_string());
                    }
                }
                rest = after;
            }
        }
    }
    found
}

// spec: S16-05
#[test]
fn every_spec_id_has_a_test_and_every_marker_is_listed() {
    let root = repo_root();
    let text = std::fs::read_to_string(root.join("tests/SPEC.md")).expect("tests/SPEC.md exists");
    let Spec { listed, deferred, stories } = parse_spec(&text);
    assert!(!listed.is_empty(), "tests/SPEC.md lists no ids");
    assert!(!deferred.is_empty(), "tests/SPEC.md has no Deferred list");
    let markers = collect_markers(&root);

    let uncovered: Vec<&String> = listed.iter().filter(|id| !markers.contains_key(*id)).collect();
    let unknown: Vec<(&String, &BTreeSet<String>)> =
        markers.iter().filter(|(id, _)| !listed.contains(*id) && !deferred.contains(*id)).collect();
    let both: Vec<&String> = listed.intersection(&deferred).collect();

    let mut msg = String::new();
    if !uncovered.is_empty() {
        msg.push_str(&format!(
            "spec ids with zero tests: {}\n",
            uncovered.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    for (id, files) in &unknown {
        msg.push_str(&format!(
            "marker names unknown id {id} in {}\n",
            files.iter().cloned().collect::<Vec<_>>().join(", ")
        ));
    }
    if !both.is_empty() {
        msg.push_str(&format!(
            "ids both listed and Deferred: {}\n",
            both.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ));
    }
    // ---- ticket 16: the story map names every story of the spec and only known ids ----
    let expected = story_numbers(&root, &stories);
    let missing: Vec<String> = expected.iter().filter(|n| !stories.contains_key(n)).map(u32::to_string).collect();
    if !missing.is_empty() {
        msg.push_str(&format!("stories missing from the `## Stories` map: {}\n", missing.join(", ")));
    }
    for (n, ids) in &stories {
        if !expected.contains(n) {
            msg.push_str(&format!("story {n} is not in the spec\n"));
        }
        if ids.is_empty() {
            msg.push_str(&format!("story {n} maps to no id\n"));
        }
        for id in ids {
            if !listed.contains(id) && !deferred.contains(id) {
                msg.push_str(&format!("story {n} maps to unknown id {id}\n"));
            }
        }
    }
    assert!(msg.is_empty(), "{msg}");
}

// spec: S16-05
#[test]
fn id_and_marker_parsing() {
    assert_eq!(ids_in("- S01-03 — x (S01-04)"), vec!["S01-03", "S01-04"]);
    assert_eq!(ids_in("XS01-03 S1-03 S01-3"), Vec::<String>::new());
    let spec = parse_spec(
        "## A\n- S01-01 — a\n- not an id\n## Stories\n- 7 — bind → S01-01, S09-99\n- 8 — x → S01-01\n## Deferred\n- S09-99 — later\n",
    );
    assert_eq!(spec.listed.into_iter().collect::<Vec<_>>(), vec!["S01-01"]);
    assert_eq!(spec.deferred.into_iter().collect::<Vec<_>>(), vec!["S09-99"]);
    assert_eq!(
        spec.stories,
        BTreeMap::from([(7, vec!["S01-01".to_string(), "S09-99".to_string()]), (8, vec!["S01-01".to_string()])])
    );
}

// spec: S16-05
#[test]
fn the_checker_fails_on_an_uncovered_id_an_unknown_marker_and_a_missing_story() {
    // The checks above, on synthetic inputs: an id with no marker that is not Deferred, a marker
    // naming an unlisted id, an id both listed and Deferred, a story mapped to nothing / to an
    // unknown id, a story number missing from the map.
    let spec = parse_spec(
        "## Ticket 01\n- S01-01 — a\n- S01-02 — b\n## Stories\n- 1 — a → S01-01\n- 2 — b → S77-01\n- 3 — c →\n## Deferred\n- S01-02 — d\n- S09-99 — later\n",
    );
    let mut markers: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    markers.insert("S01-01".into(), ["t.rs".to_string()].into_iter().collect());
    markers.insert("S42-01".into(), ["t.rs".to_string()].into_iter().collect());
    let uncovered: Vec<&String> = spec.listed.iter().filter(|id| !markers.contains_key(*id)).collect();
    assert_eq!(uncovered, vec!["S01-02"], "S01-02 is listed and unmarked (its Deferred twin does not excuse it)");
    let unknown: Vec<&String> =
        markers.keys().filter(|id| !spec.listed.contains(*id) && !spec.deferred.contains(*id)).collect();
    assert_eq!(unknown, vec!["S42-01"]);
    assert_eq!(spec.listed.intersection(&spec.deferred).collect::<Vec<_>>(), vec!["S01-02"]);
    let expected: BTreeSet<u32> = (1..=4).collect();
    let missing: Vec<u32> = expected.iter().filter(|n| !spec.stories.contains_key(n)).copied().collect();
    assert_eq!(missing, vec![4]);
    assert!(spec.stories[&3].is_empty());
    assert!(spec.stories[&2].iter().any(|id| !spec.listed.contains(id) && !spec.deferred.contains(id)));
}
