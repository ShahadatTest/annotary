//! Splicing Thread strips into a rendered Version's HTML (ticket 10) so that every Thread is in
//! the **first** HTML response at the block it anchors to — no post-load fetch, no layout shift.
//!
//! The renderer's output is regular (`docs/adr/0001`, docs/dev/rendering.md): every block it
//! opens carries `data-l="A-B"` (1-based, inclusive source lines), text and attributes are
//! escaped, tags are lowercase. [`scan`] turns that into a flat list of blocks with their byte
//! ranges and parents (tolerant of raw HTML the sanitizer let through: tags without `data-l` are
//! ignored, a stray close tag is skipped, unclosed elements close at the end). [`splice`] then
//! inserts a strip for a source line at the deepest block covering it, at a place where flow
//! content is valid: *inside* (before the close tag of) a `li` / `td` / `th` / `blockquote` /
//! `dd` / `dt` / `section`, *after* a leaf block (`p`, headings, `pre`, `hr`, `table`, lists,
//! `dl`, block `math`) whose parent allows it — else up one level; a line no block covers goes
//! after the last top-level block before it. The Document-level strip goes under the title (after
//! a leading `h1`, else first); the tail (the orphaned group) goes last.

use std::collections::BTreeMap;

/// One block-level element with `data-l` in a rendered fragment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub name: String,
    /// Source lines `a..=b`.
    pub a: u32,
    pub b: u32,
    /// Byte offsets of the open tag and the close tag (`close_start == close_end` when the
    /// element was never closed — a void element or an unclosed one).
    pub open_start: usize,
    pub open_end: usize,
    pub close_start: usize,
    pub close_end: usize,
    /// Index of the nearest enclosing block (`None` = top level).
    pub parent: Option<usize>,
}

impl Block {
    pub fn covers(&self, line: u32) -> bool {
        self.a <= line && line <= self.b
    }
    fn is_void(&self) -> bool {
        self.close_start == self.close_end
    }
}

/// Elements that never have a close tag.
const VOID: &[&str] =
    &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

/// Containers a strip may be appended *inside* (flow content is valid there).
const INSIDE: &[&str] = &["li", "td", "th", "blockquote", "dd", "dt", "section"];
/// Leaf blocks a strip goes *after* — when the parent allows flow content there.
const AFTER: &[&str] =
    &["p", "h1", "h2", "h3", "h4", "h5", "h6", "pre", "hr", "table", "ul", "ol", "dl", "math", "div", "figure"];

fn parse_l(v: &str) -> Option<(u32, u32)> {
    let (a, b) = v.split_once('-')?;
    let a: u32 = a.trim().parse().ok()?;
    let b: u32 = b.trim().parse().ok()?;
    (a >= 1 && b >= a).then_some((a, b))
}

struct Open {
    name: String,
    block: Option<usize>,
}

