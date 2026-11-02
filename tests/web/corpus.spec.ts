// The made-up corpus (`fixtures/corpus/`) in a real browser: realistic Documents rendered, commented
// on and re-anchored. The tiny per-test Documents elsewhere test one thing each; these test the mix.
// Read-only cases use the Documents the seed pushed (`harness().corpus`); a test that adds a Thread
// pushes its own copy.
import { test, expect, type Page } from '@playwright/test';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { CORPUS, CORPUS_DIR, cli, corpusText, harness, pushVersion, rangeOf, seedDoc } from './harness';
import { openDoc, selectText } from './actions';

const h = () => harness();

/** Push a copy of a corpus Document under a Name of its own, without its Assets. */
function copyOf(name: string, as: string, rel = `${name}.md`) {
  return seedDoc(as, corpusText(rel));
}

async function comment(page: Page, text: string) {
  await page.locator('#cd-pill').click();
  const ta = page.locator('.cd-composer textarea');
  await expect(ta).toBeFocused();
  await ta.fill(text);
  await page.keyboard.press('Control+Enter');
}

/** The Thread as the CLI sees it, by id. */
function thread(doc: string, t: string) {
  return JSON.parse(cli(h().owner, ['thread', 'show', doc, t, '--json']))[0];
}

test.describe('the corpus renders', () => {
  // spec: S32-04
  test('every Document opens with its first paint final, at a desktop and at a phone width', async ({ page }) => {
    for (const name of CORPUS) {
      for (const width of [1280, 390]) {
        await openDoc(page, h().corpus[name], { width });
        const overflow = await page.evaluate(() => document.documentElement.scrollWidth - document.documentElement.clientWidth);
        expect(overflow, `${name} at ${width} px is wider than the window`).toBeLessThanOrEqual(0);
      }
    }
  });

  // spec: S32-04
  test('the RFC: front matter, headings, tables, code, footnotes and relative links', async ({ page }) => {
    await openDoc(page, h().corpus['rfc-tile-cache']);
    const doc = page.locator('.cd-doc');
    await expect(doc.locator('h1')).toHaveText('RFC 014: a two-tier tile cache');
    expect(await doc.locator('h2').count()).toBeGreaterThanOrEqual(10);
    expect(await doc.locator('table').count()).toBeGreaterThanOrEqual(6);
    expect(await doc.locator('pre code').count()).toBeGreaterThanOrEqual(6);
    expect(await doc.locator('sup.footnote-ref a').count()).toBe(3);
    await expect(doc).toContainText('One request in fifty');
    await expect(doc.locator('td', { hasText: 'bulk re-render of contours' })).toBeVisible();
  });

  // spec: S32-04
  test('the API reference: deep headings, long tables and many code blocks', async ({ page }) => {
    await openDoc(page, h().corpus['api-reference']);
    const doc = page.locator('.cd-doc');
    expect(await doc.locator('h4').count()).toBeGreaterThanOrEqual(45);
    expect(await doc.locator('h5').count()).toBeGreaterThanOrEqual(100);
    expect(await doc.locator('h6').count()).toBeGreaterThanOrEqual(20);
    expect(await doc.locator('table').count()).toBeGreaterThanOrEqual(100);
    expect(await doc.locator('pre code').count()).toBeGreaterThanOrEqual(60);
    expect(await doc.locator('table').nth(0).locator('tr').count()).toBeGreaterThanOrEqual(50);
  });

  // spec: S32-04
  test('onboarding and the checklist: task lists, nesting, quotes', async ({ page }) => {
    await openDoc(page, h().corpus['onboarding']);
    const doc = page.locator('.cd-doc');
    expect(await doc.locator('input[type=checkbox]').count()).toBeGreaterThanOrEqual(10);
    expect(await doc.locator('input[type=checkbox][checked]').count()).toBeGreaterThanOrEqual(2);
    expect(await doc.locator('blockquote blockquote').count()).toBeGreaterThanOrEqual(1);
    expect(await doc.locator('ol ol').count()).toBeGreaterThanOrEqual(1);
    await openDoc(page, h().corpus['release-checklist']);
    await expect(page.locator('.cd-doc h1')).toHaveText('Style release checklist');
    await expect(page.locator('.cd-doc')).not.toContainText('review_every_days');
  });

  // spec: S32-04
  test('field notes: every image and video kind, and the missing picture', async ({ page }) => {
    await openDoc(page, h().corpus['field-notes']);
    const doc = page.locator('.cd-doc');
    expect(await doc.locator('img[src^="/a/"]').count()).toBeGreaterThanOrEqual(10);
    expect(await doc.locator('video').count()).toBeGreaterThanOrEqual(2);
    const broken = await doc.locator('img[src^="/a/"]').evaluateAll((imgs) =>
      Promise.all((imgs as HTMLImageElement[]).map((i) => (i.complete ? null : new Promise((r) => { i.onload = i.onerror = r; })))).then(() =>
        (imgs as HTMLImageElement[]).filter((i) => i.naturalWidth === 0).map((i) => i.alt),
      ),
    );
    // `a.webp` is a header only, enough for dimension probing and not displayable (`fixtures/assets/make.py`).
    // A HEIC needs a converter to get a displayable derivative: always there on macOS, not on every
    // Linux machine (S03-17, Deferred).
    const known = ['Trail marker, WebP'];
    if (process.platform !== 'darwin') known.push('Phone panorama of Haväng');
    expect(broken.filter((alt) => !known.includes(alt))).toEqual([]);
    expect(broken).toContain('Trail marker, WebP');
    await expect(doc).toContainText('Missing photo of the water tap');
  });

  // spec: S32-04
  test('the multilingual glossary: CJK, Arabic, Hebrew, combining marks and emoji survive', async ({ page }) => {
    await openDoc(page, h().corpus['glossary-multilingual']);
    const doc = page.locator('.cd-doc');
    for (const text of ['東京の顧客は', 'شركات العبارات', 'מועדון הטיולים', 'مَكْتَبَةُ الْخَرَائِطِ', '👩‍👩‍👧‍👦 Family Walk', '🇸🇪 Skåne', '１４．５ｍ']) {
      await expect(doc, text).toContainText(text);
    }
    expect(await doc.locator('tr').count()).toBeGreaterThanOrEqual(15);
  });

  // spec: S32-04
  test('maths, diagrams and raw HTML: MathML, mermaid fences, and scripts that stay inert', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await openDoc(page, h().corpus['math-and-diagrams']);
    const doc = page.locator('.cd-doc');
    expect(await doc.locator('math').count()).toBeGreaterThanOrEqual(10);
    await expect(doc.locator('pre.mermaid')).toHaveCount(3);
    await expect(doc.locator('kbd').first()).toHaveText('Shift');
    await expect(doc.locator('details summary')).toContainText('TMS');
    expect(await page.title()).not.toContain('must never execute');
    expect(await page.title()).not.toContain('nor this');
    await expect(doc.locator('script')).toHaveCount(0);
    expect(await doc.locator('[onerror]').count()).toBe(0);
    expect(await doc.locator('a[href^="javascript:"]').count()).toBe(0);
    await expect(doc).not.toContainText('A comment a reader should never see');
  });

  // spec: S32-04
  test('edge cases: the long line wraps inside the page, and a file with no final newline renders to its end', async ({ page }) => {
    await openDoc(page, h().corpus['edge-cases'], { width: 390 });
    await expect(page.locator('.cd-doc')).toContainText('has no newline after it, on purpose');
    await expect(page.locator('.cd-doc')).toContainText('eight levels down');
  });
});

