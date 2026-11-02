//! Forward-only migrations keyed by `PRAGMA user_version`; `meta.json` mirrors the version.

use annotary_store::migrations::{MIGRATIONS, SCHEMA_VERSION, current_version};
use annotary_store::{Meta, Store, StoreError};

fn table_names(conn: &rusqlite::Connection) -> Vec<String> {
    let mut stmt = conn.prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name").unwrap();
    stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().map(|r| r.unwrap()).collect()
}

// spec: S01-04, S01-19
#[test]
fn fresh_store_is_at_latest_and_reopen_is_a_no_op() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    {
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let meta = Meta::read(&path).unwrap().unwrap();
        assert_eq!(meta.schema_version, SCHEMA_VERSION);
        assert_eq!(meta.tenant_id, None);
    }
    let store = Store::open(&path).unwrap();
    assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
    let conn = rusqlite::Connection::open(path.join(annotary_store::DB_FILE)).unwrap();
    let tables = table_names(&conn);
    for t in ["members", "memberships", "tokens", "vaults", "settings", "sessions", "events", "documents", "versions"] {
        assert!(tables.contains(&t.to_string()), "missing table {t}: {tables:?}");
    }
    let mode: String = conn.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
    assert_eq!(mode.to_lowercase(), "wal");
}

// spec: S01-04
#[test]
fn migrates_forward_from_every_prior_schema_version() {
    for from in 0..SCHEMA_VERSION {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store");
        std::fs::create_dir_all(&path).unwrap();
        {
            let mut conn = rusqlite::Connection::open(path.join(annotary_store::DB_FILE)).unwrap();
            for sql in MIGRATIONS.iter().take(from as usize) {
                let tx = conn.transaction().unwrap();
                tx.execute_batch(sql).unwrap();
                tx.commit().unwrap();
            }
            conn.pragma_update(None, "user_version", from).unwrap();
            assert_eq!(current_version(&conn).unwrap(), from);
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION, "from v{from}");
        assert_eq!(Meta::read(&path).unwrap().unwrap().schema_version, SCHEMA_VERSION);
    }
}

// spec: S01-04
#[test]
fn a_newer_store_is_refused_never_downgraded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    {
        let store = Store::open(&path).unwrap();
        drop(store);
        let conn = rusqlite::Connection::open(path.join(annotary_store::DB_FILE)).unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 1).unwrap();
    }
    match Store::open(&path) {
        Err(StoreError::SchemaTooNew { found, supported }) => {
            assert_eq!((found, supported), (SCHEMA_VERSION + 1, SCHEMA_VERSION));
        }
        other => panic!("expected SchemaTooNew, got {:?}", other.map(|_| ())),
    }
    let conn = rusqlite::Connection::open(path.join(annotary_store::DB_FILE)).unwrap();
    assert_eq!(current_version(&conn).unwrap(), SCHEMA_VERSION + 1, "refusal must not touch user_version");
}
