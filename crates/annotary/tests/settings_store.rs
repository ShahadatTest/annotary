//! CMD-23 in the store: the migration, the catalog and its colors, resolution, All devices and
//! tombstones, stamps and the server clock, events, Devices (cap, seen, forget) and Copy from
//! another device, the Default Vault. The HTTP side is `settings_api.rs`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use annotary_store::settings::{self, CATALOG, PRESETS, Refusal, Stamp};
use annotary_store::{AuthVia, DeviceTouch, PutSetting, Scope, SetupInput, Store, StoreError};
use serde_json::{Value, json};

fn store() -> (tempfile::TempDir, Store, String) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(&dir.path().join("store")).unwrap();
    let out = store
        .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan Vale".into(), password: None })
        .unwrap();
    (dir, store, out.owner.id)
}

fn put(store: &Store, m: &str, key: &str, value: Value, scope: Scope) -> annotary_store::PutOutcome {
    store.put_setting(m, &PutSetting { key: key.into(), value, scope, stamp: None }).unwrap()
}

fn put_stamped(store: &Store, m: &str, key: &str, value: Value, scope: Scope, stamp: &str) -> Result<bool, StoreError> {
    store.put_setting(m, &PutSetting { key: key.into(), value, scope, stamp: Some(stamp.into()) }).map(|o| o.changed)
}

fn devices(ids: &[&str]) -> Scope {
    Scope::Devices(ids.iter().map(|s| s.to_string()).collect())
}

fn resolved(store: &Store, m: &str, key: &str, device: Option<&str>) -> (Value, &'static str) {
    let r = store.resolved_setting(m, key, device).unwrap();
    (r.value, r.source)
}

fn iphone(store: &Store, m: &str) -> String {
    let id = settings::new_device_id();
    store.ensure_device(m, &id, Some(("iphone", "iPhone 17 Pro"))).unwrap();
    id
}

fn events_of(store: &Store, kind: &str) -> Vec<Value> {
    store.events_since(0).unwrap().into_iter().filter(|e| e.kind == kind).map(|e| e.payload).collect()
}

// spec: S29-01
#[test]
fn a_v4_store_migrates_with_its_data_and_web_devices() {
    use annotary_store::migrations::MIGRATIONS;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    // A v4 store as the previous binary left it: a Member with a session, a Document, a Thread.
    let (owner, doc) = {
        let s = Store::open(&path).unwrap();
        let out =
            s.setup(SetupInput { owner_handle: "rowan".into(), owner_name: "Rowan".into(), password: None }).unwrap();
        s.open_session(&out.owner.id, "loopback").unwrap();
        let (hash, _) = s.put_blob(b"# hello\n\nworld\n").unwrap();
        let d = s.create_document(&out.owner.id, &out.vault.id, annotary_store::PushInput::new("hello", hash)).unwrap();
        s.new_thread(&out.owner.id, &d.id, annotary_store::NewThread::document("a thread")).unwrap();
        (out.owner.id, d.id)
    };
    {
        // Roll the file back to v4: drop what v5 added and reset user_version.
        let conn = rusqlite::Connection::open(path.join(annotary_store::DB_FILE)).unwrap();
        conn.execute_batch(
            "DROP TABLE device_settings; DROP TABLE all_device_settings; DROP TABLE member_settings; DROP TABLE devices;
             ALTER TABLE tokens DROP COLUMN device_id; ALTER TABLE sessions DROP COLUMN device_id;",
        )
        .unwrap();
        conn.pragma_update(None, "user_version", MIGRATIONS.len() as u32 - 1).unwrap();
    }
    let s = Store::open(&path).unwrap();
    assert_eq!(s.schema_version().unwrap(), annotary_store::SCHEMA_VERSION);
    assert_eq!(s.document(&doc).unwrap().map(|d| d.id), Some(doc.clone()));
    assert_eq!(s.threads(&doc, &Default::default()).unwrap().len(), 1);
    let devs = s.list_devices(&owner, false).unwrap();
    assert_eq!(devs.iter().map(|d| (d.id.as_str(), d.kind.as_str())).collect::<Vec<_>>(), [("web", "web")]);
    let conn = rusqlite::Connection::open(path.join(annotary_store::DB_FILE)).unwrap();
    let web_sessions: i64 =
        conn.query_row("SELECT COUNT(*) FROM sessions WHERE device_id = 'web'", [], |r| r.get(0)).unwrap();
    assert_eq!(web_sessions, 1);
}

