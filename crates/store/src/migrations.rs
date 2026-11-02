//! Forward-only schema migrations keyed by `PRAGMA user_version`.
//!
//! Rules: never edit a shipped migration; append a new SQL string. `SCHEMA_VERSION` is the
//! length of the list. A store whose `user_version` is greater than `SCHEMA_VERSION` belongs to a
//! newer binary and is refused (`StoreError::SchemaTooNew`).

use rusqlite::Connection;

use crate::error::{Result, StoreError};

/// Every migration ever shipped, in order. Migration `i` brings the schema to version `i + 1`.
pub const MIGRATIONS: &[&str] = &[
    // v1 — ticket 01: members, memberships, tokens, vaults, settings, sessions, events.
    r#"
    CREATE TABLE members (
        id            TEXT PRIMARY KEY,
        display_name  TEXT NOT NULL,
        password_hash TEXT,
        created_at    TEXT NOT NULL,
        disabled_at   TEXT
    );
    CREATE TABLE memberships (
        member_id   TEXT PRIMARY KEY REFERENCES members(id),
        handle      TEXT NOT NULL UNIQUE,
        role        TEXT NOT NULL CHECK (role IN ('owner', 'member')),
        grants      TEXT NOT NULL,
        created_at  TEXT NOT NULL,
        disabled_at TEXT
    );
    CREATE TABLE tokens (
        id           TEXT PRIMARY KEY,
        member_id    TEXT NOT NULL REFERENCES members(id),
        hash         TEXT NOT NULL UNIQUE,
        name         TEXT,
        created_at   TEXT NOT NULL,
        expires_at   TEXT,
        last_used_at TEXT,
        revoked_at   TEXT
    );
    CREATE INDEX tokens_member ON tokens(member_id);
    CREATE TABLE vaults (
        id              TEXT PRIMARY KEY,
        name            TEXT NOT NULL UNIQUE,
        default_actions TEXT NOT NULL,
        created_at      TEXT NOT NULL
    );
    CREATE TABLE settings (
        key   TEXT PRIMARY KEY,
        value TEXT NOT NULL
    );
    CREATE TABLE sessions (
        id           TEXT PRIMARY KEY,
        member_id    TEXT NOT NULL REFERENCES members(id),
        created_at   TEXT NOT NULL,
        expires_at   TEXT,
        last_seen_at TEXT
    );
    CREATE TABLE events (
        seq     INTEGER PRIMARY KEY AUTOINCREMENT,
        at      TEXT NOT NULL,
        by      TEXT,
        type    TEXT NOT NULL,
        doc_id  TEXT,
        payload TEXT NOT NULL
    );
    CREATE INDEX events_doc ON events(doc_id, seq);
    "#,
    // v2 — ticket 02: documents + versions. Markdown/assets/derived HTML live in the blob store
    // (`store/blobs/sha256/ab/<hash>`); rows only carry hashes. `manifest`/`placements` are JSON
    // arrays (ticket 03 fills manifest entries, 07 placements); `renderer`/`html_hash` are ticket 09.
    r#"
    CREATE TABLE documents (
        id               TEXT PRIMARY KEY,
        vault_id         TEXT NOT NULL REFERENCES vaults(id),
        created_at       TEXT NOT NULL,
        created_by       TEXT NOT NULL REFERENCES members(id),
        deleted_at       TEXT,
        last_activity_at TEXT NOT NULL
    );
    CREATE INDEX documents_vault_activity ON documents(vault_id, last_activity_at);
    CREATE TABLE versions (
        doc_id     TEXT NOT NULL REFERENCES documents(id),
        number     INTEGER NOT NULL,
        name       TEXT NOT NULL,
        hash       TEXT NOT NULL,
        md_hash    TEXT NOT NULL,
        by         TEXT NOT NULL REFERENCES members(id),
        at         TEXT NOT NULL,
        message    TEXT,
        manifest   TEXT NOT NULL,
        placements TEXT NOT NULL,
        renderer   TEXT,
        html_hash  TEXT,
        PRIMARY KEY (doc_id, number)
    );
    CREATE INDEX versions_hash ON versions(hash);
    "#,
    // ---- ticket 04 (threads) — one migration; renumber freely if another ticket's lands first ----
    // v3 — ticket 04: threads + comments + the append-only resolve/reopen log. Ids are per
    // Document (`t7` = threads.num 7; `t7.1` = comments.num 1 under it; comments.num 0 is the
    // root). `anchor` is the JSON of `annotary_sidecar::Anchor` (`original`/`current`/sparse
    // `history`) for span (and, from ticket 07, asset) Threads; NULL for Document-level. Every
    // stamp is `{by, at, version}` with the head Version at that moment; `resolved_*` mirrors the
    // last row of `thread_events` so open/resolved is one column read; time-slice (07) reads the log.
    r#"
    CREATE TABLE threads (
        doc_id           TEXT NOT NULL REFERENCES documents(id),
        num              INTEGER NOT NULL,
        subject          TEXT NOT NULL CHECK (subject IN ('span', 'document', 'asset')),
        anchor           TEXT,
        created_by       TEXT NOT NULL REFERENCES members(id),
        created_at       TEXT NOT NULL,
        created_version  INTEGER NOT NULL,
        resolved_by      TEXT REFERENCES members(id),
        resolved_at      TEXT,
        resolved_version INTEGER,
        PRIMARY KEY (doc_id, num)
    );
    CREATE TABLE comments (
        doc_id           TEXT NOT NULL,
        thread_num       INTEGER NOT NULL,
        num              INTEGER NOT NULL,
        author           TEXT NOT NULL REFERENCES members(id),
        body             TEXT NOT NULL,
        mentions         TEXT NOT NULL,
        created_at       TEXT NOT NULL,
        created_version  INTEGER NOT NULL,
        edited_at        TEXT,
        deleted_at       TEXT,
        deleted_by       TEXT REFERENCES members(id),
        PRIMARY KEY (doc_id, thread_num, num),
        FOREIGN KEY (doc_id, thread_num) REFERENCES threads(doc_id, num)
    );
    CREATE TABLE thread_events (
        id         INTEGER PRIMARY KEY AUTOINCREMENT,
        doc_id     TEXT NOT NULL,
        thread_num INTEGER NOT NULL,
        kind       TEXT NOT NULL CHECK (kind IN ('resolve', 'reopen')),
        by         TEXT NOT NULL REFERENCES members(id),
        at         TEXT NOT NULL,
        version    INTEGER NOT NULL,
        FOREIGN KEY (doc_id, thread_num) REFERENCES threads(doc_id, num)
    );
    CREATE INDEX thread_events_thread ON thread_events(doc_id, thread_num, id);
    "#,
    // ---- end ticket 04 ----
    // ---- ticket 05 (actions) — one migration; renumber freely if another ticket's lands first ----
    // v4 — ticket 05: the append-only Action log (`{id, by, at, version, label, intent}` — the
    // label/intent are snapshotted from the declaration in force when the Action was taken) and
    // the per-Document declaration `documents.actions_available` (JSON array of
    // `{id, label, intent}`; NULL = the Vault defaults apply, `[]` = the Document declares none).
    r#"
    CREATE TABLE actions (
        id      INTEGER PRIMARY KEY AUTOINCREMENT,
        doc_id  TEXT NOT NULL REFERENCES documents(id),
        version INTEGER NOT NULL,
        action  TEXT NOT NULL,
        label   TEXT NOT NULL,
        intent  TEXT NOT NULL CHECK (intent IN ('positive', 'neutral', 'negative')),
        by      TEXT NOT NULL REFERENCES members(id),
        at      TEXT NOT NULL
    );
    CREATE INDEX actions_doc ON actions(doc_id, id);
    ALTER TABLE documents ADD COLUMN actions_available TEXT;
    "#,
    // ---- end ticket 05 ----
    // ---- CMD-23: Member settings and Devices ----
    // v5 — CMD-23: a Device is one place a Member uses annotary (`web` is every browser, an
    // iPhone / iPad keeps a 26-character Crockford id); keyed by (member, id) so one Member's id
    // never touches another's row. `tokens` / `sessions` remember the Device they were last used
    // from (what Forget revokes). Settings: one value per Member (`member_settings`), the All
    // devices value (`all_device_settings`) and each Device's own value (`device_settings`,
    // `value` NULL = a tombstone: "follow All devices, as of `stamp`"). Values are JSON, stamps
    // the hybrid logical clock strings of `settings.rs`. Every Member who has signed in to a page
    // already has the Web Device.
    r#"
    CREATE TABLE devices (
        member_id    TEXT NOT NULL REFERENCES members(id),
        id           TEXT NOT NULL,
        name         TEXT NOT NULL,
        kind         TEXT NOT NULL CHECK (kind IN ('iphone', 'ipad', 'web', 'cli')),
        created_at   TEXT NOT NULL,
        last_seen_at TEXT,
        forgotten_at TEXT,
        PRIMARY KEY (member_id, id)
    );
    ALTER TABLE tokens ADD COLUMN device_id TEXT;
    ALTER TABLE sessions ADD COLUMN device_id TEXT;
    CREATE TABLE member_settings (
        member_id TEXT NOT NULL REFERENCES members(id),
        key       TEXT NOT NULL,
        value     TEXT NOT NULL,
        stamp     TEXT NOT NULL,
        PRIMARY KEY (member_id, key)
    );
    CREATE TABLE all_device_settings (
        member_id TEXT NOT NULL REFERENCES members(id),
        key       TEXT NOT NULL,
        value     TEXT NOT NULL,
        stamp     TEXT NOT NULL,
        PRIMARY KEY (member_id, key)
    );
    CREATE TABLE device_settings (
        member_id TEXT NOT NULL,
        device_id TEXT NOT NULL,
        key       TEXT NOT NULL,
        value     TEXT,
        stamp     TEXT NOT NULL,
        PRIMARY KEY (member_id, device_id, key),
        FOREIGN KEY (member_id, device_id) REFERENCES devices(member_id, id)
    );
    INSERT INTO devices(member_id, id, name, kind, created_at, last_seen_at)
        SELECT member_id, 'web', 'Web', 'web', MIN(created_at), MAX(COALESCE(last_seen_at, created_at))
        FROM sessions GROUP BY member_id;
    UPDATE sessions SET device_id = 'web';
    "#,
    // ---- end CMD-23 ----
];

/// The schema version this binary writes.
pub const SCHEMA_VERSION: u32 = MIGRATIONS.len() as u32;

/// Read `PRAGMA user_version`.
pub fn current_version(conn: &Connection) -> Result<u32> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))? as u32)
}

/// Apply every migration newer than the store's version, each in its own transaction.
/// Returns the resulting version.
pub fn migrate(conn: &mut Connection) -> Result<u32> {
    let found = current_version(conn)?;
    if found > SCHEMA_VERSION {
        return Err(StoreError::SchemaTooNew { found, supported: SCHEMA_VERSION });
    }
    for (i, sql) in MIGRATIONS.iter().enumerate().skip(found as usize) {
        let target = i as u32 + 1;
        let tx = conn.transaction()?;
        tx.execute_batch(sql)?;
        tx.pragma_update(None, "user_version", target)?;
        tx.commit()?;
    }
    current_version(conn)
}
