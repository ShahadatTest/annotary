//! The pure generator: `(Scale, seed)` → a [`Plan`] — the Markdown of every Version, the images
//! and the ordered steps that seed a server. Everything is a function of the seed (the PRNG in
//! `rng.rs`, no hash-map iteration, no clock), so two runs produce byte-identical plans.
//!
//! The Document is modelled as a list of lines with a kind (heading, paragraph, list item,
//! table row, code, quote, image, blank); a Version edit works on that model — a paragraph
//! inserted, a paragraph deleted, a word inserted, whitespace changed — and the model renders
//! back to text. Threads are chosen on the text of the Version that is head when they are
//! created, so every `L:C-L:C` range is on head and non-empty. To guarantee a *mix* of placement
//! statuses at any scale, some edits are targeted: a word inserted **inside** an existing span
//! (⇒ refined), the line of another span deleted (⇒ orphaned); the rest of the Threads shift
//! (⇒ anchored). The server's re-anchoring decides the truth; the seeder reads it back.

use crate::rng::Rng;

/// How big a generated fixture is. [`Scale::FULL`] is the perf suite's fixture; [`Scale::SMOKE`]
/// the small one `cargo test` runs so the generator does not rot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Scale {
    /// Target source lines of v1 (the generator stops adding sections once reached).
    pub lines: usize,
    /// Target words of v1 (sections keep being added until both targets are met).
    pub words: usize,
    /// Threads created in total across the Versions (+ one per image, + the Document-level ones).
    pub threads: usize,
    /// Number of Versions pushed (≥ 2).
    pub versions: u32,
    /// Images referenced by the Document (real PNGs with dimensions).
    pub images: usize,
    /// Small Documents pushed alongside, so the list page has a full page of rows.
    pub list_docs: usize,
}

impl Scale {
    /// The perf fixture: ≥ 5,000 lines / ~50k words / ≥ 500 Threads / 5 Versions.
    pub const FULL: Scale = Scale { lines: 5_200, words: 50_000, threads: 520, versions: 5, images: 4, list_docs: 60 };
    /// The smoke fixture (`cargo test`): the same shape, ~1/40 the size.
    pub const SMOKE: Scale = Scale { lines: 140, words: 1_300, threads: 16, versions: 3, images: 2, list_docs: 3 };

    pub fn parse(name: &str) -> Option<Scale> {
        match name {
            "full" => Some(Scale::FULL),
            "smoke" => Some(Scale::SMOKE),
            _ => None,
        }
    }
}

/// The handles the plan mentions and posts as: the owner (first-run setup) and two agents the
/// seeder adds with `member add`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Handles {
    pub owner: String,
    pub agents: [String; 2],
}

impl Default for Handles {
    fn default() -> Handles {
        Handles { owner: "rowan".into(), agents: ["scribe".into(), "linter".into()] }
    }
}

/// Who performs a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Who {
    Owner,
    Agent1,
    Agent2,
}

