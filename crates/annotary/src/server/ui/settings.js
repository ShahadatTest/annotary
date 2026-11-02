/* annotary Settings screen (CMD-23): the script behind /settings and /settings/…, inlined only
   there (after app.js; admin.js follows it on the Server pages). The server renders every control
   in its current state (the first paint is final); this script only wires what the page does:
   search (over the catalog's labels and help lines carried in #cd-set-data, no request), the
   "Applying to" menu (menuitemcheckbox items, keyboard), saving at once (PUT /api/v1/me/settings
   with scope all / devices / user; "Saving…", "Couldn't save. Try again." with the old value
   back), Mixed, the chain popover behind the layers icon (Reset = DELETE …?scope=device), Copy
   from another device (menu → an anchored confirm → POST …/devices/{id}/copy-from), the Profile
   name (POST /api/v1/me/rename), the Devices page's Rename / Forget, and the live feed
   (GET /api/v1/events?me=1: a change made elsewhere re-reads the matrix and says where it was
   made). It never builds a control's markup; it flips the state of the server's. */
(function () {
  'use strict';
  var app = document.querySelector('.cd-app.cd-settingspage');
  if (!app) return;
  var D = {};
  try { D = JSON.parse(document.getElementById('cd-set-data').textContent) || {}; } catch (e) { D = {}; }
  function $(sel, root) { return (root || app).querySelector(sel); }
  function $$(sel, root) { return Array.prototype.slice.call((root || app).querySelectorAll(sel)); }
  function esc(s) { return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; }); }
  function same(a, b) { return JSON.stringify(a) === JSON.stringify(b); }
  function copy(v) { return v === undefined ? undefined : JSON.parse(JSON.stringify(v)); }
  function svg(d, size, sw) {
    return '<svg width="' + size + '" height="' + size + '" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="' + sw +
      '" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true" focusable="false">' + d + '</svg>';
  }
  var ICON = {
    web: '<rect x="3" y="4" width="18" height="12" rx="2"/><path d="M8 20h8M12 16v4"/>',
    iphone: '<rect x="7" y="2.5" width="10" height="19" rx="2.5"/><path d="M11 18.5h2"/>',
    ipad: '<rect x="4" y="2.5" width="16" height="19" rx="2.5"/><path d="M11 18.5h2"/>',
    cli: '<rect x="3" y="4" width="18" height="16" rx="2"/><path d="M7 9l3 3-3 3M12 15h5"/>',
    check: '<path d="M5 12l5 5 9-10"/>'
  };
  function kindIcon(kind, size) { return svg(ICON[kind] || ICON.web, size || 18, '1.7'); }

  /* ---------- HTTP ---------- */
  function api(method, path, body) {
    var o = { method: method, credentials: 'same-origin', headers: {} };
    if (body !== undefined) { o.headers['Content-Type'] = 'application/json'; o.body = JSON.stringify(body); }
    return fetch(path, o).then(function (r) {
      return r.text().then(function (txt) {
        var j = null;
        try { j = txt ? JSON.parse(txt) : null; } catch (e) { j = null; }
        if (r.ok) return { ok: true, status: r.status, json: j };
        return { ok: false, status: r.status, json: j, code: j && j.error, error: (j && j.message) || ('HTTP ' + r.status) };
      });
    }, function () { return { ok: false, status: 0, json: null, code: 'unreachable', error: 'could not reach the server' }; });
  }

  /* ---------- search: the sidebar's groups, and the rows on this page ---------- */
  var q = document.getElementById('cd-set-q');
  function norm(s) { return String(s || '').toLowerCase(); }
  function runSearch() {
    var t = norm(q.value.trim());
    var hit = {};
    var labelHit = false;
    (D.search || []).forEach(function (it) {
      if (!t || norm(it.text).indexOf(t) >= 0) {
        hit[it.group] = true;
        if (it.group === D.group && !it.key) labelHit = true;
      }
    });
    var any = false;
    $$('.cd-set-gl').forEach(function (a) {
      var on = !t || !!hit[a.getAttribute('data-group')];
      a.parentNode.hidden = !on;
      if (on) any = true;
    });
    var sh = document.getElementById('cd-set-serverhead');
    if (sh) sh.hidden = !$$('.cd-set-server li').some(function (li) { return !li.hidden; });
    document.getElementById('cd-set-nomatch').hidden = any;
    // The rows of this page: all of them when the group itself matches, else the matching ones.
    $$('.cd-set-row[data-search]').forEach(function (row) {
      row.hidden = !!t && !labelHit && norm(row.getAttribute('data-search')).indexOf(t) < 0;
    });
    $$('.cd-set-foryou').forEach(function (s) { s.hidden = !$$('.cd-set-row', s).some(function (r) { return !r.hidden; }); });
  }
  if (q) {
    q.addEventListener('input', runSearch);
    q.addEventListener('keydown', function (e) { if (e.key === 'Escape' && q.value) { q.value = ''; runSearch(); } });
  }

  /* ---------- the state the controls show ---------- */
  var devices = (D.devices || []).slice();
  var matrix = D.matrix || {};
  var user = D.user || {};
  var defs = {};
  (D.settings || []).forEach(function (d) { defs[d.key] = d; });
  var presets = D.presets || [];
  var scope = null; // null = All devices; else the chosen Device ids
  var inflight = {};

  /* The current Device first, then the others in the server's order. */
  function ordered(list) {
    return list.filter(function (d) { return d.id === D.current; }).concat(list.filter(function (d) { return d.id !== D.current; }));
  }
  function deviceById(id) { for (var i = 0; i < devices.length; i++) if (devices[i].id === id) return devices[i]; return null; }
  /* "now", "5 minutes ago", "2 hours ago", "yesterday", "3 days ago", the date — the same rule as
     `settings_page::seen_words` (keep them equal). */
  function seenWords(d) {
    if (d.id === D.current) return 'now';
    if (!d.last_seen_at) return 'never';
    var t = Date.parse(d.last_seen_at);
    if (isNaN(t)) return 'never';
    var mins = Math.floor(Math.max(0, Date.now() - t) / 60000);
    var plural = function (n, w) { return n + ' ' + w + (n === 1 ? '' : 's') + ' ago'; };
    if (mins < 1) return 'now';
    if (mins < 60) return plural(mins, 'minute');
    if (mins < 1440) return plural(Math.floor(mins / 60), 'hour');
    if (mins < 2880) return 'yesterday';
    if (mins < 43200) return plural(Math.floor(mins / 1440), 'day');
    return d.last_seen_at.slice(0, 10);
  }
  function applies(def, d) { return (def.device_kinds || []).indexOf(d.kind) >= 0; }
  function allValue(key) {
    var e = matrix[key] || {}, def = defs[key];
    return e.all && e.all.value !== null && e.all.value !== undefined ? e.all.value : def['default'];
  }
  /* What a device setting's control shows for the chosen Devices (or All devices): the value they
     all resolve to, or Mixed when they disagree. */
  function shown(key) {
    var def = defs[key], e = matrix[key] || {};
    if (def.kind !== 'device') return { value: key in user ? user[key] : def['default'], mixed: false };
    var ds = devices.filter(function (d) { return applies(def, d) && (!scope || scope.indexOf(d.id) >= 0); });
    if (!ds.length && scope) ds = devices.filter(function (d) { return scope.indexOf(d.id) >= 0; });
    var vals = [];
    ds.forEach(function (d) {
      var r = (e.resolved || {})[d.id];
      var v = r ? r.value : allValue(key);
      if (!vals.some(function (x) { return same(x, v); })) vals.push(v);
    });
    if (vals.length === 1) return { value: vals[0], mixed: false };
    return { value: allValue(key), mixed: vals.length > 1 };
  }
  function presetOf(v) { for (var i = 0; i < presets.length; i++) if (presets[i].key === v) return presets[i]; return null; }
  function isHex(v) { return typeof v === 'string' && /^#[0-9a-fA-F]{6}$/.test(v); }
  function colorOf(v, dark) {
    var p = presetOf(v);
    if (p) return dark ? p.dark : p.light;
    if (isHex(v)) return v.toLowerCase();
    return colorOf('yellow', dark);
  }
  function labelOf(def, v) {
    if (v === null || v === undefined) return '';
    if (def.options) for (var i = 0; i < def.options.length; i++) if (def.options[i].value === v) return def.options[i].label;
    if (def.type === 'steps') return v + '%';
    if (def.type === 'color') { var p = presetOf(v); return p ? p.name : String(v); }
    return String(v);
  }
  function rowOf(key) { return document.getElementById('cd-set-' + key); }

  /* Flip one row's control to the state `shown` gives (no markup is built). */
  function paint(key) {
    var row = rowOf(key), def = defs[key];
    if (!row || !def) return;
    var s = shown(key);
    var name = def.label + (s.mixed ? ': Mixed' : '');
    var layers = $('.cd-set-layers', row);
    if (layers) layers.classList.toggle('cd-mixed', s.mixed);
    var pill = $('.cd-set-mixed', row);
    if (pill) pill.hidden = !s.mixed;
    if (def.type === 'choice') {
      var group = $('[role=radiogroup]', row);
      group.setAttribute('aria-label', name);
      var opts = $$('.cd-seg-o', row), anyOn = false;
      opts.forEach(function (b) {
        var on = !s.mixed && same(b.getAttribute('data-value'), s.value);
        b.classList.toggle('cd-on', on);
        b.setAttribute('aria-checked', on ? 'true' : 'false');
        b.tabIndex = on ? 0 : -1;
        anyOn = anyOn || on;
      });
      if (!anyOn && opts[0]) opts[0].tabIndex = 0;
    } else if (def.type === 'steps') {
      $('.cd-tsize', row).setAttribute('aria-label', name);
      var v = $('.cd-tsize-v', row);
      v.textContent = s.value + '%';
      v.hidden = s.mixed;
      $('.cd-set-sample-size', row).style.fontSize = Math.round(15 * s.value / 100) + 'px';
    } else if (def.type === 'color') {
      var dark = def.appearance === 'dark';
      $('.cd-swatches', row).setAttribute('aria-label', name);
      var custom = isHex(s.value);
      var cs = $('.cd-sw-custom', row);
      cs.hidden = !custom;
      if (custom) {
        cs.setAttribute('data-value', s.value.toLowerCase());
        cs.setAttribute('aria-label', 'Custom ' + s.value.toLowerCase());
        cs.style.setProperty('--cd-sw', s.value.toLowerCase());
      }
      var sws = $$('.cd-sw', row), on1 = false;
      sws.forEach(function (b) {
        var on = !s.mixed && !b.hidden && b.getAttribute('data-value') === String(s.value).toLowerCase();
        b.classList.toggle('cd-on', on);
        b.setAttribute('aria-checked', on ? 'true' : 'false');
        b.tabIndex = on ? 0 : -1;
        on1 = on1 || on;
      });
      if (!on1 && sws[0]) sws[0].tabIndex = 0;
      var c = colorOf(s.value, dark);
      $('.cd-set-mark', row).style.background = c;
      var pick = $('.cd-set-colorpick', row), hex = $('.cd-set-hex', row);
      if (document.activeElement !== hex) hex.value = c;
      pick.value = c;
    } else if (def.type === 'vault') {
      var sel = $('.cd-set-select', row);
      if (sel) sel.value = s.value;
    } else if (def.type === 'name') {
      var inp = $('.cd-set-text', row);
      if (inp && document.activeElement !== inp) inp.value = s.value;
    }
    var reset = $('.cd-set-reset', row);
    if (reset) reset.hidden = same(s.value, def['default']);
  }
  function paintAll() { Object.keys(defs).forEach(paint); }

  function setState(row, text, err) {
    var st = row && $('.cd-set-state', row);
    if (!st) return;
    st.textContent = text || '';
    st.classList.toggle('cd-set-err', !!err);
    if (err) st.setAttribute('role', 'alert'); else st.removeAttribute('role');
  }
  function failText(r) {
    if (r.code === 'low_contrast' && r.json && r.json.ratio) {
      return 'Text on that color has a contrast of ' + r.json.ratio + ':1; it needs 4.5:1.';
    }
    return 'Couldn’t save. Try again.';
  }

  /* Write a value where "Applying to" says, at once; the control shows it before the server
     answers and takes the old value back if the write fails. */
  function save(key, value) {
    var def = defs[key], row = rowOf(key);
    var body = { key: key, value: value };
    var before = def.kind === 'device' ? copy(matrix[key]) : copy(user[key]);
    if (def.kind === 'device') {
      var e = matrix[key] = copy(matrix[key]) || { devices: {}, resolved: {} };
      e.devices = e.devices || {}; e.resolved = e.resolved || {};
      if (scope) {
        body.scope = 'devices'; body.devices = scope.slice();
        scope.forEach(function (id) { e.devices[id] = { value: value }; e.resolved[id] = { value: value, source: 'device' }; });
      } else {
        body.scope = 'all';
        e.all = { value: value };
        devices.forEach(function (d) { if (e.devices[d.id]) e.devices[d.id] = { value: null }; e.resolved[d.id] = { value: value, source: 'all' }; });
        e.mixed = false;
      }
    } else {
      body.scope = 'user';
      user[key] = value;
    }
    paint(key);
    setState(row, 'Saving…');
    inflight[key] = (inflight[key] || 0) + 1;
    return api('PUT', '/api/v1/me/settings', body).then(function (r) {
      inflight[key]--;
      if (r.ok && r.json) {
        if (r.json.matrix) matrix[key] = r.json.matrix;
        if (def.kind === 'user') user[key] = r.json.value;
        paint(key);
        setState(row, '');
      } else {
        if (def.kind === 'device') matrix[key] = before; else user[key] = before;
        paint(key);
        setState(row, failText(r), true);
      }
      return r;
    });
  }

  function step(key, dir) {
    var def = defs[key], steps = def.steps || [];
    var cur = shown(key).value, i = steps.indexOf(cur);
    if (i < 0) i = steps.indexOf(100);
    var j = Math.max(0, Math.min(steps.length - 1, i + dir));
    if (j === i && !shown(key).mixed) return;
    save(key, steps[j]);
  }

  /* ---------- popovers (menus, the copy confirm, the chain) ---------- */
  var pop = null; // { el, anchor, onClose }
  function closePop(focusBack) {
    if (!pop) return;
    var p = pop;
    pop = null;
    if (p.el.parentNode) p.el.parentNode.removeChild(p.el);
    p.anchor.setAttribute('aria-expanded', 'false');
    if (focusBack) p.anchor.focus();
  }
  function openPop(el, host, anchor) {
    closePop(false);
    host.appendChild(el);
    anchor.setAttribute('aria-expanded', 'true');
    pop = { el: el, anchor: anchor };
    return el;
  }
  document.addEventListener('mousedown', function (e) {
    if (pop && !pop.el.contains(e.target) && !pop.anchor.contains(e.target)) closePop(false);
  });
  /* Arrow keys, Home / End, Escape (focus back to the button), Tab closes. */
  function menuKeys(menu) {
    menu.addEventListener('keydown', function (e) {
      var items = $$('[role^=menuitem]:not([aria-disabled=true])', menu);
      var i = items.indexOf(document.activeElement);
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        var n = items.length;
        if (!n) return;
        if (i < 0) items[e.key === 'ArrowDown' ? 0 : n - 1].focus();
        else items[(i + (e.key === 'ArrowDown' ? 1 : n - 1)) % n].focus();
      } else if (e.key === 'Home' || e.key === 'End') {
        e.preventDefault();
        if (items.length) items[e.key === 'Home' ? 0 : items.length - 1].focus();
      } else if (e.key === 'Escape') {
        e.preventDefault();
        closePop(true);
      } else if (e.key === 'Tab') {
        closePop(false);
      }
    });
  }

  /* ---------- "Applying to" ---------- */
  var scopeLine = document.getElementById('cd-set-scope');
  var scopeBtn = document.getElementById('cd-set-scopebtn');
  var copyBtn = document.getElementById('cd-set-copybtn');
  /* Menus open 6px under the "Applying to" button (round 3: 44px from the line's top); on a narrow
     page the copy menu sits under its own line. */
  function popTop(btn) {
    var top = scopeBtn.offsetTop + scopeBtn.offsetHeight + 6;
    if (btn && btn.offsetTop > scopeBtn.offsetTop + scopeBtn.offsetHeight) top = btn.offsetTop + btn.offsetHeight + 6;
    return top + 'px';
  }
  function names(ids) {
    var ns = ordered(devices).filter(function (d) { return ids.indexOf(d.id) >= 0; }).map(function (d) { return d.name; });
    if (ns.length <= 2) return ns.join(' and ');
    return ns.slice(0, -1).join(', ') + ' and ' + ns[ns.length - 1];
  }
  function scopeName() {
    if (!scope) return 'All devices';
    if (scope.length > 2) return scope.length + ' devices';
    return ordered(devices).filter(function (d) { return scope.indexOf(d.id) >= 0; }).map(function (d) { return d.name; }).join(', ');
  }
  function menuItem(role, id, checked, icon, name, sub) {
    var b = document.createElement('button');
    b.type = 'button';
    b.className = 'cd-set-mi';
    b.setAttribute('role', role);
    b.setAttribute('data-id', id);
    b.tabIndex = -1;
    if (role === 'menuitemcheckbox') b.setAttribute('aria-checked', checked ? 'true' : 'false');
    b.innerHTML = (role === 'menuitemcheckbox' ? '<span class="cd-set-check">' + svg(ICON.check, 16, '2.4') + '</span>' : '') +
      (icon ? '<span class="cd-set-micon">' + kindIcon(icon, 18) + '</span>' : '') +
      '<span class="cd-set-mtext"><span class="cd-set-mname">' + esc(name) + '</span>' + (sub ? '<span class="cd-set-msub">' + esc(sub) + '</span>' : '') + '</span>';
    return b;
  }
  function refreshScopeMenu(menu) {
    $$('[role=menuitemcheckbox]', menu).forEach(function (b) {
      var id = b.getAttribute('data-id');
      b.setAttribute('aria-checked', (id === 'all' ? !scope : !!scope && scope.indexOf(id) >= 0) ? 'true' : 'false');
    });
  }
  function toggleScope(id) {
    if (id === 'all') {
      scope = null;
    } else {
      var s = scope ? scope.slice() : [];
      var i = s.indexOf(id);
      if (i >= 0) s.splice(i, 1); else s.push(id);
      scope = s.length ? s : null;
    }
    document.getElementById('cd-set-scopename').textContent = scopeName();
    $$('.cd-set-row .cd-set-state').forEach(function (st) { if (!st.classList.contains('cd-set-err')) st.textContent = ''; });
    paintAll();
  }
  function openScope() {
    var menu = document.createElement('div');
    menu.className = 'cd-set-pop cd-set-scopemenu';
    menu.id = 'cd-set-scopemenu';
    menu.setAttribute('role', 'menu');
    menu.setAttribute('aria-label', 'Applying to');
    menu.appendChild(menuItem('menuitemcheckbox', 'all', !scope, null, 'All devices', null));
    var sep = document.createElement('div');
    sep.className = 'cd-set-sep';
    sep.setAttribute('role', 'separator');
    menu.appendChild(sep);
    ordered(devices).forEach(function (d) {
      menu.appendChild(menuItem('menuitemcheckbox', d.id, !!scope && scope.indexOf(d.id) >= 0, d.kind,
        d.name + (d.id === D.current ? '  ·  this device' : ''), 'Last seen ' + seenWords(d)));
    });
    menu.style.top = popTop();
    menu.style.left = scopeBtn.offsetLeft + 'px';
    menu.addEventListener('click', function (e) {
      var b = e.target.closest('[role=menuitemcheckbox]');
      if (!b) return;
      toggleScope(b.getAttribute('data-id'));
      refreshScopeMenu(menu);
      b.focus();
    });
    menu.addEventListener('keydown', function (e) {
      if (e.key === ' ' || e.key === 'Enter') {
        var b = e.target.closest('[role=menuitemcheckbox]');
        if (!b) return;
        e.preventDefault();
        toggleScope(b.getAttribute('data-id'));
        refreshScopeMenu(menu);
      }
    });
    menuKeys(menu);
    openPop(menu, scopeLine, scopeBtn);
    ($('[aria-checked=true]', menu) || $('[role=menuitemcheckbox]', menu)).focus();
  }
  if (scopeBtn) {
    scopeBtn.addEventListener('click', function () { if (pop && pop.anchor === scopeBtn) closePop(true); else openScope(); });
    scopeBtn.addEventListener('keydown', function (e) { if (e.key === 'ArrowDown' && !(pop && pop.anchor === scopeBtn)) { e.preventDefault(); openScope(); } });
  }

  /* ---------- Copy from another device: a menu, then an anchored confirm in its place ---------- */
  function openCopy() {
    var menu = document.createElement('div');
    menu.className = 'cd-set-pop cd-set-copymenu';
    menu.id = 'cd-set-copymenu';
    menu.setAttribute('role', 'menu');
    menu.setAttribute('aria-label', 'Copy from another device');
    var others = ordered(devices).filter(function (d) { return !scope || scope.indexOf(d.id) < 0; });
    if (!others.length) {
      var none = menuItem('menuitem', '', false, null, 'No other device', null);
      none.setAttribute('aria-disabled', 'true');
      menu.appendChild(none);
    }
    others.forEach(function (d) { menu.appendChild(menuItem('menuitem', d.id, false, d.kind, d.name, null)); });
    menu.style.top = popTop(copyBtn);
    menu.addEventListener('click', function (e) {
      var b = e.target.closest('[role=menuitem]');
      if (b && b.getAttribute('aria-disabled') !== 'true') confirmCopy(deviceById(b.getAttribute('data-id')));
    });
    menuKeys(menu);
    openPop(menu, scopeLine, copyBtn);
    var first = $('[role=menuitem]:not([aria-disabled=true])', menu) || $('[role=menuitem]', menu);
    if (first) first.focus();
  }
  function confirmCopy(from) {
    if (!from) return;
    var targets = scope ? scope.slice() : null;
    var box = document.createElement('div');
    box.className = 'cd-set-pop cd-set-confirm';
    box.id = 'cd-set-confirm';
    box.setAttribute('role', 'dialog');
    box.setAttribute('aria-label', 'Copy settings');
    box.innerHTML = '<p>Copy <b>' + esc(from.name) + '</b>’s settings to <b>' + esc(targets ? names(targets) : 'All devices') + '</b>?</p>' +
      '<div class="cd-set-state" aria-live="polite"></div>' +
      '<div class="cd-set-cbtns"><button type="button" class="cd-set-cbtn" data-act="cancel">Cancel</button>' +
      '<button type="button" class="cd-set-cbtn cd-primary" data-act="copy">Copy</button></div>';
    box.style.top = popTop(copyBtn);
    box.addEventListener('keydown', function (e) {
      if (e.key === 'Escape') { e.preventDefault(); closePop(true); }
    });
    box.addEventListener('click', function (e) {
      var b = e.target.closest('[data-act]');
      if (!b) return;
      if (b.getAttribute('data-act') === 'cancel') { closePop(true); return; }
      b.disabled = true;
      var ids = targets || ['all'];
      var chain = Promise.resolve({ ok: true });
      ids.forEach(function (id) {
        chain = chain.then(function (prev) {
          if (!prev.ok) return prev;
          return api('POST', '/api/v1/me/devices/' + encodeURIComponent(id) + '/copy-from', { from: from.id });
        });
      });
      chain.then(function (r) {
        if (r.ok && r.json && r.json.settings) {
          matrix = r.json.settings;
          closePop(true);
          paintAll();
        } else {
          b.disabled = false;
          setState(box, 'Couldn’t copy. Try again.', true);
          refresh(null);
        }
      });
    });
    openPop(box, scopeLine, copyBtn);
    $('[data-act=copy]', box).focus();
  }
  if (copyBtn) {
    copyBtn.addEventListener('click', function () { if (pop && pop.anchor === copyBtn) closePop(true); else openCopy(); });
  }

  /* ---------- the chain behind the layers icon ---------- */
  function chainLine(icon, name, note, value, resetId) {
    return '<div class="cd-set-cl"' + (resetId ? ' data-device="' + esc(resetId) + '"' : '') + '><span class="cd-set-micon">' + (icon ? kindIcon(icon, 16) : '') + '</span>' +
      '<div><span class="cd-set-clname">' + esc(name) + '</span>' + (note ? '<span class="cd-set-clnote">' + esc(note) + '</span>' : '') + '</div>' +
      '<div class="cd-set-clval"><span class="cd-set-clv">' + esc(value) + '</span>' +
      (resetId ? '<button type="button" class="cd-set-clreset" data-act="reset-device" data-device="' + esc(resetId) + '">Reset</button>' : '') + '</div></div>';
  }
  function fillChain(box, key) {
    var def = defs[key], e = matrix[key] || {};
    var ds = ordered(devices).filter(function (d) { return applies(def, d); });
    var vals = [];
    ds.forEach(function (d) { var r = (e.resolved || {})[d.id]; var v = r ? r.value : allValue(key); if (!vals.some(function (x) { return same(x, v); })) vals.push(v); });
    // All devices shows no value while the Devices disagree.
    var html = chainLine(null, 'All devices', '', vals.length > 1 ? '' : labelOf(def, allValue(key)), null);
    html += '<div class="cd-set-sep" role="separator"></div>';
    ds.forEach(function (d) {
      var own = e.devices && e.devices[d.id] && e.devices[d.id].value !== null && e.devices[d.id].value !== undefined;
      var r = (e.resolved || {})[d.id];
      html += chainLine(d.kind, d.name, own ? 'own' : 'follows All devices', labelOf(def, r ? r.value : allValue(key)), own ? d.id : null);
    });
    box.innerHTML = html;
  }
  function openChain(btn) {
    var row = btn.closest('.cd-set-row'), key = row.getAttribute('data-key');
    var box = document.createElement('div');
    box.className = 'cd-set-pop cd-set-chain';
    box.id = 'cd-set-chain';
    box.setAttribute('role', 'dialog');
    box.setAttribute('aria-label', defs[key].label + ' on each device');
    box.tabIndex = -1;
    fillChain(box, key);
    // Under the row, as round 3 draws it (frame 2: 190px from the content's top, 100px in).
    box.style.top = (row.offsetHeight + 28) + 'px';
    box.style.left = '100px';
    box.addEventListener('keydown', function (e) { if (e.key === 'Escape') { e.preventDefault(); closePop(true); } });
    box.addEventListener('click', function (e) {
      var b = e.target.closest('[data-act=reset-device]');
      if (!b) return;
      b.disabled = true;
      var id = b.getAttribute('data-device');
      api('DELETE', '/api/v1/me/settings/' + encodeURIComponent(key) + '?scope=device&device=' + encodeURIComponent(id)).then(function (r) {
        if (r.ok && r.json && r.json.matrix) {
          matrix[key] = r.json.matrix;
          paint(key);
          fillChain(box, key);
          box.focus();
        } else {
          b.disabled = false;
          setState(row, failText(r), true);
        }
      });
    });
    openPop(box, row, btn);
    box.focus();
  }

  /* ---------- the controls ---------- */
  app.addEventListener('click', function (e) {
    var t = e.target;
    var layers = t.closest('.cd-set-layers');
    if (layers) { if (pop && pop.anchor === layers) closePop(true); else openChain(layers); return; }
    var row = t.closest('.cd-set-row[data-key]');
    if (!row) return;
    var key = row.getAttribute('data-key'), def = defs[key];
    if (!def) return;
    var opt = t.closest('.cd-seg-o, .cd-sw');
    if (opt) {
      var v = opt.getAttribute('data-value');
      var s = shown(key);
      if (!s.mixed && same(String(s.value).toLowerCase(), v)) return;
      save(key, v);
      return;
    }
    var act = t.closest('[data-act]');
    if (!act) return;
    switch (act.getAttribute('data-act')) {
      case 'smaller': step(key, -1); break;
      case 'larger': step(key, 1); break;
      case 'custom': {
        var cr = $('.cd-set-customrow', row);
        cr.hidden = !cr.hidden;
        act.setAttribute('aria-expanded', cr.hidden ? 'false' : 'true');
        if (!cr.hidden) $('.cd-set-hex', row).focus();
        break;
      }
      case 'reset':
        api('DELETE', '/api/v1/me/settings/' + encodeURIComponent(key) + '?scope=user').then(function (r) {
          if (r.ok && r.json) { user[key] = r.json.value; paint(key); setState(row, ''); } else setState(row, failText(r), true);
        });
        break;
      default: break;
    }
  });
  /* Radio groups: arrows move and choose, as a radio group does. */
  app.addEventListener('keydown', function (e) {
    var opt = e.target.closest && e.target.closest('.cd-seg-o, .cd-sw');
    if (!opt || ['ArrowRight', 'ArrowDown', 'ArrowLeft', 'ArrowUp'].indexOf(e.key) < 0) return;
    var group = opt.closest('[role=radiogroup]');
    var opts = $$('[role=radio]', group).filter(function (b) { return !b.hidden; });
    var i = opts.indexOf(opt), n = opts.length;
    var next = opts[(i + (e.key === 'ArrowRight' || e.key === 'ArrowDown' ? 1 : n - 1)) % n];
    e.preventDefault();
    next.focus();
    next.click();
  });
  function applyHex(row, value) {
    var key = row.getAttribute('data-key');
    var v = String(value || '').trim().toLowerCase();
    if (!/^#[0-9a-f]{6}$/.test(v)) { setState(row, 'Use a color like #ffd54a.', true); return; }
    if (same(shown(key).value, v)) return;
    save(key, v);
  }
  app.addEventListener('change', function (e) {
    var row = e.target.closest('.cd-set-row[data-key]');
    if (!row) return;
    var key = row.getAttribute('data-key');
    if (e.target.classList.contains('cd-set-colorpick')) {
      $('.cd-set-hex', row).value = e.target.value;
      applyHex(row, e.target.value);
    } else if (e.target.classList.contains('cd-set-select')) {
      save(key, e.target.value);
    }
  });
  app.addEventListener('keydown', function (e) {
    if (e.key !== 'Enter' || !e.target.classList || !e.target.classList.contains('cd-set-hex')) return;
    e.preventDefault();
    applyHex(e.target.closest('.cd-set-row'), e.target.value);
  });

  /* ---------- Profile: the display name, saved on blur or Enter ---------- */
  var nameIn = document.getElementById('cd-set-name-in');
  if (nameIn) {
    var saved = nameIn.value;
    var nameRow = rowOf('display_name');
    var saveName = function () {
      var v = nameIn.value.trim();
      if (v === saved) { nameIn.value = saved; return; }
      if (!v) { nameIn.value = saved; setState(nameRow, 'A name can’t be empty.', true); return; }
      setState(nameRow, 'Saving…');
      api('POST', '/api/v1/me/rename', { name: v }).then(function (r) {
        if (r.ok && r.json) {
          saved = r.json.name || v;
          if (document.activeElement !== nameIn) nameIn.value = saved;
          setState(nameRow, '');
          var me = document.querySelector('.cd-me .cd-member');
          if (me) me.textContent = saved;
        } else {
          nameIn.value = saved;
          setState(nameRow, failText(r), true);
        }
      });
    };
    nameIn.addEventListener('blur', saveName);
    nameIn.addEventListener('keydown', function (e) {
      if (e.key === 'Enter') { e.preventDefault(); saveName(); }
      else if (e.key === 'Escape') { nameIn.value = saved; }
    });
  }

  /* ---------- Devices: Rename (inline) and Forget (inline yes / no), iOS Devices only ---------- */
  var devlist = document.getElementById('cd-devlist');
  if (devlist) {
    devlist.addEventListener('click', function (e) {
      var b = e.target.closest('[data-act]');
      if (!b) return;
      var card = b.closest('.cd-dev'), id = card.getAttribute('data-device');
      var slot = $('.cd-dev-inline', card), err = $('.cd-dev-err', card), nameEl = $('.cd-dev-name', card);
      var close = function () { slot.hidden = true; slot.innerHTML = ''; };
      err.hidden = true;
      switch (b.getAttribute('data-act')) {
        case 'rename':
          slot.innerHTML = '<input type="text" maxlength="80" aria-label="Device name"><button type="button" class="cd-dev-btn" data-act="rename-save">Save</button><button type="button" class="cd-dev-btn" data-act="cancel">Cancel</button>';
          slot.hidden = false;
          var inp = $('input', slot);
          inp.value = nameEl.textContent;
          inp.focus();
          inp.select();
          inp.addEventListener('keydown', function (k) {
            if (k.key === 'Enter') { k.preventDefault(); $('[data-act=rename-save]', slot).click(); }
            else if (k.key === 'Escape') { close(); b.focus(); }
          });
          break;
        case 'rename-save': {
          var v = $('input', slot).value.trim();
          if (!v) return;
          api('PATCH', '/api/v1/me/devices/' + encodeURIComponent(id), { name: v }).then(function (r) {
            if (r.ok && r.json) {
              nameEl.textContent = r.json.name;
              var d = deviceById(id);
              if (d) d.name = r.json.name;
              close();
            } else { err.textContent = r.error; err.hidden = false; }
          });
          break;
        }
        case 'forget':
          slot.innerHTML = '<span>Forget ' + esc(nameEl.textContent) + '?</span><button type="button" class="cd-dev-btn cd-danger" data-act="forget-yes">Forget</button><button type="button" class="cd-dev-btn" data-act="cancel">Cancel</button>';
          slot.hidden = false;
          $('[data-act=cancel]', slot).focus();
          break;
        case 'forget-yes':
          api('POST', '/api/v1/me/devices/' + encodeURIComponent(id) + '/forget').then(function (r) {
            if (r.ok) {
              card.parentNode.removeChild(card);
              devices = devices.filter(function (d) { return d.id !== id; });
            } else { err.textContent = r.error; err.hidden = false; }
          });
          break;
        case 'cancel':
          close();
          break;
        default: break;
      }
    });
  }

  /* ---------- live: a change made elsewhere (another device, another window) ---------- */
  var pending = null, waiting = [];
  function whereMade(evs) {
    for (var i = evs.length - 1; i >= 0; i--) {
      var p = (evs[i] && evs[i].payload) || {};
      var writer = String(p.stamp || '').split('-').slice(2).join('-');
      var d = deviceById(writer) || (p.device && p.device !== D.current ? deviceById(p.device) : null);
      if (d && d.id !== D.current) return d.name;
    }
    return 'another device';
  }
  function refresh(evs) {
    var before = {};
    Object.keys(defs).forEach(function (k) { before[k] = shown(k); });
    return Promise.all([
      api('GET', '/api/v1/me/settings/matrix'),
      api('GET', '/api/v1/me/settings?device=' + encodeURIComponent(D.current || 'web')),
      api('GET', '/api/v1/me/devices')
    ]).then(function (rs) {
      if (rs[2].ok && rs[2].json) devices = rs[2].json.devices || devices;
      if (rs[0].ok && rs[0].json) matrix = rs[0].json.settings || matrix;
      if (rs[1].ok && rs[1].json) (rs[1].json.settings || []).forEach(function (s) { if (s.kind === 'user') user[s.key] = s.value; });
      if (scope) {
        scope = scope.filter(function (id) { return !!deviceById(id); });
        if (!scope.length) scope = null;
        var sn = document.getElementById('cd-set-scopename');
        if (sn) sn.textContent = scopeName();
      }
      var where = evs ? whereMade(evs) : null;
      Object.keys(defs).forEach(function (k) {
        paint(k);
        if (where && !inflight[k] && !same(before[k], shown(k))) setState(rowOf(k), 'Changed on ' + where);
      });
      if (pop && pop.el.id === 'cd-set-chain') { var row = pop.anchor.closest('.cd-set-row'); fillChain(pop.el, row.getAttribute('data-key')); }
    });
  }
  function onEvent(ev) {
    waiting.push(ev);
    if (pending) return;
    pending = setTimeout(function () {
      var evs = waiting;
      waiting = [];
      pending = null;
      refresh(evs);
    }, 80);
  }
  function subscribe() {
    if (!window.EventSource || !D.matrix) return;
    var es;
    try { es = new EventSource('/api/v1/events?me=1&since=' + (D.seq || 0)); } catch (e) { return; }
    es.addEventListener('ready', function () { app.classList.add('cd-live'); });
    ['setting.changed', 'device.added', 'device.renamed', 'device.forgotten'].forEach(function (type) {
      es.addEventListener(type, function (e) {
        var ev = null;
        try { ev = JSON.parse(e.data); } catch (x) { ev = null; }
        onEvent(ev);
      });
    });
    es.onerror = function () { app.classList.remove('cd-live'); };
  }
  subscribe();
  app.classList.add('cd-set-ready');
})();
