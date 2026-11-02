//! `perfgen` — the perf suite's generator / seeder as a command (ticket 15). `make perf` runs
//! `perfgen seed` against the release server the Playwright runner spawned; `perfgen stats` and
//! `perfgen write` exist for looking at the fixture without a server.

use std::path::PathBuf;

use annotary_perfgen::{Handles, Plan, Scale, Target, seed};
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "perfgen", about = "annotary perf fixture generator / seeder (ticket 15)")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(clap::Args, Clone)]
struct Common {
    /// `full` (the perf fixture) or `smoke` (the small one `cargo test` runs).
    #[arg(long, default_value = "full")]
    scale: String,
    /// The generator seed (same seed ⇒ byte-identical fixture).
    #[arg(long, default_value_t = 1)]
    seed: u64,
    /// The owner's handle (first-run setup), mentioned in bodies.
    #[arg(long, default_value = "rowan")]
    owner: String,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print the generated fixture's statistics as JSON (no server needed).
    Stats {
        #[command(flatten)]
        common: Common,
    },
    /// Write the fixture (`v1.md` … `vN.md`, `img/`) into a directory for inspection.
    Write {
        #[command(flatten)]
        common: Common,
        #[arg(long)]
        out: PathBuf,
    },
    /// Seed a running server through the real binary (pushes) and `/api/v1` (Threads, …).
    Seed {
        #[command(flatten)]
        common: Common,
        /// The `annotary` binary to push with.
        #[arg(long)]
        bin: PathBuf,
        /// The server URL.
        #[arg(long)]
        server: String,
        /// The owner's Token.
        #[arg(long, env = "ANNOTARY_TOKEN")]
        token: String,
        /// A temp HOME for the CLI (created).
        #[arg(long)]
        home: PathBuf,
        /// Where the Document's files are written and pushed from (created).
        #[arg(long)]
        work: PathBuf,
        /// Print the result as JSON (one object) instead of text.
        #[arg(long)]
        json: bool,
    },
}

fn plan_for(c: &Common) -> Plan {
    let scale = Scale::parse(&c.scale).unwrap_or_else(|| {
        eprintln!("perfgen: unknown --scale {:?} (full | smoke)", c.scale);
        std::process::exit(1);
    });
    let handles = Handles { owner: c.owner.clone(), ..Handles::default() };
    Plan::generate(&scale, c.seed, &handles)
}

fn main() {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Stats { common } => {
            let p = plan_for(&common);
            println!("{}", serde_json::to_string_pretty(&p.stats).unwrap());
        }
        Cmd::Write { common, out } => {
            let p = plan_for(&common);
            std::fs::create_dir_all(&out).expect("create --out");
            for (i, v) in p.versions.iter().enumerate() {
                std::fs::write(out.join(format!("v{}.md", i + 1)), v).expect("write");
            }
            for im in &p.images {
                let path = out.join(&im.reference);
                std::fs::create_dir_all(path.parent().unwrap()).expect("img dir");
                std::fs::write(path, &im.png).expect("write png");
            }
            println!("{}", serde_json::to_string_pretty(&p.stats).unwrap());
        }
        Cmd::Seed { common, bin, server, token, home, work, json } => {
            let p = plan_for(&common);
            let target = Target { bin, server, owner_token: token, home, work };
            let mut log = |s: &str| eprintln!("perfgen: {s}");
            match seed(&p, &target, &mut log) {
                Ok(s) => {
                    if json {
                        println!("{}", serde_json::to_string(&s).unwrap());
                    } else {
                        println!(
                            "{}\t{}\tv{}\t{} threads ({} anchored, {} refined, {} orphaned, {} document-level)\t{} ms",
                            s.name,
                            s.doc_id,
                            s.head,
                            s.threads,
                            s.statuses.anchored,
                            s.statuses.refined,
                            s.statuses.orphaned,
                            s.statuses.document,
                            s.elapsed_ms
                        );
                    }
                }
                Err(e) => {
                    eprintln!("perfgen: seed failed: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
