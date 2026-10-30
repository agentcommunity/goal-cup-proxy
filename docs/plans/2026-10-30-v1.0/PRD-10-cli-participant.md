# PRD-10: CLI and participant experience

Id: PRD-10. Version: goalcup 1.0. Date: 30 October 2026. Revision 2: follows PRD-00 and PRD-01
revision 2, PRD-02 to PRD-06 revision 2 and the three reviews (section "Review answers" at the end).
Revision 3: the alignment pass against OWNER-DECISIONS.md (section "Alignment answers" at the end).
Revision 4: ledger items R3a, R3b and T4 (30 Oct): `goalcup recover`, the null-mode line, pad proofs.
Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md) (binding).

## Goal

Eight commands, about 80 messages, and one end screen that says exactly what was written, what was
lost, how long recording was off and what is still on this computer. Nothing on screen is ever
wider than what the record holds.

## Points served

6 (lead: tell the participant plainly), 3 (no start refusal that no point needs; the sleep lock and
setup never block), 5 (the upload lines and `goalcup upload`), 7 (setup and start on macOS, Linux and
WSL2, the way people run their harness), 1 and 2 (the "no model request written", "session files",
"passed through unread" and "local model" lines catch a harness that dodges the proxy), 4 (the token
stays in its file; the push block never holds it; `goalcup.log` takes fixed text only).

## Owner decisions it carries out

- Counts: the participant, status, heartbeat and server see only written counts and off time
  (BRIEF3, ledger R6). Run 1 printed "Recording works: 1736 model call(s)" for 842 written (run
  report, finding 4). That line is gone.
- Version is **1.0** (owner, 29 Oct night; "version 1 sure" in the relayed request).
- Answer 6: no need to print the `goalcup mcp` setup lines. In 1.0 the command goes too: it serves
  no point, and everything in the after-Edition-01 list that serves no point moves into this build
  (BRIEF3, ledger L6).
- Disk full: no special handling. Every disk message, the disk start refusal and the disk pause go.
- One system for every harness, no patchwork (ledger H1 to H5). The owner asked whether a gateway is
  "still a httpsproxy just different api endpoint". Yes: a gateway is one more host on the one list,
  and the participant sees it exactly like a direct key. A dodger gets the same pipeline with an honest
  line and its tier, never a false "recording works" (PRD-00, "One system for every harness").
- Hosts the participant registers are on the one host list (ledger H2). This PRD gives them the
  command to register one.
- AGENTS.md hard rules stand: records and never manages the agent, D66 `goalcup time-left` (pull
  only), never write into `~/.claude` or any app-wide config, no harness names in core code.
- Push block (run 1 finding 8) stays a participant step: revision 2's O10 settled "wording only", and
  RULES says the agent makes the repository itself (`docs/RULES.md:142-151`). This PRD makes it one
  paste instead of two (PRD-00 question 2, old 10, default yes).
- PRD-00 questions 2 (old 8 to 11) and 4 (old 15) carry this PRD's defaults: setup always writes the
  connector's auto-update switch (`update_off`) into goalcup's own harness home with no question
  (ledger H9), and the no-prompt settings there after one question; `goalcup login` is cut; one paste
  through `--prompt`; the harness is ended at the deadline and on `goalcup seal` (done by PRD-02's
  watchdog); file connectors are refused in competition runs (question 4).
- Masking is ledger M1 as written: `start` masks the `GH_TOKEN` exact value only. The wider
  exact-value masking of PRD-00 question 3 (old 12) is not built unless the owner says yes (default
  no).

## Today

| What | Where | Fact |
|---|---|---|
| Commands | `crates/goalcup-cli/src/main.rs:81-240` | 11 plus hidden `git-credential`: setup (alias `prep`), login, start, status, time-left, mcp, continue, upload, entries, seal, doctor |
| CLI source | `crates/goalcup-cli/src/commands/**`, `src/participant*`, `src/main.rs` (13,758 lines src and inline tests, `wc -l`) | Plus `src/account/` 1,046, `src/run/` 4,978, `push_lines.rs` 213, `ssh_keys.rs` 241, `host_log.rs` 253, `skill.rs` 162, `prep_file.rs` 88, `entries.rs` 208, `folder_gate.rs` 224 |
| Messages | appendix H section 7.1 | About 330 places put words in front of a participant; a lean client needs about 60 (app. H section 5) |
| The false finish line | `src/commands/start/finish.rs:437-444`, built at `:500-510` | "proxy admission: ..." then "Recording works: N model call(s)" counted at selection (`record_generation_request`) |
| Run 1's actual end screen | `final-pane.txt` on `origin/smart/tender-clarke-qupne3` | 8 technical lines (admission counters, entry, seal.json, anchor, termination, trace, verify) printed over the still-drawing harness, then "Recording works: 1736", then "Upload done." No word on the hour of lost calls |
| Seal report | `src/commands/seal.rs:399-455` | "entry X: finished locally; its terminal record is N and seal.json binds it", anchor, termination, trace, `verify:` on a folder that was pruned at the seal (run report finding 6) |
| `goalcup login` | `src/commands/login.rs:1-84` | Device approval, then stores only the server address and the account handle (`login.rs:24-27`). It grants nothing a run needs; each run is approved anyway |
| `goalcup mcp` lines | `src/commands/prep.rs:721-760`; `commands/mcp.rs` (474: 273 src, 201 tests) | Printed on every setup; four connector tables `[participant.time_left_tool]` |
| Push block | `src/participant/launch.rs:53-54`, `:96-118`, `:158-186`; `src/push_lines.rs:57-80` | 7 lines the participant pastes at the very start of the prompt. Run 1's prompt had none, so nothing was pushed (finding 8) |
| Resume on `continue` | `src/commands/start/phases.rs:296-361`, `:745-779`; `src/launch.rs:664-771` (`has_native_session`, `holds_session_file`) | Sniffs the harness's session stores per connector to choose resume or new session; D73 `resume_failed` opens a new session after a resume that exited at once |
| Sleep lock | `crates/goalcup-platform/src/awake.rs:93-106` (269 lines) | `caffeinate -i -s -w <pid>` on macOS, `systemd-inhibit --what=sleep:idle --mode=block` on Linux; none under WSL. Missing lock is a `PROBLEM:` line for the whole run (`src/run/mod.rs:562-565`, `participant/status.rs:511-513`); run 1 showed it from the start in the sandbox |
| Status problems | `src/participant/status.rs:319-344` | "the recorder restarted ... If this repeats, run goalcup status" (printed by `goalcup status`); "the recording has gaps"; never "recording is off" |
| Config dir | `src/config.rs:39`, `:71-89` | `$GOALCUP_CONFIG_DIR`, else `~/Library/Application Support/goalcup` on macOS, `$XDG_CONFIG_HOME/goalcup` or `~/.config/goalcup` elsewhere |
| Server | `src/server.rs:24` | `https://ingest.agentcommunity.org` |
| Version | `Cargo.toml:18`, `src/lib.rs:136` | `0.7.0`, `CLI_VERSION = env!("CARGO_PKG_VERSION")` |
| Contract words | `src/participant.rs:76`; `participant/approval.rs:45-61`; `participant/status.rs:10` | `Recording in chat mode`, `shows the code XXXX-XXXX.`, `PROBLEM:`; read by the matrix scripts and the pad |

## Design

### 1. The commands after the cuts

| Command | 1.0 | What it does |
|---|---|---|
| `goalcup setup [--harness ID] [--yes] [--record-host HOST]` | **keep, smaller** | Pick the harness, open it once in goalcup's own home to sign in, write the auto-update switch and the no-prompt settings there (section 7), check the machine. `--yes` (seam S15): no question, no harness opened; writes the connector's `update_off` and no-prompt values into the isolated home and prints that home's path (the pad and scripted installs). Without a terminal and without `--yes` it asks nothing, opens nothing, writes nothing and prints the steps: that replaces `doctor` and `--json`. `--record-host` adds a model host to the participant's list |
| `goalcup start [--harness ID] [--practice] [--test] [--repo NAME] [--prompt FILE] [--connector FILE] [--hosts FILE]` | **keep** | Checks, approval, open the run, then hand over to PRD-02's `watchdog::run`. `--harness` defaults to the one setup saved (app. H 7). `--prompt` is new (section 8). `--connector` and `--hosts` (seam S16) load a connector or a host list from a file; PRD-11 loads and checks them; practice and test runs only |
| `goalcup continue [RUN] [--connector FILE] [--hosts FILE]` | keep | Close a dead entry (`pipeline::close_dead_entry`), continue on the server, relaunch with the connector's resume arguments (section 3.1). The two file flags must match the run's first entry |
| `goalcup recover [RUN]` | **new** (ledger R3b) | When the watchdog and the recorder are both dead but the harness still runs: restart only the recorder on the same port with the same run certificate, so the running harness reaches the model service again and is recorded again. Never restarts the harness (section 3.2) |
| `goalcup status [PART] [--json]` | **keep, rewritten** | One plain block from `status.json`, `upload.json` and `server.json`. Lists the run's parts (replaces `entries`) |
| `goalcup seal [PART] [--yes]` | keep | Asks, then signals PRD-02: SIGUSR1 to the pid in `rec/watchdog.pid`, or SIGTERM to the recorder pid in `status.json` when no watchdog lives. On a dead entry: `close_dead_entry`, then the uploader. Refused inside the harness (D66, `seal_inside_harness`) |
| `goalcup upload [PART]` | keep | Dispatch to `upload::manual::run` (PRD-06 section 9) |
| `goalcup time-left` | keep (D66) | Prints the time left; the agent may run it (section 10) |
| `goalcup login` | **cut** | Grants nothing (`login.rs:24-27`); the per-run approval is the account tie (app. H 1). The server comes from `GOALCUP_SERVER` (section 2). PRD-00 question 2 (old 9) |
| `goalcup mcp` | **cut** | Owner answer 6, app. H 4. `goalcup time-left` and the goal-cup skill stay |
| `goalcup doctor` | **cut** | Its two remaining checks (connector loads, harness on PATH) are setup's harness step. Pins, daemon, decoders and SSH checks are cut with what they checked |
| `goalcup entries` | **cut** | `goalcup status` lists the parts |
| `git-credential` (hidden), `prep` alias, `setup --json`, `setup --dev`, `start --dev` | **cut** | Backward compatibility (app. F 8) or pins, which are cut (PRD-13). `--connector` and `--hosts` replace `--dev` |

Eight commands. Exit codes stay 0 done, 1 refused and fixable, 3 could not do its job
(`main.rs:49-60`); the sentence "never exits with 2 or 4" goes. `main.rs` keeps `Args::try_parse()`
and maps every parse error to exit 1, so clap's own exit 2 never shows. Every error is one line
`goalcup: <text>` on stderr (`main.rs:275`).

The hidden recorder subcommand is PRD-02's (watchdog section 8 step 4); `main.rs` only dispatches it
to `pipeline::run_recorder` and hides it from help.

**"Part" is the participant's word** for one `start` or one `continue` (an entry in PRD-01). The
compressed files on disk are "record files" on every screen, never "parts" (PRD-15 ask 1).

**File connectors in competition.** A real run (no `--practice`, no `--test`) with `--connector` or
`--hosts` is refused before anything is created: `--connector and --hosts are for practice and test
runs. A competition run uses the connectors built into goalcup.` (PRD-00 question 4, old 15,
default). The launch block of a run that uses them says `Connector from a file: <path> (not
verified).` and `Host list from a file: <path> (not verified).`, one line per file given.

### 2. Server, home and environment

| Name | Meaning |
|---|---|
| `GOALCUP_HOME` | Where runs live (`$GOALCUP_HOME/runs/<run>/<entry>/`, PRD-01 section 1). Replaces `GOALCUP_CONFIG_DIR` (no compatibility). Default unchanged: `~/Library/Application Support/goalcup` on macOS, `$XDG_CONFIG_HOME/goalcup` or `~/.config/goalcup` on Linux and WSL |
| `GOALCUP_SERVER` | The server. Unset: `https://ingest.agentcommunity.org`. A URL: that server (staging for every build run; `http://` only for a loopback address). `off`: a local-only run, nothing uploaded, parts kept (PRD-06 `keep`) |
| `GOALCUP_KEEP_PARTS=1` | Keep every record file (PRD-06 section 7). No flag: the pad and the acceptance runs set the variable. Passed to PRD-02 in the start facts (`keep`) |
| `GOALCUP_NO_BROWSER=1` | Print the approval address instead of opening a browser (kept, `main.rs:441-480`) |

Files PRD-10 writes, all mode 0600:

- `$GOALCUP_HOME/setup.json`: the saved harness id and the `update_off` and no-prompt values written
  (replaces `prep.json`, which no longer gates a start: app. F 4).
- `$GOALCUP_HOME/record-hosts.txt`: hosts the participant registered, one per line. PRD-11's `plan`
  reads it into the participant list of `HostList` (seam S3; `Env.goalcup_home` says where).
- Per run: `runs/<run>/server.json` (server, run, deadline, mode, repo, connector source, and per
  entry its id, harness version and whether it resumed) and the run credential in
  `runs/<run>/credential`, both written before the watchdog starts (PRD-06 interface, PRD-00 local
  files); `server.json` never holds the credential. Also `runs/<run>/first-message.txt` when `--prompt`
  was given (section 8). The local `server.json` is per run, never pruned; the fetch output of PRD-07
  is named `fetch.json` (seam S13), so the names never clash.
