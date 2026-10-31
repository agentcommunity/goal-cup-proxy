# PRD-04: Native capture (session files and harness database copies)

Id: PRD-04. Version: goalcup 1.0. Status: plan for the overnight build, revision 3. Date: 30 October
2026. Revision 2 follows PRD-00 and PRD-01 revision 2 and the three reviews (section "Review answers"
at the end). Revision 3 applies the alignment fixes against the ledger (section "Alignment answers"
at the end); where the review history disagrees with the body or those answers, the body wins. Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md). Format:
[PRD-01](PRD-01-record-format.md). Core: [PRD-02](PRD-02-recorder-core.md).

## Goal

Read every session file every harness and every subagent writes, for the whole run, with no cap of
our own, at a cost the participant never notices. Copy each harness database during the run and at
every seal. The file is the buffer: under load we read later, we never drop.

## Points served

- **2 (lead).** Most harnesses write one file per subagent or per thread. `**` patterns find a new
  subagent folder at any depth, and no slot or file cap leaves one out. Harnesses that keep subagent
  links in SQLite get a copy during the run, not only at the seal.
- **1.** The session files are the harness's own log of its calls. PRD-12's optional local log check
  ("wire calls match the harness's own log") reads them from the record; the proxy record stays the
  proof (ledger T4).
- **3.** One low-priority thread, a stated CPU duty cycle (the walk uses at most 2% of one core), no
  handle held between passes, and reads only as much as the pipeline has room for. Nothing here can
  slow a relay.
- **4.** Every byte goes through PRD-05's masker before disk. No symlink is followed, so a link in a
  session folder cannot pull an unrelated file into the record.
- **5, 6.** Native streams are ordinary records: uploaded with the rest and counted at write
  (`native_files`, `native_bytes`, `snapshots`, PRD-01 section 9). A row that finds nothing is named
  to the participant (section 2, "Rows that find nothing").
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
- Less code: PRD-00 "Not in this build" lists the hot and quiet scheduler, the tail witness and the
  20,000-file soak script as "PRD-04 decides; default cut". This revision cuts all three.

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
| Final snapshot | `native_capture.rs:47-52`, `owner.rs:146-150`; Hermes row `connectors/hermes-cli/harness.toml:111-121` | Read only when the harness exit is confirmed; otherwise skipped with a stream-less gap. "A harness killed before its seal keeps no copy of it" (the connector comment) |
| Loss reporting | `owner.rs:18-20, 139-150, 196-214` | Stream-less `LimitExceeded` gaps with no file name and no count |
| Run-start filter | `native_capture.rs:1303-1308`; `docs/DECISIONS.md:464-488` (D57) | A file is this run's once `max(mtime, ctime)` in whole seconds is at or after the run start |
| Rewrite in place | `native_capture.rs:55-61` | "Truncate-then-regrow past the old reader position between two polls ... is not detectable here" |
| No-follow reads | `sourcecap/unix.rs:87-118` (root), `:128-155` (observe), `:178-253` (open dir, open file with `O_NOFOLLOW`, `O_NONBLOCK`) | Proven, kept in spirit |
| Handle budget | `crates/goalcup-platform/src/handle_budget.rs:13` | 32 handles; not needed once no handle is retained |
| Plan glue | `native_plan.rs` (311) | Root templates `:137-151`, home isolation `:224-311`, baseline listing `:157-201` |
| Tests | `crates/goalcup-daemon/tests/it/capture_native.rs` (2,664, 45 tests); `crates/goalcup-platform/tests/it/unix_sourcecap.rs` (261); native plan tests in `runtime/capture/tests.rs:1745-2060` | Cases worth keeping are listed under Acceptance |

## Design

### 1. Shape

One native thread inside the recorder. PRD-02 starts it at boot step 5 (PRD-02 section 9.2) and
the seal calls `finish` at seal step 4 (PRD-00, "Smaller interface rulings").

```rust
pub fn start(rec: Recorder, sources: NativeSources) -> NativeReader;   // spawns the thread
impl NativeReader {
    // called by PRD-02's seal; `guard` is Some(180 s) for reasons `deadline`, `server_stop`
    // and `signal`, None for `harness_exit` and `seal_command` (section 8 step 5, PRD-00 S8)
    pub fn finish(self, harness_exited: bool, guard: Option<Duration>) -> NativeFinish;
}
pub struct NativeFinish {
    pub streams: u64, pub snapshots: u64, pub cut_files: u64, pub cut_bytes: u64, pub took_ms: u64,
}
```

The loop, once per pass:

1. **Walk** (section 3): every root, every folder a row can match below, one `fstatat` per matching
   file.
2. **Settle** each matching file against what it saw last time (section 4): grew, unchanged,
   truncated, replaced, vanished.
3. **Read** what grew, within the room the pipeline has (section 5). Copy any database set that is
   due (section 6).
4. **Sleep** until the next pass (section 3, duty cycle). If reading stopped because the 64 MiB slice
   was used, read again at once without a walk. If it stopped because `room()` was too small, wait
   100 ms, then read again without a walk. A walk still runs once the pass interval has passed, even
   with unread bytes, so a new file is never kept waiting behind a large one.

The thread runs at low priority with PRD-02's per-thread call (PRD-02 section 1). It calls only S1 in
PRD-00's final shape: `open(kind, 0, 0, from, flags, meta)`, `StreamHandle::data(t, bytes)`,
`gap(reason, start, lost, tail)`, `end(t, reason, tail)`, `event`, `room`, `now`, `native_cursors`, `counts`.
The handle owns offsets, so a native stream's DATA offsets are file offsets from `from`. The reader
never parses file content.

### 2. Sources and patterns

A source is a row of connector data (PRD-11, seam S3, PRD-00 final shape `NativeSource { name, root,
pattern, form, since, warn_missing }`):

