//! The Sidecar's typed shape (the decided top-level layout, fixed key order):
//!
//! ```text
//! format         1
//! id             8-char base32 Document id
//! vault          Vault name
//! base_version   the Version the sibling .md is
//! members        {handle: {id, name}} for every handle referenced
//! versions[]     {number, name, hash, by, at, message?}
//! assets[]       {ref, hash, size, type, missing?, via?}   for base_version
//! actions        {available[{id,label,intent}], inherited?, log[{id,by,at,version,label,intent}]}
//! threads[]      {id, subject, anchor?, author, created{at,version}, body, mentions?, edited_at?, deleted?, resolved?, replies?}
//! anchor         {original{version, range|hash+ref+line, exact?}, current{version, range|line, status, exact?}, history[{version, range|line, status}]}
//! ```
//!
//! Handles (not Member ids) everywhere a Member is named; ranges as `"L:C-L:C"`; sparse
//! `history`; empty/null fields omitted on the wire (`resolved` present ⇒ resolved; `current.exact`
//! omitted when equal to `original.exact`; a Comment's `body` is always present, even when
//! tombstoned). Field order in these structs *is* the key order the writer emits.
//!
//! Tickets: 02 fills `versions`/`members` (+ `actions.available` from the Vault defaults on pull);
//! 03 `assets`; 04 `threads` (span/document); 05 `actions`; 07 anchors across Versions.

use alloc::string::String;
use alloc::vec::Vec;

use serde::{Deserialize, Serialize};

/// The whole file. `format` is always [`crate::FORMAT`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sidecar {
    pub format: u32,
    pub id: String,
    pub vault: String,
    pub base_version: u32,
    /// `{handle: {id, name}}` in the order the server lists them (creation order).
    #[serde(default, skip_serializing_if = "Members::is_empty")]
    pub members: Members,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<VersionEntry>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assets: Vec<AssetEntry>,
    #[serde(default, skip_serializing_if = "Actions::is_empty")]
    pub actions: Actions,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub threads: Vec<Thread>,
}

/// Ordered `handle → {id, name}` table (a JSON object; insertion order kept so the file is
/// deterministic and diff-friendly).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Members(pub Vec<(String, MemberEntry)>);

impl Members {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, handle: &str) -> Option<&MemberEntry> {
        self.0.iter().find(|(h, _)| h == handle).map(|(_, m)| m)
    }

    /// Insert or replace.
    pub fn insert(&mut self, handle: impl Into<String>, entry: MemberEntry) {
        let handle = handle.into();
        match self.0.iter_mut().find(|(h, _)| *h == handle) {
            Some((_, m)) => *m = entry,
            None => self.0.push((handle, entry)),
        }
    }
}

impl Serialize for Members {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in &self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

impl<'de> Deserialize<'de> for Members {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> serde::de::Visitor<'de> for V {
            type Value = Members;
            fn expecting(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
                f.write_str("a map of handle to {id, name}")
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut m: A) -> Result<Members, A::Error> {
                let mut out = Vec::new();
                while let Some((k, v)) = m.next_entry::<String, MemberEntry>()? {
                    out.push((k, v));
                }
                Ok(Members(out))
            }
        }
        d.deserialize_map(V)
    }
}

/// `{id, name}` for one handle.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberEntry {
    pub id: String,
    pub name: String,
}

/// One row of `versions[]`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionEntry {
    pub number: u32,
    pub name: String,
    /// Version identity: see [`crate::identity::version_hash`].
    pub hash: String,
    /// Handle of the pusher.
    pub by: String,
    /// ISO-8601 UTC, seconds (`2026-08-14T09:12:04Z`).
    pub at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

/// One row of `assets[]` (the manifest of `base_version`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssetEntry {
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub missing: bool,
    /// The referencing file for a recursive Asset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

/// `actions {available[], inherited?, log[]}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Actions {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available: Vec<ActionDecl>,
    /// `true` when `available` was copied from the Vault defaults on pull; drop it (and edit
    /// `available`) to declare a per-Document set on the next push.
    #[serde(default, skip_serializing_if = "is_false")]
    pub inherited: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub log: Vec<ActionEvent>,
}

impl Actions {
    pub fn is_empty(&self) -> bool {
        self.available.is_empty() && !self.inherited && self.log.is_empty()
    }
}

/// `{id, label, intent}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionDecl {
    pub id: String,
    pub label: String,
    /// `positive | neutral | negative`.
    pub intent: String,
}

/// One row of `actions.log`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionEvent {
    pub id: String,
    pub by: String,
    pub at: String,
    pub version: u32,
    pub label: String,
    pub intent: String,
}

/// One Thread: the root Comment's fields plus `subject`, `anchor`, `resolved`, `replies`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Thread {
    pub id: String,
    /// `span | document | asset`.
    pub subject: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<Anchor>,
    pub author: String,
    pub created: Stamp,
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Tombstone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<Resolution>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub replies: Vec<Reply>,
}

/// One Reply.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reply {
    pub id: String,
    pub author: String,
    pub created: Stamp,
    pub body: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mentions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edited_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deleted: Option<Tombstone>,
}

/// `{at, version}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stamp {
    pub at: String,
    pub version: u32,
}

/// `deleted {at, by}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tombstone {
    pub at: String,
    pub by: String,
}

/// `resolved {by, at, version}`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub by: String,
    pub at: String,
    pub version: u32,
}

/// A span or asset Anchor.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Anchor {
    pub original: Placement,
    pub current: Placement,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub history: Vec<Placement>,
}

/// One placement: `original {version, range, exact}` (span) or `{version, hash, ref, line}`
/// (asset); `current`/`history` entries `{version, range|line, status, exact?}`. Which fields
/// are present depends on the role; the writer emits exactly the present ones in this order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Placement {
    pub version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(rename = "ref", default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exact: Option<String>,
}

pub(crate) fn is_false(b: &bool) -> bool {
    !*b
}
