// Playwright config of the perf suite (ticket 15): `make perf` runs
// `npx playwright test -c perf/playwright.config.ts` after building the release binaries.
// One worker, no retries, no parallelism — the numbers are the point. `perf/global-setup.ts`
// spawns a release `annotary serve` and seeds it with the generated fixture (`perfgen seed`).
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: '.',
  testMatch: /perf\.spec\.ts$/,
  globalSetup: './global-setup.ts',
  globalTeardown: './global-teardown.ts',
  outputDir: '../target/perf/results',
  timeout: 300_000,
  expect: { timeout: 10_000 },
  fullyParallel: false,
  workers: 1,
  retries: 0,
  reporter: [['list']],
  use: {
    browserName: 'chromium',
    headless: true,
    viewport: { width: 1280, height: 900 },
    locale: 'en-US',
    timezoneId: 'UTC',
    trace: 'off',
    screenshot: 'off',
    video: 'off',
  },
});
