# PRD-04: Native capture (session files and final database snapshots)

Id: PRD-04. Version: goalcup 1.0. Status: plan for the overnight build. Date: 30 October 2026.
Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md).
Core: [PRD-02](PRD-02-recorder-core.md).

## Goal

Read every session file every harness and every subagent writes, for the whole run, with no cap of
our own, at a cost the participant never notices. After the harness exits, copy its session
databases whole. The file is the buffer: under load we read later, we never drop.

## Points served

- **2 (lead).** Most harnesses write one file per subagent or per thread. `**` patterns find a new
  subagent folder at any depth, and no slot or file cap leaves one out.
- **1.** The session files are the harness's own log of its calls. The pad check "wire calls match
  the harness's own log" (PRD-12) reads them from the record.
- **3.** One low-priority thread, a stated CPU duty cycle, no handle held between passes, and reads
  only as much as the pipeline has room for. Nothing here can slow a relay.
- **4.** Every byte goes through PRD-02's masker before disk. No symlink is followed, so a link in a
  session folder cannot pull an unrelated file into the record.
- **5, 6.** Native streams are ordinary records: uploaded with the rest and counted at write
  (`native_files`, `native_bytes`, `snapshots`, PRD-01 section 9).
- **7.** Every harness is read the same way. Per harness, only data changes: roots and patterns
  (PRD-11).

## Owner decisions it carries out

- No caps of our own on subagents, files or bytes. Raise or remove the 32 native slots and the
  1,024-file cap (BRIEF, owner decisions after phase 2; BRIEF2 ask 5).
- The file path is the ID. The server orders by ID and time.
- Byte-for-byte match of subagent files against disk is no longer required.
- One process (option B): the native reader is a thread in the recorder, not a producer of a daemon.
- Session files are read the same way for every harness; per harness only data (BRIEF3, 29 Oct night).
- Record as much as possible of what the agents did (research value).
- Record raw, interpret later (AGENTS.md hard rules, kept by PRD-00).
- Disk full gets no special handling (PRD-02 section 5 owns that path).

## Today

| What | Where | Fact |
|---|---|---|
| The adapter | `crates/goalcup-daemon/src/native_capture.rs` (1,632 src lines, no inline tests) | A capture producer with its own IDs, epochs and admission (`:885-1027`) |
| Slot cap | `native_capture.rs:276-283` (default), checked `:1555-1558` | 32 open native streams (`MAX_OPEN_STREAMS / 4`). Run 1: 32 slots for 43 rollouts, 11 transcripts unread (`docs/plans/2026-10-30-audit/M-two-hour-run.md:59, 115`) |
| File cap | `native_capture.rs:280`, ceiling `:187` (20,000), checked `:1323-1327` | 1,024 tracked files, then a stream-less gap |
| Directory window | `native_capture.rs:281`, ceiling `:189`; resume logic `:792-818, 1262-1301` | 8,192 entries per poll, rotating through bigger stores |
| No `**` | `native_capture.rs:1140-1142` | "recursive `**` is not supported; name each level". Patterns at most 8 levels (`:185`) |
| Rows per harness | `crates/goalcup-protocol/src/harness/check.rs:26`, enforced `native_capture.rs:571-575` | At most 8 sources. Claude Code already uses 6 (`connectors/claude-code-cli/harness.toml:92-148`), one row per nesting depth |
| Latch on refusal | `native_capture.rs:919-975` | Any `Disabled` answer latches native off for the life of the adapter (P14 in `M-two-hour-run.md:114`) |
| Cost per poll | `process` `native_capture.rs:1415-1424`, `open_chain` `:1376-1389`; sourcecap `crates/goalcup-platform/src/sourcecap/unix.rs:178-253, 476-560` | Every tracked file re-opens its whole parent chain each poll, with a mount `statx` per level: about 16 calls per Codex file per poll, about 1.7 million calls a second at 10,000 files (`M-two-hour-run.md:576-588`, estimate) |
| Poll cadence | `crates/goalcup-daemon/src/runtime/capture/owner.rs:16` | 100 ms, on the same thread that drains the queue and writes the chain (`owner.rs:162-178`) |
| Bytes per poll | `crates/goalcup-daemon/src/runtime/capture/native_plan.rs:11` | 256 KiB raw, sized to the 8 MiB shared queue |
| Finish | `owner.rs:12-13, 128-152`; budget `crates/goalcup-protocol/src/control.rs:708` | 64 MiB of reading at the seal, 180 s. Unfinished files get `ShutdownDeadline` gaps |
| Final snapshot | `native_capture.rs:47-52`, `owner.rs:146-150` | Read only when the harness exit is confirmed; otherwise skipped with a stream-less gap |
| Loss reporting | `owner.rs:18-20, 139-150, 196-214` | Stream-less `LimitExceeded` gaps with no file name and no count |
| Run-start filter | `native_capture.rs:1303-1308`; `docs/DECISIONS.md:464-488` (D57) | A file is this run's once `max(mtime, ctime)` in whole seconds is at or after the run start |
| No-follow reads | `sourcecap/unix.rs:87-118` (root), `:128-155` (observe), `:178-253` (open dir, open file with `O_NOFOLLOW`, `O_NONBLOCK`) | Proven, kept in spirit |
| Handle budget | `crates/goalcup-platform/src/handle_budget.rs:13` | 32 handles; not needed once no handle is retained |
| Plan glue | `native_plan.rs` (311) | Root templates `:137-151`, home isolation `:224-311`, baseline listing `:157-201` |
| Tests | `crates/goalcup-daemon/tests/it/capture_native.rs` (2,664, 45 tests); `crates/goalcup-platform/tests/it/unix_sourcecap.rs` (261); native plan tests in `runtime/capture/tests.rs:1745-2060` | Cases worth keeping are listed under Acceptance |

