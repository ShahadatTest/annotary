//! Domain rows and the access model (Member, Membership, Grant, Right, Role) as stored.

use serde::{Deserialize, Serialize};

use crate::error::{Result, StoreError};

/// The four Rights on a Vault. `read` is implied by any other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Right {
    Read,
    Comment,
    Action,
    Write,
}

impl Right {
    pub const ALL: [Right; 4] = [Right::Read, Right::Comment, Right::Action, Right::Write];

    pub fn as_str(self) -> &'static str {
        match self {
            Right::Read => "read",
            Right::Comment => "comment",
            Right::Action => "action",
            Right::Write => "write",
        }
    }

    pub fn parse(s: &str) -> Option<Right> {
        Right::ALL.into_iter().find(|r| r.as_str() == s)
    }
}

impl std::fmt::Display for Right {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Role on a Membership.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Owner,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "owner" => Some(Role::Owner),
            "member" => Some(Role::Member),
            _ => None,
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Grant `{vault | "*", rights[]}` on a Membership; `*` covers Vaults created later.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub vault: String,
    pub rights: Vec<Right>,
}

impl Grant {
    /// The `*` grant with all four Rights (what `member add` and first-run setup give).
    pub fn all() -> Grant {
        Grant { vault: "*".into(), rights: Right::ALL.to_vec() }
    }

    /// Whether this Grant gives `right` on `vault` (`read` implied by any Right).
    pub fn covers(&self, vault: &str, right: Right) -> bool {
        if self.vault != "*" && self.vault != vault {
            return false;
        }
        self.gives(right)
    }

    // ---- ticket 06 ----
    /// Whether this Grant's `rights` include `right` (`read` implied by any Right), ignoring
    /// which Vault it names.
    pub fn gives(&self, right: Right) -> bool {
        match right {
            Right::Read => !self.rights.is_empty(),
            r => self.rights.contains(&r),
        }
    }

    /// The Grant that applies to `vault` in a Membership's list: the Vault's own entry when
    /// there is one, else the `*` entry. A Vault-specific Grant overrides `*` for that Vault
    /// (most specific wins) — so `member grant linter notes comment` restricts linter to
    /// `comment` on `notes` while `*` keeps covering every other (and future) Vault.
    pub fn applicable<'a>(grants: &'a [Grant], vault: &str) -> Option<&'a Grant> {
        grants.iter().find(|g| g.vault == vault).or_else(|| grants.iter().find(|g| g.vault == "*"))
    }

    /// Whether a Membership's Grants give `right` on `vault` (see [`Grant::applicable`]).
    pub fn allow(grants: &[Grant], vault: &str, right: Right) -> bool {
        Grant::applicable(grants, vault).is_some_and(|g| g.gives(right))
    }
    // ---- end ticket 06 ----

    /// Text form used by the CLI: `*:read,comment,action,write`.
    pub fn to_text(&self) -> String {
        let mut rights: Vec<&str> = self.rights.iter().map(|r| r.as_str()).collect();
        rights.sort_by_key(|r| Right::ALL.iter().position(|x| x.as_str() == *r));
        format!("{}:{}", self.vault, rights.join(","))
    }
}

/// A Member together with their (single, local-Tenant) Membership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemberView {
    pub id: String,
    pub handle: String,
    pub display_name: String,
    pub role: Role,
    pub grants: Vec<Grant>,
    pub created_at: String,
    pub disabled_at: Option<String>,
    pub has_password: bool,
}

impl MemberView {
    pub fn is_disabled(&self) -> bool {
        self.disabled_at.is_some()
    }
}

/// The authenticated caller of a request, resolved from a Token (CLI) or a session cookie
/// (browser, ticket 06).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthMember {
    pub member_id: String,
    pub handle: String,
    pub display_name: String,
    pub role: Role,
    pub grants: Vec<Grant>,
    /// The Credential this request authenticated with.
    pub via: AuthVia,
}

// ---- ticket 06 ----
/// How a request authenticated: a Token (`Authorization: Bearer`, the CLI) or a browser session
/// (cookie — issued by the login page, or automatically to loopback when no password is set).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthVia {
    Token { id: String },
    Session { id: String },
}
// ---- end ticket 06 ----

impl AuthMember {
    pub fn is_owner(&self) -> bool {
        self.role == Role::Owner
    }

