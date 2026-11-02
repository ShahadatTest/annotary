# Web UI: the Settings screen (CMD-23 — extend, don't rewrite)

Implementation notes for the Settings screen every Member has (its design decisions are in
`DESIGN.md`, "Member settings"). The store side
(catalog, Devices, resolution, stamps) and the JSON API are `crates/store/src/settings.rs`,
`devices.rs` and `server/me_settings.rs`; this file is the page.

- **Files:** `server/settings_page.rs` (the routes, the frame `render(st, auth, headers, Frame)`
  every Settings page uses — the owner pages too — the sidebar, the group bodies, one renderer per
  control type in `control()`), `server/ui/settings.css` + `settings.js` (inlined after `app.css` /
  `app.js` only on `/settings` and `/settings/…`; the Server pages add `admin.css` / `admin.js`
  after them), `server/admin.rs` (the Server pages' content), `ui::top_nav` (Documents and Settings
  for everyone). Every page opens `.cd-app` with `WebPrefs::load(…).app_attrs("cd-settingspage")`
  (`prefs.rs`), so it wears the Member's theme like every other page.
- **Routes:**

  | route | who | what |
  |---|---|---|
  | `GET /settings` | every Member | `.cd-set-layout.cd-set-index`: Profile beside the sidebar at 850px and wider; under 850px the sidebar alone is the group list (container query on `.cd-app`, like `app.css`) |
  | `GET /settings/{group}` | every Member | one catalog group (`settings::GROUPS`: `profile`, `devices`, `appearance`, `threads`, `documents`, `keyboard`, `date-and-time`); `.cd-set-grouppage` — narrow, the sidebar is hidden and "‹ Settings" shows; an unknown slug is the 404 page |
  | `GET /settings/server/{members,vaults,general}` | owner | the owner pages in the same frame (`admin.rs`); 403 page for a Member, 404 page for another slug |
  | `GET /members` | — | 308 to `/settings/server/members` |

  The card fragments `GET /members/{handle}` / `GET /settings/vaults/{name}` are unchanged and
  never redirected. Group slugs are never `server` or `vaults` (a store test keeps it so).
- **The first paint is final.** `load_ctx` reads `settings_matrix` (every non-forgotten Device's
  own and resolved value per device setting), the Web Device's `resolved_settings` (user values)
  and the readable Vaults once; each row is rendered in its state for All devices (`shown()`: the
  value every Device the setting applies to resolves to, or Mixed). The script never changes what
  the first paint shows; it flips attributes after a change. `#cd-set-data` (JSON, `<` escaped)
  carries `group`, `current` (always `web`: a browser is the Web Device), `devices` (with `seen`),
  `matrix`, the group's `settings` (catalog entries), `user`, `presets`, `seq` (the live feed's
  start) and `search` (every page: groups, every setting's label + help, Profile / Devices words,
  the Server pages for owners).
- **Markup the script and the tests key on:** `#cd-set-side` (`#cd-set-q`, `#cd-set-groups`,
  `.cd-set-gl[data-group][aria-current]`, `#cd-set-serverhead` + `.cd-set-server` for owners only,
  `#cd-set-nomatch`), `#cd-set-main` (`.cd-set-back`, `.cd-set-title`), `#cd-set-scope`
  (`#cd-set-scopebtn` / `#cd-set-scopename`, `#cd-set-copybtn`), `.cd-set-row#cd-set-KEY
  [data-key][data-kind][data-type][data-search]` (`.cd-set-layers[data-act=chain]` — a button named
  for the setting, `.cd-mixed` when amber; `.cd-seg[role=radiogroup]` of `.cd-seg-o[role=radio]
  [data-value]`; `.cd-swatches` of `.cd-sw` + `.cd-sw-custom`, `[data-act=custom]`,
  `.cd-set-customrow` (`.cd-set-colorpick`, `.cd-set-hex`), `.cd-set-sample-light|dark|size`,
  `.cd-set-mark`; `.cd-tsize` (`[data-act=smaller|larger]`, `.cd-tsize-v`); `.cd-set-select`;
  `.cd-set-mixed`; `.cd-set-reset` (user settings, hidden at the default); `.cd-set-state`),
  `.cd-set-foryou` ("For you on every device": the user settings of a group that has device ones),
  `.cd-set-yours` (groups of user settings only), `#cd-set-name-in` / `.cd-set-handle` (Profile),
  `#cd-devlist` `.cd-dev[data-device][data-kind]` (`.cd-dev-name`, `.cd-dev-this`, `.cd-dev-seen`,
  `[data-act=rename|forget]` on iOS Devices only, `.cd-dev-inline`, `.cd-dev-err`). Popovers the
  script builds: `#cd-set-scopemenu` (`role=menu`, `[role=menuitemcheckbox][data-id]`, `all` first),
  `#cd-set-copymenu` (`[role=menuitem][data-id]`), `#cd-set-confirm` (`[data-act=cancel|copy]`),
  `#cd-set-chain` (`.cd-set-cl[data-device]`, `.cd-set-clname` / `.cd-set-clnote` / `.cd-set-clv`,
  `[data-act=reset-device]`). `.cd-app.cd-set-ready` once wired; `.cd-app.cd-live` while the feed is
  connected.