impl Who {
    pub fn handle<'a>(&self, h: &'a Handles) -> &'a str {
        match self {
            Who::Owner => &h.owner,
            Who::Agent1 => &h.agents[0],
            Who::Agent2 => &h.agents[1],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Subject {
    /// `L:C-L:C` on the head at creation time, plus the exact source text there.
    Span {
        range: String,
        exact: String,
    },
    Document,
    /// The manifest `ref` (an image of the Document).
    Asset {
        reference: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Step {
    /// Push `versions[version - 1]` (v1 creates the Document).
    Push {
        version: u32,
        message: String,
    },
    /// A new Thread (the k-th `Thread` step becomes `t{k+1}`).
    Thread {
        by: Who,
        subject: Subject,
        body: String,
    },
    /// A Reply on the Thread created by the `thread`-th `Thread` step (0-based).
    Reply {
        thread: usize,
        by: Who,
        body: String,
        resolve: bool,
    },
    Resolve {
        thread: usize,
        by: Who,
    },
    Reopen {
        thread: usize,
        by: Who,
    },
    /// Tombstone a Comment: the root (`reply` = None) or the n-th Reply (1-based) of a Thread.
    Delete {
        thread: usize,
        reply: Option<usize>,
        by: Who,
    },
    /// Take an Action on the head (`version` is what the head is at that point).
    Act {
        by: Who,
        action: String,
        version: u32,
    },
}

/// A generated image: a real PNG.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    /// The reference as written in the Markdown (`img/diagram-1.png`).
    pub reference: String,
    pub width: u32,
    pub height: u32,
    pub png: Vec<u8>,
}

/// What the generator produced (for reports and the smoke assertions).
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Stats {
    pub lines_v1: usize,
    pub words_v1: usize,
    pub lines_head: usize,
    pub words_head: usize,
    pub versions: u32,
    pub threads: usize,
    pub span_threads: usize,
    pub document_threads: usize,
    pub asset_threads: usize,
    pub replies: usize,
    pub resolves: usize,
    pub reopens: usize,
    pub deletes: usize,
    pub mentions: usize,
    pub actions: usize,
    /// Total characters of every body (the server caps it at 100k per Document).
    pub body_chars: usize,
    /// Threads per Version at creation (index 0 = v1).
    pub threads_per_version: Vec<usize>,
    /// What the generator *intended*: spans it edited inside / deleted (the server decides).
    pub intended_refined: usize,
    pub intended_orphaned: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub name: String,
    pub handles: Handles,
    /// Markdown of v1..vN.
    pub versions: Vec<String>,
    pub images: Vec<Image>,
    pub steps: Vec<Step>,
    /// `(name, markdown)` of the small list Documents; every third gets a Document-level Thread,
    /// every fifth an `approve`.
    pub list_docs: Vec<(String, String)>,
    pub stats: Stats,
}

// ---------------------------------------------------------------------------------------------
// Vocabulary
// ---------------------------------------------------------------------------------------------

const WORDS: &[&str] = &[
    "the",
    "a",
    "an",
    "of",
    "to",
    "and",
    "in",
    "for",
    "on",
    "with",
    "as",
    "by",
    "from",
    "that",
    "this",
    "which",
    "document",
    "version",
    "thread",
    "comment",
    "reply",
    "anchor",
    "placement",
    "member",
    "owner",
    "agent",
    "vault",
    "server",
    "client",
    "store",
    "event",
    "cursor",
    "token",
    "handle",
    "mention",
    "action",
    "intent",
    "render",
    "markdown",
    "html",
    "block",
    "span",
    "line",
    "column",
    "range",
    "quote",
    "history",
    "sparse",
    "diff",
    "hunk",
    "histogram",
    "character",
    "word",
    "survivor",
    "sticky",
    "edge",
    "inside",
    "outside",
    "refined",
    "orphaned",
    "anchored",
    "resolved",
    "open",
    "head",
    "base",
    "push",
    "pull",
    "commit",
    "upload",
    "manifest",
    "asset",
    "image",
    "video",
    "audio",
    "file",
    "path",
    "hash",
    "blob",
    "derivative",
    "dimension",
    "width",
    "height",
    "cache",
    "budget",
    "latency",
    "paint",
    "layout",
    "viewport",
    "margin",
    "card",
    "strip",
    "overlay",
    "pill",
    "composer",
    "keyboard",
    "binding",
    "shortcut",
    "toggle",
    "banner",
    "select",
    "option",
    "filter",
    "list",
    "table",
    "row",
    "cell",
    "header",
    "footer",
    "section",
    "heading",
    "paragraph",
    "sentence",
    "draft",
    "review",
    "approve",
    "send",
    "back",
    "decide",
    "record",
    "append",
    "only",
    "never",
    "always",
    "once",
    "twice",
    "again",
    "later",
    "earlier",
    "before",
    "after",
    "while",
    "until",
    "when",
    "where",
    "because",
    "so",
    "but",
    "or",
    "not",
    "also",
    "still",
    "already",
    "yet",
    "small",
    "large",
    "fast",
    "slow",
    "cheap",
    "costly",
    "plain",
    "simple",
    "clear",
    "honest",
    "deterministic",
    "stable",
    "portable",
    "local",
    "remote",
    "hosted",
    "cloud",
    "phase",
    "ticket",
    "spec",
    "story",
    "glossary",
    "rule",
    "guard",
    "check",
    "verify",
    "measure",
    "report",
    "print",
    "parse",
    "emit",
    "read",
    "write",
    "delete",
    "restore",
    "purge",
    "retain",
    "keep",
    "drop",
    "skip",
    "count",
    "total",
    "median",
    "run",
    "cold",
    "warm",
    "first",
    "last",
    "next",
    "previous",
    "older",
    "newer",
    "same",
    "other",
    "every",
    "each",
    "some",
    "many",
    "few",
    "none",
    "people",
    "human",
    "machine",
    "tool",
    "shell",
    "terminal",
    "browser",
    "page",
    "tab",
    "window",
    "screen",
    "narrow",
    "wide",
    "column",
    "grid",
    "flow",
    "inline",
    "absolute",
    "relative",
    "sticky",
    "hidden",
    "visible",
    "ready",
    "live",
    "idle",
    "busy",
    "pending",
    "confirmed",
    "rolled",
    "error",
    "hint",
    "message",
    "detail",
    "note",
    "warning",
    "status",
    "seq",
    "wait",
    "poll",
    "stream",
    "watch",
    "wake",
    "park",
    "timeout",
    "retry",
    "resume",
    "naïve",
    "café",
    "résumé",
    "façade",
    "déjà",
    "coöperate",
    "Zürich",
    "São",
    "Paulo",
    "København",
];

const TOPICS: &[&str] = &[
    "Anchors",
    "Versions",
    "Threads",
    "Actions",
    "Vaults",
    "Members",
    "Tokens",
    "Events",
    "Rendering",
    "Assets",
    "Layouts",
    "Keyboard",
    "Budgets",
    "Storage",
    "Portability",
    "Sidecars",
    "Mentions",
    "Housekeeping",
    "Daemon",
    "Export",
    "Login",
    "Sessions",
    "Grants",
    "Rights",
    "Roles",
    "Handles",
    "Placements",
    "Diffing",
    "Tokens",
    "Latency",
    "Paint",
    "Scrolling",
    "Selection",
    "Composer",
    "Live",
    "Waiting",
    "Cursors",
    "Rollout",
    "Risks",
    "Testing",
    "Fixtures",
    "Goldens",
    "Traceability",
    "Phase 2",
    "Phase 3",
    "Backlog",
    "Decisions",
    "Glossary",
];

const LANGS: &[&str] = &["rust", "ts", "sh", "json", "toml"];

const CODE_LINES: &[&str] = &[
    "let mapping = Mapping::new(&text_n, &text_n1);",
    "for t in threads.iter().filter(|t| t.is_open()) {",
    "    placements.push(mapping.place(&span)?);",
    "}",
    "const WIDE = 850;",
    "function layout() { app.classList.add('cd-ready'); }",
    "annotary push plan.md -m \"address review\"",
    "annotary thread reply plan t7 --resolve -m \"done in v5\"",
    "annotary wait plan --on version,action,mention --timeout 30m",
    "{ \"id\": \"k7q2m9xd\", \"version\": 5, \"open_threads\": 3 }",
    "[profiles.agent]",
    "server = \"http://127.0.0.1:7377\"",
    "if status == ORPHANED { continue; }",
    "let exact = extract(&md, &range)?;",
    "return Ok(Placement { status, range, exact });",
    "export async function openDoc(page, id) { await page.goto(`/d/${id}`); }",
    "fn anchor_of(t: &ThreadView) -> Option<&Anchor> { t.anchor.as_ref() }",
    "  - name: perf",
    "    run: make perf",
    "SELECT num, subject, anchor FROM threads WHERE doc_id = ?1 ORDER BY num",
];

const QUOTES: &[&str] = &[
    "Everything feels extremely fast, or it does not ship.",
    "Quote is truth; orphans are visible, never silent.",
    "Compute at the edge: the pusher renders and re-anchors, the server stores and serves.",
    "Agents never need the Sidecar — the CLI is the interface.",
    "Threads and placements are in the first HTML response.",
    "One binary, one store, plain files in and out.",
];

// ---------------------------------------------------------------------------------------------
// The line model
// ---------------------------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Blank,
    H1,
    H2,
    H3,
    Para,
    Item,
    Quote,
    Fence,
    Code,
    TableSep,
    TableRow,
    Image,
}

#[derive(Clone, Debug)]
struct Line {
    id: u32,
    kind: Kind,
    text: String,
}

#[derive(Clone, Debug, Default)]
struct Doc {
    lines: Vec<Line>,
    next_id: u32,
}

impl Doc {
    fn push(&mut self, kind: Kind, text: String) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.lines.push(Line { id, kind, text });
        id
    }
    fn blank(&mut self) {
        self.push(Kind::Blank, String::new());
    }
    fn render(&self) -> String {
        let mut s = String::new();
        for l in &self.lines {
            s.push_str(&l.text);
            s.push('\n');
        }
        s
    }
    fn words(&self) -> usize {
        self.lines.iter().map(|l| l.text.split_whitespace().count()).sum()
    }
    fn index_of(&self, id: u32) -> Option<usize> {
        self.lines.iter().position(|l| l.id == id)
    }
}

