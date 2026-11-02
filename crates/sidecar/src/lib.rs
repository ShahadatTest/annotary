//! annotary Sidecar (`<name>.annotary`): the projection format shared by CLI, server and
//! (Phase 2) WASM. `no_std` + `alloc` without the `std` feature; builds for
//! `wasm32-unknown-unknown`.
//!
//! - [`model`] — the typed shape in the decided key order (`format, id, vault, base_version,
//!   members, versions[], assets[], actions, threads[]`).
//! - [`emit`] — the one deterministic writer (leaf-objects-inline JSON, 2-space containers).
//! - [`parse`] — the strict reader (YAML / JSONL / minified rejected with a typed error).
//! - [`Binding`] — what `push` reads (`id`, `vault`, `base_version` + hash, `actions.available`).
//! - [`identity`] — Document ids and Version identity (`sha256` over Markdown hash + manifest).
//! - [`range`] — source coordinates: `"L:C-L:C"` parse/format and quote extraction (1-based
//!   lines, 0-based code-point columns, half-open, CRLF tolerated).
//! - [`anchor`] — the sparse-history rule (`Anchor::placement_at`, `Anchor::record`, `Anchor::at`)
//!   and the Anchor constructors, shared by CLI, server and WASM (ticket 07).
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod anchor;
pub mod binding;
pub mod emit;
pub mod identity;
pub mod model;
pub mod parse;
pub mod range;

pub use binding::Binding;
pub use emit::emit;
pub use model::*;
pub use parse::{ParseError, parse};
pub use range::{Range, RangeError};

/// `format` field value written at the top of every Sidecar.
pub const FORMAT: u32 = 1;

/// File extension of a Sidecar (without the dot).
pub const EXTENSION: &str = "annotary";

/// `<name>.annotary` for a Document Name.
pub fn file_name(name: &str) -> alloc::string::String {
    alloc::format!("{name}.{EXTENSION}")
}

#[cfg(test)]
mod tests {
    use alloc::string::{String, ToString};
    use alloc::vec;
    use alloc::vec::Vec;

    use proptest::prelude::*;

    use super::*;

    const SPARSE_GOLDEN: &str = include_str!("../testdata/sparse-history.phase1-plan.annotary.json");
    const FULL_GOLDEN: &str = include_str!("../testdata/phase1-plan.annotary.json");
    const YAML_GOLDEN: &str = include_str!("../testdata/phase1-plan.annotary.yaml");
    const JSONL_GOLDEN: &str = include_str!("../testdata/phase1-plan.annotary.jsonl");

    fn minimal() -> Sidecar {
        Sidecar {
            format: FORMAT,
            id: "k7q2m9xd".into(),
            vault: "notes".into(),
            base_version: 1,
            members: Members(vec![(
                "rowan".into(),
                MemberEntry { id: "01K2WQ4M8XZ3N6P9RTB0AVYD5E".into(), name: "Rowan Vale".into() },
            )]),
            versions: vec![VersionEntry {
                number: 1,
                name: "plan".into(),
                hash: "a368af8d4bac3bd7e0282515597231425f65ab4e6021f55851b59c8c81e863fc".into(),
                by: "rowan".into(),
                at: "2026-08-14T09:12:04Z".into(),
                message: None,
            }],
            ..Default::default()
        }
    }

    fn top_keys(text: &str) -> Vec<&str> {
        text.lines().filter(|l| l.starts_with("  \"")).map(|l| l.split('"').nth(1).unwrap()).collect()
    }

