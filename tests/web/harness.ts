// Shared harness of the Playwright suite: where the servers are and what the seed created.
// `global-setup.ts` writes `target/playwright/harness.json`; tests read it through `harness()`.
import * as fs from 'node:fs';
import * as os from 'node:os';
import * as path from 'node:path';
import { execFileSync, spawn, type ChildProcess } from 'node:child_process';

export const REPO_ROOT = path.resolve(__dirname, '..', '..');
export const STATE_FILE = path.join(REPO_ROOT, 'target', 'playwright', 'harness.json');

/** The binary under test: `ANNOTARY_BIN` or the debug build (`make e2e` builds it first), in
 * `CARGO_TARGET_DIR` when that is set (CI keeps it outside the checkout). */
export function binary(): string {
  const targetDir = process.env.CARGO_TARGET_DIR || path.join(REPO_ROOT, 'target');
  return process.env.ANNOTARY_BIN || path.resolve(REPO_ROOT, targetDir, 'debug', 'annotary');
}

export interface Harness {
  /** The seeded, passwordless server (owner @rowan is the loopback browser). */
  url: string;
  /** A second server whose owner @ana has the local password `hunter2`. */
  pwUrl: string;
  /** pids to kill in teardown. */
  pids: number[];
  /** The sidecar-prototype scenario (`ra-plan`, v1..v5, Threads t1..t8, Actions on v1 and v3). */
  doc: { id: string; name: string; head: number };
  /** Documents seeded for the list page. */
  list: { one: string; gone: string; me: string; ok: string };
  /** The made-up corpus (`fixtures/corpus/`), pushed unchanged: Document ids by Name, read-only. */
  corpus: Record<string, string>;
  owner: string;
  agent: string;
  homes: string[];
}

/** The corpus Documents the suite pushes, by Name; three have a history (`fixtures/corpus/README.md`). */
export const CORPUS = [
  'api-reference', 'edge-cases', 'field-notes', 'glossary-multilingual', 'math-and-diagrams',
  'meeting-notes-2026-03-02', 'meeting-notes-2026-03-09', 'meeting-notes-2026-03-16', 'onboarding',
  'release-checklist', 'rfc-tile-cache',
];
export const CORPUS_DIR = path.join(REPO_ROOT, 'fixtures', 'corpus');
/** One corpus file's text (`name.md`, or `name/v2.md` for a Version of a history). */
export const corpusText = (rel: string): string => fs.readFileSync(path.join(CORPUS_DIR, rel), 'utf8');

let cached: Harness | null = null;
export function harness(): Harness {
  if (!cached) cached = JSON.parse(fs.readFileSync(STATE_FILE, 'utf8')) as Harness;
  return cached;
}

/** Breakpoint of layout A (margin) vs C (inline), in container px. */
export const WIDE = 850;

// ---- ticket 11: per-test Documents, driven through the real CLI (tests that mutate never touch
// the shared `ra-plan` seed — they push their own Document first) ----

/** Run the CLI against the seeded server as `token` (`ANNOTARY_TOKEN` / `ANNOTARY_SERVER`). */
export function cli(token: string, args: string[], cwd?: string): string {
  const h = harness();
  const env: NodeJS.ProcessEnv = { ...process.env, HOME: h.homes[0], USER: 'rowan', ANNOTARY_TOKEN: token, ANNOTARY_SERVER: h.url };
  for (const k of ['ANNOTARY_HOME', 'ANNOTARY_PROFILE', 'ANNOTARY_TTY', 'ANNOTARY_PASSWORD']) delete env[k];
  return execFileSync(binary(), args, { env, cwd, stdio: ['ignore', 'pipe', 'pipe'] }).toString();
}

/** Write `<name>.md` (and any extra files) into a fresh temp dir and push it as the owner — into
 *  the `web` Vault, so the ticket-10 list assertions over `notes` never see these Documents. */
export function seedDoc(name: string, md: string, files: Record<string, Buffer | string> = {}): { id: string; dir: string; file: string } {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'cd-web-doc-'));
  for (const [rel, data] of Object.entries(files)) {
    const p = path.join(dir, rel);
    fs.mkdirSync(path.dirname(p), { recursive: true });
    fs.writeFileSync(p, data);
  }
  const file = path.join(dir, `${name}.md`);
  fs.writeFileSync(file, md);
  const out = cli(harness().owner, ['push', `${name}.md`, '--vault', 'web'], dir);
  const m = new RegExp(`^${name}\\t(\\S+)\\tv1`, 'm').exec(out);
  if (!m) throw new Error('push did not print the id: ' + out);
  return { id: m[1], dir, file };
}

