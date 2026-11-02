// CMD-23: the Document page wears the Member's Web settings from its first paint — appearance
// (Follow system carries both palettes), the highlight colors of each appearance, text size, Show
// resolved, absolute times once the browser's zone is known — and applies a change made in another
// browser without a reload (`&me=1` on the live feed). Each test runs its own server: a settings
// change is the Member's, and the shared server's owner is every other spec's browser.
import { test, expect, type Browser, type BrowserContext, type Page } from '@playwright/test';
import { createHash } from 'node:crypto';
import { spawnServer, type Spawned } from './harness';

const MD = `# Theme

Alpha paragraph to anchor a Thread on.

Beta paragraph for a resolved one.
`;

async function call(srv: Spawned, method: string, path: string, body?: unknown, raw?: Buffer): Promise<{ status: number; json: any }> {
  const headers: Record<string, string> = { Authorization: 'Bearer ' + srv.ownerToken };
  if (raw) headers['Content-Type'] = 'application/octet-stream';
  else if (body !== undefined) headers['Content-Type'] = 'application/json';
  const r = await fetch(srv.url + path, { method, headers, body: raw ?? (body === undefined ? undefined : JSON.stringify(body)) });
  const text = await r.text();
  let json: any = null;
  try { json = JSON.parse(text); } catch { json = text; }
  return { status: r.status, json };
}

/** A Document with t1 open on "Alpha" and t2 resolved on "Beta" (through the API, as the owner). */
async function seed(srv: Spawned): Promise<string> {
  const md = Buffer.from(MD);
  const hash = createHash('sha256').update(md).digest('hex');
  expect((await call(srv, 'PUT', `/api/v1/blobs/${hash}`, undefined, md)).status).toBeLessThan(300);
  const doc = await call(srv, 'POST', '/api/v1/docs', { vault: 'notes', name: 'theme', md_hash: hash });
  expect(doc.status, JSON.stringify(doc.json)).toBe(201);
  const id = doc.json.id as string;
  for (const [range, body] of [['3:0-3:5', 'Alpha?'], ['5:0-5:4', 'Beta, done.']]) {
    const t = await call(srv, 'POST', `/api/v1/docs/${id}/threads`, { subject: 'span', range, body });
    expect(t.status, JSON.stringify(t.json)).toBe(201);
  }
  expect((await call(srv, 'POST', `/api/v1/docs/${id}/threads/t2/resolve`, {})).status).toBeLessThan(300);
  return id;
}

/** A setting for the owner: a device setting for All devices, a user setting for the Member. */
async function setting(srv: Spawned, key: string, value: unknown) {
  const user = ['show_resolved', 'times', 'clock', 'default_vault', 'display_name'].includes(key);
  const r = await call(srv, 'PUT', '/api/v1/me/settings', { key, value, scope: user ? 'user' : 'all' });
  expect(r.status, JSON.stringify(r.json)).toBe(200);
}

const style = (page: Page, sel: string, prop: string) =>
  page.locator(sel).first().evaluate((el, p) => getComputedStyle(el).getPropertyValue(p as string), prop);

