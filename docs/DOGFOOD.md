# Dogfood checklist — the first real run on a real machine

> **V1 checklist (hook-leg era, 2026-10-02).** On `m2/wire` real-harness rows are recorded in
> [docs/qualification/](qualification/README.md); this file describes the V1 path on `v1/trunk`.

The build proves itself with fake harnesses. These are the things only a real harness on a
real machine can prove, in the order to try them. Each item says what to run, what to look
for, and where the build is known to be guessing. Do it once per connector you intend to
hand to a tester; one connector is enough for the first pass.

Setup is `README.md` → "Installing the four binaries" and "A development run, end to end".
Use `--dev` throughout (a local pair, no grant). Keep the daemon's stdout visible.

## 1. Claude Code CLI (the one every test used)

**Before the first run, once per machine: log in outside the tracker.** Every entry gets a
FRESH `CLAUDE_CONFIG_DIR` (`{harness_home}`), and Claude Code keeps its first-run state —
`hasCompletedOnboarding`, `theme`, `oauthAccount` — in `<config dir>/.claude.json`. A home
with none of them shows the theme picker and then "Select login method", which no unattended
run can answer. `goalcup start` copies those keys out of your own `~/.claude.json` before it
spawns (`[home_seed]` in the connector manifest) and prints one line saying so; it READS that
file and never writes it. If you have never run `claude` on this machine, `goalcup start`
refuses and tells you to. **Do not type credentials into a tracked session.**

**Two more first-run prompts, seeded the same way** (dogfood fix 6, 2026-09-08). Past the
theme picker and the login wall a fresh `CLAUDE_CONFIG_DIR` also shows **"Try the new
fullscreen renderer?"** and **"Claude in Chrome extension detected …"**. The manifest's key
list now carries your answers to both — `fullscreenUpsellSeenCount`,
`hasCompletedClaudeInChromeOnboarding`, `claudeInChromeDefaultEnabled`,
`cachedChromeExtensionInstalled`. The Chrome prompt is silenced by any recorded answer; the
fullscreen prompt only once your own count has reached three, so below that it still appears
and answering it is harmless. The Chrome one is the one to watch for: on the run that found
it, the modal took the keystrokes of a typed turn, **no message was delivered at all**, and
the harness exited 0 with the transcript unchanged. Note that `claudeInChromeDefaultEnabled`
is also the feature switch: if you answered "Yes, use my browser" on your own machine, your
tracked run has browser tools too. That is your call — the tracker copies your settings, never
decides them, and records the tool calls they lead to. If a prompt you expected silent still
appears, answer it and report it — the keys are copied only where your own file has them, and
a participant who has never been asked has none.

**And the trace says which of your settings it carried** (fix 8, 2026-09-08). Every launch —
the start and each resume — chains a `home_seeded` record, which `goalcup-verify` reports as
`carried_settings` and judges not at all: the connector, the manifest's spelling of the file
read, and each copied key with its value for a boolean, a number or a string of at most 64
bytes and BY NAME ALONE for anything longer or structured (so `claudeInChromeDefaultEnabled:
true` is in it and `oauthAccount` is only named); a copied file is a name and a length, with
no digest — a digest of your credential would identify it across every trace it appears in.
The one entry the tracker DECIDES rather than copies is on a list of its own, `written`: this
run's empty workspace, marked trusted, so the harness does not stop at its safety question.

