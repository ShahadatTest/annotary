// Owner screens & login page (ticket 12): the Members screen (Add agent → a Token the CLI
// accepts at once; per-Token last_used; mint / revoke / rename / disable; role + Grants behind
// Advanced), the Settings screen (local password → login page → remembered session; the default
// Actions editor → a new Document pulls them as `actions.available` with `inherited: true`; Vault
// create / rename / delete; auto-purge), and owner-only gating for a Member. Tests that change
// Tenant-wide state (the password, the auto-purge setting) run on a private server of their own.
import { test, expect, type Browser, type Page } from '@playwright/test';
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { execFileSync } from 'node:child_process';
import { binary, cli, harness, spawnServer } from './harness';

const h = () => harness();
const uniq = () => Math.random().toString(36).slice(2, 8);

/** The CLI's exit code (0 when it succeeds) as `token` against the main server. */
function cliExit(token: string, args: string[]): number {
  const env: NodeJS.ProcessEnv = { ...process.env, HOME: h().homes[0], USER: 'rowan', ANNOTARY_TOKEN: token, ANNOTARY_SERVER: h().url };
  for (const k of ['ANNOTARY_HOME', 'ANNOTARY_PROFILE', 'ANNOTARY_TTY', 'ANNOTARY_PASSWORD']) delete env[k];
  try {
    execFileSync(binary(), args, { env, stdio: ['ignore', 'pipe', 'pipe'] });
    return 0;
  } catch (e) {
    return (e as { status?: number }).status ?? -1;
  }
}

const card = (page: Page, handle: string) => page.locator(`.cd-mem[data-handle="${handle}"]`);

