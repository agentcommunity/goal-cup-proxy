# Real-machine round, 24 Oct 2026 UTC (results)

The section 6 cases of the local review brief (`LOCAL-REVIEW-BRIEF.md`, branch
`smart/upbeat-mendel-uvoxtr-local-brief`), run on the owner's Mac (arm64) and in Docker on it
(Linux arm64), against staging, 06:30 to 07:50 UTC on 24 Oct (the evening of 23 Oct in PDT). Build: branch `smart/upbeat-mendel-uvoxtr-kit-setup` at `1f023d31`
(PR #258 stacked on #257, with `main` holding #243, #249, #259, #260 and #261), debug build. Harnesses:
Codex 0.156.1 on the host (connector names 0.153.0; image 0.153.0), Claude Code 2.1.299. Cheapest
models (gpt-5.6-luna low, haiku). Times are UTC. Raw logs, captures and DB output stay on the Mac,
outside the repository.

**Verdict: not ready to tag.** Two release blockers: R1 stops real starts on macOS and Linux
whenever more than about 30 s pass before the approval; R2 stops starts on a Mac whose only GitHub
login is `gh`. Everything after a successful start held up well: recording, upload, `.env.local`
never committed, restarts and continue, the no-session-file warning, practice isolation, the disk
pause.

## Release blockers

**R1. A start fails if more than ~30 s pass before the approval.** `goalcup start` launches
`goalcup-daemon --exit-when-idle` (`IDLE_GRACE` 30 s, `crates/goalcup-daemon/src/bin/goalcup-daemon.rs:53`)
at the top of the command, before the repo questions, the push test and the approval. When the
approval comes after the daemon has exited, the start prints `Approved.`, `Cloning your repository…`,
then `goalcup: the daemon is not reachable: http://127.0.0.1:<port>/v0/capture-workspaces ...
Connection refused`, and the server closes the run as `abandoned` ("goalcup start failed before the
launch"). The testers measured the GitHub checks alone at about 20 to 30 s on this Mac before the
code appeared, and 5 to 7 s per SSH connection through the 1Password agent (`ssh -T` 5.6 and 5.3 s,
`ls-remote` 7.0 and 6.3 s; a start makes 3 or 4), so it failed even with the approval posted the
instant the code appeared. It hit every tester that started by hand (5 of 5): 8 abandoned test runs
(`r12299b1e8db50e6f`, `r6fb7813794356832`, `ra537b3de3abce6e8`, `rb2d8ce9f576e486f`,
`r2c91477269df8e03`, `r8e3e46ce24e3a234`, `r93abb36bd5ba5d7c`, `r444616befe61a6b3`). The kit's matrix rows did
not hit it in Docker (fast checks, instant approval). Workaround used to test everything else: keep
the daemon awake by running `goalcup entries` (or a hand-started daemon) from a second shell.
Owner's direction (24 Oct): no 30 s window during the start; the start may wait as long as the
approval code is valid (15 min) and the player needs. Likely fix: start the daemon after the
approval, or keep it alive while `start` runs; the idle exit after a run can stay.

Reproduce: `goalcup start --harness codex-cli --test`, wait 60 s at `Name it:`, type a new empty
public repo, press Enter at the link, approve.

**R2. macOS: a `gh auth login` login is not found.** goalcup runs the agent's git and `gh` with the
participant's `GH_CONFIG_DIR` but a scratch `HOME` (`launch.rs`, `participant_git_env` and
`agent_base_env`). On macOS `gh` keeps its token in the login keychain by default, and with a
scratch `HOME` the keychain is not on the search list: `env -i HOME=/tmp/x GH_CONFIG_DIR=~/.config/gh
gh auth status` says "The token in default is invalid." So a Mac whose only GitHub login is
`gh auth login` (HTTPS, git helper `!gh auth git-credential`, exactly what the guide asks for) gets
`No GitHub login found on this computer`, and the guide's fix cannot help. Host runs passed tonight
only while SSH through 1Password worked. The keychain search list is the likely cause (inferred,
not traced); the symptom is shown above. Headless Linux is not affected: in the Docker B6 cell `gh`
stored its token in `hosts.yml` and goalcup's push test passed. A Linux desktop keyring (libsecret)
was not tested. Reproduce: a Mac with `gh auth login` and no usable SSH key
(here: 1Password locked), `goalcup start --harness codex-cli --test --repo <user>/<new empty repo>`.

## Results

PASS = what "must be true" says held. PARTIAL = part held, part could not be checked or differed.

| # | Result | Evidence |
|---|---|---|
| A1 | PASS (note) | Fresh config dir: "Goal Cup setup. 4 steps.", login inline, no GitHub question, no token. Note N1. |
| A2 | PASS | `goalcup prep`: same 4 steps, all `[ok]`, nothing asked. |
| A3 | PASS | Second and third setup asked nothing; keys file inode and mtime unchanged, "Open it now?" not asked again; the saved work root is kept with no flags. |
| A4 | PASS | `<work root>/.env.local`: comments only, mode 0600; only `prep.rs` touches it. Note N2. |
| A5 | PASS | "Installed: claude-code-cli, codex-cli", then "Also supported: amp-cli, hermes-cli", then "Your own harness?" link. |
| A6 | NOT RUN | No harness API keys approved for this round. |
| B1 | FAIL (R1) | Link correct (`github.com/new?name=…&owner=nembal&visibility=public`); after create and Enter the check passed and the approval came after it; then R1. Run `r6fb7813794356832` abandoned. |
| B2 | PASS | Docker cell lx-r6b: the kit made `nembal/goalcup-kit-lx-r6b-48f901`, `--repo` given, nothing asked, approval after "public, empty, and your agent's git can push to it". |
| B3 | PASS | README repo: "Create a new repository with no README, license or .gitignore, then run goalcup start again … Nothing was created." No device request, no run. |
| B4 | PASS | Private repo: treated as missing (link shown), then "GitHub does not show … it is not created yet, or it is private … Nothing was created." |
| B5 | PASS (note) | Archived (read-only) public empty repo: push test refused before approval. Note N3. |
| B6 | FAIL (R1) | The GitHub part held, the start did not. Docker, no gh login, no SSH key: guide shown ("No GitHub login found … Run: gh auth login … Press Enter when done"); after `gh auth login` (~90 s) and Enter the start went on to the push test and approval, then R1. Run `r12299b1e8db50e6f` abandoned. On the Mac host this case cannot be set up (OpenSSH reads `~/.ssh/config` from the real home). |
| B7 | NOT RUN | Cannot be set up on this Mac (no github.com keychain item; see B6). R2 is the related finding. |
| B8 | FAIL (R1) | SSH key held by the 1Password agent (no passphrase prompt reaches goalcup): two starts failed on R1 (`r93abb36bd5ba5d7c`, `r444616befe61a6b3`); a third, kept awake, chose the SSH remote (`git@github.com:…`), then the clone failed: `sign_and_send_pubkey: signing failed … agent refused operation` (1Password stopped signing from about 07:30, probably locked). SSH push itself worked earlier in the C2 run, not in B8. With a passphrase: NOT RUN (needs `~/.ssh` or account key changes). |
| B9 | PASS | Second run suggests `Name it [gc-r6-fullflow-1c81-2]:`; the used repo is refused ("is not empty … Nothing was created."); no run row. |
| B10 | PASS | Ctrl-C at `Name it:`, at the link's "press Enter" and at the approval wait: "interrupted before the run began, so nothing is left behind" / "…while it waited for your approval. Nothing was created, and no run was opened." No run, no clone, no harness. One pending device request is left (no run). |
| B11 | NOT RUN | The test build (`GOALCUP_TEST_GITHUB_API`) is held by Little Snitch; the stand-in API is ready (`ratelimit_api.py`). |
| C1 | PASS | Host (`rc6035e85c21a999d`) and Docker (`rfef5acf53c998ab1`, row `codex-cli linux-arm64 PASS`): asked to add a remote and push, the agent got `could not read Username`, `Permission denied (publickey)` / `Host key verification failed`, and `gh`: not logged in. Both repos stayed empty (GitHub commits API: 409 "Git Repository is empty", as the testers reported). DB: practice runs, 3 heartbeats each, entries and chunks equal to the local chain, sealed within a minute after the deadline; check-copy PASS. Note N4. |
| C2 | PASS | `rc4ffaea3c1a718cf`: event `test-staging`, `repo_url` the run's repo; heartbeats 1→2→3 during the run; chunks 13→15→16; agent commits `98631b5`, `3d10f1e` on GitHub. |
| C3 | PARTIAL | Start terminal: "harness termination: Confirmed", "uploaded: 3362732 bytes, acknowledged, and the seal.", "Upload done."; entry sealed, offset equals chunk sum, check-copy PASS. But the run row stays `open` with `final_commit` empty until the test run's 168 h end (31 Oct 07:09); `status` reports the seal as a PROBLEM (F1). |
| C4 | PARTIAL | The agent used the key (it sourced `.env.local` in the shell and wrote the length, 25); `.env.local` is in none of the 3 commit trees; the value appears 0 times in the chain (raw and every decoded `data_b64`). Redaction itself was not exercised: the agent never printed the value, so it never entered the model traffic. A rerun should make the agent echo the value once. |
| C5 | PASS | `goalcup status` during the run: "recording", "Codex: running", "Upload: working…", "Problems: none". |
| D1 | PASS | Codex with session writing turned off in the run's own home (`[experimental_thread_store] type = "in_memory"`, restored after): after ~10.5 min, shipped thresholds, `status`: "PROBLEM: Goal Cup has recorded 30 requests to the model service but no Codex session file yet. … Check that Codex is the supported version …". Run `rb212354005f08dcc`. Note N5. |
| D2 | PASS | Claude Code, `r9b40785d69b8c573`: `continue` printed "Your run goes on. No approval is needed…", Claude resumed the conversation; after `/exit`: "Claude Code has exited … To go on, run: goalcup continue r9b40785d69b8c573"; DB same run, segments 0, 1, 2. |
| D3 | PASS (note) | Clone deleted, `continue`: "Put the run's folder back at … and run goalcup continue again. … Nothing was changed." exit 1; no new part, no new run. Note F3. |
| D4 | PARTIAL | Docker network cut 240 s, `rddad873c0df83b8c`: recording went on (`gaps: 0`), one heartbeat lost during the cut and not retried; the upload was already waiting on its normal 30-minute timer before the cut and caught up at the seal; server next_seq/offset/head equal the local chain, chunks contiguous, check-copy PASS, pushes landed. The agent was idle during the cut. The kit row FAILs on F2. |
| D5 | PASS (brief wrong) | Window closed: Claude ended, the orphaned `goalcup start` sealed the part, uploaded and exited; nothing kept recording. New terminal `status`: "Claude Code: exited" with the `continue` PROBLEM and Next lines. This matches OPERATING case 7, not the brief ("daemon keeps recording"). |
| D6 | PASS | `goalcup seal` asked "Type yes to stop it."; "no" stopped nothing and it kept recording; `--yes`: "Codex was stopped, and this part of your run is sealed."; upload finished. |
| D7 | PASS (observation) | Repo private for 5 min mid-run: the player saw nothing (terminal normal, "Problems: none"); the agent's force-push landed while private. The judged commit cannot be checked until 31 Oct (see C3). |
| D8 | PENDING | Force-push done (GitHub: `00577eb -> 3d10f1e`); `final_force_push`/`flag_reason` are set at the run's end. Recheck `rc4ffaea3c1a718cf` after 2026-10-31 07:10. |
| D9 | PASS (note) | Shipped build, real thresholds, practice run `r1c08452aa6cf29f2`: `artifact_root` and `workspace_parent` in `daemon.toml` moved to an 8 GiB APFS image (restored after). Filled to about 590 MiB free: `disk_paused: true` within about 15 s (tester's state.json reads), terminal "The disk is almost full, so your agent is paused. … it resumes by itself.", `status` PROBLEM line, codex processes in state `T`. Filler deleted: resumed within about 12 s (host.log: "disk: 4.8 GiB is free again, so your agent was resumed."), the agent finished; the small image kept a "only 4.8 GiB of disk is free" low-disk line, as expected; run never stopped for the disk; integrity exit 0, check-copy PASS. Server side not reached: F8. |
| D10 | PASS (notes) | Second start while Claude recorded: "Your run is already going in another terminal. … Nothing was created." exit 1; the first run untouched. Notes F4, F5. |
| install.sh | PASS (note) | Against a local `file://` copy of a 0.6.1 release (`GOALCUP_RELEASE_BASE`) into a scratch prefix, never `~/.local`. During the recording run: "A Goal Cup run is recording on this computer. Update after it ends…", exit 1, prefix unchanged. After the seal: installed, exit 0. Note N6. |

Docker build: `linux-matrix.sh build codex-cli`, registry copied in with `docker cp`, built offline
in 42.7 s, exit 0.

## Other findings (not blockers)

- **F1. `status` after a chosen seal says PROBLEM.** After `goalcup seal --yes` (or a deadline-less
  test seal): "PROBLEM: Codex has exited (ended by signal 15). Your run is still open … run goalcup
  continue"; "sealed" appears only in Details ("capture: recording — run sealed"). `status --json`
  says `delivery: local_only`, `anchor: local_only` for a fully uploaded part.
- **F2. Kit: a `--test` matrix row can never PASS.** `matrix.sh` seals the part (`seal --yes <entry>`)
  but `compare_copy` then needs the server's run `status: sealed`; a test run stays `open` until its
  168 h end. Every Docker `--test` cell fails `status is sealed: server='open' local='sealed'`.
- **F3. After `clone_missing`, `status` still says "Next: run goalcup continue"**, which refuses;
  the refusal also suggests sealing a part already sealed and uploaded.
- **F4. D10 refuses only after the browser approval**, so an approval is spent for nothing.
- **F5. D10's refusal says both** "what this start made was removed" and "Nothing was created."
- **F6. A test run cannot be ended early** by any client command; every test run tonight stays open
  until 31 Oct (delete with `delete-test-runs.sh`).
- **F7. A heartbeat lost during a network cut is not retried** until the next 5-minute beat, and the
  upload catches up only on the 30-minute timer or the seal.
- **F8. A disk pause shorter than one heartbeat never reaches the server.** The pause travels only
  on the 5-minute heartbeat; D9's 2-minute pause fell between two, so `disk_paused_at` and
  `disk_paused_at` stayed empty (likely cause; it fits the beats at 07:39:03 and 07:44:02). Practice
  runs get no notices at all, so the disk email needs a test run held full for over 5 minutes.
- **F9. The hand-started cases needed workarounds for R1**: a hand-started daemon without
  `--exit-when-idle` (D1, D9), or `goalcup entries` / `goalcup status` every 4 to 8 s from a second
  shell (the C2 run, the Claude Code run of D2, D3, D5, D10). One tester found `status` did not keep
  the daemon up; `entries` and the hand-started daemon always did.
- **N1.** `goalcup setup` has no `--server`; its inline login goes to production (left one pending
  production login request, never approved). For staging: `goalcup login --server …` first.
- **N2.** `goalcup setup --json` ("asks nothing and opens nothing") also creates the keys file in the
  default work root.
- **N3.** B5's refusal blames the login ("Your git login is not one the agent can use. Easiest: run
  gh auth login") and drops GitHub's "archived" reason.
- **N4.** Practice isolation holds for git (`-F /dev/null` SSH command, empty credential helper,
  `GIT_CONFIG_NOSYSTEM=1`), but plain `ssh` from the agent still reads the real `~/.ssh/config` (D69,
  known). A practice start also prints "Cloning your repository…".
- **N5.** `goalcup-verify --research-json` reports `cross_leg native_model_activity 0, state
  "unchecked"` for the D1 run: the verifier does not flag the missing session file itself.
- **N6.** The first install after the seal printed "Could not tell whether a Goal Cup run is
  recording…" (INSTALL_UNSURE) once; the next one was clear. Not reproduced.
- **N7.** Each run leaves its 3.2 GB reserve file in `<config>-artifacts/reserve/` after the seal.
- **N8.** In a Docker cell by hand, `goalcup seal` needs `GOALCUP_CONNECTORS_DIR`.

## Setup changes made for this round

- Synthetic members `...0c0de1` and `...0c0de2` added to `goal_cup_tracer.test_operators` (owner
  approved). They were only in `test_members`, so no test run could open with the key.
- New host cells `~/goalcup-r6/h1`, `h2`; Docker volumes `goalcup-linux-matrix-codex-cli-lx-r6`,
  `-lx-r6b` (Codex and `gh` signed in on each volume; the kit gives a practice run no git login, so
  C1 ran with "git login: none"). Old cells `mac-codex`, `mac-claude` reused, with their
  work roots moved out of `~/goalcup-work`.

## Cleanup left

- Test runs (all `test-staging` or practice): `sh server/worker/scripts/delete-test-runs.sh`.
- Throwaway repos (all `nembal/gc-r6-*` and `nembal/goalcup-kit-lx-r6b-48f901`), listed in the PR.
