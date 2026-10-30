# PRD-10: CLI and participant experience

Id: PRD-10. Version: goalcup 1.0. Date: 30 October 2026. Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md) (binding).

## Goal

Seven commands, about 70 messages, and one end screen that says exactly what was written, what was
lost, how long recording was off and what is still on this computer. Nothing on screen is ever
wider than what the record holds.

## Points served

6 (lead: tell the participant plainly), 3 (no start refusal that no point needs; the sleep lock and
setup never block), 5 (the upload lines and `goalcup upload`), 7 (setup and start on macOS, Linux and
WSL2, the way people run their harness), 1 and 2 (the "no model request written" and "session files"
lines catch a harness that dodges the proxy), 4 (the token stays in its file; the push block never
holds it).

## Owner decisions it carries out

- Counts: the participant, status, heartbeat and server see only written counts and off time
  (BRIEF3). Run 1 printed "Recording works: 1736 model call(s)" for 842 written (run report,
  finding 4). That line is gone.
- Version is **1.0** (owner, 29 Oct night; "version 1 sure" in the relayed request).
- Answer 6: no need to print the `goalcup mcp` setup lines. In 1.0 the command goes too: it serves
  no point, and everything in the after-Edition-01 list that serves no point moves into this build
  (BRIEF3).
- Disk full: no special handling. Every disk message, the disk start refusal and the disk pause go.
- One system for every harness, no patchwork: a gateway is a host on the one list, and the
  participant sees it exactly like a direct key. A dodger gets an honest line and its tier, never a
  false "recording works" (PRD-00, "One system for every harness").
- Hosts the participant registers are on the one host list (29 Oct night). This PRD gives them the
  command to register one.
- AGENTS.md hard rules stand: records and never manages the agent, D66 `goalcup time-left` (pull
  only), never write into `~/.claude` or any app-wide config, no harness names in core code.
- Push block (run 1 finding 8) stays a participant step: revision 2's O10 settled "wording only", and
  RULES says the agent makes the repository itself (`docs/RULES.md:142-151`). This PRD makes it one
  paste instead of two.

## Today

| What | Where | Fact |
|---|---|---|
| Commands | `crates/goalcup-cli/src/main.rs:81-240` | 11 plus hidden `git-credential`: setup (alias `prep`), login, start, status, time-left, mcp, continue, upload, entries, seal, doctor |
| CLI source | `crates/goalcup-cli/src/commands/**`, `src/participant*`, `src/main.rs` (13,758 lines src and inline tests, `wc -l`) | Plus `src/account/` 1,046, `src/run/` 4,978, `push_lines.rs` 213, `ssh_keys.rs` 241, `host_log.rs` 253, `skill.rs` 162, `prep_file.rs` 88, `entries.rs` 208, `folder_gate.rs` 224 |
| Messages | appendix H section 7.1 | About 330 places put words in front of a participant; a lean client needs about 60 (H section 5) |
| The false finish line | `src/commands/start/finish.rs:437-444`, built at `:487-495` | "proxy admission: ..." then "Recording works: N model call(s)" counted at selection (`record_generation_request`) |
| Run 1's actual end screen | `final-pane.txt` on `origin/smart/tender-clarke-qupne3` | 8 technical lines (admission counters, entry, seal.json, anchor, termination, trace, verify) printed over the still-drawing harness, then "Recording works: 1736", then "Upload done." No word on the hour of lost calls |
| Seal report | `src/commands/seal.rs:399-455` | "entry X: finished locally; its terminal record is N and seal.json binds it", anchor, termination, trace, `verify:` on a folder that was pruned at the seal (run report finding 6) |
| `goalcup mcp` lines | `src/commands/prep.rs:721-760`; `commands/mcp.rs` (474: 273 src, 201 tests) | Printed on every setup; four connector tables `[participant.time_left_tool]` |
| Push block | `src/participant/launch.rs:53-54`, `:96-118`, `:158-186`; `src/push_lines.rs:57-80` | 7 lines the participant pastes at the very start of the prompt. Run 1's prompt had none, so nothing was pushed (finding 8) |
| Sleep lock | `crates/goalcup-platform/src/awake.rs:93-106` (269 lines) | `caffeinate -i -s -w <pid>` on macOS, `systemd-inhibit --what=sleep:idle --mode=block` on Linux; none under WSL. Missing lock is a `PROBLEM:` line for the whole run (`src/run/mod.rs:562-565`, `participant/status.rs:511-513`); run 1 showed it from the start in the sandbox |
| Status problems | `src/participant/status.rs:319-344` | "the recorder restarted ... If this repeats, run goalcup status" (printed by `goalcup status`); "the recording has gaps"; never "recording is off" |
| Config dir | `src/config.rs:39`, `:71-89` | `$GOALCUP_CONFIG_DIR`, else `~/Library/Application Support/goalcup` on macOS, `$XDG_CONFIG_HOME/goalcup` or `~/.config/goalcup` elsewhere |
| Server | `src/server.rs:24` | `https://ingest.agentcommunity.org`; `goalcup login` stores the address and the account name only (H1) |
| Version | `Cargo.toml:18`, `src/lib.rs:136` | `0.7.0`, `CLI_VERSION = env!("CARGO_PKG_VERSION")` |
| Contract words | `src/participant.rs:76`; `participant/approval.rs:45-61`; `participant/status.rs:10` | `Recording in chat mode`, `shows the code XXXX-XXXX.`, `PROBLEM:`; read by the matrix scripts and the pad |

## Design

### 1. The commands after the cuts

| Command | 1.0 | What it does |
|---|---|---|
| `goalcup setup [--harness ID] [--record-host HOST]` | **keep, smaller** | Pick the harness, open it once in goalcup's own home to sign in, write the no-prompt settings there (section 7), check the machine. Without a terminal it asks nothing, opens nothing and prints the steps: that replaces `doctor` and `--json`. `--record-host` adds a model host to the participant's list |
| `goalcup start [--harness ID] [--practice] [--test] [--repo NAME] [--prompt FILE]` | **keep** | Checks, approval, open the run, then hand over to PRD-02's `watchdog::run`. `--harness` defaults to the one setup saved (H7). `--prompt` is new (section 8) |
| `goalcup continue [RUN]` | keep | Close a dead entry (`pipeline::close_dead_entry`), continue on the server, relaunch with the connector's resume arguments |
| `goalcup status [PART] [--json]` | **keep, rewritten** | One plain block from `status.json`, `upload.json` and `server.json`. Lists the run's parts (replaces `entries`) |
| `goalcup seal [PART] [--yes]` | keep | Asks, then signals the watchdog (SIGUSR1, PRD-02 section 8). On a dead entry: `close_dead_entry`, then the uploader |
| `goalcup upload [PART]` | keep | Dispatch to `upload::manual::run` (PRD-06 section 9) |
| `goalcup time-left` | keep (D66) | Prints the time left; the agent may run it (section 10) |
| `goalcup login` | **cut** | Grants nothing; the per-run approval is the account tie (H1). The server comes from `GOALCUP_SERVER` (section 2) |
| `goalcup mcp` | **cut** | Owner answer 6, H4. `goalcup time-left` and the goal-cup skill stay |
| `goalcup doctor` | **cut** | Its two remaining checks (connector loads, harness on PATH) are setup's harness step. Pins, daemon, decoders and SSH checks are cut with what they checked |
| `goalcup entries` | **cut** | `goalcup status` lists the parts |
| `git-credential` (hidden), `prep` alias, `setup --json`, `setup --dev`, `start --dev` | **cut** | Backward compatibility (F8) or pins, which are cut (PRD-13) |

