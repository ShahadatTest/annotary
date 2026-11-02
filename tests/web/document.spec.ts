// The Document view (ticket 10): layouts A/C across the 850px threshold, the narrow header
// overlay, Thread rendering (anchored / refined / orphaned / tombstoned / resolved / Members),
// the Version time-slice + banner + Action chips, media, History, and the guarantee that
// the first HTML response carries every Thread (no post-load Thread fetch).
import { test, expect, type Page } from '@playwright/test';
import { harness } from './harness';

const h = () => harness();
const docUrl = (v?: number) => `${h().url}/d/${h().doc.id}${v ? `/v/${v}` : ''}`;

async function open(page: Page, width: number, v?: number) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto(docUrl(v));
  await page.waitForSelector('.cd-app.cd-ready');
}

async function position(page: Page, sel: string): Promise<string> {
  return page.locator(sel).first().evaluate((e) => getComputedStyle(e).position);
}

test.describe('layouts', () => {
  // spec: S10-03
  test('A at 900px: cards in the right margin at their anchor line, pushed apart; C at 800px: rows inline under their block', async ({ page }) => {
    await open(page, 900);
    const doc = (await page.locator('.cd-doc').boundingBox())!;
    const t2 = page.locator('.cd-thread[data-t="t2"]');
    const strip = page.locator('.cd-strip', { has: t2 });
    expect(await strip.evaluate((e) => getComputedStyle(e).position)).toBe('absolute');
    const sb = (await strip.boundingBox())!;
    expect(sb.x).toBeGreaterThanOrEqual(doc.x + doc.width - 1);
    // At its anchor line: the card's top is the refined mark's top (or pushed down by a card above).
    const mark = page.locator('mark.cd-hl[data-t="t2"]').first();
    const mb = (await mark.boundingBox())!;
    expect(sb.y).toBeGreaterThanOrEqual(mb.y - 2);
    expect(sb.y - mb.y).toBeLessThan(120);
    // Cards never overlap (pushed apart Docs-style).
    const boxes = await page.locator('.cd-strip:visible').evaluateAll((els) =>
      els.map((e) => { const r = e.getBoundingClientRect(); return { t: r.top, b: r.bottom }; }).sort((a, b) => a.t - b.t));
    for (let i = 1; i < boxes.length; i++) expect(boxes[i].t).toBeGreaterThanOrEqual(boxes[i - 1].b - 1);
    // A card shows its Comments; the narrow-only snippet row is hidden.
    await expect(t2.locator('.cd-body')).toBeVisible();
    await expect(t2.locator('.cd-snip')).toBeHidden();
    // Document-level Threads are pinned at the top of the margin (t1 is resolved: behind the toggle).
    await expect(page.locator('.cd-strip-doc')).toBeHidden();
    await page.locator('#cd-resolved').check();
    await expect(page.locator('.cd-strip-doc')).toBeVisible();
    const ds = (await page.locator('.cd-strip-doc').boundingBox())!;
    expect(Math.abs(ds.y - (doc.y + 24))).toBeLessThan(4);
    await expect(page.locator('.cd-thread[data-t="t1"]')).toBeVisible();

    // Narrow: one column, Threads inline as rows directly under their block.
    await page.setViewportSize({ width: 800, height: 900 });
    await page.waitForTimeout(150);
    expect(await strip.evaluate((e) => getComputedStyle(e).position)).toBe('static');
    const doc2 = (await page.locator('.cd-doc').boundingBox())!;
    const sb2 = (await strip.boundingBox())!;
    expect(sb2.x + sb2.width).toBeLessThanOrEqual(doc2.x + doc2.width + 1);
    const line = await t2.getAttribute('data-line');
    const block = page.locator(`.cd-doc p[data-l^="${line}-"]`).first();
    const pb = (await block.boundingBox())!;
    expect(sb2.y).toBeGreaterThanOrEqual(pb.y + pb.height - 1);
    expect(sb2.y - (pb.y + pb.height)).toBeLessThan(40);
    // Row: status dot · frozen quote · author · snippet · replies; expands in place on tap.
    await expect(t2.locator('.cd-row .cd-dot')).toBeVisible();
    await expect(t2.locator('.cd-row .cd-quote')).toContainText('We ship the CLI before the web UI');
    await expect(t2.locator('.cd-row .cd-snip')).toContainText('Rowan Vale');
    await expect(t2.locator('.cd-row .cd-meta')).toContainText('1 reply');
    await expect(t2.locator('.cd-body')).toBeHidden();
    await t2.locator('.cd-row').click();
    await expect(t2.locator('.cd-body')).toBeVisible();
    const sb3 = (await strip.boundingBox())!;
    expect(Math.abs(sb3.y - sb2.y)).toBeLessThan(2); // in place
    // The Document-level strip sits under the title in the flow.
    const h1 = (await page.locator('.cd-doc h1').boundingBox())!;
    const ds2 = (await page.locator('.cd-strip-doc').boundingBox())!;
    expect(ds2.y).toBeGreaterThan(h1.y);
    expect(ds2.y - (h1.y + h1.height)).toBeLessThan(40);
  });

  // spec: S10-03
  test('the threshold is the container width (849 → C, 850 → A) and ultra-wide stays centred with no third column', async ({ page }) => {
    await open(page, 849);
    expect(await position(page, '.cd-strip')).toBe('static');
    await page.setViewportSize({ width: 850, height: 900 });
    await page.waitForTimeout(100);
    expect(await position(page, '.cd-strip')).toBe('absolute');
    await page.setViewportSize({ width: 3000, height: 900 });
    await page.waitForTimeout(100);
    const cols = await page.locator('.cd-layout').evaluate((e) => getComputedStyle(e).gridTemplateColumns.split(' ').length);
    expect(cols).toBe(2);
    const doc = (await page.locator('.cd-doc').boundingBox())!;
    expect(doc.x).toBeGreaterThan(600);
    expect(doc.width).toBeLessThan(900);
    const strip = (await page.locator('.cd-strip:visible').first().boundingBox())!;
    expect(strip.x).toBeGreaterThan(doc.x + doc.width);
    expect(strip.x + strip.width).toBeLessThan(3000 - 600);
  });
});

