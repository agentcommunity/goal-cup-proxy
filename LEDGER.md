# LEDGER: M2 work tracker

Updated: 2026-10-12.

The ticket list for `m2/wire`: what exists, what is broken, what comes next. One line per
ticket: status, ID, phase tag, SHA, a pointer. Where things stand overall: [STATUS.md](STATUS.md).
Commit hashes and branch names cited here refer to private development history not included in
this repository.
The long bodies of the 2026-10-02 ledger are the private historical record discussed here.
The supporting private historical record is intentionally omitted from this public OSS cut.

**Status:** ✅ done · 🔨 in progress · 🟠 done but partial · 🐞 done but wrong ·
⬜ to do · ❓ needs a maintainer decision · 💤 later (after the pilot)

**Phase tags:** `[B]` fix, `[C]` simplify, `[D]` forward to T23, `[OS]` open-source lane,
`[pre-tag]` required before `v0.5.0`, `[post-tag]` retained work that does not block `v0.5.0`,
`[next]` after T23, `[💤]` after the pilot. Every open ticket carries one.

**Rules for agents:**
- Change a ticket's status in the same commit as the work, and add the SHA when it lands.
- New items take the next free ID in their section. Never renumber, never drop an ID.
- Open ticket: 4 lines at most. Done ticket: 1–2 lines, SHA, record link. Detail goes to the record.
- Cite git-ignored paths (`.superpowers/`, `target/`) only as "local, not in git".

**v0.5 integration cadence:** each lane keeps focused failing-first and acceptance checks plus an
independent review/rereview. Those checks are not a lane-level full gate. The integrator merges a
whole wave one lane at a time, then runs the one four-command gate (README "The gate") once on
the wave head. A timing failure seen
only under concurrent load is rerun alone; only an isolated reproduction blocks. Every lane report
includes Harmonization: module/glossary fit; replacement and remaining duplication; shims and their
owners; docs/integrator deltas; cheap duplication removed.

**Where detail lives:**
- Included public evidence records: `docs/qualification/` (one record per measurement)
- Included public contracts: `docs/CONTRACTS.md`
- Private historical task-packet records intentionally omitted from the public OSS cut:
  `docs/superpowers/plans/2026-09-25-m2-recorder/<ID>.md`; index
  `docs/superpowers/plans/2026-09-25-m2-recorder-implementation.md`
- Private historical plan records intentionally omitted from the public OSS cut:
  `2026-09-29-m2-continuation-plan.md`, `2026-09-30-m2-real-agent-goal.md`,
  `2026-10-01-m2-next-phases.md` (all under `docs/superpowers/plans/`)
- Included public decisions authority: `docs/DECISIONS.md`. Full old ticket bodies and owner quotes
  belong to the private historical record omitted from the public OSS cut.
- Local, not in git: review reports `.superpowers/reviews/2026-09-29-codebase/` (`r2-I1` = seat
  r2, finding I1); planner reports `.superpowers/sdd/`.

