# Run Lifecycle Implementation Plan (merged)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One competition run goes from `goalcup prep` to the last upload and survives 168 hours on a participant's own machine. It needs a browser-approved run credential, the harness in its normal chat mode in the participant's own terminal, rotating chain parts deleted once acknowledged, one 3 GB reservation per run, budgets instead of hard limits, loud failures, `goalcup continue`, and the Worker changes behind them.

**Architecture:** Amend what exists. `goalcup start` stays the run host (proxy, uploader, harness child) in the participant's terminal and gains one 30-second watch tick (reservation, part deletion, health, sleep lock). The daemon stays network-free. It learns three things: write the chain as rotating part files, seal a Git work folder by commit and digest, and admit every session file written since the run started. OS calls live in three `goalcup-platform` modules. Harness facts stay connector data. The Worker swaps member tokens for a device flow and a run-bound credential, and learns replace, segments and a 5-minute cron.

**Tech Stack:** Rust 2024 (`goalcup-{platform,protocol,daemon,cli,verifier}`, `libc` only, no new crates), TypeScript Cloudflare Worker (`server/worker`), Postgres via PostgREST RPC (SQL is a proposal to the website team).

**Spec:** `docs/superpowers/specs/2026-10-14-run-lifecycle-design.md` (repo `goal-cup-tracer`, main `1ea8842`), with `docs/UPLOAD-API.md`, `docs/SERVER-DESIGN.md`, D51–D53. Owner decisions made after the spec, binding here: rotating chain parts instead of hole punching; no terminal host (no pty, no detached host, no `goalcup attach`); session identity = every session file new or written since the run started.

## Global Constraints

- Platforms: macOS (Apple silicon, Intel), Linux (x86_64, arm64); Windows = WSL2. Platform-specific code only in `goalcup-platform`.
- 168 hours on the server's clock, hard stop. The run credential is valid "until the deadline plus 24 hours", stored mode 0600, never in the harness environment, argv or a URL.
- Disk: "one reservation for each run (3 GB), reused all week as uploads are confirmed, released after the last upload"; acknowledged data "removed locally after 2 hours"; "a warning below 5 GB free beyond the reservation"; clean stop only "when the reservation itself is nearly full and uploads are failing"; "no reservation for each segment".
- Timing: "the seal gets minutes, not 10 seconds, and is retried with growing waits; sync retries every minute at first, then up to every 15 minutes, without end until the deadline plus 24 hours. A daemon that is slow to answer is waited for, not treated as dead."
- Emails: 20 min without a heartbeat → "your run stopped; run `goalcup continue`", once per silence; 6 h without model calls → "your agent made no model calls for 6 hours: is it finished, out of tokens, or offline?", once per idle period; neither after the deadline.
- The tracer records; it never sends the agent a message, never types "keep going", never relaunches the harness by itself, has no finish tool, never force-pushes.
- v5 records are frozen; additions are D53-style optional skip-if-none fields or file-layout rows, each a DECISIONS row (D54 seal, D55 interactive connectors, D56 chain parts; D40 amended).
- No harness or vendor name in core code: new source files join the list in `crates/goalcup-protocol/tests/it/no_family_literals.rs`.
- Never set a model base URL; never write `~/.codex`, `~/.claude`, `~/.hermes`. A `connectors/` edit runs `sh scripts/pin-manifests.sh` in the same commit.
- Every cargo command `--offline`. Each task is one PR that passes `sh scripts/gate.sh fast` (Worker tasks also `cd server/worker && npm test`), run under `/tmp/goalcup-heavy.lock`.
- A run stays on the machine that started it: `continue` works only there, with that run's saved credential; there is no re-approval or move path.
- The website repo owns schema, sign-up, terms, approval page and mail; `server/worker/sql/proposed-goalcup-functions.sql` is a proposal.

## Review Focus

1. **The harness shares the terminal with `goalcup start`.** If the host writes to the TTY, moves the child out of the foreground group, or dies on Ctrl-C, the TUI breaks or the run ends. If the window is closed, the agent stops but the segment must still seal and upload. Expected: the host stays silent, the child is in the foreground group, and the host ignores SIGINT, SIGQUIT, SIGTSTP and SIGHUP while the child runs. Tests in T6: `the_host_is_silent_and_survives_ctrl_c`, `closing_the_window_still_seals_and_uploads_the_segment`.
2. **Reopen after acknowledged parts were deleted** (daemon restart, reboot then `continue`). Expected: resume starts at the first surviving part and appends from the true head. Test in R2: `resume_starts_from_the_first_surviving_part`.
3. **A shared harness home with thousands of old session files.** Expected: only files new or written since the run started are read, inside the walker's limits. Test in R4: `old_files_are_skipped_and_the_new_one_found_among_5000`.
4. **The device flow abandoned half way** (Ctrl-C, expired code, denial, server away). Expected: one sentence, nothing created locally, no open run on the server. Tests in A1 and T5.
5. **The agent's own build fills the disk.** Expected: the reserve file gives space back before the chain needs it, a `PROBLEM:` line appears, and the run stops cleanly only when the run's parts are near 3 GiB and uploads are failing. Test in T2: `the_agent_filling_the_disk_draws_down_the_reserve_before_any_stop`.

---

## Files and modules

| Module | One job | Replaces / required by |
|---|---|---|
| `goalcup-platform/src/disk.rs` | free bytes; a physically allocated reserve file | §5; reuses the tracecap allocators |
| `goalcup-platform/src/awake.rs` | sleep lock for the life of a value | §4 |
| `goalcup-platform/src/machine.rs` | WSL2, AC power, open a URL | prep, device flow |
| `goalcup-protocol` `export.rs` (+ `tracecap`) | chain as rotating parts | §5 pruning (D56) |
| `goalcup-daemon/src/workspace_git.rs` | commit + digest of uncommitted changes | §5 lighter seal (D54); replaces the walk for Git work folders |
| `goalcup-cli/src/account/{mod,device,credential,github}.rs` | device flow, credential store, repo check + clone + git helper | replaces the pasted token |
| `goalcup-cli/src/run/{mod,reserve,watch}.rs` | run record and `state.json`; reservation; the 30 s tick | §5, §6, `continue` |
| `goalcup-cli/src/upload/{policy,prune}.rs` | backoff and stop rule (moved from `uploader.rs`); which parts may be deleted | §5 |
| `goalcup-cli/src/commands/{prep,continue_run}.rs` | the two new commands | §3, §4 |
| `server/worker/src/{device,notices}.ts` | device flow; cron notices and deadline commit | §7, §6 |

**Deleted:** pasted token (`ServerConfig.token`, `checked_token`, `TOKEN_MIN_LEN/MAX_LEN`, the stdin read in `main.rs`); Worker `memberForToken`, `DEV_TOKENS`, `tokens`, the 409 "already has an open run"; `--capture-reserve-bytes`, `DEFAULT_CAPTURE_RESERVE_BYTES`, `TraceExport::reserve_capture`, `TraceDir::reserve_append` and its allocator fault seam; `DEADLINE_SEAL_RETRIES/_WAIT`; `Policy::finish_budget` outside test policies; `start --repo` (from prep); the Git workspace walk; both interactive connectors' headless argv and `continuation` text.

**Left out:** any terminal host; tmux requirement; hole punching; a `Clock` trait; torn-tail repair (continue closes from `local_end`); live totals; outage windows; two pinned keys; reboot login item; native Windows; finish tool; forced push.

## Wire contract (W1 writes it into `docs/UPLOAD-API.md`; W keeps the Rust stand-in in step)

