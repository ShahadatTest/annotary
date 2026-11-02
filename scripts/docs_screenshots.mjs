#!/usr/bin/env node
// Makes the screenshots the README shows, from the made-up corpus in fixtures/corpus/ on a throwaway
// server in a temp HOME. Never real Documents.
//
//     make build && npm ci && node scripts/docs_screenshots.mjs
//
// Writes docs/images/document-view.png.
import { execFileSync, spawn } from 'node:child_process';
import { copyFileSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { chromium } from '@playwright/test';

const root = resolve(import.meta.dirname, '..');
const bin = join(root, 'target/debug/annotary');
const home = mkdtempSync(join(tmpdir(), 'annotary-screenshots-'));
const port = 7391;
const url = `http://127.0.0.1:${port}`;
const env = { ...process.env, HOME: home, USER: 'rowan', ANNOTARY_SERVER: url };
for (const k of ['ANNOTARY_HOME', 'ANNOTARY_TOKEN', 'ANNOTARY_PROFILE']) delete env[k];

const server = spawn(bin, ['serve', '--port', String(port)], { cwd: home, env, stdio: ['pipe', 'pipe', 'inherit'] });
server.stdin.end('rowan\nRowan Vale\n\n');
const cli = (token, ...args) =>
  execFileSync(bin, args, { cwd: home, env: { ...env, ANNOTARY_TOKEN: token }, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'] }).trim();

try {
  const owner = await new Promise((done, fail) => {
    let out = '';
    server.stdout.on('data', (d) => {
      out += d;
      const m = out.match(/ANNOTARY_TOKEN=(\S+)/);
      if (m) done(m[1]);
    });
    server.on('exit', () => fail(new Error('the server exited')));
  });
  await waitForServer();
  const scribe = cli(owner, 'member', 'add', 'scribe', '--name', 'Scribe', '-q').split('=')[1];
  copyFileSync(join(root, 'fixtures/corpus/rfc-tile-cache.md'), join(home, 'rfc-tile-cache.md'));
  const id = JSON.parse(cli(owner, 'push', 'rfc-tile-cache.md', '-m', 'first draft', '--json', 'id')).id;

  const source = readFileSync(join(home, 'rfc-tile-cache.md'), 'utf8').split('\n');
  const span = (quote) => {
    const n = source.findIndex((l) => l.includes(quote));
    if (n < 0) throw new Error(`not in the Document: ${quote}`);
    const at = [...source[n].slice(0, source[n].indexOf(quote))].length;
    return `${n + 1}:${at}-${n + 1}:${at + [...quote].length}`;
  };
  const thread = (token, quote, body) => cli(token, 'thread', 'new', id, span(quote), '-m', body, '-q');
  thread(owner, 'a small in-process cache', 'What happens to this tier when an edge node restarts? @scribe');
  cli(scribe, 'thread', 'reply', id, 't1', '-m', 'It starts cold and fills from the object store, so the renderer never sees those misses.', '-q');
  thread(scribe, 'versioned key', 'Old versions stay in the object store until they expire. Do we want a lifetime in this RFC?');
  thread(owner, 'rolling one back is instant', 'Only if the old style is still cached. Worth saying so?');

  const browser = await chromium.launch();
  const page = await browser.newPage({ viewport: { width: 1280, height: 800 }, deviceScaleFactor: 2, colorScheme: 'light' });
  await page.goto(`${url}/d/${id}`);
  await page.waitForSelector('.cd-thread');
  await page.screenshot({ path: join(root, 'docs/images/document-view.png') });
  await browser.close();
} finally {
  server.kill();
  rmSync(home, { recursive: true, force: true });
}

async function waitForServer() {
  for (let i = 0; i < 100; i++) {
    try {
      if ((await fetch(`${url}/api/v1/health`)).ok) return;
    } catch {}
    await new Promise((r) => setTimeout(r, 100));
  }
  throw new Error('the server did not start');
}
