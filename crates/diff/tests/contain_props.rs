//! Property tests of the containment check (ticket 17), for a random base and two disjoint sets
//! of line edits A and B:
//! - a stale file (the base itself) never contains a change, on every input;
//! - when A's and B's edits are separated by at least one unchanged line that occurs once in the
//!   base: the file with only B's edits never contains A's changes (**soundness** — a concurrent
//!   edit is never silently dropped), and the file with both does (**completeness**). Without
//!   such a line the two readings can both be right (B replacing one of three blank lines where
//!   A deleted one *is* A's change plus an insertion), and the check may refuse — the safe
//!   direction, recorded in ADR 0002;
//! - determinism: same inputs, same answer.

use annotary_diff::{Containment, contains_changes};
use proptest::prelude::*;

/// What one side does to one base line.
#[derive(Clone, Debug)]
enum Op {
    Keep,
    Replace(String),
    Delete,
    InsertBefore(String),
}

fn line(prefix: &'static str) -> impl Strategy<Value = String> {
    // Words of the edits never occur in the base (distinct prefixes), so an edit cannot
    // accidentally reproduce base text.
    prop::collection::vec(0u8..6, 1..5)
        .prop_map(move |ws| ws.iter().map(|w| format!("{prefix}{w}")).collect::<Vec<_>>().join(" "))
}

fn base_line() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => prop::collection::vec(0u8..8, 1..6)
            .prop_map(|ws| ws.iter().map(|w| format!("w{w}")).collect::<Vec<_>>().join(" ")),
        1 => Just(String::new()),
        1 => (1u8..3).prop_map(|n| format!("{} Heading", "#".repeat(n as usize))),
    ]
}

fn op(prefix: &'static str) -> impl Strategy<Value = Op> {
    prop_oneof![line(prefix).prop_map(Op::Replace), Just(Op::Delete), line(prefix).prop_map(Op::InsertBefore),]
}

/// Base lines, then per line who edits it (0 = nobody, 1 = A, 2 = B) and how.
fn scenario() -> impl Strategy<Value = (Vec<String>, Vec<(Op, Op)>, Vec<u8>)> {
    prop::collection::vec(base_line(), 1..25).prop_flat_map(|base| {
        let n = base.len();
        (
            Just(base),
            prop::collection::vec((op("a"), op("b")), n),
            prop::collection::vec(prop_oneof![4 => Just(0u8), 1 => Just(1u8), 1 => Just(2u8)], n),
        )
    })
}

fn apply(base: &[String], ops: &[(Op, Op)], who: &[u8], sides: &[u8]) -> String {
    let mut out = Vec::new();
    for (i, l) in base.iter().enumerate() {
        let op = if sides.contains(&who[i]) { if who[i] == 1 { &ops[i].0 } else { &ops[i].1 } } else { &Op::Keep };
        match op {
            Op::Keep => out.push(l.clone()),
            Op::Replace(r) => out.push(r.clone()),
            Op::Delete => {}
            Op::InsertBefore(r) => {
                out.push(r.clone());
                out.push(l.clone());
            }
        }
    }
    out.iter().map(|l| format!("{l}\n")).collect()
}

/// Between any line A edits and any line B edits lies an unchanged line that occurs once in the
/// base and is not blank (an anchor both diffs must agree on).
fn anchored(base: &[String], who: &[u8]) -> bool {
    let unique = |k: usize| !base[k].is_empty() && base.iter().filter(|l| **l == base[k]).count() == 1;
    let edited: Vec<usize> = (0..who.len()).filter(|&i| who[i] != 0).collect();
    edited.windows(2).all(|w| who[w[0]] == who[w[1]] || (w[0] + 1..w[1]).any(|k| who[k] == 0 && unique(k)))
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 300, ..ProptestConfig::default() })]

    // spec: S17-01, S17-05
    #[test]
    fn a_file_without_the_heads_changes_never_contains_them((base, ops, who) in scenario()) {
        let b: String = base.iter().map(|l| format!("{l}\n")).collect();
        let a = apply(&base, &ops, &who, &[1]);
        let only_b = apply(&base, &ops, &who, &[2]);
        if a != b {
            prop_assert!(!contains_changes(&b, &a, &b).is_contained(), "a stale file never contains A");
            if anchored(&base, &who) {
                prop_assert!(
                    !contains_changes(&b, &a, &only_b).is_contained(),
                    "B alone should miss A\nbase:\n{b}\nA:\n{a}\nB:\n{only_b}"
                );
            }
        }
        prop_assert!(matches!(contains_changes(&b, &a, &a), Containment::Contained));
        prop_assert_eq!(contains_changes(&b, &a, &only_b), contains_changes(&b, &a, &only_b));
    }

    // spec: S17-01
    #[test]
    fn a_merge_contains_the_heads_changes((base, ops, who) in scenario()) {
        prop_assume!(anchored(&base, &who));
        let b: String = base.iter().map(|l| format!("{l}\n")).collect();
        let a = apply(&base, &ops, &who, &[1]);
        let ab = apply(&base, &ops, &who, &[1, 2]);
        let got = contains_changes(&b, &a, &ab);
        prop_assert!(got.is_contained(), "A∪B should contain A\nbase:\n{b}\nA:\n{a}\nA∪B:\n{ab}\n{got:?}");
    }
}
