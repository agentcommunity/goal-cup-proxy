# PRD-14: Tests and gate

Id: PRD-14. Version: goalcup 1.0. Status: plan for the overnight build. Date: 30 October 2026.
Base: main at 4b226de2 (this branch: 7fd36019). Index: [PRD-00](PRD-00-index.md). Format:
[PRD-01](PRD-01-record-format.md). Main proof: [PRD-12](PRD-12-test-pad.md).

## Goal

Real runs on the pad prove the seven points; a small set of automated tests guards the three things a
real run cannot pin down on every change: the record format, masking, and the never-stop rules. One
gate script, two tiers, the same command on a laptop, a sandbox and CI.

## Points served

All seven, each through one pad check (PRD-12 C1 to C6) and at most three named guard tests (section 2).
Point 3 also directly: the gate stops tests that pin caps, latches or failure budgets from coming back.

## Owner decisions it carries out

- "Less about building tests, more about real runs" (relayed request; BRIEF3 "test pad"). The pad is the
  main proof. Automated tests guard the format, masking and the never-stop rules.
- No backward compatibility, old crates deleted in the same build (BRIEF3, 29 Oct night): every test of
  the old crates goes with them. Lessons that matter come back as PRD-17 acceptance cases.
- No caps of our own: no test may assert a cap, a latch or a failure budget. Tests assert "past N,
  everything is recorded or a gap is written, and the relay goes on".
- AGENTS.md change 1 (PRD-00): "Every behaviour that serves one of the seven points has a test, and that
  test is not deleted while the point stands." Section 2 names those tests.