## Design

### 1. Shape

One native thread inside the recorder. PRD-02 starts it at boot step 4 (PRD-02 section 9.2) and
stops it at the seal.

```rust
pub fn start(rec: Recorder, sources: NativeSources) -> NativeReader;   // spawns the thread
impl NativeReader {
    pub fn finish(self, harness_exited: bool) -> NativeFinish;         // called by PRD-02's seal
}
pub struct NativeFinish { pub streams: u64, pub snapshots: u64, pub cut_files: u64, pub took_ms: u64 }
```

The loop, once per pass:

1. **Walk** (section 3): visit the directories that are due, list the ones that changed, `fstatat`
   the files that are due.
2. **Settle** each due file against what it saw last time (section 4): grew, unchanged, truncated,
   replaced, rewritten, vanished.
3. **Read** what grew, within the room the pipeline has (section 5).
4. **Sleep** until the next pass (section 3, duty cycle). If reading stopped because room ran out
   or the slice was used, the next pass skips the walk and reads again at once.

The thread runs at low priority (PRD-02 section 1). It calls only S1 (`open`, `data`, `gap`, `end`,
`event`, `room`, `now`, `native_cursors`). It never parses file content.

### 2. Sources and patterns

A source is a row of connector data (PRD-11, seam S3):

| Field | Meaning |
|---|---|
| `name` | 1 to 64 ASCII letters, digits, `-`, `_`, `.`. The first part of the path ID |
| `root` | An absolute folder, resolved by PRD-11 from a template (`{config_home}`, the harness's isolated home, or `{workspace}`) |
| `pattern` | `/`-separated, relative to `root`. Components: a literal name, a glob with `*` and `?`, or `**` |
| `form` | `append` (read as it grows), `snapshot` (read whole each time it changes and settles), `final` (read whole once, at the seal, after the harness exits) |

**`**` rules.** `**` is a whole component and matches zero or more folders. `a/**/b.jsonl` matches
`a/b.jsonl`, `a/x/b.jsonl` and `a/x/y/z/b.jsonl`. Anything else in a component (`*`, `?`, literal)
matches inside one name, never across `/`, with the single-component matcher copied from
`crates/goalcup-protocol/src/harness/check.rs:326-351`. Refused at load: empty components, `.`, `..`,
an absolute pattern, braces, a NUL byte, `**` glued to other text (`a**`). There is no depth limit:
since no symlink is followed and no mount is crossed (section 9), a walk cannot loop.

**Rows.** No row cap (the 8-row limit goes). Rows of one harness may overlap; a file is read once,
by the **first** row (in file order) whose pattern matches it. That keeps the path ID unique.

**Every row reads from the run start.** A file is taken once `max(mtime, ctime)`, in whole seconds,
is at or after the run's start (copied rule, D57; `native_capture.rs:1303-1308`). The flag
`since_run_start` goes: every row has it. The run start is the run's, not the entry's, so a
`goalcup continue` keeps the same floor. A file once taken stays taken, and is read whole from
offset 0 the first time (a `--continue` that appends to an old transcript gets the whole file).

**What `**` buys.** Claude Code's six rows (`connectors/claude-code-cli/harness.toml:92-148`) become
three, and a subagent started one level deeper in a future release is still found:

```toml
[[capture.native]]   # read first: spilled tool output
name = "tool-results"
root = "{config_home}/projects"
pattern = "**/tool-results/*"
form = "snapshot"

[[capture.native]]
name = "meta"
root = "{config_home}/projects"
pattern = "**/*.meta.json"
form = "snapshot"

[[capture.native]]   # main transcripts, subagents, workflow subagents, at any depth
name = "transcripts"
root = "{config_home}/projects"
pattern = "**/*.jsonl"
form = "append"
```

Proposed rows for the 15 harnesses (PRD-11 owns and verifies them; every root below is the harness's
isolated home as PRD-11 sets it; paths from `docs/plans/2026-10-30-audit/O-harness-access.md:84-100,
314-413`):

| Harness | Rows (root: pattern, form) | Source |
|---|---|---|
| Claude Code | `projects`: as above | O line 86, 580-589 |
| Codex | `sessions`: `**/rollout-*.jsonl` append. Its state SQLite (thread edges) as `final`: file name unverified | O line 87, 670-674 |
| Hermes | home: `state.db*` final; `cache/delegation/live/**` append (unverified) | O line 88, 751-760 |
| OpenCode | `.local/share/opencode`: `opencode.db*` final | O line 89, 854-855 |
| OpenClaw | `.openclaw`: `agents/*/agent/openclaw-agent.sqlite*` final; `state/openclaw.sqlite*` final | O line 90, 952-956 |
| Amp | `.cache/amp/logs/threads`: `T-*.log` append (metadata, not a transcript) | `connectors/amp-cli/harness.toml:84-90` |
| GitHub Copilot CLI | `.copilot/session-state`: `**/events.jsonl` append; `.copilot`: `session-store.db*` final | O line 94, 1307-1315 |
| Gemini CLI | `.gemini/tmp`: `**/chats/**/*.jsonl` append | O line 93, 1207-1211 |
| Goose | `.local/share/goose/sessions`: `sessions.db*` final | O line 95, 1413-1415 |
| Kilo CLI | `.local/share/kilo`: `kilo*.db*` final | O line 97, 1577-1580 |
| Qwen Code | `.qwen/projects`: `**/*.meta.json` snapshot, `**/*.jsonl` append (the transient `.jsonl.stream` is not matched) | O line 98, 1673-1679 |
| Crush | `{workspace}/.crush`: `crush.db*` final | O line 99, 1760-1762 |
| Factory Droid | `.factory/sessions`: `**/*.settings.json` snapshot, `**/*.jsonl` append | O line 100, 1848-1851 |
| Kimi Code | its home: `sessions/**/wire.jsonl` append, `session_index.jsonl` append (home path unverified) | O line 314, 354-367 |
| Cline CLI | `.cline/data`: `sessions/**` append (format unverified), `db/sessions.db*` final | O line 413 |

### 3. Discovery: the walk, cheap at 20,000 files

The reader keeps an in-memory tree of what it has seen under each root: one node per folder and per
taken file, about 200 bytes each (4 MB at 20,000 files, estimate). Each node remembers its last
`stat` (device, inode, size, mtime and ctime in ns, and on macOS the birth time, as
`sourcecap/unix.rs:448-461` does) and when it last changed.

**Open, read, close.** A pass walks depth first from each root. It holds only the chain of folders it
is in (at most the tree depth, under 10 for every row above) and one file. Nothing is held between
passes, so the reader's descriptors never grow with the number of files (today's 32-handle budget
is not needed). Each folder on the way is opened once per pass with `openat` from its parent, not
once per file as today (`native_capture.rs:1415-1424`). Each child is checked with one `fstatat`
from that open folder.

