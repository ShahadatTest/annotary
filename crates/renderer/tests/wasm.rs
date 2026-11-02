//! The renderer, the sidecar crate and (ticket 07) the diff crate — the browser bundle of
//! Phase 2's browser push — build for `wasm32-unknown-unknown`. Ignored under plain `cargo test` (it runs
//! `cargo check` for another target); `make wasm-check` runs it and `make check` includes it.
//! Needs `rustup target add wasm32-unknown-unknown`.

use std::path::Path;
use std::process::Command;

fn cargo_check(args: &[&str]) {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = Command::new(cargo)
        .current_dir(&root)
        .args(["check", "--target", "wasm32-unknown-unknown", "--quiet"])
        .args(args)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "cargo check --target wasm32-unknown-unknown {} failed (install the target with `rustup target add wasm32-unknown-unknown`):\n{}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
}

// spec: S09-12, S07-15
#[test]
#[ignore = "runs cargo for the wasm32 target; `make wasm-check`"]
fn renderer_sidecar_and_diff_build_for_wasm32() {
    cargo_check(&["-p", "annotary-renderer"]);
    cargo_check(&["-p", "annotary-sidecar", "--no-default-features"]);
    // ticket 07: the re-anchoring crate ships to the browser too.
    cargo_check(&["-p", "annotary-diff"]);
}
