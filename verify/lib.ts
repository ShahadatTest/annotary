// Helpers for `make verify-live` scenarios (CMD-22). The copy is real data: find Documents by
// what they hold, never by a hard-coded id, and change anything you need — it is thrown away.
import * as path from 'node:path';
import { execFileSync } from 'node:child_process';
import { expect, type Page } from '@playwright/test';

export const URL = process.env.VERIFY_URL!;
export const OUT = process.env.VERIFY_OUT!;

/** The CLI as the copy's owner, through the private `copy` profile (it can only reach the copy). */
export function cli(args: string[]): string {
  return execFileSync(process.env.VERIFY_BIN!, ['-p', 'copy', ...args], {
    env: { PATH: process.env.PATH ?? '', HOME: process.env.VERIFY_TMP!, ANNOTARY_HOME: process.env.VERIFY_CLI_HOME! },
    encoding: 'utf8',
  });
}

/** `cli([...args, '--json'])` parsed. */
export function cliJson<T = any>(args: string[]): T {
  return JSON.parse(cli([...args, '--json']) || 'null');
}

/** Every Document in the copy: `[{id, name, ...}]`. */
export function documents(): { id: string; name: string }[] {
  return cliJson(['list']) ?? [];
}

/** Open a Document page as the owner (loopback: signed in automatically). */
export async function openDoc(page: Page, id: string, width = 1280) {
  await page.setViewportSize({ width, height: 900 });
  await page.goto(`/d/${id}`);
  await page.waitForSelector('.cd-app.cd-ready');
}

/** Save a screenshot for the pull request (VERIFY_OUT/<name>.png). */
export async function shot(page: Page, name: string) {
  await page.screenshot({ path: path.join(OUT, `${name}.png`) });
}

export { expect };
