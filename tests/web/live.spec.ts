// Live updates (ticket 11, stories 115/116): the Document page subscribes to the events SSE and
// reflects what others do — a reply, a new Thread, resolve, an Action, a new Version (a banner
// offering to go to head, never auto-navigation) — and History lists recent events. The
// other Member is the real CLI (as @scribe) in the same test.
import { test, expect } from '@playwright/test';
import { harness, cli, seedDoc, pushVersion, rangeOf } from './harness';
import { openDoc, selectText, waitLive } from './actions';

const h = () => harness();

const MD = `# Live

Alpha paragraph to anchor a Thread on.

Beta paragraph for a second one.
`;

test.describe('live updates', () => {
  // spec: S11-07
  test('another Member\'s reply / Thread / resolve / Action / push reach the open page over SSE; what\'s new lists them', async ({ page }) => {
    const doc = seedDoc('web-l1', MD);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'Alpha paragraph'), '-m', 'Alpha? @rowan']);
    await openDoc(page, doc.id);
    await waitLive(page);
    const t1 = page.locator('.cd-thread[data-t="t1"]');
    await expect(t1).toBeVisible();
    await expect(t1.locator('.cd-c')).toHaveCount(1);
    // A reply from the CLI appears in place, rendered by the server.
    cli(h().agent, ['thread', 'reply', doc.id, 't1', '-m', 'Replying **live**.']);
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-cbody strong')).toHaveText('live');
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-member')).toHaveText('Scribe');
    // A new Thread lands at its block with its highlight; a Document-level one under the title.
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'Beta paragraph'), '-m', 'Beta too.']);
    const t2 = page.locator('.cd-thread[data-t="t2"]');
    await expect(t2).toBeVisible();
    await expect(page.locator('mark.cd-hl[data-t="t2"]')).toHaveText('Beta paragraph');
    await expect(page.locator('.cd-strip', { has: t2 })).toHaveAttribute('data-line', '5');
    cli(h().agent, ['thread', 'new', doc.id, '--document', '-m', 'Whole thing.']);
    await expect(page.locator('.cd-strip-doc .cd-thread[data-t="t3"]')).toBeVisible();
    await expect(page.locator('.cd-status')).toContainText('3 open');
    // Resolve from the CLI: the card moves behind the toggle; reopen brings it back.
    cli(h().agent, ['thread', 'resolve', doc.id, 't2']);
    await expect(t2).toHaveClass(/cd-resolved/);
    await expect(page.locator('.cd-status')).toContainText('2 open');
    cli(h().agent, ['thread', 'reopen', doc.id, 't2']);
    await expect(t2).not.toHaveClass(/cd-resolved/);
    // An Action from the CLI updates the chip and the status line.
    cli(h().agent, ['act', doc.id, 'send-back']);
    const chip = page.locator('#cd-actions .cd-chip.cd-action');
    await expect(chip).toContainText('Send back');
    await expect(chip).toContainText('Scribe');
    await expect(chip).toHaveClass(/cd-neu/);
    await expect(page.locator('.cd-status')).toContainText('send back by Scribe');
    // Edits and deletes arrive too.
    cli(h().agent, ['thread', 'edit', doc.id, 't1.1', '-m', 'Edited live.']);
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-cbody')).toContainText('Edited live.');
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-who')).toContainText('edited');
    cli(h().agent, ['thread', 'delete', doc.id, 't1.1']);
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-tomb')).toHaveText('(deleted)');
    // History lists the live events (and the seed's).
    await page.locator('#cd-whatsnew').click();
    const news = page.locator('#cd-news');
    await expect(news).toBeVisible();
    await expect(news.locator('li').first()).toContainText('deleted t1.1');
    await expect(news).toContainText('Send back v1');
    await expect(news).toContainText('opened t3');
    await expect(news).toContainText('replied on t1');
    // …and keeps growing while open.
    cli(h().agent, ['thread', 'reply', doc.id, 't3', '-m', 'while the panel is open']);
    await expect(news.locator('li').first()).toContainText('replied on t3');
    await page.keyboard.press('Escape');
    // A new Version: a banner offers to go to head; the page does not navigate; Action buttons and the pill go.
    await expect(page.locator('#cd-actbtns')).toBeVisible();
    pushVersion(doc, MD + '\nGamma paragraph.\n', 'gamma');
    const banner = page.locator('#cd-newver');
    await expect(banner).toBeVisible();
    await expect(banner).toContainText('v2 was pushed by Rowan Vale');
    await expect(banner).toContainText('gamma');
    await expect(banner.locator('#cd-gonew')).toHaveAttribute('href', `/d/${doc.id}`);
    expect(page.url()).toBe(`${h().url}/d/${doc.id}`);
    await expect(page.locator('.cd-status')).toContainText('v1');
    await expect(page.locator('#cd-actbtns')).toBeHidden();
    await expect(page.locator('#cd-vsel option').first()).toContainText('v2 (head)');
    // New Threads are refused locally now (the head moved).
    await selectText(page, '.cd-doc p[data-l="5-5"]', 'second one');
    await expect(page.locator('#cd-pill')).toBeHidden();
    // Go to head by the banner.
    await banner.locator('#cd-gonew').click();
    await page.waitForURL(`${h().url}/d/${doc.id}`);
    await page.waitForSelector('.cd-app.cd-ready');
    await expect(page.locator('.cd-status')).toContainText('v2');
    await expect(page.locator('#cd-newver')).toHaveCount(0);
  });

  // spec: S11-07, S11-03
  test('on an older-Version view the conversation stays live: a CLI reply shows up; the placement stays the time-slice', async ({ page }) => {
    const doc = seedDoc('web-l2', MD);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'Alpha paragraph'), '-m', 'on v1']);
    pushVersion(doc, MD.replace('Alpha paragraph', 'The alpha paragraph'), 'v2');
    await openDoc(page, doc.id, { v: 1 });
    await waitLive(page);
    const t1 = page.locator('.cd-thread[data-t="t1"]');
    await expect(t1).toHaveClass(/cd-st-anchored/);
    cli(h().agent, ['thread', 'reply', doc.id, 't1', '-m', 'seen from v1']);
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-cbody')).toContainText('seen from v1');
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-mono')).toHaveText('v2');
    await expect(t1).toHaveClass(/cd-st-anchored/);
    await expect(t1.locator('.cd-now')).toHaveCount(0);
    // A Thread created on head is not part of the v1 slice (it shows in History, not on the page).
    cli(h().agent, ['thread', 'new', doc.id, '--document', '-m', 'new on head']);
    await page.locator('#cd-whatsnew').click();
    await expect(page.locator('#cd-news')).toContainText('opened t2');
    await expect(page.locator('.cd-thread[data-t="t2"]')).toHaveCount(0);
  });

  // spec: S25-01
  test('a live refresh keeps an open editor, even an empty one; Cancel or posting re-draws the card with the update (CMD-13)', async ({ page }) => {
    const doc = seedDoc('web-l3', MD);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'Alpha paragraph'), '-m', 'Alpha? @rowan']);
    await openDoc(page, doc.id);
    await waitLive(page);
    const t1 = page.locator('.cd-thread[data-t="t1"]');
    await expect(t1.locator('.cd-c')).toHaveCount(1);
    // An empty Reply box, then another Member replies: the box stays open and focused; the card waits.
    await t1.locator('[data-act="reply"]').click();
    const reply = t1.locator('.cd-editor-reply textarea');
    await expect(reply).toBeFocused();
    cli(h().agent, ['thread', 'reply', doc.id, 't1', '-m', 'first live reply']);
    await expect(t1).toHaveAttribute('data-stale', '1');
    await expect(reply).toBeVisible();
    await expect(reply).toBeFocused();
    await expect(reply).toHaveValue('');
    await expect(t1.locator('.cd-c')).toHaveCount(1);
    // Cancel closes it and re-draws the card with the reply.
    await t1.locator('.cd-editor-reply [data-act="cancel"]').click();
    await expect(t1.locator('.cd-editor')).toHaveCount(0);
    await expect(t1.locator('.cd-c[data-c="t1.1"] .cd-cbody')).toContainText('first live reply');
    await expect(t1).not.toHaveAttribute('data-stale', /./);
    // An unchanged edit box survives too; saving it re-draws the card with the edit and the reply.
    await t1.locator('.cd-c[data-c="t1"] [data-act="edit"]').click();
    const edit = t1.locator('.cd-c[data-c="t1"] .cd-editor-edit textarea');
    await expect(edit).toHaveValue('Alpha? @rowan');
    await expect(edit).toBeFocused();
    cli(h().agent, ['thread', 'reply', doc.id, 't1', '-m', 'second live reply']);
    await expect(t1).toHaveAttribute('data-stale', '1');
    await expect(edit).toBeFocused();
    await expect(t1.locator('.cd-c[data-c="t1.2"]')).toHaveCount(0);
    await edit.fill('Alpha, edited.');
    await page.keyboard.press('Control+Enter');
    await expect(t1.locator('.cd-c[data-c="t1"] .cd-cbody')).toContainText('Alpha, edited.');
    await expect(t1.locator('.cd-c[data-c="t1.2"] .cd-cbody')).toContainText('second live reply');
    await expect(t1).not.toHaveAttribute('data-stale', /./);
    // An empty Reply box, a live reply, then my own reply: both land, each once.
    await t1.locator('[data-act="reply"]').click();
    await expect(reply).toBeFocused();
    cli(h().agent, ['thread', 'reply', doc.id, 't1', '-m', 'third live reply']);
    await expect(t1).toHaveAttribute('data-stale', '1');
    await expect(reply).toBeFocused();
    await reply.fill('mine after the live one');
    await page.keyboard.press('Enter');
    await expect(t1.locator('.cd-c[data-c="t1.4"] .cd-cbody')).toContainText('mine after the live one');
    await expect(t1.locator('.cd-c:not(.cd-pending)')).toHaveCount(5);
    await expect(t1.locator('.cd-c[data-c="t1.3"] .cd-cbody')).toContainText('third live reply');
    await expect(t1.locator('.cd-editor')).toHaveCount(0);
    await expect(t1).not.toHaveAttribute('data-stale', /./);
  });
});
