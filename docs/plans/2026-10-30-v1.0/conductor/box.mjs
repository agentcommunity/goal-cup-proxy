// Conductor tool for Vercel Sandbox build boxes. Credentials come from env only; never argv, never printed.
//
//   node box.mjs run --repo DIR --ref REF --script FILE [--snapshot ID] [--minutes N] [--vcpus N] [--out BOXPATH:LOCALPATH ...]
//      Bundles REF of DIR, copies it into a fresh box, clones to ~/w, runs FILE (bash) there as the
//      non-root user, prints output, copies --out files back, stops the box. Exit code = script exit code.
//      --vcpus defaults to 8.
//   node box.mjs snapshot-build
//      Makes the build snapshot (Rust toolchain from rust-toolchain.toml, nextest, zstd, Node 22, Python 3)
//      and writes its id to build-snapshot.id next to this file.
//   node box.mjs list
//   node box.mjs snaps
//      One line per snapshot: id status created expires size. Marks the build snapshot [build] and the
//      pad image ($GC_REPO/scripts/pad/image.id) [pad]; ends with the running box count.
//   node box.mjs spend --since ISO [--model-usd N] [--ledger]
//      Box minutes and vCPU minutes of every box created since ISO. --ledger appends the summary line
//      to $GC_NIGHT/lanes/integrator.log.
//   node box.mjs --help
import { execFileSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import { sessionMinutes, ledgerLine } from './spend.mjs';

const USAGE = `usage: node box.mjs <command>
  run --repo DIR --ref REF --script FILE [--snapshot ID] [--minutes N] [--vcpus N] [--out BOXPATH:LOCALPATH ...]
  snapshot-build
  list
  snaps
  spend --since ISO [--model-usd N] [--ledger]
  --help
`;

async function sdk() { return await import('@vercel/sandbox'); }

const creds = { teamId: process.env.VERCEL_TEAM_ID, projectId: process.env.VERCEL_PROJECT_ID, token: process.env.VERCEL_TOKEN };
const HERE = path.dirname(new URL(import.meta.url).pathname);
const TOOLS = process.env.GC_TOOLS ?? HERE;
const SNAP_FILE = path.join(TOOLS, 'build-snapshot.id');

function arg(name, def) { const i = process.argv.indexOf('--' + name); return i > 0 ? process.argv[i + 1] : def; }
function args(name) { const out = []; process.argv.forEach((a, i) => { if (a === '--' + name) out.push(process.argv[i + 1]); }); return out; }
function log(...m) { process.stderr.write('[box] ' + m.join(' ') + '\n'); }

async function create(snapshot, minutes, vcpus = 8) {
  const { Sandbox } = await sdk();
  for (let attempt = 1; ; attempt++) {
    try {
      const p = { ...creds, resources: { vcpus }, timeout: minutes * 60 * 1000 };
      if (snapshot) p.source = { type: 'snapshot', snapshotId: snapshot }; else p.runtime = 'node22';
      return await Sandbox.create(p);
    } catch (e) {
      log('create failed attempt', attempt, String(e.message || e).slice(0, 200));
      if (attempt >= 6) throw e;
      await new Promise(r => setTimeout(r, Math.min(300, 15 * 2 ** attempt) * 1000));
    }
  }
}

async function sh(sb, script, { sudo = false, cwd } = {}) {
  const r = await sb.runCommand({ cmd: 'bash', args: ['-lc', script], sudo, cwd,
    stdout: process.stdout, stderr: process.stderr });
  return r.exitCode;
}

async function run() {
  const repo = arg('repo'); const ref = arg('ref', 'HEAD'); const scriptFile = arg('script');
  const minutes = Number(arg('minutes', '45'));
  const vcpus = Number(arg('vcpus', '8'));
  const snapshot = arg('snapshot', fs.existsSync(SNAP_FILE) ? fs.readFileSync(SNAP_FILE, 'utf8').trim() : undefined);
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'box-'));
  const bundle = path.join(tmp, 'repo.bundle');
  const sha = execFileSync('git', ['-C', repo, 'rev-parse', ref + '^{commit}']).toString().trim();
  const tmpRef = `refs/box/job-${process.pid}-${Date.now()}`;
  execFileSync('git', ['-C', repo, 'update-ref', tmpRef, sha]);
  try { execFileSync('git', ['-C', repo, 'bundle', 'create', bundle, tmpRef], { stdio: ['ignore', 'ignore', 'inherit'] }); }
  finally { execFileSync('git', ['-C', repo, 'update-ref', '-d', tmpRef]); }
  const sb = await create(snapshot, minutes, vcpus);
  log('box', sb.name, 'ref', sha.slice(0, 12), snapshot ? 'snapshot' : 'base');
  let code = 1;
  try {
    await sb.writeFiles([
      { path: '/tmp/repo.bundle', content: fs.readFileSync(bundle) },
      { path: '/tmp/job.sh', content: fs.readFileSync(scriptFile), mode: 0o755 },
    ]);
    const c0 = await sh(sb, `rm -rf ~/w && git init -q ~/w && cd ~/w && git fetch -q /tmp/repo.bundle '+refs/*:refs/remotes/b/*' && git checkout -q ${sha} && (source ~/.cargo/env; rustup default 1.99.0 >/dev/null 2>&1; true)`);
    if (c0 !== 0) throw new Error('clone in box failed ' + c0);
    code = await sh(sb, 'cd ~/w && source ~/.cargo/env 2>/dev/null; bash /tmp/job.sh');
    for (const spec of args('out')) {
      const [from, to] = spec.split(':');
      const buf = await sb.readFileToBuffer({ path: from.startsWith('/') ? from : '/home/vercel-sandbox/w/' + from });
      if (buf) { fs.mkdirSync(path.dirname(to), { recursive: true }); fs.writeFileSync(to, buf); log('copied out', from); }
      else log('missing out', from);
    }
  } finally {
    await sb.stop().catch(e => log('stop failed', e.message));
    fs.rmSync(tmp, { recursive: true, force: true });
  }
  log('exit', code);
  process.exit(code);
}

