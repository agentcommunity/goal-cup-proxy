// The one place the pad talks to Vercel Sandbox (PRD-12 sections 3, 7 and 10), through @vercel/sandbox.
// Credentials come only from VERCEL_TOKEN, VERCEL_TEAM_ID and VERCEL_PROJECT_ID in this process's
// environment: never printed, never an argument. Boxes are never persistent (PRD-12 section 7).
//
//   create --name pad-<id> --minutes N [--snapshot ID] [--vcpus 4]   prints {"id":...,"name":...}
//   put ID BOXPATH --file LOCAL [--mode 600|644|755]
//   put-env ID BOXPATH --env VAR [--line 'TEMPLATE with {key}']     value read here, written 0600
//   push-dir ID LOCALDIR BOXDIR | exec ID --script FILE [--sudo] | get ID BOXPATH LOCALPATH
//   rm ID | list [--prefix pad-] | snapshot ID                         prints {"snapshot":...}
// A box path may start with ~/ (the box user's $HOME). Exit 2: usage; 3: missing box file; 4: unset variable.
import { createRequire } from 'node:module';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';

function loadSdk() {
  const dirs = [process.env.PAD_NODE_MODULES, ...(process.env.NODE_PATH ?? '').split(':')].filter(Boolean);
  for (const d of dirs) {
    try { return createRequire(path.join(path.resolve(d), 'x.js'))('@vercel/sandbox'); } catch {}
  }
  return createRequire(import.meta.url)('@vercel/sandbox');
}
const { Sandbox } = loadSdk();
const creds = Object.fromEntries(Object.entries({ token: process.env.VERCEL_TOKEN,
  teamId: process.env.VERCEL_TEAM_ID, projectId: process.env.VERCEL_PROJECT_ID }).filter(([, v]) => v));

const argv = process.argv.slice(2);
const cmd = argv[0];
const pos = argv.slice(1).filter((a, i, all) => !a.startsWith('--') && !(i > 0 && all[i - 1].startsWith('--') && all[i - 1] !== '--sudo'));
const opt = (n, d) => { const i = argv.indexOf('--' + n); return i > 0 ? argv[i + 1] : d; };
function usage(m) { process.stderr.write(`sbx: ${m}\n`); process.exit(2); }
function need(n) { if (pos.length < n) usage(`${cmd} needs ${n} arguments`); return pos; }
const log = m => process.stderr.write(`[sbx] ${m}\n`);
const short = e => String(e?.message ?? e).slice(0, 300);

const box = name => Sandbox.get({ name, ...creds });
async function run(sb, c, args, o = {}) { return sb.runCommand({ cmd: c, args, ...o }); }
async function home(sb) { return (await (await run(sb, 'printenv', ['HOME'])).stdout()).trim(); }
async function boxPath(sb, p) { return p.startsWith('~/') ? path.posix.join(await home(sb), p.slice(2)) : p; }
async function mkdirs(sb, dirs, mode) {
  const r = await run(sb, 'mkdir', ['-p', ...(mode ? ['-m', mode] : []), ...new Set(dirs)]);
  if (r.exitCode !== 0) throw new Error('mkdir in box failed');
}
async function write(sb, file, content, mode) {
  await mkdirs(sb, [path.posix.dirname(file)], mode === 0o600 ? '700' : undefined);
  await sb.writeFiles([{ path: file, content, mode }]);
  const r = await run(sb, 'chmod', [mode.toString(8), file]);
  if (r.exitCode !== 0) throw new Error('chmod in box failed');
}

async function create() {
  const name = opt('name') ?? usage('create needs --name');
  const p = { ...creds, name, persistent: false, resources: { vcpus: Number(opt('vcpus', '4')) },
    timeout: Number(opt('minutes', '30')) * 60 * 1000 };
  if (opt('snapshot')) p.source = { type: 'snapshot', snapshotId: opt('snapshot') }; else p.runtime = 'node22';
  for (let attempt = 1; ; attempt++) {
    try {
      if (attempt > 1) { const old = await box(name).catch(() => null); if (old) return old; }
      return await Sandbox.create(p);
    } catch (e) {
      log(`create failed attempt ${attempt}: ${short(e)}`);
      if (attempt >= 6) throw e;
      await new Promise(r => setTimeout(r, Math.min(300, 15 * 2 ** attempt) * 1000));
    }
  }
}