// spec: S29-02
#[test]
fn the_catalog_lists_every_setting_and_refuses_unknown_keys_and_values() {
    let (_d, s, m) = store();
    let cat = settings::catalog_json();
    let keys: Vec<&str> = cat["settings"].as_array().unwrap().iter().map(|v| v["key"].as_str().unwrap()).collect();
    for k in [
        "display_name",
        "appearance",
        "highlight_light",
        "highlight_dark",
        "text_size",
        "show_resolved",
        "default_vault",
        "keyboard_shortcuts",
        "send_key",
        "times",
        "clock",
    ] {
        assert!(keys.contains(&k), "{k} missing from {keys:?}");
    }
    for v in cat["settings"].as_array().unwrap() {
        for field in ["key", "kind", "label", "help", "group", "order", "type", "default", "device_kinds"] {
            assert!(!v[field].is_null(), "{} has no {field}", v["key"]);
        }
        if v["kind"] == "device" {
            assert!(!v["device_kinds"].as_array().unwrap().is_empty(), "{}", v["key"]);
        }
    }
    let slugs: Vec<&str> = cat["groups"].as_array().unwrap().iter().map(|g| g["slug"].as_str().unwrap()).collect();
    assert_eq!(slugs, ["profile", "devices", "appearance", "threads", "documents", "keyboard", "date-and-time"]);
    assert!(!slugs.contains(&"server") && !slugs.contains(&"vaults"));
    let err = s.put_setting(&m, &PutSetting { key: "nope".into(), value: json!(1), scope: Scope::User, stamp: None });
    assert!(matches!(err, Err(StoreError::Setting(Refusal::UnknownSetting(_)))), "{err:?}");
    let err = s.put_setting(
        &m,
        &PutSetting { key: "appearance".into(), value: json!("sepia"), scope: Scope::All, stamp: None },
    );
    assert!(matches!(err, Err(StoreError::Setting(Refusal::Invalid { .. }))), "{err:?}");
    let err =
        s.put_setting(&m, &PutSetting { key: "text_size".into(), value: json!(105), scope: Scope::All, stamp: None });
    assert!(matches!(err, Err(StoreError::Setting(Refusal::Invalid { .. }))), "{err:?}");
    let err = s
        .put_setting(&m, &PutSetting { key: "times".into(), value: json!("absolute"), scope: Scope::All, stamp: None });
    assert!(matches!(err, Err(StoreError::Setting(Refusal::Invalid { .. }))), "a user setting at scope all: {err:?}");
    assert!(put(&s, &m, "text_size", json!("130%"), Scope::All).changed);
    assert_eq!(resolved(&s, &m, "text_size", None), (json!(130), "all"));
}

// spec: S29-03
#[test]
fn every_highlight_preset_passes_contrast_and_stays_apart_from_refined() {
    for p in PRESETS {
        for (bg, text, refined, what) in [
            (p.light, settings::LIGHT_HL_TEXT, settings::REFINED_LIGHT, "light"),
            (p.light_selected, settings::LIGHT_HL_TEXT, settings::REFINED_LIGHT, "light selected"),
            (p.dark, settings::DARK_HL_TEXT, settings::REFINED_DARK, "dark"),
            (p.dark_selected, settings::DARK_HL_TEXT, settings::REFINED_DARK, "dark selected"),
        ] {
            let c = settings::contrast(bg, text);
            assert!(c >= settings::MIN_CONTRAST, "{} {what}: {c:.2}:1", p.key);
            if !what.ends_with("selected") {
                let de = settings::delta_e(bg, refined);
                assert!(de >= settings::MIN_REFINED_DELTA_E, "{} {what} is ΔE {de:.1} from refined", p.key);
            }
        }
    }
    assert_eq!(PRESETS[0].light, "#fff1a8", "yellow light stays today's highlight");
    let (_d, s, m) = store();
    // A custom color: accepted when the text on it passes, refused with the ratio when not.
    assert!(put(&s, &m, "highlight_light", json!("#FFE0E0"), Scope::All).changed);
    assert_eq!(resolved(&s, &m, "highlight_light", None).0, json!("#ffe0e0"));
    let err = s
        .put_setting(
            &m,
            &PutSetting { key: "highlight_dark".into(), value: json!("#202020"), scope: Scope::All, stamp: None },
        )
        .unwrap_err();
    match err {
        StoreError::Setting(Refusal::LowContrast { ratio, .. }) => assert!(ratio < 4.5 && ratio > 1.0, "{ratio}"),
        other => panic!("{other:?}"),
    }
    let err = s
        .put_setting(
            &m,
            &PutSetting { key: "highlight_light".into(), value: json!("#333333"), scope: Scope::All, stamp: None },
        )
        .unwrap_err();
    assert!(matches!(err, StoreError::Setting(Refusal::LowContrast { .. })), "{err:?}");
    assert_eq!(settings::highlight_colors("blue", true).0, "#60a5fa");
}

