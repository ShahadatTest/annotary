//! Does a local file already contain the changes the head made since the pusher's base?
//! (ticket 17, ADR 0002). `push` runs [`contains_changes`] when the Sidecar's `base_version` is
//! behind the head; the server re-runs it on commit. annotary never merges: the Member or agent
//! merges with their own tools (git, by hand, applying the printed diff) and this only checks the
//! result.
//!
//! **Algorithm** — a line-level 3-way comparison: the histogram line diffs base→head and
//! base→local (the same line diff re-anchoring uses) are grouped into chunks of overlapping
//! changes over the base. Every chunk the head changed is judged on its own:
//! - local's text for the chunk == head's ⇒ contained;
//! - local did not touch the chunk (its text is the base's) ⇒ **missing**;
//! - both changed it, differently ⇒ the same comparison again inside the chunk at **word-token**
//!   level (UAX #29 tokens, as re-anchoring uses): contained iff every token chunk the head
//!   changed reads the same in local (two edits to different sentences of one line, or a merged
//!   sentence plus an edit elsewhere on the line); otherwise missing.
//!
//! Two changes overlap when their base ranges intersect, or when one of them is a pure insertion
//! or deletion touching the other (where an insertion lands next to an edit is ambiguous, so the
//! two are judged together); two pure insertions at one point are contained when the head's is a
//! run inside local's (both appended a section). Next to another edit, or among repeated lines,
//! two diffs of the same base can place one change differently, so the comparison is repeated
//! under a few alignments (`Align`) and the change counts as contained when any of them shows
//! it — each is sound on its own (local must read exactly like the head in an aligned region), so
//! the retries only remove false refusals. What remains refused without being a real conflict are
//! edits separated only by blank or repeated lines whose alignment no diff gets right — the safe
//! direction (ADR 0002; `--discard-head-changes` is the escape). Missing changes are reported as
//! base→head unified-diff hunks with up to [`CONTEXT`] lines of context and the head's line
//! numbers on the `+` side.
//!
//! Pure function of three texts: no time limits, no randomness, CRLF tolerated and a final newline
//! ignored ([`annotary_sidecar::range::lines`] semantics). Oversized token hunks use the same
//! bounded fallback as the mapping ([`crate::mapping::MAX_HUNK_TOKENS`]).
//!
//! Assets are compared by manifest, not by text: [`missing_asset_changes`].

use std::collections::BTreeMap;

use imara_diff::{Algorithm, Diff, Interner};
use unicode_segmentation::UnicodeSegmentation;

use annotary_sidecar::range::lines;

use crate::mapping::equal_str_runs;

/// Lines of context around a missing change in the unified diff.
pub const CONTEXT: usize = 3;

/// The verdict of [`contains_changes`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Containment {
    /// Every change base→head is in the local text (or the head did not change the text).
    Contained,
    /// These base→head changes are not in the local text (never empty).
    Missing { hunks: Vec<Hunk> },
}

impl Containment {
    pub fn is_contained(&self) -> bool {
        matches!(self, Containment::Contained)
    }
}

/// One block of a base→head unified diff: `@@ -base_start,base_len +head_start,head_len @@`
/// (1-based starts; a zero-length side names the line before it, as `diff -u` does) and its lines,
/// each prefixed with `' '` (context), `'-'` (base only) or `'+'` (head only).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hunk {
    pub base_start: usize,
    pub base_len: usize,
    pub head_start: usize,
    pub head_len: usize,
    pub lines: Vec<String>,
}

impl Hunk {
    /// `@@ -12,3 +12,4 @@`.
    pub fn header(&self) -> String {
        format!("@@ -{},{} +{},{} @@", self.base_start, self.base_len, self.head_start, self.head_len)
    }
}

/// The hunks as unified-diff text: each header, then its lines, `\n`-terminated.
pub fn format_unified(hunks: &[Hunk]) -> String {
    let mut out = String::new();
    for h in hunks {
        out.push_str(&h.header());
        out.push('\n');
        for l in &h.lines {
            out.push_str(l);
            out.push('\n');
        }
    }
    out
}

