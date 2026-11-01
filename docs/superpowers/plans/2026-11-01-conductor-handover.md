# Conductor Handover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Commit to `v1.0/integration` everything a fresh cloud session in a new container needs to keep conducting the goalcup 1.0 build: portable conductor tools, the sandbox proxy patch, the ledger, reports, spend, owner drafts and a start prompt, with a keyscan that proves no secret went in.

**Architecture:** All conductor files live in `docs/plans/2026-10-30-v1.0/conductor/`. Every script finds its paths through one sourced `lib.sh` (shell) or `args.paths` (lane script), never a literal `/root` or `/home/user` path, and every committing script takes the attribution trailer from `GC_SESSION`. `install.sh` copies the folder to a working location, runs `npm ci`, applies the sandbox proxy patch and, with `--night`, rebuilds the container state folder from the committed ledger; `handoff.sh` does the reverse (state folder to repo) behind `keyscan.py`.

**Tech Stack:** bash, Node 22 ESM (`@vercel/sandbox` 3.9.1, `undici`), Python 3.9 standard library (the box default), git.

**Spec:** `/root/gc-night/finish/FINISH-PLAN.md` (Wave 0, "Conductor" and "Scribe" lanes; "Order matters if the new session is a fresh container"; section 4 rules 10 to 12), with `/root/gc-night/finish/NEW-SESSION-PROMPT.md` (draft prompt) and `/root/gc-night/lanes/integrator.log` (owner answers).

## Global Constraints

- Target layout `docs/plans/2026-10-30-v1.0/conductor/` (below: `$C`). Work on branch `v1.0/conductor-handover` from `origin/v1.0/integration` (at `04044320` or later) in worktree `$WT=$GC_WT/conductor-handover`; never edit `/home/user/goal-cup-proxy`'s own checkout.
- Push only `v1.0/*` branches, merge commits only, never force. No production, `main`, tags or website database writes.
- The tool commits reach integration only through `merge.sh` (they are not docs-only), as ONE small train of their own (this branch only, one gate), not folded into the code-wave train: the new session needs these tools first, and the mini-test wave runs them from `/root/gc-tools` (Task 10 Step 8). `handoff.sh` keeps its direct docs push for the ledger and reports.
- No key on any command line, in any file, log or commit. Keys reach a box only through `sbx.mjs put-env` (0600). `GH_TOKEN`, `DATABASE_URL`, `CLOUDFLARE_*`, `GOALCUP_TEST_APPROVAL_KEY`, `GOALCUP_TEST_MEMBER_ID` are unset for every box call except the one `put-env` that needs that one value.
- Never commit `pad-out/`, `runs/`, `dbfetch/idx/`, `.env*`, key files or `node_modules/`. Copy out only `REPORT.txt` files from pad folders.
- Never edit `OWNER-DECISIONS.md`; draft lines go to `$C/owner-decisions-draft.md`.
- Docs: plain English, no em dashes, no model names or model ids in new prose (records copied verbatim, the ledger and reports, are exempt).
- Commit trailers in this session: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6`. In the scripts this becomes `GC_SESSION` (required to commit) and `GC_COAUTHOR` (default `Claude Opus 5.5 <noreply@anthropic.com>`).
- Box rules (owner-approved execution shape, 1 Nov, overrides the spec): no box per task or fix round; reviewers are read-only; this slice gets ONE box at its end (Task 10 Step 4, `--vcpus 2 --minutes 15`) that proves every red commit fails and the head passes; its merge is the one small train of its own (Task 10 Step 7), one gate, queued as soon as the slice box and the final review pass. Tests never run on the conductor machine. Conductor-side actions that are not tests: `keyscan.py` with the real env values and the `box.mjs snaps`/`spend` API calls (no box).
- Every task commits its test alone first, with ` [red]` at the end of the subject, then the implementation; `test/red-green.sh` (Task 10) replays those commits in the one box. Commit messages carry the two trailer lines above; push after each task: `git -C "$WT" push -q origin HEAD:v1.0/conductor-handover`.
- Secret names the new environment needs (names only, `SECRETS.md`, `keyscan.NAMES`): `VERCEL_TOKEN`, `VERCEL_TEAM_ID`, `VERCEL_PROJECT_ID`, `CLAUDE_API_KEY`, `OPENAI_API_KEY`, `DEEPSEEK_API_KEY`, `GEMINI_API_KEY`, `AI_GATEWAY_API_KEY`, `DATABASE_URL`, `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_API_TOKEN`, `GOALCUP_TEST_APPROVAL_KEY`, `GOALCUP_TEST_MEMBER_ID`, `GH_TOKEN`. (The spec lists the first nine; the last five are read by `scripts/pad/pad:35-38` and `scripts/fetch-run.sh` and were missing from it.)

## Review Focus

1. A new session that forgets `GC_SESSION` commits with an empty or the old session trailer. Expect: `merge.sh`, `scribe.sh`, `handoff.sh` exit 2 before any git write, and `gc-lane.js` throws at start. Tests: Task 1 `t_merge_refuses_without_session`, `t_scribe_refuses_without_session`; Task 4 `missing_session_throws`.
2. `keyscan.py` run in a shell where the secrets are not set reports "0 hits" and is taken as proof. Expect: exit 3 and the unset names listed. Test: Task 7 `test_no_values_exit_3`.
3. A secret in a dotfile (`.env.local`) or a `.py` file is missed (the old scanners used `glob("**/*")`, which skips dotfiles, and skipped `.py`). Expect: every regular file outside `.git/` is scanned. Test: Task 7 `test_dotfile_and_py_scanned`.
4. A fresh container restored from the committed ledger re-appends the whole ledger to `NIGHT-LOG.md` on the first `scribe.sh`. Expect: the `.scribed` counters come back verbatim, and a log never scribed (today `pad-model.log`) stays without one. Test: Task 8 `t_restore_keeps_scribed`.
5. `npm ci` in a new container installs an SDK whose `base-client.js` no longer has the line the patch rewrites, so boxes fail later behind the proxy with no clear cause. Expect: `patch-sandbox.sh` exits 5 naming the file, and `package.json` pins `@vercel/sandbox` to exactly `3.9.1`. Test: Task 3 `t_patch_unknown_layout_exit5`.

---

## File Structure

| Path under `$C` | Responsibility | Task |
|---|---|---|
| `lib.sh` (new) | `GC_TOOLS`, `GC_NIGHT`, `GC_WT`, `GC_REPO`, `gc_trailer` for every shell tool | 1 |
| `merge.sh`, `mergeq.sh`, `scribe.sh` (modify), `mq-driver.sh` (new copy) | merge queue and NIGHT-LOG scribe, now path- and trailer-free | 1 |
| `box.mjs` (modify), `spend.mjs` (new), `package.json`, `package-lock.json`, `.gitignore` (new) | build boxes, `--help`, `--vcpus`, `snaps`, `spend`; replaces `snaps.mjs`, `snaps2.mjs` | 2 |
| `install.sh`, `patch-sandbox.sh`, `proxy-preload.cjs` (new) | set up a working copy in any container, sandbox proxy patch | 3, 8 |
| `gc-lane.js` (modify) | lane workflow, `args.session` and `args.paths` | 4 |
| `padrun.sh`, `padbox.sh` (new) | pad drivers from `gw3/padrun.sh` and `rung1/padbox.sh` | 5 |
| `dbfetch/idx.sh`, `dbfetch/install.sh`, `dbfetch/run.sh` (new) | the conductor's short-lived index box | 6 |
| `keyscan.py` (new) | scan and `--scrub`; replaces every `keyscan.py`, `scrub.py`, `scrub2.py` copy | 7 |
| `handoff.sh` (modify), `ledger/`, `reports/` | state folder to repo behind keyscan; restore counterpart in `install.sh --night` | 8 |
| `handoff-head.md` (modify), `STATE.md`, `SECRETS.md`, `snapshots.md`, `owner-decisions-draft.md`, `NEW-SESSION-PROMPT.md`, `FINISH-PLAN.md`, `reports/README.md` | what the next session reads | 9 |
| `test/t.sh`, `test/test_*.sh`, `test/test_*.mjs`, `test/test_keyscan.py`, `test/run-all.sh`, `test/fresh-clone.sh` | the checks | 1 to 10 |

Not committed (decided): `snaps.mjs`, `snaps2.mjs` (folded into `box.mjs`), `dbfetch/query.sql` (byte-identical to the SQL block of `scripts/fetch-run.sh` on `04044320`; `run.sh` extracts it), `dbfetch/priv.sql` (a one-off privilege probe), `rung1/rung1-patch.diff` (merged rig history).

---

### Task 1: Shared paths and the session trailer

**Files:**
- Create: `$C/lib.sh`, `$C/mq-driver.sh` (from `/root/gc-tools/mq-driver.sh`), `$C/test/t.sh`, `$C/test/test_lib.sh`
- Modify: `$C/merge.sh:12-16,41,50`, `$C/mergeq.sh:3,8,27-28`, `$C/scribe.sh:4,20-22`, `$C/handoff.sh:4,11-14,31-33` (paths and trailer only; its copy list grows in Task 8)

**Interfaces:**
- Produces (`lib.sh`, sourced as `. "$(dirname "$0")/lib.sh"`): `GC_TOOLS` (the folder holding `lib.sh`, absolute); sources `$GC_TOOLS/env.sh` if present; `GC_NIGHT=${GC_NIGHT:-$HOME/gc-night}`; `GC_WT=${GC_WT:-$HOME/gc-wt}`; `GC_COAUTHOR=${GC_COAUTHOR:-Claude Opus 5.5 <noreply@anthropic.com>}`; `gc_need_repo` (exit 2 with `GC_REPO unset (install.sh --repo)` unless `GC_REPO` is a git dir); `gc_trailer` (prints `\n\nCo-Authored-By: $GC_COAUTHOR\nClaude-Session: $GC_SESSION`, or prints `GC_SESSION unset or not a claude.ai session URL` to stderr and returns 2 unless `GC_SESSION` matches `^https://claude\.ai/code/session_[A-Za-z0-9]{8,}$`).
- Produces (`test/t.sh`): `ok NAME`, `fail NAME MSG` (sets exit 1 at end), `tmp` (prints a fresh `mktemp -d`), `fake_repo DIR` (bare origin at `DIR/origin.git` with branch `v1.0/integration`, clone at `DIR/repo`), `finish`; `t.sh` exports `GIT_AUTHOR_NAME`, `GIT_AUTHOR_EMAIL`, `GIT_COMMITTER_NAME`, `GIT_COMMITTER_EMAIL` (test values) so commits work in a bare box (prints `N passed, M failed`, exits 1 on any failure).
- `merge.sh`, `scribe.sh`, `handoff.sh` call `TRAILER=$(gc_trailer) || exit 2` and `gc_need_repo` as their first actions, before `flock` or `git`. `mergeq.sh` runs `"$GC_TOOLS/merge.sh"`; `mq-driver.sh` runs `"$GC_TOOLS/mergeq.sh"`.