**Hot and quiet.** A node is **hot** if it changed in the last 5 minutes, or if a hot node lies below
it. Hot nodes are visited every pass. Every other node is **quiet** and is visited on one pass in
ten, chosen by a hash of its path, so the quiet work is spread evenly. A quiet file that starts to
grow again is seen within 10 passes and becomes hot.

**Listing only what changed.** A folder is listed (`getdents`) only when its `stat` differs from the
last listing, or its mtime is within 2 s of the last listing (a change in the same clock tick), or the
60 s sweep is due. Adding, removing or renaming an entry changes the folder's mtime (POSIX), so a new
subagent folder is seen on the next pass that visits its parent. Each listed name is matched against
the rows: a folder is entered only if some row could still match below it (`**` always can); a file is
taken by the first matching row.

**The 60 s sweep.** Once a minute one pass visits and lists everything, hot or quiet. It bounds the
delay to 60 s on a file system whose folder mtimes are coarse or unreliable (overlay and network file
systems are unverified).

**Duty cycle (the stated guard).** The pass interval is 1 s, or 20 times the last walk's own time if
that is longer. So the walk never uses more than 5% of one core, whatever the number of files, and
reading is never delayed by it: a pass with unread bytes reads again at once. The guard limits
latency only; it never loses a byte, because the bytes stay in the files.

**Cost (estimates, about 1.5 us a call on Linux, `M-two-hour-run.md:576-585`).**

| Tracked files (folders) | Today, calls a second | 1.0 hot pass (100 hot files) | 1.0 average, calls a second | CPU, 1.0 |
|---|---:|---:|---:|---:|
| 43 (about 10) | about 7,000 | about 100 | about 100 | negligible |
| 1,000 (about 300) | about 170,000 | about 600 | about 600 | under 0.1% |
| 20,000 (about 4,000) | over 3 million (cannot finish) | about 1,500 plus 1/10 of 24,000 | about 4,000 | about 0.6% of a core |
| 20,000, 60 s sweep pass | | about 36,000 in one pass | adds about 600 a second | about 0.1% |

macOS calls are slower by an unverified factor; the duty cycle holds the CPU line there too and
stretches the interval instead.

### 4. Reading: appends, rewrites and generations

Each taken file has a **generation** (`gen` 0 first) and, while it is being read, one open `native`
stream (PRD-01 section 4) whose DATA offsets are file offsets. For each due file the reader compares
the new `stat` with the last one:

| What it sees | What it does |
|---|---|
| Same identity, size grew | Read from its offset to the size it saw (section 5) |
| Same identity, same size and mtime | Nothing |
| Same identity, smaller than the offset (truncated) | END `source_changed`; GAP `source_changed` (`lost` 0 = unknown) if bytes were unread; new generation from 0 |
| Same identity, grew, but the **tail witness** differs (rewritten in place) | As truncated: END `source_changed`, new generation from 0 |
| Other identity at the same path (replaced, for example by an atomic rename) | END `source_changed` on the old generation; new generation from 0. Snapshot form: END `complete` instead, since the old copy was whole |
| Gone, or no longer a regular file | END `source_changed`. If the last `stat` showed unread bytes, first a GAP `source_changed` with `lost` 0 and the tail `{"unread_at_least": n}` |
| Unreadable (`open` or `read` fails) | Try again next pass. No GAP unless it ends unread (above). One `note` event `native_unreadable` per path and errno |

**The tail witness.** The reader keeps the last 64 bytes it read from each append file. When the file
grew, it first reads those 64 bytes again (one `pread`) and compares. A difference means the file
was rewritten in place (a harness that rewrites its whole JSONL), or an inode was reused for a new
file at the same path. Today that case "is not detectable here" (`native_capture.rs:59-60`). Cost:
64 bytes per growing file per pass.

