// The keyboard map of the Document view (ticket 11, story 126): `j k r e c [ ] ↩ ⌘↩ esc ?` —
// and nothing more. Each binding is exercised against a Document of the test's own.
import { test, expect } from '@playwright/test';
import { harness, cli, seedDoc, pushVersion, rangeOf } from './harness';
import { openDoc, selectText } from './actions';

const h = () => harness();

const MD = `# Keys

First paragraph with some words to select.

Second paragraph, also with words.

Third paragraph at the end.
`;

test.describe('keyboard', () => {
  // spec: S11-06
  test('j k r e c [ ] ↩ ⌘↩ esc ? — and nothing more', async ({ page }) => {
    const doc = seedDoc('web-k1', MD);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'First paragraph'), '-m', 'one']);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'Second paragraph'), '-m', 'two']);
    cli(h().agent, ['thread', 'new', doc.id, rangeOf(MD, 'Third paragraph'), '-m', 'three']);
    pushVersion(doc, MD + '\nA fourth paragraph.\n', 'v2');
    await openDoc(page, doc.id);
    const t = (n: number) => page.locator(`.cd-thread[data-t="t${n}"]`);
    // j / k walk the Threads in document order (clamped at the ends).
    await page.keyboard.press('j');
    await expect(t(1)).toHaveClass(/cd-selected/);
    await expect(page.locator('mark.cd-hl[data-t="t1"]')).toHaveClass(/cd-selected/);
    await page.keyboard.press('j');
    await expect(t(2)).toHaveClass(/cd-selected/);
    await expect(t(1)).not.toHaveClass(/cd-selected/);
    await page.keyboard.press('j');
    await page.keyboard.press('j');
    await expect(t(3)).toHaveClass(/cd-selected/);
    await page.keyboard.press('k');
    await expect(t(2)).toHaveClass(/cd-selected/);
    // r opens the reply box on the selected Thread; ↩ posts it.
    await page.keyboard.press('r');
    const ta = t(2).locator('.cd-editor-reply textarea');
    await expect(ta).toBeFocused();
    await ta.pressSequentially('typed reply');
    // Inside a field the letters are text, not bindings.
    await expect(t(2)).toHaveClass(/cd-selected/);
    await ta.pressSequentially('jk');
    await expect(ta).toHaveValue('typed replyjk');
    await page.keyboard.press('Enter');
    await expect(t(2).locator('.cd-c[data-c="t2.1"] .cd-cbody')).toContainText('typed replyjk');
    await expect.poll(() => JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't2', '--json']))[0].replies.length).toBe(1);
    // e resolves the selected Thread (it hides behind the toggle and stays selected); e again reopens.
    await page.keyboard.press('e');
    await expect(t(2)).toHaveClass(/cd-resolved/);
    await expect.poll(() => JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't2', '--json']))[0].resolved).not.toBeNull();
    await page.keyboard.press('e');
    await expect(t(2)).not.toHaveClass(/cd-resolved/);
    await expect.poll(() => JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't2', '--json']))[0].resolved ?? null).toBeNull();
    // esc deselects.
    await page.keyboard.press('Escape');
    await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
    // c opens the composer on the selection; ⌘↩ posts the comment; esc closes a composer.
    await selectText(page, '.cd-doc p[data-l="7-7"]', 'at the end');
    await expect(page.locator('#cd-pill')).toBeVisible();
    await page.keyboard.press('c');
    await expect(page.locator('.cd-composer textarea')).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(page.locator('.cd-composer')).toHaveCount(0);
    await expect(page.locator('mark.cd-pending-hl')).toHaveCount(0);
    await selectText(page, '.cd-doc p[data-l="7-7"]', 'at the end');
    await expect(page.locator('#cd-pill')).toBeVisible();
    await page.keyboard.press('c');
    await page.locator('.cd-composer textarea').fill('via keyboard');
    await page.keyboard.press('Control+Enter');
    await expect(page.locator('.cd-thread[data-t="t4"]:not(.cd-pending)')).toBeVisible();
    // ? toggles the help overlay listing exactly the map; esc closes it.
    await page.keyboard.press('?');
    const help = page.locator('#cd-help');
    await expect(help).toBeVisible();
    const keys = await help.locator('kbd').allTextContents();
    expect(keys).toEqual(['j', 'k', 'r', 'e', 'c', '[', ']', '↩', '⌘↩', 'esc', '?']);
    await page.keyboard.press('Escape');
    await expect(help).toBeHidden();
    await page.keyboard.press('?');
    await expect(help).toBeVisible();
    await page.keyboard.press('?');
    await expect(help).toBeHidden();
    // Nothing more: other letters / modifiers do nothing.
    await page.keyboard.press('Escape');
    await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
    await page.keyboard.press('j');
    await expect(t(1)).toHaveClass(/cd-selected/);
    for (const k of ['n', 'p', 'x', 'a', 'Enter', 'ArrowDown', 'Shift+J']) {
      await page.keyboard.press(k);
    }
    await expect(t(1)).toHaveClass(/cd-selected/);
    await expect(page.locator('.cd-editor')).toHaveCount(0);
    await expect(page.locator('#cd-help')).toBeHidden();
    expect(page.url()).toBe(`${h().url}/d/${doc.id}`);
    // ⌘c (copy) with a selection must not open the composer.
    await selectText(page, '.cd-doc p[data-l="3-3"]', 'some words');
    await page.keyboard.press('Meta+c');
    await page.keyboard.press('Control+c');
    await expect(page.locator('.cd-composer')).toHaveCount(0);
    // [ goes to the older Version, ] back to the newer one.
    await page.keyboard.press('Escape');
    await page.keyboard.press('[');
    await page.waitForURL(`**/d/${doc.id}/v/1`);
    await page.waitForSelector('.cd-app.cd-ready');
    await page.keyboard.press('[');
    await page.waitForTimeout(200);
    expect(page.url()).toContain('/v/1');
    await page.keyboard.press(']');
    await page.waitForURL(`${h().url}/d/${doc.id}`);
    await page.waitForSelector('.cd-app.cd-ready');
    await page.keyboard.press(']');
    await page.waitForTimeout(200);
    expect(page.url()).toBe(`${h().url}/d/${doc.id}`);
  });

  // spec: S11-10
  test('esc never discards typed text: it asks inline, keep (or esc again) keeps it, discard closes; empty or unchanged editors close at once', async ({ page }) => {
    const doc = seedDoc('web-k2', MD);
    cli(h().owner, ['thread', 'new', doc.id, rangeOf(MD, 'First paragraph'), '-m', 'my comment']);
    await openDoc(page, doc.id);
    const ask = page.locator('.cd-discard');
    // The composer: typed text survives esc, and a confirm appears.
    await selectText(page, '.cd-doc p[data-l="5-5"]', 'Second paragraph');
    await expect(page.locator('#cd-pill')).toBeVisible();
    await page.keyboard.press('c');
    const composer = page.locator('.cd-composer');
    const ta = composer.locator('textarea');
    await expect(ta).toBeFocused();
    await ta.pressSequentially('half a thought');
    await page.keyboard.press('Escape');
    await expect(composer).toHaveCount(1);
    await expect(ta).toHaveValue('half a thought');
    await expect(composer.locator('.cd-discard')).toBeVisible();
    await expect(page.locator('mark.cd-hl.cd-pending-hl')).toHaveText('Second paragraph');
    // keep: the confirm goes, the text and the editor stay, the cursor is back in it.
    await ask.locator('[data-act="discard-no"]').click();
    await expect(ask).toHaveCount(0);
    await expect(ta).toBeFocused();
    await expect(ta).toHaveValue('half a thought');
    // esc while it asks = keep.
    await page.keyboard.press('Escape');
    await expect(ask).toBeVisible();
    await page.keyboard.press('Escape');
    await expect(ask).toHaveCount(0);
    await expect(composer).toHaveCount(1);
    await expect(ta).toHaveValue('half a thought');
    // discard: the composer and its pending highlight go; nothing was posted.
    await page.keyboard.press('Escape');
    await ask.locator('[data-act="discard-yes"]').click();
    await expect(composer).toHaveCount(0);
    await expect(page.locator('mark.cd-pending-hl')).toHaveCount(0);
    // An empty composer closes at once.
    await selectText(page, '.cd-doc p[data-l="5-5"]', 'Second paragraph');
    await expect(page.locator('#cd-pill')).toBeVisible();
    await page.keyboard.press('c');
    await expect(ta).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(composer).toHaveCount(0);
    await expect(ask).toHaveCount(0);
    // The reply box: the same.
    const t1 = page.locator('.cd-thread[data-t="t1"]');
    await page.keyboard.press('j');
    await expect(t1).toHaveClass(/cd-selected/);
    await page.keyboard.press('r');
    const reply = t1.locator('.cd-editor-reply textarea');
    await expect(reply).toBeFocused();
    await reply.pressSequentially('draft reply');
    await page.keyboard.press('Escape');
    await expect(reply).toHaveValue('draft reply');
    await expect(t1.locator('.cd-discard')).toBeVisible();
    await t1.locator('[data-act="discard-yes"]').click();
    await expect(t1.locator('.cd-editor-reply')).toHaveCount(0);
    await page.keyboard.press('r');
    await expect(reply).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(t1.locator('.cd-editor-reply')).toHaveCount(0);
    await expect(ask).toHaveCount(0);
    // The edit box: unchanged closes at once; changed asks, and keep keeps the change.
    await t1.locator('.cd-ctools [data-act="edit"]').click();
    const edit = t1.locator('.cd-editor-edit textarea');
    await expect(edit).toHaveValue('my comment');
    await expect(edit).toBeFocused();
    await page.keyboard.press('Escape');
    await expect(t1.locator('.cd-editor-edit')).toHaveCount(0);
    await expect(ask).toHaveCount(0);
    await expect(t1.locator('.cd-c[data-c="t1"] .cd-cbody')).toHaveText('my comment');
    await t1.locator('.cd-ctools [data-act="edit"]').click();
    await expect(edit).toBeFocused();
    await edit.pressSequentially(', revised');
    await page.keyboard.press('Escape');
    await expect(edit).toHaveValue('my comment, revised');
    await expect(t1.locator('.cd-discard')).toBeVisible();
    await t1.locator('[data-act="discard-no"]').click();
    await expect(edit).toBeFocused();
    await expect(edit).toHaveValue('my comment, revised');
    await page.keyboard.press('Escape');
    await t1.locator('[data-act="discard-yes"]').click();
    await expect(t1.locator('.cd-editor-edit')).toHaveCount(0);
    await expect(t1.locator('.cd-c[data-c="t1"] .cd-cbody')).toHaveText('my comment');
    // Nothing reached the server.
    const shown = JSON.parse(cli(h().owner, ['thread', 'show', doc.id, 't1', '--json']))[0];
    expect(shown.body).toBe('my comment');
    expect(shown.replies ?? []).toEqual([]);
    expect(JSON.parse(cli(h().owner, ['thread', 'list', doc.id, '--json'])).length).toBe(1);
  });
});