// spec: S29-04
#[test]
fn a_device_resolves_own_then_all_devices_then_default() {
    let (_d, s, m) = store();
    let phone = iphone(&s, &m);
    s.ensure_device(&m, "web", None).unwrap();
    assert_eq!(resolved(&s, &m, "appearance", Some(&phone)), (json!("system"), "default"));
    put(&s, &m, "appearance", json!("dark"), Scope::All);
    assert_eq!(resolved(&s, &m, "appearance", Some(&phone)), (json!("dark"), "all"));
    put(&s, &m, "appearance", json!("light"), devices(&["web"]));
    assert_eq!(resolved(&s, &m, "appearance", Some("web")), (json!("light"), "device"));
    assert_eq!(resolved(&s, &m, "appearance", Some(&phone)), (json!("dark"), "all"));
    // Set only on another Device: a new Device gets the default, and every All devices value.
    put(&s, &m, "text_size", json!(150), devices(&["web"]));
    let pad = settings::new_device_id();
    s.ensure_device(&m, &pad, Some(("ipad", "iPad Pro 13″"))).unwrap();
    assert_eq!(resolved(&s, &m, "text_size", Some(&pad)), (json!(100), "default"));
    assert_eq!(resolved(&s, &m, "appearance", Some(&pad)), (json!("dark"), "all"));
    // A tombstone follows All devices.
    put(&s, &m, "appearance", Value::Null, devices(&["web"]));
    assert_eq!(resolved(&s, &m, "appearance", Some("web")), (json!("dark"), "all"));
    // User settings: one value per Member.
    assert_eq!(resolved(&s, &m, "times", None), (json!("relative"), "default"));
    put(&s, &m, "times", json!("absolute"), Scope::User);
    assert_eq!(resolved(&s, &m, "times", Some(&phone)), (json!("absolute"), "user"));
    assert_eq!(resolved(&s, &m, "display_name", None), (json!("Rowan Vale"), "user"));
}

// spec: S29-05
#[test]
fn all_devices_tombstones_every_own_value_with_a_later_stamp_and_the_matrix_shows_mixed() {
    let (_d, s, m) = store();
    let phone = iphone(&s, &m);
    s.ensure_device(&m, "web", None).unwrap();
    put(&s, &m, "appearance", json!("system"), Scope::All);
    let web = put(&s, &m, "appearance", json!("light"), devices(&["web"])).stamp.unwrap();
    // A stamp from the phone far in the clock's past is still replaced.
    let (_, entries) = s.settings_matrix(&m).unwrap();
    let e = entries.iter().find(|e| e.def.key == "appearance").unwrap();
    assert!(e.mixed, "web is light, the phone follows system");
    assert_eq!(e.resolved["web"].0, json!("light"));
    assert_eq!(e.resolved[&phone], (json!("system"), "all"));
    // The keyboard settings do not apply to an iPhone, so a difference there is not Mixed.
    put(&s, &m, "send_key", json!("mod_enter"), devices(&[&phone]));
    let (_, entries) = s.settings_matrix(&m).unwrap();
    assert!(!entries.iter().find(|e| e.def.key == "send_key").unwrap().mixed);

    let all = put(&s, &m, "appearance", json!("dark"), Scope::All).stamp.unwrap();
    let (all_st, web_st) = (Stamp::parse(&all).unwrap(), Stamp::parse(&web).unwrap());
    assert!(all_st > web_st);
    let sync = s.device_sync(&m, "web").unwrap();
    let (_, own, all_row) = sync.iter().find(|(d, _, _)| d.key == "appearance").unwrap();
    let own = own.as_ref().unwrap();
    assert_eq!(own.value, None, "web's own value is now a tombstone");
    assert_eq!(own.stamp, all);
    assert_eq!(all_row.as_ref().unwrap().value, Some(json!("dark")));
    let (_, entries) = s.settings_matrix(&m).unwrap();
    assert!(!entries.iter().find(|e| e.def.key == "appearance").unwrap().mixed);
    assert_eq!(resolved(&s, &m, "appearance", Some("web")), (json!("dark"), "all"));
}