Seven commands. Exit codes stay 0 done, 1 refused and fixable, 3 could not do its job
(`main.rs:49-60`); the sentence "never exits with 2 or 4" goes. Every error is one line
`goalcup: <text>` on stderr (`main.rs:275`).

The hidden recorder subcommand is PRD-02's (watchdog section 8 step 3); `main.rs` only dispatches it
and hides it from help.

### 2. Server, home and environment

| Name | Meaning |
|---|---|
| `GOALCUP_HOME` | Where runs live (`$GOALCUP_HOME/runs/<run>/<entry>/`, PRD-01 section 1). Replaces `GOALCUP_CONFIG_DIR` (no compatibility). Default unchanged: `~/Library/Application Support/goalcup` on macOS, `$XDG_CONFIG_HOME/goalcup` or `~/.config/goalcup` on Linux and WSL |
| `GOALCUP_SERVER` | The server. Unset: `https://ingest.agentcommunity.org`. A URL: that server (`http://` only for a loopback address, for the pad's local Worker). `off`: a local-only run, nothing uploaded, parts kept (PRD-06 `keep`) |
| `GOALCUP_KEEP_PARTS=1` | Keep every part (PRD-06 section 7). No flag: the pad and the 2-hour run set the variable |
| `GOALCUP_NO_BROWSER=1` | Print the approval address instead of opening a browser (kept, `main.rs:441-480`) |

Files PRD-10 writes, all mode 0600, in `$GOALCUP_HOME`:

- `setup.json`: the saved harness id and the no-prompt values written (replaces `prep.json`, which
  no longer gates a start: F4).
- `record-hosts.txt`: hosts the participant registered, one per line. PRD-11 reads it into the
  participant list of `HostList` (seam S3).
- Per run: `runs/<run>/server.json` (server, run, deadline, mode, repo) and the run credential, both
  written before the watchdog starts (PRD-06 interface), and `runs/<run>/first-message.txt` when
  `--prompt` was given (section 8).
- Per entry: `goalcup.log` (section 12).

### 3. `goalcup start`, in order

1. **Read the connector** (PRD-11 S3) for the harness: display name, tier and verified version, launch
   plan, home, first screen, helpers to stop, `warn_missing` rows.
2. **Check the folder, and say every problem at once** (F5). One refusal lists each problem with its
   fix, then "Nothing was created." Checks kept: folder empty except `.gitignore`, `.DS_Store` and
   `.env*` files (`docs/RULES.md:142-145`); every `.env` covered by `.gitignore`; not inside a Git
   work tree; not beside goalcup's own folders; a real run's `.env.local` has a plain `GH_TOKEN=`
   line (the push block's `sed` needs it, `launch.rs:181-182`); WSL1 or a Windows drive under WSL2
   (`src/folder_gate.rs:58-107`). Turned into one warning line each: synced folder (F4), NFS or SMB
   (the entry lock is an `flock`). Cut: the btrfs, ZFS, tmpfs, overlayfs and FUSE list (overlayfs
   and tmpfs are what containers and sandboxes use; nothing in 1.0 needs more than `fdatasync`,
   `flock` and rename).
3. **Real run: the repository** (`src/commands/run_repo.rs:66-165`, kept): public, empty, checked
   without a token before the approval, so a full repo does not waste one.
4. **Approval** in the browser (device flow, copied from `src/account/device.rs`, 389 lines, through
   PRD-06's `ServerClient`). One approval per run. "shows the code XXXX-XXXX." stays word for word.
5. **Open the run** (`POST /v1/runs`, PRD-07 S5). Write `server.json` and the credential.
6. **Mask values**: `mask::workspace_values(project)` and `mask::env_values(&plan.env)` (PRD-05). A
   `MaskError` refuses with fixed text before anything launches.
7. **Stop the connector's background helpers** that must not be running at start (PRD-11 data:
   OpenCode server, Hermes gateway, Codex app server, Droid). One line when one could not be stopped.
8. **Sleep lock** (section 9). Never refuses.
9. **Install the goal-cup skill** into the harness's own home (copied `src/skill.rs`, trimmed).
10. **Print the launch block** (section 8), then call `watchdog::run(plan, facts)` (PRD-02 S8). The
    watchdog binds, starts the recorder, waits for `ready` (10 s, first boot only), then spawns the
    harness. If the first boot fails, the harness is not started: "Goal Cup could not start recording,
    so <harness> was not started. Run the command again. If it fails again, write to <support> and
    name run <id>." (H section 5, start refusals).
11. **After the harness exits** (or the seal, or the deadline): wait for `sealed`, then the end screen
    (section 5) and the upload wrap-up (PRD-06 section 9).

Refusals cut from start (none serves a point; appendix H section 7.3): disk reserve
(`src/run/reserve.rs:289-299`), provider-host TCP probe (`phases.rs:107-140`; the no-requests line
catches an offline start), `prep.json` gate (`src/prep_file.rs:23-60`), `--practice` needing setup
(`server.rs:37`), pinned connector files (`phases.rs:95-103`; pins are cut), synced folder and Git
work tree under the run directory (`per_run_home.rs:299`, `:340`, `:356`; now warnings or gone with
the run home rework in PRD-11).

**The deadline and `goalcup seal` end the harness.** This is today's behaviour (`participant/ending.rs`
`TimeUp`, "was stopped"), kept with no proof machinery: SIGTERM to the harness pid, SIGKILL after
10 s. Leftover grandchildren are not chased (termination proof is cut, PRD-02 table "Deleted").
Owner confirmation asked (Asks, P4).

### 4. Status: `goalcup status`

It reads three files and never talks to the recorder: `status.json` (PRD-02 section 6, every 1 s),
`upload.json` (PRD-06 S9) and `server.json`. It works the same while a run records, after it ends,
and after a crash.

**Recording state**, decided in this order:

| Facts | Line |
|---|---|
| `mode: sealed` | `Recording: finished at 16:54. Nothing is recorded now.` |
| `updated` older than 10 s and the recorder pid is gone, watchdog pid alive | `Recording is restarting (since 15:43:15).` After 30 s: `Recording is off since 15:43:15. Your agent keeps working; this time will show as a gap.` |
| `updated` older than 10 s, recorder and watchdog both gone, no seal | `PROBLEM: Recording is off since 15:43:15: goalcup is not running, so your agent cannot reach the model service. Run: goalcup continue <run>` |
| `mode: null` | `PROBLEM: Recording is off since 15:43:15 (it failed 3 times in a row; it tries again at 15:53). Your agent keeps working. The server is told.` |
| `mode: losing` | `Recording is losing data now: 2 streams have a gap (12 MB so far). Your agent is not slowed. The end screen will say how much.` |
| `mode: writer_slow` | `Recording is behind: nothing written for 14 s. Your agent is not slowed.` |
| `mode: recording` | `Recording: on.` |

"Off since T" uses `off.since` from `status.json` when the watchdog wrote it (null mode), else the
`updated` time of the last `status.json`. Times are local with the zone, as today (`ends_line`,
`participant/status.rs:105-111`). The 30 s step matters: a normal restart is under a second (PRD-02
section 8 step 6), so "restarting" is honest for that window and "off" after it.

**The block** (the same shape in every state):

```
Your competition run with Codex: recording.
  Ends: Sat 14 Nov, 10:12 CET (09:12 UTC), 95 h 10 min left.
  Codex: running.
  Recording: on.
  Written so far: 905 model requests (api.openai.com 812, chatgpt.com 93), 9 with missing parts.
  Session files: 43 (your agent's and its subagents'), 48 MB.
  Off: 1 time, 2 min 30 s in total.
  Upload: the server holds everything up to 14:02 (1 min ago).
  Parts: 2. Part 1 sealed and uploaded. Part 2 recording.
  Problems: none.
```

- "Written so far" is `written.model_requests` with `written.hosts` (Asks, A1), and `req_holes`. It
  is never an offered or seen count. While a stream is losing, the lost bytes are already in
  `lost_bytes` (PRD-01 section 7), so the line can say so before the GAP exists.
- "Session files" is `native_files`; "your agent's and its subagents'" is plain truth for every
  harness that keeps one file per agent, and the recorder does not interpret which is which.
- Upload lines are PRD-06 section 10's drafts, used word for word.
- `--json` prints `{"status":<status.json>,"upload":<upload.json>,"run":<server.json without the
  credential>}`. Nothing is signed (signing is cut). The installer's run guard reads `status.mode`.

**Problem lines** (prefix `PROBLEM:` kept; one per cause, never one per tick):

| Cause (facts) | Line |
|---|---|
| No model request written for 10 minutes while the harness runs and `native_bytes` grew (Asks of PRD-02, N3) | `No model request was written for 10 minutes, but <harness>'s session files grew. It may not be using the recording proxy. Its session files are still recorded. If this is a model service goalcup does not know, see below.` |
| No model request at all 5 minutes after launch | `No model request was written yet. If your agent is working, it may not use the recording proxy.` (H: `problem_no_model_calls`, kept) |
| A `warn_missing` row has no session file after 5 minutes (PRD-04) | `goalcup has found no <row name> file for <harness> yet. Its session may be stored somewhere goalcup does not read.` |
| Note `long_tunnel` (PRD-03 section 9) | `<host> streamed data for 12 minutes and was not recorded. If it is a model service, run: goalcup setup --record-host <host>, then goalcup continue.` |
| Note `pinned_host` or `ca_refused`, or GAP `pinned` / `tls_refused` | `Not recorded: <host> refused the recorder (it checks its own certificates). Its traffic passed through unread. <harness> is "not supported" for this host.` |
| Vendor backend (`listed` host whose connector says so, PRD-11 data) | `<harness> runs its model calls on its vendor's servers. goalcup records what <harness> sends there, not the vendor's own model calls.` |
| Upload behind (PRD-06 `state`) | One line, PRD-06 section 10 (F9 merges five lines into this one) |
| `status.json` older than 2 minutes, no process alive, run still open | `goalcup has not updated this run since 13:40. The computer may have slept or restarted. To go on, run: goalcup continue <run>` (H: `problem_state_stale`, kept) |
| Agent exited, run open | `<harness> has exited (exit code 0). The run is still open until <time>. To go on, run: goalcup continue <run>` (H: `problem_agent_exited` without the disk branch) |

Cut from status (appendix H): the "Details:" block (`commands/status.rs:582-700`), `this_computer`
vitals, every disk line, `problem_recorder_restarted` and `problem_recorder_lost` (both reworded into
the state lines above; the circular "run goalcup status" goes), the push section (the block lives in
`first-message.txt` or `push-lines.txt` and the launch block), the sleep `PROBLEM:` (section 9).

### 5. The end screen

Printed once the harness has left the terminal (after the watchdog restored it), never over a running
TUI as in run 1. The numbers come from `status.json` in `mode: sealed`, whose `written` and `off`
equal the seal event's `totals` (Asks of PRD-02, N1), so the screen and `goalcup-verify check` agree
by construction (PRD-08 `counts_match_seal`).

```
Codex has exited (exit code 0).
Your run is still open until Thu 5 Nov, 14:54 UTC, 165 h 59 min left.
To go on, run: goalcup continue r632072599ee7b6fc (you can do it in another terminal now).

What this part recorded (on this computer):
  905 model requests written: api.openai.com 812, chatgpt.com 93.
  9 of them have missing parts: 1.5 MB was not recorded, under heavy load.
  43 session files, 48 MB (your agent's and its subagents').
  Recording was off 2 times, 2 min 30 s in total (last: 15:43 to 15:45).
  Not recorded, passed through unread: registry.npmjs.org (105 connections, 80 MB),
    ab.chatgpt.com (11, 24 MB), and 2 more hosts.
Upload: done. The server holds your whole record (95 MB). Removed here: 3 parts the server holds.
Details for support are in /home/you/.config/goalcup/runs/r63.../e1ba.../goalcup.log
```

Rules:

1. **Never "Recording works".** When `gaps` is 0, `off.ms` is 0 and no tunnel went to a listed host,
   the second line of the block is `Nothing was lost.` Otherwise each loss has its line, with numbers.
2. **Zero written requests** gives, instead of the first line: `No model request was written.
   <harness> may not use the recording proxy, or it made no call.` plus the tier line from the
   connector when it is "not supported" or "not verified" (section 6).
3. **Off time** is the sum of `recorder_down` gaps (PRD-01 section 7), with the count and the last
   span's start and end. "Under heavy load" appears only when the gap reasons are `memory_guard`.
   Every gap reason has plain words (table below).
4. **Not recorded** lists tunnel hosts by bytes, largest first, five at most on screen, the rest as
   "and N more hosts"; the whole list is in `goalcup.log`. A tunnel to a listed host (pinned,
   refused) is never in the "and N more": it is always named, because it may have carried model
   calls.
5. **Files cut at the seal** (PRD-04 `cut_files`): `3 session files were still growing at the end;
   their last 2 MB were not read in time.`
6. **Upload lines** are PRD-06 section 10's. "Removed here" names pruning plainly, so the
   participant is not surprised that the folder is small (run report finding 6). There is no
   `verify:` line on the participant screen. When every part is still local (`keep`), `goalcup.log`
   holds `goalcup-verify check <entry dir>`; otherwise it names the server copy (PRD-08 interface).
7. The technical lines of today (admission counters `finish.rs:437`, outage lines `:446`, the seal
   report `seal.rs:399-455`, "harness exited:", "uploaded: N bytes, acknowledged, and the seal")
   go to `goalcup.log` only, where they still exist (most are gone with the daemon).

**Gap reasons in plain words** (PRD-01 section 3; an unknown number shows as "other (n)"):

| Reason | Words |
|---|---|
| `memory_guard` | under heavy load |
| `recorder_down` | recording was off |
| `unsupported_encoding`, `decode_failed` | a reply in a form goalcup cannot read |
| `source_changed` | a session file was replaced or cut short |
| `tls_refused`, `pinned` | the program refused the recorder |
| `write_error` | the disk would not take more |
| `read_error` | a session file could not be read |
| `cut_at_seal` | still arriving when the part ended |
| `h2_unsupported` | a connection goalcup could not follow |
| `mask_failed` (Asks, A3) | goalcup could not check it for secrets, so it dropped it |

### 6. Gateways, registered hosts and dodgers: what the participant sees

The user asked whether a gateway is "just an HTTPS proxy with a different endpoint". Yes. To the
participant there is nothing to do for one:

- **Direct key or gateway on the shared list** (Vercel AI Gateway, OpenRouter, Cloudflare AI Gateway,
  cloud model services): written requests are counted under the gateway's host name, like any
  provider. Same lines, same end screen.
- **A host goalcup does not know** (a self-hosted gateway, a company proxy for models):
  `goalcup setup --record-host llm.example.com` adds it to `record-hosts.txt`; the next `start` or
  `continue` decrypts and records it. The line printed: `goalcup will record llm.example.com from the
  next start. Your harness must trust goalcup's certificate for it, which goalcup sets up.` Refused:
  an IP address, a sign-in host on the `never` list (PRD-03 section 3: point 4), and a pattern
  wider than one label. Removing a host: edit the file; the path is printed.
- **A dodger** (ignores `HTTPS_PROXY`, pins certificates, or runs inference on its vendor's servers):
  never hacked around. The start line names the tier from connector data (PRD-11, PRD-12):
  `Codex 0.165.0: verified with goalcup on 0.165.0.`, or `<harness> <version>: not verified. goalcup
  records what reaches it; goalcup status says if model requests are missing.`, or `<harness>: not
  supported. goalcup records its session files and the connections it can see, not its model calls.`
  The status and end screen then say what section 4 and 5 say. The same pipeline, an honest label.

### 7. `goalcup setup` on macOS, Linux and WSL2

Steps (was four with login; login goes):

1. **Harness.** Installed harnesses from the connector list (PRD-11), pick one (Enter takes the
   saved one). Prints `<harness> <version> (<path>)` and the tier line of section 6. A harness not on
   PATH: `<harness> is not installed. Install it, then run goalcup setup again.` (was
   `harness_executable`).
2. **Home.** "Press Enter to open <harness> in its Goal Cup home. Sign in there, then quit it."
   (kept, `prep.rs:763-870`). The home is PRD-11's isolated home; goalcup never touches `~/.claude`,
   `~/.codex` or any app-wide config (AGENTS.md hard rule).
3. **No-prompt settings** (H3, owner question P1). goalcup writes the connector's `no_prompts`
   values into that same isolated home, once, after one question: `<harness> will run for days with
   nobody watching. goalcup can set it to never stop and ask for approval (it can then change any
   file you can). Set this in goalcup's own <harness> home? [Y/n]`. Then one line says what it set
   and where. "n" keeps today's advice text (`prep.rs:911-944`). A harness that rewrites its file on
   first run is re-read and warned (H3 risk).
4. **Machine.** One line each, all warnings, none holds back `Ready.`: on battery (H11); the sleep
   lock (section 9); the server reachable (one HTTPS GET of `/v1/health`); the model hosts of the
   chosen connector reachable (TCP, a warning only: F3); `git` installed; WSL1 or a Windows drive.
5. `Ready.` and the next two commands:

```
Ready.
  Practice (10 minutes, no GitHub):
    d=$(mktemp -d) && cd "$d" && goalcup start --practice
  Your run, inside the start window, in your empty project folder:
    goalcup start --repo <you>/<repo> --prompt prompt.txt
```

Without a terminal, setup prints steps 1 and 4 and `Ready.` or the step to fix, and exits 0 or 1. It
opens nothing and asks nothing. The pad (PRD-12) and the installer smoke use this.

Cut from setup (appendix H section 7.2): the login step, the `goalcup mcp` lines
(`prep.rs:721-760`) and their copy markers, the SSH key and agent lines (`prep.rs:418-429`,
`ssh_keys.rs`), the disk step (`prep.rs:946-1020`), the old GitHub token notice (`prep.rs:437-446`),
`--json`, `--dev`, the `prep` alias, "N of 4 steps need you".

**Per OS** (facts the code relies on):

| | macOS | Linux | WSL2 |
|---|---|---|---|
| Install | `curl ... | sh` (PRD-14 owns `packaging/install.sh`); curl downloads are not quarantined, so the unsigned binary runs (`install.sh:12`) | same; glibc 2.35 or later (`install.sh:65-68`) | same, inside the Linux distribution; WSL1 refused |
| Binaries | `goalcup`, `goalcup-verify` (no `goalcup-daemon`) | same | same |
| PATH | one line that exports and appends to the right profile (H8, PRD-14) | same | same |
| `GOALCUP_HOME` | `~/Library/Application Support/goalcup` (has a space: every printed path is quoted) | `~/.config/goalcup` | same as Linux |
| Sleep lock | `caffeinate -i -s -w <pid>`: no idle sleep, no system sleep on mains; closing the lid still sleeps | `systemd-inhibit --what=sleep:idle --mode=block`; none in a container or without logind | none: "set Windows sleep to Never" |
| Browser | `open` | `xdg-open`; headless: `GOALCUP_NO_BROWSER=1` prints the address | the Windows browser (copied opener, `goalcup-platform/src/machine.rs:515-534`) |
| Clipboard | `pbcopy` | `wl-copy`, else `xclip` | `clip.exe` (`push_lines.rs:57-80`) |
| Project folder | anywhere outside a Git work tree | same | inside the Linux file system, never `/mnt/c` (`folder_gate.rs`) |
| Firewall | Little Snitch and the like ask once per binary; one guide line (PRD-15) | | |

### 8. The launch block and the push block (run 1 finding 8)

The push block stays the participant's (O10, `docs/RULES.md:142-151`): goalcup writes nothing into
the project folder. What changes is that it becomes part of **one paste**.

- **`--prompt FILE`** (new, real runs only). goalcup reads the file (UTF-8, at most 64 KiB, refused
  otherwise), builds `push block + blank line + prompt`, writes it to `runs/<run>/first-message.txt`
  (0600) and copies it to the clipboard when a tool is there. The launch block says:
  `Your first message (the push block, then your prompt) is on your clipboard and in <file>. Paste it
  once as your first message and press Enter.` The prompt is not recorded specially: it reaches the
  record on the wire like any message, masked like any byte.
- **Without `--prompt`** the block is printed as today and copied to the clipboard, under one framed
  line: `Your first message must start with these lines. Without them your agent cannot push, and
  you may not send them later.` (`docs/RULES.md:98-101`: after the prompt, only "keep going".)
- The push block text is today's `prompt_block` (`participant/launch.rs:158-186`) without change.
  The token stays in `.env.local`; the credential helper reads it at each push.
- `goalcup continue` never shows the block again when written requests are above 0 (the prompt was
  sent). With 0 it shows it again with "If you never sent your prompt, send it now" (today's
  `Opens::LastSession` branch). The pre-0.7.1 branch (`launch.rs:126-130`) goes.

The launch block keeps its shape (`participant/launch.rs:62-147`): run kind, "<harness> opens now in
this terminal", the connector's first-screen hint, what to type, the end time, the upload line (PRD-06
section 10, "Upload: to <server>, every 5 minutes."), the tier line (section 6), the sleep line
(section 9), and last `Recording in chat mode: <harness> opens now.` (`READY_MARK`, word for word).

Rejected: `git init` plus remote and helper in `.git/config` before launch (H6). It breaks "goalcup
adds nothing to the folder" and "the agent makes the repository itself", and O10 settled wording
only. Rejected: a token prompt that writes `.env.local` (H5); same rule, more code.

### 9. Sleep lock

- Copied from `goalcup-platform/src/awake.rs` (269 lines, about 150 kept) into `src/cli/awake.rs`:
  same tools and flags, the thread that owns the child, `PR_SET_PDEATHSIG` on Linux, `-w <pid>` on
  macOS, the 25 s worst case on a hanging D-Bus (`awake.rs:74-76`).
- Taken by `start` and `continue` for the whole entry, held by the watchdog process (the lock dies
  with it, which is correct: then nothing is recording anyway).
- **Never a `PROBLEM:` line.** A machine with no inhibitor gets one line in the launch block and one
  in status: `This computer may sleep during the run (no sleep lock: <why>). Keep it on mains power
  with the lid open.` Run 1's sandbox showed a `PROBLEM:` line for this from start to end, which
  taught everyone to ignore `PROBLEM:` lines.
- Sleep is not "off". A sleeping machine suspends the harness too; nothing is lost and no gap is
  written. Boot-time clocks keep the deadline right (PRD-01 section 6).

### 10. `goalcup time-left` (D66)

- Inside the harness: the launch adds `GOALCUP_TIME_LEFT_FILE=<entry>/upload.json` to the plan's
  env. The command reads the server's answer there (Asks of PRD-06, N4: `server` key with
  `seconds_left` and `asked_at`), subtracts the wall time since `asked_at`, and prints today's line
  (`run/time_left.rs` `left_line`, copied). Older than 11 minutes (two 5-minute heartbeats and a
  minute): "unknown right now", exit 1, as today (`time_left.rs:47-73`).