**Append form.** Read raw to the size the `stat` saw. A partial last line is recorded as is: no line is
held back (copied rule, `native_capture.rs:5-7`). The stream stays open between passes; EOF of a live
file is not an end (`native_capture.rs:45-47`). The masker's hold (PRD-05) covers a secret cut by a
read boundary, because the stream continues.

**Snapshot form** (a file written whole: `meta.json`, settings, spilled tool output). Read it only
once its `stat` has not changed for 1 s (settled), so a file being rewritten many times a second is
copied once it rests, not every time. Read to the size seen at open, then `fstat` the open file again;
if size, mtime or ctime moved, the copy is torn: END `source_changed`, and the next settled pass reads
a new generation. Otherwise END `complete`. A changed snapshot later is a new generation (copied rule,
`native_capture.rs:47-49`).

### 5. Backpressure: the file is the buffer

- Before each read the reader asks `rec.room()` (PRD-02 S1). It reads at most `room` minus a quarter of
  the soft guard, so native never pushes the pipeline into the band where the wire starts to drop
  (PRD-02 section 3). If that is 0 it reads nothing this pass. The offset stays; the bytes stay in the
  file; the next pass reads the same offset. No GAP, no latch, no "try later" protocol.
- Each pass reads at most a 64 MiB slice in all, starting after the file where the last pass stopped
  (round robin, copied idea from `native_capture.rs:1180-1187`), so one huge file cannot starve the
  others. A slice is fairness, not a cap: unread bytes are read on the next pass, at once.
- Pieces are handed to `StreamHandle::data` as read; PRD-02 splits them into DATA records of at most
  256 KiB. Each piece's time is `rec.now()` just before its `pread`.
- The only native loss left is a file that vanishes or is rewritten while bytes in it are still
  unread (section 4). It is always a GAP with a start and an end, never silent.

This is why P14 (native latched off by a refusal) cannot happen: the reader never offers more than
there is room for, and S1 never refuses.

### 6. Snapshots of harness databases

Hermes (`state.db`), OpenCode, OpenClaw, Goose, Kilo, Crush, Copilot's index and Cline keep their
sessions in SQLite. During the run a SQLite file changes constantly and a raw copy may be torn, so
these rows have the `final` form:

1. **When.** At every seal (each entry's end), in `NativeReader::finish` (section 8). Never during the
   run. A `continue` that follows gets its own copy at its own seal.
2. **What.** Every file the row matches: `x.db`, `x.db-wal`, `x.db-shm` (if present), each its own
   `snapshot` stream with `form: "final"` (PRD-01 section 4). After a clean exit SQLite usually has
   folded and removed the `-wal`; after a crash the `-wal` holds committed rows the main file lacks,
   so it must be copied too.
3. **How.** Raw bytes, read with the same no-follow open. Never opened with SQLite and no SQLite
   dependency (record raw). Read each file to the size seen, then `fstat` again. If any file of the
   set moved during the read (a background process still writes), read the whole set again, up to 3
   times; then keep the last copy and add a `note` event `snapshot_unstable` with the path.
4. **Harness still running** (the deadline seal, or a `seal` command while it runs): take the copy
   anyway, with `"harness_exited": false` in its meta and a `note` `final_snapshot_live`. Today the copy
   is skipped and the run keeps nothing (`owner.rs:146-150`). A possibly torn copy, said plainly, is
   worth more to research than none.
5. **Size.** No cap. A week of OpenCode use may make a large file; it is read with the same room rule as
   section 5 and the seal's time guard (section 8).
6. **Masking.** PRD-05 masks snapshot bytes like any stream, with `*` of the same length (PRD-01 section
   8). Pages keep their layout. A mask inside a `-wal` frame breaks that frame's checksum, and SQLite
   then ignores that frame and later ones. Decision (PRD-01 asked PRD-04): record as is. The usage
   reader (PRD-09) recomputes WAL frame checksums before it opens a snapshot (the checksum is a
   documented running sum over the frame; about 40 lines). The wire record stays the primary record.
7. **Background daemons.** A harness's own background process (Hermes gateway, OpenCode server) can keep
   writing after the harness exits. PRD-11's launch plan names it; PRD-02's seal (or PRD-10) stops it
   before calling `finish`. If it is still running, step 3 marks the copy.

### 7. The path is the ID; cursors and resume

- **ID.** `meta.path` = `<row name>:<path relative to root>` (PRD-01 section 5). Names that are not
  UTF-8 or hold control characters are escaped as today (`native_capture.rs:1156-1178`: `\\`,
  `\u{..}`, `\x..`). The root is never in a path. `meta.gen` names the generation.
- **OPEN meta for `native`:** `path`, `gen`, `size_at_open`, `from` (the first offset, ask 2), `fid`
  (device, inode and on macOS birth time, as one short hex string, ask 2). For `snapshot`: `path`,
  `form`, `size`, `mtime_ns`, `fid`, and for `final` also `harness_exited`.
- **A rename is a new path.** Qwen's `x.jsonl.stream` renamed to `x.jsonl`, for example, is read again
  under the new path; the old path ends `source_changed`. The bytes appear twice, and the shared `fid`
  shows it. Simpler than following inodes, and nothing is lost.
- **Across boots** (PRD-02 section 9.3). At start the reader takes `rec.native_cursors()`: per path,
  `gen`, `fid` and the written offset. A file whose `fid` matches and whose size is at least the
  cursor is opened as a new `native` stream of the same `gen` with `from` = its cursor. The witness
  lived in memory and died with the old boot, so the first growth after a restart is not checked
  (see Risks). Bytes read
  but not durably written before a crash are read again from the file, so native loses nothing across a
  recorder crash. A file whose `fid` changed starts a new generation from 0. A snapshot whose `fid`,
  size and mtime equal the cursor is not read again.
- **Across entries.** `goalcup continue` copies the cursors (PRD-02 section 9.3), so a week of
  transcripts is never read twice.
- **Reboot.** Some file systems give new device numbers after a reboot. Then every file looks new and
  is read again from 0: duplicate bytes, marked by `gen` and `fid`, never a loss.

### 8. Finish at the seal

`NativeReader::finish(harness_exited)` stops the loop and runs on the native thread:

1. One full pass: every folder visited and listed, every file checked, hot or quiet.
2. Read every append file to its current size and every unsettled snapshot as it is. No 64 MiB cap
   (`owner.rs:13` goes). Reads still follow the room rule, while PRD-02's writer drains.
3. Copy the `final` rows (section 6).
4. END every open native stream `complete` when read to the last size seen.
5. **Time guard: 180 s** (today's finish budget, `control.rs:708`, kept because the seal can be the
   deadline). What is not read by then gets a GAP `cut_at_seal` per file with its exact `lost` (we know
   the size and the offset) and END `cut_at_seal`. The count goes into `NativeFinish.cut_files` and
   status (PRD-10). A GAP here names the file, unlike today's stream-less gaps (`owner.rs:139-150`).

### 9. Safety: what is never opened or followed

Copied behaviour from `crates/goalcup-platform/src/sourcecap/unix.rs`, rewritten small in
`src/native/fsx.rs` with `libc`:

- Each root is opened by path with `O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC` at the start of
  each pass (`unix.rs:87-118`), not kept open, so a root that is deleted and made again is followed. A
  root that does not exist yet is skipped without a note; it is found once it appears.
- Every folder below is opened with `openat` and `O_NOFOLLOW | O_DIRECTORY` (`unix.rs:178-212`); every
  check is `fstatat` with `AT_SYMLINK_NOFOLLOW` (`unix.rs:128-155`). A symlink is never followed, at any
  level. A symlink that matches a pattern gets one `note` `native_symlink` per path.
- Files are opened with `O_RDONLY | O_NOFOLLOW | O_NONBLOCK | O_CLOEXEC` (`unix.rs:214-253`), then
  `fstat` on the descriptor must show a regular file with the `fid` just seen. A FIFO or device is
  never read, and `O_NONBLOCK` means opening one can never hang the thread.
- **Same device only.** A folder whose `st_dev` differs from its root's is not entered (a mount
  inside a session folder, perhaps a slow network mount). This replaces the `statx` mount check
  (`unix.rs:476-560`) with a free compare.