test.describe('Members', () => {
  // spec: S12-01, S12-02
  test('Add agent → the Token line (copyable) works from the CLI at once; the card shows handle, name, role, grants, auth hint, Tokens with last used; mint / revoke / rename / Advanced / disable', async ({ page, context }) => {
    await context.grantPermissions(['clipboard-read', 'clipboard-write']);
    // CMD-23: /members 308s to the Members page under Settings → Server.
    await page.goto(h().url + '/members');
    await expect(page).toHaveURL(h().url + '/settings/server/members');
    await expect(page.locator('#cd-nav-settings')).toHaveAttribute('aria-current', 'page');
    await expect(page.locator('.cd-set-gl[data-group="server/members"]')).toHaveAttribute('aria-current', 'page');
    // What is already there: the owner (used Token ⇒ a relative "last used"), an agent by Token.
    const rowan = card(page, 'rowan');
    await expect(rowan.locator('.cd-member')).toHaveText('Rowan Vale');
    await expect(rowan.locator('.cd-chip.cd-role')).toHaveText('owner');
    await expect(rowan.locator('.cd-authhint')).toHaveText('authenticates with a Token');
    await expect(rowan.locator('.cd-grants-summary')).toHaveText('every Vault: read, comment, action, write');
    await expect(rowan.locator('.cd-tok').first().locator('.cd-lastused time')).toBeVisible();
    await expect(card(page, 'scribe').locator('.cd-member')).toHaveText('Scribe');
    await expect(card(page, 'scribe').locator('.cd-auth')).toHaveAttribute('title', 'authenticates with a Token');
    await expect(page.locator('.cd-memlist')).not.toContainText(/\bbot\b/i);
    // The Grants / role editor is behind Advanced, closed by default.
    await expect(rowan.locator('details.cd-advanced')).not.toHaveAttribute('open', '');
    await expect(rowan.locator('details.cd-advanced [data-field=role]')).toBeHidden();

    // ---- Add agent: one handle → the Token shown once as the env-var line ----
    const handle = 'agent-' + uniq();
    await page.locator('#cd-addagent').click();
    await page.locator('#cd-add-handle').fill(handle);
    await page.locator('#cd-add-name').fill('Agent Twelve');
    await page.locator('#cd-add-handle').press('Enter');
    const panel = page.locator(`.cd-tokonce[data-handle="${handle}"]`);
    await expect(panel).toBeVisible();
    const line = (await panel.locator('.cd-tokvalue').textContent())!;
    expect(line).toMatch(/^ANNOTARY_TOKEN=ant_\S+$/);
    const token = line.slice('ANNOTARY_TOKEN='.length);
    await panel.locator('[data-act=copy]').click();
    await expect(panel.locator('.cd-copied')).toBeVisible();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(line);
    const me = card(page, handle);
    await expect(me.locator('.cd-member')).toHaveText('Agent Twelve');
    await expect(me.locator('.cd-handle')).toHaveText('@' + handle);
    await expect(me.locator('.cd-chip.cd-role')).toHaveText('member');
    await expect(me.locator('.cd-grants-summary')).toHaveText('every Vault: read, comment, action, write');
    await expect(me.locator('.cd-tok')).toHaveCount(1);
    await expect(me.locator('.cd-tok .cd-lastused')).toHaveText('never');
    await expect(page.locator('#cd-memcount')).toHaveText('4 active');
    // The Token works from the CLI right away (the `*` Grant covers the seeded Document's Vault)…
    expect(cli(token, ['status', h().doc.id])).toContain('v' + h().doc.head);
    // …and the screen shows the use on the next draw (per-Token last_used_at).
    await page.reload();
    await expect(me.locator('.cd-tok .cd-lastused time')).toBeVisible();
    await expect(me.locator('.cd-tok .cd-lastused')).not.toHaveText('never');
    await expect(page.locator(`.cd-tokonce[data-handle="${handle}"]`)).toHaveCount(0); // shown once

    // ---- mint a named Token, revoke it ----
    await me.locator('[data-act=mint]').click();
    await me.locator('[data-slot=mint] [data-f=name]').fill('laptop');
    await me.locator('[data-slot=mint] [data-inline=save]').click();
    const panel2 = me.locator('.cd-tokonce');
    await expect(panel2).toBeVisible();
    const line2 = (await panel2.locator('.cd-tokvalue').textContent())!;
    expect(line2).toMatch(/^ANNOTARY_TOKEN=ant_/);
    expect(line2).not.toBe(line);
    await expect(me.locator('.cd-tok')).toHaveCount(2);
    const laptop = me.locator('.cd-tok', { hasText: 'laptop' });
    await expect(laptop).toHaveCount(1);
    expect(cliExit(line2.slice('ANNOTARY_TOKEN='.length), ['status', h().doc.id])).toBe(0);
    await laptop.locator('[data-act=revoke]').click();
    await laptop.locator('[data-act=revoke-yes]').click();
    await expect(me.locator('.cd-tok')).toHaveCount(1);
    await expect(me.locator('.cd-memtokhead')).toContainText('1 revoked');
    expect(cliExit(line2.slice('ANNOTARY_TOKEN='.length), ['status', h().doc.id])).toBe(3);

    // ---- rename (display name only — the handle never changes) ----
    await me.locator('[data-act=rename]').click();
    const nameInput = me.locator('[data-slot=rename] input');
    await expect(nameInput).toHaveValue('Agent Twelve');
    await nameInput.fill('Agent XII');
    await nameInput.press('Enter');
    await expect(me.locator('.cd-member')).toHaveText('Agent XII');
    await expect(me.locator('.cd-handle')).toHaveText('@' + handle);
    expect(cli(h().owner, ['member', 'list'])).toMatch(new RegExp(`@${handle}\\tAgent XII\\tmember`));

    // ---- Advanced: Grants narrowed, role changed ----
    await me.locator('details.cd-advanced summary').click();
    const star = me.locator('.cd-grantrow[data-vault="*"]');
    await star.locator('input[data-right=action]').uncheck();
    await star.locator('input[data-right=write]').uncheck();
    await me.locator('.cd-grantrow[data-vault="notes"] input[data-right=write]').check();
    await me.locator('[data-act=grants]').click();
    await expect(me.locator('.cd-grants-summary')).toHaveText('every Vault: read, comment · notes: write');
    await expect(me.locator('details.cd-advanced')).toHaveAttribute('open', '');
    // The CLI sees the new Rights: `action` is gone on notes (the Vault row overrides `*`).
    expect(cliExit(token, ['act', h().doc.id, 'approve'])).toBe(4);
    expect(cliExit(token, ['status', h().doc.id])).toBe(0);
    await me.locator('[data-field=role]').selectOption('owner');
    await me.locator('[data-act=role]').click();
    await expect(me.locator('.cd-chip.cd-role')).toHaveText('owner');
    await me.locator('[data-field=role]').selectOption('member');
    await me.locator('[data-act=role]').click();
    await expect(me.locator('.cd-chip.cd-role')).toHaveText('member');

    // ---- disable: the Token dies, the card says so, the handle stays ----
    await me.locator('[data-act=disable]').click();
    await me.locator('[data-slot=rename] [data-inline=yes]').click();
    await expect(me).toHaveClass(/cd-mem-disabled/);
    await expect(me.locator('.cd-chip.cd-disabled')).toHaveText('disabled');
    await expect(me.locator('[data-act=mint]')).toHaveCount(0);
    await expect(me.locator('details.cd-advanced')).toHaveCount(0);
    await expect(page.locator('#cd-memcount')).toHaveText('3 active');
    expect(cliExit(token, ['status', h().doc.id])).toBe(3);
    // A taken handle is refused in the form.
    await page.locator('#cd-addagent').click();
    await page.locator('#cd-add-handle').fill(handle);
    await page.locator('#cd-addform button[type=submit]').click();
    await expect(page.locator('#cd-addform .cd-err')).toContainText('already taken');
  });
});