test.describe('commenting on the corpus', () => {
  // spec: S32-05
  test('a selection on the RFC lands on the source range, and the next Version re-anchors it', async ({ page }) => {
    const md = corpusText('rfc-tile-cache/v1.md');
    const doc = copyOf('rfc-tile-cache', 'corpus-c1', 'rfc-tile-cache/v1.md');
    await openDoc(page, doc.id);
    const quote = 'clients recieve stale tiles';
    await selectText(page, '.cd-doc', quote);
    await comment(page, 'Typo, and is it still stale after the retry fix?');
    await expect(page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)')).toBeVisible();
    await expect(page.locator('mark.cd-hl[data-t="t1"]')).toHaveText(quote);
    const shown = thread(doc.id, 't1');
    expect(shown.anchor.original.exact).toBe(quote);
    expect(shown.anchor.original.range).toBe(rangeOf(md, quote));

    pushVersion(doc, corpusText('rfc-tile-cache/v2.md'), 'fix typos');
    await openDoc(page, doc.id);
    await expect(page.locator('mark.cd-hl.cd-st-refined[data-t="t1"]')).toHaveText('clients receive stale tiles');
  });

  // spec: S32-05
  test('moved and deleted sections of the onboarding history orphan their Threads, and the page still shows them', async ({ page }) => {
    const doc = copyOf('onboarding', 'corpus-c2', 'onboarding/v1.md');
    const md = corpusText('onboarding/v1.md');
    for (const quote of ['a map is a promise', 'take the next morning off', 'Deploys take about twelve minutes']) {
      cli(h().owner, ['thread', 'new', doc.id, rangeOf(md, quote), '-m', `about: ${quote}`]);
    }
    for (const n of [2, 3, 4]) pushVersion(doc, corpusText(`onboarding/v${n}.md`), `v${n}`);
    await openDoc(page, doc.id);
    await expect(page.locator('mark.cd-hl.cd-st-anchored[data-t="t1"]')).toHaveText('a map is a promise');
    await expect(page.locator('mark.cd-hl[data-t="t2"]')).toHaveCount(0);
    await expect(page.locator('mark.cd-hl[data-t="t3"]')).toHaveCount(0);
    const orphans = page.locator('details.cd-orphaned');
    await orphans.first().evaluate((d: HTMLDetailsElement) => { d.open = true; });
    await expect(orphans.first()).toContainText('take the next morning off');
    await expect(orphans.first()).toContainText('Deploys take about twelve minutes');
  });

  // spec: S32-05
  test('a picture in the field notes takes a whole-Asset Thread among all the others', async ({ page }) => {
    const md = corpusText('field-notes.md').replaceAll('../assets/', 'assets/');
    const doc = seedDoc('corpus-c3', md, {
      'assets/a.png': fs.readFileSync(path.join(CORPUS_DIR, '..', 'assets', 'a.png')),
      'assets/b.png': fs.readFileSync(path.join(CORPUS_DIR, '..', 'assets', 'b.png')),
    });
    await openDoc(page, doc.id);
    await page.locator('.cd-doc img[alt="First print of the overlay"]').click();
    await comment(page, 'Which handheld took this?');
    await expect(page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)')).toBeVisible();
    await expect(page.locator('img.cd-asset-hl[data-t="t1"]')).toHaveCount(1);
    const shown = thread(doc.id, 't1');
    expect(shown.subject).toBe('asset');
    expect(shown.anchor.original.ref).toBe('assets/a.png');
  });
});

