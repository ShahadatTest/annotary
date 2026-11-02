//! annotary diff / re-anchoring (ticket 07): the pure, deterministic function of
//! (text N, text N+1, anchors) that places every Thread on a new Version. The CLI runs it in
//! `push` (compute at the edge, ADR 0001) and submits the placements with the commit; the server
//! runs the same function to place Threads a client left out and to spot-verify what it was
//! sent; Phase 2's browser push runs it compiled to WASM. Same inputs ⇒ same output, everywhere.
//!
//! **Algorithm** (Anchor model): a histogram line diff of N vs N+1 maps the span's lines; inside
//! changed lines a Myers diff of the words maps the columns through the edit
//! ([`mapping`]). The mapping is *sticky inside, non-sticky at the edges*: the new span runs
//! from the first surviving character of the old span to the last surviving one, so text
//! inserted or replaced strictly inside is covered (the span grows — D5, G6), deletions
//! inside shrink it (D2–D4), and an insertion at either boundary falls outside (D6, D7).
//! If nothing survives — or no *word* of the quote does: whitespace, punctuation and bare
//! digits are not evidence (D8 type-over, D10' find-and-replace that swallows the span,
//! G4/G10/G11 lines gone, rewritten or moved) — the Thread is **orphaned**; else if the text
//! now under the span equals the frozen `original.exact` it is **anchored** (it may have
//! shifted — G1, G3, and whitespace-only line changes, the G12 divergence); otherwise
//! **refined**, carrying the current text. Orphaned is sticky: an already-orphaned Thread is
//! not looked at again (G8, no revival).
//!
//! Coordinates are the Sidecar's ([`annotary_sidecar::range`]): 1-based lines, 0-based
//! code-point columns, half-open, CRLF tolerated. Asset Threads anchor by content hash and are
//! not a diff problem: [`asset_status`].
//!
//! Ticket 17 adds [`contains_changes`] ([`contain`]): does a local file already contain the
//! changes the head made since the pusher's base? Same line diff and word tokens.

pub mod contain;
pub mod mapping;
mod text;

use annotary_sidecar::range::RangeError;
pub use annotary_sidecar::range::{self, Range};

pub use crate::contain::{
    AssetChange, AssetChangeKind, Containment, Hunk, contains_changes, format_unified, missing_asset_changes,
};
use crate::mapping::{Segment, segments};
use crate::text::Text;

/// Anchor placement status; exactly three (Anchor model).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Status {
    /// The span is found and its text equals the original quote (it may have shifted).
    Anchored,
    /// The span survives but the text under it changed (grew, shrank, edited).
    Refined,
    /// Nothing of the span survives; the last valid placement is kept by the caller.
    Orphaned,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::Anchored, Status::Refined, Status::Orphaned];

    pub fn as_str(self) -> &'static str {
        match self {
            Status::Anchored => "anchored",
            Status::Refined => "refined",
            Status::Orphaned => "orphaned",
        }
    }

    pub fn parse(s: &str) -> Option<Status> {
        Status::ALL.into_iter().find(|st| st.as_str() == s)
    }
}

impl std::fmt::Display for Status {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A span Thread as it stands on text N: the input of one re-anchoring step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Span {
    /// Where the span is on text N (its current placement — the last valid range if orphaned).
    pub range: Range,
    /// The frozen quote (`anchor.original.exact`) — what "anchored" is measured against.
    pub original: String,
    /// Already orphaned on N: stays orphaned, nothing is computed (sticky).
    pub orphaned: bool,
}

/// Where a span Thread stands on text N+1.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    pub status: Status,
    /// `Some` for anchored / refined; `None` when orphaned (keep the last valid range).
    pub range: Option<Range>,
    /// The text now under a refined span; `None` when anchored (it equals the original) or
    /// orphaned.
    pub exact: Option<String>,
}