test.describe('Settings', () => {
  // spec: S12-03, S12-06
  test('password set ⇒ the login page for a browser without a session ⇒ a remembered session; this session stays; remove ⇒ loopback trust again', async ({ browser }) => {
    const srv = await spawnServer();
    const contexts: Awaited<ReturnType<Browser['newContext']>>[] = [];
    const fresh = async () => {
      const c = await browser.newContext();
      contexts.push(c);
      return c.newPage();
    };
    try {
      const page = await fresh();
      await page.goto(srv.url + '/settings/server/general');
      const pw = page.locator('#cd-set-password');
      await expect(pw).toHaveAttribute('data-password-set', 'false');
      await expect(page.locator('form.cd-logout')).toHaveCount(0);
      await page.locator('#cd-pw-new').fill('s3cret');
      await page.locator('#cd-pw-again').fill('s3cretx');
      await page.locator('#cd-pw-set').click();
      await expect(pw.locator('.cd-err')).toContainText('differ');
      await page.locator('#cd-pw-again').fill('s3cret');
      await page.locator('#cd-pw-set').click();
      await expect(pw).toHaveAttribute('data-password-set', 'true');
      await expect(page.locator('#cd-pw-remove')).toBeVisible();
      await expect(page.locator('#cd-pwform .cd-ok')).toContainText('Password set');
      // This browser keeps its session (and now sees Sign out); the screens still work.
      await page.goto(srv.url + '/settings/server/members');
      expect(page.url()).toBe(srv.url + '/settings/server/members');
      await expect(page.locator('form.cd-logout button')).toHaveText('Sign out');
      await expect(card(page, 'rowan').locator('.cd-authhint')).toHaveText('signs in with the local password');
      // A browser without a session: the login page (restyled on the shared shell), `next` kept.
      const p2 = await fresh();
      await p2.goto(srv.url + '/');
      expect(p2.url()).toBe(srv.url + '/login?next=%2F');
      await expect(p2.locator('.cd-app.cd-loginpage #cd-loginform input[name=password]')).toBeVisible();
      await p2.locator('input[name=password]').fill('nope');
      await p2.locator('#cd-loginform button[type=submit]').click();
      await expect(p2.locator('.cd-loginpage .cd-err')).toHaveText('Wrong password.');
      await p2.locator('input[name=password]').fill('s3cret');
      await p2.locator('#cd-loginform button[type=submit]').click();
      await p2.waitForURL(srv.url + '/');
      await expect(p2.locator('#cd-nav-settings')).toBeVisible();
      // Remembered: a later page needs no login; Sign out ends it.
      await p2.goto(srv.url + '/settings');
      expect(p2.url()).toBe(srv.url + '/settings');
      await p2.locator('form.cd-logout button').click();
      await p2.waitForURL(/\/login/);
      await p2.goto(srv.url + '/settings');
      expect(p2.url()).toBe(srv.url + '/login?next=%2Fsettings');
      // Remove the password: a browser without a session is the owner on loopback again.
      await page.goto(srv.url + '/settings/server/general');
      await page.locator('#cd-pw-remove').click();
      await expect(pw).toHaveAttribute('data-password-set', 'false');
      await expect(page.locator('#cd-pw-remove')).toHaveCount(0);
      const p3 = await fresh();
      await p3.goto(srv.url + '/members');
      expect(p3.url()).toBe(srv.url + '/settings/server/members');
      await expect(p3.locator('.cd-me')).toContainText('@rowan');
    } finally {
      for (const c of contexts) await c.close();
      await srv.stop();
    }
  });

  // spec: S12-03, S12-04
  test('Vault create → default Actions edited (add / remove / reorder) → a new Document pulls them with inherited: true; rename; delete only when empty', async ({ page }) => {
    await page.goto(h().url + '/settings/server/vaults');
    await expect(page.locator('#cd-nav-settings')).toHaveAttribute('aria-current', 'page');
    const name = 'acts-' + uniq();
    await page.locator('#cd-vault-name').fill(name);
    await page.locator('#cd-vaultnew button[type=submit]').click();
    const vault = page.locator(`.cd-vault[data-vault="${name}"]`);
    await expect(vault).toBeVisible();
    await expect(vault.locator('.cd-vaultmeta')).toContainText('empty');
    // The defaults a new Vault starts with.
    const ids = () => vault.locator('.cd-actrow [data-f=id]').evaluateAll((els) => els.map((e) => (e as HTMLInputElement).value));
    expect(await ids()).toEqual(['approve', 'send-back']);
    // Add `ship` (positive), move it up, drop `send-back`, save.
    await vault.locator('[data-act=addrow]').click();
    const rows = vault.locator('.cd-actrow');
    await rows.nth(2).locator('[data-f=id]').fill('ship');
    await rows.nth(2).locator('[data-f=label]').fill('Ship it');
    await rows.nth(2).locator('[data-f=intent]').selectOption('positive');
    await rows.nth(2).locator('[data-act=up]').click();
    expect(await ids()).toEqual(['approve', 'ship', 'send-back']);
    await rows.nth(2).locator('[data-act=remove]').click();
    await vault.locator('[data-act=save-actions]').click();
    await expect(vault.locator('.cd-ok')).toHaveText('saved');
    expect(await ids()).toEqual(['approve', 'ship']);
    // A bad id is refused with the server's message, nothing saved.
    await vault.locator('[data-act=addrow]').click();
    await vault.locator('.cd-actrow').nth(2).locator('[data-f=id]').fill('Not Kebab');
    await vault.locator('[data-act=save-actions]').click();
    await expect(vault.locator('.cd-err')).toContainText('kebab-case');
    await page.reload();
    expect(await ids()).toEqual(['approve', 'ship']);
    // Push a Document into that Vault, pull it: the Sidecar carries the Vault defaults, inherited.
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cd-web-acts-'));
    fs.writeFileSync(path.join(dir, 'memo.md'), '# Memo\n\nDecide.\n');
    const pushed = cli(h().owner, ['push', 'memo.md', '--vault', name], dir);
    const id = /^memo\t(\S+)\tv1/m.exec(pushed)![1];
    const out = fs.mkdtempSync(path.join(os.tmpdir(), 'cd-web-pull-'));
    cli(h().owner, ['pull', id, '--out', out], dir);
    const sidecar = JSON.parse(fs.readFileSync(path.join(out, 'memo.annotary'), 'utf8'));
    expect(sidecar.actions.available).toEqual([
      { id: 'approve', label: 'Approve', intent: 'positive' },
      { id: 'ship', label: 'Ship it', intent: 'positive' },
    ]);
    expect(sidecar.actions.inherited).toBe(true);
    // The Document view offers those buttons on head.
    await page.goto(h().url + '/d/' + id);
    await page.waitForSelector('.cd-app.cd-ready');
    await expect(page.locator('#cd-actbtns .cd-actbtn')).toHaveText(['Approve', 'Ship it']);
    // Rename the Vault (the Document follows — it is the same Vault), then it cannot be deleted
    // while it holds a Document; an empty Vault can.
    await page.goto(h().url + '/settings/server/vaults');
    await vault.locator('[data-act=rename]').click();
    const renamed = name + '-r';
    await vault.locator('[data-slot=rename] input').fill(renamed);
    await vault.locator('[data-slot=rename] [data-inline=save]').click();
    const vault2 = page.locator(`.cd-vault[data-vault="${renamed}"]`);
    await expect(vault2).toBeVisible();
    await expect(vault).toHaveCount(0);
    await expect(vault2.locator('.cd-vaultmeta')).toContainText('1 Document');
    expect(cli(h().owner, ['list', 'memo', '--vault', renamed])).toContain(id);
    await vault2.locator('[data-act=delete]').click();
    await vault2.locator('[data-slot=rename] [data-inline=yes]').click();
    await expect(vault2.locator('[data-slot=rename] .cd-err')).toContainText('still holds 1 Document');
    await expect(vault2).toBeVisible();
    const empty = 'empty-' + uniq();
    await page.locator('#cd-vault-name').fill(empty);
    await page.locator('#cd-vaultnew button[type=submit]').click();
    const v3 = page.locator(`.cd-vault[data-vault="${empty}"]`);
    await expect(v3).toBeVisible();
    await v3.locator('[data-act=delete]').click();
    await v3.locator('[data-slot=rename] [data-inline=yes]').click();
    await expect(v3).toHaveCount(0);
    await page.reload();
    await expect(page.locator(`.cd-vault[data-vault="${empty}"]`)).toHaveCount(0);
    await expect(page.locator(`.cd-vault[data-vault="${renamed}"]`)).toBeVisible();
    // A duplicate name is refused in the create form.
    await page.locator('#cd-vault-name').fill(renamed);
    await page.locator('#cd-vaultnew button[type=submit]').click();
    await expect(page.locator('#cd-vaultnew .cd-err')).toContainText('already exists');
  });

  // spec: S12-03
  test('the auto-purge setting ("purge deleted after N days") persists; housekeeping can run now', async ({ browser }) => {
    const srv = await spawnServer();
    const ctx = await browser.newContext();
    try {
      const page = await ctx.newPage();
      await page.goto(srv.url + '/settings/server/general');
      const sec = page.locator('#cd-set-purge');
      await expect(sec).toHaveAttribute('data-days', '0');
      await expect(page.locator('#cd-purge-on')).not.toBeChecked();
      await page.locator('#cd-purge-on').check();
      await page.locator('#cd-purge-days').fill('7');
      await page.locator('#cd-purge-save').click();
      await expect(page.locator('#cd-purgeform .cd-ok')).toContainText('after 7 days');
      await expect(sec).toHaveAttribute('data-days', '7');
      await page.reload();
      await expect(page.locator('#cd-purge-on')).toBeChecked();
      await expect(page.locator('#cd-purge-days')).toHaveValue('7');
      const s = await page.request.get(srv.url + '/api/v1/settings');
      expect((await s.json()).auto_purge_days).toBe(7);
      // Run a tick now.
      await page.locator('#cd-runhk').click();
      await expect(page.locator('#cd-hkreport')).toContainText('done: purged 0 Documents');
      // Off again.
      await page.locator('#cd-purge-on').uncheck();
      await page.locator('#cd-purge-save').click();
      await expect(page.locator('#cd-purgeform .cd-ok')).toContainText('off');
      await page.reload();
      await expect(page.locator('#cd-purge-on')).not.toBeChecked();
      expect((await (await page.request.get(srv.url + '/api/v1/settings')).json()).auto_purge_days).toBeNull();
    } finally {
      await ctx.close();
      await srv.stop();
    }
  });
});