test.describe('header', () => {
  // spec: S10-04
  test('narrow: one persistent line + arrows; tap opens the overlay, outside click dismisses; wide: the full header; no bottom bar', async ({ page }) => {
    await open(page, 640);
    const line = page.locator('#cd-line');
    await expect(line.locator('.cd-name')).toHaveText('ra-plan');
    await expect(line.locator('.cd-nline')).toBeVisible();
    await expect(line.locator('.cd-nline')).toContainText('v5');
    await expect(line.locator('.cd-nline')).toContainText('6 open');
    await expect(line.locator('.cd-nline')).toContainText('2 @you');
    await expect(line.locator('.cd-arrows .cd-older')).toBeVisible();
    await expect(page.locator('#cd-rest')).toBeHidden();
    await line.locator('.cd-name').click();
    await expect(page.locator('#cd-rest')).toBeVisible();
    await expect(page.locator('#cd-rest #cd-vsel')).toBeVisible();
    await expect(page.locator('#cd-rest #cd-resolved')).toBeVisible();
    await expect(page.locator('#cd-rest #cd-whatsnew')).toBeVisible();
    await expect(page.locator('#cd-rest #cd-actions')).toBeVisible();
    await expect(page.locator('#cd-rest .cd-status')).toContainText('6 open (3 orphaned)');
    // Outside click dismisses.
    await page.locator('.cd-doc h2').first().click();
    await expect(page.locator('#cd-rest')).toBeHidden();
    // Escape too.
    await line.locator('.cd-name').click();
    await expect(page.locator('#cd-rest')).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(page.locator('#cd-rest')).toBeHidden();
    // No bottom bar anywhere.
    const fixed = await page.evaluate(() => [...document.querySelectorAll('*')].filter((e) => getComputedStyle(e).position === 'fixed').length);
    expect(fixed).toBe(0);
    // Wide: the full header, no narrow line, arrows inside the switcher only.
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.waitForTimeout(100);
    await expect(line.locator('.cd-nline')).toBeHidden();
    await expect(page.locator('#cd-rest')).toBeVisible();
    await expect(page.locator('#cd-rest .cd-vsw .cd-older')).toBeVisible();
    await expect(line.locator('.cd-arrows')).toBeHidden();
    await expect(page.locator('#cd-rest .cd-status')).toContainText('v5');
    await line.locator('.cd-name').click(); // no overlay toggling when wide
    await expect(page.locator('.cd-app')).not.toHaveClass(/cd-overlay-open/);
  });
});

