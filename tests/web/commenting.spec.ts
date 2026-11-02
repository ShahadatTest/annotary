// Commenting from the browser (ticket 11): the selection pill → composer → a Thread posted
// optimistically and confirmed by the server; rollback with a visible error on a server / network
// failure; whole-Asset Threads from inline media; reply / resolve / reopen / edit / delete from an
// older-Version view; the 100k body cap shown inline; the Action buttons (head only, by intent,
// `[]` hides, rollback); and the selection → source mapping checked against the CLI.
// Every test pushes its own Document (the shared `ra-plan` seed is never mutated).
import { test, expect, type Page } from '@playwright/test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { harness, cli, seedDoc, pushVersion, rangeOf, REPO_ROOT } from './harness';
import { openDoc, selectText, waitLive } from './actions';

const h = () => harness();

const PLAN = `# Launch plan

Ship a local server + CLI + web viewer that humans and agents can use.

## Order of work

1. Sidecar format and **anchor engine** — the core, testable in isolation.
2. Local server with content-addressed storage.

We ship the CLI before the web UI so agents can start using it early.

See [API notes](api-notes.md) for the endpoint list & more.
`;

async function postComposer(page: Page, text: string) {
  const ta = page.locator('.cd-composer textarea');
  await expect(ta).toBeFocused();
  await ta.fill(text);
  await page.keyboard.press('Control+Enter');
}