/// Does `local` contain every change `base` → `head`? See the module docs.
pub fn contains_changes(base: &str, head: &str, local: &str) -> Containment {
    let (b, h, l) = (lines(base), lines(head), lines(local));
    let head_raw = [line_diff(&b, &h, false), line_diff(&b, &h, true)];
    let head_edits = normalize(&head_raw[0], &b, &h, Align::CANONICAL);
    if head_edits.is_empty() {
        return Containment::Contained;
    }
    if b == l {
        // A stale, unedited file: every change is missing (no alignment needed).
        return Containment::Missing { hunks: unified(&b, &h, &head_edits, &head_edits) };
    }
    let local_raw = [line_diff(&b, &l, false), line_diff(&b, &l, true)];
    let mut tried: Vec<(Vec<Edit>, Vec<Edit>)> = Vec::new();
    let mut first_missing: Option<Vec<Edit>> = None;
    for (ha, la) in Align::pairs() {
        let he = normalize(&head_raw[ha.myers as usize], &b, &h, ha);
        let le = normalize(&local_raw[la.myers as usize], &b, &l, la);
        if tried.iter().any(|(x, y)| *x == he && *y == le) {
            continue;
        }
        let missing = missing_edits(&b, &h, &l, &he, &le);
        if missing.is_empty() {
            return Containment::Contained;
        }
        if first_missing.is_none() {
            first_missing = Some(missing); // the canonical alignment's: what the hunks show
        }
        tried.push((he, le));
    }
    let missing = first_missing.expect("Align::pairs() is not empty");
    Containment::Missing { hunks: unified(&b, &h, &head_edits, &missing) }
}

/// The head's line edits (`he`) that `local` lacks under one pair of alignments.
fn missing_edits(b: &[&str], h: &[&str], l: &[&str], he: &[Edit], le: &[Edit]) -> Vec<Edit> {
    let mut missing = Vec::new();
    for c in chunks(he, le) {
        if !c.head_changed || chunk_contained(&c, h, l) {
            continue;
        }
        if c.local_changed && tokens_contained(&b[c.base.clone()], &h[c.head.clone()], &l[c.local.clone()]) {
            continue;
        }
        // The head's own line changes inside the chunk (not the chunk, which may span lines only
        // local touched).
        missing.extend(he.iter().filter(|e| within(e, &c.base)).copied());
    }
    missing
}

/// How a diff is computed and normalised. Any choice is a valid alignment of the same two
/// texts; next to another change, or among identical lines / tokens, the choices disagree about
/// *where* a change happened (which of two blank lines was deleted, whether an append after an
/// edited line is part of that edit). The check is sound under every alignment — local must read
/// exactly like the head in an aligned region — so it tries a few and accepts when any of them
/// shows the change in local; only when none does is the push refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Align {
    /// Myers (minimal) instead of histogram — line level only; tokens always use Myers.
    myers: bool,
    /// Split a replacement at the earliest copy of the item that follows it ([`split_early`]):
    /// 0 = never, 1 = among the new items, 2 = among the replaced ones.
    split: u8,
    /// Slide pure insertions / deletions down instead of up ([`slide`]).
    down: bool,
}

impl Align {
    /// Histogram, no split, slid up: the alignment hunks are reported with.
    const CANONICAL: Align = Align { myers: false, split: 0, down: false };

    /// Every normalisation of one algorithm.
    fn variants(myers: bool) -> impl Iterator<Item = Align> {
        [0u8, 1, 2].into_iter().flat_map(move |split| [false, true].map(|down| Align { myers, split, down }))
    }

    /// `(base → head, base → local)` pairs, the canonical pair first: every normalisation pair of
    /// the histogram diffs, then of the Myers diffs.
    fn pairs() -> impl Iterator<Item = (Align, Align)> {
        [false, true]
            .into_iter()
            .flat_map(|m| Align::variants(m).flat_map(move |ha| Align::variants(m).map(move |la| (ha, la))))
    }
}

/// An Asset change the local manifest lacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetChange {
    pub reference: String,
    pub kind: AssetChangeKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AssetChangeKind {
    /// The head references `ref`, the base did not.
    Added,
    /// The base referenced `ref`, the head does not.
    Removed,
    /// Same `ref`, different bytes (or present ↔ missing).
    Changed,
}

impl AssetChangeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AssetChangeKind::Added => "added",
            AssetChangeKind::Removed => "removed",
            AssetChangeKind::Changed => "changed",
        }
    }
}