    // spec: S02-22
    #[test]
    fn prototype_goldens_round_trip_byte_identical() {
        for golden in [SPARSE_GOLDEN, FULL_GOLDEN] {
            let sc = parse(golden).unwrap();
            assert_eq!(emit(&sc), golden, "emit(parse(golden)) must reproduce the prototype file exactly");
            assert_eq!(sc.threads.len(), 8);
            assert_eq!(sc.members.0.len(), 3);
            assert_eq!(sc.members.0[0].0, "rowan", "member order preserved");
            assert!(sc.actions.inherited);
            assert!(sc.assets[1].missing);
            assert_eq!(sc.assets[1].hash, None);
            let t2 = sc.threads[1].anchor.as_ref().unwrap();
            assert_eq!(t2.current.exact, None, "current.exact omitted when equal");
            assert_eq!(sc.threads[5].replies[0].deleted.as_ref().unwrap().by, "scribe");
        }
    }

    // spec: S02-22
    #[test]
    fn minimal_sidecar_layout_and_key_order() {
        let text = emit(&minimal());
        assert_eq!(
            text,
            "{\n  \"format\": 1,\n  \"id\": \"k7q2m9xd\",\n  \"vault\": \"notes\",\n  \"base_version\": 1,\n  \"members\": {\n    \"rowan\": {\"id\": \"01K2WQ4M8XZ3N6P9RTB0AVYD5E\", \"name\": \"Rowan Vale\"}\n  },\n  \"versions\": [\n    {\"number\": 1, \"name\": \"plan\", \"hash\": \"a368af8d4bac3bd7e0282515597231425f65ab4e6021f55851b59c8c81e863fc\", \"by\": \"rowan\", \"at\": \"2026-08-14T09:12:04Z\"}\n  ]\n}\n"
        );
        // Empty sections are omitted, so the file has exactly these top-level keys in this order.
        assert_eq!(top_keys(&text), vec!["format", "id", "vault", "base_version", "members", "versions"]);
        // With every section present the order is the decided one.
        let full = emit(&parse(SPARSE_GOLDEN).unwrap());
        assert_eq!(
            top_keys(&full),
            vec!["format", "id", "vault", "base_version", "members", "versions", "assets", "actions", "threads"]
        );
        assert!(full.ends_with("}\n") && !full.ends_with("}\n\n"));
        assert_eq!(file_name("plan"), "plan.annotary");
    }

    // spec: S02-25
    #[test]
    fn yaml_jsonl_minified_and_junk_are_rejected() {
        assert!(matches!(parse(YAML_GOLDEN), Err(ParseError::NotJson { looks_like: Some("YAML"), .. })));
        assert!(matches!(Binding::from_text(YAML_GOLDEN), Err(ParseError::NotJson { looks_like: Some("YAML"), .. })));
        assert_eq!(parse(JSONL_GOLDEN).unwrap_err(), ParseError::MultipleValues);
        let value: serde_json::Value = serde_json::from_str(SPARSE_GOLDEN).unwrap();
        let minified = serde_json::to_string(&value).unwrap();
        assert!(!minified.contains('\n'));
        assert_eq!(parse(&minified).unwrap_err(), ParseError::Minified);
        assert_eq!(Binding::from_text(&minified).unwrap_err(), ParseError::Minified);
        assert_eq!(parse("").unwrap_err(), ParseError::Empty);
        assert_eq!(parse("[1, 2]").unwrap_err(), ParseError::NotAnObject);
        assert!(matches!(parse("# not a sidecar\n\nhello"), Err(ParseError::NotJson { .. })));
        assert_eq!(parse("{\n  \"id\": \"k7q2m9xd\"\n}"), Err(ParseError::Format { found: None }));
        assert_eq!(parse("{\n  \"format\": 2,\n  \"id\": \"k7q2m9xd\"\n}"), Err(ParseError::Format { found: Some(2) }));
        // Pretty JSON that is not leaf-inline is still one JSON object: accepted, rewritten canonically.
        let pretty = serde_json::to_string_pretty(&value).unwrap();
        let reparsed = parse(&pretty).unwrap();
        assert_eq!(reparsed.threads, parse(SPARSE_GOLDEN).unwrap().threads);
        // (`to_string_pretty` went through an unordered `Value`, so only the members table differs.)
        assert_eq!(top_keys(&emit(&reparsed)), top_keys(SPARSE_GOLDEN));
        for e in [ParseError::Minified, ParseError::MultipleValues, ParseError::Format { found: Some(3) }] {
            assert!(!e.to_string().is_empty());
        }
    }

