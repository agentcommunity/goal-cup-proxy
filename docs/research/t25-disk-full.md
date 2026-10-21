# T25: a literal ENOSPC during a run (measured 21 Oct 2026)

What a run does today when the volume that holds the recording really fills, measured on a
loop-mounted ext4 image with the real binaries. Nothing in the product was changed. This is the
input for WP-C (`docs/superpowers/plans/2026-10-20-event-readiness.md`, invariant I3) and closes
the "literal ENOSPC on a disposable volume" item of T25.

**Short answer:** today a full disk silently ends the recording. The first `ENOSPC` on a chain
append latches the daemon's chain writer for good: no `StorageFull` gap is written, the chain
keeps a torn last line, the agent keeps running and its traffic keeps flowing through the proxy
unrecorded, nothing resumes when the space comes back, the entry can be neither sealed nor
continued, and the verifier answers exit 2 (`torn_tail`). The 3 GiB reserve does not stop any of
this. It hands its blocks back to the volume, the agent's own writes take them, and the chain gets
none.

| Row | Fill | Result (measured) |
|---|---|---|
| `uploads-working` | 128 MiB, then 0, then freed; server up | Reserve 3 GiB to 0 in 24 s; no stop; writer latched 2.1 s after reaching 0; torn tail of 2,515 bytes; 0 gaps; about 1,000 echo exchanges unrecorded; no resume after freeing; seal impossible; verify exit 2 |
| `chat-continue` | 0, then freed; server up | Latched after 9 lines; agent kept answering; `continue` exit 1 (`workspace … already bound`) after the server opened an empty segment; verify exit 2 |
| `local-stop-zero` | 0; no server | Latched within 2 s while the reserve still held 2.5 GiB; stop 12 s later; seal not written; verify exit 2 |
| `local-stop` | 128 MiB; no server | Stop after the reserve drained (about 25 s); sealed; verify exit 0 |
| `server-down` | 128 MiB; server stopped; trickle | Stop 30 min after the last acknowledgement; sealed; verify exit 0 |

## How it was measured