- In the person's own terminal: the newest open run's `upload.json`, same rule.
- The goal-cup skill (`skills/goal-cup/SKILL.md`, 34 lines) tells the agent the command exists. Its
  MCP paragraph goes. `skills/goal-cup-setup/` (106 lines) is deleted (H10).

### 11. Version 1.0

- `goalcup --version` prints `goalcup 1.0.0`. The number is the workspace version (PRD-16 sets
  `1.0.0` in `Cargo.toml`); `CLI_VERSION` stays `env!("CARGO_PKG_VERSION")`.
- The `start` event carries it (PRD-01 section 11 `goalcup` key); open and continue send it; the
  Worker's floor refuses 0.7 (PRD-07 section 6, PRD-00 question 8). The refusal text the Worker
  sends is shown as is, after "goalcup: ".
- The help footer (`main.rs:49-60`) becomes:

```
Steps:
  1. goalcup setup                                   once, until it says Ready.
  2. goalcup start --practice                         a 10-minute try, no GitHub (optional)
  3. goalcup start --repo <you>/<repo> --prompt FILE  your run, inside the start window
While it runs: goalcup status (in another terminal). If the agent stopped early: goalcup continue.
Guide: https://github.com/agentcommunity/goal-cup-proxy/blob/main/docs/PARTICIPANT-GUIDE.md
Help: goalcup@agentcommunity.org
Exit codes: 0 done; 1 refused, and you can fix it; 3 the command could not do its job.
```