- One system for every harness, no patchwork (owner's gateway question). A gateway is one more host on
  the shared list. A code rule test (section 6) fails if a harness name or a model or gateway host name
  appears in recorder source, so no per-harness or per-gateway patch can slip in.
- Release: tag `v1.0.0` after the owner's merge (PRD-00 question 8), from the commit the pad verified.
- Early quick real tests during the build; mass harness testing at the end (BRIEF3, 30 Oct early). The
  gate never runs a harness or spends a key.

## Today

| What | Where | Lines | Fact |
|---|---|---:|---|
| The gate | `scripts/gate.sh` | 113 | Refuses root (`:32-39`); version check first (`:61`); builds the workspace because CLI tests spawned the sibling daemon (`:63-65`); probes a binary-name filter and falls back to every test when a binary was renamed (`:69-83`); full runs ignored e2e and `upload_worker` by name (`:85-93`); doctests (`:96`); `cargo test` fallback without nextest (`:97-105`); full adds Worker tests, every script test and the link check (`:107-111`) |
| nextest config | `.config/nextest.toml` | 61 | 30 s slow period, killed at 180 s (`:7`); two long tests start first (`:14-19`); `serial` and `connector-runs` groups (`:25-61`), all naming old binaries |
| Test volume | `crates/*/tests` | 126,112 | cli 59,973, daemon 30,260, verifier 20,724, protocol 13,471, platform 1,684 (`wc -l`, fixtures excluded); plus 16,335 lines of `#[cfg(test)]` in `crates/*/src` and 6,939 fixture lines (`J-tests-gate.md:128-140`) |
| Script tests | `scripts/test_*.py` | 10,393 | 20 files, 577 test methods |
| Version check | `scripts/check-version.py` | 206 + 128 tests | **Fails today**: at 7fd36019 it exits 1 with 86 findings, all in `docs/plans/` (tags such as `v0.22.5` in the audit, and `v1.0.0` at `PRD-00-index.md:325` and `PRD-07-worker-server.md:364`). Rule 5 flags any tag on a line about a release (`check-version.py:16-21`). So `gate fast` is red at its first step on this branch, before any code |
| Link check | `scripts/check-links.py` | 1,349 + 1,672 tests | Full tier and CI (`gate.sh:110`, `ci.yml:80`) |
| CI scope | `scripts/ci-scope.py` | 90 + 134 tests | Skips Rust on Markdown-only changes (`ci.yml:60-75`) |
| CI | `.github/workflows/ci.yml` | 93 | "NOT the gate" (`:1-2`); excludes the daemon and CLI suites because they hang a 2-core runner (`:15-16`, `:93`). The repository is public (`gh api repos/agentcommunity/goal-cup-proxy` says `public`), so `ubuntu-24.04` has 4 CPUs and 16 GB (https://docs.github.com/en/actions/reference/runners/github-hosted-runners) |
| Manual Linux CI | `.github/workflows/linux-x86_64.yml` | 63 | Runs `linux-check.sh` and `linux-harness-smoke.sh` (`:26-63`); PRD-12 cuts both scripts |
| Release | `.github/workflows/release.yml` | 120 | Builds `-p goalcup-cli -p goalcup-daemon -p goalcup-verifier` on four targets (`:69-71`), packages, smoke-tests and runs `install.sh` (`:72-86`); no Rust test runs on macOS or Linux arm64 anywhere |
| Packaging | `scripts/package-release.sh` 45, `scripts/smoke-release.sh` 55, `packaging/install.sh` 394, `scripts/third-party-licenses.py` 231 | | Three binaries (`package-release.sh:35-37`, `smoke-release.sh:31-37`, `install.sh:9`, `:312-359`, `third-party-licenses.py:24`); smoke judges `goalcup doctor --json` pins (`smoke-release.sh:39-54`), and PRD-10 cuts `doctor`; install guard parses the daemon's signed status (`install.sh:104-195`); two PATH lines (`install.sh:377-389`) |
| Release script | `scripts/release.sh` | 115 | Next-step text names `linux-check.sh` (`:74`, `:110`, `:115`); calls the version check (`:101-104`) |
| Test-only feature | `crates/goalcup-daemon/Cargo.toml:15-21, 76-79` | | `test-hooks`, turned on for its own tests by a dev-dependency on itself |
| Repo files that name old paths | `.github/CODEOWNERS:9-30`, `.github/PULL_REQUEST_TEMPLATE.md:30, 45`, `scripts/test_install.py:28` | | CODEOWNERS lists old crate files (`scripts/test_codeowners.py` fails once they go); the PR template asks for `pin-manifests.sh` and `check-connector.sh`; the install test reads status fixtures from the protocol crate |
| Build profile | `Cargo.toml:30-62` | | regex crates at opt-level 3 for tests (`:40-54`); `goalcup-daemon` at opt-level 1 (`:61`) |
| Build host | this machine | | 4 CPUs, runs as root (uid 0), Node 22, no `server/worker/node_modules`, no `zstd` or `sqlite3` on PATH, PostgreSQL under `/usr/lib/postgresql/*/bin`, `cargo-nextest` not in `~/.cargo/bin` |

Gate times on record (`J-tests-gate.md:446-458`): fast 54 s quiet, 306 s on a loaded sandbox; full
never measured. Open flake: the port reuse race at about 12 sites, for example
`crates/goalcup-cli/tests/it/m2_proxy_integration.rs:2865-2868` and
`crates/goalcup-cli/tests/full/upload_worker.rs:191-197`; about 30 sub-second wall-clock bounds
(`J-tests-gate.md:471-474`). The 2-hour run report exists only on `origin/smart/tender-clarke-qupne3`; run 1's
lesson for tests is that every automated check passed while 37.5% of calls went unrecorded.

## Design

### 1. What proves what

| Layer | Proves | Runs where | Cost |
|---|---|---|---|
| **Real runs (PRD-12 pad)** | All seven points on real harnesses, keys and the staging Worker: wire against the harness's own log, every subagent, no secret, server copy whole, counts the participant saw | Vercel Sandbox, the owner's Mac | Keys, minutes, money |
| **`gate full`** | One end-to-end run of the real binary through every part, the real Worker under `wrangler dev`, release-mode speed floors, the SQL mirror | Build host before merges to main and before the tag | About 15 minutes |
| **`gate fast`** | Format, masking, never-stop, upload contract, connectors, participant words, code rules | Every PRD branch before a merge request, every merge on integration, CI | Under 5 minutes |

Rule: an automated test is written only where a real run cannot show the failure every time. A real run
cannot plant every secret shape split at every byte, cannot kill the recorder at every point, and cannot
fill a guard on demand. It can show everything else, so the rest is left to the pad.

### 2. The seven-point guard list

These are the tests AGENTS.md's new rule protects. PRD-15 writes this table into README in place of P1 to
P7. A guard is deleted only with its point, or replaced in the same change by a test with the same name
prefix that asserts the same promise.

| Point | Real-run proof | Guard tests (fast tier unless marked) | Owner |
|---|---|---|---|
| 1 Every model call | PRD-12 C1; PRD-03 proofs | `proxy_connect.rs` (2,000 connections, no refusal), `proxy_ws.rs` (Codex deflate fixture), `lessons_*.rs` (PRD-17) | PRD-03, PRD-17 |
| 2 Every subagent | PRD-12 C2 | `native_walk.rs` (20,000 files, no cap), `pipeline_many_streams.rs` (20,000 streams), `native_resume.rs` | PRD-04, PRD-02 |
| 3 Never block | PRD-02 R1 kill test, R2 stalled writer, R6 null mode | `pipeline_guard.rs` (a held writer never makes an S1 call wait; every drop is a GAP), `pipeline_no_latch.rs` (1,000 guard episodes, recording resumes each time), `watchdog_restart.rs` (`kill -9`, same port, connection served) | PRD-02 |
| 4 No secrets | PRD-12 C3 | `mask_stream.rs` (every split point), `mask_rules.rs` (`url_password`, left guard), `secrets_scan.rs` | PRD-05, PRD-08 |
| 5 Upload everything | PRD-12 C4 | `upload_chunks.rs` (concatenated chunks equal the parts), `server_compare.rs`, Worker `integration.test.ts`; full: `upload_worker.rs` | PRD-06, PRD-08, PRD-07 |
| 6 Tell plainly | PRD-12 C6 | `check_invariants.rs` invariant 7 (COUNT equals a recount), `cli_finish.rs` ("Nothing was lost" only with zero gaps and zero off time) | PRD-08, PRD-10 |
| 7 Real harnesses | PRD-12 table, tiers | `connector_builtin.rs`, `connector_launch.rs`, `rule_code.rs` (section 6), `scripts/test_install.py` | PRD-11, PRD-14 |
| Format (base of 1, 5, 6) | PRD-01 acceptance 1 to 3 on the pad | `golden.rs`, `recovery.rs`, `check_invariants.rs` | PRD-01, PRD-08 |
| All together | PRD-00 done items 1 and 2 | full: `e2e_run.rs` (section 7) | PRD-14 |

### 3. Test budget

No test pins a cap, a byte-for-byte golden of internal facts, or a shape nobody relies on. One file per
module; cases named for the promise they keep. Estimates come from each PRD where it states one.

| PRD | Test files (from its Acceptance) | Lines (est.) |
|---|---|---:|
| PRD-01 format | `golden`, `roundtrip`, `recovery` | 800 (estimate here) |
| PRD-02 core | 9 `pipeline_*` and `watchdog_*` | 1,500 (PRD-02) |
| PRD-03 proxy | 7 `proxy_*` plus fixtures | 2,000 (estimate here) |
| PRD-04 native | 8 `native_*` | 1,000 (estimate here) |
| PRD-05 masking | 6 `mask_*` plus the copied corpus (data) | 5,000 (PRD-05) |
| PRD-06 upload | 7 `upload_*` plus `upload_worker` (full) | 1,000 (estimate here) |
| PRD-08 verifier | 6 `check_*`, `server_compare`, `secrets_scan`, `judge_view` | 1,200 (PRD-08) |
| PRD-09 usage | 5 `usage_*` | 600 (PRD-09) |
| PRD-10 CLI | 7 `cli_*` | 1,200 (PRD-10) |
| PRD-11 connectors | 4 `connector_*` | 400 (estimate here) |
| PRD-17 lessons | `lessons_*` | 1,500 (estimate here) |
| PRD-14 | `tests/common/`, `rule_code.rs`, `e2e_run.rs` | 1,800 |
| **Rust total** | | **about 18,000**, against 142,447 today (126,112 in `tests/` plus 16,335 inline) |

A builder who goes over its line says why in the night log (PRD-00 size rule). Masking is the largest
share on purpose: point 4 is the one point a real run can only sample.

### 4. Test rules (flakes do not come back)

Every closed flake in `J-tests-gate.md:479-499` had one of four causes. Each becomes a rule; the first is
checked by `rule_code.rs`.

| Cause in the old suite | Rule in 1.0 |
|---|---|
| Shared OS resources between parallel test processes: bind port 0, drop it, reuse the number (the open flake, about 12 sites) | Ports only from `tests/common/ports.rs`: `held()` keeps a listening socket for the test's life; `refusing()` keeps a socket bound but not listening, so a connect is refused and no other process can take the port. No `bind("127.0.0.1:0")` anywhere else in tests (rule check). `wrangler dev` cannot take a held socket: its starter retries on "address in use" up to 3 times with a new port |
| Whole-second or wall clocks compared with exact stamps | Compare monotonic times from the record (PRD-01 section 6), never wall seconds. A test that needs wall time reads it from a LINK |
| Fixed time limits on a loaded 4-CPU machine (about 30 `elapsed() <` checks) | No upper bound on elapsed time below 5 s, and a bound only guards a hang (`within(30 s)`). "Never waits" is shown by holding the other side forever and seeing the call return, not by a stopwatch. Speed is measured in `by_hand_*` tests and real runs (PRD-02 R8, PRD-05 A5) |
| Order of concurrent events assumed | Wait on an event (a channel, a file, a record), never on a sleep |

More rules:

- **File descriptors.** A test that opens more than 500 descriptors calls `common::fds::raise(n)` first and
  fails with a clear line when the hard limit is lower. The default soft limit is 1,024 on Linux and 256
  on macOS; `proxy_connect.rs` (2,000 connections, about 4,000 descriptors) would fail with EMFILE
  without it. Such tests run in the `heavy` group (section 8).
- **Root.** No test depends on not being root (a mode test checks mode bits, never an access refusal).
  The gate drops its root refusal (`gate.sh:32-39`): the build host runs as root.
- **Homes.** Every test uses a temporary `HOME` and `GOALCUP_HOME`. `common::home::guard()` (copied from
  `crates/goalcup-cli/tests/common/real_home_guard.rs`, 171 lines) fails a test that touched the real
  `~/.claude`, `~/.codex` or `~/.hermes` (AGENTS.md hard rule).
- **No network.** No test leaves loopback. Upstream names resolve through the test hook in section 5.
- **No retries.** nextest `retries = 0`: a retry hides a flake. A failure keeps its output in nextest's
  JUnit file (`target/nextest/default/junit.xml`), which the audit found missing for the open flake.
- **zstd in tests.** Tests write parts with `window_log` 20 (1 MiB). Only `golden.rs` and `recovery.rs`
  use 27 (141 MiB each; four test processes at once would hold about 560 MiB). See ask of PRD-01 2.
- **C code at opt-level 0.** `zstd-sys` and `sha2` compile at the dev profile's level in tests, which
  makes long-window compression slow. Ask of PRD-16 (workspace `Cargo.toml`): opt-level 3 for `zstd-sys`,
  `sha2`, `brotli-decompressor` and `flate2`'s backend in dev, as already done for regex (`Cargo.toml:40-54`);
  remove `[profile.dev.package.goalcup-daemon]` (`:61`) when that crate goes.

### 5. Test support (`crates/goalcup/tests/common/`) and the `test-hooks` feature

Published as compiling stubs at H+1.5 so other PRDs write tests against them; filled in by H+3.

| File | What | Lines (est.) | Copied from |
|---|---|---:|---|
| `mod.rs` | `within`, temp dirs, `bin()` for `CARGO_BIN_EXE_goalcup` | 60 | `cli/tests/common/m2_common/mod.rs` (240) |
| `ports.rs` | `held()`, `refusing()` (section 4) | 50 | new, `socket2` |
| `fds.rs` | `raise(n)` | 30 | new |
| `home.rs` | temp homes, real-home guard | 120 | `real_home_guard.rs` (171) |
| `pki.rs` | a test CA and leaf certificates | 120 | `m2_common` `TestPki` |
| `upstream.rs` | fake upstream: TLS with chosen ALPN, HTTP/1.1 (chunked, SSE, codings), HTTP/2 (`h2` crate server), WebSocket with permessage-deflate; scripted answers; records what it received | 700 | `m2_tls_support` (525), `m2_http_support` (486), `m2_ws_support` (783); HTTP/2 is new |
| `worker.rs` | fake Worker: position, chunk PUT, heartbeat, device and open routes from `docs/UPLOAD-API.md`; keeps chunk bodies; a script of injected answers (status, drop the answer after commit, `Retry-After`, delay) | 250 | new; never copies Worker rules (no continuity check) |
| `recorder.rs` | collects S1 calls with a settable `room()` (PRD-04) | 80 | wraps PRD-02's test constructor (ask of PRD-00 3) |
| `records.rs` | build a part folder from a list of records (for CLI and upload tests) | 100 | over `goalcup_format::PartWriter` |

**The fake Worker and the real Worker.** PRD-06 and PRD-10 ask for a fake Worker; PRD-07 asks that upload
tests run against `wrangler dev`, "not a second fake Worker". Both hold: the fast tier uses the small
scripted responder above for failure injection (it has no rules of its own, so it cannot drift into a
second Worker); the full tier's `upload_worker.rs` runs the real client against the real Worker with its
local database, then `scripts/fetch-run.sh`, and checks the fetched stream equals the local parts byte for
byte. The 2,514-line Rust stand-in (`cli/tests/common/upload_server/`) goes with the old crate.

**`test-hooks`.** One cargo feature on `crates/goalcup`, off by default, declared in PRD-02's scaffold
`Cargo.toml`. Integration tests turn it on with a dev-dependency of the crate on itself (today's pattern,
`crates/goalcup-daemon/Cargo.toml:76-79`), so `cargo nextest` builds the test binary with hooks and
`cargo build --release` builds the shipped binary without them. Every hook reads one `GOALCUP_TEST_*`
variable and is behind `#[cfg(feature = "test-hooks")]`. The registry (each owner builds its hook):

| Variable | Does | Owner | Used by |
|---|---|---|---|
| `GOALCUP_TEST_WRITER_STALL` | hold the writer for a time from a moment | PRD-02 | `pipeline_guard`, PRD-02 R2 |
| `GOALCUP_TEST_GUARD_MB` | set the memory guard | PRD-02 | PRD-02 R2, `e2e_run` |
| `GOALCUP_TEST_CRASH_AT_BOOT` | crash the next n boots | PRD-02 | PRD-02 R6 |
| `GOALCUP_TEST_SILENT_DROP` | `wire@<s>` or `native@<n>`: drop with no GAP (seam S11) | PRD-02 | PRD-08 R5, PRD-12 K3 and K4 |
| `GOALCUP_TEST_UPSTREAM` | `host=127.0.0.1:port` pairs: where the proxy connects for a name | PRD-03 | `e2e_run`, `proxy_*` through the binary |
| `GOALCUP_TEST_EXTRA_ROOTS` | a PEM file the proxy trusts for upstream TLS | PRD-03 | same |
| `GOALCUP_TEST_UPLOAD_POLICY` | round, trigger and backoff in ms | PRD-06 | `upload_*`, `e2e_run` |
| `GOALCUP_TEST_ROTATE_MB` | rotate parts at a small size | PRD-06 (PRD-01 writer) | PRD-06 U5, `e2e_run` |
| `GOALCUP_TEST_FSYNC_COUNT` | count `fdatasync` calls | PRD-06 | `upload_null` |

A hook outside this table is added here first (a night-log note to PRD-14). The release smoke (section 9)
fails if the shipped `goalcup` binary contains the text `GOALCUP_TEST_`. The pad's hook build goes to its
own folder: `cargo build --release -p goalcup --features test-hooks --target-dir target/hooks`, so a hook
binary can never be packaged or used for a "verified" row.

### 6. The code rule test (`crates/goalcup/tests/rule_code.rs`, about 180 lines)

Copied and cut from `crates/goalcup-protocol/tests/it/no_family_literals.rs` (378). Two rules:

1. **One system.** No string literal in `crates/goalcup/src/**` contains a connector id from `connectors/`
   (without `-cli`, as a whole token) or a host name from `connectors/model-hosts.toml` (providers,
   gateways such as `ai-gateway.vercel.sh` and `openrouter.ai`, vendor backends). Hosts and harnesses are
   data. Provider key shapes in `src/mask/` (for example `sk-`) are not host names and pass. An `ALLOWED`
   list holds exceptions with a reason, starts empty, and fails on a stale entry. `crates/goalcup-verify`
   is not scanned: its usage reader interprets per harness after the run, which is allowed.
2. **Ports.** No `bind("127.0.0.1:0")` or `bind("[::1]:0")` in any `crates/*/tests/**` file except
   `tests/common/ports.rs`.

### 7. The one end-to-end test (`crates/goalcup/tests/e2e_run.rs`, full tier, about 400 lines)

Replaces `m2_synthetic_vertical.rs` (881) and `m2_cli_lifecycle.rs` (1,944). One run of the real
`goalcup` binary (watchdog, recorder, proxy, native reader, masker, writer, uploader) in two minutes:

- A test connector in `crates/goalcup/tests/data/connectors/fake/` loaded through the connectors override
  (seam S13, PRD-12 ask 4). The "harness" is `sh` waiting for a stop file; the test itself plays the
  harness and its subagents: it sends traffic through `HTTPS_PROXY` and writes the session files.
- Traffic to two listed hosts on the fake upstream: HTTP/1.1 SSE with brotli, an HTTP/2 stream, a
  WebSocket with deflate carrying 1.5 MB client messages; a host that is also on the shared list as a
  gateway, called the same way; one unlisted host (a tunnel).
- 300 session files under the native roots, 100 growing at once, one truncated and rewritten.
- Planted secrets: one value per PRD-05 shape, a URL with a password, and an exact `.env` value, each
  sent split across two writes.
- One upstream outage (a `refusing()` port), two `kill -9` of the recorder, and one 20-second
  `GOALCUP_TEST_WRITER_STALL` with a 64 MiB guard.
- `goalcup seal`, upload to the fake Worker, then `goalcup-verify check --json --known`.

Pass: every response the test received is byte for byte what the upstream sent (point 3); every request
and WebSocket message is in DATA or inside a GAP (1); every session file has a native stream and its bytes
equal the file (2); zero known-value hits, every planted value has a MASK (4); the concatenated chunks the
fake Worker holds equal the local parts (5); `status.json` `written` equals the verifier's recount and off
time equals the two `recorder_down` gaps (6); the gateway host and the provider host have the same `conn`
shape (`decrypt`, `listed: shared`). Verifier exit 1 with only `named` results (the gaps are planned).

### 8. The gate (`scripts/gate.sh`, about 75 lines)

```
sh scripts/gate.sh fast    # every PRD branch before merge, every merge on integration, CI
sh scripts/gate.sh full    # phase 2 and 3 exits, before the owner's merge to main, before the tag
```

Start: check every prerequisite and print one line that names all missing ones, then fail before any
step: `cargo-nextest`, `zstd`, `python3`, `git`, `npm` with `server/worker/node_modules`; full adds
PostgreSQL (`initdb`, `pg_ctl`, `psql`, on PATH or under `/usr/lib/postgresql`). No fallback to
`cargo test`, no skipped step: a skip is a hidden pass (the same rule as PRD-12 C4). Then take the build
lock `target/.build-lock` (a `mkdir` lock with the holder's pid; a dead holder's lock is taken over),
shared with the pad's rebuild step (PRD-12 section 13 step 5) so two cargo jobs never share the 4 CPUs.

| Step | fast | full | Why |
|---|:-:|:-:|---|
| `cargo fmt --all --check` | yes | yes | cheap |
| `cargo clippy --locked $PKGS --all-targets -- -D warnings` | yes | yes | caught Linux-only bugs before (commit 84a26baf) |
| `cargo nextest run --locked $PKGS` | yes | yes | every guard in section 2 |
| `cargo test --locked $PKGS --doc` | yes | yes | nextest skips doctests; one line |
| `npm --prefix server/worker test` | yes | yes | the Worker changes in the same build (PRD-07); a wire change without it is run 1's risk again |
| `python3 -m unittest discover -s scripts` and `-s scripts/pad` | yes | yes | `test_pad.py` (PRD-12 ask 9), `test_fetch_run.py` (PRD-07), and the packaging tests below |
| `cargo nextest run --locked $PKGS --run-ignored only -E 'not test(/^(by_hand\|release)_/)'` | | yes | `e2e_run`, `upload_worker` (real Worker), other slow cases |
| `cargo build --release --locked -p goalcup -p goalcup-verify` | | yes | the binaries the pad copies; the release workflow builds the same way |
| `cargo nextest run --release --locked -p goalcup --run-ignored only -E 'test(/^release_/)'` | | yes | `release_mask_bench` floors (PRD-05 A5) |
| `sh server/worker/scripts/check-sql.sh` | | yes | PRD-07; the SQL mirror on a throwaway PostgreSQL |

- `$PKGS` is `--workspace` with one line in `gate.sh` that excludes the five old crates while they exist:
  `--exclude goalcup-cli --exclude goalcup-daemon --exclude goalcup-platform --exclude goalcup-protocol
  --exclude goalcup-verifier`. The old crates are never built, linted or tested from phase 0 (their tests
  are the old behaviour, which 1.0 replaces). PRD-13's cut deletes that one line in the same commit as the
  crates, so a new crate is in the gate by default.
- Every step prints its time; the last line stays `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.
- Test names: `by_hand_*` are `#[ignore]` and never in a tier (PRD-02 L1 becomes `by_hand_load`);
  `release_*` are `#[ignore]` and run only in the release-mode step; every other `#[ignore]` test is the
  full tier. No binary-name filter, so the probe and its fallback (`gate.sh:69-83`) go.
- `.config/nextest.toml` (about 25 lines): `fail-fast = false`, `retries = 0`, slow period 30 s with
  terminate after 6, JUnit on; group `heavy` (one at a time) for tests that kill processes, change limits
  or open thousands of descriptors: `binary(/^(watchdog_|e2e_)/) | binary(pipeline_restart) |
  binary(pipeline_many_streams) | binary(proxy_connect) | binary(proxy_restart) | binary(native_walk)`;
  e2e tests may run 600 s. A builder who adds such a test adds its binary here (a night-log note).

**Target times** (4 vCPU, warm, nothing else running): `fast` under 3 minutes, alarm at 5; `full` under
15 minutes, alarm at 20. Past an alarm, the night log names the slowest step and test (from JUnit); the
gate does not fail on time. The first measured numbers go into `README.md` "The gate" (PRD-15).

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

**`linux-x86_64.yml`** is deleted with `linux-check.sh` and `linux-harness-smoke.sh` (PRD-12 cut list).

**`release.yml`**: the same four targets. Before the release build, every target runs
`cargo nextest run --locked -p goalcup-format -p goalcup -p goalcup-verify` (Rust guards only), so macOS
arm64, macOS x86_64 and Linux arm64 run the watchdog, clock and socket-passing tests on their own machines;
nothing else does tonight (PRD-00 risk "macOS-only code paths"). Then build `-p goalcup -p goalcup-verify`,
package, smoke, install, publish (publish only on a tag push, as today, `release.yml:93-120`).

**`scripts/package-release.sh`**: two binaries; connectors from `git archive HEAD` as today.
**`scripts/smoke-release.sh`** (about 45 lines): with an empty environment, both binaries print
`<name> <version>`; the tarball's connectors equal the repository's (kept); `goalcup-verify dump` of PRD-01's
worked example equals the committed dump (ask of PRD-01 1), which proves the shipped reader on each target;
`grep -ac GOALCUP_TEST_ bin/goalcup` is 0. The `doctor` and pin checks go (`smoke-release.sh:39-54`).
**`scripts/third-party-licenses.py`**: `SHIPPED = {"goalcup", "goalcup-verify"}` (`:24`).

**`packaging/install.sh`** (PRD-10 interface): installs `goalcup` and `goalcup-verify`; on upgrade it deletes
a `goalcup-daemon` left by 0.7 in the prefix; the run guard asks `goalcup status --json` and blocks when
`status.mode` is `recording`, `losing`, `writer_slow` or `null` (PRD-02 section 6), and an answer it cannot
read is `INSTALL_UNSURE` and goes on (no reader for 0.7's signed status: no backward compatibility); one PATH
line that both exports and appends to the right profile file (H8). About 300 lines.

**`scripts/release.sh`**: the version check call goes (`:101-104`); the next-step text says "gate full, then
the pad's release check" instead of `linux-check.sh`. `tag` first runs `scripts/pad/table --release-check
<commit>` (PRD-12 interface): exit 0 only when the core set (Claude Code, Codex, Hermes) is PASS in all
three cases, with a trusted pad, built from a commit with no difference to the tag commit under `crates/`,
`server/worker/src/`, `connectors/` and `Cargo.lock`. So the tag is the binary the pad verified.

**Release order** (PRD-00 question 8): owner merges to main; `gate full` on main; `release.sh tag 1.0.0`;
`release.yml` publishes; `install.sh` from `releases/latest` checked on Linux and the owner's Mac; then the
owner deploys the Worker with `MIN_COMPETITION_VERSION` 1.0.0 (PRD-07). The Worker floor goes last, so no
participant is refused while 1.0 cannot be downloaded.

### 10. Repository files that name old paths

- `.github/CODEOWNERS`: rewritten to the new owned paths: `/connectors/`, `/crates/goalcup/src/mask/`,
  `/crates/goalcup/src/connector/`, `/crates/goalcup-format/`, `/scripts/pad/`,
  `/skills/adding-a-capture-harness/`. `scripts/test_codeowners.py` stays and keeps it honest.
- `.github/PULL_REQUEST_TEMPLATE.md`: the checklist asks for the `gate fast` PASS line, and for a connector,
  the pad's `row.json` (PRD-12 contributor path). The `pin-manifests.sh` and `check-connector.sh` lines go.
- `.github/dependabot.yml`: kept; weekly update pull requests run the gate through CI.

## Files it owns

Create: `crates/goalcup/tests/common/**` (section 5), `crates/goalcup/tests/rule_code.rs`,
`crates/goalcup/tests/e2e_run.rs`, `crates/goalcup/tests/data/connectors/fake/` (ask of PRD-00 1).

Change: `scripts/gate.sh` (113 to about 75), `.config/nextest.toml` (61 to about 25),
`.github/workflows/ci.yml` (93 to about 50), `.github/workflows/release.yml` (120 to about 125),
`scripts/package-release.sh`, `scripts/smoke-release.sh`, `packaging/install.sh` (394 to about 300),
`packaging/README.md`, `scripts/release.sh`, `scripts/third-party-licenses.py`, `scripts/test_gate.py`
(226 to about 110), `scripts/test_install.py`, `scripts/test_release.py`, `.github/CODEOWNERS`,
`.github/PULL_REQUEST_TEMPLATE.md` (the last four through ask of PRD-00 1).

Delete: `.github/workflows/linux-x86_64.yml`, `scripts/ci-scope.py`, `scripts/test_ci_scope.py`,
`scripts/check-version.py`, `scripts/test_check_version.py`, `scripts/check-links.py`,
`scripts/test_check_links.py` (ask of PRD-00 1 for the last four).

Must not touch: any test file another PRD names (`pipeline_*`, `proxy_*`, `native_*`, `mask_*`, `upload_*`,
`cli_*`, `connector_*`, `lessons_*`, `crates/goalcup-format/tests/**`, `crates/goalcup-verify/tests/**`),
`crates/goalcup/Cargo.toml` (PRD-02), the workspace `Cargo.toml` (PRD-16), `server/worker/**` and
`scripts/fetch-run.sh` (PRD-07), `scripts/pad/**` (PRD-12), `docs/*.md`, `README.md`, `AGENTS.md` (PRD-15).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-01 | `PartWriter` with a `window_log` parameter; the worked example as committed files; golden and recovery tests in the fast tier | Asks below |
| PRD-02 | `test-hooks` feature in its scaffold; hooks in section 5's registry; a test constructor for `Recorder` (ask of PRD-00 3); `pipeline_guard.rs` asserts "returns while the writer is held", and the "1 ms at the 99.9th percentile" figure moves to `by_hand_load` and R8 (section 4 rule; a sub-millisecond bound on a shared 4-CPU host is the old flake class) | Both |
| PRD-03 | `common::upstream` (TLS, HTTP/1.1, HTTP/2, WebSocket); `GOALCUP_TEST_UPSTREAM`, `GOALCUP_TEST_EXTRA_ROOTS`; `proxy_connect.rs` calls `fds::raise`; `panic = "unwind"` stays (no profile sets `abort`) | Both |
| PRD-04 | `common::recorder` with settable `room()`; `native_walk.rs` in the `heavy` group | Used there |
| PRD-05 | `mask_bench` becomes `release_mask_bench`, run in the full tier's release step | Used there |
| PRD-06 | `common::worker`; owns `upload_worker.rs` (full tier, real Worker, `fetch-run.sh`, equal bytes) | Both |
| PRD-07 | Gate runs `npm test` (fast and full), `test_fetch_run.py` (fast), `check-sql.sh` (full, required) | Used here |
| PRD-08, PRD-09 | `cargo nextest` covers `crates/goalcup-verify` in the fast tier; their own `tests/common/` record builders stay in that crate | Used here |
| PRD-10 | `install.sh` reads `status --json` `status.mode`; `cli_*.rs` use `common::worker` and `common::home` | Both |
| PRD-11 | Connectors override (S13) loads the test connector in `e2e_run.rs`; `model-hosts.toml` is read by `rule_code.rs` | Used here |
| PRD-12 | Shared build lock `target/.build-lock`; `test_pad.py` in the fast tier; `scripts/pad/table --release-check <commit>` for `release.sh tag`; hook builds in `target/hooks` | Both |
| PRD-13 | Deletes the old crates, their tests and the cut scripts, and in the same commit deletes the old-crate exclude line in `gate.sh` (one line PRD-14 lets PRD-13 edit) | Done there |
| PRD-15 | README "The gate" (two tiers, prerequisites, measured times, the guard list of section 2 as the promise list), AGENTS.md lines 68-77 and 105, `docs/OPERATING.md` "CI" (line 530), `scripts/README.md` | Written there |
| PRD-16 | Workspace `Cargo.toml` profile changes (section 4); `gate fast` at every merge; G1 to G7 in the night schedule | Used there |
| PRD-17 | `lessons_*.rs` run in the fast tier; deleting old crates waits for them | Used here |

## What is cut

| Path | Src lines | Tests that go | Who deletes |
|---|---:|---|---|
| All tests of the five old crates | | 126,112 (`crates/*/tests`, fixtures excluded), 16,335 inline, 6,939 fixture lines; only the masking corpus (PRD-05), proxy fixtures (PRD-03) and usage fixtures (PRD-09) are copied, and about 1,200 lines of test support into `tests/common/` | PRD-13 |
| Test runner tools: `test_campaign.py`, `test_linux_matrix.py`, `test_matrix.py`, `test_linux_check.py`, `test_check_connector.py` | | 4,589 | PRD-13 (PRD-12 list) |
| Research tests: `test_research_*.py` (5), `test_early_crosscheck.py` | | 2,005 | PRD-13 |
| `scripts/check-version.py` | 206 | `test_check_version.py` 128 | here. Reason: it fails today on plan docs (86 findings), serves no point, and release.yml already refuses a tag that differs from `Cargo.toml` (`release.yml:55-62`); the smoke checks every binary prints the version |
| `scripts/check-links.py` | 1,349 | `test_check_links.py` 1,672 | here. Serves no point; PRD-15 rewrites most docs tonight and would fight it all night |
| `scripts/ci-scope.py` | 90 | `test_ci_scope.py` 134 | here. CI runs the gate; nothing to scope |
| `.github/workflows/linux-x86_64.yml` | 63 | none | here. Its two scripts go (PRD-12) |
| `scripts/gate.sh` root refusal, version step, binary probe, fallback | about 40 | 6 of 16 cases in `test_gate.py` | here |
| `.config/nextest.toml` old groups and priorities | about 40 | none | here |
| Smoke `doctor` and pin check, install guard's daemon-status parsing | about 110 | install guard cases rewritten to `status.mode` fixtures written inline (no protocol crate fixtures) | here |
| **Here, in all** | **about 1,900** | **about 2,000** | |

Script tests that stay: `test_gate.py`, `test_install.py`, `test_release.py`, `test_codeowners.py`,
`test_third_party_licenses.py` (here), `test_fetch_run.py` (PRD-07), `scripts/pad/test_pad.py` (PRD-12).

## Acceptance

### Real-run proofs (first)

| # | Proof | Pass |
|---|---|---|
| G1 | **Gate time, measured.** `gate fast` and `gate full` on the build host at the end of phases 0, 2 and 3, with the step times | Fast under 5 minutes warm, full under 20; numbers and the three slowest tests (from JUnit) in the night log |
| G2 | **The guards catch the failures that matter.** A watcher makes four one-line sabotages on a scratch branch, one at a time: the writer skips one GAP record; COUNT counts a record at offer instead of at write; the `url_password` rule is removed; the proxy refuses the 1,001st connection | `gate fast` FAILs each time on a guard of section 2 named in advance (`check_invariants`/`pipeline_guard`, `check_invariants` inv 7, `mask_rules`, `proxy_connect`). Branch deleted after. A sabotage the gate misses is a missing guard: night-log note to the owning PRD |
| G3 | **No flake.** `gate fast` five times in a row on the loaded build host at the phase 3 commit, no code change | Five PASS. Any failure is fixed or named in the night log with its JUnit output; never retried away |
| G4 | **CI is the gate.** The push of the phase 3 commit to `v1.0/integration` | `check` green on the 4-CPU runner; its time in the night log |
| G5 | **Every target runs the guards.** `gh workflow run release.yml --ref v1.0/integration` after phase 3 (a manual start publishes nothing, `release.yml:93-95`) | Four green builds: Rust guards pass on macOS arm64, macOS x86_64, Linux x86_64 and Linux arm64; smoke and `install.sh` pass; `GOALCUP_TEST_` absent from every shipped `goalcup`. Whether the overnight token may start a workflow is unverified; if not, the owner starts it in the morning |
| G6 | **The tag is what the pad verified.** `scripts/release.sh tag 1.0.0` on a commit that differs from the pad's core rows under `crates/` | Refused with the differing paths named; on the verified commit it tags |
| G7 | **Install from the release.** After the owner publishes: `curl .../install.sh \| sh` on Linux and on the owner's Mac, over a 0.7 install | Two binaries installed, `goalcup-daemon` removed, one PATH line printed, `goalcup --version` prints 1.0.0 |

### Automated tests that guard them

- `scripts/test_gate.py` (about 110 lines): fast runs fmt, clippy, nextest, doctests, Worker tests and script
  tests in that order and nothing from full; full adds its four steps; missing prerequisites are all named on
  one line and no step runs; the old-crate exclude line is in `$PKGS`; the last line format; a held build
  lock waits and a dead holder's lock is taken.
- `scripts/test_install.py`: two binaries; a 0.7 `goalcup-daemon` is removed on upgrade; each `status.mode`
  value blocks or not as section 9 says; unreadable output gives `INSTALL_UNSURE`; one PATH line. Existing
  rollback and checksum cases kept.
- `scripts/test_release.py`: `tag` stops when the release check fails and says why.
- `crates/goalcup/tests/rule_code.rs` (section 6) and `e2e_run.rs` (section 7).

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Too few automated tests | A regression between pad runs ships | Every merge runs the guards; the pad reruns the core set on the release commit (G6); G2 shows the guards bite |
| Worker tests in the fast tier are slow | Their time is not recorded anywhere (`J-tests-gate.md:422`) | G1 measures; if over 90 s, they move to full with a night-log note and CI keeps them |
| CI on every integration push | Many runs overnight | Public repository, so free minutes; newest run only |
| CI hangs like the old suites | `ci.yml:15-16` | The `heavy` group runs one at a time; the hanging test moves to full |
| `e2e_run.rs` depends on every PRD | It cannot pass until phase 2 | It is full tier only; phase 1 merges need only fast |
| `rule_code.rs` false hits | A host name in an error message or a comment-like string | Only string literals; `ALLOWED` with reasons |
| Cutting the link check | Broken links in the participant guide (point 6) | PRD-15 opens each link of the guide once; cheap to restore later |
| Cutting the version check | A second version string slips into a doc | The release workflow checks tag and `Cargo.toml`; the smoke checks every binary's version |
| The build host lacks tools | `zstd`, `cargo-nextest`, `node_modules` are missing today | The gate names all missing tools on its first line; a builder installs them in phase 0 (`apt install zstd`, `cargo install cargo-nextest --locked`, `npm ci`) |
| Hooks leak into a shipped binary | A participant run with a test hook | Feature off by default, hooks binary in `target/hooks`, smoke greps the shipped binary |
| macOS only in CI | No Mac on the build host | G5 runs the guards on two macOS runners; the owner's pad run on a Mac (PRD-12 P9) |
| Load numbers only by hand | Throughput regressions are not caught by the gate | PRD-02 R8 and PRD-05 A5 on the pad; release floors in the full tier |
| `binary(/regex/)` filters in nextest groups | Unverified that a regex matching no binary is accepted (the exact form was refused, `gate.sh:73-74`) | The builder checks with `cargo nextest list` in phase 0 |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Fast target and alarm | 3 and 5 minutes, warm, idle 4 vCPU | PRD-16 after G1 |
| Full target and alarm | 15 and 20 minutes | PRD-16 after G1 |
| Worker tests | in fast | PRD-16 after G1 |
| CI | runs `gate fast` on PRs into main, pushes to main and integration | Owner (the old rule was "CI stays lean", `ci.yml:5`) |
| Release workflow | Rust guards on all four targets before the build | Owner |
| Test budget | about 18,000 Rust test lines | PRD-16 |
| nextest | required, no `cargo test` fallback, no retries | PRD-14 |
| Link and version checks | cut | Owner |
| Release check on tag | core set PASS on a commit with no code difference | Owner |

## Builder notes

Order of work:

1. **Phase 0 (by H+1.5).** `gate.sh` with the old-crate exclude line and the prerequisite check; delete the
   version check step (the gate is red today without this). `.config/nextest.toml`. `tests/common/` as
   compiling stubs (`ports`, `fds`, `home`, `pki` real; `upstream`, `worker`, `recorder` stubs). Install the
   missing tools on the build host. Measure G1 on the scaffold.
2. **By H+3.** `upstream.rs` (TLS, HTTP/1.1, WebSocket), `worker.rs` (S5 stable at H+2), `recorder.rs`
   (after PRD-02's test constructor). HTTP/2 by H+4.
3. `ci.yml`; push the integration branch once to see it run.
4. `rule_code.rs` (after PRD-11's `model-hosts.toml` at phase 0).
5. Packaging and release changes, after PRD-10's `status --json` is stable.
6. `e2e_run.rs` in phase 2. Then G1, G2, G3 at phase 3, then G4 and G5.

Traps from the audit:

- The open flake is a port reuse race; never write `bind(":0")` then drop (`J-tests-gate.md:471-472`).
- Sub-second bounds on a loaded 4-CPU host are the main flake source (`J-tests-gate.md:474`, `:496-498`).
- nextest refuses a filter naming a binary that does not exist (`gate.sh:73-74`); the gate uses test-name
  prefixes, not binary names.
- The gate's `--offline` needed a prior `cargo fetch` (AGENTS.md:76-77); new crates add dependencies
  tonight, so the gate uses `--locked` and lets cargo fetch.
- `test_install.py` reads status fixtures from the protocol crate (`:28`): rewrite before PRD-13 deletes it.
- `test_codeowners.py` fails the moment an owned path is deleted: rewrite CODEOWNERS in PRD-13's commit or
  before it.
- The build host is root; the Vercel Sandbox user is `ubuntu`. Tests must pass as both.

How a watcher proves it in a real run: run `sh scripts/gate.sh fast` and read the step times; run G2's four
sabotages and read which test failed; push and read `gh run list --workflow ci.yml --branch v1.0/integration`;
after the core-set pad rows pass, run `scripts/release.sh tag` on a scratch commit that changes a line under
`crates/` and see the refusal.

## Asks of PRD-00 and PRD-01

PRD-00:

1. **Ownership gaps.** Give PRD-14: `.github/**` (not only workflows: `CODEOWNERS`,
   `PULL_REQUEST_TEMPLATE.md`, `dependabot.yml`, `ISSUE_TEMPLATE/`), `.config/nextest.toml`,
   `scripts/check-version.py`, `scripts/check-links.py`, `scripts/third-party-licenses.py` and their tests,
   `crates/goalcup/tests/rule_*.rs`, `crates/goalcup/tests/e2e_*.rs` and `crates/goalcup/tests/data/`.
   `scripts/README.md` has no owner; PRD-15 is the natural one.
2. **The gate is red today.** `python3 scripts/check-version.py` exits 1 at 7fd36019 (86 findings in
   `docs/plans/`, including `PRD-00-index.md:325`). The merge rule "gate fast passes" cannot be met until
   PRD-14's phase 0 change lands. Put "PRD-14 gate change" first in phase 0, before any merge.
3. **Seam S1 needs a test form.** PRD-04 wants a fake `Recorder` with a settable `room()`; PRD-03 wants a
   recorder stub. `pipeline::Recorder` is a concrete type. Add to S1: PRD-02 provides
   `Recorder::for_tests(room) -> (Recorder, Calls)` under `test-hooks`, stable by H+1.5.
4. **Old crates out of the gate from phase 0.** State that the old crates are never built, linted or tested
   on integration (the gate excludes them) and are read only by PRD-17. Otherwise every merge pays for
   126,112 lines of old tests and their known flake.
5. **Fake Worker conflict.** PRD-06 and PRD-10 ask for a fake Worker in `tests/common/`; PRD-07 says "not a
   second fake Worker". Record the split of section 5: a scripted responder with no rules in the fast tier,
   the real Worker in the full tier and on the pad.
6. **Definition of done item 4** becomes: "`gate full` passes with the old crates deleted, CI `check` is
   green on the same commit, and the release workflow's manual run is green on all four targets."
7. **Release order** (question 8): add "the Worker floor `MIN_COMPETITION_VERSION` is deployed after the
   1.0.0 release is published", so no participant is refused while 1.0 cannot be downloaded.
8. **Workspace profile** (PRD-16's file): opt-level 3 in dev for `zstd-sys`, `sha2`, `brotli-decompressor`
   and the deflate backend; drop `[profile.dev.package.goalcup-daemon]` with the crate.

PRD-01:

1. **Commit the worked example as files**: `crates/goalcup-format/tests/data/worked-example.gcr` (the 733 raw
   bytes) and `worked-example.dump.jsonl` (section 16's dump), and have `golden.rs` read them. The release
   smoke runs the shipped `goalcup-verify dump` on them on every target.
2. **`window_log` as a `PartWriter` parameter**, default 27, so tests use 20 and do not hold 141 MiB each;
   `golden.rs` and `recovery.rs` keep 27.
3. **The run 1 converter** (acceptance 4, `tests/tools/`) is a `by_hand_*` test or a separate tool, never a
   test the gate builds that needs run 1's server copy.