const cmds = {
  async create() { const sb = await create(); console.log(JSON.stringify({ id: sb.name, name: sb.name })); },
  async put() {
    const [id, to] = need(2); const file = opt('file') ?? usage('put needs --file');
    const sb = await box(id); await write(sb, await boxPath(sb, to), fs.readFileSync(file), parseInt(opt('mode', '644'), 8));
  },
  async 'put-env'() {
    const [id, to] = need(2); const name = opt('env') ?? usage('put-env needs --env');
    const value = process.env[name];
    if (!value) { process.stderr.write(`sbx: ${name} is not set or empty; nothing written\n`); process.exit(4); }
    const line = opt('line');
    const sb = await box(id);
    await write(sb, await boxPath(sb, to), line === undefined ? value : line.split('{key}').join(value) + '\n', 0o600);
  },
  async 'push-dir'() {
    const [id, from, to] = need(3); const sb = await box(id); const base = await boxPath(sb, to);
    const files = [];
    (function walk(d, rel) {
      for (const e of fs.readdirSync(d, { withFileTypes: true })) {
        const full = path.join(d, e.name), r = path.posix.join(rel, e.name);
        if (e.isDirectory()) walk(full, r);
        else if (e.isFile()) files.push({ path: path.posix.join(base, r), content: fs.readFileSync(full),
          mode: fs.statSync(full).mode & 0o111 ? 0o755 : 0o644 });
      }
    })(from, '');
    await mkdirs(sb, [base, ...files.map(f => path.posix.dirname(f.path))]);
    for (let i = 0; i < files.length; i += 50) await sb.writeFiles(files.slice(i, i + 50));
    for (const mode of [0o755, 0o644]) {
      const list = files.filter(f => f.mode === mode).map(f => f.path);
      if (list.length) await run(sb, 'chmod', [mode.toString(8), ...list]);
    }
  },
  async exec() {
    const [id] = need(1); const script = opt('script') ?? usage('exec needs --script');
    const sb = await box(id); const tmp = `/tmp/pad-exec-${crypto.randomBytes(6).toString('hex')}.sh`;
    await sb.writeFiles([{ path: tmp, content: fs.readFileSync(script), mode: 0o755 }]);
    const r = await run(sb, 'bash', ['-l', tmp], { sudo: argv.includes('--sudo'), stdout: process.stdout, stderr: process.stderr });
    await run(sb, 'rm', ['-f', tmp], { sudo: true }).catch(() => {});
    process.exitCode = r.exitCode;
  },
  async get() {
    const [id, from, to] = need(3); const sb = await box(id);
    const buf = await sb.readFileToBuffer({ path: await boxPath(sb, from) });
    if (!buf) { process.stderr.write(`sbx: ${from} not found in the box\n`); process.exit(3); }
    fs.mkdirSync(path.dirname(path.resolve(to)), { recursive: true }); fs.writeFileSync(to, buf);
  },
  async rm() {
    const [id] = need(1); const sb = await box(id).catch(() => null);
    if (!sb) return;
    await sb.stop().catch(e => log(`stop: ${short(e)}`));
    await sb.delete().catch(e => log(`delete: ${short(e)}`));
  },
  async list() {
    const r = await Sandbox.list({ ...creds, ...(opt('prefix') ? { namePrefix: opt('prefix'), sortBy: 'name' } : {}) });
    const items = r.toArray ? await r.toArray() : (r.sandboxes ?? []);
    for (const s of items) console.log(JSON.stringify({ name: s.name, status: s.status, createdAt: s.createdAt }));
  },
  async snapshot() {
    const [id] = need(1); const snap = await (await box(id)).snapshot({ expiration: 0 });
    console.log(JSON.stringify({ snapshot: snap.snapshotId ?? snap.id }));
  },
};

if (!cmds[cmd]) usage('commands: ' + Object.keys(cmds).join(', '));
try { await cmds[cmd](); } catch (e) { process.stderr.write(`sbx ${cmd}: ${short(e)}\n`); process.exit(1); }