test.describe('selection pill and composer', () => {
  // spec: S11-01, S11-05, S11-08
  test('select text → pill → composer where the Thread will live → card before the server answers; the range matches the CLI', async ({ page }) => {
    const doc = seedDoc('web-c1', PLAN);
    await openDoc(page, doc.id);
    await expect(page.locator('#cd-pill')).toBeHidden();
    // No per-block "+" anywhere: the rendered Document carries no controls of its own.
    expect(await page.evaluate(() => [...document.querySelectorAll('.cd-doc button, .cd-doc [role=button]')].filter((b) => !b.closest('.cd-strip')).length)).toBe(0);
    await selectText(page, '.cd-doc p[data-l="10-10"]', 'CLI before the web UI');
    const pill = page.locator('#cd-pill');
    await expect(pill).toBeVisible();
    await expect(pill).toContainText('Comment');
    // The pill sits by the selection (above the paragraph).
    const pb = (await pill.boundingBox())!;
    const para = (await page.locator('.cd-doc p[data-l="10-10"]').boundingBox())!;
    expect(Math.abs(pb.y + pb.height - para.y)).toBeLessThan(60);
    await pill.click();
    const composer = page.locator('.cd-composer');
    await expect(composer).toBeVisible();
    // Where the Thread will live: a strip for line 10, a margin card next to the paragraph (layout A).
    const strip = page.locator('.cd-strip[data-line="10"]');
    await expect(strip).toHaveCount(1);
    expect(await strip.evaluate((e) => getComputedStyle(e).position)).toBe('absolute');
    const sb = (await strip.boundingBox())!;
    expect(Math.abs(sb.y - para.y)).toBeLessThan(40);
    await expect(composer.locator('.cd-quote q')).toHaveText('CLI before the web UI');
    await expect(page.locator('mark.cd-hl.cd-pending-hl')).toHaveText('CLI before the web UI');
    // Hold the POST so the optimistic card is observable before the server confirms.
    let release: () => void = () => {};
    const held = new Promise<void>((r) => { release = r; });
    let posted = false;
    await page.route('**/api/v1/docs/*/threads', async (route) => {
      if (route.request().method() !== 'POST') { await route.continue(); return; }
      posted = true;
      await held;
      await route.continue();
    });
    await postComposer(page, 'Does this order still hold? @scribe');
    const card = page.locator('.cd-thread.cd-pending');
    await expect(card).toBeVisible();
    await expect(card.locator('.cd-cbody')).toContainText('Does this order still hold?');
    await expect(card.locator('.cd-member')).toHaveText('Rowan Vale');
    expect(posted).toBe(true);
    await expect(page.locator('.cd-composer')).toHaveCount(0);
    // Counts updated optimistically.
    await expect(page.locator('.cd-status')).toContainText('1 open');
    release();
    // Confirmed: the real id, the server-rendered card, the highlight renamed, no pending state.
    const t1 = page.locator('.cd-thread[data-t="t1"]');
    await expect(t1).toBeVisible();
    await expect(page.locator('.cd-thread.cd-pending')).toHaveCount(0);
    await expect(t1.locator('.cd-c[data-c="t1"] .cd-cbody')).toContainText('Does this order still hold?');
    await expect(t1.locator('.cd-quote q').first()).toHaveText('CLI before the web UI');
    await expect(t1).toHaveClass(/cd-selected/);
    await expect(page.locator('mark.cd-hl[data-t="t1"]')).toHaveText('CLI before the web UI');
    await expect(page.locator('mark.cd-hl.cd-pending-hl')).toHaveCount(0);
    await expect(t1.locator('.cd-acts [data-act="reply"]')).toBeVisible();
    // The mapping matches the CLI: `thread show` prints the same range and quote.
    const shown = JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0];
    expect(shown.anchor.original.range).toBe(rangeOf(PLAN, 'CLI before the web UI'));
    expect(shown.anchor.original.exact).toBe('CLI before the web UI');
    expect(shown.body).toBe('Does this order still hold? @scribe');
    expect(shown.mentions).toEqual(['scribe']);
    // Selections over inline syntax map to the source too: the bold run inside the list item —
    // the delimiters the selection covered on one side are balanced (`**anchor engine**`).
    await selectText(page, '.cd-doc li[data-l="7-7"]', 'anchor engine — the core');
    await pill.click();
    await expect(page.locator('.cd-composer .cd-quote q')).toHaveText('**anchor engine** — the core');
    await postComposer(page, 'second');
    await expect(page.locator('.cd-thread[data-t="t2"]:not(.cd-pending)')).toBeVisible();
    const t2 = JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't2', '--json']))[0];
    expect(t2.anchor.original.range).toBe(rangeOf(PLAN, '**anchor engine** — the core'));
    expect(t2.anchor.original.exact).toBe('**anchor engine** — the core');
    // A selection across the link text and the text after it (the link target is skipped).
    await selectText(page, '.cd-doc p[data-l="12-12"]', 'notes for the endpoint list & more');
    await pill.click();
    await postComposer(page, 'third');
    await expect(page.locator('.cd-thread[data-t="t3"]:not(.cd-pending)')).toBeVisible();
    const t3 = JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't3', '--json']))[0];
    expect(t3.anchor.original.range).toBe(rangeOf(PLAN, 'notes](api-notes.md) for the endpoint list & more'));
    expect(t3.anchor.original.exact).toBe('notes](api-notes.md) for the endpoint list & more');
    // The pill never shows on an older Version, nor on a Document that is not head.
    pushVersion(doc, PLAN + '\nMore.\n');
    await openDoc(page, doc.id, { v: 1 });
    await expect(page.locator('#cd-pill')).toHaveCount(0);
    await selectText(page, '.cd-doc p[data-l="10-10"]', 'CLI before');
    await expect(page.locator('#cd-pill')).toHaveCount(0);
    await expect(page.locator('.cd-composer')).toHaveCount(0);
  });

  // spec: S27-01
  test('opening another composer over typed text asks first: keep keeps it, discard opens the new one; an empty composer is replaced at once', async ({ page }) => {
    const png = fs.readFileSync(path.join(REPO_ROOT, 'tests', 'e2e', '48-reanchor-seed', 'seed.in', 'img', 'architecture.png'));
    const doc = seedDoc('web-c12', PLAN + '\n![architecture](img/architecture.png)\n', { 'img/architecture.png': png });
    await openDoc(page, doc.id);
    const pill = page.locator('#cd-pill');
    const composer = page.locator('.cd-composer');
    const ta = composer.locator('textarea');
    const ask = page.locator('.cd-discard');
    const pendingHl = page.locator('.cd-pending-hl');
    // A composer with typed text.
    await selectText(page, '.cd-doc p[data-l="10-10"]', 'CLI before the web UI');
    await pill.click();
    await expect(ta).toBeFocused();
    await ta.pressSequentially('first draft');
    // A second span → pill: the first composer and its text stay, and it asks.
    await selectText(page, '.cd-doc p[data-l="3-3"]', 'local server');
    await expect(pill).toBeVisible();
    await pill.click();
    await expect(composer).toHaveCount(1);
    await expect(composer.locator('.cd-quote q')).toHaveText('CLI before the web UI');
    await expect(ta).toHaveValue('first draft');
    await expect(composer.locator('.cd-discard')).toBeVisible();
    await expect(ask).toContainText('Discard what you typed?');
    await expect(pill).toBeHidden();
    await expect(pendingHl).toHaveText('CLI before the web UI');
    // keep: the confirm goes, the text stays, the cursor is back in it, no second composer.
    await ask.locator('[data-act="discard-no"]').click();
    await expect(ask).toHaveCount(0);
    await expect(ta).toBeFocused();
    await expect(ta).toHaveValue('first draft');
    await expect(composer).toHaveCount(1);
    await expect(composer.locator('.cd-quote q')).toHaveText('CLI before the web UI');
    // Again, by keyboard `c`; esc while it asks = keep, and drops the waiting target.
    await ta.blur();
    await selectText(page, '.cd-doc p[data-l="3-3"]', 'local server');
    await expect(pill).toBeVisible();
    await page.keyboard.press('c');
    await expect(ask).toBeVisible();
    await expect(ta).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(ask).toHaveCount(0);
    await expect(ta).toHaveValue('first draft');
    // esc first, then a second composer is asked for: discard opens it on the new target.
    await page.keyboard.press('Escape');
    await expect(ask).toBeVisible();
    await ta.blur();
    await selectText(page, '.cd-doc p[data-l="3-3"]', 'local server');
    await expect(pill).toBeVisible();
    await page.keyboard.press('c');
    await expect(ask).toHaveCount(1);
    await expect(ta).toHaveValue('first draft');
    await ask.locator('[data-act="discard-yes"]').click();
    await expect(ask).toHaveCount(0);
    await expect(composer).toHaveCount(1);
    await expect(composer.locator('.cd-quote q')).toHaveText('local server');
    await expect(ta).toBeFocused();
    await expect(ta).toHaveValue('');
    await expect(pendingHl).toHaveCount(1);
    await expect(pendingHl).toHaveText('local server');
    await expect(page.locator('.cd-strip[data-line="10"]')).toHaveCount(0);
    // An empty composer is replaced at once, without asking.
    await selectText(page, '.cd-doc p[data-l="10-10"]', 'CLI before the web UI');
    await expect(pill).toBeVisible();
    await pill.click();
    await expect(composer).toHaveCount(1);
    await expect(composer.locator('.cd-quote q')).toHaveText('CLI before the web UI');
    await expect(ask).toHaveCount(0);
    await expect(ta).toBeFocused();
    await expect(pendingHl).toHaveText('CLI before the web UI');
    // Cancel while it asks closes the composer and opens nothing.
    await ta.pressSequentially('second draft');
    await selectText(page, '.cd-doc p[data-l="3-3"]', 'local server');
    await pill.click();
    await expect(ask).toBeVisible();
    await composer.locator('[data-act="cancel"]').click();
    await expect(composer).toHaveCount(0);
    await expect(pendingHl).toHaveCount(0);
    // Posting while it asks posts the first; the waiting target never opens.
    await selectText(page, '.cd-doc p[data-l="10-10"]', 'CLI before the web UI');
    await pill.click();
    await expect(ta).toBeFocused();
    await ta.pressSequentially('kept and posted');
    await selectText(page, '.cd-doc p[data-l="3-3"]', 'local server');
    await pill.click();
    await expect(ask).toBeVisible();
    await expect(ta).toBeFocused();
    await page.keyboard.press('Control+Enter');
    const card = page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)');
    await expect(card.locator('.cd-cbody')).toContainText('kept and posted');
    await expect(composer).toHaveCount(0);
    await expect(ask).toHaveCount(0);
    await expect(pendingHl).toHaveCount(0);
    // An inline Asset asked for over typed text: the same confirm; discard opens the Asset composer.
    // (a paragraph the posted Thread's highlight is not in, so its re-draw cannot move the selection)
    await selectText(page, '.cd-doc p[data-l="12-12"]', 'endpoint list');
    await expect(pill).toBeVisible();
    await pill.click();
    await expect(ta).toBeFocused();
    await ta.pressSequentially('third draft');
    await page.locator('.cd-doc img[src^="/a/"]').click();
    await expect(pill).toBeVisible();
    await pill.click();
    await expect(ask).toBeVisible();
    await expect(ta).toHaveValue('third draft');
    await ask.locator('[data-act="discard-yes"]').click();
    await expect(page.locator('.cd-composer.cd-asset .cd-quote q')).toHaveText('img/architecture.png');
    await expect(composer).toHaveCount(1);
    await expect(ta).toBeFocused();
    await expect(ta).toHaveValue('');
    await expect(page.locator('.cd-doc img.cd-asset-hl.cd-pending-hl')).toHaveCount(1);
    await expect(page.locator('mark.cd-pending-hl')).toHaveCount(0);
  });

  // spec: S11-08
  test('a whole list item selected by triple-click or a drag past its end maps to the item, not the next block', async ({ page }) => {
    const MD = `# Why\n\n- First item.\n- **Decisions are recorded.** Approve on a Version.\n\n## How it works\n\nNext.\n`;
    const doc = seedDoc('web-c-li', MD);
    await openDoc(page, doc.id);
    const li = page.locator('.cd-doc li[data-l="4-4"]');
    // A mouse drag from the item's first word to just inside the next block (the h2) ends there.
    const box = (await li.boundingBox())!;
    const h2 = (await page.locator('.cd-doc h2').boundingBox())!;
    await page.mouse.move(box.x + 1, box.y + 8);
    await page.mouse.down();
    await page.mouse.move(h2.x + 1, h2.y + 2, { steps: 10 });
    await page.mouse.up();
    await expect(page.locator('#cd-pill')).toBeVisible();
    await page.locator('#cd-pill').click();
    await expect(page.locator('#cd-toast')).toBeHidden();
    await expect(page.locator('.cd-composer .cd-quote q')).toContainText('Decisions are recorded.');
    await page.locator('.cd-composer textarea').press('Escape');
    await expect(page.locator('.cd-composer')).toHaveCount(0);
    // A triple-click selects to offset 0 of the next block (the h2).
    await li.click({ clickCount: 3 });
    expect(await page.evaluate(() => (window.getSelection()!.getRangeAt(0).endContainer as Element).tagName)).toBe('H2');
    await expect(page.locator('#cd-pill')).toBeVisible();
    await page.keyboard.press('c');
    await expect(page.locator('#cd-toast')).toBeHidden();
    await expect(page.locator('.cd-composer .cd-quote q')).toHaveText('**Decisions are recorded.** Approve on a Version.');
    await postComposer(page, 'whole item');
    await expect(page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)')).toBeVisible();
    const t1 = JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0];
    expect(t1.anchor.original.range).toBe(rangeOf(MD, '**Decisions are recorded.** Approve on a Version.'));
  });

  // spec: S11-08
  test('a list item that already has a card: triple-click still comments on the item; a drag into the card is refused', async ({ page }) => {
    const MD = `# Why\n\n- First item.\n- **Decisions are recorded.** Approve on a Version.\n\n## How it works\n\nNext.\n`;
    const doc = seedDoc('web-c-li-card', MD);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'Approve on a Version'), '-m', 'An existing card in this item.']);
    await openDoc(page, doc.id, { width: 600 }); // narrow: the card sits inline, inside the li
    const li = page.locator('.cd-doc li[data-l="4-4"]');
    await expect(li.locator('.cd-strip .cd-thread')).toHaveCount(1);
    // Triple-click the item's text: the browser ends the selection at offset 0 inside the card
    // (no card text selected), so the item is what gets commented.
    await li.locator('strong').click({ clickCount: 3 });
    const pill = page.locator('#cd-pill');
    expect(await page.evaluate(() => window.getSelection()!.getRangeAt(0).cloneContents().querySelectorAll('.cd-strip').length)).toBe(1);
    await expect(pill).toBeVisible();
    await pill.click();
    await expect(page.locator('#cd-toast')).toBeHidden();
    await expect(page.locator('.cd-composer .cd-quote q')).toHaveText('**Decisions are recorded.** Approve on a Version.');
    await page.locator('.cd-composer textarea').press('Escape');
    await expect(page.locator('.cd-composer')).toHaveCount(0);
    // From the item's text into the card's body text: never commentable (the card text is not
    // trimmed away to make the selection look like Document text).
    await page.evaluate(() => {
      const li = document.querySelector('.cd-doc li[data-l="4-4"]')!;
      const start = li.querySelector('strong')!.firstChild!;
      const body = li.querySelector('.cd-strip .cd-cbody')!;
      const walker = document.createTreeWalker(body, NodeFilter.SHOW_TEXT);
      const end = walker.nextNode() as Text;
      const r = document.createRange();
      r.setStart(start, 0);
      r.setEnd(end, 5);
      const sel = window.getSelection()!;
      sel.removeAllRanges();
      sel.addRange(r);
    });
    await page.waitForTimeout(150); // the page reads the selection on a timer after selectionchange
    await expect(pill).toBeHidden();
  });

  // spec: S11-05
  test('rollback with a visible error: an injected server error and a network failure', async ({ page }) => {
    const doc = seedDoc('web-c2', PLAN);
    await openDoc(page, doc.id);
    await selectText(page, '.cd-doc p[data-l="3-3"]', 'humans and agents');
    await page.locator('#cd-pill').click();
    await page.route('**/api/v1/docs/*/threads', async (route) => {
      if (route.request().method() !== 'POST') { await route.continue(); return; }
      await route.fulfill({ status: 500, contentType: 'application/json', body: JSON.stringify({ error: 'boom', message: 'injected failure', hint: 'try again' }) });
    });
    await postComposer(page, 'will not land');
    // The card appeared (optimistic) and is rolled back: the composer is back with the text and the error.
    const composer = page.locator('.cd-composer');
    await expect(composer).toBeVisible();
    await expect(composer.locator('.cd-err')).toBeVisible();
    await expect(composer.locator('.cd-err')).toContainText('injected failure');
    await expect(composer.locator('textarea')).toHaveValue('will not land');
    await expect(page.locator('.cd-thread:not(.cd-composer)')).toHaveCount(0);
    await expect(page.locator('.cd-status')).toContainText('0 open');
    expect(cli(h().owner, ['thread', 'list', doc.id]).trim()).toBe('');
    // Network failure: same rollback, a different message.
    await page.unroute('**/api/v1/docs/*/threads');
    await page.route('**/api/v1/docs/*/threads', async (route) => {
      if (route.request().method() !== 'POST') { await route.continue(); return; }
      await route.abort('connectionrefused');
    });
    await page.keyboard.press('Control+Enter');
    await expect(composer.locator('.cd-err')).toContainText('could not reach the server');
    await expect(page.locator('.cd-thread:not(.cd-composer)')).toHaveCount(0);
    // Let it through: the same text posts fine.
    await page.unroute('**/api/v1/docs/*/threads');
    await page.keyboard.press('Control+Enter');
    await expect(page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)')).toBeVisible();
    await expect(page.locator('.cd-composer')).toHaveCount(0);
  });

  // spec: S11-02
  test('inline media: click → pill → a whole-Asset Thread; Asset links stay plain text', async ({ page }) => {
    const png = fs.readFileSync(path.join(REPO_ROOT, 'tests', 'e2e', '48-reanchor-seed', 'seed.in', 'img', 'architecture.png'));
    const md = `# Media\n\n![architecture](img/architecture.png)\n\nSee [API notes](api-notes.md) for the list.\n`;
    const doc = seedDoc('web-c3', md, { 'img/architecture.png': png, 'api-notes.md': '# notes\n' });
    await openDoc(page, doc.id);
    const img = page.locator('.cd-doc img[src^="/a/"]');
    await img.click();
    const pill = page.locator('#cd-pill');
    await expect(pill).toBeVisible();
    await pill.click();
    const composer = page.locator('.cd-composer');
    await expect(composer).toBeVisible();
    await expect(composer.locator('.cd-quote')).toContainText('img/architecture.png');
    await expect(img).toHaveClass(/cd-asset-hl/);
    await postComposer(page, 'Diagram is out of date.');
    const t1 = page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)');
    await expect(t1).toBeVisible();
    await expect(t1).toHaveClass(/cd-asset/);
    await expect(page.locator('img.cd-asset-hl[data-t="t1"]')).toHaveCount(1);
    const shown = JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0];
    expect(shown.subject).toBe('asset');
    expect(shown.anchor.original.ref).toBe('img/architecture.png');
    expect(shown.anchor.original.line).toBe(3);
    // Selecting the image (drag over it) offers the same pill.
    await page.evaluate(() => {
      const el = document.querySelector('.cd-doc img[src^="/a/"]')!;
      const r = document.createRange();
      r.selectNode(el);
      const s = window.getSelection()!;
      s.removeAllRanges();
      s.addRange(r);
    });
    await expect(pill).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(pill).toBeHidden();
    // The Asset link is ordinary text: it opens in a new tab, and selecting its text gives a span Thread.
    const link = page.locator('.cd-doc a[href^="/a/"]');
    await expect(link).toHaveText('API notes');
    await expect(link).toHaveAttribute('target', '_blank');
    await selectText(page, '.cd-doc p[data-l="5-5"]', 'API notes');
    await pill.click();
    await expect(composer.locator('.cd-quote q')).toHaveText('API notes');
    await postComposer(page, 'on the link text');
    await expect(page.locator('.cd-thread[data-t="t2"]:not(.cd-pending)')).toBeVisible();
    const t2 = JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't2', '--json']))[0];
    expect(t2.subject).toBe('span');
    expect(t2.anchor.original.range).toBe(rangeOf(md, 'API notes'));
  });
});