- `scripts/t25-disk-full.sh [row ...]` (Linux, root). For each row it makes a fresh 5 GiB ext4
  image (`mkfs.ext4 -m 0`, so root's reserved blocks cannot hide the fill), loop-mounts it, and
  runs one ignored test in `crates/goalcup-cli/tests/full/t25_disk_full.rs` with `TMPDIR` on the
  image. The configuration directory, the daemon's artifacts, the run directory, the run's clone,
  `state.json`, the upload staging file and the reserve are all on the image. Logs, the workload's
  report and the upload stand-in's store are written to `$T25_WORK/out/<row>/` off the image. The
  script unmounts and deletes the image after each row and on any exit. It also adds the
  controlled endpoint's names (`api.e00.localhost`, `auth.e00.localhost`) to `/etc/hosts` while it
  runs, because this container's resolver does not answer for `.localhost`, and removes them on
  exit.
- The harness is the controlled workload (`m2-controlled-workload`, `stream` mode: 16 KiB echoes
  every 100 ms plus WebSocket messages through the recording proxy, about 0.4 MB/s of chain). The
  chat row uses the fake TUI (`m2-fake-tui`). The upload server is the Rust stand-in
  (`tests/common/upload_server`).
- A second thread in the test process fills the volume with `fallocate` processes every 200 ms,
  down to a target free size, and keeps it there, the way an agent's build would.
- The test-build doors used are `GOALCUP_TEST_DISK=3221225472,1073741824,1000,268435456`
  (the shipped 3 GiB reserve, 1 GiB start margin and 256 MiB floor, with the watch tick at 1 s
  instead of 30 s) and `GOALCUP_TEST_UPLOAD_POLICY=5000,1000,1000,5000,60000,4194304,30000,10000`
  (a chunk every 5 s, heartbeats every 10 s). `UPLOAD_FAILING_AFTER` (30 min) has no test door,
  so the `server-down` row waits it out.
- Each row writes `timeline.jsonl`: one snapshot every 2 to 5 s (free bytes, reserve blocks, chain
  length and whether it ends on a whole line, the daemon's signed capture status, the run's
  `state.json`, the workload's echo count, the stand-in's position), and every command run with
  its exit code, time and full output. The times below are seconds since the row started.

"Measured" means seen in a row's timeline. "Inferred" means read from the code or worked out
from a measurement, and not run.

## S1: under the floor, uploads working (row `uploads-working`)

The volume was filled to 128 MiB free at t=20.3 and held there. Measured:

- Each watch tick gave back reserve blocks to bring free space up to the 256 MiB floor, and the
  filler took them again. The reserve went from 3,065 MiB to 0 in 24 s (t=44.7), about 128 MiB a
  tick. At the shipped 30 s tick that is about 12 minutes (inferred). The volume then had about
  120 MiB free, and the chain was eating it at about 0.4 MB/s.
- `goalcup status` (exit 0) said `PROBLEM: only 116.1 MiB of disk is free. Free some space.` The
  agent kept running and the run did not stop, because uploads were acknowledged.
- Nothing was pruned. There was one 30 MB part, below the 256 MiB part size, and the footprint was
  far below 3 GiB. No `disk:` line was printed. Pruning cannot help in the first hours of a run:
  it deletes only parts acknowledged two hours ago, or any acknowledged part once the parts pass
  3 GiB (`watch.rs` `Pruner`, I4).
- Recording continued normally until the volume reached zero.

## S3: a literal ENOSPC while the daemon writes (rows `uploads-working`, `chat-continue`, `local-stop-zero`)

The filler was set to 0 free at t=74.9. Measured in `uploads-working`:

- **The writer latched within 2.1 s.** By t=77.0 the signed status said `capture: failed`,
  `accounting_available: false`. The chain stopped at 31,272,960 bytes. The verifier accepts
  31,270,445 bytes (4,943 records). The last 2,515 bytes are part of a line, and the file ends on
  a 4 KiB block boundary: ext4 accepted the write up to the end of the last allocated block, then
  refused the rest. The `fail()` path in `writer.rs` (`Err(error) => self.fail(&error)`) is the
  latch. `CaptureStorageFull` and its `StorageFull` gap belong to the logical quota only, and a
  literal `ENOSPC` never reaches them. After that, `append` discards everything, gaps included,
  and `sync` does nothing.
- **No gap.** `gap_count` stayed 0 throughout, and the trace has no `StorageFull` record.
- **The agent kept running and its traffic kept flowing.** The workload finished 498 echo
  exchanges by the failure and 1,496 by its end at t=228. So about 1,000 exchanges (150 s of
  traffic) went through the proxy unrecorded and unmarked. Its own native-file writer died with
  `OSError: [Errno 28]`, as an agent's own writes do on a full disk.
- **The participant was not told while the disk stayed full.** `state.json` is rewritten through a
  new file, so it froze at its last good write. For 60 s at zero free, `goalcup status` kept
  saying `recording` and `only 116.1 MiB of disk is free`. The only sign was in its details
  (`capture: failed … Local capture failed: this entry's chain accepts no more records. The
  harness is not stopped by it.`). `PROBLEM: the recording failed. Write to
  goalcup@agentcommunity.org and name run …` appeared only after space came back.
- **Uploads stopped too.** The uploader stages each chunk in `<cfg>/uploads/<entry>.chunk.zst` on
  the same volume. Every attempt failed with `the next chunk could not be built (…: No space left
  on device (os error 28)); trying again in 5s`. Inferred: after 30 minutes of this, `last_ok` is
  old enough that the watch counts uploads as failing, with the network fine, and the disk stop
  verdict fires.
- **No stop.** The disk verdict needs uploads failing for 30 minutes, and none was reached in the
  60 s the disk was held full.

In `local-stop-zero` (no server), filling straight to zero latched the writer within 2 s. The
reserve still held 2.5 GiB, and the filler took each slice the tick gave back. The stop verdict
came 12 s later, once the reserve was empty (about 6 minutes at a 30 s tick, inferred). `goalcup
start` said `stopped: the disk is nearly full and uploads are failing. The agent is stopped and
the run sealed now.`, and then `entry …: finished locally, but seal.json could not be written (no
terminal record was written), so the trace reads incomplete.` The stop did not save the chain: it
came after the latch, and the seal could not be written either.

The `chat-continue` row reproduced the latch on a chat-mode run, with the fake TUI's session file
preallocated (`fallocate --keep-size`) so the agent's own appends kept working at zero free. The
writer failed after 9 typed 3 KB lines, and the agent kept answering.

## S4: space freed (rows `uploads-working`, `chat-continue`)

Measured:

- **Recording does not resume by itself.** All fill files were deleted at t=137.6. Within 5 s the
  reserve grew back to 3,042 MiB, `state.json` was written again, and the uploader caught up to
  the last whole record (stand-in position 31,270,445). The chain never grew again. The capture
  stayed `failed` through the 90 s of `S4` while the agent kept working.
- **`goalcup start` ends with exit 0** when the agent ends, saying `entry …: finished locally, but
  seal.json could not be written (no terminal record was written), so the trace reads
  incomplete.` and `upload incomplete (the seal was not sent)`. The local finish cannot append to
  a latched writer. `TraceAppendFile` is poisoned as well (`tracecap.rs` `append_checked`).
- **`goalcup status` contradicts itself**: the headline says `sealed, not all of it is uploaded
  yet`, with `PROBLEM: the recording failed`. `goalcup entries` says `"health": "sealed"` with a
  `sealed_at`. There is no `seal.json`, and the verifier says `sealed: false`. In
  `local-stop-zero` the headline stayed `recording / running` after the agent had been stopped,
  because the last `state.json` write was the one made while the disk was full.
- **`goalcup seal`** exits 0 and prints the same `seal.json could not be written` sentence. Nothing
  changes.
- **`goalcup upload`** exits 0: `uploaded: 31270445 bytes of entry …, acknowledged. The run is not
  sealed yet; run this again once it is.` It never can be.
- **`goalcup-verify --capture-integrity` exits 2** (with the stand-in's key given as `--dev-key`):
  `integrity: invalid`, `sealed: false`, `torn_tail: true`, findings `torn_tail`,
  `producer_unclosed`, `first_failure` at the torn sequence number. The same holds after the seal
  and the upload. The chain is torn.
- **`goalcup continue` cannot recover the entry.** For the headless workload it is refused for an
  unrelated reason (`… runs headless: it was given one goal and has no session to pick up, so its
  run cannot be continued`). For the chat-mode run (`chat-continue`), after the agent was killed,
  `start` printed `To go on, run: goalcup continue <run>`. `goalcup continue` then exited 1:
  `goalcup: conflict: conflict: workspace … is already bound to entry …, and one workspace holds
  one entry`, and `Nothing was started, and what this start made was removed.` By then the
  stand-in had already opened a second entry for the run, which never got a byte or a local
  trace. The daemon frees a workspace only for a capture entry that is sealed or that it failed to
  reopen at start (`runtime/capture/mod.rs` `put_letting_go`). A latched entry is neither while its
  daemon lives. This row uses the test rig's long-lived in-process daemon, not a run-scoped one.
- Inferred: a run-scoped daemon exits only once no capture is unfinished (`daemon/spawn.rs`), so
  a latched entry keeps its daemon alive until a reboot or kill. After that, the reopen-failed path
  (F-04, OPERATING case 5) is what would let `continue` go on. That path was not run here.

## S2: the stop verdict (rows `local-stop`, `server-down`)

`local-stop` (no server, so uploads count as failing at once), filled to 128 MiB free at t=20.2.
Measured: the reserve drained from 3,064 MiB to 0 in about 25 s, and the stop came on the tick that
emptied it. `goalcup start` exited 0 at t=48.8 with `stopped: the disk is nearly full and uploads
are failing. The agent is stopped and the run sealed now.`, `finished locally; its terminal record
is 3111 and seal.json binds it.` and `harness termination: Confirmed`. With about 127 MiB still
free, the seal fit. The verifier exited 0 (`valid`, `sealed: true`, no findings). `goalcup
status` said `m2-controlled-workload: stopped (the disk is almost full)`. `goalcup continue`
exited 1 for a reason that has nothing to do with the disk: `Run … was recorded on this computer
only, never on the Goal Cup server, so it cannot be continued.`

`server-down`: a trickling harness (one 256-byte SSE event a second), with the stand-in stopped
at t=25.2 and the volume held at 128 MiB free. Measured:

- The reserve was at 0 by t=65. `last_ok_at` froze at the last acknowledgement (t≈20). The stop
  came at t≈1,822, 30 minutes after that acknowledgement, exactly `UPLOAD_FAILING_AFTER`. The
  chain grew only 1.9 MB in that time, so no `ENOSPC` came first.
- `goalcup start` exited 0 at t=1,887 (after its wrap-up) with `stopped: … The agent is stopped
  and the run sealed now.`, `finished locally; its terminal record is 1833 and seal.json binds
  it.` and `upload incomplete (the time left to finish ran out …)`. Harness termination was
  `Unresolved (ProcessAuthorityUnavailable)`, and the workload exited with code 1. The cause was
  not looked into.
- The verifier exited 0 (`valid`, `sealed: true`). After the space was freed and the server
  restarted, `goalcup seal` repeated the sealed result, and `continue` gave the headless refusal.
- While the server was down, `goalcup status` printed `PROBLEM: nothing was uploaded for 30 min
  (…)` and `PROBLEM: the server has not heard from this run for about 900 min (…)`, after a
  30-minute outage. The second number is wrong. It was not followed up here.
- A silent harness never reaches this verdict. An uploader with nothing waiting marks itself
  successful (`uploader.rs`: "Nothing waiting is an upload with nothing to complain of"). A
  first try of this row with a harness that sent nothing never stopped.

So the stop verdict works as designed when the disk is below the floor but not yet at zero. With
a busy agent the order is the other way round. At 0.4 MB/s, the last 128 MiB lasts about 5
minutes, far less than the 30-minute window, and the chain writer latches first (S3).

## What this means for WP-C

1. **The 1 GiB hold belongs inside the files the recorder writes, not in a separate file whose
   blocks go back to the volume.** Today's reserve is given back, tick by tick, to whoever writes
   next. On a disk an agent is filling, that is the agent, and the chain hits `ENOSPC` while the
   reserve still holds 2.5 GiB (`local-stop-zero`). The chat row showed what does work: a file
   with preallocated blocks (`fallocate(FALLOC_FL_KEEP_SIZE)`) takes appends at zero free space.
   So the daemon should keep the open chain part preallocated ahead of its length (for example
   the part's remaining room up to 1 GiB, renewed at rotation and after each sync), and give it
   back only by truncating that part. The other files a run writes on that volume also need room:
   the upload staging chunk, `state.json` and `acks.jsonl`. Either keep them small and inside the
   same hold, or move staging into memory or onto the preallocated budget. Otherwise the uploader
   and `status` fail with the chain, as measured. The seal (`LocalFinish` plus `seal.json`) must
   come out of the hold too.
2. **Unlatch by treating `ENOSPC` (`io::ErrorKind::StorageFull`) as storage-full, not as a
   failure.** At the export/tracecap boundary, a short write that hit `ENOSPC` must leave the file
   at the last whole record: truncate back to the known `chain_bytes`, which the hold makes
   reliable, and do not poison the `TraceAppendFile`. Keep `accounting_available` true, and map
   the error to a new non-terminal storage-full state in `ChainWriter`. `fail()` stays for real
   I/O faults only. While in that state the writer drops payload as it does for the quota, but
   keeps writing the bounded closures from the hold, and it probes again every tick and after each
   freeing of space.
3. **Write the `StorageFull` gap from the hold, once per episode, with a start and an end.** The
   opening gap is written at the first refusal (stream-less, plus one per open stream as today).
   The closing record says when recording resumed and how many bytes or records were dropped
   (`lost_bytes`). Records that cannot be attributed to an open stream (new connections opened
   during the episode) need a Recovery or reopen path, so that resumed traffic is admissible. The
   pass line in the plan ("exactly one gap per episode with start and end, recording resumes
   within 60 s of space returning") is measurable with this script. Today's value is 0 gaps and
   no resume.
4. **Tell the participant even when the disk is at zero.** `state.json` needs its own small
   preallocated room, or `status` must read the daemon's signed capture state first. Today it
   showed `recording` for a failed capture until space came back, and forever after a stop.
5. **`continue` and `seal` must handle a storage-full entry.** With the writer unlatched, `seal`
   writes the `LocalFinish` from the hold, and the torn-tail case disappears. As a safety net for
   a latch that still happens, `seal` and `continue` should close a failed entry at its last whole
   record (the F-04 behaviour, today only reachable after a daemon restart) and release its
   workspace binding, before `continue` asks the server for a new segment. Today `continue` asks
   the server first and leaves an empty segment behind when the daemon then refuses.
6. **The stop verdict should not fire for a full disk with a healthy network.** It needs uploads
   failing, but a full disk makes uploads fail by itself (the staging file). With the owner's
   decision ("never stop the agent on a full disk"), the `Stop` arm of `verdict_with` and
   `DISK_STOP_REASON` go. The watch keeps resizing and warning, and the StorageFull gap is the
   record.
7. **The tick is too slow to protect anything.** At 30 s, draining 3 GiB against a fast writer
   takes 6 to 12 minutes. A hold that never leaves the chain's own file makes the tick's speed
   irrelevant for correctness.

## Reproduce

```sh
sudo sh scripts/t25-disk-full.sh                     # uploads-working chat-continue local-stop local-stop-zero
sudo sh scripts/t25-disk-full.sh server-down         # about 35 minutes
python3 -c 'import json,sys; [print(json.loads(l)["t"], json.loads(l)["what"]) for l in open(sys.argv[1])]' \
  /var/tmp/t25/out/uploads-working/timeline.jsonl
```

Each row takes 1 to 5 minutes except `server-down`. Peak extra disk use is one sparse 5 GiB image
(the reserve and the fill are `fallocate`d, so the image file itself stays small), plus the logs.
