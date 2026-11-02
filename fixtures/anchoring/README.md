# Re-anchoring fixtures (ticket 07)

One directory per row of the empirical fixture table
([`TABLE.md`](TABLE.md): Google Docs D1–D12, GitHub G0–G14)
plus the ticket-05 additions. Each holds `v1.md, v2.md, …` (the Versions), optional
`vN.manifest.json` (`[{ref, hash}]`, for Asset Threads), `threads.json` (the Threads as created:
`{id, subject, version, range | ref}`) and `expected.json` (the *full* placement history per
Thread, one entry per Version from creation: `{version, status, range?, exact?, line?}`).
`make.py` regenerates everything from declared expectations (which text survives on which line, or
`orphaned`) — the crate never computes them. `crates/diff/tests/fixtures.rs` replays every
directory through `annotary_diff::reanchor` and fails when a research row lacks a directory;
`crates/annotary/tests/reanchor_api.rs` pushes the same Versions through the server and checks the
CLI's placements against the server's (Asset lines included).

| id | edit | expected | note |
|---|---|---|---|
| D1 | delete all of S | orphaned; v3 = v1 stays orphaned | Docs' undo revives (operation replay) — we never revive |
| D2 / D3 / D4 | delete start / middle / end of S | refined, shrinks | as Docs |
| D5 | insert inside S | refined, grows | as Docs |
| D6 / D7 | insert immediately before / after S | anchored, shifted | boundary insertions fall outside |
| D8 | type over S | orphaned; v3 = v1 stays orphaned | as Docs; no undo revival |
| D9a | copy S, paste elsewhere | anchored (single range) | Docs makes a two-range comment — we anchor once |
| D9b | …then delete the original | orphaned | a move is delete + insert; no move detection |
| D10 | find & replace overlapping S (`cc dd ee` → `cc RR ee`) | refined `bb cc` | Docs grows over the whole replacement (`bb cc RR ee`) — we keep what survived |
| D10-prime | the replace swallows S (`cc dd` → `RR`) | orphaned (t1); `bb cc dd` ⇒ refined `bb` (t2) | **deliberate divergence**: Docs keeps and extends; type-over and replace are indistinguishable to a before/after diff — quote gone ⇒ orphaned |
| D11 | delete the paragraph containing S | orphaned; v3 = v1 stays orphaned | as Docs; no undo revival |
| D12 | reload (v3 = v2) | placements persist unchanged | nothing is recomputed on read |
| G0 | baseline | all anchored | |
| G1 | insert 2 lines at top | all shift +2, anchored | as GitHub |
| G2 | edit inside A's range | refined | GitHub: outdated |
| G3 | edit the line above B | anchored, unchanged | as GitHub |
| G4 | delete C's whole range | orphaned | as GitHub |
| G5 | delete first of D's two lines | refined, shrinks | GitHub: outdated |
| G6 / G14 | insert a line between F's / L's lines | refined, grows to 3 lines | GitHub also grows |
| G7 | edit G's first line | refined | GitHub: outdated |
| G8 | rewrite G's lines, then revert byte-for-byte | orphaned, stays orphaned | as GitHub: never revived |
| G9 | a later unrelated push (append at EOF) | orphaned stays, others anchored | GitHub's force-push remap is a quirk; incremental mapping from the last valid Version |
| G10 | full rewrite, same line count | all orphaned | as GitHub |
| G11 | cut lines 11–12, paste at EOF | I orphaned, D shifts | as GitHub: a moved block is delete + add |
| G12 | trailing whitespace on J's line | anchored | **deliberate divergence**: GitHub outdates on any byte |
| G13 | rename (same text) | anchored | as GitHub |
| whitespace-only | spaces changed on the span's line | anchored, shifted | |
| type-over | S retyped with other words | orphaned | |
| replace-swallows-span | replace covering all of S | orphaned | D10' |
| overlapping-replace | replace overlapping S | refined | D10 |
| quote-restored | S deleted, later restored (twice) | orphaned throughout | sticky |
| moved-block | S's paragraph moved to the end | orphaned | no move detection |
| whitespace-survivor | only `, ` survives under S | orphaned | punctuation/whitespace/digits are not evidence |
| multiline-inner-edit | 3-line span, middle line reworded | refined, same lines | |
| boundary-insertion | lines above/below and text at both ends of a 3-line span | anchored, shifted | |
| copy-paste | D9a again | anchored | |
| asset-hash-moved | the image reference moves to another line | anchored, line updated | by content hash |
| asset-hash-replaced | the file's bytes change, later change back | orphaned, stays orphaned | sticky |
| document-level | full rewrite under a Document-level Thread | anchored throughout | no anchor, never orphaned |
| determinism | six Threads over three mixed edits | see expected.json | the checker also replays every fixture twice and in reverse order |
