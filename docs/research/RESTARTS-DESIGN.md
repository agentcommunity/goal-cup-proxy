# Restarts across session limits

22 Oct 2026. Some participants run in a cloud sandbox or VM that stops the session every 24 hours.
A run lasts 168 hours, so such a run stops six or seven times. This note says what Goal Cup Proxy
does today at each kind of stop, what the participant types, what survives, what the server and
the verifier conclude, the rough edges found, and the rules questions for the owner.

Words are the [glossary's](../ARCHITECTURE.md#12-terms): a run has parts (segments); `goalcup
continue` starts the next one (D4, D57).

## The short version

- **A stop with the disk kept works today.** After any stop, `goalcup continue` closes the part
  that was cut off, sends what it recorded, and opens the next part in the same clone, with the
  same deadline and the harness's resume arguments. No approval is asked. Each cut-off part keeps
  one named gap. A new test runs this across two stops (below).
- **A stop that wipes the disk ends the run's recording.** `goalcup continue` works only where the
  run's files are. A sandbox must keep `$HOME` (or wherever the three goalcup folders are) on a
  disk that survives the stop, at the same path, for the same user.
- **One defect fixed here.** A continue whose clone was gone (work folder on a disk that did not
  survive, or a folder moved) made the folder again, empty, and the agent went on in it with none
  of its work. It is now refused before anything changes.
- **Three rules questions** go to the owner (last section). Today's rules allow a restart after a
  crash or reboot "on the same machine" with only "keep going" typed; they do not name platform
  caps or a new VM.

## What must survive a stop

Everything is under `$HOME` by default (Linux):

| Path | Holds | Lost with it |
|---|---|---|
| `~/.config/goalcup` | run records, the run's credential, upload spool, state, harness homes (sessions, sign-in), time-left file | `continue` and `upload` refuse: "not started on this computer" |
| `~/.config/goalcup-artifacts` | each part's chain files, seal, receipt, the run's disk reserve | the unsent tail of the chain and every seal |
| `~/goalcup-work/<clone>` (prep's `--work-root`) | the clone the agent works in | all unpushed work; `continue` now refuses |
| the folder `start` or `continue` ran in | each part's own folder (`HOME` of a harness with `run_home = "empty"`) | nothing needed later |

`GOALCUP_CONFIG_DIR` moves the first two together. The harness's own sessions live in the shared
harness home under `~/.config/goalcup/capture-homes/` (D39, D59), so `resume_args` finds them
when that folder survives. Claude Code (`--continue`), Codex (`resume --last`) and Amp (`last`)
pick the last session of the current folder, so the clone must also keep its absolute path.

## What happens at each kind of stop

"Host" is the `goalcup start` or `goalcup continue` process that owns the part.

| Stop | What runs on, what is left | What the participant types |
|---|---|---|
| **A. The harness alone exits or is killed** | The host seals the part, sends the final heartbeat, uploads, says `To go on, run: goalcup continue <run id>`, exits 0. Clean part. | `goalcup continue` |
| **B. Session cap: host and harness killed (SIGKILL), daemon lives** | Nothing is sealed. The daemon (detached, D4) keeps the part open. Up to 30 min of chain is not sent yet (uploads go every 30 min). `goalcup status`: `goalcup has not updated this run for …`, `Next: run goalcup continue <run id>`. | `goalcup continue`: says `The earlier part of this run was still open, so it was sealed now.` The part has one `IngestLost` gap (verifier exit 1). |
| **C. Graceful shutdown (SIGTERM to every process, then SIGKILL)** | Same as B or D. The host does not catch SIGTERM (`goalcup-platform/src/terminal.rs`, `CAUGHT`), and the daemon on SIGTERM "seals nothing" (`goalcup-daemon/src/bin/goalcup-daemon.rs`). | as B or D |
| **D. Power-off or VM killed, disk kept** | Nothing runs after boot; no service restarts the daemon (D4). | `goalcup continue` starts a daemon, which reopens the part for status and seal only; continue seals it (one `IngestLost` gap, `producer_unclosed`, exit 1). A last line cut mid-write cannot be reopened: the part is closed at its last whole record, unsealed, nothing repaired (OPERATING drill 5). |
| **E. Disk wiped, or a new machine** | The server keeps what it acknowledged. The run stays `open` until its deadline, then turns `incomplete` 24 h later (its last part has no seal). | Nothing works: `goalcup continue` refuses (`commands/continue_run.rs`, `resolve`). Inside the start window a new competition start replaces the run and needs a new empty repository; after it, nothing. |
| **F. Disk kept, clone gone** (work folder on scratch storage) | as B or D | Before this change: the agent went on in an empty folder. Now: `Put the run's folder back at <path> and run goalcup continue again. If it is lost, write to goalcup@agentcommunity.org …` |

In B and D the participant may also end the part without going on: `goalcup seal <part> --yes`
from another terminal, then `goalcup upload`.

A cut-off part's upload backlog is not lost while the disk is kept. Each `goalcup continue`
looks at every earlier part not yet sent, closes it, and sends it on a thread of its own beside the
new part. A continue that is itself killed leaves the work for the next one. The new test checks
this with the server refusing chunks across both stops.

## The clock across stops

- The deadline never moves. It is the server's (`deadline_at`), and every part's signed receipt
  carries the same value. The server's `continue` refuses a run that is past its end.
- The host counts down from the server's `seconds_left` on a clock that runs through sleep
  (WP11). A reboot resets that clock, so after a reboot the stored count says nothing and
  `goalcup status` uses the server's `deadline_at` against the machine's date.
- The time-left file (D66) of a killed host stays behind with an old `as_of`; `goalcup
  time-left` calls it unknown after 5 minutes. No agent runs then. The next part's host writes it
  again from the `continue` answer at its first watch tick, with the same deadline. The new test
  checks this.
- Time between parts is not a gap record. Each part's receipt (`started_at`) and its last
  heartbeat show it, and RULES says downtime while nothing happened is an outage.

## What the server and the verifier conclude

- One run, one entry per part, in order (`segment` 0, 1, 2 …). Each part's chain verifies on its
  own: a part cut off by a stop is sealed with its named gap (exit 1), or, with a torn last line,
  closed unsealed at its last whole record (exit 2, named). Nothing is repaired or merged.
- Heartbeat silence over 20 minutes sends the participant an email; the run is never closed for
  silence before its deadline (`server/worker/src/db.ts`, `statusAt`).
- The server's `continue` checks only the run, its credential and that the previous part is the
  latest one. A continue from a second copy of the config folder is refused
  (`continued_elsewhere`). Machine facts are sent on the first open only, so a restored disk on a
  new VM looks like a reboot; only the client IP and country differ.

## Rough edges and defects

| # | What | Evidence | Proposal |
|---|---|---|---|
| 1 | **Fixed** (877c0be). Continue with the clone gone made an empty folder and ran the agent in it. The daemon admits a missing workspace as new and empty (`goalcup-daemon/src/workspace.rs`, `check_workspace_available`). | test `restarts::a_continue_whose_clone_is_gone_is_refused_and_changes_nothing` (failed before the fix: the harness started) | `continue` refuses unless the run's folder is a clone, before the home lock and the server. |
| 2 | A wiped disk or new machine ends recording for good. | `resolve` in `commands/continue_run.rs`; RULES "on the same machine" | Rules question 3. If allowed, a "move" would need the three folders restored at the same paths; nothing records that the machine changed. |
| 3 | SIGTERM is treated like SIGKILL. A graceful platform stop gives no seal and no last upload, so a wiped disk loses up to 30 min plus the seal. | `terminal.rs` `CAUGHT` has no SIGTERM; daemon "seal nothing" | Catch SIGTERM in the host and send the backlog at once (seal if the grace allows). Until then, a participant who knows the cap time can run `goalcup seal` before it (rules question 2). |
| 4 | Continue needs the server. A server outage at restart time keeps the agent down. | `upload_e2e::segments::a_continue_with_the_server_away_creates_nothing_and_a_later_one_goes_on` | Accept (D64 covers a running part, not a new one), or open the part locally and register it later: large. |
| 5 | Each part records again every session file written since the run started, whole (D57 `since_run_start`). With k stops the native leg is recorded about k/2 times over, which costs disk reservation and upload. | probe in the new test: part 2's native bytes held the lines of parts 0, 1 and 2 | Accept for 7 parts, or later start each part's read where the previous part's seal left the file. |
| 6 | Amp's `resume_args = ["last"]` is not measured against a logged-in home. | `connectors/amp-cli/harness.toml` comment | Measure on the 24 h step. |

No defect was found in the clock, the reserve hand-over, the shared-home lock, or the backlog
across several stops.

## Advice for a sandbox with a 24-hour cap (if the owner allows it)

1. Before the run: put `$HOME` on a persistent volume that is mounted at the same path for the
   same user after each stop. Run `goalcup prep` there. Start the run inside `tmux`.
2. After each stop: open a terminal, `cd` to the folder you ran `goalcup start` from (not inside
   the work folder), run `goalcup status`, then `goalcup continue`. Answer the harness's start-up
   screens. If the agent has stopped, type `keep going`.
3. Keep the terminal open. Run `goalcup status` from a second terminal to check.

## Questions for the owner

1. **Is a restart after a platform cap allowed?** RULES allows `goalcup continue` after "a crash,
   a reboot, a closed window" and "power or internet … restored". A planned daily stop by the
   participant's chosen platform is not named. Proposal: allowed, same deadline, with the
   downtime visible in the record and reviewed like any other.
2. **What may the participant do around a restart?** Today: answer the harness's start-up screens,
   then type only "keep going". Is a restart (and a `goalcup seal` just before a known cap) human
   intervention that the review counts against the run? Proposal: not intervention; still only
   "keep going" may be typed; no other message, no task information.
3. **A new machine mid-run?** RULES and the guide say "on the same machine", and the code refuses
   when the run's files are not there. Should a VM replaced by the platform, with the same
   persistent disk at the same paths, count as the same machine? Proposal: yes when the disk
   is the same; a run whose disk was lost cannot go on, and the participant writes to support.

## Tests

- `crates/goalcup-cli/tests/it/restarts.rs` (fast tier, about 4 s):
  `a_run_crosses_two_platform_stops_and_each_continue_keeps_it_whole` runs start, SIGKILL of host
  and agent (daemon lives), continue, SIGKILL of host, agent and the real daemon process (frozen
  first, as at a power cut), continue, `goalcup seal --yes` from another terminal. The server
  refuses chunks from before the first stop to after the second. It checks: three parts in order
  on the server and here, one approval, one clone, the same deadline in the record and every
  receipt, each earlier chain kept as the prefix of the final one, each part whole on the server,
  sealed and signed, gaps `[IngestLost]`, `[IngestLost]`, none (exit 1, 1, 0), what was typed in
  each part recorded, `resume --last` given at each continue, a new daemon after the power-off,
  and a fresh time-left file with the same deadline.
- `a_continue_whose_clone_is_gone_is_refused_and_changes_nothing`: the fix above.
- Existing single-stop tests, `full` tier, `crates/goalcup-cli/tests/full/upload_e2e.rs`
  (`segments::`): killed agent then continue, reboot with a torn tail, continue during the first
  host's wrap-up, server away, run over, a run not started here, rotated parts through a crash.
