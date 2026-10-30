# PRD-14: Tests and gate

Id: PRD-14. Version: goalcup 1.0. Status: plan for the build, revision 2 (after the three reviews of
30 October). Date: 30 October 2026. Base: main at 4b226de2 (this branch: 7f9d88a8). Index:
[PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md). Main proof: [PRD-12](PRD-12-test-pad.md).

## Goal

Real runs through the rig (PRD-12, ledger B9) prove the seven points, climbing the ladder of ledger B3.
A small set of automated tests guards only the seven points, the record format, masking and the
never-stop rules (ledger B10), where a real run cannot show the failure on every change. One gate script,
two tiers, the same command in a build box and in CI.

## Points served

All seven, each through one pad check (PRD-12 C1 to C6) and a few named guard tests (section 2).
Point 3 also directly: the gate stops tests that pin caps, latches or failure budgets from coming back.

## Owner decisions it carries out

- Ledger B10 (locked): do not overbuild tests. The rig's real runs are the main proof. Automated tests
  guard only the seven points, the format, masking and the never-stop rules. No test pins internal shapes
  or wording. A test that takes longer to change than the code it guards is deleted or simplified. Section
  3 and "What is cut" list what this removed.
- Ledger B9 (locked): one small test rig for agents, which is the pad (PRD-12). The gate never drives a
  harness; the rig does. The rig's selftest (fails on run 1's known bugs) is the one rig test in the gate.
- Ledger B3 (locked): the test ladder (micro tests, cycle runs, full runs) is the real-run layer of
  section 1. The gate is not a rung; it runs on every merge beside the ladder.
- Ledger B11 (locked): every gate run is in a Vercel Sandbox, many at once, one per merge request; this
  session only drives agents.
- No backward compatibility, old crates deleted in the same build (BRIEF3, 29 Oct night): every test of
  the old crates goes with them. Lessons that matter come back as PRD-17 acceptance cases.
- No caps of our own: no test may assert a cap, a latch or a failure budget. Tests assert "past N,
  everything is recorded or a gap is written, and the relay goes on".
- AGENTS.md for 1.0 (ledger section 7): a test stays if it proves one of the seven points. Section 2
  names those tests; any other test may be cut.
