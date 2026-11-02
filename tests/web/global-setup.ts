// Spawns the real servers the browser tests run against and seeds them with the real CLI:
//  - a passwordless server (owner @rowan, members @scribe / @linter) holding the sidecar-
//    prototype scenario `ra-plan` (tests/e2e/48-reanchor-seed/seed.in: v1 → v2 → v3 → v4 = v1 → v5;
//    Threads t1..t8 — document / span / asset / refined / orphaned / tombstoned; Actions on v1 and
//    v3; @mentions of @rowan) and a few Documents for the list page;
//  - a second server whose owner @ana has the local password `hunter2` (the login page).
// State goes to target/playwright/harness.json (`harness.ts`); `global-teardown.ts` kills them.
// spec: S10-10
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { execFileSync } from 'node:child_process';
import { CORPUS, REPO_ROOT, STATE_FILE, baseEnv, binary, spawnServer, tokenFrom, type Harness } from './harness';

export default async function globalSetup() {
  const bin = binary();
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'cd-web-'));

  // ticket 12 moved the spawning into `harness.ts` (`spawnServer`) so specs can start private servers.
  const main = await spawnServer({ ANNOTARY_SETUP_HANDLE: 'rowan', ANNOTARY_SETUP_NAME: 'Rowan Vale' });
  const pw = await spawnServer({ ANNOTARY_SETUP_HANDLE: 'ana', ANNOTARY_SETUP_NAME: 'Ana Lima', ANNOTARY_SETUP_PASSWORD: 'hunter2' });
  const home = main.home;

  const cli = (token: string, args: string[], cwd?: string): string =>
    execFileSync(bin, args, {
      env: { ...baseEnv(home), ANNOTARY_TOKEN: token, ANNOTARY_SERVER: main.url },
      cwd,
      stdio: ['ignore', 'pipe', 'pipe'],
    }).toString();
  const owner = main.ownerToken;
  const agent = tokenFrom(cli(owner, ['member', 'add', 'scribe', '--name', 'Scribe']));
  const linter = tokenFrom(cli(owner, ['member', 'add', 'linter', '--name', 'Linter']));

  // ---- the sidecar-prototype scenario, pushed for real (as 48-reanchor-seed does) ----
  const seed = path.join(tmp, 'seed');
  fs.cpSync(path.join(REPO_ROOT, 'tests', 'e2e', '48-reanchor-seed', 'seed.in'), seed, { recursive: true });
  const J = (...a: string[]) => cli(owner, a, seed);
  const A = (...a: string[]) => cli(agent, a, seed);
  const C = (...a: string[]) => cli(linter, a, seed);
  const pushed = J('push', 'ra-plan.md', '-m', 'first draft');
  const docId = /^ra-plan\t(\S+)\tv1/m.exec(pushed)?.[1];
  if (!docId) throw new Error('push did not print the id: ' + pushed);
  J('thread', 'new', 'ra-plan', '--document', '-m', 'Needs a **Risks** section before I can approve. @scribe can you add one?');
  J('thread', 'new', 'ra-plan', '16:0-16:33', '-m', 'Agreed — but the skill file must land *with* the CLI, not after. @scribe make sure the order list says so.');
  A('thread', 'new', 'ra-plan', '20:41-20:73', '-m', 'Three feels low for useful signal. Five?');
  J('thread', 'new', 'ra-plan', '--asset', 'img/architecture.png', '-m', 'Diagram is out of date — the daemon box is missing.');
  A('thread', 'new', 'ra-plan', '11:48-11:69', '-m', 'Should the anchor engine be its own package so the CLI and server share one implementation? @rowan @linter');
  J('act', 'ra-plan', 'send-back');
  A('thread', 'reply', 'ra-plan', 't1', '--resolve', '-m', 'Added a Risks section in v2.');
  A('thread', 'reply', 'ra-plan', 't2', '-m', 'Item 3 already lists the skill file with the CLI. @rowan do you want it as its own item?');
  A('push', 'v2/ra-plan.md', '--doc', 'ra-plan', '-m', 'address review: add Risks, split rollout');
  J('thread', 'new', 'ra-plan', '21:29-21:79', '-m', 'This is exactly what the sidecar-format ticket has to answer — link it here.');
  A('thread', 'new', 'ra-plan', '20:27-20:57', '-m', '(a question, deleted below)');
  J('thread', 'reply', 'ra-plan', 't7', '-m', '(Replying to the deleted question: yes — the fixture table is also the seed of the test suite.)');
  A('thread', 'delete', 'ra-plan', 't7');
  J('push', 'v3/ra-plan.md', '--doc', 'ra-plan');
  C('thread', 'new', 'ra-plan', '31:41-31:57', '-m', 'How — survey or interviews? Worth deciding before the invites go out. See ![arch](img/architecture.png) and [notes](api-notes.md).');
  J('thread', 'reply', 'ra-plan', 't3', '--resolve', '-m', 'Done — five, and two weeks of feedback.');
  J('act', 'ra-plan', 'approve');
  J('push', 'v4/ra-plan.md', '--doc', 'ra-plan', '-m', 'back to the first-draft wording for comparison');
  J('push', 'v5/ra-plan.md', '--doc', 'ra-plan', '-m', 'well before');
  // t9: fully tombstoned (root deleted, no replies) — pages must hide it.
  J('thread', 'new', 'ra-plan', '3:0-3:4', '-m', 'to be deleted');
  J('thread', 'delete', 'ra-plan', 't9');

  // ---- Documents for the list page ----
  const lst = path.join(tmp, 'lst');
  fs.mkdirSync(lst, { recursive: true });
  const write = (name: string, text: string) => fs.writeFileSync(path.join(lst, name), text);
  write('lst-one.md', '---\ntitle: The first list Document\n---\n\n# Not the title\n\nbody one\n');
  write('lst-gone.md', '# Gone\n\nthis one is deleted\n');
  write('lst-me.md', '# Mentions me\n\na Thread here mentions @rowan\n');
  write('lst-ok.md', '# Approved\n\napproved on head\n');
  const L = (...a: string[]) => cli(owner, a, lst);
  const idOf = (out: string) => /^\S+\t(\S+)\tv1/m.exec(out)![1];
  const one = idOf(L('push', 'lst-one.md'));
  const gone = idOf(L('push', 'lst-gone.md'));
  const me = idOf(L('push', 'lst-me.md'));
  const ok = idOf(L('push', 'lst-ok.md'));
  cli(agent, ['thread', 'new', 'lst-me', '--document', '-m', '@rowan please look'], lst);
  L('act', 'lst-ok', 'approve');
  L('delete', 'lst-gone');

  // ---- ticket 11: a second Vault `web` for the Documents the commenting / keyboard / live specs
  // push per test, so their mentions and Actions never leak into the list page's unfiltered
  // assertions over `notes` (created after the seed: `push` without --vault needs one Vault) ----
  const vres = await fetch(main.url + '/api/v1/vaults', {
    method: 'POST',
    headers: { Authorization: `Bearer ${owner}`, 'Content-Type': 'application/json' },
    body: JSON.stringify({ name: 'web' }),
  });
  if (vres.status !== 201) throw new Error('creating the web Vault: HTTP ' + vres.status + ' ' + (await vres.text()));

  // ---- the made-up corpus, into `web` as well: read-only views for corpus.spec.ts. Pushed from a
  // copy, because `push` writes a Sidecar beside each file ----
  const corpusCopy = path.join(tmp, 'fixtures');
  fs.cpSync(path.join(REPO_ROOT, 'fixtures', 'corpus'), path.join(corpusCopy, 'corpus'), { recursive: true });
  fs.cpSync(path.join(REPO_ROOT, 'fixtures', 'assets'), path.join(corpusCopy, 'assets'), { recursive: true });
  const corpus: Record<string, string> = {};
  for (const name of CORPUS) {
    const out = cli(owner, ['push', `${name}.md`, '--vault', 'web'], path.join(corpusCopy, 'corpus'));
    const m = new RegExp(`^${name}\\t(\\S+)\\tv1`, 'm').exec(out);
    if (!m) throw new Error('corpus push did not print the id: ' + out);
    corpus[name] = m[1];
  }

  const state: Harness = {
    url: main.url,
    pwUrl: pw.url,
    pids: [main.pid, pw.pid],
    doc: { id: docId, name: 'ra-plan', head: 5 },
    list: { one, gone, me, ok },
    corpus,
    owner,
    agent,
    homes: [home, pw.home, tmp],
  };
  fs.mkdirSync(path.dirname(STATE_FILE), { recursive: true });
  fs.writeFileSync(STATE_FILE, JSON.stringify(state, null, 2));
  // Keep the children alive beyond this function (the processes are killed in teardown).
  main.child.unref();
  pw.child.unref();
}
