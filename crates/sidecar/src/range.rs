//! Source coordinates: the `"L:C-L:C"` range strings the CLI, Sidecar, server and web all use.
//!
//! Lines are 1-based, columns 0-based **Unicode code points**, ranges half-open (`end` is the
//! first position *not* covered), CRLF tolerated (a trailing `\r` on a line is not a column and
//! never part of a quote). Zero-width ranges parse and are valid coordinates; whether a verb
//! accepts them is the verb's decision (Phase 1 refuses them on `thread new`).

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::fmt;

/// A half-open span in source coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Range {
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
}

/// Why a range string or lookup failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RangeError {
    /// Not of the form `L:C-L:C` (with L ≥ 1).
    Malformed(String),
    /// `end` before `start`.
    Backwards(String),
    /// A line number past the last line of the text (`lines` = how many the text has).
    LineOutOfBounds { line: u32, lines: u32 },
    /// A column past the end of its line (`len` = the line's length in code points).
    ColOutOfBounds { line: u32, col: u32, len: u32 },
}

impl fmt::Display for RangeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RangeError::Malformed(s) => {
                write!(f, "range {s:?} is not L:C-L:C (lines from 1, columns from 0, e.g. \"16:0-16:33\")")
            }
            RangeError::Backwards(s) => write!(f, "range {s} ends before it starts"),
            RangeError::LineOutOfBounds { line, lines } => {
                write!(
                    f,
                    "line {line} is past the end of the text ({lines} line{})",
                    if *lines == 1 { "" } else { "s" }
                )
            }
            RangeError::ColOutOfBounds { line, col, len } => {
                write!(
                    f,
                    "column {col} is past the end of line {line} ({len} character{})",
                    if *len == 1 { "" } else { "s" }
                )
            }
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for RangeError {}

impl Range {
    pub fn new(start_line: u32, start_col: u32, end_line: u32, end_col: u32) -> Range {
        Range { start_line, start_col, end_line, end_col }
    }

    /// Parse `"L:C-L:C"`.
    pub fn parse(text: &str) -> Result<Range, RangeError> {
        let t = text.trim();
        let bad = || RangeError::Malformed(text.to_string());
        let (a, b) = t.split_once('-').ok_or_else(bad)?;
        let pos = |s: &str| -> Result<(u32, u32), RangeError> {
            let (l, c) = s.trim().split_once(':').ok_or_else(bad)?;
            let l: u32 = l.trim().parse().map_err(|_| bad())?;
            let c: u32 = c.trim().parse().map_err(|_| bad())?;
            if l == 0 {
                return Err(bad());
            }
            Ok((l, c))
        };
        let (start_line, start_col) = pos(a)?;
        let (end_line, end_col) = pos(b)?;
        let r = Range { start_line, start_col, end_line, end_col };
        if (end_line, end_col) < (start_line, start_col) {
            return Err(RangeError::Backwards(t.to_string()));
        }
        Ok(r)
    }

    /// True when `start == end`.
    pub fn is_empty(&self) -> bool {
        (self.start_line, self.start_col) == (self.end_line, self.end_col)
    }
}

impl fmt::Display for Range {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}-{}:{}", self.start_line, self.start_col, self.end_line, self.end_col)
    }
}

/// The lines of `text` in source coordinates: split on `\n`, a trailing `\r` stripped from each
/// (CRLF tolerated), and no phantom empty line after a final newline.
pub fn lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<&str> = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).collect();
    if text.ends_with('\n') {
        out.pop();
    }
    out
}