- Per entry: nothing. `goalcup.log` (section 12) is appended by the seam S11 writer; `status.json`,
  `upload.json`, `rec/watchdog.pid` and the PEM files belong to PRD-02, PRD-03 and PRD-06.

### 3. `goalcup start`, in order

1. **Read the connector**: `connector::load(id_or_path, hosts_file)`, then `connector::plan(&connector,
   &env, home)` (PRD-11 section 5, seam S3; there is no `PlanCtx`). `id_or_path` is the harness id or
   the `--connector` file, `hosts_file` the `--hosts` file. `Env` holds the participant's environment,
   the workspace, `resume`, the run start, `GOALCUP_HOME` (where `plan` reads `record-hosts.txt`) and
   the server host. From the `Connector`: display name, `first_screen`, `no_prompts`, `update_off`,
   `skills_dir`; from the plan: launch plan, home. The tier comes from `connectors/tiers.toml` (PRD-12 writes it; embedded at
   build time with `include_str!`). A `refusal` in the plan (a `refuse_if_set` hit) is shown with the
   file and key, never the value, and stops the start.
2. **Check the folder, and say every problem at once** (app. F 5). One refusal lists each problem with
   its fix, then "Nothing was created." Checks kept: folder empty except `.gitignore`, `.DS_Store` and
   `.env*` files (`docs/RULES.md:142-145`); every `.env` covered by `.gitignore`; not inside a Git
   work tree; not beside goalcup's own folders; a run with `--repo` (competition or `--test`) has a
   plain `GH_TOKEN=` line in `.env.local` (the push block's `sed` needs it, `launch.rs:181-182`); the
   `--prompt` file is outside the project folder (a prompt file inside it is named, with the fix
   "move it out of the folder"); WSL1 or a Windows drive under WSL2 (`src/folder_gate.rs:58-107`).
   Turned into one warning line each: synced folder (app. F 4), NFS or SMB
   (the entry lock is an `flock`). Cut: the btrfs, ZFS, tmpfs, overlayfs and FUSE list (overlayfs
   and tmpfs are what containers and sandboxes use; nothing in 1.0 needs more than `fdatasync`,
   `flock` and rename).
3. **Real run: the repository** (`src/commands/run_repo.rs:66-165`, kept): public, empty, checked
   without a token before the approval, so a full repo does not waste one.
4. **Approval** in the browser (device flow, copied from `src/account/device.rs`, 389 lines, through
   PRD-06's `ServerClient`). One approval per run. "shows the code XXXX-XXXX." stays word for word.
   Nothing before the launch times out sooner than the approval code lives (15 minutes): a 30 s idle
   exit before launch once abandoned 8 test runs (PRD-17 P14).
5. **Open the run** (`POST /v1/runs`, PRD-07, seam S5). Write `server.json` and `credential`. The start
   facts carry the deadline as the server's `seconds_left` at the moment of the answer, read on the
   boot clock, so a local clock set back a day cannot lengthen the run (PRD-17 S7; PRD-02 runs the
   timer on boot time).
6. **Mask values**: `mask::workspace_values(project)`, which in the default build gives the `GH_TOKEN`
   value only (ledger M1, PRD-05 section 9). `mask::env_values(&plan.env)` and
   `mask::harness_values(&plan.secret_values)` are called only after the owner's yes to PRD-00
   question 3 (old 12), in the second slice; then, when PRD-05's `skipped` is above 0, the start
   screen says `N key values were not added to masking.` The values go to the watchdog in the start
   facts. A `MaskError` that PRD-05 says refuses (a built-in shape) stops the start with fixed text
   before anything launches.
7. **Sleep lock** (section 9). Never refuses.
8. **Install the goal-cup skill** into the harness's own home at the connector's `skills_dir` (copied
   `src/skill.rs`, trimmed). Only under `{home}`: a `skills_dir` under `{workspace}` installs nothing
   and writes one `goalcup.log` line, because goalcup adds nothing to the project folder.
9. **Print the launch block** (section 8), add `GOALCUP_TIME_LEFT_FILE` to `plan.env` (section 10),
   then call `watchdog::run(plan, facts) -> ExitCode` (PRD-02, seam S8). From here PRD-02 does everything: bind the port, mint the CA, start the recorder, wait for
   `ready` (10 s, first boot only), run `plan.before` (stop the connector's helpers), spawn the
   harness, end it at the deadline, on `goalcup seal` and on the server's `stop` (SIGTERM to the
   harness tree, SIGKILL after 10 s), run `plan.after`, seal. PRD-10 does none of these. If the first
   boot fails, the harness is not started and PRD-10 words it: "Goal Cup could not start recording,
   so <harness> was not started. Run the command again. If it fails again, write to <support> and
   name run <id>." (app. H section 5, start refusals).
10. **After `sealed`** (the watchdog has given the terminal back, PRD-02 step 11): the end screen
    (section 5), then the upload wrap-up progress from `upload.json` once a second (PRD-06 section 9).
    Ctrl-C here ends only the watchdog; the recorder finishes the wrap-up alone.

Refusals cut from start (none serves a point; appendix H section 7.3): disk reserve
(`src/run/reserve.rs:289-299`), provider-host TCP probe (`phases.rs:107-140`; the no-requests line
catches an offline start), `prep.json` gate (`src/prep_file.rs:23-60`), `--practice` needing setup
(`server.rs:37`), pinned connector files (`phases.rs:95-103`; pins are cut), synced folder and Git
work tree under the run directory (`per_run_home.rs:299`, `:340`, `:356`; now warnings or gone with
the run home rework in PRD-11). Kept: `home_busy`, one run per isolated home at a time (PRD-17 HF4,
Codex refresh tokens rotate).

**Ending the harness** is today's behaviour (`participant/ending.rs` `TimeUp`, "was stopped"), now
done by PRD-02's watchdog with no proof machinery (PRD-00 question 2, old 11). PRD-10 only words it:
`Time is up: <harness> was stopped.` and, for a seal, `goalcup seal stopped <harness>.`

#### 3.1 `goalcup continue`

1. If the last entry has no seal and no live recorder: `pipeline::close_dead_entry(dir)` (PRD-02 9.4).
   `StillRecording` gives "This part is still recording. To end it, run: goalcup seal".
   Before that, if the last entry has no seal and its harness is still alive (section 4, harness
   liveness), `continue` refuses (exit 1): `<harness> from the last part is still running. To record
   it again, run: goalcup recover. To start a new part instead, end <harness> first.` Two harnesses
   on one isolated home at once is what `home_busy` exists to stop (PRD-17 HF4).
2. The same steps as `start` without the folder-empty and repository checks, and without a new
   approval when the run credential is valid. PRD-02 asks for the previous entry's port.
3. **Resume or new session** (replaces the session sniffing of `phases.rs:296-361` and
   `launch.rs:664-771`; PRD-13 left the call here). Resume with the connector's resume arguments when
   the run's earlier parts wrote at least one model request or one session file (sum of their sealed
   `status.json` `written`). Open a new session instead when the previous part was a resume and its
   harness exited within 30 s with a non-zero code (D73's `resume_failed`, copied). goalcup reads no
   harness session store: the rule uses written counts only, so no harness names and no format
   knowledge enter the CLI. A Claude Code session file is written at startup before any message
   (PRD-17 HF6), so a resume of an empty session may fail once; the `resume_failed` rule then opens a
   new one on the next `continue`.
4. If the harness version differs from the previous part's (in `server.json`): one line,
   `<harness> was updated from <a> to <b> since the last part.` (PRD-11's auto-update note).

#### 3.2 `goalcup recover` (ledger R3b)

For the one case the watchdog cannot handle itself: the watchdog died, then the recorder died too
(PRD-02 section 9.1, row "The watchdog, then later the recorder"), while the harness still runs. Nobody
holds the port, so the harness cannot reach the model service. `recover` brings back only the recorder.
It never restarts, ends or signals the harness, and it never takes the terminal.

**Checks, in order** (each refusal is one line, exit 1, nothing is started):

| Check | Refusal when it fails |
|---|---|
| A run with an open last entry exists (`RUN`, else the newest run in `GOALCUP_HOME`) | `no_run_yet`, kept |
| Not run inside the harness (the same test as `seal_inside_harness`): the recorder must not start with the agent's environment, whose proxy setting points at the dead port | `goalcup recover runs outside your agent. Run it in another terminal.` |
| The entry is not sealed (`status.json` `mode` is not `sealed`) and the run is not over (deadline from `server.json`) | Sealed: `This part has ended, so there is nothing to recover. To go on, run: goalcup continue <run>`. Over: `Time is up for this run. To close the part, run: goalcup seal` |
| No watchdog is alive (`rec/watchdog.pid`, liveness as in section 4) | `goalcup is still running for this part and restarts recording itself. See: goalcup status` |
| No recorder is alive (`status.json` `pid`) | Not a refusal: `Recording is on; there is nothing to recover.` (exit 0) |
| The harness is alive (`status.json` `harness` pid and start time, section 4) | `<harness> is not running, so there is nothing to keep recording. To go on, run: goalcup continue <run>` |

**What it does** once every check passes:

1. Rebuild the start facts the way `start` and `continue` do, from the run's own files: the part's
   connector and host list as named in `server.json` (a file given with `--connector` or `--hosts` is
   read again; a changed file is refused as on `continue`), the exact mask values again from
   `mask::workspace_values` (start step 6; nothing secret is stored for this), the harness pid and
   start time from `status.json` `harness`, and the run certificate from `run-ca.pem` plus its key
   file (0600, in the run folder, kept for the whole run: ledger R3b, PRD-02 owns it).
2. Call PRD-02's recover entry (`watchdog::recover(entry_dir, facts)`, ask N9). It binds the port in
   `status.json` `port`, which must be free or held by no live process (a port left by a dead
   recorder rebinds), starts the recorder alone, and waits for `ready` for at most 10 s, as on a
   first boot. The new boot's recovery writes the `recorder_down` gap for the time recording was down
   (PRD-02 section 9.2 step 4), so the dead time is a gap with a start and an end, never silent (R4).
   With no watchdog, the recorder runs as in PRD-02's "watchdog only" row: it watches the harness pid,
   runs its own deadline timer and seals with `harness_exit` or `deadline`.
3. Write one `goalcup.log` line `recover` (off since, port, outcome).

**What it prints.** On success (exit 0):

```
Recording is on again for part 2 of your run with Codex, on the same port.
It was off from 15:43:15 to 15:51:02 (7 min 47 s); this time is in the record as a gap.
Codex was not restarted. This part ends when Codex exits or at the deadline.
Check it any time with: goalcup status
```

Port taken by another program, or the key file missing or unreadable (exit 1): `goalcup cannot
reopen recording on port <port>: <the port is used by another program | the run certificate key is
missing>. Your agent cannot reach the model service until this part ends. To go on, run: goalcup
seal, then goalcup continue <run>`. No `ready` within 10 s (exit 3): `goalcup could not restart
recording. Your agent still cannot reach the model service. Run goalcup recover once more; if it
fails again, run goalcup seal, then goalcup continue <run>, and write to <support> naming run <id>.`
plus the "Details for support are in <log>" line.

`recover` holds no sleep lock (that lived in the dead watchdog), stops no helpers and runs no
`plan.after`. An email that tells a participant to run it is a later server-side follow-up (R3b), not
part of this build.

### 4. Status: `goalcup status`

It reads three files and never talks to the recorder: `status.json` (PRD-02 section 6, seam S4, every 1 s),
`upload.json` (PRD-06, seam S9) and `server.json`. It works the same while a run records, after it ends,
and after a crash. A pid is "alive" when `kill(pid, 0)` succeeds and the process is a `goalcup`
binary (Linux `/proc/<pid>/exe`, macOS `proc_pidpath`). The harness is "alive" when its pid in
`status.json` `harness` answers `kill(pid, 0)` and its start time still matches (pid reuse), the same
test PRD-02's recorder uses when no watchdog lives.

**Recording state**, decided in this order:

| Facts | Line |
|---|---|
| `mode: sealed` | `Recording: finished at 16:54. Nothing is recorded now.` |
| Recorder pid (`status.json` `pid`) not alive, watchdog pid (`rec/watchdog.pid`) alive | `Recording is restarting (since 15:43:15).` After 30 s: `Recording is off since 15:43:15. Your agent keeps working; this time will show as a gap.` |
| Recorder and watchdog both not alive, no seal, harness alive | `PROBLEM: Recording is off since 15:43:15: goalcup stopped, so your agent cannot reach the model service. Your agent is still running. Run: goalcup recover` |
| Recorder and watchdog both not alive, no seal, harness not alive | `PROBLEM: Recording is off since 15:43:15: goalcup is not running. To go on, run: goalcup continue <run>` |
| `mode: null` | `PROBLEM: Recording is off since 15:43:15: it stopped right after each restart, so goalcup lets your agent's traffic pass unrecorded for now. Your agent keeps working. goalcup tries again at most 5 minutes apart until the run ends. The server is told.` |
| Recorder alive, `updated` older than 10 s | `Recording is not answering since 15:43:15. Your agent is not slowed.` |
| `mode: losing` | `Recording is losing data now: 2 connections or files have a gap (12 MB so far). Your agent is not slowed. The end screen will say how much.` (`losing.streams_in_gap`, `losing.dropped_bytes`) |
| `mode: writer_slow` | `Recording is behind: nothing written for 14 s. Your agent is not slowed.` |
| `mode: recording` | `Recording: on.` |

"Off since T" uses `off.since` from `status.json` in null mode, else the `updated` time of the last
`status.json`. Times are local with the zone, as today (`ends_line`, `participant/status.rs:105-111`).
A normal restart takes under a second (PRD-02 section 8 step 8), so most kills show no state change
at all; they show in the "Off:" line of the block within one status tick of the next boot. The 30 s
step keeps "restarting" honest for a slow restart and says "off" after it.

**The block** (the same shape in every state):

```
Your competition run with Codex: recording.
  Ends: Sat 14 Nov, 10:12 CET (09:12 UTC), 95 h 10 min left.
  Codex: running.
  Recording: on.
  Written so far: 905 model requests (api.openai.com 812, chatgpt.com 93); 896 whole, 9 with missing parts.
  Session files: 43, 48 MB.
  Off: 1 time, 2 min 30 s in total.
  Upload: the server holds everything up to 14:02 (1 min ago).
  Parts: 2. Part 1 sealed and uploaded. Part 2 recording.
  Problems: none.
```

- "Written so far" is `written.model_requests` with `written.hosts`, and `written.req_whole` and
  `written.req_holes`, both shown whenever `req_holes` is above 0 (PRD-01 section 9). It is
  never an offered or seen count, and it is never called "calls": it is a request count (PRD-01
  section 9). While a stream is losing, the lost bytes are already in `losing.dropped_bytes`, so the
  line can say so before the GAP exists.
- "Session files" is `written.native_files` and `written.native_bytes`. The screen does not say whose
  files they are: no connector field says which harness keeps one file per subagent, and the recorder
  does not interpret which is which.
- Upload lines are PRD-06 section 10's drafts, used word for word, with "record files" for parts.
- `--json` prints `{"status":<status.json>,"upload":<upload.json>,"run":<server.json>}`;
  `server.json` never holds the credential (section 2). Nothing is signed (signing is cut). The installer's run guard reads `status.mode`.

**Problem and notice lines** (prefix `PROBLEM:` only where the participant must act or report; one
line per cause, never one per tick). Notes come from `status.json` `notes`.

| Cause (facts) | Line |
|---|---|
| Harness running; `last_request_at` older than 10 minutes and `last_native_at` newer (PRD-02, seam S4) | `PROBLEM: No model request was written for 10 minutes, but <harness>'s session files grew. It may not be using the recording proxy. Its session files are still recorded. If it uses a model service goalcup does not know, see goalcup setup --record-host.` |
| No model request at all 5 minutes after launch | `PROBLEM: No model request was written yet. If your agent is working, it may not use the recording proxy.` (H: `problem_no_model_calls`, kept) |
| Note `native_missing` (PRD-04, a `warn_missing` row with no file after 5 minutes) | `goalcup has found no <row name> file for <harness> yet. Its session may be stored somewhere goalcup does not read.` |
| Note `native_other_device` (PRD-04: `row`, `path`) | `goalcup does not read <path> for <harness>: it is on another disk. Session files there are not recorded.` |
| Note `long_tunnel`, `why: stream` (PRD-03 section 9) | `PROBLEM: <host> streamed data for 12 minutes and was not recorded. If it is a model service, run: goalcup setup --record-host <host>. goalcup records it from the next part: to start one now, run goalcup seal, then goalcup continue.` |
| Note `long_tunnel`, `why: upload` | `PROBLEM: <host> received 3 MB from your agent and was not recorded. If it is a model service, run: goalcup setup --record-host <host>. goalcup records it from the next part: to start one now, run goalcup seal, then goalcup continue.` |
| A listed host in `tunnel_hosts` (tunnelled with GAP `pinned`, `tls_refused`, `upstream_untrusted` or `protocol_other`), or note `ca_refused` | `PROBLEM: Not recorded: <host> refused the recorder (it checks its own certificates). Its traffic passed through unread.` For `upstream_untrusted`: `... <host>'s certificate is not one goalcup trusts (a company proxy?), so goalcup passed it through unread.` No `pinned_host` note exists; the line keys off the GAP reason in `tunnel_hosts` (PRD-01) |
| Note `parse_stopped` (PRD-03 section 5) | `Recording of one connection to <host> stopped at 14:10; its traffic still passes. Requests on it after that are not recorded.` (status is `losing` while the connection lives) |
| Note `local_model` (PRD-11) | `Calls to a model on this computer (<host:port>) are not recorded.` |
| A host in `written.hosts` of kind `vendor` (the shared list's kind, the same one the record's `conn` meta carries, PRD-03 section 12) | `<harness> runs its model calls on its vendor's servers. goalcup records what <harness> sends there, not the vendor's own model calls.` |
| `written.mask_failclosed_bytes` above 0 (PRD-05) | `goalcup hid 2 MB it could not check for secrets in time. Those bytes are stars in the record.` |
| Note `fd_limit` (PRD-02) | `Many connections are open at once (limit <n>). New ones wait a moment; none is refused.` |
| Upload behind (PRD-06 `state`) | One line, PRD-06 section 10 (app. F 9 merges five lines into this one) |
| Sealed `status.json` `after_seal.bytes` above 0 (PRD-02 section 11 step 6) | The after-seal line of section 5 rule 6 |
| `status.json` older than 2 minutes, no goalcup process and no harness alive, run still open (with the harness alive, the state line says `goalcup recover` instead) | `PROBLEM: goalcup has not updated this run since 13:40. The computer may have slept or restarted. To go on, run: goalcup continue <run>` (H: `problem_state_stale`, kept) |
| Agent exited, run open | `<harness> has exited (exit code 0). The run is still open until <time>. To go on, run: goalcup continue <run>` (H: `problem_agent_exited` without the disk branch) |

Lines from the launch plan, printed once at start and kept in status (no harness names in code; they
come from plan facts): when the plan sets `HOME` to the isolated home, `goalcup runs <harness> with
its own home folder, so your git and ssh settings are not visible to it.`; when it sets
`XDG_CONFIG_HOME`, `Tools that keep their login under ~/.config (for example gh) do not see it during
the run. GH_TOKEN is passed through.` (PRD-11 risks rows).

A line about a live loss on only some subagents (review A finding 6) is not built: telling that one
subagent's calls are missing while others are written needs per-call parsing of session files, which
the recorder never does (AGENTS.md "record raw, interpret later"). The verifier's `wire_keeps_up` does
it after the run (PRD-08); the two lines above are the live net.

Cut from status (appendix H): the "Details:" block (`commands/status.rs:582-700`), `this_computer`
vitals, every disk line, `problem_recorder_restarted` and `problem_recorder_lost` (both reworded into
the state lines above; the circular "run goalcup status" goes), the push section (the block lives in
`first-message.txt` or `push-lines.txt` and the launch block), the sleep `PROBLEM:` (section 9).

### 5. The end screen

Printed once the harness has left the terminal (after the watchdog restored it and the recorder sent
`sealed`), never over a running TUI as in run 1. The numbers come from `status.json` in
`mode: sealed`, whose `written` and `off` equal the seal event's `totals` (PRD-02 section 6), so the
screen and `goalcup-verify check` agree by construction (PRD-08 `counts_match_seal`).

```
Codex has exited (exit code 0).
Your run is still open until Sat 14 Nov, 09:12 UTC, 120 h 18 min left.
To go on, run: goalcup continue r632072599ee7b6fc (you can do it in another terminal now).

What this part recorded (on this computer):
  905 model requests written: api.openai.com 812, chatgpt.com 93.
  896 of them written whole, 9 with missing parts. 1.5 MB was not recorded.
  43 session files, 48 MB.
  Recording was off 2 times, 2 min 30 s in total.
  Not recorded, passed through unread: registry.npmjs.org (105 connections, 80 MB),
    ab.chatgpt.com (11, 24 MB), and 2 more hosts.
Upload: done. The server holds your whole record (95 MB). Removed here: 3 record files the server holds.
Details for support are in "/home/you/.config/goalcup/runs/r63.../e1ba.../goalcup.log"
```

Rules:

1. **Never "Recording works".** When there is no gap, no off time, no tunnel to a listed host, no
   fail-closed byte, no file cut at the seal and no byte after the seal (`after_seal.bytes` 0), the
   second line of the block is `Nothing was lost.`
   Otherwise each loss has its line, with numbers.
2. **Zero written requests** gives, instead of the first line: `No model request was written.
   <harness> may not use the recording proxy, or it made no call.` plus the tier line (section 6).
3. **Written whole, lost bytes and off time.** `req_whole` is the "written whole" number, shown beside
   `model_requests` whenever `req_holes` is above 0; "not recorded" is `lost_bytes` (PRD-01 section
   9). Off time is `off.ms` with the count `off.spans` (the sum of `recorder_down` gaps, PRD-01
   section 7). The screen prints only what `written`, `off`, the ruled notes and the sealed keys hold
   (PRD-00 R3-2, R3-4): no last-span times and no loss cause by reason, since no status key carries
   them and the CLI adds no counter (ledger L2). Every gap reason has plain words (table below), used
   where a reason is named: the listed-tunnel lines (`tunnel_hosts` `listed`) and the guide (PRD-15).
4. **Not recorded** lists `tunnel_hosts` by bytes, largest first, five at most on screen, the rest as
   "and N more hosts"; the whole list is in `goalcup.log`. A tunnel to a listed host (pinned,
   refused, untrusted upstream, other protocol) is never in the "and N more": it is always named on
   its own line, because it may have carried model calls.
5. **Files cut at the seal** (sealed `status.json` `cut_files`, from PRD-04's `NativeFinish`):
   `3 session files were still growing at the end; their last 2 MB were not read in time.`
6. **After the stop.** Bytes sent after the seal began are in GAP 15 `after_stop`, so they are in
   `lost_bytes` (rule 3); its connection count and hosts are in the record, not on screen. Bytes sent
   after the seal's last link are in the sealed `status.json` `after_seal {bytes, hosts}` (PRD-02
   section 11 step 6); when `bytes` is above 0, the end screen and `goalcup status` say: `Programs
   <harness> started sent 2 MB after this part ended (api.openai.com); that was not recorded.`
   **Outlived processes** (note `outlived`, PRD-02 section 11):
   `3 programs <harness> started were still running at the end (node, python3). goalcup did not stop
   them.`
7. **A recovered part** (seal reason `recovered`, shown by `goalcup continue`, `seal` and `upload`):
   `This part ended without goalcup (the computer stopped, or goalcup was killed). Its record ends at
   15:43.` When the connector has `final` native rows: `<harness>'s database was not copied at the end
   of this part; the next part copies it whole.` (PRD-02 9.4).
8. **Upload lines** are PRD-06 section 10's. "Removed here" names pruning plainly, so the participant
   is not surprised that the folder is small (run report finding 6). When bytes are left at the end of
   the wrap-up: `Upload: 120 MB still waits on this computer. Run goalcup upload before Sun 15 Nov,
   09:12 UTC, when you are online. Nothing is lost yet.` (`upload_by`, PRD-00 question 1 (old 2), review A
   finding 15). After `upload_by`: PRD-06's "server closed" line. There is no `verify:` line on the
   participant screen. When every record file is still local (`keep`), `goalcup.log` holds
   `goalcup-verify check <entry dir>`; otherwise it names the server copy (PRD-08 interface).
9. The technical lines of today (admission counters `finish.rs:437`, outage lines `:446`, the seal
   report `seal.rs:399-455`, "harness exited:", "uploaded: N bytes, acknowledged, and the seal")
   go to `goalcup.log` only, where they still exist (most are gone with the daemon).

**Gap reasons in plain words** (PRD-01 section 3, final numbers; an unknown number shows as
"other (n)"):

| n | Reason | Words |
|---|---|---|
| 1 | `memory_guard` | under heavy load |
| 2 | `recorder_down` | recording was off |
| 3, 4 | `unsupported_encoding`, `decode_failed` | a reply in a form goalcup cannot read |
| 5 | `source_changed` | a session file was replaced or cut short |
| 6, 7 | `tls_refused`, `pinned` | the program refused the recorder |
| 8, 9 | `write_error`, `read_error` | reserved, never written by 1.0; shown as "other (n)" |
| 10 | `cut_at_seal` | still arriving when the part ended |
| 11 | `h2_unsupported` | a connection goalcup could not follow |
| 12 | `mask_failed` | goalcup could not check it for secrets, so it dropped it |
| 13 | `upstream_untrusted` | the model service's certificate was not one goalcup trusts |
| 14 | `protocol_other` | a connection in a form goalcup does not read |
| 15 | `after_stop` | sent after the agent ended |

A full disk ends the recorder (exit 74) and shows as off time with cause `write_error` in
`goalcup.log`, not as gap reason 8.

### 6. Gateways, registered hosts and dodgers: what the participant sees

The owner asked whether a gateway is "just an HTTPS proxy with a different endpoint", and whether
dodgers can "use the current way". Yes to both. One pipeline, three cases, no setting for the
participant to choose:

- **Direct key or gateway on the shared list** (Vercel AI Gateway, OpenRouter, Cloudflare AI Gateway,
  cloud model services): written requests are counted under the gateway's host name, like any
  provider. Same lines, same end screen. The participant configures the gateway inside goalcup's own
  harness home at setup, as they would without goalcup (PRD-11 interfaces).
- **A host goalcup does not know** (a self-hosted gateway, a company proxy for models):
  `goalcup setup --record-host llm.example.com` runs PRD-11's one check
  (`connector::hosts::check(name, Source::Participant)`) and adds it to `record-hosts.txt`; the next
  `start` or `continue` decrypts and records it. The line printed: `goalcup will record
  llm.example.com from the next part (during a run: goalcup seal, then goalcup continue). Your harness
  must trust goalcup's certificate for it, which goalcup sets up.` The check refuses: an IP address, a loopback or private name, a sign-in host on
  the `never` list (PRD-03 section 3: point 4), and a pattern wider than one label (PRD-17 N1 to N6).
  Removing a host: edit the file; the path is printed.
- **A dodger** (ignores `HTTPS_PROXY`, pins certificates, or runs inference on its vendor's servers):
  never hacked around. The start line names the tier from `connectors/tiers.toml` (PRD-12):
  `Codex 0.165.0: verified with goalcup on 0.165.0.`, or on another version `<harness> <version>:
  verified with goalcup on <v>, not on <version>.`, or `<harness> <version>: not verified. goalcup
  records what reaches it; goalcup status says if model requests are missing.`, or `<harness>: not
  supported. goalcup records its session files and the connections it can see, not its model calls.`
  The status and end screen then say what sections 4 and 5 say. The same pipeline, an honest label.
- **A model on this computer** (Ollama, LM Studio): reached past the proxy (`NO_PROXY` covers
  loopback) and not recorded. The `local_model` note line says so; nothing is built around it.

### 7. `goalcup setup` on macOS, Linux and WSL2

Steps (was four with login; login goes):

1. **Harness.** Installed harnesses from the connector list (PRD-11), pick one (Enter takes the
   saved one). Prints `<harness> <version> (<path>)` and the tier line of section 6. A harness not on
   PATH: `<harness> is not installed. Install it, then run goalcup setup again.` (was
   `harness_executable`).
2. **Home.** "Press Enter to open <harness> in its Goal Cup home. Sign in there, then quit it."
   (kept, `prep.rs:763-870`). The home is PRD-11's isolated home; goalcup never touches `~/.claude`,
   `~/.codex` or any app-wide config (AGENTS.md hard rule).
3. **Auto-update off and no-prompt settings** (app. H 3, PRD-00 question 2 (old 8), review A finding
   16). goalcup always writes the connector's `update_off` values into that isolated home, with no
   question and with or without `--yes` (ledger H9); the switch is never part of the question. Then
   it asks one question about the `no_prompts` values: `<harness> will run for days with nobody
   watching. goalcup can set it to never stop and ask for approval (it can then change any file you
   can). Set this in goalcup's own <harness> home? [Y/n]`. Then one line says what it set and where.
   "n" keeps today's advice text (`prep.rs:911-944`) and still writes `update_off`. A harness that
   rewrites its file on first run is re-read and warned (app. H 3 risk). With `--yes` the question is
   skipped and the answer is yes (seam S15).
4. **Machine.** One line each, all warnings, none holds back `Ready.`: the sleep lock (section 9);
   the server reachable (one HTTPS GET of `/v1/health`); `git` installed; WSL1 or a Windows drive.
   Then the masking paragraph (PRD-05 section 12; its default text, since PRD-00 question 3 (old 12)
   defaults to no).
5. `Ready.` and the next two commands:

```
Ready.
  Practice (10 minutes, no GitHub):
    d=$(mktemp -d) && cd "$d" && goalcup start --practice
  Your run, inside the start window, from your empty project folder, with the prompt saved outside it:
    goalcup start --repo <you>/<repo> --prompt ~/goal-prompt.txt
```

Without a terminal, setup prints steps 1 and 4 and `Ready.` or the step to fix, and exits 0 or 1. It
opens nothing, asks nothing and writes nothing. With `--yes` it also writes step 3's values (`update_off` and `no_prompts`) and prints
the isolated home's path (the pad, PRD-12, and the installer smoke use this).

Cut from setup (appendix H section 7.2): the login step, the `goalcup mcp` lines
(`prep.rs:721-760`) and their copy markers, the SSH key and agent lines (`prep.rs:418-429`,
`ssh_keys.rs`), the disk step (`prep.rs:946-1020`), the old GitHub token notice (`prep.rs:437-446`),
`--json`, `--dev`, the `prep` alias, "N of 4 steps need you".

**Per OS** (facts the code relies on):

| | macOS | Linux | WSL2 |
|---|---|---|---|
| Install | `curl ... | sh` (PRD-14 owns `packaging/install.sh`); curl downloads are not quarantined, so the unsigned binary runs (`install.sh:12`) | same; glibc 2.35 or later (`install.sh:65-68`) | same, inside the Linux distribution; WSL1 refused |
| Binaries | `goalcup`, `goalcup-verify` (no `goalcup-daemon`) | same | same |
| PATH | one line that exports and appends to the right profile (app. H 8, PRD-14) | same | same |
| `GOALCUP_HOME` | `~/Library/Application Support/goalcup` (has a space: every printed path is quoted) | `~/.config/goalcup` | same as Linux |
| Sleep lock | `caffeinate -i -s -w <pid>`: no idle sleep, no system sleep on mains; closing the lid still sleeps | `systemd-inhibit --what=sleep:idle --mode=block`; none in a container or without logind | none: "set Windows sleep to Never" |
| Browser | `open` | `xdg-open`; headless: `GOALCUP_NO_BROWSER=1` prints the address | the Windows browser (copied opener, `goalcup-platform/src/machine.rs:515-534`) |
| Clipboard | `pbcopy` | `wl-copy`, else `xclip` | `clip.exe` (`push_lines.rs:57-80`) |
| Project folder | anywhere outside a Git work tree | same | inside the Linux file system, never `/mnt/c` (`folder_gate.rs`) |
| Firewall | Little Snitch and the like ask once per binary; one guide line (PRD-15) | | |

### 8. The launch block and the push block (run 1 finding 8)

The push block stays the participant's (revision 2's O10, `docs/RULES.md:142-151`): goalcup writes
nothing into the project folder. What changes is that it becomes part of **one paste** (PRD-00
question 2, old 10).

- **`--prompt FILE`** (new, any run with `--repo`: competition and `--test`; with `--practice` or
  without `--repo` the file holds the prompt only, no push block). The file must be outside the
  project folder (section 3 step 2). goalcup reads it as UTF-8 (not UTF-8 is refused) with no size
  limit (ledger R5), builds `push block + blank line + prompt`, writes it to `runs/<run>/first-message.txt`
  (0600) and copies it to the clipboard when a tool is there. The launch block says:
  `Your first message (the push block, then your prompt) is on your clipboard and in <file>. Paste it
  once as your first message and press Enter.` The prompt is not recorded specially: it reaches the
  record on the wire like any message, masked like any byte.
- **Without `--prompt`** the block is printed as today and copied to the clipboard, under one framed
  line: `Your first message must start with these lines. Without them your agent cannot push, and
  you may not send them later.` (`docs/RULES.md:98-101`: after the prompt, only "keep going".)
- The push block text is today's `prompt_block` (`participant/launch.rs:158-186`) without change.
  The token stays in `.env.local`; the credential helper reads it at each push.
- `goalcup continue` never shows the block again when the run's earlier parts wrote a model request
  (the prompt was sent). With none it shows it again with "If you never sent your prompt, send it
  now" (today's `Opens::LastSession` branch). The pre-0.7.1 branch (`launch.rs:126-130`) goes.

The launch block keeps its shape (`participant/launch.rs:62-147`): run kind, "<harness> opens now in
this terminal", the connector's `first_screen` hint, what to type, the end time, the upload line
(PRD-06 section 10, "Upload: to <server>, every 5 minutes."), the tier line (section 6), the file
connector line (section 1), the home lines (section 4), the sleep line (section 9), and last
`Recording in chat mode: <harness> opens now.` (`READY_MARK`, word for word).

Rejected: `git init` plus remote and helper in `.git/config` before launch (app. H 6). It breaks "goalcup
adds nothing to the folder" and "the agent makes the repository itself", and O10 settled wording
only. Rejected: a token prompt that writes `.env.local` (app. H 5); same rule, more code.

### 9. Sleep lock

- Copied from `goalcup-platform/src/awake.rs` (269 lines, about 150 kept) into `src/cli/awake.rs`:
  same tools and flags, the thread that owns the child, `PR_SET_PDEATHSIG` on Linux, `-w <pid>` on
  macOS, the 25 s worst case on a hanging D-Bus (`awake.rs:74-76`).
- Taken by `start` and `continue` before `watchdog::run`, held in the watchdog process for the whole
  entry (PRD-02 interfaces row PRD-10). The lock dies with the watchdog, which is correct for the
  harness: the harness is the watchdog's child.
- **Never a `PROBLEM:` line.** A machine with no inhibitor gets one line in the launch block and one
  in status: `This computer may sleep during the run (no sleep lock: <why>). Keep it on mains power
  with the lid open.` Run 1's sandbox showed a `PROBLEM:` line for this from start to end, which
  taught everyone to ignore `PROBLEM:` lines.
- Sleep is not "off". A sleeping machine suspends the harness too; nothing is lost and no gap is
  written. PRD-01's `suspend` event says why a boot was quiet, and every timer runs on boot time
  (PRD-00 rules of the shape).

### 10. `goalcup time-left` (D66)

- Inside the harness: start step 9 adds `GOALCUP_TIME_LEFT_FILE=<entry>/upload.json` to
  `plan.env` before `watchdog::run`. The command reads `server_word` there (PRD-06, seam S9: `seconds_left`, `asked_at`), subtracts the
  wall time since `asked_at`, and prints today's line (`run/time_left.rs` `left_line`, copied). Older
  than 11 minutes (two 5-minute heartbeats and a minute) or null: "unknown right now", exit 1, as
  today (`time_left.rs:47-73`). When `server_word.stop` is true: "Time is up." exit 0.
- In the person's own terminal: the newest open run's newest entry, same rule.
- The goal-cup skill (`skills/goal-cup/SKILL.md`, 34 lines) tells the agent the command exists. Its
  MCP paragraph goes. `skills/goal-cup-setup/` (106 lines) is deleted (app. H 10). PRD-11's run-home
  `.profile` keeps `goalcup` on PATH in a login shell (PRD-17 P11).

### 11. Version 1.0

- `goalcup --version` prints `goalcup 1.0.0`. The number is the workspace version (PRD-00 phase 0
  sets `1.0.0` in `Cargo.toml`); `CLI_VERSION` stays `env!("CARGO_PKG_VERSION")`.
- The `start` event carries it (PRD-01 section 11 `goalcup` key); open and continue send it; the
  Worker's floor refuses 0.7 in every mode (PRD-07 section 6, PRD-00 question 5). Staging carries the
  1.0.0 floor from its first deploy (PRD-00). The refusal text the Worker sends is shown as is, after
  "goalcup: ".
- The help footer (`main.rs:49-60`) becomes:

```
Steps:
  1. goalcup setup                                   once, until it says Ready.
  2. goalcup start --practice                         a 10-minute try, no GitHub (optional)
  3. goalcup start --repo <you>/<repo> --prompt FILE  your run, in the start window; FILE outside the folder
While it runs: goalcup status (in another terminal). If status says so: goalcup recover.
If the agent stopped early: goalcup continue.
Guide: https://github.com/agentcommunity/goal-cup-proxy/blob/main/docs/PARTICIPANT-GUIDE.md
Help: goalcup@agentcommunity.org
Exit codes: 0 done; 1 refused, and you can fix it; 3 the command could not do its job.
```

- No 0.7 compatibility (ledger L7). One line in the release notes covers 0.7 testers (PRD-15).

### 12. `goalcup.log` (seam S11, owned here)

One plain text file per entry (`runs/<run>/<entry>/goalcup.log`, 0600), appended by the watchdog and
the recorder through one function: boots and exits, null mode, every `note` event, stopped helpers,
the full tunnel list at the seal, upload errors, the after-seal byte count. It replaces `host.log`
(`src/host_log.rs`, 253 lines; about 60 kept). It is not part of the record and is never uploaded.
The end screen and every refusal that needs support detail end with its path.

```rust
// crates/goalcup/src/cli/log.rs (PRD-10), used by PRD-02, PRD-03, PRD-06
pub fn line(entry_dir: &Path, event: &'static str, facts: &[(&'static str, Fact)]);
pub enum Fact { N(u64), Time(SystemTime), Host(Host), Path(PathBuf), Word(&'static str), Exit(ExitStatus) }
```

There is no free text: `event` and `Word` are fixed strings in the binary, `Host` is a checked host
name, and an `io::Error` is logged by its kind name only. So no header value, body byte, token or
credential can reach the file, and a panic message cannot either (PRD-02's panic hook writes file,
line and fixed text only). One `O_APPEND` write per line, at most 4 KiB, so concurrent writers never
interleave inside a line. Opened, written and closed per line (PRD-02 section 10 counts one
descriptor at a time).

### 13. The message list

The contract words stay exactly: `Recording in chat mode`, `shows the code XXXX-XXXX.`, `PROBLEM:`.
Every other message is a plain function of facts in `src/cli/text.rs` (replaces `src/participant.rs`
and `src/participant/*`, 1,788 src lines, to about 420). Tests assert facts and the three contract
words, never whole sentences (AGENTS.md "Tests").

| Group | Kept (appendix H section 5 names) | Changed or new in 1.0 | Cut |
|---|---|---|---|
| Fragments | `CONTINUE_HINT`, `KEEP_GOING`, `STILL_OPEN`, `TIME_IS_UP`, `NOTHING_MORE`, `RUN_AGAIN`, `SUPPORT_ADDRESS`, `EARLIER_PART_UPLOADING`, `NOT_STARTED_HERE` | | `DETAILS_HEADING` (one "Details for support are in <log>" line), `FREE_OUTSIDE` |
| Approval | `approval_code`, `replaces_open_run`, `APPROVED`, `server_unreachable` | one intro line; one "approval failed" with a reason (denied, expired, interrupted) | `login_done`, "two approvals are normal" (6 places, app. H 1) |
| Launch | `launch_block`, `RUN_GOES_ON`, `prompt_block` | clipboard line names the first message (section 8); tier, file connector, home, upload and sleep lines | pre-0.7.1 branch |
| Start refusals | `home_busy`, `still_recording`, `needs_terminal`, `project_of_open_run`, `cannot_record_model_service`, `repository_not_empty`, `dev_competes` (now: file flags in a competition run), `server_answer_unusable`, `not_started_here`, `local_only`, `run_over`, `last_part_closing`, `continued_elsewhere`, harness not on PATH, WSL1 or Windows drive, run credential (two), "could not start recording, so <harness> was not started", `MaskError` text, connector `refusal` | one folder refusal that lists every problem; one "project folder is gone"; one "reinstall one release" install line | disk reserve, provider-host probe, `prep.json` gate, pinned files, `--practice` needs setup, the 11 internal launch guards (one "goalcup could not launch <harness>: <why>. Write to support."), daemon prefixes (`control.rs:785-810`) |
| Start warnings | default host warning, filesystem (NFS, SMB only), synced folder, shared home was open (rotate keys), Ctrl-C may end this command too | sleep line (section 9), tier line (section 6), harness updated since last part, "N key values were not added to masking" (only with the owner's yes to PRD-00 question 3) | provider host unreachable as a refusal; helper not stopped (now a `goalcup.log` line from PRD-02) |
| Status | `ends_line`, `status_next`, `no_run_yet`, `problem_state_stale`, `problem_no_model_calls`, `problem_native_silent`, `problem_agent_exited`, `agent_finished`, `CHECK_NETWORK` | the state lines and block of section 4; `native_missing`, `native_other_device`, `long_tunnel` (two), listed tunnel, `parse_stopped`, `local_model`, fail-closed, `fd_limit`, vendor and home lines; one upload line | `STATUS_DETAILS`, `this_computer`, disk lines, recorder restarted and lost, sleep `PROBLEM:`, push section |
| End | `PRACTICE_NEXT`, `UPLOADING_REST`, `UPLOAD_DONE`, `ending_block`, `after_upload` | the "What this part recorded" block of section 5; after-seal, outlived, recovered and "run goalcup upload before" lines | "Recording works", admission line, seal report, "harness exited:", "uploaded: N bytes, acknowledged", outage lines (to the log), `helper_not_stopped` (to the log) |
| Seal | `seal_inside_harness`, `seal_question`, `seal_not_confirmed`, `seal_needs_terminal` | `seal_done` without disk words; three endings: "still finishing, run goalcup seal again", "could not finish, write to support", "already sealed" | entry, terminal record, seal.json, anchor, termination lines |
| Upload | PRD-06 section 10 lines; `entries_intro` becomes the "Parts:" line | `upload_not_opened` and `upload_local_run` merged; "server closed" | `upload_seal_missing` disk words |
| Setup | frame line, harness pick, home step, machine warnings, `Ready.` block | `update_off` line (always) and the no-prompt question (section 7); `--yes` line; `--record-host` lines | login, `goalcup mcp` lines, SSH, disk, old token notice, "N of 4 steps" |
| Continue | resume and new-session lines (D73) | harness updated line; recovered part line; "harness from the last part is still running" refusal | session store notes ("the harness's session store could not be read") |
| Recover (new, R3b) | | the success block, five refusals and two failure lines of section 3.2; the two both-dead state lines of section 4; the null-mode line without a retry count | |
| Time left | `time_left_unknown`, `no_open_run_time_left` | reads `upload.json` `server_word` | MCP answer |
| Masking | PRD-05 section 12 paragraph, in the guide and once at setup | its default text (PRD-00 question 3 defaults to no) | |

## Files it owns

Create (under `crates/goalcup/`, PRD-00 ownership map):

| Path | What | Src lines (estimate) |
|---|---|---:|
| `src/main.rs` | clap tree of eight commands, every flag (incl. `--connector`, `--hosts`, `setup --yes`), the hidden recorder entry; exit codes. Published in phase 0 with stubs that call each owner's module | 160 |
| `src/cli/mod.rs` | `GOALCUP_HOME`, `GOALCUP_SERVER`, run and entry paths, `setup.json`, `record-hosts.txt`, pid liveness | 130 |
| `src/cli/start.rs` | section 3 steps 1 to 10, `StartFacts`, competition refusal of file flags | 340 |
| `src/cli/folder.rs` | folder checks in one refusal, token line, WSL and network file system checks (from `start/project.rs` 363 and `folder_gate.rs` 224) | 170 |
| `src/cli/approve.rs` | device flow, run credential, repo pre-check (from `account/device.rs`, `credential.rs`, `commands/run_repo.rs`) | 280 |
| `src/cli/continue_run.rs` | section 3.1: close dead entry, continue, resume rule, version line | 150 |
| `src/cli/recover.rs` | section 3.2: the checks, rebuilt start facts, the call to PRD-02's recover entry | 80 |
| `src/cli/status.rs` | section 4 | 260 |
| `src/cli/finish.rs` | section 5, wrap-up progress | 160 |
| `src/cli/seal.rs`, `src/cli/upload.rs` | ask, signal, dispatch | 90 |
| `src/cli/setup.rs` | section 7, `--yes`, `--record-host` | 270 |
| `src/cli/push.rs` | push block, `--prompt`, clipboard (from `push_lines.rs` 213) | 110 |
| `src/cli/time_left.rs` | section 10 | 80 |
| `src/cli/awake.rs` | section 9 | 150 |
| `src/cli/skill.rs`, `src/cli/log.rs` | skill install; `goalcup.log` writer (seam S11) | 140 |
| `src/cli/text.rs` | every message (section 13) | 420 |
| **Total** | | **about 2,990** (PRD-00's CLI row: about 2,900; `recover` is new, ledger R3b) |

Tests: `crates/goalcup/tests/cli_*.rs` (section Acceptance), using PRD-14's `tests/common/` fake Worker
and temp homes (a private helper until common lands, removed at phase 3). Repo files:
`skills/goal-cup/SKILL.md` (change), `skills/goal-cup-setup/` (delete).

Must not touch: `src/pipeline/`, `src/watchdog/` (PRD-02), `src/proxy/` (PRD-03), `src/native/`
(PRD-04), `src/mask/` (PRD-05), `src/upload/` (PRD-06), `src/connector/` and `connectors/**`
(PRD-11; `connectors/tiers.toml` is PRD-12's and is only read here), `packaging/**` and `scripts/**`
(PRD-14, PRD-12, PRD-07), `docs/**` (PRD-15), `crates/goalcup-format/` (PRD-01), the old crates
(PRD-13).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-00 | Seams S11 (`cli::log::line`), S15 (`setup --yes`), S16 (`--connector`, `--hosts`) owned here; questions 2 (old 8 to 11) and 4 (old 15) carry this PRD's defaults; question 3 (old 12) decides the extra mask calls | Both |
| PRD-01 | COUNT and seal keys (section 9) incl. `hosts`, `tunnel_hosts`, `req_holes`, `mask_failclosed_bytes`; GAP reasons 1 to 15 (section 3); seal reasons incl. `recovered`, `server_stop` and `harness`; notes `long_tunnel`, `ca_refused`, `fd_limit`, `local_model`, `parse_stopped`, `native_missing`, `native_other_device`, `outlived` (PRD-00 R3-4); `tunnel_hosts` `listed`; `req_whole`; `goalcup` in the start event | Used here |
| PRD-02 | `watchdog::run(plan, facts) -> ExitCode` (seam S8); the hidden recorder subcommand dispatched to `pipeline::run_recorder`; `pipeline::close_dead_entry(dir) -> Result<Closed, StillRecording>`; `status.json` (seam S4: `pid`, `mode`, `harness`, `written`, `last_request_at`, `last_native_at`, `losing`, `off`, `notes`, `port`; sealed: `seal`, `cut_files`, `outlived`, `after_seal {bytes, hosts}`); `rec/watchdog.pid` (SIGUSR1 for `goalcup seal`, else SIGTERM to the recorder pid); the watchdog runs `plan.before` and `plan.after` and ends the harness; the sleep lock is held in the watchdog process. N7 and N8 ruled by PRD-00 R3-2. For `goalcup recover` (R3b): `watchdog::recover(entry_dir, facts)`, the run certificate key file (0600, run folder, kept for the run) and `status.json` `harness {pid, started}` (ask N9) | Both |
| PRD-03 | Notes `long_tunnel` (`why` stream or upload), `ca_refused`, `parse_stopped`; tunnel GAP reasons 6, 7, 13, 14; status wording in section 4 | Used here |
| PRD-04 | `written.native_files`, `native_bytes`, `NativeFinish.cut_files` (via sealed `status.json`), note `native_missing` for a `warn_missing` row, note `native_other_device`; run start in whole seconds in the start facts | Both |
| PRD-05 | `mask::workspace_values` (`GH_TOKEN` only by default); `mask::env_values`, `mask::harness_values` and `skipped` only after the owner's yes to PRD-00 question 3; `MaskError`; `mask_failclosed_bytes`; section 12 text | Used here |
| PRD-06 | `upload.json` (seam S9) incl. `state`, `upload_by`, `server_word`; section 10 lines; `upload::manual::run`; `upload::client::ServerClient` for device flow, open and continue; `keep` in the start facts | Both |
| PRD-07 | Device routes unchanged; open and continue bodies (`connector_sha256`); `sealed`; version floor refusal text | Used here |
| PRD-08 | `goalcup-verify check` line goes to `goalcup.log` only when local record files are whole; the screen's counts are the ones `counts_match_seal` recounts | Both |
| PRD-11 | Seam S3: `connector::load(id_or_path, hosts_file)` then `connector::plan(&connector, &env, home)` (no `PlanCtx`); `LaunchPlan` (`program`, `argv`, `env`, `home`, `lock`, `before`, `after`, `hosts`, `secret_values`, `notes`, `refusal`); connector fields `no_prompts`, `update_off`, `skills_dir`, `first_screen`, `warn_missing`; host kind `vendor` on the shared list; `connector::hosts::check` for `--record-host`. Deletes `[participant.time_left_tool]` from four connectors | Both |
| PRD-12 | `connectors/tiers.toml` (tier, verified version, reason per connector), read here at build time. Pad uses `GOALCUP_SERVER`, `GOALCUP_KEEP_PARTS`, `GOALCUP_NO_BROWSER`, `setup --yes`, `start --test --repo --prompt --hosts`, `first-message.txt`, `seal`, `status --json`, the three contract words | Both |
| PRD-13 | Deletes the old CLI files listed below after the new ones pass; the session sniffing call is answered in section 3.1 | Done there |
| PRD-14 | `install.sh`: two binaries, the run guard reads `status --json` `status.mode` (`recording`, `losing`, `writer_slow`, `null` mean a run is live), one PATH line (app. H 8); `tests/common/` fake Worker and homes | Used there |
| PRD-15 | The participant guide from sections 3 to 12; the masking paragraph; the gap words table; the 0.7 release-notes line; the firewall line; "part" and "record file" words | Used there |
| PRD-17 | P14 (no pre-launch timeout under 15 minutes), S7 (deadline from the server's `seconds_left` on the boot clock; PRD-06 guards the segment side), HF4 (`home_busy`), HF6 (resume rule does not trust a session file alone), P11 (acceptance 7); `--record-host` uses the one host check (N1 to N6) | Used here |

## What is cut

All under `crates/goalcup-cli/` (deleted whole by PRD-13; these are the participant parts not
carried into `src/cli/`). Lines are `wc -l` of the file, src and inline tests together unless split.

| Path | Lines | Why |
|---|---:|---|
| `src/commands/login.rs` | 84 | app. H 1, PRD-00 question 2 (old 9) |
| `src/commands/mcp.rs` | 474 (273 src, 201 tests) | Answer 6, app. H 4 |
| `src/commands/doctor.rs` | 556 | Folded into setup; its pin, daemon, decoders and SSH checks are cut |
| `src/commands/entries.rs`, `src/entries.rs` | 82, 208 | Status lists parts |
| `src/ssh_keys.rs` | 241 | app. F 10; event-rule item moved into 1.0 |
| `src/prep_file.rs` | 88 | `setup.json` without the start gate |
| `src/run/pause.rs`, `src/run/reserve.rs` | 360, 956 | Disk handling (app. F 2) |
| `src/run/watch.rs` | 1,485 | Disk watch, time-left file and heartbeat move to PRD-06 and section 10 |
| `src/participant.rs`, `src/participant/*.rs` | 1,788 src, 702 tests | Rewritten as `text.rs` (about 420) |
| `src/commands/prep.rs` | 1,364 | Rewritten as `setup.rs` (about 270) |
| `src/commands/start/*.rs` (8 files) | 4,822 | Rewritten as `start.rs`, `folder.rs`, `finish.rs` (about 670); the proxy, sink, admission, termination and helper parts belong to PRD-02 and PRD-03 |
| Session sniffing: `src/launch.rs:664-771`, `commands/start/phases.rs:296-361`, `:745-779` | about 210 | Replaced by the written-count resume rule (section 3.1) |
| `src/commands/status.rs`, `seal.rs`, `continue_run.rs`, `time_left.rs` | 814, 784, 494, 168 | Rewritten (about 580) |
| `src/account/github.rs` | 466 | Old clone token, old `github.json`, hidden credential helper (app. F 8) |
| `src/push_lines.rs`, `src/host_log.rs`, `src/skill.rs` | 213, 253, 162 | Trimmed copies (about 250) |
| Tests | `tests/it/login.rs` 159, `prep.rs` 1,600, `status_views.rs` 1,869, `preflight_doctor.rs` 478, `start_launch.rs` 1,481, `native_silence.rs` 152, `seal_scan.rs` 206, `account.rs` 1,195, `tests/m2_cli_lifecycle.rs` 1,944, `tests/full/t25_disk_full.rs` 1,173, `full/per_run_lifecycle.rs` 366 | 10,623 test lines; the cases still needed are rewritten as `tests/cli_*.rs` (about 1,200) |
| Connector data | `[participant.time_left_tool]` in four `harness.toml` files | PRD-11 deletes |
| Skill | `skills/goal-cup-setup/SKILL.md` | 106 (app. H 10) |

## Acceptance

Real-run proofs first (on the pad, PRD-12, in Vercel Sandboxes (ledger B1), staging Worker,
`GOALCUP_KEEP_PARTS=1`; macOS on the owner's Mac after the build). `cargo` builds and the
`tests/cli_*.rs` runs happen in the sandboxes too, never in the cloud coding environment. No proof
here needs a `test-hooks` build.

1. **The end screen equals the recount.** Claude Code, direct key, the pad's `full` row (the
   payments-platform prompt, many subagents). Pass: every number on the end screen (model requests
   written, per host, written whole, with missing parts, MB lost, session files, off count and time,
   tunnel hosts and bytes) equals `goalcup-verify check --json` on the same entry after the screen's
   rounding (bytes to MB with one decimal, times to the second). The screen never
   contains "Recording works".
2. **Off is shown.** Codex pad run with PRD-12's `--kill-recorder` twice (plain `kill -9`, no hooks).
   After each kill the watcher runs plain `goalcup status` once a second for 60 s and saves each
   output. Pass: the first saved output after the new boot's `status.json` `boot` changes shows the
   new "Off:" count, within 2 s; none of the 60 saved outputs contains "Recording is off" (each
   restart is under 30 s); the end screen
   says "Recording was off 2 times" with a total equal to the sum of the `recorder_down` gaps in
   `goalcup-verify dump`.
3. **Dodger is honest.** The pad's dodger case for Claude Code, Codex and Hermes (`--hosts` with the
   model host pinned). Pass: status shows the "refused the recorder" `PROBLEM:` line during the run;
   the end screen names the pinned host on its own line under "Not recorded", never says "Nothing was
   lost", and the launch block shows `Host list from a file: <path> (not verified).` (or
   `Connector from a file` when a connector file is used). A dodger run of 6 minutes or more shows,
   from minute 5, the "No model request was written yet" `PROBLEM:` line.
4. **Gateway looks like a direct key.** The pad's gateway case. Pass: written requests are listed
   under the gateway's host and the screen has no gateway-specific line.
5. **One paste pushes.** A `--test` run with `--repo` and `--prompt`. Pass: `first-message.txt`
   starts with the 7 push lines and ends with the prompt file's bytes; the pad pastes it once; the
   GitHub repository has at least one commit from the agent within the run.
6. **Setup in the pad.** `goalcup setup --harness codex-cli </dev/null` and then
   `goalcup setup --harness codex-cli --yes` in a pad box. Pass: the first exits 0 or 1, prints no
   `goalcup mcp` text, opens no browser or harness and writes nothing; the second writes the
   `update_off` and no-prompt values only inside the printed isolated home (no file under the real `~/.codex`
   changes); the sleep line is a warning (no inhibitor in the sandbox), and no `PROBLEM:` line for
   sleep appears anywhere in the run.
7. **time-left during a run.** The watcher runs `goalcup time-left` itself in the pad box during a
   run, once in the participant's shell and once as `sh -lc 'goalcup time-left'` with the harness's
   `HOME` and environment, `GOALCUP_TIME_LEFT_FILE` included (PRD-17 P11). Pass:
   both print a time within 1 minute of the server's `seconds_left` in the last heartbeat answer.
8. **Version.** Pass: `goalcup --version` prints `goalcup 1.0.0`; the start event says `1.0.0`; a
   0.7.0 client is refused by the staging Worker with the floor's text.
9. **macOS (owner, after the build).** Pass: setup prints `Ready.`; during a run `pmset -g
   assertions` lists the `caffeinate` hold; every printed path under `Application Support` pastes
   and works.
10. **Continue resumes.** A Codex `smoke` pad run, then `goalcup seal`, then `goalcup continue`. Pass:
   the harness opens with the connector's resume arguments, and `server.json` marks the second part
   as resumed.
11. **Recover brings recording back** (ledger R3b). A Codex `smoke` pad run; `kill -9` the watchdog and
   the recorder together (as PRD-02 R7); within 10 s plain `goalcup status` shows the "Run: goalcup
   recover" line; then `goalcup recover` from another shell. Pass, from the record only (ledger T4):
   the harness pid in `status.json` is unchanged; the same port is bound; model requests written after
   the recover are in the record; `goalcup-verify dump` shows one `recorder_down` gap from the kill
   to the new boot's first record; the part seals with `harness_exit` or `deadline` and verifies. If
   the harness lost its terminal and exited (PRD-02 section 9.1), the watcher writes it down, checks
   that status shows the `goalcup continue` line instead, and the case runs again on a harness the pad
   starts without a TUI.

No proof here calls an outside API to check a count (ledger T4): every pass reads the record,
`status.json` and the staging server copy. The history check is the pad's (PRD-12).

Automated tests that guard them (`tests/cli_*.rs`, facts and contract words only):

- `cli_finish.rs`: from fixture `status.json` and `upload.json` files: zero loss gives the "nothing was
  lost" claim (checked present or absent, never the sentence, ledger B10, PRD-14 section 2); any gap, off time, listed tunnel, fail-closed byte or cut file never does; counts and hosts
  printed are the fixture's; `req_whole` is shown when `req_holes` is above 0; the after-seal line
  appears only when `after_seal.bytes` is above 0; every gap reason 1 to 15 has words; a recovered seal
  gives the recovered line.
- `cli_status.rs`: each row of the state table in section 4 from a fixture, including a dead recorder
  with a live watchdog, both dead with the harness alive (the `recover` line) and with it gone (the
  `continue` line), a null-mode file whose line has no retry count and says it keeps trying, and a
  `losing` file (the memory-guard and null
  proofs move here from real runs, review B T2); one `PROBLEM:` per cause; every note in section 4
  gives its line; the `long_tunnel` fix names `seal` before `continue`.
- `cli_push.rs`: first-message composition (the push block only with `--repo`); a `--prompt` file that
  is not UTF-8 is refused and a 1 MiB one is not; the push block never holds the token value.
- `cli_folder.rs`: three folder problems give one refusal naming all three; a `--prompt` file inside
  the project folder is named in it; overlayfs is accepted.
- `cli_setup.rs`: no terminal means no question, no open and no write; `--yes` writes only into the
  isolated home; `update_off` is written with `--yes` and after an "n" answer, which writes no
  `no_prompts` value; no `goalcup mcp` text; `--record-host` refuses an IP, a loopback name and a `never`
  host.
- `cli_continue.rs`: the resume rule from fixture `status.json` files (written requests, session
  files, a quick non-zero exit after a resume); a live harness from the last part refuses with the
  `goalcup recover` line.
- `cli_recover.rs`: each check of section 3.2 from fixture files and fake pids (sealed, run over,
  watchdog alive, recorder alive, harness gone, inside the harness), each giving its line and exit
  code; a port held by another listener and a missing key file refuse with exit 1 and start nothing;
  `recover` never sends a signal to the harness pid.
- `cli_time_left.rs`: from `upload.json` `server_word`, fresh, older than 11 minutes, null, and
  `stop`.
- `cli_log.rs`: `cli::log::line` writes one line per call with `O_APPEND`; two threads never
  interleave inside a line; the API has no way to pass a free string (a compile-fail doc test).
- `cli_commands.rs`: `--help` lists exactly the eight commands; `goalcup login` and `goalcup mcp`
  are usage errors (exit 1); `--connector` or `--hosts` on a real run is refused before any file is
  written; `second_start_on_one_home_refuses`: a second `start` on one isolated home while the first
  runs gives `home_busy` (PRD-17 HF4); no timer before the launch is shorter than 15 minutes: a fake
  device route that answers after 14 minutes on a paused clock still launches (PRD-17 P14).
- `cli_start.rs`: the deadline in the start facts comes from the server's `seconds_left` on the boot
  clock; a wall clock set back 24 h does not change it (PRD-17 S7; PRD-06 guards the segment side).

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Notes roll off | `status.json` keeps the last 20 notes; in a long run a once-per-entry `native_missing` or `local_model` note falls out and its line vanishes | Ruled (PRD-00 R3-2): `status.json` keeps the first note of each `what` beside the last 20 |
| Pid reuse | A dead recorder's pid is reused by another program and status says "on" | Alive means a `goalcup` binary at that pid; `updated` older than 10 s gives "not answering" anyway |
| Missing parts alarm people | A valid run with a few missing parts looks broken | Say what was written whole first, give MB lost, and keep "Your agent is not slowed" |
| Cutting `login` | Terms not accepted show at the first practice start, not at setup; staging needs `GOALCUP_SERVER` | The approval page already says so; the guide names the variable |
| No-prompt settings (app. H 3) | The agent can change any file the user can; a harness may rewrite its settings file; `--yes` skips the question | One explicit question, default yes, plain warning; re-read and warn; `--yes` is for the pad and scripted installs and says what it set; `update_off` is written whatever the answer (ledger H9); PRD-00 question 2 |
| Resume rule without sniffing | A run with session files but no conversation (Claude Code writes one at startup, PRD-17 HF6) is resumed and the harness exits at once | D73's `resume_failed` opens a new session on the next `continue`; one wasted `continue` at most |
| File flags | A participant uses `--hosts` to keep a model host out of the record | Refused in competition runs; the start event records `connector_source` and `hosts_sha256` (PRD-01) |
| `--prompt` | A participant still pastes only the prompt; the prompt file holds the token by mistake | The framed line without `--prompt`; the `GH_TOKEN` value is masked by exact bytes on the wire in every build (ledger M1, PRD-05), and the first-message file is 0600; other keys in it are masked only where a secret rule matches |
| Status reads files only | A recorder that writes `status.json` but is wedged looks alive | `writer_slow` from PRD-02; `updated` older than 10 s gives "not answering" |
| Ending the harness | Grandchildren outlive it | PRD-02 ends the whole tree and names outlived programs; the end screen says so |
| Live partial loss | One subagent's calls go to an unlisted host in short calls while others are written | Not detectable live without parsing; `wire_keeps_up` after the run (PRD-08); the per-host upload rule of PRD-03 catches big contexts |
| No sleep lock under WSL or in containers | The machine sleeps mid-run | One honest line, never `PROBLEM:`; the guide says to set Windows sleep to Never |
| `caffeinate -s` | Lid close on a MacBook still sleeps | Line says "lid open"; unverified for clamshell mode |
| WSL2 | Not tested in this build (Linux sandboxes only) | Folder gate and opener are copied unchanged; marked unverified |
| Recover runs without a watchdog | After `goalcup recover` nobody holds the sleep lock, stops helpers or ends the harness at the deadline; the run certificate key sits on disk for the run | The recorder seals at the harness's exit or the deadline (PRD-02 "watchdog only" row); the harness is not ended, as when the watchdog dies today. The key file is 0600 and accepted by the owner (R3b): a participant who meddles with it breaks the rules anyway |
| Size | About 2,990 src lines against PRD-00's CLI row of about 2,900 (the 80 over is `recover`, R3b) | The device flow, sleep lock and folder rules are copied, not new; the sleep lock, `--prompt` and WSL rules come in the third slice and can be dropped if the day runs out (the old behaviour without them is still honest) |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Restarting before "off" | 30 s | PRD-10 |
| `status.json` not answering | 10 s | PRD-10 with PRD-02 |
| No-request alarm | 10 minutes with session files growing; 5 minutes after launch with nothing written | PRD-10 |
| Tunnel hosts on screen | 5, listed hosts always | PRD-10 |
| time-left freshness | 11 minutes | PRD-10 with PRD-06's heartbeat cadence |
| Quick exit after a resume | 30 s, non-zero exit | PRD-10 |
| No-prompt question default | yes | owner (PRD-00 question 2) |
| File flags in competition | refused | owner (PRD-00 question 4) |

## Asks of other PRDs

Revision 1's asks, as ruled by PRD-00 and PRD-01 revision 2: A1 (`hosts`, `tunnel_hosts`) taken in
PRD-01 section 9; A2 (`recovered`) taken in section 11; A3 (`mask_failed` is 12) taken; A4 to A6
taken (command list, `GOALCUP_HOME`, `GOALCUP_SERVER`, seams S9 and S11); P1 to P4 are PRD-00 owner
question 2 (old 8 to 11). N1 to N6 of PRD-02 are taken in seams S4 and S8; N4 of PRD-06 is seam S9 `server_word`.

Revision 2's asks, all ruled by PRD-00. N9 (revision 4, ledger R3b) is open and goes to the replan's
Amendments note:

| # | Of | Ask | Ruling |
|---|---|---|---|
| A7 | PRD-01 | Each `tunnel_hosts` value gains `listed`: the GAP reason name (`tls_refused`, `pinned`, `upstream_untrusted`, `protocol_other`) when the host was listed, null otherwise | R3-4; PRD-01 section 9 has it |
| N7 | PRD-02 | `status.json` `written` carries every COUNT key as entry totals, and `notes` keeps the first note of each `what` beside the last 20 | R3-2; PRD-02 section 6 has it |
| N8 | PRD-02 | Sealed `status.json` carries `cut_files {files, bytes}`, `outlived`, `seal {reason, workspace_head}` and `after_seal {bytes, hosts}`; `close_dead_entry` writes the same sealed file | R3-2; PRD-02 section 6 has it |
| C1 | PRD-11 | `update_off` is a key of its own, always written by setup with no question (ledger H9); `connector::load` takes a path for `--connector` and a host file for `--hosts`; keep `first_screen`, `warn_missing`, `skills_dir` as PRD-00 seam S3 says | R3-11; closed: PRD-11 has `update_off` and `skills_dir`, and `load` and `plan` replace `PlanCtx` |
| T1 | PRD-12 | `connectors/tiers.toml` shape: one table per connector id with `tier` (`verified`, `not verified`, `not supported`), `version` and `reason` | R3-11 |
| W1 | PRD-15 | "Part" is one `start` or `continue`; the compressed files are "record files" in the guide | R3-11 |
| N9 | PRD-02 | For R3b: keep the run certificate key in a 0600 file in the run folder for the whole run (PRD-02 step 3 says today the key never touches disk), and give `watchdog::recover(entry_dir, facts) -> Result<Recovered, NotRecovered>` (`PortTaken`, `KeyMissing`, `NoReady`) that binds the part's port, starts the recorder alone with those facts and waits for `ready`. For R3a: the null-mode retry is 30 s, 1, 2 then 5 minutes, never more than 5 minutes apart (PRD-02 step 9 says 5 doubling to 10) | Met: PRD-02 now carries R3a and R3b (section 8 steps 8 and 9, section 8.1, `rec/run-ca.key`) |

## Builder notes

Order of work (PRD-00 slices):

1. **Phase 0 (publish within the first hour):** `main.rs` with every command and flag as stubs (the
   hidden recorder entry, `--connector`, `--hosts`, `setup --yes`), each calling a function in the
   owner's module; `cli/mod.rs` (paths, env); `cli/log.rs` (seam S11, stable by H+1.5); `cli/text.rs`
   skeleton with the three contract words; `start` with `GOALCUP_SERVER=off` against PRD-02's stubbed
   `watchdog::run` and PRD-11's stubbed `LaunchPlan`.
2. **Core slice (blocks phase 1a):** `start` and `seal` with `GOALCUP_SERVER=off`; `status` and the
   end screen from fixture JSON files, before the pipeline exists; status lines; the file flags
   (seam S16, stable by H+3; the pad's dodger case and PRD-14's e2e test use them).
3. **Second slice:** the device flow and open against staging (PRD-14's fake Worker for tests);
   `upload` dispatch; `setup --yes` (seam S15, stable by H+4, needed by the pad); the `GH_TOKEN` mask
   value call (the other two only after the owner's yes to PRD-00 question 3).
4. **Third slice:** `continue` with the resume rule, `time-left`, interactive `setup`, `--prompt`
   composition, the sleep lock, the WSL folder rules; `skills/goal-cup/SKILL.md` without the MCP
   paragraph; delete `skills/goal-cup-setup/`.

Traps from the audit:

- Count only what `status.json` calls written. Never add a counter in the CLI (run 1's
  `record_generation_request`, `finish.rs:405-444`). Never call the number "calls".
- Print the end screen only after `sealed` and after the watchdog gave the terminal back: run 1
  printed it over Codex's TUI.
- Never print or log the run credential or a token; there is no proxy credential in 1.0. The
  first-message file is 0600; quote every printed path (macOS `Application Support`).
- A missing clipboard tool is normal (sandboxes, SSH). Never fail on it.
- `PROBLEM:` only for something the participant can act on or must report. Sleep without a lock in a
  sandbox is not one.
- Keep "shows the code XXXX-XXXX." and "Recording in chat mode" byte for byte; the pad reads them.
- Do not stop helpers, end the harness or chase descendants in the CLI: PRD-02's watchdog does all
  three. Do not read harness session stores to decide a resume.
- `goalcup recover` restarts only the recorder. It never restarts or signals the harness, never takes
  the terminal and never writes the key file (PRD-02 does).
- Nothing before the launch may time out sooner than the approval code (15 minutes, PRD-17 P14).
- No harness names in `src/cli/`: every harness-specific word comes from connector data or plan facts.
- Keep `Args::try_parse()` (`main.rs:256` today) and map every parse error to exit 1; clap's own
  `parse()` exits 2, which the help footer does not list.
- The run credential lives in `runs/<run>/credential` (0600), never in `server.json`, so
  `status --json` can print `server.json` whole.
- Builds and `tests/cli_*.rs` run in Vercel Sandboxes (ledger B1), never in the cloud coding
  environment.

How a watcher proves it in a real run: start the pad with `GOALCUP_KEEP_PARTS=1`; every 10 s save
`goalcup status --json`, and after each `--kill-recorder` kill run plain `goalcup status` once a
second for 60 s and save each output; at the end save the terminal's last screen; run `goalcup-verify check --json`
on the entry. Compare every number on the end screen with the check (acceptance 1), and every "Off:"
change in the saved status files with the `recorder_down` GAP times in `goalcup-verify dump`
(acceptance 2). Run `goalcup time-left` from the box during the run (acceptance 7).

## Review answers

This section is history (alignment X2-11). The body above, the ledger and the alignment answers below
win over it. It may move to `REVIEW-HISTORY.md` in one later step by one agent.

Sources: consistency check (CC), review A (RA), review B (RB), and asks other PRDs made of PRD-10.

| Item | What it said | Answer |
|---|---|---|
| CC 8 | PRD-00 table still lists `login` | Done in PRD-00 revision 2. No change here |
| CC 12 | `src/watchdog.rs` is a folder | Accepted. Must-not-touch says `src/watchdog/` |
| CC 23, RB A3 | P1 to P4 sit in three places | Accepted. They are PRD-00 owner questions 8 to 11; this PRD only cites them |
| CC 28, CC 77, RB C2 | Gap numbers and words for `write_error`, `upstream_untrusted`, `mask_failed` | Accepted. Section 5 table uses PRD-01's final 1 to 15; 8 and 9 shown as reserved; a full disk is off time with cause `write_error` |
| CC 31 | Seal reason `recovered` | Accepted. Section 5 rule 7 |
| CC 36 | `pinned_host` dropped | Accepted. The line keys off the GAP reason in `tunnel_hosts` and the `ca_refused` note (ask A7 to PRD-01) |
| CC 38 | `hosts`, `tunnel_hosts` | Accepted; already in PRD-01 section 9 |
| CC 41 | Proxy credential | Accepted. No credential exists; the trap line now says so |
| CC 47, CC 76 | Helpers and harness end have no owner | Accepted. PRD-02's watchdog owns `plan.before`, `plan.after` and ending the harness; start step 7 and the ending paragraph are gone; only the words stay |
| CC 48 | `status.json` keys, `watchdog.pid`, log | Accepted. Section 4 reads PRD-02's S4 keys; `goalcup seal` uses `rec/watchdog.pid`; S11 is section 12. Asks N7 and N8 for what is still missing |
| CC 61, CC 73, RB C14 | Connector fields cut by PRD-11 | Accepted PRD-00 S3: `no_prompts`, `first_screen`, `warn_missing`, `vendor` stay. The `warn_missing` line now keys off PRD-04's `native_missing` note. Ask C1 to PRD-11 |
| CC 71, RB C8, PRD-06 P5 | `upload.json` key collides | Accepted. `time-left` reads `server_word`; the finish line uses `upload_by`; the "server closed" line is in section 5 rule 8 |
| CC 72 | `server.json` scope | Accepted. Per run; fetch output is `fetch.json` (section 2) |
| CC 74, CC 75, RB C12, PRD-12 ask 4 | Flags missing, one override | Accepted. `--connector`, `--hosts` on `start` and `continue`; no `GOALCUP_CONNECTORS_DIR`; refused in competition runs (owner question 15) |
| CC 78 | Sample date before the start window | Accepted. Sat 14 Nov |
| CC 86, RB T2, RB C15 | Hook-based real-run proofs | Accepted. Old acceptance 2 (null mode) and 3 (memory guard) moved to `cli_status.rs` fixtures; acceptance 2 is now a plain `kill -9` run. No proof here needs `test-hooks` |
| CC 98, RB M8, PRD-12 ask 5 | Tier source | Accepted. `connectors/tiers.toml` (PRD-12), read at build time; ask T1 for its shape |
| CC 105 | `failed` means null mode | Agreed; section 4 null row says "The server is told" |
| RA 5 | Panic text and core files | Partly. Core dumps and the panic hook are PRD-02's. Here: S11 takes no free string, so a panic message or value cannot reach `goalcup.log` (section 12) |
| RA 6 | Unlisted model host is a silent tunnel | Partly. The live line (requests stopped while session files grow) and both `long_tunnel` lines are in section 4. A live line for a partial loss is rejected: it needs per-call parsing, which the recorder never does; the verifier does it after the run |
| RA 7 | Local model server | Accepted. `local_model` note line (section 4) and the section 6 bullet; nothing built around it |
| RA 8 | Processes that outlive the harness | Accepted for the words: the `after_stop` and `outlived` lines (section 5 rule 6). The mechanics are PRD-02's |
| RA 9 | SQLite copied only at the seal | Words only here: the recovered-part line says the database copy was not taken (section 5 rule 7). The mechanics are PRD-04's and PRD-02's |
| RA 15 | Worker outage near the end | Accepted. "Run goalcup upload before <time>" (section 5 rule 8) |
| RA 16 | Auto-update switches in settings | Accepted. Setup writes them with the no-prompt values into the isolated home (owner question 8); ask C1 to PRD-11 |
| RA 3, PRD-05 ask 2 | Fail-closed masking | Accepted for this PRD's part: `harness_values` call in start step 6; status and end screen show `mask_failclosed_bytes` when above 0 |
| RB B3 | PRD-10 is too big | Partly. Order of work follows PRD-00's slices: core slice is `start`, `seal`, `status`, end screen with `GOALCUP_SERVER=off`; `setup --yes` moves up to the second slice because the pad needs S15 by H+4; `--prompt`, sleep lock and WSL rules are last and can be dropped. Size is about 3,040, stated in Risks |
| RB C13, PRD-12 ask 3 | `setup --yes` | Accepted as S15 (section 1, section 7, acceptance 6) |
| RB A4, RB O3, RB M2 | Phase 1a needs `start` and `main.rs` stubs early | Accepted. Builder notes step 1 |
| RB M7 | Schemas shaped by two PRDs | Accepted as PRD-00 S4 and S9; this PRD reads them and asks only N7, N8 |
| RB W1 | Agent may not run `time-left` | Accepted. The watcher runs it in the pad box (acceptance 7) |
| RB A11 | Dodger exit-code pass line | Accepted for this PRD: acceptance 3 checks the screen and status only, not a tier line per case (the tier is per harness, the dodger case is a host-list copy) |
| PRD-02 ask 6 | Drop helper stop and harness end; call `watchdog::run` | Accepted (section 3 step 9) |
| PRD-03 ask 5 | Words for `parse_stopped`, `long_tunnel` upload; pinned line off GAP | Accepted (section 4) |
| PRD-04 interfaces | `native_missing` words, `cut_files` | Accepted (sections 4 and 5); ask N8 for `cut_files` in sealed status |
| PRD-11 risks rows | HOME and XDG swaps, version change at `continue` | Accepted, from plan facts with no harness names (sections 4 and 3.1) |
| PRD-13 rows | Session sniffing on `continue`; pasted `goalcup mcp` lines | Decided: sniffing is replaced by the written-count resume rule with D73's quick-exit rule (section 3.1), checked against `phases.rs:296-361` and `launch.rs:664-771`; `goalcup mcp` leftovers get no compatibility, one guide line |
| PRD-15 ask 1 | "Part" means two things | Accepted. "Part" is an entry; files are "record files" on screen (section 1, ask W1) |
| PRD-17 P14, S7, X4, X6 | Lessons routed here | Accepted: no pre-launch timeout under 15 minutes; deadline from the server's `seconds_left` on the boot clock; `home_busy` kept; the resume rule does not trust a session file alone |
| User request (30 Oct) | Is a gateway just an HTTPS proxy with another endpoint; one system, no patchwork; dodgers on the current way | Yes. Section 6 shows one pipeline and three cases; the participant does nothing extra for a gateway; dodgers get tunnels, session files and an honest label |

## Alignment answers

Fix lists g1, g2 and g3 of the alignment pass, checked against OWNER-DECISIONS.md, PRD-00 revision 4,
PRD-01, PRD-02, PRD-04, PRD-05 and PRD-11 as they now stand, and the code. No locked ledger item changes.

| Id | Answer |
|---|---|
| L4-1 | Applied for this file. Start step 6 calls `mask::workspace_values` only (the `GH_TOKEN` value, ledger M1); `env_values`, `harness_values` and the `skipped` line wait for the owner's yes to PRD-00 question 3. Owner decisions, setup step 4, section 13, interfaces, the `--prompt` risk row and builder step 3 say the same |
| F3 | Applied for this file: `cli_commands.rs` names `second_start_on_one_home_refuses` (PRD-17 HF4) |
| L3-1 | Partly. Auto-update is no longer part of the no-prompt question, and ask C1 says `update_off` is its own key. Which step writes it follows PRD-00 question 2 and seam S15 as already revised (G3-X16): setup writes it always, with no question; writing it from `plan` at every start is not taken here, so PRD-10 and PRD-00 agree. If PRD-11 takes the `plan` form, the replan picks one |
| F4 | Applied: citations read PRD-17 HF4 and HF6 |
| L2-12 | Applied as PRD-02 now has it: the sealed `status.json` `after_seal {bytes, hosts}` gives one line on the end screen and in `goalcup status` (section 5 rule 6; section 4 row) |
| L3-6 | Rejected: X2-03 deletes the model-host reachability warning from setup outright (ledger L2), so there is no line to reword |
| L3-11 | Applied: the vendor line keys off a written host of kind `vendor` on the shared list (the `conn` meta kind, PRD-03 section 12); `vendor` is gone from the connector fields read here |
| G2-07 | Applied with G3-X41: section 3 step 1 and the PRD-11 interface row call `connector::load` then `connector::plan(&connector, &env, home)` |
| P04-5 | Applied: a plain line for note `native_other_device` in section 4 and section 13; PRD-04 interface row |
| X2-03 | Applied for this file: no `--prompt` size limit (UTF-8 check kept); setup step 4 drops the model-host probe and the battery line; files table `setup.rs` 270, `text.rs` 420, total about 2,910 against PRD-00's about 2,900. The sleep lock stays (owner item) |
| G2-33 | Applied: status block and end screen show `req_whole` beside `model_requests` when `req_holes` is above 0 (section 4 bullet, section 5 rule 3, `cli_finish.rs`) |
| P05-15 | Applied, only with the owner's yes to question 3: start step 6 shows "N key values were not added to masking." when `skipped` is above 0 |
| X2-08 | Applied: the 0.7 `goalcup mcp` paragraph and its risk row are gone; one release-notes line (PRD-15) |
| X2-11 | Partly. This pass edits only this file, so `REVIEW-HISTORY.md` is not written here (many writers of one new file would race). "Review answers" now opens with a line saying it is history and that the body, the ledger and these answers win; the Asks section marks every ask ruled. The move may run once, later, by one agent |
| G3-X16 | Applied: setup always writes `update_off`, with no question, with or without `--yes`, and after an "n" (section 1 row, section 7 step 3, acceptance 6, `cli_setup.rs`, interface row, ask C1) |
| G3-10a | Applied: `--prompt` adds the push block on any run with `--repo` (competition and `--test`); the `GH_TOKEN` check is for any run with `--repo` |
| G3-10b | Applied: the prompt file lives outside the project folder (start step 2, section 8, the `Ready.` block, help footer, `cli_folder.rs`) |
| G3-X20 | Applied: the screens print only what `written`, `off`, the ruled notes and sealed keys hold; the last-span times, the "under heavy load" cause and the after-stop host names are gone (the after-seal hosts stay: PRD-02 now holds them). Asks A7, N7, N8 are marked ruled by R3-2 and R3-4 |
| G3-X17 | Applied: both `long_tunnel` lines and section 6 say `goalcup seal`, then `goalcup continue`; `cli_status.rs` checks the order |
| G3-10c | Applied: acceptance 2 and the watcher note use a plain `goalcup status` once a second for 60 s after each kill |
| G3-10d | Applied: `cli_commands.rs` 15-minute pre-launch timer case (PRD-17 P14) and `cli_start.rs` deadline case (PRD-17 S7, beside PRD-06's segment test from F3) |
| G3-10e | Applied: `:500-510`, checked against `finish.rs` (the `recording_works_line` function) |
| G3-10f | Partly. Appendix items read "app. H 1", seams read "seam S3", owner questions carry their PRD-00 revision 4 number with the old one. Ask T1 keeps its id (ids stay stable, and PRD-00 R3-11 cites it as T1); it is always written as an ask of PRD-12 |
| G3-X40 | Partly: the Size row now compares with PRD-00's current CLI row (about 2,900), not 3,040, which X2-03 changed |
| G3-10g | Applied: the tier line for a verified harness on another version |
| G3-X37 | Applied: `Host list from a file: <path> (not verified).` (section 1, acceptance 3) |
| G3-10h | Partly: the "(your agent's and its subagents')" words are dropped instead of being gated, because no connector field says which harness keeps one file per subagent and adding one is new scope |
| G3-10i | Applied: the run credential is `runs/<run>/credential` (0600); `server.json` never holds it; `--json` prints `server.json` whole |
| G3-X38 | Applied: start step 8 installs the skill only under `{home}` |
| G3-10j | Applied: acceptance 1 compares after the screen's rounding |
| G3-10k | Applied: acceptance 3 adds the no-request `PROBLEM:` line from minute 5 (the dodger case writes no request at all); acceptance 10 is a Codex seal and continue with the resume arguments |
| G3-X39 | Applied: acceptance 7 runs `sh -lc 'goalcup time-left'` with the harness's `HOME` and environment |
| G3-10l | Applied: acceptance intro and builder traps name Vercel Sandboxes (ledger B1) |
| G3-10m | Applied: section 1 exit codes and a builder trap keep `Args::try_parse()` with exit 1 |
| G3-10n | Applied: section 10 and start step 9 add `GOALCUP_TIME_LEFT_FILE` to `plan.env` before `watchdog::run` |
| G3-10o | Applied (same change as X2-03): the 64 KiB refusal, its test case and its default row are gone |
| G3-X41 | Applied (see G2-07) |
| R3a, R3b, T4 carried in | R3a: the null-mode status line no longer says "failed 3 times in a row"; it says goalcup passes traffic unrecorded for now and tries again at most 5 minutes apart until the run ends. R3b: new command `goalcup recover` (section 1 row, section 3.2: checks for an unsealed entry, no live watchdog, no live recorder, a live harness, a free port; success and refusal lines), the both-dead state line splits into "Run: goalcup recover" while the harness is alive and "goalcup continue" once it is gone, the stale row and `continue` defer to it, help footer, message list, `recover.rs` (80 lines), risk row, builder trap, acceptance 11 and `cli_recover.rs`; ask N9 to PRD-02 for the key file and the recover entry. T4: acceptance says no proof calls an outside API; acceptance 11 passes from the record only; the history check stays PRD-12's |
| B10 sweep (30 Oct) | Applied. `cli_finish.rs` checks the "nothing was lost" claim is present or absent, never the sentence (PRD-14 section 2) |

## Amendments

### 2026-10-30, replan after PRD-17 phase 0 slice (merge 1423f8f9)

- **Two cases the lessons gate waits on** (lessons log 10:45Z; `LESSONS.md` marks them `(name
  pending)`). Name each in the night log when written; the lessons lane puts the name in
  `LESSONS.md`:
  - PRD-17 SC1 (`cli_folder.rs`): a project folder inside `$GOALCUP_HOME`, or one that holds it,
    also through a symlink, is refused (checked on the spelled and the resolved path).
  - PRD-17 SC6 (`cli_finish.rs`): a `status.json` with no seal never prints the finished line.

### 2026-10-30, replan after PRD-05 phase 0 slice (merge 82484521)

- **S2 is on integration as the stub.** Where PRD-10 builds a masker (`Masker::new`), it passes a
  typed slice (`&plan.mask_rules`), never a bare `&[]`: the stub is generic over the rule type until
  S3 lands, and PRD-05 switches it to `&[connector::MaskRule]` then (PRD-05 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 1 (merge d55030ee)

- **Slice 1 is on integration:** `main.rs` (eight commands, every flag, the hidden
  `recorder [--null]` dispatched to `pipeline::run_recorder(null) -> ExitCode`), `cli/mod.rs`,
  `cli/log.rs` (S11) and `cli/text.rs` with the three contract words. Every command body is a stub
  that prints `<command> is not built yet` and exits 3; the competition refusal of `--connector` and
  `--hosts` already runs (exit 1, before anything is read).
- **Builder notes step 1's last item moved to step 2.** `start` with `GOALCUP_SERVER=off` is the first
  item of the core slice. It needs `watchdog::run` (S8) and `connector::load` and `plan` (S3); the
  watchdog scaffold on integration has neither yet (its module doc says `run` and `recover` follow S3).
  Build against PRD-00's final shapes meanwhile and poll as the lane rules say. PRD-00's core slice row
  already lists `start` with `GOALCUP_SERVER=off`, so the build order does not change.
- **S11 as merged** (users PRD-02, PRD-03, PRD-06): `line(entry_dir, event: &'static str, facts:
  &[(&'static str, Fact)])` as section 12. `Fact` has a seventh variant, `Io(io::ErrorKind)`, which is
  section 12's "logged by its kind name only". `Fact::Host` takes `cli::log::Host`, built only by
  `Host::checked(&str)` (host syntax; IP literals, `localhost` and a port allowed). When S3's
  `connector::hosts::Host` is on integration, PRD-10 adds `From<connector::hosts::Host>` in `log.rs`.
  `Fact::Path` carries runtime text the caller answers for: a path goalcup built (home, run or entry
  paths, or a file flag the participant gave), never a value from traffic or the environment. Section
  12's "There is no free text" reads with these two caller-vouched facts; the rule that no header
  value, body byte, token or credential reaches the file is unchanged. `line` never creates a
  folder: a line for a missing entry folder is lost, by design.
- **`cli::home()` returns `Result<PathBuf, &'static str>`**, always absolute, else `text::NO_HOME`. A
  relative `HOME` or `XDG_CONFIG_HOME` is ignored; a relative `GOALCUP_HOME` is accepted and made
  absolute against the current folder (review A1's allowed option). So `GOALCUP_HOME=gc` started in
  the project folder gives `<project>/gc`, and only PRD-17 SC1's refusal of "a project folder that
  holds `$GOALCUP_HOME`" stops it: SC1 in `cli_folder.rs` is load-bearing. The core slice's SC1 case
  takes the home from `cli::home()`, never from the raw variable, and includes a relative
  `GOALCUP_HOME` among its inputs in the same test (no new test file, ledger B10).
- **Spawn names:** `cli::RECORDER_SUBCOMMAND` and `cli::NULL_FLAG` are the recorder's argv; `main.rs`
  derives its parser from them and one unit test parses the watchdog's argv. PRD-02 spawns with them.
- **Open nits for the next PRD-10 touch** (the core slice), none a behaviour change:
  - A8: the `home()` doc says the SC1 guard is what protects a relative `GOALCUP_HOME`. When
    `std::path::absolute` fails (the current folder was deleted), the refusal must not say "HOME is not
    set to a full path": give that case its own wording in `text.rs`, or word `NO_HOME` to cover both.
  - B13: the header of `tests/cli_log.rs` drops "no free text" (no test there checks it; the two
    `compile_fail` doctests in `log.rs` do) or names what the file tests.
  - B14: `parse_error` in `main.rs` as one match whose arms each yield the `ExitCode`; `cli/mod.rs`
    uses one import style for the `text` constants.
- **Own file estimates** (the row total holds, see PRD-16 note): `main.rs` 190 src lines against 160;
  `cli/mod.rs` 152 against 130, before pid liveness; `cli/log.rs` 202 alone against 140 for
  `skill.rs` and `log.rs` together (about 50 lines are `Host::checked`, about 25 are doctests). When
  S3 lands, check whether `Host::checked` can shrink to the `From` impl plus the IP, `localhost` and
  port cases; keep `skill.rs` small.
- PRD-17 SC1 and SC6 names are still pending: `cli_folder.rs` and `cli_finish.rs` come with the core
  slice.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 2 (merge b9fa50f8)

- **S3 is on integration.** `connector::hosts::Host { name, kind, pinned }` is there, so
  `From<connector::hosts::Host>` for `cli::log::Host` can land. `hosts::check` is a stub (label shape
  only) until PRD-11's hosts slice; `Host::checked` keeps its IP, `localhost` and port forms until then.
- **Refusals to word** (file, key, home, program and the wanted prefix only, never a value):
  `Refusal::SettingSet { file, key }`, `HomeLocked { home }`, `VersionMismatch { program, want }`.
- `goalcup setup` calls `connector::write_settings(home, &c.no_prompts)` and
  `write_settings(home, &c.update_off)`. It writes nothing until PRD-11's launch slice, so a setup test
  that reads the written file waits for that merge. After that merge both lists name `{home}` files
  only (PRD-11 note, F1b). `start` with `GOALCUP_SERVER=off` still needs S8 `watchdog::run`.
