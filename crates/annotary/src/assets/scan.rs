//! Pure reference extraction from Markdown text (no filesystem): what a Document points at,
//! in document order — Markdown images `![](p)`, links `[](p)`, reference-style definitions
//! `[x]: p`, and HTML `<img>/<video>/<audio>/<source>` `src` and `<a>` `href` — then the
//! classification of each target (relative path → Asset candidate; `http(s)://`, `#frag`,
//! `mailto:`, `data:` and other schemes ignored silently; absolute filesystem paths and
//! `file://` warned and ignored) and the lexical normalisation of a relative path against the
//! referencing file's directory. The same parser options the renderer uses (ticket 09), so what
//! the scanner sees is what gets rendered — the options, classification and normalisation are
//! one definition in `annotary_renderer::refs`, re-exported here.

use pulldown_cmark::{Event, LinkType, Parser, Tag};

// ---- ticket 09: these definitions moved to the renderer crate (single source of truth) ----
pub use annotary_renderer::refs::{Classified, MARKDOWN_OPTIONS, classify, dir_of, normalize, percent_decode};
// ---- end ticket 09 ----

/// Where a reference was written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    MdImage,
    MdLink,
    MdRefDef,
    HtmlImg,
    HtmlVideo,
    HtmlAudio,
    HtmlSource,
    HtmlA,
}

/// One reference as written, before classification.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawRef {
    pub target: String,
    pub syntax: Syntax,
}

/// Every reference in `md`, in document order (code spans and fenced code are not references;
/// autolinks are URLs and come out as such).
pub fn references(md: &str) -> Vec<RawRef> {
    references_at(md).into_iter().map(|(_, r)| r).collect()
}

// ---- ticket 07 ----
/// The 1-based source line where each relative reference of the root Markdown is first written
/// (`ref` normalised against the root directory, i.e. as the manifest lists it): the display-only
/// "referencing line" of an Asset Thread. Recursive Assets (referenced from another Asset) are
/// not in this list.
pub fn reference_lines(md: &str) -> Vec<(String, u32)> {
    let mut out: Vec<(String, u32)> = Vec::new();
    for (offset, raw) in references_at(md) {
        let Classified::Relative(rel) = classify(&raw.target) else { continue };
        let reference = normalize("", &rel);
        if reference.is_empty() || out.iter().any(|(r, _)| *r == reference) {
            continue;
        }
        let line = md[..offset.min(md.len())].bytes().filter(|b| *b == b'\n').count() as u32 + 1;
        out.push((reference, line));
    }
    out
}
// ---- end ticket 07 ----

/// [`references`] with the byte offset each reference was written at.
pub fn references_at(md: &str) -> Vec<(usize, RawRef)> {
    let mut found: Vec<(usize, RawRef)> = Vec::new();
    let parser = Parser::new_ext(md, MARKDOWN_OPTIONS);
    // Reference definitions are not events; take them from the parser's table (with spans).
    for (_label, def) in parser.reference_definitions().iter() {
        found.push((def.span.start, RawRef { target: def.dest.to_string(), syntax: Syntax::MdRefDef }));
    }
    let mut html_buf = String::new();
    let mut html_start = 0usize;
    let flush = |buf: &mut String, start: usize, found: &mut Vec<(usize, RawRef)>| {
        if !buf.is_empty() {
            for (off, syntax, target) in html_refs(buf) {
                found.push((start + off, RawRef { target, syntax }));
            }
            buf.clear();
        }
    };
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Html(h) | Event::InlineHtml(h) => {
                if html_buf.is_empty() {
                    html_start = range.start;
                }
                html_buf.push_str(&h);
                continue;
            }
            _ => flush(&mut html_buf, html_start, &mut found),
        }
        match event {
            Event::Start(Tag::Image { dest_url, .. }) => {
                found.push((range.start, RawRef { target: dest_url.to_string(), syntax: Syntax::MdImage }));
            }
            Event::Start(Tag::Link { link_type, dest_url, .. }) => {
                let syntax = match link_type {
                    LinkType::Autolink | LinkType::Email => continue,
                    _ => Syntax::MdLink,
                };
                found.push((range.start, RawRef { target: dest_url.to_string(), syntax }));
            }
            _ => {}
        }
    }
    flush(&mut html_buf, html_start, &mut found);
    found.sort_by_key(|(off, _)| *off);
    found
}