/// Manifests as `(ref, hash)` pairs (`None` = referenced but missing). For every `ref` whose
/// entry differs between `base` and `head` (added, removed, other bytes), `local` must have the
/// head's entry; the refs it lacks are returned in `ref` order. A `ref` the head did not touch is
/// free for local to change.
pub fn missing_asset_changes(
    base: &[(String, Option<String>)],
    head: &[(String, Option<String>)],
    local: &[(String, Option<String>)],
) -> Vec<AssetChange> {
    let map = |m: &[(String, Option<String>)]| -> BTreeMap<String, Option<String>> { m.iter().cloned().collect() };
    let (b, h, l) = (map(base), map(head), map(local));
    let mut refs: Vec<&String> = b.keys().chain(h.keys()).collect();
    refs.sort();
    refs.dedup();
    refs.into_iter()
        .filter_map(|r| {
            let (bv, hv) = (b.get(r), h.get(r));
            if bv == hv || l.get(r) == hv {
                return None;
            }
            let kind = match (bv, hv) {
                (None, Some(_)) => AssetChangeKind::Added,
                (Some(_), None) => AssetChangeKind::Removed,
                _ => AssetChangeKind::Changed,
            };
            Some(AssetChange { reference: r.clone(), kind })
        })
        .collect()
}

// ---------------------------------------------------------------- edits and chunks

/// Base items `bs..be` became side items `ss..se`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Edit {
    bs: usize,
    be: usize,
    ss: usize,
    se: usize,
}

impl Edit {
    fn delta(&self) -> isize {
        (self.se - self.ss) as isize - (self.be - self.bs) as isize
    }
}

/// A group of overlapping changes: its base range and the matching head / local ranges.
struct Chunk {
    base: std::ops::Range<usize>,
    head: std::ops::Range<usize>,
    local: std::ops::Range<usize>,
    head_changed: bool,
    local_changed: bool,
}

fn within(e: &Edit, r: &std::ops::Range<usize>) -> bool {
    e.bs >= r.start && e.be <= r.end
}

/// Whether an edit on base `s..e` belongs to the chunk `cs..ce`: the ranges intersect, or one of
/// them is empty (a pure insertion — or a chunk of insertions) and they touch.
fn overlaps(cs: usize, ce: usize, e: &Edit) -> bool {
    (e.bs < ce && cs < e.be) || ((cs == ce || e.bs == e.be) && e.bs <= ce && cs <= e.be)
}

fn chunks(head: &[Edit], local: &[Edit]) -> Vec<Chunk> {
    let (mut i, mut j) = (0usize, 0usize);
    let (mut dh, mut dl) = (0isize, 0isize);
    let mut out = Vec::new();
    while i < head.len() || j < local.len() {
        let first = if j >= local.len() || (i < head.len() && head[i].bs <= local[j].bs) { head[i] } else { local[j] };
        let (cs, mut ce) = (first.bs, first.be);
        let (dh0, dl0) = (dh, dl);
        let (mut hc, mut lc) = (false, false);
        loop {
            if i < head.len() && overlaps(cs, ce, &head[i]) {
                ce = ce.max(head[i].be);
                dh += head[i].delta();
                hc = true;
                i += 1;
            } else if j < local.len() && overlaps(cs, ce, &local[j]) {
                ce = ce.max(local[j].be);
                dl += local[j].delta();
                lc = true;
                j += 1;
            } else {
                break;
            }
        }
        let at = |n: usize, d: isize| (n as isize + d) as usize;
        out.push(Chunk {
            base: cs..ce,
            head: at(cs, dh0)..at(ce, dh),
            local: at(cs, dl0)..at(ce, dl),
            head_changed: hc,
            local_changed: lc,
        });
    }
    out
}

/// Raw line diff `a` → `b` as edits: histogram, or minimal Myers.
fn line_diff(a: &[&str], b: &[&str], myers: bool) -> Vec<Edit> {
    let mut interner: Interner<&str> = Interner::new(a.len() + b.len());
    let before: Vec<_> = a.iter().map(|l| interner.intern(l)).collect();
    let after: Vec<_> = b.iter().map(|l| interner.intern(l)).collect();
    let mut diff = Diff::default();
    let algorithm = if myers { Algorithm::MyersMinimal } else { Algorithm::Histogram };
    diff.compute_with(algorithm, &before, &after, interner.num_tokens());
    diff.hunks()
        .map(|h| Edit {
            bs: h.before.start as usize,
            be: h.before.end as usize,
            ss: h.after.start as usize,
            se: h.after.end as usize,
        })
        .collect()
}