- `POST /v1/device` (no auth) `{"purpose":"login"|"run","mode":"competition"|"practice","connector_id","tracer_version"}` → `200 {"device_code":<64 hex>,"user_code":"BCDF-GHJK","verification_uri","verification_uri_complete":"<uri>?code=<user_code>","interval":5,"expires_in":900}`. `user_code` alphabet `BCDFGHJKLMNPQRSTVWXZ`.
- `POST /v1/device/token` `{"device_code"}` → `202 {"status":"pending"}`; `200 {"status":"approved","purpose":"run","credential":"gcr_<64 hex>","expires_at"}` or `{"status":"approved","purpose":"login","account":{"handle"}}`; `403 denied`; `410 expired` (also any second pickup).
- `POST /v1/device/approve` (test only; `Authorization: Bearer <TEST_APPROVAL_KEY>`) `{"user_code","member_id","approve":bool}`. It exists only with `DB_BACKEND=local` or `STAGING=on`; otherwise a set `TEST_APPROVAL_KEY` makes every call `500 misconfigured`.
- All `/v1/runs/**`: `Authorization: Bearer gcr_…`; unknown or expired → `401`; another run's id → `404`. A credential is unbound until `POST /v1/runs` binds it; expiry `approved_at + 1 h` unbound, `deadline_at + 24 h` bound. Only SHA-256 of codes and credentials is stored.
- `POST /v1/runs`: body unchanged (keeps `repo_url` and the GitHub check); mode from the credential; same `session_id` again → `200` same run. Competition inside the window: the account's other open competition run gets `replaced_at = now`. Outside the window: `403` "after the start window you can only `goalcup continue` your run". Practice: practice event, never replaces.
- `ends_at = min(deadline_at, replaced_at)`; every deadline rule (stop, `awaiting_final`, wrap-up, abandon at +24 h) uses it.
- `POST /v1/runs/{id}/continue` `{"previous_entry_id","connector_id","manifest_sha256","connector_dir_sha256","workspace","session_id","harness_version","tracer_version"}` → `201 {"entry_id","segment","official_start","deadline_at"}`; same `session_id` → `200`; stale `previous_entry_id` → `409`; at/after `ends_at` → `410`. Receipt, chunks, heartbeat and seal address the named `entry_id`; `GET /v1/runs/{id}?entry_id=` gives that entry's position.
- Heartbeat adds optional `"recording":"ok"|"gaps"|"failed"|"unknown"` and `"model_requests":<u64 cumulative>`; answer adds `"reason":"deadline"|"replaced"|null`.
- Seal answers `{"status":"sealed","run_status":"open"|"sealed"}`: a run is sealed only when its current entry is sealed and now ≥ `ends_at`. Abandonment at `ends_at + 24 h` marks a run `sealed` when its current entry is sealed, else `abandoned`.

## Lanes, file ownership, merge order, dates

Ownership is by file. The only shared lines are `pub mod` lines in `crates/goalcup-cli/src/lib.rs` and test-module lines in `tests/it/main.rs`. A lane's files pass to T once that lane is done.

| Lane | Tasks (serial within the lane) | Owns | Dates |
|---|---|---|---|
| P platform | P1, P2 | `crates/goalcup-platform/src/{disk,awake,machine}.rs` + tests | 14–15 Oct |
| R recorder | R1 → R2 → R3 → R4 → R5 | `goalcup-protocol/**`, `goalcup-daemon/**`, `goalcup-verifier/**`, `goalcup-platform/src/tracecap*`, `docs/DECISIONS.md`, `docs/CONNECTOR-SCHEMA.md` | 14–19 Oct |
| W Worker | W1 → W2 → W3 | `server/worker/**`, `docs/UPLOAD-API.md`, `crates/goalcup-cli/tests/common/upload_server/**`, `tests/full/upload_worker.rs` | 14–21 Oct |
| A account | A1 → A2 | `crates/goalcup-cli/src/{account/**, server.rs, upload/http.rs, commands/login.rs}`, `tests/it/account.rs` | 15–18 Oct |
| T tracer | T1 → T2 → T3 → T4 → T5 → T6 → T7 → T8 | everything else in `goalcup-cli` (`main.rs`, `commands/start.rs`, `launch.rs`, `upload/**` but `http.rs`, `run/**`, `retry.rs`, `daemon/**`, `proxy/**`, `key_scan.rs`), `connectors/**`, `pins/**`, `skills/goal-cup/**` | 14–22 Oct |
| D docs and measure | D1, D2 | `scripts/matrix.sh`, `docs/OPERATING.md`, `docs/harnesses/*.md`, `README.md`, `TODO.md`, `docs/RULES.md`, `docs/PARTICIPANT-GUIDE.md`, `skills/goal-cup-setup/**` | 20–25 Oct |

Cross-lane waits: T2 needs P1, R1. T3 needs R2. T4 needs P2, W1. T5 needs A1, A2, W1. T6 needs R3, R4, T5. T7 needs W2, T6. T8 needs everything in T. **Walking skeleton 20 Oct** (after T6): `goalcup start --practice` → browser approval (staging switch) → chat-mode harness in the cloned repo → parts upload → practice deadline → `goalcup-verify` exit 0 on the server copy. **Full path 22 Oct** (after T8). Matrix and 8-hour runs 20–24 Oct (D1). Docs 24–25 (D2). **Freeze 26 Oct; dry run 28 Oct – 4 Nov, fixes only, in the owning lane's files.**

---

### Task P1: Free space and the reserve file

**Files:** Create `crates/goalcup-platform/src/disk.rs`; modify `crates/goalcup-platform/src/lib.rs`, `tracecap/unix.rs` (expose `allocate` as `pub(crate)`, no behaviour change); test `crates/goalcup-platform/tests/it/disk.rs`.

**Interfaces:**
- Produces: `pub fn free_bytes(dir: &Path) -> io::Result<u64>` (`statvfs` `f_bavail * f_frsize`); `pub struct ReserveFile`; `ReserveFile::hold(path: &Path, bytes: u64) -> io::Result<ReserveFile>` (create 0600 `O_NOFOLLOW` or reopen; physical allocation to `bytes`, APFS `F_PREALLOCATE`, ext4/XFS `fallocate(0)`, elsewhere zeros in 1 MiB writes; `ErrorKind::StorageFull` before allocating when short, no file left); `fn allocated(&self) -> u64` (`st_blocks * 512`); `fn resize(&mut self, bytes: u64) -> io::Result<()>`; `fn release(self) -> io::Result<()>` (unlink).

- [ ] **Step 1: Failing tests:** `a_reserve_file_holds_its_blocks_and_gives_them_back` (hold 64 MiB → allocated ≥ 64 MiB; resize 16 MiB → < 20 MiB; resize 48 MiB → ≥ 48 MiB; release → gone), `hold_reopens_without_doubling`, `hold_refuses_more_than_the_volume_has` (free + 1 TiB → `StorageFull`, no file).
- [ ] **Step 2:** `cargo nextest run --offline -p goalcup-platform -E 'test(/disk::/)'` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Same command → PASS; `sh scripts/linux-check.sh` once (ext4); `sh scripts/gate.sh fast` → PASS.
- [ ] **Step 5:** `git commit -m "platform: free space and a run reserve file"`

### Task P2: Sleep lock and machine facts

**Files:** Create `crates/goalcup-platform/src/{awake,machine}.rs`; modify `lib.rs`; tests `tests/it/{awake,machine}.rs`.

**Interfaces:**
- Produces: `pub struct SleepLock` (owns the inhibitor child; `Drop` kills and reaps); `pub fn hold(why: &str) -> Result<SleepLock, String>`: macOS `/usr/bin/caffeinate -i -s -w <own pid>`; Linux `systemd-inhibit --what=sleep:idle --mode=block --who=goalcup --why=<why> sleep infinity` with `PR_SET_PDEATHSIG(SIGTERM)` when on `PATH`; WSL2 `Err("the Windows host decides when this machine sleeps; set its power settings")`; none `Err("no sleep inhibitor on this machine")`. `machine::{is_wsl() -> bool, on_ac_power() -> Option<bool>, on_windows_drive(p: &Path) -> bool, open_url(url: &str) -> bool}` (`open`; `wslview` then `cmd.exe /c start ""`; `xdg-open`; detached, stdio null, never blocks). Pure parsers `is_wsl_from(&str)`, `ac_from_pmset(&str)` are `#[doc(hidden)] pub`.

