# Lane F: week-long run risks

Appendix to the 2026-10-30 audit. Repo at 4b226de2, read only. Nothing was built or run. This file merges two parts:
the operating-system part (sleep, lid, network, disk, memory, file handles, clocks, process supervision, WSL) and the
harness and provider part (updates, sign-in refresh, rate limits, crashes of our own processes, Worker outages, fresh
sessions). "Inferred" means worked out from code or a measurement, not run. Line counts use `wc -l`, split at the first
`#[cfg(test)]` in the file ("src" is above it, "tests" is the in-file test module); integration tests under
`crates/*/tests/` are named and, where the OS part measured them, counted. The 2-hour run report
(`docs/research/results/2026-10-30-two-hour-run.md`) does not exist yet, so no finding here leans on it.

## Lane summary

A 168-hour run fails in two quiet ways and one loud way. Quiet: a daemon crash stops all recording and nobody is told
(F12), and a closed laptop lid or a closed terminal tab stops the run for hours (F5, F6). Loud: the disk pause freezes
the agent with `SIGSTOP`, which breaks point 3 and the owner's "disk full: no special handling" (F2). Most OS-level
fears are already handled well: file handles are not the limit for subagents (F1), clocks count through sleep and the
deadline comes from the server (F8), and network changes are picked up by keepalive and fresh DNS (F7). The biggest
cut is the disk code (about 1,000 src and 1,800 test lines). The biggest new cost, once the subagent caps go, is the
native poll that stats every tracked file 10 times a second (F3). Several "caps of our own" should go: the 5-restart
recorder cap (F10), the 32 outage windows (F22), the 10,000 tunnel rows (F25) and the 64 MiB finish cap for
`final_snapshot` (F23). Small additions worth doing for Friday: tell the participant when the daemon stops answering
(F12), let `goalcup continue` wait for the server (F14), warn Hermes users on an OAuth provider (F20), and set
`DISABLE_UPDATES` for Claude Code (F16).

## Top findings

1. **F12, a daemon crash is silent.** Nothing restarts the daemon, a restarted one admits no producer, the sender gives
   up after about 90 s, and the watch tick says nothing. A crash on day 1 can leave 6 days unrecorded. Add a "recording
   has stopped" line after 3 silent ticks (about +25 src, +30 tests).
2. **F2, the disk code blocks the harness and does not protect the chain.** Cut the 3 GiB reserve (the two parts
   differ on this, see F2), the `SIGSTOP` pause and the pressure rules. On `ENOSPC`, truncate the chain to its last
   whole record so the trace is "incomplete", not "invalid".
3. **F3, native polling grows with every subagent file.** At 2,000 tracked files, about 100,000 syscalls a second,
   forever (inferred). Set `NATIVE_EVERY` to 1 s (one constant).
4. **F1, raising the stream caps costs no file handles.** A native file holds no handle between polls and wire streams
   share one socket. Raise the caps, keep the 32-handle budget, move the rlimit raise to the start of `main`.
5. **F5 and F6, lid close and a closed terminal are the likeliest way to lose hours.** Software cannot stop lid sleep.
   Change the words: lid open always, `tmux` for everyone, pause OS updates.
6. **F14, `goalcup continue` needs the Worker.** Make it keep asking for up to 30 minutes with a visible line.
7. **F20, Hermes OAuth sign-in cannot refresh** under the replace-mode CA. Warn or refuse at `prep` and `start`.
8. **F10, F22, F23, F25: caps of our own.** Remove the 5-restart cap, the 32 outage windows, the 64 MiB finish cap
   for `final_snapshot` and the 10,000-row tunnel cap.

## Risk table

Likelihood is this lane's estimate for one 168-hour run on a participant's own laptop. It is not measured.

| ID | Risk | Likelihood in a 168-hour run | What happens today | Smallest fix | Points |
|---|---|---|---|---|---|
| F1 | More than 32 subagent session files, or more than 1,024 files | High for subagent-heavy runs | Later subagent files wait unread until the finish; handles are not the limit | Raise `MAX_OPEN_STREAMS` and the native share, drop the 1,024-file cap, move the rlimit raise to `main` (about 10 src) | 1, 2, 3 |
| F2 | Disk fills, or drops below 1 GiB beyond the reserve | Low to medium (higher with a long Worker outage, F13) | `SIGSTOP` freezes the harness; first real `ENOSPC` latches the chain with a torn last line | Cut reserve, pause and pressure rules; truncate to the last whole record on `StorageFull` (about +10 src) | 3, 5, 6 |
| F3 | CPU load from polling thousands of tracked files every 100 ms | High once the file cap is removed | About 100,000 syscalls a second at 2,000 files (inferred) | `NATIVE_EVERY` to 1 s (1 line) | 2, 3 |
| F4 | Memory growth over a week | Low | All maps bounded or tiny; redactor holds up to 16 KiB per open stream | None (keep) | 3 |
| F5 | Laptop sleeps (lid closed) | High on laptops | Lock stops idle sleep only; run stalls; `status` may say "nothing uploaded for 8 h" after wake | Words: lid line always; optional: heartbeat and upload at once after wake (about 10 src) | 1, 3, 6 |
| F6 | Terminal closed, SSH drop, reboot, OS update restart | Medium | Harness exits, the part ends; `SIGTERM` seals nothing but the chain is synced every 800 ms | Docs: `tmux` for everyone, pause OS updates | 3, 5, 7 |
| F7 | Wi-Fi switch, VPN, DNS change; corporate proxy or TLS inspection | High for changes (handled); low for corporate proxy (total failure) | Keepalive drops dead paths in about 2 min, fresh DNS each connect; no upstream proxy; Mozilla roots only | Docs: say a proxy-only or TLS-inspecting network will not work | 1, 3, 7 |
| F8 | NTP jump, wrong date, sleep, daylight saving | High that clocks move; handled | Times carry wall and continuous clock; deadline from server, refreshed per heartbeat | Keep; cut older-server fallbacks (about -40 src, -20 tests) | 5 |
| F9 | WSL2 clock drift, idle shutdown, untested platform | Low (few WSL2 users) | Deadline fixed at next heartbeat; wall times can be hours early | Docs: keep the Ubuntu window open; lid action is separate | 3, 7 |
| F10 | Recorder accept loop dies more than 5 times | Very low | Sixth death leaves the port unserved; every model call fails for the rest of the part | Restart always with a 1 s pause; drop the cap and its carry | 1, 3 |
| F11 | `goalcup start` (the proxy) crashes | Low (unverified) | Harness keeps a dead `HTTPS_PROXY`; every model call fails | Docs: "exit the agent, run `goalcup continue`" | 1, 3 |
| F12 | Daemon crashes mid-run | Low but unknown (no week-long run yet); impact severe | Not restarted; no producer admitted again; participant not told | "Recording has stopped" line after 3 silent ticks (about +25 src, +30 tests) | 1, 2, 6 |
| F13 | Worker down for hours mid-run | Low to medium | Upload and heartbeats retry forever, 60 s to 15 min; deadline also kept locally | Keep; cut the disk pause (F2) | 5 |
| F14 | Worker down when `goalcup continue` runs | Low | 3 tries 3 s apart, then "Nothing was started" | Keep asking up to 30 min with one line a minute (about +15 src) | 3 |
| F15 | A goalcup release during the week | Low with a process rule | Minimum version checked at start only; older binaries refuse `final_snapshot` | Process rule: no client release after 6 Nov | 7 |
| F16 | Claude Code updates itself mid-week | Medium | `DISABLE_AUTOUPDATER` stops only the background check; `claude update` still works | `DISABLE_UPDATES = "1"` (1 line) | 1, 7 |
| F17 | Codex update prompt swallows a turn | Low (handled) | `check_for_update_on_startup=false` | None (keep) | 3, 7 |
| F18 | Hermes `hermes update` mid-run; Amp update | Low | Hermes gateway may restart without the proxy (unverified); Amp check skipped | One guide line for Hermes; pin a Hermes version | 1, 7 |
| F19 | Not knowing which harness build ran in each part | Medium (follows F16) | Version line and build digests per launch | None (keep) | 6, 7 |
| F20 | Sign-in token refresh through the proxy | High that a refresh happens; certain failure for Hermes on OAuth | Works for Claude Code, Codex, Amp; Hermes OAuth refresh fails handshake | Warn or refuse Hermes OAuth at `prep` and `start` (about +20 src) | 3, 7 |
| F21 | 429 storms, usage limits, provider outages | High for subscription users | Every answer recorded; connect retried up to 30 s; Claude Code waits out limits | None (keep); later, a status line for a run stuck on 429s | 1, 3 |
| F22 | Outage windows lost or capped | Medium | 32 kept in memory, printed at finish only, never uploaded | Write each closed window to the chain; drop the cap (about -60/+25 src, -120 tests) | 5, 6 |
| F23 | Hermes `state.db` passes 64 MiB in a week | Medium for Hermes | Rest becomes a gap at seal | No byte cap for `final_snapshot` reads, only the 180 s budget (1 constant) | 1, 2 |
| F24 | `goalcup continue` after a crash does not resume the session | Medium (Codex, Amp unmeasured) | Old part closed; resume flags unmeasured for Codex and Amp | Measure in the 2-hour run | 3, 7 |
| F25 | Tunnel list stops naming new hosts, or is lost | High (`npm install` loops pass 10,000) | First 10,000 rows kept, written once at finish | Per-host rows, or append on close and upload (the parts differ, see F25) | 5, 6 |