const REPEATS = `# Repeats

- Ship the CLI first.
- Ship the CLI first.

Later the plan says ship it, then the plan says wait.
`;

test.describe('highlights after a reload', () => {
  // spec: S10-11
  test('the highlight lands on the occurrence the Thread is anchored to, not an earlier identical one', async ({ page }) => {
    const doc = seedDoc('web-c-repeat', REPEATS);
    await openDoc(page, doc.id);
    // From the browser: the text of the SECOND list item (the same words as the first).
    await selectText(page, '.cd-doc li[data-l="4-4"]', 'Ship the CLI first');
    await page.locator('#cd-pill').click();
    await postComposer(page, 'on the second item');
    await expect(page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)')).toBeVisible();
    expect(JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0].anchor.original.range).toBe('4:2-4:20');
    // Twice on one line: a Thread on the second "the plan says".
    const col = REPEATS.split('\n')[5].lastIndexOf('the plan says');
    cli(h().agent, ['thread', 'new', doc.id, `6:${col}-6:${col + 13}`, '-m', 'the second one']);
    await page.reload();
    await page.waitForSelector('.cd-app.cd-ready');
    const m1 = page.locator('mark.cd-hl[data-t="t1"]');
    await expect(m1).toHaveText('Ship the CLI first');
    expect(await m1.evaluate((m) => m.closest('[data-l]')!.getAttribute('data-l'))).toBe('4-4');
    await expect(page.locator('.cd-doc li[data-l="3-3"] mark')).toHaveCount(0);
    const m2 = page.locator('mark.cd-hl[data-t="t2"]');
    await expect(m2).toHaveText('the plan says');
    const before = await m2.evaluate((m) => {
      const r = document.createRange();
      r.setStart(m.closest('p')!, 0);
      r.setEndBefore(m);
      return r.toString();
    });
    expect(before).toBe('Later the plan says ship it, then ');
  });
});

