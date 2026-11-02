//! The annotary perf suite's generator (ticket 15): a **deterministic** large Document —
//! ≥ 5,000 lines / ~50k words of realistic Markdown (headings, paragraphs, lists, tables, code,
//! block quotes, a few images with real dimensions), ≥ 500 Threads across ≥ 4 Versions whose
//! edits leave the placements mixed (anchored / refined / orphaned), Replies, @mentions,
//! resolves / reopens, a few tombstones and Actions on two Versions — plus a page of small
//! Documents for the list. The generator is checked in; its output is not: `make perf` (and the
//! smoke test under `cargo test`) regenerate it from the seed every time.
//!
//! Two halves:
//! - [`Plan::generate`] — pure: `(Scale, seed)` → the Markdown of every Version, the images, and
//!   the ordered [`Step`]s (push, Thread, Reply, …). Same inputs ⇒ byte-identical output
//!   (`tests` assert it).
//! - [`seed::seed`] — drives a running server through the **real** path: the real `annotary`
//!   binary for every `push` (so hashing, the Asset scan, rendering and re-anchoring run at the
//!   edge exactly as for a user) and `member add`; the same `/api/v1` the CLI uses for Threads,
//!   Replies, resolves and Actions.

pub mod plan;
pub mod rng;
pub mod seed;

pub use plan::{Handles, Image, Plan, Scale, Stats, Step, Subject, Who};
pub use seed::{Seeded, Target, seed};
