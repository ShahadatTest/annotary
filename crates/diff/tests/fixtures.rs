//! The empirical fixture table drives the crate: every directory under `fixtures/anchoring/`
//! (`{v1.md, v2.md, …, [vN.manifest.json], threads.json, expected.json}`, one per row of the
//! research table D1–D12 / G0–G14 plus the ticket-05 additions — `make.py` regenerates them)
//! is replayed Version by Version through [`annotary_diff::reanchor`] and must reproduce
//! `expected.json` exactly (status, range, current text; asset Threads by content hash). A
//! second checker fails when a row id of the research doc has no directory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use annotary_diff::{Placement, Range, Span, Status, asset_status, range::extract, reanchor};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

fn fixtures_dir() -> PathBuf {
    root().join("fixtures/anchoring")
}

/// The three corpus Documents with a Version history (`fixtures/corpus/README.md`): the same
/// files as an anchoring fixture, at document scale.
fn corpus_histories() -> Vec<Fixture> {
    ["glossary-multilingual", "onboarding", "rfc-tile-cache"]
        .iter()
        .map(|n| load(&root().join("fixtures/corpus").join(n)))
        .collect()
}

#[derive(Debug, Deserialize)]
struct ThreadSpec {
    id: String,
    subject: String,
    version: u32,
    #[serde(default)]
    range: Option<String>,
    #[serde(rename = "ref", default)]
    reference: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq)]
struct Expected {
    version: u32,
    status: String,
    #[serde(default)]
    range: Option<String>,
    #[serde(default)]
    exact: Option<String>,
    #[serde(default)]
    line: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct ManifestEntry {
    #[serde(rename = "ref")]
    reference: String,
    hash: String,
}

/// One loaded fixture directory.
struct Fixture {
    id: String,
    versions: Vec<String>,
    manifests: BTreeMap<u32, Vec<ManifestEntry>>,
    threads: Vec<ThreadSpec>,
    expected: BTreeMap<String, Vec<Expected>>,
}

fn load(dir: &Path) -> Fixture {
    let id = dir.file_name().unwrap().to_string_lossy().to_string();
    let mut versions = Vec::new();
    for n in 1.. {
        let p = dir.join(format!("v{n}.md"));
        if !p.exists() {
            break;
        }
        versions.push(std::fs::read_to_string(&p).unwrap());
    }
    assert!(!versions.is_empty(), "{id}: no v1.md");
    let mut manifests = BTreeMap::new();
    for n in 1..=versions.len() as u32 {
        let p = dir.join(format!("v{n}.manifest.json"));
        if p.exists() {
            manifests.insert(n, serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap());
        }
    }
    let threads: Vec<ThreadSpec> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("threads.json")).unwrap()).unwrap();
    let expected: BTreeMap<String, Vec<Expected>> =
        serde_json::from_str(&std::fs::read_to_string(dir.join("expected.json")).unwrap()).unwrap();
    Fixture { id, versions, manifests, threads, expected }
}

fn all_fixtures() -> Vec<Fixture> {
    let mut dirs: Vec<PathBuf> =
        std::fs::read_dir(fixtures_dir()).unwrap().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    dirs.sort();
    dirs.iter().map(|d| load(d)).collect()
}