test.describe('replies, resolve, edit, delete', () => {
  // spec: S11-03, S11-05
  test('from an older-Version view: reply (+resolve), reopen, edit own Comment, delete (tombstone); new Threads head-only; the body cap shown inline', async ({ page }) => {
    const doc = seedDoc('web-c4', PLAN);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(PLAN, 'humans and agents'), '-m', 'Equal standing — really? @rowan']);
    cli(h().agent, ['thread', 'new', doc.id, '--document', '-m', 'Overall shape is fine.']);
    pushVersion(doc, PLAN.replace('before the web UI', 'well before the web UI'), 'v2');
    await openDoc(page, doc.id, { v: 1 });
    await expect(page.locator('#cd-banner')).toBeVisible();
    await expect(page.locator('#cd-pill')).toHaveCount(0);
    await expect(page.locator('#cd-actbtns')).toHaveCount(0);
    const t1 = page.locator('.cd-thread[data-t="t1"]');
    await expect(t1).toHaveClass(/cd-st-anchored/);
    // Reply with "resolve": instant, then confirmed, stamped with the head Version (v2).
    await t1.locator('[data-act="reply"]').click();
    const ed = t1.locator('.cd-editor-reply');
    await expect(ed.locator('textarea')).toBeFocused();
    await ed.locator('textarea').fill('Yes — see the glossary.');
    await ed.locator('.cd-resolve-too').check();
    await page.keyboard.press('Enter');
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-cbody')).toContainText('Yes — see the glossary.');
    await expect(t1).toHaveClass(/cd-resolved/);
    await expect(t1.locator('.cd-resline')).toContainText('resolved by Rowan Vale');
    // Resolved Threads hide behind the toggle; the counts follow.
    await expect(page.locator('.cd-status')).toContainText('1 open');
    await expect(page.locator('.cd-res-n')).toHaveText('1');
    await expect.poll(() => JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0].replies.length).toBe(1);
    const shown = JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0];
    expect(shown.replies[0].created.version).toBe(2);
    expect(shown.resolved.version).toBe(2);
    // The card keeps the v1 placement label (the page is a time-slice) but the live conversation.
    await expect(t1).toHaveClass(/cd-st-anchored/);
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-mono')).toHaveText('v2');
    // Reopen from the card (the toggle shows resolved Threads).
    await page.locator('#cd-resolved').check();
    await t1.locator('[data-act="reopen"]').click();
    await expect(t1).not.toHaveClass(/cd-resolved/);
    await expect.poll(() => JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0].resolved ?? null).toBeNull();
    // Edit my own reply; the agent's root shows edit/delete too (I am an owner).
    const reply = t1.locator('.cd-c[data-c="t1.1"]');
    await reply.locator('[data-act="edit"]').click();
    const edEd = reply.locator('.cd-editor-edit textarea');
    await expect(edEd).toHaveValue('Yes — see the glossary.');
    await edEd.fill('Yes — see the **glossary**.');
    await page.keyboard.press('Control+Enter');
    await expect(reply.locator('.cd-cbody strong')).toHaveText('glossary');
    await expect(reply.locator('.cd-who')).toContainText('edited');
    expect(JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0].replies[0].body).toBe('Yes — see the **glossary**.');
    // Delete the reply: an inline confirm, then a tombstone.
    await reply.locator('[data-act="delete"]').click();
    await reply.locator('[data-act="delete-yes"]').click();
    await expect(reply.locator('.cd-tomb')).toHaveText('(deleted)');
    await expect.poll(() => JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0].replies[0].deleted).not.toBeNull();
    // A rejected resolve rolls back with the error on the card (the store refuses: deleted Document).
    const t2 = page.locator('.cd-thread[data-t="t2"]');
    cli(h().owner, ['delete', doc.id]);
    await t2.locator('[data-act="resolve"]').click();
    await expect(t2.locator('.cd-err')).toBeVisible();
    await expect(t2).not.toHaveClass(/cd-resolved/);
    cli(h().owner, ['restore', doc.id]);
    // The 100k cap: fill the Document's budget through the CLI, then a web reply is refused inline.
    cli(h().agent, ['thread', 'reply', doc.id, 't2', '-m', 'x'.repeat(99_900)]);
    await t2.locator('[data-act="reply"]').click();
    await t2.locator('.cd-editor-reply textarea').fill('y'.repeat(200));
    await page.keyboard.press('Enter');
    await expect(t2.locator('.cd-editor-reply .cd-err')).toBeVisible();
    await expect(t2.locator('.cd-editor-reply .cd-err')).toContainText('100000');
    await expect(t2.locator('.cd-editor-reply textarea')).toHaveValue('y'.repeat(200));
    await expect(t2.locator('.cd-c.cd-pending')).toHaveCount(0);
  });
});

