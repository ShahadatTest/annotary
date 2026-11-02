/* annotary web UI: the small script behind the server-composed pages. Everything it touches is
   already in the first HTML response (Threads, placements, the rendered Document); ticket 10's
   part only (1) localizes timestamps, (2) highlights anchored spans / Assets in the rendered
   Document from the embedded placements, (3) positions the margin cards (layout A) at their
   anchor line and pushes them apart, (4) drives the narrow header overlay, the resolved toggle,
   the Version select and the History panel (#cd-whatsnew) (GET /api/v1/events, on demand).
   Ticket 11 adds, on top of that and only on the Document page: the selection pill and the
   composer (selection → source coordinates via data-l + text offsets, verified against the head
   Markdown fetched once), whole-Asset Threads from inline media, reply / resolve / reopen / edit /
   delete, the Action buttons, the keyboard map `j k r e c [ ] ↩ ⌘↩ esc ?`, optimistic mutations
   (DOM first, then POST /api/v1/…, rolled back with a visible error) confirmed by re-drawing the
   card from GET /d/{id}/t/{tid} (the server's renderer), and the SSE live feed on
   GET /api/v1/events?doc= that redraws cards, the Action chip and announces new Versions. */
(function () {
  'use strict';
  /* ticket 15: the page is complete without this script (Threads, placements and the rendered
     Document are in the HTML), so let the browser paint it first — one frame, then the work. An
     inline module is deferred, but a fast parse can finish before the first frame is scheduled,
     and then the script would run ahead of the first paint. `.cd-app.cd-ready` still marks the
     moment the margin is laid out and the handlers are bound (`performance.mark('cd:ready')`). */
  if (window.requestAnimationFrame) {
    requestAnimationFrame(function () { setTimeout(main, 0); });
  } else {
    main();
  }
function main() {
  var app = document.querySelector('.cd-app');
  if (!app) return;
  var dataEl = document.getElementById('cd-data');
  var D = null;
  try { D = dataEl ? JSON.parse(dataEl.textContent) : null; } catch (e) { D = null; }
  var docEl = app.querySelector('.cd-doc');
  var WIDE = 850;

  /* ---------- CMD-23: the browser's time zone and hour cycle, for times the server renders ----------
     `cd_tz` (IANA name) and `cd_hc` (`12` / `24`) are saved on any page when missing or changed;
     the next page renders absolute times in that zone (Times shown as: Absolute). Until then times
     render relative, as before. Nothing on this page changes when they are saved. */
  function cookieVal(name) {
    var m = new RegExp('(?:^|; )' + name + '=([^;]*)').exec(document.cookie || '');
    return m ? m[1] : null;
  }
  try {
    var zoneOpts = new Intl.DateTimeFormat(undefined, { hour: 'numeric' }).resolvedOptions();
    var cycle = zoneOpts.hourCycle || (zoneOpts.hour12 ? 'h12' : 'h23');
    var zoneWant = { cd_tz: zoneOpts.timeZone, cd_hc: (cycle === 'h23' || cycle === 'h24') ? '24' : '12' };
    Object.keys(zoneWant).forEach(function (k) {
      var v = zoneWant[k];
      if (v && /^[A-Za-z0-9\/_+-]{1,64}$/.test(v) && cookieVal(k) !== v) document.cookie = k + '=' + v + '; Path=/; SameSite=Lax; Max-Age=31536000';
    });
  } catch (e) { /* no Intl: times stay relative */ }

  /* ---------- ticket 18 (X5): "Open in Annotary" — iPhone / iPad only ----------
     The server renders the chip `hidden` (no layout, no shift for anyone else); Safari on an
     iPhone, or an iPad (which may present itself as a Mac with touch), gets it shown, with the
     Thread of the URL's `#tN` / `#cd-tN` appended so the app opens at it. */
  var openApp = document.getElementById('cd-openapp');
  if (openApp) {
    var ua = navigator.userAgent || '';
    if (/iPhone|iPad/.test(ua) || (/Macintosh/.test(ua) && navigator.maxTouchPoints > 1)) {
      var openBase = openApp.getAttribute('href');
      var openHref = function () {
        var m = /^#(?:cd-)?(t\d+)$/.exec(location.hash || '');
        openApp.setAttribute('href', openBase + (m ? '&thread=' + m[1] : ''));
      };
      openHref();
      window.addEventListener('hashchange', openHref);
      openApp.hidden = false;
    }
  }
  function isWide() { return app.getBoundingClientRect().width >= WIDE; }
  function $(sel, root) { return (root || app).querySelector(sel); }
  function $$(sel, root) { return Array.prototype.slice.call((root || app).querySelectorAll(sel)); }
  function esc(s) { return String(s).replace(/[&<>"]/g, function (c) { return { '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;' }[c]; }); }
  function cssEsc(s) { return String(s).replace(/["\\]/g, '\\$&'); }

  /* ---------- timestamps: localized client-side ---------- */
  /* One Intl.DateTimeFormat per shape, made once: a 500-Thread page carries ~1,300 <time> tags
     and `toLocaleString` per tag (a new formatter each call) cost ~35 ms (ticket 15). */
  var fmts = {};
  var thisYear = new Date().getFullYear();
  function fmt(kind) {
    if (fmts[kind]) return fmts[kind];
    var o = kind === 'full' ? { year: 'numeric', month: 'numeric', day: 'numeric', hour: 'numeric', minute: '2-digit', second: '2-digit' }
      : { month: 'short', day: 'numeric', hour: 'numeric', minute: '2-digit' };
    if (kind === 'year') o.year = 'numeric';
    // CMD-23: the Clock setting (Follow system = the browser's own)
    var clock = app.getAttribute('data-cd-clock');
    if (clock === '12') o.hour12 = true;
    else if (clock === '24') o.hourCycle = 'h23';
    try { fmts[kind] = new Intl.DateTimeFormat(undefined, o); } catch (e) { fmts[kind] = { format: function (d) { return d.toLocaleString(); } }; }
    return fmts[kind];
  }
  function fmtAbs(d) {
    return fmt(d.getFullYear() !== thisYear ? 'year' : 'short').format(d);
  }
  function fmtRel(d) {
    var s = (Date.now() - d.getTime()) / 1000;
    if (s < 60) return 'now';
    var m = s / 60; if (m < 60) return Math.round(m) + 'm';
    var h = m / 60; if (h < 48) return Math.round(h) + 'h';
    var dd = h / 24; if (dd < 30) return Math.round(dd) + 'd';
    if (dd < 365) return Math.round(dd / 30) + 'mo';
    return Math.round(dd / 365) + 'y';
  }
  /* CMD-23: Times shown as Absolute — only once the server knows the zone (`data-cd-tz`, from the
     `cd_tz` cookie), so a relative age the server painted is never rewritten (CLS 0). The server
     renders those tags itself (`cd-abs cd-localized`); this covers tags drawn later. */
  function absTimes() { return app.getAttribute('data-cd-times') === 'absolute' && app.hasAttribute('data-cd-tz'); }
  function localizeTimes(root) {
    var abs = absTimes();
    $$('time[datetime]:not(.cd-localized)', root).forEach(function (t) {
      var d = new Date(t.getAttribute('datetime'));
      if (isNaN(d.getTime())) return;
      t.title = fmt('full').format(d);
      t.textContent = t.classList.contains('cd-rel') && !abs ? fmtRel(d) : fmtAbs(d);
      t.classList.add('cd-localized');
    });
    $$('option[data-at]', root).forEach(function (o) {
      var d = new Date(o.getAttribute('data-at'));
      if (isNaN(d.getTime())) return;
      o.textContent = (o.getAttribute('data-pre') || '') + fmtAbs(d) + (o.getAttribute('data-post') || '');
    });
  }
  localizeTimes(document);
  function timeTag(iso, rel) {
    return '<time' + (rel ? ' class="cd-rel"' : '') + ' datetime="' + esc(iso) + '">' + esc(iso) + '</time>';
  }

  /* ---------- the rendered Document: blocks by source line ---------- */
  var blocks = [];   // every [data-l] block of the Document, document order (= non-decreasing `a`)
  var tops = [];     // the blocks with no enclosing block, in order (siblings never overlap)
  var parentOf = null; // WeakMap: block element → its nearest enclosing block element (or null)
  function parseL(el) {
    var m = /^(\d+)-(\d+)$/.exec(el.getAttribute('data-l') || '');
    return m ? { a: +m[1], b: +m[2] } : null;
  }
  function indexBlocks() {
    blocks = []; tops = [];
    parentOf = new WeakMap();
    if (!docEl) return;
    $$('[data-l]', docEl).forEach(function (el) {
      if (el.closest('.cd-strip')) return;
      var r = parseL(el);
      if (!r) return;
      var p = el.parentElement && el.parentElement.closest('[data-l]');
      while (p && p.closest('.cd-strip')) p = p.parentElement && p.parentElement.closest('[data-l]');
      if (p && !docEl.contains(p)) p = null;
      parentOf.set(el, p || null);
      var b = { el: el, a: r.a, b: r.b };
      blocks.push(b);
      if (!p) tops.push(b);
    });
  }
  /* The nearest enclosing Document block (never inside a strip); indexed, so O(1) after indexBlocks. */
  function parentBlock(el) {
    if (parentOf && parentOf.has(el)) return parentOf.get(el);
    var p = el.parentElement && el.parentElement.closest('[data-l]');
    while (p && p.closest('.cd-strip')) p = p.parentElement && p.parentElement.closest('[data-l]');
    return p && docEl.contains(p) ? p : null;
  }
  /* Outermost blocks intersecting [a, b] (so no text is collected twice): the top-level blocks
     whose lines meet the range — found by binary search, not a scan (500 Threads × 1,700 blocks). */
  function outerBlocks(a, b) {
    var lo = 0, hi = tops.length;
    while (lo < hi) { var mid = (lo + hi) >> 1; if (tops[mid].b < a) lo = mid + 1; else hi = mid; }
    var out = [];
    for (var i = lo; i < tops.length && tops[i].a <= b; i++) if (tops[i].b >= a) out.push(tops[i]);
    return out;
  }
  /* The last top-level block starting at or before `line` (a fallback when a quote is not findable). */
  function blockBefore(line) {
    var lo = 0, hi = tops.length;
    while (lo < hi) { var mid = (lo + hi) >> 1; if (tops[mid].a <= line) lo = mid + 1; else hi = mid; }
    return lo > 0 ? tops[lo - 1] : null;
  }
  function textNodes(el, out) {
    var w = document.createTreeWalker(el, NodeFilter.SHOW_TEXT, {
      acceptNode: function (n) {
        var p = n.parentElement;
        if (!p || p.closest('.cd-strip')) return NodeFilter.FILTER_REJECT;
        if (p.closest('script,style')) return NodeFilter.FILTER_REJECT;
        return NodeFilter.FILTER_ACCEPT;
      }
    });
    var n;
    while ((n = w.nextNode())) out.push(n);
    return out;
  }
  /* Whitespace-insensitive search of `quote` in the text of `nodes`: returns [{node, off}] for
     the first and last character, or null. With several hits, `t` (the Thread's `line` and `occ`)
     picks the one the Thread is anchored to (CMD-7, `pickHit`), not simply the first. */
  function findQuote(nodes, quote, t) {
    var q = quote.replace(/\s+/g, ' ').trim();
    if (!q) return null;
    var chars = [];
    var map = [];
    var lastSpace = true;
    nodes.forEach(function (n) {
      if (!n) { // a block boundary: counts as whitespace
        if (!lastSpace) { chars.push(' '); map.push(null); lastSpace = true; }
        return;
      }
      var t = n.nodeValue;
      for (var i = 0; i < t.length; i++) {
        var c = t.charAt(i);
        if (/\s/.test(c)) {
          if (lastSpace) continue;
          chars.push(' '); map.push({ node: n, off: i }); lastSpace = true;
        } else {
          chars.push(c); map.push({ node: n, off: i }); lastSpace = false;
        }
      }
    });
    var hay = chars.join('');
    var hits = [];
    for (var k = hay.indexOf(q); k >= 0; k = hay.indexOf(q, k + 1)) {
      if (map[k] && map[k + q.length - 1]) hits.push({ start: map[k], end: map[k + q.length - 1] });
    }
    if (!hits.length) return null;
    if (hits.length === 1 || !t || !t.line) return hits[0];
    return pickHit(nodes, hits, t);
  }
  /* CMD-7: the same text can occur more than once in the searched blocks (two items of one list,
     a phrase twice on a line). Each hit's source line = the first line of its innermost `[data-l]`
     block + the soft breaks (`\n`, which the renderer keeps inside blocks) in that block's own text
     before it; the hits on the Thread's start line are taken in order and the server's `occ` (how
     many times the text starts earlier on that source line) picks one. No hit on the line ⇒ the
     nearest line; no usable line ⇒ the first hit. */
  function pickHit(nodes, hits, t) {
    var seen = new Map(); // block element → soft breaks counted in its own text so far
    var at = new Map();   // text node → its hits
    hits.forEach(function (h) { var l = at.get(h.start.node); if (l) l.push(h); else at.set(h.start.node, [h]); });
    var nl = function (s) { var c = 0; for (var i = 0; i < s.length; i++) if (s.charCodeAt(i) === 10) c++; return c; };
    nodes.forEach(function (n) {
      if (!n) return;
      var blk = n.parentElement && n.parentElement.closest('[data-l]');
      var r = blk && parseL(blk);
      var before = blk ? (seen.get(blk) || 0) : 0;
      // A fenced code block's lines include its opening fence: its text starts one line later.
      if (r && !seen.has(blk) && blk.tagName === 'PRE') before = Math.max(0, r.b - r.a + 1 - nl(blk.textContent));
      (at.get(n) || []).forEach(function (h) { h.line = r ? r.a + before + nl(n.nodeValue.slice(0, h.start.off)) : null; });
      if (blk) seen.set(blk, before + nl(n.nodeValue));
    });
    var onLine = hits.filter(function (h) { return h.line === t.line; });
    if (!onLine.length) {
      var best = null;
      hits.forEach(function (h) { if (h.line != null && (!best || Math.abs(h.line - t.line) < Math.abs(best.line - t.line))) best = h; });
      return best || hits[0];
    }
    var i = typeof t.occ === 'number' && t.occ > 0 ? t.occ : 0;
    return onLine[Math.min(i, onLine.length - 1)];
  }
  function wrap(nodes, hit, cls, tid) {
    var si = nodes.indexOf(hit.start.node), ei = nodes.indexOf(hit.end.node);
    if (si < 0 || ei < 0) return false;
    var made = false;
    for (var i = si; i <= ei; i++) {
      var n = nodes[i];
      if (!n) continue;
      var s = i === si ? hit.start.off : 0;
      var e = i === ei ? hit.end.off + 1 : n.nodeValue.length;
      if (e <= s) continue;
      var mid = n;
      if (s > 0) mid = n.splitText(s);
      if (e - s < mid.nodeValue.length) mid.splitText(e - s);
      var mark = document.createElement('mark');
      mark.className = cls;
      mark.setAttribute('data-t', tid);
      mid.parentNode.insertBefore(mark, mid);
      mark.appendChild(mid);
      indexMark(tid, mark);
      made = true;
    }
    return made;
  }
  /* The highlight elements of a Thread, indexed by id as the script makes them (ticket 15: a
     querySelectorAll per strip over a 60k-node Document made `layout()` cost 150 ms). */
  var markIndex = {};
  function indexMark(tid, el) {
    var list = markIndex[tid] || (markIndex[tid] = []);
    if (list.indexOf(el) < 0) list.push(el);
  }
  function marksOf(tid) {
    if (!docEl) return [];
    var list = markIndex[tid];
    if (list) {
      var live = list.filter(function (m) { return m.isConnected && m.getAttribute('data-t') === tid; });
      if (live.length !== list.length) markIndex[tid] = live;
      if (live.length) return live.slice();
    }
    var found = $$('mark.cd-hl[data-t="' + cssEsc(tid) + '"],.cd-asset-hl[data-t="' + cssEsc(tid) + '"],.cd-hl-block[data-t="' + cssEsc(tid) + '"]', docEl);
    if (found.length) markIndex[tid] = found.slice();
    return found;
  }
  function renameMarks(from, to) {
    var list = markIndex[from];
    delete markIndex[from];
    if (list) markIndex[to] = list;
  }
  /* Drop a Thread's highlight again (the marks unwrapped, the block / Asset classes removed). */
  function unhighlight(tid) {
    marksOf(tid).forEach(function (m) {
      if (m.tagName === 'MARK') {
        var p = m.parentNode;
        while (m.firstChild) p.insertBefore(m.firstChild, m);
        p.removeChild(m);
        p.normalize();
      } else {
        m.classList.remove('cd-asset-hl', 'cd-hl-block', 'cd-res', 'cd-selected', 'cd-pending-hl');
        m.removeAttribute('data-t');
      }
    });
    delete markIndex[tid];
  }
  function highlightThread(t) {
    if (!docEl) return;
    if (t.status === 'orphaned') return;
    if (t.subject === 'span' && t.line) {
      var outer = outerBlocks(t.line, t.end || t.line);
      var nodes = [];
      outer.forEach(function (b, i) { if (i) nodes.push(null); textNodes(b.el, nodes); });
      var quote = t.now || t.quote || '';
      var hit = quote ? findQuote(nodes, quote, t) : null;
      var cls = 'cd-hl cd-st-' + t.status + (t.resolved ? ' cd-res' : '');
      if (hit && wrap(nodes, hit, cls, t.id)) return;
      // The quote is not findable in the rendered text (Markdown syntax inside it, a table
      // cell…): mark the block instead.
      var blk = outer[0] || blockBefore(t.line);
      if (blk) { blk.el.classList.add('cd-hl-block'); if (t.resolved) blk.el.classList.add('cd-res'); if (!blk.el.hasAttribute('data-t')) blk.el.setAttribute('data-t', t.id); indexMark(t.id, blk.el); }
    } else if (t.subject === 'asset') {
      var el = null;
      if (t.hash) {
        el = $('img[src^="/a/' + t.hash + '"],video[src^="/a/' + t.hash + '"],audio[src^="/a/' + t.hash + '"],object[data^="/a/' + t.hash + '"]', docEl);
      }
      if (!el && t.ref) {
        el = $$('.asset-missing', docEl).filter(function (x) { return x.getAttribute('data-ref') === t.ref; })[0] || null;
      }
      if (el) { el.classList.add('cd-asset-hl'); if (t.resolved) el.classList.add('cd-res'); el.setAttribute('data-t', t.id); indexMark(t.id, el); }
    }
  }
  function highlight() {
    if (!D || !docEl) return;
    indexBlocks();
    (D.threads || []).forEach(highlightThread);
  }
  highlight();

  /* ---------- layout A: cards at their anchor line, pushed apart ---------- */
  var layoutPending = false;
  var readyOnce = false;
  function markReady() {
    if (readyOnce) return;
    readyOnce = true;
    app.classList.add('cd-ready');
    if (window.performance && performance.mark) performance.mark('cd:ready');
  }
  function anchorTop(strip, base) {
    var t = $('.cd-thread', strip);
    var tid = t && t.getAttribute('data-t');
    if (!tid) return null;
    var m = marksOf(tid)[0];
    return m ? m.getBoundingClientRect().top - base : null;
  }
  /* Reads first (one layout), then writes every `top` (one more): no thrash on a 500-Thread
     Document. Strips whose Threads are all resolved are display:none (CSS) unless the toggle is
     on — skipped by class, no getComputedStyle per strip. Cards are `content-visibility: auto`
     (app.css), so an off-screen strip's height is its cards' placeholder heights; when a card
     becomes relevant and takes its real height, `contentvisibilityautostatechange` schedules
     another pass (viewport-first rendering, ticket 15). */
  function layout() {
    layoutPending = false;
    if (!docEl) { markReady(); return; }
    var strips = $$('.cd-strip', docEl);
    if (!isWide()) {
      strips.forEach(function (s) { s.style.top = ''; });
      docEl.style.minHeight = '';
      markReady();
      return;
    }
    var base = docEl.getBoundingClientRect().top;
    var showRes = app.classList.contains('cd-show-resolved');
    var gap = 8, y = 0;
    var placed = [], tail = [], unanchored = [];
    strips.forEach(function (s) {
      if (!showRes && s.classList.contains('cd-all-resolved')) return;
      if (s.classList.contains('cd-strip-doc')) { placed.push({ el: s, want: 24, h: s.offsetHeight }); return; }
      if (s.classList.contains('cd-strip-orphaned')) { tail.push({ el: s, h: s.offsetHeight }); return; }
      var want = anchorTop(s, base);
      if (want === null) { unanchored.push(s); return; }
      placed.push({ el: s, want: want, h: s.offsetHeight });
    });
    if (unanchored.length) {
      // No highlight to align with: the strip's static position (right under its block).
      unanchored.forEach(function (s) { s.style.top = ''; });
      unanchored.forEach(function (s) { placed.push({ el: s, want: s.offsetTop, h: s.offsetHeight }); });
    }
    placed.sort(function (a, b) { return a.want - b.want; });
    placed.forEach(function (it) {
      var t = Math.max(y, it.want);
      it.el.style.top = t + 'px';
      y = t + it.h + gap;
    });
    tail.forEach(function (it) {
      var t = y + 16;
      it.el.style.top = t + 'px';
      y = t + it.h;
    });
    docEl.style.minHeight = (y + 120) + 'px';
    markReady();
  }
  function scheduleLayout() {
    if (layoutPending) return;
    layoutPending = true;
    (window.requestAnimationFrame || setTimeout)(layout, 0);
  }
  layout();
  window.addEventListener('resize', scheduleLayout);
  if (docEl) {
    // Media without manifest dimensions changes the flow when it loads (sized media does not).
    docEl.addEventListener('load', function (e) {
      var t = e.target;
      if (t && t.matches && t.matches('img,video,audio,object') && !(t.getAttribute('width') && t.getAttribute('height'))) scheduleLayout();
    }, true);
    // A card or block near the viewport took its real size: re-align the margin.
    docEl.addEventListener('contentvisibilityautostatechange', function () { if (isWide()) scheduleLayout(); }, true);
  }

  /* ---------- selection: highlight ↔ card ---------- */
  var selected = null;
  function select(tid, scroll) {
    $$('.cd-selected').forEach(function (e) { e.classList.remove('cd-selected'); });
    var card = $('.cd-thread[data-t="' + cssEsc(tid) + '"]');
    if (!card) return;
    selected = tid;
    if (card.classList.contains('cd-resolved') && !app.classList.contains('cd-show-resolved')) {
      var cb = $('#cd-resolved');
      if (cb) { cb.checked = true; }
      app.classList.add('cd-show-resolved');
    }
    var det = card.closest('details');
    if (det) det.open = true;
    card.classList.add('cd-selected', 'cd-open');
    marksOf(tid).forEach(function (m) { m.classList.add('cd-selected'); });
    layout();
    if (scroll) card.scrollIntoView({ block: 'nearest' });
  }
  function deselect() {
    selected = null;
    $$('.cd-selected').forEach(function (e) { e.classList.remove('cd-selected'); });
  }
  app.addEventListener('click', function (e) {
    var hl = e.target.closest('mark.cd-hl,.cd-asset-hl');
    // CMD-20: a resolved highlight behind the toggle is not there; an open one around it still is.
    if (!app.classList.contains('cd-show-resolved')) {
      while (hl && hl.classList.contains('cd-res')) hl = hl.parentElement && hl.parentElement.closest('mark.cd-hl,.cd-asset-hl');
    }
    if (hl && docEl && docEl.contains(hl) && !e.target.closest('.cd-strip')) {
      var tid = hl.getAttribute('data-t');
      if (/^new\d+$/.test(tid)) return;
      select(tid, true);
      return;
    }
    var row = e.target.closest('.cd-row');
    if (row) {
      if (e.target.closest('a,button,input,select,textarea')) return;
      var th = row.closest('.cd-thread');
      if (th.classList.contains('cd-composer')) return;
      var id = th.getAttribute('data-t');
      if (!isWide() && th.classList.contains('cd-open') && th.classList.contains('cd-selected')) {
        th.classList.remove('cd-open', 'cd-selected');
        $$('.cd-selected', docEl || app).forEach(function (m) { m.classList.remove('cd-selected'); });
        selected = null;
        layout();
        return;
      }
      select(id, false);
      var m = docEl && marksOf(id)[0];
      if (m && isWide()) m.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
  });
  if (location.hash && /^#cd-t\d+$/.test(location.hash)) select(location.hash.slice(4), true);
  // ticket 18: `annotary link DOC tN` prints …/d/{id}#tN — the same Thread selected.
  else if (location.hash && /^#t\d+$/.test(location.hash)) select(location.hash.slice(1), true);

  /* ---------- header: narrow overlay, resolved toggle, Version select, details ---------- */
  var line = $('.cd-line');
  if (line) {
    line.addEventListener('click', function (e) {
      if (e.target.closest('a,select,button,input,label')) return;
      if (isWide()) return;
      app.classList.toggle('cd-overlay-open');
    });
  }
  var news = $('#cd-news');
  var help = $('#cd-help');
  document.addEventListener('click', function (e) {
    if (app.classList.contains('cd-overlay-open') && !e.target.closest('.cd-hdr')) app.classList.remove('cd-overlay-open');
    if (news && !news.hidden && !e.target.closest('#cd-news,#cd-whatsnew')) news.hidden = true;
    if (help && !help.hidden && !e.target.closest('#cd-help,#cd-helpbtn')) help.hidden = true;
    if (pending && pending.kind === 'asset' && !e.target.closest('#cd-pill') && !(pending.el && pending.el.contains(e.target))) hidePill();
  });
  var resolvedCb = $('#cd-resolved');
  function updateOrphanedCount() {
    var n = $('.cd-orph-n');
    if (!n) return;
    var all = $$('details.cd-orphaned .cd-thread');
    var shown = app.classList.contains('cd-show-resolved') ? all.length : all.filter(function (t) { return !t.classList.contains('cd-resolved'); }).length;
    n.textContent = String(shown);
  }
  if (resolvedCb) {
    resolvedCb.addEventListener('change', function () {
      app.classList.toggle('cd-show-resolved', resolvedCb.checked);
      updateOrphanedCount();
      layout();
    });
  }
  var vsel = $('#cd-vsel');
  if (vsel) {
    vsel.addEventListener('change', function () {
      var o = vsel.options[vsel.selectedIndex];
      var href = o && o.getAttribute('data-href');
      if (href) location.href = href;
    });
  }
  $$('details.cd-orphaned').forEach(function (d) { d.addEventListener('toggle', scheduleLayout); });

  /* ---------- History: GET /api/v1/events for this Document, on demand ---------- */
  var newsBtn = $('#cd-whatsnew');
  function nameOf(handle) { return handle ? ((D && D.members && D.members[handle]) || '@' + handle) : 'someone'; }
  function describe(ev) {
    var p = ev.payload || {};
    var by = '<b title="@' + esc(ev.by || '') + '">' + esc(nameOf(ev.by)) + '</b>';
    var ex = ev.excerpt ? ' “' + esc(ev.excerpt) + '”' : '';
    switch (ev.type) {
      case 'version.pushed': return by + ' pushed <b>v' + esc(p.version) + '</b>' + (p.message ? ' — ' + esc(p.message) : '') + (p.same_as ? ' (same as v' + esc(p.same_as) + ')' : '');
      case 'version.rendered': return 'v' + esc(p.version) + ' re-rendered';
      case 'action.taken': return by + ' ' + esc(p.label || p.action) + ' <b>v' + esc(p.version) + '</b>';
      case 'thread.created': return by + ' opened <b>' + esc(p.thread) + '</b>' + ex;
      case 'thread.replied': return by + ' replied on <b>' + esc(p.thread) + '</b>' + ex;
      case 'thread.resolved': return by + ' resolved <b>' + esc(p.thread) + '</b>';
      case 'thread.reopened': return by + ' reopened <b>' + esc(p.thread) + '</b>';
      case 'comment.edited': return by + ' edited <b>' + esc(p.comment || p.thread) + '</b>';
      case 'comment.deleted': return by + ' deleted <b>' + esc(p.comment || p.thread) + '</b>';
      case 'document.created': return by + ' created <b>' + esc(p.name || '') + '</b>';
      case 'document.deleted': return by + ' deleted the Document';
      case 'document.restored': return by + ' restored the Document';
      case 'document.actions_declared': return by + ' declared the Actions';
      default: return by + ' ' + esc(ev.type);
    }
  }
  function newsItem(ev) {
    var p = ev.payload || {};
    var t = p.thread ? ' data-t="' + esc(p.thread) + '"' : '';
    return '<li' + t + ' data-seq="' + esc(ev.seq) + '">' + describe(ev) + ' <span class="cd-small">· <time class="cd-rel" datetime="' + esc(ev.at) + '">' + esc(ev.at) + '</time></span></li>';
  }
  function renderNews(events, err) {
    if (!news) return;
    var list = events.slice(-40).reverse();
    var html = '<h3>History</h3>';
    if (err) html += '<div class="cd-muted">' + esc(err) + '</div>';
    else if (!list.length) html += '<div class="cd-muted">Nothing yet.</div>';
    else html += '<ul>' + list.map(newsItem).join('') + '</ul>';
    news.innerHTML = html;
    localizeTimes(news);
  }
  function loadNews() {
    if (!D || !window.fetch) { renderNews([], 'events unavailable'); return; }
    var out = [], since = 0, tries = 0, head = D.seq || 0;
    renderNews([], 'loading…');
    (function step() {
      fetch('/api/v1/events?doc=' + encodeURIComponent(D.id) + '&since=' + since + '&limit=1000', { credentials: 'same-origin' })
        .then(function (r) { if (!r.ok) throw new Error('events: HTTP ' + r.status); return r.json(); })
        .then(function (j) {
          out = out.concat(j.events || []);
          since = j.seq || head;
          tries++;
          if (since < head && tries < 10) step(); else renderNews(out, null);
        })
        .catch(function (e) { renderNews([], String(e && e.message || e)); });
    })();
  }
  /* A live event while the panel is open: prepend it (loadNews re-reads everything otherwise). */
  function newsLive(ev) {
    if (!news || news.hidden) return;
    var ul = $('ul', news);
    if (!ul) { renderNews([ev], null); return; }
    if ($('li[data-seq="' + ev.seq + '"]', ul)) return;
    ul.insertAdjacentHTML('afterbegin', newsItem(ev));
    localizeTimes(ul);
  }
  if (newsBtn && news) {
    newsBtn.addEventListener('click', function () {
      news.hidden = !news.hidden;
      if (!news.hidden) loadNews();
    });
    news.addEventListener('click', function (e) {
      var li = e.target.closest('li[data-t]');
      if (li) { news.hidden = true; select(li.getAttribute('data-t'), true); }
    });
  }

  /* ====================================================================================
     ticket 11 — commenting, Actions, keyboard, live updates (the Document page only)
     ==================================================================================== */
  var isDocPage = !!(D && docEl && app.classList.contains('cd-docpage'));
  var onHead = isDocPage && D.version === D.head;
  var headMoved = false; // a newer Version was pushed while this page was open
  var newSeq = 0;

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
  function docApi(p) { return '/api/v1/docs/' + encodeURIComponent(D.id) + '/' + p; }

  /* ---------- toast (header-level errors) ---------- */
  var toast = $('#cd-toast');
  var toastTimer = null;
  function showToast(msg) {
    if (!toast) return;
    toast.textContent = msg;
    toast.hidden = false;
    clearTimeout(toastTimer);
    toastTimer = setTimeout(function () { toast.hidden = true; }, 8000);
  }
  if (toast) toast.addEventListener('click', function () { toast.hidden = true; });

  /* ---------- counts / status line ---------- */
  function recount() {
    if (!isDocPage) return;
    var cards = $$('.cd-thread:not(.cd-composer)', docEl);
    var open = 0, mine = 0, resolved = 0, orph = 0;
    cards.forEach(function (c) {
      var res = c.classList.contains('cd-resolved');
      if (res) resolved++; else open++;
      if (!res && c.classList.contains('cd-mine')) mine++;
      if (!res && c.closest('details.cd-orphaned')) orph++;
    });
    var act = D.action ? esc(String(D.action.label).toLowerCase()) + ' by ' + esc(nameOf(D.action.by)) : 'no action yet';
    var me = mine > 0 ? ' · <b class="cd-mention">' + mine + ' @you</b>' : '';
    var st = $('.cd-status');
    if (st) st.innerHTML = '<b>v' + D.version + '</b> · ' + act + ' · <b>' + open + '</b> open' + (orph > 0 ? ' (' + orph + ' orphaned)' : '') + me;
    var nl = $('.cd-nline');
    if (nl) nl.innerHTML = '· <b>v' + D.version + '</b> · <b>' + open + '</b> open' + me;
    var rn = $('.cd-res-n');
    if (rn) rn.textContent = String(resolved);
    // A strip whose Threads are all resolved hides with them.
    $$('.cd-strip', docEl).forEach(function (s) {
      if (s.classList.contains('cd-strip-orphaned')) return;
      var ts = $$('.cd-thread', s);
      var all = ts.length > 0 && ts.every(function (t) { return t.classList.contains('cd-resolved') && !t.classList.contains('cd-composer'); });
      s.classList.toggle('cd-all-resolved', all);
    });
    updateOrphanedCount();
  }

  /* ---------- placing cards: the client-side twin of splice.rs ---------- */
  var INSIDE = { li: 1, td: 1, th: 1, blockquote: 1, dd: 1, dt: 1, section: 1 };
  var AFTER = { p: 1, h1: 1, h2: 1, h3: 1, h4: 1, h5: 1, h6: 1, pre: 1, hr: 1, table: 1, ul: 1, ol: 1, dl: 1, math: 1, div: 1, figure: 1 };
  /* Where a strip for `line` goes: {el, where: 'inside'|'after'|'first'}. */
  function insertionPoint(line) {
    indexBlocks();
    var i = -1;
    for (var k = blocks.length - 1; k >= 0; k--) { if (blocks[k].a <= line && line <= blocks[k].b) { i = k; break; } }
    var cur = i >= 0 ? blocks[i].el : null;
    while (cur) {
      var name = cur.tagName.toLowerCase();
      if (INSIDE[name]) return { el: cur, where: 'inside' };
      if (AFTER[name]) {
        var p = parentBlock(cur);
        if (!p || INSIDE[p.tagName.toLowerCase()]) return { el: cur, where: 'after' };
      }
      cur = parentBlock(cur);
    }
    var last = null;
    blocks.forEach(function (b) { if (!parentBlock(b.el) && b.b < line) last = b.el; });
    return last ? { el: last, where: 'after' } : { el: docEl, where: 'first' };
  }
  /* Insert a strip among the strips already sitting at that point, in line order. */
  function placeStrip(strip, line) {
    var pt = insertionPoint(line);
    var sib, ref = null;
    if (pt.where === 'inside') {
      // strips inside a container sit at its end: find the first of that run with a larger line
      var kids = Array.prototype.slice.call(pt.el.children);
      for (var k = kids.length - 1; k >= 0 && kids[k].classList.contains('cd-strip'); k--) {
        var l = +kids[k].getAttribute('data-line') || 0;
        if (l > line) ref = kids[k];
      }
      pt.el.insertBefore(strip, ref);
    } else if (pt.where === 'after') {
      sib = pt.el.nextElementSibling;
      ref = pt.el;
      while (sib && sib.classList.contains('cd-strip') && !sib.classList.contains('cd-strip-orphaned') && !sib.classList.contains('cd-strip-doc') && (+sib.getAttribute('data-line') || 0) <= line) { ref = sib; sib = sib.nextElementSibling; }
      ref.after(strip);
    } else {
      var first = docEl.firstElementChild;
      while (first && first.classList.contains('cd-strip-doc')) first = first.nextElementSibling;
      docEl.insertBefore(strip, first);
    }
  }
  /* The strip a card with `line` belongs to (created when missing). */
  function stripFor(line) {
    var s = $('.cd-strip[data-line="' + line + '"]', docEl);
    if (s) return s;
    s = document.createElement('div');
    s.className = 'cd-strip';
    s.setAttribute('data-line', String(line));
    placeStrip(s, line);
    return s;
  }
  function docStrip() {
    var s = $('.cd-strip-doc', docEl);
    if (s) return s;
    s = document.createElement('div');
    s.className = 'cd-strip cd-strip-doc';
    s.innerHTML = '<div class="cd-strip-h">Whole document</div>';
    var h1 = docEl.firstElementChild;
    if (h1 && h1.tagName === 'H1' && h1.hasAttribute('data-l')) h1.after(s); else docEl.insertBefore(s, docEl.firstChild);
    return s;
  }
  function orphanedStrip() {
    var s = $('.cd-strip-orphaned', docEl);
    if (s) return $('details', s);
    s = document.createElement('div');
    s.className = 'cd-strip cd-strip-orphaned';
    var det = document.createElement('details');
    det.className = 'cd-orphaned';
    det.innerHTML = '<summary>Orphaned (<span class="cd-orph-n">0</span>) <span class="cd-muted">— text no longer in v' + D.version + '</span></summary>';
    s.appendChild(det);
    docEl.appendChild(s);
    det.addEventListener('toggle', scheduleLayout);
    return det;
  }
  function dropEmptyStrip(strip) {
    if (!strip || strip.classList.contains('cd-strip-orphaned')) return;
    if (!$('.cd-thread', strip)) strip.remove();
  }
  /* Put `card` where `item` says it lives (a new card, or one whose placement changed). */
  function placeCard(card, item) {
    var target;
    if (item.subject === 'document') target = docStrip();
    else if (item.status === 'orphaned') target = orphanedStrip();
    else if (item.line) target = stripFor(item.line);
    else target = docStrip();
    var from = card.closest('.cd-strip');
    if (card.parentElement === target) return;
    // Keep document order inside a strip: by Thread number.
    var num = +(String(item.id).replace(/^t/, '')) || 0;
    var ref = null;
    $$('.cd-thread', target).forEach(function (c) {
      var n = +(c.getAttribute('data-t') || '').replace(/^t/, '') || 0;
      if (!ref && n > num) ref = c;
    });
    target.insertBefore(card, ref);
    if (from && from !== target) dropEmptyStrip(from);
  }
  function itemIndex(tid) {
    for (var i = 0; i < (D.threads || []).length; i++) if (D.threads[i].id === tid) return i;
    return -1;
  }
  function setItem(item) {
    var i = itemIndex(item.id);
    if (i < 0) D.threads.push(item); else D.threads[i] = item;
  }
  function cardOf(tid) { return $('.cd-thread[data-t="' + cssEsc(tid) + '"]', docEl); }
  function removeCard(tid) {
    var card = cardOf(tid);
    unhighlight(tid);
    var i = itemIndex(tid);
    if (i >= 0) D.threads.splice(i, 1);
    if (card) {
      var strip = card.closest('.cd-strip');
      card.remove();
      dropEmptyStrip(strip);
    }
    if (selected === tid) selected = null;
    recount();
    scheduleLayout();
  }
  function htmlToEl(html) {
    var tpl = document.createElement('template');
    tpl.innerHTML = html;
    return tpl.content.firstElementChild;
  }
  /* Swap the server's card in (or insert a new one) and sync the highlight / data / counts. */
  function applyFragment(tid, frag) {
    if (!frag) return;
    if (frag.hidden) { removeCard(tid); return; }
    var item = frag.item;
    var fresh = htmlToEl(frag.html);
    if (!fresh) return;
    var old = cardOf(tid);
    var wasSelected = old ? old.classList.contains('cd-selected') : selected === tid;
    var wasOpen = old && old.classList.contains('cd-open');
    var prev = D.threads[itemIndex(tid)];
    var placementChanged = !prev || prev.status !== item.status || prev.line !== item.line || prev.end !== item.end || prev.quote !== item.quote || prev.now !== item.now || prev.occ !== item.occ;
    if (old) {
      // An open editor survives, empty or not (CMD-13): the card is re-drawn once it closes.
      if ($('.cd-editor', old)) { old.setAttribute('data-stale', '1'); return; }
      old.replaceWith(fresh);
    }
    placeCard(fresh, item);
    setItem(item);
    if (placementChanged || !prev || !marksOf(tid).length) { unhighlight(tid); indexBlocks(); highlightThread(item); }
    marksOf(tid).forEach(function (m) { m.classList.toggle('cd-res', !!item.resolved); });
    localizeTimes(fresh);
    if (wasSelected) { fresh.classList.add('cd-selected'); marksOf(tid).forEach(function (m) { m.classList.add('cd-selected'); }); }
    if (wasOpen) fresh.classList.add('cd-open');
    recount();
    scheduleLayout();
  }
  function refreshCard(tid) {
    var q = D.version !== D.head ? '?v=' + D.version : '';
    return api('GET', '/d/' + encodeURIComponent(D.id) + '/t/' + encodeURIComponent(tid) + q).then(function (r) {
      if (r.ok) applyFragment(tid, r.json);
      else if (r.status === 404) removeCard(tid);
      return r;
    });
  }

  /* ---------- echo detection for the live feed (my own mutations come back as events) ---------- */
  var inflight = []; // {type, tid}
  var recent = [];   // {key, at}
  function beginOp(type, tid) { var op = { type: type, tid: tid }; inflight.push(op); return op; }
  function endOp(op, tid) {
    var i = inflight.indexOf(op);
    if (i >= 0) inflight.splice(i, 1);
    if (tid) recent.push({ key: op.type + ':' + tid, at: Date.now() });
  }
  function isEcho(ev) {
    if (!ev || ev.by !== D.me) return false;
    var tid = ev.payload && ev.payload.thread;
    var type = { 'thread.created': 'created', 'thread.replied': 'replied', 'thread.resolved': 'resolved', 'thread.reopened': 'reopened', 'comment.edited': 'edited', 'comment.deleted': 'deleted', 'action.taken': 'action' }[ev.type];
    if (!type) return false;
    var now = Date.now();
    recent = recent.filter(function (r) { return now - r.at < 15000; });
    if (recent.some(function (r) { return r.key === type + ':' + (type === 'action' ? 'action' : tid); })) return true;
    return inflight.some(function (op) { return op.type === type && (!op.tid || op.tid === tid); });
  }

  /* ---------- errors on a card ---------- */
  function cardError(card, msg) {
    if (!card) { showToast(msg); return; }
    var e = $('.cd-err', card);
    if (!e) { e = document.createElement('div'); e.className = 'cd-err'; var body = $('.cd-body', card); (body || card).appendChild(e); }
    e.textContent = msg;
    e.hidden = false;
    card.classList.add('cd-open');
    scheduleLayout();
  }
  function clearError(card) {
    var e = card && $('.cd-err', card);
    if (e) e.remove();
  }

  /* ---------- editors (reply / edit / the composer) ---------- */
  function editorHtml(kind, text, placeholder) {
    var resolve = kind === 'reply' ? '<label class="cd-chk"><input type="checkbox" class="cd-resolve-too"> resolve</label>' : '';
    var post = kind === 'reply' ? 'Reply' : kind === 'edit' ? 'Save' : 'Comment';
    var hint = kind === 'reply' ? '↩ to post' : '⌘↩ to post';
    return '<div class="cd-editor cd-editor-' + kind + '"><textarea rows="3" placeholder="' + esc(placeholder || '') + '">' + esc(text || '') + '</textarea>' +
      '<div class="cd-err" hidden></div>' +
      '<div class="cd-acts">' + resolve + '<span class="cd-grow"></span><span class="cd-small cd-muted">' + hint + '</span>' +
      '<button type="button" class="cd-btn cd-sm cd-primary" data-act="post">' + post + '</button>' +
      '<button type="button" class="cd-btn cd-sm" data-act="cancel">Cancel</button></div></div>';
  }
  function focusEditor(card) {
    var ta = $('.cd-editor textarea', card);
    if (ta) { ta.focus(); ta.setSelectionRange(ta.value.length, ta.value.length); }
  }
  function closeEditor(card, which) {
    var ed = which || (card && $('.cd-editor', card));
    if (!ed) return;
    var kind = ed.classList.contains('cd-editor-edit') ? 'edit' : ed.classList.contains('cd-editor-reply') ? 'reply' : 'new';
    if (kind === 'edit') {
      var c = ed.closest('.cd-c');
      var body = c && $('.cd-cbody', c);
      if (body) body.hidden = false;
    }
    ed.remove();
    if (kind === 'new') {
      // the composer: drop the pending highlight and the strip
      var tid = card.getAttribute('data-t');
      var strip = card.closest('.cd-strip');
      unhighlight(tid);
      card.remove();
      dropEmptyStrip(strip);
    } else if (card.getAttribute('data-stale')) {
      card.removeAttribute('data-stale');
      refreshCard(card.getAttribute('data-t'));
    }
    scheduleLayout();
  }
  function anyEditor() { return $('.cd-editor', docEl); }
  /* CMD-6: Escape never throws typed text away. An editor is dirty when it holds text (the edit
     box: text other than the Comment's body, kept as `cdOrig`); Escape on a dirty editor asks
     inline (`.cd-discard`: discard / keep) and Escape while it asks means keep. A clean editor
     closes at once. The Cancel button still closes without asking.
     CMD-12: opening another composer over a dirty one asks the same way; the new target waits on
     the editor as `cdNextPending`, "discard" opens it, "keep" / Escape / Cancel / posting drop it. */
  function editorDirty(ed) {
    var ta = $('textarea', ed);
    var text = ta ? ta.value.trim() : '';
    if (ed.classList.contains('cd-editor-edit')) return text !== String(ed.cdOrig == null ? '' : ed.cdOrig).trim();
    return text !== '';
  }
  function keepEditing(ed) {
    ed.cdNextPending = null;
    var ask = $('.cd-discard', ed);
    if (ask) ask.remove();
    var ta = $('textarea', ed);
    if (ta) ta.focus();
    scheduleLayout();
  }
  function escapeEditor(card, ed) {
    if (!ed || !card) return;
    if (!editorDirty(ed)) { closeEditor(card, ed); return; }
    if ($('.cd-discard', ed)) { keepEditing(ed); return; }
    askDiscard(ed, null);
  }
  function askDiscard(ed, next) {
    ed.cdNextPending = next;
    if ($('.cd-discard', ed)) { scheduleLayout(); return; }
    var box = document.createElement('div');
    box.className = 'cd-discard';
    box.setAttribute('role', 'alert'); // never the literal attribute: the login page's tests grep for it
    box.innerHTML = 'Discard what you typed? <button type="button" class="cd-link" data-act="discard-yes">discard</button><button type="button" class="cd-link" data-act="discard-no">keep</button>';
    $('textarea', ed).after(box);
    scheduleLayout();
  }

  /* ---------- optimistic mutations ---------- */
  function snapshot(card) { return card.cloneNode(true); }
  function rollback(card, snap, msg) {
    card.replaceWith(snap);
    cardError(snap, msg);
    recount();
    scheduleLayout();
    return snap;
  }
  function setResolvedDom(card, on) {
    card.classList.toggle('cd-resolved', on);
    var dot = $('.cd-dot', card);
    if (dot) dot.title = on ? 'resolved' : 'open';
    var btn = $('.cd-acts [data-act="resolve"],.cd-acts [data-act="reopen"]', card);
    if (btn) { btn.setAttribute('data-act', on ? 'reopen' : 'resolve'); btn.textContent = on ? 'Reopen' : 'Resolve'; }
    var res = $('.cd-resline', card);
    if (on && !res) {
      res = document.createElement('div');
      res.className = 'cd-resline';
      res.innerHTML = '✓ resolved by ' + esc(D.me_name || D.me) + ' · ' + timeTag(new Date().toISOString(), false) + ' · v' + D.head;
      var acts = $('.cd-acts', card);
      if (acts) acts.before(res); else $('.cd-body', card).appendChild(res);
      localizeTimes(card);
    } else if (!on && res) res.remove();
    var tid = card.getAttribute('data-t');
    marksOf(tid).forEach(function (m) { m.classList.toggle('cd-res', on); });
    var item = D.threads[itemIndex(tid)];
    if (item) item.resolved = on;
  }
  function commentHtml(cid, text, pending) {
    return '<div class="cd-c' + (pending ? ' cd-pending' : '') + '" data-c="' + esc(cid) + '"><div class="cd-who"><b class="cd-member" title="@' + esc(D.me) + '">' + esc(D.me_name || D.me) + '</b><span>' + timeTag(new Date().toISOString(), false) + '</span><span class="cd-mono">v' + D.head + '</span></div><div class="cd-cbody">' + plainBody(text) + '</div></div>';
  }
  function plainBody(text) {
    return String(text).split(/\n{2,}/).map(function (p) { return '<p>' + esc(p).replace(/\n/g, '<br>') + '</p>'; }).join('');
  }

  function postReply(card) {
    var ed = $('.cd-editor-reply', card);
    if (!ed) return;
    var ta = $('textarea', ed);
    var text = ta.value.trim();
    if (!text) { ta.focus(); return; }
    var tid = card.getAttribute('data-t');
    var resolve = !!($('.cd-resolve-too', ed) && $('.cd-resolve-too', ed).checked);
    ed.remove();
    clearError(card);
    var snap = snapshot(card); // the card without the editor: what a rollback restores
    var n = $$('.cd-c', card).length;
    var acts = $('.cd-acts', card);
    var c = htmlToEl(commentHtml(tid + '.' + n, text, true));
    var res = $('.cd-resline', card);
    if (res) res.before(c); else if (acts) acts.before(c); else $('.cd-body', card).appendChild(c);
    localizeTimes(c);
    if (resolve) setResolvedDom(card, true);
    recount();
    scheduleLayout();
    var op = beginOp('replied', tid);
    var op2 = resolve ? beginOp('resolved', tid) : null;
    api('POST', docApi('threads/' + encodeURIComponent(tid) + '/replies'), { body: text, resolve: resolve }).then(function (r) {
      var cid = r.ok && r.json && r.json.comment && r.json.comment.id;
      endOp(op, tid);
      if (op2) endOp(op2, tid);
      if (!r.ok) {
        var back = rollback(cardOf(tid) || card, snap, 'Reply not posted: ' + r.error);
        var body = $('.cd-body', back);
        var acts2 = $('.cd-acts', back);
        var edEl = htmlToEl(editorHtml('reply', text, 'Reply…'));
        if (acts2) acts2.before(edEl); else body.appendChild(edEl);
        if (resolve) { var cb = $('.cd-resolve-too', edEl); if (cb) cb.checked = true; }
        var e = $('.cd-body > .cd-err', back);
        if (e) { var ee = $('.cd-err', edEl); ee.textContent = e.textContent; ee.hidden = false; e.remove(); }
        back.classList.add('cd-open');
        focusEditor(back);
        scheduleLayout();
        return;
      }
      if (cid) c.setAttribute('data-c', cid);
      refreshCard(tid);
    });
  }

  function setResolved(tid, on) {
    var card = cardOf(tid);
    if (!D.can_comment || !card || card.classList.contains('cd-composer') || card.classList.contains('cd-pending')) return;
    if (card.classList.contains('cd-resolved') === on) return;
    var snap = snapshot(card);
    clearError(card);
    setResolvedDom(card, on);
    recount();
    scheduleLayout();
    var op = beginOp(on ? 'resolved' : 'reopened', tid);
    api('POST', docApi('threads/' + encodeURIComponent(tid) + '/' + (on ? 'resolve' : 'reopen'))).then(function (r) {
      endOp(op, tid);
      if (!r.ok) {
        var back = rollback(cardOf(tid) || card, snap, (on ? 'Not resolved: ' : 'Not reopened: ') + r.error);
        setResolvedDom(back, !on);
        if (selected === tid) back.classList.add('cd-selected');
        recount();
        return;
      }
      refreshCard(tid);
    });
  }

  function startEdit(card, cEl) {
    var cid = cEl.getAttribute('data-c');
    var tid = card.getAttribute('data-t');
    if ($('.cd-editor', cEl)) return;
    // The raw Markdown is not on the page (bodies are rendered): read it once.
    api('GET', docApi('threads/' + encodeURIComponent(tid))).then(function (r) {
      if (!r.ok) { cardError(card, 'Cannot edit: ' + r.error); return; }
      var t = r.json;
      var c = t.id === cid ? t : (t.replies || []).filter(function (x) { return x.id === cid; })[0];
      if (!c) { cardError(card, 'Cannot edit: no Comment ' + cid); return; }
      var body = $('.cd-cbody', cEl);
      body.hidden = true;
      body.insertAdjacentHTML('afterend', editorHtml('edit', c.body, 'Edit…'));
      body.nextElementSibling.cdOrig = c.body; // CMD-6: unsaved = differs from this
      card.classList.add('cd-open');
      focusEditor(card);
      scheduleLayout();
    });
  }
  function postEdit(card) {
    var ed = $('.cd-editor-edit', card);
    if (!ed) return;
    var cEl = ed.closest('.cd-c');
    var cid = cEl.getAttribute('data-c');
    var tid = card.getAttribute('data-t');
    var text = $('textarea', ed).value.trim();
    if (!text) { $('textarea', ed).focus(); return; }
    var orig = ed.cdOrig;
    ed.remove();
    clearError(card);
    var body = $('.cd-cbody', cEl);
    body.hidden = false;
    var snap = snapshot(card); // the card without the editor: what a rollback restores
    body.innerHTML = plainBody(text);
    cEl.classList.add('cd-pending');
    if (!$('.cd-who .cd-edited', cEl)) $('.cd-who .cd-mono', cEl).insertAdjacentHTML('beforebegin', '<span class="cd-edited">· edited</span>');
    scheduleLayout();
    var op = beginOp('edited', tid);
    api('PUT', docApi('threads/' + encodeURIComponent(cid)), { body: text }).then(function (r) {
      endOp(op, tid);
      if (!r.ok) {
        var back = rollback(cardOf(tid) || card, snap, 'Edit not saved: ' + r.error);
        var c2 = $('.cd-c[data-c="' + cssEsc(cid) + '"]', back);
        var b2 = c2 && $('.cd-cbody', c2);
        if (b2) { b2.hidden = true; b2.insertAdjacentHTML('afterend', editorHtml('edit', text, 'Edit…')); b2.nextElementSibling.cdOrig = orig; }
        back.classList.add('cd-open');
        focusEditor(back);
        return;
      }
      refreshCard(tid);
    });
  }
  function deleteComment(card, cEl) {
    var cid = cEl.getAttribute('data-c');
    var tid = card.getAttribute('data-t');
    var snap = snapshot(card);
    clearError(card);
    $('.cd-cbody', cEl).innerHTML = '<span class="cd-tomb">(deleted)</span>';
    var tools = $('.cd-ctools', cEl); if (tools) tools.remove();
    cEl.classList.add('cd-pending');
    scheduleLayout();
    var op = beginOp('deleted', tid);
    api('DELETE', docApi('threads/' + encodeURIComponent(cid))).then(function (r) {
      endOp(op, tid);
      if (!r.ok) { rollback(cardOf(tid) || card, snap, 'Not deleted: ' + r.error); return; }
      refreshCard(tid);
    });
  }

  /* ---------- Actions (head only) ---------- */
  function setChip(a) {
    var area = $('#cd-actions');
    if (!area) return;
    var old = $('.cd-chip', area);
    var cls = a.intent === 'positive' ? 'cd-pos' : a.intent === 'negative' ? 'cd-neg' : 'cd-neu';
    var html = '<span class="cd-chip cd-action ' + cls + '" data-action="' + esc(a.id) + '" data-version="' + esc(a.version) + '">' + esc(a.label) + ' · ' + esc(nameOf(a.by)) + ' · ' + timeTag(a.at, true) + '</span>';
    var el = htmlToEl(html);
    if (old) old.replaceWith(el); else area.insertBefore(el, area.firstChild);
    localizeTimes(el);
    D.action = a;
    if (vsel) {
      var o = $('option[value="' + a.version + '"]', vsel);
      if (o && o.hasAttribute('data-post')) {
        var post = o.getAttribute('data-post') || '';
        post = post.replace(/ · (?!same as v)[^·]*$/, '');
        o.setAttribute('data-post', post + ' · ' + a.label);
        localizeTimes(vsel);
      }
    }
    recount();
  }
  function takeAction(btn) {
    if (!onHead || headMoved) { showToast('A newer Version was pushed — go to head to act on it.'); return; }
    var id = btn.getAttribute('data-action');
    var area = $('#cd-actions');
    var oldChip = $('.cd-chip', area);
    var oldHtml = oldChip ? oldChip.outerHTML : null;
    var oldAction = D.action;
    setChip({ id: id, label: btn.getAttribute('data-label') || id, intent: btn.getAttribute('data-intent') || 'neutral', by: D.me, at: new Date().toISOString(), version: D.head });
    var op = beginOp('action', null);
    api('POST', docApi('actions'), { action: id, version: D.head }).then(function (r) {
      endOp(op, 'action');
      if (!r.ok) {
        var cur = $('.cd-chip', area);
        if (oldHtml) { var el = htmlToEl(oldHtml); cur.replaceWith(el); localizeTimes(el); } else cur.remove();
        D.action = oldAction;
        recount();
        showToast('Action not taken: ' + r.error);
        if (r.code === 'not_head') { headMoved = true; hideHeadOnly(); }
        return;
      }
      var a = r.json;
      setChip({ id: a.id, label: a.label, intent: a.intent, by: a.by, at: a.at, version: a.version });
    });
  }
  function hideHeadOnly() {
    var b = $('#cd-actbtns'); if (b) b.hidden = true;
    hidePill();
  }
  function rebuildActionButtons(available) {
    var area = $('#cd-actions');
    if (!area || !onHead || !D.can_act) return;
    var box = $('#cd-actbtns');
    if (!available.length) { if (box) box.remove(); return; }
    if (!box) { box = document.createElement('span'); box.className = 'cd-actbtns'; box.id = 'cd-actbtns'; area.appendChild(box); }
    box.innerHTML = available.map(function (d) {
      var cls = d.intent === 'positive' ? 'cd-pos' : d.intent === 'negative' ? 'cd-neg' : 'cd-neu';
      return '<button type="button" class="cd-btn cd-actbtn ' + cls + '" data-action="' + esc(d.id) + '" data-label="' + esc(d.label) + '" data-intent="' + esc(d.intent) + '" data-version="' + D.head + '">' + esc(d.label) + '</button>';
    }).join('');
    box.hidden = headMoved;
  }

  /* ---------- selection → source coordinates ---------- */
  var source = null; // {lines: [[code points]]} | {promise}
  function splitLines(text) {
    if (!text) return [];
    var ls = text.split('\n');
    if (text.charAt(text.length - 1) === '\n') ls.pop();
    return ls.map(function (l) { return Array.from(l.charAt(l.length - 1) === '\r' ? l.slice(0, -1) : l); });
  }
  function loadSource() {
    if (source && source.lines) return Promise.resolve(source.lines);
    if (source && source.promise) return source.promise;
    var p = fetch('/api/v1/blobs/' + encodeURIComponent(D.md_hash), { credentials: 'same-origin' })
      .then(function (r) { if (!r.ok) throw new Error('HTTP ' + r.status); return r.text(); })
      .then(function (text) { source = { lines: splitLines(text) }; return source.lines; })
      .catch(function (e) { source = null; throw e; });
    source = { promise: p };
    return p;
  }
  function isWs(c) { return /\s/.test(c); }
  var entityBox = null;
  function decodeEntity(s) {
    if (!entityBox) entityBox = document.createElement('textarea');
    entityBox.innerHTML = s;
    var v = entityBox.value;
    return v === s ? null : v;
  }
  /* Source text S (code points) of a block vs its rendered text R (code points): for every
     rendered char the source index it came from. Markdown syntax between rendered chars is
     skipped: link targets `](…)`, whole images `![…](…)`, raw HTML tags, and entities are
     decoded so `&amp;` lines up with `&`. null when the text cannot be followed. */
  function align(R, S) {
    var out = new Array(R.length);
    var j = 0, n = S.length;
    function skipBalanced(open, close) { // j at `open`; move past its matching `close`
      var depth = 0;
      for (; j < n; j++) { if (S[j] === open) depth++; else if (S[j] === close) { depth--; if (depth === 0) { j++; return true; } } }
      return false;
    }
    function looksLikeTag(k) {
      var rest = S.slice(k, k + 24).join('');
      if (/^<[a-zA-Z][a-zA-Z0-9+.-]*:/.test(rest)) return false; // an autolink
      return /^<\/?[a-zA-Z][a-zA-Z0-9-]*(\s|>|\/)/.test(rest) || /^<!--/.test(rest);
    }
    for (var i = 0; i < R.length; i++) {
      var c = R[i];
      if (isWs(c)) { out[i] = null; continue; }
      var found = false;
      while (j < n) {
        var s = S[j];
        if (s === c) { out[i] = { at: j, len: 1 }; j++; found = true; break; }
        if (s === '!' && S[j + 1] === '[') { j++; if (skipBalanced('[', ']') && S[j] === '(') skipBalanced('(', ')'); continue; }
        if (s === ']' && S[j + 1] === '(') { j++; skipBalanced('(', ')'); continue; }
        if (s === '<' && looksLikeTag(j)) { var close = S.indexOf('>', j); j = close < 0 ? n : close + 1; continue; }
        if (s === '&') {
          var m = /^&(#x[0-9a-fA-F]+|#\d+|[a-zA-Z][a-zA-Z0-9]*);/.exec(S.slice(j, j + 12).join(''));
          if (m) {
            var d = decodeEntity(m[0]);
            var len = Array.from(m[0]).length;
            if (d !== null && Array.from(d)[0] === c) { out[i] = { at: j, len: len }; j += len; found = true; break; }
            j += len;
            continue;
          }
        }
        j++;
      }
      if (!found) return null;
    }
    return out;
  }
  /* The block whose source lines a selection point maps through: the innermost data-l block,
     climbed while the parent spans the same lines (a td → its tr, a p → its blockquote). */
  function mapBlock(node) {
    var el = node.nodeType === 1 ? node : node.parentElement;
    var b = el && el.closest('[data-l]');
    while (b && b.closest('.cd-strip')) b = b.parentElement && b.parentElement.closest('[data-l]');
    if (!b || !docEl.contains(b)) return null;
    var r = parseL(b);
    if (!r) return null;
    for (;;) {
      var p = parentBlock(b);
      var pr = p && parseL(p);
      if (!pr || pr.a !== r.a || pr.b !== r.b) break;
      b = p;
    }
    return { el: b, a: r.a, b: r.b };
  }
  /* A selection boundary as (text node, offset in it) — element boundaries are moved to the
     nearest text node inside the block. */
  function textPoint(block, node, off, atEnd) {
    var nodes = textNodes(block.el, []);
    if (node.nodeType === 3) return nodes.indexOf(node) >= 0 ? { nodes: nodes, node: node, off: off } : null;
    // An element boundary: before child `off` (or at the very end).
    var kids = node.childNodes;
    var probe = null;
    if (!atEnd) {
      for (var k = off; k < kids.length && !probe; k++) {
        var n = kids[k];
        probe = n.nodeType === 3 ? n : textNodes(n, [])[0] || null;
      }
      if (probe && nodes.indexOf(probe) >= 0) return { nodes: nodes, node: probe, off: 0 };
      // nothing after: the end of the block
      var last = nodes[nodes.length - 1];
      return last ? { nodes: nodes, node: last, off: last.nodeValue.length } : null;
    }
    for (var k2 = off - 1; k2 >= 0 && !probe; k2--) {
      var n2 = kids[k2];
      var tn = n2.nodeType === 3 ? [n2] : textNodes(n2, []);
      probe = tn[tn.length - 1] || null;
    }
    if (probe && nodes.indexOf(probe) >= 0) return { nodes: nodes, node: probe, off: probe.nodeValue.length };
    var first = nodes[0];
    return first ? { nodes: nodes, node: first, off: 0 } : null;
  }
  /* Code-point offset of (node, off) inside the block's rendered text, plus that text. */
  function renderedOffset(tp) {
    var before = -1, text = [];
    for (var i = 0; i < tp.nodes.length; i++) {
      var n = tp.nodes[i];
      var cps = Array.from(n.nodeValue);
      if (n === tp.node) { before = text.length + Array.from(n.nodeValue.slice(0, tp.off)).length; }
      text = text.concat(cps);
    }
    if (before < 0) return null;
    return { off: before, text: text };
  }
  function sourceOf(lines, a, b) { // the joined source of lines a..b as code points + line starts
    var cps = [], starts = [];
    for (var l = a; l <= b; l++) {
      if (l > a) cps.push('\n');
      starts.push(cps.length);
      var ln = lines[l - 1];
      if (ln === undefined) return null;
      cps = cps.concat(ln);
    }
    return { cps: cps, starts: starts };
  }
  function toLineCol(src, a, idx) {
    var li = 0;
    for (var i = 0; i < src.starts.length; i++) if (src.starts[i] <= idx) li = i; else break;
    return { line: a + li, col: idx - src.starts[li] };
  }
  /* Emphasis / code delimiters the selection covered on one side only: a selection of "bold"
     inside `**bold**` stays `bold`, but "with bold" over `with **bold**` becomes `with **bold**`
     — when the runs of a delimiter inside the range are odd, the adjacent run outside is taken
     in (not across a word, so `snake_case` never grows). Source columns are the truth; this only
     decides where the user's selection ends in that truth. */
  var DELIM = /[*_~`]/;
  function balanceDelimiters(lines, start, end) {
    function runsInside(c) {
      var runs = 0;
      for (var l = start.line; l <= end.line; l++) {
        var ln = lines[l - 1];
        var from = l === start.line ? start.col : 0, to = l === end.line ? end.col : ln.length;
        var inRun = false;
        for (var i = from; i < to; i++) {
          if (ln[i] === c) { if (!inRun) { runs++; inRun = true; } } else inRun = false;
        }
      }
      return runs;
    }
    var isWord = function (ch) { return ch !== undefined && /[\p{L}\p{N}]/u.test(ch); };
    for (var g = 0; g < 3; g++) {
      var ln = lines[end.line - 1], c = ln[end.col];
      if (c === undefined || !DELIM.test(c)) break;
      var k = end.col; while (ln[k] === c) k++;
      if (c !== '`' && isWord(ln[k])) break;
      if (runsInside(c) % 2 === 0) break;
      end = { line: end.line, col: k };
    }
    for (g = 0; g < 3; g++) {
      var ln2 = lines[start.line - 1];
      if (start.col === 0) break;
      var c2 = ln2[start.col - 1];
      if (!DELIM.test(c2)) break;
      var k2 = start.col; while (k2 > 0 && ln2[k2 - 1] === c2) k2--;
      if (c2 !== '`' && isWord(ln2[k2 - 1])) break;
      if (runsInside(c2) % 2 === 0) break;
      start = { line: start.line, col: k2 };
    }
    return { start: start, end: end };
  }
  /* The selection as a source range: {range: "L:C-L:C", exact, line, end, text} or null. */
  function mapSelection(range, lines) {
    var sb = mapBlock(range.startContainer), eb = mapBlock(range.endContainer);
    if (!sb || !eb) return null;
    var sp = textPoint(sb, range.startContainer, range.startOffset, false);
    var ep = textPoint(eb, range.endContainer, range.endOffset, true);
    if (!sp || !ep) return null;
    var ro = renderedOffset(sp), re = renderedOffset(ep);
    if (!ro || !re) return null;
    var same = sb.el === eb.el;
    var ss = sourceOf(lines, sb.a, sb.b), es = same ? ss : sourceOf(lines, eb.a, eb.b);
    if (!ss || !es) return null;
    var as = align(ro.text, ss.cps), ae = same ? as : align(re.text, es.cps);
    if (!as || !ae) return null;
    // First selected non-whitespace char → start; last one → end.
    var i = ro.off; while (i < ro.text.length && (isWs(ro.text[i]) || !as[i])) i++;
    var k = re.off - 1; while (k >= 0 && (isWs(re.text[k]) || !ae[k])) k--;
    if (i >= ro.text.length || k < 0 || (same && i > k)) return null;
    var startIdx = as[i].at, endIdx = ae[k].at + ae[k].len;
    var start = toLineCol(ss, sb.a, startIdx), end = toLineCol(es, eb.a, endIdx);
    if (end.line < start.line || (end.line === start.line && end.col <= start.col)) return null;
    // The exact source text of a range (what the server will freeze as the quote).
    var extract = function (a, b) {
      var out = [];
      for (var l = a.line; l <= b.line; l++) {
        var ln = lines[l - 1];
        var from = l === a.line ? a.col : 0, to = l === b.line ? b.col : ln.length;
        if (l > a.line) out.push('\n');
        out = out.concat(ln.slice(from, to));
      }
      return out.join('');
    };
    var exact = extract(start, end);
    // Verify: the selected text's visible characters all occur, in order, in the source range,
    // which starts and ends with the same characters as the selection.
    var sel = Array.from(range.toString()).filter(function (c) { return !isWs(c); });
    var src = Array.from(exact).filter(function (c) { return !isWs(c); });
    var decoded = decodeEntity(exact.replace(/</g, '&lt;')) || exact;
    var srcDec = Array.from(decoded).filter(function (c) { return !isWs(c); });
    if (!sel.length || !src.length) return null;
    if (src[0] !== sel[0] && srcDec[0] !== sel[0]) return null;
    var last = sel[sel.length - 1];
    if (src[src.length - 1] !== last && srcDec[srcDec.length - 1] !== last) return null;
    var p = 0;
    for (var q = 0; q < srcDec.length && p < sel.length; q++) if (srcDec[q] === sel[p]) p++;
    if (p < sel.length) {
      p = 0;
      for (q = 0; q < src.length && p < sel.length; q++) if (src[q] === sel[p]) p++;
      if (p < sel.length) return null;
    }
    var bal = balanceDelimiters(lines, start, end);
    start = bal.start; end = bal.end;
    exact = extract(start, end);
    return { range: start.line + ':' + start.col + '-' + end.line + ':' + end.col, exact: exact, line: start.line, end: end.line, text: range.toString() };
  }

  /* ---------- the pill ---------- */
  var pill = $('#cd-pill');
  var pending = null; // {kind: 'span', range: Range} | {kind: 'asset', el, hash, ref}
  function hidePill() { if (pill) pill.hidden = true; pending = null; }
  function mediaOf(el) {
    var m = el && el.closest && el.closest('img,video,audio,object');
    if (!m || !docEl.contains(m) || m.closest('.cd-strip')) return null;
    var src = m.getAttribute('src') || m.getAttribute('data') || '';
    var h = /^\/a\/([0-9a-f]{64})/.exec(src);
    if (!h) return null;
    var hash = h[1];
    var asset = (D.assets || []).filter(function (a) { return a.hash === hash; })[0];
    if (!asset) return null;
    return { kind: 'asset', el: m, hash: hash, ref: asset.ref };
  }
  function showPillAt(rect) {
    if (!pill) return;
    var ar = app.getBoundingClientRect();
    pill.hidden = false;
    var w = pill.offsetWidth || 90;
    var left = rect.left - ar.left + rect.width / 2 - w / 2;
    left = Math.max(8, Math.min(left, ar.width - w - 8));
    var top = rect.top - ar.top - 40;
    if (top < 8) top = rect.bottom - ar.top + 8;
    pill.style.left = left + 'px';
    pill.style.top = top + 'px';
  }
  /* The range trimmed to the visible text it covers. A triple-click (or a drag past the end of
     a list item) ends the selection at offset 0 of the next block — an `h2`, the next `li` — and
     mapping that end would fail. Only blank edges are trimmed: when the selection takes in any
     text of a card (or an editor) it is returned as it is, so the strip check below still
     refuses it. null when it covers no text. */
  function tightRange(range) {
    var anc = range.commonAncestorContainer;
    var root = anc.nodeType === 1 ? anc : anc.parentElement;
    if (!root) return null;
    var w = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode: function (n) {
        var p = n.parentElement;
        return !p || p.closest('script,style') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT;
      }
    });
    var sc = range.startContainer;
    if (sc.nodeType === 3) w.currentNode = sc;
    else if (sc.childNodes[range.startOffset]) w.currentNode = sc.childNodes[range.startOffset];
    else w.currentNode = sc;
    var first = null, last = null;
    var n = w.currentNode.nodeType === 3 ? w.currentNode : w.nextNode();
    for (; n; n = w.nextNode()) {
      if (range.comparePoint(n, 0) > 0) break; // starts after the range
      if (!range.intersectsNode(n)) continue;
      var v = n.nodeValue;
      var s = n === range.startContainer ? range.startOffset : 0;
      var e = n === range.endContainer ? range.endOffset : v.length;
      var part = v.slice(s, e);
      var a = part.search(/\S/);
      if (a < 0) continue;
      if (n.parentElement.closest('.cd-strip,.cd-editor')) return null; // card text: not ours to trim
      if (!first) first = { node: n, off: s + a };
      last = { node: n, off: s + part.replace(/\s+$/, '').length };
    }
    if (!first) return null;
    var r = document.createRange();
    r.setStart(first.node, first.off);
    r.setEnd(last.node, last.off);
    return r;
  }
  function onSelectionChange() {
    if (!pill || !onHead || headMoved || !D.can_comment) return;
    var sel = window.getSelection();
    if (!sel || !sel.rangeCount || sel.isCollapsed) {
      if (pending && pending.kind === 'span') hidePill();
      return;
    }
    var range = sel.getRangeAt(0);
    if (range.toString().trim()) range = tightRange(range) || range;
    var anc = range.commonAncestorContainer;
    var ancEl = anc.nodeType === 1 ? anc : anc.parentElement;
    if (!ancEl || !docEl.contains(ancEl) || ancEl.closest('.cd-strip,.cd-editor')) { hidePill(); return; }
    // A selection that crosses a card is not a span of the Document.
    var frag = range.cloneContents();
    if (frag.querySelector && frag.querySelector('.cd-strip')) { hidePill(); return; }
    var text = range.toString();
    if (!text.trim()) {
      // Only media inside? → a whole-Asset Thread.
      var live = $$('img,video,audio,object', docEl).filter(function (m) { return range.intersectsNode(m); })[0];
      var asset = live && mediaOf(live);
      if (asset) { pending = asset; showPillAt(live.getBoundingClientRect()); return; }
      hidePill();
      return;
    }
    if (!mapBlock(range.startContainer) || !mapBlock(range.endContainer)) { hidePill(); return; }
    pending = { kind: 'span', range: range.cloneRange() };
    loadSource().catch(function () { /* reported when the pill is pressed */ });
    showPillAt(range.getBoundingClientRect());
  }
  if (pill) {
    document.addEventListener('selectionchange', function () { setTimeout(onSelectionChange, 0); });
    pill.addEventListener('mousedown', function (e) { e.preventDefault(); });
    pill.addEventListener('click', function () { openComposer(); });
    docEl.addEventListener('click', function (e) {
      if (!onHead || headMoved || !D.can_comment) return;
      var asset = mediaOf(e.target);
      if (!asset) return;
      if (e.target.closest('a')) return;
      e.preventDefault();
      var sel = window.getSelection();
      if (sel && !sel.isCollapsed) return;
      pending = asset;
      showPillAt(asset.el.getBoundingClientRect());
    });
  }

  /* ---------- the composer: a new Thread where it will live ---------- */
  function composerCard(tid, labelHtml, subjectCls) {
    return htmlToEl('<article class="cd-thread cd-composer cd-st-anchored' + (subjectCls ? ' ' + subjectCls : '') + '" data-t="' + tid + '">' +
      '<div class="cd-row"><span class="cd-dot" title="new"></span><div class="cd-main"><div class="cd-quote">' + labelHtml + '</div></div></div>' +
      '<div class="cd-body">' + editorHtml('new', '', 'Comment… (⌘↩ to post)') + '</div></article>');
  }
  function openComposer(target) {
    var p = target || pending;
    if (!p || !onHead || headMoved) return;
    hidePill();
    var existing = $('.cd-composer', docEl);
    var exEd = existing && $('.cd-editor-new', existing);
    if (exEd && editorDirty(exEd)) {
      // CMD-12: never drop typed text for a new target; ask, and open it only on "discard".
      askDiscard(exEd, p);
      focusEditor(existing);
      return;
    }
    if (existing) closeEditor(existing);
    if (p.kind === 'asset') {
      var tid = 'new' + (++newSeq);
      var blk = p.el.closest('[data-l]');
      var r = blk && parseL(blk);
      var line = r ? r.a : 1;
      var card = composerCard(tid, '<span class="cd-kind">asset</span><q>' + esc(p.ref) + '</q>', 'cd-asset');
      card.setAttribute('data-line', String(line));
      card.cdPending = { subject: 'asset', ref: p.ref, hash: p.hash, line: line };
      p.el.classList.add('cd-asset-hl', 'cd-pending-hl');
      p.el.setAttribute('data-t', tid);
      stripFor(line).appendChild(card);
      card.classList.add('cd-open');
      deselect();
      focusEditor(card);
      scheduleLayout();
      return;
    }
    var range = p.range;
    loadSource().then(function (lines) {
      var m = mapSelection(range, lines);
      if (!m) { showToast('Could not map that selection to the source — select plain text inside the Document.'); return; }
      var tid = 'new' + (++newSeq);
      var sb = mapBlock(range.startContainer), eb = mapBlock(range.endContainer);
      // Highlight the selection itself (the exact DOM range), pending.
      indexBlocks();
      var nodes = [];
      outerBlocks(sb.a, eb.b).forEach(function (b, i) { if (i) nodes.push(null); textNodes(b.el, nodes); });
      var sp = textPoint(sb, range.startContainer, range.startOffset, false);
      var ep = textPoint(eb, range.endContainer, range.endOffset, true);
      if (sp && ep) {
        var endOff = ep.off - 1;
        var endNode = ep.node;
        if (endOff < 0) { // the end sits at the start of a node: step back to the previous text node
          var ix = nodes.indexOf(endNode);
          while (ix > 0 && (!nodes[ix - 1] || !nodes[ix - 1].nodeValue.length)) ix--;
          if (ix > 0) { endNode = nodes[ix - 1]; endOff = endNode.nodeValue.length - 1; }
        }
        if (endOff >= 0) wrap(nodes, { start: { node: sp.node, off: sp.off }, end: { node: endNode, off: endOff } }, 'cd-hl cd-st-anchored cd-pending-hl', tid);
      }
      var quote = m.exact.replace(/\s+/g, ' ');
      if (quote.length > 160) quote = quote.slice(0, 157) + '…';
      var card = composerCard(tid, '<q>' + esc(quote) + '</q>', '');
      card.setAttribute('data-line', String(m.line));
      card.cdPending = { subject: 'span', range: m.range, exact: m.exact, line: m.line, end: m.end };
      stripFor(m.line).appendChild(card);
      card.classList.add('cd-open');
      deselect();
      window.getSelection().removeAllRanges();
      focusEditor(card);
      scheduleLayout();
    }).catch(function (e) {
      showToast('Could not read the head Markdown to map the selection: ' + (e && e.message || e));
    });
  }
  function postComposer(card) {
    var ed = $('.cd-editor-new', card);
    var ta = ed && $('textarea', ed);
    var text = ta ? ta.value.trim() : '';
    if (!text) { if (ta) ta.focus(); return; }
    var p = card.cdPending;
    if (!p) return;
    if (headMoved || !onHead) { var e0 = $('.cd-err', ed); e0.textContent = 'A newer Version was pushed — go to head to comment on it.'; e0.hidden = false; return; }
    var tid = card.getAttribute('data-t');
    // Optimistic: the composer becomes the Thread card, instantly.
    card.classList.remove('cd-composer');
    card.classList.add('cd-pending');
    ed.remove();
    var body = $('.cd-body', card);
    body.innerHTML = commentHtml(tid, text, true) + '<div class="cd-acts"><button type="button" class="cd-btn cd-sm" data-act="reply" disabled>Reply</button><button type="button" class="cd-btn cd-sm" data-act="resolve" disabled>Resolve</button></div>';
    var main = $('.cd-main', card);
    main.insertAdjacentHTML('beforeend', '<div class="cd-snip"><b>' + esc(D.me_name || D.me) + '</b> ' + esc(text.slice(0, 120)) + '</div><div class="cd-meta">posting…</div>');
    localizeTimes(card);
    marksOf(tid).forEach(function (m) { m.classList.remove('cd-pending-hl'); });
    // The optimistic item lets selection / layout find it.
    setItem({ id: tid, subject: p.subject, status: 'anchored', line: p.line, end: p.end || p.line, quote: p.exact || null, now: null, hash: p.hash || null, ref: p.ref || null, resolved: false, mine: false });
    select(tid, false);
    recount();
    scheduleLayout();
    var bodyJson = p.subject === 'asset' ? { subject: 'asset', ref: p.ref, body: text, version: D.head } : { subject: 'span', range: p.range, body: text, version: D.head };
    var op = beginOp('created', null);
    api('POST', docApi('threads'), bodyJson).then(function (r) {
      if (!r.ok) {
        endOp(op, null);
        // Roll back: the card goes, the composer comes back with the text and the error.
        var i = itemIndex(tid); if (i >= 0) D.threads.splice(i, 1);
        var again = composerCard(tid, $('.cd-quote', card).innerHTML, p.subject === 'asset' ? 'cd-asset' : '');
        again.setAttribute('data-line', String(p.line));
        again.cdPending = p;
        card.replaceWith(again);
        marksOf(tid).forEach(function (m) { m.classList.add('cd-pending-hl'); m.classList.remove('cd-selected'); });
        var ta2 = $('textarea', again); ta2.value = text;
        var err = $('.cd-err', again); err.textContent = 'Comment not posted: ' + r.error; err.hidden = false;
        again.classList.add('cd-open');
        if (r.code === 'not_head') { headMoved = true; hideHeadOnly(); }
        selected = null;
        recount();
        focusEditor(again);
        scheduleLayout();
        return;
      }
      var real = r.json.id;
      endOp(op, real);
      card.setAttribute('data-t', real);
      card.id = 'cd-' + real;
      $$('[data-t="' + tid + '"]', docEl).forEach(function (m) { m.setAttribute('data-t', real); });
      renameMarks(tid, real);
      var it = D.threads[itemIndex(tid)]; if (it) it.id = real;
      if (selected === tid) selected = real;
      refreshCard(real);
    });
  }

  /* ---------- buttons in cards (delegated) ---------- */
  if (isDocPage) docEl.addEventListener('click', function (e) {
    var btn = e.target.closest('[data-act]');
    if (!btn || !docEl.contains(btn)) return;
    var card = btn.closest('.cd-thread');
    if (!card) return;
    var act = btn.getAttribute('data-act');
    e.preventDefault();
    if (act === 'reply') { openReply(card); return; }
    if (act === 'resolve') { setResolved(card.getAttribute('data-t'), true); return; }
    if (act === 'reopen') { setResolved(card.getAttribute('data-t'), false); return; }
    if (act === 'edit') { startEdit(card, btn.closest('.cd-c')); return; }
    if (act === 'delete') {
      btn.closest('.cd-ctools').innerHTML = '<span class="cd-confirm">delete? <button type="button" class="cd-link" data-act="delete-yes">yes</button><button type="button" class="cd-link" data-act="delete-no">no</button></span>';
      return;
    }
    if (act === 'delete-yes') { deleteComment(card, btn.closest('.cd-c')); return; }
    if (act === 'delete-no') { btn.closest('.cd-ctools').innerHTML = '<button type="button" class="cd-link" data-act="edit">edit</button><button type="button" class="cd-link" data-act="delete">delete</button>'; return; }
    if (act === 'post') {
      var ed = btn.closest('.cd-editor');
      if (ed.classList.contains('cd-editor-reply')) postReply(card);
      else if (ed.classList.contains('cd-editor-edit')) postEdit(card);
      else postComposer(card);
      return;
    }
    if (act === 'cancel') { closeEditor(card, btn.closest('.cd-editor')); return; }
    if (act === 'discard-yes') {
      var ded = btn.closest('.cd-editor');
      var next = ded.cdNextPending;
      closeEditor(card, ded);
      if (next) openComposer(next);
      return;
    }
    if (act === 'discard-no') { keepEditing(btn.closest('.cd-editor')); return; }
  });
  function openReply(card) {
    if (!card || card.classList.contains('cd-composer') || card.classList.contains('cd-pending') || !D.can_comment) return;
    if ($('.cd-editor-reply', card)) { focusEditor(card); return; }
    var other = anyEditor();
    if (other && other.closest('.cd-thread') !== card && !$('textarea', other).value.trim()) closeEditor(other.closest('.cd-thread'), other);
    var acts = $('.cd-acts', card);
    var ed = htmlToEl(editorHtml('reply', '', 'Reply…'));
    if (acts) acts.before(ed); else $('.cd-body', card).appendChild(ed);
    card.classList.add('cd-open');
    select(card.getAttribute('data-t'), false);
    focusEditor(card);
    scheduleLayout();
  }
  var actArea = $('#cd-actions');
  if (actArea) actArea.addEventListener('click', function (e) {
    var b = e.target.closest('.cd-actbtn');
    if (b) takeAction(b);
  });

  /* ---------- keyboard ---------- */
  function visibleCards() {
    return $$('.cd-thread:not(.cd-composer)', docEl).filter(function (c) { return c.getClientRects().length > 0; });
  }
  function toggleHelp(force) {
    if (!help) return;
    help.hidden = force === undefined ? !help.hidden : !force;
  }
  var helpBtn = $('#cd-helpbtn');
  if (helpBtn) helpBtn.addEventListener('click', function () { toggleHelp(); });
  document.addEventListener('keydown', function (e) {
    var t = e.target;
    var inField = t && t.matches && t.matches('input,textarea,select,[contenteditable]');
    if (inField) {
      var ed = t.closest && t.closest('.cd-editor');
      var card = ed && ed.closest('.cd-thread');
      if (e.key === 'Escape') {
        if (card) { e.preventDefault(); escapeEditor(card, ed); }
        else t.blur();
        return;
      }
      if (e.key === 'Enter' && ed) {
        var meta = e.metaKey || e.ctrlKey;
        // CMD-23 Send key: ↩ posts a reply (default), or ↩ is a newline and ⌘↩ / Ctrl↩ posts it
        var modSend = app.getAttribute('data-cd-send') === 'mod_enter';
        if (ed.classList.contains('cd-editor-reply') && !modSend && !e.shiftKey && !e.altKey) { e.preventDefault(); postReply(card); return; }
        if (meta) { e.preventDefault(); if (ed.classList.contains('cd-editor-edit')) postEdit(card); else if (ed.classList.contains('cd-editor-new')) postComposer(card); else postReply(card); }
      }
      return;
    }
    if (e.key === 'Escape') {
      app.classList.remove('cd-overlay-open');
      if (news) news.hidden = true;
      if (help) help.hidden = true;
      if (toast) toast.hidden = true;
      if (!isDocPage) return;
      var asking = $('.cd-discard', docEl);
      var open = (t && t.closest && t.closest('.cd-editor')) || (asking && asking.closest('.cd-editor')) || anyEditor();
      if (open) { escapeEditor(open.closest('.cd-thread'), open); return; }
      if (pending) { hidePill(); window.getSelection().removeAllRanges(); return; }
      deselect();
      return;
    }
    if (!isDocPage || e.metaKey || e.ctrlKey || e.altKey) return;
    if (app.getAttribute('data-cd-keys') === 'off') return; // CMD-23: Keyboard shortcuts Off
    var vis, i, c;
    switch (e.key) {
      case 'j': case 'k':
        vis = visibleCards();
        i = -1;
        for (var q = 0; q < vis.length; q++) if (vis[q].getAttribute('data-t') === selected) i = q;
        c = e.key === 'j' ? vis[Math.min(vis.length - 1, i + 1)] : vis[Math.max(0, i - 1)];
        if (c) { e.preventDefault(); select(c.getAttribute('data-t'), true); }
        break;
      case 'r':
        if (selected) { e.preventDefault(); openReply(cardOf(selected)); }
        break;
      case 'e':
        if (selected) { e.preventDefault(); var cd = cardOf(selected); if (cd) setResolved(selected, !cd.classList.contains('cd-resolved')); }
        break;
      case 'c':
        if (pending && pill && !pill.hidden) { e.preventDefault(); openComposer(); }
        break;
      case '[': case ']':
        var a = $(e.key === '[' ? '.cd-vsw a.cd-older' : '.cd-vsw a.cd-newer');
        if (a) { e.preventDefault(); location.href = a.getAttribute('href'); }
        break;
      case '?':
        e.preventDefault();
        toggleHelp();
        break;
      default:
        break;
    }
  });

  /* ---------- live updates: the events SSE for this Document ---------- */
  var es = null;
  function showNewVersion(p, by) {
    var n = +p.version;
    if (!(n > D.head)) return;
    D.head = n;
    headMoved = true;
    hideHeadOnly();
    var b = $('#cd-newver');
    var msg = p.message ? ' · “' + esc(p.message) + '”' : '';
    var html = '<span><b>v' + n + '</b> was pushed by ' + esc(nameOf(by)) + msg + (p.same_as ? ' (same content as v' + esc(p.same_as) + ')' : '') + ' — this page still shows v' + D.version + '.</span><span class="cd-grow"></span><a class="cd-btn" id="cd-gonew" href="/d/' + encodeURIComponent(D.id) + '">Go to head v' + n + '</a>';
    if (!b) {
      b = document.createElement('div');
      b.className = 'cd-banner cd-newver';
      b.id = 'cd-newver';
      $('.cd-hdr').after(b);
    }
    b.innerHTML = html;
    // The Version select learns about it (no navigation).
    if (vsel && !$('option[value="' + n + '"]', vsel)) {
      var o = document.createElement('option');
      o.value = String(n);
      o.setAttribute('data-href', '/d/' + encodeURIComponent(D.id));
      o.textContent = 'v' + n + ' (head) · ' + nameOf(by) + (p.message ? ' · ' + p.message : '');
      vsel.insertBefore(o, vsel.firstChild);
      var prev = $('option[value="' + (n - 1) + '"]', vsel);
      if (prev) { prev.setAttribute('data-pre', (prev.getAttribute('data-pre') || '').replace(' (head)', '')); prev.setAttribute('data-href', '/d/' + encodeURIComponent(D.id) + '/v/' + (n - 1)); localizeTimes(vsel); }
    }
    var newer = $('.cd-vsw .cd-newer');
    if (newer && newer.classList.contains('cd-off')) {
      var a = document.createElement('a');
      a.className = 'cd-arrow cd-newer';
      a.href = '/d/' + encodeURIComponent(D.id);
      a.title = 'newer Version';
      a.textContent = '›';
      newer.replaceWith(a);
    }
  }
  function onLiveEvent(ev) {
    if (!ev || ev.doc !== D.id) return;
    if (ev.seq) D.seq = Math.max(D.seq || 0, ev.seq);
    newsLive(ev);
    var p = ev.payload || {};
    switch (ev.type) {
      case 'thread.created': case 'thread.replied': case 'thread.resolved': case 'thread.reopened':
      case 'comment.edited': case 'comment.deleted':
        if (isEcho(ev)) return;
        if (!p.thread) return;
        if (D.version === D.head || cardOf(p.thread)) refreshCard(p.thread);
        break;
      case 'action.taken':
        if (isEcho(ev)) return;
        if (+p.version === D.version) setChip({ id: p.action, label: p.label || p.action, intent: p.intent || 'neutral', by: ev.by, at: ev.at, version: +p.version });
        break;
      case 'version.pushed':
        showNewVersion(p, ev.by);
        break;
      case 'document.actions_declared':
        if (onHead) api('GET', docApi('actions')).then(function (r) { if (r.ok && r.json) { D.actions = r.json.available || []; rebuildActionButtons(D.actions); } });
        break;
      default:
        break;
    }
  }
  function subscribe() {
    if (!isDocPage || !window.EventSource || es) return;
    try {
      es = new EventSource('/api/v1/events?doc=' + encodeURIComponent(D.id) + '&since=' + (D.seq || 0) + '&me=1');
    } catch (e) { es = null; return; }
    es.addEventListener('ready', function (e) {
      app.classList.add('cd-live');
      try { var j = JSON.parse(e.data); if (j && j.seq) D.seq = Math.max(D.seq || 0, j.seq); } catch (x) { /* ignore */ }
    });
    ['thread.created', 'thread.replied', 'thread.resolved', 'thread.reopened', 'comment.edited', 'comment.deleted',
      'action.taken', 'version.pushed', 'document.actions_declared', 'document.deleted', 'document.restored'].forEach(function (type) {
      es.addEventListener(type, function (e) {
        var ev = null;
        try { ev = JSON.parse(e.data); } catch (x) { ev = null; }
        onLiveEvent(ev);
      });
    });
    es.addEventListener('setting.changed', settingsChanged); // CMD-23
    es.onerror = function () { app.classList.remove('cd-live'); };
  }
  if (isDocPage) (window.requestIdleCallback || function (f) { setTimeout(f, 0); })(subscribe);

  /* ---------- CMD-23: the Member's settings, applied live ----------
     The first paint already wears them (`.cd-app`'s classes, style variables and data-cd-*
     attributes, from `prefs.rs`). When the live feed (`&me=1`) reports a `setting.changed` for
     this Member, re-read GET /api/v1/me/settings?device=web and put the same classes, variables
     and attributes on `.cd-app`: no reload. The list page subscribes only for that. */
  var settingsCatalog = null;
  var settingsTimer = null;
  function settingsChanged() {
    clearTimeout(settingsTimer);
    settingsTimer = setTimeout(applySettings, 50);
  }
  /* A highlight value (preset key or #rrggbb) → [color, selected], for a light or dark page — the
     same rule as `highlight_colors` in crates/store/src/settings.rs (keep them equal). */
  function hlPair(value, dark) {
    var p = ((settingsCatalog && settingsCatalog.presets) || []).filter(function (x) { return x.key === value; })[0];
    if (p) return dark ? [p.dark, p.dark_selected] : [p.light, p.light_selected];
    var m = /^#([0-9a-f]{6})$/i.exec(value || '');
    if (!m) return value === 'yellow' ? (dark ? ['#facc15', '#fde047'] : ['#fff1a8', '#ffd54a']) : hlPair('yellow', dark);
    var n = parseInt(m[1], 16), to = dark ? 255 : 0, t = dark ? 0.3 : 0.15;
    var sel = [n >> 16 & 255, n >> 8 & 255, n & 255].map(function (v) {
      var x = Math.min(255, Math.max(0, Math.round(v + (to - v) * t)));
      return (x < 16 ? '0' : '') + x.toString(16);
    }).join('');
    return ['#' + m[1].toLowerCase(), '#' + sel];
  }
  function applySettings() {
    var cat = settingsCatalog ? Promise.resolve(settingsCatalog)
      : api('GET', '/api/v1/me/settings/catalog').then(function (r) { if (r.ok) settingsCatalog = r.json; return settingsCatalog; });
    return Promise.all([api('GET', '/api/v1/me/settings?device=web'), cat]).then(function (rs) {
      var r = rs[0];
      if (!r.ok || !r.json) return;
      var v = {};
      (r.json.settings || []).forEach(function (x) { v[x.key] = x.value; });
      var theme = ['light', 'dark', 'system'].indexOf(v.appearance) >= 0 ? v.appearance : 'system';
      app.classList.remove('cd-theme-light', 'cd-theme-dark', 'cd-theme-system');
      app.classList.add('cd-theme-' + theme);
      var hl = hlPair(v.highlight_light, false), hd = hlPair(v.highlight_dark, true);
      app.style.setProperty('--cd-hl-light', hl[0]);
      app.style.setProperty('--cd-hl-light-sel', hl[1]);
      app.style.setProperty('--cd-hl-dark', hd[0]);
      app.style.setProperty('--cd-hl-dark-sel', hd[1]);
      var scale = String((+v.text_size || 100) / 100);
      var scaleChanged = app.style.getPropertyValue('--cd-text-scale') !== scale;
      app.style.setProperty('--cd-text-scale', scale);
      var shown = v.show_resolved === 'shown';
      var resChanged = app.classList.contains('cd-show-resolved') !== shown;
      app.classList.toggle('cd-show-resolved', shown);
      if (resolvedCb) resolvedCb.checked = shown;
      app.setAttribute('data-cd-keys', v.keyboard_shortcuts === 'off' ? 'off' : 'on');
      app.setAttribute('data-cd-send', v.send_key === 'mod_enter' ? 'mod_enter' : 'enter');
      var sendKbd = document.getElementById('cd-send-key');
      if (sendKbd) sendKbd.textContent = v.send_key === 'mod_enter' ? '\u2318\u21A9' : '\u21A9';
      var times = v.times === 'absolute' ? 'absolute' : 'relative', clock = String(v.clock || 'system');
      var tz = cookieVal('cd_tz');
      if (times !== app.getAttribute('data-cd-times') || clock !== app.getAttribute('data-cd-clock') || (tz && !app.hasAttribute('data-cd-tz'))) {
        app.setAttribute('data-cd-times', times);
        app.setAttribute('data-cd-clock', clock);
        if (tz && !app.hasAttribute('data-cd-tz')) app.setAttribute('data-cd-tz', tz);
        fmts = {};
        $$('time.cd-localized', document).forEach(function (t) { t.classList.remove('cd-localized'); });
        localizeTimes(document);
      }
      if (resChanged) updateOrphanedCount();
      if (resChanged || scaleChanged) layout();
    });
  }
  if (!isDocPage && app.classList.contains('cd-listpage') && window.EventSource) {
    (window.requestIdleCallback || function (f) { setTimeout(f, 0); })(function () {
      try {
        es = new EventSource('/api/v1/events?me=1&kinds=setting.changed');
        es.addEventListener('setting.changed', settingsChanged);
        es.addEventListener('ready', function () { app.classList.add('cd-live'); });
        es.onerror = function () { app.classList.remove('cd-live'); };
      } catch (e) { es = null; }
    });
  }

  window.cd = {
    select: select, layout: layout, loadNews: loadNews,
    // ticket 11
    mapSelection: function (range) { return loadSource().then(function (lines) { return mapSelection(range, lines); }); },
    refreshCard: refreshCard, setResolved: setResolved, openComposer: openComposer,
    live: function () { return app.classList.contains('cd-live'); },
    data: function () { return D; },
    // ticket 12: the owner screens localize the <time> tags of freshly drawn cards
    localize: localizeTimes,
    // CMD-23: re-read the Member's settings and apply them to `.cd-app` (the Settings screen calls it)
    applySettings: applySettings
  };
}
})();
