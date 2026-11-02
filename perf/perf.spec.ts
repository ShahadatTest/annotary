// The perf suite (ticket 15): Playwright + CDP against the release server `global-setup.ts`
// seeded with the generated fixture. Every metric is measured on a fresh browser context with
// the browser cache disabled and cleared (cold), five runs, the median asserted against the
// budgets of perf/BUDGETS.md (the only place they live). The table is printed at the end and
// written to target/perf/results.{md,json}; any miss fails the suite.
//
// Metrics (how, exactly — perf/BUDGETS.md repeats this next to each budget):
//  - fcp           `first-contentful-paint` of the Document view (head, 1280×900 = layout A)
//  - interactive   max(DOMContentLoaded end, the page's `cd:ready` mark), extended to the end of a
//                  long task (≥ 50 ms) still running then — the page takes input
//  - switch        in-page click on "older Version" → FCP of `/d/{id}/v/{n-1}`
//  - list          `first-contentful-paint` of `/` (a full page of 50 rows + "more")
//  - cls           Σ layout-shift values (no recent input) with startTime ≥ FCP, until ready + 500 ms
//  - optimistic    click on the reply editor's Post → the next painted frame after the pending
//                  Comment is in the DOM, the server's answer held back meanwhile
//  - reported      server TTFB of each page, DOMContentLoaded, page bytes, layout C (600 px) FCP / interactive
import { test, expect, type Browser, type BrowserContext, type Page } from '@playwright/test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { STATE_FILE, type PerfHarness } from './global-setup';
import { BUDGET_IDS, loadBudgets, type Budget, type BudgetId } from './budgets';

const RUNS = Number(process.env.PERF_RUNS || '5');
const REPO_ROOT = path.resolve(__dirname, '..');
const OUT_DIR = path.join(REPO_ROOT, 'target', 'perf');

let H: PerfHarness;
const budgets = loadBudgets();

/** Every number collected, by metric id (the budgeted ones and the reported ones). */
const samples: Record<string, number[]> = {};
const labels: Record<string, { metric: string; unit: string; budget?: number }> = {};
function record(id: string, metric: string, unit: string, value: number, budget?: number) {
  (samples[id] ||= []).push(value);
  labels[id] = { metric, unit, budget };
}
function median(xs: number[]): number {
  const s = [...xs].sort((a, b) => a - b);
  const m = s.length >> 1;
  return s.length % 2 ? s[m] : (s[m - 1] + s[m]) / 2;
}

interface Vitals {
  ttfb: number;
  responseEnd: number;
  dcl: number;
  fcp: number | null;
  ready: number | null;
  longtasks: { t: number; d: number }[];
  shifts: { t: number; v: number; input: boolean }[];
  bytes: number;
}

/** A fresh context + page with the cache disabled and cleared (cold) and the observers installed. */
async function cold(browser: Browser, width = 1280): Promise<{ ctx: BrowserContext; page: Page }> {
  const ctx = await browser.newContext({ viewport: { width, height: 900 } });
  const page = await ctx.newPage();
  const cdp = await ctx.newCDPSession(page);
  await cdp.send('Network.enable');
  await cdp.send('Network.setCacheDisabled', { cacheDisabled: true });
  await cdp.send('Network.clearBrowserCache');
  await cdp.detach();
  await page.addInitScript(() => {
    const w = window as unknown as { __perf: { shifts: { t: number; v: number; input: boolean }[]; longtasks: { t: number; d: number }[]; fcp: number | null } };
    w.__perf = { shifts: [], longtasks: [], fcp: null };
    new PerformanceObserver((l) => {
      for (const e of l.getEntries() as (PerformanceEntry & { value: number; hadRecentInput: boolean })[]) w.__perf.shifts.push({ t: e.startTime, v: e.value, input: e.hadRecentInput });
    }).observe({ type: 'layout-shift', buffered: true });
    new PerformanceObserver((l) => {
      for (const e of l.getEntries()) w.__perf.longtasks.push({ t: e.startTime, d: e.duration });
    }).observe({ type: 'longtask', buffered: true });
    new PerformanceObserver((l) => {
      for (const e of l.getEntries()) if (e.name === 'first-contentful-paint') w.__perf.fcp = e.startTime;
    }).observe({ type: 'paint', buffered: true });
  });
  return { ctx, page };
}