/// Replay one Thread through every Version after its creation: the full (dense) history the
/// sparse Sidecar history must be able to reproduce. `reverse` feeds the spans of a Version in
/// reverse order (the function must not care).
fn replay(f: &Fixture, t: &ThreadSpec) -> Vec<Expected> {
    let n = f.versions.len() as u32;
    let mut out = Vec::new();
    match t.subject.as_str() {
        "document" => {
            for v in t.version..=n {
                out.push(Expected { version: v, status: "anchored".into(), range: None, exact: None, line: None });
            }
        }
        "asset" => {
            let reference = t.reference.as_deref().expect("asset ref");
            let hash_at = |v: u32| -> Option<String> {
                f.manifests.get(&v).and_then(|m| m.iter().find(|e| e.reference == reference).map(|e| e.hash.clone()))
            };
            let hash = hash_at(t.version)
                .unwrap_or_else(|| panic!("{}: {} not in v{}.manifest.json", f.id, reference, t.version));
            let mut orphaned = false;
            for v in t.version..=n {
                let present = f.manifests.get(&v).is_some_and(|m| m.iter().any(|e| e.hash == hash));
                let st = if v == t.version { Status::Anchored } else { asset_status(orphaned, present) };
                orphaned = st == Status::Orphaned;
                out.push(Expected { version: v, status: st.as_str().into(), range: None, exact: None, line: None });
            }
        }
        "span" => {
            let created = &f.versions[t.version as usize - 1];
            let range = Range::parse(t.range.as_deref().expect("span range")).unwrap();
            let original = extract(created, &range).unwrap();
            out.push(Expected {
                version: t.version,
                status: "anchored".into(),
                range: Some(range.to_string()),
                exact: Some(original.clone()),
                line: None,
            });
            let mut span = Span { range, original: original.clone(), orphaned: false };
            for v in t.version + 1..=n {
                let p = reanchor(&f.versions[v as usize - 2], &f.versions[v as usize - 1], std::slice::from_ref(&span))
                    .unwrap()
                    .remove(0);
                match p {
                    Placement { status: Status::Orphaned, .. } => {
                        span.orphaned = true;
                        out.push(Expected {
                            version: v,
                            status: "orphaned".into(),
                            range: None,
                            exact: None,
                            line: None,
                        });
                    }
                    Placement { status, range: Some(r), exact } => {
                        span.range = r;
                        let exact = exact.unwrap_or_else(|| original.clone());
                        assert_eq!(
                            extract(&f.versions[v as usize - 1], &r).unwrap(),
                            exact,
                            "{}/{} v{v}: the range must cover exactly the reported text",
                            f.id,
                            t.id
                        );
                        out.push(Expected {
                            version: v,
                            status: status.as_str().into(),
                            range: Some(r.to_string()),
                            exact: Some(exact),
                            line: None,
                        });
                    }
                    other => panic!("{}/{}: {other:?}", f.id, t.id),
                }
            }
        }
        other => panic!("{}: unknown subject {other}", f.id),
    }
    out
}

// spec: S07-01, S07-02, S07-03, S07-04, S07-09
#[test]
fn every_fixture_directory_reproduces_its_expected_history() {
    let fixtures = all_fixtures();
    assert!(fixtures.len() >= 40, "{} fixture directories", fixtures.len());
    let failures = mismatches(&fixtures);
    assert!(failures.is_empty(), "{} fixture mismatches:\n{}", failures.len(), failures.join("\n"));
}

// spec: S32-01
#[test]
fn the_corpus_histories_reproduce_their_expected_history() {
    let fixtures = corpus_histories();
    for f in &fixtures {
        assert_eq!(f.versions.len(), 4, "{}: v1 to v4", f.id);
        assert!(f.threads.len() >= 10, "{}: {} Threads", f.id, f.threads.len());
        // Every kind of outcome turns up at document scale, not only in the abstract rows.
        let statuses: Vec<&str> = f.expected.values().flatten().map(|e| e.status.as_str()).collect();
        for want in ["anchored", "refined", "orphaned"] {
            assert!(statuses.contains(&want), "{}: no {want} placement", f.id);
        }
    }
    let failures = mismatches(&fixtures);
    assert!(failures.is_empty(), "{} corpus mismatches:\n{}", failures.len(), failures.join("\n"));
}

// spec: S32-01
#[test]
fn the_corpus_replays_the_same_in_any_order() {
    for f in corpus_histories() {
        for (i, w) in f.versions.windows(2).enumerate() {
            let n = i as u32 + 1;
            // Each Thread as it stands on Version n, from its declared history.
            let spans: Vec<Span> = f
                .threads
                .iter()
                .filter(|t| t.subject == "span")
                .filter(|t| t.version <= n)
                .filter_map(|t| {
                    // A Thread alive at n has an expectation there; only an orphan has no range.
                    let at = f.expected[&t.id]
                        .iter()
                        .find(|e| e.version == n)
                        .unwrap_or_else(|| panic!("{} {} has no expectation at v{n}", f.id, t.id));
                    let range = Range::parse(at.range.as_deref()?).unwrap();
                    let created = Range::parse(t.range.as_deref().unwrap()).unwrap();
                    let original = extract(&f.versions[t.version as usize - 1], &created).unwrap();
                    Some(Span { range, original, orphaned: false })
                })
                .collect();
            let once = reanchor(&w[0], &w[1], &spans).unwrap();
            let rev: Vec<Span> = spans.iter().rev().cloned().collect();
            let mut back = reanchor(&w[0], &w[1], &rev).unwrap();
            back.reverse();
            assert_eq!(once, back, "{} v{n}: a span's placement does not depend on the others", f.id);
        }
    }
}