// spec: S29-06
#[test]
fn client_stamps_are_kept_checked_for_skew_and_staleness_and_the_clock_survives_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("store");
    let (m, phone, last) = {
        let s = Store::open(&path).unwrap();
        let m = s
            .setup(SetupInput { owner_handle: "rowan".into(), owner_name: "J".into(), password: None })
            .unwrap()
            .owner
            .id;
        let phone = iphone(&s, &m);
        let now = settings::wall_ms();
        let st = format!("{}-0-{phone}", now - 1000);
        assert!(put_stamped(&s, &m, "appearance", json!("dark"), devices(&[&phone]), &st).unwrap());
        assert_eq!(s.device_sync(&m, &phone).unwrap()[0].1.as_ref().unwrap().stamp, st, "kept as given");
        // Older or equal with another value: stale, with what is stored.
        match put_stamped(&s, &m, "appearance", json!("light"), devices(&[&phone]), &st) {
            Err(StoreError::Stale { stored_value, stored_stamp, .. }) => {
                assert_eq!(stored_value.as_deref(), Some(&json!("dark")));
                assert_eq!(stored_stamp, st);
            }
            other => panic!("{other:?}"),
        }
        let older = format!("{}-0-{phone}", now - 5000);
        assert!(matches!(
            put_stamped(&s, &m, "appearance", json!("light"), devices(&[&phone]), &older),
            Err(StoreError::Stale { .. })
        ));
        // The same value with an older stamp changes nothing.
        assert!(!put_stamped(&s, &m, "appearance", json!("dark"), devices(&[&phone]), &older).unwrap());
        // More than five minutes ahead: refused with the server's time.
        let ahead = format!("{}-0-{phone}", now + settings::MAX_AHEAD_MS + 60_000);
        match put_stamped(&s, &m, "appearance", json!("light"), devices(&[&phone]), &ahead) {
            Err(StoreError::ClockSkew { server_ms, .. }) => assert!(server_ms >= now),
            other => panic!("{other:?}"),
        }
        // Four minutes ahead is accepted, and the server's next stamp comes after it.
        let soon = format!("{}-0-{phone}", now + 4 * 60_000);
        assert!(put_stamped(&s, &m, "appearance", json!("light"), devices(&[&phone]), &soon).unwrap());
        let next = put(&s, &m, "appearance", json!("system"), Scope::All).stamp.unwrap();
        assert!(Stamp::parse(&next).unwrap() > Stamp::parse(&soon).unwrap());
        (m, phone, next)
    };
    let s = Store::open(&path).unwrap();
    let after = put(&s, &m, "text_size", json!(115), devices(&[&phone])).stamp.unwrap();
    assert!(Stamp::parse(&after).unwrap() > Stamp::parse(&last).unwrap(), "{after} after {last}");
    assert!(s.setting("hlc").unwrap().is_some());
}

