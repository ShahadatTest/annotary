/* annotary owner screens (ticket 12): the script behind /members and /settings, inlined only
   there (after app.js, which localizes timestamps and owns the shared chrome). Every mutation is a
   fetch to /api/v1 with the session cookie; afterwards the card is re-drawn from the server's
   fragment (GET /members/{handle}, GET /settings/vaults/{name}) — the page never builds card
   markup itself beyond the small inline editors, confirms and the token-once panel. */
(function () {
  'use strict';
  var app = document.querySelector('.cd-app.cd-adminpage');
  if (!app) return;
  function $(sel, root) { return (root || app).querySelector(sel); }
  function $$(sel, root) { return Array.prototype.slice.call((root || app).querySelectorAll(sel)); }
  function esc(s) { return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; }); }
  function localize(root) { if (window.cd && window.cd.localize) window.cd.localize(root); }

  /* ---------- HTTP ---------- */
  function api(method, path, body) {
    var o = { method: method, credentials: 'same-origin', headers: {} };
    if (body !== undefined) { o.headers['Content-Type'] = 'application/json'; o.body = JSON.stringify(body); }
    return fetch(path, o).then(function (r) {
      return r.text().then(function (txt) {
        var j = null;
        try { j = txt ? JSON.parse(txt) : null; } catch (e) { j = null; }
        if (r.ok) return { ok: true, status: r.status, json: j };
        var msg = (j && j.message) || ('HTTP ' + r.status);
        if (j && j.hint) msg += ' — ' + j.hint;
        return { ok: false, status: r.status, json: j, error: msg, code: j && j.error };
      });
    }, function () {
      return { ok: false, status: 0, json: null, error: 'could not reach the server', code: 'unreachable' };
    });
  }

  /* ---------- errors / ok lines ---------- */
  function errBox(el) { return el.classList.contains('cd-err') ? el : $('.cd-err', el); }
  function showErr(scope, msg) {
    var e = errBox(scope);
    if (!e) return;
    e.textContent = msg;
    e.hidden = false;
  }
  function clearErr(scope) {
    var e = errBox(scope);
    if (e) { e.textContent = ''; e.hidden = true; }
  }
  function flashOk(scope, msg) {
    var ok = $('.cd-ok', scope);
    if (!ok) return;
    ok.textContent = msg;
    ok.hidden = false;
    clearTimeout(ok._t);
    ok._t = setTimeout(function () { ok.hidden = true; }, 6000);
  }
  function busy(btn, on) {
    if (!btn) return;
    btn.disabled = !!on;
    if (on) btn.setAttribute('aria-busy', 'true'); else btn.removeAttribute('aria-busy');
  }

  /* ---------- inline editors (rename) and confirms ---------- */
  function inlineEdit(slot, value, label, onSave) {
    slot.innerHTML = '<label>' + esc(label) + ' <input type="text" value="' + esc(value) + '" spellcheck="false"></label> ' +
      '<button type="button" class="cd-btn cd-sm cd-primary" data-inline="save">Save</button> ' +
      '<button type="button" class="cd-btn cd-sm" data-inline="cancel">Cancel</button><div class="cd-err" hidden></div>';
    slot.hidden = false;
    var input = $('input', slot);
    input.focus();
    input.select();
    function close() { slot.innerHTML = ''; slot.hidden = true; }
    function save() {
      clearErr(slot);
      busy($('[data-inline=save]', slot), true);
      onSave(input.value.trim()).then(function (res) {
        if (res && !res.ok) { busy($('[data-inline=save]', slot), false); showErr(slot, res.error); return; }
        close();
      });
    }
    slot.onclick = function (e) {
      var b = e.target.closest('[data-inline]');
      if (!b) return;
      e.preventDefault();
      if (b.getAttribute('data-inline') === 'cancel') close(); else save();
    };
    slot.onkeydown = function (e) {
      if (e.key === 'Enter') { e.preventDefault(); save(); }
      if (e.key === 'Escape') { e.preventDefault(); close(); }
    };
  }
  function inlineConfirm(slot, text, yesLabel, onYes) {
    slot.innerHTML = '<span class="cd-confirm">' + esc(text) + '</span> ' +
      '<button type="button" class="cd-btn cd-sm cd-primary" data-inline="yes">' + esc(yesLabel) + '</button> ' +
      '<button type="button" class="cd-btn cd-sm" data-inline="cancel">Cancel</button><div class="cd-err" hidden></div>';
    slot.hidden = false;
    function close() { slot.innerHTML = ''; slot.hidden = true; }
    slot.onclick = function (e) {
      var b = e.target.closest('[data-inline]');
      if (!b) return;
      e.preventDefault();
      if (b.getAttribute('data-inline') === 'cancel') { close(); return; }
      clearErr(slot);
      busy(b, true);
      onYes().then(function (res) {
        if (res && !res.ok) { busy(b, false); showErr(slot, res.error); return; }
        close();
      });
    };
    slot.onkeydown = function (e) { if (e.key === 'Escape') { e.preventDefault(); close(); } };
    var y = $('[data-inline=yes]', slot);
    if (y) y.focus();
  }

  /* ---------- the token-once panel ---------- */
  function tokenPanel(handle, token) {
    var line = 'ANNOTARY_TOKEN=' + token;
    var div = document.createElement('div');
    div.className = 'cd-tokonce';
    div.setAttribute('data-handle', handle);
    div.innerHTML = '<div class="cd-tokonce-h">Token for @' + esc(handle) + ' — shown once. Paste it into the agent’s environment:</div>' +
      '<pre class="cd-tokline"><code class="cd-tokvalue">' + esc(line) + '</code></pre>' +
      '<button type="button" class="cd-btn cd-sm cd-primary" data-act="copy">Copy</button> ' +
      '<button type="button" class="cd-btn cd-sm" data-act="dismiss">Dismiss</button><span class="cd-copied" hidden>copied</span>' +
      '<div class="cd-muted cd-small">A person can use it too: <code>annotary profile add NAME --token …</code>. Lost it? Revoke it and mint another.</div>';
    return div;
  }
  function copyText(text) {
    if (navigator.clipboard && navigator.clipboard.writeText) return navigator.clipboard.writeText(text);
    return new Promise(function (resolve, reject) {
      var ta = document.createElement('textarea');
      ta.value = text;
      ta.style.position = 'absolute';
      ta.style.left = '-9999px';
      document.body.appendChild(ta);
      ta.select();
      var ok = false;
      try { ok = document.execCommand('copy'); } catch (e) { ok = false; }
      document.body.removeChild(ta);
      if (ok) resolve(); else reject(new Error('copy failed'));
    });
  }
  /* ---------- ticket 18: the pairing panel (QR of the annotary://pair URL, shown once) ---------- */
  function pairPanel(handle, res) {
    var div = document.createElement('div');
    div.className = 'cd-tokonce cd-paironce';
    div.setAttribute('data-handle', handle);
    // `qr_svg` is the server's own SVG (black on white, no external references).
    div.innerHTML = '<div class="cd-tokonce-h">Pair a device for @' + esc(handle) + ' — ' + esc(res.server || '') + '</div>' +
      '<div class="cd-qr">' + (res.qr_svg || '') + '</div>' +
      '<p class="cd-pairnote">Scan with the iPhone or iPad camera, or Annotary. The code holds a new Token named \u201C' + esc(res.name || 'device') + '\u201D; it is shown once and can be revoked under TOKENS.</p>' +
      '<code class="cd-tokvalue cd-pairurl" hidden>' + esc(res.pair_url || '') + '</code>' +
      '<button type="button" class="cd-btn cd-sm cd-primary" data-act="copy">Copy link</button> ' +
      '<button type="button" class="cd-btn cd-sm" data-act="dismiss">Dismiss</button><span class="cd-copied" hidden>copied</span>';
    return div;
  }

  app.addEventListener('click', function (e) {
    var b = e.target.closest('.cd-tokonce [data-act]');
    if (!b) return;
    var panel = b.closest('.cd-tokonce');
    var act = b.getAttribute('data-act');
    if (act === 'dismiss') { panel.remove(); return; }
    if (act === 'copy') {
      var v = $('.cd-tokvalue', panel).textContent;
      copyText(v).then(function () {
        var c = $('.cd-copied', panel);
        if (c) { c.hidden = false; setTimeout(function () { c.hidden = true; }, 3000); }
      }, function () {
        var sel = window.getSelection(), r = document.createRange();
        r.selectNodeContents($('.cd-tokvalue', panel));
        sel.removeAllRanges();
        sel.addRange(r);
      });
    }
  });

  /* ====================================================================================
     Members
     ==================================================================================== */
  var memlist = $('#cd-memlist');
  function memberCard(handle) { return $('.cd-mem[data-handle="' + handle.replace(/"/g, '\\"') + '"]'); }
  function refreshMember(handle) {
    return api('GET', '/members/' + encodeURIComponent(handle)).then(function (res) {
      if (!res.ok || !res.json || !res.json.html) return res;
      var tmp = document.createElement('div');
      tmp.innerHTML = res.json.html;
      var fresh = tmp.firstElementChild;
      var old = memberCard(handle);
      if (old) {
        // An open Advanced disclosure stays open across a re-draw.
        var wasOpen = old.querySelector('details.cd-advanced');
        if (wasOpen && wasOpen.open) { var d = fresh.querySelector('details.cd-advanced'); if (d) d.open = true; }
        old.replaceWith(fresh);
      } else if (memlist) {
        memlist.appendChild(fresh);
      }
      localize(fresh);
      recountMembers();
      return { ok: true, card: fresh };
    });
  }
  function recountMembers() {
    var n = $('#cd-memcount');
    if (!n) return;
    var active = $$('.cd-mem:not(.cd-mem-disabled)', memlist).length;
    n.textContent = active + ' active';
  }
  if (memlist) {
    memlist.addEventListener('click', function (e) {
      var b = e.target.closest('[data-act]');
      if (!b || b.closest('.cd-tokonce') || b.closest('.cd-inline')) return;
      var card = b.closest('.cd-mem');
      if (!card) return;
      var handle = card.getAttribute('data-handle');
      var act = b.getAttribute('data-act');
      var base = '/api/v1/members/' + encodeURIComponent(handle);
      clearErr(card);
      e.preventDefault();
      if (act === 'rename') {
        var name = ($('.cd-member', card) || {}).textContent || '';
        inlineEdit($('[data-slot=rename]', card), name, 'Display name', function (value) {
          if (!value) return Promise.resolve({ ok: false, error: 'a display name is needed' });
          return api('POST', base + '/rename', { name: value }).then(function (res) { return res.ok ? refreshMember(handle) : res; });
        });
      } else if (act === 'disable') {
        inlineConfirm($('[data-slot=rename]', card), 'Disable @' + handle + '? Its Tokens and sessions are refused from now on; history and attribution stay. The handle is never reused.', 'Disable', function () {
          return api('POST', base + '/disable').then(function (res) { return res.ok ? refreshMember(handle) : res; });
        });
      } else if (act === 'mint') {
        var slot = $('[data-slot=mint]', card);
        slot.innerHTML = '<label>Name <input type="text" data-f="name" placeholder="optional, e.g. laptop" spellcheck="false"></label> ' +
          '<label>Expires <input type="text" data-f="expires" placeholder="optional: 2027-01-01 or ISO" size="22" spellcheck="false"></label> ' +
          '<button type="button" class="cd-btn cd-sm cd-primary" data-inline="save">Mint</button> <button type="button" class="cd-btn cd-sm" data-inline="cancel">Cancel</button><div class="cd-err" hidden></div>';
        slot.hidden = false;
        $('[data-f=name]', slot).focus();
        var closeMint = function () { slot.innerHTML = ''; slot.hidden = true; };
        var mint = function () {
          clearErr(slot);
          var body = { member: handle };
          var nm = $('[data-f=name]', slot).value.trim();
          var ex = $('[data-f=expires]', slot).value.trim();
          if (nm) body.name = nm;
          if (ex) body.expires_at = /^\d{4}-\d{2}-\d{2}$/.test(ex) ? ex + 'T00:00:00Z' : ex;
          busy($('[data-inline=save]', slot), true);
          api('POST', '/api/v1/tokens', body).then(function (res) {
            if (!res.ok) { busy($('[data-inline=save]', slot), false); showErr(slot, res.error); return; }
            var token = res.json && res.json.token;
            refreshMember(handle).then(function (r) {
              var fresh = (r && r.card) || memberCard(handle);
              var where = fresh && $('.cd-tokonce-slot', fresh);
              if (where && token) where.appendChild(tokenPanel(handle, token));
            });
          });
        };
        slot.onclick = function (ev) {
          var x = ev.target.closest('[data-inline]');
          if (!x) return;
          ev.preventDefault();
          if (x.getAttribute('data-inline') === 'cancel') closeMint(); else mint();
        };
        slot.onkeydown = function (ev) {
          if (ev.key === 'Enter') { ev.preventDefault(); mint(); }
          if (ev.key === 'Escape') { ev.preventDefault(); closeMint(); }
        };
      } else if (act === 'pair') {
        // ticket 18: a Token named "device" + its annotary://pair URL as a QR code; the card
        // re-draws (the new Token joins its table) and the panel goes into the fresh card.
        busy(b, true);
        api('POST', '/api/v1/pairing', { member: handle, name: 'device' }).then(function (res) {
          if (!res.ok) {
            busy(b, false);
            var me = $('.cd-memerr', card);
            if (me) { me.textContent = res.error; me.hidden = false; }
            return;
          }
          var pr = res.json || {};
          refreshMember(handle).then(function (r) {
            var fresh = (r && r.card) || memberCard(handle);
            var where = fresh && $('.cd-paironce-slot', fresh);
            if (where) where.appendChild(pairPanel(handle, pr));
          });
        });
      } else if (act === 'revoke') {
        var id = b.getAttribute('data-token');
        var cell = b.parentElement;
        cell.innerHTML = '<span class="cd-confirm">revoke?</span> <button type="button" class="cd-btn cd-sm cd-primary" data-act="revoke-yes" data-token="' + esc(id) + '">Revoke</button> <button type="button" class="cd-btn cd-sm" data-act="revoke-no" data-token="' + esc(id) + '">Keep</button>';
      } else if (act === 'revoke-no') {
        refreshMember(handle);
      } else if (act === 'revoke-yes') {
        busy(b, true);
        api('DELETE', '/api/v1/tokens/' + encodeURIComponent(b.getAttribute('data-token'))).then(function (res) {
          if (!res.ok) { showErr(card, res.error); busy(b, false); return; }
          refreshMember(handle);
        });
      } else if (act === 'role') {
        var sel = $('[data-field=role]', card);
        busy(b, true);
        api('POST', base + '/role', { role: sel.value }).then(function (res) {
          if (!res.ok) { busy(b, false); showErr(card, res.error); return; }
          refreshMember(handle);
        });
      } else if (act === 'grants') {
        var grants = [];
        $$('.cd-grantrow', card).forEach(function (row) {
          var rights = $$('input[data-right]:checked', row).map(function (i) { return i.getAttribute('data-right'); });
          if (rights.length) grants.push({ vault: row.getAttribute('data-vault'), rights: rights });
        });
        busy(b, true);
        api('PUT', base + '/grants', { grants: grants }).then(function (res) {
          if (!res.ok) { busy(b, false); showErr(card, res.error); return; }
          refreshMember(handle);
        });
      }
    });
  }

  /* ---------- Add agent ---------- */
  var addBtn = $('#cd-addagent');
  var addForm = $('#cd-addform');
  if (addBtn && addForm) {
    addBtn.addEventListener('click', function () {
      addForm.hidden = !addForm.hidden;
      if (!addForm.hidden) $('#cd-add-handle').focus();
    });
    addForm.addEventListener('click', function (e) {
      var c = e.target.closest('[data-act=cancel]');
      if (c) { e.preventDefault(); addForm.hidden = true; clearErr(addForm); }
    });
    addForm.addEventListener('submit', function (e) {
      e.preventDefault();
      clearErr(addForm);
      var handle = $('#cd-add-handle').value.trim().replace(/^@/, '');
      var name = $('#cd-add-name').value.trim();
      if (!handle) { showErr(addForm, 'a handle is needed (kebab-case, e.g. scribe)'); return; }
      var body = { handle: handle };
      if (name) body.name = name;
      var submit = $('button[type=submit]', addForm);
      busy(submit, true);
      api('POST', '/api/v1/members', body).then(function (res) {
        busy(submit, false);
        if (!res.ok) { showErr(addForm, res.error); return; }
        var token = res.json && res.json.token;
        var h = (res.json && res.json.handle) || handle;
        addForm.hidden = true;
        $('#cd-add-handle').value = '';
        $('#cd-add-name').value = '';
        var out = $('#cd-addresult');
        if (out && token) out.appendChild(tokenPanel(h, token));
        refreshMember(h).then(function (r) {
          if (r && r.card) r.card.scrollIntoView({ block: 'nearest' });
        });
      });
    });
  }

  /* ====================================================================================
     Settings
     ==================================================================================== */
  /* ---------- the local password ---------- */
  var pwSection = $('#cd-set-password');
  var pwForm = $('#cd-pwform');
  function setPasswordState(passwordSet, mine) {
    if (!pwSection) return;
    pwSection.setAttribute('data-password-set', passwordSet ? 'true' : 'false');
    pwSection.setAttribute('data-mine', mine ? 'true' : 'false');
    var state = $('#cd-pw-state');
    if (state) {
      state.innerHTML = passwordSet
        ? (mine ? 'A local password is set (yours). Browsers sign in at <code>/login</code>; <code>annotary login</code> turns it into a Token for the CLI.'
                : 'A local password is set by another owner. Setting one here sets <b>your</b> password.')
        : 'No local password is set — a browser on this machine is signed in as the founding owner automatically. Set one and the login page appears on the next request without a session.';
    }
    var setBtn = $('#cd-pw-set');
    if (setBtn) setBtn.textContent = mine ? 'Change password' : 'Set password';
    var rm = $('#cd-pw-remove');
    if (mine && !rm && setBtn) {
      rm = document.createElement('button');
      rm.type = 'button';
      rm.className = 'cd-btn';
      rm.id = 'cd-pw-remove';
      rm.setAttribute('data-act', 'remove-password');
      rm.textContent = 'Remove password';
      setBtn.insertAdjacentElement('afterend', rm);
      setBtn.insertAdjacentText('afterend', ' ');
    } else if (!mine && rm) {
      rm.remove();
    }
  }
  if (pwForm) {
    pwForm.addEventListener('submit', function (e) {
      e.preventDefault();
      clearErr(pwSection);
      var a = $('#cd-pw-new').value, b = $('#cd-pw-again').value;
      if (!a) { showErr(pwSection, 'enter a password'); return; }
      if (a !== b) { showErr(pwSection, 'the two passwords differ'); return; }
      var btn = $('#cd-pw-set');
      busy(btn, true);
      api('PUT', '/api/v1/settings/password', { password: a }).then(function (res) {
        busy(btn, false);
        if (!res.ok) { showErr(pwSection, res.error); return; }
        $('#cd-pw-new').value = '';
        $('#cd-pw-again').value = '';
        setPasswordState(true, true);
        flashOk(pwForm, 'Password set — browsers without a session now see the login page; this session stays signed in.');
      });
    });
    pwSection.addEventListener('click', function (e) {
      var b = e.target.closest('[data-act=remove-password]');
      if (!b) return;
      e.preventDefault();
      clearErr(pwSection);
      busy(b, true);
      api('PUT', '/api/v1/settings/password', { password: null }).then(function (res) {
        busy(b, false);
        if (!res.ok) { showErr(pwSection, res.error); return; }
        setPasswordState(!!(res.json && res.json.password_set), false);
        flashOk(pwForm, res.json && res.json.password_set ? 'Your password is removed (another owner still has one).' : 'Password removed — a loopback browser is signed in automatically again.');
      });
    });
  }

  /* ---------- Vaults ---------- */
  var vaults = $('#cd-vaults');
  function vaultCard(name) { return $('.cd-vault[data-vault="' + name.replace(/"/g, '\\"') + '"]'); }
  function refreshVault(name, replaceEl) {
    return api('GET', '/settings/vaults/' + encodeURIComponent(name)).then(function (res) {
      if (!res.ok || !res.json || !res.json.html) return res;
      var tmp = document.createElement('div');
      tmp.innerHTML = res.json.html;
      var fresh = tmp.firstElementChild;
      var old = replaceEl || vaultCard(name);
      if (old) old.replaceWith(fresh); else if (vaults) vaults.appendChild(fresh);
      localize(fresh);
      return { ok: true, card: fresh };
    });
  }
  function newActionRow() {
    var tr = document.createElement('tr');
    tr.className = 'cd-actrow';
    tr.innerHTML = '<td><input type="text" data-f="id" value="" placeholder="id (kebab-case)" spellcheck="false"></td>' +
      '<td><input type="text" data-f="label" value="" placeholder="label"></td>' +
      '<td><select data-f="intent"><option value="positive">positive</option><option value="neutral" selected>neutral</option><option value="negative">negative</option></select></td>' +
      '<td class="cd-actrowtools"><button type="button" class="cd-btn cd-sm" data-act="up" title="move up">↑</button><button type="button" class="cd-btn cd-sm" data-act="down" title="move down">↓</button><button type="button" class="cd-btn cd-sm" data-act="remove" title="remove">×</button></td>';
    return tr;
  }
  if (vaults) {
    vaults.addEventListener('click', function (e) {
      var b = e.target.closest('[data-act]');
      if (!b || b.closest('.cd-inline')) return;
      var card = b.closest('.cd-vault');
      if (!card) return;
      var name = card.getAttribute('data-vault');
      var act = b.getAttribute('data-act');
      var base = '/api/v1/vaults/' + encodeURIComponent(name);
      e.preventDefault();
      clearErr(card);
      if (act === 'rename') {
        inlineEdit($('[data-slot=rename]', card), name, 'Vault name', function (value) {
          if (!value) return Promise.resolve({ ok: false, error: 'a name is needed' });
          return api('POST', base + '/rename', { name: value }).then(function (res) {
            return res.ok ? refreshVault(res.json.name, card) : res;
          });
        });
      } else if (act === 'delete') {
        var n = +card.getAttribute('data-documents') || 0;
        inlineConfirm($('[data-slot=rename]', card), n ? 'Delete Vault ' + name + '? It still holds ' + n + ' Document' + (n === 1 ? '' : 's') + ' — the server will refuse until they are purged.' : 'Delete the empty Vault ' + name + '? Grants naming it are dropped.', 'Delete', function () {
          return api('DELETE', base).then(function (res) {
            if (res.ok) card.remove();
            return res;
          });
        });
      } else if (act === 'addrow') {
        var tbody = $('.cd-actsed tbody', card);
        var row = newActionRow();
        tbody.appendChild(row);
        $('[data-f=id]', row).focus();
      } else if (act === 'remove') {
        b.closest('tr').remove();
      } else if (act === 'up' || act === 'down') {
        var tr = b.closest('tr');
        var sib = act === 'up' ? tr.previousElementSibling : tr.nextElementSibling;
        if (sib) {
          if (act === 'up') sib.before(tr); else sib.after(tr);
        }
      } else if (act === 'save-actions') {
        var available = $$('.cd-actrow', card).map(function (r) {
          return { id: $('[data-f=id]', r).value.trim(), label: $('[data-f=label]', r).value.trim() || undefined, intent: $('[data-f=intent]', r).value };
        });
        busy(b, true);
        api('PUT', base + '/actions', { available: available }).then(function (res) {
          if (!res.ok) { busy(b, false); showErr(card, res.error); return; }
          refreshVault(name, card).then(function (r) {
            if (r && r.card) flashOk(r.card, 'saved');
          });
        });
      }
    });
  }
  var vaultNew = $('#cd-vaultnew');
  if (vaultNew) {
    vaultNew.addEventListener('submit', function (e) {
      e.preventDefault();
      clearErr(vaultNew);
      var name = $('#cd-vault-name').value.trim();
      if (!name) { showErr(vaultNew, 'a name is needed'); return; }
      var btn = $('button[type=submit]', vaultNew);
      busy(btn, true);
      api('POST', '/api/v1/vaults', { name: name }).then(function (res) {
        busy(btn, false);
        if (!res.ok) { showErr(vaultNew, res.error); return; }
        $('#cd-vault-name').value = '';
        refreshVault(res.json.name).then(function (r) { if (r && r.card) r.card.scrollIntoView({ block: 'nearest' }); });
      });
    });
  }

  /* ---------- auto-purge ---------- */
  var purgeForm = $('#cd-purgeform');
  var purgeSection = $('#cd-set-purge');
  if (purgeForm) {
    purgeForm.addEventListener('submit', function (e) {
      e.preventDefault();
      clearErr(purgeSection);
      var on = $('#cd-purge-on').checked;
      var days = parseInt($('#cd-purge-days').value, 10);
      if (on && !(days >= 1)) { showErr(purgeSection, 'enter how many days a deleted Document waits before it is purged'); return; }
      var btn = $('#cd-purge-save');
      busy(btn, true);
      api('PUT', '/api/v1/settings/auto-purge', { days: on ? days : null }).then(function (res) {
        busy(btn, false);
        if (!res.ok) { showErr(purgeSection, res.error); return; }
        var d = res.json && res.json.auto_purge_days;
        purgeSection.setAttribute('data-days', d || 0);
        flashOk(purgeForm, d ? 'saved — deleted Documents are purged after ' + d + ' day' + (d === 1 ? '' : 's') : 'saved — auto-purge is off');
      });
    });
    var run = $('#cd-runhk');
    if (run) run.addEventListener('click', function () {
      var out = $('#cd-hkreport');
      run.disabled = true;
      if (out) out.textContent = ' running…';
      api('POST', '/api/v1/housekeeping').then(function (res) {
        run.disabled = false;
        if (!res.ok) { if (out) out.textContent = ' ' + res.error; return; }
        var j = res.json || {};
        var purged = (j.purged || []).length;
        if (out) out.textContent = ' done: purged ' + purged + ' Document' + (purged === 1 ? '' : 's') + ', GC removed ' + (j.gc_removed || 0) + ' blob' + (j.gc_removed === 1 ? '' : 's') + ', ' + (j.html_dropped || 0) + ' stale render' + (j.html_dropped === 1 ? '' : 's') + ' cleaned (' + (j.took_ms || 0) + ' ms)';
      });
    });
  }
})();
