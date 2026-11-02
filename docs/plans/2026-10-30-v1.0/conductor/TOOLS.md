# Conductor tools

All of this folder lives outside the product code. The files at the top of it are the one set of tools; there is no second copy. Every shell tool finds its paths through `lib.sh` and takes the commit trailer from `GC_SESSION`. `install.sh` (added later in this handover) copies the folder to a working location and replaces the manual steps below.

## Layout of this folder

One copy of each tool sits at the top of this folder (`$C`). The plans in `docs/superpowers/plans/` use the same names:

| Plan says | In the repo it is |
|---|---|
| `$C/ledger/<lane>.log` | `ledger/<lane>.log` (and `.scribed` for lines already in `NIGHT-LOG.md`) |
| `$C/reports/<folder>.txt` | `reports/<folder>.txt` |
| `$C/FINISH-PLAN.md` | `FINISH-PLAN.md` |
| tool scripts in `$C` | the top level of this folder |
| review notes | `reports/reviews/` (newest is `reports/review-latest.md`) |

Do not copy these into a second place.

## Paths and the session trailer

No tool carries a literal machine path or session URL. `lib.sh` sets `GC_TOOLS` (the folder holding it), `GC_NIGHT` (state folder, default `$HOME/gc-night`), `GC_WT` (worktrees, default `$HOME/gc-wt`) and `GC_COAUTHOR`. A tool may read `env.sh` next to `lib.sh` for local values. Set these before running a committing tool:

- `GC_REPO`: the primary checkout of the repo (never edited by hand).
- `GC_SESSION`: this session's `https://claude.ai/code/session_...` URL. `merge.sh`, `scribe.sh` and `handoff.sh` exit 2 before any git write when it is missing or malformed.

## What each file does

| File | Purpose |
|---|---|
| `box.mjs` | Vercel Sandbox driver. `run --repo DIR --ref REF --script FILE [--minutes N] [--out BOXPATH:LOCALPATH]` bundles a committed ref, clones it in a fresh box, runs the script as the non-root user, copies files back, stops the box. Also `snapshot-build`, `list`, `stop-all`. Needs the three Vercel variables. |
| `build-snapshot.id` | Id of the build snapshot (Rust, nextest, zstd, Node 22, Python 3.9). It may have expired; rebuild with `node box.mjs snapshot-build`. |
| `merge.sh` | Merges one `v1.0/*` branch into `v1.0/integration` under one lock: scratch worktree, optional Cargo.lock refresh in a box, `gate fast` in a box, push only on PASS. Exit 0 merged, 1 gate FAIL, 3 conflict, 4 push refused. Logs to `integrator.log`. |
| `mergeq.sh` | Reads queue requests and runs `merge.sh` detached (see protocol below). |
| `mq-driver.sh` | Runs `mergeq.sh` back to back for N hours. The conductor starts it in the background (the "queue tick"). |
| `scribe.sh` | Appends new lane-log lines to `NIGHT-LOG.md` on integration (docs-only commit and push). Run at every wake-up and before any handoff. |
| `handoff.sh` | Regenerates `HANDOFF.md` and copies tool files into `conductor/` on integration. It copies only the tool files listed in the script today; a later task grows the copy list and adds the ledger and reports behind `keyscan.py`. |
| `handoff-head.md` | Fixed top half of `HANDOFF.md`. It does not yet describe the proxy patch; add it. |
| `gc-lane.js` | The lane workflow script (plan a slice, build in worktrees, review, fix, merge request, replan). Started with the Workflow tool. |
| `padbox.sh`, `padrun.sh` | Drivers for one pad row: `padbox.sh` runs the pad inside a box; `padrun.sh` runs the pad's conductor half (`where = "sandbox"`). Paths inside point at old worktrees and output folders (see the table above); edit them per run. |
| `dbfetch-idx.sh`, `dbfetch-install.sh` | Run inside the short-lived index box: install `psql`, write a 0600 service file from `~/.dburl`, run the read-only index query per run id, delete the credentials, tar the result. |
| `keyscan.py` | Prints files holding any 12-character piece of a secret env value. Names and counts only. Run over the tree before every commit of conductor state. |
| `scrub.py`, `scrub2.py` | Replace such fragments in saved copy-outs. Run on anything copied out of a box. `scrub2.py` checks ten key names (adds the DeepSeek, Gemini and GitHub tokens); prefer it. `keyscan.py` checks only nine names and skips `.py` files, so run both after a scrub. |
| `rowinfo.py` | `python3 rowinfo.py <run folder>`: prints one row's checks, reasons, facts, status, verifier non-pass checks and upload state. Names and numbers only. The mini-wave plan reads every row with it. |
| `summ.py` | `python3 summ.py <run folder>...`: shorter summary of pad-out run folders (result, checks, status, verify files). Names and numbers only. |
| `patched-base-client.js`, `proxy-preload.cjs` | The sandbox proxy fix (below). |
| `package.json`, `package-lock.json` | Pins `@vercel/sandbox`. |