## Where the two parts disagree

| Topic | OS part | Harness part |
|---|---|---|
| The 3 GiB reserve file (F2) | Cut it with the pause; T25 shows it does not protect the chain | Cut the pause; "whether the reserve file stays is for the disk lane" |
| Pruning (F2) | Keep only "delete a part the server acknowledged, never the last one", no 2-hour timer, no pressure; or keep as is for Friday | No change proposed |
| What a full disk leaves (F2) | Today: a torn tail and no gap (T25); proposes a truncate so the trace is "incomplete" | "The chain write fails and the run ends with a gap" |
| Recorder restart cap lines (F10) | About -60 src, -50 tests; cites `proxy/mod.rs:1019-1125`, `phases.rs:516-529`, `1505-1510` | About -40 src, -20 tests; cites `proxy/mod.rs:1018-1110`, `phases.rs:518-529`, `1060-1073`, `1505-1510`, `forked.rs:71`; adds a rate-limited log line |
| Outage windows (F22) | Listed under memory as bounded (32 kept); keep | Change: write each window to the chain, drop the 32 cap |
| Tunnel list (F25) | One row per (host, port, reason) with counts and first/last time; stays local ("only shown, never uploaded") | Append each row when the tunnel closes, upload with the parts, drop the cap |
| Local deadline (F8, F13) | Cut the "older server" `LocalDeadline::of` and `::until` fallbacks | Relies on the deadline being "kept locally as well as from the server" when the Worker is down |
| Watch tick (F2, F12) | Cuts the disk part of `run/watch.rs` and `reserve.rs` | Hangs the daemon alarm on the 30 s watch tick defined at `run/reserve.rs:41` |

The last two are tensions, not contradictions. Before cutting `LocalDeadline` fallbacks, check that the local deadline
path the harness part relies on (`crates/goalcup-cli/src/upload/uploader/lifecycle.rs:129-133`,
`crates/goalcup-cli/src/run/time_left.rs:16`) is not the same code. If `reserve.rs` goes, the 30 s tick constant must
move to `run/watch.rs` so F12 still has a clock.

## How the parts fit (what matters for a week)

- `goalcup start` runs the recording proxy and its sender on a thread of its own process. "When this process dies they
  die with it" (`crates/goalcup-cli/src/commands/start/proxy.rs:62-66`).
- The daemon is a separate process, started on demand, detached with `setsid`
  (`crates/goalcup-cli/src/daemon/spawn.rs:1-24`). It writes the chain and runs the native file reader.
- The uploader runs inside `goalcup start` and talks to the Worker. It never gives up during a run
  (`crates/goalcup-cli/src/upload/policy.rs:1-7`).
- One run is many segments ("parts"). `goalcup continue` opens the next one
  (`crates/goalcup-cli/src/commands/continue_run.rs:1-28`).

## Handle budget, counted

| What | Handles | Where held | Evidence |
|---|---|---|---|
| One proxied connection (intercepted or tunnelled) | 2 (client socket + upstream socket) | `goalcup start` (host) | `crates/goalcup-cli/src/commands/start/proxy.rs:25-31`, `crates/goalcup-cli/src/proxy/mod.rs:110-112` |
| One open wire stream | 0 extra | none: streams of a producer share one IPC socket | `crates/goalcup-daemon/src/ingest.rs:4-10`, `22-37` |
| The host's IPC sender (one per proxy producer) | 1 in host, 1 in daemon | both | `crates/goalcup-daemon/src/ingest.rs:4-6` |
| One native session file (open stream or not) | 0 between polls; during a poll, one retained root per source plus at most 8 directory handles and 1 file | daemon | `crates/goalcup-daemon/src/native_capture.rs:74-81`, `1415-1465` (the file is opened, read, dropped inside `process`) |
| Native handles per capture, hard guard | 32 | daemon | `crates/goalcup-platform/src/handle_budget.rs:13`, `30-40` |
| Chain file, registry, control server, logs | a handful | daemon | inferred |

Limits today:

- The host asks for a soft limit of `2 * 1024 + 256 = 2,304`, capped by the hard limit
  (`crates/goalcup-cli/src/commands/start/proxy.rs:25-61`). It refuses connection 1,025 and later with a `503`
  (`crates/goalcup-cli/src/proxy/mod.rs:994-998`). Accept errors such as `EMFILE` pause 10 ms and retry, no busy loop
  (`crates/goalcup-cli/src/proxy/mod.rs:108`, `1008-1011`).
- The daemon is started before that raise (`crates/goalcup-cli/src/main.rs:595`, the raise is at
  `crates/goalcup-cli/src/commands/start/phases.rs:516`), so it keeps the shell's limit: 256 on a default macOS shell,
  1,024 on most Linux. From the table above, it needs well under 100 for one run, however many streams. The daemon
  also pauses on accept errors (`crates/goalcup-daemon/src/ingest.rs:612-618`).
- The harness is spawned after the raise, so it inherits 2,304. That helps a Node harness.

