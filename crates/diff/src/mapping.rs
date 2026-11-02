//! The mapping from Version N's characters to Version N+1's: a histogram **line** diff first
//! (`imara-diff`), then — inside every changed hunk — a Myers diff (`similar`) of the hunk's
//! **tokens** (UAX #29 word boundaries: words, runs of spaces, punctuation, one token per CJK
//! ideograph, and the line breaks), so a character survives an edit iff the word it belongs to
//! survives. Characters of unchanged lines and of surviving tokens map 1:1 to their new offsets;
//! everything else was deleted or replaced. The result is a sorted list of equal segments
//! `(old offset, new offset, length)` in flat coordinates ([`crate::text::Text`]).
//!
//! Tokens rather than single characters because a Myers LCS of raw characters between two
//! unrelated sentences is never empty (stray letters and spaces match), which would turn a
//! type-over or a rewrite into a "refined" placement on garbage; with words as tokens the
//! survivors are the words that actually survived — what the fixture table calls surviving text.
//!
//! Everything here is a pure function of the two texts: no time limits, no randomness. Very
//! large hunks (more than [`MAX_HUNK_TOKENS`] tokens on both sides together — a contiguous
//! rewrite of tens of thousands of words) fall back from `similar`'s minimal Myers to
//! `imara-diff`'s heuristic Myers, which is bounded in time and still deterministic.

use imara_diff::{Algorithm, Diff, Interner};
use unicode_segmentation::UnicodeSegmentation;

use crate::text::Text;

/// Above this many tokens (old + new) in one hunk the token diff uses the bounded heuristic
/// Myers instead of the minimal one.
pub const MAX_HUNK_TOKENS: usize = 20_000;

/// `len` characters at `old` in text N are the same characters at `new` in text N+1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Segment {
    pub old: usize,
    pub new: usize,
    pub len: usize,
}

impl Segment {
    pub fn old_end(&self) -> usize {
        self.old + self.len
    }
}

/// One token of a hunk: its text, its flat start offset and its length in code points.
struct Tok<'a> {
    text: &'a str,
    start: usize,
    len: usize,
}

fn tokens<'a>(t: &Text<'a>, lines: std::ops::Range<usize>) -> Vec<Tok<'a>> {
    let mut out = Vec::new();
    for i in lines {
        let mut at = t.start_of(i);
        for w in t.lines[i].split_word_bounds() {
            let len = w.chars().count();
            out.push(Tok { text: w, start: at, len });
            at += len;
        }
        out.push(Tok { text: "\n", start: at, len: 1 });
    }
    out
}

/// Append a segment, merging with the previous one when contiguous on both sides.
fn push(segs: &mut Vec<Segment>, s: Segment) {
    if s.len == 0 {
        return;
    }
    if let Some(last) = segs.last_mut()
        && last.old_end() == s.old
        && last.new + last.len == s.new
    {
        last.len += s.len;
        return;
    }
    segs.push(s);
}

/// The equal segments between `old` and `new`, sorted by old offset.
pub(crate) fn segments(old: &Text<'_>, new: &Text<'_>) -> Vec<Segment> {
    // 1. Histogram line diff.
    let mut interner: Interner<&str> = Interner::new(old.line_count() + new.line_count());
    let before: Vec<_> = old.lines.iter().map(|l| interner.intern(l)).collect();
    let after: Vec<_> = new.lines.iter().map(|l| interner.intern(l)).collect();
    let mut diff = Diff::default();
    diff.compute_with(Algorithm::Histogram, &before, &after, interner.num_tokens());

    let mut segs: Vec<Segment> = Vec::new();
    let (mut ob, mut nb) = (0usize, 0usize); // next unpaired old / new line
    let mut hunks: Vec<(std::ops::Range<usize>, std::ops::Range<usize>)> = diff
        .hunks()
        .map(|h| (h.before.start as usize..h.before.end as usize, h.after.start as usize..h.after.end as usize))
        .collect();
    // A sentinel "hunk" at the end closes the last unchanged run.
    hunks.push((old.line_count()..old.line_count(), new.line_count()..new.line_count()));
    for (hb, ha) in hunks {
        // Unchanged lines before this hunk map 1:1, including their newlines.
        debug_assert_eq!(hb.start - ob, ha.start - nb, "unchanged runs pair up");
        if hb.start > ob {
            push(
                &mut segs,
                Segment {
                    old: old.start_of(ob),
                    new: new.start_of(nb),
                    len: old.start_of(hb.start) - old.start_of(ob),
                },
            );
        }
        // 2. Inside the hunk: token diff (nothing survives a pure insertion / removal).
        if !hb.is_empty() && !ha.is_empty() {
            let ot = tokens(old, hb.clone());
            let nt = tokens(new, ha.clone());
            for (oi, ni, len) in equal_runs(&ot, &nt) {
                for k in 0..len {
                    let (a, b) = (&ot[oi + k], &nt[ni + k]);
                    push(&mut segs, Segment { old: a.start, new: b.start, len: a.len });
                }
            }
        }
        ob = hb.end;
        nb = ha.end;
    }
    segs
}