fn cap(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// A sentence of `n` words, optionally with a little inline Markdown.
fn sentence(rng: &mut Rng, lo: usize, hi: usize, inline: bool) -> String {
    let n = rng.range(lo, hi);
    let mut ws: Vec<String> = (0..n).map(|_| (*rng.pick(WORDS)).to_string()).collect();
    if inline && n >= 5 {
        let k = rng.below(n - 2) + 1;
        match rng.below(6) {
            0 => ws[k] = format!("`{}`", ws[k]),
            1 => ws[k] = format!("**{}**", ws[k]),
            2 => ws[k] = format!("*{}*", ws[k]),
            3 => ws[k] = format!("[{}](https://example.com/{})", ws[k], ws[k].to_lowercase()),
            _ => {}
        }
    }
    let mut s = cap(&ws.join(" "));
    if n >= 9 && rng.chance(0.4) {
        // a comma somewhere in the middle
        let k = rng.range(3, n - 3);
        let mut parts: Vec<&str> = s.split(' ').collect();
        let joined = format!("{},", parts[k]);
        parts[k] = &joined;
        s = parts.join(" ");
    }
    s.push_str(match rng.below(12) {
        0 => "?",
        1 => " — and that is the point.",
        _ => ".",
    });
    s
}

fn paragraph(rng: &mut Rng, lo: usize, hi: usize) -> String {
    let words = rng.range(lo, hi);
    let mut out = Vec::new();
    let mut left = words;
    while left > 0 {
        let n = rng.range(6, 16).min(left.max(3));
        out.push(sentence(rng, n, n, true));
        left = left.saturating_sub(n);
    }
    out.join(" ")
}

fn body_text(rng: &mut Rng, handles: &Handles, me: Who, mention_p: f64, mentions: &mut usize) -> String {
    let inline = rng.chance(0.3);
    let mut s = sentence(rng, 4, 10, inline);
    if rng.chance(0.18) {
        s.push(' ');
        s.push_str(&sentence(rng, 4, 9, false));
    }
    if rng.chance(mention_p) {
        // mention someone else
        let others: Vec<Who> = [Who::Owner, Who::Agent1, Who::Agent2].into_iter().filter(|w| *w != me).collect();
        let who = *rng.pick(&others);
        s = format!("@{} {}", who.handle(handles), s);
        *mentions += 1;
    }
    s
}

fn pick_who(rng: &mut Rng) -> Who {
    match rng.below(20) {
        0..=7 => Who::Owner,
        8..=14 => Who::Agent1,
        _ => Who::Agent2,
    }
}

// ---------------------------------------------------------------------------------------------
// v1
// ---------------------------------------------------------------------------------------------

fn gen_v1(rng: &mut Rng, scale: &Scale, images: &[Image]) -> Doc {
    let mut d = Doc::default();
    d.push(Kind::H1, "Perf plan: a long review document".into());
    d.blank();
    d.push(Kind::Para, paragraph(rng, 45, 45));
    d.blank();
    let mut image_i = 0;
    let mut section = 0usize;
    loop {
        if d.lines.len() >= scale.lines && d.words() >= scale.words {
            break;
        }
        section += 1;
        let topic = TOPICS[section % TOPICS.len()];
        d.push(Kind::H2, format!("## {topic} ({section})"));
        d.blank();
        for _ in 0..rng.range(1, 3) {
            d.push(Kind::Para, paragraph(rng, 26, 62));
            d.blank();
        }
        if rng.chance(0.55) {
            let ordered = rng.chance(0.3);
            for i in 0..rng.range(4, 9) {
                let marker = if ordered { format!("{}. ", i + 1) } else { "- ".to_string() };
                d.push(Kind::Item, format!("{marker}{}", sentence(rng, 6, 13, true)));
            }
            d.blank();
        }
        if rng.chance(0.28) {
            let cols = rng.range(3, 4);
            let heads: Vec<String> = (0..cols).map(|_| cap(rng.pick(WORDS))).collect();
            d.push(Kind::TableRow, format!("| {} |", heads.join(" | ")));
            d.push(Kind::TableSep, format!("|{}", " --- |".repeat(cols)));
            for _ in 0..rng.range(3, 7) {
                let cells: Vec<String> = (0..cols)
                    .map(|_| {
                        let n = rng.range(1, 3);
                        (0..n).map(|_| *rng.pick(WORDS)).collect::<Vec<_>>().join(" ")
                    })
                    .collect();
                d.push(Kind::TableRow, format!("| {} |", cells.join(" | ")));
            }
            d.blank();
        }
        if rng.chance(0.3) {
            d.push(Kind::Fence, format!("```{}", rng.pick(LANGS)));
            for _ in 0..rng.range(4, 10) {
                d.push(Kind::Code, (*rng.pick(CODE_LINES)).to_string());
            }
            d.push(Kind::Fence, "```".into());
            d.blank();
        }
        if rng.chance(0.22) {
            d.push(Kind::Quote, format!("> {}", rng.pick(QUOTES)));
            if rng.chance(0.5) {
                d.push(Kind::Quote, format!("> {}", sentence(rng, 8, 14, false)));
            }
            d.blank();
        }
        if image_i < images.len() && (rng.chance(0.18) || section >= 12 + image_i * 8) {
            d.push(Kind::Image, format!("![diagram {}]({})", image_i + 1, images[image_i].reference));
            d.blank();
            image_i += 1;
        }
        if rng.chance(0.4) {
            d.push(Kind::H3, format!("### {} in detail", topic));
            d.blank();
            d.push(Kind::Para, paragraph(rng, 25, 55));
            d.blank();
            if rng.chance(0.5) {
                for _ in 0..rng.range(3, 6) {
                    d.push(Kind::Item, format!("- {}", sentence(rng, 5, 11, false)));
                }
                d.blank();
            }
        }
    }
    // Any image the loop did not place yet goes at the end (the smoke scale is small).
    while image_i < images.len() {
        d.push(Kind::Image, format!("![diagram {}]({})", image_i + 1, images[image_i].reference));
        d.blank();
        image_i += 1;
    }
    d
}

// ---------------------------------------------------------------------------------------------
// Spans (threads on a line)
// ---------------------------------------------------------------------------------------------

/// A word token of a line: code-point columns `[start, end)`.
#[derive(Clone, Debug)]
struct Tok {
    start: usize,
    end: usize,
}

fn tokens(text: &str) -> Vec<Tok> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let mut col = 0usize;
    for c in text.chars() {
        if c.is_whitespace() {
            if let Some(s) = start.take() {
                out.push(Tok { start: s, end: col });
            }
        } else if start.is_none() {
            start = Some(col);
        }
        col += 1;
    }
    if let Some(s) = start {
        out.push(Tok { start: s, end: col });
    }
    out
}