    /// Whether the caller holds `right` on `vault` through their Grants ([`Grant::allow`]: the
    /// Vault's own Grant when there is one, else `*`; `read` implied by any Right).
    pub fn has_right(&self, vault: &str, right: Right) -> bool {
        Grant::allow(&self.grants, vault, right)
    }
}

/// A Vault row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Vault {
    pub id: String,
    pub name: String,
    pub default_actions: serde_json::Value,
    pub created_at: String,
}

/// A stored Token (never the plain text).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokenInfo {
    pub id: String,
    pub member_id: String,
    pub name: Option<String>,
    pub created_at: String,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub revoked_at: Option<String>,
    /// sha256 hex of the plain Token — what is at rest.
    pub hash: String,
}

/// One row of the append-only `events` table.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub seq: i64,
    pub at: String,
    pub by: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub doc_id: Option<String>,
    pub payload: serde_json::Value,
}

// ---------------------------------------------------------------- documents & versions

/// A Document with its head Version (every Document has at least v1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentView {
    pub id: String,
    pub vault_id: String,
    pub vault_name: String,
    pub created_at: String,
    pub created_by: String,
    pub deleted_at: Option<String>,
    /// Bumped by every Version/Thread/Action event; `list` sorts by it.
    pub last_activity_at: String,
    pub head: VersionView,
    // ---- ticket 05 ----
    /// The Document's own Action declaration (JSON array of `{id, label, intent}`; `[]` = none);
    /// `None` = the Vault defaults apply (`inherited`). See `crate::actions`.
    pub actions_available: Option<serde_json::Value>,
    // ---- end ticket 05 ----
}

impl DocumentView {
    pub fn is_deleted(&self) -> bool {
        self.deleted_at.is_some()
    }
    /// The Document's Name = its head Version's Name.
    pub fn name(&self) -> &str {
        &self.head.name
    }
}

/// One Version row. `by` is a Member id; `by_handle` its current handle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionView {
    pub doc_id: String,
    pub number: u32,
    pub name: String,
    /// Version identity ([`annotary_sidecar::identity::version_hash`]).
    pub hash: String,
    /// Content address of the Markdown blob.
    pub md_hash: String,
    pub by: String,
    pub by_handle: String,
    pub at: String,
    pub message: Option<String>,
    /// JSON array of manifest entries `{ref, hash, size, type, missing?, via?, width?, height?, derivative?}` (ticket 03).
    pub manifest: serde_json::Value,
    /// JSON array of re-anchoring placements submitted with the commit (ticket 07).
    pub placements: serde_json::Value,
    pub renderer: Option<String>,
    pub html_hash: Option<String>,
    /// Lowest earlier Version number with the same identity, derived at read time — never stored.
    pub same_as: Option<u32>,
}

/// What a commit (create Document or push Version) carries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushInput {
    pub name: String,
    pub md_hash: String,
    /// Manifest entries `{ref, hash, size, type, missing?, via?, width?, height?, derivative?}`
    /// (validated and normalised by `crate::manifest` on commit).
    pub manifest: serde_json::Value,
    pub message: Option<String>,
    /// The Sidecar's `base_version` (None for a new Document or an unbound push).
    pub base_version: Option<u32>,
    // ---- ticket 17 ----
    /// With `base_version` behind the head: the head the pusher checked its file against
    /// (`concurrent::check_head_changes` found every change since the base contained). A commit
    /// over a moved head needs this (== the head at commit time) or `discard_head_changes`.
    pub contains_head: Option<u32>,
    /// `--discard-head-changes`: push over the head's changes since `base_version` (recorded as
    /// `discarded` on the `version.pushed` event).
    pub discard_head_changes: bool,
    // ---- end ticket 17 ----
    // ---- ticket 07 ----
    /// The re-anchoring placements the pusher computed from the head Version to this one:
    /// JSON `[{thread, status, range?, line?, exact?}]` (see `crate::anchors`); `[]` = none sent
    /// (the server places every Thread itself). Threads left out are placed by the server too.
    pub placements: serde_json::Value,
    /// The Version the placements were computed *from*: when it is not the head at commit
    /// time (a concurrent push), the submission is ignored and the server places every Thread.
    pub placements_from: Option<u32>,
    /// `(ref, line)` — the 1-based line of the *new* Markdown where each Asset `ref` is first
    /// referenced (display-only, for Asset Threads; the server handler computes it with the
    /// Asset scanner, the store only stores it).
    pub asset_lines: Vec<(String, u32)>,
    // ---- end ticket 07 ----
    pub renderer: Option<String>,
    pub html_hash: Option<String>,
    // ---- ticket 05 ----
    /// The Sidecar's `actions.available` when it is the Document's own (`inherited` absent):
    /// replaces the Document's declaration on commit (`Some([])` = declares none); `None` =
    /// nothing sent, the stored declaration stays.
    pub actions_available: Option<serde_json::Value>,
    // ---- end ticket 05 ----
}