test.describe('a Member', () => {
  // spec: S12-05
  // (CMD-23 changed this on purpose: every Member has Settings; the owner pages moved under
  // /settings/server/*, which a Member gets as the 403 page.)
  test('gets the Settings link and screen, but no Server group and the 403 page for the owner pages', async ({ page }) => {
    await page.setExtraHTTPHeaders({ Authorization: 'Bearer ' + h().agent });
    await page.goto(h().url + '/');
    await expect(page.locator('#cd-nav-docs')).toBeVisible();
    await expect(page.locator('#cd-nav-members')).toHaveCount(0);
    await expect(page.locator('#cd-nav-settings')).toHaveAttribute('href', '/settings');
    await expect(page.locator('.cd-me')).toContainText('@scribe');
    const ok = await page.goto(h().url + '/settings');
    expect(ok!.status()).toBe(200);
    await expect(page.locator('.cd-set-server')).toHaveCount(0);
    for (const p of ['/members', '/settings/server/members', '/settings/server/vaults', '/settings/server/general']) {
      const res = await page.goto(h().url + p);
      expect(res!.status()).toBe(403);
      await expect(page.locator('.cd-err')).toContainText('owner-only');
      await expect(page.locator('#cd-addagent, #cd-pwform, #cd-vaults')).toHaveCount(0);
    }
    // The owner sees the Settings link too, and nothing else new in the top bar.
    await page.setExtraHTTPHeaders({ Authorization: 'Bearer ' + h().owner });
    await page.goto(h().url + '/');
    await expect(page.locator('#cd-nav-members')).toHaveCount(0);
    await expect(page.locator('#cd-nav-settings')).toHaveAttribute('href', '/settings');
  });
});
