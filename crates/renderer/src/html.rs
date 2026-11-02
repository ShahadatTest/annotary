//! The event → HTML writer. One pass over the pulldown-cmark event stream (with byte offsets),
//! stamping every block-level element it opens with `data-l="L-L"` (1-based, inclusive source
//! lines of the block's content), rewriting local Asset references through the manifest,
//! collecting footnote definitions for the end, and sanitizing raw HTML.

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use pulldown_cmark::{Alignment, CodeBlockKind, CowStr, Event, HeadingLevel, LinkType, Parser, Tag, TagEnd};

use crate::assets::{Asset, Kind, Manifest, Target};
use crate::autolink;
use crate::escape::{escape_attr, escape_text, push_attr, push_missing_placeholder, url_allowed};
use crate::refs::MARKDOWN_OPTIONS;
use crate::sanitize;

/// What a render needs besides the Markdown.
#[derive(Clone, Copy, Debug)]
pub struct Options<'a> {
    /// The Version's manifest, as [`Asset::from_manifest`] reduces it.
    pub assets: &'a [Asset],
    /// Render images / media inline (`true`, Documents) or as plain links (`false`, Comment
    /// bodies — "the Document superset minus embeds").
    pub embeds: bool,
}

impl Default for Options<'_> {
    fn default() -> Self {
        Options { assets: &[], embeds: true }
    }
}

/// The result of a render.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Rendered {
    /// The viewer-agnostic HTML fragment (no `<html>`/`<body>`, no CSS, no scripts).
    pub html: String,
    /// The Document's display title: front matter `title:` else the first `# H1` (plain text).
    /// Display-only; never rendered as text of its own.
    pub title: Option<String>,
}

/// Render `md` with `opts`. Pure and deterministic: the same input gives the same bytes.
pub fn render(md: &str, opts: &Options<'_>) -> Rendered {
    let events = collect(md);
    let mut w = Writer::new(md, opts);
    w.prescan(&events);
    for (ev, range) in events {
        w.event(ev, range);
    }
    w.finish()
}

/// The event stream with offsets; adjacent `Text` events merged (pulldown splits text around
/// unmatched delimiters, which would cut bare URLs).
fn collect(md: &str) -> Vec<(Event<'_>, Range<usize>)> {
    let mut out: Vec<(Event<'_>, Range<usize>)> = Vec::new();
    for (ev, range) in Parser::new_ext(md, MARKDOWN_OPTIONS).into_offset_iter() {
        if let Event::Text(t) = &ev
            && let Some((Event::Text(prev), prange)) = out.last_mut()
            && prange.end == range.start
        {
            let mut s = String::with_capacity(prev.len() + t.len());
            s.push_str(prev);
            s.push_str(t);
            *prev = CowStr::from(s);
            prange.end = range.end;
            continue;
        }
        out.push((ev, range));
    }
    out
}

/// Byte offset → 1-based source line.
struct Lines {
    starts: Vec<usize>,
}

impl Lines {
    fn new(md: &str) -> Lines {
        let mut starts = vec![0];
        for (i, b) in md.bytes().enumerate() {
            if b == b'\n' {
                starts.push(i + 1);
            }
        }
        Lines { starts }
    }

    fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&s| s <= offset).max(1)
    }

    /// The content lines of a block: from the first byte of `range` to its last non-whitespace
    /// byte (a block's range often runs on over the blank line that ends it).
    fn span(&self, md: &str, range: &Range<usize>) -> (usize, usize) {
        let start = self.line_of(range.start);
        let bytes = md.as_bytes();
        let mut end = range.end.min(bytes.len());
        while end > range.start && bytes[end - 1].is_ascii_whitespace() {
            end -= 1;
        }
        let end_line = if end > range.start { self.line_of(end - 1) } else { start };
        (start, end_line.max(start))
    }
}

enum CodeMode {
    Code,
    Mermaid,
    Math(Range<usize>),
}

