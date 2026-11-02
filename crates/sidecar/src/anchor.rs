//! Anchor bookkeeping shared by CLI, server and WASM (ticket 07): the **sparse history rule**
//! (spec 139 — an entry only where the placement changed; the placement at Version N is the
//! last entry with `version ≤ N`) and how a newly computed placement is recorded on push
//! (`current` rewritten, `history` appended when it changed, orphaned sticky).
//!
//! Shapes (Anchor model): span `original {version, range, exact}` (frozen) · asset `original
//! {version, hash, ref, line}`; `current {version, range|line, status, exact?}` — for an
//! anchored/refined placement `version` is the Version it is on and `exact` is present only when
//! refined (the text now under the span); for an orphaned one `current` keeps the **last valid**
//! `range|line` stamped with the last Version it was valid on and `status: orphaned`;
//! `history [{version, range|line, status}]` (an orphaned entry has no range/line).

use alloc::string::{String, ToString};
use alloc::vec;

use crate::model::{Anchor, Placement};

pub const ANCHORED: &str = "anchored";
pub const REFINED: &str = "refined";
pub const ORPHANED: &str = "orphaned";

impl Placement {
    /// `range` for spans, `line` for assets — the position part of a placement.
    pub fn position_key(&self) -> (Option<&str>, Option<u32>) {
        (self.range.as_deref(), self.line)
    }

    pub fn is_orphaned(&self) -> bool {
        self.status.as_deref() == Some(ORPHANED)
    }
}

impl Anchor {
    /// A span Anchor at creation on Version `version`: `original` frozen, `current` = the same
    /// range with `status: anchored`, one history entry.
    pub fn new_span(version: u32, range: &str, exact: &str) -> Anchor {
        let at = Placement {
            version,
            range: Some(range.to_string()),
            status: Some(ANCHORED.to_string()),
            ..Default::default()
        };
        Anchor {
            original: Placement {
                version,
                range: Some(range.to_string()),
                exact: Some(exact.to_string()),
                ..Default::default()
            },
            current: at.clone(),
            history: vec![at],
        }
    }

    /// An asset Anchor at creation: content `hash` + `ref` + the referencing `line` (display
    /// only; `None` when the reference is not in the root Markdown itself).
    pub fn new_asset(version: u32, hash: &str, reference: &str, line: Option<u32>) -> Anchor {
        let at = Placement { version, line, status: Some(ANCHORED.to_string()), ..Default::default() };
        Anchor {
            original: Placement {
                version,
                hash: Some(hash.to_string()),
                reference: Some(reference.to_string()),
                line,
                ..Default::default()
            },
            current: at.clone(),
            history: vec![at],
        }
    }

    pub fn is_asset(&self) -> bool {
        self.original.hash.is_some()
    }

    pub fn is_orphaned(&self) -> bool {
        self.current.is_orphaned()
    }

    /// The placement at Version `n` per the sparse rule: the last history entry with
    /// `version ≤ n`, stamped `n` (its range/line is what stood on every Version from the entry
    /// up to the next one); for an orphaned entry the last valid range/line stamped with the last
    /// Version it was valid on. `None` before the Thread existed. A refined placement comes
    /// back without `exact` (history does not carry text; the caller extracts it from Version
    /// `n` if it wants it).
    pub fn placement_at(&self, n: u32) -> Option<Placement> {
        let idx = self.history.iter().rposition(|h| h.version <= n)?;
        let e = &self.history[idx];
        if e.is_orphaned() {
            let last_valid = self.history[..idx].iter().rev().find(|h| !h.is_orphaned());
            let (range, line) = match last_valid {
                Some(v) => (v.range.clone(), v.line),
                None => (self.original.range.clone(), self.original.line),
            };
            return Some(Placement {
                version: e.version.saturating_sub(1).max(self.original.version),
                range,
                line,
                status: Some(ORPHANED.to_string()),
                ..Default::default()
            });
        }
        Some(Placement {
            version: n,
            range: e.range.clone(),
            line: e.line,
            status: e.status.clone(),
            ..Default::default()
        })
    }

    /// Record the placement computed for Version `n` (`status` one of the three; `range` for a
    /// span / `line` for an asset when not orphaned; `exact` = the text now under a refined
    /// span). Rewrites `current`; appends a history entry only when `(range|line, status)`
    /// differ from the last entry — orphaned is sticky, so once orphaned nothing changes again.
    /// Returns whether history grew.
    pub fn record(
        &mut self,
        n: u32,
        status: &str,
        range: Option<String>,
        line: Option<u32>,
        exact: Option<String>,
    ) -> bool {
        let orphaned = status == ORPHANED || self.is_orphaned();
        let entry = if orphaned {
            Placement { version: n, status: Some(ORPHANED.to_string()), ..Default::default() }
        } else {
            Placement { version: n, range, line, status: Some(status.to_string()), ..Default::default() }
        };
        let changed = match self.history.last() {
            Some(last) => last.position_key() != entry.position_key() || last.status != entry.status,
            None => true,
        };
        if orphaned {
            if !self.is_orphaned() {
                // Freeze the last valid placement, stamped with the last Version it was valid on.
                self.current = Placement {
                    version: n.saturating_sub(1).max(self.original.version),
                    range: self.current.range.clone(),
                    line: self.current.line,
                    status: Some(ORPHANED.to_string()),
                    ..Default::default()
                };
            }
        } else {
            self.current = Placement {
                version: n,
                range: entry.range.clone(),
                line: entry.line,
                status: Some(status.to_string()),
                exact: if status == REFINED { exact } else { None },
                ..Default::default()
            };
        }
        if changed {
            self.history.push(entry);
        }
        changed
    }

