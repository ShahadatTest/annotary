// Playwright (TypeScript) — the browser tests of the annotary web UI (ticket 10 stands the
// harness up; 11/12/15 add to it). `tests/web/global-setup.ts` spawns a real `annotary serve`
// on an ephemeral port with a seeded temp store (plus a second, password-protected server);
// tests read the URLs from `tests/web/harness.ts`. Run via `make e2e` (which builds the binary
// first) or `npx playwright test`; `ANNOTARY_BIN` overrides the binary path.
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'tests/web',
  testMatch: /.*\.spec\.ts$/,
  globalSetup: './tests/web/global-setup.ts',
  globalTeardown: './tests/web/global-teardown.ts',
  outputDir: 'target/playwright/results',
  timeout: 30_000,
  expect: { timeout: 5_000 },
  fullyParallel: true,
  workers: process.env.CI ? 2 : 4,
  retries: 0,
  reporter: process.env.CI ? 'dot' : [['list']],
  use: {
    browserName: 'chromium',
    headless: true,
    viewport: { width: 1280, height: 900 },
    locale: 'en-US',
    timezoneId: 'UTC',
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
});
