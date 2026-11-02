// Access from a browser (ticket 10 / story 24, 131): a loopback browser is the owner with no
// login step while no password is set (the cookie is issued on the first page); the login page
// appears only on a server with a local password and a session is remembered afterwards.
import { test, expect } from '@playwright/test';
import { harness } from './harness';

const h = () => harness();

test.describe('login', () => {
  // spec: S10-02
  test('no password: the first page signs the loopback browser in and issues the cookie; /login just sends you on', async ({ page, context }) => {
    const res = await page.goto(h().url + '/');
    expect(res!.status()).toBe(200);
    await expect(page.locator('.cd-me .cd-member')).toHaveText('Rowan Vale');
    await expect(page.locator('.cd-me')).toContainText('@rowan');
    const cookies = await context.cookies(h().url);
    const session = cookies.find((c) => c.name === 'cd_session');
    expect(session).toBeTruthy();
    expect(session!.httpOnly).toBe(true);
    expect(session!.value.startsWith('cds_')).toBe(true);
    await expect(page.locator('form.cd-logout')).toHaveCount(0);
    await page.goto(h().url + '/login');
    await page.waitForURL(h().url + '/');
    await expect(page.locator('table.cd-list')).toBeVisible();
    // The cookie is the credential for the Document view and the API alike.
    await page.goto(h().url + '/d/' + h().doc.id);
    await expect(page.locator('#cd-line .cd-name')).toHaveText('ra-plan');
    const me = await page.request.get(h().url + '/api/v1/me');
    expect(me.status()).toBe(200);
  });

  // spec: S10-02
  test('a password set: pages redirect to /login (next kept), the form signs in, the session is remembered, Sign out ends it', async ({ page }) => {
    const res = await page.goto(h().pwUrl + '/');
    expect(res!.status()).toBe(200); // the login page itself
    expect(page.url()).toBe(h().pwUrl + '/login?next=%2F');
    await expect(page.locator('input[name="password"]')).toBeVisible();
    // A Document URL keeps its `next`.
    await page.goto(h().pwUrl + '/d/zzzzzzzz');
    expect(page.url()).toBe(h().pwUrl + '/login?next=%2Fd%2Fzzzzzzzz');
    // Wrong password: 401 + the form again.
    await page.locator('input[name="password"]').fill('nope');
    await page.locator('button[type="submit"]').click();
    await expect(page.locator('.cd-err')).toContainText('Wrong password');
    // Right password: on to `next` (an unknown Document ⇒ its 404 page, signed in).
    await page.locator('input[name="password"]').fill('hunter2');
    await page.locator('button[type="submit"]').click();
    await page.waitForURL(h().pwUrl + '/d/zzzzzzzz');
    await expect(page.locator('.cd-err')).toContainText('404');
    // Remembered: the list renders without a login step now, with Sign out.
    await page.goto(h().pwUrl + '/');
    await expect(page.locator('table.cd-list, #cd-empty')).toBeVisible();
    await expect(page.locator('.cd-me')).toContainText('@ana');
    await expect(page.locator('form.cd-logout button')).toHaveText('Sign out');
    await page.locator('form.cd-logout button').click();
    await page.waitForURL(/\/login/);
    await expect(page.locator('input[name="password"]')).toBeVisible();
    await page.goto(h().pwUrl + '/');
    expect(page.url()).toContain('/login?next=');
  });
});
