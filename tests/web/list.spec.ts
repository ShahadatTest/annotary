// The Document list page (ticket 10): the CLI's list — columns, activity order, filters (Name
// substring, deleted, mentions-me, Action), the display-only title, limit + more, no unread state.
import { test, expect } from '@playwright/test';
import { harness } from './harness';

const h = () => harness();

test.describe('list', () => {
  // spec: S10-08
  test('columns, order, title and the filters of `annotary list`', async ({ page }) => {
    await page.goto(h().url + '/');
    const heads = await page.locator('table.cd-list thead th').allTextContents();
    expect(heads).toEqual(['ID', 'NAME', 'VER', 'ACTION', 'OPEN', 'UPDATED']);
    const ids = await page.locator('tr.cd-docrow').evaluateAll((rows) => rows.map((r) => r.getAttribute('data-id')));
    expect(ids).toContain(h().doc.id);
    expect(ids).toContain(h().list.one);
    expect(ids).toContain(h().list.me);
    expect(ids).toContain(h().list.ok);
    expect(ids).not.toContain(h().list.gone);
    // Activity order: lst-ok (approved last) before lst-me before lst-one (pushed first, untouched).
    expect(ids.indexOf(h().list.ok)).toBeLessThan(ids.indexOf(h().list.me));
    expect(ids.indexOf(h().list.me)).toBeLessThan(ids.indexOf(h().list.one));
    const one = page.locator(`tr.cd-docrow[data-id="${h().list.one}"]`);
    await expect(one.locator('.cd-docname')).toHaveText('lst-one');
    await expect(one.locator('.cd-title')).toHaveText('The first list Document');
    await expect(one.locator('.cd-col-ver')).toHaveText('v1');
    await expect(one.locator('.cd-col-action')).toHaveText('-');
    const ok = page.locator(`tr.cd-docrow[data-id="${h().list.ok}"]`);
    await expect(ok.locator('.cd-col-action .cd-chip')).toHaveText('approve');
    await expect(ok.locator('.cd-col-action .cd-chip')).toHaveClass(/cd-pos/);
    const plan = page.locator(`tr.cd-docrow[data-id="${h().doc.id}"]`);
    await expect(plan.locator('.cd-col-ver')).toHaveText('v5');
    await expect(plan.locator('.cd-col-open')).toContainText('6');
    await expect(plan.locator('.cd-col-open .cd-mention')).toContainText('2 @you');
    await expect(plan.locator('.cd-col-updated time')).toHaveClass(/cd-localized/);
    expect(await page.locator('.cd-app').evaluate((e) => /unread/i.test(e.textContent || ''))).toBe(false);
    // Rows link to the Document view.
    await expect(plan.locator('a.cd-docname')).toHaveAttribute('href', `/d/${h().doc.id}`);

    // Filters through the form.
    await page.locator('#cd-filters input[name="name"]').fill('lst-');
    await page.locator('#cd-filters button[type="submit"]').click();
    await page.waitForURL(/\?.*name=lst-/);
    await expect(page.locator('tr.cd-docrow')).toHaveCount(3);
    await page.locator('#cd-filters input[name="deleted"]').check();
    await page.locator('#cd-filters button[type="submit"]').click();
    await page.waitForURL(/deleted=1/);
    await expect(page.locator('tr.cd-docrow')).toHaveCount(1);
    await expect(page.locator('tr.cd-docrow .cd-docname')).toHaveText('lst-gone');
    await expect(page.locator('tr.cd-docrow .cd-deleted')).toHaveText('deleted');
    // Filters through the URL (what a bookmark keeps). `vault=notes`: the ticket-11 specs push
    // their own Documents into the `web` Vault while this runs.
    await page.goto(h().url + '/?mentions_me=1&vault=notes');
    const mine = await page.locator('tr.cd-docrow').evaluateAll((rows) => rows.map((r) => r.getAttribute('data-id')));
    expect(mine.sort()).toEqual([h().doc.id, h().list.me].sort());
    await page.goto(h().url + '/?action=approve&vault=notes');
    await expect(page.locator('tr.cd-docrow')).toHaveCount(1);
    await expect(page.locator('tr.cd-docrow .cd-docname')).toHaveText('lst-ok');
    await page.goto(h().url + '/?action=none&name=lst-');
    await expect(page.locator('tr.cd-docrow')).toHaveCount(2);
    await page.goto(h().url + '/?name=zzz-nothing');
    await expect(page.locator('#cd-empty')).toBeVisible();
    // limit + more.
    await page.goto(h().url + '/?name=lst-&limit=2');
    await expect(page.locator('tr.cd-docrow')).toHaveCount(2);
    await expect(page.locator('#cd-listfoot')).toContainText('showing 2 of 3');
    await page.locator('#cd-more').click();
    await page.waitForURL(/limit=52/);
    await expect(page.locator('tr.cd-docrow')).toHaveCount(3);
    await expect(page.locator('#cd-more')).toHaveCount(0);
    // A row opens the Document view.
    await page.locator(`tr.cd-docrow[data-id="${h().list.one}"] a.cd-docname`).click();
    await page.waitForURL(`**/d/${h().list.one}`);
    await expect(page.locator('#cd-line .cd-name')).toHaveText('lst-one');
    await expect(page.locator('.cd-doc h1')).toHaveText('Not the title');
  });
});