async function vitals(page: Page): Promise<Vitals> {
  return page.evaluate(() => {
    const w = window as unknown as { __perf: { shifts: { t: number; v: number; input: boolean }[]; longtasks: { t: number; d: number }[]; fcp: number | null } };
    const nav = performance.getEntriesByType('navigation')[0] as PerformanceNavigationTiming;
    const ready = performance.getEntriesByName('cd:ready')[0];
    // Paint entries can arrive after the observer callback: read the buffer directly too.
    const paint = performance.getEntriesByName('first-contentful-paint')[0];
    return {
      ttfb: nav.responseStart - nav.requestStart,
      responseEnd: nav.responseEnd,
      dcl: nav.domContentLoadedEventEnd,
      fcp: paint ? paint.startTime : w.__perf.fcp,
      ready: ready ? ready.startTime : null,
      longtasks: w.__perf.longtasks,
      shifts: w.__perf.shifts,
      bytes: nav.transferSize || nav.encodedBodySize || 0,
    };
  });
}

/** The moment the page takes input: max(DCL, cd:ready), pushed to the end of a long task running then. */
function interactiveOf(v: Vitals): number {
  let t = Math.max(v.dcl, v.ready ?? 0);
  for (const lt of v.longtasks) if (lt.t <= t && lt.t + lt.d > t) t = lt.t + lt.d;
  return t;
}

function clsAfterFirstPaint(v: Vitals): number {
  const fcp = v.fcp ?? 0;
  return v.shifts.filter((s) => !s.input && s.t >= fcp).reduce((a, s) => a + s.v, 0);
}

async function settle(page: Page) {
  await page.waitForSelector('.cd-app.cd-ready', { timeout: 30_000 });
  await page.waitForTimeout(500); // late shifts / long tasks after ready count too
}

test.describe.configure({ mode: 'serial' });

test.beforeAll(() => {
  H = JSON.parse(fs.readFileSync(STATE_FILE, 'utf8')) as PerfHarness;
});

const docUrl = () => `${H.url}/d/${H.seeded.doc_id}`;

// spec: S15-02, S15-03
test('Document view: FCP, interactive, CLS, TTFB (layout A, 1280px) — and layout C (600px) reported', async ({ browser }) => {
  for (let i = 0; i < RUNS; i++) {
    const { ctx, page } = await cold(browser);
    await page.goto(docUrl(), { waitUntil: 'load' });
    await settle(page);
    const v = await vitals(page);
    expect(v.fcp, 'first-contentful-paint recorded').not.toBeNull();
    record('fcp', budgets.fcp.metric, 'ms', v.fcp!, budgets.fcp.budget);
    record('interactive', budgets.interactive.metric, 'ms', interactiveOf(v), budgets.interactive.budget);
    record('cls', budgets.cls.metric, 'score', clsAfterFirstPaint(v), budgets.cls.budget);
    record('ttfb_doc', 'server TTFB — Document view', 'ms', v.ttfb);
    record('dcl_doc', 'DOMContentLoaded — Document view', 'ms', v.dcl);
    record('bytes_doc', 'page bytes — Document view', 'KB', Math.round(v.bytes / 1024));
    await ctx.close();
  }
  // Layout C: the same page in a 600 px viewport (strips in the flow) — reported, not budgeted.
  for (let i = 0; i < RUNS; i++) {
    const { ctx, page } = await cold(browser, 600);
    await page.goto(docUrl(), { waitUntil: 'load' });
    await settle(page);
    const v = await vitals(page);
    record('fcp_narrow', 'Document view FCP — layout C (600px)', 'ms', v.fcp ?? 0);
    record('interactive_narrow', 'Document view interactive — layout C (600px)', 'ms', interactiveOf(v));
    record('cls_narrow', 'CLS after first paint — layout C (600px)', 'score', clsAfterFirstPaint(v));
    await ctx.close();
  }
});

