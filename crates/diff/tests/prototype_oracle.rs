//! The sidecar prototype's scenario as an oracle: its `build.py` computed every span Thread's
//! placement on v1..v4 by hand (`pos()` = find the expected quote on the expected line) and wrote
//! `sparse-history.phase1-plan.annotary.json`. Replaying v1 → v2 → v3 → v4 through
//! `annotary_diff::reanchor` and recording with the sidecar crate's sparse rule must reproduce
//! those anchors byte for byte (`current` and `history`), and the sparse history must answer
//! `placement_at(N)` exactly like the full (dense) history for every N — the oracle the
//! prototype's `sparse()` function implied.

use std::path::Path;

use annotary_diff::{Range, Span, Status, range::extract, reanchor};
use annotary_sidecar::anchor::{ANCHORED, ORPHANED, REFINED};
use annotary_sidecar::{Anchor, Placement, parse};

fn proto(file: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/prototype-sidecar").join(file);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

/// A golden the sidecar crate keeps under its own name (`crates/sidecar/testdata/`).
fn sidecar_golden(file: &str) -> String {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../sidecar/testdata").join(file);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

// spec: S07-01, S07-08, S07-11
#[test]
fn prototype_scenario_reproduces_the_sparse_golden_and_full_history_agrees() {
    let versions = [proto("versions/v1.md"), proto("versions/v2.md"), proto("versions/v3.md"), proto("versions/v1.md")];
    let golden = parse(&sidecar_golden("sparse-history.phase1-plan.annotary.json")).unwrap();
    let mut checked = 0;
    for t in golden.threads.iter().filter(|t| t.subject == "span") {
        let g = t.anchor.as_ref().unwrap();
        let created = g.original.version;
        let range = Range::parse(g.original.range.as_deref().unwrap()).unwrap();
        let original = extract(&versions[created as usize - 1], &range).unwrap();
        assert_eq!(Some(original.as_str()), g.original.exact.as_deref(), "{}: the frozen quote", t.id);
        let mut anchor = Anchor::new_span(created, &range.to_string(), &original);
        // Full history: one placement per Version.
        let mut full: Vec<Placement> = vec![anchor.history[0].clone()];
        let mut span = Span { range, original: original.clone(), orphaned: false };
        for n in created + 1..=4 {
            let p = reanchor(&versions[n as usize - 2], &versions[n as usize - 1], std::slice::from_ref(&span))
                .unwrap()
                .remove(0);
            let (status, range_s) = match p.status {
                Status::Orphaned => {
                    span.orphaned = true;
                    (ORPHANED, None)
                }
                Status::Anchored => {
                    span.range = p.range.unwrap();
                    (ANCHORED, Some(span.range.to_string()))
                }
                Status::Refined => {
                    span.range = p.range.unwrap();
                    (REFINED, Some(span.range.to_string()))
                }
            };
            anchor.record(n, status, range_s.clone(), None, p.exact.clone());
            full.push(Placement { version: n, range: range_s, status: Some(status.to_string()), ..Default::default() });
        }
        // The stored shape equals the prototype's golden exactly.
        let mut want = g.clone();
        // The golden's `current` omits `exact` when it equals the original; ours carries `exact`
        // only when refined — the same rule; t3 ends anchored, so nothing to strip.
        want.current.exact = want.current.exact.take().filter(|e| Some(e) != g.original.exact.as_ref());
        assert_eq!(anchor, want, "{}: sparse anchor", t.id);
        // Sparse ⇔ full: placement_at(N) is the dense entry for every N (stamped N, or the last
        // valid Version once orphaned).
        for (i, dense) in full.iter().enumerate() {
            let n = created + i as u32;
            let got = anchor.placement_at(n).unwrap();
            if dense.status.as_deref() == Some(ORPHANED) {
                assert_eq!(got.status.as_deref(), Some(ORPHANED), "{} v{n}", t.id);
                let last_valid = full[..i].iter().rev().find(|d| d.status.as_deref() != Some(ORPHANED)).unwrap();
                assert_eq!(got.range, last_valid.range, "{} v{n}: last valid range", t.id);
                assert_eq!(got.version, last_valid.version, "{} v{n}: last valid Version", t.id);
            } else {
                assert_eq!(
                    (got.version, got.range.as_deref(), got.status.as_deref()),
                    (n, dense.range.as_deref(), dense.status.as_deref()),
                    "{} v{n}",
                    t.id
                );
            }
        }
        checked += 1;
    }
    assert_eq!(checked, 6, "t2, t3, t4, t5, t7, t8");
    // And the whole file round-trips through the writer unchanged (the sidecar crate's job, but
    // it proves the anchors above are what the golden literally holds).
    assert_eq!(annotary_sidecar::emit(&golden), sidecar_golden("sparse-history.phase1-plan.annotary.json"));
}