/// The text a range covers: characters `[start_col, end_col)` on one line, or from `start_col`
/// to the end of `start_line`, the whole lines between, and `[0, end_col)` of `end_line`, joined
/// with `\n` (whatever the source's line endings). Errors when a line or column is out of bounds.
pub fn extract(text: &str, r: &Range) -> Result<String, RangeError> {
    let ls = lines(text);
    let n = ls.len() as u32;
    let line = |l: u32| -> Result<Vec<char>, RangeError> {
        if l == 0 || l > n {
            return Err(RangeError::LineOutOfBounds { line: l, lines: n });
        }
        Ok(ls[(l - 1) as usize].chars().collect())
    };
    let check_col = |l: u32, c: u32, chars: &[char]| -> Result<(), RangeError> {
        if c as usize > chars.len() {
            Err(RangeError::ColOutOfBounds { line: l, col: c, len: chars.len() as u32 })
        } else {
            Ok(())
        }
    };
    let first = line(r.start_line)?;
    check_col(r.start_line, r.start_col, &first)?;
    if r.start_line == r.end_line {
        check_col(r.end_line, r.end_col, &first)?;
        return Ok(first[r.start_col as usize..r.end_col as usize].iter().collect());
    }
    let last = line(r.end_line)?;
    check_col(r.end_line, r.end_col, &last)?;
    let mut out: String = first[r.start_col as usize..].iter().collect();
    for l in r.start_line + 1..r.end_line {
        out.push('\n');
        out.push_str(ls[(l - 1) as usize]);
    }
    out.push('\n');
    out.extend(last[..r.end_col as usize].iter());
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: S04-05
    #[test]
    fn parses_and_formats_ranges() {
        let r = Range::parse("16:0-16:33").unwrap();
        assert_eq!(r, Range::new(16, 0, 16, 33));
        assert_eq!(r.to_string(), "16:0-16:33");
        assert_eq!(Range::parse(" 2:3 - 4:0 ").unwrap(), Range::new(2, 3, 4, 0));
        assert!(Range::parse("3:5-3:5").unwrap().is_empty());
        for bad in ["", "16", "16:0", "16:0-", "0:0-1:0", "a:b-c:d", "16:0-16", "1:0-2:x", "-1:0-2:0"] {
            assert!(matches!(Range::parse(bad), Err(RangeError::Malformed(_))), "{bad:?}");
        }
        assert!(matches!(Range::parse("3:4-3:2"), Err(RangeError::Backwards(_))));
        assert!(matches!(Range::parse("3:4-2:9"), Err(RangeError::Backwards(_))));
        assert!(!RangeError::Malformed("x".into()).to_string().is_empty());
    }

    // spec: S04-05
    #[test]
    fn extracts_code_points_half_open_and_tolerates_crlf() {
        let text = "# Title\n\nWe ship the CLI before the web UI so agents can start.\nÜber façade 😀 done\n";
        assert_eq!(lines(text).len(), 4, "no phantom line after the final newline");
        assert_eq!(extract(text, &Range::parse("3:0-3:33").unwrap()).unwrap(), "We ship the CLI before the web UI");
        // Columns are code points, not bytes: `Ü`, `ç` and the emoji each count once.
        assert_eq!(extract(text, &Range::parse("4:0-4:4").unwrap()).unwrap(), "Über");
        assert_eq!(extract(text, &Range::parse("4:5-4:11").unwrap()).unwrap(), "façade");
        assert_eq!(extract(text, &Range::parse("4:12-4:13").unwrap()).unwrap(), "😀");
        assert_eq!(extract(text, &Range::parse("4:14-4:18").unwrap()).unwrap(), "done");
        // Half-open: end col == line length is the end of the line; one past is out of bounds.
        assert!(matches!(
            extract(text, &Range::parse("4:14-4:19").unwrap()),
            Err(RangeError::ColOutOfBounds { line: 4, col: 19, len: 18 })
        ));
        assert!(matches!(
            extract(text, &Range::parse("5:0-5:1").unwrap()),
            Err(RangeError::LineOutOfBounds { line: 5, lines: 4 })
        ));
        assert_eq!(extract(text, &Range::parse("2:0-2:0").unwrap()).unwrap(), "", "empty line, zero-width");
        // Multi-line spans join with `\n`, whatever the source's line endings.
        assert_eq!(extract(text, &Range::parse("1:2-3:2").unwrap()).unwrap(), "Title\n\nWe");
        let crlf = text.replace('\n', "\r\n");
        assert_eq!(extract(&crlf, &Range::parse("3:0-3:33").unwrap()).unwrap(), "We ship the CLI before the web UI");
        assert_eq!(extract(&crlf, &Range::parse("1:2-3:2").unwrap()).unwrap(), "Title\n\nWe");
        assert_eq!(extract(&crlf, &Range::parse("1:0-1:7").unwrap()).unwrap(), "# Title", "the \\r is not a column");
        assert!(matches!(
            extract(&crlf, &Range::parse("1:0-1:8").unwrap()),
            Err(RangeError::ColOutOfBounds { line: 1, col: 8, len: 7 })
        ));
        // A text without a final newline still has its last line.
        assert_eq!(extract("a\nbc", &Range::parse("2:0-2:2").unwrap()).unwrap(), "bc");
        assert_eq!(lines("").len(), 0);
    }
}