- No file is ever written, locked, renamed or deleted by the reader.

### 10. The three pad cases

Native capture is the same in every case. Direct key and gateway: the session files sit beside a full
wire record, and the pad compares the two. Dodger: the session files and final snapshots are what we
have, beside the tunnel records and the honest label (PRD-00, "One system for every harness").

## Files it owns

Create (all under `crates/goalcup/`):

| Path | What | Lines (estimate) |
|---|---|---:|
| `src/native/mod.rs` | `start`, `NativeReader`, `finish`, the loop, duty cycle | 150 |
| `src/native/pattern.rs` | Row patterns with `**`, the single-component matcher (copied), first-match rule | 100 |
| `src/native/walk.rs` | The tree, hot and quiet, listing on change, the 60 s sweep | 200 |
| `src/native/tail.rs` | Settle, generations, tail witness, append and snapshot reads, room rule, cursors | 220 |
| `src/native/snapshot.rs` | Final copies, re-stat and retry, live copy | 80 |
| `src/native/fsx.rs` | No-follow `openat`, `fstatat`, `fid`, macOS and Linux `stat` fields (copied) | 120 |
| `tests/native_*.rs` | Section "Acceptance" | 900 |

About 870 src lines against PRD-00's estimate of 800; the extra is `**` and the snapshot retry.