// spec: S29-05, S29-06
#[test]
fn an_older_stamped_all_devices_push_never_moves_the_all_stamp_back() {
    let (_d, s, m) = store();
    let now = settings::wall_ms();
    let all = put(&s, &m, "appearance", json!("dark"), Scope::All).stamp.unwrap();
    // A Device the server meets afterwards pushes an own value stamped before that All write.
    let phone = iphone(&s, &m);
    let own = format!("{}-0-{phone}", now - 50_000);
    assert!(put_stamped(&s, &m, "appearance", json!("light"), devices(&[&phone]), &own).unwrap());
    // An All devices push with the same value, older than the All row but newer than the
    // override: the override is tombstoned, the All row and its stamp stay.
    let older = format!("{}-0-{phone}", now - 30_000);
    assert!(put_stamped(&s, &m, "appearance", json!("dark"), Scope::All, &older).unwrap());
    let sync = s.device_sync(&m, &phone).unwrap();
    let (_, own_row, all_row) = sync.iter().find(|(d, _, _)| d.key == "appearance").unwrap();
    assert_eq!(all_row.as_ref().unwrap().stamp, all, "the All stamp never moves backwards");
    assert_eq!(own_row.as_ref().unwrap().value, None);
    assert_eq!(own_row.as_ref().unwrap().stamp, older);
    assert_eq!(resolved(&s, &m, "appearance", Some(&phone)), (json!("dark"), "all"));
    // Again: nothing left to change.
    assert!(!put_stamped(&s, &m, "appearance", json!("dark"), Scope::All, &older).unwrap());
    // Another value, older than the All row: stale.
    assert!(matches!(
        put_stamped(&s, &m, "appearance", json!("light"), Scope::All, &older),
        Err(StoreError::Stale { .. })
    ));
}

// spec: S29-06
#[test]
fn a_user_setting_takes_no_client_stamp_so_a_reset_cannot_be_undone_by_an_older_push() {
    let (_d, s, m) = store();
    put(&s, &m, "times", json!("absolute"), Scope::User);
    assert!(s.reset_setting(&m, "times", &Scope::User).unwrap().changed);
    let old = format!("{}-0-{}", settings::wall_ms() - 60_000, settings::new_device_id());
    let err = put_stamped(&s, &m, "times", json!("absolute"), Scope::User, &old);
    assert!(matches!(err, Err(StoreError::Setting(Refusal::Invalid { .. }))), "{err:?}");
    assert_eq!(resolved(&s, &m, "times", None), (json!("relative"), "default"));
    // The display name too: refused, not silently written.
    let err = put_stamped(&s, &m, "display_name", json!("Rowan A."), Scope::User, &old);
    assert!(matches!(err, Err(StoreError::Setting(Refusal::Invalid { .. }))), "{err:?}");
    assert_eq!(resolved(&s, &m, "display_name", None), (json!("Rowan Vale"), "user"));
}

// spec: S29-07
#[test]
fn a_write_that_changes_nothing_appends_no_event_and_events_carry_no_values() {
    let (_d, s, m) = store();
    s.ensure_device(&m, "web", None).unwrap();
    let before = s.last_seq().unwrap();
    assert!(put(&s, &m, "show_resolved", json!("shown"), Scope::User).changed);
    assert!(!put(&s, &m, "show_resolved", json!("shown"), Scope::User).changed);
    assert!(put(&s, &m, "appearance", json!("dark"), Scope::All).changed);
    assert!(!put(&s, &m, "appearance", json!("dark"), Scope::All).changed);
    assert!(!put(&s, &m, "appearance", Value::Null, devices(&["web"])).changed, "no own value to tombstone");
    assert!(put(&s, &m, "appearance", json!("light"), devices(&["web"])).changed);
    assert!(!put(&s, &m, "appearance", json!("light"), devices(&["web"])).changed);
    assert!(!s.reset_setting(&m, "times", &Scope::User).unwrap().changed);
    let evs = events_of(&s, "setting.changed");
    assert_eq!(evs.len(), 3);
    assert_eq!(s.last_seq().unwrap(), before + 3);
    for e in &evs {
        let keys: Vec<&str> = e.as_object().unwrap().keys().map(String::as_str).collect();
        assert!(keys.iter().all(|k| ["member", "key", "scope", "device", "stamp"].contains(k)), "{e}");
        assert_eq!(e["member"], json!(m));
    }
    assert_eq!(evs[2]["device"], json!("web"));
    assert!(!serde_json::to_string(&evs).unwrap().contains("light"), "no value in the log");
}

