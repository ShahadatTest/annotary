//! The server's full projection of a Document as a [`Sidecar`] — what `pull` writes beside the
//! `.md`. Built here (not in the store) so later tickets add their sections in one place:
//! `assets` is the base Version's manifest (`{ref, hash, size, type, missing, via}` — the
//! decided Sidecar shape; dimensions/derivatives stay on the Version record); ticket 04 fills
//! `threads[]` (every Thread, by id — resolved and fully-tombstoned ones included, the projection
//! is lossless) with handles and adds every handle they name to `members`; ticket 05 fills
//! `actions` (`available` = the effective declaration, `inherited: true` when it is the Vault
//! defaults, `log` = every Action taken, oldest first, with snapshotted label/intent — whatever
//! `base_version` is, the projection is the whole Document); 07 anchor history.

use annotary_sidecar::{
    ActionEvent, Actions, AssetEntry, MemberEntry, Members, Reply, Resolution, Sidecar, Stamp, Thread, Tombstone,
    VersionEntry,
};
use annotary_store::{
    CommentView, DocumentView, Store, ThreadFilter, ThreadStatusFilter, ThreadView, VersionView, time,
};

use super::error::ApiError;

/// Build the projection of `doc` with `base_version` = the Version the sibling `.md` is.
pub fn build(store: &Store, doc: &DocumentView, base_version: u32) -> Result<Sidecar, ApiError> {
    let versions = store.versions(&doc.id)?;
    let base = versions
        .iter()
        .find(|v| v.number == base_version)
        .ok_or_else(|| ApiError::not_found(format!("Document {} has no v{base_version}", doc.id)))?;

    let mut threads = store.threads(
        &doc.id,
        &ThreadFilter { status: ThreadStatusFilter::All, include_hidden: true, ..Default::default() },
    )?;
    threads.sort_by_key(|t| t.num); // the projection lists Threads by id (creation order)

    // ---- ticket 05: the effective declaration and the Action log ----
    let (available, inherited) = store.available_actions(doc)?;
    let log = store.actions(&doc.id)?;
    // ---- end ticket 05 ----

    // Handles referenced anywhere in the projection (Version pushers, Comment authors, mentions,
    // resolvers, deleters, Action takers), listed in Membership creation order.
    let mut referenced: Vec<&str> = Vec::new();
    for v in &versions {
        referenced.push(&v.by_handle);
    }
    for a in &log {
        referenced.push(&a.by_handle);
    }
    for t in &threads {
        for c in std::iter::once(&t.root).chain(t.replies.iter()) {
            referenced.push(&c.author_handle);
            referenced.extend(c.mention_handles.iter().map(String::as_str));
            if let Some(d) = &c.deleted {
                referenced.push(&d.by_handle);
            }
        }
        if let Some(r) = &t.resolved {
            referenced.push(&r.by_handle);
        }
    }
    let mut members = Members::default();
    for m in store.list_members()? {
        if referenced.contains(&m.handle.as_str()) {
            members.insert(m.handle.clone(), MemberEntry { id: m.id.clone(), name: m.display_name.clone() });
        }
    }

    Ok(Sidecar {
        format: annotary_sidecar::FORMAT,
        id: doc.id.clone(),
        vault: doc.vault_name.clone(),
        base_version,
        members,
        versions: versions.iter().map(version_entry).collect(),
        assets: assets_of(base),
        actions: Actions { available, inherited, log: log.iter().map(action_entry).collect() },
        threads: threads.iter().map(thread_entry).collect(),
    })
}

/// One `threads[]` entry from a stored Thread (handles, seconds, empty/null omitted by the writer).
pub fn thread_entry(t: &ThreadView) -> Thread {
    Thread {
        id: t.id.clone(),
        subject: t.subject.as_str().to_string(),
        anchor: t.anchor.clone(),
        author: t.root.author_handle.clone(),
        created: Stamp { at: time::to_seconds(&t.root.at), version: t.root.version },
        body: t.root.body.clone(),
        mentions: t.root.mention_handles.clone(),
        edited_at: t.root.edited_at.as_deref().map(time::to_seconds),
        deleted: tombstone(&t.root),
        resolved: t.resolved.as_ref().map(|r| Resolution {
            by: r.by_handle.clone(),
            at: time::to_seconds(&r.at),
            version: r.version,
        }),
        replies: t.replies.iter().map(reply_entry).collect(),
    }
}

fn reply_entry(c: &CommentView) -> Reply {
    Reply {
        id: c.id.clone(),
        author: c.author_handle.clone(),
        created: Stamp { at: time::to_seconds(&c.at), version: c.version },
        body: c.body.clone(),
        mentions: c.mention_handles.clone(),
        edited_at: c.edited_at.as_deref().map(time::to_seconds),
        deleted: tombstone(c),
    }
}

fn tombstone(c: &CommentView) -> Option<Tombstone> {
    c.deleted.as_ref().map(|d| Tombstone { at: time::to_seconds(&d.at), by: d.by_handle.clone() })
}

/// One `actions.log[]` entry (handle, seconds, snapshotted label/intent).
pub fn action_entry(a: &annotary_store::ActionView) -> ActionEvent {
    ActionEvent {
        id: a.id.clone(),
        by: a.by_handle.clone(),
        at: time::to_seconds(&a.at),
        version: a.version,
        label: a.label.clone(),
        intent: a.intent.clone(),
    }
}

pub fn version_entry(v: &VersionView) -> VersionEntry {
    VersionEntry {
        number: v.number,
        name: v.name.clone(),
        hash: v.hash.clone(),
        by: v.by_handle.clone(),
        at: time::to_seconds(&v.at),
        message: v.message.clone().filter(|m| !m.is_empty()),
    }
}

/// `assets[]` for a Version from its manifest entries.
pub fn assets_of(v: &VersionView) -> Vec<AssetEntry> {
    let Some(entries) = v.manifest.as_array() else { return Vec::new() };
    entries
        .iter()
        .filter_map(|e| {
            let s = |k: &str| e.get(k).and_then(|x| x.as_str()).map(str::to_string);
            Some(AssetEntry {
                reference: s("ref")?,
                hash: s("hash"),
                size: e.get("size").and_then(|x| x.as_u64()),
                content_type: s("type").unwrap_or_else(|| "application/octet-stream".into()),
                missing: e.get("missing").and_then(|x| x.as_bool()).unwrap_or(false),
                via: s("via"),
            })
        })
        .collect()
}
