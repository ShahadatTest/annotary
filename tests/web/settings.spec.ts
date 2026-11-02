// The Settings screen (CMD-23): the plan's example — Web plus an iPhone and an iPad simulated as API
// clients (a Token with `X-Annotary-Device` / `X-Annotary-Device-Info`) — All devices, one
// Device's own value, Mixed with the amber layers icon and its chain, made uniform again; Copy from
// another device behind its confirm; the scope menu from the keyboard; search; the sidebar and group
// URLs wide and narrow; the Server group for owners only; Profile and Devices. Every test that
// changes settings runs on a private server of its own (the owner's settings would change the pages
// the other specs look at).
import { test, expect, type Page } from '@playwright/test';
import { harness, spawnServer, type Spawned } from './harness';

const CROCKFORD = '0123456789ABCDEFGHJKMNPQRSTVWXYZ';
const newId = () => Array.from({ length: 26 }, () => CROCKFORD[Math.floor(Math.random() * 32)]).join('');

interface Dev { id: string; kind: string; name: string }

/** A request as an iOS Device would send it: the owner's Token plus the device headers. */
async function asDevice(srv: Spawned, d: Dev, method: string, path: string, body?: unknown): Promise<any> {
  const res = await fetch(srv.url + path, {
    method,
    headers: {
      Authorization: 'Bearer ' + srv.ownerToken,
      'X-Annotary-Device': d.id,
      'X-Annotary-Device-Info': `kind=${d.kind}; name=${encodeURIComponent(d.name)}`,
      'Content-Type': 'application/json',
    },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${text}`);
  return text ? JSON.parse(text) : null;
}

/** What `key` resolves to on Device `d`. */
async function resolved(srv: Spawned, d: Dev, key: string): Promise<unknown> {
  const j = await asDevice(srv, d, 'GET', '/api/v1/me/settings');
  return j.settings.find((s: { key: string }) => s.key === key).value;
}

async function withDevices(fn: (srv: Spawned, iphone: Dev, ipad: Dev) => Promise<void>) {
  const srv = await spawnServer();
  try {
    const iphone = { id: newId(), kind: 'iphone', name: 'iPhone 17 Pro' };
    const ipad = { id: newId(), kind: 'ipad', name: 'iPad Pro 13″' };
    await asDevice(srv, iphone, 'GET', '/api/v1/me/settings');
    await asDevice(srv, ipad, 'GET', '/api/v1/me/settings');
    await fn(srv, iphone, ipad);
  } finally {
    await srv.stop();
  }
}

async function open(page: Page, url: string) {
  await page.goto(url);
  await expect(page.locator('.cd-app.cd-set-ready')).toHaveCount(1);
}

const row = (page: Page, key: string) => page.locator(`#cd-set-${key}`);
const seg = (page: Page, key: string, value: string) => row(page, key).locator(`.cd-seg-o[data-value="${value}"]`);
const item = (page: Page, id: string) => page.locator(`#cd-set-scopemenu [data-id="${id}"]`);

/** Tick or untick one entry of "Applying to" (the menu closes with Escape). */
async function pick(page: Page, id: string) {
  await page.locator('#cd-set-scopebtn').click();
  await item(page, id).click();
  await page.keyboard.press('Escape');
  await expect(page.locator('#cd-set-scopemenu')).toHaveCount(0);
}

test.describe('Settings: Applying to, Mixed and the chain', () => {
  // spec: S29-21
  test('All devices → Follow system; Web alone → Light; back on All devices it is Mixed with the amber layers icon and the chain; a value makes it uniform', async ({ page }) => {
    await withDevices(async (srv, iphone, ipad) => {
      // The iPhone has its own Dark, so the first paint is already Mixed.
      await asDevice(srv, iphone, 'PUT', '/api/v1/me/settings', { key: 'appearance', value: 'dark', scope: 'devices', devices: [iphone.id] });
      await open(page, srv.url + '/settings/appearance');
      await expect(page.locator('#cd-set-scopename')).toHaveText('All devices');
      await expect(row(page, 'appearance').locator('.cd-set-mixed')).toBeVisible();
      await expect(row(page, 'appearance').locator('[aria-checked=true]')).toHaveCount(0);

      // All devices → Follow system: every Device follows it.
      await seg(page, 'appearance', 'system').click();
      await expect(seg(page, 'appearance', 'system')).toHaveAttribute('aria-checked', 'true');
      await expect(row(page, 'appearance').locator('.cd-set-mixed')).toBeHidden();
      await expect(row(page, 'appearance').locator('.cd-set-state')).toBeEmpty();
      expect(await resolved(srv, iphone, 'appearance')).toBe('system');
      expect(await resolved(srv, ipad, 'appearance')).toBe('system');

      // Web alone → Light: Web gets its own value, the others keep following.
      await pick(page, 'web');
      await expect(page.locator('#cd-set-scopename')).toHaveText('Web');
      await seg(page, 'appearance', 'light').click();
      await expect(seg(page, 'appearance', 'light')).toHaveAttribute('aria-checked', 'true');
      await expect(row(page, 'appearance').locator('.cd-set-state')).toBeEmpty();
      expect(await resolved(srv, iphone, 'appearance')).toBe('system');

      // Back on All devices: Mixed — no value selected, the pill, the amber icon, "Mixed" to a screen reader.
      await pick(page, 'all');
      const ap = row(page, 'appearance');
      await expect(ap.locator('.cd-set-mixed')).toBeVisible();
      await expect(ap.locator('.cd-set-mixed')).toHaveText('Mixed');
      await expect(ap.locator('[aria-checked=true]')).toHaveCount(0);
      await expect(ap.getByRole('radiogroup', { name: 'Appearance: Mixed' })).toBeVisible();
      const layers = ap.getByRole('button', { name: 'Appearance', exact: true });
      await expect(layers).toHaveClass(/cd-mixed/);
      expect(await layers.evaluate((el) => getComputedStyle(el).color)).toBe('rgb(180, 83, 9)');
      // A setting where they agree keeps a gray icon.
      await expect(row(page, 'text_size').locator('.cd-set-layers')).not.toHaveClass(/cd-mixed/);

      // The chain: All devices blank (they disagree), Web own Light with Reset, the others following.
      await layers.click();
      const chain = page.locator('#cd-set-chain');
      await expect(chain).toBeVisible();
      const lines = chain.locator('.cd-set-cl');
      await expect(lines).toHaveCount(4);
      await expect(lines.nth(0).locator('.cd-set-clname')).toHaveText('All devices');
      await expect(lines.nth(0).locator('.cd-set-clv')).toHaveText('');
      await expect(lines.nth(1)).toContainText('Web');
      await expect(lines.nth(1).locator('.cd-set-clnote')).toHaveText('own');
      await expect(lines.nth(1).locator('.cd-set-clv')).toHaveText('Light');
      await expect(lines.nth(1).locator('[data-act=reset-device]')).toBeVisible();
      for (const [i, name] of [[2, 'iPhone 17 Pro'], [3, 'iPad Pro 13″']] as const) {
        await expect(lines.nth(i).locator('.cd-set-clname')).toHaveText(name);
        await expect(lines.nth(i).locator('.cd-set-clnote')).toHaveText('follows All devices');
        await expect(lines.nth(i).locator('.cd-set-clv')).toHaveText('Follow system');
        await expect(lines.nth(i).locator('[data-act=reset-device]')).toHaveCount(0);
      }
      await page.keyboard.press('Escape');
      await expect(chain).toHaveCount(0);
      await expect(layers).toBeFocused();

      // A value with All devices applies to every Device: Mixed goes away.
      await seg(page, 'appearance', 'dark').click();
      await expect(ap.locator('.cd-set-mixed')).toBeHidden();
      await expect(seg(page, 'appearance', 'dark')).toHaveAttribute('aria-checked', 'true');
      await expect(layers).not.toHaveClass(/cd-mixed/);
      for (const d of [iphone, ipad]) expect(await resolved(srv, d, 'appearance')).toBe('dark');
      // …and the first paint of a reload says the same.
      await open(page, srv.url + '/settings/appearance');
      await expect(seg(page, 'appearance', 'dark')).toHaveAttribute('aria-checked', 'true');
      await expect(row(page, 'appearance').locator('.cd-set-mixed')).toBeHidden();
    });
  });

  // spec: S29-21
  test('Reset in the chain makes a Device follow All devices; swatches, Custom… (low contrast refused inline) and text size save at once', async ({ page }) => {
    await withDevices(async (srv, iphone) => {
      await asDevice(srv, iphone, 'PUT', '/api/v1/me/settings', { key: 'text_size', value: 130, scope: 'devices', devices: [iphone.id] });
      await open(page, srv.url + '/settings/appearance');
      const ts = row(page, 'text_size');
      await expect(ts.locator('.cd-set-mixed')).toBeVisible();
      await ts.locator('.cd-set-layers').click();
      await page.locator(`#cd-set-chain .cd-set-cl[data-device="${iphone.id}"] [data-act=reset-device]`).click();
      await expect(ts.locator('.cd-set-mixed')).toBeHidden();
      expect(await resolved(srv, iphone, 'text_size')).toBe(100);
      await page.keyboard.press('Escape');

      // Text size: A+ twice → 130%, the sample grows (15px × 1.3 ≈ 20px).
      await ts.locator('[data-act=larger]').click();
      await expect(ts.locator('.cd-tsize-v')).toHaveText('115%');
      await ts.locator('[data-act=larger]').click();
      await expect(ts.locator('.cd-tsize-v')).toHaveText('130%');
      await expect(ts.locator('.cd-set-sample-size')).toHaveCSS('font-size', '20px');
      await expect(ts.locator('.cd-set-state')).toBeEmpty();
      expect(await resolved(srv, iphone, 'text_size')).toBe(130);

      // A swatch, named for screen readers.
      const hl = row(page, 'highlight_light');
      await hl.getByRole('radio', { name: 'Green' }).click();
      await expect(hl.getByRole('radio', { name: 'Green' })).toHaveAttribute('aria-checked', 'true');
      await expect(hl.locator('.cd-set-mark')).toHaveCSS('background-color', 'rgb(200, 242, 208)');
      expect(await resolved(srv, iphone, 'highlight_light')).toBe('green');
      // The dark sample sits on its fixed black ground with black highlighted text.
      const dk = row(page, 'highlight_dark');
      await expect(dk.locator('.cd-set-sample-dark')).toHaveCSS('background-color', 'rgb(0, 0, 0)');
      await expect(dk.locator('.cd-set-mark')).toHaveCSS('color', 'rgb(0, 0, 0)');

      // Custom…: too little contrast is refused with the ratio, and the old value comes back.
      await hl.locator('[data-act=custom]').click();
      await hl.locator('.cd-set-hex').fill('#333333');
      await hl.locator('.cd-set-hex').press('Enter');
      await expect(hl.locator('.cd-set-state.cd-set-err')).toContainText(/contrast of [\d.]+:1; it needs 4\.5:1/);
      await expect(hl.getByRole('radio', { name: 'Green' })).toHaveAttribute('aria-checked', 'true');
      expect(await resolved(srv, iphone, 'highlight_light')).toBe('green');
      // A light custom color is taken.
      await hl.locator('.cd-set-hex').fill('#ffe0b0');
      await hl.locator('.cd-set-hex').press('Enter');
      await expect(hl.getByRole('radio', { name: 'Custom #ffe0b0' })).toHaveAttribute('aria-checked', 'true');
      expect(await resolved(srv, iphone, 'highlight_light')).toBe('#ffe0b0');
    });
  });

  // spec: S29-21
  test('a failed save shows "Couldn’t save. Try again." with the old value back; "Saving…" while it is in flight', async ({ page }) => {
    await withDevices(async (srv) => {
      await open(page, srv.url + '/settings/keyboard');
      let release!: () => void;
      const held = new Promise<void>((r) => { release = r; });
      await page.route('**/api/v1/me/settings', async (route) => {
        if (route.request().method() !== 'PUT') return route.continue();
        await held;
        await route.fulfill({ status: 500, body: '{"error":"internal","message":"boom"}' });
      });
      const ks = row(page, 'keyboard_shortcuts');
      await seg(page, 'keyboard_shortcuts', 'off').click();
      await expect(ks.locator('.cd-set-state')).toHaveText('Saving…');
      await expect(seg(page, 'keyboard_shortcuts', 'off')).toHaveAttribute('aria-checked', 'true');
      release();
      await expect(ks.locator('.cd-set-state.cd-set-err')).toHaveText('Couldn’t save. Try again.');
      await expect(seg(page, 'keyboard_shortcuts', 'on')).toHaveAttribute('aria-checked', 'true');
    });
  });

  test('a change made on another device shows without a reload, and says where', async ({ page }) => {
    await withDevices(async (srv, iphone) => {
      await open(page, srv.url + '/settings/appearance');
      await expect(page.locator('.cd-app.cd-live')).toHaveCount(1);
      // An iOS sync push carries the Device's own stamp.
      await asDevice(srv, iphone, 'PUT', '/api/v1/me/settings', {
        key: 'appearance', value: 'dark', scope: 'devices', devices: [iphone.id], stamp: `${Date.now()}-0-${iphone.id}`,
      });
      const ap = row(page, 'appearance');
      await expect(ap.locator('.cd-set-mixed')).toBeVisible();
      await expect(ap.locator('.cd-set-state')).toHaveText('Changed on iPhone 17 Pro');
    });
  });
});

test.describe('Settings: Copy from another device', () => {
  // spec: S29-21, S29-22
  test('asks first: Cancel, Escape and a click outside copy nothing; Copy copies every device setting the source has onto the chosen Devices', async ({ page }) => {
    await withDevices(async (srv, iphone, ipad) => {
      await asDevice(srv, iphone, 'PUT', '/api/v1/me/settings', { key: 'text_size', value: 130, scope: 'devices', devices: [iphone.id] });
      // The iPad's own Send key: an iPhone has no keyboard settings, so a copy from it leaves this alone.
      await asDevice(srv, ipad, 'PUT', '/api/v1/me/settings', { key: 'send_key', value: 'mod_enter', scope: 'devices', devices: [ipad.id] });
      await open(page, srv.url + '/settings/appearance');
      await pick(page, ipad.id);
      await expect(page.locator('#cd-set-scopename')).toHaveText('iPad Pro 13″');

      // The menu lists every other Device, by icon and name.
      await page.locator('#cd-set-copybtn').click();
      const menu = page.locator('#cd-set-copymenu');
      await expect(menu.getByRole('menuitem')).toHaveText(['Web', 'iPhone 17 Pro']);
      await menu.getByRole('menuitem', { name: 'iPhone 17 Pro' }).click();
      // …replaced in place by the confirm.
      await expect(menu).toHaveCount(0);
      const confirm = page.locator('#cd-set-confirm');
      await expect(confirm.locator('p')).toHaveText('Copy iPhone 17 Pro’s settings to iPad Pro 13″?');
      await expect(confirm.locator('b')).toHaveText(['iPhone 17 Pro', 'iPad Pro 13″']);
      await confirm.getByRole('button', { name: 'Cancel' }).click();
      await expect(confirm).toHaveCount(0);
      expect(await resolved(srv, ipad, 'text_size')).toBe(100);

      // Escape closes it, focus back on the button; a click outside too.
      await page.locator('#cd-set-copybtn').click();
      await menu.getByRole('menuitem', { name: 'iPhone 17 Pro' }).click();
      await page.keyboard.press('Escape');
      await expect(confirm).toHaveCount(0);
      await expect(page.locator('#cd-set-copybtn')).toBeFocused();
      await page.locator('#cd-set-copybtn').click();
      await menu.getByRole('menuitem', { name: 'iPhone 17 Pro' }).click();
      await page.locator('.cd-set-title').click();
      await expect(confirm).toHaveCount(0);
      expect(await resolved(srv, ipad, 'text_size')).toBe(100);

      // Copy: every device setting the iPhone has; the iPad keeps its own Keyboard settings.
      await page.locator('#cd-set-copybtn').click();
      await menu.getByRole('menuitem', { name: 'iPhone 17 Pro' }).click();
      await confirm.getByRole('button', { name: 'Copy' }).click();
      await expect(confirm).toHaveCount(0);
      await expect(row(page, 'text_size').locator('.cd-tsize-v')).toHaveText('130%');
      expect(await resolved(srv, ipad, 'text_size')).toBe(130);
      expect(await resolved(srv, ipad, 'send_key')).toBe('mod_enter');

      // With All devices chosen the confirm says so.
      await pick(page, 'all');
      await page.locator('#cd-set-copybtn').click();
      await menu.getByRole('menuitem', { name: 'iPhone 17 Pro' }).click();
      await expect(confirm.locator('p')).toHaveText('Copy iPhone 17 Pro’s settings to All devices?');
    });
  });
});

test.describe('Settings: the scope menu from the keyboard', () => {
  // spec: S29-21
  test('Enter opens it, arrows move, Space and Enter toggle, the last Device unticked goes back to All devices, Escape returns focus', async ({ page }) => {
    await withDevices(async (srv, iphone) => {
      await open(page, srv.url + '/settings/appearance');
      const btn = page.locator('#cd-set-scopebtn');
      await btn.focus();
      await page.keyboard.press('Enter');
      const menu = page.getByRole('menu', { name: 'Applying to' });
      await expect(menu).toBeVisible();
      await expect(btn).toHaveAttribute('aria-expanded', 'true');
      const items = menu.getByRole('menuitemcheckbox');
      await expect(items).toHaveCount(4);
      // All devices, then the current Device first ("· this device"), with last seen.
      await expect(items.nth(0)).toContainText('All devices');
      await expect(items.nth(1)).toContainText('Web');
      await expect(items.nth(1)).toContainText('this device');
      await expect(items.nth(1)).toContainText('Last seen now');
      await expect(items.nth(0)).toBeFocused();
      await expect(items.nth(0)).toHaveAttribute('aria-checked', 'true');
      await page.keyboard.press('ArrowDown');
      await expect(items.nth(1)).toBeFocused();
      await page.keyboard.press(' ');
      await expect(items.nth(1)).toHaveAttribute('aria-checked', 'true');
      await expect(items.nth(0)).toHaveAttribute('aria-checked', 'false');
      await expect(btn).toContainText('Web');
      await page.keyboard.press('ArrowDown');
      await expect(items.nth(2)).toBeFocused();
      await page.keyboard.press('Enter');
      await expect(items.nth(2)).toHaveAttribute('aria-checked', 'true');
      await expect(btn).toContainText('Web, iPhone 17 Pro');
      await page.keyboard.press('ArrowUp');
      await page.keyboard.press(' ');
      await page.keyboard.press('ArrowDown');
      await page.keyboard.press(' ');
      await expect(items.nth(0)).toHaveAttribute('aria-checked', 'true');
      await expect(btn).toContainText('All devices');
      await page.keyboard.press('End');
      await expect(items.nth(3)).toBeFocused();
      await page.keyboard.press('ArrowDown');
      await expect(items.nth(0)).toBeFocused();
      await page.keyboard.press('Escape');
      await expect(menu).toHaveCount(0);
      await expect(btn).toBeFocused();
      await expect(btn).toHaveAttribute('aria-expanded', 'false');
      void iphone;
    });
  });
});

test.describe('Settings: sidebar, URLs, search', () => {
  // spec: S29-20
  test('wide: the sidebar and one URL per group; /settings is Profile', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await open(page, harness().url + '/settings');
    await expect(page.locator('#cd-nav-settings')).toHaveAttribute('aria-current', 'page');
    await expect(page.locator('#cd-set-side')).toBeVisible();
    await expect(page.locator('.cd-set-title')).toHaveText('Profile');
    await expect(page.locator('#cd-set-groups .cd-set-gl')).toHaveText(['Profile', 'Devices', 'Appearance', 'Threads', 'Documents', 'Keyboard', 'Date and time']);
    await expect(page.locator('.cd-set-back')).toBeHidden();
    await page.locator('.cd-set-gl[data-group=appearance]').click();
    await expect(page).toHaveURL(harness().url + '/settings/appearance');
    await expect(page.locator('.cd-set-title')).toHaveText('Appearance');
    await expect(page.locator('.cd-set-gl[data-group=appearance]')).toHaveAttribute('aria-current', 'page');
    await expect(page.locator('#cd-set-scope')).toBeVisible();
    for (const g of ['threads', 'documents', 'date-and-time']) {
      await page.locator(`.cd-set-gl[data-group="${g}"]`).click();
      await expect(page).toHaveURL(harness().url + '/settings/' + g);
      await expect(page.locator('#cd-set-scope')).toHaveCount(0); // user settings only: no picker
      await expect(page.locator('.cd-set-yours')).toContainText('These settings are yours on');
    }
    await page.locator('.cd-set-gl[data-group=keyboard]').click();
    await expect(page.locator('#cd-set-scope')).toBeVisible();
    // No help line under a setting's name.
    expect((await page.locator('.cd-set-row .cd-set-label').allTextContents()).join(' | ')).toBe('Keyboard shortcuts | Send key for replies');
  });

  // spec: S29-20
  test('narrow: /settings is the group list, a group has "‹ Settings"', async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await open(page, harness().url + '/settings');
    await expect(page.locator('.cd-set-listtitle')).toBeVisible();
    await expect(page.locator('#cd-set-main')).toBeHidden();
    await expect(page.locator('#cd-set-q')).toBeHidden();
    await expect(page.locator('.cd-set-gl[data-group=profile] .cd-set-chev')).toBeVisible();
    await page.locator('.cd-set-gl[data-group=appearance]').click();
    await expect(page).toHaveURL(harness().url + '/settings/appearance');
    await expect(page.locator('#cd-set-side')).toBeHidden();
    await expect(page.locator('.cd-set-title')).toHaveText('Appearance');
    // "Copy from another device" wraps onto its own line under "Applying to".
    const a = await page.locator('#cd-set-scopebtn').boundingBox();
    const c = await page.locator('#cd-set-copybtn').boundingBox();
    expect(c!.y).toBeGreaterThan(a!.y + a!.height - 1);
    // A row stacks: label above the control.
    const l = await page.locator('#cd-set-appearance .cd-set-label').boundingBox();
    const s = await page.locator('#cd-set-appearance .cd-seg').boundingBox();
    expect(s!.y).toBeGreaterThan(l!.y + l!.height - 1);
    await page.locator('.cd-set-back').click();
    await expect(page).toHaveURL(harness().url + '/settings');
    await expect(page.locator('#cd-set-groups')).toBeVisible();
    // Wide again: the same URL shows Profile beside the sidebar.
    await page.setViewportSize({ width: 1280, height: 900 });
    await expect(page.locator('#cd-set-main')).toBeVisible();
    await expect(page.locator('.cd-set-listtitle')).toBeHidden();
  });

  // spec: S29-26
  test('search filters settings by name and help line across groups, with no request', async ({ page }) => {
    await open(page, harness().url + '/settings/appearance');
    const requests: string[] = [];
    page.on('request', (r) => requests.push(r.url()));
    const visible = () => page.locator('.cd-set-gl:visible');
    await page.locator('#cd-set-q').fill('highlight');
    await expect(visible()).toHaveText(['Appearance']);
    await expect(row(page, 'appearance')).toBeHidden();
    await expect(row(page, 'highlight_light')).toBeVisible();
    await expect(row(page, 'highlight_dark')).toBeVisible();
    // A help line matches too ("Where new Documents go…" is Default Vault's).
    await page.locator('#cd-set-q').fill('new documents');
    await expect(visible()).toHaveText(['Documents']);
    await page.locator('#cd-set-q').fill('reply');
    await expect(visible()).toHaveText(['Keyboard']);
    // The owner's Server pages are searched too.
    await page.locator('#cd-set-q').fill('vault');
    await expect(visible()).toHaveText(['Documents', 'Vaults']);
    await expect(page.locator('#cd-set-serverhead')).toBeVisible();
    await page.locator('#cd-set-q').fill('zzzz');
    await expect(visible()).toHaveCount(0);
    await expect(page.locator('#cd-set-nomatch')).toBeVisible();
    await page.locator('#cd-set-q').press('Escape');
    await expect(visible()).toHaveCount(10);
    await expect(row(page, 'appearance')).toBeVisible();
    expect(requests).toEqual([]);
  });
});

test.describe('Settings: the Server group', () => {
  // spec: S29-20, S12-05
  test('owners see the Server group; a Member does not and gets the 403 page there', async ({ page }) => {
    await open(page, harness().url + '/settings/profile');
    await expect(page.locator('#cd-set-serverhead')).toHaveText('Server');
    await expect(page.locator('.cd-set-server .cd-set-gl')).toHaveText(['Members', 'Vaults', 'Server settings']);
    await page.locator('.cd-set-gl[data-group="server/vaults"]').click();
    await expect(page).toHaveURL(harness().url + '/settings/server/vaults');
    await expect(page.locator('.cd-set-title')).toHaveText('Vaults');
    await expect(page.locator('#cd-vaults')).toBeVisible();

    await page.setExtraHTTPHeaders({ Authorization: 'Bearer ' + harness().agent });
    await open(page, harness().url + '/settings');
    await expect(page.locator('#cd-nav-settings')).toBeVisible();
    await expect(page.locator('.cd-set-title')).toHaveText('Profile');
    await expect(page.locator('#cd-set-serverhead')).toHaveCount(0);
    await expect(page.locator('.cd-set-server')).toHaveCount(0);
    const res = await page.goto(harness().url + '/settings/server/members');
    expect(res!.status()).toBe(403);
    await expect(page.locator('body')).toContainText('owner-only');
  });
});

test.describe('Settings: Profile and Devices', () => {
  // spec: S29-23
  test('Profile renames the Member on blur or Enter; Devices lists icon, name and last seen, with Rename and Forget for iOS Devices only', async ({ page }) => {
    await withDevices(async (srv, iphone, ipad) => {
      await open(page, srv.url + '/settings/profile');
      await expect(page.locator('.cd-set-yours')).toContainText('These settings are yours on');
      await expect(page.locator('.cd-set-yours b')).toHaveText(new URL(srv.url).host);
      const name = page.locator('#cd-set-name-in');
      await expect(name).toHaveValue('Rowan Vale');
      await expect(page.locator('.cd-set-handle')).toHaveText('@rowan');
      await expect(page.locator('.cd-set-row', { hasText: 'Role' })).toContainText('Owner');
      await name.fill('Rowan A.');
      await name.press('Enter');
      await expect(page.locator('#cd-set-display_name .cd-set-state')).toBeEmpty();
      await expect(page.locator('.cd-me .cd-member')).toHaveText('Rowan A.');
      const me = await (await fetch(srv.url + '/api/v1/me', { headers: { Authorization: 'Bearer ' + srv.ownerToken } })).json();
      expect(me.name ?? me.display_name).toBe('Rowan A.');
      await name.fill('Rowan Vale');
      await page.locator('.cd-set-title').click(); // blur saves
      await expect(page.locator('.cd-me .cd-member')).toHaveText('Rowan Vale');

      await open(page, srv.url + '/settings/devices');
      const cards = page.locator('.cd-dev');
      await expect(cards.locator('.cd-dev-name')).toHaveText(['Web', 'iPhone 17 Pro', 'iPad Pro 13″']);
      const web = page.locator('.cd-dev[data-device=web]');
      await expect(web.locator('.cd-dev-this')).toHaveText('this device');
      await expect(web.locator('.cd-dev-seen')).toHaveText('Last seen now');
      await expect(web.locator('[data-act]')).toHaveCount(0); // no Rename, no Forget for Web
      await expect(web.locator('.cd-dev-icon svg')).toHaveCount(1);
      const ph = page.locator(`.cd-dev[data-device="${iphone.id}"]`);
      await expect(ph.locator('.cd-dev-this')).toHaveCount(0);
      await expect(ph.locator('.cd-dev-seen')).toHaveText(/^Last seen (now|\d+ minutes? ago)$/);
      // Rename inline.
      await ph.getByRole('button', { name: 'Rename' }).click();
      await ph.locator('.cd-dev-inline input').fill('Rowan’s iPhone');
      await ph.locator('.cd-dev-inline input').press('Enter');
      await expect(ph.locator('.cd-dev-name')).toHaveText('Rowan’s iPhone');
      const list = await asDevice(srv, ipad, 'GET', '/api/v1/me/devices');
      expect(list.devices.find((d: Dev) => d.id === iphone.id).name).toBe('Rowan’s iPhone');
      // Forget inline yes / no: no first, then yes.
      const pad = page.locator(`.cd-dev[data-device="${ipad.id}"]`);
      await pad.getByRole('button', { name: 'Forget' }).click();
      await expect(pad.locator('.cd-dev-inline')).toContainText('Forget iPad Pro 13″?');
      await pad.getByRole('button', { name: 'Cancel' }).click();
      await expect(pad.locator('.cd-dev-inline')).toBeHidden();
      await pad.getByRole('button', { name: 'Forget' }).click();
      await pad.locator('[data-act=forget-yes]').click();
      await expect(pad).toHaveCount(0);
      // (Forget revoked the Token last used from the iPad — the harness's owner Token — so check
      // through the page, which is Web on its own session.)
      await open(page, srv.url + '/settings/devices');
      await expect(page.locator('.cd-dev .cd-dev-name')).toHaveText(['Web', 'Rowan’s iPhone']);
    });
  });
});