- [ ] **Step 1: Write the failing test** `test/test_lib.sh`
```bash
. "$(dirname "$0")/t.sh"; C=$(cd "$(dirname "$0")/.." && pwd)
# t_no_root_paths
if grep -nE '/root/|/home/user|session_01V2' "$C"/*.sh "$C"/*.mjs "$C"/*.js "$C"/*.cjs "$C"/*.py "$C"/dbfetch/*.sh 2>/dev/null; then fail t_no_root_paths "literal path or old session"; else ok t_no_root_paths; fi
# t_trailer_needs_session
( unset GC_SESSION; . "$C/lib.sh"; gc_trailer ) 2>/dev/null && fail t_trailer_needs_session "returned 0" || ok t_trailer_needs_session
( GC_SESSION=https://example.com/x; . "$C/lib.sh"; gc_trailer ) 2>/dev/null && fail t_trailer_rejects_other_url "returned 0" || ok t_trailer_rejects_other_url
# t_trailer_text
out=$(GC_SESSION=https://claude.ai/code/session_TESTabcd123 bash -c ". '$C/lib.sh'; gc_trailer")
[ "$out" = $'\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>\nClaude-Session: https://claude.ai/code/session_TESTabcd123' ] && ok t_trailer_text || fail t_trailer_text "$out"
# t_merge_refuses_without_session: exit 2, no worktree, no ledger line
d=$(tmp); fake_repo "$d"; mkdir -p "$d/night/lanes"
( unset GC_SESSION; GC_REPO=$d/repo GC_NIGHT=$d/night bash "$C/merge.sh" v1.0/x "m" ) >/dev/null 2>&1; rc=$?
[ $rc = 2 ] && [ ! -e "$d/night/merge-wt" ] && [ ! -s "$d/night/lanes/integrator.log" ] && ok t_merge_refuses_without_session || fail t_merge_refuses_without_session "rc=$rc"
# t_scribe_refuses_without_session
( unset GC_SESSION; GC_REPO=$d/repo GC_NIGHT=$d/night bash "$C/scribe.sh" ) >/dev/null 2>&1; [ $? = 2 ] && ok t_scribe_refuses_without_session || fail t_scribe_refuses_without_session "not 2"
# t_mergeq_runs_first_request: a copied tool folder with a stub merge.sh; result lands in GC_NIGHT/mrq
cp -r "$C" "$d/tools"; printf '#!/bin/bash\necho "stub merged $1"; exit 0\n' > "$d/tools/merge.sh"; chmod +x "$d/tools/merge.sh"
mkdir -p "$d/night/mrq/done"; echo '{"branch":"v1.0/x","message":"m","first":true}' > "$d/night/mrq/a1.req"
echo 'not json' > "$d/night/mrq/bad.req"
GC_REPO=$d/repo GC_NIGHT=$d/night timeout 60 bash "$d/tools/mergeq.sh" 1
for i in $(seq 30); do [ -f "$d/night/mrq/a1.res" ] && break; sleep 1; done
python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));assert d["exit"]==0 and "stub merged v1.0/x" in d["output"]' "$d/night/mrq/a1.res" && [ -f "$d/night/mrq/done/a1.req" ] && ok t_mergeq_runs_first_request || fail t_mergeq_runs_first_request "no result"
[ -f "$d/night/mrq/done/bad.req" ] && ok t_mergeq_bad_request_to_done || fail t_mergeq_bad_request_to_done "bad request left in queue"
finish
```
- [ ] **Step 2: Commit the test alone as the red commit (no box)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "test(conductor): paths and trailer checks [red]"` (trailer lines as in Global Constraints). `test/red-green.sh` later runs `bash $C/test/test_lib.sh` at this commit.
Expected at the red run: FAIL, `t_no_root_paths` lists `merge.sh:12`, `scribe.sh:4`, `gc-lane.js:19` and others; `t_trailer_needs_session` fails (no `lib.sh`).
- [ ] **Step 3: Implement `lib.sh`, then switch `merge.sh`, `mergeq.sh`, `scribe.sh`, `mq-driver.sh` to it**
In all four scripts replace `REPO=`/`NIGHT=`/`Q=` with `$GC_REPO`/`$GC_NIGHT`/`$GC_NIGHT/mrq`, the two `node /root/gc-tools/box.mjs` calls with `node "$GC_TOOLS/box.mjs"`, every literal trailer with `$TRAILER`, and in `handoff.sh` the `/root/gc-tools/` and `/home/user/gc-scripts/` sources with `$GC_TOOLS/` and `${GC_LANE:-$GC_TOOLS/gc-lane.js}`. In `mergeq.sh` pass `$GC_TOOLS` and `$GC_NIGHT` into the `setsid` child as positional arguments (it does not source `lib.sh`). `box.mjs` still fails `t_no_root_paths` until Task 2 and `gc-lane.js` until Task 4.
- [ ] **Step 4: Commit the implementation (no box; the pass is proven in the slice box, Task 10)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "feat(conductor): lib.sh paths and GC_SESSION trailer"`. Slice-box command for this task: `bash $C/test/test_lib.sh`.
Expected in the slice box at this commit: every case `ok` except `t_no_root_paths` (naming only `box.mjs` and `gc-lane.js`); final line `7 passed, 1 failed`.
- [ ] **Step 5: Push the branch**
```bash
git -C "$WT" push -q origin HEAD:v1.0/conductor-handover
```

### Task 2: box.mjs help, size, snapshots and spend

**Files:**
- Modify: `$C/box.mjs:1-19` (header, lazy SDK import), `:25-28` (`--vcpus`), `:82` (`GC_REPO`), `:102-114` (commands)
- Create: `$C/spend.mjs`, `$C/package.json`, `$C/package-lock.json` (from `/root/gc-tools/`), `$C/.gitignore` (`node_modules/`), `$C/test/test_box.mjs`

