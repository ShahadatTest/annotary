# Anchoring behavior fixture table — Google Docs & GitHub PR review (empirical)

Recorded 2026-08-17 by driving the real products (Google Docs via the human's browser session; GitHub via REST + GraphQL API on a private scratch repo, then verified in the web UI). Every row below was actually observed, not inferred. This table is the seed of the re-anchoring test suite (ticket 15) and the evidence base for the anchor-model decision (ticket 05).

Scratch artifacts were cleaned up after recording (the GitHub repo deleted; Google Doc trashed).

## Vocabulary used below

- **S** — the commented span (Docs) or line range (GitHub) at comment time.
- **survives** — comment still shows at a location in the document body.
- **orphaned** (Docs) — Docs' "Original content deleted" state: no body highlight; the comment remains in the *All comments* panel, still open, still replyable/resolvable.
- **outdated** (GitHub) — `isOutdated=true`, `line=null`; thread hidden from *Files changed*, shown in *Conversation* with an **Outdated** badge and the *original* hunk snippet frozen; still replyable/resolvable.

---

## Part A — Google Docs: span comment under edits

Setup: 11 paragraphs `Xxx: aa bb cc dd ee ff gg.`; one comment on **S = `bb cc dd`** in each. Docs panel header shows the *quoted text* of the comment; the body highlight shows the *live anchor*.

| # | Edit | Survives? | Live anchor after edit | Panel quote header | Undo restores? |
|---|---|---|---|---|---|
| D1 | Delete all of S | **orphaned** | none — no highlight; removed from margin | "Original content deleted" | yes, fully (highlight + quote back) |
| D2 | Delete start of S (`bb `) | yes | shrinks to `cc dd` | still `bb cc dd` (original quote frozen) | yes |
| D3 | Delete middle of S (`cc`) | yes | shrinks to `bb dd` (one contiguous range) | still `bb cc dd` | yes |
| D4 | Delete end of S (` dd`) | yes | shrinks to `bb cc` | still `bb cc dd` | yes |
| D5 | Insert text inside S (`XX` between cc and dd) | yes | **grows** to `bb cc XX dd` | still `bb cc dd` | yes |
| D6 | Insert immediately *before* S (`ZZ ` before `bb`) | yes | unchanged `bb cc dd`, shifted right; inserted text **not** part of S | `bb cc dd` | yes |
| D7 | Insert immediately *after* S (` YY` after `dd`) | yes | unchanged `bb cc dd`; inserted text **not** part of S | `bb cc dd` | yes |
| D8 | Select all of S and type over (`QQ QQ QQ`) | **orphaned** | none — typed replacement is *not* adopted | "Original content deleted" | yes — undo revives the anchor |
| D9a | Copy S, paste elsewhere in same doc (observed because the cut behaved as copy) | yes | **one comment, two ranges** — original *and* pasted copy both highlighted; clicking either selects the same comment; count unchanged | `bb cc dd` | yes |
| D9b | …then delete the original S (= net effect of cut & paste) | yes | anchor lives on the pasted copy only | `bb cc dd` | yes |
| D10 | Find & replace `cc dd ee` → `cc RR ee` (Replace all) — match *overlaps* S (S=`bb cc dd`) | yes | **grows to cover the whole replacement**: `bb cc RR ee` | `bb cc dd` | yes |
| D10' | same replace where the match *fully contains* the (already shrunk) S=`cc dd` | yes (!) | `cc RR ee` — unlike D8, replace-in-place keeps the anchor and extends it | `bb cc dd` | yes |
| D11 | Delete the whole paragraph containing S | **orphaned** | none | "Original content deleted" | yes — paragraph and anchor return |
| D12 | Reload the document after all of the above | — | all live anchors persisted server-side exactly as shown | headers unchanged: **original quote is frozen forever**; only the range moves | — |

### Docs — behavioral summary (what to copy / what to diverge from)

1. **Two things are stored, not one:** the *original quote* (frozen, used as the label) and the *live range* (a character range that shrinks/grows/moves). Docs never re-derives the range from the quote — it's carried by the editor's OT/CRDT position transforms.
2. **Range semantics are "sticky-inside, non-sticky at the edges":** insertions strictly inside the range extend it; insertions at either boundary fall outside it. Partial deletion shrinks it. Full deletion (or type-over, which is delete+insert) orphans it.
3. **Orphan ≠ delete:** orphaned comments stay open, replyable, resolvable — just not placed. They are visible only in the *All comments* panel, not the margin.
4. **Undo revives** an orphan — because Docs replays operations. Annotary cannot do this (content is uploaded, not edited in place); an equivalent would be "re-anchor if a later Version restores the quote," which we should decide on explicitly (GitHub does *not* — see G7/G8).
5. **Copy/paste duplicates the anchor** into a multi-range comment. Not something to emulate; note as a divergence: Annotary anchors are single-range.
6. **Find & replace is anchor-preserving even when it swallows the span** (D10') — an artifact of Docs implementing replace as an in-place mutation. Since Annotary only sees before/after text, D8 and D10' are indistinguishable to us; we must pick one behavior and document the divergence (recommend: outdated/orphaned — quote gone ⇒ not anchored).

---

## Part B — GitHub PR review: line comments across pushes

Setup: PR adds `doc.md` (30 lines, all in the diff). Comments A–H created on commit `fa6a429` (REST `line`/`start_line`, side RIGHT). One push per scenario; snapshot of `line / original_line / start_line / original_start_line / commit_id / isOutdated` after each. Raw snapshots in `log.md`.

| # | Push | Effect on the targeted comment | `line` | `original_line` | `commit_id` | outdated |
|---|---|---|---|---|---|---|
| G0 | baseline | A 5–6, B 9–10, C 13–14, D 17–18, E 21, F 24–25, G 28–29, H 2 | = orig | = orig | fa6a429 | false |
| G1 | Insert 2 lines at top of file (nothing in any range touched) | **all shift +2**; `line`/`start_line` advance, `original_*` frozen; `commit_id` advances to new head | orig+2 | frozen | new head | false |
| G2 | Edit *inside* A's range (its first line) | **outdated**; `line`/`start_line` → null; `commit_id` frozen at last commit where it was valid | null | 6 | abcf01a (frozen) | **true** |
| G3 | Edit the line *immediately above* B (not in range) | B untouched — survives, same `line` | 12 | 10 | new head | false |
| G4 | Delete C's whole range | outdated | null | 14 | frozen | true |
| G5 | Delete *part* of D's range (first of two lines) | **outdated** — partial deletion of a multi-line range kills the whole thread (no shrink) | null | 18 | frozen | true |
| G6 | Insert a new line *between* F's two lines (F's own lines unchanged) | survives; **range grows** to 3 lines (REST `start_line`=23,`line`=25; UI: "Comment on lines R20 to R22" in the repeat run L) | 25 | 25 | new head | false |
| G7 | Edit G's first line | outdated | null | 29 | frozen | true |
| G8 | Revert G7 byte-for-byte (G's lines identical to original again) | **stays outdated** — GitHub never re-guesses once outdated | null | 29 | frozen | true |
| G9 | **Force-push** (amend head; append a line at EOF; B/E/F/H lines untouched) | B/E/F/H survive unchanged. **Surprise:** G's REST `line`/`start_line` re-populated (29/28) and GraphQL `line` too, yet `isOutdated` stayed **true** and `commit_id` stayed frozen — GitHub re-mapped from the frozen `commit_id` to the new head, but the outdated flag is sticky | 29 (!) | 29 | a49ae45 (frozen) | **true** |
| G10 | Full rewrite (every line changed, same count) | everything outdated | null | frozen | frozen | true |
| G11 | Cut a 2-line block (I) and paste it at EOF (content identical, moved) | **outdated** — no move detection; a moved block is delete+add | null | 11 | frozen | true |
| G12 | Trailing whitespace added to J's line (whitespace-only) | **outdated** — byte-level; whitespace changes count | null | 5 | frozen | true |
| G13 | `git mv doc.md → doc-renamed.md` (no content change) | survives; `path` updated to new name; `line` unchanged | 18 | 20 | new head | false |
| G14 | (repeat of G6 for UI check) insert between L's lines | survives; UI shows lines R20–R22 (grown) | 22 | 21 | new head | false |

Additional API facts observed:
- `original_line`/`original_start_line`/`original_commit_id`/`original_position` are **frozen at creation** and are what the UI renders for outdated threads (with the original hunk snippet).
- `commit_id` **advances with every push while the comment is still valid** and **freezes at the last valid commit** when it becomes outdated — so a client can always find the last content the comment was anchored to.
- `position` (diff-relative) is meaningless once outdated (returns 1) — don't rely on it.
- REST and GraphQL briefly disagreed on F's `startLine` right after the force-push (GraphQL 24 vs REST 23), then agreed on the L repeat (`startLine`=20, `line`=22). Treat REST as canonical; possibly a cache artifact.
- UI: outdated threads are hidden from *Files changed* by default and listed in *Conversation* with an **Outdated** badge; still replyable/resolvable; outdated ⊥ resolved.

### GitHub — behavioral summary

1. Line-diff mapping with a **binary** outcome per thread: either every line in the range maps cleanly (possibly shifted, possibly *grown* by pure insertions inside) or the thread is outdated. No shrink, no partial.
2. **Never re-guessed**: reverts don't revive; force-push re-mapping can repopulate `line` but the flag stays.
3. **Byte-exact**: whitespace-only or moved-block changes outdate. Renames don't.
4. `original_*` frozen + `commit_id` advancing/freezing is the storage model to copy.

---

## Surprises worth carrying into ticket 05 (anchor model)

1. **Both systems grow the range on pure insertion inside it** (D5, G6/G14). Google Docs additionally *shrinks* on partial deletion (D2–D4) where GitHub outdates (G5). Our line-then-quote model can do better than GitHub here: line-level mapping says "changed", quote refinement can find the surviving substring → **refined** state rather than outdated. Decide whether shrinking is desirable (Docs says yes; the quote-is-truth stance from Hypothesis says only if the *surviving* text is a substring of the original quote).
2. **Insertion at the boundary is outside the span** in Docs (D6/D7). Our quote-based refinement naturally agrees (prefix/suffix context matches). Line-level: an insertion of a *new line* immediately above/below a range is outside it in GitHub too (G3, G1). Consistent — adopt.
3. **Type-over vs find-and-replace are indistinguishable to a before/after differ** but Docs treats them oppositely (D8 orphan vs D10' survive-and-extend). We must pick: recommend **quote gone ⇒ outdated**, and document D10' as a deliberate divergence.
4. **Orphan/outdated is a state, not a deletion**, in both products: still listed, still open, still replyable, still resolvable; just not placed. Adopt verbatim (matches the prior-art recommendation "never silent").
5. **Never revive automatically** (G8) vs **undo revives** (Docs). Docs' revival is operation-replay, not re-anchoring. Recommendation for Annotary: on each new Version, re-anchor only *currently anchored* threads; outdated threads stay outdated even if the quote reappears — unless we explicitly choose Hypothesis-style re-search of the quote (cheap, deterministic, but risks anchoring to a *different* occurrence). Ticket 05 must decide; fixture G8 must be in the suite either way.
6. **Moved blocks** (G11) — histogram/patience diffs also see delete+add. If we want "moved" (the map's Notes mention *moved*), it needs an explicit second pass (search the quote/lines elsewhere) — same mechanism as revival, same risk. Decide together with (5).
7. **Whitespace-only edits** outdate in GitHub (G12). For Markdown, trailing-whitespace churn is common (editors strip it). Recommend: line-diff on **rstripped** lines, or treat whitespace-only line changes as "unchanged" for anchoring. Needs a fixture.
8. **File rename is transparent** (G13) — irrelevant to us (Document identity is separate from filename) but confirms nothing to do.
9. **Force-push behaviour** (G9) is a GitHub quirk (re-map from frozen commit); irrelevant to Annotary since Versions are linear, but it *does* show that "map from the last commit where the anchor was valid, not from the original" is how GitHub computes `line` — i.e. mapping is **incremental N→N+1**, not original→head. Adopt: re-anchor from Version N's resolved position to N+1, storing both `original` (frozen) and `current`.
10. **Docs' panel label is the frozen original quote** even after the range has changed (D12). Adopt: `anchor.original.quote` is what agents and the UI quote; `anchor.current` is where it points now.

## Fixture-suite skeleton (for ticket 15)

Each row above becomes a case `{before, after, anchor_before, expected: {status, current_range|null}}` on Markdown text. Minimum set: D1–D8, D10, D11, G1–G8, G10–G13. Add Annotary-specific rows once ticket 05 decides: whitespace-only line change; quote reappears at same/different location; multi-line span with one line partially edited; span crossing a Markdown block boundary; asset-referencing line edited.