/// Scan a rendered fragment into its `data-l` blocks (document order).
pub fn scan(html: &str) -> Vec<Block> {
    let b = html.as_bytes();
    let n = b.len();
    let mut blocks: Vec<Block> = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut i = 0;
    let is_name = |c: u8| c.is_ascii_alphanumeric() || c == b'-';
    while i < n {
        if b[i] != b'<' {
            i += 1;
            continue;
        }
        if html[i..].starts_with("<!--") {
            i = html[i..].find("-->").map(|p| i + p + 3).unwrap_or(n);
            continue;
        }
        if i + 1 < n && b[i + 1] == b'/' {
            // A close tag.
            let ns = i + 2;
            let mut ne = ns;
            while ne < n && is_name(b[ne]) {
                ne += 1;
            }
            let name = html[ns..ne].to_ascii_lowercase();
            let end = match html[ne..].find('>') {
                Some(p) => ne + p + 1,
                None => n,
            };
            if let Some(pos) = stack.iter().rposition(|o| o.name == name) {
                while stack.len() > pos + 1 {
                    if let Some(bi) = stack.pop().and_then(|o| o.block) {
                        blocks[bi].close_start = i;
                        blocks[bi].close_end = i;
                    }
                }
                if let Some(bi) = stack.pop().and_then(|o| o.block) {
                    blocks[bi].close_start = i;
                    blocks[bi].close_end = end;
                }
            }
            i = end;
            continue;
        }
        // An open tag?
        let ns = i + 1;
        let mut ne = ns;
        while ne < n && is_name(b[ne]) {
            ne += 1;
        }
        if ne == ns || !b[ns].is_ascii_alphabetic() {
            i += 1;
            continue;
        }
        let name = html[ns..ne].to_ascii_lowercase();
        // Attributes, honouring quotes, until `>` or `/>`.
        let mut j = ne;
        let mut data_l = None;
        let mut self_closing = false;
        loop {
            while j < n && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j >= n {
                break;
            }
            if b[j] == b'>' {
                j += 1;
                break;
            }
            if b[j] == b'/' {
                if j + 1 < n && b[j + 1] == b'>' {
                    self_closing = true;
                    j += 2;
                    break;
                }
                j += 1;
                continue;
            }
            let an_s = j;
            while j < n && !b[j].is_ascii_whitespace() && b[j] != b'=' && b[j] != b'>' && b[j] != b'/' {
                j += 1;
            }
            let an = html[an_s..j].to_ascii_lowercase();
            while j < n && b[j].is_ascii_whitespace() {
                j += 1;
            }
            let mut val: Option<&str> = None;
            if j < n && b[j] == b'=' {
                j += 1;
                while j < n && b[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < n && (b[j] == b'"' || b[j] == b'\'') {
                    let q = b[j];
                    j += 1;
                    let vs = j;
                    while j < n && b[j] != q {
                        j += 1;
                    }
                    val = Some(&html[vs..j.min(n)]);
                    if j < n {
                        j += 1;
                    }
                } else {
                    let vs = j;
                    while j < n && !b[j].is_ascii_whitespace() && b[j] != b'>' {
                        j += 1;
                    }
                    val = Some(&html[vs..j]);
                }
            }
            if an == "data-l" {
                data_l = val.and_then(parse_l);
            }
        }
        let open_end = j;
        let void = VOID.contains(&name.as_str()) || self_closing;
        let block = data_l.map(|(a, bb)| {
            let parent = stack.iter().rev().find_map(|o| o.block);
            blocks.push(Block {
                name: name.clone(),
                a,
                b: bb,
                open_start: i,
                open_end,
                close_start: open_end,
                close_end: open_end,
                parent,
            });
            blocks.len() - 1
        });
        if !void {
            stack.push(Open { name, block });
        }
        i = open_end;
    }
    // Unclosed elements close at the end of the fragment.
    while let Some(o) = stack.pop() {
        if let Some(bi) = o.block {
            blocks[bi].close_start = n;
            blocks[bi].close_end = n;
        }
    }
    blocks
}

/// Where a strip for `line` goes: the byte offset to insert at (see the module docs).
pub fn insertion_point(html_len: usize, blocks: &[Block], line: u32) -> usize {
    // Deepest block covering the line = the last one in document order that covers it (children
    // follow their parents).
    let mut cur = blocks.iter().rposition(|bl| bl.covers(line));
    while let Some(i) = cur {
        let bl = &blocks[i];
        let name = bl.name.as_str();
        if INSIDE.contains(&name) && !bl.is_void() {
            return bl.close_start;
        }
        if AFTER.contains(&name) {
            let parent_ok = match bl.parent {
                None => true,
                Some(p) => INSIDE.contains(&blocks[p].name.as_str()),
            };
            if parent_ok {
                return bl.close_end;
            }
        }
        cur = bl.parent;
    }
    // No block covers it: after the last top-level block that ends before it, else first.
    blocks
        .iter()
        .filter(|bl| bl.parent.is_none() && bl.b < line)
        .map(|bl| bl.close_end)
        .next_back()
        .unwrap_or(0)
        .min(html_len)
}

/// Where the Document-level strip goes: under the title — after a leading `h1`, else at the top.
pub fn title_point(blocks: &[Block]) -> usize {
    match blocks.first() {
        Some(bl) if bl.parent.is_none() && bl.name == "h1" => bl.close_end,
        _ => 0,
    }
}

/// Insert strips into a rendered fragment: `head` under the title, one strip per `(line, html)`
/// at its line (strips at the same point keep their order), `tail` at the end.
pub fn splice(html: &str, head: Option<&str>, strips: &[(u32, String)], tail: Option<&str>) -> String {
    if head.is_none() && strips.is_empty() && tail.is_none() {
        return html.to_string();
    }
    let blocks = scan(html);
    let mut at: BTreeMap<usize, Vec<&str>> = BTreeMap::new();
    if let Some(h) = head {
        at.entry(title_point(&blocks)).or_default().push(h);
    }
    for (line, s) in strips {
        at.entry(insertion_point(html.len(), &blocks, *line)).or_default().push(s);
    }
    if let Some(t) = tail {
        at.entry(html.len()).or_default().push(t);
    }
    let extra: usize = at.values().flatten().map(|s| s.len()).sum();
    let mut out = String::with_capacity(html.len() + extra + 16);
    let mut last = 0;
    for (off, pieces) in at {
        let off = off.min(html.len());
        out.push_str(&html[last..off]);
        for p in pieces {
            out.push_str(p);
        }
        last = off;
    }
    out.push_str(&html[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOC: &str = "<h1 data-l=\"1-1\" id=\"t\">Title</h1>\n<p data-l=\"3-4\">one\ntwo</p>\n<ul data-l=\"6-8\">\n<li data-l=\"6-6\">a</li>\n<li data-l=\"7-8\">b\n<ul data-l=\"8-8\">\n<li data-l=\"8-8\">c</li>\n</ul>\n</li>\n</ul>\n<hr data-l=\"10-10\">\n<table data-l=\"12-13\">\n<thead data-l=\"12-12\">\n<tr data-l=\"12-12\"><th data-l=\"12-12\">h</th></tr>\n</thead>\n<tbody data-l=\"13-13\">\n<tr data-l=\"13-13\"><td data-l=\"13-13\">d</td></tr>\n</tbody>\n</table>\n<blockquote data-l=\"15-15\">\n<p data-l=\"15-15\">q</p>\n</blockquote>\n<pre data-l=\"17-19\"><code>x &lt;y&gt;\n</code></pre>\n";

    #[test]
    fn scans_blocks_with_parents() {
        let b = scan(DOC);
        let names: Vec<&str> = b.iter().map(|x| x.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "h1",
                "p",
                "ul",
                "li",
                "li",
                "ul",
                "li",
                "hr",
                "table",
                "thead",
                "tr",
                "th",
                "tbody",
                "tr",
                "td",
                "blockquote",
                "p",
                "pre"
            ]
        );
        let li_c = b.iter().find(|x| x.name == "li" && x.a == 8).unwrap();
        assert_eq!(b[li_c.parent.unwrap()].name, "ul");
        assert_eq!(b[b[li_c.parent.unwrap()].parent.unwrap()].name, "li");
        let hr = b.iter().find(|x| x.name == "hr").unwrap();
        assert!(hr.is_void());
        assert_eq!(&DOC[hr.open_start..hr.open_end], "<hr data-l=\"10-10\">");
        let pre = b.iter().find(|x| x.name == "pre").unwrap();
        assert_eq!(&DOC[pre.close_start..pre.close_end], "</pre>");
    }

    #[test]
    fn strips_land_where_flow_content_is_valid() {
        let strip = |l: u32| (l, format!("<div class=\"s\">{l}</div>"));
        let out = splice(
            DOC,
            Some("<div class=\"doc\"></div>"),
            &[strip(3), strip(6), strip(8), strip(12), strip(13), strip(15), strip(18), strip(10)],
            Some("<div class=\"tail\"></div>"),
        );
        // Under the title.
        assert!(out.starts_with("<h1 data-l=\"1-1\" id=\"t\">Title</h1><div class=\"doc\"></div>\n<p"));
        // After the paragraph.
        assert!(out.contains("two</p><div class=\"s\">3</div>\n<ul"));
        // Inside the list items (never between them).
        assert!(out.contains("<li data-l=\"6-6\">a<div class=\"s\">6</div></li>"));
        assert!(out.contains("<li data-l=\"8-8\">c<div class=\"s\">8</div></li>"));
        // Inside table cells.
        assert!(out.contains("<th data-l=\"12-12\">h<div class=\"s\">12</div></th>"));
        assert!(out.contains("<td data-l=\"13-13\">d<div class=\"s\">13</div></td>"));
        // After the paragraph inside the blockquote.
        assert!(out.contains("<p data-l=\"15-15\">q</p><div class=\"s\">15</div>\n</blockquote>"));
        // After the code block (a leaf), after the hr (void).
        assert!(out.contains("</code></pre><div class=\"s\">18</div>"));
        assert!(out.contains("<hr data-l=\"10-10\"><div class=\"s\">10</div>"));
        // The tail last.
        assert!(out.ends_with("<div class=\"tail\"></div>"));
        // Nothing lost.
        let stripped = out.replace("<div class=\"doc\"></div>", "").replace("<div class=\"tail\"></div>", "");
        let mut stripped = stripped;
        for l in [3, 6, 8, 12, 13, 15, 18, 10] {
            stripped = stripped.replace(&format!("<div class=\"s\">{l}</div>"), "");
        }
        assert_eq!(stripped, DOC);
    }

    #[test]
    fn uncovered_lines_and_empty_docs() {
        let out = splice(DOC, None, &[(2, "<s/>".into())], None);
        // Line 2 is between the h1 and the p: after the h1.
        assert!(out.contains("</h1><s/>\n<p"));
        let out = splice(DOC, None, &[(99, "<s/>".into())], None);
        assert!(out.ends_with("</code></pre><s/>\n"), "{out}");
        assert_eq!(splice("", Some("<d/>"), &[(1, "<s/>".into())], Some("<t/>")), "<d/><s/><t/>");
        assert_eq!(splice("<p data-l=\"1-1\">x</p>", None, &[], None), "<p data-l=\"1-1\">x</p>");
        // No h1 first: the Document-level strip goes to the top.
        assert_eq!(splice("<p data-l=\"1-1\">x</p>", Some("<d/>"), &[], None), "<d/><p data-l=\"1-1\">x</p>");
    }

    #[test]
    fn tolerates_raw_html_and_stray_tags() {
        let html = "<p data-l=\"1-1\">a <b>bold</p>\n<div><span>raw</div>\n</em><p data-l=\"3-3\">c &lt;p data-l=\"9-9\"&gt;</p>";
        let b = scan(html);
        assert_eq!(b.len(), 2);
        assert_eq!(b[1].a, 3);
        let out = splice(html, None, &[(3, "<s/>".into()), (1, "<r/>".into())], None);
        assert!(out.ends_with("&gt;</p><s/>"));
        assert!(out.contains("</p><r/>\n<div>"));
    }
}