/// Every Thread of every fixture replayed against its `expected.json`.
fn mismatches(fixtures: &[Fixture]) -> Vec<String> {
    let mut failures = Vec::new();
    for f in fixtures {
        for t in &f.threads {
            let want = f.expected.get(&t.id).unwrap_or_else(|| panic!("{}: expected.json lacks {}", f.id, t.id));
            let got = replay(f, t);
            // Asset expectations carry the display line, which is not this crate's business.
            let want_cmp: Vec<Expected> = want
                .iter()
                .map(|e| Expected {
                    version: e.version,
                    status: e.status.clone(),
                    range: e.range.clone(),
                    exact: e.exact.clone(),
                    line: None,
                })
                .collect();
            if got != want_cmp {
                failures.push(format!("{}/{}:\n  want {want_cmp:#?}\n  got  {got:#?}", f.id, t.id));
            }
        }
    }
    failures
}

// spec: S07-05
#[test]
fn reanchoring_is_deterministic_and_order_independent() {
    for f in all_fixtures() {
        for w in f.versions.windows(2) {
            let (a, b) = (&w[0], &w[1]);
            let spans: Vec<Span> = f
                .threads
                .iter()
                .filter(|t| t.subject == "span" && t.version == 1)
                .map(|t| {
                    let range = Range::parse(t.range.as_deref().unwrap()).unwrap();
                    Span { range, original: extract(&f.versions[0], &range).unwrap(), orphaned: false }
                })
                .collect();
            if spans.is_empty() {
                continue;
            }
            let once = reanchor(a, b, &spans).unwrap();
            let twice = reanchor(a, b, &spans).unwrap();
            assert_eq!(once, twice, "{}: same inputs, same output", f.id);
            let rev: Vec<Span> = spans.iter().rev().cloned().collect();
            let mut back = reanchor(a, b, &rev).unwrap();
            back.reverse();
            assert_eq!(once, back, "{}: a span's placement does not depend on the others", f.id);
            let solo: Vec<Placement> =
                spans.iter().map(|s| reanchor(a, b, std::slice::from_ref(s)).unwrap().remove(0)).collect();
            assert_eq!(once, solo, "{}: alone or together, same placement", f.id);
        }
    }
}

/// The additions the ticket asks for by name (over and above the research rows).
const ADDITIONS: &[&str] = &[
    "whitespace-only",
    "type-over",
    "replace-swallows-span",
    "overlapping-replace",
    "quote-restored",
    "moved-block",
    "whitespace-survivor",
    "multiline-inner-edit",
    "boundary-insertion",
    "copy-paste",
    "asset-hash-moved",
    "asset-hash-replaced",
    "document-level",
    "determinism",
];

// spec: S07-04
#[test]
fn every_row_of_the_research_table_has_a_fixture_directory() {
    let doc = root().join("fixtures/anchoring/TABLE.md");
    let text = std::fs::read_to_string(&doc).expect("the research fixture table is in the repo");
    let mut rows: Vec<String> = Vec::new();
    for line in text.lines() {
        // Table rows look like `| D1 | …` / `| G14 | …`.
        let Some(rest) = line.strip_prefix("| ") else { continue };
        let Some((cell, _)) = rest.split_once(" |") else { continue };
        let cell = cell.trim();
        let is_row = (cell.starts_with('D') || cell.starts_with('G'))
            && cell.len() >= 2
            && cell[1..].chars().next().is_some_and(|c| c.is_ascii_digit());
        if is_row && !rows.contains(&cell.to_string()) {
            rows.push(cell.to_string());
        }
    }
    assert!(rows.len() >= 26, "found {} row ids in {}: {rows:?}", rows.len(), doc.display());
    let mut missing = Vec::new();
    for r in rows.iter().map(String::as_str).chain(ADDITIONS.iter().copied()) {
        let dir = r.replace('\'', "-prime");
        if !fixtures_dir().join(&dir).is_dir() {
            missing.push(dir);
        }
    }
    assert!(missing.is_empty(), "fixture directories missing for {missing:?}");
}
