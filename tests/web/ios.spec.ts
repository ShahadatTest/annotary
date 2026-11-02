// Ticket 18 — the iOS app's prerequisites on the web: the hidden "Open in Annotary" chip on
// the Document page (shown on iPhone / iPad only, with the Thread of `#tN`), the `#tN` fragment
// `annotary link` prints selecting that Thread, and "Pair a device" on the owner Members
// screen (a QR panel on a server with a public URL; the `no_public_url` error on one without).
import { test, expect } from '@playwright/test';
import { harness, spawnServer } from './harness';

const h = () => harness();
const docUrl = () => `${h().url}/d/${h().doc.id}`;
const IPHONE_UA =
  'Mozilla/5.0 (iPhone; CPU iPhone OS 18_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.0 Mobile/15E148 Safari/604.1';

test.describe('Open in Annotary (desktop)', () => {
  // spec: S18-12
  test('is in the header but hidden for a desktop browser; its href hands the app this server and the Document', async ({ page }) => {
    await page.goto(docUrl());
    await page.waitForSelector('.cd-app.cd-ready');
    const link = page.locator('#cd-openapp');
    await expect(link).toHaveCount(1);
    await expect(link).toBeHidden();
    await expect(link).toHaveAttribute('href', `annotary://open?server=${encodeURIComponent(h().url)}&doc=${h().doc.id}`);
  });

  // spec: S18-06
  test('a #tN fragment (what `annotary link DOC tN` prints) selects that Thread', async ({ page }) => {
    await page.goto(docUrl() + '#t2');
    await page.waitForSelector('.cd-app.cd-ready');
    await expect(page.locator('.cd-thread[data-t="t2"]')).toHaveClass(/cd-selected/);
  });
});

test.describe('Open in Annotary (iPhone)', () => {
  test.use({ userAgent: IPHONE_UA });

  // spec: S18-12
  test('is shown on an iPhone and carries the Thread of #tN', async ({ page }) => {
    await page.goto(docUrl() + '#t2');
    await page.waitForSelector('.cd-app.cd-ready');
    const link = page.locator('#cd-openapp');
    await expect(link).toBeVisible();
    await expect(link).toHaveText('Open in Annotary');
    await expect(link).toHaveAttribute('href', `annotary://open?server=${encodeURIComponent(h().url)}&doc=${h().doc.id}&thread=t2`);
    // A later fragment change follows; no fragment, no thread.
    await page.evaluate(() => { location.hash = '#t3'; });
    await expect(link).toHaveAttribute('href', /&thread=t3$/);
    await page.goto(docUrl());
    await page.waitForSelector('.cd-app.cd-ready');
    await expect(page.locator('#cd-openapp')).toHaveAttribute('href', `annotary://open?server=${encodeURIComponent(h().url)}&doc=${h().doc.id}`);
  });
});

test.describe('Pair a device', () => {
  // spec: S18-10
  test('without a public URL the card says so (message + hint) and shows no code', async ({ page }) => {
    await page.goto(h().url + '/members');
    const card = page.locator('.cd-mem[data-handle="rowan"]');
    await card.locator('[data-act=pair]').click();
    await expect(card.locator('.cd-memerr')).toBeVisible();
    await expect(card.locator('.cd-memerr')).toContainText('no public URL is set');
    await expect(card.locator('.cd-memerr')).toContainText('annotary serve --url http://HOST:PORT');
    await expect(page.locator('.cd-paironce')).toHaveCount(0);
  });

  // spec: S18-10
  test('with one: the QR panel (shown once, Copy link, Dismiss) and the new "device" Token in the card', async ({ browser }) => {
    const srv = await spawnServer(undefined, ['--url', 'http://phone-test.example:7377']);
    const ctx = await browser.newContext();
    try {
      await ctx.grantPermissions(['clipboard-read', 'clipboard-write'], { origin: srv.url });
      const page = await ctx.newPage();
      await page.goto(srv.url + '/members');
      const card = page.locator('.cd-mem[data-handle="rowan"]');
      await expect(card.locator('.cd-tok', { hasText: 'device' })).toHaveCount(0);
      await card.locator('[data-act=pair]').click();
      const panel = page.locator('.cd-paironce[data-handle="rowan"]');
      await expect(panel).toBeVisible();
      await expect(panel.locator('.cd-qr svg')).toBeVisible();
      await expect(panel).toContainText('Scan with the iPhone or iPad camera, or Annotary. The code holds a new Token named “device”; it is shown once and can be revoked under TOKENS.');
      // The card was re-drawn with the new Token; the panel sits in the fresh card.
      const fresh = page.locator('.cd-mem[data-handle="rowan"]');
      await expect(fresh.locator('.cd-tok', { hasText: 'device' })).toHaveCount(1);
      await expect(fresh.locator('.cd-paironce')).toHaveCount(1);
      // Copy link → the annotary://pair URL, whose Token works.
      await panel.locator('[data-act=copy]').click();
      await expect(panel.locator('.cd-copied')).toBeVisible();
      const copied: string = await page.evaluate(() => navigator.clipboard.readText());
      expect(copied).toMatch(/^annotary:\/\/pair\?v=1&server=http%3A%2F%2Fphone-test\.example%3A7377&token=ant_[^&]+&profile=rowan%40phone-test$/);
      const token = decodeURIComponent(/token=([^&]+)/.exec(copied)![1]);
      const me = await page.request.get(srv.url + '/api/v1/me', { headers: { Authorization: 'Bearer ' + token } });
      expect(me.status()).toBe(200);
      expect((await me.json()).handle).toBe('rowan');
      // Dismiss: gone for good (the page never shows that Token again).
      await panel.locator('[data-act=dismiss]').click();
      await expect(page.locator('.cd-paironce')).toHaveCount(0);
      await page.reload();
      await expect(page.locator('.cd-paironce')).toHaveCount(0);
      expect(await page.content()).not.toContain(token);
    } finally {
      await ctx.close();
      await srv.stop();
    }
  });
});