impl Placement {
    pub const ORPHANED: Placement = Placement { status: Status::Orphaned, range: None, exact: None };
}

/// The computed mapping N → N+1, reusable for every Thread of the Document.
pub struct Mapping<'a> {
    old: Text<'a>,
    new: Text<'a>,
    segs: Vec<Segment>,
}

impl<'a> Mapping<'a> {
    /// Diff `text_n` against `text_n1` (both the raw Markdown; CRLF tolerated).
    pub fn new(text_n: &'a str, text_n1: &'a str) -> Mapping<'a> {
        let old = Text::new(text_n);
        let new = Text::new(text_n1);
        let segs = segments(&old, &new);
        Mapping { old, new, segs }
    }

    /// Place one span. `Err` only when `span.range` is not on text N at all (a corrupt anchor).
    pub fn place(&self, span: &Span) -> Result<Placement, RangeError> {
        if span.orphaned {
            return Ok(Placement::ORPHANED);
        }
        let (s, e) = self.old.span(&span.range)?;
        if s >= e {
            return Ok(Placement::ORPHANED); // zero-width: nothing to survive
        }
        // What counts as surviving is as strict as the quote allows: a letter when the quote has
        // letters (digits, punctuation and spaces alone are not evidence — list markers and line
        // labels survive every rewrite), a digit when it has only digits, any non-blank
        // character when it has neither (a `---` rule, an emoji).
        let significant: fn(&char) -> bool = if span.original.chars().any(char::is_alphabetic) {
            |c| c.is_alphabetic()
        } else if span.original.chars().any(char::is_alphanumeric) {
            |c| c.is_alphanumeric()
        } else {
            |c| !c.is_whitespace()
        };
        let first = self.segs.partition_point(|g| g.old_end() <= s);
        let (mut new_s, mut new_e, mut survives) = (usize::MAX, 0usize, false);
        for g in &self.segs[first..] {
            if g.old >= e {
                break;
            }
            let os = s.max(g.old);
            let oe = e.min(g.old_end());
            let ns = g.new + (os - g.old);
            new_s = new_s.min(ns);
            new_e = new_e.max(ns + (oe - os));
            survives |= self.old.chars[os..oe].iter().any(significant);
        }
        if !survives {
            return Ok(Placement::ORPHANED);
        }
        // Never gain leading/trailing whitespace the quote does not have.
        if !span.original.starts_with(char::is_whitespace) {
            while new_s < new_e && self.new.chars[new_s].is_whitespace() {
                new_s += 1;
            }
        }
        if !span.original.ends_with(char::is_whitespace) {
            while new_e > new_s && self.new.chars[new_e - 1].is_whitespace() {
                new_e -= 1;
            }
        }
        let exact = self.new.slice(new_s, new_e);
        let range = Some(self.new.range(new_s, new_e));
        Ok(if exact == span.original {
            Placement { status: Status::Anchored, range, exact: None }
        } else {
            Placement { status: Status::Refined, range, exact: Some(exact) }
        })
    }
}

/// Re-anchor every span from text N to text N+1 (one mapping, one placement per span, same
/// order). See [`Mapping::place`] for the error case.
pub fn reanchor(text_n: &str, text_n1: &str, spans: &[Span]) -> Result<Vec<Placement>, RangeError> {
    let m = Mapping::new(text_n, text_n1);
    spans.iter().map(|s| m.place(s)).collect()
}