fn slice_cp(text: &str, start: usize, end: usize) -> String {
    text.chars().skip(start).take(end - start).collect()
}

/// Tokens a span may cover: not the line's Markdown marker, never a table bar.
fn eligible_tokens(kind: Kind, text: &str) -> Vec<Tok> {
    let toks = tokens(text);
    let skip = match kind {
        Kind::Item | Kind::Quote | Kind::H2 | Kind::H3 | Kind::H1 => 1,
        _ => 0,
    };
    toks.into_iter()
        .skip(skip)
        .filter(|t| {
            let s = slice_cp(text, t.start, t.end);
            s != "|" && s.chars().any(|c| c.is_alphanumeric())
        })
        .collect()
}

/// A span Thread the generator knows about: its line id and the current columns of its span.
#[derive(Clone, Debug)]
struct Known {
    line_id: u32,
    start: usize,
    end: usize,
    /// Number of word tokens in the span (≥ 2 ⇒ a word can be inserted inside).
    words: usize,
    alive: bool,
}

// ---------------------------------------------------------------------------------------------
// Images
// ---------------------------------------------------------------------------------------------

fn make_image(i: usize, rng: &mut Rng) -> Image {
    let sizes: [(u32, u32); 4] = [(640, 360), (800, 450), (480, 320), (320, 240)];
    let (w, h) = sizes[i % sizes.len()];
    let mut img = image::RgbImage::new(w, h);
    let (r0, g0, b0) = (rng.below(200) as u8, rng.below(200) as u8, rng.below(200) as u8);
    for (x, y, p) in img.enumerate_pixels_mut() {
        let stripe = ((x / 40 + y / 40) % 2) as u8;
        *p = image::Rgb([
            r0.wrapping_add(stripe * 40),
            g0.wrapping_add((x % 256) as u8 / 4),
            b0.wrapping_add(stripe * 20),
        ]);
    }
    let mut png = Vec::new();
    image::ImageEncoder::write_image(
        image::codecs::png::PngEncoder::new(&mut png),
        img.as_raw(),
        w,
        h,
        image::ExtendedColorType::Rgb8,
    )
    .expect("png encodes");
    Image { reference: format!("img/diagram-{}.png", i + 1), width: w, height: h, png }
}