| Field | Meaning |
|---|---|
| `name` | 1 to 64 ASCII letters, digits, `-`, `_`, `.`. The first part of the path ID |
| `root` | An absolute folder, resolved by PRD-11 from the template `{home}` (the harness's isolated home) or `{workspace}` |
| `pattern` | `/`-separated, relative to `root`. Components: a literal name, a glob with `*` and `?`, or `**` |
| `form` | `append` (read as it grows), `snapshot` (read whole each time it changes and settles), `final` (a database: copied as a set during the run and at every seal, section 6) |
| `since` | The run start, the same for every row (below) |
| `warn_missing` | Default `true` (PRD-11); `false` silences the note. When on, a row that finds no file is named (below) |

**`**` rules.** `**` is a whole component and matches zero or more folders. `a/**/b.jsonl` matches
`a/b.jsonl`, `a/x/b.jsonl` and `a/x/y/z/b.jsonl`. Anything else in a component (`*`, `?`, literal)
matches inside one name, never across `/`, with the single-component matcher copied from
`crates/goalcup-protocol/src/harness/check.rs:326-351`. Refused at load: empty components, `.`, `..`,
an absolute pattern, braces, a NUL byte, `**` glued to other text (`a**`). There is no depth limit:
since no symlink is followed and no mount is crossed (section 9), a walk cannot loop.

**Rows.** No row cap (the 8-row limit goes). Row names are unique within a connector (refused at
load, PRD-11). Rows may overlap; a file is read once, by the **first** row (in file order) that
matches it, compared by the file's absolute path under the resolved roots. That keeps the path ID
unique.

**Every row reads from the run start.** A file is taken once `max(mtime, ctime)`, in whole seconds,
is at or after `since` (copied rule, D57; `native_capture.rs:1303-1308`; lesson PRD-17 F1). The flag
`since_run_start` goes: every row has it. The run start is the run's, not the entry's, so a
`goalcup continue` keeps the same floor. A file once taken stays taken, and is read whole from
offset 0 the first time (a `--continue` that appends to an old transcript gets the whole file).

**Rows that find nothing.** For a row with `warn_missing` on, if no file has been taken 5 minutes
after the first model request was written (`rec.counts()`), the reader writes one `note` event
`native_missing` with the row name, once per entry. A run that never makes a model request gets no
such note. PRD-10 words the line from that note (PRD-02's `status.json` keeps the last 20 notes).

**What `**` buys.** Claude Code's six rows (`connectors/claude-code-cli/harness.toml:92-148`) become
three, and a subagent started one level deeper in a future release is still found. PRD-11 section 6
writes them as (abridged: PRD-11 section 6 is the source, and its `tool-results` and `meta` rows
also carry `warn_missing = false`):

```toml
native = [
  { name = "tool-results", root = "{home}/projects", pattern = "**/tool-results/*", form = "snapshot" },
  { name = "meta", root = "{home}/projects", pattern = "**/*.meta.json", form = "snapshot" },
  { name = "transcripts", root = "{home}/projects", pattern = "**/*.jsonl" },
]
```

**The rows of the 15 harnesses live in PRD-11 section 6 only.** This PRD does not keep a second
table. It gives the rule a row must follow:

| File kind | Form |
|---|---|
| A file that only grows (JSONL transcript, log) | `append` (the default) |
| A file written whole and rewritten (JSON meta, settings, a manifest) | `snapshot` |
| A SQLite database | `final`, with a pattern that ends in `*` (`state.db*`) so the `-wal` and `-shm` files come too |

PRD-11 already gives Hermes's `manifest.json` its own `snapshot` row before the `task-*.log` row.
One point for PRD-11 to check on the pad (unverified): the Codex row `*.sqlite*` matches every SQLite
file in the Codex home, not only the state database; that is allowed (no caps), but a large log
database would be copied too.

### 3. Discovery: a full walk on a duty cycle

The reader keeps a map from path ID to file state: last `stat` (device, inode, size, mtime and ctime
in ns, and on macOS the birth time, as `sourcecap/unix.rs:448-461` does), generation, offset and the
open stream. About 200 bytes a file (4 MB at 20,000 files, estimate).

**Open, read, close.** A pass walks depth first from each root. It holds only the chain of folders it
is in (at most the tree depth, under 10 for every PRD-11 row) and one file. Nothing is held between
passes, so the reader's descriptors never grow with the number of files (today's 32-handle budget is
not needed). Each folder on the way is opened once per pass with `openat` from its parent and listed
(`getdents`), not re-opened once per file as today (`native_capture.rs:1415-1424`). Each listed name
is matched against the rows: a folder is entered only if some row could still match below it (`**`
always can); a matching file gets one `fstatat` from its open folder.

Every pass lists every folder. Folder mtimes are not trusted, so a file system with coarse or odd
folder times (overlay, network, FUSE) is no risk to discovery.

**Duty cycle (the stated guard).** The pass interval is 1 s, or 50 times the last walk's own time if
that is longer. So the walk never uses more than 2% of one core, whatever the number of files. The
guard limits latency only; it never loses a byte, because the bytes stay in the files. Reading is not
held back by it: a pass with unread bytes reads again (section 1, step 4).

**Cost (estimates, about 1.5 us a call on Linux, `M-two-hour-run.md:576-588`).**

| Taken files (folders) | Today, calls a second | 1.0 calls a pass | 1.0 walk time | 1.0 interval | CPU, 1.0 |
|---|---:|---:|---:|---:|---:|
| 43 (about 10) | about 7,000 | about 80 | under 0.2 ms | 1 s | negligible |
| 1,000 (about 300) | about 170,000 | about 2,000 | about 3 ms | 1 s | about 0.3% |
| 20,000 (about 4,000) | over 3 million (cannot finish) | about 36,000 | about 55 ms | about 2.7 s | 2% |

A new file is therefore seen within about 1 s at run 1's size and within about 3 s at 20,000 files.
macOS calls are slower by an unverified factor; the duty cycle holds the CPU line there too and
stretches the interval instead.

### 4. Reading: appends, rewrites and generations

Each taken file has a **generation** (`gen` 0 first) and, while it is being read, one open `native`
stream (PRD-01 section 4). A file's `native` stream is opened when the file is first taken, before
any read, so a later loss always has a stream to carry its GAP. A file still unreadable at `finish`
gets `gap(SourceChanged, start, Some(size - offset), {"unread_at_least": size - offset})` and END
`source_changed` (PRD-01 GAP 5; reason 9 `read_error` stays reserved). For each matching file the
reader compares the new `stat` with the last one:

| What it sees | What it does |
|---|---|
| Same identity, size grew | Read from its offset to the size it saw (section 5) |
| Same identity, same size and mtime | Nothing |
| Same identity, smaller than the offset (truncated) | END `source_changed`; if bytes were unread, first `gap(SourceChanged, start, None, {"unread_at_least": n})`; new generation from 0 |
| Other identity at the same path (replaced, for example by an atomic rename) | END `source_changed` on the old generation; new generation from 0. Snapshot form: END `complete` instead, since the old copy was whole |
| Gone, or no longer a regular file | END `source_changed`. If the last `stat` showed unread bytes, first the GAP as above |
| Unreadable (`open` or `read` fails) | Try again next pass. No GAP unless it ends unread (above). One `note` event `native_unreadable` per path and errno |

A GAP's `start` is the time of the `stat` that last showed the unread bytes; its end is the time of
the call (S1). `None` means the loss is unknown; the tail gives the lower bound. An END
`source_changed` needs no GAP when every byte seen before the change was read (PRD-01 section 3).

**Rewrite in place.** A file rewritten in place that is already longer than the old offset when the
next pass looks keeps its identity and grew, so it is read as an append (stated limit, as today:
`native_capture.rs:55-61`; lesson PRD-17 F2). No harness of the 15 is known to do this to an append
file; a harness that rewrites a file whole belongs in a `snapshot` row. The 64-byte tail witness of
revision 1 is cut (review B N9).

**Append form.** Read raw to the size the `stat` saw. A partial last line is recorded as is: no line is
held back (copied rule, `native_capture.rs:5-7`). The stream stays open between passes; EOF of a live
file is not an end (`native_capture.rs:45-47`). The masker's hold (PRD-05) covers a secret cut by a
read boundary, because the stream continues.

**Snapshot form** (a file written whole: `meta.json`, settings, spilled tool output). Read it only
once its `stat` has not changed for 1 s (settled), so a file being rewritten many times a second is
copied once it rests, not every time. Read to the size seen at open, then `fstat` the open file again;
if size, mtime or ctime moved, the copy is torn: END `source_changed`, and the next settled pass reads
it again as a new `snapshot` stream (snapshots carry no `gen`; `mtime_ns` and `fid` tell copies
apart). Otherwise END `complete`. A changed snapshot later is a new `snapshot` stream too (copied
rule, `native_capture.rs:47-49`).

### 5. Backpressure: the file is the buffer

- Before each read the reader asks `rec.room()` (S1) and reads at most `room()`. `room()` already
  answers against three quarters of the soft guard (PRD-00 S1, PRD-02 section 3), so native never
  pushes the wire into the band where it drops, and the reader subtracts nothing of its own. If
  `room()` is 0 it reads nothing; the offset stays, the bytes stay in the file, and
  it asks again after 100 ms. No GAP, no latch, no "try later" protocol.
- Each pass reads at most a 64 MiB slice in all, starting after the file where the last pass stopped
  (round robin, copied idea from `native_capture.rs:1180-1187`), so one huge file cannot starve the
  others. A slice is fairness, not a cap: unread bytes are read at once, in the next slice.
- Pieces are handed to `StreamHandle::data` as read; PRD-02 splits them into DATA records of at most
  256 KiB. Each piece's time is `rec.now()` just before its `pread`.
- The only native loss left is a file that vanishes or is truncated or replaced while bytes in it are
  still unread (section 4), and bytes still unread when the seal's time guard runs out (section 8,
  only at the end of the run window). Each is a GAP with a start and an end, never silent.

This is why P14 (native latched off by a refusal) cannot happen: the reader never offers more than
there is room for, and S1 never refuses.

### 6. Harness databases

Hermes (`state.db`), OpenCode, OpenClaw, Goose, Kilo, Crush, Copilot's store and Cline keep their
sessions and subagent links in SQLite. A raw copy of a live SQLite file may be torn, so these rows
have the `final` form and are copied as a **set** (`x.db`, `x.db-wal`, `x.db-shm`, whatever the row
matches). A set is one database: the files of one row's matches that share a name once a `-wal`,
`-shm` or `-journal` suffix is removed. Each set has its own `set` id, interval and retry.

1. **When.** At every seal, in `NativeReader::finish` (section 8). And during the run, a **periodic
   copy** when a file of the set changed since the last copy and the interval has passed: 60 minutes,
   or one minute per 2 MiB of the set's size if that is longer (a 400 MiB database every 200
   minutes). The first is due 60 minutes after the entry starts. So periodic copies add at most about
   2 MiB a minute raw (about 3 GB a day before zstd): a cadence chosen for upload and storage cost; it
   never skips the seal copy and never drops a byte. It is not a cap: every database is copied whole
   at every seal. The interval restarts at each boot (a restart may copy one interval late). A
   `continue` that follows gets its own copies.
2. **What.** Each file of the set is its own `snapshot` stream with `form: "final"` and one shared
   `set` (`"<boot>.<n>"`, PRD-01 section 4). After a clean exit SQLite usually has folded and removed
   the `-wal`; after a crash, and in every periodic copy, the `-wal` holds committed rows the main file
   lacks, so it is copied too. A set is taken whole: if any file of a `final` row's set passes the
   run-start rule, every file of that set is copied, whatever its own times.
3. **How.** Raw bytes, read with the same no-follow open, at the room rule of section 5. Never opened
   with SQLite and no SQLite dependency (record raw). Read each file to the size seen, then `fstat`
   again. If any file of the set moved during the read, END every stream of that copy
   `source_changed`, then read the whole set once more as a new `set`. If that copy also moved, END
   it `complete` and add a `note` event `snapshot_unstable` with the path. Both copies stay in the
   record; PRD-09 takes the newest that opens. One retry, not three (review B N8).
4. **Harness still running.** Periodic copies always have `"harness_exited": false`. A seal copy while
   the harness still runs (a seal the recorder runs with no live watchdog, for example the server's
   `stop`, or a harness the watchdog could not end) is taken anyway, with
   `"harness_exited": false` and a `note` `final_snapshot_live`. Today the copy is skipped and the run
   keeps nothing (`owner.rs:146-150`). A possibly torn copy, said plainly, is worth more to research
   than none.
5. **Size.** No cap. A week of OpenCode use may make a large file; the room rule and the seal's time
   guard (section 8) apply as to any read.