- One system for every harness, no patchwork (owner's gateway question; AGENTS.md hard rule "no harness
  names in core code"). A gateway is one more host on the shared list. One code rule (section 6) fails if
  a harness name or a model or gateway host name appears in recorder source, so no per-harness or
  per-gateway patch can slip in.
- Builds, tests and the gate run in Vercel Sandbox build boxes as `ubuntu` (ledger B1, PRD-00 "Where
  the build runs"). The conductor never compiles. The gate keeps its root refusal.
- Every PRD-14 change, the first gate change included, is a merge request on its own `v1.0/prd-14-*`
  branch, read by reviewers A and B before the integrator merges it (ledger B2).
- No speed number fails a gate (PRD-00 rules of the shape). Speed is measured and logged.
- Release: tag `v1.0.0` after the owner's merge (PRD-00 question 5), from the commit the pad verified.
- Early quick real tests during the build; mass harness testing at the end (BRIEF3, 30 Oct early). The
  gate never runs a harness or spends a key.

PRD-00 revision 2 rulings this PRD follows: old crates leave the workspace through `exclude` in phase 0
(no `--exclude` line in `gate.sh`); seam S16 (`--connector`, `--hosts`) is the one connectors override;
the silent-drop hook is cut; `test-hooks` is allowed in `crates/goalcup/Cargo.toml`; PRD-14 alone edits
`gate.sh`; G2 and G5 run after the merge to main; `tests/common/` starts at H+0 in its own lane.

## Today

| What | Where | Lines | Fact |
|---|---|---:|---|
| The gate | `scripts/gate.sh` | 113 | Refuses root unless `GOALCUP_GATE_ALLOW_ROOT=1` (`:32-39`); version check first (`:61`); builds the workspace because CLI tests spawned the sibling daemon (`:63-65`); probes a binary-name filter and falls back to every test when a binary was renamed (`:69-83`); full runs ignored e2e and `upload_worker` by name (`:85-93`); doctests (`:96`); `cargo test` fallback without nextest (`:97-105`); only full runs Worker tests, every script test and the link check (`:107-111`) |
| nextest config | `.config/nextest.toml` | 61 | 30 s slow period, killed at 180 s (`:7`); two long tests start first (`:14-19`); `serial` and `connector-runs` groups (`:25-61`), all naming old binaries |
| Test volume | `crates/*/tests` | 126,112 | cli 59,973, daemon 30,260, verifier 20,724, protocol 13,471, platform 1,684 (`wc -l`, fixtures excluded); plus 16,335 lines of `#[cfg(test)]` in `crates/*/src` and 6,939 fixture lines (`J-tests-gate.md:128-140`) |
| Script tests | `scripts/test_*.py` | 10,393 | 20 files, 577 test methods. 13 of them name `cargo`, `target/`, `crates/` or the daemon (`grep -l`), so several cannot pass once the old crates leave the workspace |
| Version check | `scripts/check-version.py` | 206 + 128 tests | **Fails today**: at 7f9d88a8 it exits 1 with 95 findings, all in `docs/plans/` (rerun 30 Oct). Rule 5 flags any tag on a line about a release (`check-version.py:16-21`). So `gate fast` is red at its first step on this branch, before any code |
| Link check | `scripts/check-links.py` | 1,349 + 1,672 tests | Full tier and CI (`gate.sh:110`, `ci.yml:80`) |
| CI scope | `scripts/ci-scope.py` | 90 + 134 tests | Skips Rust on Markdown-only changes (`ci.yml:60-75`) |
| CI | `.github/workflows/ci.yml` | 93 | "NOT the gate" (`:1-2`); excludes the daemon and CLI suites because they hang a 2-core runner (`:15-16`, `:93`). The repository is public, so `ubuntu-24.04` has 4 CPUs and 16 GB (https://docs.github.com/en/actions/reference/runners/github-hosted-runners) |
| Manual Linux CI | `.github/workflows/linux-x86_64.yml` | 63 | Runs `linux-check.sh` (106, tests 107) and `linux-harness-smoke.sh` (`:26-63`) |
| Release | `.github/workflows/release.yml` | 120 | Builds `-p goalcup-cli -p goalcup-daemon -p goalcup-verifier` on four targets (`:69-71`), packages, smoke-tests and runs `install.sh`, then diffs the installed `connectors/` against the repository (`:72-86`); refuses a tag that differs from `Cargo.toml` (`:55-62`); no Rust test runs on macOS or Linux arm64 anywhere |
| Packaging | `scripts/package-release.sh` 45, `scripts/smoke-release.sh` 55, `packaging/install.sh` 394, `scripts/third-party-licenses.py` 231 | | Three binaries (`package-release.sh:35-37`, `smoke-release.sh:31-37`, `install.sh:9`, `:312-359`, `third-party-licenses.py:24`); the tarball ships `connectors/` from `git archive HEAD` (`package-release.sh:11-16, 38`); smoke judges `goalcup doctor --json` pins (`smoke-release.sh:39-54`), and PRD-10 cuts `doctor`; install guard parses the daemon's signed status (`install.sh:104-195`); two PATH lines (`install.sh:377-389`) |
| Release script | `scripts/release.sh` | 115 | Next-step text names `linux-check.sh` (`:74`, `:110`, `:115`); calls the version check (`:101-104`) |
| Test-only feature | `crates/goalcup-daemon/Cargo.toml:12-21, 76-79` | | `test-hooks`, turned on for its own tests by a dev-dependency on itself. Its comment names the trap: "every other gate command builds dev-dependencies, which turn this feature on for the whole package" |
| Build profile | `Cargo.toml:30-62` | | regex crates at opt-level 3 for tests (`:40-54`); `goalcup-daemon` at opt-level 1 (`:61`). The version comment (`:14-17`) says "the gate fails on any other hard-coded copy" |
| Where builds run | PRD-16 section 1 | | Build boxes: Vercel Sandboxes, 8 vCPU, 16 GB, user `ubuntu`, a build snapshot with nextest, zstd, Node, Python and a warm cargo cache. The conductor (4 CPUs, root) never compiles |

Gate times on record (`J-tests-gate.md:446-458`): fast 54 s quiet, 306 s on a loaded sandbox; full
never measured. Open flake: the port reuse race at about 12 sites, for example
`crates/goalcup-cli/tests/it/m2_proxy_integration.rs:2865-2868` and
`crates/goalcup-cli/tests/full/upload_worker.rs:191-197`; about 30 sub-second wall-clock bounds
(`J-tests-gate.md:471-474`). Run 1's lesson for tests (report on `origin/smart/tender-clarke-qupne3`):
every automated check passed while 37.5% of calls went unrecorded.

## Design

### 1. What proves what

| Layer | Proves | Runs where | Cost |
|---|---|---|---|
| **Real runs (PRD-12 rig, the main proof)** | All seven points on real harnesses, keys and the staging Worker: the proxy record as the proof (no unexplained gap, the history check; the harness's own log only as an optional extra, ledger T4), every subagent, no secret, server copy whole, counts the participant saw. Run as the ladder of ledger B3: micro tests (one harness, one key, local capture first, then one layer at a time), cycle runs per pad (a few minutes), then 20-minute full runs for every pad in parallel | Pad sandboxes, many at once (ledger B11); the owner's Mac after the build | Keys, minutes, money |
| **`gate full`** | One short end-to-end run of the real binary through every part, the real Worker under `wrangler dev`, the shipped binary built and checked for hook text | Build box at phase 2 and 3 exits, before the owner's merge, before the tag | About 15 minutes |
| **`gate fast`** | The section 2 guards: format, masking, never-stop, and the few seven-point behaviours a real run cannot hit every time | Build box before every merge into integration, one sandbox per merge request; CI on every push | Under 5 minutes |

Rule (ledger B10): an automated test is written only where a real run cannot show the failure every time,
and only for the seven points, the format, masking or the never-stop rules. A real run cannot plant every
secret shape split at every byte, cannot kill the recorder at every point, and cannot fill a guard on
demand. It can show everything else, so the rest is left to the rig. A test asserts a promise (a call is
recorded or a gap is written, no secret, bytes equal), never an internal shape, a field order or a
sentence of wording. A test that takes longer to change than the code it guards is deleted or simplified
in the same merge request; reviewer B (code quality and less code, ledger B2) checks this on every diff.

### 2. The seven-point guard list

These are the tests AGENTS.md's new rule protects. PRD-15 writes this table into README in place of P1 to
P7. A guard is deleted only with its point, or replaced in the same change by a test with the same name
prefix that asserts the same promise.

| Point | Real-run proof | Guard tests (fast tier unless marked) | Owner |
|---|---|---|---|
| 1 Every model call | PRD-12 C1; PRD-03 proofs | `proxy_connect.rs` (2,000 connections, no refusal), `proxy_ws.rs` (Codex deflate fixture), `proxy_http1.rs`, `lessons_*.rs` (PRD-17) | PRD-03, PRD-17 |
| 2 Every subagent | PRD-12 C2 | `native_walk.rs` (20,000 files, no cap), `native_tail.rs` (live EOF, truncation, a vanished file gives a GAP), `native_finish.rs` (finish reads everything, no byte cap), `native_resume.rs`, `pipeline_many_streams.rs` (20,000 streams) | PRD-04, PRD-02 |
| 3 Never block | PRD-02 R1 kill test (plain `kill -9`, no hooks) | `pipeline_guard.rs` (every S1 call returns while the writer is held; every drop is a GAP), `pipeline_stall.rs` (real binary, 90 s stall, every client exchange completes), `pipeline_no_latch.rs` (1,000 guard episodes, recording resumes each time), `watchdog_restart.rs` (`kill -9`, same port, connection served), `watchdog_null.rs` (crash loop, connections answered) | PRD-02 |
| 4 No secrets | PRD-12 C3 | `mask_stream.rs` (every split point), `mask_rules.rs` (`url_password`, left guard), `secrets_scan.rs` | PRD-05, PRD-08 |
| 5 Upload everything | PRD-12 C4 | `upload_chunks.rs` (concatenated chunks equal the parts), `server_compare.rs`, Worker `test/integration.test.ts`; full: `upload_worker.rs` | PRD-06, PRD-08, PRD-07 |
| 6 Tell plainly | PRD-12 C6 | `pipeline_guard.rs` (COUNT equals a recount of what was written), `check_invariants.rs` invariant 7 (the verifier's recount), `cli_finish.rs` (the "nothing was lost" claim appears only with zero gaps and zero off time; it checks the claim is there or not, never the sentence), `upload_beat.rs` (`failed` while recording is off) | PRD-02, PRD-08, PRD-10, PRD-06 |
| 7 Real harnesses | PRD-12 table, tiers | `connector_builtin.rs`, `connector_launch.rs`, `rule_code.rs` (section 6), `scripts/test_install.py` (the install never breaks a running recording; no profile or app-wide file written) | PRD-11, PRD-14 |
| Usage stats (ledger S2) | PRD-12 C5 (the main proof) | `usage_run1.rs` only (run 1's record in, model, tokens and status per call and per subagent out). No test pins the output layout | PRD-09 |
| Format (base of 1, 5, 6) | PRD-01 acceptance on the pad | `golden.rs`, `recovery.rs`, `check_invariants.rs` | PRD-01, PRD-08 |
| All together | PRD-00 done items 1 and 2 | full: `e2e_run.rs` (section 7) | PRD-14 |

### 3. Test budget

No test pins a cap, a byte-for-byte golden of internal facts, an internal shape or a sentence of wording
(ledger B10). The on-disk record format (PRD-01) and the upload contract (`docs/UPLOAD-API.md`) are
promises, not internal shapes, so their goldens stay. One file per module; cases named for the promise
they keep. This table is the one source (PRD-00 size table, PRD-13 copies it). Numbers come from each PRD
where it states one, and each is a ceiling, not a target: a builder who can guard the promise in fewer
lines writes fewer.

| PRD | Test files (from its Acceptance) | Lines (est.) |
|---|---|---:|
| PRD-01 format | `zstd_close`, `golden`, `roundtrip`, `recovery` | 800 (estimate here) |
| PRD-02 core | 14 `pipeline_*` and `watchdog_*` | 1,500 (PRD-02) |
| PRD-03 proxy | 7 `proxy_*` plus fixtures | 2,000 (PRD-03) |
| PRD-04 native | `native_*` | 1,000 (PRD-04 expects about 800) |
| PRD-05 masking | 6 `mask_*` plus the copied corpus (data) | 2,000 (PRD-05) |
| PRD-06 upload | `upload_*` plus `upload_worker` (full) | 1,000 (estimate here) |
| PRD-08 verifier | `check_invariants`, `check_named`, `check_conn_bytes`, `check_run1_shape`, `server_compare`, `secrets_scan` (no `judge_view`, deferred) | 1,100 (PRD-08) |
| PRD-09 usage | `usage_run1` only (B10: usage stats are proved by PRD-12 C5 on real runs; `usage_out` and other layout tests cut) | 200 (cut here from PRD-09's 600) |
| PRD-10 CLI | `cli_*` | 1,200 (PRD-10) |
| PRD-11 connectors | 4 `connector_*` | 400 (estimate here) |
| PRD-17 lessons | `lessons_*` | 1,300 (PRD-17) |
| PRD-14 | `tests/common/` (1,480), `rule_code.rs` (80, rule 1 only), `e2e_run.rs` (140) | 1,700 |
| **Rust total** | | **at most about 14,200** (the rows sum to 14,200; 14,750 before the B10 cuts), against 142,413 today (PRD-00 and PRD-13 measure) |

A builder who goes over its line says why in the night log (PRD-00 size rule). Masking and the proxy are
the largest shares: point 4 is the one point a real run can only sample, and the masking corpus is copied
as data, not as test code (PRD-05).

### 4. Test rules (flakes do not come back)

Every closed flake in `J-tests-gate.md:479-499` had one of four causes. Each becomes a rule that reviewers
check on the diff; none is a test of its own (ledger B10).

| Cause in the old suite | Rule in 1.0 |
|---|---|
| Shared OS resources between parallel test processes: bind port 0, drop it, reuse the number (the open flake, about 12 sites) | Ports only from `tests/common/ports.rs`: `held()` keeps a listening socket for the test's life; `refusing()` keeps a socket bound but not listening, so a connect is refused and no other process can take the port. No `bind("127.0.0.1:0")` anywhere else in tests (reviewers check; no rule test, B10). `wrangler dev` cannot take a held socket: its starter retries on "address in use" up to 3 times with a new port |
| Whole-second or wall clocks compared with exact stamps | Compare monotonic times from the record (PRD-01 section 6), never wall seconds. A test that needs wall time reads it from a LINK |
| Fixed time limits on a loaded machine (about 30 `elapsed() <` checks) | No upper bound on elapsed time below 5 s, and a bound only guards a hang (`within(30 s)`). "Never waits" is shown by holding the other side forever and seeing the call return, not by a stopwatch. Speed is measured in `by_hand_*` tests and real runs (PRD-02 R8, PRD-05 A5), never as a pass line |
| Order of concurrent events assumed | Wait on an event (a channel, a file, a record), never on a sleep |

More rules:

- **File descriptors.** A test that opens more than 500 descriptors calls `common::fds::raise(n)` first and
  fails with a clear line when the hard limit is lower. The default soft limit is 1,024 on Linux and 256
  on macOS; `proxy_connect.rs` (2,000 connections, about 4,000 descriptors) would fail with EMFILE
  without it. Such tests run in the `heavy` group (section 8).
- **Root.** The gate keeps its root refusal (`gate.sh:32-39`) and runs in build boxes as `ubuntu` and in
  CI as the runner user. Tests still check mode bits, never an access refusal, so a test does not depend on
  the user it runs as.
- **Homes.** Every test uses a temporary `HOME` and `GOALCUP_HOME`. `common::home::guard()` (copied from
  `crates/goalcup-cli/tests/common/real_home_guard.rs`, 171 lines) fails a test that touched the real
  `~/.claude`, `~/.codex` or `~/.hermes` (AGENTS.md hard rule).
- **No network.** No test leaves loopback. Upstream names resolve through the test hook in section 5.
- **No retries.** nextest `retries = 0`: a retry hides a flake. A failure keeps its output in nextest's
  JUnit file (`target/nextest/default/junit.xml`), which the audit found missing for the open flake.
- **zstd in tests.** Tests open a `PartWriter` with `PartOptions { window_log: 20, .. }` (1 MiB). Only
  `golden.rs` and `recovery.rs` keep 27 (141 MiB each; four test processes at once would hold about
  560 MiB). PRD-01 section 12 gives `PartOptions`.
- **C code at opt-level 0.** `zstd-sys` and `sha2` compile at the dev profile's level in tests, which
  makes long-window compression slow. PRD-16's phase 0 workspace edit sets opt-level 3 for `zstd-sys`,
  `sha2`, `brotli-decompressor` and the deflate backend in dev, as already done for regex
  (`Cargo.toml:40-54`), and removes `[profile.dev.package.goalcup-daemon]` (`:61`).
- **Linux-only and macOS-only cases.** A test that reads `/proc` or calls a Linux-only API is
  `#[cfg(target_os = "linux")]`; a macOS-only case is gated the same way with `"macos"`. So the build
  (Linux only, ledger T9) and G5's macOS targets each run exactly the cases that apply to them.
- **Hooks reach any binary built with dev-dependencies.** `cargo nextest run`, `cargo clippy --all-targets`
  and any `cargo nextest run --release` (such as a by-hand run of `by_hand_mask_bench`) build `goalcup`
  with `test-hooks` on (the trap named at `crates/goalcup-daemon/Cargo.toml:17-19`). So `gate full` runs no
  release-mode test step; it builds the shipped binary with `cargo build --release` (no dev-dependencies),
  and its last step checks that binary has no hook text (section 8).

### 5. Test support (`crates/goalcup/tests/common/`) and the `test-hooks` feature

Starts at H+0 in its own lane (PRD-00 phase 0 step 8). Published as compiling stubs at H+1.5 so other
PRDs write tests against them; filled in by H+3. Until a helper lands, a PRD may keep a private one,
removed at phase 3.

| File | What | Lines (est.) | Copied from |
|---|---|---:|---|
| `mod.rs` | `within`, temp dirs, `bin()` for `CARGO_BIN_EXE_goalcup` | 60 | `cli/tests/common/m2_common/mod.rs` (240) |
| `ports.rs` | `held()`, `refusing()` (section 4) | 50 | new, `socket2` |
| `fds.rs` | `raise(n)` | 30 | new |
| `home.rs` | temp homes, real-home guard | 120 | `real_home_guard.rs` (171) |
| `pki.rs` | a test CA and leaf certificates | 120 | `m2_common` `TestPki` |
| `upstream.rs` | fake upstream: TLS with chosen ALPN, HTTP/1.1 (chunked, SSE, codings), HTTP/2 (`h2` crate server), WebSocket with permessage-deflate; scripted answers, including "hold the handshake or the answer for N seconds" and "close after N bytes"; records what it received | 700 | `m2_tls_support` (525), `m2_http_support` (486), `m2_ws_support` (783); HTTP/2 is new |
| `worker.rs` | fake Worker: position, chunk PUT, heartbeat, device and open routes from `docs/UPLOAD-API.md` (S5); keeps chunk bodies; a chunk answer is `{offset, next_seq, head, received_at, sealed}` and a heartbeat with a bad `counts` is answered `200` with the `counts` dropped, never `400` (PRD-07 section 4); a script of injected answers (status, drop the answer after commit, `Retry-After`, delay) | 250 | new; never copies Worker rules (no continuity check) |
| `recorder.rs` | helpers over `Recorder::for_tests(room)` (S1): read the collected `Calls`, change `room()` during a test; `real(dir)` starts the pipeline with masking and the writer in a temp entry folder over `Recorder::real_for_tests(dir, masker)` (PRD-02) and returns the folder (PRD-17 H10 and K15) | 50 | none; S1 provides the test constructors |
| `records.rs` | build a part folder from a list of records (for CLI and upload tests) | 100 | over `goalcup_format::PartWriter` |

Every dev-dependency these need (`socket2`, `h2`, `tempfile`, `x509-parser`, the TLS crates the proxy
already uses) is declared in PRD-02's scaffold `Cargo.toml`; a missing one is a night-log note to PRD-02.
`crates/goalcup-verify` keeps its own record builders in its own `tests/common/` (PRD-08).

**The fake Worker and the real Worker.** The fast tier uses the small scripted responder above for
failure injection (it has no rules of its own, so it cannot drift into a second Worker); the full tier's
`upload_worker.rs` (PRD-06) runs the real client against the real Worker under `wrangler dev` with its
local database, then `scripts/fetch-run.sh --local`, and checks the fetched `rec.gcr.zst` (S13) equals
the local parts byte for byte. The 2,514-line Rust stand-in (`cli/tests/common/upload_server/`) goes
with the old crate.

**`test-hooks`.** One cargo feature on `crates/goalcup`, off by default, declared in PRD-02's scaffold
`Cargo.toml`. Integration tests turn it on with a dev-dependency of the crate on itself (today's pattern,
`crates/goalcup-daemon/Cargo.toml:76-79`). `cargo build --release` without dev-dependencies builds the
shipped binary without hooks. `goalcup-format` and `goalcup-verify` declare no hooks. Every hook reads
one `GOALCUP_TEST_*` variable and is behind `#[cfg(feature = "test-hooks")]`. The registry (each owner
builds its hook; no real-run proof needs a hook build):

| Variable | Does | Owner | Used by |
|---|---|---|---|
| `GOALCUP_TEST_WRITER_STALL` | hold the writer for a time from a moment | PRD-02 | `pipeline_guard`, `pipeline_stall` |
| `GOALCUP_TEST_GUARD_MB` | set the memory guard | PRD-02 | `pipeline_stall` |
| `GOALCUP_TEST_CRASH_AT_BOOT` | crash the next n boots | PRD-02 | `watchdog_null` |
| `GOALCUP_TEST_ROTATE_MB` | sets `PartOptions.rotate_at` in PRD-02's writer | PRD-02 | `e2e_run` (two parts in two minutes) |
| `GOALCUP_TEST_UPSTREAM` | `host=127.0.0.1:port` pairs: where the proxy connects for a name | PRD-03 | `proxy_*` through the binary, `pipeline_stall`, `pipeline_restart`, `e2e_run` |
| `GOALCUP_TEST_EXTRA_ROOTS` | a PEM file added to `ProxyCtx.extra_roots` | PRD-03 | same |
| `GOALCUP_TEST_UPLOAD_POLICY` | round, trigger and backoff in ms | PRD-06 | `upload_*`, `e2e_run` |
| `GOALCUP_TEST_FSYNC_COUNT` | count `fdatasync` calls | PRD-06 | `upload_null` |

A hook outside this table is added here first (a night-log note to PRD-14). A hook nothing uses is
removed. There is no silent-drop hook (cut, PRD-00). The release smoke (section 9) and the last step of
`gate full` fail if the shipped `goalcup` binary contains the text `GOALCUP_TEST_`.

### 6. The code rule test (`crates/goalcup/tests/rule_code.rs`, about 80 lines)

Copied and cut from `crates/goalcup-protocol/tests/it/no_family_literals.rs` (378). One rule, because it
guards point 7 and an AGENTS.md hard rule ("no harness names in core code"). Rules 2 to 4 of revision 2
are cut (ledger B10; see "What is cut"):

1. **One system.** No string literal in `crates/goalcup/src/**` or `crates/goalcup-format/src/**` contains a connector id (the file names
   of `connectors/*.toml`, leaving out `model-hosts.toml` and `tiers.toml`, as whole tokens) or a host
   name from `connectors/model-hosts.toml` (providers, gateways such as `ai-gateway.vercel.sh` and
   `openrouter.ai`, vendor backends). Hosts and harnesses are data. Provider key shapes in `src/mask/`
   (for example `sk-`) are not host names and pass. An `ALLOWED` list holds exceptions by file and text
   with a reason, and fails on a stale entry. It starts with one file: `src/connector/builtin.rs`, the
   one `include_str!` list that embeds the connector files by id (PRD-11). `crates/goalcup-verify` is
   not scanned: its usage reader interprets per harness after the run, which is allowed.

Cut from revision 2, each with its reason (ledger B10):

- **Ports rule.** Test hygiene, not a point; section 4 keeps it as a review rule.
- **No-return word list** (PRD-13's patterns and the Worker half). It pins words, not behaviour: our own
  caps are guarded by the never-stop tests (`pipeline_no_latch`, `native_walk`, `native_finish`), not by
  a grep, and a word list costs more to keep in step than the code it scans. PRD-13 and PRD-07 run their
  greps once at their cut merges and put the output in the night log; the shipped-binary hook grep stays
  (section 8).
- **After-the-cut reference guard.** A one-time cleanup check, not a point; PRD-13 runs it once at the
  phase 3 merge.

### 7. The one end-to-end test (`crates/goalcup/tests/e2e_run.rs`, full tier, about 140 lines)

Replaces `m2_synthetic_vertical.rs` (881) and `m2_cli_lifecycle.rs` (1,944). Small on purpose (review B
N12, ledger B10): it proves the parts join without a key; each part's depth is in its own tests, and the
real proof is the rig's micro tests and cycle runs (ledger B3). One run of the real
`goalcup` binary (watchdog, recorder, proxy, native reader, masker, writer, uploader) in about a minute:

- `goalcup start --test --connector tests/data/connectors/fake.toml --hosts tests/data/connectors/fake-hosts.toml`
  (S16). The "harness" is `sh` waiting for a stop file; the test plays the harness: it sends traffic
  through `HTTPS_PROXY` and writes the session file. `GOALCUP_SERVER` points at the fake Worker.
- Traffic: one HTTP/1.1 SSE answer with brotli to a `provider` host; one WebSocket with deflate carrying a
  1.5 MB client message; the same call to a `gateway` host; one unlisted host (a tunnel).
- One session file that grows while the run goes on.
- One planted secret (an `sk-` shaped value) sent split across two writes.
- One `kill -9` of the recorder; `GOALCUP_TEST_ROTATE_MB=1` so the entry has two parts.
- `goalcup seal`, upload to the fake Worker, then
  `goalcup-verify check --json --known <file with the planted value>`.

Pass, each assertion naming its owning PRD in its failure text so a failure points somewhere: every
response the test received is byte for byte what the upstream sent (PRD-03); every request and WebSocket
message is in DATA or inside a GAP (PRD-02); the session file has a native stream whose bytes equal the
file (PRD-04); zero known-value hits and the planted value has a MASK (PRD-05); the concatenated chunks the
fake Worker holds equal the local parts (PRD-06); `status.json` `written` equals the verifier's recount and
off time equals the one `recorder_down` gap (PRD-02, PRD-08); the gateway call is recorded like the
provider call (its response bytes are in DATA) (PRD-03, PRD-11). Verifier exit 1 with only `named`
results (the gap is planned). Cut from revision 2: the check that both hosts have the same `conn` fields
(`decrypt`, `listed: shared`); it pinned an internal shape, and the rig's gateway case proves the promise.

What it no longer does (each lives elsewhere): HTTP/2 (`proxy_h2.rs`), 300 session files
(`native_walk.rs`), an upstream outage (`proxy_connect.rs`), a writer stall (`pipeline_stall.rs`), a
second kill (`watchdog_restart.rs`).

### 8. The gate (`scripts/gate.sh`, about 75 lines)

```
sh scripts/gate.sh fast    # before every merge into integration (build box), CI on every push
sh scripts/gate.sh full    # phase 2 and 3 exits, before the owner's merge to main, before the tag
```

Start: refuse root unless `GOALCUP_GATE_ALLOW_ROOT=1` (kept as today, `:32-39`). Check every
prerequisite and print one line that names all missing ones, then fail before any step: `cargo-nextest`,
`zstd`, `python3`, `git`, `npm` with `server/worker/node_modules`. No fallback to `cargo test`, no
skipped step: a skip is a hidden pass (the same rule as PRD-12 C4). Then take the build lock
`target/.build-lock` (a `mkdir` lock with the holder's pid; a dead holder's lock is taken over). It
matters only where two cargo jobs share one build box: the gate and PRD-12's rebuild step. Nothing ever
builds on the conductor, even while sandboxes are down (ledger B1, PRD-16 S8).

| Step | fast | full | Why |
|---|:-:|:-:|---|
| `cargo fmt --all --check` | yes | yes | cheap |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | yes | yes | caught Linux-only bugs before (commit 84a26baf) |
| `cargo nextest run --locked --workspace` | yes | yes | every guard in section 2 |
| `cargo test --locked --workspace --doc` | yes | yes | nextest skips doctests; one line |
| `npm --prefix server/worker test` | yes | yes | the Worker changes in the same build (PRD-07); a wire change without it is run 1's risk again |
| `python3 -m unittest` on the kept script tests, and `discover -s scripts/pad` | yes | yes | `test_install.py` (point 7, hard rule), `test_fetch_run.py` (PRD-07, point 5), `scripts/pad/test_pad.py` (PRD-12, the rig selftest of ledger B9). A named list, not `discover -s scripts`: until the cut, 13 old script tests name old crates or `cargo` and would fail with the old crates out of the workspace. The list holds only script tests that exist on integration: `discover -s scripts/pad` joins it in the PRD-14 edit right after PRD-12's first `test_pad.py` merge (PRD-12 writes a night-log note), and a new script test is added the same way, only if it guards a point, the format, masking or never-stop |
| `cargo nextest run --locked --workspace --run-ignored only -E 'not test(/^by_hand_/)'` | | yes | `e2e_run`, `upload_worker` (real Worker), other slow cases |
| `cargo build --release --locked -p goalcup -p goalcup-verify` | | yes | the binaries the pad copies, built without dev-dependencies |
| `grep -ac GOALCUP_TEST_ target/release/goalcup` is 0 | | yes | section 4 "Hooks reach any binary built with dev-dependencies" |

- `--workspace` holds only the new crates: PRD-16's phase 0 workspace edit moves the five old crates to
  `exclude`, so they are never built, linted or tested again, and `gate.sh` has no exclude line.
- The version check and the link check are not steps. `server/worker/scripts/check-sql.sh` is not a step
  either: no schema change in this build (ledger S3) and `stopped-runs.sql` is after the merge (PRD-07).
- Every step prints its time; the last line stays `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.
- Test names: `by_hand_*` are `#[ignore]` and never in a tier (PRD-02 `by_hand_load`, PRD-05
  `by_hand_mask_bench`, run once by hand in a release build, report only); every other `#[ignore]` test is
  the full tier. The gate's steps use no binary-name filter, so the probe and its fallback
  (`gate.sh:69-83`) go.
- `.config/nextest.toml` (about 25 lines): `fail-fast = false`, `retries = 0`, slow period 30 s with
  terminate after 6, JUnit on; group `heavy` (one at a time) for tests that kill processes, change limits
  or open thousands of descriptors, as one regex:
  `binary(/^(watchdog_.*|e2e_.*|lessons_launch|pipeline_restart|pipeline_stall|pipeline_many_streams|proxy_connect|proxy_restart|native_walk)$/)`;
  e2e tests may run 600 s. In phase 0 the builder runs `cargo nextest list` on the scaffold; if nextest
  refuses a regex that matches nothing, the group starts empty and PRD-14 adds each name in the merge that
  brings its binary. A builder who adds such a test adds its binary here (a night-log note).

**Target times** (an 8 vCPU build box, warm): `fast` under 3 minutes, alarm at 5; `full` under 15
minutes, alarm at 20. CI's 4-CPU runner may take up to twice that. Past an alarm, the night log names the
slowest step and test (from JUnit); the gate does not fail on time. The first measured numbers go into
`README.md` "The gate" (PRD-15).

### 9. CI and release

**`ci.yml`** (about 50 lines, one job named `check`, so a required check keeps its name): on pull requests
into `main`, pushes to `main` and to `v1.0/integration`, and by hand. Checkout, toolchain from
`rust-toolchain.toml`, nextest from a digest-pinned install action, `npm ci --prefix server/worker`,
`sh scripts/gate.sh fast`, and the JUnit file as an artifact on failure. 30-minute timeout, read-only
token, no secret (`ci.yml:20-21` rule kept). The scope step and its script go: CI now runs the gate,
nothing less. Pushes to the integration branch give PRD-16 an independent record of every merge;
`cancel-in-progress` keeps only the newest. If the job hangs the runner as the old suites did
(`ci.yml:15-16`), the night log says which test, and that test moves to the full tier; CI is never
narrowed by crate.

**`linux-x86_64.yml`** is deleted with `scripts/linux-check.sh` and `scripts/test_linux_check.py`
(PRD-00 gives them to PRD-14). `linux-harness-smoke.sh` is PRD-13's delete.

**`release.yml`**: the same four targets. Before the release build, every target runs
`cargo nextest run --locked -p goalcup-format -p goalcup -p goalcup-verify` (Rust guards only), so macOS
arm64, macOS x86_64 and Linux arm64 run the watchdog, clock and socket-passing tests on their own machines.
Then build `-p goalcup -p goalcup-verify`, package, smoke, install, publish (publish only on a tag push,
as today, `release.yml:93-120`). The `diff -r connectors` step goes: connectors are embedded in the binary
(PRD-11). This workflow first runs at the owner's release (G5, after merge, PRD-00).

**`scripts/package-release.sh`**: two binaries; no `share/goalcup/connectors/` (embedded, PRD-11).
**`scripts/smoke-release.sh`** (about 40 lines): with an empty environment, both binaries print
`<name> <version>`; `goalcup-verify dump` of PRD-01's worked example (`crates/goalcup-format/tests/data/
worked-example.gcr`, 998 bytes) equals `worked-example.dump.jsonl` as parsed values, which proves the
shipped reader on each target; `grep -ac GOALCUP_TEST_ bin/goalcup` is 0. The `doctor`, pin and connector
checks go (`smoke-release.sh:39-54`). **`scripts/third-party-licenses.py`**: `SHIPPED = {"goalcup",
"goalcup-verify"}` (`:24`).

**`packaging/install.sh`** (PRD-10 interface): installs `goalcup` and `goalcup-verify`; the run guard
asks `goalcup status --json` and blocks when `status.mode` is `recording`, `losing`, `writer_slow` or
`null` (PRD-02 S4), and an answer it cannot read is `INSTALL_UNSURE` and goes on (no reader for 0.7's
signed status, and no 0.7 clean-up: no backward compatibility, ledger L7); it prints one line that, when
the participant runs it, exports PATH and appends itself to the right profile file (audit H, item 8).
`install.sh` writes no profile itself. About 300 lines.

**`scripts/release.sh`**: the version check call goes (`:101-104`); the next-step text says "gate full, then
the pad's release check" instead of `linux-check.sh`. `tag` first runs `scripts/pad/table --release-check
<commit>` (PRD-12 section 9): exit 0 only when the core rows and the Codex kill row are PASS with a trusted
pad, built from a commit with no difference to the tag commit under `crates/`, `server/worker/src/`,
`connectors/`, `Cargo.toml`, `Cargo.lock` and `rust-toolchain.toml`, leaving out `connectors/tiers.toml`
(the same path list as PRD-12 section 9; a profile or toolchain change makes a different binary). So the tag is the binary the pad
verified, with only the tier text newer.

**Release order** (PRD-00 question 5, PRD-16 section 14): owner merges to main; `gate full` on main in a
build box; `release.sh tag 1.0.0`; `release.yml` publishes; `install.sh` from `releases/latest` checked on
Linux and the owner's Mac; then the owner deploys the production Worker (whose `MIN_COMPETITION_VERSION`
is 1.0.0, PRD-07). The floor reaches production last, so no participant is refused while 1.0 cannot be
downloaded.

### 10. Repository files that name old paths

- `.github/CODEOWNERS`: rewritten to the new owned paths: `/connectors/`, `/crates/goalcup/src/mask/`,
  `/crates/goalcup/src/connector/`, `/crates/goalcup-format/`, `/scripts/pad/`,
  `/skills/adding-a-capture-harness/`. It lands once those paths exist on integration (second slice).
  `scripts/test_codeowners.py` is deleted (B10: not a point); reviewers check the paths.
- `.github/PULL_REQUEST_TEMPLATE.md`: the checklist asks for the `gate fast` PASS line, and for a connector,
  the pad's `row.json` (PRD-12 contributor path). The `pin-manifests.sh` and `check-connector.sh` lines go.
- `.github/dependabot.yml`: kept; weekly update pull requests run the gate through CI.

**When PRD-14's edits land** (review B M5, PRD-00 phase 3): every edit here lands as soon as it is green,
working with the old crates still on disk and out of the workspace. The phase 3 merge needs nothing from
this PRD except deleting `check-links.py` and its test after PRD-15's last run.

## Files it owns

Create: `crates/goalcup/tests/common/**` (section 5), `crates/goalcup/tests/rule_code.rs`,
`crates/goalcup/tests/e2e_run.rs`, `crates/goalcup/tests/data/connectors/fake.toml` and `fake-hosts.toml`.

Change: `scripts/gate.sh` (113 to about 75), `.config/nextest.toml` (61 to about 25),
`.github/workflows/ci.yml` (93 to about 50), `.github/workflows/release.yml` (120 to about 120),
`scripts/package-release.sh`, `scripts/smoke-release.sh`, `packaging/install.sh` (394 to about 300),
`packaging/README.md`, `scripts/release.sh`, `scripts/third-party-licenses.py`, `scripts/test_install.py`,
`.github/CODEOWNERS`, `.github/PULL_REQUEST_TEMPLATE.md`.

Delete: in phase 0, `scripts/check-version.py`, `scripts/test_check_version.py`, `scripts/ci-scope.py`,
`scripts/test_ci_scope.py`, and (ledger B10) `scripts/test_gate.py`, `scripts/test_release.py`,
`scripts/test_codeowners.py`, `scripts/test_third_party_licenses.py`; with `ci.yml`, `.github/workflows/linux-x86_64.yml`, `scripts/linux-check.sh`,
`scripts/test_linux_check.py`; in the phase 3 merge, after PRD-15's last run, `scripts/check-links.py` and
`scripts/test_check_links.py`.

Must not touch: any test file another PRD names (`pipeline_*`, `watchdog_*`, `proxy_*`, `native_*`,
`mask_*`, `upload_*`, `cli_*`, `connector_*`, `lessons_*`, `crates/goalcup-format/tests/**`,
`crates/goalcup-verify/tests/**`), `crates/goalcup/Cargo.toml` (PRD-02), the workspace `Cargo.toml`
(PRD-16), `server/worker/**` and `scripts/fetch-run.sh` (PRD-07), `scripts/pad/**` and
`connectors/tiers.toml` (PRD-12), `connectors/*.toml` (PRD-11), `docs/*.md`, `README.md`, `AGENTS.md`,
`scripts/README.md` (PRD-15).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | `PartOptions { window_log, rotate_at }`; the worked example as committed files (998 bytes and its dump); golden and recovery tests in the fast tier | Used here |
| PRD-02 | `test-hooks` feature in its scaffold with every dev-dependency `tests/common/` needs; hooks in section 5's registry (incl. `GOALCUP_TEST_ROTATE_MB`); `Recorder::for_tests(room)` (S1); `pipeline_guard.rs` asserts "returns while the writer is held", and the 99.9th percentile figure is reported by `by_hand_load` | Both |
| PRD-03 | `common::upstream` (TLS, HTTP/1.1, HTTP/2, WebSocket, held handshakes); `GOALCUP_TEST_UPSTREAM`, `GOALCUP_TEST_EXTRA_ROOTS`; `proxy_connect.rs` calls `fds::raise`; `panic = "unwind"` stays (no profile sets `abort`) | Both |
| PRD-04 | `Recorder::for_tests(room)` and `common::recorder`; `native_walk.rs` in the `heavy` group | Used there |
| PRD-05 | `mask_bench.rs` holds one `#[ignore]` test, `by_hand_mask_bench`, in no gate tier; run once by hand in a release build, report only | Used there |
| PRD-06 | `common::worker` (S5); `upload_worker.rs` per PRD-06 Amendments (full tier: real client against `wrangler dev`, `fetch-run.sh --local`, equal bytes) | Both |
| PRD-07 | Gate runs `npm test` (fast and full) and `test_fetch_run.py` (fast). `check-sql.sh` is not a gate step in this build | Used here |
| PRD-08, PRD-09 | `cargo nextest` covers `crates/goalcup-verify` in the fast tier; the six PRD-08 files of section 3 and PRD-09's `usage_run1` only (B10 cut, section 3; a note to PRD-09 through the replan agent, ledger B2a); their own `tests/common/` record builders stay in that crate; that crate has no `test-hooks` | Used here |
| PRD-10 | `install.sh` reads `status --json` `status.mode`; `cli_*.rs` use `common::worker` and `common::home`; `start --connector --hosts` (S16) loads `e2e_run`'s fake connector | Both |
| PRD-11 | `model-hosts.toml` and the connector file names are read by `rule_code.rs`; `src/connector/builtin.rs` is its one allowed file; connectors are embedded, so packaging ships none | Used here |
| PRD-12 | The rig (ledger B9) is the main proof and the ladder (ledger B3) runs there, never in the gate; shared build lock `target/.build-lock` inside a box; `test_pad.py` (the rig selftest) in the fast tier; `scripts/pad/table --release-check <commit>` for `release.sh tag`, leaving out `connectors/tiers.toml`; the pad uses no hook build | Both |
| PRD-13 | Deletes the old crates, their tests and the cut scripts. Its reference and no-return greps are no longer in `rule_code.rs` (B10 cut, section 6): PRD-13 runs them once at its cut merge and logs the output (a note to PRD-13 through the replan agent, ledger B2a); it does not edit `gate.sh` | Done there |
| PRD-15 | README "The gate" (two tiers, prerequisites, measured times, the guard list of section 2 as the promise list), AGENTS.md lines 68-77 and 105, `docs/OPERATING.md` "CI" (line 530), `scripts/README.md`; runs `check-links.py` on its final docs before PRD-14 deletes it | Written there |
| PRD-16 | Workspace `Cargo.toml` (old crates in `exclude`, profile changes, the version comment); `gate fast` at every merge in a build box as `ubuntu`; G1, G3, G4 in its schedule; G2, G5, G6, G7 are release steps after the merge | Used there |
| PRD-17 | `lessons_*.rs` run in the fast tier, `lessons_launch` in the `heavy` group; `common::recorder::real(dir)` for H10 and K15; deleting old crates waits for them | Both |

## What is cut

| Path | Src lines | Tests that go | Who deletes |
|---|---:|---|---|
| All tests of the five old crates | | 126,112 (`crates/*/tests`, fixtures excluded), 16,335 inline, 6,939 fixture lines; only the masking corpus (PRD-05), proxy fixtures (PRD-03) and usage fixtures (PRD-09) are copied, and about 900 lines of test support into `tests/common/` | PRD-13 |
| Test runner tools: `test_campaign.py`, `test_linux_matrix.py`, `test_matrix.py`, `test_check_connector.py` | | 4,482 | PRD-13 (PRD-12 list) |
| Research tests: `test_research_*.py` (5), `test_early_crosscheck.py` | | 2,005 | PRD-13 |
| `scripts/check-version.py` | 206 | `test_check_version.py` 128 | here, phase 0. Reason: it fails today on plan docs (95 findings), serves no point, and `release.yml:55-62` already refuses a tag that differs from `Cargo.toml`; the smoke checks every binary prints the version |
| `scripts/check-links.py` | 1,349 | `test_check_links.py` 1,672 | here, phase 3 merge, after PRD-15's last run; out of the gate from phase 0. Serves no point; PRD-15 rewrites most docs and would fight it all day |
| `scripts/ci-scope.py` | 90 | `test_ci_scope.py` 134 | here. CI runs the gate; nothing to scope |
| `scripts/linux-check.sh`, `.github/workflows/linux-x86_64.yml` | 169 | `test_linux_check.py` 107 | here. It ran old test names in Docker; the build is tested natively in Linux build boxes |
| `scripts/gate.sh` version step, binary probe, `cargo test` fallback, old steps | about 40 | all of `test_gate.py` (B10 table below) | here |
| `.config/nextest.toml` old groups and priorities | about 40 | none | here |
| Smoke `doctor`, pin and connector checks; packaged connectors; install guard's daemon-status parsing | about 120 | install guard cases rewritten to `status.mode` fixtures written inline (no protocol crate fixtures) | here |
| **Here, in all (revision 2)** | **about 2,000** | **about 2,100** | |

**Cut by ledger B10** (tests that go beyond the seven points, the format, masking and never-stop, or pin
internal shapes or wording), one line each:

| Planned test | Why it goes |
|---|---|
| `scripts/test_gate.py` (226 today, was to be rewritten to 110) | Pins the gate's step order, prerequisite line and last-line wording; costs more to change than the 75-line script it guards. A gate break shows on the next merge |
| `scripts/test_release.py` | Release tooling, not a point. G6 proves the release check by a real run |
| `scripts/test_codeowners.py` | Repository hygiene, not a point. Reviewers check the paths |
| `scripts/test_third_party_licenses.py` | Packaging, not a point. The script changes one line (`SHIPPED`) |
| `test_install.py` cases "two binaries" and "one PATH line printed" | Pin packaging shape and wording. Kept: the run guard per `status.mode`, `INSTALL_UNSURE` goes on, no profile file written, and the existing rollback and checksum cases only where they pass unchanged |
| `rule_code.rs` rules 2 to 4 (ports, no-return words, after-the-cut guard) | Hygiene and word lists, not behaviour (section 6). About 140 lines |
| `e2e_run.rs` same-`conn`-shape check | Pins an internal shape (section 7) |
| PRD-09 `usage_*` beyond `usage_run1` | Usage stats are proved by C5 on real runs; the rest pin output layout (section 3) |
| `cli_finish.rs` exact sentence | It checks the claim is there or not, never the wording (section 2) |

Script tests that stay: `test_install.py` (here), `test_fetch_run.py` (PRD-07), `scripts/pad/test_pad.py`
(PRD-12).

## Acceptance

### Real-run proofs (first)

| # | Proof | When | Pass |
|---|---|---|---|
| G1 | **Gate time, measured.** `gate fast` and `gate full` in a build box at the end of phases 0, 2 and 3, with the step times | Build | Step times and the three slowest tests (from JUnit) in the night log. Past an alarm (5 or 20 minutes) the slowest step and test are named and PRD-16 decides. No pass or fail on time |
| G3 | **No flake.** Every `gate fast` run in a build box and every CI `check` on integration during the build | Build, read at phase 3 | No red run that a rerun with no code change turned green. Any such run is named in the night log with its JUnit output and the test is fixed; never retried away |
| G4 | **CI is the gate.** The push of the phase 3 commit to `v1.0/integration` | Build | `check` green on the 4-CPU runner; its time in the night log |
| G2 | **The guards catch the failures that matter.** Four one-line sabotages on a scratch branch, one at a time: the writer skips one GAP record; COUNT counts a record at offer instead of at write; the `url_password` rule is removed; the proxy refuses the 1,001st connection | After merge (PRD-00) | `gate fast` FAILs each time on a guard named in advance (`pipeline_guard` for the GAP sabotage and the COUNT sabotage, `mask_rules`, `proxy_connect`). Branch deleted after. A sabotage the gate misses is a missing guard: a note to the owning PRD |
| G5 | **Every target runs the guards.** `release.yml` on the release commit | Owner's release (PRD-00) | Four green builds: Rust guards pass on macOS arm64, macOS x86_64, Linux x86_64 and Linux arm64; smoke and `install.sh` pass; `GOALCUP_TEST_` absent from every shipped `goalcup` |
| G6 | **The tag is what the pad verified.** In the build, a watcher runs `scripts/pad/table --release-check <N1 commit>` and `--release-check <a scratch commit that changes one line under crates/>`. Nothing is tagged during the build. At the owner's release, `release.sh tag 1.0.0` runs the same check first | Build, at the end (PRD-16 N5); owner's release | The N1 commit exits 0; the scratch commit exits 1 with the differing paths named (a differing `connectors/tiers.toml` alone does not refuse). At the owner's release, `release.sh tag` on the scratch commit stops and says why (a real run, no `test_release.py`, ledger B10) |
| G7 | **Install from the release.** `curl .../install.sh \| sh` on Linux and on the owner's Mac | Owner's release | Two binaries installed, one PATH line printed, `goalcup --version` prints 1.0.0 |

### Automated tests that guard them

The gate script itself has no test (ledger B10; `test_gate.py` is cut): G1, G3 and G4 run it for real on
every merge. The tests this PRD writes:

- `scripts/test_install.py`: each `status.mode` value blocks or not as section 9 says (the install never
  breaks a running recording); unreadable output gives `INSTALL_UNSURE` and goes on; no profile file
  written. Existing rollback and checksum cases kept only where they pass unchanged.
- `crates/goalcup/tests/rule_code.rs` (section 6, rule 1 only) and `e2e_run.rs` (section 7).

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Too few automated tests | A regression between rig runs ships | Every merge runs the guards; the ladder (ledger B3) runs micro tests and cycle runs on every new core feature and connector, many at once in sandboxes (ledger B11); the rig reruns the core set on the release commit (G6); G2 after merge shows the guards bite |
| Too many automated tests | Changing a test takes longer than changing the code, and builders slow down | Ledger B10: reviewer B rejects a test that pins an internal shape or wording; such a test is deleted or simplified in the same merge request. The section 3 numbers are ceilings |
| Worker tests in the fast tier are slow | Their time is not recorded anywhere (`J-tests-gate.md:422`) | G1 measures; if over 90 s, they move to full with a night-log note and CI keeps them |
| CI on every integration push | Many runs during the build | Public repository, so free minutes; newest run only |
| CI hangs like the old suites | `ci.yml:15-16` | The `heavy` group runs one at a time; the hanging test moves to full |
| `e2e_run.rs` depends on every PRD | It cannot pass until phase 2 | Full tier only; phase 1 merges need only fast. Each assertion names its owning PRD |
| `rule_code.rs` false hits | A host name in an error message | Only string literals; `ALLOWED` with reasons |
| No-return words come back | With rule 3 cut, an old name (a cap, `receipt`) could return unseen | The behaviour is what matters and the never-stop guards catch a cap; PRD-13 and PRD-07 grep once at their cut merges; reviewer A reads each diff for the seven points |
| A hook binary ships | A by-hand `cargo nextest run --release` leaves a hooked `goalcup` in `target/release` | `gate full` has no release-mode test step and builds the shipped binary without dev-dependencies; its last step and the release smoke grep the binary |
| Named script-test list goes stale | A new `scripts/test_*.py` is never run | The list is one line in `gate.sh`; a new script test adds itself there (night-log note) and reviewers check it |
| Cutting the link check | Broken links in the participant guide (point 6) | PRD-15 runs it once on the final docs before the delete; cheap to restore later |
| Cutting the version check | A second version string slips into a doc | The release workflow checks tag and `Cargo.toml`; the smoke checks every binary's version |
| Build box lacks tools | nextest, zstd or `node_modules` missing | The gate names all missing tools on its first line; PRD-16's build snapshot holds them (phase 0 step 6) |
| macOS only at release | No Mac in the build; G5 waits for the owner's release | A macOS-only guard failure shows at G5 before publish (publish needs a green build); the owner's pad run on a Mac (PRD-12 P9) |
| Load numbers only by hand | Throughput regressions are not caught by the gate | PRD-02 R8 and PRD-05 A5 (`by_hand_load`, `by_hand_mask_bench`) reported in the night log; never a pass line |
| `binary(/regex/)` filter in the `heavy` group | Unverified that a regex matching no binary is accepted (the exact form was refused, `gate.sh:73-74`) | One regex (section 8); the builder checks with `cargo nextest list` in phase 0; if refused, the group starts empty and each name is added in the merge that brings its binary |
| No test reproduces run 1's exact load | The suite passed while run 1 lost 37.5% | `pipeline_stall.rs` replays its shape (20 clients, 1.5 MB bodies, a WebSocket, a stalled writer); the pad's `full` rows and A1 are the real proof |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Fast target and alarm | 3 and 5 minutes, warm 8 vCPU build box | PRD-16 after G1 |
| Full target and alarm | 15 and 20 minutes | PRD-16 after G1 |
| Worker tests | in fast | PRD-16 after G1 |
| CI | runs `gate fast` on PRs into main, pushes to main and integration | Owner (the old rule was "CI stays lean", `ci.yml:5`) |
| Release workflow | Rust guards on all four targets before the build; first run at the owner's release | Owner (may start it by hand on integration earlier for an early macOS signal) |
| Test budget | at most about 14,200 Rust test lines, a ceiling (ledger B10) | PRD-16, only downward; up needs the owner (B10 is locked) |
| nextest | required, no `cargo test` fallback, no retries | PRD-14 |
| Link, version and SQL-mirror checks | out of the gate | Owner |
| Release check on tag | core set PASS on a commit with no code difference, `tiers.toml` left out | Owner |
| Root refusal | kept, with `GOALCUP_GATE_ALLOW_ROOT=1` as today | Owner |

## Builder notes

Order of work:

1. **H+0, by PRD-14's lane, first in its queue, reviewed by reviewers A and B and merged by the
   integrator (PRD-00 phase 0 step 2), from this section's text.** One merge request: `gate.sh` (drop the
   version step, the binary probe and the fallback; add the prerequisite check and the build lock;
   `--workspace` steps as in section 8; the named script tests), the new `.config/nextest.toml`, and the
   deletes of `check-version.py`, `ci-scope.py` and their tests, and of `test_gate.py`,
   `test_release.py`, `test_codeowners.py` and `test_third_party_licenses.py` (ledger B10). The
   named script list holds only tests that exist on integration; PRD-14 adds `discover -s scripts/pad` in
   the edit that follows PRD-12's first `test_pad.py` merge (PRD-12 writes a night-log note). It goes
   through the merge rule like any merge request: reviewers A and B read the diff (ledger B2). Rule 3 of
   the merge rule is met by the new gate run on the merged result, because the old gate is red on
   `docs/plans/` (PRD-00 Today). The new gate is run on the result of the first workspace edit (PRD-16
   section 2), and the two merge back to back. This must land before any other merge: the gate is red
   today without it.
2. **H+0, own lane.** `tests/common/` as compiling stubs by H+1.5 (`ports`, `fds`, `home`, `pki` real;
   `upstream`, `worker`, `recorder` stubs). Measure G1 on the scaffold.
3. **By H+3.** `upstream.rs` (TLS, HTTP/1.1, WebSocket), `worker.rs` (S5 stable at H+2), `recorder.rs`
   (after PRD-02's `for_tests`). HTTP/2 by the third slice.
4. `ci.yml`; delete `linux-x86_64.yml` and `linux-check.sh`; push the integration branch once to see CI run.
5. `rule_code.rs` (after PRD-11's `model-hosts.toml` in phase 0).
6. Packaging and release changes, after PRD-10's `status --json` is stable; CODEOWNERS once its paths exist.
7. `e2e_run.rs` in phase 2. Then G1 and G3 at phase 3, then G4. Delete `check-links.py` in the phase 3
   merge after PRD-15's last run.

Traps from the audit:

- The open flake is a port reuse race; never write `bind(":0")` then drop (`J-tests-gate.md:471-472`).
- Sub-second bounds on a loaded machine are the main flake source (`J-tests-gate.md:474`, `:496-498`).
- nextest refuses a filter naming a binary that does not exist (`gate.sh:73-74`); the gate's steps use
  test-name prefixes, not binary names, and the `heavy` group is one regex checked with `cargo nextest
  list` in phase 0 (section 8).
- The gate's `--offline` needed a prior `cargo fetch` (AGENTS.md:76-77); new crates add dependencies, so
  the gate uses `--locked` and lets cargo fetch.
- `test_install.py` reads status fixtures from the protocol crate (`:28`): rewrite before PRD-13 deletes it.
- CODEOWNERS lists only new paths before the cut, so it never names a deleted path
  (`test_codeowners.py` is deleted in phase 0, ledger B10).
- Tests run as `ubuntu` in build boxes and as the runner user in CI; neither is root.
- Dev-dependency features unify into every test build, `--release` included (section 4).

Parallel work (ledger B11): notes 1, 2, 4 and 5 start at once in separate worktrees and sandboxes; each
gate run takes its own sandbox. A builder never adds a test outside section 2 (ledger B10).

How a watcher proves it in a real run: run `sh scripts/gate.sh fast` in a build box and read the step
times; push and read `gh run list --workflow ci.yml --branch v1.0/integration`; after the core-set pad rows
pass, run `scripts/pad/table --release-check <N1 commit>` (exit 0) and `--release-check <a scratch commit
that changes one line under crates/>` (exit 1, the differing paths named) (G6). Nothing is tagged during
the build.

## Asks (after PRD-00 revision 2)

PRD-00 revision 2 ruled on all eight asks of revision 1 (accepted, item 6 partly: the release workflow is
the owner's step) and PRD-01 accepted all three (`PartOptions`, the 998-byte worked example as files, the
converter cut). All three asks below are ruled: PRD-00 R3-11 accepted them, and they are applied
there. PRD-16 ask 3 (drop the conductor fallback from the build lock): applied (section 8, G3-X9).

1. **PRD-16** (ruled, PRD-00 R3-11, applied there): section 5 says "PRD-14 G2 (sabotage) and G3 (five clean runs) run here" at phase 3. PRD-00
   moved G2 and G5 after the merge; G3 is now read from the build's own gate and CI runs, at no extra
   cost. Section 1 and traps: the build box is the only place the gate runs; this PRD keeps the root
   refusal. Phase 0 workspace edit: update the `Cargo.toml` version comment (`:14-17`, "the gate fails on
   any other hard-coded copy"), which is no longer true. Interfaces row PRD-11: tiers come from
   `connectors/tiers.toml`.
2. **PRD-13** (ruled, PRD-00 R3-11, applied there): section 5 "same merge" no longer needs PRD-14's edits (they land earlier, section 10);
   section 6's guards are kept in `rule_code.rs` rules 3 and 4, and the `test-hooks` pattern is dropped
   from the no-return list; `check-version.py`, `check-links.py`, `ci-scope.py` and `linux-check.sh` with
   their tests are PRD-14's deletes, so drop them from group D and section 7. Section 7's
   `db-local.ts` reason stands (`wrangler dev` in the full tier).
3. **PRD-07** (ruled, PRD-00 R3-11, applied there): `check-sql.sh` is not a gate step in this build; keep it for the owner's next schema change.

## Review answers

Items from the consistency check (CC), review A and review B that concern this PRD, or where it is the
natural owner. "Accepted" means the change is in the body. This section is history: where it differs
from the body (for example the S8 conductor fallback, `release_mask_bench`, or `install.sh` removing 0.7
files), the body, the ledger and the alignment answers below win.

### Consistency check

| Item | Answer |
|---|---|
| CC 9 Ownership gaps | Accepted as ruled by PRD-00: `.github/**`, `.config/**`, `third-party-licenses.py`, `linux-check.sh`, `rule_code`, `e2e_run`, `tests/data/` are here ("Files it owns") |
| CC 10, B C21 Check scripts with two owners | Accepted. PRD-14 deletes `check-version.py` and `ci-scope.py` in phase 0; `check-links.py` leaves the gate in phase 0 and is deleted in the phase 3 merge after PRD-15's last run. Ask 2 to PRD-13 |
| CC 13, B C17 Old crates out of the gate | Accepted. PRD-16's workspace `exclude`; no exclude line in `gate.sh`, and the `test_gate.py` case for it is gone |
| CC 14, B O1 Phase 0 order | Accepted. Builder notes step 1: the integrator lands the gate change at H+0, before any other merge. Checked: `check-version.py` exits 1 with 95 findings at 7f9d88a8 |
| CC 15, B C16, CC 93 Root and where the gate runs | Accepted. Root refusal kept; gate in build boxes as `ubuntu`, CI as the runner user; the "Root" bullet and the trap line rewritten; build lock reasons narrowed to a shared box or PRD-16's S8 fallback |
| CC 18 Done item 4 | Accepted as ruled by PRD-00: `gate full` plus CI `check` green; the release workflow is G5 at the owner's release |
| CC 22 Floor after the release | Accepted (release order, section 9) |
| CC 51 `pipeline_guard.rs` bound | Accepted; PRD-02 already took the form; interfaces row PRD-02 says so |
| CC 57, CC 75, B C12 One override | Accepted. `e2e_run` uses `goalcup start --connector --hosts` (S16); `GOALCUP_CONNECTORS_DIR` and "S13 override" are gone |
| CC 86, B C15 `test-hooks` against the no-return guard | Accepted. The pattern is dropped from rule 3; the shipped binary is grepped instead, now also in `gate full` |
| CC 90 Test budget | Accepted. This table is the one source; updated with the owners' revised numbers (PRD-08 1,100 without `judge_view`; PRD-14 1,850); total about 18,000 |
| CC 94 `GOALCUP_TEST_ROTATE_MB` owner | Accepted. PRD-02's hook sets `PartOptions.rotate_at`; `goalcup-format` has no hooks |
| CC 95 Seam numbers | Accepted. The silent-drop hook is cut (S14 unused); the override is S16 |
| CC 103 `check-links.py` timing | Accepted (CC 10 answer) |
| CC 104 18-CPU conductor | Accepted as ruled by PRD-00; target times now name the 8 vCPU build box |

### Review A

| Finding | Answer |
|---|---|
| 1 Refusal learning test needs "a `test-hooks` delay on upstream TLS" | Partly. No new hook: `common::upstream` can hold a handshake or answer for N seconds, which gives PRD-03 the same case with no code in the shipped binary |
| 5 Core file and panic text test | No change here; `pipeline_secure.rs` is PRD-02's and runs in fast like every guard |
| Coverage gaps (sleep, route drop, long many-subagent run) | Routed by PRD-00 to PRD-12 and PRD-16; they are real-run cases, not gate steps |

### Review B

| Item | Answer |
|---|---|
| C12, C15, C16, C17, C21 | Accepted (see CC 57, 86, 93, 13, 10) |
| A9 Dependencies | Accepted. `tests/common/`'s dev-dependencies are in PRD-02's scaffold list; a missing one is a note to PRD-02 |
| N12 `e2e_run.rs` too big | Accepted, reduced: about 150 lines (from 400); one call per protocol, one gateway, one tunnel, one file, one secret, one kill, upload, verify. Not cut: it is the only automated check that the parts join through the real binary, and each assertion names its owning PRD |
| T2 Hook proofs as automated tests | Accepted. Registry "Used by" names `pipeline_stall` and `watchdog_null`; no real run needs a hook build, so the pad's hook build lines are gone |
| T3 Speed floors fail the gate | Accepted. `release_mask_bench` reports only; no floor anywhere in the gate |
| T4 G2, G3, G5 | Partly. G2 and G5 after the merge (PRD-00). G3 is kept but costs nothing: it is read from the build's own gate and CI runs instead of five extra runs |
| T9 Byte goldens of internal output | Accepted for `judge_view.rs` (deferred with the judge view). `usage_out.rs` is PRD-09's to settle; section 3's rule stands |
| M3, O1 `tests/common/` and the gate change late | Accepted. Both start at H+0 (builder notes 1 and 2); private helpers until `tests/common/` lands |
| M5 The phase 3 merge | Accepted. Every PRD-14 edit lands when green; the phase 3 merge needs only the `check-links.py` delete (section 10) |
| M9 `gate.sh` owner | Accepted. PRD-14 alone; PRD-13's guards live in `rule_code.rs`, so PRD-13 never edits the gate |

### Also found while checking

| Item | Answer |
|---|---|
| PRD-02 ask 8 | Accepted (silent-drop row gone; `ROTATE_MB` is PRD-02's) |
| PRD-05 ask 5 | Accepted (`release_mask_bench` reports, no floor) |
| PRD-08 ask 4 | Accepted (`judge_view` out, `check_conn_bytes.rs` in, no silent-drop row) |
| PRD-12 ask 2 | Accepted (no pad hook build; `--release-check` leaves out `connectors/tiers.toml`) |
| PRD-13 asks for a reference guard as a `gate fast` step | Accepted as `rule_code.rs` rule 4, with rule 3 for the no-return patterns |
| Connectors are embedded (PRD-11) | Self-found. Packaging, smoke, `release.yml` and `install.sh` no longer ship or compare a `connectors/` folder; `install.sh` removes 0.7's |
| `check-sql.sh` in `gate full` | Self-found. Removed: no schema change in this build and `stopped-runs.sql` is after the merge (PRD-07); PostgreSQL leaves the prerequisites |
| Old script tests before the cut | Self-found. 13 script tests name `cargo`, `target/` or old crates; the gate runs a named list of kept tests instead of `discover -s scripts` |
| Hooks in `target/release` | Self-found. `cargo nextest run --release` builds a hooked `goalcup`; the release build now runs after it and `gate full` greps the result |

## Alignment answers

Fix lists g1, g2 and g3 of the alignment review, checked against the ledger (OWNER-DECISIONS.md), the
PRDs they touch and the code. Where two fixes disagree, the ledger and the later PRD text win.

| Id | Answer |
|---|---|
| L6-06 | Applied with G3-X9's wording (section 8 build lock): the lock matters only inside one build box; nothing ever builds on the conductor (ledger B1, PRD-16 S8) |
| L7-05 | Applied (PRD-14 part). Rule 1 scans `crates/goalcup/src/**` and `crates/goalcup-format/src/**`. The AGENTS.md map line is PRD-15's |
| X3-2 | Applied (PRD-14 part). Builder note 1: by PRD-14's lane, first in its queue, reviewed by reviewers A and B and merged by the integrator. Owner decisions gain a B2 line |
| X2-02 | Applied (PRD-14 part). Section 3: PRD-05 at 2,000; total about 15,000 (matching PRD-00 and PRD-13); the "largest share" sentence now says masking and the proxy, with the corpus as data. Overridable default updated |
| G2-35 | Applied. The release-mode test step is gone from section 8; `by_hand_mask_bench` is a `by_hand_*` test in no tier (as PRD-05 already says); the full-tier filter is `not test(/^by_hand_/)`; G1, section 1, section 4 hooks bullet, `test_gate.py`, the PRD-05 interface row and the risk rows follow |
| P04-25 | Applied (PRD-14 part): `native_capture.rs` dropped from point 2 (with G3-14b) |
| X2-08 | Applied (PRD-14 part). `install.sh` no longer deletes 0.7 files; G7 and `test_install.py` drop the 0.7 removal (ledger L7) |
| X2-11 | Partly. Moving "Review answers" and ruled asks into `REVIEW-HISTORY.md` needs a write to a second file, which this pass may not make, and many writers of one new file would race. "Review answers" now opens with a line saying it is history and that the body, the ledger and these answers win; the asks are marked ruled. The move may run once, after every alignment pass, by one agent |
| G3-X9 | Applied (section 8 build lock) |
| G3-X27 | Applied. Builder note 1 is one merge request with `gate.sh`, `.config/nextest.toml`, `test_gate.py` and the deletes; the named script list holds only tests on integration, and `discover -s scripts/pad` joins after PRD-12's first `test_pad.py` merge; the new gate runs on the first workspace edit and the two merge back to back. Note 2 drops `.config/nextest.toml`. Section 8 script row says the same |
| G3-X15 | Applied. The `heavy` group is one regex including `lessons_launch`; phase 0 `cargo nextest list` check with an empty-group fallback; trap and risk rows updated |
| G3-X28 | Applied. G6 and the watcher line: `scripts/pad/table --release-check` on the N1 commit (exit 0) and a scratch commit (exit 1); nothing tagged during the build; `release.sh tag` runs the check at the owner's release (ledger B5, B6) |
| G3-14a | Applied. G2 guards: `pipeline_guard` for the GAP and COUNT sabotages, `mask_rules`, `proxy_connect`. Point 6 row names `pipeline_guard.rs` first (PRD-02 says it asserts COUNT equals a recount) |
| G3-14b | Applied. Point 2 row names `native_tail.rs` and `native_finish.rs` (PRD-04 acceptance) |
| G3-X29 | Applied. Interfaces row PRD-06 points to PRD-06's `upload_worker.rs`; sections 2 and 5 already agree |
| G3-X14 | Applied, with one change: `test-hooks` is allowed only in `crates/goalcup/Cargo.toml`, as PRD-13 section 6 now says (not "two allowed crates"). Rule 3 scans the two feature patterns over src, tests and `Cargo.toml`, and gains the Worker half, which joins after PRD-07's cut. Rule 4 keeps only the reference guard and its pre-cut `exclude` check |
| G3-X4 | Applied. Builder note 1 and owner decisions: reviewers A and B read the diff (ledger B2); merge rule 3 is met by the new gate on the merged result |
| G3-X22 | Applied. `common::worker`: chunk answer `{offset, next_seq, head, received_at, sealed}`; a bad `counts` is dropped and answered `200`, never `400` (PRD-07 section 4) |
| G3-X32 | Applied. `common::recorder::real(dir)` over PRD-02's `Recorder::real_for_tests(dir, masker)`; `recorder.rs` 50 lines; interfaces row PRD-17 names it and `lessons_launch` in `heavy` |
| G3-14c | Partly. G1 pass: step times and the three slowest tests, named past an alarm, PRD-16 decides, no pass or fail on time. The `release_mask_bench` report is left out (G2-35) |
| G3-14d | Partly. PRD-01 row adds `zstd_close`; PRD-17 row 1,300; "today" is PRD-00's 142,413. PRD-02 row says 14, not 13: PRD-02 lists ten `pipeline_*` and four `watchdog_*` files. Total about 15,000, not 17,750, because masking is 2,000 (X2-02) |
| G3-14e | Applied (section 7: `--known <file with the planted value>`) |
| G3-14f | Rejected. Superseded by G2-35: PRD-05 already names the test `by_hand_mask_bench` in no gate tier, and the release-mode step is gone |
| G3-X43 | Applied. The release check compares `Cargo.toml` and `rust-toolchain.toml` too; PRD-12 section 9 takes the same list (G3-X43 in its fix list) |
| G3-14g | Applied. `install.sh` prints one line that exports PATH and appends itself to the profile when the participant runs it; it writes no profile itself. `test_install.py` checks no profile file is written |
| G3-14h | Applied (section 4: Linux-only and macOS-only cases are `#[cfg(target_os = ...)]`) |
| G3-14i | Applied. Point 6 row adds `upload_beat.rs`; new usage stats row (ledger S2, PRD-12 C5, `usage_run1.rs`, `usage_join.rs`, PRD-09) |
| G3-14j | Applied. Asks 1 to 3 marked ruled by PRD-00 R3-11 and applied there; PRD-16 ask 3 applied (section 8, G3-X9) |
| B3, B9, B10, B11 carried in | Applied (ledger, locked; they win over earlier rows above, such as the `test_gate.py`, rule 3 and rule 4 rows and the 15,000 total). Goal and section 1: the rig (PRD-12, B9) is the main proof, run as the B3 ladder (micro tests with local capture first, cycle runs per pad, 20-minute full runs in parallel); the gate is not a rung. Owner decisions gain B3, B9, B10, B11 lines. B10 cuts, each listed with a line in "What is cut": `test_gate.py`, `test_release.py`, `test_codeowners.py`, `test_third_party_licenses.py` deleted in phase 0; `test_install.py` loses its packaging-shape and wording cases; `rule_code.rs` keeps rule 1 only (220 to 80 lines; ports, no-return words and the after-the-cut guard go to review and one-time greps by PRD-13 and PRD-07); `e2e_run.rs` drops its `conn` shape check (150 to 140); PRD-09 keeps `usage_run1` only (600 to 200); `cli_finish.rs` checks the claim, not the sentence. Section 1 rule and section 3: no test pins internal shapes or wording, a costly test is deleted or simplified, reviewer B enforces it, budget is a ceiling (at most about 14,200). B11: one sandbox per gate run, builder notes 1, 2, 4 and 5 start at once. Notes to PRD-09 and PRD-13 go through the replan agent (B2a) |

## Amendments

### 2026-10-30, replan after PRD-07 slice 1 (merge 4dcc1e7b)

- **`test_fetch_run.py` can join the gate's named list now.** The rewrite on integration (4dcc1e7b)
  builds nothing: no database, no R2, no zstd and no verifier (a directory stands in for the bucket).
  It ran 17/17 in a box on Python 3.9. The `gate.sh` comment at line 90 ("today it builds the old
  goalcup-verifier") is no longer true.
- **The real Worker's answers match the fake Worker row** (`worker.rs`): chunk answer
  `{offset, next_seq, head, received_at, sealed}`, a bad `counts` dropped with `200`, no 413 path.
  Until PRD-07 step 3 the real chunk answer also carries `ack` and the position answer `last_ack`; the
  fake Worker need not send them.
- **`fetch-run.sh --local` keeps only each entry's last heartbeat** (the local store holds
  `last_vitals` only), and its default bucket under `--local` is `goalcup-traces-staging`
  (`wrangler dev` keeps local R2 under `preview_bucket_name`). A full-tier case reads one heartbeat
  row per entry.
- **Build snapshot Python is 3.9**: `scripts/check-links.py` needs 3.10 and was skipped in a box at
  10:49Z. It is deleted in phase 3 (PRD-16 section 4); until then a gate step that runs it needs 3.10
  or a skip line.

### 2026-10-30, replan after PRD-07 slice 2 (merge 2f4bc6a0)

- **The real Worker now matches the fake Worker row exactly**: the chunk answer has no `ack` and the
  position answer no `last_ack` on integration at 2f4bc6a0; `/receipt` and `/abandon` answer
  `404 not_found`.
- **`test_fetch_run.py` is still not in the gate** at 2f4bc6a0 (`scripts/gate.sh:90` keeps the stale
  "builds the old goalcup-verifier" comment); the gate log for this merge shows Worker `npm test`
  253/253 and the named unittest list (27 tests, `test_install.py`) only. The earlier note stands: add
  it to the named list; it ran 17 tests green on Python 3.9 in the fixer's box.

### 2026-10-30, replan after PRD-12 slice 2 (merge c7649890)

- **`test_pad.py` is on integration and not yet in the gate.** `scripts/gate.sh` says the pad tests
  join "after PRD-12's first test_pad.py merge"; that merge is c7649890. Add
  `python3.11 -m unittest discover -s scripts/pad` to `gate fast`. The build snapshot's `python3` is
  3.9 and `test_pad.py` (and `pad`, which needs `tomllib`) needs 3.11, so the step installs
  `python3.11` (`sudo dnf install -y -q python3.11`) when it is missing. The pad lane ran it green in
  build boxes (12 tests at 62bd0ed3). `check.py` and `table` stay 3.9-clean.
- `gate fast` passed on the merge: 76 s at c7649890.

### 2026-10-30, replan after PRD-05 phase 0 slice (merge 82484521)

- **The release smoke cannot see `test-hooks` yet** (PRD-05 review A3). At 82484521 no
  `GOALCUP_TEST_` text is in `crates/goalcup/src`; the only `test-hooks` item is
  `Masker::passthrough_for_tests()`, which carries no such text. A release build with the feature on
  would pass the grep today. It is caught once PRD-02's first `GOALCUP_TEST_` read is merged, since
  every build with the feature then carries that text. Nothing outside tests calls the passthrough.
  The release smoke checks this before trusting a clean grep: at least one `GOALCUP_TEST_` string in
  a `--features test-hooks` build. List `Masker::passthrough_for_tests` beside
  `Recorder::for_tests` as a test-only item.
- **Mask tests so far:** 367 lines (179 inline in `src/mask/mod.rs`, 188 in `tests/mask_rules.rs`)
  of PRD-05's 2,000. PRD-05's core slice cuts about 105 of them (PRD-05 note).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-12 slice 3 (merge 9e259b4d)

- **`test_pad.py` is still not in `gate fast`.** At 9e259b4d `scripts/gate.sh` runs only
  `scripts/test_install.py`; `gate fast: PASS in 82s` there did not run the 17 pad tests. The earlier
  note stands: add `discover -s scripts/pad` under python3.11.
- **`table --release-check <commit>` reads `HEAD`'s rows today** (PRD-12 nit B3). Until PRD-12's next
  slice reads `<sha>:pad-rows.jsonl`, `release.sh tag` runs it from a checkout at `<commit>`, or it
  judges another commit's rows. No change to the release steps once B3 lands.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 2 (merge b9fa50f8)

- **Test budget:** the replan decision sets PRD-11's connector tests ceiling to 620 (row 400; PRD-11
  note), with `connector_hosts.rs` at 90 because the N1 to N6 host cases live only in PRD-17's
  `lessons_hosts.rs`.
- `rule_code.rs`: `src/connector/builtin.rs` stays its one allowed file. It remains the hand-written
  list (no `build.rs`); if PRD-11's third slice uses a macro with `concat!`, the ids are still string
  literals in that file only.

### 2026-10-30, replan after PRD-14 slice 1 (merge 22fe5926)

- **Done in slice 1** (builder notes steps 2, 4 and 5): `tests/common/` with `mod`, `ports`, `fds`,
  `home`, `pki` and `records` real; `upstream.rs` and `worker.rs` typed with `start()` still `todo!`;
  `recorder.rs` with `data_of` and `gaps` only. `rule_code.rs` rule 1. `ci.yml` runs `gate fast`;
  `linux-x86_64.yml`, `linux-check.sh` and `test_linux_check.py` are deleted. `gate fast` now runs
  `test_install.py` and `test_fetch_run.py` (44 tests) and `discover -s scripts/pad` on python3.11
  (17 tests), so the earlier notes on `test_fetch_run.py` and `test_pad.py` are closed.
  `gate fast: PASS in 87s` on 22fe5926.
- **Rule 1 is wider than section 6 says** (review A2): it also bans each flat connector's `program`
  (`claude`, `codex`, `hermes`, `kilo` today), read at test time, matched case-insensitively as whole
  tokens (`_`, `-`, `.` and spaces are boundaries). Every flat `connectors/*.toml` other than
  `model-hosts.toml` and `tiers.toml` must hold a line that starts exactly `program = "`, or the test
  panics (PRD-11 note). A literal that hits a short program name used as an ordinary word (for example
  `amp` in `&amp;`) goes into `ALLOWED` with its file, text and reason.
- **`tests/common` is compiled through `rule_code.rs`** (`#[allow(unused)] mod common;`, review A1), so
  every merge compiles and lints it. Every other test binary declares it the same way, or
  `clippy -D warnings` fails on the helpers that binary does not call (PRD-16 note).
- **Slice 2, by H+3:** `upstream.rs` first (TLS with a chosen ALPN, HTTP/1.1 with chunked, SSE and
  codings, WebSocket with permessage-deflate), because PRD-17's `lessons_decode` and `lessons_http1`
  wait on it; then `worker.rs`; then `recorder::real` once PRD-02's `Recorder::real_for_tests` is on
  integration. HTTP/2 in slice 3, as planned.
- **Review nit B8 (reviewer B), for the slice that makes `tag` run the release check (section 9):**
  `scripts/release.sh` writes the same next-step sentence three times (lines 74, 106 and 111). Set it
  once, `next="run sh scripts/gate.sh full, then scripts/pad/table --release-check <commit>, on that
  origin/main commit before sh scripts/release.sh tag $new"`, and use `"$next"` in the echo, the pull
  request body and the final echo; that slice then changes one line, and the script gets shorter.
  Review nits A4, A5, B2 and B5 were declined as nits by the fixer; reviewer A found the declines
  sound.
- **CI on integration never finishes yet, so G4 is not measured.** `ci.yml` has
  `group: ci-${{ github.ref }}` with `cancel-in-progress: true`, and each merge is followed within
  about two minutes by a docs-only replan push. Run 41060500797 (22fe5926) was cancelled at 1m35s by
  the push of c6f5deee at 13:07Z, and that run by 8fa991c9's at 13:08Z. Section 9's "an independent
  record of every merge" does not hold at this push rate, and PRD-16's phase 3 exit needs "CI green".
  Next CI edit (one line, this lane): `cancel-in-progress: ${{ github.event_name == 'pull_request' }}`,
  so every push to integration and main runs to the end and only superseded pull request runs are
  cancelled. Until it lands, a watcher reads G4 only from a run that shows `completed success`, never
  from a cancelled one.
- **Size against section 3** (`wc -l` at 22fe5926): `tests/common/` 740 of 1,480 (`mod` 60, `ports`
  33, `fds` 27, `home` 171, `pki` 115, `upstream` 126, `worker` 107, `recorder` 29, `records` 72);
  `rule_code.rs` 213 of 80; `e2e_run.rs` not started (140). In all 953 of the PRD-14 test row of 1,700.
  `rule_code.rs` is 166% over its line: the lexer for raw, byte and char literals and nested comments
  is the guard itself (the char-literal desync reviewer B found hid a host literal), so the extra lines
  serve point 7. The PRD-14 row is not raised: slice 2 keeps `upstream.rs` and `worker.rs` at or under
  700 and 250, and the whole of `tests/common/`, `rule_code.rs` and `e2e_run.rs` within 1,870 (the
  row plus 10%); `home.rs` (171 of 120) is cut first if it is not. Scripts: `gate.sh` 121 (section 8
  says about 75), `ci.yml` 57 (about 50), `release.sh` 111; 384 lines deleted.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 1 (merge d10feef7)

- `crates/goalcup-verify` is a workspace member (94d204f1) and in `gate fast`: 16 of the 83 tests on
  d10feef7 are its own (`check_invariants.rs`, `check_named.rs`). `gate fast: PASS in 119s` against
  83 to 87 s on the merges before; the extra is the first compile of the crate, `rusqlite` with
  `bundled` (SQLite's C) included. Still under the 3-minute target; G1 measures it warm.
- The crate declares no test hook and reads no `GOALCUP_TEST_*` variable; PRD-14's grep of the shipped
  binary needs no exception for it.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 2 (merge 04886295)

- **Done in slice 2:** `upstream.rs` real for TLS with a chosen ALPN or plain TCP, HTTP/1.1 (whole,
  chunked, SSE, codings), WebSocket with permessage-deflate, held handshakes and answers and close
  after N bytes; a connection whose TLS chose `h2` panics its task with a clear line until slice 3.
  `worker.rs` real for the S5 routes with its own position, the S5 `counts` bound (0 to 2^53-1, as
  the real Worker's `isFlatCounts`; a bad `counts` is dropped with `200`) and injected answers that
  carry status and body. `ci.yml`, `CODEOWNERS` (section 10 paths) and `release.sh tag` (section 9,
  nit B8 taken) are in. `gate fast: PASS in 122s` on the lane branch before the merge. A
  `ci.yml` YAML parse ran in a box once (reviewer A, 14:56Z); `shellcheck` was skipped (not
  installable in the box).
- **CI rule changed from the slice 1 note** (review A1 and F1): the push group is per sha, not
  `ci-${{ github.ref }}`: `group: ci-${{ github.event_name == 'pull_request' && github.ref ||
  github.sha }}` with `cancel-in-progress` true only for pull requests. A fixed group with
  `cancel-in-progress: false` still lets a third push cancel the second's pending run, which this
  avoids. Section 9's "`cancel-in-progress` keeps only the newest" now holds for pull requests only;
  every push to integration and main runs to the end. Run 41078054029 (04886295) is the first run
  under the new rule (in progress at 15:10Z). The earlier runs on 9e89a22d and 48d0aeb7 were still
  cancelled under the old rule. G4 is read from completed runs from 04886295 on.
- **`release.sh tag` runs the release check from a worktree at origin/main's sha**, so PRD-12's nit
  B3 (`table` reading `HEAD`'s rows) no longer affects the tag (PRD-12 note). It runs `python3`;
  `table` stays 3.9-clean.
- **Still open from sections 9 and 10:** `.github/PULL_REQUEST_TEMPLATE.md` (the `row.json` line for a
  connector; it already asks for the `gate fast` line and names no `pin-manifests.sh`), packaging,
  `smoke-release.sh`, `install.sh`, `test_install.py`, `third-party-licenses.py` and `release.yml`
  (after PRD-10's `status --json`), the `check-links.py` delete (phase 3). `test_codeowners.py` is
  already gone from integration.
- **Slice 3 adds to `recorder.rs`:** `real(dir)` once `Recorder::real_for_tests` is on integration,
  and the `opens`, `ends` and `events` helpers over `Calls` PRD-04 asked for at 14:31Z (native,
  proxy and pipeline tests each carry a copy today). Their owners delete the copies when the helpers
  land (PRD-04 note), so the Rust test total falls.
- **Size against section 3** (`wc -l` at 04886295): `tests/common/` 1,444 of 1,480 (`mod` 61,
  `ports` 33, `fds` 27, `home` 171, `pki` 115, `upstream` 602 of 700, `worker` 334 of 250,
  `recorder` 29, `records` 72); `rule_code.rs` 213; `e2e_run.rs` not started. In all 1,657 of 1,700.
  `worker.rs` broke the slice 1 note's 250 by 84 (134%): the review fixes added the S5 `counts` bound
  (G3-X22) and injected answers with status and body and closing live connections on drop (PRD-06's
  backoff and drop-after-commit cases). **Decision:** those lines serve point 5 (upload) and point 3
  (never-stop rules), so `worker.rs` keeps them; the ceiling for the whole of `tests/common/`,
  `rule_code.rs` and `e2e_run.rs` stays 1,870. Slice 3 has 213 lines left for HTTP/2 (at most 98,
  to 700), `recorder.rs` (`real` and the three helpers) and `e2e_run.rs` (140): it cuts `home.rs`
  to 120 first, and a result past 1,870 needs a decision line in the night log naming the point.
- **Housekeeping:** the stray branch `v1.0/prd-14-tests-lane-merge-wip` (same sha as the lane merge
  8a7f5c05) is still on origin; the remote refused its delete. The integrator removes it at the end.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-04 core slice part 2 (merge 01afcf22)

- **`native_walk` binary is on integration and not yet in the `heavy` group.** `.config/nextest.toml`
  names it in the comment but has no `[[profile.default.overrides]]` entry for it, so the 20,000 and
  5,000 file cases run in parallel with the rest (gate fast passed anyway: 182 of 182 at 01afcf22).
  PRD-14's next slice adds `binary(/^native_walk$/)` to the heavy override (PRD-04 ask 14:31Z).
- The `opens`, `ends` and `events` helpers over `Calls` (slice 3) also replace `lost` in
  `native_tail.rs` and `events` in `native_walk.rs`; PRD-04 deletes its copies after that merge.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-09 slice 2 (merge dab0ce1f)

- **`usage_run1.rs` is 299 lines** (6 cases) against section 3's 200. The PRD-09 replan note sets its
  ceiling to 380 with the native-only and VR2 panic cases still to come; the end-of-build report nets
  it against the test total (at most about 14,200).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 3 (merge 00fac28d)

- **Done in slice 3:** HTTP/2 in `upstream.rs` (h2 crate; `close_after` counts DATA bytes actually
  sent, claimed from the connection count before each send, so overlapping streams cut exactly);
  `opens`, `ends` and `events` in `recorder.rs` over one `pick` helper; `home.rs` cut to 121;
  `install.sh` run guard over `goalcup status --json` `.status.mode` (an awk walk, since `run` also
  has a `mode` key); `test_install.py` rewritten (28 tests in the gate); `package-release.sh`,
  `smoke-release.sh`, `third-party-licenses.py` and `release.yml` for the two binaries; `gate full`
  and `release.yml` both refuse a hooked `target/debug/goalcup` with no `GOALCUP_TEST_` text.
  `gate fast: PASS in 101s` on 00fac28d (nextest 188 of 188).
- **`gate full` and every release run are red until a `GOALCUP_TEST_` read is reachable from the
  `goalcup` binary** (review A2, kept on purpose). On integration `Policy::from_env` is the only
  reader and nothing calls it, also on `v1.0/prd-06-upload` at this replan; PRD-06's next slice wires
  it (PRD-06 note). This is the known seam, not a regression; a watcher reads `gate full` only after
  that merge.
- **Heavy group, next touch of `.config/nextest.toml`:** `native_walk` is on integration since
  01afcf22, so the review's A10 is unblocked: add `binary(/^native_walk$/)` to a `heavy` override
  now. `proxy_connect` joins in the merge that brings its 2,000-connection case (PRD-03).
- **Review nit B12 for the next touch of `upstream.rs`:** `h2_stream` computes "bytes left before the
  cut" twice (`room` and the `share` closure). Define it once before the loop,
  `let left = |w: u64| limit.map_or(u64::MAX, |l| l.saturating_sub(w));`, and use it for
  `reserve_capacity`, the `written.update` claim and `n`. Same lines, same behaviour, one definition.
- **Still open:** `recorder::real(dir)` (waits on PRD-02's `Recorder::real_for_tests`, on no branch
  at this replan), `e2e_run.rs` with `fake.toml` and `fake-hosts.toml` (phase 2), the
  `check-links.py` and `test_check_links.py` delete (phase 3), `.github/PULL_REQUEST_TEMPLATE.md`'s
  `row.json` line.
- **Size against section 3** (`wc -l` at 00fac28d): `tests/common/` 1,523 (`mod` 61, `ports` 33,
  `fds` 27, `home` 121, `pki` 115, `upstream` 709, `worker` 334, `recorder` 51, `records` 72),
  `rule_code.rs` 213, `e2e_run.rs` 0: 1,736 of the 1,700 line, inside the 1,870 ceiling the slice 2
  note set. `recorder::real` and `e2e_run.rs` share the 134 lines left; `e2e_run.rs` drops cases
  that serve no point before it passes that. The fixer's 16:08Z decision line names point 1 for
  `upstream.rs` at 709 of 700. Scripts: `install.sh` 355 (section says about 300), `gate.sh` 128
  (about 75), `release.yml` 132 (about 120), `test_install.py` 314 (was 582), `smoke-release.sh` 53,
  `package-release.sh` 40, `third-party-licenses.py` 231.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-14 slice 4 (merge ec0d79d8)

- **Done in slice 4:** `native_walk` is in a `heavy` override in `.config/nextest.toml`
  (`filter = 'binary(/^native_walk$/)'`, the group's only binary today; its 7 tests run one at a
  time), which closes the PRD-04 ask and review A10. `gate.sh` went from 128 to 105 lines by comments
  only: with comment lines stripped its code equals the code at 722b846b, so no step changed (review
  B1). Nit B12 is done: one `left` closure in `h2_stream`, `upstream.rs` stays 709. `gate fast: PASS
  in 134s` on ec0d79d8 (nextest 188 of 188). `gate full` was not run; it stays red at the
  hooked-binary line until PRD-06 calls `Policy::from_env` (slice 3 note).
- **Next touch of `scripts/gate.sh`** (reviews A1, A2, A4, B1, B2; nits, no behaviour change):
  - Put back one header line: "Every step prints its time; the last line is `gate <tier>: PASS in
    <N>s` or `gate <tier>: FAIL`." CI and the night-log watchers read that format (A1).
  - On the full tier's ignored-tests step, say again that it fails until the first `#[ignore]` test
    (`e2e_run.rs` or `upload_worker.rs`) merges in phase 2, and that phase 1 needs only `fast` (A2).
    Without it a watcher reads that failure as a regression.
  - Next to the named script-test list, restore the admission rule as one clause: a new script test
    joins only if it guards one of the seven points, the format, masking or the never-stop rules
    (section 8, ledger B10, risk row "Named script-test list goes stale") (A4, B2).
  - Pay for those lines with B1's cuts: `[ -z "$missing" ] || { echo "gate.sh: missing:$missing";
    fail; }` for the four-line `if` (-3); define `fail()` before the root check and end that block
    with `fail` (-1); drop the comment above the root check that repeats its echo (-1). That lands
    near 103 lines. Section 8's "about 75" cannot be reached without dropping what section 8 itself
    requires (the build lock, the python3.11 probe, the two hook greps), so the overage stands and
    needs no further cut.
- **Next touch of `.config/nextest.toml`** (review A3): the comment says each binary joins "in the
  merge that brings it" and lists `proxy_connect` as still to come, but `proxy_connect.rs` is on
  integration and stays out of `heavy` on purpose (its cases today are ClientHello parses). Word it
  "joins in the merge that brings its heavy case". The rule stands: `proxy_connect` joins with
  PRD-03's 2,000-connection case (PRD-03 note), `lessons_launch` with PRD-17's `lessons_launch.rs`,
  `watchdog_*`, `e2e_*`, `pipeline_restart`, `pipeline_stall`, `pipeline_many_streams` and
  `proxy_restart` with their binaries. Each adds its name to the one override's alternation,
  `binary(/^(native_walk|...)$/)`, not a second override block.
- **Still open:** `recorder::real(dir)` (waits on PRD-02's `Recorder::real_for_tests`, on no branch
  at this replan), `e2e_run.rs` with `fake.toml` and `fake-hosts.toml` (phase 2), the
  `check-links.py` and `test_check_links.py` delete (phase 3). `.github/PULL_REQUEST_TEMPLATE.md`
  already has the `row.json` line, so that item from the slice 3 note is closed.
- **Size against section 3** (`wc -l` at ec0d79d8): unchanged at 1,736 (`tests/common/` 1,523,
  `rule_code.rs` 213, `e2e_run.rs` 0) of the 1,700 line, inside the 1,870 ceiling; 134 lines left for
  `recorder::real` and `e2e_run.rs`. Scripts: `gate.sh` 105 (about 75, see above), `.config/nextest.toml`
  21, the rest as in the slice 3 note.
- No locked ledger item is touched and no owner question comes from this slice.