// ---------------------------------------------------------------------------------------------
// The plan
// ---------------------------------------------------------------------------------------------

impl Plan {
    /// Generate the plan for `scale` from `seed` (pure).
    pub fn generate(scale: &Scale, seed: u64, handles: &Handles) -> Plan {
        assert!(scale.versions >= 2, "at least two Versions");
        let mut rng = Rng::new(seed);
        let mut stats = Stats { versions: scale.versions, ..Default::default() };
        let images: Vec<Image> = (0..scale.images).map(|i| make_image(i, &mut rng)).collect();
        let mut doc = gen_v1(&mut rng, scale, &images);
        stats.lines_v1 = doc.lines.len();
        stats.words_v1 = doc.words();

        let mut versions: Vec<String> = Vec::new();
        let mut steps: Vec<Step> = Vec::new();
        let mut known: Vec<Known> = Vec::new();
        let mut thread_count = 0usize;
        let mut reply_counts: Vec<usize> = Vec::new(); // per thread
        let mut resolved: Vec<bool> = Vec::new();
        let mut first_reply_deleted: Vec<bool> = Vec::new();
        let mut body_chars = 0usize;
        let mut mentions = 0usize;

        // Threads per Version: decreasing weights (v1 gets the most).
        let n = scale.versions as usize;
        let weights: Vec<usize> = (0..n).map(|i| n - i).collect();
        let wsum: usize = weights.iter().sum();
        let doc_level_total = (scale.threads * 3).div_ceil(100).max(1);
        let span_total = scale.threads.saturating_sub(doc_level_total);
        let mut per_version: Vec<usize> = weights.iter().map(|w| span_total * w / wsum).collect();
        let mut rem = span_total - per_version.iter().sum::<usize>();
        let mut i = 0;
        while rem > 0 {
            per_version[i % n] += 1;
            rem -= 1;
            i += 1;
        }
        let mut doc_level_left = doc_level_total;

        for v in 1..=scale.versions {
            // ---- push this Version ----
            let text = doc.render();
            versions.push(text.clone());
            steps.push(Step::Push {
                version: v,
                message: if v == 1 {
                    "first draft".into()
                } else {
                    format!("revision {v}: {}", sentence(&mut rng, 5, 5, false))
                },
            });

            // ---- Threads on the head ----
            let mut created_here = 0usize;
            if v == 1 {
                for im in &images {
                    let by = pick_who(&mut rng);
                    let body = body_text(&mut rng, handles, by, 0.2, &mut mentions);
                    body_chars += body.chars().count();
                    steps.push(Step::Thread { by, subject: Subject::Asset { reference: im.reference.clone() }, body });
                    thread_count += 1;
                    reply_counts.push(0);
                    resolved.push(false);
                    first_reply_deleted.push(false);
                    stats.asset_threads += 1;
                    created_here += 1;
                }
            }
            // Document-level ones spread over the Versions.
            let dl_here =
                if v == scale.versions { doc_level_left } else { doc_level_total.div_ceil(n).min(doc_level_left) };
            for _ in 0..dl_here {
                let by = pick_who(&mut rng);
                let body = body_text(&mut rng, handles, by, 0.3, &mut mentions);
                body_chars += body.chars().count();
                steps.push(Step::Thread { by, subject: Subject::Document, body });
                thread_count += 1;
                reply_counts.push(0);
                resolved.push(false);
                first_reply_deleted.push(false);
                stats.document_threads += 1;
                doc_level_left -= 1;
                created_here += 1;
            }
            // Span Threads: lines with eligible tokens, weighted to prose.
            let candidates: Vec<usize> = doc
                .lines
                .iter()
                .enumerate()
                .filter(|(_, l)| {
                    matches!(
                        l.kind,
                        Kind::Para | Kind::Item | Kind::Quote | Kind::H2 | Kind::H3 | Kind::TableRow | Kind::Code
                    ) && eligible_tokens(l.kind, &l.text).len() >= 2
                })
                .map(|(i, _)| i)
                .collect();
            let want = per_version[(v - 1) as usize];
            let mut made = 0usize;
            let mut guard = 0usize;
            while made < want && guard < want * 20 && !candidates.is_empty() {
                guard += 1;
                let li = *rng.pick(&candidates);
                let line = &doc.lines[li];
                // prose twice as likely as code / table
                if matches!(line.kind, Kind::Code | Kind::TableRow) && rng.chance(0.5) {
                    continue;
                }
                let toks = eligible_tokens(line.kind, &line.text);
                let run = rng.range(1, 5).min(toks.len());
                let first = rng.below(toks.len() - run + 1);
                let (s, e) = (toks[first].start, toks[first + run - 1].end);
                // Two spans on one line are fine; identical spans are not interesting.
                if known.iter().any(|k| k.line_id == line.id && k.start == s && k.end == e) {
                    continue;
                }
                let exact = slice_cp(&line.text, s, e);
                let range = format!("{}:{}-{}:{}", li + 1, s, li + 1, e);
                let by = pick_who(&mut rng);
                let body = body_text(&mut rng, handles, by, 0.2, &mut mentions);
                body_chars += body.chars().count();
                steps.push(Step::Thread { by, subject: Subject::Span { range, exact }, body });
                known.push(Known { line_id: line.id, start: s, end: e, words: run, alive: true });
                thread_count += 1;
                reply_counts.push(0);
                resolved.push(false);
                first_reply_deleted.push(false);
                stats.span_threads += 1;
                made += 1;
                created_here += 1;
            }
            stats.threads_per_version.push(created_here);

            // ---- Replies / resolves / reopens / a few tombstones, stamped with this Version ----
            let existing = thread_count;
            let reply_on = rng.sample(existing, existing * 45 / 100 / (n - v as usize + 1).max(1) + 1);
            for t in reply_on {
                if reply_counts[t] >= 4 {
                    continue;
                }
                let k = rng.range(1, 2);
                for j in 0..k {
                    let by = pick_who(&mut rng);
                    let body = body_text(&mut rng, handles, by, 0.2, &mut mentions);
                    body_chars += body.chars().count();
                    let resolve = j == k - 1 && !resolved[t] && rng.chance(0.35);
                    if resolve {
                        resolved[t] = true;
                        stats.resolves += 1;
                    }
                    steps.push(Step::Reply { thread: t, by, body, resolve });
                    reply_counts[t] += 1;
                    stats.replies += 1;
                }
            }
            // bare resolves
            for t in rng.sample(existing, existing / 12 + 1) {
                if !resolved[t] {
                    resolved[t] = true;
                    stats.resolves += 1;
                    steps.push(Step::Resolve { thread: t, by: pick_who(&mut rng) });
                }
            }
            // a couple of reopens
            if v > 1 {
                let res: Vec<usize> = (0..existing).filter(|t| resolved[*t]).collect();
                for t in rng.sample(res.len(), (res.len() / 15).min(3)) {
                    let t = res[t];
                    resolved[t] = false;
                    stats.reopens += 1;
                    steps.push(Step::Reopen { thread: t, by: pick_who(&mut rng) });
                }
            }
            // tombstones: one reply per ~40 Threads
            for t in rng.sample(existing, existing / 40) {
                if reply_counts[t] >= 1 && !first_reply_deleted[t] {
                    first_reply_deleted[t] = true;
                    steps.push(Step::Delete { thread: t, reply: Some(1), by: Who::Owner });
                    stats.deletes += 1;
                }
            }
            // ---- Actions on two Versions: the second and the second-to-last ----
            if v == 2 {
                steps.push(Step::Act { by: Who::Owner, action: "send-back".into(), version: v });
                stats.actions += 1;
            }
            // `send-back` on v2, `approve` on the second-to-last Version (the head when there
            // are fewer than five): two Versions carry an Action, the head page shows a chip.
            let approve_on = if scale.versions >= 5 { scale.versions - 1 } else { scale.versions };
            if v == approve_on {
                steps.push(Step::Act { by: Who::Agent2, action: "approve".into(), version: v });
                stats.actions += 1;
            }

            // ---- the edit that makes the next Version ----
            if v < scale.versions {
                let last = v + 1 == scale.versions;
                edit(&mut rng, &mut doc, &mut known, &mut stats, last, &images);
            }
        }

        let head = doc.render();
        stats.lines_head = head.lines().count();
        stats.words_head = doc.words();
        stats.threads = thread_count;
        stats.mentions = mentions;
        stats.body_chars = body_chars;
        assert!(body_chars < 90_000, "generated bodies total {body_chars} chars — the server caps a Document at 100k");

        // ---- the list Documents ----
        let mut list_docs = Vec::new();
        for i in 0..scale.list_docs {
            let title = format!("{} {}", cap(rng.pick(WORDS)), rng.pick(TOPICS));
            let md = if i % 4 == 0 {
                format!("---\ntitle: {title}\n---\n\n# Not the title\n\n{}\n", paragraph(&mut rng, 30, 30))
            } else {
                format!(
                    "# {title}\n\n{}\n\n- {}\n- {}\n",
                    paragraph(&mut rng, 25, 25),
                    sentence(&mut rng, 7, 7, false),
                    sentence(&mut rng, 6, 6, false)
                )
            };
            list_docs.push((format!("perf-list-{:02}", i + 1), md));
        }

        Plan { name: "perf-plan".into(), handles: handles.clone(), versions, images, steps, list_docs, stats }
    }

