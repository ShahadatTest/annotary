//! What `push` reads from a Sidecar — and nothing else: `id`, `vault`, `base_version` (plus that
//! Version's `hash` from `versions[]` and the `assets[]` manifest, for the local-edit check) and
//! `actions.available` when `inherited` is absent. Every other field is server projection and is
//! ignored here, whatever a hand edit did to it.
//!
//! Ticket 17: a Sidecar git left with conflict markers (`<<<<<<<` / `=======` / `>>>>>>>`, and
//! diff3's `|||||||` base section) needs no resolving. Each side is read on its own; the two must
//! name the same Document ([`ParseError::Conflict`] otherwise), `base_version` is the lower of the
//! two (the content check then covers more of the head's changes), and `actions.available` is
//! used only when both sides agree. The next push or pull rewrites the file clean.
//!
//! CMD-5: a Document id is unique in the Tenant and a Document never moves between Vaults, so two
//! sides with the same `id` and different `vault` names are one Document across a Vault rename
//! (one branch pulled before it, one after) — not a conflict. The newer side's (higher
//! `base_version`) name is kept; nothing else is taken from it (push binds by id and never reads
//! `vault` for an existing Document).

use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::Value;

use crate::identity::{is_document_id, version_hash};
use crate::model::ActionDecl;
use crate::parse::{ParseError, parse_object};

/// The authoritative part of a Sidecar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub id: String,
    pub vault: String,
    pub base_version: u32,
    /// `versions[].hash` of `base_version`, when listed.
    pub base_hash: Option<String>,
    /// `versions[].name` of `base_version`, when listed.
    pub base_name: Option<String>,
    /// `assets[]` as `(ref, hash)` — the manifest of `base_version` (`None` hash = missing).
    pub base_assets: Vec<(String, Option<String>)>,
    /// `actions.available` when the declaration is the Document's own (`inherited` absent);
    /// `Some(vec![])` declares "no Actions"; `None` = inherited or absent (nothing to send).
    pub actions_available: Option<Vec<ActionDecl>>,
    /// The file had git conflict markers (read side by side, see the module docs).
    pub conflicted: bool,
    /// Conflicted, and the two sides declare different `actions.available` (none is sent).
    pub actions_disagree: bool,
}

impl Binding {
    /// Read the binding out of Sidecar text. Same rejections as [`crate::parse`] for the
    /// container; lenient about everything that is not read; git conflict markers tolerated.
    pub fn from_text(text: &str) -> Result<Binding, ParseError> {
        let Some((ours, theirs)) = conflict_sides(text)? else {
            return Binding::from_side(text);
        };
        let (a, b) = (Binding::from_side(&ours)?, Binding::from_side(&theirs)?);
        if a.id != b.id {
            return Err(ParseError::Conflict { what: "Document ids" });
        }
        let actions_disagree = a.actions_available != b.actions_available;
        // The newer side's Vault name (a rename between the two pulls); a tie keeps ours.
        let vault = if b.base_version > a.base_version { b.vault.clone() } else { a.vault.clone() };
        let mut lower = if b.base_version < a.base_version { b } else { a };
        lower.vault = vault;
        if actions_disagree {
            lower.actions_available = None;
        }
        lower.conflicted = true;
        lower.actions_disagree = actions_disagree;
        Ok(lower)
    }

