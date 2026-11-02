// The budgets live in perf/BUDGETS.md and nowhere else (ticket 15's budget-raise rule): the runner
// reads the table below "## The budgets" — `| id | metric | budget | unit | … |` — so changing a
// budget means editing that file (with the rule's record), never a constant in code.
import * as fs from 'node:fs';
import * as path from 'node:path';

export const PERF_DIR = __dirname;
export const BUDGETS_FILE = path.join(PERF_DIR, 'BUDGETS.md');

export interface Budget {
  id: string;
  metric: string;
  budget: number;
  unit: string;
}

/** Every id the suite asserts; the file must list exactly these (the rule allows raising a value, not dropping a budget). */
export const BUDGET_IDS = ['fcp', 'interactive', 'switch', 'list', 'cls', 'optimistic'] as const;
export type BudgetId = (typeof BUDGET_IDS)[number];

export function parseBudgets(md: string): Record<string, Budget> {
  const out: Record<string, Budget> = {};
  let inTable = false;
  for (const raw of md.split('\n')) {
    const line = raw.trim();
    if (line.startsWith('## ')) { inTable = /^## The budgets/.test(line); continue; }
    if (!inTable || !line.startsWith('|')) continue;
    const cells = line.split('|').slice(1, -1).map((c) => c.trim());
    if (cells.length < 4) continue;
    const [id, metric, budget, unit] = cells;
    if (id === 'id' || /^-+$/.test(id)) continue; // header / separator
    const value = Number(budget);
    if (!Number.isFinite(value)) throw new Error(`perf/BUDGETS.md: budget of "${id}" is not a number: ${JSON.stringify(budget)}`);
    out[id] = { id, metric, budget: value, unit };
  }
  return out;
}

export function loadBudgets(): Record<BudgetId, Budget> {
  const all = parseBudgets(fs.readFileSync(BUDGETS_FILE, 'utf8'));
  for (const id of BUDGET_IDS) {
    if (!all[id]) throw new Error(`perf/BUDGETS.md lists no budget "${id}"`);
  }
  return all as Record<BudgetId, Budget>;
}