- No 0.7 compatibility: an old `goalcup mcp` line left in a goalcup harness home makes that harness
  show a failed MCP server at its start and go on. The guide tells 0.7 testers to remove it (PRD-15).

### 12. `goalcup.log`

One plain text file per entry (`runs/<run>/<entry>/goalcup.log`, 0600), appended by the watchdog and
the recorder: boots and exits, null mode, every `note` event, stopped helpers, the full tunnel list
at the seal, upload errors. No header value, body byte, token or credential, ever. It replaces
`host.log` (`src/host_log.rs`, 253 lines; about 60 kept). It is not part of the record and is never
uploaded. The end screen and every refusal that needs support detail end with its path.

### 13. The message list

The contract words stay exactly: `Recording in chat mode`, `shows the code XXXX-XXXX.`, `PROBLEM:`.
Every other message is a plain function of facts in `src/cli/text.rs` (replaces `src/participant.rs`
and `src/participant/*`, 1,788 src lines, to about 500). Tests assert facts and the three contract
words, never whole sentences (AGENTS.md "Tests").

| Group | Kept (appendix H section 5 names) | Changed in 1.0 | Cut |
|---|---|---|---|
| Fragments | `CONTINUE_HINT`, `KEEP_GOING`, `STILL_OPEN`, `TIME_IS_UP`, `NOTHING_MORE`, `RUN_AGAIN`, `SUPPORT_ADDRESS`, `EARLIER_PART_UPLOADING`, `NOT_STARTED_HERE` | | `DETAILS_HEADING` (one "Details for support are in <log>" line), `FREE_OUTSIDE` |
| Approval | `approval_code`, `replaces_open_run`, `APPROVED`, `server_unreachable` | one intro line; one "approval failed" with a reason (denied, expired, interrupted) | `login_done`, "two approvals are normal" (6 places, H1) |
| Launch | `launch_block`, `RUN_GOES_ON`, `prompt_block` | clipboard line names the first message (section 8); tier, upload and sleep lines | pre-0.7.1 branch |
| Start refusals | `home_busy`, `still_recording`, `needs_terminal`, `project_of_open_run`, `cannot_record_model_service`, `repository_not_empty`, `dev_competes`, `server_answer_unusable`, `not_started_here`, `local_only`, `run_over`, `last_part_closing`, `continued_elsewhere`, harness not on PATH, WSL1 or Windows drive, run credential (two), "could not start recording, so <harness> was not started", `MaskError` text | one folder refusal that lists every problem; one "project folder is gone"; one "reinstall one release" install line | disk reserve, provider-host probe, `prep.json` gate, pinned files, `--practice` needs setup, the 11 internal launch guards (one "goalcup could not launch <harness>: <why>. Write to support."), daemon prefixes (`control.rs:785-810`) |
| Start warnings | default host warning, filesystem (NFS, SMB only), synced folder, shared home was open (rotate keys), Ctrl-C may end this command too | sleep line (section 9), tier line (section 6), helper not stopped | provider host unreachable as a refusal |
| Status | `ends_line`, `status_next`, `no_run_yet`, `problem_state_stale`, `problem_no_model_calls`, `problem_native_silent`, `problem_agent_exited`, `agent_finished`, `CHECK_NETWORK` | the state lines and block of section 4; one upload line | `STATUS_DETAILS`, `this_computer`, disk lines, recorder restarted and lost, sleep `PROBLEM:`, push section |
| End | `helper_not_stopped`, `PRACTICE_NEXT`, `UPLOADING_REST`, `UPLOAD_DONE`, `ending_block`, `after_upload` | the "What this part recorded" block of section 5 | "Recording works", admission line, seal report, "harness exited:", "uploaded: N bytes, acknowledged", outage lines (to the log) |
| Seal | `seal_inside_harness`, `seal_question`, `seal_not_confirmed`, `seal_needs_terminal` | `seal_done` without disk words; three endings: "still finishing, run goalcup seal again", "could not finish, write to support", "already sealed" | entry, terminal record, seal.json, anchor, termination lines |
| Upload | PRD-06 section 10 lines; `entries_intro` becomes the "Parts:" line | `upload_not_opened` and `upload_local_run` merged | `upload_seal_missing` disk words |
| Setup | frame line, harness pick, home step, machine warnings, `Ready.` block | no-prompt question (section 7) | login, `goalcup mcp` lines, SSH, disk, old token notice, "N of 4 steps" |
| Time left | `time_left_unknown`, `no_open_run_time_left` | reads `upload.json` | MCP answer |
| Masking | PRD-05 section 12 paragraph, in the guide and once at setup | | |