// spec: S29-08
#[test]
fn devices_are_well_formed_bounded_and_seen_without_waking_anyone() {
    let (_d, s, m) = store();
    assert!(matches!(s.ensure_device(&m, "not-a-device", None), Err(StoreError::BadDevice(_))));
    assert!(matches!(
        s.ensure_device(&m, &settings::new_device_id().to_lowercase(), None),
        Err(StoreError::BadDevice(_))
    ));
    let id = settings::new_device_id();
    let d = s.ensure_device(&m, &id, Some(("ipad", "iPad Pro 13″"))).unwrap();
    assert_eq!((d.kind.as_str(), d.name.as_str()), ("ipad", "iPad Pro 13″"));
    assert_eq!(s.ensure_device(&m, &id, Some(("iphone", "other"))).unwrap().name, "iPad Pro 13″", "an upsert");
    assert_eq!(events_of(&s, "device.added").len(), 1);
    // Web is fixed: never renamed or forgotten.
    s.ensure_device(&m, "web", None).unwrap();
    assert!(matches!(s.rename_device(&m, "web", "Safari"), Err(StoreError::WebDevice)));
    assert!(matches!(s.forget_device(&m, "web"), Err(StoreError::WebDevice)));
    // The cap: 20 that aren't forgotten (web + the iPad + 18 more).
    for _ in 0..18 {
        s.ensure_device(&m, &settings::new_device_id(), None).unwrap();
    }
    assert!(matches!(s.ensure_device(&m, &settings::new_device_id(), None), Err(StoreError::TooManyDevices(20))));
    s.forget_device(&m, &id).unwrap();
    s.ensure_device(&m, &settings::new_device_id(), None).unwrap();
    // Seen: stamped, at most once a minute, outside the commit hook.
    let fired = Arc::new(AtomicUsize::new(0));
    let f = fired.clone();
    s.set_commit_hook(Some(Box::new(move || {
        f.fetch_add(1, Ordering::SeqCst);
    })));
    let other = s.list_devices(&m, false).unwrap().into_iter().find(|d| d.kind == "iphone").unwrap();
    let seen = other.last_seen_at.clone();
    assert_eq!(s.touch_device(&m, &other.id, None).unwrap(), DeviceTouch::Known);
    assert_eq!(s.device(&m, &other.id).unwrap().unwrap().last_seen_at, seen, "seen less than a minute ago");
    assert_eq!(s.touch_device(&m, &settings::new_device_id(), None).unwrap(), DeviceTouch::Unknown);
    assert_eq!(s.list_devices(&m, false).unwrap().len(), 20, "touching never makes a Device");
    assert_eq!(fired.load(Ordering::SeqCst), 0);
}

// spec: S29-09
#[test]
fn forget_revokes_what_was_last_used_from_the_device_and_refuses_its_id() {
    let (_d, s, m) = store();
    let (agent, _) = s.add_member(&m, "agent", None).unwrap();
    let phone = iphone(&s, &m);
    let tok = s.mint_token(&m, &m, Some("device")).unwrap();
    let other = s.mint_token(&m, &m, Some("cli")).unwrap();
    let a = s.authenticate(&tok.plain).unwrap().unwrap();
    let AuthVia::Token { id: tok_id } = &a.via else { panic!() };
    assert_eq!(s.touch_device(&m, &phone, Some(tok_id)).unwrap(), DeviceTouch::Known);
    put(&s, &m, "appearance", json!("dark"), devices(&[&phone]));
    // Another Member's Device of the same id is never touched: ids are per Member.
    assert!(matches!(s.forget_device(&agent.id, &phone), Err(StoreError::DeviceNotFound(_))));
    let (forgotten, tokens, _) = s.forget_device(&m, &phone).unwrap();
    assert!(forgotten);
    assert_eq!(tokens, 1);
    assert!(s.authenticate(&tok.plain).unwrap().is_none(), "the Token used from it is revoked");
    assert!(s.authenticate(&other.plain).unwrap().is_some(), "other Tokens keep working");
    assert!(!s.forget_device(&m, &phone).unwrap().0, "idempotent");
    assert!(matches!(s.ensure_device(&m, &phone, None), Err(StoreError::DeviceForgotten(_))));
    assert_eq!(s.touch_device(&m, &phone, None).unwrap(), DeviceTouch::Forgotten);
    assert!(s.list_devices(&m, false).unwrap().iter().all(|d| d.id != phone));
    assert!(s.list_devices(&m, true).unwrap().iter().any(|d| d.id == phone && d.forgotten_at.is_some()));
    // Its values stay: Copy from another device still reads them.
    s.ensure_device(&m, "web", None).unwrap();
    s.copy_device_settings(&m, &phone, Some("web")).unwrap();
    assert_eq!(resolved(&s, &m, "appearance", Some("web")), (json!("dark"), "device"));
    assert_eq!(events_of(&s, "device.forgotten").len(), 1);
}