    /// The Anchor as it stood at Version `n`: `current` = [`Self::placement_at`], `history`
    /// cut to entries `≤ n`. `None` before the Thread existed.
    pub fn at(&self, n: u32) -> Option<Anchor> {
        let current = self.placement_at(n)?;
        Some(Anchor {
            original: self.original.clone(),
            current,
            history: self.history.iter().filter(|h| h.version <= n).cloned().collect(),
        })
    }
}

impl core::fmt::Display for Placement {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "v{}", self.version)?;
        if let Some(r) = &self.range {
            write!(f, ":{r}")?;
        } else if let Some(l) = self.line {
            write!(f, ":{l}")?;
        }
        if let Some(s) = &self.status {
            write!(f, " {s}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::ToString;

    use super::*;

    // spec: S07-08, S07-11
    #[test]
    fn sparse_history_records_only_changes_and_looks_up_by_last_entry() {
        let mut a = Anchor::new_span(1, "16:0-16:33", "We ship the CLI before the web UI");
        assert_eq!(a.history.len(), 1);
        assert!(!a.is_orphaned() && !a.is_asset());
        // Unchanged (same range, anchored) on v2 and v3: current moves, history does not.
        assert!(!a.record(2, ANCHORED, Some("16:0-16:33".into()), None, None));
        assert!(!a.record(3, ANCHORED, Some("16:0-16:33".into()), None, None));
        assert_eq!(a.current.version, 3);
        assert_eq!(a.history.len(), 1);
        // Shifted on v4, refined on v5 (with the current text), back to anchored on v6.
        assert!(a.record(4, ANCHORED, Some("18:0-18:33".into()), None, None));
        assert!(a.record(5, REFINED, Some("18:0-18:30".into()), None, Some("We ship the CLI before the UI".into())));
        assert_eq!(a.current.exact.as_deref(), Some("We ship the CLI before the UI"));
        assert!(a.record(6, ANCHORED, Some("18:0-18:33".into()), None, None));
        assert_eq!(a.current.exact, None, "current.exact only when refined");
        assert_eq!(a.history.iter().map(|h| h.version).collect::<alloc::vec::Vec<_>>(), vec![1, 4, 5, 6]);
        // Placement at N = last entry ≤ N, stamped N.
        assert_eq!(a.placement_at(0), None);
        assert_eq!(a.placement_at(2).unwrap().to_string(), "v2:16:0-16:33 anchored");
        assert_eq!(a.placement_at(3).unwrap().to_string(), "v3:16:0-16:33 anchored");
        assert_eq!(a.placement_at(4).unwrap().to_string(), "v4:18:0-18:33 anchored");
        assert_eq!(a.placement_at(5).unwrap().to_string(), "v5:18:0-18:30 refined");
        assert_eq!(a.placement_at(9).unwrap().to_string(), "v9:18:0-18:33 anchored");
        // Orphaned on v7: current freezes the last valid placement stamped v6; sticky after that.
        assert!(a.record(7, ORPHANED, None, None, None));
        assert_eq!(a.current.to_string(), "v6:18:0-18:33 orphaned");
        assert!(a.is_orphaned());
        assert!(!a.record(8, ANCHORED, Some("1:0-1:5".into()), None, None), "orphaned is sticky");
        assert_eq!(a.current.to_string(), "v6:18:0-18:33 orphaned");
        assert_eq!(a.history.last().unwrap().to_string(), "v7 orphaned");
        assert_eq!(a.placement_at(7).unwrap().to_string(), "v6:18:0-18:33 orphaned");
        assert_eq!(a.placement_at(8).unwrap().to_string(), "v6:18:0-18:33 orphaned");
        assert_eq!(a.placement_at(6).unwrap().to_string(), "v6:18:0-18:33 anchored");
        let at5 = a.at(5).unwrap();
        assert_eq!(at5.history.len(), 3);
        assert_eq!(at5.current.status.as_deref(), Some(REFINED));
        assert_eq!(at5.original, a.original, "original is immutable");
        // Assets: line instead of range; orphaned freezes the last line.
        let mut b = Anchor::new_asset(2, "8f18", "img/architecture.png", Some(3));
        assert!(b.is_asset());
        assert!(b.record(3, ANCHORED, None, Some(9), None));
        assert!(b.record(4, ORPHANED, None, None, None));
        assert_eq!(b.current.to_string(), "v3:9 orphaned");
        assert_eq!(b.placement_at(2).unwrap().to_string(), "v2:3 anchored");
        assert_eq!(b.placement_at(4).unwrap().to_string(), "v3:9 orphaned");
    }
}