**Interfaces:**
- Produces (`spend.mjs`): `export function sessionMinutes(sessions: Array<{vcpus:number, startedAt?:number, createdAt:number, stoppedAt?:number}>, nowMs: number) -> {minutes:number, vcpuMinutes:number}` (end is `stoppedAt ?? nowMs`, start is `startedAt ?? createdAt`, both rounded to 0.1); `export function ledgerLine(t: {boxes:number, minutes:number, vcpuMinutes:number, since:string, modelUsd?:number}, now: Date) -> string` = `HH:MMZ | conductor spend | <boxes> boxes since <since>: <minutes> box minutes, <vcpuMinutes> vCPU minutes` plus `; model about <modelUsd> USD billed` when given.
- Produces (`box.mjs`): `--help`/`-h`/no command prints usage and exits 0 (help) or 2 (no command) without loading `@vercel/sandbox` (`await import('@vercel/sandbox')` only inside commands); `run ... [--vcpus N]` (default 8, unchanged gate size); `snaps` prints one line per snapshot `id status created expires size` (iterating every page with `for await`), marks the id in `$GC_TOOLS/build-snapshot.id` with `[build]` and the one in `$GC_REPO/scripts/pad/image.id` with `[pad]`, then `running <n> of <total> boxes`; `spend --since ISO [--model-usd N] [--ledger]` iterates every sandbox with `createdAt >= since` (all pages), sums `sessionMinutes` over `Sandbox.get({name}).listSessions()` (all pages), prints one line per box and the `ledgerLine`; `--ledger` appends that line to `$GC_NIGHT/lanes/integrator.log` (`GC_NIGHT` default `$HOME/gc-night`). `list` iterates all pages too.
- `package.json`: `{"name":"gc-conductor","private":true,"type":"module","dependencies":{"@vercel/sandbox":"3.9.1"}}`, no npm default fields; regenerate the lock to match.

- [ ] **Step 1: Write the failing test** `test/test_box.mjs` (node, no network): copies `box.mjs` and `spend.mjs` to a temp dir; first with no `node_modules`, then with a stub `node_modules/@vercel/sandbox/index.mjs` whose `Sandbox.list` yields two pages and `Snapshot.list` one.
```js
// help_without_sdk
let r = spawnSync('node', [box, '--help']); assert.equal(r.status, 0); assert.match(r.stdout + r.stderr, /run --repo DIR --ref REF --script FILE/);
// sessionMinutes: running session counts to now
assert.deepEqual(sessionMinutes([{vcpus:2,startedAt:0,createdAt:0,stoppedAt:600000},{vcpus:4,createdAt:0}], 300000), {minutes:15, vcpuMinutes:40});
// ledgerLine
assert.equal(ledgerLine({boxes:3,minutes:12.5,vcpuMinutes:50,since:'2026-10-30T00:00:00Z',modelUsd:27}, new Date('2026-11-01T23:10:00Z')),
  '23:10Z | conductor spend | 3 boxes since 2026-10-30T00:00:00Z: 12.5 box minutes, 50 vCPU minutes; model about 27 USD billed');
// spend_pages_and_since: stub has a@page1 (2 vCPU, 10 min), b@page2 (4 vCPU, 5 min), old@page2 created before --since
r = spawnSync('node', [box, 'spend', '--since', '2026-10-30T00:00:00Z'], {env: {...process.env, GC_NIGHT: night}});
assert.equal(r.status, 0); assert.match(r.stdout, /2 boxes since 2026-10-30T00:00:00Z: 15 box minutes, 40 vCPU minutes/);
// spend_ledger appends exactly one line
spawnSync('node', [box, 'spend', '--since', '2026-10-30T00:00:00Z', '--ledger'], {env: {...process.env, GC_NIGHT: night}});
assert.equal(fs.readFileSync(night + '/lanes/integrator.log', 'utf8').trim().split('\n').length, 1);
// snaps_marks_ids: stub snapshot ids equal build-snapshot.id and a fake GC_REPO/scripts/pad/image.id
assert.match(snapsOut, /snap_build .* \[build\]/); assert.match(snapsOut, /snap_pad .* \[pad\]/); assert.match(snapsOut, /running 1 of 3 boxes/);
```
- [ ] **Step 2: Commit the test alone as the red commit (no box)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "test(conductor): box.mjs help, snaps, spend [red]"` (trailer lines as in Global Constraints). `test/red-green.sh` later runs `node $C/test/test_box.mjs` at this commit.
Expected at the red run: FAIL at `help_without_sdk` (`Cannot find package '@vercel/sandbox'`).
- [ ] **Step 3: Implement `spend.mjs` and the `box.mjs` changes; delete nothing in the repo (`snaps*.mjs` were never committed)**
- [ ] **Step 4: Commit the implementation (no box; the pass is proven in the slice box, Task 10)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "feat(conductor): box.mjs help, vcpus, snaps, spend"`. Slice-box command for this task: `node $C/test/test_box.mjs && bash $C/test/test_lib.sh`.
Expected: `test_box: 6 passed`; `test_lib` now names only `gc-lane.js` in `t_no_root_paths`.
- [ ] **Step 5: Push the branch** (as Task 1 Step 5)

### Task 3: install.sh and the sandbox proxy patch

**Files:**
- Create: `$C/install.sh`, `$C/patch-sandbox.sh`, `$C/proxy-preload.cjs`, `$C/test/test_install.sh`

**Interfaces:**
- Produces: `patch-sandbox.sh NODE_MODULES` rewrites line `import { Agent } from "undici";` in `NODE_MODULES/@vercel/sandbox/dist/api-client/base-client.js` to `import { EnvHttpProxyAgent as Agent } from "undici";`; exit 0 with `patched` or `already patched`; exit 5 with `unknown @vercel/sandbox layout: <file>` when neither line is present or the file is missing. This is the patch on the old machine (ESM build, 3.9.1); the CJS build (`base-client.cjs:9`, used by `scripts/pad/sbx.mjs` through `createRequire`) is handled by the preload.
- Produces: `proxy-preload.cjs`: `const u = require(require('path').join(__dirname, 'node_modules', 'undici')); u.Agent = u.EnvHttpProxyAgent;` (the content of `/root/gc-night/rung1/proxy-preload.cjs` with the path made relative). Callers set `NODE_OPTIONS="--use-openssl-ca --no-warnings --require $GC_TOOLS/proxy-preload.cjs"`.
- Produces: `install.sh --dest DIR [--repo DIR] [--night DIR] [--wt DIR] [--session URL] [--coauthor TEXT] [--lane-dir DIR] [--skip-npm]`: copies `$C` (minus `test/`, `ledger/`, `reports/`, `node_modules/`) to `DIR`, runs `npm ci --silent` there unless `--skip-npm`, runs `patch-sandbox.sh DIR/node_modules`, writes `DIR/env.sh` (mode 0644) with `export GC_REPO=... GC_NIGHT=... GC_WT=... GC_SESSION=... GC_COAUTHOR=...` for the options given and nothing else, copies `gc-lane.js` into `--lane-dir` when given. Never deletes a file in `DIR` other than `node_modules/` (so a `build-snapshot.id` already there survives). Exit 2 usage, 5 patch refused. (`--night` restore is Task 8.)