/// A raw diff normalised per `align` (split, then slide).
fn normalize(raw: &[Edit], a: &[&str], b: &[&str], align: Align) -> Vec<Edit> {
    let mut edits = match align.split {
        0 => raw.to_vec(),
        n => split_early(raw, a, b, n == 2),
    };
    slide(&mut edits, a, b, align.down);
    edits
}

/// A replacement `a[bs..be] → b[ss..se]` followed by the equal item `a[be] == b[se]` may as well
/// match that item to its earliest copy inside the replacement. Among the new items (`b[k]`):
/// the edit becomes `a[bs..be] → b[ss..k]` plus a pure insertion of `b[k+1..=se]` after `a[be]`
/// (an edited last line followed by an appended section, which a diff likes to fold into one
/// edit). Among the replaced items (`deleted_side`, `a[j]`): `a[bs..j] → b[ss..se]` plus a pure
/// deletion of `a[j+1..=be]` (an edit followed by the deletion of a line equal to a later one).
fn split_early(raw: &[Edit], a: &[&str], b: &[&str], deleted_side: bool) -> Vec<Edit> {
    let mut out: Vec<Edit> = Vec::with_capacity(raw.len() + 1);
    let mut pending: Option<Edit> = None; // the pure edit produced by the previous split
    for (i, &e) in raw.iter().enumerate() {
        let mut e = e;
        if let Some(p) = pending.take() {
            if p.be == e.bs && p.se == e.ss {
                e = Edit { bs: p.bs, be: e.be, ss: p.ss, se: e.se };
            } else {
                out.push(p);
            }
        }
        let next_bs = raw.get(i + 1).map_or(a.len(), |n| n.bs);
        let splittable = e.bs < e.be && e.ss < e.se && e.be < next_bs && e.be < a.len();
        if !splittable {
            out.push(e);
            continue;
        }
        if deleted_side {
            match (e.bs..e.be).find(|&j| a[j] == a[e.be]) {
                Some(j) => {
                    out.push(Edit { bs: e.bs, be: j, ss: e.ss, se: e.se });
                    pending = Some(Edit { bs: j + 1, be: e.be + 1, ss: e.se + 1, se: e.se + 1 });
                }
                None => out.push(e),
            }
        } else {
            match (e.ss..e.se).find(|&k| b[k] == a[e.be]) {
                Some(k) => {
                    if e.bs < e.be || e.ss < k {
                        out.push(Edit { bs: e.bs, be: e.be, ss: e.ss, se: k });
                    }
                    pending = Some(Edit { bs: e.be + 1, be: e.be + 1, ss: k + 1, se: e.se + 1 });
                }
                None => out.push(e),
            }
        }
    }
    out.extend(pending);
    out
}

/// Move every pure insertion / deletion as far up (or down) as equal items allow — a blank line
/// deleted from a pair of blank lines is the first one (or the last) — so two diffs from the same
/// base put the same change at the same place whatever else differs around it.
fn slide(edits: &mut [Edit], a: &[&str], b: &[&str], down: bool) {
    let n = edits.len();
    for k in 0..n {
        let e = edits[k];
        let (pure_insert, pure_delete) = (e.bs == e.be, e.ss == e.se);
        if !(pure_insert || pure_delete) {
            continue;
        }
        let mut e = e;
        if down {
            let (next_bs, next_ss) = if k + 1 < n { (edits[k + 1].bs, edits[k + 1].ss) } else { (a.len(), b.len()) };
            while e.be < next_bs && e.se < next_ss && if pure_insert { b[e.ss] == b[e.se] } else { a[e.bs] == a[e.be] }
            {
                e = Edit { bs: e.bs + 1, be: e.be + 1, ss: e.ss + 1, se: e.se + 1 };
            }
        } else {
            let (prev_be, prev_se) = if k > 0 { (edits[k - 1].be, edits[k - 1].se) } else { (0, 0) };
            while e.bs > prev_be
                && e.ss > prev_se
                && if pure_insert { b[e.ss - 1] == b[e.se - 1] } else { a[e.bs - 1] == a[e.be - 1] }
            {
                e = Edit { bs: e.bs - 1, be: e.be - 1, ss: e.ss - 1, se: e.se - 1 };
            }
        }
        edits[k] = e;
    }
}