test.describe('settings on the Document page', () => {
  let srv: Spawned;
  const contexts: BrowserContext[] = [];
  async function open(browser: Browser, opts: Parameters<Browser['newContext']>[0] = {}) {
    const c = await browser.newContext(opts);
    contexts.push(c);
    return c.newPage();
  }
  test.beforeEach(async () => { srv = await spawnServer(); });
  test.afterEach(async () => {
    for (const c of contexts.splice(0)) await c.close();
    await srv.stop();
  });

  // spec: S29-24
  test('the highlight color of each appearance, from the first paint; Follow system takes the system\'s, Light / Dark override it', async ({ browser }) => {
    const id = await seed(srv);
    await setting(srv, 'highlight_light', 'green');
    await setting(srv, 'highlight_dark', 'pink');
    const light = await open(browser, { colorScheme: 'light' });
    const dark = await open(browser, { colorScheme: 'dark' });
    const mark = 'mark.cd-hl[data-t="t1"]';
    const looks = async (page: Page) => ({
      hl: await style(page, mark, 'background-color'),
      ink: await style(page, mark, 'color'),
      app: await style(page, '.cd-app', 'background-color'),
      canvas: await style(page, 'html', 'background-color'),
    });
    const LIGHT = { hl: 'rgb(200, 242, 208)', ink: 'rgb(28, 28, 30)', app: 'rgb(255, 255, 255)', canvas: 'rgb(255, 255, 255)' };
    const DARK = { hl: 'rgb(244, 114, 182)', ink: 'rgb(0, 0, 0)', app: 'rgb(0, 0, 0)', canvas: 'rgb(0, 0, 0)' };
    // Follow system (the default): each browser its system's palette, from the server's HTML alone.
    for (const [page, want] of [[light, LIGHT], [dark, DARK]] as const) {
      await page.goto(`${srv.url}/d/${id}`);
      await expect(page.locator('.cd-app')).toHaveClass(/cd-theme-system/);
      expect(await looks(page)).toEqual(want);
      await page.waitForSelector('.cd-app.cd-ready');
      expect(await looks(page)).toEqual(want);
    }
    // Refined and selected keep their own colors; the selected shade follows the preset.
    await light.locator(mark).click();
    await expect.poll(() => style(light, mark, 'background-color')).toBe('rgb(134, 226, 155)');
    // Light and Dark override the system.
    await setting(srv, 'appearance', 'dark');
    await light.reload();
    expect(await looks(light)).toEqual(DARK);
    await setting(srv, 'appearance', 'light');
    await dark.reload();
    expect(await looks(dark)).toEqual(LIGHT);
    // The list page wears it too.
    await dark.goto(srv.url + '/');
    expect(await style(dark, '.cd-app', 'background-color')).toBe('rgb(255, 255, 255)');
  });

  // spec: S29-25
  test('a change made in one browser shows on an open Document page and list in another, without a reload', async ({ browser }) => {
    const id = await seed(srv);
    const a = await open(browser, { colorScheme: 'light' });
    await a.goto(`${srv.url}/d/${id}`);
    await a.waitForSelector('.cd-app.cd-ready');
    await a.waitForSelector('.cd-app.cd-live', { timeout: 10_000 });
    await a.evaluate(() => { (window as any).__same = 1; });
    const list = await open(browser, { colorScheme: 'light' });
    await list.goto(srv.url + '/');
    await list.waitForSelector('.cd-app.cd-live', { timeout: 10_000 });
    // Browser B signs in (its own session) and changes settings through the API.
    const b = await open(browser);
    await b.goto(srv.url + '/');
    const put = async (key: string, value: unknown, scope = 'all') => {
      const r = await b.request.put(srv.url + '/api/v1/me/settings', { data: { key, value, scope } });
      expect(r.status(), await r.text()).toBe(200);
    };
    const mark = 'mark.cd-hl[data-t="t1"]';
    expect(await style(a, mark, 'background-color')).toBe('rgb(255, 241, 168)');
    await put('highlight_light', 'blue');
    await expect.poll(() => style(a, mark, 'background-color')).toBe('rgb(207, 227, 255)');
    await put('appearance', 'dark');
    await expect(a.locator('.cd-app')).toHaveClass(/cd-theme-dark/);
    await expect.poll(() => style(a, '.cd-app', 'background-color')).toBe('rgb(0, 0, 0)');
    await expect.poll(() => style(a, mark, 'background-color')).toBe('rgb(250, 204, 21)');
    await expect(list.locator('.cd-app')).toHaveClass(/cd-theme-dark/);
    // Show resolved (a user setting): the resolved card shows and the toggle is checked.
    await expect(a.locator('.cd-thread[data-t="t2"]')).toBeHidden();
    await put('show_resolved', 'shown', 'user');
    await expect(a.locator('.cd-thread[data-t="t2"]')).toBeVisible();
    await expect(a.locator('#cd-resolved')).toBeChecked();
    // Text size scales the Document text.
    expect(await style(a, '.cd-doc', 'font-size')).toBe('15px');
    await put('text_size', 130);
    await expect.poll(() => style(a, '.cd-doc', 'font-size')).toBe('19.5px');
    // Keyboard shortcuts off: `j` selects nothing.
    await put('keyboard_shortcuts', 'off');
    await expect(a.locator('.cd-app')).toHaveAttribute('data-cd-keys', 'off');
    await a.locator('h1').click();
    await a.keyboard.press('j');
    await expect(a.locator('.cd-thread.cd-selected')).toHaveCount(0);
    // Never reloaded.
    expect(await a.evaluate(() => (window as any).__same)).toBe(1);
  });

  // spec: S29-24
  test('absolute times once the zone cookie is known, Show resolved and text size — with CLS 0; the Send key', async ({ browser }) => {
    const id = await seed(srv);
    await setting(srv, 'times', 'absolute');
    await setting(srv, 'clock', '24');
    await setting(srv, 'text_size', 150);
    await setting(srv, 'show_resolved', 'shown');
    await setting(srv, 'appearance', 'dark');
    const page = await open(browser, { timezoneId: 'Asia/Tokyo' });
    await page.addInitScript(() => {
      (window as any).__cls = 0;
      new PerformanceObserver((l) => {
        for (const e of l.getEntries() as any[]) if (!e.hadRecentInput) (window as any).__cls += e.value;
      }).observe({ type: 'layout-shift', buffered: true });
    });
    // First visit: the server does not know the zone yet, so times stay relative; the script saves it.
    await page.goto(`${srv.url}/d/${id}`);
    await page.waitForSelector('.cd-app.cd-ready');
    await expect(page.locator('time.cd-abs')).toHaveCount(0);
    await expect(page.locator('.cd-thread[data-t="t1"] .cd-meta time')).toHaveText('now');
    const jar = await page.context().cookies(srv.url);
    expect(jar.find((c) => c.name === 'cd_tz')?.value).toBe('Asia/Tokyo');
    expect(jar.find((c) => c.name === 'cd_hc')?.value).toMatch(/^(12|24)$/);
    // Next page: absolute, in Tokyo, 24-hour, painted by the server — and nothing shifts.
    await page.reload();
    await page.waitForSelector('.cd-app.cd-ready');
    await expect(page.locator('.cd-app')).toHaveAttribute('data-cd-tz', 'Asia/Tokyo');
    expect(await page.locator('time.cd-abs').count()).toBeGreaterThan(3);
    await expect(page.locator('.cd-thread[data-t="t1"] .cd-c time')).toHaveText(/^[A-Z][a-z]{2} \d{1,2}, \d{2}:\d{2}$/);
    await expect(page.locator('.cd-thread[data-t="t2"]')).toBeVisible();
    await expect(page.locator('#cd-resolved')).toBeChecked();
    expect(await style(page, '.cd-doc', 'font-size')).toBe('22.5px');
    // Layout A still aligns a margin card with its highlight at the larger text.
    const markBox = await page.locator('mark.cd-hl[data-t="t1"]').boundingBox();
    const cardBox = await page.locator('.cd-thread[data-t="t1"]').boundingBox();
    expect(Math.abs(markBox!.y - cardBox!.y)).toBeLessThan(4);
    expect(cardBox!.x).toBeGreaterThan(markBox!.x + markBox!.width);
    await page.waitForTimeout(600);
    expect(await page.evaluate(() => (window as any).__cls)).toBe(0);
    // Send key ⌘↩: ↩ is a newline in a reply, ⌘↩ / Ctrl↩ posts it.
    await setting(srv, 'send_key', 'mod_enter');
    await page.reload();
    await page.waitForSelector('.cd-app.cd-ready');
    const t1 = page.locator('.cd-thread[data-t="t1"]');
    await t1.locator('[data-act="reply"]').click();
    const ta = t1.locator('.cd-editor-reply textarea');
    await ta.pressSequentially('line one');
    await page.keyboard.press('Enter');
    await ta.pressSequentially('line two');
    await expect(ta).toHaveValue('line one\nline two');
    await expect(t1.locator('.cd-c')).toHaveCount(1);
    await page.keyboard.press('Control+Enter');
    await expect(t1.locator('.cd-c[data-c="t1.1"]:not(.cd-pending) .cd-cbody')).toContainText('line two');
    await expect(page.locator('#cd-send-key')).toHaveText('⌘↩');
  });
});