    /// The `Thread` steps in order (index = Thread number − 1).
    pub fn thread_steps(&self) -> Vec<&Step> {
        self.steps.iter().filter(|s| matches!(s, Step::Thread { .. })).collect()
    }
}

/// Edit `doc` in place to make the next Version: targeted edits on known spans (inside ⇒ refined,
/// line deleted ⇒ orphaned), plus paragraph inserts / deletes / word touches / whitespace
/// changes elsewhere; the last edit also drops one image (its Asset Thread orphans).
fn edit(rng: &mut Rng, doc: &mut Doc, known: &mut [Known], stats: &mut Stats, last: bool, images: &[Image]) {
    // ---- targeted: refine ~12% and orphan ~6% of the alive spans ----
    let alive: Vec<usize> = known.iter().enumerate().filter(|(_, k)| k.alive).map(|(i, _)| i).collect();
    let refine_n = (alive.len() * 12 / 100).max(if alive.len() >= 2 { 1 } else { 0 });
    let orphan_n = (alive.len() * 6 / 100).max(if alive.len() >= 3 { 1 } else { 0 });
    let picks = rng.sample(alive.len(), refine_n + orphan_n);
    let mut touched_lines: Vec<u32> = Vec::new();
    let mut deleted_lines: Vec<u32> = Vec::new();
    for (j, pi) in picks.iter().enumerate() {
        let ki = alive[*pi];
        let line_id = known[ki].line_id;
        if touched_lines.contains(&line_id) || deleted_lines.contains(&line_id) {
            continue;
        }
        let Some(li) = doc.index_of(line_id) else { continue };
        if j < refine_n && known[ki].words >= 2 {
            // Insert a word between the span's first and second word.
            let text = doc.lines[li].text.clone();
            let toks = tokens(&text);
            let Some(first) = toks.iter().find(|t| t.start >= known[ki].start) else { continue };
            let at = first.end; // right after the first word of the span
            let word = format!(" {}", rng.pick(WORDS));
            let wl = word.chars().count();
            let new_text: String = {
                let mut s: String = text.chars().take(at).collect();
                s.push_str(&word);
                s.extend(text.chars().skip(at));
                s
            };
            doc.lines[li].text = new_text;
            touched_lines.push(line_id);
            stats.intended_refined += 1;
            // shift every known span on that line
            for k in known.iter_mut().filter(|k| k.line_id == line_id) {
                if at < k.start {
                    k.start += wl;
                    k.end += wl;
                } else if at < k.end {
                    k.end += wl;
                }
            }
        } else {
            // Delete the line (a paragraph goes with its trailing blank; an item / row alone).
            let kind = doc.lines[li].kind;
            match kind {
                Kind::Para | Kind::Quote => {
                    doc.lines.remove(li);
                    if li < doc.lines.len()
                        && doc.lines[li].kind == Kind::Blank
                        && li > 0
                        && doc.lines[li - 1].kind == Kind::Blank
                    {
                        doc.lines.remove(li);
                    }
                }
                Kind::Item | Kind::Code | Kind::TableRow => {
                    // keep lists / tables / fences well-formed: a table needs its header row
                    if kind == Kind::TableRow && li + 1 < doc.lines.len() && doc.lines[li + 1].kind == Kind::TableSep {
                        continue;
                    }
                    doc.lines.remove(li);
                }
                _ => continue,
            }
            deleted_lines.push(line_id);
            stats.intended_orphaned += 1;
            for k in known.iter_mut().filter(|k| k.line_id == line_id) {
                k.alive = false;
            }
        }
    }
    // ---- untargeted structural edits ----
    let paras: Vec<usize> =
        doc.lines.iter().enumerate().filter(|(_, l)| l.kind == Kind::Para).map(|(i, _)| i).collect();
    // delete ~1% of paragraphs (not ones we just touched)
    let del = rng.sample(paras.len(), (paras.len() / 100).max(1));
    let mut removed_ids: Vec<u32> = Vec::new();
    for pi in del {
        let id = doc.lines[paras[pi]].id;
        if touched_lines.contains(&id) {
            continue;
        }
        removed_ids.push(id);
    }
    for id in &removed_ids {
        if let Some(li) = doc.index_of(*id) {
            doc.lines.remove(li);
            if li < doc.lines.len()
                && doc.lines[li].kind == Kind::Blank
                && li > 0
                && doc.lines[li - 1].kind == Kind::Blank
            {
                doc.lines.remove(li);
            }
            for k in known.iter_mut().filter(|k| k.line_id == *id) {
                k.alive = false;
            }
        }
    }
    // insert new paragraphs after ~2% of the remaining paragraphs
    let paras: Vec<u32> = doc.lines.iter().filter(|l| l.kind == Kind::Para).map(|l| l.id).collect();
    for pi in rng.sample(paras.len(), (paras.len() * 2 / 100).max(1)) {
        if let Some(li) = doc.index_of(paras[pi]) {
            let text = paragraph(rng, 20, 50);
            let id = doc.next_id;
            doc.next_id += 1;
            // after the paragraph's trailing blank
            let at = (li + 2).min(doc.lines.len());
            doc.lines.insert(at, Line { id: id + 1, kind: Kind::Blank, text: String::new() });
            doc.lines.insert(at, Line { id, kind: Kind::Para, text });
            doc.next_id += 1;
        }
    }
    // touch ~2% of paragraphs: a word inserted at a random word boundary
    let paras: Vec<u32> = doc.lines.iter().filter(|l| l.kind == Kind::Para).map(|l| l.id).collect();
    for pi in rng.sample(paras.len(), (paras.len() * 2 / 100).max(1)) {
        let id = paras[pi];
        if touched_lines.contains(&id) {
            continue;
        }
        if let Some(li) = doc.index_of(id) {
            let text = doc.lines[li].text.clone();
            let toks = tokens(&text);
            if toks.len() < 3 {
                continue;
            }
            let at = toks[rng.range(1, toks.len() - 1)].start; // before a word
            let word = format!("{} ", rng.pick(WORDS));
            let wl = word.chars().count();
            let mut s: String = text.chars().take(at).collect();
            s.push_str(&word);
            s.extend(text.chars().skip(at));
            doc.lines[li].text = s;
            for k in known.iter_mut().filter(|k| k.line_id == id) {
                if at <= k.start {
                    k.start += wl;
                    k.end += wl;
                } else if at < k.end {
                    k.end += wl;
                }
            }
        }
    }
    // whitespace-only changes on ~1% of items (a double space between two words)
    let items: Vec<u32> = doc.lines.iter().filter(|l| l.kind == Kind::Item).map(|l| l.id).collect();
    for pi in rng.sample(items.len(), (items.len() / 100).max(1)) {
        let id = items[pi];
        if let Some(li) = doc.index_of(id) {
            let text = doc.lines[li].text.clone();
            let toks = tokens(&text);
            if toks.len() < 3 {
                continue;
            }
            let at = toks[2].start; // before the third word
            let mut s: String = text.chars().take(at).collect();
            s.push(' ');
            s.extend(text.chars().skip(at));
            doc.lines[li].text = s;
            for k in known.iter_mut().filter(|k| k.line_id == id) {
                if at <= k.start {
                    k.start += 1;
                    k.end += 1;
                } else if at < k.end {
                    k.end += 1;
                }
            }
        }
    }
    // the last edit drops one image reference (its Asset Thread orphans)
    if last && images.len() >= 2 {
        let target = &images[images.len() - 1].reference;
        if let Some(li) = doc.lines.iter().position(|l| l.kind == Kind::Image && l.text.contains(target.as_str())) {
            doc.lines.remove(li);
            if li < doc.lines.len()
                && doc.lines[li].kind == Kind::Blank
                && li > 0
                && doc.lines[li - 1].kind == Kind::Blank
            {
                doc.lines.remove(li);
            }
        }
    }
}

