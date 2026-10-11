# Platform portability seams

What does Goal Cup assume about macOS and Unix, and what is the smallest change at each seam so
Linux and Windows can be added later without a rewrite? For maintainers planning V1.1 platform
work and reviewing new harness code. Updated: 2026-10-11.

This is a design note, not a plan of record and not evidence. Nothing here is qualified or
measured unless it links a record. Platform scope is set by [D42](../DECISIONS.md) (macOS
supported, Linux best-effort, Windows planned); this note does not change it. Code is named by
symbol; file paths are relative to `crates/` unless they start with another top-level directory.

## Summary

- **Linux builds and most primitives already have Linux arms.** The one hard blocker for a live
  capture is the chain reservation: `AppendFileHandle::reserve` refuses everywhere but macOS
  APFS, so every `goalcup start --goal-file` is refused on Linux.
- **Windows does not build.** `goalcup-platform` has a `compile_error!` on every non-Unix target.
  The `cfg(not(unix))` stubs elsewhere are the shape of the work, not the work.
- **The v5 records carry no platform field and no platform-shaped value that needs one.** Every
  seam below can be filled without a new record field. Two values are worth knowing:
  `TerminationMethodV1::JobObjectClose` and `GracefulRequest` already exist, so a Windows job
  object fits the frozen format; and the reservation is not recorded at all (see seam 1).
- **One harness-contract question must be decided before more connectors land:** the single
  `[capture.executable].sha256` per connector cannot pin a second platform or architecture.

## Seams

