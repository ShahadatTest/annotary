//! The renderer over the Markdown corpus in `fixtures/render/`: `insta` HTML snapshots (one per
//! file; `*.manifest.json` beside a file is its manifest), the output guardrails (no `<style>` /
//! `<script>` / `style=` / handlers; `data-l` on every block the renderer opens; sane ranges),
//! byte-stability across runs, and the display title.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use annotary_renderer::{Asset, Options, RENDERER_VERSION, render, title};

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/render")
}

fn corpus() -> Vec<(String, String, Vec<Asset>)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(corpus_dir())
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "md") && p.file_name().is_some_and(|n| n != "README.md"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|p| {
            let name = p.file_name().unwrap().to_string_lossy().to_string();
            let md = std::fs::read_to_string(&p).unwrap();
            let manifest = p.with_extension("manifest.json");
            let assets = if manifest.is_file() {
                let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
                Asset::from_manifest(&v)
            } else {
                Vec::new()
            };
            (name, md, assets)
        })
        .collect()
}

fn embeds_for(name: &str) -> bool {
    !name.contains("comment-body")
}

fn render_file(name: &str, md: &str, assets: &[Asset]) -> String {
    render(md, &Options { assets, embeds: embeds_for(name) }).html
}

// spec: S09-01, S09-04, S09-05, S09-06
#[test]
fn corpus_snapshots() {
    assert!(corpus().len() >= 15, "the corpus covers every syntax area and every Asset type");
    let mut settings = insta::Settings::clone_current();
    settings.set_snapshot_path("snapshots");
    settings.set_prepend_module_to_snapshot(false);
    settings.set_omit_expression(true);
    settings.bind(|| {
        for (name, md, assets) in corpus() {
            let html = render_file(&name, &md, &assets);
            insta::assert_snapshot!(name.clone(), html);
        }
    });
}

/// Every `<tag …>` open tag in `html`: (name, attribute text).
fn open_tags(html: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let b = html.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' && i + 1 < b.len() && b[i + 1].is_ascii_alphabetic() {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
                j += 1;
            }
            let name = html[i + 1..j].to_string();
            let mut k = j;
            let mut quote: Option<u8> = None;
            while k < b.len() {
                match (quote, b[k]) {
                    (Some(q), c) if c == q => quote = None,
                    (Some(_), _) => {}
                    (None, b'"') | (None, b'\'') => quote = Some(b[k]),
                    (None, b'>') => break,
                    _ => {}
                }
                k += 1;
            }
            out.push((name, html[j..k].to_string()));
            i = k;
        } else {
            i += 1;
        }
    }
    out
}

const BLOCKS: &[&str] = &[
    "p",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "ul",
    "ol",
    "li",
    "blockquote",
    "pre",
    "table",
    "thead",
    "tbody",
    "tr",
    "th",
    "td",
    "hr",
    "section",
    "dl",
    "dt",
    "dd",
];

/// Raw block-level HTML written in the source keeps its own tags (no `data-l`; the guardrail
/// applies to what the renderer opens).
const RAW_HTML_FIXTURES: &[&str] = &["09-html.md"];

// spec: S09-02, S09-03
#[test]
fn guardrails_no_styles_scripts_and_data_l_on_every_block() {
    for (name, md, assets) in corpus() {
        let html = render_file(&name, &md, &assets);
        let lower = html.to_ascii_lowercase();
        for forbidden in
            ["<style", "<script", "<iframe", "<form", "<input type=\"text\"", "=\"javascript:", "=\"vbscript:"]
        {
            assert!(!lower.contains(forbidden), "{name}: output contains {forbidden:?}");
        }
        let line_count = md.lines().count().max(1);
        for (tag, attrs) in open_tags(&html) {
            assert!(!attrs.contains(" style="), "{name}: <{tag}{attrs}> carries a style attribute");
            assert!(!attrs.split_whitespace().any(|a| a.starts_with("on")), "{name}: <{tag}{attrs}> carries a handler");
            if let Some(rest) = attrs.split("data-l=\"").nth(1) {
                let range = rest.split('"').next().unwrap();
                let (a, b) = range.split_once('-').unwrap_or_else(|| panic!("{name}: bad data-l {range}"));
                let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
                assert!(
                    a >= 1 && a <= b && b <= line_count,
                    "{name}: <{tag} data-l=\"{range}\"> out of range (doc has {line_count} lines)"
                );
            } else if BLOCKS.contains(&tag.as_str()) && !RAW_HTML_FIXTURES.contains(&name.as_str()) {
                panic!("{name}: block <{tag}{attrs}> has no data-l");
            }
        }
        // Blocks with content-bearing source lines never point at a blank line.
        for (tag, attrs) in open_tags(&html) {
            if !matches!(tag.as_str(), "p" | "li" | "h1" | "h2" | "h3" | "pre" | "hr") {
                continue;
            }
            let Some(rest) = attrs.split("data-l=\"").nth(1) else { continue };
            let range = rest.split('"').next().unwrap();
            let (a, b) = range.split_once('-').unwrap();
            let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
            let lines: Vec<&str> = md.lines().collect();
            assert!(!lines[a - 1].trim().is_empty(), "{name}: <{tag}> data-l starts on a blank line ({a})");
            assert!(!lines[b - 1].trim().is_empty(), "{name}: <{tag}> data-l ends on a blank line ({b})");
        }
    }
    // The raw-HTML fixture: what must and must not survive sanitizing.
    let (_, md, assets) = corpus().into_iter().find(|(n, _, _)| n == "09-html.md").unwrap();
    let html = render_file("09-html.md", &md, &assets);
    for kept in [
        "<details>",
        "<summary>",
        "<kbd>",
        "<abbr title=\"HyperText\">",
        "<span>span loses",
        "<div align=\"center\">",
        "<img src=\"/a/",
        "<video src=\"/a/",
        "<source src=\"/a/",
        "<a href=\"/a/",
        "class=\"asset-missing\"",
        "<table>",
        "obsolete tags lose their tags, keep their text",
        "unknown elements keep their text",
        "a stray &lt; b and a &gt; c and &amp;&amp; here",
    ] {
        assert!(html.contains(kept), "09-html.md: expected {kept:?} in\n{html}");
    }
    for gone in [
        "alert(\"gone\")",
        "onclick",
        "onerror",
        "color:red",
        "class=\"x\"",
        "data-l=\"9-9\"",
        "dropped with content",
        "evil.example",
        "<button",
        "<font",
        "<center",
        "<unknown-element",
        "an inline comment",
    ] {
        assert!(!html.contains(gone), "09-html.md: {gone:?} survived in\n{html}");
    }
}