// spec: S15-02, S15-03
test('Version switch: older-Version click → FCP of the time-slice page', async ({ browser }) => {
  for (let i = 0; i < RUNS; i++) {
    const { ctx, page } = await cold(browser);
    await page.goto(docUrl(), { waitUntil: 'load' });
    await settle(page);
    const t0 = await page.evaluate(() => {
      const a = document.querySelector('.cd-vsw a.cd-older') as HTMLAnchorElement | null;
      if (!a) throw new Error('no older-Version arrow');
      const t = Date.now();
      a.click();
      return t;
    });
    await page.waitForURL(/\/v\/\d+$/);
    await settle(page);
    const v = await vitals(page);
    const fcpAbs = await page.evaluate(() => performance.timeOrigin) + (v.fcp ?? 0);
    record('switch', budgets.switch.metric, 'ms', fcpAbs - t0, budgets.switch.budget);
    record('ttfb_version', 'server TTFB — Version page (time-slice)', 'ms', v.ttfb);
    record('fcp_version', 'Version page FCP (own navigation)', 'ms', v.fcp ?? 0);
    await ctx.close();
  }
});

// spec: S15-02, S15-03
test('Document list: FCP and TTFB', async ({ browser }) => {
  for (let i = 0; i < RUNS; i++) {
    const { ctx, page } = await cold(browser);
    await page.goto(`${H.url}/`, { waitUntil: 'load' });
    await page.waitForSelector('#cd-list tr.cd-docrow');
    await page.waitForTimeout(200);
    const v = await vitals(page);
    const rows = await page.locator('tr.cd-docrow').count();
    expect(rows, 'a full page of rows').toBeGreaterThanOrEqual(50);
    record('list', budgets.list.metric, 'ms', v.fcp ?? 0, budgets.list.budget);
    record('ttfb_list', 'server TTFB — Document list', 'ms', v.ttfb);
    record('bytes_list', 'page bytes — Document list', 'KB', Math.round(v.bytes / 1024));
    await ctx.close();
  }
});

// spec: S15-02
test('Optimistic reply: Post click → painted, the server held back', async ({ browser }) => {
  for (let i = 0; i < RUNS; i++) {
    const { ctx, page } = await cold(browser);
    await page.goto(docUrl(), { waitUntil: 'load' });
    await settle(page);
    // A visible open Thread card in the margin (not the composer, not resolved, not orphaned).
    const card = page.locator('.cd-strip:not(.cd-strip-orphaned) .cd-thread:not(.cd-resolved):not(.cd-composer)').first();
    const tid = (await card.getAttribute('data-t'))!;
    await card.locator('.cd-acts [data-act="reply"]').click();
    const ta = card.locator('.cd-editor-reply textarea');
    await ta.fill(`perf run ${i + 1}: optimistic reply`);
    // Hold the server's answer until the measurement is done, then let it through.
    let release!: () => void;
    const held = new Promise<void>((r) => { release = r; });
    await page.route('**/api/v1/docs/*/threads/*/replies', async (route) => { await held; await route.continue(); });
    const m = await page.evaluate((id) => new Promise<{ dom: number; paint: number }>((resolve, reject) => {
      const card = document.querySelector(`.cd-thread[data-t="${id}"]`)!;
      const btn = card.querySelector('.cd-editor [data-act="post"]') as HTMLButtonElement | null;
      if (!btn) { reject(new Error('no Post button')); return; }
      let done = false;
      let t0 = 0;
      const finish = () => {
        if (done) return;
        done = true;
        const dom = performance.now() - t0;
        mo.disconnect();
        // The next frame, then a macrotask: the update has been painted.
        requestAnimationFrame(() => setTimeout(() => resolve({ dom, paint: performance.now() - t0 }), 0));
      };
      const mo = new MutationObserver(() => { if (card.querySelector('.cd-c.cd-pending')) finish(); });
      mo.observe(card, { childList: true, subtree: true, attributes: true });
      setTimeout(() => { if (!done) reject(new Error('no pending Comment appeared within 3 s')); }, 3000);
      t0 = performance.now();
      btn.click();
      // The page applies the reply synchronously in the click handler; the observer would fire
      // as a microtask — measure right away in that case.
      if (card.querySelector('.cd-c.cd-pending')) finish();
    }), tid);
    record('optimistic', budgets.optimistic.metric, 'ms', m.paint, budgets.optimistic.budget);
    record('optimistic_dom', 'Optimistic reply — DOM updated', 'ms', m.dom);
    release();
    await expect(card.locator('.cd-c.cd-pending')).toHaveCount(0, { timeout: 10_000 }); // confirmed by the server
    await ctx.close();
  }
});