- [ ] **Step 1: Failing tests:** `a_sleep_lock_dies_with_its_value`, `no_inhibitor_is_an_err_not_a_panic`, `wsl_detection_reads_osrelease` (`"5.15.167.4-microsoft-standard-WSL2"` true, `"6.8.0-45-generic"` false), `ac_power_parses_pmset`, `only_mnt_letter_paths_are_windows_drives` (`/mnt/c/x` true on WSL, `/mnt/data` false).
- [ ] **Step 2–4:** `cargo nextest run --offline -p goalcup-platform -E 'test(/awake::|machine::/)'` FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "platform: sleep lock and machine facts"`

### Task R1: Quota-only chains; delete per-entry preallocation (amends D40)

**Files:** Modify `crates/goalcup-protocol/src/capture.rs:39` (add `pub const CHAIN_QUOTA_BYTES: u64 = 1 << 40;` and make `DEFAULT_CAPTURE_RESERVE_BYTES` an alias of it until R5), `export.rs` (delete `reserve_capture`, `capture_reserved`), `control.rs:656` (doc: `reserve_bytes` is a logical quota), `crates/goalcup-daemon/src/runtime/capture/mod.rs:383,747` (`configure_capture_quota` only), `crates/goalcup-platform/src/tracecap.rs` + `tracecap/unix.rs` (delete `reserve_append`, `reserve`, `ReservationFault`), `docs/DECISIONS.md` (amend D40: one reservation per run, held by the run host; chains are quota-only; H-11 closed). Delete the physical cases in `crates/goalcup-protocol/tests/capture_reserved_storage.rs`.

**Interfaces:** Produces `CHAIN_QUOTA_BYTES`. `CaptureStorage.reserved_bytes` reports 0.

- [ ] **Step 1: Failing tests:** daemon `a_sealed_entry_holds_no_blocks_beyond_its_end` (`allocated ≤ len + one block`), `a_restart_reserves_nothing` (reopen five unsealed entries: no allocation beyond their lengths).
- [ ] **Step 2–4:** `cargo nextest run --offline -p goalcup-daemon -E 'test(/holds_no_blocks|reserves_nothing/)'` FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "capture: quota-only chains; no per-entry preallocation (D40, H-11)"`

### Task R2: The chain as rotating parts (D56)

**Files:** Modify `crates/goalcup-platform/src/tracecap.rs` (`open_part_for_create`, `open_part_for_resume`, `list_parts`, same `O_NOFOLLOW`/identity rules as `open_chain_for_create`), `crates/goalcup-protocol/src/export.rs`, `crates/goalcup-daemon/src/runtime/capture/writer.rs` (rotate after each successful `sync_capture`), `crates/goalcup-verifier/src/capture_analysis/reader.rs:217`, `docs/DECISIONS.md` (D56). Tests: `crates/goalcup-protocol/tests/it/roundtrip.rs`, `crates/goalcup-daemon/tests/it/capture_lifecycle.rs`, `crates/goalcup-verifier/tests/it/m2_capture_integrity.rs`.

**Interfaces:**
- Produces (protocol `export`): `pub const PART_MAX_BYTES: u64 = 256 << 20; pub const PART_MAX_AGE: Duration = 1 h;` `pub fn part_file_name(start: u64) -> String` (`0` → `"chain.jsonl"`, else `format!("chain.{start:020}.jsonl")`); `pub fn parse_part_name(name: &str) -> Option<u64>`; `pub struct ChainPart { pub start: u64, pub len: u64, pub path: PathBuf }`; `pub fn chain_parts(trace_dir: &Path) -> io::Result<Vec<ChainPart>>` (sorted by `start`; `InvalidData` when `parts[i+1].start != parts[i].start + parts[i].len`); `TraceExport::rotate_if_due(&mut self, now: Instant) -> Result<bool, ExportError>` (only at a durable frontier: closes the part, opens `part_file_name(chain_bytes)`). Test builds set limits with `TraceExport::set_part_limits(bytes, age)` under `test-hooks`.
- Resume: validates every surviving part in order and appends to the last. When the first part's `start > 0`, it begins at the first record with `next_seq = link.seq`, `head = link.prev` and `ChainPhaseReplay::after_pruned_prefix()` (capture format, baseline seen, phase open); `validated_bytes` and `chain_bytes` stay logical.
- Verifier: reads parts as one chain. With part 0 missing it reports the finding `chain_pruned_locally` (incomplete, exit 1, "pruned on this machine: verify the server copy") and skips acks whose `seq` is before the first surviving record.
- D56: "A capture chain may be split into part files named by their logical start offset; records and hashes are unchanged; the server stores one stream. Readers before this build read only single-file chains."