/// Raw token diff `a` → `b` as edits: the gaps between the equal runs.
fn token_diff(a: &[&str], b: &[&str]) -> Vec<Edit> {
    let mut runs = equal_str_runs(a, b);
    runs.push((a.len(), b.len(), 0)); // closes the tail
    let (mut ai, mut bi) = (0usize, 0usize);
    let mut out = Vec::new();
    for (oi, ni, len) in runs {
        if oi > ai || ni > bi {
            out.push(Edit { bs: ai, be: oi, ss: bi, se: ni });
        }
        ai = oi + len;
        bi = ni + len;
    }
    out
}

/// The word tokens of some lines, each line followed by a `\n` token.
fn tokens<'a>(ls: &[&'a str]) -> Vec<&'a str> {
    let mut out = Vec::new();
    for l in ls {
        out.extend(l.split_word_bounds());
        out.push("\n");
    }
    out
}

/// The token-level check of one line chunk both sides changed: the same chunk comparison on
/// word tokens, under every split / slide normalisation pair (see [`Align`]).
fn tokens_contained(base: &[&str], head: &[&str], local: &[&str]) -> bool {
    let (b, h, l) = (tokens(base), tokens(head), tokens(local));
    let (hr, lr) = (token_diff(&b, &h), token_diff(&b, &l));
    Align::variants(false).any(|ha| {
        let he = normalize(&hr, &b, &h, ha);
        Align::variants(false).any(|la| {
            let le = normalize(&lr, &b, &l, la);
            chunks(&he, &le).iter().all(|c| !c.head_changed || chunk_contained(c, &h, &l))
        })
    })
}

/// One chunk the head changed reads as the head's in local: the same items, or — when both sides
/// only inserted at the same point — the head's insertion as one run inside local's (both
/// appended a section, local after merging the head's).
fn chunk_contained(c: &Chunk, h: &[&str], l: &[&str]) -> bool {
    let (hr, lr) = (&h[c.head.clone()], &l[c.local.clone()]);
    hr == lr || (c.base.is_empty() && lr.windows(hr.len().max(1)).any(|w| w == hr))
}

// ---------------------------------------------------------------- unified diff