## Files it owns

Create (under `crates/goalcup/`, PRD-00 ownership map):

| Path | What | Src lines (estimate) |
|---|---|---:|
| `src/main.rs` | clap tree of seven commands plus the hidden recorder entry; exit codes | 150 |
| `src/cli/mod.rs` | `GOALCUP_HOME`, `GOALCUP_SERVER`, run and entry paths, `setup.json`, `record-hosts.txt` | 120 |
| `src/cli/start.rs` | section 3 steps 1 to 11, `StartFacts`, deadline and seal end of the harness | 380 |
| `src/cli/folder.rs` | folder checks in one refusal, token line, WSL and network file system checks (from `start/project.rs` 363 and `folder_gate.rs` 224) | 170 |
| `src/cli/approve.rs` | device flow, run credential, repo pre-check (from `account/device.rs`, `credential.rs`, `commands/run_repo.rs`) | 280 |
| `src/cli/continue_run.rs` | close dead entry, continue, relaunch | 130 |
| `src/cli/status.rs` | section 4 | 230 |
| `src/cli/finish.rs` | section 5, wrap-up progress | 140 |
| `src/cli/seal.rs`, `src/cli/upload.rs` | ask, signal, dispatch | 90 |
| `src/cli/setup.rs` | section 7 | 300 |
| `src/cli/push.rs` | push block, `--prompt`, clipboard (from `push_lines.rs` 213) | 110 |
| `src/cli/time_left.rs` | section 10 | 80 |
| `src/cli/awake.rs` | section 9 | 150 |
| `src/cli/skill.rs`, `src/cli/log.rs` | skill install, `goalcup.log` | 130 |
| `src/cli/text.rs` | every message (section 13) | 500 |
| **Total** | | **about 2,960** |

