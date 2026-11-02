// Browser-side helpers of the ticket-11 specs: selecting rendered text the way a user would
// (a DOM Range over the text nodes of a block) and opening a Document page ready for input.
import { type Page } from '@playwright/test';
import { harness } from './harness';

export const docUrl = (id: string, v?: number) => `${harness().url}/d/${id}${v ? `/v/${v}` : ''}`;

export async function openDoc(page: Page, id: string, opts: { v?: number; width?: number } = {}) {
  await page.setViewportSize({ width: opts.width ?? 1280, height: 900 });
  await page.goto(docUrl(id, opts.v));
  await page.waitForSelector('.cd-app.cd-ready');
}

/** Wait for the page's live feed (the SSE subscription) to be connected. */
export async function waitLive(page: Page) {
  await page.waitForSelector('.cd-app.cd-live', { timeout: 10_000 });
}

/**
 * Select `text` inside the first element matching `within` (a block of the rendered Document):
 * the selection is a real DOM Range over the text nodes, so it may start and end in different
 * inline elements (a `<strong>`, a `<code>`, a link…). Whitespace in `text` matches any run.
 */
export async function selectText(page: Page, within: string, text: string) {
  await page.evaluate(([sel, t]) => {
    const root = document.querySelector(sel as string);
    if (!root) throw new Error('no element ' + sel);
    const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
      acceptNode: (n) => (n.parentElement && n.parentElement.closest('.cd-strip') ? NodeFilter.FILTER_REJECT : NodeFilter.FILTER_ACCEPT),
    });
    const nodes: Text[] = [];
    let n: Node | null;
    while ((n = walker.nextNode())) nodes.push(n as Text);
    const all = nodes.map((x) => x.nodeValue || '').join('');
    const k = all.indexOf(t as string);
    if (k < 0) throw new Error(`"${t}" not in the text of ${sel}: ${JSON.stringify(all)}`);
    const e = k + (t as string).length;
    const locate = (off: number, end: boolean) => {
      let acc = 0;
      for (const node of nodes) {
        const len = (node.nodeValue || '').length;
        if (end ? off <= acc + len : off < acc + len) return { node, off: off - acc };
        acc += len;
      }
      const last = nodes[nodes.length - 1];
      return { node: last, off: (last.nodeValue || '').length };
    };
    const s = locate(k, false), f = locate(e, true);
    const r = document.createRange();
    r.setStart(s.node, s.off);
    r.setEnd(f.node, f.off);
    const selection = window.getSelection()!;
    selection.removeAllRanges();
    selection.addRange(r);
  }, [within, text]);
}