/// The missing edits as `diff -u`-style hunks: up to [`CONTEXT`] lines of context, never reaching
/// into another base→head change (so context lines read the same in base and head); missing edits
/// whose context meets are one hunk unless another change lies between them.
fn unified(b: &[&str], h: &[&str], all: &[Edit], missing: &[Edit]) -> Vec<Hunk> {
    // Context may not cross the neighbouring base→head edits.
    let bounds = |e: &Edit| -> (usize, usize) {
        let k = all.iter().position(|x| x == e).expect("missing edits are head edits");
        let lo = if k == 0 { 0 } else { all[k - 1].be };
        let hi = if k + 1 == all.len() { b.len() } else { all[k + 1].bs };
        (e.bs.saturating_sub(CONTEXT).max(lo), (e.be + CONTEXT).min(hi))
    };
    let mut groups: Vec<Vec<Edit>> = Vec::new();
    for e in missing {
        if let Some(g) = groups.last_mut() {
            let last = *g.last().expect("groups are never empty");
            let adjacent = all.iter().position(|x| *x == last).map(|k| all.get(k + 1) == Some(e)).unwrap_or(false);
            if adjacent && bounds(&last).1 >= bounds(e).0 {
                g.push(*e);
                continue;
            }
        }
        groups.push(vec![*e]);
    }
    groups
        .into_iter()
        .map(|g| {
            let (first, last) = (g[0], *g.last().expect("non-empty"));
            let (start, _) = bounds(&first);
            let (_, end) = bounds(&last);
            // The head offset of base line `start` (no edits between `start` and `first`).
            let head_start = first.ss - (first.bs - start);
            let mut out = Vec::new();
            let mut at = start;
            for e in &g {
                out.extend(b[at..e.bs].iter().map(|l| format!(" {l}")));
                out.extend(b[e.bs..e.be].iter().map(|l| format!("-{l}")));
                out.extend(h[e.ss..e.se].iter().map(|l| format!("+{l}")));
                at = e.be;
            }
            out.extend(b[at..end].iter().map(|l| format!(" {l}")));
            let base_len = end - start;
            let head_len =
                base_len - g.iter().map(|e| e.be - e.bs).sum::<usize>() + g.iter().map(|e| e.se - e.ss).sum::<usize>();
            // `diff -u`: a zero-length side names the line before it.
            let one = |s: usize, n: usize| if n == 0 { s } else { s + 1 };
            Hunk {
                base_start: one(start, base_len),
                base_len,
                head_start: one(head_start, head_len),
                head_len,
                lines: out,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn missing(base: &str, head: &str, local: &str) -> String {
        match contains_changes(base, head, local) {
            Containment::Contained => String::new(),
            Containment::Missing { hunks } => format_unified(&hunks),
        }
    }

    const BASE: &str = "# Plan\n\n## Storage\nSQLite, one file.\n\n## API\nREST over HTTP.\n\n## Later\nNothing yet.\n";

    // spec: S17-01
    #[test]
    fn contained_missing_and_overlaps() {
        let head = BASE.replace("SQLite, one file.", "SQLite, one file, WAL mode.\nBlobs are content-addressed.");
        // local == head ⇒ contained; local == base (stale, unedited) ⇒ missing.
        assert!(contains_changes(BASE, &head, &head).is_contained());
        assert_eq!(
            missing(BASE, &head, BASE),
            "@@ -1,7 +1,8 @@\n # Plan\n \n ## Storage\n-SQLite, one file.\n+SQLite, one file, WAL mode.\n+Blobs are content-addressed.\n \n ## API\n REST over HTTP.\n"
        );
        // Head unchanged ⇒ contained whatever local did.
        assert!(contains_changes(BASE, BASE, "anything\n").is_contained());
        // The head's change merged plus an edit of local's own elsewhere ⇒ contained.
        let merged = head.replace("Nothing yet.", "Search, later.");
        assert!(contains_changes(BASE, &head, &merged).is_contained());
        // Local's own edit elsewhere, the head's change not merged ⇒ missing (only the head's).
        let own = BASE.replace("Nothing yet.", "Search, later.");
        let m = missing(BASE, &head, &own);
        assert!(m.contains("+SQLite, one file, WAL mode.") && !m.contains("Search"), "{m}");
    }

    // spec: S17-01
    #[test]
    fn overlapping_edits_are_judged_by_word_tokens() {
        let base = "We store everything in SQLite. Blobs go to disk. Tokens are hashed.\n";
        let head = "We store everything in SQLite with WAL. Blobs go to disk. Tokens are hashed.\n";
        // Different sentences of the same line ⇒ contained.
        let local = "We store everything in SQLite with WAL. Blobs go to disk. Tokens are argon2-hashed.\n";
        assert!(contains_changes(base, head, local).is_contained());
        // Local edited the line but not where the head did ⇒ missing.
        let local = "We store everything in SQLite. Blobs go to disk. Tokens are argon2-hashed.\n";
        assert_eq!(missing(base, head, local), format!("@@ -1,1 +1,1 @@\n-{}+{}", base, head));
        // Merged the head's sentence and then reworded that same sentence ⇒ missing (open
        // question 1 of ticket 17: refuse; `--discard-head-changes` is the escape).
        let local = "We store everything in SQLite using WAL. Blobs go to disk. Tokens are hashed.\n";
        assert!(!contains_changes(base, head, local).is_contained());
        // Adjacent lines: head edits one, local merges it and edits the next ⇒ contained.
        let base = "alpha one\nbeta two\ngamma three\n";
        let head = "alpha ONE\nbeta two\ngamma three\n";
        assert!(contains_changes(base, head, "alpha ONE\nbeta TWO\ngamma three\n").is_contained());
        assert!(!contains_changes(base, head, "alpha one\nbeta TWO\ngamma three\n").is_contained());
        // Both appended at the end, local after merging the head's section ⇒ contained; local's
        // own edit of the last line plus both appends ⇒ contained too.
        let head = format!("{base}\n## Head\nfrom head\n");
        assert!(contains_changes(base, &head, &format!("{head}\n## Local\nfrom local\n")).is_contained());
        let local = format!("{}\n## Head\nfrom head\n\n## Local\n", base.replace("three", "3"));
        assert!(contains_changes(base, &head, &local).is_contained());
        assert!(!contains_changes(base, &head, &format!("{base}\n## Local\nfrom local\n")).is_contained());
        // Head inserts a line right where local edits ⇒ judged together.
        let head = "alpha one\nNEW\nbeta two\ngamma three\n";
        assert!(contains_changes(base, head, "alpha one\nNEW\nbeta 2\ngamma three\n").is_contained());
        assert!(!contains_changes(base, head, "alpha one\nbeta 2\ngamma three\n").is_contained());
    }

    // spec: S17-01, S17-05
    #[test]
    fn several_hunks_some_missing_and_edge_cases() {
        let base: String = (1..=30).map(|i| format!("line {i}\n")).collect();
        let head = base.replace("line 3\n", "line three\n").replace("line 20\n", "line twenty\n");
        let local = base.replace("line 3\n", "line three\n");
        assert_eq!(
            missing(&base, &head, &local),
            "@@ -17,7 +17,7 @@\n line 17\n line 18\n line 19\n-line 20\n+line twenty\n line 21\n line 22\n line 23\n"
        );
        // Two missing changes close together share one hunk; context stops at a contained change.
        let head =
            base.replace("line 10\n", "ten\n").replace("line 13\n", "thirteen\n").replace("line 15\n", "fifteen\n");
        let local = base.replace("line 15\n", "fifteen\n");
        assert_eq!(
            missing(&base, &head, &local),
            "@@ -7,8 +7,8 @@\n line 7\n line 8\n line 9\n-line 10\n+ten\n line 11\n line 12\n-line 13\n+thirteen\n line 14\n"
        );
        // CRLF and a missing final newline are not changes.
        let crlf = local.replace('\n', "\r\n");
        assert_eq!(missing(&base, &local, crlf.trim_end()), "");
        // Empty base: everything the head wrote is a change.
        assert_eq!(missing("", "a\nb\n", ""), "@@ -0,0 +1,2 @@\n+a\n+b\n");
        assert!(contains_changes("", "a\nb\n", "a\nb\nc\n").is_contained());
        // Whole file rewritten by the head, local untouched ⇒ every line missing.
        let m = missing("x\ny\n", "p\nq\n", "x\ny\n");
        assert_eq!(m, "@@ -1,2 +1,2 @@\n-x\n-y\n+p\n+q\n");
        // Head deletes, local keeps ⇒ missing; local deletes too ⇒ contained.
        assert_eq!(missing("a\nb\nc\n", "a\nc\n", "a\nb\nc\n"), "@@ -1,3 +1,2 @@\n a\n-b\n c\n");
        assert!(contains_changes("a\nb\nc\n", "a\nc\n", "a\nc\nd\n").is_contained());
        // Determinism.
        assert_eq!(missing(&base, &head, &local), missing(&base, &head, &local));
    }

    // spec: S17-01
    #[test]
    fn oversized_hunks_use_the_bounded_fallback() {
        let words = |w: &str, n: usize| -> String { (0..n).map(|i| format!("{w}{i} ")).collect::<String>() + "\n" };
        let base = words("a", 12_000);
        let head = words("b", 12_000);
        let local = words("c", 12_000);
        assert!(!contains_changes(&base, &head, &local).is_contained());
        assert!(contains_changes(&base, &head, &head).is_contained());
    }

    // spec: S17-06
    #[test]
    fn asset_changes() {
        let m = |xs: &[(&str, Option<&str>)]| -> Vec<(String, Option<String>)> {
            xs.iter().map(|(r, h)| (r.to_string(), h.map(str::to_string))).collect()
        };
        let base = m(&[("a.png", Some("h1")), ("b.png", Some("h2")), ("c.png", Some("h3"))]);
        let head = m(&[("a.png", Some("h1x")), ("c.png", Some("h3")), ("d.png", Some("h4"))]);
        // Stale copy: changed a, removed b, added d all missing.
        let got = missing_asset_changes(&base, &head, &base);
        let kinds: Vec<(&str, &str)> = got.iter().map(|c| (c.reference.as_str(), c.kind.as_str())).collect();
        assert_eq!(kinds, vec![("a.png", "changed"), ("b.png", "removed"), ("d.png", "added")]);
        // Local has the head's entries and changed c (untouched by the head) ⇒ nothing missing.
        let local = m(&[("a.png", Some("h1x")), ("c.png", Some("h3y")), ("d.png", Some("h4"))]);
        assert!(missing_asset_changes(&base, &head, &local).is_empty());
        // Present ↔ missing counts as a change.
        let head2 = m(&[("a.png", None), ("b.png", Some("h2")), ("c.png", Some("h3"))]);
        assert_eq!(missing_asset_changes(&base, &head2, &base)[0].kind, AssetChangeKind::Changed);
    }
}
