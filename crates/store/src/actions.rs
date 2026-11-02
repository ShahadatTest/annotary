//! Actions on the [`Store`] (ticket 05): the declaration (`{id, label, intent}` — Vault defaults
//! or a Document's own set), taking an Action on head, and the reads behind `status`, `list`,
//! `show`, `actions` and the Sidecar's `actions` block.
//!
//! - **Declaration** = a list of `{id, label, intent ∈ positive|neutral|negative}`. Every Vault
//!   carries defaults (`approve`/positive, `send-back`/neutral — owner-editable,
//!   [`Store::set_vault_actions`]); a Document may declare its own set on push
//!   (`documents.actions_available`, [`declare`]) which replaces the Vault set entirely (`[]` =
//!   no Actions). Changing the Vault defaults affects only Documents without their own
//!   declaration, and only future Actions. [`Store::available_actions`] is the effective set.
//! - **Taking an Action** ([`Store::act`]) is head-only (`expect_version` ≠ head ⇒ `NotHead`),
//!   needs a declared id (`UndeclaredAction` otherwise), snapshots `label`/`intent` from the
//!   declaration in force, is append-only (no note, no retract), bumps `last_activity_at` and
//!   appends `action.taken {action, version, label, intent, name}` to `events`. Several
//!   Members / repeated Actions on one Version are all recorded; the last event on a Version
//!   wins for reporting ([`Store::head_action`]). A new Version starts with no Action — nothing
//!   is carried forward or recomputed; older Versions keep theirs in the log.

use std::collections::HashMap;

use rusqlite::{OptionalExtension, Transaction, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use annotary_sidecar::ActionDecl;

use crate::docs::bump_activity;
use crate::error::{Result, StoreError};
use crate::model::{DocumentView, Vault};
use crate::store::{Store, append_event};
use crate::time;

/// The three intents, in the declared order.
pub const INTENTS: [&str; 3] = ["positive", "neutral", "negative"];

/// Maximum length of an Action id (same as a handle).
pub const ACTION_ID_MAX_LEN: usize = 39;

/// One recorded Action (a row of the append-only log).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionView {
    /// The Action id as declared (`approve`).
    pub id: String,
    /// Label snapshotted when taken.
    pub label: String,
    /// Intent snapshotted when taken.
    pub intent: String,
    /// Member id of who took it; `by_handle` their current handle.
    pub by: String,
    pub by_handle: String,
    pub at: String,
    /// The head Version it was taken on.
    pub version: u32,
}

/// Validate an Action id: kebab-case (`[a-z0-9]+(-[a-z0-9]+)*`), 1–39 chars, and never `none`
/// (reserved by `list --action none`).
pub fn validate_action_id(id: &str) -> std::result::Result<(), String> {
    if id.is_empty() {
        return Err("id must not be empty".into());
    }
    if id.len() > ACTION_ID_MAX_LEN {
        return Err(format!("id {id:?} is longer than {ACTION_ID_MAX_LEN} characters"));
    }
    if !id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        || id.starts_with('-')
        || id.ends_with('-')
        || id.contains("--")
    {
        return Err(format!("id {id:?} must be kebab-case (lowercase letters, digits, '-' between words)"));
    }
    if id == "none" {
        return Err("id \"none\" is reserved (`list --action none`)".into());
    }
    Ok(())
}