// spec: S09-07
#[test]
fn output_is_byte_stable_across_runs() {
    let corpus = corpus();
    let first: Vec<String> = corpus.iter().map(|(n, md, a)| render_file(n, md, a)).collect();
    // A second pass in reverse order, and a third from another thread: same bytes.
    let second: Vec<String> =
        corpus.iter().rev().map(|(n, md, a)| render_file(n, md, a)).collect::<Vec<_>>().into_iter().rev().collect();
    assert_eq!(first, second);
    let third = std::thread::spawn({
        let corpus = corpus.clone();
        move || corpus.iter().map(|(n, md, a)| render_file(n, md, a)).collect::<Vec<String>>()
    })
    .join()
    .unwrap();
    assert_eq!(first, third);
    assert_eq!(RENDERER_VERSION.split('.').count(), 3);
}

// spec: S09-05
#[test]
fn display_title_is_front_matter_then_first_h1() {
    let titles: BTreeMap<String, Option<String>> = corpus().into_iter().map(|(n, md, _)| (n, title(&md))).collect();
    let t = |n: &str| titles.get(n).unwrap().clone();
    assert_eq!(t("07-front-matter.md").as_deref(), Some("Quoted: The Title"));
    assert_eq!(t("08-title-h1.md").as_deref(), Some("The First H1 wins"), "plain text, whitespace collapsed");
    assert_eq!(t("14-front-matter-only.md").as_deref(), Some("Front matter only"));
    assert_eq!(t("01-basics.md").as_deref(), Some("Basics"));
    assert_eq!(t("13-empty.md"), None);
    assert_eq!(t("11-comment-body.md"), None);
    // The front matter itself is never rendered.
    let (_, md, _) = corpus().into_iter().find(|(n, _, _)| n == "07-front-matter.md").unwrap();
    let html = render(&md, &Options::default()).html;
    assert!(!html.contains("author") && !html.contains("Quoted"), "{html}");
    assert!(html.starts_with("<h1 data-l=\"7-7\""), "{html}");
}

// spec: S09-06
#[test]
fn every_asset_type_has_a_rendered_form() {
    let (_, md, assets) = corpus().into_iter().find(|(n, _, _)| n == "10-assets.md").unwrap();
    let html = render(&md, &Options { assets: &assets, embeds: true }).html;
    for a in &assets {
        if a.missing {
            assert!(
                html.contains(&format!("<span class=\"asset-missing\" data-ref=\"{}\">", a.reference)),
                "{}",
                a.reference
            );
            assert!(
                html.contains(&format!("<a class=\"asset-missing\" data-ref=\"{}\">", a.reference)),
                "{}",
                a.reference
            );
            continue;
        }
        let hash = a.hash.as_deref().unwrap();
        assert!(html.contains(&format!("/a/{hash}")), "{} ({}) is not referenced by hash", a.reference, a.content_type);
        if a.derivative.is_some() {
            assert!(html.contains(&format!("<img src=\"/a/{hash}?derivative=1\"")), "{}: derivative", a.reference);
        }
        if let (Some(w), Some(h)) = (a.width, a.height) {
            assert!(html.contains(&format!("width=\"{w}\" height=\"{h}\"")), "{}: dimensions", a.reference);
        }
    }
    for expected in [
        "<video controls preload=\"metadata\" src=\"/a/",
        "<audio controls preload=\"metadata\" src=\"/a/",
        "<object type=\"application/pdf\" data=\"/a/",
        "<a href=\"design.docx\">design</a>",
        "<img src=\"data.bin\" alt=\"raw\">",
        "<img src=\"https://example.com/i.png\" alt=\"Ext\">",
        "<img src=\"data:image/png;base64,AAAA\" alt=\"Data\">",
        "<img alt=\"Bad\">",
        "<img src=\"/srv/pics/x.png\" alt=\"absolute\">",
    ] {
        assert!(html.contains(expected), "expected {expected:?} in\n{html}");
    }
    assert!(!html.contains("javascript:"));
}