V1 on `v1/trunk` (the private V1 repository's `main` at `0c5f477`) is covered in
[STATUS.md](STATUS.md), not here.

---

## 0. Current goal

- **Capability** Reviewed v5 raw capture, own-session native discovery, physical reservation,
  local finish/restart and isolated homes are merged. Paired short real rows pass on Codex,
  Claude (`3c542aa5`) and Hermes (row 1, `8e1d8344`). Long duration and acknowledged delivery
  are not qualified.
- ✅ **G-3** Short real-agent goal COMPLETE (C1/C2/C3), final audit accepted. Installed `3c542aa5`
  (reviewed source `112560a5`); preserved Codex/C3 rows on `70e8d129`, useful Claude `e00964ce700aa5043`,
  useful Codex `e00f8daf7d0d8a5c8`. [Record](docs/qualification/m2-real-agent-progress.md);
  partner guide `550c06f4`.
- ✅ **G-2** Fix, simplify, carry M2 through reviewed T23. Whole-branch fixes `6ba5a224`; one full
  Part B PASS on `3c542aa5`, both scoped reviews approved.
  [Record](docs/qualification/m2-synthetic-vertical.md).
- ✅ **G-1** Reviewed E01, then wave 2 (handoffs dated 2026-09-28 in `docs/superpowers/handoffs/`).

## 1. M2 plan tasks

### Built and merged

- ✅ **T00** Contracts bootstrap. `f0b05bb`.
- ✅ **T01** Allocation proof. `494a0e3`. APFS only, `F_ALLOCATEALL|F_ALLOCATEPERSIST`.
  [Record](docs/qualification/m2/reservation.md).
- ✅ **T02** Launch proof, both harnesses take an automatic goal. `1c0774f`.
  [Recipes](docs/qualification/m2/launch-recipes.md).
- 🟠 **T03** v5 writer and local finish. `89587eb`. Seal ordering fixed `0d4457c` (F-04);
  the repair half is 💤 T25.
- ✅ **T03R** Physical reservation on the real chain file. `49ad41a`. `seal.json` is outside it.
- ✅ **T04** Streaming redaction, fail-closed past the 16 KiB hold. `51a5fd8`. F-07 unreached.
- ✅ **T04b** Extra connector redaction patterns on M2 (D39). `a3b1ebd`.
- ✅ **T05** Native file capture. `530b6ea`.
- ✅ **T06** Daemon ingest receiver, TCP loopback, with CP-A4. `e7f006e`.
- ✅ **T09** TLS and CONNECT. `0efb83d`.
- ✅ **T10** HTTP relay. `557338f`. Opaque-fallback capture stop fixed `02bf0f1` (F-05).
- ✅ **T11** WebSocket relay. `b5937d1`.
- ✅ **T12** Capture sender over IPC. `99d25a0`.
- ✅ **T13** Proxy composition. `856f376`. Connection cap fixed `02bf0f1` (F-06).
- ✅ **T14** Harness profiles, headless launch, pins. `87153ff`.
- ✅ **T18** Integrity reader, exits 0/1/2/3. `c7ce7cc`, CP-A4 `f9d0e00`; F-01/F-02/F-10 `5c9a384`.
- ✅ **T19** Native decoders. `2ca83dd`. Human-by-position labelling fixed `8487788` (F-03).
- ✅ **T20** OpenAI decoder. `12f945f`.
- ✅ **T21** Anthropic decoder. `41f05e5`. Unbounded `tool_result` fixed `8487788` (r2-M11).
- ✅ **CP2** Sender and receiver composed (T13 → T12 → T06). `d40248a`.
- ✅ **E00** Component capture, real TLS/HTTP/WebSocket. `af510be`.
  [Record](docs/qualification/m2-component-capture.md).
- ✅ **E01** Early live capture, both real harnesses. `6ecaf7d`.
  [Record](docs/qualification/m2-early-live-capture.md).
- 🟠 **E02** Early unattended run. `c073026`. Codex scored partial on an unstated rule; the frozen
  cross-check failed 2 of 6 (F-16, T27). [Record](docs/qualification/m2-early-unattended.md).

### Still to build, in dependency order

- ✅ **T15** `[D]` Local daemon lifecycle (v5 create/reserve, ingest, Launch, local finish). `5044ba4`.
  Manual v5 resume with Assistance stays refused (reopen at T25).
- ✅ **T22** `[D]` Research report built from the decoders. `a6b7234`.
- ✅ **T16** `[D]` `goalcup start --goal-file` and `seal` over v5, proxy in-process. `bf761bc`.
- ✅ **T23** `[D]` Synthetic production vertical, merged `4b0d40af` (`128aee5`); D3 green `b2d1991`.
  Part B `load-20261001T071828Z` PASS. [Record](docs/qualification/m2-synthetic-vertical.md).
- 🟠 **T24N** `[short real qualification measured; hardening deferred]` Own-session native binding.
  d8 (`d8d5b1cf`) failed rows are preserved ([Codex](docs/qualification/m2-installed-codex.md),
  [Claude](docs/qualification/m2-installed-claude.md)); `3c542aa5` rows pass.
- 🟠 **T24** `[local shipping and short real qualification measured]` Private three-binary
  m2-pilot; d8 rows FAILED ([record](docs/qualification/m2-live-capture.md)); `6d92adc6` default
  Codex FAIL; `3c542aa5` paired tiny + useful Claude PASS
  ([record](docs/qualification/m2-real-agent-progress.md)). Unsigned.
- 🔨 **T25** `[next]` Fault campaign. Storage slice (4 rows, `74d07df9..a35c93c8`) and crash slice
  (4 rows, `d4e9ed43..8ce2b8d2`) PASS, reviewed ([storage](docs/qualification/m2-failure-campaign.md),
  [crash](docs/qualification/m2-crash-campaign.md)). Open: H-11, literal ENOSPC on a disposable
  volume, live-receiver TCP close, RSS. Readmission refusal is proven in-process only.
- ⬜ **T26** `[next]` Research acceptance with independent annotations. Needs T22 and T24.
- ⬜ **T27** `[next]` Sustained qualification: unattended outcomes plus an eight-hour load.
  Needs E02, T25, T26. Owns F-16.
- ⬜ **T28** `[next]` Local pilot release: a qualified three-binary artifact. Needs T27.
- 💤 **T07** Signed delivery. 💤 **T08** Detached uploads. 💤 **T17** Rich status (browser, MCP).

---

## 2. Fixes: wrong or unsafe behaviour (2026-09-29 review)

- ✅ **F-01** `[B]` `5c9a384`. Reader binds `baseline.json` to its chain digest (r2-I1).
- ✅ **F-02** `[B]` `5c9a384`. Exit 0 needs a Launch bound to a complete goal stream (r2-I2).
- ✅ **F-03** `[B]` `8487788`. Human labels need a positive marker, not position (r2-I3).
- 🟠 **F-04** `[B]`/`[💤]` Seal ordering ✅ `0d4457c` (`seal.json` before the terminal). Repair
  half 💤: `repair_seal` refuses every v5 terminal; T25 reopens it only on a real crash (r2-I4).
- ✅ **F-05** `[B]` `02bf0f1`. Opaque fallback now counts a loss and chains a `DecodeFailed` gap.
- ✅ **F-06** `[B]` `02bf0f1`. Connection cap fails open under `capture_table_full` (r1-I2).
- 💤 **F-07** `[💤]` A base64url run of 16 KiB or more redacts the rest of a one-line JSON body.
  Premise measured false on the pilot path; parked 2026-09-29; re-measure on every T24/T27 chain.
- ✅ **F-08** `[B]` `02bf0f1`. Labels truncate after redaction, not before (r1-M1).
- 💤 **F-09** `[💤]` The sender never authenticates the receiver (r1-M3). Single-user host;
  residual recorded in DECISIONS (H-05).
- ✅ **F-10** `[B]` `5c9a384`. v5 lines must be canonical (r2-M3).
- ✅ **F-11** `[B]` `47af44b`. `clock_skew_seconds` is read through `Uploader::new` (c1-I2).
- ✅ **F-12** Closed, not a bug: `verify_strict` rejects weak keys at all five sites (c3-I5).
- 💤 **F-13** `[💤]` `DaemonError::Refused` has 455 uses; a diagnostics taxonomy after the pilot (c1-I9).
- 🟠 **F-14** `[B]`/`[💤]` Review minors, triaged. Fixed: r2-M1 `5c9a384`, r2-M7 `0d4457c`,
  r2-M11 `8487788`, r1-M5 `02bf0f1`. The remaining items were parked with rulings, dropped or
  moved.
  The supporting private historical record is intentionally omitted from this public OSS cut.
- 🟠 **F-15** `[B]`/`[💤]` Wave-2 minors, triaged. Closed: T03R N1.1 `0d4457c`, T12 N-2 `02bf0f1`.
  N1.2, N2.4 and the slow-owner case went to T25.
  The supporting private historical record is intentionally omitted from this public OSS cut.
- ⬜ **F-16** `[next]` Replace the E02 cross-check matcher before T27 (start-anchored match instead
  of end tolerance). Corrected script `MATCH_TOL_S = 0.05` is pre-registered for T23 Part B.
  Script only.

---

## 3. Simplify: less code, clearer code (no new features)

- ✅ **S-01** `[C]` Comments rewritten in the present tense at gate C1: `80171f9`, `2408f58`/`cb33559`,
  `55b9bed`/`fa39b41`/`6f8c07f`, `15368c8`/`3b33b0b`/`0652f6f`. `D##` cites may stay.
- ✅ **S-02** `[C]` Dead code removed: daemon `ee299fc`, CLI `f075080`, libraries `115cca2`.
- 🟠 **S-03** `[C]`/`[next]` Test-hook audit prose cut `fa39b41`; the `testing` module
  consolidation (S-03b, eight T15 files) after T23. c1-I1.
- 🟠 **S-04** `[C]`/`[next]` Shared m2 test support `0652f6f` (S-04a); the five C2 oracles stay
  local (S-04b after T23). c2-I6.
- 🟠 **S-05** `[C]`/`[next]` `acknowledgement_delta` deduplicated `ee299fc`. `constant_time_eq` is one
  copy now, in `goalcup-protocol::constant_time` (daemon control router and CLI proxy both use it;
  `ee299fc` had left two). Still duplicated: `private_dir` ×2 (`per_run_home.rs`, `start.rs`),
  `read_fixed` ×4, `sha256_hex` ×2 (`daemon/workspace.rs`, `verifier/legs.rs`). Chain walkers, unsafe
  `readdir`, `trace.toml` schemas remain. c1-I5, c3-I3/I4/I6.
- 🟠 **S-06** `[C]`/`[next]` CLI command flow. Retry duplicate removed `f075080`; the
  `resume_with` drop-guard, one retry loop and the shared start/resume spawn remain (opus, serial).
- 🟠 **S-07** `[💤]` Lite ✅ `115cca2`. The verifier's M0 legacy input path stays until the
  maintainer retires the two frozen-baseline rows. c3-I2.
- ✅ **S-08** `b4954f64`: framing/budget/close, lineage; adapter kept; 2/3/2 fixtures intentionally
  regenerated; v5 corpus unchanged. No qualification, live re-decode or publication claim.
- ⬜ **S-09** `[next]` Remove user-facing jargon from `--help` and error text. c2-I7, c1-M1.
- ⬜ **S-10** `[next]` Stop tests sleeping through the 12-second retry schedule (needs a clock
  seam). c2-I8, c2-M10.
- 🟠 **S-11** `[C]`/`[next]` Items a, b, c, g done `6f8c07f`/`2408f58`/`cb33559`. Daemon twins, CLI
  leftovers remain. `tungstenite` → `sha1` + `base64` done (the accept key, RFC 6455 sample as a
  test); stale `__proxy` and tungstenite Cargo comments fixed; devserver `serde_json` is a
  dev-dependency.
- ✅ **S-12** Decided by D42 (2026-10-04): the `Platform::Windows` and `cfg(not(unix))` stubs stay,
  labelled "not yet supported; Windows is planned" (`goalcup-platform` still refuses to compile on a
  non-unix host); Linux is best-effort until tested. Nothing is deleted.
- ✅ **S-13** Deleted the E00–E02 experiment harness (about 9,900 lines: the `m2_capture_experiment`
  example, `m2_capture_support/*` and `tests/m2_component_capture.rs`). `wire.rs` and `endpoint.rs`
  moved to `crates/goalcup-cli/tests/m2_vertical_support/`.

---

## 4. Open source readiness (c4 review; verdict: not ready)

- 🟠 **OS-01** `[OS]` Public repo cut: v0.5.0 is the single parentless commit `d2b3206`, an
  allowlist copy; development history is kept in a
  separate private repository. The in-tree fixture scrub (vendor prompts,
  re-pinned digests) is not evidenced here.
  [Record](docs/qualification/v0.5.1-public-release-cleanup.md#source-release).
- 🟠 **OS-02** `[OS]` Personal data. OS-02a (prose) done `04ec186`. Decided 2026-09-29: accept the
  generic home path in hash-pinned fixtures, scrub prose only, and drop the obsolete private
  project identity residue.
- ✅ **OS-03** `[OS]` `04ec186`. Build prerequisites documented; `--offline` works on a warm cache.
- 🟠 **OS-04** `[OS]` `docs/RULES.md` corrections. OS-04a done `04ec186`; OS-04b
  (`docs/TRACKER-CONTRACT.md` from the shipped tree) after T23. c4-C3.
- ✅ **OS-05** Closed 2026-10-11 by D48: the V1 walkthrough and `real_daemon_end_to_end` were
  deleted with the V1 path (OS-30); nothing is left to re-execute.
- ✅ **OS-06** `[OS]` Allowlist copy published as public v0.5.0 (`d2b3206`). c4-I2/I3.
  [Record](docs/qualification/v0.5.1-public-release-cleanup.md#source-release).
- ✅ **OS-07** `[OS]` Private reporting on, `main` ruleset requires `check` (2026-10-11), no DCO.
  [Record](docs/qualification/v0.5.1-public-release-cleanup.md#applied-2026-10-11).
- ⬜ **OS-08** `[next]` Re-state that `m2_native_observations` fixtures passed redaction in the
  scrub notes before the public cut. c4-M10.
- ⬜ **OS-09** `[next]` Decided 2026-09-29: keep `goalcup-git` in the tree, exclude it from the
  public allowlist (`3a4672d` kept the directory off the workspace). Prune its Git/publisher half
  after M2.
- 🟠 **OS-10** `[OS]`/`[next]` Minors. OS-10a done `04ec186`; OS-10b Cargo metadata after Phase D;
  OS-10c done by D48: `rusqlite` and `age` left the tree with the V1 path (the next SQLite
  harness is a separate core PR); OS-10g README to about 200 lines after T23.
- ✅ **OS-11** `[OS]` D42 gate (branch `m2/v05-v1-gate`): the V1/v4 path compiled only with
  `v1-compat`. Superseded by D48: the gated path was deleted (OS-30).
- 🔨 **OS-12** `[post-tag]` Seam refactors. Nothing is merged. Local branch-only coordination
  state: Seam-D has three branch commits plus an uncommitted continuation; Seam-R and Seam-FM
  have two branch commits each; Seam-C, Seam-CLI, Seam-T-daemon and Seam-T-cli are not started.
  Resume after `v0.5.0`; no seam is a tag prerequisite.
- ⬜ **OS-13** `[post-tag]` S5-shims: remove the `capture.rs` status re-exports, move the unfrozen
  ingest seam to `capture_ingest.rs`, migrate verifier test materializers, and finish the named
  import/test splits. Needs OS-18, Seam-CLI, OS-26 and Seam-T-daemon; does not block `v0.5.0`.
- ✅ **OS-14** S4a daemon harness loading and pin enforcement. Lane `6298b8aa`, merged
  `783b05a9` then integrated by `b3c4211d`; covered by the W1 gate at `9fc730e6`.
- ✅ **OS-15** Merged `b4954f64` (lane `1059d16b`): `Pinned`, `Dev/unpinned`, or absent/unbound.
  Presentation-only; no frozen-v5 `Origin` and no integrity downgrade.
- ✅ **OS-16** PH-A `cf05d4fe` (`652e1090`): evidence/rereview approved; D43/glossary landed.
  W1 PASS at tested `9fc730e6`; evidence is local, not in git.
- ✅ **OS-17** S4b CLI harness loading and capture-family retirement. Lane `e0807b3b`, merged
  `8a1d0b64` then integrated by `b3c4211d`; covered by the W1 gate at `9fc730e6`.
- ✅ **OS-18** S5 family retirement derives V1-file allowance from the two retained V1 manifests.
  Lane `11f6bf17`, merged `9b29ce10`; covered by the W1 gate at `9fc730e6`.
- ✅ **OS-19** S6 offline kit had four synthetic cases (six since PR #8's two keyed cases). The two
  data-only case commits are `f9cd4288` and `8e5596d0`, not a fourth source connector or a third
  package profile; lane `1affe6ac`, merged `b047d854`, covered by the W1 gate at `9fc730e6`.
- ✅ **OS-20** C2-0700 private configured roots. Lane `4d72f1d4`, merged `31e9f04f`; covered
  by the W1 gate at `9fc730e6`. The 0600 mirrors, SAFETY comments and lint remain OS-26.
- ✅ **OS-21** S8 schema, contributor, architecture/security, governance and package guidance
  merged as `2ec63430`, `14843a91`, `2ac5e7ec`, `bc789f7d` and `cc52f967`; focused evidence
  passed. The exact-head whole-tree gate remains Step11; these merges are not the tested W1 head.
- ✅ **OS-22** OSS-cut link repair: 0 broken links on the exact public v0.5.0 tree (`d2b3206`).
  [Record](docs/qualification/v0.5.1-public-release-cleanup.md#local-validation).
- ⬜ **OS-23** `[OS]` Outsider Gemini proof: an outsider agent enters through only the shipped
  skill for the Gemini scenario and the sources it names, with no hidden maintainer context; any
  fix becomes a reviewed wave and the proof repeats. The skill shipped (H-32); its one
  [trial](docs/research/harness-onboarding-trial.md) was by its author. Evidence not yet linked.
- ⬜ **OS-24** `[OS]` Fresh-clone proof of the public cut: source identity, link check,
  dependency-fetch boundary, build/test commands and results, without the private worktree or its
  `target/`, saying whether the dependency cache was warm. Evidence not yet linked.
- ✅ **OS-25** `v0.5.0` published 2026-10-10 as source only at public `d2b3206`, no attached files;
  no binary or qualification. Published before OS-23/OS-24 closed, with no exact-head gate (no
  check runs on `d2b3206`). [Record](docs/qualification/v0.5.1-public-release-cleanup.md).
- ⬜ **OS-26** `[post-tag]` Remainder of C2: 0600 status mirrors, missing daemon SAFETY comments
  and `clippy::undocumented_unsafe_blocks`. Starts after Seam-D and Seam-FM; does not block
  `v0.5.0`.
- ⬜ **OS-27** `[post-tag]` C3: V1 env-deny validation, exact `OTEL_LOG_*` allowlist, interpreter
  policy, doctor notice and CLI SAFETY comments/lint. Starts after OS-18 and Seam-CLI; does not
  block `v0.5.0`.
- ⬜ **OS-28** `[post-tag]` Tail cleanup: the undated C1/C3/C5/C10 cleanup and the `CaptureSpec`
  rename. Starts after every other post-tag lane; changes no frozen record shape.
- ⬜ **OS-29** `[post-tag]` Remove the build's retained V1 manifests parsing dependency. This does
  not block source-only `v0.5.0`.
- ✅ **OS-30** D48 (owner, 2026-10-11), merged `975075d` (PR #5): the `v1-compat` path, its
  tests, fixtures and `open`/`rusqlite`/`age` deps are deleted; devserver stays.
- ⬜ **OS-31** `[post-tag]` D48 follow-up: move `ENTRY_WINDOW_SECONDS`, `DEV_SERVER_SEED` and
  `dev_server_public_key_hex` out of `goalcup-devserver`, drop the daemon's dev-dependency on it
  (tests use it only for a placeholder key), consider deleting the crate, and remove the V1-only
  registry functions D48 lists under "Kept" with their tests.

---

## 5. Housekeeping and open questions

- ⬜ **H-01** `[next]` Decided 2026-09-29: the user deletes `/tmp/test.db` (left by the E02 Claude run).
- ⬜ **H-02** `[next]` Decided 2026-09-29: push only after reviewed T23 with the walkthrough
  re-executed, into the fresh public repository (OS-01). Source is published; the walkthrough
  re-execution (OS-05) was closed by D48, which deleted it.
  [Record](docs/qualification/v0.5.1-public-release-cleanup.md#source-release).
- ✅ **H-03** `[next]` Done 2026-10-02, merged `23756d71`, independently reviewed APPROVED. Five causes:
  four test/fixture defects (native-pid rename guess, 400 ms Busy poll, verify before ack, inherited
  zombie) made deterministic (`e81fed7b`, `93777d1f`, `f5287bfb`, `7e029c27`, `a6300169`) and one
  fail-safe product race: a root exiting between `guard_registered`'s two reads is absent, not
  recycled (`32bc150b`). Repeat runs 5/5+ per binary; load test 90/90 (was 5/30 failing). Minor
  leftovers: TERMINATION.md Lost row and the phase-A "always Rooted" line. Unreproduced api_gate
  flake split out as H-25.
- ⬜ **H-04** `[next]` Open assumptions: CP-A4 assumes one proxy per entry; the 8 MiB queue holds
  about 2 MiB of payload (R6); `seal.json` sits outside the reservation; the network-filter rule is
  bound to the E01 binary hash; the create response carries the ingest endpoint (R5, H-05).
- ✅ **H-05** `[B]` `cc8ac43`. CONTRACTS and packet amendments for the Phase B changes.
- ✅ **H-06** `[C]` `0652f6f`. Trap installed before the fork (term-ignoring descendant test).
- ✅ **H-07** `[C]` `964a062`. `finalization_recovery` pause wait fails instead of hanging.
- ⬜ **H-08** `[next]` `connectors/claude-code-cli/manifest.toml` says the daemon creates
  `raw-bodies`; the code and `docs/DOGFOOD.md` say the harness does. Fixing it re-pins manifests.
- ✅ **H-09** `[next]` `35b5f2a0` + `db8bbc94`, rereviewed APPROVED 2026-10-01. Connections bounded
  and ownership-checked; Recovery names only a wire or true zero-record predecessor. Residual:
  sink and writer classify CP-A4 orphans differently with two live wire producers.
- ✅ **H-10** `[next]` Merged `c20d9e92`: 4a `cfed3b09`+`c371675c`, 4b `6b999eee`+`b4a249eb`+
  `5a170e38`, record `5227ddb8`. One finish budget (drain + commit reserve), pre-exec spawn gate.
  [Record](docs/qualification/m2-h10-finish-and-spawn-gate.md). Residual: a CLI crash after release
  plus a raced manual seal (two faults) can leave a harness running on a sealed entry.
- ⬜ **H-11** `[next]` T25: reservation stays allocated after seal. Only grow-then-shrink frees
  APFS blocks and has a torn-tail window. Ruled D40: release deferred; a separate-reservation
  design that never touches the chain is due before T27. Fixed handles/status `6ba5a224`.
- ⬜ **H-12** `[next]` User-deferred testing on the T25/T27 backlog: recovery/counter exit-1
  coverage, bare-token scanner expansion, finer frontier/RSS sampling, vendor deny/ask matrices,
  broader CSV edge cases. No extra hardening tests now.
  The supporting private historical record is intentionally omitted from this public OSS cut.
- ✅ **H-13** `[next]` Stopped 2026-10-02 with owner OK. The three stray daemons (PIDs 52294,
  69232, 95284) were launchd evidence installs; all 10 entries sealed; traces cloned to the
  local, not-in-git evidence archive; `launchctl bootout`. Teardown gap → H-19.
- ✅ **H-14** `[next]` `3fd8b6ac`, merged `b6fa0018`. Finish re-reads `control.json` once after a
  daemon restart.
- ✅ **H-15** `[next]` `8b1a411e`, merged `b6fa0018`. `goalcup seal` on a torn entry exits 1 with a
  plain message.
- ✅ **H-16** `[next]` `76e88bad`, merged `c20d9e92`. The proxy chains each loss as a stream-zero
  `Loss` frame (stream-less `Gap`); a refused Loss leaves the chain incomplete. Residuals: old CLI
  on a new daemon keeps old behaviour; gap counts count records, not losses; handled finishes show
  doubled daemon-clocked gaps. Reconsider the 2 s reserve if a release run exceeds 500 ms.
- ✅ **H-17** `[next]` `164d1dbb`. Incident 2026-10-02: a Lane A test created empty
  `~/.hermes/sessions` in the real HOME; root removed it. Tests now run in a child with a
  private HOME and check the real HOME before and after.
- 🔨 **H-18** `[next]` Hermes Lane B residuals. Done: key-scan verdict bound to the trace chain
  sha256/bytes and run-dir listing `b133d9af`; `check_verdict` returns Current/Stale/Unbound/
  Missing but no retention step calls it; iCloud Desktop/Documents goal files refused when a local
  signal is present `c752517d`. Open: fragments the redactor removed are invisible to the decoded
  scan; models.dev is contacted; marker timing tested only with a fake late marker; an in-place
  rewrite with the old mtime restored is not detected.
- ✅ **H-19** `[next]` `b93cd27d` (lane `7e385f60`). Evidence installs keep a persistent launchd
  agent, now crash-only with `ThrottleInterval = 10`; teardown names only the owned/recorded label.
  Policy and shell checks are static; no launchd or installation lifecycle was measured.
- ⬜ **H-20** `[next]` Redaction residuals: invalid UTF-8 inside a key body leaves the key raw (fix
  before any raw-byte capture surface); a lone BEGIN redacts to end of record and breaks its JSON.
- ⬜ **H-21** `[next]` `M2_PILOT_CONNECTORS` in `scripts/package-release.py` lists only `codex-cli`
  and `claude-code-cli`; the m2-pilot bundle cannot run a Hermes row. Add `hermes-cli` (with the
  installer's manifest-hash checks and `test-m2-pilot.sh`) or record why not.
- ⬜ **H-22** `[next]` `crates/goalcup-mcp/Cargo.toml` comment says it links `goalcup-protocol` and
  nothing else; it also depends on `goalcup-platform`. Fix the comment.
- ⬜ **H-23** `[next]` CONTRACTS C2 still says "reject M2 profile extra_patterns in preflight"; R7
  (D39, T04b) ported them later in the same file. C2 needs an owner-approved amendment note.
- ⬜ **H-24** `[next]` `docs/qualification/m2-hermes-readiness.md` cites code as `file.rs:line`;
  those citations rot. Replace them with symbol names, or mark the source SHA they were taken at.
- ✅ **H-25** Closed 2026-10-11 by D48: `Entry::sweep_foreign` and the V1 runtime it lived in
  were deleted (OS-30).
- ⬜ **H-26** `[later]` Opened 2026-10-05 with D45, for the server phase: when a run's stop cannot be
  confirmed (`Unresolved`), record the final pushed Git commit (for example a GitHub commit hash) as a
  "last known state" marker. Open question: which branch counts when the work is not on main.
- ✅ **H-27** Owner ruling 2026-10-07: `DecodeOutcome::undecoded` remains Decoder v1 API only.
  Research-report schema 1 is unchanged and does not serialize it; a future schema 2 projection is
  deferred, not implemented. D45 is unchanged (`bc789f7d`).
- ⬜ **H-28** `[post-tag]` Anthropic usage publication safety. Its private research observations
  currently pass through provider usage objects; before any publisher/research-release consumer,
  structurally allowlist token fields, add private-sentinel non-leak tests and bump decoder version.
  D26 requires plan/credit/billing stripping before publication. Not a PH-V/frozen-v0.5 blocker.
- ⬜ **H-29** `[post-tag]` Shipped ids separate native/goal and wire; reader lacks Open/Connection
  checks. Audit: Connection marks wire; orphan Recovery retires all streams; reader rejects later
  native Segment pre-callback; exit-2 report covers preceding validated prefix. Direct nonvalidating
  DecodeSession consumes it. Post-tag test both-way rejection in CaptureState::apply; not PH-V/v0.5.
- ✅ **H-30** PR #4, merged `7652eda`: the CLI and daemon contract goldens derive their connector
  list from `connectors/`; a new schema-1 connector needs no `.rs` edit (proven in the CLI half).
- ✅ **H-31** PR #6, merged `15f7b43`: Linux script fixes (the key loader reads the mode with BSD,
  then GNU `stat`) and [the portability note](docs/research/platform-portability.md).
- ✅ **H-32** PR #7, merged `4835aa5`: the `adding-a-capture-harness` skill and
  `scripts/check-connector.sh` as the contributor entry point. Outsider proof is OS-23.
- ✅ **H-33** PR #8, merged `8703825`: Amp core prep. D49 `AMP_API_KEY`, D50 keyed `stdout_json`
  identity, `*_URL` env refusal, keyed kit cases; no `connectors/amp-cli`.
  [Record](docs/research/amp-core-prep.md).
- ❓ **H-34** `[next]` Amp key-fingerprint file: Amp writes `.legacy-state-owner.json` with a
  token-derived `identityKey`, which D41 forbids persisting. Owner picks: a setting that stops it,
  a D41 amendment confining it to the per-run directory, or Amp stays blocked. Needs the live probe.
- ❓ **H-35** `[next]` Amp with no local own-session store: if the live probe finds sessions are
  server-only, schema 1 has no fit (one `[[capture.native]]` row required). Owner picks: stay
  blocked, a core PR for a native-less connector, or a server-relay design.
- ❓ **H-36** `[next]` Amp managed settings (machine-wide `managed-settings.json`, server-fetched
  workspace settings and plugins) cannot be isolated: record their presence, refuse to start, or
  disclose as a residual.
- ❓ **H-37** `[next]` Amp token grammar: D49 assumes 20 to 256 bytes of `[A-Za-z0-9_-]`, approved
  before the live probe. A real token outside it is refused by name; widening is a core PR.
- ❓ **H-38** `[next]` Amp identity carrier: the keyed `stdout_json` path (D50) matters only if the
  live probe shows the `init` event arrives before the first model request and names the session.
- ⬜ **H-39** `[next]` Optional hardening, not started: a private `TMPDIR` per capture launch
  (Amp readiness core change 4).
- ⬜ **H-40** `[next]` Settings seed under the run home (Amp readiness core change 6), only if the
  live probe shows Amp needs one. Not started.
- ❓ **H-41** `[next]` Decide the per-target executable pin shape (schema-2 map, or one directory
  per target) before Amp's connector ([portability](docs/research/platform-portability.md), seam 7).
- ⬜ **H-42** `[next]` Post-merge review leftovers: `enumeration.json` hard-codes `manifest_files`
  as `["manifest.toml"]` (false for `hermes-cli`; derive or drop after OS-29/OS-31 decide the V1
  manifests); `check-connector.sh` does not check the kit id or `BODY` (the skill greps by hand)
  and calls a pin diff with non-pin lines connector-only; the six-case count repeats across docs.
