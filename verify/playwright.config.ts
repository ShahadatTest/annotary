// `make verify-live` (CMD-22): scenarios against a throwaway copy of the live store, served by the
// build under test on a loopback port that `deploy/verify_live.py` starts and deletes. Never run
// directly: it needs VERIFY_URL / VERIFY_CLI_HOME / VERIFY_BIN / VERIFY_OUT / VERIFY_TMP from
// that script. Results and traces go inside the copy's folder (they hold Document text), so they
// are deleted with it; only screenshots a scenario saves to VERIFY_OUT stay (for the PR).
import { defineConfig } from '@playwright/test';
import * as path from 'node:path';

// The last guard: scenarios change data, so they only ever run against a loopback copy.
const tmp = process.env.VERIFY_TMP;
const target = process.env.VERIFY_URL ? new URL(process.env.VERIFY_URL) : null;
const home = process.env.VERIFY_CLI_HOME;
if (!tmp || !target || !home) throw new Error('run through `make verify-live`, not playwright directly');
if (target.hostname !== '127.0.0.1' || target.port === '' || target.port === '7377') {
  throw new Error(`VERIFY_URL ${target} is not a loopback copy (127.0.0.1, never port 7377)`);
}
if (!path.basename(tmp).startsWith('cd-verify-live-') || path.relative(tmp, home).startsWith('..')) {
  throw new Error('VERIFY_TMP must be a cd-verify-live-* copy folder holding VERIFY_CLI_HOME');
}

export default defineConfig({
  testDir: 'scenarios',
  testMatch: /.*\.spec\.ts$/,
  outputDir: `${tmp}/playwright-results`,
  timeout: 60_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list'], ['json', { outputFile: `${tmp}/report.json` }]],
  use: {
    baseURL: process.env.VERIFY_URL,
    browserName: 'chromium',
    headless: true,
    viewport: { width: 1280, height: 900 },
    locale: 'en-US',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
});
