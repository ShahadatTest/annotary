// M3b: the title bar's icon in a browser. Its markup, the routes and the `<link>` tags are
// `crates/annotary/tests/brand.rs`; this is what only layout can tell: the size, what it sits next
// to, both appearances, and the narrow header. Real data is `verify/scenarios/brand.spec.ts`.
import { test, expect, type Page } from '@playwright/test';
import { harness } from './harness';

async function theIcon(page: Page) {
  const brand = page.locator('.cd-hdr .cd-brand');
  await expect(brand).toHaveCount(1);
  await expect(brand).toHaveAttribute('href', '/');
  const box = await brand.locator('svg').boundingBox();
  expect(box && [box.width, box.height]).toEqual([18, 18]);
  // It paints the ink ground, whichever palette the page is in.
  expect(await brand.locator('svg rect').first().getAttribute('fill')).toBe('#1c1c1e');
  return box!;
}

for (const scheme of ['light', 'dark'] as const) {
  // spec: S31-05
  test(`the list: icon then wordmark, ${scheme}`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto(harness().url + '/');
    const icon = await theIcon(page);
    const name = await page.locator('.cd-hdr .cd-name').boundingBox();
    expect(name!.x).toBeGreaterThan(icon.x + icon.width);
    await expect(page.locator('.cd-hdr .cd-name')).toHaveText('annotary');
  });

  // spec: S31-05
  test(`a Document: icon then crumb, no wordmark, ${scheme}`, async ({ page }) => {
    const h = harness();
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto(`${h.url}/d/${h.doc.id}`);
    const icon = await theIcon(page);
    const crumb = await page.locator('.cd-hdr a.cd-crumb').boundingBox();
    expect(crumb!.x).toBeGreaterThan(icon.x + icon.width);
    await expect(page.locator('.cd-hdr .cd-name')).toHaveText(h.doc.name);
  });
}

// spec: S31-05
test('the narrow Document header keeps the icon whole, its overlay still opens, and the icon goes to the list', async ({ page }) => {
  const h = harness();
  await page.setViewportSize({ width: 390, height: 800 });
  await page.goto(`${h.url}/d/${h.doc.id}`);
  await page.waitForSelector('.cd-app.cd-ready');
  await theIcon(page);
  const app = page.locator('.cd-app');
  await expect(app).not.toHaveClass(/cd-overlay-open/);
  await page.locator('.cd-hdr .cd-name').click();
  await expect(app).toHaveClass(/cd-overlay-open/);
  await page.locator('.cd-hdr .cd-brand').click();
  await expect(page).toHaveURL(h.url + '/');
  await expect(page.locator('.cd-hdr .cd-name')).toHaveText('annotary');
});

// spec: S31-03
test('a browser loads the favicon on the login page, before sign-in', async ({ page }) => {
  const icons: string[] = [];
  page.on('response', (r) => {
    if (/favicon|icon\.svg/.test(r.url()) && r.status() === 200) icons.push(r.url());
  });
  await page.goto(harness().pwUrl + '/login');
  const ok = await page.evaluate(async () => {
    const hrefs = [...document.querySelectorAll('link[rel~="icon"], link[rel="manifest"], link[rel="apple-touch-icon"]')]
      .map((l) => l.getAttribute('href')!);
    return Promise.all(hrefs.map(async (h) => [h, (await fetch(h, { credentials: 'omit' })).status]));
  });
  expect(ok).toEqual([
    ['/favicon.ico', 200],
    ['/icon.svg', 200],
    ['/apple-touch-icon.png', 200],
    ['/manifest.webmanifest', 200],
  ]);
  expect(icons.length).toBeGreaterThan(0);
});