/// `src`/`href` of the media/anchor tags in an HTML fragment: `(offset, syntax, value)`.
fn html_refs(html: &str) -> Vec<(usize, Syntax, String)> {
    let bytes = html.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let tag_start = i;
        i += 1;
        let name_start = i;
        while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
            i += 1;
        }
        let name = html[name_start..i].to_ascii_lowercase();
        let (syntax, attr) = match name.as_str() {
            "img" => (Syntax::HtmlImg, "src"),
            "video" => (Syntax::HtmlVideo, "src"),
            "audio" => (Syntax::HtmlAudio, "src"),
            "source" => (Syntax::HtmlSource, "src"),
            "a" => (Syntax::HtmlA, "href"),
            _ => continue,
        };
        if i >= bytes.len() || !(bytes[i].is_ascii_whitespace() || bytes[i] == b'/' || bytes[i] == b'>') {
            continue;
        }
        // Attributes until `>`.
        loop {
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            if i >= bytes.len() || bytes[i] == b'>' {
                break;
            }
            if bytes[i] == b'/' {
                i += 1;
                continue;
            }
            let an = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>' | b'/') {
                i += 1;
            }
            let aname = html[an..i].to_ascii_lowercase();
            while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            let mut value: Option<String> = None;
            if i < bytes.len() && bytes[i] == b'=' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i] == b'"' || bytes[i] == b'\'') {
                    let q = bytes[i];
                    i += 1;
                    let vs = i;
                    while i < bytes.len() && bytes[i] != q {
                        i += 1;
                    }
                    value = Some(html[vs..i].to_string());
                    if i < bytes.len() {
                        i += 1;
                    }
                } else {
                    let vs = i;
                    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                        i += 1;
                    }
                    value = Some(html[vs..i].to_string());
                }
            }
            if aname == attr
                && let Some(v) = value
            {
                out.push((tag_start, syntax, decode_entities(&v)));
            }
        }
    }
    out
}

