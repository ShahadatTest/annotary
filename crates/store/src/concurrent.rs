//! Concurrent edits (ticket 17, ADR 0002): a push whose `base_version` is behind the head is
//! accepted only when the pushed file already contains the head's changes since that base — the
//! pusher merged them with their own tools — or when it says `discard_head_changes`. The check is
//! [`check_head_changes`]: `annotary_diff::contains_changes` on the Markdown plus
//! `missing_asset_changes` on the manifests. `push` runs it before uploading anything and sends
//! `contains_head: N` (the head it checked); `Store::push_version` refuses a commit that claims
//! nothing, claims a head that has moved on again, or (with `Store::verify_edge`) claims a
//! containment the recheck does not confirm.

use std::fmt;

pub use annotary_diff::{AssetChange, AssetChangeKind, Containment, Hunk, format_unified};

use crate::docs::manifest_pairs;

/// What the head changed since the base that the pushed file lacks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadChanges {
    /// Base→head Markdown hunks missing from the pushed text (empty when contained).
    pub hunks: Vec<Hunk>,
    /// Assets the head added, removed or changed whose new entry the pushed manifest lacks.
    pub assets: Vec<AssetChange>,
}

impl HeadChanges {
    pub fn is_contained(&self) -> bool {
        self.hunks.is_empty() && self.assets.is_empty()
    }
}

/// Does the pushed `local` Markdown + manifest contain every change `base` → `head` made? The
/// manifests are the commit's JSON arrays (`[{ref, hash?, …}]`; a `missing` entry has no hash).
pub fn check_head_changes(
    base_md: &str,
    head_md: &str,
    local_md: &str,
    base_manifest: &serde_json::Value,
    head_manifest: &serde_json::Value,
    local_manifest: &serde_json::Value,
) -> HeadChanges {
    let hunks = match annotary_diff::contains_changes(base_md, head_md, local_md) {
        Containment::Contained => Vec::new(),
        Containment::Missing { hunks } => hunks,
    };
    let assets = annotary_diff::missing_asset_changes(
        &manifest_pairs(base_manifest),
        &manifest_pairs(head_manifest),
        &manifest_pairs(local_manifest),
    );
    HeadChanges { hunks, assets }
}

/// `v5` | `v5–v7`: the Versions after `base` up to `head`.
pub fn versions_word(base: u32, head: u32) -> String {
    if head == base + 1 { format!("v{head}") } else { format!("v{}–v{head}", base + 1) }
}

/// `@a, @b`.
pub fn handles_word(by: &[String]) -> String {
    by.iter().map(|h| format!("@{h}")).collect::<Vec<_>>().join(", ")
}

/// Why a push onto a moved head was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeadMovedReason {
    /// The commit neither claims `contains_head` nor `discard_head_changes` (an older CLI, or a
    /// raw API call).
    Unchecked,
    /// It claims the head's changes are contained, and the server's recheck disagrees.
    Missing,
    /// It checked against an older head: someone pushed between the check and the commit.
    MovedAgain { checked: u32 },
}

/// A push refused because the head moved since `base` (409 `head_moved`, exit 5).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadMovedError {
    /// The Document's Name (as pushed).
    pub name: String,
    pub base: u32,
    pub head: u32,
    /// Handles of the Members who pushed `base+1..=head` (deduplicated, in order).
    pub by: Vec<String>,
    pub reason: HeadMovedReason,
}

impl fmt::Display for HeadMovedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let changed = format!(
            "{} by {} changed {} since your v{}",
            versions_word(self.base, self.head),
            handles_word(&self.by),
            self.name,
            self.base
        );
        match self.reason {
            HeadMovedReason::Unchecked => {
                write!(f, "{changed} — this push does not say whether your file contains those changes")
            }
            HeadMovedReason::Missing => write!(f, "{changed} — those changes are not in your file"),
            HeadMovedReason::MovedAgain { checked } => {
                write!(f, "head moved again (now v{}) — your file was checked against v{checked}", self.head)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // spec: S17-06
    #[test]
    fn text_and_assets_together() {
        let base = json!([{"ref": "a.png", "hash": "h1"}]);
        let head = json!([{"ref": "a.png", "hash": "h2"}]);
        let c = check_head_changes("x\n", "x\n", "x\ny\n", &base, &head, &base);
        assert!(c.hunks.is_empty());
        assert_eq!(c.assets[0].reference, "a.png");
        assert!(!c.is_contained());
        assert!(check_head_changes("x\n", "x\n", "x\ny\n", &base, &head, &head).is_contained());
        assert_eq!(check_head_changes("x\n", "z\n", "x\n", &base, &base, &base).hunks.len(), 1);
    }

    #[test]
    fn wording() {
        assert_eq!(versions_word(4, 5), "v5");
        assert_eq!(versions_word(4, 7), "v5–v7");
        let e = HeadMovedError {
            name: "plan".into(),
            base: 4,
            head: 5,
            by: vec!["scribe".into()],
            reason: HeadMovedReason::Missing,
        };
        assert_eq!(e.to_string(), "v5 by @scribe changed plan since your v4 — those changes are not in your file");
        let again = HeadMovedError { reason: HeadMovedReason::MovedAgain { checked: 5 }, head: 6, ..e };
        assert_eq!(again.to_string(), "head moved again (now v6) — your file was checked against v5");
    }
}
