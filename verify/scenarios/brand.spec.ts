// M3b on the real data: the favicon set answers before sign-in, and the Annotary icon sits in the
// title bar of a real list page and a real Document page, in light and dark. The Document is found
// by what it holds: the first one on the list whose Markdown has a plain heading, which must then
// be on its page. Screenshots go to VERIFY_OUT for the pull request.
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test } from '@playwright/test';
import { cli, cliJson, documents, expect, openDoc, shot, URL } from '../lib';

/** A request nobody has signed in on: the server treats a forwarded request as remote (CMD-17). */
const SIGNED_OUT = { 'X-Forwarded-For': '100.64.0.9', Accept: 'text/html' };

async function signedOut(path: string) {
  const r = await fetch(`${URL}${path}`, { headers: SIGNED_OUT, redirect: 'manual' });
  return { status: r.status, headers: r.headers, body: Buffer.from(await r.arrayBuffer()) };
}

const PNG = Buffer.from([0x89, 0x50, 0x4e, 0x47]);

/** The Document's Markdown, through the CLI's `pull` into the copy's own folder. */
function markdownOf(id: string): string {
  const out = fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'brand-pull-'));
  cli(['pull', id, '--out', out, '--force']);
  const file = fs.readdirSync(out).find((f) => f.endsWith('.md'));
  return file ? fs.readFileSync(path.join(out, file), 'utf8') : '';
}

test('the favicon routes answer before sign-in', async () => {
  // The request really is signed out: the live data stays behind the login page.
  const home = await signedOut('/');
  expect(home.status).toBe(303);
  expect(home.headers.get('location')).toMatch(/^\/login/);

  const ico = await signedOut('/favicon.ico');
  expect(ico.status).toBe(200);
  expect(ico.headers.get('content-type')).toBe('image/x-icon');
  expect(ico.body.readUInt16LE(2)).toBe(1); // an icon
  expect(ico.body.readUInt16LE(4)).toBe(2); // 32 and 16
  expect([ico.body[6], ico.body[22]]).toEqual([32, 16]);

  const svg = await signedOut('/icon.svg');
  expect(svg.status).toBe(200);
  expect(svg.headers.get('content-type')).toBe('image/svg+xml');
  expect(svg.body.toString()).toMatch(/^<svg .*#f472b6.*<\/svg>\s*$/s);

  const touch = await signedOut('/apple-touch-icon.png');
  expect(touch.status).toBe(200);
  expect(touch.headers.get('content-type')).toBe('image/png');
  expect(touch.body.subarray(0, 4)).toEqual(PNG);
  expect(touch.body.readUInt32BE(16)).toBe(180);

  const manifest = await signedOut('/manifest.webmanifest');
  expect(manifest.status).toBe(200);
  expect(manifest.headers.get('content-type')).toBe('application/manifest+json');
  const m = JSON.parse(manifest.body.toString());
  expect(m.name).toBe('Annotary');
  expect(m.icons.length).toBe(3);
  for (const icon of m.icons) {
    const r = await signedOut(icon.src);
    expect(r.status, icon.src).toBe(200);
    expect(r.body.subarray(0, 4), icon.src).toEqual(PNG);
    expect(r.body.readUInt32BE(16), icon.src).toBe(Number(icon.sizes.split('x')[0]));
  }
  for (const r of [ico, svg, touch, manifest]) expect(r.headers.get('set-cookie')).toBeNull();

  // The login page, the page a signed-out browser gets, links all of them.
  const login = (await signedOut('/login')).body.toString();
  for (const href of ['/favicon.ico', '/icon.svg', '/apple-touch-icon.png', '/manifest.webmanifest']) {
    expect(login).toContain(`href="${href}"`);
  }
});

test('the title bar wears the icon on real pages, in light and dark', async ({ page }) => {
  await page.goto('/'); // the browser's Device exists from here on

  // A real Document by its content: on the list's first page, with a plain heading in its Markdown
  // (outside code fences, where `#` is a comment). Its head Version must predate this run: other
  // scenarios push into the same copy (agent-flow's `verify-live-agent`), and that proves nothing.
  const started = Date.now() - 10 * 60 * 1000;
  let found: { id: string; name: string; heading: string } | null = null;
  for (const d of documents()) {
    if ((await page.locator(`a[href="/d/${d.id}"]`).count()) === 0) continue;
    const versions = cliJson<{ number: number; at: string }[]>(['versions', d.id]) ?? [];
    const head = versions.reduce((a, b) => (b.number > a.number ? b : a));
    if (!(Date.parse(head.at) < started)) continue;
    const md = markdownOf(d.id).replace(/^```.*?^```/gms, '');
    const heading = md.match(/^#{1,2} +([A-Za-z][A-Za-z0-9 ]{3,60})$/m)?.[1]?.trim();
    if (heading) { found = { id: d.id, name: d.name, heading }; break; }
  }
  expect(found, 'the live data has a Document on the list with a heading').not.toBeNull();
  test.info().annotations.push({ type: 'data', description: `${found!.id}: ${found!.heading}` });

  const icon = page.locator('.cd-hdr .cd-brand svg.cd-icon');

  const checkIcon = async () => {
    await expect(icon).toHaveCount(1);
    expect(await icon.boundingBox().then((b) => b && [b.width, b.height])).toEqual([18, 18]);
    await expect(page.locator('.cd-hdr .cd-brand')).toHaveAttribute('href', '/');
  };

  // The appearance goes back at the end: the copy is shared with the scenarios after this one.
  const before = cliJson<{ value: string; source: string }>(['settings', 'get', 'appearance', '--device', 'Web']);
  try {
    for (const mode of ['light', 'dark'] as const) {
      cli(['settings', 'set', 'appearance', mode, '--device', 'Web']);
      const ground = mode === 'dark' ? 'rgb(0, 0, 0)' : 'rgb(255, 255, 255)';

      await page.setViewportSize({ width: 1280, height: 900 });
      await page.goto('/');
      await expect(page.locator('.cd-app')).toHaveClass(new RegExp(`cd-theme-${mode}`));
      await expect(page.locator('.cd-app')).toHaveCSS('background-color', ground);
      await checkIcon();
      await expect(page.locator('.cd-hdr .cd-name')).toHaveText('annotary');
      await expect(page.locator(`a[href="/d/${found!.id}"]`).first()).toBeVisible();
      await shot(page, `brand-list-${mode}`);

      await openDoc(page, found!.id);
      await expect(page.locator('.cd-app')).toHaveClass(new RegExp(`cd-theme-${mode}`));
      await expect(page.locator('.cd-app')).toHaveCSS('background-color', ground);
      await checkIcon();
      await expect(page.locator('.cd-hdr a.cd-crumb')).toBeVisible();
      await expect(page.locator('.cd-hdr .cd-name')).toHaveText(found!.name);
      await expect(page.locator('.cd-doc').getByRole('heading', { name: found!.heading }).first()).toBeVisible();
      await shot(page, `brand-document-${mode}`);
    }
  } finally {
    if (before.source === 'device') cli(['settings', 'set', 'appearance', before.value, '--device', 'Web']);
    else cli(['settings', 'reset', 'appearance', '--device', 'Web']);
  }
});
