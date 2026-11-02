// Kills the servers `global-setup.ts` spawned and removes their temp homes.
import * as fs from 'node:fs';
import { STATE_FILE, type Harness } from './harness';

export default async function globalTeardown() {
  if (!fs.existsSync(STATE_FILE)) return;
  const state = JSON.parse(fs.readFileSync(STATE_FILE, 'utf8')) as Harness;
  for (const pid of state.pids) {
    try { process.kill(pid, 'SIGTERM'); } catch { /* already gone */ }
  }
  // Give them a moment to exit cleanly (server.json removal), then drop the temp dirs.
  await new Promise((r) => setTimeout(r, 300));
  for (const pid of state.pids) {
    try { process.kill(pid, 'SIGKILL'); } catch { /* gone */ }
  }
  if (!process.env.CD_WEB_KEEP) {
    for (const h of state.homes) {
      try { fs.rmSync(h, { recursive: true, force: true }); } catch { /* best effort */ }
    }
    try { fs.unlinkSync(STATE_FILE); } catch { /* fine */ }
  }
}
