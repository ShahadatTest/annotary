//! The flat text model behind the mapping: a Version's Markdown as one `Vec<char>` in which every
//! source line is followed by `\n` (a final newline in the file never changes coordinates, and
//! CRLF is tolerated — [`annotary_sidecar::range::lines`] semantics), plus the start offset of
//! every line, so `L:C` source coordinates and flat offsets convert both ways.

use annotary_sidecar::range::{Range, RangeError, lines};

/// One text, flattened.
pub(crate) struct Text<'a> {
    /// The source lines (no line terminators).
    pub lines: Vec<&'a str>,
    /// Every line's characters followed by `\n`.
    pub chars: Vec<char>,
    /// `line_starts[i]` = flat offset of line `i` (0-based); `line_starts[lines.len()]` = total.
    pub line_starts: Vec<usize>,
}

impl<'a> Text<'a> {
    pub fn new(text: &'a str) -> Text<'a> {
        let lines = lines(text);
        let mut chars = Vec::with_capacity(text.len() + 1);
        let mut line_starts = Vec::with_capacity(lines.len() + 1);
        for l in &lines {
            line_starts.push(chars.len());
            chars.extend(l.chars());
            chars.push('\n');
        }
        line_starts.push(chars.len());
        Text { lines, chars, line_starts }
    }

    pub fn line_count(&self) -> usize {
        self.lines.len()
    }

    /// Flat offset of line `i` (0-based).
    pub fn start_of(&self, i: usize) -> usize {
        self.line_starts[i]
    }

    /// Length in code points of line `i` (0-based), without the `\n`.
    pub fn len_of(&self, i: usize) -> usize {
        self.line_starts[i + 1] - self.line_starts[i] - 1
    }

    /// `(line, col)` (1-based line, 0-based col) → flat offset, checked against the text.
    pub fn flat(&self, line: u32, col: u32) -> Result<usize, RangeError> {
        let n = self.line_count() as u32;
        if line == 0 || line > n {
            return Err(RangeError::LineOutOfBounds { line, lines: n });
        }
        let i = (line - 1) as usize;
        let len = self.len_of(i) as u32;
        if col > len {
            return Err(RangeError::ColOutOfBounds { line, col, len });
        }
        Ok(self.start_of(i) + col as usize)
    }

    /// A half-open range → `[start, end)` flat offsets.
    pub fn span(&self, r: &Range) -> Result<(usize, usize), RangeError> {
        let s = self.flat(r.start_line, r.start_col)?;
        let e = self.flat(r.end_line, r.end_col)?;
        Ok((s, e))
    }

    /// Flat offset → `(line, col)` (1-based line, 0-based col). An offset on a line's `\n` is
    /// that line's end column; the offset one past the whole text is the end of the last line.
    pub fn position(&self, flat: usize) -> (u32, u32) {
        if self.lines.is_empty() {
            return (1, 0);
        }
        // The last line whose start is ≤ flat.
        let i = match self.line_starts.binary_search(&flat) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        let i = i.min(self.line_count() - 1);
        let col = (flat - self.start_of(i)).min(self.len_of(i));
        (i as u32 + 1, col as u32)
    }

    pub fn range(&self, start: usize, end: usize) -> Range {
        let (sl, sc) = self.position(start);
        let (el, ec) = self.position(end);
        Range::new(sl, sc, el, ec)
    }

    pub fn slice(&self, start: usize, end: usize) -> String {
        self.chars[start..end].iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_and_back_agree_with_range_semantics() {
        let t = Text::new("ab\r\ncde\n\nx");
        assert_eq!(t.lines, vec!["ab", "cde", "", "x"]);
        assert_eq!(t.chars.iter().collect::<String>(), "ab\ncde\n\nx\n");
        assert_eq!(t.line_starts, vec![0, 3, 7, 8, 10]);
        assert_eq!(t.flat(1, 0).unwrap(), 0);
        assert_eq!(t.flat(1, 2).unwrap(), 2, "end col == line length is the line's end");
        assert!(matches!(t.flat(1, 3), Err(RangeError::ColOutOfBounds { line: 1, col: 3, len: 2 })));
        assert!(matches!(t.flat(5, 0), Err(RangeError::LineOutOfBounds { line: 5, lines: 4 })));
        assert_eq!(t.flat(4, 1).unwrap(), 9);
        assert_eq!(t.position(0), (1, 0));
        assert_eq!(t.position(2), (1, 2));
        assert_eq!(t.position(3), (2, 0));
        assert_eq!(t.position(6), (2, 3));
        assert_eq!(t.position(7), (3, 0));
        assert_eq!(t.position(9), (4, 1));
        assert_eq!(t.position(10), (4, 1), "one past the end = end of the last line");
        let r = Range::parse("1:1-2:2").unwrap();
        assert_eq!(t.span(&r).unwrap(), (1, 5));
        assert_eq!(t.slice(1, 5), "b\ncd");
        assert_eq!(t.range(1, 5), r);
        assert_eq!(Text::new("").line_count(), 0);
        assert_eq!(Text::new("").position(0), (1, 0));
    }
}
