// The shipped examples name made-up people. The real owner still shows where the data names them;
// only the CLI's help and the Members screen's placeholder are examples.
import { test } from '@playwright/test';
import { cli, expect } from '../lib';

test('help and the Members screen give made-up examples', async ({ page }) => {
  expect(cli(['wait', '--help'])).toContain('@rowan');
  expect(cli(['member', '--help'])).toContain('linter');

  await page.goto('/settings/server/members');
  await expect(page.locator('#cd-add-handle')).toHaveAttribute('placeholder', 'scribe');
});