- [ ] **Step 1: Write the failing test** `test/test_install.sh`
```bash
. "$(dirname "$0")/t.sh"; C=$(cd "$(dirname "$0")/.." && pwd); d=$(tmp)
fake_sdk() { mkdir -p "$1/@vercel/sandbox/dist/api-client" "$1/undici"; printf '%s\n' 'import { APIError } from "./api-error.js";' "$2" > "$1/@vercel/sandbox/dist/api-client/base-client.js"
  echo 'class A{}; class E{}; module.exports={Agent:A, EnvHttpProxyAgent:E};' > "$1/undici/index.js"; }
# t_patch_rewrites_import
fake_sdk "$d/nm" 'import { Agent } from "undici";'; bash "$C/patch-sandbox.sh" "$d/nm" | grep -qx patched && grep -qxF 'import { EnvHttpProxyAgent as Agent } from "undici";' "$d/nm/@vercel/sandbox/dist/api-client/base-client.js" && ok t_patch_rewrites_import || fail t_patch_rewrites_import ""
# t_patch_idempotent
bash "$C/patch-sandbox.sh" "$d/nm" | grep -qx 'already patched' && ok t_patch_idempotent || fail t_patch_idempotent ""
# t_patch_unknown_layout_exit5
fake_sdk "$d/nm2" 'import { Agent as X } from "undici-next";'; bash "$C/patch-sandbox.sh" "$d/nm2" 2>"$d/err"; [ $? = 5 ] && grep -q 'unknown @vercel/sandbox layout' "$d/err" && ok t_patch_unknown_layout_exit5 || fail t_patch_unknown_layout_exit5 ""
# t_preload_swaps_agent
mkdir -p "$d/pre"; cp "$C/proxy-preload.cjs" "$d/pre/"; fake_sdk "$d/pre/node_modules" x
node --require "$d/pre/proxy-preload.cjs" -e "const u=require('$d/pre/node_modules/undici'); process.exit(u.Agent===u.EnvHttpProxyAgent?0:1)" && ok t_preload_swaps_agent || fail t_preload_swaps_agent ""
# t_install_env_has_no_secret: env.sh holds the given paths, never a secret value
fake_sdk "$C/node_modules" 'import { Agent } from "undici";' 2>/dev/null || true
CLAUDE_API_KEY=sk-test-0123456789abcdef bash "$C/install.sh" --dest "$d/tools" --repo "$d" --night "$d/night" --session https://claude.ai/code/session_TESTabcd123 --skip-npm
grep -q "GC_SESSION=https://claude.ai/code/session_TESTabcd123" "$d/tools/env.sh" && ! grep -q 0123456789abcdef "$d/tools/env.sh" && [ ! -e "$d/tools/test" ] && ok t_install_env_has_no_secret || fail t_install_env_has_no_secret ""
# t_installed_help_runs
node "$d/tools/box.mjs" --help >/dev/null && ok t_installed_help_runs || fail t_installed_help_runs ""
finish
```
(The test runs on a clean clone in the box, so `$C/node_modules` is the fake one it makes.)
- [ ] **Step 2: Commit the test alone as the red commit (no box)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "test(conductor): install and proxy patch [red]"` (trailer lines as in Global Constraints). `test/red-green.sh` later runs `bash $C/test/test_install.sh` at this commit.
Expected at the red run: FAIL, `patch-sandbox.sh: No such file`.
- [ ] **Step 3: Implement `patch-sandbox.sh`, `proxy-preload.cjs`, `install.sh`**
`install.sh` with `--skip-npm` keeps a `node_modules` already in the source folder by copying it; without it, `npm ci` in `DIR`.
- [ ] **Step 4: Commit the implementation (no box; the pass is proven in the slice box, Task 10)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "feat(conductor): install.sh, sandbox proxy patch, relative preload"`. Slice-box command for this task: `bash $C/test/test_install.sh`.
Expected: `6 passed, 0 failed`.
- [ ] **Step 5: Push the branch**

### Task 4: gc-lane.js parameters

**Files:**
- Modify: `$C/gc-lane.js:9-37` (`A`, `COMMON`), `:120`, `:132`, `:137`, `:148-150` (every `/root/gc-night`, `/root/gc-tools`, `/root/gc-wt`, `/home/user/goal-cup-proxy`)
- Create: `$C/test/test_gc_lane.mjs`

**Interfaces:**
- Consumes: none.
- Produces: required `args.session` (same pattern as `gc_trailer`) and `args.paths = {repo, tools, night, wt}` (absolute strings); optional `args.coauthor` (default `Claude Opus 5.5 <noreply@anthropic.com>`). At the top: `throw new Error('gc-lane: args.<name> is required')` for a missing or malformed one. Every path in prompts comes from `args.paths`; the commit trailer lines in `COMMON` come from `args.coauthor` and `args.session`. Start args shape: `{"lane":..., "prds":[...], "session":"https://claude.ai/code/session_...", "paths":{"repo":"...","tools":"...","night":"...","wt":"..."}}`.

- [ ] **Step 1: Write the failing test** `test/test_gc_lane.mjs`: reads `gc-lane.js`, replaces `export const meta` with `const meta`, runs the body as `new AsyncFunction('args', 'agent', 'parallel', 'log', body)`.
```js
const P = {repo:'/r', tools:'/t', night:'/n', wt:'/w'}, S = 'https://claude.ai/code/session_TESTabcd123';
// missing_session_throws
await assert.rejects(run({lane:'x', prds:[], paths:P}), /args\.session is required/);
// missing_paths_throws
await assert.rejects(run({lane:'x', prds:[], session:S}), /args\.paths is required/);
// empty_lane_returns
assert.deepEqual(await run({lane:'x', prds:[], session:S, paths:P}), []);
// prompts_use_params: the planner prompt for one PRD
const seen = []; await run({lane:'x', session:S, paths:P, prds:[{id:'PRD-99', slug:'prd-99-x', file:'PRD-99.md', core:false}]},
  async (p) => { seen.push(p); return {done:true, notes:'', tasks:[]}; });
assert.match(seen[0], /Claude-Session: https:\/\/claude\.ai\/code\/session_TESTabcd123/);
assert.match(seen[0], /node \/t\/box\.mjs run/); assert.match(seen[0], /\/n\/lanes\/x\.log/);
assert.doesNotMatch(seen.join('\n'), /\/root\/|\/home\/user|session_01V2/);
```
- [ ] **Step 2: Commit the test alone as the red commit (no box)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "test(conductor): gc-lane parameters [red]"` (trailer lines as in Global Constraints). `test/red-green.sh` later runs `node $C/test/test_gc_lane.mjs` at this commit.
Expected at the red run: FAIL at `missing_session_throws` (resolves instead of rejecting).
- [ ] **Step 3: Implement the `args.session`, `args.paths`, `args.coauthor` checks and substitutions in `gc-lane.js`**
- [ ] **Step 4: Commit the implementation (no box; the pass is proven in the slice box, Task 10)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "feat(conductor): gc-lane takes session and paths as args"`. Slice-box command for this task: `node $C/test/test_gc_lane.mjs && bash $C/test/test_lib.sh`.
Expected: `test_gc_lane: 4 passed`; `test_lib: 8 passed, 0 failed`.
- [ ] **Step 5: Push the branch**

### Task 5: Pad drivers

**Files:**
- Create: `$C/padrun.sh` (from `/root/gc-night/gw3/padrun.sh`), `$C/padbox.sh` (from `/root/gc-night/rung1/padbox.sh`), `$C/test/test_pad_drivers.sh`

**Interfaces:**
- Consumes: `lib.sh` (Task 1), `proxy-preload.cjs` (Task 3), `keyscan.py --scrub` (Task 7; until then `padbox.sh` checks `[ -f "$GC_TOOLS/keyscan.py" ]` and fails the scrub step loudly if absent).
- Produces: `padrun.sh SETTINGS OUTDIR` runs `python3.11 scripts/pad/pad SETTINGS` in `$GC_PAD_WT` (required, exit 2 if unset) with `PAD_NODE_MODULES=$GC_TOOLS/node_modules`, `NODE_OPTIONS="--use-openssl-ca --no-warnings --require $GC_TOOLS/proxy-preload.cjs"`, `GH_TOKEN` and `PYTHONPATH` unset; writes `OUTDIR/logs/<tag>.log` ending in `PAD_EXIT=<n>` and `== end <time>`; prints the log path. `GC_PYTHON` overrides `python3.11` (for the test).
- Produces: `padbox.sh SETTINGS KEYNAME ENVVAR OUTDIR` (the inside-box diagnosis driver): `SBX=${GC_SBX:-node $GC_PAD_WT/scripts/pad/sbx.mjs}`; every sbx call runs under `env -u GH_TOKEN -u GOALCUP_TEST_APPROVAL_KEY -u GOALCUP_TEST_MEMBER_ID -u DATABASE_URL -u CLOUDFLARE_API_TOKEN -u CLOUDFLARE_ACCOUNT_ID`; order: `create --name pad-dx-<tag>-<HHMMSS> --minutes 45 --snapshot <image.id> --vcpus 4`, `push-dir` pad and connectors, `put` settings, `put` `goalcup` and `goalcup-verify` from `$GC_BUILD` (required), `put-env ~/.pad-keys/KEYNAME --env ENVVAR`, `exec` inbox, `get /tmp/pad-out.tar`, `keyscan.py --scrub OUTDIR/pad-out/<tag>`; `rm` runs from an EXIT trap whatever fails.

