// Spawns a **release** `annotary serve` on a temp HOME and seeds it with the generated perf
// fixture through the real binary (`perfgen seed`: `member add`, five `push`es of the ≥ 5,000-line
// Document — rendering and re-anchoring at the edge — and `/api/v1` for the ≥ 500 Threads, Replies,
// resolves, Actions, plus 60 small Documents for the list). State goes to target/perf/harness.json.
// `ANNOTARY_BIN` / `PERFGEN_BIN` name the binaries (`make perf` sets them to target/release/…).
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { REPO_ROOT, spawnServer } from '../tests/web/harness';

export const STATE_FILE = path.join(REPO_ROOT, 'target', 'perf', 'harness.json');

export interface PerfHarness {
  url: string;
  pid: number;
  home: string;
  work: string;
  owner: string;
  /** What `perfgen seed --json` printed. */
  seeded: {
    doc_id: string;
    name: string;
    head: number;
    threads: number;
    statuses: { anchored: number; refined: number; orphaned: number; document: number };
    resolved: number;
    replies: number;
    list_ids: string[];
    tokens: [string, string][];
    pushes_ms: number[];
    elapsed_ms: number;
    stats: Record<string, unknown>;
  };
  bin: string;
  scale: string;
  seed: number;
}

export function perfgenBinary(): string {
  return process.env.PERFGEN_BIN || path.join(REPO_ROOT, 'target', 'release', 'perfgen');
}

export default async function globalSetup() {
  if (!process.env.ANNOTARY_BIN) {
    // The budgets are about the product: a release build, never the debug binary.
    process.env.ANNOTARY_BIN = path.join(REPO_ROOT, 'target', 'release', 'annotary');
  }
  const bin = process.env.ANNOTARY_BIN;
  const gen = perfgenBinary();
  for (const [what, p] of [['annotary', bin], ['perfgen', gen]] as const) {
    if (!fs.existsSync(p)) throw new Error(`${what} binary not found at ${p} — run \`make perf\` (it builds the release binaries first)`);
  }
  const scale = process.env.PERF_SCALE || 'full';
  const seed = Number(process.env.PERF_SEED || '1');
  const srv = await spawnServer({ ANNOTARY_SETUP_HANDLE: 'rowan', ANNOTARY_SETUP_NAME: 'Rowan Vale' });
  const work = fs.mkdtempSync(path.join(os.tmpdir(), 'cd-perf-'));
  const t0 = Date.now();
  // Asynchronous on purpose: the server's stderr is a pipe this process drains (`spawnServer`);
  // a synchronous exec here would block the event loop, fill that pipe and stall the server.
  const { stdout, stderr } = await promisify(execFile)(gen, [
    'seed', '--bin', bin, '--server', srv.url, '--token', srv.ownerToken,
    '--home', path.join(work, 'home'), '--work', path.join(work, 'work'),
    '--scale', scale, '--seed', String(seed), '--json',
  ], { maxBuffer: 64 * 1024 * 1024 });
  if (stderr) process.stderr.write(stderr);
  const seeded = JSON.parse(stdout.trim().split('\n').pop() || '{}') as PerfHarness['seeded'];
  // eslint-disable-next-line no-console
  console.log(`perf: seeded ${seeded.name} ${seeded.doc_id} v${seeded.head} — ${seeded.threads} Threads (${seeded.statuses.anchored} anchored, ${seeded.statuses.refined} refined, ${seeded.statuses.orphaned} orphaned), ${seeded.replies} replies, ${seeded.list_ids.length} list Documents in ${Date.now() - t0} ms (pushes: ${seeded.pushes_ms.join(' / ')} ms)`);
  const state: PerfHarness = { url: srv.url, pid: srv.pid, home: srv.home, work, owner: srv.ownerToken, seeded, bin, scale, seed };
  fs.mkdirSync(path.dirname(STATE_FILE), { recursive: true });
  fs.writeFileSync(STATE_FILE, JSON.stringify(state, null, 2));
  srv.child.unref();
}