- [ ] **Step 1: Failing tests:** `a_chain_rotates_at_the_part_size_after_a_sync`; `parts_concatenate_to_the_single_file_chain` (the same records written with and without rotation: concatenated bytes equal); `resume_reads_every_part_and_appends_to_the_last`; `resume_starts_from_the_first_surviving_part` (delete the first two parts: head and `next_seq` equal the full resume's); `a_gap_between_parts_is_corrupt`; daemon `a_restart_after_parts_were_deleted_keeps_appending`; verifier `a_rotated_chain_verifies_like_one_file`, `a_chain_missing_its_first_part_says_verify_the_server_copy`.
- [ ] **Step 2:** `cargo nextest run --offline --workspace -E 'test(/rotat|parts|surviving_part|first_part/)'` → FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4:** Same → PASS; `sh scripts/linux-check.sh` once; gate PASS.
- [ ] **Step 5:** `git commit -m "capture: the chain rotates into part files (D56)"`

### Task R3: Lighter seal and minutes for the finish (D54)

**Files:** Create `crates/goalcup-daemon/src/workspace_git.rs`. Modify `crates/goalcup-protocol/src/seal.rs` (fields; correct the doc naming a nonexistent `attribution.rs`), `control.rs:802` (`LOCAL_CAPTURE_FINISH_BUDGET` 10 s → 180 s; `FINISH_COMMIT_RESERVE` stays 2 s), `crates/goalcup-daemon/src/workspace.rs:269-293` (R3 admission), `runtime/capture/mod.rs:409` and `finish.rs:405-420` (git first, walk otherwise; baseline `home`), `docs/DECISIONS.md` (D54). Tests: `crates/goalcup-daemon/tests/it/workspace_layout.rs`, `capture_lifecycle.rs`, `crates/goalcup-verifier/tests/it/v5_corpus.rs`.

**Interfaces:**
- Produces: `seal::WorkspaceGit { pub commit: Option<String> /* 40 hex, None = unborn HEAD */, pub dirty_sha256: String /* 64 hex */ }`; `SealManifest.git: Option<WorkspaceGit>` and `SealManifest.home: Option<Vec<SealedFile>>`, both `#[serde(default, skip_serializing_if = "Option::is_none")]`. With `git`, `files` is empty. `home` appears on the baseline only: names, sizes and SHA-256 of the connector's config home, excluding every native root, bounded to 10 000 files.
- `workspace_git::state(workspace: &Path, budget: Duration) -> Option<WorkspaceGit>`: `None` unless `<workspace>/.git` is a directory; otherwise git runs with `env_clear` + `PATH`, a fresh empty `HOME`, `GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`, `-c core.fsmonitor=false -c core.hooksPath=/dev/null -c diff.external=`, stdin null.
- Digest: SHA-256 over `b"diff\0"` ‖ `git diff --binary --no-ext-diff --no-textconv --no-color <base>` (`HEAD`, or the empty tree `4b825dc642cb6eb9a060e54bf8d69288fbee4904`) ‖ `b"untracked\0"` ‖ for each `git ls-files -z --others --exclude-standard` path, sorted bytewise: `path ‖ 0x00 ‖ hex(sha256(content)) ‖ b"\n"` (a file over 64 MiB contributes `size:<n>`).
- R3 amended: a workspace that is the root of its own Git work tree (no work tree above it) is admitted non-empty; anything else keeps R3 unchanged.

- [ ] **Step 1: Failing tests:** `a_fresh_clone_seals_no_commit_and_the_empty_digest` (digest = `sha256(b"diff\0untracked\0")`); `the_digest_moves_with_an_untracked_file_and_a_tracked_edit`; `an_agent_written_fsmonitor_is_never_run`; `a_clone_is_admitted_non_empty_and_not_walked` (baseline `files == []`, `git` set); `a_workspace_inside_another_work_tree_is_refused`; `a_non_git_workspace_still_walks`; `the_baseline_lists_the_harness_home_without_its_session_folder`; `a_finish_may_take_three_minutes` (`remaining_finish_ms: Some(170_000)` → 170 s); verifier `a_seal_with_git_verifies`; `a_manifest_without_the_new_fields_keeps_its_digest`.
- [ ] **Step 2–4:** `cargo nextest run --offline --workspace -E 'test(/clone|digest|fsmonitor|work_tree|still_walks|harness_home|three_minutes|seal_with_git|keeps_its_digest/)'` FAIL → implement, write D54 → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "seal: git commit and uncommitted digest; the finish gets minutes (D54)"`

### Task R4: Chat-mode connectors as data; session files since the run started (D55)

**Files:** Modify `crates/goalcup-protocol/src/harness.rs` (fields and rules; rule at `:1919` admits `none` on a shared home when `since_run_start`), `control.rs` (`CaptureWorkspaceRequest.native_since: Option<u64>`, skip-if-none), `crates/goalcup-daemon/src/native_capture.rs` (`NativeSourceSpec.modified_since`), `runtime/capture/{mod.rs,native_scope.rs}`, `docs/CONNECTOR-SCHEMA.md`, `docs/DECISIONS.md` (D55). Tests: `crates/goalcup-protocol/tests/it/harness_schema.rs`, `crates/goalcup-daemon/tests/it/capture_native.rs`.

**Interfaces:**
- Schema: `[capture] interactive = true|false` (default false). With it: no `goal_flag`, `args` need not end in `--`, no `credential`, identity `none`. `[capture] resume_args = [...]` (optional; appended after `args` on `continue`; D51 argv rules). `[[capture.native]] since_run_start = true|false`. `[capture.home] skills_dir = "<rel>"` (optional, relative to the config home).
- Daemon: `modified_since: Option<SystemTime>` admits a path into the tracked set only once its mtime ≥ `modified_since`, checked before it counts against `max_files`; once admitted it stays. Every admitted file is recorded; none is refused and none is preferred.
- D55: "Shipped connectors run the harness in its normal chat mode in the participant's terminal; the participant types the prompt; connectors set no model, approval, sandbox or permission option (amends D52); identity is every session file new or written since the run started, which is safe because runs on a shared home are serialized."

- [ ] **Step 1: Failing tests:** `an_interactive_profile_takes_no_goal_and_no_credential`; `none_on_a_shared_home_needs_since_run_start`; `resume_args_follow_the_argv_rules` (`--url` refused); `old_files_are_skipped_and_the_new_one_found_among_5000` (5 000 rollouts dated yesterday plus one new: exactly the new one); `two_new_session_files_are_both_recorded`; `an_old_file_written_during_the_run_is_recorded`.
- [ ] **Step 2–4:** `cargo nextest run --offline --workspace -E 'test(/interactive_profile|since_run_start|resume_args_follow|among_5000|both_recorded|written_during/)'` FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "connectors: chat mode as data; record every session file since the start (D55)"`

### Task R5: Fine-grained GitHub token redaction; drop the reserve alias

**Files:** `crates/goalcup-daemon/src/redact.rs:151` and `redact/stream.rs` (`("github_token", r"github_pat_[A-Za-z0-9_]{22,}")` beside `gh[pousr]_`), `spikes/redact.py` (same rule), `crates/goalcup-protocol/src/capture.rs` (delete the `DEFAULT_CAPTURE_RESERVE_BYTES` alias once T2 has merged); test `crates/goalcup-daemon/tests/it/redaction.rs`.

- [ ] **Step 1:** Failing test `a_fine_grained_github_token_is_redacted_in_stream_and_whole`.
- [ ] **Step 2–4:** FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "redact: fine-grained GitHub tokens"`

### Task W1: Device flow, run-bound credential, stand-in

**Files:** Create `server/worker/src/device.ts`. Modify `server/worker/src/{index,auth,db,db-local,db-supabase,env,context}.ts`, `sql/proposed-goalcup-functions.sql` (`device_requests`, `run_credentials` replacing `tokens`, `goalcup_approve_device` for the website), `wrangler.toml` (`APPROVAL_URL`), `README.md`, `docs/UPLOAD-API.md` (device section, auth, heartbeat fields, `continue` contract). Rust stand-in `crates/goalcup-cli/tests/common/upload_server/**`: device routes, auto-approve with a test member, a deny/expire knob, credential auth, and acceptance of the new heartbeat fields.

**Interfaces:**
- Produces: the wire contract above. `Db.createDevice`, `Db.pollDevice(device_sha256, now)` → `pending | approved{purpose, credential_plain?, expires_at?, handle?} | denied | expired` (the credential is minted at pickup, returned once, stored hashed), `Db.approveDevice(user_code, member_id, approve, now)`, `Db.credential(sha256, now) → {member_id, run_id|null, mode, expires_at} | null`, `Db.bindCredential(sha256, run_id, expires_at)`. Stand-in: `UploadServer::set_approval(Approval::{Auto, Deny, Expire})`.

- [ ] **Step 1: Failing tests:** `pending, approve, pickup once, then 410`; `a denied device is 403`; `a credential opens one run and is 404 on another`; `repeat open with the same session_id returns the same run`; `credential expiry: approved+1h unbound, deadline+24h bound`; `TEST_APPROVAL_KEY outside local or STAGING is misconfigured`; `the device code never appears in a URL`.
- [ ] **Step 2–4:** `cd server/worker && npm test` FAIL → implement (Worker + stand-in) → PASS; `cargo nextest run --offline -p goalcup-cli --test it upload_server` PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "worker: device flow and a run-bound credential replace member tokens"`

### Task W2: Replace, practice, start window, segments

**Files:** `server/worker/src/{index,runs,chunks,db,db-local,db-supabase,context,env}.ts`, SQL (`runs.mode`, `runs.replaced_at`; unique index `(member_id) where status = 'open' and mode = 'competition' and replaced_at is null`; entries per segment; `upload_continue_run`), `wrangler.toml` (`PRACTICE_EVENT_ID`, `PRACTICE_RUN_MINUTES = "10"`), `docs/UPLOAD-API.md`, stand-in.

**Interfaces:** `isPastDeadline(run.deadline_at)` call sites become `isPastEnd(run)`; `Db.continueRun(...) → created | conflict | closed`; `EntryRow { entry_id, run_id, segment, offset, next_seq, head, last_ack, final, receipt, sealed_at }`; heartbeat answer `reason`.

- [ ] **Step 1: Failing tests:** `a second start in the window replaces the first, whose heartbeat says stop/replaced`; `the replaced run still takes its wrap-up chunks`; `after the window a start is 403 and continue works`; `practice never replaces and ends after PRACTICE_RUN_MINUTES`; `continue mints segment 1 with its own entry and receipt`; `stale previous_entry_id is 409`; `continue after ends_at is 410`; `chunks and seal address an older segment by entry_id`; `a segment seal before the end leaves the run open`; `abandonment marks a run whose current entry is sealed as sealed`.
- [ ] **Step 2–4:** `npm test` FAIL → implement (Worker + stand-in) → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "worker: replace rule, practice runs, continue and segments"`

### Task W3: Heartbeat signals, notices, deadline commit, staging

**Files:** Create `server/worker/src/notices.ts`, `test/notices.test.ts`. Modify `src/{index,runs,db,db-local,db-supabase}.ts` (`scheduled` export), `wrangler.toml` (`[triggers] crons = ["*/5 * * * *"]`; `[env.staging]` with `STAGING = "on"`), SQL (`heartbeats.recording`, `runs.model_requests`, `runs.last_model_request_at`, `runs.flagged_at`, `runs.silence_notice_for`, `runs.idle_notice_for`, `runs.final_commit`, `notices` outbox), `docs/UPLOAD-API.md` ("What the server does on its own").

**Interfaces:** A heartbeat with `recording ∈ {gaps, failed}` sets `flagged_at` once; a `model_requests` above the stored value sets `last_model_request_at = now` (start: `official_start`). `queueNotices(now) → number`: `silence` when open, non-practice, `now < ends_at`, `last_heartbeat_at < now − 20 min` and `silence_notice_for ≠ last_heartbeat_at`; `idle` when `last_heartbeat_at ≥ now − 20 min`, `last_model_request_at < now − 6 h` and `idle_notice_for ≠ last_model_request_at`. `recordFinalCommits(now)`: for `ends_at ≤ now` without `final_commit`, `GET /repos/{o}/{r}/commits/HEAD` → `final_commit`; failure retried next tick.

- [ ] **Step 1: Failing tests:** `20 minutes of silence queues one notice; a second silence another`; `six hours without model_requests growth queues one idle notice`; `no notice after the end or for practice`; `recording failed flags the run once`; `a heartbeat without the new fields is still 200`; `the deadline commit is recorded once and retried after a GitHub failure`; config `TEST_APPROVAL_KEY is absent from the production block`.
- [ ] **Step 2–4:** `npm test` FAIL → implement → PASS; gate PASS. Owner deploys `--env staging` on 19 Oct.
- [ ] **Step 5:** `git commit -m "worker: flags, silence and idle notices, the commit at the deadline, staging"`

### Task A1: Device-flow client, run credential store, login

**Files:** Create `crates/goalcup-cli/src/account/{mod.rs,device.rs,credential.rs}`, `tests/it/account.rs`. Modify `crates/goalcup-cli/src/server.rs` (`ServerConfig { url, account: Option<String> }`; `DEFAULT_SERVER_URL = "https://ingest.agentcommunity.org"`; the token fields stay until T5 removes their callers), `upload/http.rs` (`ServerClient::anonymous(url: &str)`, `ServerClient::with_credential(url: &str, c: &RunCredential)`), `commands/login.rs` (`pub struct LoginArgs { server: Option<String> }`, `pub fn run(args: &LoginArgs, cfg: &Path, open: &dyn Fn(&str) -> bool, out: &mut dyn Write) -> Result<(), CliError>`), `lib.rs` (`pub mod account;`).

**Interfaces:**
- Consumes: W1 stand-in; P2 `machine::open_url`.
- Produces: `pub enum Purpose { Login, Run { practice: bool } }`; `pub enum Approved { Login { handle: String }, Run(RunCredential) }`; `pub fn approve(server: &ServerClient, purpose: Purpose, connector_id: &str, open: &dyn Fn(&str) -> bool, out: &mut dyn Write) -> Result<Approved, CliError>`. It prints the URL and user code (never the device code), opens the browser once, polls at `interval`, and stops at 410/403/15 min/Ctrl-C. `pub struct RunCredential` (Debug prints `<redacted>`, `expires_at`); `credential::store(run_dir: &Path, c: &RunCredential) -> Result<(), CliError>` (file `credential`, 0600, dir 0700); `credential::load(run_dir: &Path) -> Result<RunCredential, CliError>` (refuses a group/world-readable file).

- [ ] **Step 1: Failing tests:** `login_stores_the_handle_and_no_secret` (no `gcr_`, no 64-hex string under cfg); `only_the_user_code_is_printed`; `an_expired_code_says_start_again_and_creates_nothing`; `a_denied_approval_creates_nothing`; `a_credential_file_readable_by_others_is_refused`.
- [ ] **Step 2–4:** `cargo nextest run --offline -p goalcup-cli --test it account::` FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "account: device-flow approval and a run credential store"`

### Task A2: GitHub repository, clone and credential helper

**Files:** Create `crates/goalcup-cli/src/account/github.rs`; tests in `tests/it/account.rs`.

**Interfaces:**
- Produces: `pub struct GithubSetup { pub repo_url: String, token: String }` at `<cfg>/github.json` (0600; the token read from stdin unshown); `check(api_base: &str, s: &GithubSetup) -> Result<(), CliError>` (`GET /repos/{o}/{r}`: refuse unless `private == false`, `size == 0`, `permissions.push == true`, each with its own sentence); `clone_into(s: &GithubSetup, dest: &Path, goalcup_exe: &Path) -> Result<(), CliError>` (`git clone` with no token anywhere, then repo-local `credential.https://github.com.helper = "!<abs goalcup> git-credential"`, `credential.useHttpPath = true`); `credential_helper(cfg: &Path, op: &str, input: &mut dyn BufRead, out: &mut dyn Write) -> Result<(), CliError>` (for `get` with `protocol=https`, `host=github.com`, `path=<o>/<r>.git` prints `username=x-access-token` and `password=<token>`; anything else prints nothing; `store`/`erase` no-ops). Test builds point the API base and clone source at local fakes through `test-support` only.

- [ ] **Step 1: Failing tests:** `private_non_empty_or_read_only_repos_are_refused_with_distinct_sentences`; `the_helper_answers_only_for_its_own_repository`; `the_clone_has_a_helper_and_no_token_in_its_config`; `the_token_never_appears_in_a_git_argv` (fake `git` on `PATH` logs argv).
- [ ] **Step 2–4:** FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "account: GitHub repository check, clone and credential helper"`

### Task T1: Budgets and retries instead of limits

**Files:** Create `crates/goalcup-cli/src/upload/policy.rs` (move `Policy`, `backoff` from `uploader.rs`). Modify `upload/{mod.rs,uploader.rs}`, `retry.rs` (`retry_capture_seal_until`), `commands/start.rs:1300-1340,2105-2107`, `commands/upload.rs`, `daemon/spawn.rs:74`. Tests: `tests/it/upload_chunks.rs`, `policy.rs` and `retry.rs` units, `tests/full/run_scoped_daemon.rs`.

**Interfaces:**
- Produces: `ServerError::ends_upload(&self) -> bool` (true only for 401, 404, 410; everything else retried with `backoff`); `Running::finish(self) -> Report` returns when caught up and sealed, when `ends_upload`, or at `wrap_up_until = deadline_at + 24 h` (continuous clock); `retry::retry_capture_seal_until<T>(until: Instant, call: impl FnMut() -> Result<T, CaptureSealError>) -> Result<T, CaptureSealError>` (waits 10 s doubling to 5 min, ±25 % jitter; local-only runs pass `now + 30 min`). `DEADLINE_SEAL_RETRIES/_WAIT` deleted. `SPAWN_WAIT` (10 s) still applies to a daemon pid that is dead; while the spawned or lock-holding pid lives, wait up to `DAEMON_READY_BUDGET = 600 s`, printing "the daemon is reopening the run (N s)" every 10 s.

- [ ] **Step 1: Failing tests:** `only_401_404_410_stop_the_uploader` (400, 409, 413, 422, 429, 500 → retried); `a_rejected_chunk_is_retried_not_the_end`; `finish_keeps_sending_after_the_seal_until_the_server_answers`; `the_wrap_up_stops_at_deadline_plus_a_day`; `an_expired_finish_is_retried_with_growing_waits` (10, 20 ms with the test policy); `a_daemon_still_reopening_is_waited_for`; `a_dead_daemon_is_not_waited_for`.
- [ ] **Step 2–4:** `cargo nextest run --offline -p goalcup-cli -E 'test(/stop_the_uploader|retried_not|keeps_sending|wrap_up|growing_waits|reopening|dead_daemon/)'` FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "budgets: the seal gets minutes and retries; sync retries until deadline plus 24 h"`

### Task T2: One reservation per run, and the watch tick

**Files:** Create `crates/goalcup-cli/src/run/{mod.rs,reserve.rs,watch.rs}`. Modify `lib.rs` (`pub mod run;`), `main.rs` (delete `--capture-reserve-bytes`), `commands/start.rs` (send `CHAIN_QUOTA_BYTES`; take the reserve right after the run dir; one stop channel), `upload/uploader.rs` (`begin(.., stop: Sender<String>)`; delete `take_stop`; `Running::last_ok() -> Arc<AtomicU64>`; `Running::acked_offset() -> Arc<AtomicU64>`), `daemon/mock.rs`, `tests/common/support/mod.rs`. Tests: `run/reserve.rs` units, `tests/it/start_launch.rs`.

**Interfaces:**
- Consumes: P1 `ReserveFile`, `free_bytes`; R1 `CHAIN_QUOTA_BYTES`; R2 `chain_parts`.
- Produces: `run::RunRecord { run_id, server_url: Option<String>, practice, connector_id, workspace: PathBuf, started_at_unix: u64, deadline_at: Option<String>, entries: Vec<EntryRef { entry_id, trace_dir }> }` at `<cfg>/runs/<run_id>/run.json` (local runs use the entry id as `run_id`); `run::{write, read, newest(cfg), of_entry(cfg, entry_id)}`; `run::RunState` at `<run_dir>/state.json` (Task T4 fills it).
- `reserve.rs`: `RUN_RESERVE_BYTES = 3 << 30`, `START_MARGIN = 1 << 30`, `LOW_DISK_BYTES = 5 << 30`, `FREE_FLOOR = 256 << 20`, `UPLOAD_FAILING_AFTER = 30 min`; `pub fn reserve_path(artifact_root: &Path, run_id: &str) -> PathBuf` (`<artifact_root>/reserve/<run_id>`, same volume as the traces); `pub fn target(footprint: u64, free: u64, reserve: u64) -> u64` = `min(RUN_RESERVE_BYTES.saturating_sub(footprint), (reserve + free).saturating_sub(FREE_FLOOR))`; `pub enum DiskVerdict { Ok, Low { free: u64 }, Stop }`; `pub fn verdict(footprint: u64, free_beyond_reserve: u64, uploads_failing: bool) -> DiskVerdict` (`Stop` iff `footprint ≥ RUN_RESERVE_BYTES * 9 / 10 && uploads_failing`; `Low` iff free beyond the reserve < `LOW_DISK_BYTES`); `pub fn pressure(footprint: u64) -> bool` (`footprint > RUN_RESERVE_BYTES`); `RunReserve::take(path) -> Result<RunReserve, CliError>` (refuses "this run needs 3 GiB free plus 1 GiB; N are free"); `release_stale(cfg, artifact_root, now)` (removes reserves of runs past `deadline_at + 24 h`, called by `start` and `continue`).
- `watch::spawn(scope, Watch { reserve, run: RunRecord, stop: Sender<String>, uploads: Vec<Arc<AtomicU64>> /* last_ok */, done: Arc<AtomicBool> })` ticks every 30 s. Footprint is the sum of `chain_parts` lengths over the run's entries. It resizes the reserve to `target`, and on `Stop` sends "the disk is nearly full and uploads are failing" once; that stop takes the deadline path (seal, upload). The reserve is kept across segments and released only at the run's end (deadline or replaced stop after the last ack, or a local run's end).

- [ ] **Step 1: Failing tests:** `the_reserve_is_three_gib_minus_the_parts_on_disk`; `the_agent_filling_the_disk_draws_down_the_reserve_before_any_stop` (free 100 MiB, reserve 2 GiB → target gives back to the floor, `Low`, no `Stop`); `a_full_reservation_with_failing_uploads_stops`; `a_full_reservation_with_working_uploads_does_not_stop`; it `a_run_holds_one_reserve_file_and_releases_it_at_the_end`; `capture_reserve_bytes_is_no_longer_a_flag`; `a_stale_reserve_is_released_by_the_next_start`.
- [ ] **Step 2–4:** FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "disk: one 3 GB reservation per run and the watch tick"`

### Task T3: Upload across parts; delete acknowledged parts after 2 hours

**Files:** Create `crates/goalcup-cli/src/upload/prune.rs`. Modify `upload/uploader.rs` (`build_chunk`, `waiting_bytes`, `local_link_before`, `local_end` over `chain_parts`), `key_scan.rs:484,625` (whole = parts contiguous from 0; part 0 missing → `Incomplete`), `run/watch.rs` (delete each tick). Tests: `prune.rs` units, `tests/it/upload_chunks.rs`.

**Interfaces:**
- Consumes: R2 `chain_parts`, `ChainPart`; T2 watch, `pressure`, `Running::acked_offset`.
- Produces: `build_chunk(parts: &[ChainPart], pos: &Position, spool: &Path)`. A chunk never crosses a part's end; offsets are logical. `pub const PRUNE_AFTER: Duration = 2 h`; `pub struct AckAges(HashMap<(String, u64), Instant>)` (first time this process saw a part wholly acknowledged; monotonic, so wall-clock jumps cannot prune early); `pub fn deletable(entry: &str, parts: &[ChainPart], acked_offset: u64, ages: &mut AckAges, now: Instant, pressure: bool) -> Vec<PathBuf>`. A part qualifies only if it is not the last part and `start + len < acked_offset` (the part holding the last acknowledged record stays, so `local_link_before` always finds it), and if it was first seen acknowledged ≥ `PRUNE_AFTER` ago or `pressure`. Deletion is `std::fs::remove_file`. After a restart, `AckAges` starts empty, so nothing is deleted for 2 h.

- [ ] **Step 1: Failing tests:** `a_chunk_stops_at_a_part_end`; `upload_continues_from_the_server_offset_after_parts_were_deleted`; `only_parts_acknowledged_two_hours_ago_are_deleted`; `the_part_holding_the_last_acknowledged_record_is_kept`; `the_live_part_is_never_deleted`; `pressure_deletes_young_acknowledged_parts`; `local_end_reads_the_last_part`.
- [ ] **Step 2–4:** FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "disk: upload chain parts and delete acknowledged ones after two hours"`

### Task T4: Loud failures, model requests, sleep lock

**Files:** Modify `run/mod.rs` (`RunState`), `run/watch.rs`, `commands/status.rs`, `upload/{api.rs,uploader.rs}`, `proxy/mod.rs` (`ProxyHealth.model_requests`), `proxy/ingest.rs` (consecutive failures), `commands/start.rs` (hold the sleep lock). Tests: `tests/it/status_views.rs`, `tests/it/upload_server.rs`, `tests/it/m2_http_relay.rs`.

**Interfaces:**
- Consumes: P2 `awake::hold`; W1 stand-in fields.
- Produces: `RunState { updated_at, entry_id, harness: running | exited{status}, recording: Recording, upload: { last_ok_at, problem }, disk: { free_bytes, reserve_bytes, low }, sleep_lock: Result<(), String> }`, written atomically (0600) each tick and on every harness transition; `run::read_state(run_dir) -> Option<RunState>`. `pub enum Recording { Ok, Gaps, Failed, Unknown }` from `M2Status.capture` (`Recording`→Ok, `Incomplete`→Gaps, `Failed`→Failed, no answer within `SLOW_CALL_TIMEOUT`→Unknown, never "dead"). `ProxyHealth.model_requests: u64` counts each request relayed on an intercepted connection to a `[[capture.hosts]]` host and each client→server WebSocket message there. `HeartbeatRequest.{recording, model_requests}`, `HeartbeatAnswer.reason` (`replaced` stops the run with "a newer run on your account replaced this one"). `goalcup status` prints one `PROBLEM:` line per problem before anything else (`recording has gaps`, `recording failed`, `last upload <age> ago: <why>`, `disk: <free> free beyond the reservation`, `the machine may sleep: <why>`, `the agent's process exited (<status>): run goalcup continue`).

- Also (coordinator's self-review: a week of short daemon stalls must not switch the wire leg off for good, per "one failed attempt never ends a run"): `MAX_CONNECTION_FAILURES` in `proxy/ingest.rs` counts **consecutive** failures; a successful hand-off resets it to 0. Nothing else in that module changes.

- [ ] **Step 1: Failing tests:** `a_successful_connection_resets_the_failure_count` (9 failures, 1 success, 9 failures: wire capture still on); `status_puts_every_problem_first`; `status_with_no_problem_prints_no_problem_line`; `the_heartbeat_carries_recording_and_model_requests`; `two_requests_on_one_connection_count_two`; `a_replaced_answer_stops_the_run_with_its_reason`; `a_daemon_that_does_not_answer_is_unknown_not_failed`.
- [ ] **Step 2–4:** FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "status: problems first; heartbeat says recording and model requests; keep the machine awake"`

### Task T5: Wire the account into `start`, `upload` and the CLI

**Files:** Modify `main.rs` (`login` uses `commands::login::run`; delete the stdin paste and `EchoOff`; hidden `git-credential <get|store|erase>`; `start --practice`; delete `--repo`), `commands/start.rs` (with a server: approval `Purpose::Run` before anything local is created, then `POST /v1/runs` with the credential, then `credential::store` in the run dir; workspace = `account::github::clone_into` of prep's repo into `<work_root>/goalcup-<run_id>`), `upload/mod.rs`, `commands/upload.rs` (credential via `run::of_entry`), `server.rs` (delete token fields and `checked_token`). Tests: `tests/it/upload_open.rs`, `tests/it/start_launch.rs`.

**Interfaces:**
- Consumes: A1, A2, W1, T2 `RunRecord`.
- Produces: `start` refuses "run `goalcup prep` first" when a server run has no prep repo or GitHub token; WSL2 refuses a work root on a Windows drive; nothing reaches the harness environment but what the connector sets.

- [ ] **Step 1: Failing tests:** `start_gets_a_run_bound_credential_before_creating_anything` (credential 0600; `run.json` matches); `the_credential_never_reaches_the_harness` (fake harness dumps env and argv: no `gcr_`); `ctrl_c_while_waiting_for_approval_creates_nothing`; `upload_finds_the_credential_from_the_entry`; `the_workspace_is_a_clone_whose_remote_holds_no_token`; `start_without_prep_refuses_before_creating_anything`.
- [ ] **Step 2–4:** FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "start: approve each run in the browser; the workspace is the participant's clone"`

### Task T6: The harness in chat mode in the participant's terminal

**Files:** Modify `launch.rs` (`goal: Option<&[u8]>`; interactive children inherit stdio and stay in the foreground process group), `commands/start.rs` (no goal for interactive; `native_since`; silent host; signals; skill copy), `main.rs` (`--goal-file` only for headless connectors), `connectors/{codex-cli,claude-code-cli}/harness.toml`, `pins/*` (`sh scripts/pin-manifests.sh`), `skills/goal-cup/SKILL.md` (placeholder; the owner supplies the sponsor text), `scripts/package-release.sh` (ship it), `docs/harnesses/{codex-cli,claude-code-cli}.md`. Tests: `tests/it/start_launch.rs`, `tests/it/harness_launch_plan.rs`, fixture `tests/fixtures/m2-fake-tui.sh` (reads lines from its tty, appends each to a new session file).

**Interfaces:**
- Consumes: R3, R4, T5.
- Produces: `native_since = RunRecord.started_at_unix` (run start, for every segment). While the child runs, the host ignores SIGINT, SIGQUIT, SIGTSTP and SIGHUP and writes nothing to the terminal: every message goes to `<run_dir>/host.log` and `state.json`. After the child exits, it prints only if the terminal is still there. Before launch the host copies `skills/goal-cup/` into `<config_home>/<skills_dir>/goal-cup/`, replacing only that folder. Connectors (data only, measured by hand first, results in the facts sheets):
  - codex-cli: `interactive = true`; `args = ["-c", "check_for_update_on_startup=false", "-c", 'projects={{ "{workspace}" = {{ trust_level = "trusted" }} }}', "-C", "{workspace}"]`; identity `none`; native `root = "{config_home}/sessions"`, `pattern = "*/*/*/rollout-*.jsonl"`, `since_run_start = true`; `skills_dir = "skills"`.
  - claude-code-cli: `interactive = true`; `args = []`; identity `none`; native `root = "{config_home}/projects"`, `pattern = "*/*.jsonl"`, `since_run_start = true`; `skills_dir = "skills"`; `DISABLE_AUTOUPDATER` and `CLAUDE_CODE_RETRY_WATCHDOG` stay. hermes-cli stays headless.

- [ ] **Step 1: Measure by hand** (logged-in harnesses under `harnesses/`): when each harness creates its session file in chat mode; whether a typed message appears in it; what `resume --last` and `--continue` do in the tracer's home. Write the answers in `docs/harnesses/<id>.md`.
- [ ] **Step 2: Failing tests:** `the_host_is_silent_and_survives_ctrl_c` (spawn `goalcup start` on a test pty, type `hello`, send SIGINT to the foreground group: the fake TUI records `hello`; between the launch line and the child's exit the pty shows only TUI bytes; the host seals after the TUI exits); `closing_the_window_still_seals_and_uploads_the_segment` (SIGHUP to the group: TUI exits, the host seals, sends the final heartbeat and uploads); `the_typed_prompt_is_recorded_from_the_session_file`; `heartbeats_flow_before_the_first_session_file_appears`; `a_goal_file_with_an_interactive_connector_is_refused`; `the_goal_cup_skill_is_in_the_harness_home_and_nothing_else_changed`.
- [ ] **Step 3–5:** FAIL → implement, edit connectors, `sh scripts/pin-manifests.sh` → PASS; gate PASS. One 10-minute practice run per connector on this Mac against staging.
- [ ] **Step 6:** `git commit -m "launch: the harness runs in chat mode in the participant's terminal (D55)"`

### Task T7: Segments: an exit keeps the run open; `goalcup continue`

**Files:** Create `commands/continue_run.rs`. Modify `main.rs` (`continue [RUN_ID]`), `commands/start.rs` (`Opening::{Fresh, Continue(RunRecord)}`; end-of-segment path; release the shared-home lock at the segment's seal), `upload/{mod.rs,api.rs,uploader.rs}` (`continue` call, `?entry_id=`), `connectors/*/harness.toml` (`resume_args`) + pins. Test: `tests/full/upload_e2e.rs`.

**Interfaces:**
- Consumes: W2, T6, A1 `credential::load`.
- Produces: when the harness exits before the run's end, the host seals the entry, sends that entry's final heartbeat, releases the shared-home lock, and prints (if the terminal is there) "The agent's process ended (<status>). Your run stays open until <deadline_at>; run `goalcup continue` to restart it." It then keeps uploading its entry until caught up, and exits 0. The reserve file stays. `continue_with(client, cfg, env, run: Option<&str>, out) -> Result<(), CliError>` (default `run::newest`). Steps: load the saved credential (no browser; a run with no `RunRecord` or credential on this machine is refused with "this run was not started on this machine"); refuse while another host holds the shared-home lock, naming the run; seal the previous entry if the daemon holds it, otherwise end its upload at `local_end` with a final heartbeat (an honest incomplete segment); drain the previous entry in a background thread; `POST continue`; launch with `args ++ resume_args`; append the entry to `RunRecord.entries`. `resume_args`: codex-cli `["resume", "--last"]`, claude-code-cli `["--continue"]` (as measured in T6).

- [ ] **Step 1: Failing tests:** `a_killed_agent_leaves_the_run_open_and_continue_makes_segment_one` (no second approval; launched with `resume_args`; both entries uploaded); `continue_after_the_deadline_is_refused_and_creates_nothing`; `continue_of_a_run_not_started_here_is_refused`; `continue_after_a_reboot_closes_an_unreopenable_segment_from_its_tail` (host and daemon killed, torn tail appended); `continue_while_a_run_records_is_refused_by_name`; `a_second_start_replaces_the_first_run` (stand-in: first heartbeat answers stop/replaced; first host seals and uploads).
- [ ] **Step 2–4:** `cargo nextest run --offline -p goalcup-cli --test full -E 'test(/killed_agent|continue_|replaces_the_first/)'` FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "continue: a stopped agent restarts as a new segment of the same run"`

### Task T8: `goalcup prep`

**Files:** Create `commands/prep.rs`; modify `main.rs` (`prep [--harness ID] [--json]`), `commands/doctor.rs` (export the per-harness check), `launch.rs` (`prep_launch`: the bare harness, no proxy, the same home env as a run). Test: `tests/it/prep.rs`.

**Interfaces:**
- Consumes: A1, A2, P1, P2, T2, T6.
- Produces: `<cfg>/prep.json` `{harness, repo_url, work_root}`. Six steps, each printed `[ok]`, `[todo] <what to do>` or `[fail] <why>`, re-runnable, exit 0 only when all are ok: (1) login; (2) harness picked from shipped interactive connectors, installed, version shown; (3) harness home: `prep_launch` opens the harness in its tracer home so the participant logs in and adds settings, MCP servers and skills; (4) GitHub (A2 check, token stored); (5) machine: AC power (todo on battery), free space ≥ 3 GiB + 5 GiB on the artifact root (fail below 4 GiB), server and connector hosts reachable (`proxy::preflight::upstreams_reachable`), `git` on `PATH`, WSL2 work root not on a Windows drive, sleep lock available or the Windows power-settings sentence; (6) practice: ok when a practice `RunRecord` of this harness has every entry sealed and uploaded, else todo `goalcup start --practice` (prep never starts a run). `--json` prints `[{"step","state","human","say"}]` for the agent skill.

- [ ] **Step 1: Failing tests:** `prep_reports_each_step_and_exits_1_until_all_are_ok`; `a_second_prep_skips_passed_steps`; `prep_fails_a_wsl_work_root_on_a_windows_drive`; `prep_does_not_start_a_run`; `prep_json_marks_the_human_steps`.
- [ ] **Step 2–4:** FAIL → implement → PASS; gate PASS.
- [ ] **Step 5:** `git commit -m "prep: one command from install to a practice run"`

### Task D1: The 10-minute matrix and the edge cases (20–24 Oct)

**Files:** Create `scripts/matrix.sh`; modify `docs/OPERATING.md`, `docs/harnesses/*.md`.

- [ ] **Step 1:** `scripts/matrix.sh <connector> <server-url>` runs a practice run against staging, waits for the server stop, runs `goalcup-verify --capture-integrity` on the server copy, and prints `<connector> <os>-<arch> <harness_version> PASS|FAIL <date>`. Check with `sh -n`.
- [ ] **Step 2:** Run it on macOS arm64, macOS x86_64, Linux x86_64, Linux arm64 and WSL2; record the rows. By hand, per `docs/OPERATING.md`: 30 min offline, laptop sleep, agent killed → `continue`, disk nearly full (small loop volume), reboot → `continue`, tokens run out, window closed → `continue`, a human message other than "keep going" is in the session file, the deadline stop, a second start replaces. Run two 8-hour runs on staging (macOS, Linux). File failures to the owning lane.
- [ ] **Step 3:** `git commit -m "scripts: the run-lifecycle matrix and edge-case runbook"`

### Task D2: Docs, explainer, agent setup skill (24–25 Oct)

**Files:** Create `docs/PARTICIPANT-GUIDE.md` (prep, start, leave, `status`, `continue`, the "keep going" rule, what is recorded, WSL2 power settings), `skills/goal-cup-setup/SKILL.md` (an agent loops on `goalcup prep --json`, does each non-human step, and stops with the `say` text at each human one). Modify `README.md`, `TODO.md`, `docs/RULES.md` (R3 Git workspace; one main run per account; human messages other than "keep going" remove a run at review; idle permitted), `docs/SERVER-DESIGN.md` (pointer: the lifecycle spec wins).

- [ ] **Step 1:** Write the pages; every command named exists in `goalcup --help`.
- [ ] **Step 2:** `python3 scripts/check-links.py --root .` → no broken links; gate PASS.
- [ ] **Step 3:** `git commit -m "docs: the participant's path and the setup skill"`; tag on 26 Oct after `sh scripts/gate.sh full`.

---

## Website team brief

1. Adopt (rename freely, tell us) from `server/worker/sql/proposed-goalcup-functions.sql`: `device_requests`, `run_credentials` (replaces `tokens`), `runs.mode`, `runs.replaced_at`, `model_requests`, `last_model_request_at`, `flagged_at`, `silence_notice_for`, `idle_notice_for`, `final_commit`; `entries` one per segment; `heartbeats.recording`; a `notices` outbox; the `upload_*` functions the Worker calls.
2. `goalcup_approve_device(user_code, member_id, approve)`: the only way a credential is minted. It refuses expired or decided codes, and a member without a Goal Cup sign-up or current terms and privacy acceptance.
3. Approval page at `APPROVAL_URL?code=XXXX-XXXX`: sign-in; sign-up and terms if missing, then back to the same code. It shows login, practice or competition, the harness, and "this replaces your current run" when one is open, with Approve and Deny.
4. `events`: the competition (window opens 6 Nov, 24 h, 168 h runs), the dry run (28 Oct), an always-open practice event (10-minute runs).
5. Mailer: send each `notices` row once. `silence`: "your run stopped; run `goalcup continue`". `idle`: "your agent made no model calls for 6 hours: is it finished, out of tokens, or offline?". Never after a run's end.
6. A member page: their runs, segments, last heartbeat, flags, final commit.
7. Terms and privacy: raw traces are write-once, no deletion on request; only the main run counts; human messages other than "keep going" are reviewed.
8. Staging by 19 Oct: same schema, `STAGING=on` and a `TEST_APPROVAL_KEY` on the staging Worker only; R2 staging bucket on the same account.

## Self-review

- **Spec coverage.** §2 accounts and replace → W2, T7; login per run → W1, A1, T5; harness freedom → T6, D55; platforms and WSL2 → P2, T5, T8; records only → T6 (no input, no relaunch); keep awake → P2, T4; disk → P1, R1, R2, T2, T3; timing → R3, T1; emails → W3, T4; deadline and final commit → existing path + W3; data → website brief. §3 prep → T8, A2; explainer and agent skill → D2; device flow → W1, A1, T5; chat-mode launch in the clone → T5, T6; push token to git only and redaction → A2, R5. §4 terminal → owner: none; closed window → T6 SIGHUP rule + T7; sleep → P2; harness exit → T7, W3. §5 → R1, R2, R3, T1, T2, T3. §6 → T4, W3. §7 → W1–W3. §8 → each task's tests + D1. §9 → T6 facts sheets.
- **Owner overrides applied.** Rotating parts (R2, T3), no terminal host (none of pty, relay, attach), identity = every session file new or written since the run started (R4, T6). Module counts were not optimized.
- **Types checked:** `ChainPart`, `chain_parts`, `part_file_name`, `WorkspaceGit`, `ReserveFile`, `RunReserve`, `DiskVerdict`, `RunRecord`, `RunState`, `Recording`, `RunCredential`, `Approved`, `Purpose`, `AckAges`, `CHAIN_QUOTA_BYTES`, `ends_at` are each defined once and used under the same names.
- **Gaps accepted:** a torn tail ends a segment as incomplete rather than being repaired. `MAX_CONNECTION_FAILURES` now counts consecutive failures (T4).
