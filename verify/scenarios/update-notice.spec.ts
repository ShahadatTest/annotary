// The update notice on the copy's real CLI home, with the build under test: a fake Releases API on
// loopback stands in for GitHub, so nothing leaves the machine. `ANNOTARY_TTY=1` stands in for a
// terminal. On a build stamped with a real version the fake's far-future tag is still newer.
import { execFile } from 'node:child_process';
import * as fs from 'node:fs';
import * as http from 'node:http';
import type { AddressInfo } from 'node:net';
import * as path from 'node:path';
import { test } from '@playwright/test';
import { cli, documents, expect } from '../lib';

/** Runs `-V` without blocking this process's event loop, where the fake API lives. */
function version(home: string, env: Record<string, string>): Promise<{ stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    execFile(
      process.env.VERIFY_BIN!,
      ['-V'],
      { env: { PATH: process.env.PATH ?? '', HOME: process.env.VERIFY_TMP!, ANNOTARY_HOME: home, ANNOTARY_TTY: '1', ...env } },
      (error, stdout, stderr) => (error ? reject(error) : resolve({ stdout, stderr })),
    );
  });
}

/** A fake Releases API answering every request with `status` and `body`. */
async function fakeApi(status: number, body: string) {
  let asked = 0;
  const server = http.createServer((_req, res) => {
    asked += 1;
    res.writeHead(status, { 'Content-Type': 'application/json' });
    res.end(body);
  });
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const url = `http://127.0.0.1:${(server.address() as AddressInfo).port}/releases/latest`;
  return { url, asked: () => asked, close: () => new Promise((resolve) => server.close(resolve)) };
}

/** A copy of the private CLI home, so each step starts with no cached answer. */
function freshHome(name: string): string {
  const home = path.join(process.env.VERIFY_TMP!, `update-${name}`);
  fs.mkdirSync(home, { recursive: true });
  fs.copyFileSync(path.join(process.env.VERIFY_CLI_HOME!, 'config.toml'), path.join(home, 'config.toml'));
  return home;
}

test('an older install is told, once a day, and the CLI still reaches the copy', async () => {
  const api = await fakeApi(200, '{"tag_name":"v99.0.0"}');
  const home = freshHome('newer');
  const first = await version(home, { ANNOTARY_UPDATE_URL: api.url });
  expect(first.stdout).toMatch(/^annotary \d+\.\d+\.\d+\n$/);
  expect(first.stderr).toContain('annotary 99.0.0 is available');
  expect(first.stderr).toContain('docs/install.md#upgrade');
  const second = await version(home, { ANNOTARY_UPDATE_URL: api.url });
  expect(second.stderr).toContain('annotary 99.0.0 is available');
  expect(api.asked()).toBe(1);
  await api.close();
  // A normal verb on the real data prints no notice: it is on `-V` and `serve` start only.
  const file = path.join(fs.mkdtempSync(path.join(process.env.VERIFY_TMP!, 'update-doc-')), 'update-notice-check.md');
  fs.writeFileSync(file, '# Update notice\n\nThe CLI still works after the check.\n');
  expect(cli(['push', file, '-m', 'verify-live'])).not.toContain('is available');
  expect(cli(['list'])).not.toContain('is available');
  expect(documents().map((d) => d.name)).toContain('update-notice-check');
});

test('off with the setting asks nothing and says nothing', async () => {
  const api = await fakeApi(200, '{"tag_name":"v99.0.0"}');
  const home = freshHome('off-config');
  fs.writeFileSync(path.join(home, 'config.toml'), `update_check = false\n${fs.readFileSync(path.join(home, 'config.toml'), 'utf8')}`);
  const byConfig = await version(home, { ANNOTARY_UPDATE_URL: api.url });
  expect(byConfig.stderr).toBe('');
  const byEnv = await version(freshHome('off-env'), { ANNOTARY_UPDATE_URL: api.url, ANNOTARY_UPDATE_CHECK: 'off' });
  expect(byEnv.stderr).toBe('');
  expect(api.asked()).toBe(0);
  await api.close();
});

test('a repo that answers 404, or no network at all, is silent', async () => {
  const api = await fakeApi(404, '{"message":"Not Found"}');
  const gone = await version(freshHome('404'), { ANNOTARY_UPDATE_URL: api.url });
  expect(gone.stderr).toBe('');
  expect(api.asked()).toBe(1);
  const closed = api.url;
  await api.close();
  const offline = await version(freshHome('offline'), { ANNOTARY_UPDATE_URL: closed });
  expect(offline.stderr).toBe('');
});