Tests: `crates/goalcup/tests/cli_*.rs` (section Acceptance), using PRD-14's `tests/common/` fake Worker
and temp homes. Repo files: `skills/goal-cup/SKILL.md` (change), `skills/goal-cup-setup/` (delete).

Must not touch: `src/pipeline/`, `src/watchdog.rs` (PRD-02), `src/proxy/` (PRD-03), `src/native/`
(PRD-04), `src/mask/` (PRD-05), `src/upload/` (PRD-06), `src/connector/` and `connectors/**`
(PRD-11), `packaging/**` and `scripts/**` (PRD-14, PRD-12, PRD-07), `docs/**` (PRD-15),
`crates/goalcup-format/` (PRD-01), the old crates (PRD-13).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | COUNT and seal keys (section 9), gap reasons (section 3), `goalcup` in the start event | Used here; Asks A1 to A3 |
| PRD-02 | `watchdog::run(plan, facts) -> ExitCode` (S8); `pipeline::close_dead_entry(dir)`; `status.json` (section 6); SIGUSR1 to `rec/watchdog.pid` for `goalcup seal`. Needs, as asks to PRD-02: **N1** in `mode: sealed`, `written` and `off` equal the seal `totals`; **N2** `status.json` carries `harness: {pid, started, exit}` (written by the watchdog) and the last 20 `note` events; **N3** `last_request_at` and `last_native_at` (wall times of the last written model request and native DATA); **N5** the watchdog ends the harness on deadline and on seal (SIGTERM, SIGKILL after 10 s) and calls the plan's `before_seal` hook (stop helpers) before telling the recorder to seal; **N6** the recorder appends its plain lines to the entry's `goalcup.log` through `cli::log::line` | Both |
| PRD-03 | `note` values `long_tunnel`, `ca_refused`, `pinned_host`; `conn` meta `listed`, `reason`; status wording in section 4 | Used here |
| PRD-04 | `native_files`, `NativeFinish.cut_files`, `warn_missing` rows; run start in whole seconds in the start facts | Both |
| PRD-05 | `mask::workspace_values`, `mask::env_values`, `MaskError`; section 12 text | Used here |
| PRD-06 | `upload.json` (S9), section 10 lines, `upload::manual::run`, `ServerClient` for device, open and continue; `GOALCUP_KEEP_PARTS`. Ask **N4**: `upload.json` carries `server: {seconds_left, asked_at, stop, reason}` from the last heartbeat answer, so `time-left` needs no channel into the recorder process | Both |
| PRD-07 | Device routes unchanged; open and continue bodies (`connector_sha256`); `sealed`; version floor refusal text | Used here |
| PRD-08 | `goalcup-verify check` line goes to `goalcup.log` only when local parts are whole; the screen's counts are the ones `counts_match_seal` recounts | Both |
| PRD-11 | S3: connector display name, tier and verified version, `LaunchPlan` (argv, env, resume), isolated home for setup, `no_prompts` values, first-screen hint, helpers to stop, `warn_missing`, vendor-backend flag; reads `record-hosts.txt` into the participant list. Deletes `[participant.time_left_tool]` from four connectors | Both |
| PRD-12 | Pad uses `GOALCUP_SERVER`, `GOALCUP_KEEP_PARTS`, `GOALCUP_NO_BROWSER`, setup without a terminal, `status --json`, the three contract words | Used there |
| PRD-13 | Deletes the old CLI files listed below after the new ones pass | Done there |
| PRD-14 | `install.sh`: two binaries, the run guard reads `status --json` `status.mode` (`recording`, `losing`, `writer_slow`, `null` mean a run is live), one PATH line (H8) | Used there |
| PRD-15 | The participant guide from sections 3 to 10; the masking paragraph; the 0.7 MCP leftover line; the firewall line | Used there |