/// Extract the code-point range `L:C-L:C` from `text` (the generator's own check that every
/// span it planned is on the Version it was planned for).
pub fn extract(text: &str, range: &str) -> Option<String> {
    let (a, b) = range.split_once('-')?;
    let (l1, c1) = a.split_once(':')?;
    let (l2, c2) = b.split_once(':')?;
    let (l1, c1, l2, c2): (usize, usize, usize, usize) =
        (l1.parse().ok()?, c1.parse().ok()?, l2.parse().ok()?, c2.parse().ok()?);
    let lines: Vec<&str> = text.split('\n').collect();
    if l1 == 0 || l2 < l1 || l2 > lines.len() {
        return None;
    }
    let mut out = String::new();
    for l in l1..=l2 {
        let line = lines[l - 1];
        let from = if l == l1 { c1 } else { 0 };
        let to = if l == l2 { c2 } else { line.chars().count() };
        if to < from || to > line.chars().count() {
            return None;
        }
        if l > l1 {
            out.push('\n');
        }
        out.extend(line.chars().skip(from).take(to - from));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: S15-01
    #[test]
    fn deterministic() {
        let a = Plan::generate(&Scale::SMOKE, 1, &Handles::default());
        let b = Plan::generate(&Scale::SMOKE, 1, &Handles::default());
        assert_eq!(a, b);
        let c = Plan::generate(&Scale::SMOKE, 2, &Handles::default());
        assert_ne!(a.versions[0], c.versions[0]);
    }

    // spec: S15-01
    #[test]
    fn smoke_scale_has_the_shape() {
        let p = Plan::generate(&Scale::SMOKE, 1, &Handles::default());
        assert_eq!(p.versions.len(), 3);
        assert!(p.stats.lines_v1 >= Scale::SMOKE.lines, "{:?}", p.stats);
        assert!(p.stats.words_v1 >= Scale::SMOKE.words, "{:?}", p.stats);
        assert!(p.stats.threads >= Scale::SMOKE.threads, "{:?}", p.stats);
        assert_eq!(p.stats.asset_threads, 2);
        assert!(p.stats.document_threads >= 1);
        assert!(p.stats.replies > 0 && p.stats.resolves > 0 && p.stats.mentions > 0);
        assert_eq!(p.stats.actions, 2);
        assert!(p.stats.intended_refined >= 1 && p.stats.intended_orphaned >= 1, "{:?}", p.stats);
        assert_eq!(p.list_docs.len(), 3);
        let v1 = &p.versions[0];
        for kind in ["## ", "- ", "| ", "```", "> ", "![diagram 1](img/diagram-1.png)"] {
            assert!(v1.contains(kind), "v1 lacks {kind:?}");
        }
    }

    // spec: S15-01
    #[test]
    fn every_span_is_on_the_head_it_was_planned_for() {
        let p = Plan::generate(&Scale::SMOKE, 3, &Handles::default());
        let mut head = 0usize;
        for s in &p.steps {
            match s {
                Step::Push { version, .. } => head = *version as usize,
                Step::Thread { subject: Subject::Span { range, exact }, .. } => {
                    let got = extract(&p.versions[head - 1], range).unwrap_or_default();
                    assert_eq!(&got, exact, "range {range} on v{head}");
                    assert!(!exact.trim().is_empty());
                }
                _ => {}
            }
        }
    }

    // spec: S15-01
    #[test]
    fn full_scale_meets_the_spec_numbers() {
        let p = Plan::generate(&Scale::FULL, 1, &Handles::default());
        assert!(p.stats.lines_v1 >= 5_000, "{:?}", p.stats);
        assert!(p.stats.lines_head >= 5_000, "{:?}", p.stats);
        assert!((40_000..=65_000).contains(&p.stats.words_v1), "{:?}", p.stats);
        assert!(p.stats.threads >= 500, "{:?}", p.stats);
        assert_eq!(p.stats.versions, 5);
        assert!(p.stats.body_chars < 90_000, "{:?}", p.stats);
        assert!(p.stats.intended_refined >= 30 && p.stats.intended_orphaned >= 15, "{:?}", p.stats);
        assert_eq!(p.images.len(), 4);
        assert!(p.images[0].png.starts_with(b"\x89PNG"));
    }
}