/** Push a new Version of a seeded Document (rewrites the file, pushes with its Sidecar). */
export function pushVersion(doc: { dir: string; file: string }, md: string, message?: string): string {
  fs.writeFileSync(doc.file, md);
  const args = ['push', path.basename(doc.file)];
  if (message) args.push('-m', message);
  return cli(harness().owner, args, doc.dir);
}

/** `L:C-L:C` of the first occurrence of `needle` in `md` (1-based lines, 0-based code-point columns, half-open). */
export function rangeOf(md: string, needle: string): string {
  const lines = md.split('\n');
  for (let i = 0; i < lines.length; i++) {
    const k = lines[i].indexOf(needle);
    if (k >= 0) {
      const col = Array.from(lines[i].slice(0, k)).length;
      const len = Array.from(needle).length;
      return `${i + 1}:${col}-${i + 1}:${col + len}`;
    }
  }
  throw new Error(`"${needle}" not in the Markdown`);
}

// ---- ticket 12: a private server for tests that change Tenant-wide state (the local password,
// the auto-purge setting) — spawned from the spec, killed at its end, so the shared servers and the
// tests running in parallel against them never see the change. `global-setup.ts` uses it too. ----

export interface Spawned {
  url: string;
  pid: number;
  child: ChildProcess;
  ownerToken: string;
  home: string;
  /** SIGTERM the server, wait for it to exit, remove its temp home. */
  stop(): Promise<void>;
}

export function baseEnv(home: string): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { ...process.env, HOME: home, USER: 'rowan' };
  for (const k of ['ANNOTARY_HOME', 'ANNOTARY_TOKEN', 'ANNOTARY_SERVER', 'ANNOTARY_PROFILE', 'ANNOTARY_TTY', 'ANNOTARY_PASSWORD']) delete env[k];
  return env;
}

export function tokenFrom(out: string): string {
  const m = /ANNOTARY_TOKEN=(\S+)/.exec(out);
  if (!m) throw new Error('no ANNOTARY_TOKEN line in:\n' + out);
  return m[1];
}

/** First-run setup (`serve --setup-only` with the `ANNOTARY_SETUP_*` answers in `setupEnv`) in a
 *  fresh temp HOME, then a real `serve --port 0`; resolves once `/api/v1/health` answers. */
export async function spawnServer(
  setupEnv: NodeJS.ProcessEnv = { ANNOTARY_SETUP_HANDLE: 'rowan', ANNOTARY_SETUP_NAME: 'Rowan Vale' },
  // ticket 18: extra `serve` flags (e.g. `--url http://HOST:PORT` for the pairing flow)
  serveArgs: string[] = [],
): Promise<Spawned> {
  const bin = binary();
  if (!fs.existsSync(bin)) throw new Error(`annotary binary not found at ${bin} — run \`cargo build\` (make e2e does)`);
  const home = fs.mkdtempSync(path.join(os.tmpdir(), 'cd-web-srv-'));
  const out = execFileSync(bin, ['serve', '--setup-only'], { env: { ...baseEnv(home), ...setupEnv }, stdio: ['ignore', 'pipe', 'pipe'] }).toString();
  const ownerToken = tokenFrom(out);
  const child = spawn(bin, ['serve', '--port', '0', ...serveArgs], { env: baseEnv(home), stdio: ['ignore', 'ignore', 'pipe'], detached: false });
  let stderr = '';
  child.stderr?.on('data', (d) => { stderr += d.toString(); });
  const serverJson = path.join(home, '.annotary', 'server.json');
  const deadline = Date.now() + 15_000;
  while (Date.now() < deadline) {
    if (fs.existsSync(serverJson)) {
      try {
        const info = JSON.parse(fs.readFileSync(serverJson, 'utf8'));
        if (info.url) {
          const res = await fetch(info.url + '/api/v1/health').catch(() => null);
          if (res && res.ok) {
            return {
              url: info.url, pid: child.pid!, child, ownerToken, home,
              async stop() {
                const gone = new Promise<void>((resolve) => {
                  if (child.exitCode !== null) return resolve();
                  child.once('exit', () => resolve());
                  setTimeout(() => { try { process.kill(child.pid!, 'SIGKILL'); } catch { /* gone */ } resolve(); }, 3000).unref();
                });
                try { process.kill(child.pid!, 'SIGTERM'); } catch { /* gone */ }
                await gone;
                if (!process.env.CD_WEB_KEEP) { try { fs.rmSync(home, { recursive: true, force: true }); } catch { /* best effort */ } }
              },
            };
          }
        }
      } catch { /* partial write */ }
    }
    if (child.exitCode !== null) throw new Error('serve exited: ' + stderr);
    await new Promise((r) => setTimeout(r, 25));
  }
  throw new Error('server.json never appeared: ' + stderr);
}
