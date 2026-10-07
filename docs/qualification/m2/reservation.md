# T01 — Qualify real chain-file reservation (macOS/APFS)

**Public-cut authority.** This included qualification record is authoritative for the T01
measurements and interface facts below. The private packet, C probe, run script, committed
`RESULTS.md` log and local, not in git, raw output are historical source identities, not files
shipped in the public cut and not prerequisites for reading this record or claiming its measured facts.

Historical packet identity (omitted): `docs/superpowers/plans/2026-09-25-m2-recorder/T01-reservation-proof.md`.
Historical probe and run-script identities (omitted): `spikes/m2-reservation/`; historical
`RESULTS.md` identity (omitted): `spikes/m2-reservation/RESULTS.md`, a sibling of the
`spikes/m2-reservation/out/` directory whose contents were local, not in git. The public cut
retains this record as the evidence authority.

**Fix round 2 notice.** A scoped re-review of fix round 1 (`fa9ff15..4f5c3c9`)
found that seven of eight findings were addressed, but the persistence
finding (C1) was only partly fixed: the corrected recipe
(`F_ALLOCATEALL|F_ALLOCATEPERSIST`, `0xc`) had only been exercised on a
dedicated persistence volume, while chainA, chainB, and the allocation-
failure/reduced-reserve/unaligned-length experiments still reserved with
plain `all` (`0x4`) — even though this document claimed those experiments
used the persist flag. The re-review also caught an arithmetic error in the
"percentage of reservation spent" figure, several places where "full
allowance"/"entire"/"fully-consumed" overstated what was measured, a gap in
the host-disk safety guard (it ran after the first attach but not after a
remount's re-attach), and a stale "same process" persistence claim left over
from a step this document's own fix round 1 had already removed from the
omitted historical `run.sh`. All of those are fixed below: every reservation on the main
experiment volume now uses `all,persist`, a real `hdiutil detach`/`attach`
remount of that volume happens after reserving and before any append, the
device/size safety check now runs after every attach including a remount's,
and every number in this record was re-derived from the fix-round-2 re-run captured in the
omitted historical `spikes/m2-reservation/RESULTS.md`, not hand-edited. The resulting measured
facts are preserved here; the omitted log is not needed or claimed to be present.

## Overall verdict

**PASS on APFS, for `F_ALLOCATEALL|F_ALLOCATEPERSIST`, measured end-to-end
on the recipe T03R is actually told to use.** Plain `F_ALLOCATEALL` FAILS to
survive a real unmount/remount. Measured, not assumed:

- The reservation is real physical allocation, consumed from the volume's
  free space immediately (confirmed via `df` and independently via
  `st_blocks`), not a sparse/no-op accounting trick.
- It does **not** extend logical EOF (`st_size` unchanged after the call).
- **Persistence is conditional on the flag, confirmed by an actual
  `hdiutil detach`/`attach` of the disposable image, not just a fresh
  process on the same still-mounted volume:**
  - `F_ALLOCATEALL` alone (0x4): survives close/reopen and abnormal child
    exit (`SIGKILL`) *as long as the volume stays mounted*, but is **released
    by a real remount** — confirmed twice, independently, on two separate
    files (one reserved directly, one reserved by a child that was then
    `SIGKILL`ed with the fd open).
  - `F_ALLOCATEALL|F_ALLOCATEPERSIST` (0xc): survives the same real
    remount, twice, on both a directly-reserved file and a SIGKILLed-child
    file.
- **This is not just an isolated claim about a separate test volume: the
  two chain files that are actually exhausted, appended to, and driven to
  their terminal record in Experiments 4/5 below were themselves reserved
  with `0xc` and were themselves remounted (once, on the main experiment
  volume, right after reserving and before any fill or append) and
  reconfirmed intact by a fresh-process `stat` before anything else
  happened to them.** The recipe qualified and the recipe exercised by
  every other experiment in this report are now the same recipe.
- Once reserved and confirmed to have survived that remount, the chain file
  can still be appended to (with per-group `fsync`) after a competing writer
  has driven the volume to **confirmed, real exhaustion** (not just the
  first `ENOSPC`): a brand new file's `open()` call itself fails with
  `ENOSPC` before any chain append is attempted. Payload and a terminal
  record together were driven to within 4,650 bytes of the file's entire
  22,024,192-byte physical allocation (99.979% of the requested reservation
  spent — see Experiment 4 for the exact arithmetic), and every byte of both
  writes succeeded.
- **A reservation is a protected minimum for its own file, not a ceiling.**
  With chainA's physical allocation consumed to within 4,650 bytes of its
  full 22,024,192-byte grant, one further append past that
  point — requiring the kernel to grant a *new* block beyond what the file
  already holds — failed cleanly with `ENOSPC` on the confirmed-exhausted
  volume. This is the failure half of "floor not ceiling," and it is what
  this run measured directly; the complementary "succeeds if the volume has
  free space elsewhere" half follows from ordinary POSIX write semantics but
  was not separately exercised as its own measurement in this run (see
  Experiment 5). A second, independently reserved file (chainB) spent
  4,505,600 of its own separate 5,242,880-byte reservation (about 86%)
  immediately afterward, completely unaffected by whatever had just happened
  to chainA. **T03R's own byte-budget accounting is what enforces the
  logical limit; the physical reservation only guarantees that budget is
  actually available when spent.**
- `F_ALLOCATEALL` gives true all-or-nothing preflight refusal: requesting far
  more than free space fails the `fcntl` with `errno=28` (`ENOSPC`) and
  leaves the file's allocation exactly as it was — no partial grant.
- A second `F_PREALLOCATE` call with a **smaller** `fst_length` than an
  already-satisfied reservation is **not a shrink or a no-op**: it grants
  *additional* space on top, measured for both block-aligned and
  non-block-aligned lengths. **`fst_bytesalloc` echoes the requested byte
  count back verbatim on success — it does not reflect the real, rounded-up
  physical allocation.** T03R must compute actual consumption from
  `st_blocks × 512`, never from `fst_bytesalloc`.
- Filesystem-type gating (the omitted T03R packet's requirement) must compare
  against the literal `statfs(2)` `f_fstypename` strings measured here:
  `"apfs"` (supported), `"hfs"` (unsupported), `"msdos"` (unsupported) — not
  guessed spellings like `"hfs+"` or `"fat32"`.

**Two other filesystems were measured as required by the omitted packet's
"unsupported filesystem/platform" step, and both FAIL, confirmed against a
real remount, not just a same-mount fresh process:**

- **HFS+**: `F_PREALLOCATE` (with or without `F_ALLOCATEPERSIST`) returns
  `rc=0` and reports the requested bytes allocated — but the reservation is
  released as soon as the reserving process's last descriptor closes, even
  *without* an intervening unmount (a stronger, faster failure than APFS's).
  An explicit remount afterward confirms it stays gone. No errno is ever
  set; the only way to see this is to actually reopen and measure.
- **FAT32 (MS-DOS FAT32)**: `F_PREALLOCATE` returns `rc=0` and reports the
  requested bytes allocated, and a real unmount/remount shows this is
  released too (block count drops from 2,049 back to 1, the bare-seed
  value), just like HFS+, only visible once a genuine remount is performed
  rather than a same-mount fresh-process check. Separately, in the original
  (pre-fix-round-1) run, appending into a file immediately after
  `F_PREALLOCATE` in the *same* mount session failed with `errno=22`
  (`EINVAL`), reproduced twice; in every run since (with a remount
  interposed first, which clears the stale reservation state), the
  identical append has succeeded as an ordinary small write. Both
  observations are genuine; the `EINVAL` evidence itself now rests only on
  the superseded `fa9ff15` log rather than a committed run, and is reported
  here as historical context, not a re-verified claim. Either way, FAT32's
  `F_PREALLOCATE` behavior is unreliable and must not be relied on for
  anything.

Neither filesystem is the platform T01 is qualifying (the tracker's supported
target is APFS), so neither blocks T03R. They are recorded because the
omitted packet explicitly asks for the unsupported-filesystem experiment and an
honest errno/behavior, and because they reinforce why T03R must gate on
filesystem type and must not trust a bare `rc==0`.

## Environment

```
$ sw_vers
ProductName:            macOS
ProductVersion:         26.6.2
BuildVersion:           25G83
$ uname -m
arm64
$ xcrun --show-sdk-path
/Applications/Xcode.app/Contents/Developer/Platforms/MacOSX.platform/Developer/SDKs/MacOSX.sdk
```

Toolchain: Apple clang version 21.0.0 (clang-2100.3.34.2), target
`arm64-apple-darwin25.6.0`, from `/Applications/Xcode.app/.../XcodeDefault.xctoolchain`.

## SDK header inspection (not guessed names)

Quoted directly from
`$(xcrun --show-sdk-path)/usr/include/sys/fcntl.h` on this machine:

```c
#define F_PREALLOCATE   42              /* Preallocate storage */
...
/* allocate flags (F_PREALLOCATE) */

#define F_ALLOCATECONTIG  0x00000002    /* allocate contigious space */
#define F_ALLOCATEALL     0x00000004    /* allocate all requested space or no space at all */
#define F_ALLOCATEPERSIST 0x00000008    /* do not free space upon close(2) */

/* Position Modes (fst_posmode) for F_PREALLOCATE */

#define F_PEOFPOSMODE 3                 /* Make it past all of the SEEK pos modes so that */
                                        /* we can keep them in sync should we desire */
#define F_VOLPOSMODE    4               /* specify volume starting postion */
...
/* fstore_t type used by F_PREALLOCATE command */

typedef struct fstore {
	unsigned int fst_flags; /* IN: flags word */
	int     fst_posmode;    /* IN: indicates use of offset field */
	off_t   fst_offset;     /* IN: start of the region */
	off_t   fst_length;     /* IN: size of the region */
	off_t   fst_bytesalloc; /* OUT: number of bytes allocated */
} fstore_t;
```

The `F_ALLOCATEPERSIST` comment ("do not free space upon close(2)") reads as
if plain `F_ALLOCATEALL` frees space at close — Experiment 3 confirms this is
literally true on a real unmount, not just close(2): plain `F_ALLOCATEALL`
reservations are gone after a real remount even though a same-mount
fresh-process `stat` can still show them intact (the vnode stays warm on the
same mount). `man 2 fcntl` also states `F_PEOFPOSMODE` allocates "from the
physical end of file," with `fst_length` being "the number of newly
allocated bytes desired" — this is the documented anchor for the
additive-repeat-call behavior described in Experiment 5. `F_VOLPOSMODE` and
`F_ALLOCATECONTIG` were not used — the omitted packet's mechanism is "behind current
EOF" (`F_PEOFPOSMODE`), and contiguity is not part of the requirement.

## Historical compiler invocation (source omitted from the public cut)

```
clang -std=c11 -Wall -Wextra -O2 -o out/reserve_probe reserve_probe.c
```

Zero warnings, zero errors. No crates, no Cargo, no shared manifests — plain
libc via the system toolchain, per the omitted packet's ownership rule.

## Method

The omitted historical `spikes/m2-reservation/reserve_probe.c` is a small standalone C program with
subcommands (`stat`, `fstype`, `seed`, `reserve`, `append`, `fill`,
`child_abort`, `child_kill`, `roundtrip`, `unlink`) that each do one measured
thing and print `RESULT`/`ERR` lines carrying exact
`st_size`/`st_blocks`/`fst_bytesalloc`/`errno`/`f_fstypename`. Every file the
probe treats as "the chain" is opened with `O_NOFOLLOW|O_CLOEXEC` and mode
`0600`, and the probe never calls `ftruncate`/`truncate`.

The omitted historical `spikes/m2-reservation/run.sh` builds the probe, then creates disposable,
size-limited APFS/HFS+/FAT32 disk images under `spikes/m2-reservation/out/`
with `hdiutil create -type SPARSE` + `hdiutil attach -nobrowse -mountpoint`,
runs the full experiment sequence against them (including a real `hdiutil
detach` + `attach` of the same backing image both for the dedicated
persistence check and for the main experiment volume, between reserving and
appending), and **always** detaches and deletes every image it created via
an `EXIT` trap — verified after the run: no leftover mounts, no leftover
`.sparseimage` files, no leftover test disks in `diskutil list`. The host
disk was never at risk: images are 16–64 MiB, and the competing-writer step
only fills the disposable volume, not the host disk (host free space was
~528–540 GiB throughout, unchanged before/after, across every round of this
qualification).

**Host-disk safety hardening** (review findings C2 / review-2 #1, closed
further in fix round 2): the omitted historical `run.sh` uses `set -euo pipefail` so any unexpected
`hdiutil` failure aborts into the cleanup trap instead of silently falling
through to a plain host-filesystem directory; `make_volume` refuses to reuse
a stale image from a previous run and checks `hdiutil create`/`attach` exit
codes explicitly. A shared `verify_disposable_mount` check — confirming the
mountpoint is really a different device from `out/` (`stat -f %d`) and that
its reported `df` capacity is within 2x the requested image size — now runs
after **every** `hdiutil attach`, including a `remount_volume` re-attach,
not just the first attach of a run (the fix-round-1 version only checked the
first attach; a re-review caught that gap). `reserve_probe.c`'s `fill`
subcommand takes a mandatory, enforced `max_bytes` argument and additionally
clamps to an absolute internal 512 MiB ceiling regardless of that argument
(`FILL_HARD_CEILING_BYTES`), so the competing-writer loop cannot become
unbounded even under a caller mistake. These guards were exercised for real,
not just in theory: fix round 1's development caught and fixed an actual
bash scoping bug in `remount_volume` via the omitted historical script's own `set -u` aborting
cleanly rather than misbehaving.

**Historical `reserve_probe.c` heap-overflow fix** (review finding I4 / review-2 #2):
`cmd_append`'s per-record buffer was sized from `record_bytes`, but used the
*unclamped* return value of `snprintf` (the length that would have been
written if untruncated) to place the closing `"`,`}`,`\n` bytes — for a
`record_bytes` too small to hold the label, this indexed and wrote past the
allocation. Fixed by validating `prefix_len + 3 <= record_bytes` before use
and reporting a clear `ERR` instead of corrupting the heap. None of the
actual experiment calls in the omitted historical `run.sh` trigger this (all use `record_bytes` far
above the floor), so no committed result was affected; a re-review rebuilt
the probe with AddressSanitizer and confirmed the guard rejects `record_bytes`
of 32 and 57 (need ≥58 for the label used) while accepting 58, with no
sanitizer report.

The historical command-by-command output for the run this record describes was captured in the
omitted `spikes/m2-reservation/RESULTS.md` (with a local, not in git, `out/full-run.log` raw copy). This
record, not those omitted artifacts, is the public evidence authority.

## Experiment 1 — setup

Disposable APFS volume `apfs`, 64 MiB image (`hdiutil create -size 64m -fs
APFS -type SPARSE`), mounted at the omitted historical run's
`spikes/m2-reservation/out/mnt-apfs`.

```
$ diskutil info out/mnt-apfs | grep -E "File System Personality|Device Identifier"
   Device Identifier:         disk6s1
   File System Personality:   APFS
$ df -k out/mnt-apfs
Filesystem   1024-blocks Used Available Capacity iused  ifree %iused  Mounted on
/dev/disk6s1       65496   28     64688     1%       2 646880    0%   out/mnt-apfs
```

`chainA.jsonl` opened `O_CREAT|O_RDWR|O_NOFOLLOW|O_CLOEXEC`, mode 0600.
Seeded with one JSONL line (`{"seq":0,"seed":true}\n`, 22 bytes):

| | st_size | st_blocks | alloc bytes (blocks×512) |
|---|---|---|---|
| before seed | 0 | 0 | 0 |
| after seed | 22 | 8 | 4,096 |

## Experiment 2 — reservation behind logical EOF, no ftruncate

Requested 22,020,096 bytes (21 MiB = 20 MiB payload budget + 1 MiB terminal
budget, modeling the production 10 GiB + 1 MiB terminal-allowance shape at
probe scale) via `fcntl(fd, F_PREALLOCATE, &fs)` with `fst_flags =
F_ALLOCATEALL|F_ALLOCATEPERSIST` (`0xc` — the qualified recipe, used here
from the start as of fix round 2), `fst_posmode = F_PEOFPOSMODE`,
`fst_offset = 0`, `fst_length = 22020096`. No `ftruncate` call anywhere in
the probe.

| | st_size | st_blocks | alloc bytes | fst_bytesalloc | flags | rc |
|---|---|---|---|---|---|---|
| before | 22 | 8 | 4,096 | — | — | — |
| after | **22 (unchanged)** | 43,016 | 22,024,192 | 22,020,096 (== requested) | 0xc | 0 |

`22,024,192 = 4,096 (pre-existing seed allocation) + 22,020,096 (new
reservation)` exactly. `df` on the volume dropped from `Available 64688` to
`Available 43180` (1 KiB units) — a ~21 MiB real drop, confirming this is
physical allocation, not sparse accounting. Logical EOF (`st_size`) never
moved from 22. A second, independent entry `chainB.jsonl` was reserved for
5,242,880 bytes the same way, also with `0xc` (`st_blocks` 8 → 10,248 =
4,096 + 5,242,880; `df` dropped a further ~5,124 KiB to `Available 38056`).
**PASS**: reservation is consumed by real blocks, behind logical EOF,
without extending it, `F_ALLOCATEALL` reports the full requested amount
rather than a partial grant, and this is measured on the exact flag
combination (`0xc`) T03R is told to use.

## Experiment 3 — persistence: qualified recipe, measured against a real remount, on the files that are actually used

**Two complementary remount measurements**, both against a real `hdiutil
detach` + `attach` of the same backing image at the same mountpoint (not a
same-mount fresh process, which cannot distinguish a durable on-disk
reservation from a vnode the kernel simply hasn't reclaimed yet):

**(a) Dedicated comparison volume**, isolating the flag as the only
variable, on a 16 MiB APFS volume with four independently reserved files
(each 2,097,152 bytes behind a 22-byte seed):

| File | Flags | Baseline (same mount) | After remount 1 | After remount 2 |
|---|---|---|---|---|
| `persist_plain.jsonl` | `F_ALLOCATEALL` (0x4) | 4,104 blocks | **8 blocks (LOST)** | **8 blocks (LOST)** |
| `persist_flag.jsonl` | `F_ALLOCATEALL\|F_ALLOCATEPERSIST` (0xc) | 4,104 blocks | 4,104 blocks (retained) | 4,104 blocks (retained) |
| `killplain.jsonl` (child `SIGKILL`ed, fd open) | 0x4 | 4,104 blocks | **8 blocks (LOST)** | **8 blocks (LOST)** |
| `killpersist.jsonl` (child `SIGKILL`ed, fd open) | 0xc | 4,104 blocks | 4,104 blocks (retained) | 4,104 blocks (retained) |

`waitpid`/`WIFSIGNALED` confirmed both children actually died by `SIGKILL`
(signal 9) with the fd still open, before either remount. `df` independently
confirms the real space returned: `Used` on this volume dropped from 8,236
KiB (baseline) to 4,156 KiB after remount 1 and 4,168 KiB after remount 2 —
consistent with exactly the two plain (non-persist) reservations (≈4,096
KiB combined) being physically released, while the two persist-flag
reservations (≈4,096 KiB combined) remain charged in both cases.

**(b) The actual experiment volume — fix round 2's addition.** This is the
piece fix round 1 was missing: chainA and chainB, the two files that
Experiments 4 and 5 below actually exhaust, append to, and drive to a
terminal record, are reserved with `0xc` and are themselves put through one
real remount, immediately after reserving and before any fill or append:

```
RESULT step=reserve_step2_chainA_fcntl requested=22020096 bytesalloc=22020096 flags=0xc rc=0
RESULT step=reserve_chainB_fcntl requested=5242880 bytesalloc=5242880 flags=0xc rc=0
-- remount the main volume --
RESULT step=stat path=.../chainA.jsonl size=22 blocks=43016 alloc_bytes=22024192   # fresh process, after a real remount
RESULT step=stat path=.../chainB.jsonl size=22 blocks=10248 alloc_bytes=5246976    # fresh process, after a real remount
```

Both files retain their full reservation across the remount. Every
subsequent measurement in Experiments 4 and 5 — exhaustion, payload,
terminal record, the over-budget attempt, chainB's independent append — is
therefore performed on a reservation that has already been proven, on this
exact volume and these exact files, to survive a real unmount/remount, not
inferred from the separate comparison volume in (a).

**Finding: `F_ALLOCATEALL` alone does not survive a real unmount/remount on
APFS. Only `F_ALLOCATEALL|F_ALLOCATEPERSIST` does, confirmed across two
independent remounts on a dedicated comparison volume (including after a
`SIGKILL`ed child) and once more on the actual files the rest of this report
exercises.**

## Experiment 4 — real exhaustion, then bounded append + terminal record within its protected budget

**Real exhaustion, not just the first `ENOSPC`.** With `chainA` (21 MiB
reserved, `0xc`, remount-confirmed) and `chainB` (5 MiB reserved, `0xc`,
remount-confirmed) both live on the 64 MiB volume, a competing writer filled
with decreasing chunk sizes:

| Fill pass | Chunk size | Bytes written before failure |
|---|---|---|
| junk1 | 1 MiB | 36,700,160 |
| junk2 | 64 KiB | 393,216 |
| junk3 | 4 KiB | 57,344 |

After these three passes, `df` reported `Available 1768` KiB, `Capacity
98%`. **Exhaustion was then confirmed directly, not assumed**: opening a
brand-new small metadata-style file (`registry.json`) failed immediately
with `errno=28` (`ENOSPC`) on its very first `open()` call — the same
measurement doubles as the metadata-write-honesty check the packet also
asks for (Experiment 6).

**Payload sized to the reservation, terminal record written into the actual
protected tail.** `chainA`'s total reservation is 22,020,096 bytes =
20,971,520 bytes payload budget (20 MiB) + 1,048,576 bytes terminal budget
(1 MiB). With the volume already confirmed exhausted:

| Step | Records | Bytes/record | Committed | Resulting st_size | st_blocks (unchanged throughout) |
|---|---|---|---|---|---|
| payload | 20,480 | 1,024 | 20,480 / 20,480 | 20,971,542 | 43,016 |
| terminal | 1,310 | 800 | 1,310 / 1,310 | 22,019,542 | 43,016 |

Every one of 21,790 writes succeeded with `fsync` per group (1,000 for
payload, 200 for terminal), on a volume where a plain new file could not
even be opened. `st_blocks` never moved from the 43,016 set at reservation
time — the appends consumed already-reserved blocks, not new ones.

**Percentage of the reservation actually spent (corrected).** The
22,020,096-byte reservation was granted starting from the file's physical
end of file at the time of the call, which already included the seed's own
4,096-byte block. The bytes actually consumed by subsequent logical growth
are `22,019,542 (final st_size) − 4,096 (the seed's pre-existing block) =
22,015,446 bytes`, which is `22,015,446 / 22,020,096 = 99.979%` of the
22,020,096-byte reservation — leaving exactly the 4,650-byte remainder
against the file's full 22,024,192-byte physical allocation
(`22,024,192 − 22,019,542 = 4,650`). (The original fix-round-1 report
computed this as `22,019,520 / 22,020,096 = 99.997%`, which counted the
4,074 bytes that went into the seed's own pre-existing block as if they were
newly spent from the reservation; that arithmetic was wrong and is corrected
here.) **PASS**: the reservation, held under the qualified `0xc` recipe and
confirmed to have survived a real remount, protects real appends — including
a terminal record actually written near the tail of its protected budget —
against a volume independently confirmed exhausted before either write
began.

## Experiment 5 — floor not ceiling, independent entries, allocation failure, reduced reserve, unaligned length, unsupported filesystem

**A reservation is a protected minimum, not a cap — the failure half,
measured by writing past it.** With chainA sitting 4,650 bytes short of its
entire 22,024,192-byte physical allocation on a volume already confirmed
exhausted, one further 8,000-byte record was attempted — large enough to
require a *new* block beyond the 43,016 the file already holds:

```
ERR step=overbudget_chainA_write errno=28 strerror=No space left on device at_record=0
RESULT step=overbudget_chainA_summary committed=0 requested=1 record_bytes=8000 stop_errno=28
RESULT step=overbudget_chainA size=22019542 blocks=43016   # unchanged — no new allocation granted
```

The write failed cleanly with `ENOSPC`, and `st_blocks` stayed at 43,016 —
no partial or silent growth. **This run measures only the failure half**:
growing past a file's own physical allocation on a genuinely exhausted
volume fails cleanly, just like any other write would. The complementary
half — that the same over-budget write would instead *succeed* if the
volume had free space elsewhere — is not a novel claim (it follows directly
from ordinary POSIX write semantics: a write either has somewhere to go or
it doesn't) but it was not separately exercised as its own measurement in
this run, and is not claimed as one here. What's demonstrated instead:
`chainB.jsonl` (its own, separate 5,242,880-byte reservation) received
4,400×1,024=4,505,600 bytes of its own payload (about 86% of its
reservation) immediately after chainA's over-budget attempt, with
`st_blocks` unchanged at 10,248 throughout — chainA's over-budget attempt,
whichever way it had gone, had zero effect on chainB's capacity. **T03R's
own byte-budget accounting is what must enforce each entry's logical limit;
the physical reservation guarantees that budget is actually spendable, it
does not, by itself, stop a software bug from writing past it if the volume
happens to have free space elsewhere.**

**Allocation failure (preflight refusal).** On a fresh, otherwise-empty 16
MiB APFS volume (`apfs2`), requesting 104,857,600 bytes (100 MiB, far more
than free) with the qualified `0xc` flags:

```
ERR step=reserve_toolarge_chainF_fcntl errno=28 strerror=No space left on device requested=104857600 flags=0xc
RESULT step=reserve_after_fail size=22 blocks=8 alloc_bytes=4096   # unchanged from before the call
```

`fcntl` returns -1/`ENOSPC` and the file's allocation is provably unchanged
(still just the 4,096-byte seed allocation) — true all-or-nothing refusal,
no partial grant leaked. **PASS**.

**Reduced reserve — an interface trap, not a shrink.** On the same volume,
`chainG.jsonl` was reserved for 2,097,152 bytes with `0xc` (`st_blocks` →
4,104 / 2,101,248 bytes), then `F_PREALLOCATE` was called again on the
*same file* requesting a **smaller** 524,288-byte `fst_length`, also `0xc`:

```
RESULT step=reserve_chainG_reduced_512kib_fcntl requested=524288 bytesalloc=524288 flags=0xc rc=0
RESULT step=reserve_after size=22 blocks=5128 alloc_bytes=2625536
```

`2,625,536 = 2,101,248 + 524,288` exactly — the second call **added**
524,288 more bytes on top of the existing reservation. Per `man 2 fcntl`,
`F_PEOFPOSMODE` allocates "from the physical end of file" and `fst_length`
is "the number of newly allocated bytes desired" — this is documented,
additive-by-design behavior, but it is exactly the kind of thing an
implementer would get wrong without reading the man page or measuring it:
**repeat `F_PREALLOCATE` calls with `F_PEOFPOSMODE` are additive from the
current physical end of file, not "set total reservation to N."**

**Non-block-aligned reservation length.** On the same volume, `chainH.jsonl`
was reserved for 12,411 bytes (= 4,096×3 + 123, deliberately unaligned),
with `0xc`:

```
RESULT step=reserve_chainH_unaligned_first_fcntl requested=12411 bytesalloc=12411 flags=0xc rc=0
RESULT step=reserve_after size=22 blocks=40 alloc_bytes=20480
```

`st_blocks` grew by 32 blocks (16,384 bytes: `ceil(12411/4096)=4` blocks) —
the kernel rounds the *real* allocation up to the next 4,096-byte boundary —
but **`fst_bytesalloc` reported back exactly `12411`, the requested byte
count, not the rounded 16,384 actually allocated.** A second, unaligned
repeat call for 777 more bytes (`0xc`) showed the same pattern:
`fst_bytesalloc=777` (echoing the request), but `st_blocks` grew by exactly
one more 4,096-byte block (40 → 48 blocks). **`fst_bytesalloc` is not a
reliable measure of real physical consumption on an unaligned request — it
echoes the requested length back on success, not the rounded allocation.
T03R must always derive real consumption from `st_blocks × 512`, never from
`fst_bytesalloc`,** especially when accumulating a running total across
repeated, possibly-unaligned reservation calls.

**Unsupported filesystem/platform — HFS+ (with real remount).** Disposable
16 MiB HFS+ volume, `File System Personality: HFS+` confirmed, `statfs`
`f_fstypename="hfs"` (`f_type=25`).

```
RESULT step=reserve_hfs_fcntl requested=1048576 bytesalloc=1048576 flags=0x4 rc=0
RESULT step=reserve_after blocks=2056       # same process, right after the call
RESULT step=stat blocks=8                   # FRESH PROCESS, same mount, right after
```

Repeated with `F_ALLOCATEPERSIST` set: identical outcome (`rc=0`, full
allocation reported in-process, then gone on fresh-process reopen — *before
any remount at all*). An explicit remount afterward confirms both stay
gone (`st_blocks=8` for both files). **FAIL for HFS+, on both mechanisms
available**: the allocation does not survive even a same-mount last-close,
let alone a remount, and no errno ever signals it.

**Unsupported filesystem/platform — FAT32 (with real remount).** Disposable
64 MiB "MS-DOS FAT32" volume (FAT32 needs ≥~32 MiB; a 16 MiB image fails
formatting with `newfs_msdos: FAT32 is impossible for disk size of
16383KiB`), `statfs` `f_fstypename="msdos"` (`f_type=24`).

```
RESULT step=reserve_fat_persist_fcntl requested=1048576 bytesalloc=1048576 flags=0xc rc=0
RESULT step=reserve_after blocks=2049
RESULT step=stat blocks=2049                # fresh process, SAME mount: appears to persist
-- remount --
RESULT step=stat blocks=1                   # after a REAL remount: gone
```

Unlike HFS+, the reported block count *does* survive a same-mount
fresh-process reopen — but a genuine unmount/remount shows this was also
never durable: `st_blocks` drops from 2,049 back to 1 (the bare 512-byte
seed allocation), just like HFS+, only visible with the stronger test. A
subsequent 1,024-byte append into the now-unreserved file succeeded as an
ordinary write (`committed=1/1`, no error). A different, earlier run (the
original `fa9ff15` submission, superseded by this report) had observed the
same append failing with `errno=22` (`EINVAL`) when performed in the *same*
mount session immediately after `F_PREALLOCATE`, without an intervening
remount; that observation is not re-verified in the committed run this
report describes and is mentioned only as historical context. **FAIL for
FAT32**: the reservation does not survive a real remount, on the one
mechanism measured against it.

Both unsupported-filesystem results reinforce the same point from two
different angles: **a `rc==0` from `F_PREALLOCATE` is not sufficient
evidence of a working reservation on any filesystem** — only APFS, measured
end-to-end with the persist flag and against a real remount on the actual
files exercised, qualifies.

## Filesystem-type gate values

T03R's own packet says to gate reservation on filesystem type via `statfs`.
The probe calls `statfs(2)` directly (not the shell-level `diskutil info`
label) and quotes the literal bytes production Rust code would see:

| Volume | `diskutil` "File System Personality" | `statfs` `f_fstypename` | `f_type` |
|---|---|---|---|
| APFS (`apfs`, `apfs2`, `persist`) | APFS | `apfs` | 26 |
| HFS+ (`hfs`) | HFS+ | `hfs` | 25 |
| FAT32 (`fat`) | MS-DOS FAT32 | `msdos` | 24 |

Note the mismatches with the human-readable `diskutil` labels: HFS+ reports
`f_fstypename="hfs"`, not `"hfs+"`; FAT32 reports `"msdos"`, not `"fat32"` or
`"vfat"`. T03R's gate must compare against these exact lowercase strings,
not a guessed spelling derived from the `diskutil`/Finder-facing name.

## Experiment 6 — parsing, metadata/registry honesty, cleanup and permissions

**Parsing verification**, independent of the probe's own writer (Python's
`json.loads` per line, not our C code):

```
RESULT step=parse_check path=out/mnt-apfs/chainA.jsonl lines=21791 bytes=22019542 all_valid_json=true trailing_newline=true
RESULT step=parse_check path=out/mnt-apfs/chainB.jsonl lines=4401 bytes=4505622 all_valid_json=true trailing_newline=true
```

21,791 = 1 seed line + 20,480 payload + 1,310 terminal records for chainA;
4,401 = 1 seed line + 4,400 payload for chainB. Every line parses as valid
JSON and the file ends with a newline. **PASS**.

**Metadata/registry writes are reported honestly.** As described in
Experiment 4, the metadata-write check and the exhaustion-confirmation check
are the same measurement: `registry.json`'s `open()` failed with `errno=28`
(`ENOSPC`) on the very first attempt, immediately after the competing
writer's three fill passes, with `df` still reporting `1768` KiB
"available." `df`'s "Available" figure is not a reliable predictor of
whether a given write will succeed — a plain metadata write failed with
`ENOSPC` while `df` reported over a megabyte free, because APFS reserves
some of that for its own bookkeeping and cannot actually hand it to a new
file. T03/T03R should never gate a decision on a free-space number; attempt
the write and handle its real result. **PASS** as an honesty check.

**Unlink/close/reopen cleanup and retained-file permissions.** Both
`chainA.jsonl` and `chainB.jsonl` were `0600` for their entire lifetime
(verified at every `stat`/`seed`/`reserve` step above) and immediately
before unlink:

```
RESULT step=unlink_before_perms path=.../chainA.jsonl mode=0600 size=22019542 blocks=43016
RESULT step=unlink_ok path=.../chainA.jsonl
RESULT step=unlink_before_perms path=.../chainB.jsonl mode=0600 size=4505622 blocks=10248
RESULT step=unlink_ok path=.../chainB.jsonl
```

Both files gone afterward (`ls` → "No such file or directory" for both), and
the volume's free space rose back from 1,768 KiB to 28,400 KiB — confirming
the physically reserved blocks were returned to the filesystem on unlink,
not leaked. **PASS**.

## Cleanup and disposable-volume safety

Every volume created by the omitted historical `run.sh` this round (`apfs` 64 MiB, `persist` 16
MiB, `apfs2` 16 MiB, `hfs` 16 MiB, `fat` 64 MiB) was detached and its backing
`.sparseimage` deleted by the script's own `EXIT` trap. Verified after the
run: `mount | grep spikes/m2-reservation` → no matches; `ls
spikes/m2-reservation/out/*.sparseimage` → no matches; `diskutil list`
restricted to this run's volumes → no matches. The host disk's free space
was unaffected (~528 GiB free before and after, consistent with prior
rounds' baselines). The device/size safety check (`verify_disposable_mount`)
now runs after every `hdiutil attach` in the script, including each of the
five `remount_volume` re-attaches performed this round (main volume once,
persistence volume twice, HFS+ once, FAT32 once) — not only the first
attach of a run, closing the gap a re-review found in the previous round's
guard.

## Interface facts for T03R

1. **Mechanism**: `fcntl(fd, F_PREALLOCATE, &fstore_t{fst_flags:
   F_ALLOCATEALL|F_ALLOCATEPERSIST, fst_posmode: F_PEOFPOSMODE, fst_offset:
   0, fst_length: N})` on an `O_NOFOLLOW|O_CLOEXEC`, mode-0600 fd, never
   followed by `ftruncate`. **`F_ALLOCATEPERSIST` is required, not
   optional** — plain `F_ALLOCATEALL` is released on a real unmount/remount.
   Qualified on APFS only, and qualified on the exact files that were
   exhausted, appended to, and driven to a terminal record in this report,
   not inferred from a separate comparison volume alone.
2. **Persistence**: survives a fresh process reopening the file on the same
   mount, and a real `hdiutil detach`/`attach` remount (exercised three
   times across this qualification: twice on a dedicated comparison volume,
   once on the actual experiment volume between reserving and appending),
   and abnormal process exit (`SIGKILL`) with the fd open — all only with
   `F_ALLOCATEPERSIST` set. No same-process close-then-reopen-in-one-process
   step is part of this qualification's evidence; every measurement is a
   separate process or a separate mount. Without `F_ALLOCATEPERSIST`, the
   reservation looks intact on the original mount but is silently gone
   after a real remount — an unclean shutdown or reboot is the
   production-relevant case this difference matters for.
3. **Failure signature to check**: `fcntl` returns `-1`; `errno=28`
   (`ENOSPC`) when the request exceeds free space, with `F_ALLOCATEALL`
   guaranteeing no partial grant (verify via `fstat` if in doubt). Measured
   on the qualified `0xc` flag combination, not just plain `F_ALLOCATEALL`.
4. **Repeat calls are additive, not absolute, and `fst_bytesalloc` is not
   the real consumption.** Calling `F_PREALLOCATE` again with a smaller
   `fst_length` adds that many more bytes on top of the current physical
   EOF (documented behavior — see `man 2 fcntl`'s `F_PEOFPOSMODE`
   description). Separately, and not documented as clearly: on an unaligned
   request, `fst_bytesalloc` echoes the requested byte count back verbatim,
   not the real, block-rounded allocation. `reserve_capture` must track
   cumulative reserved bytes from `st_blocks × 512` (measured before/after
   its own calls), never from summing `fst_bytesalloc` values, or its
   internal accounting will silently drift from the real physical
   reservation on any non-block-aligned request. Measured on `0xc`.
5. **Filesystem gate required, exact strings measured.** T03R must confirm
   filesystem type via `statfs(2)`'s `f_fstypename` before reserving, and
   compare against the literal lowercase strings measured here: `"apfs"`
   (proceed), `"hfs"` (refuse — reservation is released at last close, no
   errno), `"msdos"` (refuse — reservation is released on remount). Do not
   guess spellings like `"hfs+"` or `"fat32"` — they do not match what the
   kernel reports. If the participant's home volume is ever something other
   than APFS, T03R should refuse reservation rather than proceed on an
   unverified assumption (this qualification did not attempt to enumerate
   every non-APFS case, only these two).
6. **A reservation is a protected minimum for its file, not a ceiling.**
   Growing a file past its own already-granted physical allocation requires
   a *new* block allocation. Measured directly: an 8,000-byte append past
   chainA's allocation (consumed to within 4,650 bytes of its full
   22,024,192-byte grant) failed with `errno=28` on the confirmed-exhausted
   volume, `st_blocks` unchanged. Only this failure case was measured in
   this run; that the same write would instead succeed against a volume
   with free space elsewhere is ordinary POSIX semantics, not a separately
   verified finding here. **T03R's own byte-budget accounting is the only
   thing that enforces each entry's *logical* allowance** — the physical
   reservation guarantees that budget is actually available to spend, it
   does not by itself prevent a bookkeeping bug from writing past the
   intended budget if the volume has room elsewhere.
7. **`df`/free-space numbers are not a substitute for attempting the write**
   — a metadata write failed with `ENOSPC` while `df` still reported over a
   megabyte free. Any headroom check for registry/ACK/metadata writes should
   attempt the write and handle its real result, not gate on a threshold.
8. **Cost model validated under real, confirmed exhaustion, on the
   qualified recipe**: the reserved region is consumed exactly by later
   appends (`st_blocks` unchanged across 21,790 append calls spanning
   22,015,446 of the 22,020,096-byte reservation — 99.979% — on a volume
   where a new file could not even be opened) — no re-allocation events, no
   fragmentation-driven partial failures observed at this scale.

## Limitations

- Probe scale (16–64 MiB volumes, up to ~22 MiB single-file reservations)
  is far below the production 10 GiB target; the mechanism was validated,
  not a week-long duration or multi-GB fragmentation profile. The packet
  explicitly allows a smaller configurable allocation with identical
  mechanism.
- Only APFS (sparse disk image, default settings) was exercised as the
  "supported" filesystem. Encrypted APFS, Fusion-drive/rotational-backed
  APFS, and network/external APFS volumes were not tested; a real
  participant's home volume is assumed to be a local APFS instance, which
  the tracker's platform layer should confirm (via the measured
  `f_fstypename` gate) rather than assume.
- Two non-APFS filesystems (HFS+, FAT32) were measured as the packet's
  "unsupported filesystem/platform" requirement; other filesystems
  (exFAT, exchange formats, network filesystems) were not tested and should
  not be assumed to behave like either of these two.
- FAT32's `EINVAL`-on-write observation is historical only (from the
  superseded `fa9ff15` run) and is not re-verified against the current
  committed run; do not treat it as a currently-reproduced finding.
- The "a write past a file's own reservation succeeds when the volume has
  free space elsewhere" half of the floor-not-ceiling claim was not
  separately measured in this run; only the failure-under-exhaustion half
  was.
- The remount tests used a clean `hdiutil detach` (not a forced/dirty
  detach or a simulated power loss). An unclean unmount or power loss while
  a descriptor is open was not measured and is not claimed to behave the
  same as the clean remount tested here.
- `F_ALLOCATECONTIG` and `F_VOLPOSMODE` (declared in the SDK header) were
  not exercised — out of scope for "behind current EOF" reservation.
- This is a measurement of the syscall/filesystem contract only; it does
  not implement `reserve_capture` or any part of T03/T03R's production
  code path, per this task's ownership boundary (no Rust, no Cargo).
- The competing-writer and metadata-write experiments used single-threaded,
  single-process contention on a small disposable volume; concurrent
  multi-process contention for the same free-space slack was not tested.
- The probe reopens by path with `open()`, not `openat()` against a held
  directory descriptor (the production "intended capability path" per
  `goalcup-platform`); allocation behavior is per-vnode, so the result
  should transfer, but T03R should re-verify this on its actual open path
  rather than assume it from this probe alone. `O_NOFOLLOW` was never
  exercised against a real symlink in this qualification.
- The allocation-failure request (100 MiB against a 16 MiB volume) is
  larger than the whole volume, not just larger than free space; it
  confirms all-or-nothing refusal but does not distinguish a preflight size
  check from an attempted-and-rolled-back allocation.

## Appendix — T03R implementation evidence (2026-09-28)

Appended by T03R under its report lease (CONTRACTS Wave 2, R15/R16). Everything
above is T01's probe measurement and is unchanged. This section records the
same contract measured through the production code path instead of the C
probe, which also closes T01's `openat` limitation above.

**Production method.** `TraceExport::reserve_capture(bytes)`
(`crates/goalcup-protocol/src/export.rs`) first applies
`configure_capture_quota`'s quota checks. It then calls
`TraceDir::reserve_append` (`crates/goalcup-platform/src/tracecap.rs`) on the
retained `chain.jsonl` append handle. That handle was opened with `openat`
beneath the held trace-directory descriptor, with
`O_RDWR|O_APPEND|O_NOFOLLOW|O_CLOEXEC|O_NONBLOCK`. The name binding is checked
before and after. The platform step
(`crates/goalcup-platform/src/tracecap/unix.rs`) does five things, in order:

1. Refuses unless `fstatfs` on that descriptor reports exactly `"apfs"`.
2. Measures `st_size` and `st_blocks × 512`.
3. Requests only the missing `quota − st_blocks×512`, if any. The request uses
   `F_PREALLOCATE` with `F_ALLOCATEALL|F_ALLOCATEPERSIST`, `F_PEOFPOSMODE` and
   `fst_offset 0`. It never calls `ftruncate`.
4. Remeasures.
5. Succeeds only if `st_size` is unchanged and `st_blocks×512 ≥ quota`.

Only then does `capture_storage().reserved_bytes` report the quota. A missing
amount larger than `f_bavail × f_bsize`, or one not representable as an
`off_t`, is refused before any allocation (`InsufficientSpace`).
`fst_bytesalloc` is never read.

**Environment.** macOS 26.6.2 (25G83), arm64, rustc 1.98.0. The default-gate
tests ran on the host temp directory, `/System/Volumes/Data` (APFS, FileVault
on). The by-hand tests used disposable sparse `hdiutil` images of at most
64 MiB in a private (0700) directory under the crate's `target/tmp`. Each mount
was checked for a distinct device, a size no more than twice the image, and
the expected `f_fstypename` after every attach and before any write. Every
image was force-detached and deleted when its guard dropped, and afterwards
`mount`/`hdiutil info` showed no leftover volume.

**By-hand command (R16, never in the default gate).** Two locks guard it:
`#[ignore]`, and `GOALCUP_T03R_VOLUME=1`, which follows the repository's
`GOALCUP_REGEN` convention. Without the variable, even
`--include-ignored` attaches no image.

```sh
GOALCUP_T03R_VOLUME=1 cargo test -p goalcup-protocol --offline --test capture_reserved_storage -- \
    --ignored --exact --nocapture --test-threads=1 \
    reserved_chains_survive_remount_and_exhaustion unsupported_allocator_never_claims_reserved_bytes
```

### Disposable 64 MiB APFS volume (`reserved_chains_survive_remount_and_exhaustion`)

Allocations are `st_blocks × 512`. "Size" is `st_size`.

| Entry | Quota (bytes) | Size after reserve | Allocated after reserve | Allocated after real remount |
|---|---|---|---|---|
| A | 5,243,003 | 400 | 5,246,976 | 5,246,976 |
| B | 2,097,152 | 400 | 2,097,152 | 2,097,152 |
| C (reserved by a child, then `SIGKILL`ed with the descriptor open) | 1,310,843 | 2,070 | 1,314,816 | 1,314,816 |

- **Remount and reopen.** All three reservations survived a real
  `hdiutil detach`/`attach`. After the remount, `TraceExport::resume` followed
  by `reserve_capture` with the same quota remeasured each chain and added
  nothing: allocations were byte-identical.
- **Exhaustion.** A competing writer wrote 55,705,600 bytes in 3 files. After
  that, creating a new metadata-style file failed with `ENOSPC` (errno 28),
  while `statfs` still reported 1,851,392 bytes available.
- **Entry A on the exhausted volume.** It appended 246 payload segments of
  16 KiB until `CaptureStorageFull { terminal_allowance: false }`, calling
  `sync_capture` (F_FULLFSYNC plus directory fsync) every 32 appends. It then
  appended 2,210 closure gaps, and every append and sync succeeded.
  `write_local_finish` appended the LocalFinish record: `st_size` went from
  5,235,135 to 5,235,754, within the quota of 5,243,003, and the allocation
  stayed 5,246,976. It then failed to create `seal.json` with errno 28, and
  returned that as `Err`, not as success. **The chain reservation protects the
  terminal record, not `seal.json`.**
- **Entry B, independently, on the same exhausted volume.**
  - Payload, a closure and a sync succeeded, and the allocation was unchanged.
  - Growing the reservation beyond the reported free space was refused before
    allocation (`InsufficientSpace`).
  - Growing it by exactly the reported 1,851,392 free bytes reached the kernel,
    which refused it with `ENOSPC` (errno 28). The allocation stayed at
    2,097,152 (all or nothing), and the reported storage was unchanged.
  - B's `write_local_finish`, including `seal.json`, succeeded only after the
    first competing file was deleted. That file held 54,525,952 bytes
    (52 MiB), measured in the fix-round-1 rerun, after which `statfs`
    reported 56,365,056 bytes available. This shows that the seal succeeds
    once ample headroom returns. It does not measure how little headroom a
    seal needs.
- **Entry C** appended a 64 KiB segment and synced on the exhausted volume, and
  its allocation was unchanged.
- **After a final remount**, every chain was complete JSONL, and B and C
  resumed.
- **Mutant check.** With `F_ALLOCATEPERSIST` removed from the production call,
  the same test fails: after the remount, all three allocations dropped to
  4,096 bytes. This reproduces Experiment 3 through the production code.

### HFS+ and FAT32 (`unsupported_allocator_never_claims_reserved_bytes`)

On a 16 MiB HFS+ volume (`f_fstypename` `hfs`) and a 64 MiB FAT32 volume
(`msdos`), `reserve_capture(1,310,843)` returned
`Refused { reason: UnsupportedFileSystem }`. `quota_bytes` and `reserved_bytes`
stayed 0. `chain.jsonl` stayed at `st_size 0`, `st_blocks 0`, and was
unchanged after a real remount.

### Default-gate evidence (host APFS temp directory)

`capture_reserved_storage` runs eight tests, alongside the platform unit tests
beside the implementation:

- **EOF.** Reservation leaves logical EOF unchanged.
- **Top-up, not additive.** Repeated, smaller and larger requests top up to the
  target and are not additive.
- **Remeasure without reallocating.** Close/reopen, a `SIGKILL`ed child and a
  restart each keep the allocation, report nothing reserved until remeasured,
  and remeasure without allocating again.
- **Admission still stops at the quota.** Payload stops in front of the
  terminal allowance, and closures stop at the quota. No byte lands outside
  the reserved blocks.
- **Refusals claim nothing.** An impossible request (2^60 bytes, or
  `u64::MAX`) is refused before any allocation, leaving the trace directory
  byte-identical. A chain length this writer did not write is refused. A
  replaced chain name is refused and poisons the stream.
- **No quota-only fallback.** Fix round 1 added this test. An unqualified
  filesystem and an allocator that grants nothing are forced through the
  platform's debug-only `ReservationFault` seam. The seam is fail-closed: it
  can only force a refusal. Both are refused, as `UnsupportedFileSystem` and
  `AllocationNotRetained`, with no quota, no reported reservation and no v5
  selection. An already-reserved chain keeps its measured budget. A mutant
  that falls back to quota-only on either refusal fails this test. Before
  this test, the export-level unsupported-filesystem refusal on macOS was
  covered only by the by-hand HFS+/FAT32 run.

### Pilot-supported allocation scope

Pilot storage preflight is qualified only in this configuration:

- **Platform and filesystem.** macOS, with the chain file on a volume whose
  `statfs` `f_fstypename` is exactly `apfs`.
- **Recipe.** The recipe above, measured by `st_blocks × 512 ≥ quota` with
  `st_size` unchanged.
- **Reservation size.** The reserved quota includes the 1 MiB terminal
  allowance, never on top of it.
- **After every reopen.** `reserve_capture` is repeated after every reopen.
  Until then nothing is reported reserved.

The following are refused:

- HFS+ and FAT32 (measured).
- Every other filesystem name (unmeasured, refused by name).
- Every other platform. The non-macOS path is written to always return
  `UnsupportedFileSystem`. It was reviewed by eye, not compiled: no Linux
  target is installed here.

`configure_capture_quota` alone remains the explicitly unreserved experiment
(`reserved_bytes 0`).

**Not covered by this evidence:**

- Headroom for `seal.json`, the registry, ACKs and harness output (see entry A
  above).
- Power loss or a forced, dirty detach. `SIGKILL` and clean remounts are not
  power-loss proof.
- APFS local snapshots taken after reserving. Copy-on-write could then need
  new blocks for writes into the reserved range; this was not measured.
- APFS variants other than the host data volume and sparse images, such as
  external, network or Fusion volumes.
- Multi-GB scale.
- Concurrent multi-process contention.