6. **Masking.** PRD-05 masks database bytes like any stream, with `*` of the same length (PRD-01
   section 8). The main file has no checksums, and a masked value inside a row keeps the row's
   length, so pages keep their layout and SQLite reads them. A mask inside a `-wal` frame breaks that
   frame's checksum, and SQLite then ignores that frame and later ones; the usage reader (PRD-09)
   recomputes WAL frame checksums before it opens a set (a documented running sum, about 40 lines).
   A mask that falls on page structure (a page header, a cell pointer) cannot be repaired. That needs
   a secret-shaped run of text across binary structure bytes, so it should be rare (unverified);
   PRD-09 then reports that copy `readable: false` (review B A7), and another copy of the entry may
   still open. Decision: record as is. The wire record stays the primary record.
7. **Background daemons.** A harness's own background process (Hermes gateway, OpenCode server) can
   keep writing after the harness exits. The watchdog runs the connector's `after` commands (PRD-11
   data) before it sends the `seal` line, so they run before the seal calls `finish` (PRD-00 S8). If the process still writes, step 3 marks the copy.
8. **Dead entries.** No copy is taken in `close_dead_entry`: that recovery writes a closed frame and
   nothing after it (PRD-02 section 9.4), and nobody knows whether the harness still runs. The
   periodic copies cover a dead entry up to its last interval, and a harness database holds every
   session in that home, so the next entry's copies after `goalcup continue` hold the old sessions
   too.

### 7. The path is the ID; cursors and resume

- **ID.** `meta.path` = `<row name>:<path relative to root>` (PRD-01 section 5). Names that are not
  UTF-8 or hold control characters are escaped as today (`native_capture.rs:1156-1178`: `\\`,
  `\u{..}`, `\x..`). The root is never in a path; the harness's own folder names may hold workspace
  text (PRD-01 section 5). `meta.gen` names the generation.
- **OPEN meta for `native`:** `path`, `gen`, `size_at_open`, `from` (the first offset), `fid`
  (device, inode and on macOS birth time, as one short hex string). For `snapshot`: `path`, `form`,
  `size`, `mtime_ns`, `fid`, and for `final` also `set` and `harness_exited` (PRD-01 section 4).
- **A rename is a new path.** Qwen's `x.jsonl.stream` renamed to `x.jsonl`, for example, is read again
  under the new path; the old path ends `source_changed`. The bytes appear twice, and the shared `fid`
  shows it. Simpler than following inodes, and nothing is lost.
- **Across boots** (PRD-02 section 9.3). At start the reader takes `rec.native_cursors()`: per path,
  `gen`, `fid` and the written offset, and for a snapshot its `mtime_ns`. A file whose `fid` matches
  and whose size is at least the cursor is opened as a new `native` stream of the same `gen` with
  `from` = its cursor. Bytes read but not durably written before a crash are read again from the
  file, so native loses nothing across a recorder crash (PRD-01 invariant 14: an overlap is expected,
  a hole is not). A file whose `fid` changed starts a new generation from 0. A snapshot whose `fid`,
  size and `mtime_ns` equal the cursor is not read again.
- **Across entries.** `goalcup continue` copies the cursors (PRD-02 section 9.3), so a week of
  transcripts is never read twice.
- **Reboot.** Some file systems give new device numbers after a reboot. Then every file looks new and
  is read again from 0: duplicate bytes, marked by `gen` and `fid`, never a loss.

### 8. Finish at the seal

`NativeReader::finish(harness_exited, guard)` stops the loop and runs on the native thread. The seal's
stop (PRD-02 section 11 step 2) applies to wire streams only (`conn`, `http_req`, `http_resp`,
`ws_up`, `ws_down`). `native` and `snapshot` streams are never stopped: the native loop's S1 calls stay
admitted from the stop until `finish` returns. The drain (step 3) waits for wire bytes only. `finish`
is seal step 4 (PRD-00 R3-3).

1. One full pass: every folder listed, every file checked.
2. Read every append file to the size seen in step 1's pass, and every unsettled snapshot as it is.
   No 64 MiB cap (`owner.rs:13` goes). Reads still follow the room rule, while PRD-02's writer drains.
3. Copy every `final` set (section 6), with `harness_exited` as given.
4. END every open native stream `complete` when read to the last size seen.
5. **Time guard: 180 s, only when the seal reason is `deadline`, `server_stop` or `signal`** (today's
   finish budget, `control.rs:708`); it guards the end of the run window and is part of the seal bound
   PRD-02 owns (drain 60 s plus native finish 180 s, PRD-00 S8). PRD-02 passes it as `guard`. On
   `harness_exit` and `seal_command`, `guard` is `None`: finish reads every file to the size seen in
   step 1 with no time guard, and status shows `sealing: reading session files` meanwhile (PRD-00 S8).
   Where the guard applies, what is not read by then gets a GAP `cut_at_seal` per file with its exact
   `lost` (we know the size and the offset) and END `cut_at_seal`. A database set cut part way is cut
   for every file of that set, and the record keeps the last periodic copy whole. The count and bytes
   go into `NativeFinish.cut_files` and `cut_bytes`, and PRD-02 writes them as sealed `status.json`
   `cut_files {files, bytes}` (PRD-10). A GAP here names the file, unlike today's stream-less gaps
   (`owner.rs:139-150`). At PRD-05's 50 MB/s target (unmeasured), 180 s is about 9 GB; at 30 MB/s
   about 5 GB.

### 9. Safety: what is never opened or followed

Copied behaviour from `crates/goalcup-platform/src/sourcecap/unix.rs`, rewritten small in
`src/native/fsx.rs` with `libc` (lesson PRD-17 F3):

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
  (`unix.rs:476-560`) with a free compare. Each such folder gets one `note` `native_other_device`
  (`row`, the path relative to the root) per entry, so the miss is named in status and on the end
  screen (PRD-01 note list, PRD-10).
- No file is ever written, locked, renamed or deleted by the reader. No row may name a `.env` or key
  file: those are PRD-05's exact values, never recorded. PRD-11's connector test refuses a native row
  whose pattern matches any of the connector's own `secret_files`; `src/native/` holds no such check
  and no exclusion list.

### 10. The three pad cases

Native capture is the same in every case. Direct key and gateway: the session files sit beside a full
wire record, and the pad compares the two. Dodger: the session files and database copies are what we
have, beside the tunnel records and the honest label (PRD-00, "One system for every harness").

## Files it owns

Create (all under `crates/goalcup/`):

| Path | What | Lines (estimate) |
|---|---|---:|
| `src/native/mod.rs` | `start`, `NativeReader`, `finish`, the loop, duty cycle, `native_missing` | 150 |
| `src/native/pattern.rs` | Row patterns with `**`, the single-component matcher (copied), first-match rule | 100 |
| `src/native/walk.rs` | The full walk, the file map, matching folders and files | 110 |
| `src/native/tail.rs` | Settle, generations, append and snapshot reads, room rule, cursors | 190 |
| `src/native/snapshot.rs` | Database sets: periodic and seal copies, re-stat and one retry, live copy | 110 |
| `src/native/fsx.rs` | No-follow `openat`, `fstatat`, `fid`, macOS and Linux `stat` fields (copied) | 120 |
| `tests/native_*.rs` | Section "Acceptance" | 800 |

About 780 src lines, under PRD-00's 870: the hot and quiet scheduler and the witness are cut, the
periodic copy is added. Test lines follow PRD-14's per-area table (budget 1,000; this PRD expects
about 800).