## What is cut

All under `crates/goalcup-cli/` (deleted whole by PRD-13; these are the participant parts not
carried into `src/cli/`). Lines are `wc -l` of the file, src and inline tests together unless split.

| Path | Lines | Why |
|---|---:|---|
| `src/commands/login.rs` | 84 | H1 |
| `src/commands/mcp.rs` | 474 (273 src, 201 tests) | Answer 6, H4 |
| `src/commands/doctor.rs` | 556 | Folded into setup; its pin, daemon, decoders and SSH checks are cut |
| `src/commands/entries.rs`, `src/entries.rs` | 82, 208 | Status lists parts |
| `src/ssh_keys.rs` | 241 | F10; event-rule item moved into 1.0 |
| `src/prep_file.rs` | 88 | `setup.json` without the start gate |
| `src/run/pause.rs`, `src/run/reserve.rs` | 360, 956 | Disk handling (F2) |
| `src/run/watch.rs` | 1,485 | Disk watch, time-left file and heartbeat move to PRD-06 and section 10 |
| `src/participant.rs`, `src/participant/*.rs` | 1,788 src, 702 tests | Rewritten as `text.rs` (about 500) |
| `src/commands/prep.rs` | 1,364 | Rewritten as `setup.rs` (about 300) |
| `src/commands/start/*.rs` (8 files) | 4,822 | Rewritten as `start.rs`, `folder.rs`, `finish.rs` (about 690); the proxy, sink, admission and termination parts belong to PRD-02 and PRD-03 |
| `src/commands/status.rs`, `seal.rs`, `continue_run.rs`, `time_left.rs` | 814, 784, 494, 168 | Rewritten (about 530) |
| `src/account/github.rs` | 466 | Old clone token, old `github.json`, hidden credential helper (F8) |
| `src/push_lines.rs`, `src/host_log.rs`, `src/skill.rs` | 213, 253, 162 | Trimmed copies (about 250) |
| Tests | `tests/it/login.rs` 159, `prep.rs` 1,600, `status_views.rs` 1,869, `preflight_doctor.rs` 478, `start_launch.rs` 1,481, `native_silence.rs` 152, `seal_scan.rs` 206, `account.rs` 1,195, `tests/m2_cli_lifecycle.rs` 1,944, `tests/full/t25_disk_full.rs` 1,173, `full/per_run_lifecycle.rs` 366 | 10,623 test lines; the cases still needed are rewritten as `tests/cli_*.rs` (about 1,200) |
| Connector data | `[participant.time_left_tool]` in four `harness.toml` files | PRD-11 deletes |
| Skill | `skills/goal-cup-setup/SKILL.md` | 106 (H10) |

## Acceptance

Real-run proofs first (on the pad, PRD-12, Linux sandboxes; macOS on the owner's Mac after the build):

1. **The end screen equals the recount.** Claude Code, direct key, one subagent. Pass: every number
   on the end screen (model requests written, per host, with missing parts, MB lost, session files,
   off count and time, tunnel hosts and bytes) equals `goalcup-verify check --json` on the same entry,
   exactly. The screen never contains "Recording works".
2. **Off is shown while it happens.** Codex pad run with PRD-02's kill test (`kill -9` the recorder
   twice) and its null-mode test hook (5 minutes). Pass: a `goalcup status` loop every second shows
   "Recording is restarting" within 2 s of each kill, "Recording is off since T" (T within 1 s of the
   null mode start) during null mode, and "Recording: on." after it; the end screen says "Recording
   was off 3 times" with a total equal to the sum of the `recorder_down` gaps.
3. **Losing is shown.** Codex run with PRD-02's memory-guard test hook. Pass: status shows "losing
   data now" while the gap is open; the end screen's "with missing parts" equals `req_holes`.
4. **Dodger is honest.** The pad's dodger case for Claude Code, Codex and Hermes. Pass: the end
   screen names the pinned host under "refused the recorder", never says "Nothing was lost", and the
   start line shows the tier "not supported" for that case.
5. **Gateway looks like a direct key.** The pad's gateway case. Pass: written requests are listed
   under the gateway's host and the screen has no gateway-specific line.
6. **One paste pushes.** A `--test` run with `--repo` and `--prompt`. Pass: `first-message.txt`
   starts with the 7 push lines and ends with the prompt file's bytes; the participant pastes once;
   the GitHub repository has at least one commit from the agent within the run.
7. **Setup without a terminal.** `goalcup setup --harness codex-cli </dev/null` in a sandbox. Pass:
   exits 0 or 1, prints no `goalcup mcp` text, opens no browser or harness, prints the sleep line
   as a warning (no inhibitor in the sandbox), and never a `PROBLEM:` line for sleep anywhere in the
   run.
8. **time-left inside the harness.** The agent runs `goalcup time-left` in a pad run. Pass: the
   printed time is within 1 minute of the server's `seconds_left` at that moment.
9. **Version.** Pass: `goalcup --version` prints `goalcup 1.0.0`; the start event says `1.0.0`; a
   0.7.0 client is refused by the local Worker with the floor's text.
10. **macOS (owner, after the build).** Pass: setup prints `Ready.`; during a run `pmset -g
    assertions` lists the `caffeinate` hold; every printed path under `Application Support` pastes
    and works.

Automated tests that guard them (`tests/cli_*.rs`, facts and contract words only):

- `cli_finish.rs`: from fixture `status.json` and `upload.json` files: zero gaps gives "Nothing was
  lost"; any gap, off time or listed tunnel never does; counts and hosts printed are the fixture's.
- `cli_status.rs`: each row of the state table in section 4 from a fixture, including a stale
  `status.json` with dead pids, and a null-mode one; one `PROBLEM:` per cause.
- `cli_push.rs`: first-message composition; `--prompt` over 64 KiB or not UTF-8 is refused; the
  push block never holds the token value.
- `cli_folder.rs`: three folder problems give one refusal naming all three; overlayfs is accepted.
- `cli_setup.rs`: no terminal means no question and no open; no `goalcup mcp` text; the no-prompt
  values are written only into the isolated home and only after "y".