/// The handful of entities that show up in attribute values.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets(md: &str) -> Vec<(Syntax, String)> {
        references(md).into_iter().map(|r| (r.syntax, r.target)).collect()
    }

    // spec: S03-01
    #[test]
    fn every_syntax_is_found_in_document_order() {
        let md = "\
# Doc

![arch](img/architecture.png \"title\")
See [API notes](api-notes.md) and [the spec][spec] and <https://example.com/auto>.

<img src=\"img/inline.png\" alt=x> <a href='files/notes.pdf'>pdf</a>
<video src=vid/clip.mp4 controls></video>

<audio controls>
  <source src=\"snd/a.mp3\" type=\"audio/mpeg\">
</audio>

[spec]: docs/spec.md
[unused]: img/unused.gif \"Unused\"

`![not](code.png)`

```
[nor](fenced.png)
```

[![nested image](img/thumb.png)](img/big.png)
";
        assert_eq!(
            targets(md),
            vec![
                (Syntax::MdImage, "img/architecture.png".into()),
                (Syntax::MdLink, "api-notes.md".into()),
                (Syntax::MdLink, "docs/spec.md".into()),
                (Syntax::HtmlImg, "img/inline.png".into()),
                (Syntax::HtmlA, "files/notes.pdf".into()),
                (Syntax::HtmlVideo, "vid/clip.mp4".into()),
                (Syntax::HtmlSource, "snd/a.mp3".into()),
                (Syntax::MdRefDef, "docs/spec.md".into()),
                (Syntax::MdRefDef, "img/unused.gif".into()),
                (Syntax::MdLink, "img/big.png".into()),
                (Syntax::MdImage, "img/thumb.png".into()),
            ]
        );
        // Angle-bracket destinations with spaces, entities in HTML, multi-line HTML blocks.
        assert_eq!(targets("[x](<my file.png>)"), vec![(Syntax::MdLink, "my file.png".into())]);
        assert_eq!(targets("<div>\n<img\n  src=\"a&amp;b.png\">\n</div>\n"), vec![(Syntax::HtmlImg, "a&b.png".into())]);
        assert_eq!(targets("<IMG SRC=\"caps.png\"/>"), vec![(Syntax::HtmlImg, "caps.png".into())]);
        assert!(targets("<span src=\"no.png\">x</span> <b href=x.pdf>").is_empty());
        assert!(targets("<img alt=\"no src\">").is_empty());
        assert!(targets("").is_empty());
    }

    // spec: S07-09
    #[test]
    fn reference_lines_are_the_first_root_level_mention_of_each_normalised_ref() {
        let md = "# Doc\n\n![a](./img/a.png)\ntext [b](docs/../b.md) and ![a again](img/a.png)\n\n<img src=\"img/c%20d.png\">\n[x]: https://example.com\n";
        assert_eq!(
            reference_lines(md),
            vec![("img/a.png".to_string(), 3), ("b.md".to_string(), 4), ("img/c d.png".to_string(), 6)]
        );
        assert!(reference_lines("no refs\n").is_empty());
    }

    // spec: S03-01
    #[test]
    fn classification_ignores_urls_and_flags_absolute_paths() {
        for ignored in [
            "http://x/y.png",
            "https://x/y.png",
            "HTTPS://X",
            "#frag",
            "#",
            "mailto:a@b.c",
            "data:image/png;base64,AAAA",
            "ftp://x/y",
            "//cdn/x.png",
            "",
            "   ",
            "?only=query",
        ] {
            assert_eq!(classify(ignored), Classified::Ignored, "{ignored:?}");
        }
        for abs in
            ["/srv/pics/x.png", "/x.png", "file:///srv/pics/x.png", "FILE://x", "C:\\photos\\x.png", "\\\\srv\\x"]
        {
            assert_eq!(classify(abs), Classified::Absolute, "{abs:?}");
        }
        assert_eq!(classify("img/a.png"), Classified::Relative("img/a.png".into()));
        assert_eq!(classify("./img/a.png"), Classified::Relative("./img/a.png".into()));
        assert_eq!(classify("../a.png#frag"), Classified::Relative("../a.png".into()));
        assert_eq!(classify("a.png?v=2#x"), Classified::Relative("a.png".into()));
        assert_eq!(classify("my%20file.png"), Classified::Relative("my file.png".into()));
        assert_eq!(classify("<sp ace.png>"), Classified::Relative("sp ace.png".into()));
        assert_eq!(classify("100%.png"), Classified::Relative("100%.png".into()));
        assert_eq!(classify("notes.md:1"), Classified::Relative("notes.md:1".into()), "not a scheme");
        assert_eq!(percent_decode("%C3%9Cber.png"), "Über.png");
        assert_eq!(percent_decode("%ZZ"), "%ZZ");
    }

    // spec: S03-03
    #[test]
    fn normalisation_is_lexical_and_root_relative() {
        assert_eq!(normalize("", "img/a.png"), "img/a.png");
        assert_eq!(normalize("", "./img/../a.png"), "a.png");
        assert_eq!(normalize("docs", "img/flow.svg"), "docs/img/flow.svg");
        assert_eq!(normalize("docs", "../img/x.png"), "img/x.png");
        assert_eq!(normalize("docs/deep", "../../x.png"), "x.png");
        assert_eq!(normalize("", "../shared/x.png"), "../shared/x.png");
        assert_eq!(normalize("../shared", "../y.png"), "../y.png");
        assert_eq!(normalize("", "a//b.png"), "a/b.png");
        assert_eq!(dir_of("docs/notes.md"), "docs");
        assert_eq!(dir_of("notes.md"), "");
    }
}