async function snapshotBuild() {
  const repo = arg('repo', process.env.GC_REPO);
  const sb = await create(undefined, 60);
  log('snapshot box', sb.name);
  const tc = fs.readFileSync(path.join(repo, 'rust-toolchain.toml'));
  await sb.writeFiles([{ path: '/tmp/rust-toolchain.toml', content: tc }]);
  const steps = `set -e
sudo dnf install -y -q gcc gcc-c++ make cmake clang perl openssl-devel pkgconf zstd python3 python3-pip sqlite tar xz git >/dev/null
curl -sSf https://sh.rustup.rs | sh -s -- -y -q --profile minimal --default-toolchain none
source ~/.cargo/env
mkdir -p /tmp/tc && cp /tmp/rust-toolchain.toml /tmp/tc/ && cd /tmp/tc && rustup show >/dev/null && rustc --version
curl -LsSf https://get.nexte.st/latest/linux | tar zxf - -C ~/.cargo/bin
cargo nextest --version
node --version; python3 --version; zstd --version
`;
  const code = await sh(sb, steps);
  if (code !== 0) { await sb.stop(); throw new Error('snapshot setup failed ' + code); }
  const snap = await sb.snapshot({ expiration: 0 });
  const id = snap.snapshotId ?? snap.id;
  fs.writeFileSync(SNAP_FILE, id + '\n');
  log('snapshot', id);
}

async function pages(p) { const out = []; for await (const x of await p) out.push(x); return out; }

async function list() {
  const { Sandbox } = await sdk();
  for (const s of await pages(Sandbox.list({ ...creds }))) console.log(s.name ?? s.id, s.status, s.createdAt);
}

function readId(file) { try { return fs.readFileSync(file, 'utf8').trim(); } catch { return ''; } }
function iso(ms) { return ms ? new Date(ms).toISOString() : 'none'; }

async function snaps() {
  const { Sandbox, Snapshot } = await sdk();
  const build = readId(SNAP_FILE);
  const pad = process.env.GC_REPO ? readId(path.join(process.env.GC_REPO, 'scripts/pad/image.id')) : '';
  for (const s of await pages(Snapshot.list({ ...creds }))) {
    const id = s.id ?? s.snapshotId;
    const mark = (id === build ? ' [build]' : '') + (id === pad ? ' [pad]' : '');
    const gb = ((s.sizeBytes ?? 0) / 1024 ** 3).toFixed(2) + 'GB';
    console.log(id, s.status, iso(s.createdAt), iso(s.expiresAt), gb + mark);
  }
  const boxes = await pages(Sandbox.list({ ...creds }));
  const running = boxes.filter(b => b.status === 'running').length;
  console.log(`running ${running} of ${boxes.length} boxes`);
}

async function spend() {
  const since = arg('since');
  if (!since || Number.isNaN(Date.parse(since))) { console.error('spend needs --since ISO'); process.exit(2); }
  const modelArg = arg('model-usd');
  const { Sandbox } = await sdk();
  const sinceMs = Date.parse(since);
  const nowMs = Date.now();
  let boxes = 0; let minutes = 0; let vcpuMinutes = 0;
  for (const s of await pages(Sandbox.list({ ...creds }))) {
    if (s.createdAt < sinceMs) continue;
    const sb = await Sandbox.get({ ...creds, name: s.name });
    const t = sessionMinutes(await pages(sb.listSessions()), nowMs);
    console.log(s.name, iso(s.createdAt), t.minutes + ' min', t.vcpuMinutes + ' vcpu-min');
    boxes++; minutes += t.minutes; vcpuMinutes += t.vcpuMinutes;
  }
  const t = { boxes, minutes: Math.round(minutes * 10) / 10, vcpuMinutes: Math.round(vcpuMinutes * 10) / 10, since };
  if (modelArg !== undefined) t.modelUsd = Number(modelArg);
  const line = ledgerLine(t, new Date());
  console.log(line);
  if (process.argv.includes('--ledger')) {
    const dir = path.join(process.env.GC_NIGHT ?? path.join(os.homedir(), 'gc-night'), 'lanes');
    fs.mkdirSync(dir, { recursive: true });
    fs.appendFileSync(path.join(dir, 'integrator.log'), line + '\n');
  }
}

const cmd = process.argv[2];
if (cmd === '--help' || cmd === '-h' || cmd === 'help') { process.stdout.write(USAGE); process.exit(0); }
else if (cmd === 'run') await run();
else if (cmd === 'snapshot-build') await snapshotBuild();
else if (cmd === 'list') await list();
else if (cmd === 'snaps') await snaps();
else if (cmd === 'spend') await spend();
else { process.stderr.write(USAGE); process.exit(2); }