    // spec: S02-24
    #[test]
    fn binding_reads_only_the_authoritative_fields() {
        let b = Binding::from_text(SPARSE_GOLDEN).unwrap();
        assert_eq!(b.id, "k7q2m9xd");
        assert_eq!(b.vault, "notes");
        assert_eq!(b.base_version, 4);
        assert_eq!(b.base_hash.as_deref(), Some("a368af8d4bac3bd7e0282515597231425f65ab4e6021f55851b59c8c81e863fc"));
        assert_eq!(b.base_name.as_deref(), Some("phase1-plan"));
        assert_eq!(b.base_assets.len(), 4);
        assert_eq!(b.base_assets[1], ("img/cli-screenshot.png".to_string(), None));
        assert_eq!(b.actions_available, None, "inherited: true ⇒ nothing to declare");

        // Hand edits to projection fields change nothing that push reads.
        let edited = SPARSE_GOLDEN
            .replace("\"body\": \"Needs a **Risks** section", "\"body\": \"HAND EDITED")
            .replace("\"name\": \"Rowan Vale\"", "\"name\": \"Somebody Else\"")
            .replace("\"resolved\": {\"by\": \"scribe\"", "\"resolved\": {\"by\": \"nobody\"")
            .replace("\"threads\": [", "\"extra_key\": {\"a\": [1, 2]},\n  \"threads\": [");
        assert_ne!(edited, SPARSE_GOLDEN);
        assert_eq!(Binding::from_text(&edited).unwrap(), b);
        // The typed reader also tolerates unknown keys.
        assert!(parse(&edited).is_ok());

        // Dropping `inherited` makes `available` a per-Document declaration; `[]` declares none.
        let declared = SPARSE_GOLDEN.replace("    \"inherited\": true,\n", "");
        let b2 = Binding::from_text(&declared).unwrap();
        let ids: Vec<&str> = b2.actions_available.as_ref().unwrap().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["approve", "send-back"]);
        let none = declared.replace(
            "\"available\": [\n      {\"id\": \"approve\", \"label\": \"Approve\", \"intent\": \"positive\"},\n      {\"id\": \"send-back\", \"label\": \"Send back\", \"intent\": \"neutral\"}\n    ]",
            "\"available\": []",
        );
        assert_eq!(Binding::from_text(&none).unwrap().actions_available, Some(vec![]));