- `cli_time_left.rs`: from `upload.json`, fresh and older than 11 minutes.
- `cli_commands.rs`: `--help` lists exactly the seven commands; `goalcup login` and `goalcup mcp`
  are usage errors (exit 1).

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Per-host counts need new keys | Without A1 the screen cannot name hosts from written counts | Ask A1; until it lands the line omits hosts rather than use seen counts |
| "Under heavy load" alarms people | A valid run with a few missing parts looks broken | Say what was kept first, give MB lost, and keep "Your agent is not slowed" |
| Cutting `login` | Terms not accepted show at the first practice start, not at setup; staging needs `GOALCUP_SERVER` | The approval page already says so; the guide names the variable |
| No-prompt settings (H3) | The agent can change any file the user can; a harness may rewrite its settings file | One explicit question, default yes, plain warning; re-read and warn; owner question P1 |
| `--prompt` | A participant still pastes only the prompt; the prompt file holds the token by mistake | The framed line without `--prompt`; masking of `.env` values covers the token on the wire, and the first-message file is 0600 |
| Status reads files only | A recorder that writes `status.json` but is wedged looks alive | `writer_slow` from PRD-02; `updated` older than 10 s is "restarting" then "off" |
| Ending the harness at the deadline | Grandchildren outlive it and keep using the proxy until the watchdog exits | After exit their connections are refused; nothing after the deadline is judged; owner confirms (P4) |
| No sleep lock under WSL or in containers | The machine sleeps mid-run | One honest line, never `PROBLEM:`; the guide says to set Windows sleep to Never |
| `caffeinate -s` | Lid close on a MacBook still sleeps | Line says "lid open"; unverified for clamshell mode |
| WSL2 | Not tested tonight (Linux sandboxes only) | Folder gate and opener are copied unchanged; marked unverified |
| 0.7 leftovers | An old `goalcup mcp` line in a goalcup harness home shows a failed MCP server | No compatibility; one guide line |
| Size | 2,960 src lines against PRD-00's 2,000 for CLI and connectors | Ask A4; the device flow, sleep lock and folder rules are copied, not new |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Restarting before "off" | 30 s | PRD-10 |
| `status.json` stale | 10 s | PRD-10 with PRD-02 |
| No-request alarm | 10 minutes with session files growing; 5 minutes after launch with nothing written | PRD-10 |
| Tunnel hosts on screen | 5, listed hosts always | PRD-10 |
| time-left freshness | 11 minutes | PRD-10 with PRD-06's heartbeat cadence |
| Harness end grace | SIGTERM, SIGKILL after 10 s | owner (P4) |
| No-prompt question default | yes | owner (P1) |
| `--prompt` size | 64 KiB | PRD-10 |

## Asks of PRD-00 and PRD-01

| # | Of | Ask | Why |
|---|---|---|---|
| A1 | PRD-01 | Add `hosts` (`{host: model requests written}`, decrypted hosts only) to COUNT and seal `totals`, and `tunnel_hosts` (`{host: {conns, up, down}}`) to seal `totals`; PRD-08 recounts both | The end screen names hosts from written counts only (run 1 named hosts from the selection count) |
| A2 | PRD-01 | State that seal `reason` may be `recovered` (PRD-02 asks it too) and that `tunnel_hosts` is in the seal of a recovered entry | `goalcup continue` and `upload` show a dead entry's end screen |
| A3 | PRD-01 | Give `mask_failed` (PRD-02's ask) a GAP reason number, 12 | Section 5's plain-words table must cover every reason |
| A4 | PRD-00 | Command list for PRD-10 is `setup, start, continue, status, seal, upload, time-left`; `login` leaves the PRD table and the `src/cli/` line; size budget for the CLI alone about 3,000 src lines (connectors separate) | Login is cut (H1); the old budget of 2,000 covered CLI and connectors together |
| A5 | PRD-00 | Name `GOALCUP_HOME` (replaces `GOALCUP_CONFIG_DIR`, owned here) and `GOALCUP_SERVER` (`off` means local only) in the defaults; the pad server row uses `GOALCUP_SERVER` | PRD-01 and PRD-06 already use `$GOALCUP_HOME` and "no server" without defining them |
| A6 | PRD-00 | Add seams: **S9** `upload.json` with the `server` key (N4) and **S11** `goalcup.log` writer (`cli::log::line`, owned here, used by 02, 03, 06) | Today nothing names a shared log or the time-left source |
| P1 | PRD-00 owner questions | Reverse D67: setup writes the connector's no-prompt settings into goalcup's own harness home after one question (default yes)? Default: yes | Top cause of stalled runs (PARTICIPANT-REVIEW B1, a Codex run waited 8.7 of 10 minutes at a prompt) |
| P2 | PRD-00 owner questions | Cut `goalcup login` (one approval fewer on every OS)? Default: yes | H1; it grants nothing |
| P3 | PRD-00 owner questions | Push block: one paste through `--prompt` (default), not `git init` in the folder? | Run 1 finding 8; O10 settled wording only |
| P4 | PRD-00 owner questions | Keep ending the harness at the deadline and on `goalcup seal` (today's behaviour), with no termination proof? Default: yes. PRD-15 words it beside D66 in AGENTS.md, since the D65 exception goes | AGENTS.md "records and never manages the agent" lists exceptions |

## Builder notes

Order of work:

1. `main.rs` with the seven commands as stubs, `cli/mod.rs` (paths, env), `cli/text.rs` skeleton with
   the three contract words. Publish within the first hour; the pad and PRD-02 need the hidden
   recorder entry and `GOALCUP_SERVER` early.
2. `cli/start.rs` against PRD-02's stubbed `watchdog::run` and PRD-11's stubbed `LaunchPlan`, with
   `GOALCUP_SERVER=off` first (no approval), then the device flow against PRD-14's fake Worker.
   This is what phase 1a's early real test runs.
3. `cli/status.rs` and `cli/finish.rs` from fixture JSON files, before the pipeline exists.
4. `cli/setup.rs`, `cli/push.rs`, `cli/awake.rs`, `cli/time_left.rs`, `cli/seal.rs`, `cli/upload.rs`.
5. `skills/goal-cup/SKILL.md` without the MCP paragraph; delete `skills/goal-cup-setup/`.

Traps from the audit:

- Count only what `status.json` calls written. Never add a counter in the CLI (run 1's
  `record_generation_request`, `finish.rs:405-444`).
- Print the end screen only after the harness left the terminal: run 1 printed it over Codex's TUI.
- Never print or log the run credential, the proxy credential or a token; the first-message file is
  0600; quote every printed path (macOS `Application Support`).
- A missing clipboard tool is normal (sandboxes, SSH). Never fail on it.
- `PROBLEM:` only for something the participant can act on or must report. Sleep without a lock in a
  sandbox is not one.
- Keep "shows the code XXXX-XXXX." and "Recording in chat mode" byte for byte; the pad reads them.
- Stop a connector helper before the seal, or its late writes miss the final snapshot (PRD-04 section 6
  item 7, Hermes gateway).

How a watcher proves it in a real run: start the pad with `GOALCUP_KEEP_PARTS=1`; every 10 s save
`goalcup status --json`; at the end save the terminal's last screen; run `goalcup-verify check --json`
on the entry. Compare every number on the end screen with the check (acceptance 1), and every state
change in the saved status files with the GAP times in `goalcup-verify dump` (acceptance 2 and 3).