- **Saving:** a click writes at once (`PUT /api/v1/me/settings`): scope `all` with All devices
  chosen, `devices` + the ticked ids otherwise, `user` for a user setting. The control shows the new
  value before the answer and "Saving…"; the answer's `matrix` entry replaces the local one; a
  failure puts the old value back with "Couldn't save. Try again." (a `low_contrast` refusal says
  the ratio instead). Reset in the chain is `DELETE …/{key}?scope=device&device=ID`; a user
  setting's Reset `?scope=user`. Profile's name: `POST /api/v1/me/rename` on blur or Enter (the top
  bar's name follows). Devices: `PATCH /api/v1/me/devices/{id}`, `POST …/forget` (Forget revokes the
  Tokens last used from that Device — a test that drives iOS with the owner's Token loses it).
- **Mixed** (`shown()` in both files): the chosen Devices (All devices = every non-forgotten one) of
  a kind the setting applies to resolve to more than one value ⇒ no segment selected, the amber
  pill, the amber layers icon, and the radio group named "LABEL: Mixed". The chain's All devices
  line is blank when the Devices disagree. A choice with All devices writes scope `all`, which
  tombstones every Device's own value, so Mixed goes away.
- **Copy from another device:** the menu lists every Device not chosen; picking one replaces the
  menu in place with the confirm ("Copy **X**'s settings to **Y**?", Y = the chosen names or All
  devices); Copy posts `POST /api/v1/me/devices/{id}/copy-from {from}` per chosen Device (`all` for
  All devices) and takes the last answer's `settings` as the matrix. Cancel, Escape and a click
  outside close it with nothing copied. Popovers fade in over 120 ms (`prefers-reduced-motion`:
  none).
- **Live:** `EventSource('/api/v1/events?me=1&since=SEQ')`; any `setting.changed` / `device.*`
  re-reads the matrix, the user values and the Devices; a control whose shown value changed (and
  that has no write in flight) says "Changed on NAME" — NAME from the stamp's writer (an iOS push
  stamps with its Device id) or the event's Device, else "another device".
- **Search** filters with no request: a sidebar group shows when its label, one of its settings'
  label or help line, or its extra words match; the rows of the current page that don't match are
  hidden (all of them show when the group itself matches). Help lines are only searched, never
  shown under a name.
- **Decisions to keep:** card markup has one renderer (Rust) — the script builds only popovers and
  inline editors; no `position: fixed` (popovers are absolute inside the scope line or the row);
  `admin.css` / `admin.js` only on the Server pages; the highlight samples sit on fixed grounds
  (`#fff` / `#000`, black highlighted text on dark) whatever the page's appearance; colors are the
  shared `--cd-*` tokens with their light values as fallbacks; last seen reads "now", "N minutes
  ago", "N hours ago", "yesterday", "N days ago", then the date (`seen_words` / `seenWords`, keep
  them equal); the current Device is "now" and first in every list.
- **Tests:** `crates/annotary/tests/web_settings.rs` (route table, sidebar, first paint, Profile,
  Devices), `web_owner.rs` (`S12-05`, changed on purpose by CMD-23), `access_api.rs` rows for
  `/settings`, `/settings/{group}`, `/settings/server/*`; `tests/web/settings.spec.ts` (the plan's
  Web / iPhone / iPad example on private servers — iOS Devices are the owner's Token with
  `X-Annotary-Device` / `X-Annotary-Device-Info` — copy, keyboard, search, wide / narrow, the
  Server group, Profile and Devices).