Must not touch: `src/pipeline/` and `src/watchdog/` (PRD-02; the cursor file, the seal and S1 are
there), `src/mask/` (PRD-05), `src/connector/` (PRD-11; rows are parsed there), `src/proxy/` (PRD-03),
`crates/goalcup-format/` (PRD-01), `connectors/**` (PRD-11), the old crates (PRD-13).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-00 | S1 final shape; S3 `NativeSource`; S8 (the watchdog runs `after` before `finish`); "Smaller interface rulings" row PRD-02 and PRD-04 | Used here |
| PRD-01 | `StreamKind::{Native, Snapshot}`; meta keys `from`, `fid`, `set`, `harness_exited`; END `source_changed`, `complete`, `cut_at_seal`; GAP 5 `source_changed` with tail `unread_at_least`, GAP 10 `cut_at_seal` (written by PRD-04 for unread native bytes, through PRD-02's handle); notes `native_unreadable`, `native_symlink`, `snapshot_unstable`, `final_snapshot_live`, `native_missing`, `native_other_device` | Used here |
| PRD-02 | S1 calls; `room()` (already 3/4 of the soft guard; nothing subtracted here); `native_cursors()` carrying per path `gen`, `fid`, written offset, and `mtime_ns` for snapshots (the writer reads it from the OPEN meta); `counts()` for the `native_missing` clock; per-thread low priority (PRD-02 section 1); `native::start` at boot step 5; seal step 4 calls `NativeReader::finish(harness_exited, guard)` with `guard` 180 s for `deadline`, `server_stop`, `signal` and `None` otherwise; the stop covers wire streams only, so native S1 calls stay admitted until `finish` returns (PRD-00 R3-3); PRD-02 writes `NativeFinish.cut_files` and `cut_bytes` into sealed `status.json` | Both |
| PRD-05 | Masks every native and snapshot stream, binary bytes included; `*` of the same length | Used here |
| PRD-08 | Checks: per path, DATA contiguous from `from` across boots (invariants 4 and 14); every GAP of a native stream has a reason in section 4 or 8; no two streams of one boot share a path and `gen` | Used there |
| PRD-09 | Reads native streams by `path`; groups database copies by `set`, takes the latest readable set, repairs WAL checksums first, reports `readable: false` for a copy it cannot open | Used there |
| PRD-10 | Status shows `native_files`, `cut_files {files, bytes}`; the `native_missing` note gives the "no session file" line for a `warn_missing` row; one plain line for `native_other_device`; `sealing: reading session files` while an unguarded finish runs | Used there |
| PRD-11 | S3 rows (section 6 is the one source of the rows), roots resolved and isolated, `{home}` and `{workspace}` templates, `warn_missing` per row (default `true`), row names unique per connector (refused at load), a connector test that refuses a native row matching the connector's own `secret_files`, `after` commands that stop daemons | Used here |
| PRD-12 | Pad check "every subagent recorded": list files under each root matching the rows and changed since the start; each must have a native stream whose last END `seen` equals its size at the seal (`seen` is an end offset, PRD-01), or a named GAP | Used there |
| PRD-14 | `Recorder::for_tests(room)` (PRD-00 S1) collects S1 calls with a settable `room()`; `native_walk.rs` in the `heavy` group | Used here |
| PRD-17 | Lessons F1 to F7 on session files: run-start rule, truncate and replace, no-follow and FIFO, partial lines, Hermes atomic rewrite and `request_dump_*`, final copy, one stream per rollout | Used here |

## What is cut

| Path | Lines | Fate |
|---|---:|---|
| `crates/goalcup-daemon/src/native_capture.rs` | 1,632 src | Deleted (PRD-13). Copied behaviour: pattern parsing without the `**` refusal (`:1122-1154`), path escaping (`:1156-1178`), run-start rule (`:1303-1308`), generation and settle rules (`:1498-1543`), snapshot read and re-check (`:1597-1632`), round robin (`:1180-1187`). Cut: producer IDs, epochs and admission (`:885-1027`, `MAX_FRESH_IDS` `:194`), the latch, limits and their checks (`:183-189, 259-310, 1323-1327, 1555-1558`), loss and finish reports (`:399-515, 1087-1112`), `NativeHint` (`:203-223`), discovery windows (`:792-818, 1262-1301`), the drain callback (`:674-738`) |
| `crates/goalcup-daemon/src/runtime/capture/native_plan.rs` | 311 src | Deleted. Root templates (`:137-151`), home isolation (`:224-311`) and root creation (`:96-134`) move to PRD-11. Baseline listing (`:157-201`) is cut with the baseline. `NATIVE_POLL_BYTES` (`:11`) is cut |
| `crates/goalcup-daemon/src/runtime/capture/owner.rs` native parts | about 90 of 388 src | `NATIVE_FINISH_BYTES`, `NATIVE_EVERY`, `NATIVE_LOSS_EVERY`, `NATIVE_NOT_ADMITTED` (`:12-25`), finish glue (`:128-152`), poll and stream-less gaps (`:171-214`). The file is deleted by PRD-02 |
| `crates/goalcup-platform/src/sourcecap.rs`, `sourcecap/unix.rs` | 616 + 669 src | Deleted. About 120 lines of no-follow calls rewritten in `fsx.rs`. Cut: `statx` mount identity (`unix.rs:476-560`), symlink reading (`sourcecap.rs:281-330`), journal tokens (`:18-26`), the one-cursor guard (`:188-211`), metadata-changed retry loops |
| `crates/goalcup-platform/src/handle_budget.rs` | 79 src | Deleted |
| `MAX_NATIVE_ROWS` and the `**` refusal in `crates/goalcup-protocol/src/harness/check.rs:26` | a few | Gone with the protocol crate; PRD-11's schema has neither |
| `since_run_start`, `protocol_hint` connector fields (`crates/goalcup-protocol/src/harness/schema.rs:659-668`) | a few | Every row reads from the run start; hints are not recorded (PRD-09 maps by connector and path) |
| Revision 1 parts never built | none | Hot and quiet scheduler, list-on-change, the 60 s sweep, the 64-byte tail witness, the 20,000-file soak script, three snapshot retries (review B N8, N9, T1) |
| Tests | `crates/goalcup-daemon/tests/it/capture_native.rs` 2,664; `crates/goalcup-platform/tests/it/unix_sourcecap.rs` 261; `runtime/capture/tests.rs:1745-2060` about 315; the native part of `tests/full/capture_finish_budget.rs` (1,267, shared with PRD-02) | Deleted with the crates. Cases copied below |

Net: about 3,400 src lines (native capture, plan, sourcecap, handle budget, owner glue) become about
780.

## Acceptance

### Real-run proofs (pad, PRD-12, Linux sandbox)

1. **Claude Code, direct key, one subagent.** Pass: every `.jsonl` under the isolated `projects/`
   changed since the start, at any depth, has a native stream; for each, the last END `seen` (an end
   offset, PRD-01) equals the file's size at the seal; the subagent's `subagents/agent-*.jsonl` and its
   `.meta.json` are there. Zero native GAPs. Core slice (phase 1a).
2. **Codex, direct key, prompt asking for subagents.** Pass: one native stream per rollout file, and
   the number of rollout files equals the threads in the harness's own record (`session_meta` with
   `thread_spawn`, as in run 1's `final-reconcile.txt`). No file missing, whatever their number
   (run 1 lost 11 of 43; lesson PRD-17 F7). Core slice (phase 1a).
3. **Hermes, direct key, one delegated child.** Pass: seal copy of `state.db` (and its `-wal` if
   present) with one `set` and `harness_exited: true`. `goalcup-verify usage --json` (PRD-09) on the
   entry reports `readable: true`, `db_set` naming the seal set, and a thread whose parent comes from
   `sessions.parent_session_id` (column name unverified on 0.22.5). This copy is what C1 option (a)
   of PRD-00 compares per session.
4. **OpenCode and OpenClaw** (not release blocking). Pass: database copies present and open with
   `sqlite3`; or the harness is marked "not verified" with the reason.
5. **Kill test** (PRD-01 acceptance 3, Codex, plain `kill -9`). Pass: for every rollout path, DATA
   offsets across all boots are contiguous from 0 with no native GAP; an overlap after the kill is
   allowed, a hole is not (invariant 14).
6. **Load, logged.** In every `full` pad row (20 minutes, many subagents) the watcher logs the native
   thread's CPU from `/proc/<pid>/task/<tid>/stat` and the recorder's descriptor count. Logged, never
   a pass line (PRD-14: no speed number fails a gate). A thread above 2% of one core on average is a
   night-log line for PRD-04.
7. **A1, the 2-hour Codex run (PRD-16).** At least one periodic `final` set with
   `harness_exited: false` per Codex database that changed, and a seal set. Logged, as acceptance 6.

### Automated tests that guard them

| Test file | Cases |
|---|---|
| `native_pattern.rs` | `**` matches zero and many levels; first matching row wins; refused patterns (empty, `.`, `..`, absolute, brace, NUL, `a**`) |
| `native_walk.rs` | A folder made at depth 6 is found; a store that does not exist yet is found once it appears (`capture_native.rs:1074`); 20,000 files in 4,000 folders are all taken (no cap) and the walk time is printed, not asserted; no descriptor stays open between passes; a folder whose `st_dev` differs (an existing mount under the test root, or skipped with a printed reason when none exists) is not entered and gives one `native_other_device` note; old files are skipped and the new one found among 5,000 (`:2128`); backdated mtime with a new ctime is taken (`:2261`); a file a second before the start is skipped (`:2320`) |
| `native_tail.rs` | Live EOF is not an end (`:1238`); a partial last line is kept (`:566, 1710`); truncate then append never reuses offsets (`:1321`); rename over is a new generation (`:1278`); a vanished file with unread bytes gives a GAP with `unread_at_least`; a file unreadable from the first pass to the seal gives an OPEN, one GAP `source_changed` and an END; binary bytes survive (`:1207`); a rewrite in place that grows past the offset is read as an append (stated limit) |
| `native_room.rs` | With `room()` 0 nothing is read and nothing is lost; after room returns, every byte is read in order; a read never exceeds `room()`; one huge file does not starve a small one |
| `native_snapshot.rs` | A snapshot is read once it settles; a torn read is a new generation (`:1962`); a periodic copy is taken when the set changed and the interval passed, not when it is unchanged, and the interval grows with size (interval as a test parameter); every file of one copy shares a `set`; a set still written during the copy ends its first copy `source_changed` and gives `snapshot_unstable` after one retry, both copies kept; a database file older than the run start whose `-wal` is new is copied with its `-wal`; a seal copy while the harness runs is marked; `-wal` and `-shm` each get a stream |
| `native_safety.rs` | A symlinked file and a symlinked folder are not read (`:631, 2059`); a FIFO is never opened and the test does not hang (`:686`); the path never holds the root (`:717`); the Hermes rows from PRD-11's embedded connector, run against a home holding `sessions/request_dump_x.json`, match no such file (`:1906`); `src/native/` holds no exclusion list and no harness name |
| `native_resume.rs` | Cursors from a previous boot: the new stream starts at `from`; a changed `fid` or a size below the cursor starts a new generation; an unchanged snapshot is not read again |
| `native_finish.rs` | Finish reads everything with no byte cap; with `guard` `None` it reads every file to the size seen in its first pass, however long that takes; with a guard, past it each file gets `cut_at_seal` with its exact `lost`; a set cut part way is cut for every file and the earlier periodic copy stays whole; `cut_files` and `cut_bytes` match the GAPs; native S1 calls made after the seal's stop are written; a `warn_missing` row with no file gives one `native_missing` note |

## Risks and blind spots

- **Walk cost grows with all files, not only active ones.** At 20,000 files the interval stretches to
  about 2.7 s (estimate). Nothing is lost, only seen later. If a pad row or the 2-hour run logs more,
  the hot and quiet scheduler of revision 1 can come back (about 90 lines); not before.
- **A file deleted with unread bytes** (a harness that cleans up at exit, or room at 0 for a long time
  first) loses them. Named by a GAP; the amount is a lower bound only.
- **Rewrite in place.** An append file rewritten in place and grown past the old offset between two
  passes is read as an append, so the record holds a mix (stated, as today). The tail witness that
  would catch it is cut. If a pad row shows a JSON line broken at a read boundary, add it back (about
  30 lines) for that case.
- **Renames read twice.** Bytes of a renamed file appear under both paths. Visible through `fid`.
- **Large rewritten snapshots** (a session JSON of a few MB rewritten each turn) make many generations.
  The 1 s settle helps; the 128 MiB compression window removes most repeats (PRD-01 section 14); still
  more bytes than an append file. Watch in the pad.
- **Periodic database copies cost bytes.** Up to about 3 GB a day raw for a large, busy database. A
  copy an hour apart is outside the zstd window, so it does not compress against the last one. The
  interval is an overridable default.
- **Database copies during the run are possibly inconsistent.** Said in the meta; the reader must cope,
  and can fall back to another copy of the entry.
- **A mask on SQLite page structure** leaves a copy unreadable (section 6, step 6). Rare, unverified;
  PRD-09 reports it.
- **Databases hold more than transcripts.** OpenClaw keeps `auth_profile_store` in its agent SQLite
  (`O-harness-access.md:956`). Only PRD-05's rules mask it; an opaque token with no known shape could
  pass. The pad's "no secret" check scans for the exact keys it set (PRD-12). A connector may decline a
  database that is mostly credentials (PRD-11's call).
- **Shared homes.** The run-start rule is safe only while runs on one home are serialized (D57). A
  second run on the same home at the same time would read the other's files.
- **Memory with many open append streams.** Each open `native` stream keeps a masker state (PRD-05) and
  a per-sid entry (PRD-02). 20,000 streams open at once is a few tens of MB if a quiet JSONL holds
  nothing undecided (unverified; the pad logs recorder memory).
- **macOS:** `stat` field names and the birth time differ (`sourcecap/unix.rs:420-475`); call costs are
  unverified. The owner's Mac run is a pad run with subagents; it logs the native thread's CPU.
- **A harness that writes outside its declared root** (Crush's workspace folder, Droid's cloud sync, a
  harness that starts another harness as a subagent and writes into the real home) is read only if
  PRD-11 names that root. The pad check "every subagent recorded" catches the miss.

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Pass interval | 1 s | PRD-04, with pad numbers |
| Duty cycle of the walk | 2% of one core (interval at least 50 times the walk) | PRD-04 |
| Wait when `room()` is too small | 100 ms | PRD-04 |
| Snapshot settle | 1 s unchanged | PRD-04 |
| Native share of the pipeline | `room()` (3/4 of the soft guard, PRD-02) | PRD-02 |
| Slice per pass | 64 MiB | PRD-04 |
| Periodic database copy | every 60 minutes when changed, or one minute per 2 MiB of the set if longer; never skips the seal copy | Owner (it costs upload and storage) |
| Snapshot retries | 1 | PRD-04 |
| `native_missing` delay | 5 minutes after the first model request is written | PRD-04 and PRD-10 |
| Finish time guard | 180 s at `deadline`, `server_stop`, `signal`; none otherwise | Owner (PRD-02 passes it, PRD-00 S8) |
| Seal copy of a database when the harness still runs | on | Owner |

## Asks of PRD-00, PRD-01 and PRD-02 (after revision 3)

Ruled. PRD-01 has the notes `native_missing` and `native_other_device` and lists PRD-04 as a writer
of GAP 10 `cut_at_seal`. PRD-00 R3-3 rules that the seal's stop covers wire streams only, that
native and snapshot calls stay admitted until `finish` returns, that the drain waits for wire bytes
only, and that `cursors.json` keeps `mtime_ns` for snapshot paths. PRD-02 uses the names
`native::start` and `NativeReader::finish`. Still open, one small item:

1. **PRD-00 and PRD-02:** the ruling row "PRD-02 and PRD-04" and PRD-02's interface row carry the
   `guard: Option<Duration>` argument of `finish` (section 1), so the seal passes 180 s only for
   `deadline`, `server_stop` and `signal` (PRD-00 S8 already states the rule), and the sealed
   `status.json` carries `cut_files {files, bytes}` from `NativeFinish.cut_files` and `cut_bytes`.
   PRD-02 and PRD-10 give `status.json` the `sealing: reading session files` state.

## Builder notes

Order of work, by PRD-00's slices:

1. **Core slice** (blocks phase 1a, Claude Code and Codex with one subagent): `fsx.rs` and
   `pattern.rs` with their tests (no pipeline needed); `walk.rs` against `Recorder::for_tests`; then
   `tail.rs` with append and snapshot reads and the room rule. Prove the 20,000-file test early: it is
   the case the old code could not do.
2. **Second slice:** `snapshot.rs` (Hermes `state.db` sets, periodic and seal copies), `finish` with
   the time guard, `native_missing`, then `native_resume.rs` once PRD-02's cursors exist (the kill
   test needs it).

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
- Do not hold any lock or folder descriptor while asking `room()` or waiting. When `room()` is too
  small, wait 100 ms; do not spin.
- The run start comes from the start facts (PRD-10), in whole seconds, compared with
  `max(mtime, ctime)` (D57); a `continue` passes the run's start, not its own.
- Never parse a session file, not even to find a newline. Raw bytes only.
- Never take a database copy in `close_dead_entry`, and never add a row for a `.env` or key file.
  The check that a row does not match a connector's `secret_files` is PRD-11's connector test, not
  code in `src/native/`.
- Mount tests: use `find <root> -xdev` to list what a same-device walk should see.

How a watcher proves it in a real run: after any pad run, `goalcup-verify dump <entry> | jq -c
'select(.k=="open" and (.skind=="native" or .skind=="snapshot")) | .meta.path'` gives the recorded
paths. Compare with `find <root> -xdev -type f \( -newermt @<since> -o -newerct @<since> \)`, with
`<since>` the run start in whole seconds, filtered by the connector's rows: every
file must appear. For each path, the last END `seen` of its last generation equals the file's size at
the seal, or a GAP names why. During the run, read `status.json` once a minute: `native_files` must
never fall. Log the native thread's CPU (`top -H -p <pid>`); a value above 2% is a night-log line, not
a failure.

## Review answers

History of revision 2. Where an answer here disagrees with the body or the "Alignment answers"
section, those win (ask numbers here are revision 2's). Sources: CC = `prd-consistency.md` item;
A = review A finding; B = review B item.

| Item | What it asked | Answer and what changed |
|---|---|---|
| CC 59 | `{config_home}` against PRD-11's `{home}` | Accepted. Section 2 and the example use `{home}` and `{workspace}` |
| CC 60 | Rows table duplicated with drift | Accepted. The table is gone; PRD-11 section 6 is the one source. Section 2 keeps the form rule and two points for PRD-11 to check (Hermes `manifest.json`, Codex `*.sqlite*`), both marked unverified |
| CC 61, B C14 | `warn_missing` cut by PRD-11 but kept here | Closed by PRD-00 S3 (kept per row). Section 2 now says what it does: one `native_missing` note after 5 minutes, which PRD-10 words. Note name asked of PRD-01 |
| CC 62 | Snapshot meta `set` | Accepted. Sections 6 and 7, `"<boot>.<n>"` as PRD-01 section 4 fixed it |
| CC 45 | Native API names in PRD-02 | Closed by PRD-00's ruling. Asked of PRD-02 to use the names (ask 3) |
| CC 47 | Who stops helpers before `finish` | Closed by PRD-00 S8: the watchdog runs `after`. Section 6 step 7 says so |
| CC 49 | Seal time budgets add up | Accepted. The 180 s guard is PRD-02's, inside its seal bound; defaults table changed. Found while checking: PRD-02's seal step 1 makes every S1 call a no-op before native finish runs; ask 2 |
| CC 28, 29, 30, 36, 40 | Reason 9, `from`, meta keys, notes, path text | Closed by PRD-01 revision 2. Text here uses them; acceptance 1 no longer sums `seen` over boots (PRD-01: `seen` is an end offset) |
| CC 24, 90 | Size and test budgets | Accepted. About 780 src lines (under 870); tests follow PRD-14's table (1,000), about 800 expected |
| B C1 | One S1 shape | Accepted. Section 1 lists PRD-00's final calls; an unknown loss is `lost: None` with the tail `unread_at_least`; a GAP ends at the call time |
| B A4 | Phase 1a needs native append rows | Accepted. Builder notes split the work by PRD-00's core and second slices |
| B A7 | WAL repair cannot fix a mask on page structure | Partly accepted. Section 6 step 6 states the limit and why it should be rare (masks are length preserving, the main file has no checksums); PRD-09 owns `readable: false` and C1 INCONCLUSIVE |
| B A8, R1 | Hermes C1 cannot pass | Routed to PRD-09 and PRD-12 by PRD-00 (option a). Acceptance 3 names the copy that comparison uses; periodic copies add mid-run data |
| B A11 | Dodger pass line | No change here; PRD-12 owns it. Native capture is the same in every case (section 10) |
| B N8 | Cut hot and quiet, list-on-change, 3x retry | Accepted. Section 3 is a full walk each pass with a 2% duty cycle (tighter than 5%, since the walk now visits every folder); folder mtimes are no longer trusted, which also removes the overlay risk. One retry for database sets. Can return if pad numbers say so (Risks) |
| B N9 | Defer the tail witness | Accepted. Cut; the rewrite-in-place limit is stated as today (`native_capture.rs:55-61`, PRD-17 F2) with a test that pins it |
| B T1 | Cut the soak script | Accepted. Acceptance 6 is now a logged figure from normal `full` pad rows; `native_walk.rs` keeps the 20,000-file case |
| B T3 | No speed number fails a gate | Accepted. CPU and walk time are logged, never asserted |
| A 9 | SQLite subagents only at the seal | Partly accepted. Periodic copies every 60 minutes when changed, scaled by size so they add at most about 2 MiB a minute (section 6 step 1). Rejected: a copy in `close_dead_entry` (that frame is closed and the harness state unknown; the next entry's copies hold the old sessions, section 6 step 8); splitting one copy across mask workers (one stream keeps one mask state, and PRD-00 starts with one worker); exempting the seal copy from the guard (the bound is PRD-02's, on the deadline path). A set cut at the seal falls back to the last periodic copy |
| A 6 | Live check of native growth against the wire | No change here. PRD-10 owns the line; it reads `native_bytes` and `last_native_at` from PRD-02's status, which count native writes already |
| A 8 | Processes that outlive the harness | No change here (PRD-02). The seal copy still marks a database a leftover process writes (`snapshot_unstable`) |
| A 12 | Watch `.env` files on the native thread | Rejected here. PRD-00 routes it to PRD-05 (owner question 12). The native thread must never record a `.env` file; section 9 and the builder notes say so |
| A 16 | Cross-harness subagents write into the real home | No change here. A child harness's rows are just more rows (no row cap); PRD-11 decides whether to add them or name the case "wire only". Listed in Risks |
| Own check | Room at 0 made the loop spin | Fixed: wait 100 ms, then read again (section 1) |

## Alignment answers

Revision 3, from the fix lists of the alignment reviews (group 1 and group 2; no group 3 list). The
ledger wins over every row here.

| Fix | What was done |
|---|---|
| G2-22 | Applied. Section 8 says the seal's stop covers wire streams only, native and snapshot calls stay admitted until `finish` returns, and the drain waits for wire bytes only (PRD-00 R3-3). Old ask 2 is ruled and removed |
| G2-28, L3-4 | Applied. Step 2 reads to the size seen in step 1's pass. Step 5: the 180 s guard applies only to `deadline`, `server_stop` and `signal`; on `harness_exit` and `seal_command` finish has no time guard and status shows `sealing: reading session files`. Defaults row changed to the owner. To make that buildable, `finish` takes `guard: Option<Duration>` (section 1); asked of PRD-00 and PRD-02 (ask 1), since PRD-00 S8 already states the rule |
| G2-29, L2-09 | Applied. Section 5 reads at most `room()`, which is already 3/4 of the soft guard; the reserve, its test case and the "Own check" review row are gone. Defaults row owned by PRD-02 |
| P04-4 | Applied. A file's stream opens when first taken; a file still unreadable at `finish` gets a GAP `source_changed` (PRD-01 GAP 5, not 9: 9 stays reserved) and END; new `native_tail.rs` case |
| P04-5, L2-10 | Applied. Section 9 writes one `native_other_device` note per folder per entry; `native_walk.rs` case and builder note (`find -xdev`) added. PRD-01 already lists the note |
| P04-6 | Applied. A `final` set is taken whole when any of its files passes the run-start rule; new `native_snapshot.rs` case |
| P04-7 | Applied. Section 6 defines a set; a moved copy ends `source_changed` and the retry is a new `set`; both copies stay |
| P04-8 | Applied. `NativeFinish` gains `cut_bytes`; PRD-02 writes `cut_files {files, bytes}` to sealed status (in ask 1) |
| P04-9 | Applied. `warn_missing` defaults to `true` (PRD-11); the 5-minute clock starts at the first written model request; example marked abridged |
| P04-10 | Applied. Boot step 5; seal step 2 is the stop and step 4 is native finish |
| P04-11 | Applied. The Hermes point is dropped (PRD-11 already has the `manifest.json` row); the Codex point stays |
| P04-12 | Applied. Section 6 step 4 names the seals where the harness can still run |
| P04-13 | Applied. The `after` commands run before the `seal` line, so before `finish` |
| P04-14 | Applied. `native_safety.rs` case uses PRD-11's Hermes rows; no exclusion list or harness name in `src/native/` |
| P04-15 | Applied. The `secret_files` check is PRD-11's connector test (section 9, interfaces, builder notes); none in `src/native/` |
| P04-16 | Applied. Row names unique per connector, refused at load by PRD-11; first match by absolute path |
| P04-17 | Applied. Section 1 step 4: a walk still runs once the interval has passed |
| P04-18 | Applied. A torn or changed snapshot is a new `snapshot` stream; snapshots carry no `gen` |
| P04-19 | Applied. Acceptance 3 reads `goalcup-verify usage --json` (PRD-09) |
| P04-20 | Applied. Acceptance 7: A1 logs periodic and seal sets per changed Codex database |
| P04-21 | Applied. Watcher proof uses `find -xdev` with `-newermt` or `-newerct` at the run start in whole seconds |
| P04-22 | Applied. Net is about 3,400 src lines (the listed parts sum to 3,397) |
| P04-23 | Applied. 180 s is about 9 GB at PRD-05's 50 MB/s target, about 5 GB at 30 MB/s |
| P04-24, L2-18 | Applied. The periodic interval is a cadence for upload and storage cost, not a cap; it never skips the seal copy and restarts at each boot |
| X2-11 | Partly. Moving the review sections into a new `REVIEW-HISTORY.md` is not done here: this pass may edit only this file, and many writers of one new file would race. Instead "Review answers" now says it is history and that the body and these answers win. Every ruling still binding is in the body. The move may run once, after every PRD's alignment pass, by one agent |

## Amendments

### 2026-10-30, replan after PRD-05 phase 0 slice (merge 82484521)

- **Native OPEN meta under the S2 stub.** The stub's `mask_meta` stars every string leaf, so a native
  stream's `meta.path` (`<row name>:<path>`, the stream ID, point 5) is written as stars of the same
  length until PRD-05's core slice merges. PRD-04 keeps its own clear path-to-stream map in memory and
  never reads a path back from written meta (PRD-02 note). Native tests that assert a written path use
  `Masker::passthrough_for_tests()` (feature `test-hooks`). An entry built on the stub cannot be
  rebuilt by native path; that is the stub, not a defect.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-04 core slice part 1 (merge 403d9165)

- **On integration:** `src/native/fsx.rs` and `src/native/pattern.rs`; `mod.rs` only declares them.
  `start`, `NativeReader::finish`, `walk.rs`, `tail.rs`, `snapshot.rs` and the loop are the next
  slice, which waits for PRD-11's S3 stub (`connector::NativeSources` is still a placeholder).
  `fsx.rs` takes `Fid` from `crate::pipeline::cursors` (PRD-02), so the cursor's `fid` and the
  walk's are one type.
- **`parse` is one refusal stricter than section 2** (review A7): it also refuses a trailing `**`
  (`**` alone, or a pattern ending in `/**`), because `**` matches folders only and such a row could
  never match a file. PRD-11's connector test calls `native::pattern::parse` as PRD-04's pattern
  check, so this rule reaches connector rows too. No row in PRD-11 section 6 or in `connectors/*.toml`
  at 403d9165 ends in `**`.
- **The `finish` seam is still open** (builder log 12:28Z, ask 1 above). PRD-04 section 1 and PRD-02
  ask 10 both pass the time guard into `finish`; PRD-00's ruling row "PRD-02 and PRD-04" omits it. The
  next slice builds section 1's shape, `finish(self, harness_exited: bool, guard: Option<Duration>)`,
  with `Some(180 s)` only for `deadline`, `server_stop` and `signal`, so PRD-02's seal has one shape to
  call (PRD-00 and PRD-02 notes).
- **Open nits for the next touch of these files** (acceptance lists any still open as known):
  - A8: `open_file` returns a real errno when a regular file is replaced by a socket between
    `stat_at` and `open_file` (ENXIO on Linux, EOPNOTSUPP on macOS), so `tail.rs` would write
    `native_unreadable` where section 4 wants the next walk to settle it. The next slice maps both
    errnos to `NotFound` beside ELOOP when it writes `tail.rs`.
  - B8: the `!rel.as_os_str().is_empty() &&` guard in `Pattern::matches` is dead now that every
    accepted pattern ends in a glob; drop it.
  - B9: one `stat_by` helper for `stat_at` and `file_stat` (and one `openat` helper for `open_root`
    and `open_child`), one `dev_t` explanation instead of two; about 12 lines, same behaviour.
  - B10: `file_stat` has no caller in `src` yet; `tail.rs` will use it. Keep it, written through the
    B9 helper.
- **Size** (`wc -l` at 403d9165): src 401 (`fsx.rs` 226, `pattern.rs` 167, `mod.rs` 8) against the
  row's 780; the two files carry 393 lines against their estimates of 220 (120 and 100). Tests 246
  (`native_pattern.rs` 123, `native_safety.rs` 123), no inline test modules. That leaves 379 src
  lines under the row (457 under its 10% line) for `mod.rs`, `walk.rs`, `tail.rs` and `snapshot.rs`,
  whose estimates add to 560. Built to those estimates the area lands near 950, about 22% over the
  row. The next slice therefore plans to the 379 lines left: B8 and B9 first, then less code in the
  new files; passing the 10% line needs a replan `decision` line that names the point the extra lines
  serve (PRD-00 "Size budget").
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 2 (merge b9fa50f8)

- **S3 is on integration.** `connector::NativeSources(Vec<NativeSource>)` with
  `NativeSource { name, root: PathBuf, pattern: String, form, since, warn_missing }` and
  `Form::{Append, Snapshot, Final}`, in connector file order. The next PRD-04 slice (`start`,
  `finish`, walk, tail, snapshot) is unblocked; `start` parses each `pattern` with `pattern::parse`.
- **Correction to the 403d9165 note:** at b9fa50f8 neither PRD-11's loader nor `connector_builtin.rs`
  calls `native::pattern::parse`; the loader's own check still accepts a trailing `**`. PRD-11's next
  slice switches both to `native::pattern` (PRD-11 note). No shipped row has that shape.

### 2026-10-30, replan after PRD-14 slice 2 (merge 04886295)

- PRD-04's ask (14:31Z) for `opens`, `ends` and `events` helpers over `Calls` in
  `tests/common/recorder.rs` is taken into PRD-14 slice 3 (PRD-14 note). Until it lands, native tests
  keep their private copy; the slice that follows its merge deletes the copy and calls `common::recorder`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-04 core slice part 2 (merge 01afcf22)

- **On integration:** `walk.rs` (`Walker::pass(rec, rows) -> Walked`, `Row`, `Found`) and `tail.rs`
  (`Tail::new(since, cursors)`, `settle`, `sweep(rec, unlisted)`, `read(rec, slice, settle) -> Stop`,
  `close(self, rec, cut) -> Closed`). Nits A8, B8, B9 and B10 from the part 1 note are closed (ENXIO
  and EOPNOTSUPP map to `NotFound`; one `stat_by` helper; the dead `is_empty` guard is gone).
  `tail.rs` already copies a single snapshot file whole, tears it on a change and gives the exact or
  cut GAP for its last state (review A-R1, A-R2). The next slice (`start`, the loop and duty cycle,
  `NativeReader::finish(self, harness_exited, guard)`, `native_missing`, `snapshot.rs`) builds on these
  APIs; `snapshot.rs` keeps only what `tail.rs` does not do: grouping a database and its `-wal` and
  `-shm` into one `set`, the periodic interval, the one retry with `snapshot_unstable`, and the live
  seal mark.
- **Size decision** (review RB8; `wc -l` at 01afcf22, PRD-00 row 780 src and 1,000 tests): src 909
  (`tail.rs` 363, `fsx.rs` 213, `pattern.rs` 174, `walk.rs` 131, `mod.rs` 28), 117% of the row;
  tests 1,236 (`native_tail.rs` 469, `native_safety.rs` 240, `native_walk.rs` 202,
  `native_pattern.rs` 132, `native_resume.rs` 106, `native_room.rs` 87), 124% of the row. The extra
  lines serve point 1 and point 3, not features: a truncation still past the offset gives a GAP, a
  folder the walk could not list is not taken as gone, a snapshot gone or cut with bytes unread gives
  its exact GAP, and a snapshot torn during a copy names its last state lost (review A-R1, A-R2, C1).
  This replan sets the PRD-04 ceiling to **1,150 src and 1,600 tests** (370 and 600 over the row):
  `mod.rs` grows by about 120 and `snapshot.rs` stays at or under 110; `native_snapshot.rs` and
  `native_finish.rs` come within the test ceiling. Cuts the next slice makes first: RB7 below, and the
  private `opens`, `ends` and `lost` helpers in `native_tail.rs` once PRD-14's `common::recorder`
  helpers land (PRD-14 note). Past 1,150 or 1,600 needs a new `decision` line. The end-of-build report
  counts this against the Rust totals.
- **Open nit for the next touch of `tail.rs`** (acceptance lists it as known if still open):
  - RB7: in `end()` the guard `(exact || !e.snap)` in `else if n > 0 && (exact || !e.snap)` is always
    true (settle passes only append entries, sweep passes `exact = e.snap`, close passes `true`).
    The line becomes `else if n > 0 {`; same behaviour, one dead path less.
- **Stated limits** (fixer decisions 15:03Z; acceptance lists them as known unless closed):
  - C2: a nested root on another device that holds a file first-matching the outer row is taken by
    no row; `native_other_device` names the outer row.
  - RB3: `native_safety.rs` guards only the "no harness name" half of the acceptance row; the "no
    exclusion list in `src/native/`" half is unguarded. The next slice either adds a small check or
    acceptance lists it as unguarded.
- **Seams still open:** `native_walk` is not yet in the nextest `heavy` group (PRD-14 note, `.config`
  is PRD-14's); `SnapshotCursor` has no `fid`, so resume skips a snapshot replaced with the same size
  and `mtime_ns` (PRD-02 note). Until PRD-02 adds it, `native_resume.rs` keeps the size and
  `mtime_ns` check only and acceptance lists the gap as known.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 3 (merge 00fac28d)

- **The `Calls` helpers are on integration** in `tests/common/recorder.rs`: `opens(c) -> Vec<(Sid,
  StreamKind, Value)>`, `ends(c) -> Vec<(Sid, EndReason, u64, Option<Value>)>` (the END tail is the
  fourth field), `events(c) -> Vec<Value>`, `gaps(c) -> Vec<Call>`, `data_of(c, sid)` and a generic
  `pick(c, f)`. PRD-04's next slice deletes the private `opens`, `ends` and `lost` in `native_tail.rs`
  and `events` in `native_walk.rs` and calls `common::recorder`; `lost`'s shape is one `pick` over
  `Call::Gap` at the call site. The lines saved count against PRD-04's 1,600 test ceiling.
- `native_walk` joins PRD-14's `heavy` group in PRD-14's next touch of `.config/nextest.toml`.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 4 (merge ec0d79d8)

- **`native_walk` is in PRD-14's `heavy` group** (`.config/nextest.toml` at ec0d79d8): its 7 tests
  run one at a time, after the parallel binaries. The seam named in the core slice part 2 and PRD-14
  slice 3 notes is closed. `gate fast` on the merge ran all 7 and passed.
- The flake the PRD-10 slice 4 note lists (`old_files_are_skipped_and_the_new_one_found_among_five_thousand`
  failed 1 of 4 runs in a box, while it still ran beside other binaries) now runs alone. If it fails
  again under `heavy`, the cause is in the test or in `src/native/`, not load from other binaries;
  PRD-04 owns that fix, and acceptance lists it as known if it is still open at the end.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-04 core slice part 3 (merge a302d688)

- **On integration:** `native::start(rec, sources) -> NativeReader` and
  `NativeReader::finish(self, harness_exited, guard: Option<Duration>) -> NativeFinish { streams,
  snapshots, cut_files, cut_bytes, took_ms }`, the loop (a walk every max(1 s, 50 times the walk),
  64 MiB slices between walks, a 100 ms wait when room is short), finish steps 1, 2, 4 and 5, and
  `native_missing` through the public `Missing` clock. Nits RB7 and RB3 from the part 2 note are
  closed: `end()` lost its dead guard, and `native_safety.rs` now refuses quoted secret-file names in
  `src/native/` (the no-exclusion-list half). The `native_walk` run-start flake named in the PRD-14
  slice 4 note is fixed in the test (3ede1ec3: it waits past the kernel's coarse file clock); close it.
- **Interim behaviour until `snapshot.rs` lands** (the next slice removes each one):
  - A `final` row is not copied during the run at all: it is walked and counted for
    `native_missing`, then copied once at the seal as single files with OPEN meta `form: "snapshot"`,
    no `set` and no `harness_exited`. A dead entry therefore holds no database copy, and
    `NativeFinish.snapshots` is always 0. `finish` ignores `harness_exited`.
  - The next slice restores section 6 in full: `-wal`, `-shm` and `-journal` grouped into one `set`,
    periodic copies, `form: "final"` with `set` and `harness_exited`, one retry with
    `snapshot_unstable`, `final_snapshot_live`, and `snapshots` counted. It hooks step 3 into
    `Reader::finish` between step 2 and `close`, so a guard that runs out cuts a set for every file
    of it (section 8 step 5).
- **Behaviour the slice chose, kept** (point 3, no silent loss): a reader thread that fails to spawn
  writes `native_missing` for every row, not only `warn_missing` rows; a row whose pattern fails
  `pattern::parse` is named at once; a `NativeReader` dropped without `finish` is finished with a zero
  guard, so every unread file gets its `cut_at_seal` GAP. At finish, a snapshot torn by the state the
  seal pass saw is copied again once; one torn by a newer state is not restarted and `close` names its
  loss, so an unguarded finish cannot loop on a file that never stops changing.
- **Real runs cannot write `native_missing` yet:** `Missing::check` takes
  `rec.counts().model_requests()`, and `Recorder::counts()` returns `Counts::default()` at a302d688,
  so the count stays 0 and the five-minute clock never starts. `native_finish.rs` passes counts by
  hand, so the tests do not show it. The fix is PRD-02's (PRD-02 note); nothing changes here.
- **Size decision** (`wc -l` at a302d688; ceiling set by the part 2 note: 1,150 src, 1,600 tests):
  src 1,144 (`tail.rs` 368, `mod.rs` 258, `fsx.rs` 213, `pattern.rs` 174, `walk.rs` 131), 6 lines
  under the ceiling, with `snapshot.rs` (estimate 110) and the step 3 hookup still to come; `mod.rs`
  is 258 against the part 2 note's "grows by about 120" (builder decision 15:59Z names points 3 and 6:
  spawn failure names every row, a dropped reader closes with GAPs, `Missing`). Tests 1,474
  (`native_tail.rs` 469, `native_safety.rs` 250, `native_finish.rs` 227, `native_walk.rs` 203,
  `native_pattern.rs` 132, `native_resume.rs` 106, `native_room.rs` 87), 126 under 1,600.
  This replan sets the PRD-04 src ceiling to **1,260** (1,144 plus `snapshot.rs` at or under 110 and
  about 6 lines of hookup); the extra lines serve point 1 (every database set copied whole with its
  `-wal` at each seal, and periodic copies that keep a dead entry's sessions) and point 3 (one retry,
  a live copy said plainly, never a skipped seal copy). The test ceiling stays **1,600**. Cuts the
  next slice makes first: the lines the interim `final` skip in `Reader::pass` and the single-file
  `final` path no longer need, and the private test helpers still kept despite the PRD-14 slice 3
  note (`opens`, `ends`, `lost` in `native_tail.rs`; `opens`, `ends`, `notes` in `native_finish.rs`;
  `notes` in `native_safety.rs`; `events` in `native_walk.rs`), replaced by `common::recorder`
  (`opens`, `ends` with the tail as fourth field, `events`, `pick`), so `native_snapshot.rs` fits.
  Past 1,260 src or 1,600 tests needs a new `decision` line. The end-of-build report counts it
  against the Rust totals.
- **Stated limits from review** (fixer 16:11Z, accepted by reviewer A 16:31Z): N1, N2, N3 and N5 were
  declined as nits and stay as they are; acceptance lists them as known by those names if the last
  slice does not close them. Still known from earlier notes: C2 (nested root on another device) and
  the missing `fid` in `SnapshotCursor` on resume (PRD-02 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, fix round on the database set slice (4f218cbc)

- **Slice per reader** (review A9): the loop gives `tail.read` and `sets.read` a 64 MiB slice each,
  so one pass reads at most 128 MiB. The slice is fairness, not a cap, and nothing is lost; section 5
  reads as a 64 MiB slice per reader.
- **Test helpers** (review A7): `tests/common/` stays PRD-14's. The four helpers PRD-04 added there
  (`append`, `notes`, `end_of`, `gap_rows`) now live in `tests/native_common/mod.rs`, read under the
  `tests/native_*` row as PRD-10 reads `cli_common/`. Moving them into `common::recorder` is a request
  to PRD-14, not done here.
- **Gone sets** (review A8): a set none of whose files was in the last walk, with no `unlisted`
  folder holding one, and whose last state a copy already took, gets no seal copy. A gone set never
  copied still gets one, and its GAP names the real loss.
- **Short streams** (review B8): `tail::end_stream` is the one place that names a stream cut short,
  for a file and for a database copy.

### 2026-10-31, replan after PRD-04 second slice (merge 7e8ed89b)

- **PRD-04 code is complete on integration:** `snapshot.rs` (database sets, section 6) and the finish
  step 3 hookup (section 8). The interim behaviour of the part 3 note is gone: a `final` row is copied
  during the run and at each seal as one `set` per database (`-wal`, `-shm`, `-journal` beside it),
  OPEN meta `form: "final"` with `set`, `fid` and `harness_exited`; one retry with
  `snapshot_unstable`; `final_snapshot_live` when the harness still ran; `NativeFinish.snapshots`
  counts sets; `finish` honours `harness_exited`. Notes in use: `native_unreadable` (`path`, `errno`),
  `snapshot_unstable`, `final_snapshot_live`, `native_missing`; the `cut_at_seal` GAP.
- **Lost set files** (reviews A1, A2, B2, A8): a set file that vanishes or is unreadable mid-copy ends
  its stream with a GAP naming the bytes not read (exact when the file is gone), never short and
  unnamed; a whole set gone after a whole copy is reported lost at the seal; a gone set whose last
  state a copy already took gets no seal copy; a gone set never copied still gets one.
- **Test helpers:** the private helpers of the part 3 note are replaced by `common::recorder` (`opens`,
  `ends`, `events`, `pick`) as asked; `lost` is a pick over `Call::Gap` at each call site. The four
  PRD-04 helpers stay in `tests/native_common/` (fix-round note, review A7, nit A10: PRD-00 note).
- **Size decision** (`wc -l` at 7e8ed89b; PRD-00 row 780 src and 1,000 tests; ceilings of the part 3
  note 1,260 and 1,600). Src 1,488: `tail.rs` 410, `snapshot.rs` 289, `mod.rs` 271, `fsx.rs` 213,
  `pattern.rs` 174, `walk.rs` 131; 191% of the row, 228 over the 1,260 ceiling (`snapshot.rs` is 289
  against its estimate of 110, `tail.rs` +42 for the shared `end_stream`). Tests 1,768: `native_tail.rs`
  441, `native_snapshot.rs` 346, `native_safety.rs` 245, `native_walk.rs` 194, `native_finish.rs` 173,
  `native_pattern.rs` 132, `native_resume.rs` 106, `native_room.rs` 87, `native_common/mod.rs` 44; 177%
  of the row, 168 over the 1,600 ceiling. The extra lines serve point 1 (every database set copied whole
  with its `-wal` and `-shm`, one set id across its files, periodic copies that keep a dead entry's
  sessions) and point 3 (a file lost mid-copy named by a GAP, `native_unreadable` and
  `snapshot_unstable` said plainly, a live copy said plainly, no skipped seal copy). This replan sets
  the PRD-04 ceilings to **1,500 src and 1,780 tests**. PRD-04 has no further slice planned; a change
  past either ceiling needs a new `decision` line. The end-of-build report counts it against the Rust
  totals. Cuts left for a later touch, not required: the `native_tail.rs` cases that only restate
  `native_safety.rs` ones.
- **Still open, and not PRD-04's to close:** `native::start` and `NativeReader::finish` have no caller
  (PRD-02 wires boot step 5 and seal step 4; PRD-02 note); `Recorder::counts()` still returns
  `Counts::default()`, so `native_missing` never fires in a real run; `SnapshotCursor` has no `fid` on
  resume. Acceptance lists as known, if still open at the end: C2 (nested root on another device), the
  missing snapshot `fid` on resume, and review nits N1, N2, N3 and N5. The part 3 line about `final` rows
  copied only at the seal is closed.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-02 core step 1 (merge 2fd96576)

- **`SnapshotCursor.fid` is a joint change, still open.** PRD-02's merge left `SnapshotCursor` as `size`
  and `mtime_ns` (its doc comment says so), because adding `fid` in `cursors.rs` breaks the literals in
  `tests/native_resume.rs` and the `Tail::new` comparison, which are this PRD's paths. When PRD-02's
  next slice adds `fid: Fid`, this lane compares it in `Tail::new` and fixes its literals in the same
  merge window (PRD-02 note, same date). Each lane edits only its own paths. Until then a snapshot
  replaced by a file of the same size and `mtime_ns` is skipped on resume, as the known-gap line says.
- **`Recorder::counts()` is filled** (counts at the last LINK) and `Recorder::live()` is filled, so
  `native_missing` can start from `rec.counts().model_requests()`. In a real run both stay `Default`
  until PRD-02's `boot.rs` lands; a test that needs a non-zero `model_requests` builds it through the
  recorder's own write path, not a literal.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-05 core slice (merge 2c95e8f1)

- **The real `mask_meta` is on integration**, so the stub note on native OPEN meta no longer holds. It
  scrubs the `path` string with the whole-record rules, stars a credential query value and a userinfo
  before the last `@`, and leaves `method`, `status`, `host`, `port` and `alpn` clear. Keep reading the
  path for the stream map before `mask_meta` runs, not after. An entry built on a stub-era build still
  cannot be used as evidence.
- No locked ledger item is touched and no owner question comes from this slice.
