//! `cargo run -p annotary-renderer --example render -- FILE.md [manifest.json]` — print the
//! rendered HTML of a Markdown file (a developer aid; the CLI's `push` is the real path).
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let md = std::fs::read_to_string(&args[1]).expect("read markdown");
    let assets = match args.get(2) {
        Some(p) => {
            let text = std::fs::read_to_string(p).expect("read manifest");
            let v: serde_json::Value = serde_json::from_str(&text).expect("manifest json");
            annotary_renderer::Asset::from_manifest(&v)
        }
        None => Vec::new(),
    };
    let r = annotary_renderer::render(&md, &annotary_renderer::Options { assets: &assets, embeds: true });
    if let Some(t) = &r.title {
        eprintln!("title: {t}");
    }
    print!("{}", r.html);
}