test.describe('Actions', () => {
  // spec: S11-04, S11-05
  test('buttons on head only, coloured by intent, hidden for `[]`; taking one shows the chip at once; rollback on failure', async ({ page }) => {
    const doc = seedDoc('web-c5', PLAN);
    await openDoc(page, doc.id);
    const btns = page.locator('#cd-actbtns .cd-actbtn');
    await expect(btns).toHaveCount(2);
    await expect(btns.nth(0)).toHaveText('Approve');
    await expect(btns.nth(0)).toHaveClass(/cd-pos/);
    await expect(btns.nth(1)).toHaveText('Send back');
    await expect(btns.nth(1)).toHaveClass(/cd-neu/);
    const pos = await btns.nth(0).evaluate((e) => getComputedStyle(e).backgroundColor);
    const neu = await btns.nth(1).evaluate((e) => getComputedStyle(e).backgroundColor);
    expect(pos).not.toBe(neu);
    await expect(page.locator('#cd-actions .cd-chip')).toHaveText('no action yet');
    // Hold the POST: the chip appears before the server answers.
    let release: () => void = () => {};
    const held = new Promise<void>((r) => { release = r; });
    await page.route('**/api/v1/docs/*/actions', async (route) => {
      if (route.request().method() !== 'POST') { await route.continue(); return; }
      await held;
      await route.continue();
    });
    await btns.nth(0).click();
    const chip = page.locator('#cd-actions .cd-chip.cd-action');
    await expect(chip).toContainText('Approve');
    await expect(chip).toContainText('Rowan Vale');
    await expect(chip).toHaveClass(/cd-pos/);
    await expect(page.locator('.cd-status')).toContainText('approve by Rowan Vale');
    release();
    await expect.poll(() => JSON.parse(cli(h().owner, ['status', doc.id, '--json'])).action?.id).toBe('approve');
    await expect(chip).toHaveAttribute('data-version', '1');
    await page.unroute('**/api/v1/docs/*/actions');
    // Failure: the previous chip comes back and the error is visible.
    await page.route('**/api/v1/docs/*/actions', async (route) => {
      if (route.request().method() !== 'POST') { await route.continue(); return; }
      await route.fulfill({ status: 409, contentType: 'application/json', body: JSON.stringify({ error: 'not_head', message: 'head is v9, not v1' }) });
    });
    await btns.nth(1).click();
    await expect(page.locator('#cd-toast')).toBeVisible();
    await expect(page.locator('#cd-toast')).toContainText('head is v9');
    await expect(page.locator('#cd-actions .cd-chip.cd-action')).toContainText('Approve');
    await page.unroute('**/api/v1/docs/*/actions');
    // An older Version shows its own chip and no buttons.
    pushVersion(doc, PLAN + '\nv2.\n');
    await openDoc(page, doc.id, { v: 1 });
    await expect(page.locator('#cd-actbtns')).toHaveCount(0);
    await expect(page.locator('#cd-actions .cd-chip.cd-action')).toContainText('Approve');
    await openDoc(page, doc.id);
    await expect(page.locator('#cd-actbtns .cd-actbtn')).toHaveCount(2);
    await expect(page.locator('#cd-actions .cd-chip')).toHaveText('no action yet');
    // A Document declaring `actions.available: []` shows no buttons at all.
    const md = '# No actions\n\nnothing to decide\n';
    const bytes = Buffer.from(md);
    const hash = (await import('node:crypto')).createHash('sha256').update(bytes).digest('hex');
    const put = await page.request.put(`${h().url}/api/v1/blobs/${hash}`, { headers: { Authorization: `Bearer ${h().owner}`, 'Content-Type': 'application/octet-stream' }, data: bytes });
    expect(put.ok()).toBeTruthy();
    const created = await page.request.post(`${h().url}/api/v1/docs`, { headers: { Authorization: `Bearer ${h().owner}` }, data: { vault: 'web', name: 'web-c5-none', md_hash: hash, actions_available: [] } });
    expect(created.status()).toBe(201);
    const none = (await created.json()).id as string;
    await openDoc(page, none);
    await expect(page.locator('#cd-actbtns')).toHaveCount(0);
    await expect(page.locator('#cd-actions .cd-chip')).toHaveText('no action yet');
  });
});