- [ ] **Step 1: Write the failing test** `test/test_pad_drivers.sh` with a fake `$GC_PAD_WT` (stub `scripts/pad/pad` printing `GH=${GH_TOKEN-unset} PNM=$PAD_NODE_MODULES NO=$NODE_OPTIONS`) and a stub sbx that appends `"$1 $(env | cut -d= -f1 | grep -E '^(GH_TOKEN|DATABASE_URL|CLAUDE_API_KEY)$' | tr '\n' ,) $*"` to `$d/calls`, exits 1 on `exec`.
```bash
# t_padrun_env
GH_TOKEN=ghp_testtesttest GC_PAD_WT=$d/wt GC_PYTHON=bash bash "$C/padrun.sh" "$d/s1.toml" "$d/out"
grep -q 'GH=unset' "$d/out/logs/s1.log" && grep -q "PNM=$C/node_modules" "$d/out/logs/s1.log" && grep -q -- "--require $C/proxy-preload.cjs" "$d/out/logs/s1.log" && grep -q 'PAD_EXIT=0' "$d/out/logs/s1.log" && ok t_padrun_env || fail t_padrun_env ""
# t_padbox_cleans_up_on_failure: exec fails, rm still runs, and runs last
CLAUDE_API_KEY=sk-test-0123456789abcdef DATABASE_URL=postgresql://u:pw0123456789abc@h/db GC_SBX="bash $d/sbx" GC_PAD_WT=$d/wt GC_BUILD=$d/build bash "$C/padbox.sh" "$d/s1.toml" claude CLAUDE_API_KEY "$d/out"
[ "$(tail -1 "$d/calls" | cut -d' ' -f1)" = rm ] && ok t_padbox_cleans_up_on_failure || fail t_padbox_cleans_up_on_failure ""
# t_padbox_no_value_in_argv_and_db_never_in_env
! grep -q 0123456789abcdef "$d/calls" && ! grep -q 'DATABASE_URL' "$d/calls" && ok t_padbox_no_value_in_argv || fail t_padbox_no_value_in_argv ""
grep -q '^put-env CLAUDE_API_KEY,' "$d/calls" && [ "$(grep -c 'CLAUDE_API_KEY,' "$d/calls")" -ge 1 ] && ok t_padbox_key_reaches_put_env || fail t_padbox_key_reaches_put_env ""
finish
```
- [ ] **Step 2: Commit the test alone as the red commit (no box)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "test(conductor): pad drivers [red]"` (trailer lines as in Global Constraints). `test/red-green.sh` later runs `bash $C/test/test_pad_drivers.sh` at this commit.
Expected at the red run: FAIL, `padrun.sh: No such file`.
- [ ] **Step 3: Implement `padrun.sh` and `padbox.sh`**
- [ ] **Step 4: Commit the implementation (no box; the pass is proven in the slice box, Task 10)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "feat(conductor): portable pad drivers"`. Slice-box command for this task: `bash $C/test/test_pad_drivers.sh && bash $C/test/test_lib.sh`.
Expected: `4 passed, 0 failed`; `test_lib` still all passed.
- [ ] **Step 5: Push the branch**

### Task 6: Index box scripts

**Files:**
- Create: `$C/dbfetch/idx.sh`, `$C/dbfetch/install.sh` (verbatim from `/root/gc-night/dbfetch/`), `$C/dbfetch/run.sh`, `$C/test/test_dbfetch.sh`

**Interfaces:**
- Consumes: `lib.sh`; `keyscan.py` (Task 7, same fallback as Task 5).
- Produces: `dbfetch/run.sh IDS_FILE OUTDIR`: extracts the block between `<<'SQL'` and `^SQL$` of `$GC_PAD_WT/scripts/fetch-run.sh` into a temp `query.sql`, exit 4 `fetch-run.sh SQL block not found or not read-only` unless it starts `BEGIN READ ONLY;` and ends `ROLLBACK;`; then with `SBX` as in Task 5: `create --name pad-dbf-conductor-<HHMMSS> --minutes 10 --vcpus 1`, `put` `ids.txt`, `query.sql`, `install.sh`, `idx.sh`, `exec install.sh --sudo`, `put-env ~/.dburl --env DATABASE_URL` (the only call with `DATABASE_URL` in its env), `exec idx.sh`, `get /tmp/idx.tar`, untar to `OUTDIR/idx/`, `keyscan.py OUTDIR/idx` (a hit deletes `OUTDIR/idx` and exits 1); `rm` from an EXIT trap.

- [ ] **Step 1: Write the failing test** `test/test_dbfetch.sh` (fake `$GC_PAD_WT/scripts/fetch-run.sh` holding the SQL heredoc; same stub sbx as Task 5, here `get` writes a tar with `idx/r1.json`).
```bash
# t_dbfetch_order
DATABASE_URL=postgresql://u:pw0123456789abc@h/db GC_SBX="bash $d/sbx" GC_PAD_WT=$d/wt bash "$C/dbfetch/run.sh" "$d/ids.txt" "$d/out"
[ "$(cut -d' ' -f1 "$d/calls" | tr '\n' ' ')" = "create put put put put exec put-env exec get rm " ] && [ -f "$d/out/idx/r1.json" ] && ok t_dbfetch_order || fail t_dbfetch_order "$(cut -d' ' -f1 "$d/calls" | tr '\n' ' ')"
# t_dburl_only_in_put_env
[ "$(grep -c 'DATABASE_URL,' "$d/calls")" = 1 ] && grep -q '^put-env DATABASE_URL,' "$d/calls" && ! grep -q pw0123456789abc "$d/calls" && ok t_dburl_only_in_put_env || fail t_dburl_only_in_put_env ""
# t_refuses_write_sql
sed -i 's/BEGIN READ ONLY;/BEGIN;/' "$d/wt/scripts/fetch-run.sh"; : > "$d/calls"
GC_SBX="bash $d/sbx" GC_PAD_WT=$d/wt bash "$C/dbfetch/run.sh" "$d/ids.txt" "$d/out2"; [ $? = 4 ] && [ ! -s "$d/calls" ] && ok t_refuses_write_sql || fail t_refuses_write_sql ""
finish
```
- [ ] **Step 2: Commit the test alone as the red commit (no box)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "test(conductor): index box driver [red]"` (trailer lines as in Global Constraints). `test/red-green.sh` later runs `bash $C/test/test_dbfetch.sh` at this commit.
Expected at the red run: FAIL, `dbfetch/run.sh: No such file`.
- [ ] **Step 3: Implement `dbfetch/run.sh`; copy `idx.sh` and `install.sh` unchanged**
- [ ] **Step 4: Commit the implementation (no box; the pass is proven in the slice box, Task 10)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "feat(conductor): index box scripts and driver"`. Slice-box command for this task: `bash $C/test/test_dbfetch.sh`.
Expected: `3 passed, 0 failed`.
- [ ] **Step 5: Push the branch**

### Task 7: keyscan.py (scan and scrub)

**Files:**
- Create: `$C/keyscan.py`, `$C/test/test_keyscan.py`