test.describe('threads', () => {
  // spec: S10-05
  test('anchored highlight, refined = frozen quote + now + dotted underline, orphaned behind the disclosure, tombstones, Members, resolved toggle', async ({ page }) => {
    await open(page, 1280);
    // Anchored: a yellow mark in the doc for t5.
    const m5 = page.locator('mark.cd-hl.cd-st-anchored[data-t="t5"]').first();
    await expect(m5).toHaveText('testable in isolation');
    expect(await m5.evaluate((e) => getComputedStyle(e).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)');
    // Refined t2: the card shows the frozen quote and the now: line; the doc mark is dotted-underlined.
    const t2 = page.locator('.cd-thread[data-t="t2"]');
    await expect(t2).toHaveClass(/cd-st-refined/);
    await expect(t2.locator('.cd-quote > q').first()).toHaveText('We ship the CLI before the web UI');
    await expect(t2.locator('.cd-now')).toContainText('now:');
    await expect(t2.locator('.cd-now q')).toHaveText('We ship the CLI well before the web UI');
    const m2 = page.locator('mark.cd-hl.cd-st-refined[data-t="t2"]').first();
    await expect(m2).toHaveText('We ship the CLI well before the web UI');
    const deco = await m2.evaluate((e) => {
      const s = getComputedStyle(e);
      return { line: s.textDecorationLine, style: s.textDecorationStyle };
    });
    expect(deco.line).toContain('underline');
    expect(deco.style).toBe('dotted');
    // Orphaned: t6, t7, t8 grouped at the end behind the disclosure, hidden by default.
    const det = page.locator('details.cd-orphaned');
    await expect(det).toHaveCount(1);
    await expect(det).not.toHaveAttribute('open', '');
    await expect(det.locator('summary')).toContainText('Orphaned (3)');
    for (const t of ['t6', 't7', 't8']) {
      await expect(det.locator(`.cd-thread[data-t="${t}"]`)).toHaveCount(1);
      await expect(page.locator(`.cd-thread[data-t="${t}"]`)).toBeHidden();
      await expect(page.locator(`mark.cd-hl[data-t="${t}"]`)).toHaveCount(0);
    }
    const strips = await page.locator('.cd-strip').evaluateAll((els) => els.map((e) => e.className));
    expect(strips[strips.length - 1]).toContain('cd-strip-orphaned');
    await det.locator('summary').click();
    await expect(page.locator('.cd-thread[data-t="t6"]')).toBeVisible();
    await expect(page.locator('.cd-thread[data-t="t6"] .cd-out')).toContainText('orphaned — text no longer in v5');
    // Tombstoned root shows (deleted), its reply stays; a fully tombstoned Thread (t9) is gone.
    await expect(page.locator('.cd-thread[data-t="t7"] .cd-c[data-c="t7"] .cd-tomb')).toHaveText('(deleted)');
    await expect(page.locator('.cd-thread[data-t="t7"] .cd-c[data-c="t7.1"]')).toContainText('fixture table');
    await expect(page.locator('.cd-thread[data-t="t9"]')).toHaveCount(0);
    // Asset Thread t4 highlights the image; its label names the ref.
    await expect(page.locator('img.cd-asset-hl[data-t="t4"]')).toHaveCount(1);
    await expect(page.locator('.cd-thread[data-t="t4"] .cd-quote')).toContainText('img/architecture.png');
    // Members: display name + auth-method glyph, never "bot"; harness/model hook empty.
    const who = page.locator('.cd-thread[data-t="t5"] .cd-c[data-c="t5"] .cd-who');
    await expect(who.locator('.cd-member')).toHaveText('Scribe');
    await expect(who.locator('.cd-member')).toHaveAttribute('title', '@scribe');
    await expect(who.locator('.cd-auth')).toHaveAttribute('title', /Token/);
    expect((await page.content()).toLowerCase()).not.toContain('bot badge');
    expect(await page.locator('.cd-app').evaluate((e) => /\bbot\b/i.test(e.textContent || ''))).toBe(false);
    // @mention of me shows on the Thread.
    await expect(page.locator('.cd-thread[data-t="t5"]')).toHaveClass(/cd-mine/);
    // Resolved Threads (t1, t3) are behind the toggle — and so are their highlights.
    await expect(page.locator('.cd-thread[data-t="t3"]')).toBeHidden();
    const m3 = page.locator('mark.cd-hl[data-t="t3"]').first();
    expect(await m3.evaluate((e) => getComputedStyle(e).backgroundColor)).toBe('rgba(0, 0, 0, 0)');
    await page.locator('#cd-resolved').check();
    await expect(page.locator('.cd-thread[data-t="t3"]')).toBeVisible();
    await expect(page.locator('.cd-thread[data-t="t3"] .cd-resline')).toContainText('resolved by Rowan Vale');
    expect(await m3.evaluate((e) => getComputedStyle(e).backgroundColor)).not.toBe('rgba(0, 0, 0, 0)');
    // Clicking a highlight selects its card.
    await m5.click();
    await expect(page.locator('.cd-thread[data-t="t5"]')).toHaveClass(/cd-selected/);
  });

  // spec: S10-09
  test('Comment bodies are the Document superset minus embeds', async ({ page }) => {
    await open(page, 1280);
    await page.locator('details.cd-orphaned summary').click();
    const body = page.locator('.cd-thread[data-t="t8"] .cd-c[data-c="t8"] .cd-cbody');
    await expect(body).toBeVisible();
    // Comments are never asset-scanned: the image reference is a plain link as written.
    await expect(body.locator('img')).toHaveCount(0);
    await expect(body.locator('a[href="img/architecture.png"]')).toHaveText('arch');
    await expect(body.locator('a[href="api-notes.md"]')).toHaveText('notes');
    const t1 = page.locator('.cd-thread[data-t="t1"] .cd-c[data-c="t1"] .cd-cbody');
    await page.locator('#cd-resolved').check();
    await expect(t1.locator('strong')).toHaveText('Risks');
  });
});

test.describe('versions', () => {
  // spec: S10-06
  test('the Version select, the older-Version banner, Go to head, an Action chip per Version, no diff view', async ({ page }) => {
    await open(page, 1280);
    await expect(page.locator('#cd-banner')).toHaveCount(0);
    const sel = page.locator('#cd-vsel');
    await expect(sel.locator('option')).toHaveCount(5);
    const labels = await sel.locator('option').allTextContents();
    expect(labels[0]).toMatch(/^v5 \(head\) · Rowan Vale · /);
    expect(labels[1]).toContain('v4');
    expect(labels[1]).toContain('same as v1');
    expect(labels[2]).toContain('Approve');
    expect(labels[4]).toContain('Send back');
    // Timestamps are localized client-side (no trailing Z left in the labels).
    expect(labels[0]).not.toMatch(/\dZ/);
    await expect(page.locator('#cd-actions .cd-chip')).toHaveText('no action yet');
    await expect(page.locator('.cd-vsw .cd-newer')).toHaveClass(/cd-off/);
    // Switch to v2.
    await sel.selectOption('2');
    await page.waitForURL(`**/d/${h().doc.id}/v/2`);
    await page.waitForSelector('.cd-app.cd-ready');
    const banner = page.locator('#cd-banner');
    await expect(banner).toBeVisible();
    await expect(banner).toContainText('Viewing v2');
    await expect(banner).toContainText('Scribe');
    await expect(banner).toContainText('address review: add Risks, split rollout');
    await expect(banner).toContainText('Threads as they were; replies/resolve work, new comments only on head');
    // Threads as of v2: t6/t7 exist, t8 (created on v3) does not; t3 still open; nothing orphaned.
    await expect(page.locator('.cd-thread[data-t="t6"]')).toHaveCount(1);
    await expect(page.locator('.cd-thread[data-t="t7"]')).toHaveCount(1);
    await expect(page.locator('.cd-thread[data-t="t8"]')).toHaveCount(0);
    await expect(page.locator('.cd-thread[data-t="t3"]')).toBeVisible();
    await expect(page.locator('details.cd-orphaned')).toHaveCount(0);
    await expect(page.locator('#cd-actions .cd-chip')).toHaveText('no action on v2');
    await expect(page.locator('#cd-line .cd-name')).toHaveText('ra-plan');
    // v3: its own Approve chip (positive); v1: Send back (neutral); older arrow off on v1.
    await page.locator('.cd-vsw .cd-newer').click();
    await page.waitForURL(`**/d/${h().doc.id}/v/3`);
    const chip3 = page.locator('#cd-actions .cd-chip.cd-action');
    await expect(chip3).toHaveClass(/cd-pos/);
    await expect(chip3).toContainText('Approve');
    await expect(chip3).toContainText('Rowan Vale');
    await expect(chip3).toHaveAttribute('data-version', '3');
    await expect(page.locator('.cd-thread[data-t="t8"]')).toHaveCount(1);
    await page.goto(docUrl(1));
    await page.waitForSelector('.cd-app.cd-ready');
    const chip1 = page.locator('#cd-actions .cd-chip.cd-action');
    await expect(chip1).toHaveClass(/cd-neu/);
    await expect(chip1).toContainText('Send back');
    await expect(page.locator('.cd-vsw .cd-older')).toHaveClass(/cd-off/);
    // t1 was resolved while v1 was head (before v2 was pushed): resolved on v1 too; t2 open and anchored.
    await expect(page.locator('.cd-thread[data-t="t1"]')).toBeHidden();
    await expect(page.locator('.cd-thread[data-t="t2"]')).toBeVisible();
    await expect(page.locator('.cd-thread[data-t="t2"]')).toHaveClass(/cd-st-anchored/);
    await expect(page.locator('.cd-thread[data-t="t2"] .cd-now')).toHaveCount(0);
    // v4 notes "same content as v1".
    await page.goto(docUrl(4));
    await expect(page.locator('.cd-chip.cd-same')).toHaveText('same content as v1');
    // Go to head.
    await page.locator('#cd-gohead').click();
    await page.waitForURL(`**/d/${h().doc.id}`);
    await expect(page.locator('#cd-banner')).toHaveCount(0);
    // No diff view anywhere.
    expect(await page.locator('[class*="diff"], [id*="diff"]').count()).toBe(0);
  });
});

test.describe('media and what\'s new', () => {
  // spec: S10-07
  test('inline media from /a/{hash} with reserved boxes, missing placeholder, asset links in a new tab, localized times', async ({ page }) => {
    await open(page, 1280);
    const img = page.locator('.cd-doc img[src^="/a/"]').first();
    await expect(img).toHaveAttribute('width', /\d+/);
    await expect(img).toHaveAttribute('height', /\d+/);
    expect(await img.evaluate((e: HTMLImageElement) => e.complete && e.naturalWidth > 0)).toBe(true);
    await expect(page.locator('.cd-doc .asset-missing[data-ref="img/cli-screenshot.png"]')).toBeVisible();
    await expect(page.locator('.cd-doc a.asset-missing[data-ref="api-notes.md"]')).toBeVisible();
    // Every timestamp on the page is localized by the script.
    const times = await page.locator('time[datetime]').evaluateAll((els) => els.map((e) => ({ t: e.textContent || '', local: e.classList.contains('cd-localized') })));
    expect(times.length).toBeGreaterThan(3);
    for (const t of times) { expect(t.local).toBe(true); expect(t.t).not.toMatch(/Z$/); }
  });

  // spec: S10-01
  test('History lists this Document\'s events from the events log, on demand', async ({ page }) => {
    await open(page, 1280);
    const calls: string[] = [];
    page.on('request', (r) => { if (r.url().includes('/api/v1/')) calls.push(r.url()); });
    // The button and its panel are called History.
    await expect(page.locator('#cd-whatsnew')).toHaveText('History');
    await page.locator('#cd-whatsnew').click();
    const news = page.locator('#cd-news');
    await expect(news).toBeVisible();
    await expect(news.locator('h3')).toHaveText('History');
    await expect(news.locator('li').first()).toContainText('deleted t9');
    await expect(news.locator('li').first()).toContainText('Rowan Vale');
    await expect(news.locator('li').nth(2)).toContainText('pushed v5');
    await expect(news).toContainText('Approve v3');
    await expect(news).toContainText('opened t8');
    expect(calls.some((u) => u.includes('/api/v1/events?doc='))).toBe(true);
    // Clicking an event selects its Thread.
    await news.locator('li[data-t="t5"]').first().click();
    await expect(page.locator('.cd-thread[data-t="t5"]')).toHaveClass(/cd-selected/);
    await expect(news).toBeHidden();
  });
});

test.describe('first response', () => {
  // spec: S10-01
  test('the first HTML response contains every Thread and placement; nothing fetches Threads after load', async ({ page }) => {
    const res = await page.request.get(docUrl());
    expect(res.status()).toBe(200);
    const html = await res.text();
    for (const t of ['t1', 't2', 't3', 't4', 't5', 't6', 't7', 't8']) expect(html).toContain(`data-t="${t}"`);
    expect(html).not.toContain('data-t="t9"');
    expect(html).toContain('id="cd-data"');
    expect(html).toContain('"status":"refined"');
    expect(html).toContain('"status":"orphaned"');
    const requests: string[] = [];
    page.on('request', (r) => requests.push(r.url()));
    await open(page, 1280);
    // Ticket 11 subscribes to the events SSE once the page is ready (a long-lived request, so
    // `networkidle` never comes): wait for the subscription instead, then look at what was asked.
    await page.waitForSelector('.cd-app.cd-live');
    expect(requests.filter((u) => u.includes('/api/v1/') && !u.includes('/api/v1/events?'))).toEqual([]);
    expect(requests.filter((u) => u.includes('/threads'))).toEqual([]);
    // The page is one document plus its images: no stylesheet / script requests.
    expect(requests.filter((u) => /\.(css|js)(\?|$)/.test(u))).toEqual([]);
  });
});
