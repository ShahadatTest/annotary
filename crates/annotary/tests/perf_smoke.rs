//! The perf suite's smoke variant (ticket 15): the generator (`crates/perfgen`) at its small
//! scale, seeded through the real binary against the in-process server — so the generator, the
//! seeder and the fixture's shape cannot rot between `make perf` runs — plus the page-level
//! guarantees the budgets rest on (every Thread in the first HTML response, media boxes from
//! manifest dimensions, the deferred module script) and the perf suite's own plumbing
//! (`perf/BUDGETS.md` lists exactly the six budgets, `make perf` builds release, where it runs is
//! documented). The real numbers are `make perf` (release build, Playwright + CDP).

use std::path::{Path, PathBuf};

use annotary::testing::TestServer;
use annotary_perfgen::{Handles, Plan, Scale, Step, Subject, Target, seed};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

/// A browser-like page request (loopback, `Accept: text/html`, no credential): status + body.
fn page(ts: &TestServer, path: &str) -> (u16, String) {
    let mut r = ureq::get(format!("{}{path}", ts.url))
        .header("Accept", "text/html")
        .config()
        .http_status_as_error(false)
        .max_redirects(0)
        .build()
        .call()
        .unwrap();
    (r.status().as_u16(), r.body_mut().read_to_string().unwrap_or_default())
}