**Interfaces:**
- Produces: `NAMES` (tuple, the 14 names of Global Constraints, in that order); `N = 12`; `values(env) -> list[tuple[str, str]]` (for `DATABASE_URL`, the URL's password and hostname instead of the whole URL, raw and `urllib.parse.unquote`d; for the rest the value; anything under 12 characters dropped); `scan(paths, vals) -> list[tuple[str, str]]` (name, file) walking with `os.walk`, every regular file outside `.git/`, dotfiles and `.py` included; `scrub(paths, vals) -> int` (replaces every run of 12 or more characters of a value with `[KEY-FRAGMENT-REDACTED]`, the algorithm of `/root/gc-night/full4/scrub2.py`). CLI: `python3 keyscan.py [--scrub] PATH...`; scan prints `HIT <NAME> <file>` per hit and `keyscan: <h> hit(s), <k> keys checked, unset: <names or none>`; exit 0 clean, 1 hits, 3 when `k == 0` (`keyscan: no secret values in env; nothing proven`), 2 usage. Scrub prints `scrubbed <n> fragment(s) in <file>`, exit 0 (3 when `k == 0`). Never prints a value. Python 3.9 compatible.

- [ ] **Step 1: Write the failing test** `test/test_keyscan.py` (unittest, runs the CLI with a controlled env)
```python
K = "sk-test-" + "0123456789abcdef"
def test_dotfile_and_py_scanned(self):   # .env.local and a .py file both hit
    w(d, ".env.local", "X=" + K); w(d, "a.py", "k = '%s'" % K[3:19])
    r = cli([d], {"CLAUDE_API_KEY": K}); self.assertEqual(r.returncode, 1)
    self.assertEqual(sorted(l.split()[2].rsplit("/", 1)[1] for l in r.stdout.splitlines() if l.startswith("HIT")), [".env.local", "a.py"])
def test_no_values_exit_3(self):
    r = cli([d], {}); self.assertEqual(r.returncode, 3); self.assertIn("nothing proven", r.stdout)
def test_clean_exit_0_names_unset(self):
    r = cli([d], {"OPENAI_API_KEY": K}); self.assertEqual(r.returncode, 0); self.assertIn("unset: VERCEL_TOKEN", r.stdout)
def test_database_url_password_not_scheme(self):  # docs mentioning the scheme do not hit; the password does
    u = "postgresql://reader:pw0123456789abc@db.example.internal:5432/x"
    w(d, "doc.md", "connect with postgresql://reader@host"); self.assertEqual(cli([d], {"DATABASE_URL": u}).returncode, 0)
    w(d, "leak.txt", "pw0123456789abc"); self.assertEqual(cli([d], {"DATABASE_URL": u}).returncode, 1)
def test_scrub_then_clean(self):
    w(d, "log.txt", "a " + K + " b"); cli(["--scrub", d], {"CLAUDE_API_KEY": K})
    self.assertIn("[KEY-FRAGMENT-REDACTED]", open(d + "/log.txt").read()); self.assertEqual(cli([d], {"CLAUDE_API_KEY": K}).returncode, 0)
def test_never_prints_value(self):
    w(d, "x", K); r = cli([d], {"CLAUDE_API_KEY": K}); self.assertNotIn(K[:12], r.stdout + r.stderr)
def test_git_dir_skipped(self):
    w(d, ".git/objects/x", K); self.assertEqual(cli([d], {"CLAUDE_API_KEY": K}).returncode, 0)
```
- [ ] **Step 2: Commit the test alone as the red commit (no box)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "test(conductor): keyscan [red]"` (trailer lines as in Global Constraints). `test/red-green.sh` later runs `python3 -m unittest discover -s $C/test -p 'test_keyscan.py'` at this commit.
Expected at the red run: FAIL, `can't open file .../keyscan.py`.
- [ ] **Step 3: Implement `keyscan.py`**
- [ ] **Step 4: Commit the implementation (no box; the pass is proven in the slice box, Task 10)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "feat(conductor): keyscan.py scan and scrub"`. Slice-box command for this task: `python3 -m unittest discover -s $C/test -p 'test_keyscan.py' && bash $C/test/test_pad_drivers.sh && bash $C/test/test_dbfetch.sh`.
Expected: `Ran 7 tests ... OK`; pad and dbfetch tests still pass.
- [ ] **Step 5: Push the branch**

### Task 8: handoff.sh bundle and the restore

**Files:**
- Modify: `$C/handoff.sh:11-12` (the copy list, as changed in Task 1), `$C/install.sh` (add `--night`)
- Create: `$C/test/test_handoff.sh`

**Interfaces:**
- Consumes: `lib.sh` (Task 1), `keyscan.py` (Task 7), `install.sh` (Task 3).
- Produces (`handoff.sh`, after `TRAILER=$(gc_trailer) || exit 2` and `gc_need_repo`): in its worktree copies into `$C`: from `$GC_TOOLS` the files `box.mjs spend.mjs merge.sh mergeq.sh mq-driver.sh scribe.sh handoff.sh lib.sh install.sh patch-sandbox.sh proxy-preload.cjs padrun.sh padbox.sh keyscan.py package.json package-lock.json .gitignore build-snapshot.id handoff-head.md` and `dbfetch/{idx.sh,install.sh,run.sh}`; `gc-lane.js` from `${GC_LANE:-$GC_TOOLS/gc-lane.js}`; `ledger/<lane>.log` and `ledger/<lane>.log.scribed` from `$GC_NIGHT/lanes/` (every one present; `.pending` never); `reports/<folder>.txt` from each `$GC_NIGHT/*/REPORT.txt`; `reports/review-latest.md` from `$GC_NIGHT/reviews/LATEST.md`; `reports/next-pad-verify.md` from `$GC_NIGHT/backlog/`; `FINISH-PLAN.md` from `$GC_NIGHT/finish/`; appends `| <folder> | not summarised yet |` to `reports/README.md` for any report without a row. Then `python3 keyscan.py "$C"`: exit 1 or 3 stops with exit 6 (`handoff: keyscan refused`), nothing committed or pushed; `GC_KEYSCAN_ALLOW_EMPTY=1` lets exit 3 pass (tests only). Then writes `HANDOFF.md` as today and commits with `$TRAILER`, pushes `HEAD:v1.0/integration`.
- Produces (`install.sh --night DIR`): creates `DIR/{lanes,mrq/done,gate-logs,status,reviews,builds,finish}` and an empty `DIR/integration.lock`; for each `ledger/*.log` copies it and its `.scribed` (if any) into `DIR/lanes/` only when `DIR/lanes/<name>` does not exist (never overwrites, prints `kept existing <name>`).

- [ ] **Step 1: Write the failing test** `test/test_handoff.sh` (fake origin and repo from `fake_repo`; fake night with `lanes/integrator.log` (3 lines) + `.scribed` (`2`), `lanes/pad-model.log` without `.scribed`, `lanes/x.log.pending`, `gw3/REPORT.txt`, `gw3/pad-out/secret.txt`, `gw3/.env`, `finish/FINISH-PLAN.md`; tools from `install.sh --skip-npm`)
```bash
# t_handoff_bundle
GC_SESSION=$S GC_REPO=$d/repo GC_NIGHT=$d/night GC_KEYSCAN_ALLOW_EMPTY=1 bash "$d/tools/handoff.sh"
git -C "$d/repo" fetch -q origin; T=$(git -C "$d/repo" ls-tree -r --name-only origin/v1.0/integration -- "$C")
for f in ledger/integrator.log ledger/integrator.log.scribed ledger/pad-model.log reports/gw3.txt FINISH-PLAN.md mq-driver.sh keyscan.py package-lock.json; do echo "$T" | grep -qx "$C/$f" || fail t_handoff_bundle "missing $f"; done
echo "$T" | grep -qE 'pad-out|\.env|\.pending|node_modules' && fail t_handoff_bundle "copied a forbidden file" || ok t_handoff_bundle
# t_handoff_trailer
git -C "$d/repo" log -1 --format=%B origin/v1.0/integration | tail -1 | grep -qx "Claude-Session: $S" && ok t_handoff_trailer || fail t_handoff_trailer ""
# t_handoff_keyscan_refuses: a planted value stops it, nothing pushed
before=$(git -C "$d/repo" rev-parse origin/v1.0/integration); echo "k=sk-test-0123456789abcdef" >> "$d/night/lanes/integrator.log"
CLAUDE_API_KEY=sk-test-0123456789abcdef GC_SESSION=$S GC_REPO=$d/repo GC_NIGHT=$d/night bash "$d/tools/handoff.sh"; rc=$?
git -C "$d/repo" fetch -q origin; [ $rc = 6 ] && [ "$(git -C "$d/repo" rev-parse origin/v1.0/integration)" = "$before" ] && ok t_handoff_keyscan_refuses || fail t_handoff_keyscan_refuses "rc=$rc"
# t_restore_keeps_scribed: restore from the pushed tree into an empty night
git -C "$d/repo" worktree add -q --detach "$d/new" origin/v1.0/integration
bash "$d/new/$C/install.sh" --dest "$d/tools2" --night "$d/night2" --skip-npm
[ "$(cat "$d/night2/lanes/integrator.log.scribed")" = 2 ] && [ ! -e "$d/night2/lanes/pad-model.log.scribed" ] && [ -d "$d/night2/mrq/done" ] && [ -e "$d/night2/integration.lock" ] && ok t_restore_keeps_scribed || fail t_restore_keeps_scribed ""
# t_restore_never_overwrites
echo local > "$d/night2/lanes/integrator.log"; bash "$d/new/$C/install.sh" --dest "$d/tools2" --night "$d/night2" --skip-npm | grep -q 'kept existing integrator.log' && [ "$(cat "$d/night2/lanes/integrator.log")" = local ] && ok t_restore_never_overwrites || fail t_restore_never_overwrites ""
finish
```
- [ ] **Step 2: Commit the test alone as the red commit (no box)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "test(conductor): handoff bundle and restore [red]"` (trailer lines as in Global Constraints). `test/red-green.sh` later runs `bash $C/test/test_handoff.sh` at this commit.
Expected at the red run: FAIL at `t_handoff_bundle` (`missing ledger/integrator.log`).
- [ ] **Step 3: Implement the `handoff.sh` copy list, keyscan gate, README rows, and `install.sh --night`**
- [ ] **Step 4: Commit the implementation (no box; the pass is proven in the slice box, Task 10)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "feat(conductor): handoff copies ledger and reports behind keyscan; install restores state"`. Slice-box command for this task: `bash $C/test/test_handoff.sh && bash $C/test/test_install.sh`.
Expected: `5 passed, 0 failed`; install test still `6 passed`.
- [ ] **Step 5: Push the branch**

### Task 9: What the next session reads

**Files:**
- Modify: `$C/handoff-head.md:20-40` (paths, install, proxy patch section)
- Create: `$C/STATE.md`, `$C/SECRETS.md`, `$C/snapshots.md`, `$C/owner-decisions-draft.md`, `$C/NEW-SESSION-PROMPT.md`, `$C/reports/README.md`, `$C/test/test_docs.sh`

**Interfaces:**
- Consumes: `keyscan.NAMES` (Task 7), `box.mjs snaps` and `spend` (Task 2), `install.sh` options (Tasks 3, 8), `gc-lane.js` args (Task 4).
- Produces:
  - `handoff-head.md` gains a "Sandbox proxy patch" section: what (`base-client.js` ESM import to `EnvHttpProxyAgent`, CJS through `proxy-preload.cjs`), why (the SDK's `undici.Agent` ignores `HTTPS_PROXY`), and that `install.sh` applies it; and a "Start in a new container" section: `install.sh --dest ... --repo ... --night ... --wt ... --session ...`, `export NODE_OPTIONS=...` for `sbx.mjs`, `box.mjs snaps`. Paths written as `$GC_NIGHT`, `$GC_TOOLS`, `$GC_WT`.
  - `STATE.md`: the ledger summary at its write time: integration head; merged packages since 31 Oct by PRD (from `ledger/integrator.log` merge lines); open branches (`v1.0/prd-05-masking-exact-trie-matcher`); rulings `OWN-1` to `OWN-11`, next `OWN-12`; the four answered owner asks (22:05Z, 22:03Z, 22:27Z, 20:32Z) and the open asks list from spec section 5; the spend line from `box.mjs spend --since 2026-10-30T00:00:00Z --model-usd 27 --ledger`.
  - `SECRETS.md`: the 14 names, one per line as `` - `NAME`: what reads it ``, plus "push access to `v1.0/*`" and "network access to the Vercel API", and the rule "check by name only".
  - `snapshots.md`: the output of `node box.mjs snaps` (ids, expiry; build and pad ids marked).
  - `owner-decisions-draft.md`: one numbered line per owner answer not yet in `OWNER-DECISIONS.md`, each ending with its ledger time: 06:17Z (keys and box length), 17:44Z and 18:04Z (exact-value masking, sources 1 to 3), 18:56Z (Hermes `state.db` until a link reader), 19:17Z and 19:31Z (gateway model and the cheap-model rule), 20:32Z (deltas-only stated loss), 20:35Z (sandbox cost rule), 22:03Z (Hermes C2 on task logs), 22:05Z (cap 32, then the single-pass matcher), and the section of `OWNER-DECISIONS.md` it belongs in; a head line saying the owner copies these, agents never edit that file. Owner wording is quoted, so model names may appear here.
  - `NEW-SESSION-PROMPT.md`: `/root/gc-night/finish/NEW-SESSION-PROMPT.md` rewritten so that: step 1 is `install.sh --dest $HOME/gc-tools --repo <checkout> --night $HOME/gc-night --wt $HOME/gc-wt --session <this session's URL> --coauthor "<this session's co-author line>" --lane-dir <working dir>` (with `--night` only when `$HOME/gc-night` is absent); the stop condition names `ledger/integrator.log`, `package.json`, `install.sh` and the `handoff-head.md` patch section; step 3 is `box.mjs snaps`; lane starts pass `session` and `paths`; the model pin is described as "the pinned cheap model in the rig's harness files", with no model ids; secrets point to `SECRETS.md`; "first action: one ledger line `Conductor resumed in a new session at <time>Z on <head>.`".
  - `reports/README.md`: a table `| Report | Head | What it shows |`, one row per `reports/*.txt` copied by Task 10, each row read from that report's first three lines.

- [ ] **Step 1: Write the failing test** `test/test_docs.sh`
```bash
. "$(dirname "$0")/t.sh"; C=$(cd "$(dirname "$0")/.." && pwd); P="$C/STATE.md $C/SECRETS.md $C/snapshots.md $C/owner-decisions-draft.md $C/NEW-SESSION-PROMPT.md $C/reports/README.md $C/handoff-head.md"
# t_docs_exist
for f in $P; do [ -s "$f" ] || fail t_docs_exist "$f"; done; ok t_docs_exist
# t_no_em_dash
grep -n $'\xe2\x80\x94' $P && fail t_no_em_dash "" || ok t_no_em_dash
# t_no_model_ids (records and the owner-quote draft exempt)
grep -nEi 'claude-(haiku|opus|sonnet)|haiku|opus|sonnet|gpt-[0-9]|deepseek-v[0-9]|gemini-[0-9]' "$C/STATE.md" "$C/SECRETS.md" "$C/NEW-SESSION-PROMPT.md" "$C/reports/README.md" "$C/handoff-head.md" && fail t_no_model_ids "" || ok t_no_model_ids
# t_secrets_match_keyscan
want=$(python3 -c "import sys;sys.path.insert(0,'$C');import keyscan;print('\n'.join(sorted(keyscan.NAMES)))")
got=$(grep -oE '^- `[A-Z_]+`' "$C/SECRETS.md" | tr -d '`- ' | sort)
[ "$want" = "$got" ] && ok t_secrets_match_keyscan || fail t_secrets_match_keyscan "$(diff <(echo "$want") <(echo "$got"))"
# t_draft_cites_ledger: every draft line's time is in the ledger copy (checked after Task 10 adds ledger/)
for t in $(grep -oE '[0-9]{2}:[0-9]{2}Z' "$C/owner-decisions-draft.md" | sort -u); do grep -q "^$t" "$C/ledger/integrator.log" 2>/dev/null || [ ! -d "$C/ledger" ] || fail t_draft_cites_ledger "$t"; done; ok t_draft_cites_ledger
# t_prompt_names_install_and_session
grep -q 'install.sh --dest' "$C/NEW-SESSION-PROMPT.md" && grep -q -- '--session' "$C/NEW-SESSION-PROMPT.md" && ! grep -q 'session_01V2' "$C/NEW-SESSION-PROMPT.md" && ok t_prompt_names_install_and_session || fail t_prompt_names_install_and_session ""
# t_patch_written_down
grep -q 'EnvHttpProxyAgent' "$C/handoff-head.md" && grep -q 'proxy-preload.cjs' "$C/handoff-head.md" && ok t_patch_written_down || fail t_patch_written_down ""
finish
```
- [ ] **Step 2: Commit the test alone as the red commit (no box)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "test(conductor): handover docs checks [red]"` (trailer lines as in Global Constraints). `test/red-green.sh` later runs `bash $C/test/test_docs.sh` at this commit.
Expected at the red run: FAIL at `t_docs_exist` (`STATE.md`).
- [ ] **Step 3: Write the docs.** On the conductor (API calls, no box), with `ln -s /root/gc-tools/node_modules "$WT/$C/node_modules"; BOXJS=$WT/$C/box.mjs`: `node "$BOXJS" snaps > "$WT/$C/snapshots.md"` and `node "$BOXJS" spend --since 2026-10-30T00:00:00Z --model-usd 27 --ledger` (its line goes into `STATE.md`). Then `python3 "$WT/$C/keyscan.py" "$WT/$C"` with the real env: `0 hit(s)` and `unset: none` required before committing.
- [ ] **Step 4: Commit the implementation (no box; the pass is proven in the slice box, Task 10)**
Run: `git -C "$WT" add -A && git -C "$WT" commit -qm "docs(conductor): state, secrets, snapshots, owner draft, new-session prompt"`. Slice-box command for this task: `bash $C/test/test_docs.sh`.
Expected: `7 passed, 0 failed`.
- [ ] **Step 5: Push the branch**

### Task 10: The slice box, keyscan of the tree, merge train and first handoff

**Files:**
- Create: `$C/test/run-all.sh`, `$C/test/red-green.sh`, `$C/test/fresh-clone.sh`

**Interfaces:**
- Consumes: everything above.
- Produces: `test/run-all.sh` runs every `test/test_*.sh`, `test/test_*.mjs` and `python3 -m unittest discover -s test -p 'test_*.py'` from the repo root, prints one `PASS <file>` or `FAIL <file>` line each, exit 1 on any failure. `test/red-green.sh BASE` for each commit in `BASE..HEAD` whose subject ends ` [red]`: checks it out in a scratch worktree, runs the one test file that commit added, and requires a nonzero exit (`RED-OK <subject>`, else `RED-MISSING <subject>` and exit 1); then runs `run-all.sh` at HEAD. `test/fresh-clone.sh`: with a new temp `HOME`, runs `install.sh --dest $HOME/gc-tools --repo ~/w --night $HOME/gc-night` (real `npm ci`), then asserts `grep -c EnvHttpProxyAgent $HOME/gc-tools/node_modules/@vercel/sandbox/dist/api-client/base-client.js` is 1, `node $HOME/gc-tools/box.mjs --help` exits 0, `GC_REPO=~/w bash $HOME/gc-tools/mergeq.sh 1` exits 0 on an empty queue, and `grep -rnE '/root/|/home/user' $HOME/gc-tools --include='*.sh' --include='*.mjs' --include='*.js' --include='*.cjs' --include='*.py'` prints nothing; prints `no ledger yet` when `$C/ledger/` is absent, else asserts `$HOME/gc-night/lanes/integrator.log` exists.

- [ ] **Step 1: Write `run-all.sh`, `red-green.sh`, `fresh-clone.sh`** as above.
- [ ] **Step 2: Commit them** (`chore(conductor): slice box scripts`; no red commit: these are the harness, proven by the box run itself).
- [ ] **Step 3: Set up the box runner**
`ln -s /root/gc-tools/node_modules "$WT/$C/node_modules"; BOXJS=$WT/$C/box.mjs; BOXV="--vcpus 2"`
- [ ] **Step 4: The one slice box**
Run (conductor, starts one box): `J=$(mktemp "$GC_WT/job.XXXXXX"); printf 'set -e\nbash %s/test/red-green.sh origin/v1.0/integration\nbash %s/test/fresh-clone.sh\n' "$C" "$C" > "$J"; node "$BOXJS" run --repo "$WT" --ref HEAD --script "$J" --minutes 15 $BOXV; rm -f "$J"` (the bundle carries `origin/v1.0/integration` as `refs/remotes/b/...`; `red-green.sh` takes the merge base of HEAD with it when the name is missing).
Expected: nine `RED-OK` lines (Tasks 1 to 9), every `PASS` line from `run-all.sh` (test_lib 8, test_box 6, test_install 6, test_gc_lane 4, test_pad_drivers 4, test_dbfetch 3, test_keyscan 7, test_handoff 5, test_docs 7), then the fresh-clone asserts with `no ledger yet`, `[box] exit 0`. On a failure: fix every finding in one batch, then one more box (the owner's one fix round).
- [ ] **Step 5: Keyscan the tree and the history (conductor, real env, no box)**
Run: `python3 "$WT/$C/keyscan.py" "$WT"` and `L=$(mktemp); git -C "$WT" log -p --since=2026-10-30 HEAD > "$L"; python3 "$WT/$C/keyscan.py" "$L"; rm -f "$L"`
Expected: both `keyscan: 0 hit(s), 14 keys checked, unset: none`. A hit stops the task and goes to the owner.
- [ ] **Step 6: Final whole-branch review (read-only, no box)**
One review of `git diff origin/v1.0/integration...HEAD` against this plan's Global Constraints and Review Focus. Every finding is fixed in one batch (and rerun in the one fix-round box if it touches code) before Step 7.
- [ ] **Step 7: Queue the merge as its own small train**
Write `{"branch": "v1.0/conductor-handover", "message": "conductor handover: portable tools, proxy patch, keyscan, state docs", "refresh_lock": false}` to `$GC_NIGHT/mrq/$(date -u +%H%M%S)-conductor-handover.req` (via a `.tmp` name and `mv`). The conductor runs it at once as a train of this one branch (one gate on its head); it does not wait for, or join, the code-wave train. Branches already queued for the code-wave train stay queued and rebase on the result.
Expected: `<id>.res` with `"exit": 0`; the ledger shows `merge ... v1.0/conductor-handover ...`.
- [ ] **Step 8: Install the merged tools and push the first full handoff (conductor, no box)**
Only when no merge is in flight (no `$GC_NIGHT/mrq/*.claimed`). Lanes started after this pass `session` and `paths` (Task 4).
```bash
git -C /home/user/goal-cup-proxy fetch -q origin
git -C /home/user/goal-cup-proxy worktree add -q --detach "$GC_WT/handover-install" origin/v1.0/integration
bash "$GC_WT/handover-install/$C/install.sh" --dest /root/gc-tools --repo /home/user/goal-cup-proxy --night /root/gc-night --wt /root/gc-wt --session https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6 --lane-dir /home/user/gc-scripts
GC_SESSION=https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6 bash /root/gc-tools/scribe.sh
GC_SESSION=https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6 bash /root/gc-tools/handoff.sh
git -C /home/user/goal-cup-proxy worktree remove --force "$GC_WT/handover-install"
```
Expected: `scribed <n> lines`, then `handoff pushed <sha>` (its keyscan 0 hits). Read-only check: `git ls-tree -r --name-only origin/v1.0/integration -- $C/ledger $C/reports` lists `ledger/integrator.log` and every `reports/*.txt`, and each time in `owner-decisions-draft.md` is found at a line start of `ledger/integrator.log`. Then one ledger line: `<HH:MM>Z | integrator conductor | handover | conductor state on integration at <sha>; new session may start`.

---

## Self-review

1. Spec coverage: tools with local patches (Tasks 1 to 3, 5, 6); `@vercel/sandbox` patch and the preload (Task 3, written down in Task 9); `gc-lane.js` (Task 4); keyscan and scrub (Task 7); ledger full copy (Task 8) and summary `STATE.md` (Task 9); report summaries (Tasks 8, 9); trailer as a parameter (Tasks 1, 4); spend line with box minutes (Tasks 2, 9); `owner-decisions-draft.md` (Task 9); NEW-SESSION-PROMPT (Task 9); snapshot ids (Tasks 2, 8, 9); keyscan proof of the committed tree and history (Task 10); secret names (Global Constraints, `SECRETS.md`, test in Task 9); fresh clone runs `box.mjs --help` and `mergeq.sh` with no `/root` paths (Task 10 `fresh-clone.sh`, also `t_no_root_paths`). The spec's `LESSONS-CONDUCTOR.md` and `branches.md` are not separate files: the branch table stays in `HANDOFF.md` (handoff.sh) and section 4 lessons travel in the committed `FINISH-PLAN.md`.
2. Step scan: each code step names its file, signature and pinned values; test steps carry assertions.
3. Type consistency: `gc_trailer`, `gc_need_repo`, `GC_*` names, `sessionMinutes`, `ledgerLine`, `keyscan.NAMES`, `scan`, `scrub`, `install.sh` options and `args.paths` keys match across tasks.
4. Review Focus: all five have tests in their owning tasks (1, 4, 7, 7, 8, 3).
5. Box use and merge shape (re-checked against the owner-approved execution shape): one slice box (Task 10 Step 4), at most one more for the single fix round, one gate for the small tool train (not the code-wave train), no box per task. The Execution section names a lane, order and model for every task.
6. Proportion: bodies given only for exact copy (the patch line, the preload, trailer text, ledger line format) and for the test assertions.

---

## Execution

Lane script: `docs/plans/2026-10-30-v1.0/conductor/gc-lane.js` (the current one on the conductor machine; Task 4 only changes it for the next session). One lane = one worktree and branch `v1.0/conductor-handover-<lane>`; no box per lane or fix round (reviewers read-only). Task numbering and the red-then-green commits stay as above.

| Lane | Tasks | Starts | Model |
|---|---|---|---|
| tools portability | 1, 2, 3, 4 (in order; 1 first) | now | sonnet |
| run scripts | 5, 6 (write against the Interfaces blocks of Tasks 1, 3, 7) | now, parallel | sonnet |
| state files | 7, then 8 and 9 | 7 now, parallel; 8 and 9 wait for the tools lane (they edit `install.sh`, `handoff.sh`, `handoff-head.md`) | sonnet |
| keyscan runs | Task 7 CLI runs, Task 9 Step 3 and Task 10 Step 5 scans | after the lane that needs them | haiku |
| copying | verbatim copies (Task 6 `idx.sh`, `install.sh`; Task 9 `snaps`; Task 10 Step 8 install and handoff) | with its task | haiku |
| assembly | merge the three lane branches into `v1.0/conductor-handover` (merge commits), fix conflicts, push | when all lanes are done | sonnet |
| slice box | Task 10 Steps 1 to 5 | after assembly | sonnet |
| final review | Task 10 Step 6, whole branch, read-only | after the slice box passes | default (most capable) |
| small train | Task 10 Steps 7 and 8 | after a clean review | sonnet |

Order: the three build lanes run together (beside the other plans' code-wave lanes), then assembly, one slice box, final review, small train. The small train and the Task 10 Step 8 install come before the H1 build: the mini-test wave and the one-hour runs use `/root/gc-tools/padrun.sh`, `keyscan.py` and `proxy-preload.cjs`. Boxes: 1 slice box (2 with the fix round), 1 gate. No task here is core design, so the most capable model is used only for the final review (and for the owner-asked breaker, if one is requested).
