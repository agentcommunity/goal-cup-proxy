// Checks for box.mjs and spend.mjs. No network: a stub @vercel/sandbox stands in for the SDK.
import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const C = path.resolve(path.dirname(new URL(import.meta.url).pathname), '..');
const work = fs.mkdtempSync(path.join(os.tmpdir(), 'test-box-'));
const night = path.join(work, 'night');
fs.mkdirSync(night);
fs.copyFileSync(path.join(C, 'box.mjs'), path.join(work, 'box.mjs'));
fs.copyFileSync(path.join(C, 'spend.mjs'), path.join(work, 'spend.mjs'));
const box = path.join(work, 'box.mjs');
let passed = 0;
function ok(name) { passed++; console.log('ok ' + name); }
const env = { ...process.env, GC_NIGHT: night };
for (const k of ['VERCEL_TOKEN', 'VERCEL_TEAM_ID', 'VERCEL_PROJECT_ID']) env[k] = 'stub';

// help works with no SDK installed at all
let r = spawnSync('node', [box, '--help'], { env });
assert.equal(r.status, 0);
assert.match(r.stdout + r.stderr, /run --repo DIR --ref REF --script FILE/);
ok('help_without_sdk');

// no command exits 2
r = spawnSync('node', [box], { env });
assert.equal(r.status, 2);
ok('no_command_exit_2');

const { sessionMinutes, ledgerLine } = await import(path.join(work, 'spend.mjs'));
assert.deepEqual(sessionMinutes([{ vcpus: 2, startedAt: 0, createdAt: 0, stoppedAt: 600000 }, { vcpus: 4, createdAt: 0 }], 300000), { minutes: 15, vcpuMinutes: 40 });
ok('session_minutes');
assert.equal(ledgerLine({ boxes: 3, minutes: 12.5, vcpuMinutes: 50, since: '2026-10-30T00:00:00Z', modelUsd: 27 }, new Date('2026-11-01T23:10:00Z')),
  '23:10Z | conductor spend | 3 boxes since 2026-10-30T00:00:00Z: 12.5 box minutes, 50 vCPU minutes; model about 27 USD billed');
ok('ledger_line');

// stub SDK: two pages of sandboxes, one page of snapshots
const sdk = path.join(work, 'node_modules/@vercel/sandbox');
fs.mkdirSync(sdk, { recursive: true });
fs.writeFileSync(path.join(sdk, 'package.json'), '{"name":"@vercel/sandbox","type":"module","exports":"./index.mjs"}');
fs.writeFileSync(path.join(sdk, 'index.mjs'), `
const since = Date.parse('2026-10-30T00:00:00Z');
const boxes = [
  [{ name: 'a', status: 'stopped', createdAt: since + 1000 }],
  [{ name: 'b', status: 'running', createdAt: since + 2000 }, { name: 'old', status: 'stopped', createdAt: since - 86400000 }],
];
const sessions = {
  a: [{ vcpus: 2, createdAt: since + 1000, startedAt: since + 1000, stoppedAt: since + 1000 + 600000 }],
  b: [{ vcpus: 4, createdAt: since + 2000, startedAt: since + 2000, stoppedAt: since + 2000 + 300000 }],
  old: [{ vcpus: 8, createdAt: since - 86400000, stoppedAt: since - 86400000 + 60000 }],
};
function pager(pages) {
  return { async *[Symbol.asyncIterator]() { for (const p of pages) yield* p; } };
}
export const Sandbox = {
  async list() { return pager(boxes); },
  async get({ name }) { return { async listSessions() { return pager([sessions[name]]); } }; },
};
export const Snapshot = {
  async list() { return pager([[
    { id: 'snap_build', status: 'created', createdAt: since, expiresAt: since + 86400000, sizeBytes: 3 * 1024 * 1024 * 1024 },
    { id: 'snap_pad', status: 'created', createdAt: since, sizeBytes: 1024 * 1024 * 1024 },
    { id: 'snap_other', status: 'created', createdAt: since, sizeBytes: 1024 },
  ]]); },
};
`);

r = spawnSync('node', [box, 'spend', '--since', '2026-10-30T00:00:00Z'], { env, encoding: 'utf8' });
assert.equal(r.status, 0, r.stderr);
assert.match(r.stdout, /2 boxes since 2026-10-30T00:00:00Z: 15 box minutes, 40 vCPU minutes/);
ok('spend_pages_and_since');

spawnSync('node', [box, 'spend', '--since', '2026-10-30T00:00:00Z', '--ledger'], { env, encoding: 'utf8' });
assert.equal(fs.readFileSync(path.join(night, 'lanes/integrator.log'), 'utf8').trim().split('\n').length, 1);
ok('spend_ledger');

// snaps marks the build and pad ids
const repo = path.join(work, 'repo');
fs.mkdirSync(path.join(repo, 'scripts/pad'), { recursive: true });
fs.writeFileSync(path.join(repo, 'scripts/pad/image.id'), 'snap_pad\n');
fs.writeFileSync(path.join(work, 'build-snapshot.id'), 'snap_build\n');
r = spawnSync('node', [box, 'snaps'], { env: { ...env, GC_REPO: repo, GC_TOOLS: work }, encoding: 'utf8' });
assert.equal(r.status, 0, r.stderr);
assert.match(r.stdout, /snap_build .* \[build\]/);
assert.match(r.stdout, /snap_pad .* \[pad\]/);
assert.match(r.stdout, /running 1 of 3 boxes/);
ok('snaps_marks_ids');

fs.rmSync(work, { recursive: true, force: true });
console.log(`test_box: ${passed} passed`);