// spec: S15-02, S15-04
test('budgets: the table, then the assertions (perf/BUDGETS.md is the only source of the numbers)', async () => {
  const rows: { id: string; metric: string; budget: string; median: string; min: string; max: string; result: string; unit: string }[] = [];
  const fmt = (x: number, unit: string) => (unit === 'score' ? x.toFixed(4).replace(/\.?0+$/, '') || '0' : unit === 'KB' ? String(Math.round(x)) : x.toFixed(1));
  const misses: string[] = [];
  const ordered = [...BUDGET_IDS, ...Object.keys(samples).filter((k) => !(BUDGET_IDS as readonly string[]).includes(k))];
  const json: Record<string, unknown> = { runs: RUNS, at: new Date().toISOString(), url: H.url, doc: H.seeded.doc_id, seeded: H.seeded, metrics: {} };
  for (const id of ordered) {
    const xs = samples[id];
    if (!xs || !xs.length) continue;
    const l = labels[id];
    const med = median(xs);
    const b: Budget | undefined = (BUDGET_IDS as readonly string[]).includes(id) ? budgets[id as BudgetId] : undefined;
    let result = '—';
    if (b) {
      const ok = med <= b.budget;
      result = ok ? 'pass' : 'FAIL';
      if (!ok) misses.push(`${b.metric}: median ${fmt(med, l.unit)} ${l.unit} > budget ${b.budget} ${b.unit}`);
    }
    rows.push({ id, metric: l.metric, budget: b ? `${b.budget} ${b.unit}` : '(reported)', median: fmt(med, l.unit), min: fmt(Math.min(...xs), l.unit), max: fmt(Math.max(...xs), l.unit), result, unit: l.unit });
    (json.metrics as Record<string, unknown>)[id] = { metric: l.metric, unit: l.unit, budget: b?.budget ?? null, median: med, samples: xs, pass: b ? med <= b.budget : null };
  }
  const w = (s: string, n: number) => s + ' '.repeat(Math.max(0, n - s.length));
  const head = `${w('metric', 44)} ${w('budget', 12)} ${w('median', 9)} ${w('min', 9)} ${w('max', 9)} result`;
  const lines = rows.map((r) => `${w(r.metric, 44)} ${w(r.budget, 12)} ${w(r.median, 9)} ${w(r.min, 9)} ${w(r.max, 9)} ${r.result}`);
  const table = [head, '-'.repeat(head.length), ...lines].join('\n');
  const intro = `perf: ${H.seeded.name} ${H.seeded.doc_id} — ${H.seeded.stats['lines_head']} lines / ${H.seeded.stats['words_head']} words / ${H.seeded.threads} Threads (${H.seeded.statuses.anchored} anchored, ${H.seeded.statuses.refined} refined, ${H.seeded.statuses.orphaned} orphaned) on v${H.seeded.head}; release binary ${H.bin}; ${RUNS} cold runs each, median asserted (units: ms unless noted)`;
  // eslint-disable-next-line no-console
  console.log(`\n${intro}\n\n${table}\n`);
  fs.mkdirSync(OUT_DIR, { recursive: true });
  fs.writeFileSync(path.join(OUT_DIR, 'results.json'), JSON.stringify(json, null, 2));
  const md = [
    `# Perf results (${new Date().toISOString()})`, '', intro, '',
    '| metric | budget | median | min | max | result |', '|---|---|---|---|---|---|',
    ...rows.map((r) => `| ${r.metric} | ${r.budget} | ${r.median} | ${r.min} | ${r.max} | ${r.result} |`), '',
  ].join('\n');
  fs.writeFileSync(path.join(OUT_DIR, 'results.md'), md);
  expect(misses, 'budgets missed').toEqual([]);
});