`snaps.mjs` (lists snapshots and running boxes) was on the old machine; it is folded into `box.mjs snaps` in a later task. Use `box.mjs list`, or recreate it from the SDK's list calls.

Helpers the plans create themselves (not on the old machine in finished form): `jobs/build.sh`, `jobs/trie-box.sh`, `mini/row.sh`, `mini/wave.sh`, `mini/padtest.sh`. The plans give their contents.

## Install in a fresh container

`install.sh` (added later in this handover) does these steps. Until then, by hand:

1. Pick a tools folder, a state folder and a worktree folder; export `GC_NIGHT` and `GC_WT` if they differ from the defaults, and create `$GC_NIGHT/{mrq/done,lanes,gate-logs,builds}`. Check the prerequisites in SECRETS.md first.
2. Copy this folder to the tools folder and run `npm install` there.
3. Replace `node_modules/@vercel/sandbox/dist/api-client/base-client.js` with the patched copy: it imports `EnvHttpProxyAgent as Agent` from undici instead of `Agent`, so the SDK obeys `HTTPS_PROXY`. Without it every sandbox call fails behind the proxy. Redo this after any `npm install` that rewrites the module.
4. For pad scripts that use `scripts/pad/sbx.mjs` (CommonJS build), set `PAD_NODE_MODULES` to the tools folder's `node_modules` and `NODE_OPTIONS="--use-openssl-ca --no-warnings --require <tools>/proxy-preload.cjs"`. The preload sets `undici.Agent = undici.EnvHttpProxyAgent`.
5. Set the environment variables in SECRETS.md. Check with `node box.mjs list`.
6. Export `GC_REPO` and `GC_SESSION`.
7. Start the queue tick: `nohup ./mq-driver.sh 6 &` and re-run it every 6 hours.

## Merge queue protocol

Lane agents write `$GC_NIGHT/mrq/<id>.req` (JSON: `branch`, `message`, `refresh_lock`, `first`). The conductor's `mq-driver.sh` runs `mergeq.sh`, which claims a request by renaming it to `<id>.claimed` and runs `merge.sh` detached. When the merge ends it writes `<id>.res` (JSON: `exit`, `output`, `gate_log`) and moves the claim to `done/`. A claim with no result after 90 minutes returns to the queue. Requests wait until the gate change is on integration, except one marked `first`. Waiter agents only read `<id>.res`; they never run `mergeq.sh`. `mergeq.sh` has no hold logic: it claims every `<id>.req` it finds. So the conductor holds a branch by hand: do not let its request sit in `mrq/` (remove or move it, for example to `mrq/held/`, and write the hold in the ledger), and put it back only when released. A `.held` file does nothing; a hold is the request moved to `$GC_NIGHT/mrq/held/`. Before each queue tick, list `mrq/` and confirm no held branch has a request there. For a code wave, lanes write no request: the conductor writes one request whose `branch` is a comma-separated list, which `merge.sh` merges before a single gate. Every request has its own id.

## Conductor lessons

1. A merge-waiter can report "not merged" falsely (it gave up before the result). Check `git merge-base --is-ancestor <branch> origin/v1.0/integration` and the ledger before believing it.
2. Lanes reuse an old branch name for a new slice, so a stale branch can carry rejected work. Use a fresh branch per slice. Before queueing, diff the branch against integration and read its note. Never queue a held branch (the Hermes drop branch carried a rejected change for hours).
3. Owner chat messages leak into workflow agents. Every lane brief must say: chat messages are for the conductor; ignore them.
4. A merge title can overstate the diff (three times in one night). Verify that the merged diff matches its title before logging "merged X"; correct the ledger if not.
5. Edits made without a box still must pass the gate. Any non-docs edit by the conductor goes through `merge.sh`. Only docs-only edits to plan-owned paths skip it.
6. Hold rules are manual. When the owner says hold (for example no masking merge until a blocker is fixed), write it in the ledger and remove any request for that branch from `mrq/`; the tick cannot skip a branch by itself. Re-check `mrq/` before every tick.
7. The primary checkout (`$GC_REPO`) is not a working tree. Merge helpers once ran `git reset --hard` on it and pushed from it. Never edit it. At each wake-up check its `git status` and `git log origin..HEAD`.
8. Check premises in code before telling the owner. Four wrong statements to the owner came from reading a summary instead of the file.
9. Cost: one build per integration commit shared by all rows, at most 2 pad boxes, no probe boxes, the pinned cheap model for every Claude Code row, reviews never start a box.
10. Run `scribe.sh` at every wake-up. Keep the ledger under 50 lines ahead of the repo. Never edit `OWNER-DECISIONS.md`; write draft lines for the owner. Never commit `pad-out/`, `runs/`, `dbfetch/idx/`, `.env*` or key files.