struct TableState {
    aligns: Vec<Alignment>,
    head: bool,
    cell: usize,
    body_open: bool,
    end_line: usize,
    /// The current row's lines (a cell the source row lacks has an empty range).
    row: (usize, usize),
}

struct ImageInfo {
    dest: String,
    title: String,
}

struct FnDef {
    name: String,
    html: String,
    lines: (usize, usize),
}

struct Writer<'a> {
    md: &'a str,
    lines: Lines,
    manifest: Manifest<'a>,
    embeds: bool,
    /// Output buffers; the last one is where writes go (headings, image alt text, footnote
    /// definitions and front matter are captured before they are placed).
    bufs: Vec<String>,
    /// Plain-text sinks (heading slugs, image alt text); text goes to the innermost one.
    sinks: Vec<String>,
    link_depth: usize,
    code: Option<CodeMode>,
    headings: Vec<(HeadingLevel, Range<usize>)>,
    slugs: HashMap<String, usize>,
    images: Vec<ImageInfo>,
    fn_defined: HashSet<String>,
    fn_order: Vec<String>,
    fn_refs: HashMap<String, usize>,
    fn_defs: Vec<FnDef>,
    fn_stack: Vec<(String, Range<usize>)>,
    table: Option<TableState>,
    html_buf: String,
    /// An inline `<script>`/`<style>`/… was opened in raw HTML and not closed in the same
    /// event: inline events are dropped until its closing tag (or the block ends).
    raw_drop: Option<String>,
    metadata_depth: usize,
    front_matter: Option<String>,
    first_h1: Option<String>,
}