impl PushInput {
    pub fn new(name: impl Into<String>, md_hash: impl Into<String>) -> PushInput {
        PushInput {
            name: name.into(),
            md_hash: md_hash.into(),
            manifest: serde_json::Value::Array(Vec::new()),
            message: None,
            base_version: None,
            contains_head: None,
            discard_head_changes: false,
            placements: serde_json::Value::Array(Vec::new()),
            placements_from: None,
            asset_lines: Vec::new(),
            renderer: None,
            html_hash: None,
            actions_available: None,
        }
    }
}

/// The head moved between the pusher's `base_version` and now, and the push was accepted
/// (ticket 17: the file contains the head's changes, or they were discarded).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadMoved {
    pub base: u32,
    pub head: u32,
    /// Handles of the Members who pushed `base+1..=head` (deduplicated, in order).
    pub by: Vec<String>,
}

/// Result of a push onto an existing Document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushOutcome {
    /// The Version the content now is (new, or the unchanged head).
    pub version: VersionView,
    /// True when the content was already the head (no Version created, no event).
    pub already: bool,
    pub head_moved: Option<HeadMoved>,
    /// Ticket 17: the Versions whose changes `discard_head_changes` pushed over (empty otherwise).
    pub discarded: Vec<u32>,
    // ---- ticket 07 ----
    /// How the Threads were re-anchored onto the new Version (`None` when no Version was made).
    pub reanchored: Option<crate::anchors::Reanchored>,
    // ---- end ticket 07 ----
}

/// Which Documents a query sees.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DeletedFilter {
    /// Only live Documents (the default everywhere).
    #[default]
    Exclude,
    /// Only deleted Documents (`list --deleted`, `restore`, `purge`).
    Only,
    /// Both (`pull`, `versions`, id lookups).
    Any,
}

/// Filters for listing Documents.
#[derive(Clone, Debug, Default)]
pub struct DocFilter {
    /// Restrict to these Vault ids (`None` = every Vault).
    pub vault_ids: Option<Vec<String>>,
    /// Case-insensitive substring of the head Name.
    pub name_contains: Option<String>,
    /// Exact head Name.
    pub name_exact: Option<String>,
    pub deleted: DeletedFilter,
    /// `last_activity_at >= since` (stored timestamp text).
    pub since: Option<String>,
    /// Maximum rows (`None` = all).
    pub limit: Option<usize>,
}

/// Maximum Name length in bytes (what filesystems allow for one component, minus `.md`).
pub const NAME_MAX_BYTES: usize = 252;

/// Validate a Document Name (the filename stem): verbatim Unicode/case, but never empty, never
/// with path separators or control characters, never starting with `.`.
pub fn validate_name(name: &str) -> Result<()> {
    let err = |reason: &'static str| StoreError::InvalidName { name: name.to_string(), reason };
    if name.is_empty() {
        return Err(err("must not be empty"));
    }
    if name.len() > NAME_MAX_BYTES {
        return Err(err("longer than 252 bytes"));
    }
    if name.contains('/') || name.contains('\\') {
        return Err(err("must not contain path separators"));
    }
    if name.chars().any(char::is_control) {
        return Err(err("must not contain control characters"));
    }
    if name.starts_with('.') {
        return Err(err("must not start with '.'"));
    }
    Ok(())
}

/// Default Vault Actions: `approve` (positive) and `send-back` (neutral).
pub fn default_vault_actions() -> serde_json::Value {
    serde_json::json!([
        {"id": "approve", "label": "Approve", "intent": "positive"},
        {"id": "send-back", "label": "Send back", "intent": "neutral"}
    ])
}

/// Maximum handle length.
pub const HANDLE_MAX_LEN: usize = 39;