/// `(old token index, new token index, run length)` of equal token runs.
fn equal_runs(ot: &[Tok<'_>], nt: &[Tok<'_>]) -> Vec<(usize, usize, usize)> {
    let os: Vec<&str> = ot.iter().map(|t| t.text).collect();
    let ns: Vec<&str> = nt.iter().map(|t| t.text).collect();
    equal_str_runs(&os, &ns)
}

/// [`equal_runs`] on plain token texts (ticket 17's containment check diffs tokens it cut
/// itself): minimal Myers up to [`MAX_HUNK_TOKENS`], the bounded heuristic Myers above.
pub(crate) fn equal_str_runs(os: &[&str], ns: &[&str]) -> Vec<(usize, usize, usize)> {
    if os.len() + ns.len() <= MAX_HUNK_TOKENS {
        return similar::capture_diff_slices(similar::Algorithm::Myers, os, ns)
            .into_iter()
            .filter_map(|op| match op {
                similar::DiffOp::Equal { old_index, new_index, len } => Some((old_index, new_index, len)),
                _ => None,
            })
            .collect();
    }
    // Oversized hunk: the heuristic (bounded) Myers of imara-diff on the same tokens.
    let mut interner: Interner<&str> = Interner::new(os.len() + ns.len());
    let before: Vec<_> = os.iter().map(|s| interner.intern(s)).collect();
    let after: Vec<_> = ns.iter().map(|s| interner.intern(s)).collect();
    let mut diff = Diff::default();
    diff.compute_with(Algorithm::Myers, &before, &after, interner.num_tokens());
    let (mut oi, mut ni) = (0usize, 0usize);
    let mut out = Vec::new();
    let mut hunks: Vec<(usize, usize, usize, usize)> = diff
        .hunks()
        .map(|h| (h.before.start as usize, h.before.end as usize, h.after.start as usize, h.after.end as usize))
        .collect();
    hunks.push((os.len(), os.len(), ns.len(), ns.len()));
    for (bs, be, _, ae) in hunks {
        if bs > oi {
            out.push((oi, ni, bs - oi));
        }
        oi = be;
        ni = ae;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(old: &str, new: &str) -> Vec<Segment> {
        segments(&Text::new(old), &Text::new(new))
    }

    #[test]
    fn unchanged_lines_shift_and_changed_lines_map_by_word() {
        // A line inserted at the top shifts everything below by its length.
        let s = map("a\nb b\nc\n", "top\na\nb b\nc\n");
        assert_eq!(s, vec![Segment { old: 0, new: 4, len: 8 }]);
        // A changed word inside a line: the words around it survive, the word does not.
        let s = map("invite three external developers", "invite five external developers");
        // "invite " survives, "three" → "five" does not, " external developers\n" survives.
        assert_eq!(s, vec![Segment { old: 0, new: 0, len: 7 }, Segment { old: 12, new: 11, len: 21 }]);
        // Nothing survives a pure removal or a pure insertion hunk.
        assert!(map("gone\n", "").is_empty());
        assert_eq!(map("", "new\n"), vec![]);
        // Punctuation and spaces are their own tokens; CJK ideographs one each.
        let s = map("日本語の文章。", "日本語の別の文章。");
        assert_eq!(s.iter().map(|s| s.len).sum::<usize>(), 8, "日本語の + 文章。 + newline");
        // Determinism: same inputs, same segments.
        assert_eq!(map("x y z\nq\n", "x Y z\nq\n"), map("x y z\nq\n", "x Y z\nq\n"));
    }
}