impl<'a> Writer<'a> {
    fn new(md: &'a str, opts: &Options<'a>) -> Writer<'a> {
        Writer {
            md,
            lines: Lines::new(md),
            manifest: Manifest::new(opts.assets),
            embeds: opts.embeds,
            bufs: vec![String::with_capacity(md.len() * 2)],
            sinks: Vec::new(),
            link_depth: 0,
            code: None,
            headings: Vec::new(),
            slugs: HashMap::new(),
            images: Vec::new(),
            fn_defined: HashSet::new(),
            fn_order: Vec::new(),
            fn_refs: HashMap::new(),
            fn_defs: Vec::new(),
            fn_stack: Vec::new(),
            table: None,
            html_buf: String::new(),
            raw_drop: None,
            metadata_depth: 0,
            front_matter: None,
            first_h1: None,
        }
    }

    /// Footnote definitions can follow their references: know them up front.
    fn prescan(&mut self, events: &[(Event<'_>, Range<usize>)]) {
        for (ev, _) in events {
            if let Event::Start(Tag::FootnoteDefinition(name)) = ev {
                self.fn_defined.insert(name.to_string());
            }
        }
    }

    fn out(&mut self) -> &mut String {
        self.bufs.last_mut().expect("an output buffer")
    }

    fn w(&mut self, s: &str) {
        self.out().push_str(s);
    }

    fn push_buf(&mut self) {
        self.bufs.push(String::new());
    }

    fn pop_buf(&mut self) -> String {
        self.bufs.pop().expect("a captured buffer")
    }

    /// Feed the innermost plain-text sink.
    fn sink(&mut self, s: &str) {
        if let Some(sink) = self.sinks.last_mut() {
            sink.push_str(s);
        }
    }

    fn span(&self, range: &Range<usize>) -> (usize, usize) {
        self.lines.span(self.md, range)
    }

    /// `<tag data-l="a-b"{extra}>` — container blocks start on their own line.
    fn open_block(&mut self, tag: &str, range: &Range<usize>, extra: &str) {
        let (a, b) = self.span(range);
        let out = self.out();
        if matches!(tag, "ul" | "ol" | "blockquote" | "pre" | "table" | "dl" | "section")
            && !out.is_empty()
            && !out.ends_with('\n')
        {
            out.push('\n');
        }
        out.push('<');
        out.push_str(tag);
        out.push_str(&format!(" data-l=\"{a}-{b}\""));
        out.push_str(extra);
        out.push('>');
    }

    fn flush_html(&mut self) {
        if self.html_buf.is_empty() {
            return;
        }
        let raw = std::mem::take(&mut self.html_buf);
        let mut cleaned = String::with_capacity(raw.len());
        sanitize::sanitize(&raw, &sanitize::HTML, &self.manifest, &mut cleaned);
        if cleaned.trim().is_empty() {
            return; // a dropped `<script>`/`<style>` block leaves nothing behind
        }
        self.w(&cleaned);
    }

    fn slug(&mut self, text: &str) -> String {
        let mut s = String::with_capacity(text.len());
        for c in text.trim().chars() {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                for l in c.to_lowercase() {
                    s.push(l);
                }
            } else if c.is_whitespace() {
                s.push('-');
            }
        }
        if s.is_empty() {
            s.push_str("section");
        }
        let n = self.slugs.entry(s.clone()).or_insert(0);
        let out = if *n == 0 { s.clone() } else { format!("{s}-{n}") };
        *n += 1;
        out
    }

    fn event(&mut self, ev: Event<'_>, range: Range<usize>) {
        if !matches!(ev, Event::Html(_)) {
            self.flush_html();
        }
        if let Some(name) = self.raw_drop.clone() {
            match &ev {
                Event::InlineHtml(h) if sanitize::is_close_tag(h, &name) => {
                    self.raw_drop = None;
                    return;
                }
                Event::Start(_) | Event::End(_) => self.raw_drop = None,
                _ => return,
            }
        }
        match ev {
            Event::Start(tag) => self.start(tag, range),
            Event::End(tag) => self.end(tag),
            Event::Text(t) => self.text(&t),
            Event::Code(c) => {
                self.sink(&c);
                self.w("<code>");
                let mut s = String::new();
                escape_text(&c, &mut s);
                self.w(&s);
                self.w("</code>");
            }
            Event::InlineMath(m) => {
                self.sink(&m);
                let html = mathml(&m, false);
                self.w(&html);
            }
            Event::DisplayMath(m) => {
                self.sink(&m);
                let html = mathml(&m, true);
                self.w(&html);
            }
            Event::Html(h) => self.html_buf.push_str(&h),
            Event::InlineHtml(h) => {
                if let Some(name) = sanitize::dropped_open_tag(&h, &sanitize::HTML) {
                    self.raw_drop = Some(name);
                    return;
                }
                let mut cleaned = String::new();
                sanitize::sanitize(&h, &sanitize::HTML, &self.manifest, &mut cleaned);
                self.w(&cleaned);
            }
            Event::FootnoteReference(name) => self.footnote_ref(&name),
            Event::SoftBreak => {
                self.sink(" ");
                self.w("\n");
            }
            Event::HardBreak => {
                self.sink(" ");
                self.w("<br>\n");
            }
            Event::Rule => {
                let (a, b) = self.span(&range);
                self.w(&format!("<hr data-l=\"{a}-{b}\">\n"));
            }
            Event::TaskListMarker(checked) => {
                self.w(if checked {
                    "<input type=\"checkbox\" checked disabled> "
                } else {
                    "<input type=\"checkbox\" disabled> "
                });
            }
        }
    }

    fn text(&mut self, t: &str) {
        if self.metadata_depth > 0 {
            self.w(t);
            return;
        }
        match &self.code {
            Some(CodeMode::Math(_)) => {
                self.w(t);
                return;
            }
            Some(_) => {
                let mut s = String::new();
                escape_text(t, &mut s);
                self.w(&s);
                return;
            }
            None => {}
        }
        self.sink(t);
        let mut s = String::new();
        if self.link_depth == 0 && self.images.is_empty() {
            autolink::linkify(t, &mut s);
        } else {
            escape_text(t, &mut s);
        }
        self.w(&s);
    }

    fn start(&mut self, tag: Tag<'_>, range: Range<usize>) {
        match tag {
            Tag::Paragraph => self.open_block("p", &range, ""),
            Tag::Heading { level, .. } => {
                self.headings.push((level, range));
                self.push_buf();
                self.sinks.push(String::new());
            }
            Tag::BlockQuote(_) => {
                self.open_block("blockquote", &range, "");
                self.w("\n");
            }
            Tag::CodeBlock(kind) => {
                let lang = match &kind {
                    CodeBlockKind::Fenced(info) => info.split([' ', '\t', ',']).next().unwrap_or("").trim().to_string(),
                    CodeBlockKind::Indented => String::new(),
                };
                if lang.eq_ignore_ascii_case("mermaid") {
                    self.open_block("pre", &range, " class=\"mermaid\"");
                    self.code = Some(CodeMode::Mermaid);
                } else if lang.eq_ignore_ascii_case("math") {
                    self.push_buf();
                    self.code = Some(CodeMode::Math(range));
                } else {
                    self.open_block("pre", &range, "");
                    if lang.is_empty() {
                        self.w("<code>");
                    } else {
                        let mut s = String::from("<code class=\"language-");
                        escape_attr(&lang, &mut s);
                        s.push_str("\">");
                        self.w(&s);
                    }
                    self.code = Some(CodeMode::Code);
                }
            }
            Tag::HtmlBlock => {}
            Tag::List(start) => {
                match start {
                    Some(1) => self.open_block("ol", &range, ""),
                    Some(n) => self.open_block("ol", &range, &format!(" start=\"{n}\"")),
                    None => self.open_block("ul", &range, ""),
                }
                self.w("\n");
            }
            Tag::Item => self.open_block("li", &range, ""),
            Tag::FootnoteDefinition(name) => {
                self.fn_stack.push((name.to_string(), range));
                self.push_buf();
            }
            Tag::DefinitionList => {
                self.open_block("dl", &range, "");
                self.w("\n");
            }
            Tag::DefinitionListTitle => self.open_block("dt", &range, ""),
            Tag::DefinitionListDefinition => self.open_block("dd", &range, ""),
            Tag::Table(aligns) => {
                self.open_block("table", &range, "");
                self.w("\n");
                let end_line = self.span(&range).1;
                let row = self.span(&range);
                self.table = Some(TableState { aligns, head: false, cell: 0, body_open: false, end_line, row });
            }
            Tag::TableHead => {
                self.open_block("thead", &range, "");
                self.w("\n");
                self.open_block("tr", &range, "");
                let row = self.span(&range);
                if let Some(t) = &mut self.table {
                    t.head = true;
                    t.cell = 0;
                    t.row = row;
                }
            }
            Tag::TableRow => {
                let row_start = self.span(&range).0;
                if let Some(t) = &mut self.table
                    && !t.body_open
                {
                    t.body_open = true;
                    let end = t.end_line.max(row_start);
                    self.w(&format!("<tbody data-l=\"{row_start}-{end}\">\n"));
                }
                self.open_block("tr", &range, "");
                let row = self.span(&range);
                if let Some(t) = &mut self.table {
                    t.cell = 0;
                    t.row = row;
                }
            }
            Tag::TableCell => {
                let (tag, align, row) = match &self.table {
                    Some(t) => (
                        if t.head { "th" } else { "td" },
                        match t.aligns.get(t.cell) {
                            Some(Alignment::Left) => " align=\"left\"",
                            Some(Alignment::Center) => " align=\"center\"",
                            Some(Alignment::Right) => " align=\"right\"",
                            _ => "",
                        },
                        t.row,
                    ),
                    None => ("td", "", (0, 0)),
                };
                if range.is_empty() && row.0 > 0 {
                    // A cell the source row does not have: it belongs to the row's lines.
                    self.w(&format!("<{tag} data-l=\"{}-{}\"{align}>", row.0, row.1));
                } else {
                    self.open_block(tag, &range, align);
                }
            }
            Tag::Emphasis => self.w("<em>"),
            Tag::Strong => self.w("<strong>"),
            Tag::Strikethrough => self.w("<del>"),
            Tag::Superscript => self.w("<sup>"),
            Tag::Subscript => self.w("<sub>"),
            Tag::Link { link_type, dest_url, title, .. } => {
                self.link_depth += 1;
                let mut s = String::from("<a");
                // `<me@example.com>`: pulldown hands over the bare address.
                let dest_url: CowStr<'_> = if link_type == LinkType::Email && !dest_url.starts_with("mailto:") {
                    CowStr::from(format!("mailto:{dest_url}"))
                } else {
                    dest_url
                };
                match self.manifest.resolve(&dest_url) {
                    Target::Asset(a) if a.missing => {
                        s.push_str(" class=\"asset-missing\"");
                        push_attr(&mut s, "data-ref", &a.reference);
                    }
                    Target::Asset(a) => push_attr(&mut s, "href", &a.url().unwrap_or_default()),
                    Target::External => {
                        if url_allowed(&dest_url, false) {
                            push_attr(&mut s, "href", &dest_url);
                        }
                    }
                    Target::AsWritten => push_attr(&mut s, "href", &dest_url),
                }
                if !title.is_empty() {
                    push_attr(&mut s, "title", &title);
                }
                s.push('>');
                self.w(&s);
            }
            Tag::Image { dest_url, title, .. } => {
                self.images.push(ImageInfo { dest: dest_url.to_string(), title: title.to_string() });
                self.push_buf();
                self.sinks.push(String::new());
            }
            Tag::MetadataBlock(_) => {
                self.metadata_depth += 1;
                self.push_buf();
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.w("</p>\n"),
            TagEnd::Heading(_) => {
                let html = self.pop_buf();
                let text = self.sinks.pop().unwrap_or_default();
                let (level, range) = self.headings.pop().expect("an open heading");
                let slug = self.slug(&text);
                if level == HeadingLevel::H1 && self.first_h1.is_none() {
                    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !t.is_empty() {
                        self.first_h1 = Some(t);
                    }
                }
                let tag = match level {
                    HeadingLevel::H1 => "h1",
                    HeadingLevel::H2 => "h2",
                    HeadingLevel::H3 => "h3",
                    HeadingLevel::H4 => "h4",
                    HeadingLevel::H5 => "h5",
                    HeadingLevel::H6 => "h6",
                };
                let mut extra = String::new();
                push_attr(&mut extra, "id", &slug);
                self.open_block(tag, &range, &extra);
                self.w(&html);
                self.w(&format!("</{tag}>\n"));
            }
            TagEnd::BlockQuote(_) => self.w("</blockquote>\n"),
            TagEnd::CodeBlock => match self.code.take() {
                Some(CodeMode::Code) => self.w("</code></pre>\n"),
                Some(CodeMode::Mermaid) => self.w("</pre>\n"),
                Some(CodeMode::Math(range)) => {
                    let src = self.pop_buf();
                    let (a, b) = self.span(&range);
                    let html = mathml(src.trim_end_matches(['\n', '\r']), true);
                    // `<math …>` → `<math data-l="a-b" …>`
                    let html = html.replacen("<math", &format!("<math data-l=\"{a}-{b}\""), 1);
                    if !self.out().is_empty() && !self.out().ends_with('\n') {
                        self.w("\n");
                    }
                    self.w(&html);
                    self.w("\n");
                }
                None => {}
            },
            TagEnd::HtmlBlock => {}
            TagEnd::List(ordered) => self.w(if ordered { "</ol>\n" } else { "</ul>\n" }),
            TagEnd::Item => self.w("</li>\n"),
            TagEnd::FootnoteDefinition => {
                let html = self.pop_buf();
                let (name, range) = self.fn_stack.pop().expect("an open footnote definition");
                if !self.fn_defs.iter().any(|d| d.name == name) {
                    let lines = self.span(&range);
                    self.fn_defs.push(FnDef { name, html, lines });
                }
            }
            TagEnd::DefinitionList => self.w("</dl>\n"),
            TagEnd::DefinitionListTitle => self.w("</dt>\n"),
            TagEnd::DefinitionListDefinition => self.w("</dd>\n"),
            TagEnd::Table => {
                if let Some(t) = self.table.take()
                    && t.body_open
                {
                    self.w("</tbody>\n");
                }
                self.w("</table>\n");
            }
            TagEnd::TableHead => {
                self.w("</tr>\n</thead>\n");
                if let Some(t) = &mut self.table {
                    t.head = false;
                }
            }
            TagEnd::TableRow => self.w("</tr>\n"),
            TagEnd::TableCell => {
                let head = self.table.as_ref().is_some_and(|t| t.head);
                self.w(if head { "</th>" } else { "</td>" });
                if let Some(t) = &mut self.table {
                    t.cell += 1;
                }
            }
            TagEnd::Emphasis => self.w("</em>"),
            TagEnd::Strong => self.w("</strong>"),
            TagEnd::Strikethrough => self.w("</del>"),
            TagEnd::Superscript => self.w("</sup>"),
            TagEnd::Subscript => self.w("</sub>"),
            TagEnd::Link => {
                self.link_depth = self.link_depth.saturating_sub(1);
                self.w("</a>");
            }
            TagEnd::Image => {
                let _discarded = self.pop_buf();
                let alt = self.sinks.pop().unwrap_or_default();
                let info = self.images.pop().expect("an open image");
                let html = self.image_html(&info, &alt);
                self.w(&html);
            }
            TagEnd::MetadataBlock(_) => {
                let text = self.pop_buf();
                self.metadata_depth = self.metadata_depth.saturating_sub(1);
                if self.front_matter.is_none() {
                    self.front_matter = Some(text);
                }
            }
        }
    }

    fn footnote_ref(&mut self, name: &str) {
        if !self.fn_defined.contains(name) {
            let mut s = String::from("[^");
            escape_text(name, &mut s);
            s.push(']');
            self.w(&s);
            return;
        }
        let n = match self.fn_order.iter().position(|f| f == name) {
            Some(i) => i + 1,
            None => {
                self.fn_order.push(name.to_string());
                self.fn_order.len()
            }
        };
        let count = self.fn_refs.entry(name.to_string()).or_insert(0);
        *count += 1;
        let id = if *count == 1 { format!("fnref-{n}") } else { format!("fnref-{n}-{count}") };
        self.w(&format!("<sup class=\"footnote-ref\"><a href=\"#fn-{n}\" id=\"{id}\">{n}</a></sup>"));
    }

    /// The element an image reference becomes.
    fn image_html(&self, info: &ImageInfo, alt: &str) -> String {
        let mut s = String::new();
        let title = |s: &mut String| {
            if !info.title.is_empty() {
                push_attr(s, "title", &info.title);
            }
        };
        let link = |s: &mut String, href: &str, text: &str| {
            s.push_str("<a");
            push_attr(s, "href", href);
            title(s);
            s.push('>');
            escape_text(text, s);
            s.push_str("</a>");
        };
        let target = self.manifest.resolve(&info.dest);
        match target {
            Target::Asset(a) if a.missing => {
                if self.embeds {
                    push_missing_placeholder(&mut s, &a.reference, alt);
                } else {
                    s.push_str("<a class=\"asset-missing\"");
                    push_attr(&mut s, "data-ref", &a.reference);
                    s.push('>');
                    escape_text(if alt.trim().is_empty() { &a.reference } else { alt }, &mut s);
                    s.push_str("</a>");
                }
            }
            Target::Asset(a) => {
                let url = a.url().unwrap_or_default();
                let text = if alt.trim().is_empty() { a.reference.as_str() } else { alt };
                if !self.embeds {
                    link(&mut s, &url, text);
                    return s;
                }
                match a.kind() {
                    Kind::Image => {
                        s.push_str("<img");
                        push_attr(&mut s, "src", &a.display_url().unwrap_or_default());
                        push_attr(&mut s, "alt", alt);
                        title(&mut s);
                        push_dims(&mut s, a);
                        s.push('>');
                    }
                    Kind::Video => {
                        s.push_str("<video controls preload=\"metadata\"");
                        push_attr(&mut s, "src", &url);
                        title(&mut s);
                        push_dims(&mut s, a);
                        s.push('>');
                        link(&mut s, &url, text);
                        s.push_str("</video>");
                    }
                    Kind::Audio => {
                        s.push_str("<audio controls preload=\"metadata\"");
                        push_attr(&mut s, "src", &url);
                        title(&mut s);
                        s.push('>');
                        link(&mut s, &url, text);
                        s.push_str("</audio>");
                    }
                    Kind::Pdf => {
                        s.push_str("<object type=\"application/pdf\"");
                        push_attr(&mut s, "data", &url);
                        title(&mut s);
                        s.push('>');
                        link(&mut s, &url, text);
                        s.push_str("</object>");
                    }
                    Kind::Text | Kind::Other => link(&mut s, &url, text),
                }
            }
            Target::External | Target::AsWritten => {
                let allowed = matches!(target, Target::AsWritten) || url_allowed(&info.dest, true);
                if !self.embeds {
                    let text = if alt.trim().is_empty() { info.dest.as_str() } else { alt };
                    if allowed {
                        link(&mut s, &info.dest, text);
                    } else {
                        escape_text(text, &mut s);
                    }
                    return s;
                }
                s.push_str("<img");
                if allowed {
                    push_attr(&mut s, "src", &info.dest);
                }
                push_attr(&mut s, "alt", alt);
                title(&mut s);
                s.push('>');
            }
        }
        s
    }

    fn finish(mut self) -> Rendered {
        self.flush_html();
        self.footnotes_section();
        let html = self.bufs.pop().unwrap_or_default();
        let title = self.front_matter.as_deref().and_then(front_matter_title).or(self.first_h1.take());
        Rendered { html, title }
    }

    fn footnotes_section(&mut self) {
        if self.fn_defs.is_empty() {
            return;
        }
        // Referenced definitions in first-reference order, then unreferenced ones in source order.
        let mut ordered: Vec<usize> = Vec::new();
        for name in &self.fn_order {
            if let Some(i) = self.fn_defs.iter().position(|d| &d.name == name) {
                ordered.push(i);
            }
        }
        for (i, _) in self.fn_defs.iter().enumerate() {
            if !ordered.contains(&i) {
                ordered.push(i);
            }
        }
        let min = ordered.iter().map(|&i| self.fn_defs[i].lines.0).min().unwrap_or(1);
        let max = ordered.iter().map(|&i| self.fn_defs[i].lines.1).max().unwrap_or(min);
        let mut s = format!("<section class=\"footnotes\" data-l=\"{min}-{max}\">\n<ol data-l=\"{min}-{max}\">\n");
        for (pos, &i) in ordered.iter().enumerate() {
            let n = pos + 1;
            let def = &self.fn_defs[i];
            let backref = format!(" <a href=\"#fnref-{n}\" class=\"footnote-backref\">\u{21A9}</a>");
            let mut body = def.html.clone();
            if let Some(stripped) = body.strip_suffix("</p>\n") {
                body = format!("{stripped}{backref}</p>\n");
            } else {
                body = format!("{}{backref}\n", body.trim_end_matches('\n'));
            }
            s.push_str(&format!("<li id=\"fn-{n}\" data-l=\"{}-{}\">\n{body}</li>\n", def.lines.0, def.lines.1));
        }
        s.push_str("</ol>\n</section>\n");
        self.w(&s);
    }
}