// spec: S29-10
#[test]
fn copy_from_another_device_restamps_every_device_setting() {
    let (_d, s, m) = store();
    let phone = iphone(&s, &m);
    let pad = settings::new_device_id();
    s.ensure_device(&m, &pad, Some(("ipad", "iPad"))).unwrap();
    put(&s, &m, "highlight_light", json!("green"), Scope::All);
    put(&s, &m, "appearance", json!("dark"), devices(&[&phone]));
    put(&s, &m, "text_size", json!(130), devices(&[&phone]));
    let pad_light = put(&s, &m, "highlight_light", json!("pink"), devices(&[&pad])).stamp.unwrap();
    put(&s, &m, "send_key", json!("mod_enter"), devices(&[&pad]));
    let n = s.copy_device_settings(&m, &phone, Some(&pad)).unwrap();
    assert_eq!(n, settings::device_settings().filter(|d| d.applies_to("iphone")).count());
    // An iPhone has no keyboard settings: the iPad keeps its own.
    assert_eq!(resolved(&s, &m, "send_key", Some(&pad)), (json!("mod_enter"), "device"));
    assert_eq!(resolved(&s, &m, "appearance", Some(&pad)), (json!("dark"), "device"));
    assert_eq!(resolved(&s, &m, "text_size", Some(&pad)), (json!(130), "device"));
    // The phone follows All devices for the light highlight, so the iPad now does too.
    assert_eq!(resolved(&s, &m, "highlight_light", Some(&pad)), (json!("green"), "all"));
    let rows = s.device_sync(&m, &pad).unwrap();
    for (def, own, _) in rows.iter().filter(|(d, _, _)| d.applies_to("iphone")) {
        let own = own.as_ref().unwrap_or_else(|| panic!("{} has a fresh row", def.key));
        assert!(Stamp::parse(&own.stamp).unwrap() > Stamp::parse(&pad_light).unwrap(), "{} stamped now", def.key);
    }
    // Onto All devices: what the phone resolves to becomes everyone's.
    s.copy_device_settings(&m, &phone, None).unwrap();
    assert_eq!(resolved(&s, &m, "appearance", Some(&pad)), (json!("dark"), "all"));
    assert_eq!(resolved(&s, &m, "appearance", Some(&phone)), (json!("dark"), "all"));
    assert!(matches!(s.copy_device_settings(&m, &phone, Some(&phone)), Err(StoreError::Invalid { .. })));
}

// spec: S29-14
#[test]
fn default_vault_is_an_id_that_follows_a_rename_and_falls_back_on_delete() {
    let (_d, s, m) = store();
    let v = s.create_vault(&m, "drafts").unwrap();
    put(&s, &m, "default_vault", json!(v.id), Scope::User);
    s.rename_vault(&m, &v.id, "papers").unwrap();
    assert_eq!(resolved(&s, &m, "default_vault", None), (json!(v.id), "user"));
    s.delete_vault(&m, &v.id).unwrap();
    assert_eq!(resolved(&s, &m, "default_vault", None), (json!("any"), "default"));
    let err = s.put_setting(
        &m,
        &PutSetting { key: "default_vault".into(), value: json!("01NOPE"), scope: Scope::User, stamp: None },
    );
    assert!(matches!(err, Err(StoreError::VaultNotFound(_))), "{err:?}");
    assert!(put(&s, &m, "default_vault", json!("any"), Scope::User).changed);
    assert_eq!(CATALOG.iter().find(|d| d.key == "default_vault").unwrap().default, "\"any\"");
}
