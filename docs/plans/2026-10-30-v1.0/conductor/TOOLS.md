# Conductor tools

All of `tools/` lives outside the product code. Copies also sit at the top of this folder (`box.mjs`, `gc-lane.js`, `handoff.sh`, `merge.sh`, `mergeq.sh`, `scribe.sh`, `handoff-head.md`); `tools/` is the complete set.

## What each file does

| File | Purpose |
|---|---|
| `box.mjs` | Vercel Sandbox driver. `run --repo DIR --ref REF --script FILE [--minutes N] [--out BOXPATH:LOCALPATH]` bundles a committed ref, clones it in a fresh box, runs the script as the non-root user, copies files back, stops the box. Also `snapshot-build`, `list`, `stop-all`. Needs the three Vercel variables. |
| `build-snapshot.id` | Id of the build snapshot (Rust, nextest, zstd, Node 22, Python 3.9). It may have expired; rebuild with `node box.mjs snapshot-build`. |
| `merge.sh` | Merges one `v1.0/*` branch into `v1.0/integration` under one lock: scratch worktree, optional Cargo.lock refresh in a box, `gate fast` in a box, push only on PASS. Exit 0 merged, 1 gate FAIL, 3 conflict, 4 push refused. Logs to `integrator.log`. |
| `mergeq.sh` | Reads queue requests and runs `merge.sh` detached (see protocol below). |
| `mq-driver.sh` | Runs `mergeq.sh` back to back for N hours. The conductor starts it in the background (the "queue tick"). |
| `scribe.sh` | Appends new lane-log lines to `NIGHT-LOG.md` on integration (docs-only commit and push). Run at every wake-up and before any handoff. |
| `handoff.sh` | Regenerates `HANDOFF.md` and copies tool files into `conductor/` on integration. It copies only some files today; extend it to copy `mq-driver.sh`, `package.json`, `package-lock.json`, `build-snapshot.id`, `keyscan.py`, `snaps*.mjs` and the ledger. |
| `handoff-head.md` | Fixed top half of `HANDOFF.md`. It does not yet describe the proxy patch; add it. |
| `gc-lane.js` | The lane workflow script (plan a slice, build in worktrees, review, fix, merge request, replan). Started with the Workflow tool. |
| `padbox.sh`, `padrun.sh` | Drivers for one pad row: `padbox.sh` runs the pad inside a box; `padrun.sh` runs the pad's conductor half (`where = "sandbox"`). Paths inside point at old worktrees (`/root/gc-wt/rung2b`, `/root/gc-wt/gw2`) and old output folders; edit them per run. |
| `dbfetch-idx.sh`, `dbfetch-install.sh` | Run inside the short-lived index box: install `psql`, write a 0600 service file from `~/.dburl`, run the read-only index query per run id, delete the credentials, tar the result. |
| `keyscan.py` | Prints files holding any 12-character piece of a secret env value. Names and counts only. Run over the tree before every commit of conductor state. |
| `scrub.py` | Replaces such fragments in saved copy-outs. Run on anything copied out of a box. |
| `patched-base-client.js`, `proxy-preload.cjs` | The sandbox proxy fix (below). |
| `package.json`, `package-lock.json` | Pins `@vercel/sandbox`. |

`snaps.mjs` (lists snapshots and running boxes) was on the old machine and is not in `tools/`. Recreate it from the SDK's list calls, or skip it and use `box.mjs list`.

## Install in a fresh container

