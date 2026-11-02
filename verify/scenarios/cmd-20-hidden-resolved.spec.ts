// CMD-20 on the real data: with "Show resolved" off, clicking text that carries a resolved
// Thread's (invisible) highlight opens nothing and leaves the toggle off; with it on, the same
// click opens the Thread. Uses a resolved span Thread already in the copy, or makes one.
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test, type Page } from '@playwright/test';
import { cli, documents, expect, openDoc, shot } from '../lib';

async function resolvedMarkOn(page: Page): Promise<string | null> {
  const m = page.locator('mark.cd-hl.cd-res');
  for (let i = 0; i < (await m.count()); i++) {
    const tid = await m.nth(i).getAttribute('data-t');
    // Only a mark that is not inside an open Thread's mark: that one would rightly open.
    const nested = await m.nth(i).evaluate((e) => !!e.parentElement?.closest('mark.cd-hl:not(.cd-res)'));
    if (tid && !nested) return tid;
  }
  return null;
}

test('a hidden resolved highlight does nothing when clicked; with the toggle on it opens', async ({ page }) => {
  let found: { id: string; tid: string } | null = null;
  for (const d of documents()) {
    await openDoc(page, d.id);
    const tid = await resolvedMarkOn(page);
    if (tid) { found = { id: d.id, tid }; break; }
  }
  test.info().annotations.push({ type: 'data', description: found ? `existing resolved Thread ${found.tid} on ${found.id}` : 'made one' });
  if (!found) {
    // None in the copy: resolve a new span Thread on the first Document's first text line.
    const d = documents()[0];
    const dir = fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'pull-'));
    cli(['pull', d.id, '--out', dir]);
    const md = fs.readdirSync(dir).find((f) => f.endsWith('.md'))!;
    const lines = fs.readFileSync(path.join(dir, md), 'utf8').split('\n');
    const n = lines.findIndex((l) => /^[A-Za-z]{6}/.test(l));
    const tid = JSON.parse(cli(['thread', 'new', d.id, `${n + 1}:1-${n + 1}:6`, '-m', 'verify-live CMD-20', '--json', 'id'])).id;
    cli(['thread', 'resolve', d.id, tid]);
    found = { id: d.id, tid };
    await openDoc(page, d.id);
  }
  const toggle = page.locator('#cd-resolved');
  if (await toggle.isChecked()) await toggle.uncheck();
  const mark = page.locator(`mark.cd-hl.cd-res[data-t="${found.tid}"]`).first();
  await mark.scrollIntoViewIfNeeded();
  await mark.click();
  await expect(toggle).not.toBeChecked();
  await expect(page.locator(`.cd-thread[data-t="${found.tid}"]`)).toBeHidden();
  await expect(page.locator('.cd-thread.cd-selected')).toHaveCount(0);
  await shot(page, 'cmd-20-toggle-off-click');
  await toggle.check();
  await mark.click();
  await expect(page.locator(`.cd-thread[data-t="${found.tid}"]`)).toHaveClass(/cd-selected/);
});