/// Validate and normalise a declaration (`actions_available` of a commit, or new Vault
/// defaults): an array of objects `{id, label?, intent?}`; ids kebab-case and unique; `label`
/// defaults to the id, `intent` to `neutral` when absent; a present but unknown intent is
/// refused. Unknown keys are dropped.
pub fn validate_declaration(value: &Value) -> Result<Vec<ActionDecl>> {
    let invalid = |m: String| StoreError::InvalidActions(m);
    let Some(items) = value.as_array() else {
        return Err(invalid("must be an array of {id, label, intent}".into()));
    };
    let mut out: Vec<ActionDecl> = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        let Some(obj) = item.as_object() else {
            return Err(invalid(format!("entry {i} must be an object {{id, label, intent}}")));
        };
        let id = match obj.get("id") {
            Some(Value::String(s)) => s.trim().to_string(),
            Some(_) => return Err(invalid(format!("entry {i}: id must be a string"))),
            None => return Err(invalid(format!("entry {i}: missing id"))),
        };
        validate_action_id(&id).map_err(|m| invalid(format!("entry {i}: {m}")))?;
        if out.iter().any(|d| d.id == id) {
            return Err(invalid(format!("id {id:?} is declared twice")));
        }
        let label = match obj.get("label") {
            None | Some(Value::Null) => id.clone(),
            Some(Value::String(s)) if !s.trim().is_empty() => s.trim().to_string(),
            Some(Value::String(_)) => id.clone(),
            Some(_) => return Err(invalid(format!("{id}: label must be a string"))),
        };
        if label.chars().count() > 80 {
            return Err(invalid(format!("{id}: label is longer than 80 characters")));
        }
        let intent = match obj.get("intent") {
            None | Some(Value::Null) => "neutral".to_string(),
            Some(Value::String(s)) if INTENTS.contains(&s.trim()) => s.trim().to_string(),
            Some(other) => {
                return Err(invalid(format!(
                    "{id}: intent {} is not one of positive|neutral|negative",
                    other.as_str().map(|s| format!("{s:?}")).unwrap_or_else(|| other.to_string())
                )));
            }
        };
        out.push(ActionDecl { id, label, intent });
    }
    Ok(out)
}

/// A declaration as stored (canonical JSON of `[{id, label, intent}]`).
pub fn declaration_json(decls: &[ActionDecl]) -> Value {
    serde_json::to_value(decls).unwrap_or(Value::Array(Vec::new()))
}

/// Read a stored declaration (`None`/unparsable ⇒ empty).
pub fn declaration_from(value: Option<&Value>) -> Vec<ActionDecl> {
    value.and_then(|v| serde_json::from_value(v.clone()).ok()).unwrap_or_default()
}

/// Store a Document's own declaration inside the caller's transaction (push / create). Appends
/// `document.actions_declared {name, available}` when it actually changed.
pub(crate) fn declare(tx: &Transaction, by: &str, doc_id: &str, name: &str, decls: &[ActionDecl]) -> Result<()> {
    let text = serde_json::to_string(&declaration_json(decls))?;
    let current: Option<Option<String>> =
        tx.query_row("SELECT actions_available FROM documents WHERE id = ?1", [doc_id], |r| r.get(0)).optional()?;
    let Some(current) = current else {
        return Err(StoreError::DocumentNotFound(doc_id.to_string()));
    };
    if current.as_deref() == Some(text.as_str()) {
        return Ok(());
    }
    tx.execute("UPDATE documents SET actions_available = ?1 WHERE id = ?2", params![text, doc_id])?;
    append_event(
        tx,
        Some(by),
        "document.actions_declared",
        Some(doc_id),
        serde_json::json!({ "name": name, "available": declaration_json(decls) }),
    )?;
    Ok(())
}

/// The effective declaration of a Document: its own, else the Vault defaults. Returns
/// `(available, inherited)`.
pub fn effective(doc: &DocumentView, vault: Option<&Vault>) -> (Vec<ActionDecl>, bool) {
    match &doc.actions_available {
        Some(own) => (declaration_from(Some(own)), false),
        None => (declaration_from(vault.map(|v| &v.default_actions)), true),
    }
}

const ACTION_SELECT: &str = "SELECT a.action, a.label, a.intent, a.by, ms.handle, a.at, a.version, a.doc_id
     FROM actions a JOIN memberships ms ON ms.member_id = a.by";

fn row_to_action(r: &rusqlite::Row<'_>) -> rusqlite::Result<(String, ActionView)> {
    Ok((
        r.get(7)?,
        ActionView {
            id: r.get(0)?,
            label: r.get(1)?,
            intent: r.get(2)?,
            by: r.get(3)?,
            by_handle: r.get(4)?,
            at: r.get(5)?,
            version: r.get::<_, i64>(6)? as u32,
        },
    ))
}

impl Store {
    /// The effective declaration of a Document (`(available, inherited)`).
    pub fn available_actions(&self, doc: &DocumentView) -> Result<(Vec<ActionDecl>, bool)> {
        let vault = self.vault_by_id(&doc.vault_id)?;
        Ok(effective(doc, vault.as_ref()))
    }