// spec: S15-01, S15-05, S15-06
#[test]
fn smoke_scale_seeds_through_the_real_binary_and_the_page_carries_every_thread() {
    let ts = TestServer::start();
    let tmp = tempfile::tempdir().unwrap();
    let plan = Plan::generate(&Scale::SMOKE, 1, &Handles::default());
    // Same seed, same plan — the fixture is deterministic (the crate's unit tests say so too;
    // this is the seam `make perf` relies on).
    assert_eq!(plan, Plan::generate(&Scale::SMOKE, 1, &Handles::default()));
    let target = Target {
        bin: PathBuf::from(env!("CARGO_BIN_EXE_annotary")),
        server: ts.url.clone(),
        owner_token: ts.owner_token.clone(),
        home: tmp.path().join("home"),
        work: tmp.path().join("work"),
    };
    let mut log = Vec::new();
    let seeded = seed(&plan, &target, &mut |l: &str| log.push(l.to_string()))
        .unwrap_or_else(|e| panic!("seed: {e}\n{}", log.join("\n")));

    // ---- the Document the generator promised ----
    assert_eq!(seeded.head, Scale::SMOKE.versions);
    assert_eq!(seeded.threads, plan.stats.threads);
    assert!(seeded.threads >= Scale::SMOKE.threads, "{:?}", seeded.statuses);
    assert!(plan.stats.lines_v1 >= Scale::SMOKE.lines && plan.stats.words_v1 >= Scale::SMOKE.words);
    // Mixed placements on the head: anchored, refined and orphaned are all present; Document-level
    // Threads have no placement; an Asset Thread of the image dropped in the last Version orphaned.
    assert!(seeded.statuses.anchored >= 1, "{:?}", seeded.statuses);
    assert!(seeded.statuses.refined >= 1, "{:?}", seeded.statuses);
    assert!(seeded.statuses.orphaned >= 1, "{:?}", seeded.statuses);
    assert_eq!(seeded.statuses.document, plan.stats.document_threads);
    assert_eq!(seeded.replies, plan.stats.replies);
    assert!(seeded.resolved >= 1);
    assert_eq!(seeded.list_ids.len(), Scale::SMOKE.list_docs);
    assert_eq!(seeded.pushes_ms.len(), Scale::SMOKE.versions as usize);
    // Actions on two Versions, through the real API.
    let (status, acts) = ts.get(&format!("/api/v1/docs/{}/actions", seeded.doc_id), Some(&ts.owner_token));
    assert_eq!(status, 200, "{acts}");
    let versions: Vec<u64> =
        acts["actions"].as_array().unwrap().iter().map(|a| a["version"].as_u64().unwrap()).collect();
    assert_eq!(versions.len(), 2, "{acts}");
    assert_ne!(versions[0], versions[1]);
    // Mentions resolved to Members (the three handles exist).
    let (status, threads) =
        ts.get(&format!("/api/v1/docs/{}/threads?status=all&hidden=1", seeded.doc_id), Some(&ts.owner_token));
    assert_eq!(status, 200);
    let with_mentions = threads["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| t["mentions"].as_array().is_some_and(|m| !m.is_empty()))
        .count();
    assert!(with_mentions >= 1, "no Thread root mentions anyone");

    // ---- the page: one response with every Thread at its block, media boxes sized ----
    let (status, html) = page(&ts, &format!("/d/{}", seeded.doc_id));
    assert_eq!(status, 200, "{html}");
    let visible: Vec<String> = threads["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|t| !t["hidden"].as_bool().unwrap_or(false))
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert!(visible.len() >= Scale::SMOKE.threads);
    for id in &visible {
        assert!(html.contains(&format!("data-t=\"{id}\"")), "Thread {id} missing from the first response");
    }
    assert!(html.contains("<script type=\"module\">"), "the script is an inline module (deferred)");
    assert!(html.contains("content-visibility:auto"), "viewport-first rendering is in the stylesheet");
    // Every image the head still references has its manifest dimensions on the page.
    let head_md = plan.versions.last().unwrap();
    for im in &plan.images {
        if head_md.contains(&im.reference) {
            let needle = format!("width=\"{}\" height=\"{}\"", im.width, im.height);
            assert!(html.contains(&needle), "image {} lacks its dimensions ({needle})", im.reference);
        }
    }
    // The embedded placements agree with the API: one `#cd-data` entry per visible Thread.
    let start = html.find("<script type=\"application/json\" id=\"cd-data\">").unwrap();
    let data = &html[start..];
    for id in &visible {
        assert!(data.contains(&format!("\"id\":\"{id}\"")), "{id} not in #cd-data");
    }
    // Every planned span was on the head it was planned for (the generator's own coordinates).
    let mut head = 0usize;
    for s in &plan.steps {
        match s {
            Step::Push { version, .. } => head = *version as usize,
            Step::Thread { subject: Subject::Span { range, exact }, .. } => {
                assert_eq!(
                    annotary_perfgen::plan::extract(&plan.versions[head - 1], range).as_deref(),
                    Some(exact.as_str())
                );
            }
            _ => {}
        }
    }
}

// spec: S15-04, S15-07
#[test]
fn budgets_live_only_in_perf_budgets_md_and_the_runner_is_documented() {
    let root = repo_root();
    let md = std::fs::read_to_string(root.join("perf/BUDGETS.md")).expect("perf/BUDGETS.md");
    // The same table `perf/budgets.ts` parses: `| id | metric | budget | unit | how |` under "## The budgets".
    let mut in_table = false;
    let mut budgets: Vec<(String, f64, String)> = Vec::new();
    for line in md.lines() {
        let l = line.trim();
        if let Some(h) = l.strip_prefix("## ") {
            in_table = h.starts_with("The budgets");
            continue;
        }
        if !in_table || !l.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = l.split('|').map(str::trim).collect();
        let cells = &cells[1..cells.len() - 1];
        if cells.len() < 4 || cells[0] == "id" || cells[0].chars().all(|c| c == '-') {
            continue;
        }
        let value: f64 =
            cells[2].parse().unwrap_or_else(|_| panic!("budget of {} is not a number: {:?}", cells[0], cells[2]));
        budgets.push((cells[0].to_string(), value, cells[3].to_string()));
    }
    let ids: Vec<&str> = budgets.iter().map(|(id, _, _)| id.as_str()).collect();
    assert_eq!(
        ids,
        ["fcp", "interactive", "switch", "list", "cls", "optimistic"],
        "the six budgets of spec 128, in order"
    );
    for (id, value, unit) in &budgets {
        assert!(*value >= 0.0);
        assert_eq!(unit, if id == "cls" { "score" } else { "ms" }, "{id}");
    }
    assert!(md.contains("## Budget-raise rule"), "the rule is recorded next to the budgets");
    assert!(md.contains("### Raised budgets"), "raised budgets (if any) are listed");
    // Nothing in the runner repeats a budget number: it loads the table.
    let runner = std::fs::read_to_string(root.join("perf/perf.spec.ts")).unwrap();
    assert!(runner.contains("loadBudgets()"));
    let budgets_ts = std::fs::read_to_string(root.join("perf/budgets.ts")).unwrap();
    assert!(budgets_ts.contains("BUDGETS.md"));
    // `make perf` builds the release binaries and runs the perf config; where it runs is documented
    // (a dedicated Mac, never a GitHub-hosted runner). `phase1_done.rs` checks which workflow has it.
    let makefile = std::fs::read_to_string(root.join("project.mk")).unwrap();
    let perf_target = makefile.split("\nperf:").nth(1).expect("a `perf:` target");
    assert!(perf_target.contains("build --release"), "make perf builds release");
    assert!(perf_target.contains("perf/playwright.config.ts"));
    let readme = std::fs::read_to_string(root.join("perf/README.md")).expect("perf/README.md");
    assert!(readme.contains("make perf"));
    assert!(readme.to_lowercase().contains("github-hosted"));
    assert!(readme.contains("Mac"), "the machine it needs");
}