Must not touch: `src/pipeline/` (PRD-02; the cursor file, the seal and S1 are there), `src/mask/`
(PRD-05), `src/connector/` (PRD-11; rows are parsed there), `src/proxy/` (PRD-03),
`crates/goalcup-format/` (PRD-01), `connectors/**` (PRD-11), the old crates (PRD-13).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | `StreamKind::{Native, Snapshot}`, meta keys, END `source_changed`, `complete`, `cut_at_seal`; GAP `source_changed`, `cut_at_seal`; `note` values (asks below) | Used here |
| PRD-02 | S1: `open(kind, 0, 0, from, flags, meta)`, `data`, `gap`, `end`, `event`, `room()`, `now()`, `native_cursors()`. Needs from PRD-02: (a) the soft guard value, or a `native_room()` that already leaves a quarter of the soft guard free; (b) cursors that carry, per path, `gen`, `fid` and the written offset, and for snapshots `size` and `mtime_ns` (the writer reads these from the OPEN meta); (c) a `low_priority()` helper used at thread start; (d) the seal calls `NativeReader::finish(harness_exited)` (this replaces PRD-02's sketch `native::run` and `native::finish(&recorder, ..)`) | Both |
| PRD-05 | Masks every native and snapshot stream, binary bytes included; `*` of the same length | Used here |
| PRD-08 | Checks: per path, DATA contiguous from `from` across boots; every GAP of a native stream has a reason in section 4 or 8; no two streams of one boot share a path and `gen` | Used there |
| PRD-09 | Reads native streams by `path`; repairs WAL checksums in final snapshots before opening them | Used there |
| PRD-10 | Status shows `native_files`, `cut_files`, and "no session file" for a row with `warn_missing` (kept as connector data); stops a harness's background daemon before the seal when PRD-11 names one | Used there |
| PRD-11 | S3 `NativeSources { rows: Vec<Row{name, root: PathBuf, pattern, form}>, since: SystemTime }`, roots resolved and isolated, `{workspace}` root template for Crush, the rows of section 2, daemons to stop | Used here |
| PRD-12 | Pad check "every subagent recorded": list files under each root matching the rows and changed since the start; each must have a native stream whose last END `seen` equals its size at the seal (or a named GAP) | Used there |
| PRD-14 | A fake `Recorder` in `tests/common/` that collects S1 calls with a settable `room()` | Used here |
| PRD-17 | Lessons on session files: inode reuse, atomic rewrite, Hermes `request_dump_*` never matched (D41), the run-start rule (D57) | Used here |

## What is cut

| Path | Lines | Fate |
|---|---:|---|
| `crates/goalcup-daemon/src/native_capture.rs` | 1,632 src | Deleted (PRD-13). Copied behaviour: pattern parsing without the `**` refusal (`:1122-1154`), path escaping (`:1156-1178`), run-start rule (`:1303-1308`), generation and settle rules (`:1498-1543`), snapshot read and re-check (`:1597-1632`), round robin (`:1180-1187`). Cut: producer IDs, epochs and admission (`:885-1027`, `MAX_FRESH_IDS` `:194`), the latch, limits and their checks (`:183-189, 259-310, 1323-1327, 1555-1558`), loss and finish reports (`:399-515, 1087-1112`), `NativeHint` (`:203-223`), discovery windows (`:792-818, 1262-1301`), the drain callback (`:674-738`) |
| `crates/goalcup-daemon/src/runtime/capture/native_plan.rs` | 311 src | Deleted. Root templates (`:137-151`), home isolation (`:224-311`) and root creation (`:96-134`) move to PRD-11. Baseline listing (`:157-201`) is cut with the baseline (PRD-00 question 4). `NATIVE_POLL_BYTES` (`:11`) is cut |
| `crates/goalcup-daemon/src/runtime/capture/owner.rs` native parts | about 90 of 388 src | `NATIVE_FINISH_BYTES`, `NATIVE_EVERY`, `NATIVE_LOSS_EVERY`, `NATIVE_NOT_ADMITTED` (`:12-25`), finish glue (`:128-152`), poll and stream-less gaps (`:171-214`). The file is deleted by PRD-02 |
| `crates/goalcup-platform/src/sourcecap.rs`, `sourcecap/unix.rs` | 616 + 669 src | Deleted. About 120 lines of no-follow calls rewritten in `fsx.rs`. Cut: `statx` mount identity (`unix.rs:476-560`), symlink reading (`sourcecap.rs:281-330`), journal tokens (`:18-26`), the one-cursor guard (`:188-211`), metadata-changed retry loops |
| `crates/goalcup-platform/src/handle_budget.rs` | 79 src | Deleted |
| `MAX_NATIVE_ROWS` and the `**` refusal in `crates/goalcup-protocol/src/harness/check.rs:26` | a few | Gone with the protocol crate; PRD-11's schema has neither |
| `since_run_start`, `protocol_hint` connector fields (`crates/goalcup-protocol/src/harness/schema.rs:659-668`) | a few | Every row reads from the run start; hints are not recorded (PRD-09 maps by connector and path) |
| Tests | `crates/goalcup-daemon/tests/it/capture_native.rs` 2,664; `crates/goalcup-platform/tests/it/unix_sourcecap.rs` 261; `runtime/capture/tests.rs:1745-2060` about 315; the native part of `tests/full/capture_finish_budget.rs` (1,267, shared with PRD-02) | Deleted with the crates. Cases copied below |

Net: about 3,700 src lines (native capture, plan, sourcecap, handle budget, owner glue) become about
870.

## Acceptance

### Real-run proofs (pad, PRD-12, Linux sandbox)

1. **Claude Code, direct key, one subagent.** Pass: every `.jsonl` under the isolated `projects/`
   changed since the start, at any depth, has a native stream; for each, the last END `seen` (summed
   over boots) equals the file's size at the seal; the subagent's `subagents/agent-*.jsonl` and its
   `.meta.json` are there. Zero native GAPs.
2. **Codex, direct key, prompt asking for subagents.** Pass: one native stream per rollout file, and
   the number of rollout files equals the threads in the harness's own record (`session_meta` with
   `thread_spawn`, as in run 1's `final-reconcile.txt`). No file missing, whatever their number
   (run 1 lost 11 of 43).
3. **Hermes, direct key, one delegated child.** Pass: final snapshot streams for `state.db` (and its
   `-wal` if present) with `harness_exited: true`; after the WAL checksum repair (PRD-09), `sqlite3`
   opens the copy and `sessions` has a row with `parent_session_id` set.
4. **OpenCode and OpenClaw** (not release blocking). Pass: final snapshots present and open with
   `sqlite3`; or the harness is marked "not verified" with the reason.
5. **Kill test** (PRD-01 acceptance 3, Codex). Pass: for every rollout path, DATA offsets across all
   boots are contiguous from 0 with no native GAP and no offset written twice in two boots.
6. **Soak, no harness** (watcher, sandbox): a script makes 20,000 files in 4,000 folders over 10 minutes,
   at depths 2 to 7 under a `**` row, 100 of them growing for 30 minutes, 1,000 replaced by rename, 100
   truncated. Pass: every file's bytes reach the record (lengths equal, generations as expected);
   native thread CPU (from `/proc/<pid>/task/<tid>/stat`) under 2% of one core on average after
   discovery; a new folder with a file in it shows as an OPEN within 3 s when its parent is hot and
   within 12 s when quiet; the recorder's descriptor count does not grow with the file count.

### Automated tests that guard them

| Test file | Cases |
|---|---|
| `native_pattern.rs` | `**` matches zero and many levels; first matching row wins; refused patterns (empty, `.`, `..`, absolute, brace, NUL, `a**`) |
| `native_walk.rs` | A folder made at depth 6 is found; a store that does not exist yet is found once it appears (`capture_native.rs:1074`); 20,000 files are all taken (no cap); no descriptor stays open between passes; a folder on another device is not entered; old files are skipped and the new one found among 5,000 (`:2128`); backdated mtime with a new ctime is taken (`:2261`); a file a second before the start is skipped (`:2320`) |
| `native_tail.rs` | Live EOF is not an end (`:1238`); a partial last line is kept (`:566, 1710`); truncate then append never reuses offsets (`:1321`); rename over is a new generation (`:1278`); an in-place rewrite is caught by the witness; a vanished file with unread bytes gives a GAP; binary bytes survive (`:1207`) |
| `native_room.rs` | With `room()` 0 nothing is read and nothing is lost; after room returns, every byte is read in order; one huge file does not starve a small one |
| `native_snapshot.rs` | A snapshot is read once it settles; a torn read is a new generation (`:1962`); a `final` row is read only at finish (`:2553`); a live copy is marked when the harness has not exited; a file still written during the copy gives `snapshot_unstable` after 3 tries; `-wal` and `-shm` each get a stream |
| `native_safety.rs` | A symlinked file and a symlinked folder are not read (`:631, 2059`); a FIFO is never opened and the test does not hang (`:686`); the path never holds the root (`:717`); Hermes `request_dump_*` is never matched (`:1906`) |
| `native_resume.rs` | Cursors from a previous boot: the new stream starts at `from`, no byte twice; a changed `fid` or a size below the cursor starts a new generation; an unchanged snapshot is not read again |
| `native_finish.rs` | Finish reads everything with no byte cap; past the time guard each file gets `cut_at_seal` with its exact `lost` |

## Risks and blind spots

- **Folder mtimes on overlay, network and FUSE file systems** may not change as POSIX says
  (unverified, and Vercel Sandboxes may use overlay). The 60 s sweep bounds the delay; nothing is lost,
  only late. The soak (acceptance 6) runs in the same sandbox type the pad uses.
- **A file deleted with unread bytes** (a harness that cleans up at exit, or room at 0 for a long time
  first) loses them. Named by a GAP; the amount is a lower bound only.
- **Rewrite detection** is by the 64-byte witness only. A rewrite that keeps the last 64 bytes before
  the offset the same, and grows, is read as an append. Very unlikely for JSONL with timestamps.
- **Witness across a restart.** The witness holds raw file bytes in the native thread's memory only and
  is never written (it could hold part of a secret). After a recorder restart the first growth of each
  file is therefore taken on `fid` and size alone; a rewrite in place during the dead time is missed
  once. Rare: it needs a crash and a rewrite of the same file in the same seconds.
- **Renames read twice.** Bytes of a renamed file appear under both paths. Visible through `fid`.
- **Large rewritten snapshots** (a session JSON of a few MB rewritten each turn) make many generations.
  The 1 s settle helps; the 128 MiB compression window removes most repeats (PRD-01 section 14); still
  more bytes than an append file. Watch in the pad.
- **Live database copies are possibly inconsistent.** Said in the meta and a note; the reader must cope.
- **Databases hold more than transcripts.** OpenClaw keeps `auth_profile_store` in its agent SQLite
  (O line 956). Only PRD-05's rules mask it; an opaque token with no known shape could pass. The pad's
  "no secret" check scans for the exact keys it set (PRD-12). A connector may decline a database that
  is mostly credentials (PRD-11's call).
- **Shared homes.** The run-start rule is safe only while runs on one home are serialized (D57). A
  second run on the same home at the same time would read the other's files.
- **Memory with many open append streams.** Each open `native` stream keeps a masker state (PRD-05) and
  a per-sid entry (PRD-02). 20,000 streams open at once is a few tens of MB if a quiet JSONL holds
  nothing undecided (unverified; the soak measures recorder RSS).
- **macOS:** `stat` field names and the birth time differ (`sourcecap/unix.rs:420-475`); call costs are
  unverified. The owner's Mac run should include the soak script.
- **A harness that writes outside its declared root** (Crush's workspace folder, Droid's cloud sync) is
  read only if PRD-11 names that root. The pad check "every subagent recorded" catches the miss.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Pass interval | 1 s | PRD-04, with soak numbers |
| Duty cycle of the walk | 5% of one core (interval at least 20 times the walk) | PRD-04 |
| Hot window | 5 minutes since the last change | PRD-04 |
| Quiet visits | 1 pass in 10 | PRD-04 |
| Full sweep | every 60 s | PRD-04 |
| Same-tick listing window | 2 s | PRD-04 |
| Snapshot settle | 1 s unchanged | PRD-04 |
| Native share of the pipeline | `room()` minus a quarter of the soft guard | PRD-02 and PRD-04 together |
| Slice per pass | 64 MiB | PRD-04 |
| Tail witness | 64 bytes | PRD-04 |
| Finish time guard | 180 s | Owner (it sits on the deadline path) |
| Snapshot retries | 3 | PRD-04 |
| Live copy of a database when the harness still runs | on | Owner |

## Asks of PRD-00 and PRD-01

PRD-00:

1. **Seam S1 naming for native.** Replace "`native::run(recorder, sources)` and
   `native::finish(&recorder, harness_exited)`" (PRD-02 interfaces) with `native::start(rec, sources)
   -> NativeReader` and `NativeReader::finish(self, harness_exited) -> NativeFinish`. The reader's
   state lives on its thread; a free function cannot reach it.
2. **Seam S3 `NativeSource`.** Fix its shape as in Interfaces (PRD-11 row): `name`, absolute `root`,
   `pattern` with `**`, `form` of `append`, `snapshot` or `final`, and one `since` for the run. No row
   cap. Add the `{workspace}` root template (Crush keeps its database in the project folder). Drop
   `since_run_start` (always on) and `protocol_hint` (not recorded). `warn_missing` stays as PRD-10 data.
3. **Seal order.** PRD-02 section 11 step 3 should say: stop the harness's named background processes
   (PRD-11 data) before `finish`, so final database copies are whole.
4. **Size budget.** Native at about 870 src lines, not 800 (`**` and the snapshot retry).

PRD-01:

1. **Section 5, "the path never holds the home folder or the user name."** Too strong. The path never
   holds the root, but a harness's own folder names can hold the workspace path: Claude Code names
   project folders after the working folder with non-alphanumerics as `-`
   (`O-harness-access.md:582`), for example `-Users-alice-goalcup-run`. Say "never holds the root";
   the same text is inside the transcripts anyway.
2. **Native and snapshot meta keys.** Add `from` to `native` (PRD-02 ask 3 too), `fid` to both, and
   `harness_exited` to `snapshot` with `form: "final"`.
3. **END `source_changed` with no GAP.** Allowed when the reader had read everything it had seen before
   the file changed or vanished (bytes written in the last interval are unknowable). Invariant 4 is
   unaffected.
4. **GAP tail key** `unread_at_least` (bytes) for a native GAP with `lost` 0.
5. **`note` values:** `native_unreadable`, `native_symlink`, `snapshot_unstable`, `final_snapshot_live`.
6. **Section 4 rules, snapshots:** say that `-shm` is copied when a row matches it but carries nothing a
   reader needs, and that WAL checksum repair before reading is the reader's job (PRD-09), as decided
   in section 6 here (closes the PRD-01 risk "PRD-04 decides").
7. **GAP reason 9 `read_error`:** this PRD never writes it (a failed read is retried; a file that ends
   unread is `source_changed`). Mark it reserved, as PRD-02 asks for reason 8.

## Builder notes

Order of work:

1. `fsx.rs` (no-follow calls, `fid`, both platforms) and `pattern.rs` with their tests. No pipeline
   needed.
2. `walk.rs` against a fake `Recorder` (PRD-14): tree, listing on change, hot and quiet, sweep, duty
   cycle. Prove the 20,000-file test early: it is the case the old code could not do.
3. `tail.rs`: append reads, settle table, witness, room rule. This is the part the early real test
   (PRD-00 phase 1a, Claude Code with one subagent) needs, so land it before snapshots.
4. `snapshot.rs` and `finish`. Then `native_resume.rs` once PRD-02's cursors exist.

Traps from the old code and the audit:

- Open files with `O_NONBLOCK`, then check `S_ISREG` on the descriptor. Without it, a FIFO named like a
  transcript hangs the thread for the rest of the run (`capture_native.rs:686`).
- Never re-open the parent chain per file (`native_capture.rs:1415-1424`). That is what made the old
  poll cost grow with files times depth.
- Never read past the size the `stat` saw in one go for a snapshot; for an append file it is fine to stop
  there too, the next pass reads the rest.
- `st_dev` is `i32` and `st_nlink`, `st_mode` are `u16` on macOS; use the cast form with the comments in
  `sourcecap/unix.rs:420-447`. Birth time is macOS only.
- Do not sort folder listings for a "newest first" walk: that existed only for the 8,192-entry window,
  which is gone.
- Do not hold any lock or folder descriptor while asking `room()` or waiting.
- The run start comes from the start facts (PRD-10), in whole seconds, compared with
  `max(mtime, ctime)` (D57); a `continue` passes the run's start, not its own.
- Never parse a session file, not even to find a newline. Raw bytes only.

How a watcher proves it in a real run: after any pad run, `goalcup-verify dump <entry> | jq -c
'select(.k=="open" and (.skind=="native" or .skind=="snapshot")) | .meta.path'` gives the recorded
paths. Compare with `find <root> -type f -newermt <run start>` filtered by the connector's rows: every
file must appear. For each path, the sum of DATA lengths over its last generation equals the file's
size at the seal, or a GAP names why. During the run, read `status.json` once a minute:
`native_files` must never fall, and the native thread's CPU (`top -H -p <pid>`) must stay under 5%.
