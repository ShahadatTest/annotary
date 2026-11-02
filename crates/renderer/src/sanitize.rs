//! Raw HTML written in the Markdown (blocks and inline tags) passes through an allow-list
//! sanitizer so the output stays viewer-agnostic and script-free: `<script>`, `<style>`,
//! `<iframe>`, forms, `on*` handlers, `style=` / `class=` / `data-*` attributes and
//! `javascript:` URLs never survive; local Asset references in `src` / `href` / `poster` are
//! rewritten to `/a/{hash}` exactly like Markdown ones. The same tokenizer, with a MathML policy,
//! cleans what the LaTeX → MathML converter emits.

use crate::assets::{Kind, Manifest, Target};
use crate::escape::{push_attr, push_missing_placeholder, url_allowed};

/// What a policy lets through.
pub struct Policy {
    /// Elements kept (lower-case). Anything else loses its tags; see `drop_with_content`.
    pub tags: &'static [&'static str],
    /// Elements that take no closing tag.
    pub void: &'static [&'static str],
    /// Elements dropped *with* their content (scripts, styles, embedded documents, form controls).
    pub drop_with_content: &'static [&'static str],
    /// Attributes kept (lower-case); `on*`, `style`, `class`, `data-*` are never in here.
    pub attrs: &'static [&'static str],
    /// Emit `/>` for self-closing tags (XML-ish foreign content such as MathML).
    pub xml_self_closing: bool,
    /// Rewrite Asset references and apply the URL policy (HTML) or not (MathML has no URLs).
    pub urls: bool,
}

/// Raw HTML in a Document.
pub static HTML: Policy = Policy {
    tags: &[
        "a",
        "abbr",
        "address",
        "article",
        "aside",
        "audio",
        "b",
        "bdi",
        "bdo",
        "blockquote",
        "br",
        "caption",
        "cite",
        "code",
        "col",
        "colgroup",
        "data",
        "dd",
        "del",
        "details",
        "dfn",
        "div",
        "dl",
        "dt",
        "em",
        "figcaption",
        "figure",
        "footer",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "header",
        "hgroup",
        "hr",
        "i",
        "img",
        "ins",
        "kbd",
        "li",
        "main",
        "mark",
        "nav",
        "ol",
        "p",
        "picture",
        "pre",
        "q",
        "rp",
        "rt",
        "ruby",
        "s",
        "samp",
        "section",
        "small",
        "source",
        "span",
        "strong",
        "sub",
        "summary",
        "sup",
        "table",
        "tbody",
        "td",
        "tfoot",
        "th",
        "thead",
        "time",
        "tr",
        "track",
        "u",
        "ul",
        "var",
        "video",
        "wbr",
    ],
    void: &["br", "col", "hr", "img", "source", "track", "wbr"],
    drop_with_content: &[
        "script",
        "style",
        "textarea",
        "title",
        "iframe",
        "noscript",
        "template",
        "svg",
        "math",
        "object",
        "applet",
        "xmp",
        "plaintext",
        "noembed",
        "noframes",
        "select",
        "option",
        "datalist",
        "head",
        "frame",
        "frameset",
    ],
    attrs: &[
        "abbr",
        "align",
        "alt",
        "border",
        "cellpadding",
        "cellspacing",
        "checked",
        "cite",
        "colspan",
        "controls",
        "datetime",
        "default",
        "dir",
        "disabled",
        "headers",
        "height",
        "href",
        "hreflang",
        "id",
        "kind",
        "label",
        "lang",
        "loop",
        "muted",
        "name",
        "open",
        "playsinline",
        "poster",
        "preload",
        "rel",
        "reversed",
        "rowspan",
        "scope",
        "span",
        "src",
        "start",
        "summary",
        "target",
        "title",
        "type",
        "valign",
        "value",
        "width",
    ],
    xml_self_closing: false,
    urls: true,
};