Each seam gives: where it lives, the current assumption, Linux status, Windows status, the
portable interface (existing or missing), the smallest next step, and the risk to the v5 freeze
([ARCHITECTURE §6](../ARCHITECTURE.md#6-chain-records-and-verdicts), "Freeze").

### 1. Physical disk reservation

- **Where.** `goalcup-platform` `tracecap::TraceDir::reserve_append`, implemented by
  `tracecap/unix.rs` `AppendFileHandle::reserve`; called through
  `goalcup-protocol` `TraceExport::reserve_capture` from the daemon's `open_capture`
  (`goalcup-daemon` `runtime/capture/mod.rs`).
- **Assumption.** `fcntl(F_PREALLOCATE)` with `F_ALLOCATEALL | F_ALLOCATEPERSIST` and
  `F_PEOFPOSMODE`, on a volume whose `f_fstypename` is exactly `apfs` (`QUALIFIED_ALLOCATOR`),
  with `st_blocks * 512` as the evidence. Qualified in
  [T01](../qualification/m2/reservation.md). D37 makes real allocation mandatory for an
  installed launch; D40 defers release because APFS frees blocks only by grow-then-shrink (H-11).
- **Linux.** Refused: the `cfg(not(target_os = "macos"))` arm returns `UnsupportedFileSystem`.
  This is why `capture_lifecycle.rs`, `m2_cli_lifecycle.rs`, `harness_contract_kit.rs` and the
  other `#![cfg(target_os = "macos")]` capture tests say "macOS only".
- **Windows.** No implementation; the crate does not compile.
- **Portable interface.** Present: `reserve` is already a per-OS method returning
  `AppendAllocation` or a typed refusal. Nothing above it needs to change.
- **Smallest next step (Linux).** A `cfg(target_os = "linux")` `reserve` using
  `fallocate(fd, FALLOC_FL_KEEP_SIZE, st_size, missing)`, gated by an allowlist of `fstatfs`
  `f_type` magics, starting with ext4 and XFS, and keeping the same remeasure check
  (`logical_bytes` unchanged, `st_blocks * 512 >= total`). Every other filesystem stays refused.
  Then a Linux platform gate written like T01, before any capacity claim.
- **Semantics against D37, D40 and H-11** (each to be measured, none is measured here):
  - `KEEP_SIZE` allocates without moving EOF, which is the D37 shape ("without extending logical
    EOF"). On ext4 and XFS the blocks are unwritten extents that persist across close, SIGKILL
    and remount (believed; the gate must show it, as T01 did for APFS against HFS+ and FAT).
  - btrfs accepts the call but copy-on-write (a snapshot, compression, reflinks) can still need
    new space on write, so it is a weak reservation and should stay refused. ZFS, tmpfs,
    overlayfs, NFS and FUSE backends vary or refuse: keep them out of the first allowlist.
  - Release may be easier than on APFS: a hole punch or a truncate to the current size frees
    blocks beyond EOF without a grow step, so the D40 zero-tail crash window may not exist. That
    could let Linux release at seal while macOS keeps D40's deferral. UNKNOWN until measured
    per filesystem; D40 stays the rule until the owner rules otherwise.
  - D37 also allows explicitly unreserved component experiments. An unreserved Linux mode would
    be the fastest way to a first capture, but see the risk below.
- **Windows step.** `SetFileInformationByHandle(FileAllocationInfo)` sets allocation without
  moving EOF on NTFS. Whether NTFS keeps it after the last handle closes is UNKNOWN (believed not),
  so a Windows gate may need the daemon to hold the handle, which a daemon crash loses.
- **v5 freeze risk.** None for the format: the reservation is in `CaptureStorage` (status, outside
  the freeze) and the control API, not in any record. The real risk is the reverse: because no
  record says whether a chain was reserved, an unreserved trace is indistinguishable from a
  reserved one. Any unreserved mode must be `--dev`-only and must never feed a capacity or
  release claim. Recording it would be a v6 field. Whether `seal.json` could carry it without
  v6 is UNKNOWN.

### 2. Process identity, tree tracking and termination

- **Where.** `goalcup-daemon` `terminate::os::ProcessOs` (the trait) and `RealOs`; `terminate`
  `mod.rs` (`terminate_with`), `authority.rs`, `darwin.rs`; primitives in `goalcup-protocol`
  `process` (`snapshot`, `liveness`, `process_start_identity`); the CLI's `released_child`.
  Contract: [TERMINATION](../TERMINATION.md).
- **Assumption.** Discovery spawns `/bin/ps` by absolute path. Signals are SIGTERM and SIGKILL.
  macOS adds a kqueue fork witness per member and an audit-token singleton authority.
- **Linux.**
  - Built: creation identity from boot ID plus `/proc/<pid>/stat` start tick; instance-bound
    delivery through `pidfd_open` and `pidfd_send_signal` (`PidFd`), so `all_gone` is reachable
    ([2026-09-16 Linux arm](../qualification/2026-09-16-linux-arm.md)).
  - Gap: no fork witness (`fork_witness` returns `None`). A process forked and orphaned between
    two snapshots is never seen. The test
    `a_report_does_not_claim_the_harness_gone_while_a_post_root_death_orphan_lives`
    (`goalcup-daemon/tests/process_kill.rs`) fails on Linux: the report says `observed_gone` and
    `all_gone` while the orphan runs. Re-run alone at `6befa1f` on Linux 6.18 while writing
    this note (a local run, no committed record): it failed with three pids targeted and the
    orphan not among them. This is inside TERMINATION's stated Linux window `W`, and whether
    `W` is acceptable is an open owner ruling
    ([TERMINATION, OPEN](../TERMINATION.md#open-an-owner-ruling-not-a-defect--how-wide-is-linuxs-w),
    [2026-09-18 record](../qualification/2026-09-18-f0-linux-residual.md)). Not fixed here.
  - Gap: `/bin/ps` is absent on minimal images (no procps). Then every snapshot is an error and
    the seal binds `Unresolved(ProcessAuthorityUnavailable)`: fail-closed, but useless.
- **Windows.** `UnsupportedPlatform` from the termination path. No `ps`, no signals, no
  process groups in the Unix sense.
- **Portable interface.** Present: `ProcessOs` already abstracts snapshot, liveness, identity,
  `bind` (instance-bound target), `fork_witness`, `instance_authority`, `kill` and `killpg`.
  A new platform is a new `ProcessOs` implementation plus a TERMINATION section.
- **Smallest next steps.**
  - Linux snapshot: read `/proc` directly instead of `/bin/ps`, behind the same `snapshot`
    signature. Size S. Removes the procps dependency.
  - Linux orphan gap, in order of cost: (a) the owner picks answer 1 in TERMINATION (say plainly
    that Linux `Confirmed` excludes fork-and-leave wrappers), no code; (b) make the launcher a
    child subreaper (`prctl(PR_SET_CHILD_SUBREAPER)`) so orphans reparent to it instead of pid 1
    and stay visible to a walk rooted there; residual if the CLI dies first; (c) run each capture
    in its own cgroup v2 (for example a systemd user scope), whose `cgroup.procs` lists every
    descendant and whose `cgroup.kill` ends them all; this closes the window but needs a
    delegated cgroup, which is UNKNOWN on many hosts.
  - Windows: a `ProcessOs` over a Job Object. Assign the child before it runs, enumerate with
    `JobObjectBasicProcessIdList`, end with `TerminateJobObject`. A job contains every descendant
    unless breakaway is allowed, so the orphan window does not exist. The graceful step is
    `CTRL_BREAK_EVENT` to a new process group, or none; UNKNOWN which harnesses honour it.
- **v5 freeze risk.** None. `TerminationMethodV1` already has `JobObjectClose` and
  `GracefulRequest`, and `TerminationFailureV1` already has `UnsupportedPlatform` (both in
  `goalcup-protocol` `provenance.rs`). `ProcessHandle.process_start` is an opaque string
  that only reaches a record through `process_handoff_digest`, so a Windows identity format
  (creation time plus pid) changes a value, not a shape.

### 3. Spawn gate

- **Where.** `goalcup-cli` `launch::spawn_gated` and `GatedSpawn` (D40, CONTRACTS C6 gate).
- **Assumption.** `fork`, a Unix socketpair, and a `pre_exec` closure that parks until the
  launcher writes `GATE_RELEASE`.
- **Linux.** Works (shared Unix code).
- **Windows.** No `fork` or `pre_exec`. The equivalent is `CreateProcess` with
  `CREATE_SUSPENDED`, job assignment, durable registration, then `ResumeThread`. Note that a
  suspended Windows process has already loaded its image, unlike a parked Unix child.
- **Portable interface.** Missing: `GatedSpawn` is `cfg(unix)` with no trait. Smallest step: a
  `GatedSpawn` trait (`pid`, `release`, `abort`) with the Unix body as its first implementation.
- **v5 freeze risk.** None.

### 4. File identity, file type and no-follow opens

- **Where.** `goalcup-platform` `filecap`, `readcap`, `sourcecap` (`sourcecap/unix.rs`),
  `tracecap` (`tracecap/unix.rs`); `goalcup-cli` `launch::open_no_follow` (the executable re-open
  in the spawn gate); `per_run_home` private directory checks.
- **Assumption.** `openat` relative to a held directory descriptor with
  `O_NOFOLLOW | O_DIRECTORY | O_CLOEXEC | O_NONBLOCK`; `st_mode & S_IFMT` for file type;
  `(st_dev, st_ino)` as identity (plus birth time on macOS); `st_nlink == 1`; exclusive rename;
  mount identity for the source root.
- **Linux.** Built. `rename_exclusive` uses `renameat2(RENAME_NOREPLACE)` and refuses
  `ENOSYS`, `EINVAL` and `EOPNOTSUPP` as `UnsupportedFileSystem`; `mount_identity` uses
  `statx(STATX_MNT_ID)` (kernel 5.8 or later, else `UnsupportedMountBinding`). Unit tests run on
  Linux CI; the end-to-end capture tests do not (seam 1).
- **Windows.** Missing, and one stub is dangerous: `open_no_follow` on `cfg(not(unix))` opens
  without any flag, so it would follow a reparse point silently. `fsutil::create_new` says
  openly that it does not protect the file. Both are unreachable today only because
  `goalcup-platform` refuses to compile.
- **Portable interface.** Partly present: the three capability types are the interface, and
  their `unix.rs` files are the cfg-split implementations. A `windows.rs` beside each would use
  handle-relative opens (`NtCreateFile` with a root handle), `FILE_FLAG_OPEN_REPARSE_POINT`,
  `FILE_ID_INFO` (volume serial plus 128-bit file id) for identity, and `FileRenameInfoEx`
  without replace for the exclusive rename.
- **Smallest next step.** Make every `cfg(not(unix))` open path that today opens without
  protection refuse by name (`UnsupportedPlatform`), so a later Windows build cannot compile into
  a silent weakening. Size S.
- **v5 freeze risk.** None. The source identity token (`journal_token`) is in memory in M2
  native capture; no v5 record holds an inode, device or mode.

### 5. Per-run home and the keychain link

- **Where.** `goalcup-protocol` `harness::RunHome` (`empty`, `keychain_link`) and `SeedMode`
  (`0600`, `0400`); `goalcup-cli` `commands/start.rs` `RunDir::create`, `per_run_home`
  (`create_per_run_home`, `refuse_synced_folder_under`, `icloud_folder_synced`); D38, D39.
- **Assumption.** `HOME` is what a harness and its tools use for user state. Claude Code's
  subscription login is in the macOS Keychain, reached through `Library/Keychains`. Private means
  mode `0700` or `0600` and owner equal to the effective UID. Synced folders are iCloud and File
  Provider paths under `Library/`.
- **Linux.** `HOME` is the right variable, and the capture environment is built from nothing
  (`build_capture_launch`), so no inherited `XDG_*` can point a harness at the real
  `~/.config` (and `capture_policy` refuses `XDG_*` in a connector). But `keychain_link` makes a
  dangling `Library/Keychains` link on Linux. Claude Code on Linux is believed to keep its login
  in a credentials file under `CLAUDE_CONFIG_DIR`, which the shared config home already carries;
  UNKNOWN until measured. Synced-folder detection knows no Linux clients (Dropbox, Nextcloud).
- **Windows.** Harnesses and tool children read `USERPROFILE`, `APPDATA` and `LOCALAPPDATA`, not
  `HOME`. Setting `HOME` alone would let `git`, `npm` or a harness write into the real profile,
  which breaks the "never write app-wide config" rule in [AGENTS](../../AGENTS.md). Modes map to
  ACLs.
- **Portable interface.** The schema is the interface: `run_home` names what the run needs, not
  how. Smallest step: define `keychain_link` as "the platform credential store is reachable"
  and refuse it off macOS until a platform recipe is measured (fail closed, no schema change).
  On Windows, `RunDir::create` must also set and isolate the profile variables, and the private
  checks become owner-only ACL checks behind one `private_dir` function.
- **v5 freeze risk.** None (no record field). A renamed enum value would be harness schema 2.

### 6. Service manager

- **Where.** `packaging/macos/com.agentcommunity.goalcup.plist`,
  `packaging/macos/install-assisted.sh`, `install_assisted.py`, `uninstall-assisted.sh`.
- **Assumption.** A launchd user agent in `gui/<uid>`, restarted only on crash. No Rust code
  calls `launchctl`; the daemon is a plain foreground process with a `--config` flag and
  SIGTERM handling (`Signals` in `goalcup-daemon` `bin/goalcup-daemon.rs`).
- **Linux.** Nothing shipped. A systemd user unit (`Restart=on-failure`, `ExecStart` with the
  same `--config`) is the direct analogue; running the daemon in a terminal also works.
- **Windows.** Nothing. The daemon's non-Unix `Signals` stops on Ctrl-C only, so a supervisor
  has no polite stop. A per-user logon task or simply a foreground process is enough for a
  pilot; a Windows service runs in another session and is UNKNOWN for a per-user tool.
- **Portable interface.** Present by accident: the daemon does not know who supervises it.
  Smallest step: keep it that way (rule 6 below) and add `packaging/linux/` with a unit
  template only when a Linux installer is in scope. Size S.
- **v5 freeze risk.** None.

### 7. Executable pinning

- **Where.** `connectors/*/harness.toml` `[capture.executable]` (`program`, `version`, `sha256`);
  `goalcup-protocol` `harness::ExecutableSpec` (`deny_unknown_fields`); the spawn gate re-hash.
- **Assumption.** One SHA-256 per connector, and the manifest does not say which target it
  was measured on. The Codex pin is the `aarch64-apple-darwin` native binary
  ([launch recipes](../qualification/m2/launch-recipes.md)); Claude Code also ships per-target
  binaries. So a pin matches one platform and one architecture, and an Intel Mac is believed
  not to match either. Whether Hermes's entry point is the same bytes on every install is
  UNKNOWN.
- **Linux and Windows.** Every pinned connector is refused, because the bytes differ. A `--dev`
  development connector with its own pin is the only route today, and it reads as
  `Dev/unpinned` (OS-15).
- **Portable interface.** Missing. Two shapes:
  - a per-target map, for example `[capture.executable.sha256]` keyed by
    `aarch64-apple-darwin`, `x86_64-unknown-linux-gnu` and so on. This is a harness-contract
    schema change: an older `goalcup` refuses the new key (`deny_unknown_fields`, `NEWER_SENTENCE`),
    so it is `schema_version = 2` once v0.5.0 is tagged;
  - or one connector directory per target (`claude-code-cli` and `claude-code-cli-linux`). No
    schema change, but duplicated recipes and pins.
- **Smallest next step.** Decide the shape now (see "Decide now"), before Amp's connector is
  written against schema 1. Size S to decide, M to build with tests and the conformance kit.
- **v5 freeze risk.** None. `Launch.profile_digest` and `connector_dir_sha256` are digests of the
  connector bytes; a bigger manifest changes their values, not the record shape.

### 8. Path and environment conventions

- **Where.** `goalcup-platform` `config_dir_with` and `Platform`; `goalcup-cli` `config`;
  `goalcup-mcp` main; `launch::CAPTURE_PATH` (`/usr/bin:/bin:/usr/sbin:/sbin`),
  `CAPTURE_IDENTITY_ENV` (`USER`, `LOGNAME`, `TMPDIR`); `capture_policy::SHELL_VALUES`.
- **Assumption.** Config paths are already per platform: `~/Library/Application Support`,
  `$XDG_CONFIG_HOME` or `~/.config`, `%LOCALAPPDATA%` or `%USERPROFILE%\AppData\Local`, tested on
  one host through `Platform`. The capture launch PATH, identity allowlist and `SHELL` values
  are Unix.
- **Linux.** Config paths: done. `CAPTURE_PATH` is fine for self-contained binaries. The shipped
  Claude and Codex connectors set `SHELL = "/bin/zsh"`, which many Linux installs lack; the
  failure would then come inside the run, not at preflight (how each harness reacts is
  UNKNOWN).
- **Windows.** Config paths: done. The capture launch would need `SystemRoot`, `PATHEXT`,
  `COMSPEC`, `TEMP`/`TMP` and the profile variables of seam 5; `SHELL` means nothing to a native
  Windows harness. Workspace paths are already written with `/` in the seal manifest
  (`goalcup-daemon` `workspace`).
- **Portable interface.** Present for config; missing for the capture environment. Smallest
  step: preflight refuses a `SHELL` value whose file is not an executable regular file, naming
  it (no schema change, Size S). Later, a per-platform base environment in
  `build_capture_launch`, keeping "built from nothing" as the rule.
- **v5 freeze risk.** None.

### 9. CA and proxy environment

- **Where.** `capture_policy::CA_REGISTRY` (`CODEX_CA_CERTIFICATE`, `NODE_EXTRA_CA_CERTS`
  extend; `HERMES_CA_BUNDLE` replace); `build_capture_launch` sets `HTTPS_PROXY`,
  `https_proxy`, `NO_PROXY`, `no_proxy`; D41 and D43.
- **Assumption.** Process-scoped variables and a loopback TCP proxy. Nothing is OS-specific in
  the mechanism.
- **Linux.** Expected to work; the same variables are read by the same harness code. UNKNOWN
  until one live Linux capture is recorded. Linux hosts are more often multi-user, and loopback
  ingest is unauthenticated toward the receiver (D37, "Ingest trust"); the single-user
  assumption should be restated for Linux.
- **Windows.** Node documents `NODE_EXTRA_CA_CERTS` on Windows; Codex's and Hermes's variables
  on Windows are UNKNOWN. The CA file's privacy needs an ACL (seam 5).
- **Portable interface.** Present: the registry is data. Smallest step: none before a live run.
- **v5 freeze risk.** None.

### 10. Shell scripts

- **Where.** `scripts/with-deepseek-key.sh`, `scripts/pin-manifests.sh`, `harnesses/install.sh`,
  `packaging/macos/*.sh`, test fixtures under `goalcup-cli/tests/fixtures` and
  `goalcup-daemon/tests/fixtures`.
- **Assumption.** A POSIX `sh` and Unix tools; some scripts are macOS-only by design
  (`packaging/macos`, `uname -s` checks, BSD `stat -f '%u'` in `uninstall-assisted.sh`).
- **Linux.** Fixed in the commit before this note: `with-deepseek-key.sh` read the mode with
  BSD `stat -f %Lp` and its `||` fallback captured GNU's file-system dump ahead of the mode, so
  every 0600 file was refused. It now accepts a form only when it succeeds and prints octal digits
  alone. `loader_script.rs`'s positive control assumed `/bin/sh` is bash (true on macOS); on a
  dash `/bin/sh` it now uses `/bin/bash`. `pin-manifests.sh` already handles `shasum` and
  `sha256sum`.
- **Windows.** None of these run without a POSIX shell. A PowerShell copy of each would be a
  second implementation to keep in step.
- **Portable interface.** Smallest step: move logic, not shells. `pin-manifests.sh` duplicates
  `goalcup_protocol::manifest::connector_dir_sha256`; a small Rust or Python entry point that
  writes the pin tables would serve all three platforms. Keep the key loader POSIX: it exists
  for its shell-level guarantees, and a Windows loader would be a separate design.
- **v5 freeze risk.** None.

### 11. macOS-only tests and CI

- **Where.** `#![cfg(target_os = "macos")]` on ten test files: in `goalcup-cli/tests`
  `capture_crash_campaign.rs`, `capture_failure_campaign.rs`, `harness_contract_kit.rs`,
  `m2_cli_lifecycle.rs`, `m2_hermes_lifecycle.rs`, `m2_synthetic_vertical.rs`; in
  `goalcup-daemon/tests` `capture_deferred_scope.rs`, `capture_finish_budget.rs`,
  `capture_harness_load.rs`, `capture_lifecycle.rs`. Item-level macOS gates in
  `goalcup-protocol/tests/capture_reserved_storage.rs`, in `goalcup-cli/tests`
  `status_views.rs` and `hermes_per_run_home.rs`, and in `goalcup-daemon/tests`
  `harness_contract_golden.rs` (the only one of the three `harness_contract_golden.rs` files
  with macOS gates) and `process_kill.rs`. CI: `.github/workflows/ci.yml`, one
  `ubuntu-24.04` job that skips the daemon and CLI suites.
- **Assumption.** The capture tests are macOS-only because of seam 1, not because of their own
  logic. The CI is lean by owner rule ([OPERATING, CI](../OPERATING.md#ci)): no matrix, no
  daemon or CLI suites.
- **Linux.** On this branch, `python3 -m unittest discover -s scripts` is green on Linux
  (`test_install_assisted.py` is skipped off macOS), and `cargo test -p goalcup-cli --test
  loader_script` passes on Linux. Neither is in CI.
- **Windows.** No CI; nothing builds.
- **Smallest next step.** After seam 1 lands, widen the ten gates to
  `any(target_os = "macos", target_os = "linux")` one file at a time and fix what fails. Ask
  the owner before adding any CI step; the cheapest candidates are the scripts unittest run and
  the single `loader_script` target, which spawn no process trees.
- **v5 freeze risk.** None. The reader corpus (`goalcup-verifier/tests/fixtures/v5-corpus/`) is
  platform-free and should stay the guard on every platform.

## Roadmap

Sizes: S is a day or less, M is a few days with tests, L is a week or more or needs a design.

### Linux first: one harness live-capturable

Hermes is the closest: no keychain, `run_home = "empty"`, a per-run config home, no `SHELL` in
its connector, and a key loader that now works on Linux. Codex is second (`/bin/zsh`).

1. **Linux reservation (M).** `fallocate(FALLOC_FL_KEEP_SIZE)` behind an ext4 and XFS
   allowlist, plus a written Linux gate like T01 (persistence, SIGKILL, remount, `ENOSPC`). This
   is the one blocker for any capture (seam 1).
2. **Un-gate the capture tests on Linux (M).** Widen the ten `#![cfg]` gates file by file and
   fix fallout; run them by hand, not in CI (seam 11).
3. **A Linux executable pin (S, first route).** A `--dev` Hermes connector with the Linux
   SHA-256, reading as `Dev/unpinned`, for the first recorded run. The real fix is the pin
   decision (seam 7).
4. **Process discovery without procps (S)** and **the owner's ruling on Linux `W` (S to decide,
   M or L to build a subreaper or cgroup)** before any Linux `Confirmed` is cited (seam 2).
5. **`SHELL` preflight (S)** so Codex and Claude refuse clearly where `/bin/zsh` is missing.

### Windows: what must exist before a port starts

1. **Decisions:** the per-target executable pin (seam 7), the meaning of `keychain_link` off
   macOS, the Windows graceful-stop step, and whether a reservation that does not survive the
   last handle is acceptable under D37.
2. **Refuse instead of weaken (S):** every `cfg(not(unix))` open that is unprotected today
   refuses by name (seam 4).
3. **`GatedSpawn` and `ProcessOs` behind traits (S to M):** the Unix code moved behind the
   seams, so a Windows body is additive (seams 2 and 3).
4. **Windows capability backends (L):** `filecap`, `sourcecap`, `tracecap` `windows.rs`, then
   lifting the `compile_error!`.
5. **Job-object `ProcessOs`, suspended spawn gate, profile isolation (L).**
6. **A build-only Windows check (S),** if and when the owner accepts it into CI.

### Top items across both

| # | Item | Size | Unblocks |
|---|---|---|---|
| 1 | Linux `fallocate` reservation and its gate | M | Any Linux capture |
| 2 | Decide the per-target executable pin shape | S, then M | Any pinned non-arm64-Mac run |
| 3 | Owner ruling on Linux `W`, then subreaper or cgroup | S, then M/L | Linux `Confirmed` |
| 4 | Un-gate macOS-only capture tests on Linux | M | Linux evidence by hand |
| 5 | Unprotected `cfg(not(unix))` opens refuse by name | S | A safe start to Windows |

## Decide now

These should be settled before the Amp connector and its tests land, so that work does not add
new macOS-only assumptions:

- **Executable pin shape** (seam 7): schema 2 per-target map, or one directory per target.
- **`keychain_link` off macOS** (seam 5): refuse until measured, or treat as `empty`.
- **`SHELL` in connectors** (seam 8): keep `/bin/zsh` with a preflight check, or prefer
  `/bin/sh` for new connectors.
- **Linux `W`** (seam 2): one of the three answers in TERMINATION.

### Rules for new code

1. No new `cfg(target_os = "macos")` in non-test code outside `goalcup-platform`,
   `goalcup-protocol` `process`, `terminate` and `per_run_home` (test gates follow rule 7).
   Every platform `cfg` has an explicit other arm that refuses
   by name; never a silent no-op or an unprotected fallback.
2. Platform calls go through the existing seams: `goalcup-platform` capabilities for files,
   `ProcessOs` for processes. New code adds no direct `kill`, `/bin/ps` or `libc` file calls.
3. No absolute harness or install locations (`/bin/zsh`, `/opt/homebrew`, `/usr/local`) in
   code or connectors beyond `SHELL_VALUES` and `CAPTURE_PATH`. Harness executables are
   resolved and pinned by hash. Absolute system-tool paths used to stop PATH substitution
   (`/bin/ps`, `/bin/sh -p`, `/usr/bin/stat`) are fine.
4. Paths stay `Path` and `OsStr` until a record needs text; records use `/` via the existing
   normalization. Config and state locations come from `config_dir_with`, never `~/Library`.
5. Shell scripts are POSIX `sh` and run under both dash and bash, with no BSD-only or GNU-only
   flags (`stat -f`, `sed -i`, `date -j`, `readlink -f`), unless every dialect's form is tried
   by absolute path and its output is validated before use. Existing users: the key loader
   `scripts/with-deepseek-key.sh` tries `stat -f %Lp` and `stat -c %a` and accepts only octal
   digits; the macOS-only `packaging/macos/uninstall-assisted.sh` runs `stat -f '%u'` and
   refuses unless it equals `id -u`, but does not yet use an absolute path. Anything longer
   than a wrapper is Python or Rust.
6. The daemon does not learn who supervises it: no `launchctl`, `systemctl` or service API.
7. A macOS-only test says which seam makes it so in its `cfg` comment, so the gate can be
   widened when that seam lands.
8. No platform fact becomes a record field (v5 is frozen); platform facts go to `doctor` and
   status output.
9. A new harness declares its credential store, CA variable and home needs through existing
   schema values; code never infers them from the OS.