fn push_dims(s: &mut String, a: &Asset) {
    if let Some(w) = a.width {
        push_attr(s, "width", &w.to_string());
    }
    if let Some(h) = a.height {
        push_attr(s, "height", &h.to_string());
    }
}

/// `title:` from a YAML front matter block (top-level key, quotes stripped).
pub fn front_matter_title(yaml: &str) -> Option<String> {
    for line in yaml.lines() {
        if let Some(rest) = line.strip_prefix("title:") {
            let v = rest.trim();
            let v = v
                .strip_prefix('"')
                .and_then(|s| s.strip_suffix('"'))
                .map(|s| s.replace("\\\"", "\""))
                .or_else(|| v.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')).map(|s| s.replace("''", "'")))
                .unwrap_or_else(|| v.to_string());
            let v = v.trim().to_string();
            return if v.is_empty() { None } else { Some(v) };
        }
    }
    None
}

/// LaTeX → MathML (`<math …>…</math>`), sanitized; a parse failure renders the source inside
/// `<merror>` so nothing is silently dropped.
pub fn mathml(src: &str, display: bool) -> String {
    let style = if display { latex2mathml::DisplayStyle::Block } else { latex2mathml::DisplayStyle::Inline };
    let src = &math_aliases(src);
    let converted =
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| latex2mathml::latex_to_mathml(src, style)));
    let mut out = String::new();
    match converted {
        Ok(Ok(m)) => sanitize::sanitize(&m, &sanitize::MATHML, &Manifest::default(), &mut out),
        _ => {
            out.push_str("<math xmlns=\"http://www.w3.org/1998/Math/MathML\" display=\"");
            out.push_str(if display { "block" } else { "inline" });
            out.push_str("\"><merror><mtext>");
            escape_text(src, &mut out);
            out.push_str("</mtext></merror></math>");
        }
    }
    out
}