        // Garbage in the read fields is a typed error.
        assert!(matches!(
            Binding::from_text(&SPARSE_GOLDEN.replace("\"id\": \"k7q2m9xd\"", "\"id\": \"nope\"")),
            Err(ParseError::Invalid(_))
        ));
        assert!(matches!(
            Binding::from_text(&SPARSE_GOLDEN.replace("\"base_version\": 4", "\"base_version\": \"four\"")),
            Err(ParseError::Invalid(_))
        ));
        // Local-edit check: identity of Markdown + recorded manifest vs the recorded hash.
        let md_hash = identity::sha256_hex(b"whatever");
        let expect = identity::version_hash(&md_hash, b.base_assets.iter().map(|(r, h)| (r.as_str(), h.as_deref())));
        assert_eq!(b.local_identity(&md_hash), expect);
        assert_eq!(b.matches_base(&md_hash), Some(false));
        let ours = emit(&minimal());
        let bm = Binding::from_text(&ours).unwrap();
        assert_eq!(bm.base_hash.as_deref(), Some("a368af8d4bac3bd7e0282515597231425f65ab4e6021f55851b59c8c81e863fc"));
    }

    /// Wrap the line `from` of `text` in git conflict markers, `to` on the other side (`diff3`
    /// adds a `|||||||` base section holding `from` again).
    fn conflict(text: &str, from: &str, to: &str, diff3: bool) -> String {
        assert!(text.contains(from), "{from}");
        let base = if diff3 { format!("||||||| merged common ancestors\n{from}\n") } else { String::new() };
        text.replacen(from, &format!("<<<<<<< HEAD\n{from}\n{base}=======\n{to}\n>>>>>>> feature"), 1)
    }

    // spec: S17-07
    #[test]
    fn a_conflicted_sidecar_reads_both_sides() {
        let clean = Binding::from_text(SPARSE_GOLDEN).unwrap();
        assert!(!clean.conflicted);
        // base_version 4 vs 2 — two-way and diff3 style: the lower one wins, its hash too.
        for diff3 in [false, true] {
            let text = conflict(SPARSE_GOLDEN, "  \"base_version\": 4,", "  \"base_version\": 2,", diff3);
            let b = Binding::from_text(&text).unwrap();
            assert!(b.conflicted && !b.actions_disagree);
            assert_eq!((b.id.as_str(), b.vault.as_str(), b.base_version), ("k7q2m9xd", "notes", 2));
            assert_eq!(
                b.base_hash.as_deref(),
                Some("8d53e39d55aac410b73ed0bc07c0ca33fdc967e1deb1544dec0c6ce504949f5a")
            );
            // Either side may hold the lower one.
            let flipped = conflict(SPARSE_GOLDEN, "  \"base_version\": 4,", "  \"base_version\": 9,", diff3);
            assert_eq!(Binding::from_text(&flipped).unwrap().base_version, 4);
            // The typed reader still refuses it (only push / pull read a conflicted Sidecar).
            assert!(parse(&text).is_err());
        }
        // Markers in the Threads only: nothing push reads differs.
        let threads = conflict(SPARSE_GOLDEN, "      \"id\": \"t1\",", "      \"id\": \"t1\", \"x\": 1,", false);
        let b = Binding::from_text(&threads).unwrap();
        assert!(b.conflicted);
        assert_eq!(Binding { conflicted: false, ..b }, clean);
        // Different Document ids: one side has to be kept.
        let ids = conflict(SPARSE_GOLDEN, "  \"id\": \"k7q2m9xd\",", "  \"id\": \"p3x8w2ka\",", false);
        assert_eq!(Binding::from_text(&ids).unwrap_err(), ParseError::Conflict { what: "Document ids" });
        assert!(ParseError::Conflict { what: "Document ids" }.to_string().contains("different Document ids"));
        // Action declarations: used when both sides agree, dropped (and flagged) when not.
        let declared = SPARSE_GOLDEN.replace("    \"inherited\": true,\n", "");
        let same = conflict(&declared, "  \"base_version\": 4,", "  \"base_version\": 3,", false);
        let b = Binding::from_text(&same).unwrap();
        assert_eq!(b.actions_available.as_ref().map(Vec::len), Some(2));
        assert!(!b.actions_disagree);
        let differ = conflict(
            &declared,
            "      {\"id\": \"send-back\", \"label\": \"Send back\", \"intent\": \"neutral\"}",
            "      {\"id\": \"reject\", \"label\": \"Reject\", \"intent\": \"negative\"}",
            false,
        );
        let b = Binding::from_text(&differ).unwrap();
        assert!(b.conflicted && b.actions_disagree);
        assert_eq!(b.actions_available, None);
        // Markers out of order, or a side that is not a Sidecar: today's errors.
        let open = SPARSE_GOLDEN.replacen("  \"vault\"", "<<<<<<< HEAD\n  \"vault\"", 1);
        assert!(matches!(Binding::from_text(&open), Err(ParseError::Invalid(_))));
        let broken = conflict(SPARSE_GOLDEN, "  \"base_version\": 4,", "  \"base_version\": four,", false);
        assert!(matches!(Binding::from_text(&broken), Err(ParseError::NotJson { .. })));
    }

    // spec: S26-04
    #[test]
    fn a_conflict_over_the_vault_name_alone_is_a_vault_rename_not_two_documents() {
        // Same id, different `vault` (one branch pulled before the rename, one after): resolved
        // without a human — the newer side's Vault name, the lower side's base for the check.
        for diff3 in [false, true] {
            let renamed = conflict(SPARSE_GOLDEN, "  \"vault\": \"notes\",", "  \"vault\": \"work\",", diff3);
            let b = Binding::from_text(&renamed).unwrap();
            assert!(b.conflicted);
            assert_eq!((b.id.as_str(), b.base_version), ("k7q2m9xd", 4));
            assert_eq!(b.vault, "notes", "a tie keeps our side's name");
            assert_eq!(b.base_hash, Binding::from_text(SPARSE_GOLDEN).unwrap().base_hash);
        }
        // Both differ, theirs older: their base (2) and its hash, our newer name.
        let both = conflict(
            SPARSE_GOLDEN,
            "  \"vault\": \"notes\",\n  \"base_version\": 4,",
            "  \"vault\": \"drafts\",\n  \"base_version\": 2,",
            false,
        );
        let b = Binding::from_text(&both).unwrap();
        assert_eq!((b.vault.as_str(), b.base_version), ("notes", 2));
        assert_eq!(b.base_hash.as_deref(), Some("8d53e39d55aac410b73ed0bc07c0ca33fdc967e1deb1544dec0c6ce504949f5a"));
        // Theirs newer: their name, our lower base.
        let both = conflict(
            SPARSE_GOLDEN,
            "  \"vault\": \"notes\",\n  \"base_version\": 4,",
            "  \"vault\": \"Überblick\",\n  \"base_version\": 9,",
            true,
        );
        let b = Binding::from_text(&both).unwrap();
        assert_eq!((b.vault.as_str(), b.base_version), ("Überblick", 4));
    }

    // ---- proptest: projection → emit → parse → emit is byte-identical, for arbitrary content

    fn arb_text() -> impl Strategy<Value = String> {
        // Any Unicode text incl. quotes, backslashes, control chars and emoji.
        proptest::string::string_regex("[\\p{L}\\p{N} \"\\\\/\\n\\t\\-—:.@#*_`{}\\[\\]😀]{0,40}").unwrap()
    }

    fn arb_handle() -> impl Strategy<Value = String> {
        proptest::string::string_regex("[a-z][a-z0-9]{0,6}(-[a-z0-9]{1,4}){0,2}").unwrap()
    }

    fn arb_stamp() -> impl Strategy<Value = Stamp> {
        (1u32..50, 0u32..1_000_000).prop_map(|(v, s)| Stamp {
            at: alloc::format!("2026-08-{:02}T{:02}:{:02}:{:02}Z", 1 + s % 28, s % 24, s % 60, s % 59),
            version: v,
        })
    }

    fn arb_placement(span: bool, with_status: bool) -> impl Strategy<Value = Placement> {
        (1u32..50, 1u32..500, 0u32..80, 0u32..80, arb_text(), any::<bool>(), 0usize..3).prop_map(
            move |(v, l, c1, c2, exact, has_exact, st)| {
                let status = ["anchored", "refined", "orphaned"][st].to_string();
                if span {
                    Placement {
                        version: v,
                        range: Some(alloc::format!("{l}:{c1}-{}:{c2}", l + (c2 % 3))),
                        status: with_status.then_some(status),
                        exact: (has_exact || !with_status).then_some(exact),
                        ..Default::default()
                    }
                } else {
                    Placement {
                        version: v,
                        hash: (!with_status).then(|| identity::sha256_hex(exact.as_bytes())),
                        reference: (!with_status).then(|| alloc::format!("img/{c1}.png")),
                        line: Some(l),
                        status: with_status.then_some(status),
                        ..Default::default()
                    }
                }
            },
        )
    }

    fn arb_reply() -> impl Strategy<Value = Reply> {
        (
            arb_handle(),
            arb_stamp(),
            arb_text(),
            proptest::collection::vec(arb_handle(), 0..3),
            proptest::option::of(arb_stamp()),
            any::<bool>(),
        )
            .prop_map(|(author, created, body, mentions, edited, deleted)| Reply {
                id: "t1.1".into(),
                deleted: deleted.then(|| Tombstone { at: created.at.clone(), by: author.clone() }),
                author,
                edited_at: edited.map(|s| s.at),
                created,
                body,
                mentions,
            })
    }

    fn arb_thread() -> impl Strategy<Value = Thread> {
        (
            0usize..3,
            arb_placement(true, false),
            arb_placement(true, true),
            proptest::collection::vec(arb_placement(true, true), 0..4),
            arb_placement(false, false),
            arb_placement(false, true),
            arb_handle(),
            arb_stamp(),
            arb_text(),
            proptest::collection::vec(arb_handle(), 0..3),
            // resolved; ticket 16: the root's own `edited_at` / `deleted` (a tombstoned root keeps the Thread).
            (proptest::option::of(arb_stamp()), proptest::option::of(arb_stamp()), any::<bool>()),
            proptest::collection::vec(arb_reply(), 0..3),
        )
            .prop_map(
                |(
                    subject,
                    so,
                    sc,
                    hist,
                    ao,
                    ac,
                    author,
                    created,
                    body,
                    mentions,
                    (resolved, edited, deleted),
                    replies,
                )| {
                    let subject_s = ["document", "span", "asset"][subject];
                    let anchor = match subject {
                        1 => Some(Anchor { original: so, current: sc, history: hist }),
                        2 => Some(Anchor { original: ao, current: ac.clone(), history: vec![ac] }),
                        _ => None,
                    };
                    Thread {
                        id: "t1".into(),
                        subject: subject_s.into(),
                        anchor,
                        resolved: resolved.map(|s| Resolution { by: author.clone(), at: s.at, version: s.version }),
                        deleted: deleted.then(|| Tombstone { at: created.at.clone(), by: author.clone() }),
                        author,
                        created,
                        body,
                        mentions,
                        edited_at: edited.map(|s| s.at),
                        replies,
                    }
                },
            )
    }

    fn arb_sidecar() -> impl Strategy<Value = Sidecar> {
        (
            proptest::collection::vec((arb_handle(), arb_text()), 1..4),
            proptest::collection::vec((arb_text(), arb_handle(), arb_stamp(), proptest::option::of(arb_text())), 1..5),
            proptest::collection::vec(
                (arb_text(), any::<bool>(), 0u64..100_000, proptest::option::of(arb_text())),
                0..4,
            ),
            proptest::collection::vec((arb_handle(), arb_text()), 0..3),
            any::<bool>(),
            // ticket 16: the Action log `[{id, by, at, version, label, intent}]`.
            proptest::collection::vec((arb_handle(), arb_handle(), arb_stamp(), arb_text(), 0usize..3), 0..4),
            proptest::collection::vec(arb_thread(), 0..4),
        )
            .prop_map(|(members, versions, assets, decls, inherited, log, threads)| {
                let mut ms = Members::default();
                for (h, n) in members {
                    ms.insert(h, MemberEntry { id: "01K2WQ4M8XZ3N6P9RTB0AVYD5E".into(), name: n });
                }
                let versions: Vec<VersionEntry> = versions
                    .into_iter()
                    .enumerate()
                    .map(|(i, (name, by, at, message))| VersionEntry {
                        number: i as u32 + 1,
                        hash: identity::sha256_hex(name.as_bytes()),
                        name,
                        by,
                        at: at.at,
                        message,
                    })
                    .collect();
                let base_version = versions.len() as u32;
                Sidecar {
                    format: FORMAT,
                    id: "k7q2m9xd".into(),
                    vault: "notes".into(),
                    base_version,
                    members: ms,
                    versions,
                    assets: assets
                        .into_iter()
                        .map(|(r, missing, size, via)| AssetEntry {
                            reference: r.clone(),
                            hash: (!missing).then(|| identity::sha256_hex(r.as_bytes())),
                            size: (!missing).then_some(size),
                            content_type: "image/png".into(),
                            missing,
                            via,
                        })
                        .collect(),
                    actions: Actions {
                        available: decls
                            .into_iter()
                            .map(|(id, label)| ActionDecl { id, label, intent: "neutral".into() })
                            .collect(),
                        inherited,
                        log: log
                            .into_iter()
                            .map(|(id, by, at, label, intent)| ActionEvent {
                                id,
                                by,
                                version: at.version,
                                at: at.at,
                                label,
                                intent: ["positive", "neutral", "negative"][intent].into(),
                            })
                            .collect(),
                    },
                    threads,
                }
            })
    }

    proptest! {
        // spec: S02-22, S02-23, S16-02
        #[test]
        fn projection_emit_parse_emit_is_byte_identical(sc in arb_sidecar()) {
            let once = emit(&sc);
            let back = parse(&once).unwrap();
            prop_assert_eq!(&back, &sc);
            let twice = emit(&back);
            prop_assert_eq!(&twice, &once);
            // Every top-level key present appears in the decided order.
            let order = ["format", "id", "vault", "base_version", "members", "versions", "assets", "actions", "threads"];
            let mut last = 0;
            for k in top_keys(&once) {
                let pos = order.iter().position(|o| *o == k).expect("known key");
                prop_assert!(pos >= last, "key order");
                last = pos;
            }
            // The Binding agrees with the projection.
            let b = Binding::from_text(&once).unwrap();
            prop_assert_eq!(b.base_version, sc.base_version);
            prop_assert_eq!(b.base_hash.as_deref(), Some(sc.versions.last().unwrap().hash.as_str()));
        }

        // spec: S17-07
        #[test]
        fn a_sidecar_in_conflict_markers_reads_the_lower_base(
            sc in arb_sidecar(),
            lower in 1u32..5,
            drop_threads in 0usize..3,
            widen in (0usize..8, 0usize..8),
            diff3 in any::<bool>(),
        ) {
            // "Theirs" = the same Document on another branch: an older base, fewer Threads.
            let mut other = sc.clone();
            other.base_version = lower.min(sc.base_version);
            let keep = other.threads.len().saturating_sub(drop_threads);
            other.threads.truncate(keep);
            let (a, b) = (emit(&sc), emit(&other));
            let (al, bl): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
            let pre = al.iter().zip(&bl).take_while(|(x, y)| x == y).count();
            let suf = al.iter().rev().zip(bl.iter().rev()).take_while(|(x, y)| x == y).count().min(al.len().min(bl.len()) - pre);
            // The conflict around the differing lines, widened by up to 8 lines each way (any section).
            let start = pre.saturating_sub(widen.0);
            let suf = suf.saturating_sub(widen.1);
            let mut text: String = al[..start].iter().map(|l| format!("{l}\n")).collect();
            text.push_str("<<<<<<< HEAD\n");
            for l in &al[start..al.len() - suf] { text.push_str(l); text.push('\n'); }
            if diff3 {
                text.push_str("||||||| base\n{ not even json\n");
            }
            text.push_str("=======\n");
            for l in &bl[start..bl.len() - suf] { text.push_str(l); text.push('\n'); }
            text.push_str(">>>>>>> other\n");
            for l in &al[al.len() - suf..] { text.push_str(l); text.push('\n'); }
            let got = Binding::from_text(&text).unwrap();
            prop_assert!(got.conflicted);
            prop_assert_eq!(got.base_version, other.base_version);
            prop_assert_eq!(&got.id, &sc.id);
        }
    }
}