    /// Replace a Vault's default Actions (owner-only at the API). Appends
    /// `vault.actions_set {vault, name, available}`. Documents with their own declaration and
    /// Actions already taken are untouched.
    pub fn set_vault_actions(&self, by: &str, vault_id: &str, declaration: &Value) -> Result<Vault> {
        let decls = validate_declaration(declaration)?;
        let json = declaration_json(&decls);
        self.tx(|tx| {
            let n = tx.execute(
                "UPDATE vaults SET default_actions = ?1 WHERE id = ?2",
                params![serde_json::to_string(&json)?, vault_id],
            )?;
            if n == 0 {
                return Err(StoreError::VaultNotFound(vault_id.to_string()));
            }
            let name: String = tx.query_row("SELECT name FROM vaults WHERE id = ?1", [vault_id], |r| r.get(0))?;
            append_event(
                tx,
                Some(by),
                "vault.actions_set",
                None,
                serde_json::json!({ "vault": vault_id, "name": name, "available": json }),
            )?;
            Ok(())
        })?;
        self.vault_by_id(vault_id)?.ok_or_else(|| StoreError::VaultNotFound(vault_id.to_string()))
    }

    /// Take Action `action` on the head Version of `doc_id` (`action` right at the API).
    /// `expect_version` ≠ head ⇒ `NotHead`; an id not in the effective declaration ⇒
    /// `UndeclaredAction` (naming the available ids); a deleted Document ⇒ `DocumentDeleted`.
    pub fn act(&self, by: &str, doc_id: &str, action: &str, expect_version: Option<u32>) -> Result<ActionView> {
        let action = action.trim();
        let row_id = self.tx(|tx| {
            let deleted: Option<Option<String>> =
                tx.query_row("SELECT deleted_at FROM documents WHERE id = ?1", [doc_id], |r| r.get(0)).optional()?;
            match deleted {
                None => return Err(StoreError::DocumentNotFound(doc_id.to_string())),
                Some(Some(_)) => return Err(StoreError::DocumentDeleted(doc_id.to_string())),
                Some(None) => {}
            }
            let (head, name): (u32, String) = tx.query_row(
                "SELECT number, name FROM versions WHERE doc_id = ?1 ORDER BY number DESC LIMIT 1",
                [doc_id],
                |r| Ok((r.get::<_, i64>(0)? as u32, r.get(1)?)),
            )?;
            if let Some(v) = expect_version
                && v != head
            {
                return Err(StoreError::NotHead { expected: v, head });
            }
            let (own, defaults): (Option<String>, String) = tx.query_row(
                "SELECT d.actions_available, vl.default_actions FROM documents d JOIN vaults vl ON vl.id = d.vault_id
                 WHERE d.id = ?1",
                [doc_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let available: Vec<ActionDecl> = match own {
                Some(o) => serde_json::from_str(&o).unwrap_or_default(),
                None => serde_json::from_str(&defaults).unwrap_or_default(),
            };
            let Some(decl) = available.iter().find(|d| d.id == action) else {
                return Err(StoreError::UndeclaredAction {
                    action: action.to_string(),
                    available: available.iter().map(|d| d.id.clone()).collect(),
                });
            };
            let now = time::now();
            tx.execute(
                "INSERT INTO actions(doc_id, version, action, label, intent, by, at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![doc_id, head as i64, decl.id, decl.label, decl.intent, by, now],
            )?;
            let row_id = tx.last_insert_rowid();
            bump_activity(tx, doc_id, &now)?;
            append_event(
                tx,
                Some(by),
                "action.taken",
                Some(doc_id),
                serde_json::json!({
                    "action": decl.id, "version": head, "label": decl.label, "intent": decl.intent, "name": name,
                }),
            )?;
            Ok(row_id)
        })?;
        let conn = self.conn();
        let (_, view) = conn.query_row(&format!("{ACTION_SELECT} WHERE a.id = ?1"), [row_id], row_to_action)?;
        Ok(view)
    }

    /// The Action log of a Document, oldest first.
    pub fn actions(&self, doc_id: &str) -> Result<Vec<ActionView>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!("{ACTION_SELECT} WHERE a.doc_id = ?1 ORDER BY a.id"))?;
        let rows = stmt.query_map([doc_id], row_to_action)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?.1);
        }
        Ok(out)
    }

    /// The Action that counts for reporting: the last one taken on the head Version (`None`
    /// when the head has none — a new Version starts clean).
    pub fn head_action(&self, doc_id: &str, head: u32) -> Result<Option<ActionView>> {
        let conn = self.conn();
        Ok(conn
            .query_row(
                &format!("{ACTION_SELECT} WHERE a.doc_id = ?1 AND a.version = ?2 ORDER BY a.id DESC LIMIT 1"),
                params![doc_id, head as i64],
                row_to_action,
            )
            .optional()?
            .map(|(_, v)| v))
    }

    /// The last Action of every Document that has any, keyed by Document id — one pass for
    /// `list`. Actions are always taken on the head at the time, so a Document's last row is on
    /// its highest acted-on Version: it is the head Action iff `version` equals the current head.
    pub fn last_actions_all(&self) -> Result<HashMap<String, ActionView>> {
        let conn = self.conn();
        let mut stmt = conn.prepare(&format!(
            "{ACTION_SELECT} JOIN (SELECT doc_id, MAX(id) AS last_id FROM actions GROUP BY doc_id) l
                 ON l.doc_id = a.doc_id AND l.last_id = a.id"
        ))?;
        let rows = stmt.query_map([], row_to_action)?;
        let mut out = HashMap::new();
        for r in rows {
            let (doc_id, view) = r?;
            out.insert(doc_id, view);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::model::PushInput;
    use crate::store::{SetupInput, SetupOutcome};

    fn fresh() -> (tempfile::TempDir, Store, SetupOutcome) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("store")).unwrap();
        let out = store
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan Vale".into(), password: None })
            .unwrap();
        (dir, store, out)
    }

    fn doc(store: &Store, out: &SetupOutcome, md: &str) -> DocumentView {
        let (h, _) = store.put_blob(md.as_bytes()).unwrap();
        store.create_document(&out.owner.id, &out.vault.id, PushInput::new("plan", h)).unwrap()
    }

    fn push(store: &Store, out: &SetupOutcome, id: &str, md: &str) -> u32 {
        let (h, _) = store.put_blob(md.as_bytes()).unwrap();
        store.push_version(&out.owner.id, id, PushInput::new("plan", h)).unwrap().version.number
    }

    // spec: S05-01, S05-16
    #[test]
    fn declarations_are_validated_and_normalised() {
        let ok = validate_declaration(&json!([
            {"id": "approve", "label": "Approve", "intent": "positive"},
            {"id": "ship-it", "extra": 1},
        ]))
        .unwrap();
        assert_eq!(ok[0], ActionDecl { id: "approve".into(), label: "Approve".into(), intent: "positive".into() });
        assert_eq!(ok[1], ActionDecl { id: "ship-it".into(), label: "ship-it".into(), intent: "neutral".into() });
        assert_eq!(validate_declaration(&json!([])).unwrap(), Vec::<ActionDecl>::new());
        for bad in [
            json!({"id": "x"}),
            json!(["approve"]),
            json!([{"label": "no id"}]),
            json!([{"id": "Approve"}]),
            json!([{"id": "none"}]),
            json!([{"id": "a--b"}]),
            json!([{"id": "a"}, {"id": "a"}]),
            json!([{"id": "a", "intent": "good"}]),
            json!([{"id": "a", "label": 3}]),
        ] {
            assert!(matches!(validate_declaration(&bad), Err(StoreError::InvalidActions(_))), "{bad}");
        }
    }

    // spec: S05-01, S05-02, S05-03
    #[test]
    fn act_is_head_only_snapshotted_and_a_new_version_starts_clean() {
        let (_d, store, out) = fresh();
        let d = doc(&store, &out, "# v1\n");
        let (avail, inherited) = store.available_actions(&d).unwrap();
        assert!(inherited);
        assert_eq!(avail.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["approve", "send-back"]);
        assert!(store.head_action(&d.id, 1).unwrap().is_none());

        let a = store.act(&out.owner.id, &d.id, "approve", Some(1)).unwrap();
        assert_eq!(
            (a.id.as_str(), a.label.as_str(), a.intent.as_str(), a.version),
            ("approve", "Approve", "positive", 1)
        );
        assert_eq!(a.by_handle, "rowan");
        assert_eq!(store.head_action(&d.id, 1).unwrap().as_ref().map(|a| a.id.as_str()), Some("approve"));

        // Guards: --version ≠ head, undeclared id.
        assert!(matches!(
            store.act(&out.owner.id, &d.id, "approve", Some(2)),
            Err(StoreError::NotHead { expected: 2, head: 1 })
        ));
        match store.act(&out.owner.id, &d.id, "ship", None) {
            Err(StoreError::UndeclaredAction { action, available }) => {
                assert_eq!(action, "ship");
                assert_eq!(available, vec!["approve", "send-back"]);
            }
            other => panic!("{other:?}"),
        }

        // A new Version starts with no Action; the log keeps v1's.
        assert_eq!(push(&store, &out, &d.id, "# v2\n"), 2);
        assert!(store.head_action(&d.id, 2).unwrap().is_none());
        assert!(store.last_actions_all().unwrap().get(&d.id).is_some_and(|a| a.version == 1));
        assert_eq!(store.actions(&d.id).unwrap().len(), 1);

        // Repeated Actions on one Version are all recorded; the last wins.
        store.act(&out.owner.id, &d.id, "approve", None).unwrap();
        store.act(&out.owner.id, &d.id, "send-back", None).unwrap();
        assert_eq!(store.head_action(&d.id, 2).unwrap().unwrap().id, "send-back");
        assert_eq!(store.actions(&d.id).unwrap().len(), 3);
        let events = store.events_since(0).unwrap();
        let taken: Vec<_> = events.iter().filter(|e| e.kind == "action.taken").collect();
        assert_eq!(taken.len(), 3);
        assert_eq!(taken[2].payload["action"], "send-back");
        assert_eq!(taken[2].payload["version"], 2);
        assert_eq!(taken[2].payload["name"], "plan");
        assert_eq!(taken[2].doc_id.as_deref(), Some(d.id.as_str()));
        let d2 = store.document(&d.id).unwrap().unwrap();
        assert!(d2.last_activity_at >= d.last_activity_at);
    }

    // spec: S05-04, S05-05, S05-06
    #[test]
    fn per_document_declaration_replaces_the_vault_set_and_snapshots_survive() {
        let (_d, store, out) = fresh();
        let (h, _) = store.put_blob(b"# v1\n").unwrap();
        let mut input = PushInput::new("plan", h.clone());
        input.actions_available = Some(json!([{"id": "ship", "label": "Ship it", "intent": "positive"}]));
        let d = store.create_document(&out.owner.id, &out.vault.id, input).unwrap();
        let (avail, inherited) = store.available_actions(&d).unwrap();
        assert!(!inherited);
        assert_eq!(avail.iter().map(|a| a.id.as_str()).collect::<Vec<_>>(), vec!["ship"]);
        assert!(matches!(store.act(&out.owner.id, &d.id, "approve", None), Err(StoreError::UndeclaredAction { .. })));
        let a = store.act(&out.owner.id, &d.id, "ship", None).unwrap();
        assert_eq!(a.label, "Ship it");

        // Redeclare with a different label: the log keeps the snapshot.
        let mut input = PushInput::new("plan", h.clone());
        input.actions_available = Some(json!([{"id": "ship", "label": "Launch", "intent": "neutral"}]));
        let outcome = store.push_version(&out.owner.id, &d.id, input).unwrap();
        assert!(outcome.already, "identical content, but the declaration still applies");
        let d = store.document(&d.id).unwrap().unwrap();
        assert_eq!(store.available_actions(&d).unwrap().0[0].label, "Launch");
        assert_eq!(store.actions(&d.id).unwrap()[0].label, "Ship it");
        let b = store.act(&out.owner.id, &d.id, "ship", None).unwrap();
        assert_eq!((b.label.as_str(), b.intent.as_str()), ("Launch", "neutral"));
        let declared: Vec<_> =
            store.events_since(0).unwrap().into_iter().filter(|e| e.kind == "document.actions_declared").collect();
        assert_eq!(declared.len(), 2);

        // `[]` declares none; nothing sent keeps the declaration.
        let mut input = PushInput::new("plan", h.clone());
        input.actions_available = Some(json!([]));
        store.push_version(&out.owner.id, &d.id, input).unwrap();
        let d = store.document(&d.id).unwrap().unwrap();
        assert_eq!(store.available_actions(&d).unwrap(), (vec![], false));
        match store.act(&out.owner.id, &d.id, "ship", None) {
            Err(StoreError::UndeclaredAction { available, .. }) => assert!(available.is_empty()),
            other => panic!("{other:?}"),
        }
        let (h2, _) = store.put_blob(b"# v2\n").unwrap();
        store.push_version(&out.owner.id, &d.id, PushInput::new("plan", h2)).unwrap();
        let d = store.document(&d.id).unwrap().unwrap();
        assert_eq!(store.available_actions(&d).unwrap(), (vec![], false), "nothing sent ⇒ unchanged");
        // Invalid declarations are refused before anything is written.
        let mut input = PushInput::new("plan", h.clone());
        input.actions_available = Some(json!([{"id": "Bad Id"}]));
        assert!(matches!(store.push_version(&out.owner.id, &d.id, input), Err(StoreError::InvalidActions(_))));
    }

    // spec: S05-01, S05-04
    #[test]
    fn vault_defaults_change_only_future_actions_of_inheriting_documents() {
        let (_d, store, out) = fresh();
        let inheriting = doc(&store, &out, "# a\n");
        let (h, _) = store.put_blob(b"# b\n").unwrap();
        let mut input = PushInput::new("plan", h);
        input.actions_available = Some(json!([{"id": "approve", "label": "Own approve", "intent": "positive"}]));
        let own = store.create_document(&out.owner.id, &out.vault.id, input).unwrap();
        let first = store.act(&out.owner.id, &inheriting.id, "approve", None).unwrap();
        assert_eq!(first.label, "Approve");

        let v = store
            .set_vault_actions(
                &out.owner.id,
                &out.vault.id,
                &json!([{"id": "lgtm", "label": "LGTM", "intent": "positive"}]),
            )
            .unwrap();
        assert_eq!(v.default_actions, json!([{"id": "lgtm", "label": "LGTM", "intent": "positive"}]));
        let inheriting = store.document(&inheriting.id).unwrap().unwrap();
        assert_eq!(store.available_actions(&inheriting).unwrap().0[0].id, "lgtm");
        assert!(matches!(
            store.act(&out.owner.id, &inheriting.id, "approve", None),
            Err(StoreError::UndeclaredAction { .. })
        ));
        assert_eq!(
            store.actions(&inheriting.id).unwrap()[0].label,
            "Approve",
            "already-taken Actions keep their snapshot"
        );
        let own = store.document(&own.id).unwrap().unwrap();
        assert_eq!(store.available_actions(&own).unwrap().0[0].label, "Own approve", "own declaration untouched");
        assert!(matches!(
            store.set_vault_actions(&out.owner.id, &out.vault.id, &json!([{"id": "none"}])),
            Err(StoreError::InvalidActions(_))
        ));
        assert!(matches!(
            store.set_vault_actions(&out.owner.id, "nope", &json!([])),
            Err(StoreError::VaultNotFound(_))
        ));
        let ev = store.events_since(0).unwrap().into_iter().find(|e| e.kind == "vault.actions_set").unwrap();
        assert_eq!(ev.payload["name"], "notes");
    }

    // spec: S05-03, S05-13
    #[test]
    fn deleted_documents_refuse_and_purge_drops_the_log() {
        let (_d, store, out) = fresh();
        let d = doc(&store, &out, "# a\n");
        store.act(&out.owner.id, &d.id, "approve", None).unwrap();
        store.delete_document(&out.owner.id, &d.id).unwrap();
        assert!(matches!(store.act(&out.owner.id, &d.id, "approve", None), Err(StoreError::DocumentDeleted(_))));
        assert_eq!(store.actions(&d.id).unwrap().len(), 1, "the log survives a soft delete");
        store.purge_document(&out.owner.id, &d.id).unwrap();
        assert!(store.actions(&d.id).unwrap().is_empty());
        assert!(store.last_actions_all().unwrap().is_empty());
        assert!(matches!(store.act(&out.owner.id, &d.id, "approve", None), Err(StoreError::DocumentNotFound(_))));
    }
}
