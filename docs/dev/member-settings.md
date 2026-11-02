# Member settings and Devices: store and API (CMD-23 — extend, don't rewrite)

The web Settings screen is [web-settings.md](web-settings.md), the Document page's use of the
settings [web-document-view.md](web-document-view.md), the CLI verbs
[cli-conventions.md](cli-conventions.md), the deploy export `docs/ops-health-check.md` P7.

- **Files:** `crates/store/src/settings.rs` (pure: `CATALOG` / `GROUPS` / `PRESETS` /
  `TEXT_SIZES`, `validate` → `Refusal`, `catalog_json`, the colors `contrast` / `delta_e` /
  `highlight_colors`, `Stamp` + `MAX_AHEAD_MS`, Device ids `is_device_id` / `new_device_id` /
  `WEB_DEVICE`), `crates/store/src/devices.rs` (the `Store` methods below, the server clock),
  migration v5 in `migrations.rs`, `server/me_settings.rs` (the routes; its module docs have the
  table and the device headers), `server/prefs.rs` (`WebPrefs`: what a page applies),
  `auth.rs::check_device`, the `me=1` rule in `server/events.rs` + `EventQuery.member_events`.
- **Schema v5:** `devices (member_id, id)` (`name`, `kind` iphone|ipad|web|cli, `created_at`,
  `last_seen_at`, `forgotten_at`); `tokens.device_id` / `sessions.device_id` (the Device last
  used from); `member_settings`, `all_device_settings` (`value` JSON, `stamp`), `device_settings`
  (`value` NULL = tombstone). The migration gives every Member with a session the Web Device.
  The previous binary can't open a v5 store (`SchemaTooNew`): the deploy exports first (P7).
- **Devices:** `ensure_device(member, id, Some((kind, name)))` is the only maker (an upsert in one
  `tx`; `device.added`; cap `MAX_DEVICES` = 20 not forgotten, Web included → `TooManyDevices`;
  a forgotten id → `DeviceForgotten`; a malformed one → `BadDevice`); only the settings routes
  and `WebPrefs::load` / `open_session` (Web) call it. `touch_device(member, id, token_id)`
  (every Token request with `X-Annotary-Device`, from `auth::check_device`) and
  `authenticate_session` (Web) stamp `last_seen_at` at most once a minute (`SEEN_EVERY_SECS`)
  and the Token's `device_id`, in a hand-made transaction that never fires the commit hook.
  `rename_device` / `forget_device` refuse Web (`WebDevice` → 409 `web_device`); forget revokes
  the Tokens with that `device_id` and deletes its sessions, sets `forgotten_at`, keeps the row
  and values (`device.forgotten {member, device, tokens, sessions}`).
- **Reading:** `resolved_settings(member, device)` / `resolved_setting` → `Resolved {def, value,
  source}` (`device` > `all` > `default` for device settings; `user` / `default` for user ones;
  `display_name` comes from `members`; a Default Vault id that no longer exists reads as the
  default). `settings_matrix(member)` → the live Devices + one `MatrixEntry` per device setting
  (`all`, each Device's `own` row, `resolved`, `mixed` = more than one value among Devices the
  setting `applies_to`). `device_sync(member, device)` → `SyncRow`s for iOS.
- **Writing:** `put_setting(member, PutSetting {key, value, scope, stamp})` → `PutOutcome
  {changed, stamp}`; `reset_setting` (user: row deleted; all: the default written as All;
  device: a tombstone); `copy_device_settings(member, from, to | None)` (forced, re-stamped
  writes of the settings `from`'s kind has; an iPhone's copy leaves the keyboard settings alone). All of them go through `write_one` inside `Store::tx`; a change appends
  `setting.changed {member, key, scope, device?, stamp}` (one per Device at scope `devices`),
  never the value; no change, no event. Scope `all` tombstones every live Device's own value
  older than the new stamp. `display_name` writes go through `rename_member`.
- **Stamps:** `MS-COUNTER-WRITER` (`Stamp`, ordered by all three). The server's writer is
  `s<tenant id>`; its clock is the `settings` row `hlc` (`hlc_next` / `hlc_observe`): every
  accepted client stamp moves it, so the next server stamp is later than anything stored. A
  client stamp more than five minutes ahead of the wall clock → `ClockSkew` (409 `clock_skew`,
  `server_time_ms`); not newer than the stored one → `Stale` (409 `stale`, `stored {value,
  stamp}`) unless the value is the same (a no-op). A stamped `all` push older than the All row never
  rewrites it (it only tombstones own values older than the push). User settings take no client
  stamp (400 `invalid_setting`): a reset deletes the row, so there is nothing an older push
  could be checked against, and only device settings sync across servers.
- **Errors** (`server/error.rs`): 400 `unknown_setting` / `invalid_setting` / `low_contrast`
  (+`ratio`) / `bad_device` → exit 1; 404 no Device → 2; 409 `device_forgotten` /
  `too_many_devices` / `web_device` / `clock_skew` / `stale` → 5.
- **Live feed:** `setting.changed` and `device.*` (`events::is_member_event`) are readable only
  by `payload.member`, owners included, and only on a feed with `me=1`; `me=1` adds them beside
  `doc=` / `kinds=` (SQL: `EventQuery.member_events`).
- **Decisions to keep:** a Device id is a label keyed per Member, not a credential (any of the
  Member's credentials may name any of their Devices); the browser never stamps; Web is one
  Device for every browser and has no cookie of its own; the events log never holds a value;
  the catalog is the one list (a new setting = an entry in `CATALOG` + client behavior); light
  Purple and Blue are the presets nearest refined (ΔE 9.6 / 10.6), so `MIN_REFINED_DELTA_E` is 9
  and refined keeps its dotted underline.
- **Tests:** `tests/settings_store.rs`, `tests/settings_api.rs`, the CMD-23 rows in
  `tests/access_api.rs`; spec ids S29-01…S29-15.
