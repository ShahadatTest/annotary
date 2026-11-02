// An agent's round trip on the copy through the CLI (push, Thread, reply, resolve), seen by the
// owner's browser: the CLI and the web agree on real data with the build under test.
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test } from '@playwright/test';
import { cli, cliJson, expect, openDoc } from '../lib';

test('push → Thread → reply → resolve from the CLI shows in the browser', async ({ page }) => {
  const dir = fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'agent-'));
  const file = path.join(dir, 'verify-live-agent.md');
  fs.writeFileSync(file, '# Verify live\n\nAn agent pushed this into the throwaway copy.\n');
  cli(['push', file, '-m', 'verify-live']);
  const id = cliJson<{ id: string }>(['status', file]).id;
  const tid = cliJson<{ id: string }>(['thread', 'new', id, '3:4-3:9', '-m', 'Is this right?']).id;
  cli(['thread', 'reply', id, tid, '-m', 'Yes, checked.', '--resolve']);
  await openDoc(page, id);
  const card = page.locator(`.cd-thread[data-t="${tid}"]`);
  await expect(card).toBeHidden(); // resolved, behind the toggle
  await page.locator('#cd-resolved').check();
  await expect(card).toBeVisible();
  await expect(card.locator('.cd-c')).toHaveCount(2);
  await expect(card).toHaveClass(/cd-resolved/);
});