OS facts: macOS shells start at a soft limit of 256 with an unlimited hard limit
([QuestDB](https://questdb.io/blog/max-open-file-limit-macos-jvm),
[Apple forums](https://developer.apple.com/forums/thread/735798); community sources, not Apple docs). On macOS
`setrlimit` refuses `RLIM_INFINITY` for the soft open-file limit; the man page says to use `min(OPEN_MAX, rlim_max)`
([setrlimit(2)](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/setrlimit.2.html)).
`OPEN_MAX` is 10,240 there (from memory of `<sys/syslimits.h>`; unverified on a 2026 macOS). On Linux, systemd 240 and
later gives soft 1,024 and hard 524,288, and warns that `select(2)` breaks above 1,023
([Proton wiki](https://github.com/ValveSoftware/Proton/wiki/File-Descriptors),
[Caddy forum quoting systemd](https://caddy.community/t/systemd-service-file-directives/19114)). Rust's std does not
raise the limit by itself (unverified for 2026 std; true when last checked).

## Findings: operating system

### F1. File-handle limits and the stream caps the owner wants to raise

- **Points:** 1, 2, 3.
- **Today:** two different "32"s. `MAX_GIT_LIVE_HANDLES_V1 = 32` is a real handle guard
  (`crates/goalcup-platform/src/handle_budget.rs:13`). `max_open_streams: MAX_OPEN_STREAMS / 4` (32) and
  `max_files: 1024` are logical limits that hold no handle (`crates/goalcup-daemon/src/native_capture.rs:274-283`).
  `MAX_OPEN_STREAMS = 128` is per entry across wire and native (`crates/goalcup-protocol/src/capture.rs:58`). An
  append-only native stream stays open until the finish (`crates/goalcup-daemon/src/native_capture.rs:43-46`), so
  after 32 session files, later subagent files wait unread until the finish.
- **Verdict:** change the stream caps (the owner already decided to), keep the 32-handle budget. Raise
  `MAX_OPEN_STREAMS` and the native share without touching any rlimit in the daemon. Also move
  `raise_open_file_limit` to the start of `main` so the daemon and any command get it (1-line move), and let the host
  ask for more: `min(hard, 10240)` on macOS, `min(hard, 65536)` on Linux, then set the connection cap to about half of
  what it got.
- **Lines:** about 10 src changed. No cut.
- **Risk:** low. If the connection cap rises above what the limit allows, accept fails with `EMFILE` and retries every
  10 ms, which is a slow refusal, not a crash. A harness or tool that uses `select(2)` could break above 1,024 in its
  own process, because it inherits the raised limit (rare in Node, Rust or Python tools; unverified for each harness).
- **What could go wrong:** the redactor keeps per-stream state, up to 16 KiB of held bytes per open stream
  (`crates/goalcup-daemon/src/redact/stream.rs:122-125`). With thousands of append-only streams open all week, worst
  case is tens of MB (5,000 x 16 KiB = 80 MB). Fine on a laptop, but say it. The real cost of more open files is CPU,
  not handles (F3).

### F2. Disk filling: reserve, pause, prune and the ENOSPC latch (merged: OS F2, harness F12)

- **Points:** 3 (pause freezes the harness), 5, 6. Owner decision: "Disk full: no special handling. The harness stops
  and the user cleans up."
- **Today:**
  - A 3 GiB physically allocated reserve file per run, resized every 30 s (`crates/goalcup-cli/src/run/reserve.rs:1-45`,
    `crates/goalcup-platform/src/disk.rs:50-80`).
  - Below 1 GiB free beyond the reserve, the watch tick sends `SIGSTOP` to the harness tree and `SIGCONT` above 3 GiB
    (`crates/goalcup-cli/src/run/pause.rs:1-18`; the harness part cites `pause.rs:1-20` and `run/reserve.rs:31-35`).
    That is the harness being blocked, against point 3 and against "Goal Cup Proxy records and never manages the
    agent".
  - Pruning deletes acknowledged chain parts after 2 hours, or at once under disk pressure
    (`crates/goalcup-cli/src/upload/prune.rs:1-30`). Only acknowledged parts are pruned
    (`crates/goalcup-cli/src/run/watch.rs:1-5`).
  - The T25 measurement shows the reserve does not protect the chain: the agent's own writes take the blocks the
    reserve gives back, and the first real `ENOSPC` latches the chain writer for good with a torn last line and no gap
    (`docs/research/t25-disk-full.md:158-164`, `228-237`). The latch is `self.fail(&error)`
    (`crates/goalcup-daemon/src/runtime/capture/writer.rs:154`, `216`, `221`) and the poisoned append
    (`crates/goalcup-platform/src/tracecap.rs:673-689`).
- **Week-long chain of events (harness part):** the Worker is down, nothing is acknowledged, nothing is pruned, the
  chain grows, the disk drops below 1 GiB, and the agent is frozen until the Worker returns. That breaks point 3.
- **Verdict:** cut the pause and the pressure rules (both parts). The OS part also cuts the reserve; the harness part
  leaves the reserve to the disk lane. Keep pruning only as "delete a part the server acknowledged, never the last
  one", with no 2-hour timer and no pressure (or keep it as is if the cut is too large for Friday; it is not an OS
  risk). Also cut the server's pause notice (`server/worker/src/notices.ts:151-152`), the `PAUSE_BELOW` and
  `RESUME_ABOVE` policy, and the pause calls in `run/watch.rs` and `crates/goalcup-cli/src/commands/start/phases.rs:1090-1095`.
  Change one thing: on a write error that is `StorageFull`, truncate the chain to its last whole record before
  latching. Then a full disk gives an "incomplete" trace (exit 1) and a sealable entry, not a torn tail (exit 2,
  "invalid").
- **Lines (src / tests, measured with wc -l and the `#[cfg(test)]` split):** `pause.rs` 211 / 149; `reserve.rs` about
  534 / 422; `platform/src/disk.rs` 239 / 137 (`tests/it/disk.rs`); `prune.rs` 173 / 248 (shrinks, not all goes);
  `watch.rs` 718 / 767 (the disk part only, estimated 250 / 350); `tests/full/t25_disk_full.rs` 1,173 and
  `scripts/t25-disk-full.sh` 93 go with them. Total about 1,000 src and 1,800 test lines, plus plumbing in
  `start/phases.rs` and `run/mod.rs` (not counted). The truncate change is about 10 src lines and one test. If only the
  pause goes (harness part's scope): about -211 src plus callers, -149 tests.
- **Risk:** medium. Without the reserve, a full disk fails the recording at the same moment as the agent's own writes.
  That is what the owner asked for. Without the pause, the agent keeps working unrecorded after the chain stops, as it
  does today after the latch.
- **What could go wrong:** the truncate must use the writer's known whole-record length, or it can cut a good record.
  The uploader's staging chunk and `state.json` also live on the same disk (`docs/research/t25-disk-full.md:244-248`,
  `238-243`), so `goalcup status` can still say `recording` while the disk is full. Accept that, or have `status` read
  the daemon's signed capture state first. Docs that promise the pause (`docs/PARTICIPANT-GUIDE.md:242-244`, D65) must
  change in the same PR. If `reserve.rs` goes, the 30 s watch tick defined there (`run/reserve.rs:41`) must move,
  because F12 uses it.

### F3. Native polling cost grows with every subagent file, every 100 ms, all week

- **Points:** 3 (never slow the harness), 2.
- **Today:** the owner thread polls the native reader every 100 ms (`NATIVE_EVERY`,
  `crates/goalcup-daemon/src/runtime/capture/owner.rs:16`, used at `173`). Each poll walks the source tree up to 8,192
  directory entries (`crates/goalcup-daemon/src/native_capture.rs:274-283`, `792-818`) and, for every tracked file,
  re-opens its directory chain and stats it (`open_chain` and `observe`,
  `crates/goalcup-daemon/src/native_capture.rs:1415-1435`). The tracked-file map never shrinks
  (`crates/goalcup-daemon/src/native_capture.rs:734-738`). A deeper poll is done at once when the last one had more
  bytes than its budget (`owner.rs:173`, "when it reported no deferred work").
- **Inferred cost:** with 2,000 tracked subagent files at 3 levels deep, about 2,000 x 5 syscalls x 10 polls a second
  = 100,000 syscalls a second, forever, even when nothing changes. That is a steady CPU load on a laptop and keeps it
  from idling. Not measured.
- **Verdict:** change. Poll quiet files less often. The least code: set `NATIVE_EVERY` to 1 s (one constant). Native
  latency does not matter for a week-long record, and a busy poll already repeats at once. Better, still small: open
  each parent directory once per poll and stat its tracked files from that one handle.
- **Lines:** 1 src line (constant), or about 40 for the grouping.
- **Risk:** low for the constant. A test that expects a native byte within a few hundred ms may need a longer wait.
- **What could go wrong:** at 1 s and 256 KiB per poll (`native_plan.rs:11`, `212`), a harness writing faster than
  256 KiB a second falls behind, but `more_work` makes the next poll come at once, so it catches up. Measure on the
  2-hour run with many subagents before Friday.

### F4. Memory over 168 hours

- **Points:** 3.
- **Today, checked map by map:**
  - Wire connection maps shrink when a connection closes (`crates/goalcup-daemon/src/runtime/capture/sink.rs:514-517`,
    `writer.rs:336`, `352`).
  - The sink queue is bounded at 8 MiB (`crates/goalcup-protocol/src/capture.rs:51`,
    `crates/goalcup-daemon/src/runtime/capture/sink.rs:7-16`).
  - Native tracked files never shrink, by design (offsets are needed). A few hundred bytes each: 20,000 files is under
    10 MB (inferred).
  - The sealed-connection list keeps the first 10,000 tunnels and then only counts
    (`crates/goalcup-cli/src/proxy/tunnels.rs:20`). Outage windows keep 32 (`crates/goalcup-cli/src/proxy/outage.rs:21`).
    Both bounded. (Both are changed for other reasons, F22 and F25.)
  - `AckAges` in pruning grows by one entry per chain part (`prune.rs:30-38`); tiny.
  - The redactor holds up to 16 KiB per open stream (F1).
- **Verdict:** keep. The tunnel list change is in F25.

### F5. Sleep, wake and a closed lid

- **Points:** 1, 3, 6.
- **Today:** the host holds `caffeinate -i -s -w <pid>` on macOS and `systemd-inhibit --what=sleep:idle --mode=block`
  on Linux, from launch to the end of the wrap-up (`crates/goalcup-platform/src/awake.rs:76-143`,
  `crates/goalcup-cli/src/commands/start/mod.rs:531-541`). Under WSL it refuses and says to set Windows sleep to Never
  (`awake.rs:87-89`, `crates/goalcup-cli/src/commands/prep.rs:1116-1131`). A lock that fails is one `PROBLEM` line
  (`crates/goalcup-cli/src/run/mod.rs:562-566`, `crates/goalcup-cli/src/participant/status.rs:511-513`). The guide
  says "mains power, awake, online and the lid open" (`docs/PARTICIPANT-GUIDE.md:242`).
- **OS facts:**
  - `caffeinate -s` is valid only on AC power; `-i` stops idle sleep ([ss64 caffeinate](https://ss64.com/mac/caffeinate.html)).
  - Neither stops lid-close sleep on a MacBook. Only closed-display mode (external display, keyboard or mouse, and
    power) keeps it awake
    ([Macworld](https://www.macworld.com/article/673295/how-to-use-macbook-with-lid-closed-stop-closed-mac-sleeping.html);
    third-party source, Apple page not fetched).
  - On Linux, `LidSwitchIgnoreInhibited=` defaults to `yes`: a "sleep" lock does not stop the lid from suspending the
    machine; only a `handle-lid-switch` lock does
    ([logind.conf](https://www.freedesktop.org/software/systemd/man/latest/logind.conf.html)).
- **What happens at wake (from code):** recorded times stay right (F8). Upstream connections that died during sleep are
  found by keepalive within about 2 minutes (F7). The deadline check runs every second on the sleep-counting clock, so
  a deadline that passed during sleep stops the run at wake (`crates/goalcup-cli/src/upload/uploader/heartbeat.rs:157-195`).
  The next heartbeat and upload are timed with `Instant`, which does not count sleep on macOS or Linux, so they come
  up to 5 and 30 minutes of awake time later (`heartbeat.rs:173-187`, `crates/goalcup-cli/src/upload/policy.rs:88`).
  Meanwhile `goalcup status` compares the last good upload to the wall clock and can say "nothing was uploaded for
  8 h" right after a night's sleep (`crates/goalcup-cli/src/run/mod.rs:520-531`; inferred, not run).
- **Verdict:** keep the sleep lock (269 src, 256 test lines). Change words only: the lid line must be said whether or
  not the lock was taken, on macOS and Linux both, because software cannot stop lid sleep. Optional, small: after a
  wake (the continuous clock jumped ahead of `Instant` by more than a minute), send the next heartbeat and upload at
  once. That removes the false "nothing uploaded" line. About 10 src lines.
- **Risk:** low.
- **What could go wrong:** the `PROBLEM: this computer may go to sleep` line also shows in a cloud sandbox or
  container with no `systemd-inhibit` (the Vercel Sandbox, likely; unverified). A participant there sees a problem
  that is not one. Consider making it a start-time note, not a `PROBLEM`.

### F6. Process supervision: closed terminal, SSH drop, reboot, shutdown

- **Points:** 3, 5, 7.
- **Today:**
  - The daemon calls `setsid` and outlives the terminal (`crates/goalcup-cli/src/daemon/spawn.rs:16`, `398-405`).
  - The host catches `SIGHUP`, `SIGINT`, `SIGQUIT`, `SIGTSTP` so it can seal and upload after the harness goes
    (`crates/goalcup-platform/src/terminal.rs:30`, `57-98`). The harness gets the hangup and exits, so a closed window
    or SSH drop ends the part. `goalcup continue` starts the next part with the same deadline
    (`docs/research/RESTARTS-DESIGN.md:50-57`).
  - `SIGTERM` (logout, shutdown, a service manager) is not caught by the host, and the daemon seals nothing on it
    (`crates/goalcup-daemon/src/bin/goalcup-daemon.rs:26-30`). The chain is synced every 800 ms (`owner.rs:10`), so at
    most that much plus the unsent upload waits on disk for the next `goalcup` command (`RESTARTS-DESIGN.md:53-55`).
  - The guide says to leave the terminal open and to use `tmux` over SSH (`docs/PARTICIPANT-GUIDE.md:216`, `242`).
- **Verdict:** keep. No new code. Rough edge 3 in `RESTARTS-DESIGN.md:99` (catch `SIGTERM` in the host) is not needed
  when the disk is kept, because `continue` and `upload` send the rest.
- **Docs change:** tell everyone, not only SSH users, to start the run inside `tmux` (or `screen`), because an
  accidental Cmd-W on a terminal tab ends the part and nobody may notice for hours. Tell them to pause OS updates for
  the week: Windows restarts after updates outside "active hours", and active hours are at most 18 hours
  ([Microsoft](https://learn.microsoft.com/windows/deployment/update/waas-restart)). macOS automatic updates can also
  restart (from `docs/research/LOCAL-NEXT-STEPS.md:105`; unverified against Apple docs).
- **Risk:** low.
- **What could go wrong:** a participant who never checks `goalcup status` loses the hours between the closed window
  and their next look. The server's silence email after 20 minutes (`RESTARTS-DESIGN.md:86-87`) is the safety net;
  check it is on for Friday.

### F7. Network changes: Wi-Fi switch, VPN, IP change, DNS change

- **Points:** 1, 3, 7.
- **Today:**
  - Upstream sockets get TCP keepalive: 60 s idle, then 4 probes 15 s apart, so a dead path is dropped after about 2
    minutes. There is no idle timeout, because SSE and WebSocket streams idle legitimately
    (`crates/goalcup-cli/src/proxy/connect.rs:91-98`, `958-961`, `1030-1036`).
  - Every connect resolves DNS afresh and tries every address (`crates/goalcup-cli/src/proxy/connect.rs:946-990`). A
    DNS change is picked up at the next connection. No in-process DNS cache.
  - The uploader's client has a 15 s connect and 60 s call timeout and retries transport errors
    (`crates/goalcup-cli/src/upload/http.rs:14-17`, `41-52`).
  - Upstream TLS trusts only the compiled-in Mozilla roots (`crates/goalcup-cli/src/proxy/tls.rs:22-24`, `231-234`).
    The launch sets `HTTPS_PROXY` to our loopback proxy for the harness and every tool it runs
    (`crates/goalcup-cli/src/launch.rs:855-859`). No code reads a participant's own upstream proxy (grep for upstream
    proxy support found none).
- **Verdict:** keep. Docs change: say plainly that a network that needs its own HTTP proxy, or a VPN that inspects TLS
  with its own root certificate, will not work with Goal Cup Proxy.
- **Risk:** low for most people. For a participant on a corporate laptop with TLS inspection, every model call fails at
  upstream TLS (point 7). Adding system-root support is a small change (`rustls-native-certs` or the platform
  verifier) but it widens trust; not for Friday unless a participant needs it.
- **What could go wrong:** a VPN that turns on mid-run may cut live streams. Those streams end as cut and are recorded
  as gaps; the harness retries. That is point 3 working as designed.

### F8. Clocks: NTP jumps, daylight saving, monotonic versus wall

- **Points:** 5.
- **Today:**
  - `continuous_ns` reads `CLOCK_MONOTONIC_RAW` on macOS and `CLOCK_BOOTTIME` on Linux, both counting through sleep,
    and the file says why std `Instant` is wrong for records (`crates/goalcup-protocol/src/clock.rs:1-19`). Std
    `Instant` is `CLOCK_UPTIME_RAW` on Apple, which stops during sleep
    ([Rust std source](https://doc.rust-lang.org/src/std/sys/time/unix.rs.html),
    [Instant docs](https://doc.rust-lang.org/std/time/struct.Instant.html)).
  - Each capture time has both the UTC wall time and the monotonic count (`crates/goalcup-protocol/src/capture.rs:88-92`,
    `crates/goalcup-cli/src/proxy/metadata.rs:50-65`). Order in the chain is by sequence number, so a wall-clock step
    back cannot reorder records.
  - The daemon keeps a high-water mark so a wall-clock rollback cannot move a phase backwards
    (`crates/goalcup-daemon/src/runtime/daemon.rs:59-62`).
  - The deadline is the server's `seconds_left`, counted on the continuous clock, and set again by every heartbeat
    answer (`heartbeat.rs:200-212`, `283-292`). The machine's date is used only after a reboot
    (`crates/goalcup-cli/src/run/time_left.rs:11-16`).
  - Daylight saving: all times are UTC. The US change is 1 Nov 2026 and Europe's 25 Oct 2026, both before the start
    window ([timeanddate 2026](https://timeanddate.com/time/dst/2026.html)). No big zone changes during 7 to 15 Nov
    was found (the list says it may change).
- **Verdict:** keep all of it. Cut the "older server" fallbacks, since there is no backward compatibility:
  `LocalDeadline::of` and `LocalDeadline::until` and their branches (`heartbeat.rs:204-209`, `243-277`), and the
  `time_left.rs` note about counts with no boot id written by an older build (`time_left.rs:15-16`) if its code path
  exists only for them.
- **Lines:** about 40 src and 20 test lines.
- **Risk:** low, if the Worker always sends `seconds_left`. Check that it does on every answer (open, continue,
  heartbeat) before cutting.
- **What could go wrong:** after a reboot the count is gone and the date decides until the first heartbeat answer. A
  machine with a wrong date shows a wrong time left for those few minutes. The harness part relies on the local
  deadline when the Worker is down (F13, `crates/goalcup-cli/src/upload/uploader/lifecycle.rs:129-133`); make sure the
  cut removes only the older-server branches, not that path.

### F9. WSL2 specifics

- **Points:** 3, 7.
- **Today:** WSL1 and Windows-drive folders are refused (`crates/goalcup-platform/src/machine.rs:95-156`,
  `README.md:190-193`). No sleep lock; the participant is told to set Windows sleep to Never. Native capture polls and
  never uses inotify, so the `/mnt/c` file-watching problem does not apply (`native_capture.rs:62-81`).
- **OS facts:**
  - The WSL2 clock falls behind after the host sleeps; `hwclock -s` or a resume task fixes it (known issue,
    [WSL #5324 summary](https://omkarmoghe.github.io/2020/08/07/WSL-2-Clock-Drift.html),
    [scivision](https://scivision.dev/wsl-date-time-skew-error)). Our deadline is corrected at the next heartbeat
    (F8), so no code is needed. Recorded wall times can be hours early until the guest clock is fixed; the monotonic
    count and the server's receive times still order them.
  - WSL shuts a distro down after `instanceIdleTimeout` (15 s) and the VM after `vmIdleTimeout` (60 s) of idleness
    ([wsl-config](https://learn.microsoft.com/en-us/windows/wsl/wsl-config)). Whether a background daemon with no
    terminal counts as idle is not stated there (unverified). The harness is in the terminal anyway, so keeping the
    terminal open already keeps WSL up.
- **Verdict:** keep. Docs change: under WSL2, also say "keep the Ubuntu window open" and "the laptop lid setting in
  Windows (Power, lid close action) is separate from sleep timeout".
- **Risk:** low.
- **What could go wrong:** WSL2 is still marked "real-machine validation is pending" (`README.md:182`). A first WSL2
  participant may hit something none of this covers.

## Findings: our own processes

### F10. The recording proxy's accept-loop restart cap (merged: OS F10, harness F8)

- **Points:** 1, 3.
- **Today:** `supervise_accept` restarts a dead accept loop on the same port, at most 5 times per run, and the count is
  carried across `goalcup continue` through `state.json`. The OS part cites `crates/goalcup-cli/src/proxy/mod.rs:1019-1125`,
  `crates/goalcup-cli/src/commands/start/phases.rs:516-529`, `1505-1510`. The harness part cites
  `crates/goalcup-cli/src/proxy/mod.rs:1018-1110`, `commands/start/phases.rs:518-529`, `1060-1073`, `1505-1510`, and
  `commands/start/forked.rs:71`. The loop itself does not end on an accept error (it pauses and retries,
  `mod.rs:1008-1011`), and a connection task that panics does not end it either (tasks live in a `JoinSet`). So the
  loop dies only on a panic in its own few lines or on the test kill hook. The sixth death leaves the port unserved:
  every later model call of the harness fails (point 3). The owner listed it as open (progress tracker, Task 10).
- **Verdict:** change. Keep a plain restart with a 1 s pause and no budget. Cut `MAX_RECORDER_RESTARTS`, the
  `recorder_lost` path for the cap, its carry across continues, its `state.json` field and its participant lines. A cap
  here is "a cap of our own" that guards no cost.
- **Lines (the parts differ):** OS part: about -60 src (the budget in `proxy/mod.rs`, 13 lines in `phases.rs`, about 20
  in `run/mod.rs`, `forked.rs`, participant text) and about -50 test lines
  (`crates/goalcup-cli/tests/it/m2_proxy_integration.rs:2967-3025`, the "at most five" and "keeps the run's restart
  count" tests). Harness part: about -40 src, -20 tests. Keep `a_dead_accept_loop_is_restarted_on_the_same_port`
  (`:2920`).
- **Risk:** low. A loop that panics on every accept would restart once a second forever and log each time; that is
  noise, not harm. The harness part suggests rate-limiting the restart line to one per minute so `host.log` does not
  grow.
- **What could go wrong:** the rebind can fail if something else took the port in the gap (`mod.rs:1093-1101`). Then
  the recorder is lost as today; keep that path.

### F11. `goalcup start` crashes (keep, docs)

- **Points:** 1, 3.
- The proxy and sender live in `goalcup start` (`crates/goalcup-cli/src/commands/start/proxy.rs:62-66`). If that
  process dies (panic, out of memory, killed), the harness keeps `HTTPS_PROXY` pointing at a dead port and every model
  call fails. Nothing restarts it.
- Recovery exists: exit the agent, run `goalcup continue`; it closes the old part from the daemon or from the chain
  tail (`crates/goalcup-cli/src/commands/continue_run.rs:85-100`).
- **Verdict:** keep. A separate proxy process would add code for a rare case. Add one line to the guide ("exit the
  agent, run `goalcup continue`").
- **Risk:** during the outage the harness is blocked, which the participant sees at once as errors.

### F12. The daemon crashes mid-run (change, highest risk)

- **Points:** 1, 2, 6.
- **What happens today:**
  - The CLI starts a daemon only for `start`, `seal`, `status` and `entries`
    (`crates/goalcup-cli/src/main.rs:595, 675, 710, 767`; seal path `crates/goalcup-cli/src/daemon/mod.rs:480-510`).
    Nothing restarts it during a run.
  - Even a restarted daemon "reopens every other entry for status and seal only: no producer is admitted again"
    (`crates/goalcup-daemon/src/runtime/capture/mod.rs:68-73`).
  - The proxy's sender reconnects 1, 2, 5, then every 10 s, and stops for good after 10 failures in a row, about 90 s
    (`crates/goalcup-cli/src/proxy/ingest.rs:44-60, 93-108`). It could not reach a new daemon anyway: the port and
    token were fixed at launch.
  - The native reader lives in the daemon, so subagent transcripts stop too.
  - The watch tick reads "no answer" as `Recording::Unknown` and says nothing (`crates/goalcup-cli/src/run/watch.rs:389-391`,
    `run/mod.rs:505-509`).
- **Result:** a daemon crash on day 1 leaves 6 days unrecorded, and the participant is not told. Points 1, 2 and 6 fail.
  Point 3 holds (the harness runs on, because the relay never stops).
- **Change for Friday:** in the watch tick (every 30 s, `crates/goalcup-cli/src/run/reserve.rs:41`), count ticks in a
  row where the daemon did not answer. After 3 (about 1.5 min plus 30 s call timeouts), write once to `host.log` and
  `state.json`: "Recording has stopped. When the agent finishes its current step, exit it and run goalcup continue."
  `goalcup status` reads it back. Also say it when the proxy health shows the sink disabled. About +25 src, +30 tests.
- **Bigger option (not for Friday):** let a restarted daemon admit a fresh producer into the open entry (the sender
  already has a fresh-epoch `Recovery` path), and let the CLI restart the daemon and hand the proxy the new port and
  token. That is new code across the daemon, sender and CLI, and it touches the orphan rule. Only worth it if the
  2-hour or 24-hour runs show daemon deaths.
- **Risk:** a daemon that is alive but slow (a long chain reopen) can trigger the line. Wording says "may have stopped"
  and `goalcup status` confirms. If F2 removes `reserve.rs`, the tick constant must move with it.

## Findings: the server

### F13. Worker down for hours mid-run (keep)

- **Points:** 5.
- Upload: "No answer ever ends a run's upload (D64)"; chunks every 30 min or at 256 MiB, backoff 60 s to 15 min,
  `Retry-After` honoured (`crates/goalcup-cli/src/upload/policy.rs:1-7, 76-90, 283-290`; `upload/http.rs:44-59`). The
  client has a 15 s connect and 60 s call timeout (F7).
- Deadline: kept locally as well as from the server (`crates/goalcup-cli/src/upload/uploader/lifecycle.rs:129-133`,
  `run/time_left.rs:16`), so a dead Worker does not stop a run from ending on time.
- The server does not abandon a silent run; it only sends a notice email (`server/worker/src/notices.ts:19-23, 151-163`).
- Edge: the credential ends 24 hours after the deadline (`server/worker/src/db.ts:55-62`). A Worker outage longer than
  the run's last day plus 24 hours would strand the tail of the upload. Owner decision only; no code for Friday.
- The Worker runs on a database backend (`server/worker/src/db-supabase.ts`); a backend outage is a Worker outage for
  the client.
- **Verdict:** keep the upload; cut the disk pause (F2). Risk: the local chain grows while nothing is acknowledged
  (F2).

### F14. Worker down when `goalcup continue` runs (change)

- **Points:** 3.
- Today: `continue` makes every local refusal first, then asks the server for the next part, then launches
  (`crates/goalcup-cli/src/commands/continue_run.rs:17-28`, `commands/start/server.rs:395-450`). The call is tried 3
  times, 3 s apart; then "Nothing was started" (`crates/goalcup-cli/src/upload/mod.rs:44-45, 106-117, 177-208`).
- Week-long risk: the agent crashes at night; the Worker (or its database) is down; the participant cannot restart
  their agent under recording while the 168-hour clock runs. Point 3 fails.
- Change for Friday: in `continue` only, keep asking with backoff up to 30 minutes, printing one line per minute ("The
  Goal Cup server is not answering; still trying. Ctrl-C to stop."). About +15 src, +10 tests.
- Later (protocol change, fits "the file path is the ID"): the client names the next part itself (the run id plus part
  number), launches at once, and the server accepts the part on its first upload. That removes the server from the
  restart path. Blind spot: the server may refuse the part later (a run already over, or continued elsewhere), after
  the participant has worked on it.
- `goalcup start` also needs the Worker for approval (`commands/start/server.rs:117-195`). A Worker outage at the start
  window is the owner's operations risk, not harness code.
- Risk if we do it: the participant still waits during a long outage.

### F15. A goalcup update during the week (keep, process)

- **Points:** 7.
- `MIN_COMPETITION_VERSION` is checked only when a competition run starts (`server/worker/src/runs.ts:150-164`), not on
  `continue`. A client and daemon of different control versions are told apart
  (`crates/goalcup-cli/src/daemon/spawn.rs:315-342`). Older binaries refuse the `final_snapshot` form (progress
  tracker, Task 6).
- Rule: no client release between 6 Nov and the end of the last run, except a fix the owner signs off. No code.

## Findings: harnesses and providers

### F16. Claude Code updates itself (keep, one-line change)

- **Points:** 1, 7.
- Today: the connector sets `DISABLE_AUTOUPDATER = "1"` (`connectors/claude-code-cli/harness.toml:60`).
- Vendor docs: native installs "check for updates on startup and periodically while running. Updates download and
  install in the background, then take effect the next time you start Claude Code." The launcher `~/.local/bin/claude`
  is a symlink into `~/.local/share/claude/versions/`, and the installer keeps "any version a session on the same
  machine is running" ([setup docs](https://code.claude.com/docs/en/setup)). So a running session is not replaced
  under itself.
- Vendor docs: "`DISABLE_AUTOUPDATER` only stops the background check; `claude update` and `claude install` still
  work. To block all update paths ... set `DISABLE_UPDATES`" ([setup docs](https://code.claude.com/docs/en/setup),
  [env vars](https://code.claude.com/docs/en/env-vars)).
- Week-long risk: the agent itself can run `claude update` in its shell, and the participant's own `claude` outside the
  run (no variable set) can move the symlink. The next `goalcup continue` then starts a newer build. That is allowed
  (D53: "a version never stops a run") and each part records the version and the build digests (F19).
- What a newer build can change: hosts (a new host would be tunnelled, not recorded, and listed only in the tunnel
  file, F25) and file layout (the subagent rows use `warn_missing = false`,
  `connectors/claude-code-cli/harness.toml:116-150`, so a moved folder is silent). Wire capture still covers the model
  calls if the host stays the same.
- Change: `DISABLE_UPDATES = "1"` instead of `DISABLE_AUTOUPDATER`. One line (1 src / 0 tests). It stops the agent's
  own update inside the run. It does not stop the participant's own install outside the run.
- Blind spot: a model the participant picks may need a newer build than the one they started with
  ([setup docs](https://code.claude.com/docs/en/setup), "A newly launched model can require a Claude Code version
  newer than the stable channel serves"). They can still update between parts by hand.

### F17. Codex update prompt (keep)

- **Points:** 3, 7.
- Today: `-c check_for_update_on_startup=false` (`connectors/codex-cli/harness.toml:46`). The Codex config reference
  lists only this startup check, no background self-replace
  ([config reference](https://learn.chatgpt.com/docs/config-file/config-reference)).
- Why it matters: the update selector is a modal that swallows a typed turn (`docs/harnesses/codex-cli.md:179`).
- Risk of keeping: none.

### F18. Hermes and Amp updates (keep, one guide line)

- **Points:** 1, 7.
- Hermes: no background updater found. `hermes update` is manual and refreshes running gateways
  ([mirrored Hermes docs](https://hermesagent.org.cn/en/docs/getting-started/updating); third-party mirror,
  unverified against the repository).
- Risk: Hermes 0.22 sends model calls through a gateway "which keeps the environment of whoever started it"
  (`connectors/hermes-cli/harness.toml:134-145`). If the participant runs `hermes update` during a run, the refreshed
  gateway may start without the recording proxy and send calls unrecorded (unverified). The connector stops the
  gateway only before launch and after exit.
- Change: one line in the Hermes section of the setup guide: "Do not run `hermes update` while a run is open." Pin a
  Hermes version for the week, as the phase 2 notes ask (`docs/plans/2026-10-29-record-everything-phase2-notes.md:182-185`).
- Amp: `AMP_SKIP_UPDATE_CHECK = "1"` (`connectors/amp-cli/harness.toml:45`), measured to stop the version check
  (`docs/harnesses/amp-cli.md:132`). Keep.

### F19. Version line and build digests per part (keep)

- **Points:** 6, 7.
- `harness_version.rs` (202 src / 1 test line) runs `<program> --version` once per launch
  (`crates/goalcup-cli/src/harness_version.rs`). `harness_build.rs` (89 src / 90 tests) hashes the program file and the
  running image at the launch post, after the harness was released, so it never delays the harness
  (`crates/goalcup-daemon/src/runtime/capture/harness_build.rs:1-30`, `runtime/capture/mod.rs:761-772`).
- Over a week this is the only record that the harness changed between parts (F16). `docs/CHEAT-CHECKLIST.md:102`
  relies on the digests.
- Risk of keeping: none for the harness. Hashing a 200 MB binary costs a few seconds of daemon time per part.

### F20. Sign-in expiry and token refresh through the proxy

- **Points:** 3, 7.
- What happens today:
  - The proxy intercepts only declared model hosts. "Authentication hosts, other hosts and mismatched ClientHellos
    tunnel opaquely. Upstream TLS is always validated." (`docs/ARCHITECTURE.md:212-215`). So a refresh call reaches the
    real login host with its real certificate.
  - Claude Code, Codex and Amp add the proxy CA to their roots (`mode = "extend"`), so the real certificate still
    verifies. Refresh works.
  - Codex refreshes its token bundle when `last_refresh` is about 8 days old
    ([Codex CI/CD auth](https://developers.openai.com/codex/auth/ci-cd-auth); the 8-day figure is from secondary
    sources, unverified). A participant who signed in more than a day before the start will refresh inside the 168
    hours. That goes through the tunnel and works. Copying `auth.json` elsewhere and refreshing there invalidates the
    run's token (`docs/harnesses/codex-cli.md:144`); the shared, serialized `CODEX_HOME` already handles this. Keep.
  - Hermes is the exception. `HERMES_CA_BUNDLE` replaces the roots with the proxy CA alone
    (`connectors/hermes-cli/harness.toml:88-92`). A refresh to an OAuth login host (Nous Portal, Codex-OAuth,
    xAI-OAuth) tunnels with its real certificate and fails its handshake. Model calls then fail once the token from prep
    expires, and the start does not refuse (`docs/harnesses/hermes-cli.md:106`, `TODO.md:337-341`). Traced from source,
    not measured.
- Verdict:
  - Keep the tunnel for auth hosts. `auth_hosts` itself is small: it only marks hosts for connect retry and outage
    counting (`crates/goalcup-cli/src/proxy/connect.rs:277, 906`) and for the setup probe
    (`crates/goalcup-cli/src/commands/prep.rs:1035-1043`).
  - Change for Hermes: at `prep` and `start`, if Hermes's `auth.json` holds an OAuth provider for the configured
    service, say plainly that the sign-in will not refresh during a week-long run and ask for an API-key service. This
    is the open item A11 (`TODO.md:162`). About +20 src, +15 tests.
- Risk: participants whose only Hermes access is a subscription login cannot do a week-long Hermes run. A 2-hour run is
  fine if the token from prep lasts that long (unverified).
- Related: upstream TLS for every other host trusts only the Mozilla roots (F7).

### F21. Rate limits, 429 storms, provider outages (keep)

- **Points:** 1, 3.
- Every request on an opened host is recorded, error answers included (Task 1, progress tracker). A 429 is a recorded
  exchange like any other.
- The proxy retries only the TCP connect, only for provider hosts, only until the admission deadline (30 s, minus a
  reserve of up to 5 s for TLS). It never replays a request because nothing was sent yet
  (`crates/goalcup-cli/src/proxy/connect.rs:79-95, 880-1000`; the findings table in the harness part cites
  `79-89, 888-1000`). Other hosts get one round, so the agent's own traffic is not changed.
- Claude Code: the connector sets `CLAUDE_CODE_RETRY_WATCHDOG = "1"` (`connectors/claude-code-cli/harness.toml:61-65`).
  Vendor docs: it "Retries `429` and `529` capacity errors indefinitely", backs off up to 5 minutes or until the limit
  resets, "so a session that hits a usage limit waits out the remaining window", and raises other transient retries to
  300, "roughly three hours of backoff". A spend-limit or exhausted-credit 429 still fails at once
  ([env vars](https://code.claude.com/docs/en/env-vars)). The API's spend-cap 429 has no `retry-after`
  ([rate limits](https://platform.claude.com/docs/en/api/rate-limits)).
- Codex: `request_max_retries` default 4 and `stream_max_retries` default 5 per provider
  ([config reference](https://learn.chatgpt.com/docs/config-file/config-reference)). A long outage ends the turn; the
  session stays open and waits for the participant. The connector sets no limit (D52), so this stays the
  participant's setting.
- Hermes: retry behaviour not measured.
- Verdict: keep all of it. The connect retry is about 60 src lines and helps Codex and Hermes over short DNS or TCP
  blips.
- Blind spot: a Claude Code run that reaches a weekly subscription limit can sit waiting for days. It is recorded (one
  429 every few minutes), heartbeats keep coming, so the server's "silent run" notice does not fire
  (`server/worker/src/notices.ts:19-23`). The participant may think the run is working. Optional, later: a
  `goalcup status` line when the last N recorded answers were all 429. Not needed for Friday.

### F22. Outage windows (change; the parts differ)

- **Points:** 5, 6.
- Today: failed provider connects coalesce into windows, at most 32 kept, "reported by `goalcup start` at finish, not
  chained" because "the v5 chain is frozen" (`crates/goalcup-cli/src/proxy/outage.rs:1-15, 21`). 144 src / 172 tests.
- Harness part: a failed TCP connect leaves no request record. So over a week the only trace of an outage is a terminal
  line at finish, lost if `goalcup start` dies, and never uploaded. That misses point 5 (the server cannot rebuild the
  story by time) and point 6. The freeze no longer applies (owner decisions, 29 Oct). Change: when a window closes (or
  at finish if still open), append one stream-less record with first failure, last failure, count and recovery time.
  Drop `MAX_KEPT_OUTAGE_WINDOWS`, `finish_lines` and their tests.
- OS part: lists the 32-window list as bounded memory and keeps it (F4).
- Lines: about -60 src and -120 tests in `outage.rs`, about +25 src in the emitter and protocol.
- Risk: a flapping network makes one record per window; coalescing keeps that small. Verifier and Worker must accept
  the new record (they change together now).

### F23. The harness starts fresh sessions; the 64 MiB finish cap (keep patterns, lift one cap)

- **Points:** 1, 2.
- Claude Code: every transcript new or written since the run started is read
  (`connectors/claude-code-cli/harness.toml:96-150`). A new session after `/clear` or a restart is a new file and is
  picked up. Compaction is written in the same file (`isCompactSummary`, `docs/harnesses/claude-code-cli.md:57`).
  Vendor docs say auto-compaction applies to the main conversation and subagents
  ([env vars](https://code.claude.com/docs/en/env-vars), `CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`).
- Codex: each session is a new rollout under the same pattern (`connectors/codex-cli/harness.toml`, rollout row).
  Compaction is a `compacted` record in the rollout (`docs/harnesses/codex-cli.md:135`).
- Hermes: `compression.in_place false` makes each compaction a new session file
  (`connectors/hermes-cli/harness.toml:45-57`). Keep: without it the file freezes. A snapshot file is read again whole
  each time it changes (`crates/goalcup-daemon/src/native_capture.rs:47-50`), so the new file per compaction also
  keeps re-reads small.
- Hermes 0.21.1 and later write no session file (D72). Their native leg is `state.db`, read once at each part's seal
  (`connectors/hermes-cli/harness.toml:112-120`), inside `NATIVE_FINISH_BYTES` of 64 MiB
  (`crates/goalcup-daemon/src/runtime/capture/owner.rs:13`). A week of Hermes on a shared home can pass 64 MiB; the
  rest becomes a gap (phase 2 notes, Part D risk 3). A killed harness keeps no copy.
- Change: give `final_snapshot` reads no byte cap of their own, only the existing finish time budget
  (`LOCAL_CAPTURE_FINISH_BUDGET`, 180 s, `crates/goalcup-protocol/src/control.rs:708`). One constant (1 src / 0 tests).
- Risk: a very large `state.db` makes the seal take longer.

### F24. `goalcup continue` after a crash (keep, measure)

- **Points:** 3, 7.
- Closing the old part: sealed by its host, sealed now, or closed from the chain tail with no repair
  (`crates/goalcup-cli/src/commands/continue_run.rs:85-100`). Keep.
- Resume flags: Claude Code `--continue` measured to append to the same file; Hermes `--continue` measured; Codex
  `resume --last` and Amp `last` "not yet measured against a logged-in home" (connector comments at
  `connectors/codex-cli/harness.toml:50-52`, `connectors/amp-cli/harness.toml:31-33`).
- The per-launch CA lasts 8 days (`crates/goalcup-cli/src/proxy/tls.rs:52-59`), and every part makes a new one. A
  single part never outlives it. Keep.
- Change: none in code. Measure Codex `resume --last` in the 2-hour run. Risk if it fails: Codex opens a fresh session,
  which is still recorded, but the agent loses its context.

### F25. Sealed-connection (tunnel) list (change; the parts differ on how) (merged: OS F4 change, harness F16)

- **Points:** 5, 6.
- Today: written once at finish, the first 10,000 rows kept, then only counts (`crates/goalcup-cli/src/proxy/tunnels.rs:1-20`).
  Per-connection byte counts are only shown, never uploaded (`tunnels.rs:1-4`).
- Why it matters: after a day of `npm install` loops the list stops naming new hosts, so a sealed model host that shows
  up on day 5 is not listed for the participant (point 6). After a harness update moves a model call to a new host
  (F16), this list is the only place that host shows. A crash loses it.
- OS part's change: one row per (host, port, reason) with counts and first/last time, not the first 10,000
  connections. A map keyed by host is bounded by the number of distinct hosts, so it needs no cap of our own. About 30
  src changed in `tunnels.rs`, tests adjusted. Risk low; the per-connection byte counts are lost, and since they are
  never uploaded nothing on the server changes.
- Harness part's change: append each row when the tunnel closes, upload it with the parts, drop the cap. Small.
  Overlaps the multi-provider lane. Risk: the file grows with a busy agent.
- The two can combine: per-host rows written as they change and uploaded would fix crash loss, the cap and growth.

## Harness and provider table (as written in the harness part)

The "Part id" column is the id in the harness part; "ID" is the id in this file.

| ID | Part id | Risk | Points | Code today | Verdict | Lines involved (src / tests) | Risk if we do it |
|---|---|---|---|---|---|---|---|
| F16 | F1 | Claude Code updates itself | 1, 7 | `DISABLE_AUTOUPDATER = "1"` (`connectors/claude-code-cli/harness.toml:60`) | change one line to `DISABLE_UPDATES = "1"` | 1 / 0 | none known |
| F17 | F2 | Codex update prompt | 3, 7 | `check_for_update_on_startup=false` (`connectors/codex-cli/harness.toml:46`) | keep | 0 | none |
| F18 | F3 | Hermes and Amp updates | 1, 7 | Hermes: nothing; Amp: `AMP_SKIP_UPDATE_CHECK` (`connectors/amp-cli/harness.toml:45`) | keep, add one guide line for Hermes | docs only | none |
| F19 | F4 | Knowing which build ran in each part | 6, 7 | version line per launch (`crates/goalcup-cli/src/harness_version.rs`, 202 / 1), build digests (`crates/goalcup-daemon/src/runtime/capture/harness_build.rs`, 89 / 90) | keep | 0 | none |
| F20 | F5 | Sign-in refresh through the proxy | 3, 7 | auth hosts are tunnelled with their real certificate; Hermes trusts only the proxy CA (`connectors/hermes-cli/harness.toml:88-92`) | keep for Claude Code, Codex, Amp; change for Hermes (refuse or warn on an OAuth provider) | about +20 src | participants on a Hermes OAuth provider are turned away |
| F21 | F6 | 429 storms and provider outages | 1, 3 | every request is recorded; proxy retries a provider TCP connect until the 30 s admission deadline (`crates/goalcup-cli/src/proxy/connect.rs:79-89, 888-1000`); Claude Code watchdog (`connectors/claude-code-cli/harness.toml:61-65`) | keep | 0 | none |
| F22 | F7 | Outage windows | 5, 6 | counted in memory, 32 kept, printed at finish only, never uploaded (`crates/goalcup-cli/src/proxy/outage.rs:1-15, 21`) | change: write each closed window to the chain; drop the 32 cap and the finish-only print | about -60 src, -120 tests, +25 src | more records in a long outage (still one per window) |
| F10 | F8 | Recorder accept loop dies | 3 | restarted up to 5 times per run, the count carried across `continue` (`crates/goalcup-cli/src/proxy/mod.rs:1018-1110`, `commands/start/phases.rs:518-529, 1060-1073, 1505-1510`) | change: no cap, 1 s pause between restarts, no carry | about -40 src, -20 tests | a loop that panics at once restarts once a second forever (cheap) |
| F11 | F9 | `goalcup start` (the proxy) crashes | 1, 3 | nothing restarts it; the harness keeps a dead `HTTPS_PROXY` | keep; add one guide line ("exit the agent, run `goalcup continue`") | docs only | the harness's calls fail until the participant acts |
| F12 | F10 | The daemon crashes mid-run | 1, 2, 6 | not restarted during a run; a restarted daemon admits no producer again (`crates/goalcup-daemon/src/runtime/capture/mod.rs:68-73`); the sender gives up after 10 failures in a row (`crates/goalcup-cli/src/proxy/ingest.rs:93-108`); the watch tick says nothing (`crates/goalcup-cli/src/run/mod.rs:505-509`) | change: tell the participant after 3 silent ticks | about +25 src, +30 tests | a slow but alive daemon may trigger a false alarm |
| F13 | F11 | Worker down for hours mid-run | 5 | upload and heartbeats retry forever, 60 s to 15 min backoff (`crates/goalcup-cli/src/upload/policy.rs:76-90, 283-290`); the deadline is also kept locally | keep the upload; cut the disk pause (see F2) | 0 | none |
| F2 | F12 | Worker down plus a filling disk freezes the agent | 3 | below 1 GiB free the watch tick sends `SIGSTOP` to the harness (`crates/goalcup-cli/src/run/pause.rs:1-20`, `run/reserve.rs:31-35`) | cut (owner: disk full gets no special handling) | about -211 src, -149 tests, plus pause branches in `run/watch.rs` and `commands/start/phases.rs:1090-1095` | a full disk now stops the harness by itself, as the owner chose |
| F14 | F13 | Worker down when `goalcup continue` runs | 3 | 3 tries 3 s apart, then "Nothing was started" (`crates/goalcup-cli/src/upload/mod.rs:44-45, 106-117, 177-208`) | change: keep asking with a visible line for up to 30 min (Friday); later, let the client name the next part and tell the server on upload | about +15 src (Friday) | the participant still waits during a long outage |
| F23 | F14 | Harness hits its context or session limit and starts fresh | 1, 2 | new files are found by pattern with `since_run_start`; Hermes `compression.in_place false` makes a new file per compaction (`connectors/hermes-cli/harness.toml:45-57`); Hermes `state.db` read once at seal inside a 64 MiB finish budget (`crates/goalcup-daemon/src/runtime/capture/owner.rs:13`) | keep the patterns; change: lift the 64 MiB finish cap for `final_snapshot` | 1 / 0 | a long seal on a huge `state.db` |
| F24 | F15 | `goalcup continue` after a crash | 3, 7 | closes the old part (seal, or from the chain tail) then resumes with `resume_args` (`crates/goalcup-cli/src/commands/continue_run.rs:85-100`) | keep; measure Codex and Amp resume in the 2-hour run | 0 | Codex may open a fresh session (still recorded) |
| F25 | F16 | Sealed-connection list | 5 | written once at finish, 10,000 rows kept (`crates/goalcup-cli/src/proxy/tunnels.rs:1-20`) | change: append as tunnels close, drop the cap | small | file grows with a busy agent |
| F15 | F17 | A goalcup update during the week | 7 | minimum version checked at start only (`server/worker/src/runs.ts:150-164`); older binaries refuse `final_snapshot` (progress log, Task 6) | keep code; process rule: no client release after 6 Nov | 0 | none |

## Id map (OS part)

| ID here | OS part id |
|---|---|
| F1 | F1 |
| F2 | F2 (merged with harness F12) |
| F3 | F3 |
| F4 | F4 (its tunnel change moved to F25) |
| F5 | F5 |
| F6 | F6 |
| F7 | F7 |
| F8 | F8 |
| F9 | F9 |
| F10 | F10 (merged with harness F8) |

## Lines at a glance (whole lane)

Negative is a cut, positive is new code. Where the parts give different numbers, both are shown.

| Change | src | tests |
|---|---|---|
| F2 cut reserve, pause, pressure pruning (OS part scope) | about -1,000, plus plumbing in `start/phases.rs` and `run/mod.rs` | about -1,800 |
| F2 cut disk pause only (harness part scope) | about -211 plus callers | about -149 |
| F2 truncate to last whole record on `ENOSPC` | about +10 | +1 test |
| F3 `NATIVE_EVERY` to 1 s (or directory grouping) | 1 (or about 40) | 0 |
| F1 raise stream caps, move rlimit raise | about 10 changed | 0 |
| F5 heartbeat and upload at once after wake (optional) | about +10 | not given |
| F8 cut older-server deadline fallbacks | about -40 | about -20 |
| F10 restart cap removed | OS: about -60; harness: about -40 | OS: about -50; harness: about -20 |
| F12 tell the participant | about +25 | about +30 |
| F14 continue waits for the server | about +15 | about +10 |
| F20 Hermes OAuth warning | about +20 | about +15 |
| F22 outage windows to the chain | about -60, +25 | about -120 |
| F25 tunnel list per host (OS) or append and upload (harness) | about 30 changed (OS); small (harness) | adjusted |
| F16, F23 one constant or line each | 2 | 0 |
| F6, F7, F9, F11, F18 | docs only | 0 |

## Ranked lists from the two parts

OS part ranking:

| Rank | Finding (ID here) | Verdict | Src / test lines | Why first |
|---|---|---|---|---|
| 1 | F2 Disk: cut reserve, pause, pressure pruning; truncate to last whole record on `ENOSPC` | cut + small change | about -1,000 / -1,800; +10 / +1 test | Biggest cut; the pause blocks the harness (point 3); owner already decided |
| 2 | F3 Native polling every 100 ms over all tracked files | change (1 constant) | 1 | Becomes the real cost once the file cap is removed for thousands of subagents |
| 3 | F1 Stream caps versus handles | change caps, keep handle budget, move the rlimit raise to `main` | about 10 | Answers the owner's handle question: raising caps costs no handles |
| 4 | F5 Lid close beats any sleep lock | keep code, change words | 0 to 10 | Most likely way a laptop run silently loses hours |
| 5 | F6 Closed terminal, updates | keep code, docs: tmux for all, pause OS updates | 0 | Second most likely way to lose hours |
| 6 | F10 Accept-loop restart cap | change | about -60 / -50 | A cap of our own that ends in a blocked harness |
| 7 | F8 Clocks | keep; cut older-server fallbacks | about -40 / -20 | Correct today; small cut |
| 8 | F25 (OS F4) Tunnel list capped at 10,000 rows | change to per-host rows | about 30 | Point 6 accuracy late in a week |
| 9 | F7 Network: no upstream proxy, Mozilla roots only | keep; docs | 0 | Rare, but total failure for that participant |
| 10 | F9 WSL2 | keep; docs | 0 | Covered by heartbeat refresh; still unmeasured on a real machine |

Harness part ranking (most important first):

1. **F12 (part F10), daemon crash is silent.** Days can go unrecorded and nobody is told. Add the "recording has
   stopped" line for Friday.
2. **F2 (part F12), the disk pause freezes the agent**, and a long Worker outage makes it more likely. Cut it; it is
   also the owner's decision.
3. **F14 (part F13), `goalcup continue` needs the Worker.** Make it wait and say so for Friday; plan a client-named part
   later.
4. **F20 (part F5), Hermes OAuth refresh fails** under the replace-mode CA. Warn or refuse at prep and start.
5. **F23 (part F14), the 64 MiB finish cap** cuts a week-long Hermes `state.db`. Lift it.
6. **F10 (part F8), the cap of 5 recorder restarts** per run. Remove it.
7. **F22 and F25 (part F7 and F16), outage windows and tunnels are finish-only** and never uploaded. Write them as they
   happen.
8. **F16 and F18 (part F1 and F3), updates.** `DISABLE_UPDATES` for Claude Code; a guide line and a pinned version for
   Hermes.
9. **F24 and F21 (part F15 and F6), measure** Codex resume in the 2-hour run; consider a status line for a run stuck on
   429s.

## Sources (web)

- [logind.conf, LidSwitchIgnoreInhibited](https://www.freedesktop.org/software/systemd/man/latest/logind.conf.html)
- [caffeinate(8) on ss64](https://ss64.com/mac/caffeinate.html)
- [Macworld: MacBook with the lid closed](https://www.macworld.com/article/673295/how-to-use-macbook-with-lid-closed-stop-closed-mac-sleeping.html)
- [Apple setrlimit(2), OPEN_MAX note](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/setrlimit.2.html)
- [QuestDB: macOS max open files](https://questdb.io/blog/max-open-file-limit-macos-jvm)
- [Apple developer forums: maxfiles](https://developer.apple.com/forums/thread/735798)
- [Proton wiki: systemd file descriptor defaults](https://github.com/ValveSoftware/Proton/wiki/File-Descriptors)
- [Caddy forum quoting systemd on select(2)](https://caddy.community/t/systemd-service-file-directives/19114)
- [Rust std unix time source](https://doc.rust-lang.org/src/std/sys/time/unix.rs.html) and [Instant docs](https://doc.rust-lang.org/std/time/struct.Instant.html)
- [WSL configuration (instanceIdleTimeout, vmIdleTimeout)](https://learn.microsoft.com/en-us/windows/wsl/wsl-config)
- [WSL 2 clock drift](https://omkarmoghe.github.io/2020/08/07/WSL-2-Clock-Drift.html), [scivision: WSL date skew](https://scivision.dev/wsl-date-time-skew-error)
- [Microsoft: manage device restarts after updates](https://learn.microsoft.com/windows/deployment/update/waas-restart)
- [timeanddate: DST 2026](https://timeanddate.com/time/dst/2026.html)
- [Claude Code setup and updates](https://code.claude.com/docs/en/setup)
- [Claude Code environment variables](https://code.claude.com/docs/en/env-vars)
- [Claude API rate limits](https://platform.claude.com/docs/en/api/rate-limits)
- [Codex config reference](https://learn.chatgpt.com/docs/config-file/config-reference)
- [Codex CI/CD auth](https://developers.openai.com/codex/auth/ci-cd-auth) (8-day refresh: secondary sources, unverified)
- [Hermes updating (mirror)](https://hermesagent.org.cn/en/docs/getting-started/updating) (third-party mirror, unverified)
