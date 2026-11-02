// Kills the release server `global-setup.ts` spawned and removes its temp dirs
// (`CD_PERF_KEEP=1` keeps them for a look at the seeded store).
import * as fs from 'node:fs';
import { STATE_FILE, type PerfHarness } from './global-setup';

export default async function globalTeardown() {
  if (!fs.existsSync(STATE_FILE)) return;
  const state = JSON.parse(fs.readFileSync(STATE_FILE, 'utf8')) as PerfHarness;
  try { process.kill(state.pid, 'SIGTERM'); } catch { /* gone */ }
  await new Promise((r) => setTimeout(r, 400));
  try { process.kill(state.pid, 'SIGKILL'); } catch { /* gone */ }
  if (!process.env.CD_PERF_KEEP) {
    for (const d of [state.home, state.work]) {
      try { fs.rmSync(d, { recursive: true, force: true }); } catch { /* best effort */ }
    }
  }
}