/// The MathML the LaTeX converter emits (plain presentation MathML, no URLs, no scripts).
pub static MATHML: Policy = Policy {
    tags: &[
        "math",
        "menclose",
        "merror",
        "mfenced",
        "mfrac",
        "mi",
        "mmultiscripts",
        "mn",
        "mo",
        "mover",
        "mpadded",
        "mphantom",
        "mprescripts",
        "mroot",
        "mrow",
        "ms",
        "mspace",
        "msqrt",
        "mstyle",
        "msub",
        "msubsup",
        "msup",
        "mtable",
        "mtd",
        "mtext",
        "mtr",
        "munder",
        "munderover",
        "none",
        "semantics",
        "annotation",
    ],
    void: &[],
    drop_with_content: &[
        "annotation-xml",
        "maction",
        "mglyph",
        "script",
        "style",
        "svg",
        "iframe",
        "object",
        "embed",
        "template",
    ],
    attrs: &[
        "accent",
        "accentunder",
        "align",
        "bevelled",
        "close",
        "columnalign",
        "columnlines",
        "columnspacing",
        "columnspan",
        "denomalign",
        "depth",
        "dir",
        "display",
        "displaystyle",
        "fence",
        "form",
        "frame",
        "height",
        "largeop",
        "linethickness",
        "lspace",
        "mathsize",
        "mathvariant",
        "maxsize",
        "minsize",
        "movablelimits",
        "notation",
        "numalign",
        "open",
        "rowalign",
        "rowlines",
        "rowspacing",
        "rowspan",
        "rspace",
        "scriptlevel",
        "separator",
        "separators",
        "stretchy",
        "symmetric",
        "voffset",
        "width",
        "xmlns",
    ],
    xml_self_closing: true,
    urls: false,
};

fn has(list: &[&str], name: &str) -> bool {
    list.contains(&name)
}

/// One parsed attribute: name (lower-cased), value (`None` = boolean attribute), raw quoting
/// already stripped and entities decoded for the few that matter in URLs.
struct Attr {
    name: String,
    value: Option<String>,
}

/// A parsed open tag.
struct OpenTag {
    name: String,
    attrs: Vec<Attr>,
    self_closing: bool,
    /// Byte offset just past the `>`.
    end: usize,
}

/// Parse an open tag starting at `start` (`bytes[start] == b'<'`, `bytes[start+1]` alphabetic).
/// `None` when it is not terminated.
fn parse_open_tag(html: &str, start: usize) -> Option<OpenTag> {
    let bytes = html.as_bytes();
    let mut i = start + 1;
    let name_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'-') {
        i += 1;
    }
    let name = html[name_start..i].to_ascii_lowercase();
    if name.is_empty() || (i < bytes.len() && !(bytes[i].is_ascii_whitespace() || bytes[i] == b'/' || bytes[i] == b'>'))
    {
        return None;
    }
    let mut attrs = Vec::new();
    let mut self_closing = false;
    loop {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        if bytes[i] == b'>' {
            i += 1;
            break;
        }
        if bytes[i] == b'/' {
            self_closing = true;
            i += 1;
            continue;
        }
        let an = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() && !matches!(bytes[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        if i == an {
            // A stray character that cannot start an attribute name (`"`, `'`, `=`): skip it.
            i += 1;
            continue;
        }
        let aname = html[an..i].to_ascii_lowercase();
        let mut j = i;
        while j < bytes.len() && bytes[j].is_ascii_whitespace() {
            j += 1;
        }
        let mut value: Option<String> = None;
        if j < bytes.len() && bytes[j] == b'=' {
            i = j + 1;
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
                if i >= bytes.len() {
                    return None;
                }
                value = Some(decode_entities(&html[vs..i]));
                i += 1;
            } else {
                let vs = i;
                while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                    i += 1;
                }
                value = Some(decode_entities(&html[vs..i]));
            }
        }
        attrs.push(Attr { name: aname, value });
    }
    Some(OpenTag { name, attrs, self_closing, end: i })
}

/// The handful of entities that show up in attribute values.
fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&amp;", "&").replace("&quot;", "\"").replace("&#39;", "'").replace("&lt;", "<").replace("&gt;", ">")
}