    /// One side (or a file without conflict markers).
    fn from_side(text: &str) -> Result<Binding, ParseError> {
        let value = parse_object(text)?;
        let obj = value.as_object().expect("parse_object returns an object");
        let id = match obj.get("id").and_then(Value::as_str) {
            Some(s) if is_document_id(s) => s.to_string(),
            Some(s) => return Err(ParseError::Invalid(alloc::format!("id {s:?} is not a Document id"))),
            None => return Err(ParseError::Invalid("missing `id`".into())),
        };
        let vault = match obj.get("vault").and_then(Value::as_str) {
            Some(s) if !s.trim().is_empty() => s.to_string(),
            _ => return Err(ParseError::Invalid("missing `vault`".into())),
        };
        let base_version = match obj.get("base_version").and_then(Value::as_u64) {
            Some(n) if n >= 1 && n <= u64::from(u32::MAX) => n as u32,
            _ => return Err(ParseError::Invalid("missing or invalid `base_version`".into())),
        };
        let mut base_hash = None;
        let mut base_name = None;
        if let Some(versions) = obj.get("versions").and_then(Value::as_array) {
            for v in versions {
                if v.get("number").and_then(Value::as_u64) == Some(u64::from(base_version)) {
                    base_hash = v.get("hash").and_then(Value::as_str).map(str::to_string);
                    base_name = v.get("name").and_then(Value::as_str).map(str::to_string);
                }
            }
        }
        let base_assets = obj
            .get("assets")
            .and_then(Value::as_array)
            .map(|assets| {
                assets
                    .iter()
                    .filter_map(|a| {
                        let r = a.get("ref").and_then(Value::as_str)?;
                        Some((r.to_string(), a.get("hash").and_then(Value::as_str).map(str::to_string)))
                    })
                    .collect()
            })
            .unwrap_or_default();
        let actions_available = match obj.get("actions").and_then(Value::as_object) {
            Some(actions) if !actions.get("inherited").and_then(Value::as_bool).unwrap_or(false) => {
                match actions.get("available") {
                    None | Some(Value::Null) => None,
                    Some(Value::Array(list)) => {
                        // Every entry needs an `id`; label/intent are validated by the server
                        // (label defaults to the id, intent to neutral). Ticket 05.
                        let mut decls = Vec::with_capacity(list.len());
                        for (i, d) in list.iter().enumerate() {
                            let Some(id) = d.get("id").and_then(Value::as_str) else {
                                return Err(ParseError::Invalid(alloc::format!(
                                    "actions.available[{i}] has no `id` (entries are {{id, label, intent}})"
                                )));
                            };
                            decls.push(ActionDecl {
                                id: id.to_string(),
                                label: d.get("label").and_then(Value::as_str).unwrap_or("").to_string(),
                                intent: d.get("intent").and_then(Value::as_str).unwrap_or("neutral").to_string(),
                            });
                        }
                        Some(decls)
                    }
                    Some(_) => {
                        return Err(ParseError::Invalid(
                            "actions.available must be a list of {id, label, intent}".into(),
                        ));
                    }
                }
            }
            _ => None,
        };
        Ok(Binding {
            id,
            vault,
            base_version,
            base_hash,
            base_name,
            base_assets,
            actions_available,
            conflicted: false,
            actions_disagree: false,
        })
    }

    /// The Version identity the local `.md` (hashed to `md_hash`) has together with the recorded
    /// manifest of `base_version` — equal to `base_hash` iff the Markdown is unedited since pull.
    pub fn local_identity(&self, md_hash: &str) -> String {
        version_hash(md_hash, self.base_assets.iter().map(|(r, h)| (r.as_str(), h.as_deref())))
    }

    /// True when the local Markdown (hashed to `md_hash`) is what `base_version` recorded.
    /// `None` when the Sidecar does not list `base_version` (nothing to compare against).
    pub fn matches_base(&self, md_hash: &str) -> Option<bool> {
        self.base_hash.as_deref().map(|h| h == self.local_identity(md_hash))
    }
}

/// The two sides of a text with git conflict markers (`None` when it has none): lines outside the
/// conflicts go to both, `<<<<<<<`…`|||||||`/`=======` to ours, `=======`…`>>>>>>>` to theirs,
/// diff3's `|||||||`…`=======` base section to neither. A marker out of order is not a conflict
/// git wrote: [`ParseError::Invalid`].
fn conflict_sides(text: &str) -> Result<Option<(String, String)>, ParseError> {
    #[derive(PartialEq)]
    enum At {
        Both,
        Ours,
        Base,
        Theirs,
    }
    let marker = |l: &str, m: &str| l.starts_with(m) && (l.len() == 7 || l[7..].starts_with(' '));
    if !text.lines().any(|l| marker(l, "<<<<<<<")) {
        return Ok(None);
    }
    let bad = |what: &str| ParseError::Invalid(alloc::format!("git conflict markers out of order ({what})"));
    let (mut ours, mut theirs) = (String::new(), String::new());
    let mut at = At::Both;
    for l in text.split_inclusive('\n') {
        let bare = l.trim_end_matches(['\n', '\r']);
        if marker(bare, "<<<<<<<") {
            if at != At::Both {
                return Err(bad("nested <<<<<<<"));
            }
            at = At::Ours;
        } else if marker(bare, "|||||||") {
            if at != At::Ours {
                return Err(bad("||||||| outside a conflict"));
            }
            at = At::Base;
        } else if marker(bare, "=======") && matches!(at, At::Ours | At::Base) {
            at = At::Theirs;
        } else if marker(bare, ">>>>>>>") {
            if at != At::Theirs {
                return Err(bad(">>>>>>> without ======="));
            }
            at = At::Both;
        } else {
            match at {
                At::Both => {
                    ours.push_str(l);
                    theirs.push_str(l);
                }
                At::Ours => ours.push_str(l),
                At::Theirs => theirs.push_str(l),
                At::Base => {}
            }
        }
    }
    if at != At::Both {
        return Err(bad("unterminated conflict"));
    }
    Ok(Some((ours, theirs)))
}