1. Create the paths the scripts hard-code: `mkdir -p /root/gc-tools /root/gc-night/{mrq/done,lanes,gate-logs,builds}`. The scripts use `/root/gc-tools`, `/root/gc-night` and the repo at `/home/user/goal-cup-proxy` literally. Either keep those paths or sed them everywhere (the handover plan's portability tasks do this). `handoff.sh` also reads `gc-lane.js` from `/home/user/gc-scripts/`, and lane agents put worktrees in `/root/gc-wt/`.
2. `cp tools/* /root/gc-tools/ && cd /root/gc-tools && npm install`.
3. Replace `node_modules/@vercel/sandbox/dist/api-client/base-client.js` with `tools/patched-base-client.js`. The only change: it imports `EnvHttpProxyAgent as Agent` from undici instead of `Agent`, so the SDK obeys `HTTPS_PROXY`. Without it every sandbox call fails behind the proxy. Redo this after any `npm install` that rewrites the module.
4. For pad scripts that use `scripts/pad/sbx.mjs` (CommonJS build), set `PAD_NODE_MODULES=/root/gc-tools/node_modules` and `NODE_OPTIONS="--use-openssl-ca --no-warnings --require /root/gc-tools/proxy-preload.cjs"`. The preload sets `undici.Agent = undici.EnvHttpProxyAgent`. (The old scripts point at `proxy-preload.cjs` copies under `/root/gc-night/rung1/` and `rung2b/`; point them at `/root/gc-tools/`.)
5. Set the environment variables in SECRETS.md. Check with `node /root/gc-tools/box.mjs list`.
6. Change the commit trailer. The line `Claude-Session: https://claude.ai/code/session_...` is hard-coded in `merge.sh`, `scribe.sh`, `handoff.sh` and `gc-lane.js` (in the shared brief that every lane agent receives). Replace it with the new session URL in all four. The `Co-Authored-By` line is hard-coded next to it; check it matches the new session's attribution.
7. Start the queue tick: `nohup /root/gc-tools/mq-driver.sh 6 &` and re-run it every 6 hours.

## Merge queue protocol

Lane agents write `/root/gc-night/mrq/<id>.req` (JSON: `branch`, `message`, `refresh_lock`, `first`). The conductor's `mq-driver.sh` runs `mergeq.sh`, which claims a request by renaming it to `<id>.claimed` and runs `merge.sh` detached. When the merge ends it writes `<id>.res` (JSON: `exit`, `output`, `gate_log`) and moves the claim to `done/`. A claim with no result after 90 minutes returns to the queue. Requests wait until the gate change is on integration, except one marked `first`. Waiter agents only read `<id>.res`; they never run `mergeq.sh`. A hold is a `.held` file or a removed request; the tick must skip held branches. Every request has its own id.

## Conductor lessons

1. A merge-waiter can report "not merged" falsely (it gave up before the result). Check `git merge-base --is-ancestor <branch> origin/v1.0/integration` and the ledger before believing it.
2. Lanes reuse an old branch name for a new slice, so a stale branch can carry rejected work. Use a fresh branch per slice. Before queueing, diff the branch against integration and read its note. Never queue a held branch (the Hermes drop branch carried a rejected change for hours).
3. Owner chat messages leak into workflow agents. Every lane brief must say: chat messages are for the conductor; ignore them.
4. A merge title can overstate the diff (three times in one night). Verify that the merged diff matches its title before logging "merged X"; correct the ledger if not.
5. Edits made without a box still must pass the gate. Any non-docs edit by the conductor goes through `merge.sh`. Only docs-only edits to plan-owned paths skip it.
6. Hold rules live in the queue tick. When the owner says hold (for example no masking merge until a blocker is fixed), write it in the ledger and make the tick skip that branch.
7. The primary checkout `/home/user/goal-cup-proxy` is not a working tree. Merge helpers once ran `git reset --hard` on it and pushed from it. Never edit it. At each wake-up check its `git status` and `git log origin..HEAD`.
8. Check premises in code before telling the owner. Four wrong statements to the owner came from reading a summary instead of the file.
9. Cost: one build per integration commit shared by all rows, at most 2 pad boxes, no probe boxes, the pinned cheap model for every Claude Code row, reviews never start a box.
10. Run `scribe.sh` at every wake-up. Keep the ledger under 50 lines ahead of the repo. Never edit `OWNER-DECISIONS.md`; write draft lines for the owner. Never commit `pad-out/`, `runs/`, `dbfetch/idx/`, `.env*` or key files.