test.describe('first paint stays complete', () => {
  // spec: S11-01
  test('the head page carries the pill, the help and the data the script needs; no Thread is fetched on load', async ({ page }) => {
    const doc = seedDoc('web-c6', PLAN);
    const requests: string[] = [];
    page.on('request', (r) => requests.push(r.url()));
    await openDoc(page, doc.id);
    await waitLive(page);
    expect(requests.filter((u) => u.includes('/threads'))).toEqual([]);
    expect(requests.filter((u) => u.includes('/api/v1/') && !u.includes('/api/v1/events?'))).toEqual([]);
    const data = await page.evaluate(() => JSON.parse(document.getElementById('cd-data')!.textContent!));
    expect(data.me).toBe('rowan');
    expect(data.can_comment).toBe(true);
    expect(data.can_act).toBe(true);
    expect(typeof data.md_hash).toBe('string');
    expect(data.actions.map((a: { id: string }) => a.id)).toEqual(['approve', 'send-back']);
    await expect(page.locator('#cd-help')).toBeHidden();
    await expect(page.locator('#cd-pill')).toBeHidden();
  });
});

test.describe('hidden resolved Threads (CMD-20)', () => {
  // spec: S28-01
  test('with the toggle off, clicking a resolved Thread\'s text does nothing; an open Thread under it still opens', async ({ page }) => {
    const md = `# Hidden

Alpha paragraph with a resolved note inside it.

Beta paragraph stays plain.
`;
    const doc = seedDoc('web-cmd20', md);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(md, 'Alpha paragraph with a resolved note'), '-m', 'Open one.']);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(md, 'resolved note'), '-m', 'Resolved one.']);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(md, 'Beta paragraph'), '-m', 'Resolved alone.']);
    cli(h().agent, ['thread', 'resolve', doc.id, 't2']);
    cli(h().agent, ['thread', 'resolve', doc.id, 't3']);
    await openDoc(page, doc.id);
    const toggle = page.locator('#cd-resolved');
    await expect(toggle).not.toBeChecked();
    // A resolved Thread alone on its text: the click selects nothing and leaves the toggle off.
    await page.locator('mark.cd-hl[data-t="t3"]').first().click();
    await expect(toggle).not.toBeChecked();
    await expect(page.locator('.cd-app')).not.toHaveClass(/cd-show-resolved/);
    await expect(page.locator('.cd-thread[data-t="t3"]')).toBeHidden();
    await expect(page.locator('.cd-selected')).toHaveCount(0);
    // A resolved Thread inside an open one's text: the click opens the open one.
    await page.locator('mark.cd-hl[data-t="t2"]').first().click();
    await expect(toggle).not.toBeChecked();
    await expect(page.locator('.cd-thread[data-t="t2"]')).toBeHidden();
    await expect(page.locator('.cd-thread[data-t="t1"]')).toHaveClass(/cd-selected/);
    // With the toggle on, the resolved Thread's text opens it as before.
    await toggle.check();
    await page.locator('mark.cd-hl[data-t="t3"]').first().click();
    await expect(page.locator('.cd-thread[data-t="t3"]')).toHaveClass(/cd-selected/);
  });
});