test.describe('selecting non-Latin text', () => {
  const md = corpusText('glossary-multilingual/v1.md');
  const CASES: [string, string][] = [
    ['Japanese', '地図の品質にとても厳しい'],
    ['Arabic', 'تعتمد شركات العبّارات في الخليج'],
    ['Hebrew', 'המפות שלנו'],
    ['Arabic with vowel marks', 'مَكْتَبَةُ الْخَرَائِطِ'],
    ['Hebrew with Latin inside', 'דרך Mount Carmel ואורכו'],
    ['combining marks', 'Hà Nội'],
    ['a joined emoji', '👩‍👩‍👧‍👦 Family Walk'],
  ];
  for (const [label, quote] of CASES) {
    // spec: S32-06
    test(`${label}: the quote and the code-point range match the source`, async ({ page }) => {
      const doc = seedDoc(`corpus-rtl-${label.replace(/\W+/g, '-').toLowerCase()}`, md);
      await openDoc(page, doc.id);
      await selectText(page, '.cd-doc', quote);
      await comment(page, `checking: ${label}`);
      await expect(page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)')).toBeVisible();
      await expect(page.locator('mark.cd-hl[data-t="t1"]')).toHaveText(quote);
      const shown = thread(doc.id, 't1');
      expect(shown.anchor.original.exact).toBe(quote);
      expect(shown.anchor.original.range).toBe(rangeOf(md, quote));
    });
  }

  // spec: S32-06
  test('double-clicking a Hebrew word, as a reader would, selects it and makes a Thread on it', async ({ page }) => {
    const doc = seedDoc('corpus-rtl-mouse', md);
    await openDoc(page, doc.id);
    const para = page.locator('.cd-doc p', { hasText: 'מועדון הטיולים בחיפה' });
    await para.scrollIntoViewIfNeeded();
    const word = 'בחיפה';
    const box = await para.evaluate((p, w) => {
      const node = [...p.childNodes].find((n) => n.nodeType === Node.TEXT_NODE && (n.nodeValue || '').includes(w as string))!;
      const r = document.createRange();
      const at = node.nodeValue!.indexOf(w as string);
      r.setStart(node, at);
      r.setEnd(node, at + (w as string).length);
      const b = r.getBoundingClientRect();
      return { x: b.x + b.width / 2, y: b.y + b.height / 2 };
    }, word);
    await page.mouse.dblclick(box.x, box.y);
    await expect(page.locator('#cd-pill')).toBeVisible();
    await comment(page, 'Should this say Haifa in the English column?');
    await expect(page.locator('.cd-thread[data-t="t1"]:not(.cd-pending)')).toBeVisible();
    const shown = thread(doc.id, 't1');
    expect(shown.anchor.original.exact.trim()).toBe(word);
    await expect(page.locator('mark.cd-hl[data-t="t1"]')).toContainText(word);
  });
});