/// An Asset Thread on the new Version: anchored iff its content hash is still referenced
/// (`present`); orphaned otherwise, and sticky once orphaned.
pub fn asset_status(already_orphaned: bool, present: bool) -> Status {
    match (already_orphaned, present) {
        (true, _) => Status::Orphaned,
        (false, true) => Status::Anchored,
        (false, false) => Status::Orphaned,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span(range: &str, original: &str) -> Span {
        Span { range: Range::parse(range).unwrap(), original: original.into(), orphaned: false }
    }

    fn place(old: &str, new: &str, s: Span) -> Placement {
        reanchor(old, new, &[s]).unwrap().remove(0)
    }

    // spec: S07-01, S07-02, S07-03
    #[test]
    fn the_three_statuses_and_the_edge_rules() {
        let doc = "Xxx: aa bb cc dd ee ff gg.\n";
        let s = span("1:8-1:16", "bb cc dd");
        assert_eq!(range::extract(doc, &s.range).unwrap(), "bb cc dd");
        // Untouched ⇒ anchored, same range; shifted by a line above ⇒ anchored, new range.
        assert_eq!(
            place(doc, doc, s.clone()),
            Placement { status: Status::Anchored, range: Some(s.range), exact: None }
        );
        let p = place(doc, "New first line\nXxx: aa bb cc dd ee ff gg.\n", s.clone());
        assert_eq!((p.status, p.range.unwrap().to_string()), (Status::Anchored, "2:8-2:16".into()));
        // Delete the middle ⇒ shrinks, refined (D3); insert inside ⇒ grows, refined (D5).
        let p = place(doc, "Xxx: aa bb dd ee ff gg.\n", s.clone());
        assert_eq!((p.status, p.exact.as_deref()), (Status::Refined, Some("bb dd")));
        let p = place(doc, "Xxx: aa bb cc XX dd ee ff gg.\n", s.clone());
        assert_eq!((p.status, p.exact.as_deref()), (Status::Refined, Some("bb cc XX dd")));
        // Insertions at the boundaries fall outside (D6, D7).
        let p = place(doc, "Xxx: aa ZZ bb cc dd YY ee ff gg.\n", s.clone());
        assert_eq!((p.status, p.range.unwrap().to_string()), (Status::Anchored, "1:11-1:19".into()));
        // Type-over ⇒ orphaned (D8), whatever the replacement; whole span deleted (D1) too.
        assert_eq!(place(doc, "Xxx: aa QQ QQ QQ ee ff gg.\n", s.clone()).status, Status::Orphaned);
        assert_eq!(place(doc, "Xxx: aa be cool dude ee ff gg.\n", s.clone()).status, Status::Orphaned);
        assert_eq!(place(doc, "Xxx: aa ee ff gg.\n", s.clone()).status, Status::Orphaned);
        // A changed word under the span ⇒ refined with the current text; a revert ⇒ anchored again.
        let s3 = span("1:0-1:32", "invite three external developers");
        let p = place("invite three external developers", "invite five external developers", s3.clone());
        assert_eq!(
            p,
            Placement {
                status: Status::Refined,
                range: Some(Range::new(1, 0, 1, 31)),
                exact: Some("invite five external developers".into())
            }
        );
        let back = Span { range: p.range.unwrap(), original: s3.original.clone(), orphaned: false };
        assert_eq!(
            place("invite five external developers", "invite three external developers", back).status,
            Status::Anchored
        );
        // Orphaned is sticky, and a corrupt range is an error rather than a guess.
        let o = Span { orphaned: true, ..s.clone() };
        assert_eq!(place(doc, doc, o), Placement::ORPHANED);
        assert!(reanchor(doc, doc, &[span("9:0-9:2", "x")]).is_err());
        assert_eq!(place(doc, doc, span("1:8-1:8", "")), Placement::ORPHANED, "zero-width");
        // A quote without word characters still anchors while it is unchanged.
        let hr = span("1:0-1:3", "---");
        assert_eq!(place("---\n\ntext\n", "---\n\ntext changed\n", hr).status, Status::Anchored);
        // Statuses print and parse.
        for st in Status::ALL {
            assert_eq!(Status::parse(st.as_str()), Some(st));
        }
        assert_eq!(asset_status(false, true), Status::Anchored);
        assert_eq!(asset_status(false, false), Status::Orphaned);
        assert_eq!(asset_status(true, true), Status::Orphaned);
    }
}