/// Common LaTeX spellings the converter lacks, mapped onto ones it has (whole command names
/// only: `\le` never touches `\left` or `\leq`).
const MATH_ALIASES: &[(&str, &str)] = &[
    ("le", "\\leq"),
    ("ge", "\\geq"),
    ("dots", "\\ldots"),
    ("dfrac", "\\frac"),
    ("tfrac", "\\frac"),
    ("displaystyle", ""),
    ("limits", ""),
    ("nolimits", ""),
    ("big", ""),
    ("Big", ""),
    ("mathcal", "\\mathscr"),
    ("mathtt", "\\texttt"),
    ("neg", "\\lnot"),
    ("varnothing", "\\emptyset"),
    ("stackrel", "\\overset"),
    ("vert", "|"),
    ("lvert", "|"),
    ("rvert", "|"),
    ("Vert", "\\|"),
    ("lVert", "\\|"),
    ("rVert", "\\|"),
    ("prime", "'"),
    ("ast", "*"),
    ("gcd", "\\operatorname{gcd}"),
    ("deg", "\\operatorname{deg}"),
    ("Pr", "\\operatorname{Pr}"),
    ("bmod", "\\operatorname{mod}"),
];

fn math_aliases(src: &str) -> String {
    if !src.contains('\\') {
        return src.to_string();
    }
    let mut out = String::with_capacity(src.len());
    let bytes = src.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_alphabetic() {
                j += 1;
            }
            let name = &src[i + 1..j];
            if !name.is_empty()
                && let Some((_, to)) = MATH_ALIASES.iter().find(|(from, _)| *from == name)
            {
                out.push_str(to);
                i = j;
                continue;
            }
            // Copy the backslash and whatever follows it (a command we keep, or an escape).
            let step = if j > i + 1 { j } else { i + 1 + src[i + 1..].chars().next().map(char::len_utf8).unwrap_or(0) };
            out.push_str(&src[i..step.max(i + 1)]);
            i = step.max(i + 1);
            continue;
        }
        let ch = src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn math_aliases_replace_whole_commands_only() {
        assert_eq!(
            math_aliases(r"a \le b \left( \leq \right) \\ \dfrac{1}{2}"),
            r"a \leq b \left( \leq \right) \\ \frac{1}{2}"
        );
        assert_eq!(math_aliases(r"\mathcal{L} \gcd(a,b) x\prime"), r"\mathscr{L} \operatorname{gcd}(a,b) x'");
        assert_eq!(math_aliases("no commands"), "no commands");
        assert_eq!(math_aliases(r"\{ \} \$ \% \_ \ünï"), r"\{ \} \$ \% \_ \ünï");
        assert!(mathml(r"x \le y", false).contains("≤"));
        assert!(mathml("", true).starts_with("<math"));
    }
}