**Esc once does not clear a non-empty input; a driver must Esc twice** ("Esc again to
clear"). A single Esc leaves the pending text in place, so anything typed next is appended
to it and Enter submits the pair.

**And the credential itself, which those keys are not** (measured 2026-09-07). With the keys
copied, Claude Code 2.1.259 comes up in the tracked workspace with no theme picker, no login
screen and no trust question — and answers the first prompt with `Not logged in · Run
/login`. On macOS the token is in the login keychain, and a run under a fresh
`CLAUDE_CONFIG_DIR` does not inherit it. Two ways round it; pick one before the first run and
write down which:

- `/login` once inside the launched Claude Code, per entry — the login happens in the
  harness, not in the tracker; or
- `export ANTHROPIC_API_KEY=…` in the shell that runs `goalcup start`, so the launched
  harness inherits it.

- `goalcup doctor --harness claude-code-cli` — every line Pass or Warn; note the harness
  version it probed. Without `--harness` the report covers every shipped connector and exits
  1 if any of them failed, which includes one you are not running.
- `goalcup start --dev --harness claude-code-cli`, type one real prompt, let it answer,
  then quit the harness normally.
  - The first line after the launch banner should be the seeding line, naming your
    `~/.claude.json` and the per-run home. **No theme picker, no login screen, and no
    "Quick safety check: Is this a project you created or one you trust?"** — that third
    one is asked per project, its default answer is `No, exit`, and the same step marks
    this run's own empty workspace trusted in the per-run file so it never appears.
- `goalcup status` while it runs: `arming` reached `Started`, `posts_dropped: 0`,
  `harness_version` is the real version, not `unknown`. The daemon does not probe it — the
  CLI's own preflight measures it and sends it on `WorkspaceOpts` — so `unknown` here means
  the probe found nothing, and a run whose `doctor` printed a number and whose status says
  `unknown` is the D5 bug coming back.
- `goalcup seal` → run the printed `goalcup-verify --dev …` line → overall `pass`, empty queue,
  human messages ≥ 1.
- `{artifact_root}/raw-bodies` is created by CLAUDE CODE on its first raw body, not by the
  daemon at start. An empty artifact root before the first turn is expected; an empty one
  after several turns is a finding.
- `goalcup start --print-launch` prints the line and abandons the entry it rendered it from.
  `goalcup entries` afterwards must not list it. The **workspace stays** — that is the
  tracker refusing to delete a directory you may have worked in, and the line it prints says
  so; a line claiming the workspace was removed is dogfood C7 coming back.
- A run you abandon without ever starting the harness still seals, and `goalcup seal` warns
  that it will. The verify line then exits **2** with a `no_session` Fail in the queue — a
  verdict you can file, not an unreadable trace.

## 2. OpenCode (the TUI shape, now measured)
The launch and resume recipes moved from headless `serve` to the TUI with
`--port <reserved> --hostname 127.0.0.1` because the M0 spike could not drive a TUI.

**2026-09-07: the shape HELD, live, end to end.** OpenCode 1.18.27 came up under a pty on the
reserved port, took a typed prompt, wrote and ran `hello.py`, resumed with the BOUND session
id and continued the same conversation, and `goalcup seal` terminated the open TUI. 281 chain
rows across both legs. No login: the connector's default provider needs none, which makes
this the one connector a tester can run with nothing set up.

**2026-09-07, second run, after the fixes: `goalcup-verify --dev` exits 0 with an empty
review queue.** Two human messages (the goal and `keep going`), `hello.py` attributed to the
`write` call that made it, non-zero token totals, V1-V13 all Pass, and the one Flag under
strict verification is `placeholder_server_key`, which is what a development key is. That
trace is committed as `crates/goalcup-verifier/tests/traces/opencode-live/` — the first live
capture this repository has, and one of the two fixtures that measure the capture rather than
the reader. The other is `crates/goalcup-verifier/tests/traces/codex-live/`, the Codex run that
dogfood fix 7 made pass.

**It has been re-recorded once, and dogfood fix 4 is why.** The trace carries its own copy of
the connector manifest it was judged by, because the start receipt binds that copy's digest;
fix 4 deleted a field from that manifest (`arming.plugin_loads_per_launch`) and from the
schema, so that build could no longer parse the fix-2 copy. The bytes were left untouched —
editing an export is how a trace stops being one — and the run was made again under the fixed
connector instead (entry `e954cb9ad8ff2e86c`, recorded on `3dd4c57`). Those are the bytes
committed today, and `tests/end_to_end.rs::the_live_opencode_run_verifies` runs again. The
same rule holds for the next change to `connectors/opencode/manifest.toml`: re-record the run,
do not edit the export.

The first run produced four defects, all fixed since (dogfood O1-O3, O7): the parser read tool
arguments from the leg-1 part row, which OpenCode inserts EMPTY and upserts later, so every
written file was `unexplained_seal_file`; `.opencode/.gitignore` and `.opencode/package.json`
were not excluded; the lock DIRECTORY the harness creates at startup refused every resume;
and V9 read the two `plugin.loaded`s that run emitted per launch as a re-arm (fix 4 finished
that one: V9 counts LAUNCHES, from the distinct shim ids the tracker's plugin stamps on its
posts). If the run below fails differently, the manifest comment in
`connectors/opencode/manifest.toml` says what is still assumed.
- `goalcup start --dev --harness opencode` — the TUI must come up and be usable; the daemon
  must see leg-1 rows (`status` shows records growing) and leg-2 posts.
- Quit the TUI; `goalcup resume` — the relaunch carries `--session <id>` and the daemon's
  BOUND session id (not the minted one); the same conversation continues.
- `goalcup seal` with the TUI still open — the daemon terminates it (dispose-on-SIGTERM for a
  TUI is unmeasured); the verify line passes.
- If any of this fails, the manifest comment in `connectors/opencode/manifest.toml` says what
  was assumed; fix the recipe there and regenerate the pins (`scripts/pin-manifests.sh`).
- **Check `tracked_sessions` in the sealed `trace.toml` names a `ses_…`** and not a `msg_…`.
  Two runs on 2026-09-07 bound a MESSAGE id, because the connector's `session_id_field` was
  `data.info.id` — the id of whatever the event is about, which is the session on a
  `session.created.1` and the message on the `message.updated.1` that lands a few
  milliseconds later. It is `data.sessionID` now (dogfood O14). A run that binds a message
  reports a second root thread and leaves the file the agent wrote unexplained, and nothing
  downstream recovers.

## 3. Codex CLI
**The sign-in, first.** `CODEX_HOME` is the per-run home, so a fresh entry has no
`auth.json` and Codex opens on its sign-in screen — which is where the first live attempt
stopped, on 2026-09-07. `goalcup start` now copies that one file whole out of your own
`~/.codex` (0600, never written back), and says so on one line. If you would rather it did
not, sign in against the entry's own home instead: `CODEX_HOME=<the entry's harness home>
codex login`, before the start. A file the per-run home already holds is never overwritten.
**Measured on the 2026-09-07 rerun**: the start printed its seeding line, the copy landed in
the entry's own home at `-rw------- 4182`, and `~/.codex/auth.json`'s mtime was the same
before and after (`1788293130`) — read, never written. What is **still not verified live** is
the pair below: the trust question and the update modal both sit past the sign-in screen an
uncredentialed home stops at, so what the seeded keys do to them is read out of Codex's own
binary and not yet watched happening.

**The directory-trust question, seeded away.** Codex asks "Do you trust the contents of this
directory?" for the workspace and gates hook loading on the answer, which `--dangerously-
bypass-hook-trust` does not suppress: unanswered, no `SessionStart` fires and the run dies at
the 300 s arming gate — one entry was spent that way on 2026-09-07. Answering it writes
`[projects."<workspace>"] trust_level = "trusted"` into `$CODEX_HOME/config.toml`, so the
tracker writes that file first, into this run's own home. **No question should appear.** If
one does, say so: the seeded keys are what Codex 0.153.0 wrote and nothing else was guessed.

**And the "Update available!" modal, whose default answer runs `npm install -g`.** It is a
selector — `› 1. Update now (runs npm install -g @openai/codex)`, then `2. Skip`, then
`3. Skip until next version` — and Enter takes the first. On 2026-09-07 it fired inside a
tracked run and took the harness down with it. The same per-run `config.toml` carries
`check_for_update_on_startup = false`, which is Codex's own key for the check: with it, the
home never gets a `version.json` and the modal never appears. **If it appears anyway, choose
`2. Skip` — never `1` — and report it.** A tracked run must not replace its own pinned binary
mid-run, and the version `goalcup doctor` printed is the version the trace claims.

- `goalcup start --dev --harness codex-cli`, one prompt, quit.
- **Check `$CODEX_HOME/config.toml` after the run.** The tracker writes it ONCE — only when
  the home does not already hold one — and then never watches it, never restores it and
  chains nothing about it. It is Codex's own file: it writes `[tui.model_availability_nux]`
  there on every launch, keyed by the model slug the account resolves, and a watched copy
  would have been restored and re-chained once per launch, failing V9 on every honest run.
  So expect the two seeded keys plus whatever Codex added, and **expect no
  `config_deviation` for this file at all**. One appearing is the finding. The hooks are
  elsewhere and are still guarded — see the next line.
- **Delete `.codex/hooks.json` mid-run and check it comes back.** That file and the shim
  beside it are watched, restored and hash-pinned, which is what the unwatched `config.toml`
  costs nothing: the removal must chain a `config_deviation` with `arming_lost`, put the
  file back, and Fail V9 as `unattributed_config_change`.
- **Check `tracked_sessions` in the sealed `trace.toml` names the THREAD** — the same uuid
  the rollout filename ends with, and the one every hook post sends as `payload.session_id`
  — and not a `msg_…`. The rerun on 2026-09-07 bound a turn, because the connector's
  `session_id_field` was `payload.id`: that is the thread's id on the opening `session_meta`
  and the ITEM's id on every `response_item` after it, and the arming gate keeps the last one
  it saw. It is `payload.session_id` now (dogfood C1, the codex twin of O14). A run that binds
  a turn cannot be resumed at all — `codex resume msg_…` answers "No saved session found" —
  and reports the real thread as a second root.
- `goalcup resume` twice: once normally — **it must go ahead with nothing to remove by
  hand** — and once with the harness still open in another terminal, which must refuse with
  exit 4, name the lock file and the holder, and never print a `kill` command. The lock is
  the per-thread file `$CODEX_HOME/thread-writer-locks/<thread id>.lock`, held while the
  thread is open and gone after a clean quit. Neither the directory nor the
  `.coordination.lock` inside it is a lock: the harness makes both at startup and leaves
  both behind, and reading either as one refused every resume of a run that had ended
  properly (dogfood C4, and O3 before it for OpenCode). On a FIRST resume this machine's
  record still names the id the daemon minted, so the refusal you see comes from the daemon
  and names the path rather than the pid.

## 4. Things a tester will do to you (docs/CHEAT-CHECKLIST.md)
Try at least: edit a chained transcript line; start a second harness session in the workspace;
`kill -9` the daemon mid-run and restart it; change the connector's app-wide file mid-run.
Each has a stated expected outcome in that file.

## 5. What to write down
Real harness versions probed; the `harness_version` `goalcup status` showed; any
`posts_dropped` above zero; which of the two Claude Code login routes you took; and the
verdict JSON of each sealed run (keep the trace directories — they are the first real
fixtures, and `crates/goalcup-verifier/tests/traces/opencode-live/` and
`crates/goalcup-verifier/tests/traces/codex-live/` are two of them).

Two surfaces worth a look after the seal, both fixed on 2026-09-07: `goalcup status <entry>`
on a SEALED run reads `health: sealed` with the moment the chain closed and no countdown, and
it still answers after the daemon has been restarted — it falls back to the signed document
in `<config_dir>/cache/`, which it verifies against the same pinned run key. A run that never
started seals, says "sealed, never started — nothing to upload", and leaves `goalcup
entries`.
