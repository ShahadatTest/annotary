# Renderer corpus

Markdown inputs for the renderer's `insta` snapshots (`crates/renderer/tests/corpus.rs`): one
file per area of the superset, plus `10-assets.md` with `10-assets.manifest.json` (a manifest
naming every supported Asset type, with fake hashes) so every Asset kind — present, missing,
derivative — has a rendered form. `12-crlf.md` has CRLF line endings on purpose (keep them).
Snapshots live in `crates/renderer/tests/snapshots/`; review any diff, then
`cargo insta accept` (or `INSTA_UPDATE=always cargo test -p annotary-renderer`).