/// Find the end (offset past `>`) of the closing tag `</name…>` at or after `from`,
/// case-insensitively; `None` when there is none.
fn find_close(html: &str, from: usize, name: &str) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut at = from;
    while let Some(pos) = html[at..].find("</") {
        let abs = at + pos;
        let after = abs + 2 + name.len();
        if after <= bytes.len()
            && html.is_char_boundary(after)
            && html[abs + 2..after].eq_ignore_ascii_case(name)
            && (after == bytes.len() || bytes[after].is_ascii_whitespace() || bytes[after] == b'>')
        {
            return Some(html[after..].find('>').map(|p| after + p + 1).unwrap_or(bytes.len()));
        }
        at = abs + 2;
    }
    None
}

/// `&name;` / `&#123;` / `&#x1F;` at `i` → the offset past `;`.
fn entity_end(bytes: &[u8], i: usize) -> Option<usize> {
    let mut j = i + 1;
    if j < bytes.len() && bytes[j] == b'#' {
        j += 1;
        let hex = j < bytes.len() && (bytes[j] == b'x' || bytes[j] == b'X');
        if hex {
            j += 1;
        }
        let ds = j;
        while j < bytes.len() && (if hex { bytes[j].is_ascii_hexdigit() } else { bytes[j].is_ascii_digit() }) {
            j += 1;
        }
        if j == ds || j - ds > 8 || j >= bytes.len() || bytes[j] != b';' {
            return None;
        }
        return Some(j + 1);
    }
    let ns = j;
    while j < bytes.len() && bytes[j].is_ascii_alphanumeric() {
        j += 1;
    }
    if j == ns || j - ns > 32 || j >= bytes.len() || bytes[j] != b';' {
        return None;
    }
    Some(j + 1)
}