/// Validate a Handle: kebab-case (`[a-z0-9]+(-[a-z0-9]+)*`), 1–39 chars.
pub fn validate_handle(handle: &str) -> Result<()> {
    let err = |reason: &'static str| StoreError::InvalidHandle { handle: handle.to_string(), reason };
    if handle.is_empty() {
        return Err(err("must not be empty"));
    }
    if handle.len() > HANDLE_MAX_LEN {
        return Err(err("longer than 39 characters"));
    }
    if !handle.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') {
        return Err(err("only lowercase letters, digits and '-' (kebab-case)"));
    }
    if handle.starts_with('-') || handle.ends_with('-') || handle.contains("--") {
        return Err(err("'-' may only separate words (kebab-case)"));
    }
    Ok(())
}

/// Turn arbitrary text (e.g. `$USER`) into a valid handle suggestion, or `None` if nothing remains.
pub fn suggest_handle(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut last_dash = true;
    for ch in text.chars() {
        let ch = ch.to_ascii_lowercase();
        if ch.is_ascii_lowercase() || ch.is_ascii_digit() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    let out: String = out.chars().take(HANDLE_MAX_LEN).collect();
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_are_kebab_case() {
        for ok in ["rowan", "scribe", "a1", "x-y-z", "linter2"] {
            validate_handle(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for bad in ["", "Rowan", "scribe_bot", "-x", "x-", "a--b", "a b", "ünïcode", &"a".repeat(40)] {
            assert!(validate_handle(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    // spec: S02-05
    #[test]
    fn names_are_verbatim_but_never_empty_separators_control_or_dotfiles() {
        for ok in ["plan", "Plan", "phase1-plan", "Übersicht 2026", "notes.draft", "a b", "日本語", "x"] {
            validate_name(ok).unwrap_or_else(|e| panic!("{ok}: {e}"));
        }
        for bad in ["", "a/b", "a\\b", "tab\there", "nl\nx", ".hidden", "..", &"x".repeat(300)] {
            assert!(validate_name(bad).is_err(), "{bad:?} should be rejected");
        }
        match validate_name(".env") {
            Err(StoreError::InvalidName { name, reason }) => {
                assert_eq!(name, ".env");
                assert!(reason.contains("start with"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn handle_suggestions_normalise() {
        assert_eq!(suggest_handle("Rowan Vale").as_deref(), Some("rowan-vale"));
        assert_eq!(suggest_handle("rowan").as_deref(), Some("rowan"));
        assert_eq!(suggest_handle("__").as_deref(), None);
        assert_eq!(suggest_handle("Scribe_Bot!").as_deref(), Some("scribe-bot"));
    }

    #[test]
    fn grants_cover_rights_and_read_is_implied() {
        let g = Grant { vault: "notes".into(), rights: vec![Right::Comment] };
        assert!(g.covers("notes", Right::Read));
        assert!(g.covers("notes", Right::Comment));
        assert!(!g.covers("notes", Right::Write));
        assert!(!g.covers("drafts", Right::Read));
        assert!(Grant::all().covers("anything", Right::Write));
        assert_eq!(Grant::all().to_text(), "*:read,comment,action,write");
    }

    // spec: S06-01, S06-02
    #[test]
    fn a_vault_grant_overrides_star_and_star_covers_later_vaults() {
        let star = Grant::all();
        let notes = Grant { vault: "notes".into(), rights: vec![Right::Comment] };
        let both = vec![star.clone(), notes.clone()];
        // `notes` has its own entry: comment (and the implied read) only.
        assert!(Grant::allow(&both, "notes", Right::Comment));
        assert!(Grant::allow(&both, "notes", Right::Read));
        assert!(!Grant::allow(&both, "notes", Right::Action));
        assert!(!Grant::allow(&both, "notes", Right::Write));
        // Any other Vault — including one created later — falls back to `*`.
        assert!(Grant::allow(&both, "created-later", Right::Write));
        assert_eq!(Grant::applicable(&both, "notes"), Some(&notes));
        assert_eq!(Grant::applicable(&both, "drafts"), Some(&star));
        // Only a Vault grant: nothing elsewhere.
        assert!(!Grant::allow(std::slice::from_ref(&notes), "drafts", Right::Read));
        assert!(Grant::allow(std::slice::from_ref(&notes), "notes", Right::Read));
        // No Grants: nothing at all; an empty rights list gives nothing either (not even read).
        assert!(!Grant::allow(&[], "notes", Right::Read));
        assert!(!Grant::allow(&[Grant { vault: "*".into(), rights: vec![] }], "notes", Right::Read));
    }
}