/// Sanitize `html` under `policy`, appending to `out`. `manifest` rewrites local Asset references
/// (HTML policy only).
pub fn sanitize(html: &str, policy: &Policy, manifest: &Manifest<'_>, out: &mut String) {
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => {
                let rest = &html[i..];
                if rest.starts_with("<!--") {
                    i = rest.find("-->").map(|p| i + p + 3).unwrap_or(bytes.len());
                } else if rest.starts_with("<!") || rest.starts_with("<?") {
                    i = rest.find('>').map(|p| i + p + 1).unwrap_or(bytes.len());
                } else if rest.starts_with("</") && rest.len() > 2 && rest.as_bytes()[2].is_ascii_alphabetic() {
                    let ns = i + 2;
                    let mut j = ns;
                    while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'-') {
                        j += 1;
                    }
                    let name = html[ns..j].to_ascii_lowercase();
                    match html[j..].find('>') {
                        Some(p) => {
                            if has(policy.tags, &name) && !has(policy.void, &name) {
                                out.push_str("</");
                                out.push_str(&name);
                                out.push('>');
                            }
                            i = j + p + 1;
                        }
                        None => {
                            out.push_str("&lt;");
                            i += 1;
                        }
                    }
                } else if rest.len() > 1 && rest.as_bytes()[1].is_ascii_alphabetic() {
                    match parse_open_tag(html, i) {
                        Some(tag) => {
                            if has(policy.drop_with_content, &tag.name) {
                                i = find_close(html, tag.end, &tag.name).unwrap_or(bytes.len());
                            } else {
                                if has(policy.tags, &tag.name) {
                                    emit_tag(&tag, policy, manifest, out);
                                }
                                i = tag.end;
                            }
                        }
                        None => {
                            out.push_str("&lt;");
                            i += 1;
                        }
                    }
                } else {
                    out.push_str("&lt;");
                    i += 1;
                }
            }
            b'>' => {
                out.push_str("&gt;");
                i += 1;
            }
            b'&' => match entity_end(bytes, i) {
                Some(end) => {
                    out.push_str(&html[i..end]);
                    i = end;
                }
                None => {
                    out.push_str("&amp;");
                    i += 1;
                }
            },
            _ => {
                // Copy one UTF-8 character.
                let ch = html[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
}

/// When `html` (one inline raw-HTML event) is exactly an open tag of an element the policy drops
/// with its content and the closing tag is not in the same event, its name — the caller then
/// drops the inline events up to the matching `</name>` (see `dropped_close_tag`).
pub fn dropped_open_tag(html: &str, policy: &Policy) -> Option<String> {
    let t = html.trim();
    if !t.starts_with('<') {
        return None;
    }
    let tag = parse_open_tag(t, 0)?;
    if !has(policy.drop_with_content, &tag.name) {
        return None;
    }
    if find_close(t, tag.end, &tag.name).is_some() {
        return None;
    }
    Some(tag.name)
}

/// Whether `html` (one inline raw-HTML event) is the closing tag `</name>`.
pub fn is_close_tag(html: &str, name: &str) -> bool {
    let t = html.trim().to_ascii_lowercase();
    t.strip_prefix("</").and_then(|r| r.strip_prefix(name)).is_some_and(|r| r.trim_start().starts_with('>'))
}

/// Which attributes carry a URL, per element (`src`-like ones are "media" for the `data:` rule).
fn url_attr(tag: &str, attr: &str) -> Option<bool> {
    match (tag, attr) {
        ("a", "href") | ("area", "href") => Some(false),
        ("img", "src") | ("source", "src") | ("video", "src") | ("audio", "src") | ("track", "src") => Some(true),
        ("video", "poster") => Some(true),
        ("q", "cite") | ("blockquote", "cite") | ("del", "cite") | ("ins", "cite") => Some(false),
        _ => None,
    }
}

fn emit_tag(tag: &OpenTag, policy: &Policy, manifest: &Manifest<'_>, out: &mut String) {
    let name = tag.name.as_str();
    let mut attrs: Vec<(String, Option<String>)> = Vec::new();
    let mut missing_ref: Option<String> = None;
    let mut asset_dims: Option<(Option<u32>, Option<u32>)> = None;
    for a in &tag.attrs {
        let an = a.name.as_str();
        if an.starts_with("on") || an.starts_with("data-") || !has(policy.attrs, an) {
            continue;
        }
        if attrs.iter().any(|(n, _)| n == an) {
            continue; // first occurrence wins
        }
        let mut value = a.value.clone();
        if policy.urls
            && let Some(media) = url_attr(name, an)
        {
            let Some(v) = value.as_deref() else { continue };
            match manifest.resolve(v) {
                Target::Asset(asset) if asset.missing => {
                    missing_ref = Some(asset.reference.clone());
                    continue; // the attribute is dropped; the element is marked below
                }
                Target::Asset(asset) => {
                    let url = if name == "img" { asset.display_url() } else { asset.url() };
                    let Some(url) = url else { continue };
                    if matches!(name, "img" | "video") && asset.kind() != Kind::Other {
                        asset_dims = Some((asset.width, asset.height));
                    }
                    value = Some(url);
                }
                Target::External => {
                    if !url_allowed(v, media) {
                        continue;
                    }
                }
                Target::AsWritten => {}
            }
        }
        attrs.push((an.to_string(), value));
    }
    if name == "img"
        && let Some(reference) = &missing_ref
    {
        let alt = attrs.iter().find(|(n, _)| n == "alt").and_then(|(_, v)| v.clone()).unwrap_or_default();
        push_missing_placeholder(out, reference, &alt);
        return;
    }
    out.push('<');
    out.push_str(name);
    for (n, v) in &attrs {
        match v {
            Some(v) => push_attr(out, n, v),
            None => {
                out.push(' ');
                out.push_str(n);
            }
        }
    }
    if let Some((w, h)) = asset_dims {
        if let Some(w) = w
            && !attrs.iter().any(|(n, _)| n == "width")
        {
            push_attr(out, "width", &w.to_string());
        }
        if let Some(h) = h
            && !attrs.iter().any(|(n, _)| n == "height")
        {
            push_attr(out, "height", &h.to_string());
        }
    }
    if let Some(reference) = &missing_ref {
        out.push_str(" class=\"asset-missing\"");
        push_attr(out, "data-ref", reference);
    }
    if tag.self_closing && policy.xml_self_closing {
        out.push_str("/>");
    } else {
        out.push('>');
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::assets::Asset;

    fn clean(html: &str) -> String {
        let mut out = String::new();
        sanitize(html, &HTML, &Manifest::default(), &mut out);
        out
    }

    #[test]
    fn scripts_styles_handlers_and_bad_urls_never_survive() {
        assert_eq!(clean("<script>alert(1)</script>after"), "after");
        assert_eq!(clean("<SCRIPT src=x>\n</SCRIPT >tail"), "tail");
        assert_eq!(clean("<style>p{color:red}</style><p style=\"x\">t</p>"), "<p>t</p>");
        assert_eq!(clean("<img src=x onerror=\"alert(1)\" alt=a>"), "<img src=\"x\" alt=\"a\">");
        assert_eq!(clean("<a href=\"javascript:alert(1)\">x</a>"), "<a>x</a>");
        assert_eq!(clean("<a href=\"JavaScript:alert(1)\">x</a>"), "<a>x</a>");
        assert_eq!(clean("<a href=\"data:text/html,x\">x</a>"), "<a>x</a>");
        assert_eq!(clean("<img src=\"data:image/png;base64,AA\">"), "<img src=\"data:image/png;base64,AA\">");
        assert_eq!(clean("<iframe src=x>inner</iframe>ok"), "ok");
        assert_eq!(clean("<object data=x>fallback</object>"), "");
        assert_eq!(clean("<form action=x><input type=text><button>go</button></form>"), "go");
        assert_eq!(clean("<div class=\"c\" id=\"i\" data-l=\"1-1\">t</div>"), "<div id=\"i\">t</div>");
        assert_eq!(clean("<span srcset=\"a 1x\" style=\"c\">t</span>"), "<span>t</span>");
        assert_eq!(clean("<!-- hidden --><!DOCTYPE html><?xml x?>t"), "t");
        assert_eq!(clean("<svg><script>1</script></svg>t"), "t");
        assert_eq!(clean("<math><mi>x</mi></math>t"), "t", "raw MathML is dropped; math comes from `$`");
        assert_eq!(clean("<template><p>x</p></template>t"), "t");
        assert_eq!(clean("<unknown-tag attr=1>keep</unknown-tag>"), "keep");
        assert_eq!(clean("<center><font color=red>t</font></center>"), "t");
    }

    #[test]
    fn allowed_markup_is_kept_normalised() {
        assert_eq!(
            clean("<DETAILS OPEN><Summary>s</Summary>body</DETAILS>"),
            "<details open><summary>s</summary>body</details>"
        );
        assert_eq!(clean("<br/><br /><hr>"), "<br><br><hr>");
        assert_eq!(clean("<td colspan=2 align='center'>x</td>"), "<td colspan=\"2\" align=\"center\">x</td>");
        assert_eq!(
            clean("a < b && c > d &amp; &#169; &#x1F600; &bogus"),
            "a &lt; b &amp;&amp; c &gt; d &amp; &#169; &#x1F600; &amp;bogus"
        );
        assert_eq!(
            clean("<a href=\"https://x.y/?a=1&b=2\" title='t\"q'>l</a>"),
            "<a href=\"https://x.y/?a=1&amp;b=2\" title=\"t&quot;q\">l</a>"
        );
        assert_eq!(clean("<img alt=\"only alt\">"), "<img alt=\"only alt\">");
        assert_eq!(clean("<p>unterminated <b"), "<p>unterminated &lt;b");
        assert_eq!(
            clean("<video controls src=\"https://x/v.mp4\" poster=\"https://x/p.png\"></video>"),
            "<video controls src=\"https://x/v.mp4\" poster=\"https://x/p.png\"></video>"
        );
        assert_eq!(clean("Ünïcödé <em>é</em>"), "Ünïcödé <em>é</em>");
        assert_eq!(clean("<a href=\"#frag\" name=\"n\" id=\"i\">x</a>"), "<a href=\"#frag\" name=\"n\" id=\"i\">x</a>");
        assert_eq!(clean("<a href=\"x\" href=\"y\">d</a>"), "<a href=\"x\">d</a>", "first occurrence wins");
    }

    #[test]
    fn asset_references_are_rewritten_like_markdown_ones() {
        let assets = vec![
            Asset {
                reference: "img/a.png".into(),
                hash: Some("h1".into()),
                content_type: "image/png".into(),
                width: Some(10),
                height: Some(20),
                ..Default::default()
            },
            Asset {
                reference: "img/b.heic".into(),
                hash: Some("h2".into()),
                content_type: "image/heic".into(),
                derivative: Some("d2".into()),
                ..Default::default()
            },
            Asset {
                reference: "img/gone.png".into(),
                content_type: "image/png".into(),
                missing: true,
                ..Default::default()
            },
            Asset {
                reference: "clip.mp4".into(),
                hash: Some("h3".into()),
                content_type: "video/mp4".into(),
                width: Some(640),
                height: Some(360),
                ..Default::default()
            },
            Asset {
                reference: "notes.md".into(),
                hash: Some("h4".into()),
                content_type: "text/markdown".into(),
                ..Default::default()
            },
        ];
        let m = Manifest::new(&assets);
        let clean = |html: &str| {
            let mut out = String::new();
            sanitize(html, &HTML, &m, &mut out);
            out
        };
        assert_eq!(clean("<img src=\"img/a.png\" alt=a>"), "<img src=\"/a/h1\" alt=\"a\" width=\"10\" height=\"20\">");
        assert_eq!(
            clean("<img src=\"./img/a.png\" width=5>"),
            "<img src=\"/a/h1\" width=\"5\" height=\"20\">",
            "own dimensions win"
        );
        assert_eq!(clean("<img src=\"img/b.heic\">"), "<img src=\"/a/h2?derivative=1\">");
        assert_eq!(
            clean("<img src=\"img/gone.png\" alt=\"Arch\">"),
            "<span class=\"asset-missing\" data-ref=\"img/gone.png\">missing asset: img/gone.png (Arch)</span>"
        );
        assert_eq!(
            clean("<video src=\"clip.mp4\" controls></video>"),
            "<video src=\"/a/h3\" controls width=\"640\" height=\"360\"></video>"
        );
        assert_eq!(clean("<source src=\"clip.mp4\" type=\"video/mp4\">"), "<source src=\"/a/h3\" type=\"video/mp4\">");
        assert_eq!(clean("<a href=\"notes.md#top\">n</a>"), "<a href=\"/a/h4\">n</a>");
        assert_eq!(
            clean("<a href=\"img/gone.png\">n</a>"),
            "<a class=\"asset-missing\" data-ref=\"img/gone.png\">n</a>"
        );
        assert_eq!(
            clean("<video src=\"img/gone.png\"></video>"),
            "<video class=\"asset-missing\" data-ref=\"img/gone.png\"></video>"
        );
        assert_eq!(
            clean("<a href=\"other.docx\">n</a>"),
            "<a href=\"other.docx\">n</a>",
            "not in the manifest: as written"
        );
        assert_eq!(clean("<a href=\"/abs/x.png\">n</a>"), "<a href=\"/abs/x.png\">n</a>");
        assert_eq!(clean("<img src=\"https://x/y.png\">"), "<img src=\"https://x/y.png\">");
    }

    #[test]
    fn mathml_policy_keeps_presentation_markup_only() {
        let clean = |s: &str| {
            let mut out = String::new();
            sanitize(s, &MATHML, &Manifest::default(), &mut out);
            out
        };
        assert_eq!(
            clean("<math display=\"block\"><mo><</mo><mspace width=\"1em\"/><mtext>a<b}</mtext></math>"),
            "<math display=\"block\"><mo>&lt;</mo><mspace width=\"1em\"/><mtext>a&lt;b}</mtext></math>"
        );
        assert_eq!(clean("<mtext><script>x</script></mtext>"), "<mtext></mtext>");
        assert_eq!(clean("<mi onclick=\"x\" mathvariant=\"normal\">x</mi>"), "<mi mathvariant=\"normal\">x</mi>");
        assert_eq!(clean("<annotation-xml><b>x</b></annotation-xml><mi>y</mi>"), "<mi>y</mi>");
    }
}
