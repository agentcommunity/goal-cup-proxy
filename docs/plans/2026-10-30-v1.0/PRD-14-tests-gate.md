# PRD-14: Tests and gate

Id: PRD-14. Version: goalcup 1.0. Status: plan for the build, revision 2 (after the three reviews of
30 October). Date: 30 October 2026. Base: main at 4b226de2 (this branch: 7f9d88a8). Index:
[PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md). Main proof: [PRD-12](PRD-12-test-pad.md).

## Goal

Real runs on the pad prove the seven points; a small set of automated tests guards the three things a
real run cannot pin down on every change: the record format, masking, and the never-stop rules. One
gate script, two tiers, the same command in a build box and in CI.

## Points served

All seven, each through one pad check (PRD-12 C1 to C6) and a few named guard tests (section 2).
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
- Builds, tests and the gate run in Vercel Sandbox build boxes as `ubuntu` (ledger B1, PRD-00 "Where
  the build runs"). The conductor never compiles. The gate keeps its root refusal.
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
| **Real runs (PRD-12 pad)** | All seven points on real harnesses, keys and the staging Worker: wire against the harness's own log, every subagent, no secret, server copy whole, counts the participant saw | Pad boxes; the owner's Mac after the build | Keys, minutes, money |
| **`gate full`** | One short end-to-end run of the real binary through every part, the real Worker under `wrangler dev`, the shipped binary built and checked for hook text, the masking bench reported | Build box at phase 2 and 3 exits, before the owner's merge, before the tag | About 15 minutes |
| **`gate fast`** | Format, masking, never-stop, upload contract, connectors, participant words, code rules | Build box before every merge into integration; CI on every push | Under 5 minutes |

Rule: an automated test is written only where a real run cannot show the failure every time. A real run
cannot plant every secret shape split at every byte, cannot kill the recorder at every point, and cannot
fill a guard on demand. It can show everything else, so the rest is left to the pad.

### 2. The seven-point guard list

These are the tests AGENTS.md's new rule protects. PRD-15 writes this table into README in place of P1 to
P7. A guard is deleted only with its point, or replaced in the same change by a test with the same name
prefix that asserts the same promise.

| Point | Real-run proof | Guard tests (fast tier unless marked) | Owner |
|---|---|---|---|
| 1 Every model call | PRD-12 C1; PRD-03 proofs | `proxy_connect.rs` (2,000 connections, no refusal), `proxy_ws.rs` (Codex deflate fixture), `proxy_http1.rs`, `lessons_*.rs` (PRD-17) | PRD-03, PRD-17 |
| 2 Every subagent | PRD-12 C2 | `native_walk.rs` (20,000 files, no cap), `native_capture.rs`, `native_resume.rs`, `pipeline_many_streams.rs` (20,000 streams) | PRD-04, PRD-02 |
| 3 Never block | PRD-02 R1 kill test (plain `kill -9`, no hooks) | `pipeline_guard.rs` (every S1 call returns while the writer is held; every drop is a GAP), `pipeline_stall.rs` (real binary, 90 s stall, every client exchange completes), `pipeline_no_latch.rs` (1,000 guard episodes, recording resumes each time), `watchdog_restart.rs` (`kill -9`, same port, connection served), `watchdog_null.rs` (crash loop, connections answered) | PRD-02 |
| 4 No secrets | PRD-12 C3 | `mask_stream.rs` (every split point), `mask_rules.rs` (`url_password`, left guard), `secrets_scan.rs` | PRD-05, PRD-08 |
| 5 Upload everything | PRD-12 C4 | `upload_chunks.rs` (concatenated chunks equal the parts), `server_compare.rs`, Worker `test/integration.test.ts`; full: `upload_worker.rs` | PRD-06, PRD-08, PRD-07 |
| 6 Tell plainly | PRD-12 C6 | `check_invariants.rs` invariant 7 (COUNT equals a recount), `cli_finish.rs` ("Nothing was lost" only with zero gaps and zero off time) | PRD-08, PRD-10 |
| 7 Real harnesses | PRD-12 table, tiers | `connector_builtin.rs`, `connector_launch.rs`, `rule_code.rs` (section 6), `scripts/test_install.py` | PRD-11, PRD-14 |
| Format (base of 1, 5, 6) | PRD-01 acceptance on the pad | `golden.rs`, `recovery.rs`, `check_invariants.rs` | PRD-01, PRD-08 |
| All together | PRD-00 done items 1 and 2 | full: `e2e_run.rs` (section 7) | PRD-14 |

### 3. Test budget

No test pins a cap, a byte-for-byte golden of internal facts, or a shape nobody relies on. One file per
module; cases named for the promise they keep. This table is the one source (PRD-00 size table, PRD-13
copies it). Numbers come from each PRD where it states one.

| PRD | Test files (from its Acceptance) | Lines (est.) |
|---|---|---:|
| PRD-01 format | `golden`, `roundtrip`, `recovery` | 800 (estimate here) |
| PRD-02 core | 12 `pipeline_*` and `watchdog_*` | 1,500 (PRD-02) |
| PRD-03 proxy | 7 `proxy_*` plus fixtures | 2,000 (PRD-03) |
| PRD-04 native | `native_*` | 1,000 (PRD-04 expects about 800) |
| PRD-05 masking | 6 `mask_*` plus the copied corpus (data) | 5,000 (PRD-05) |
| PRD-06 upload | `upload_*` plus `upload_worker` (full) | 1,000 (estimate here) |
| PRD-08 verifier | `check_invariants`, `check_named`, `check_conn_bytes`, `check_run1_shape`, `server_compare`, `secrets_scan` (no `judge_view`, deferred) | 1,100 (PRD-08) |
| PRD-09 usage | 5 `usage_*` | 600 (PRD-09) |
| PRD-10 CLI | `cli_*` | 1,200 (PRD-10) |
| PRD-11 connectors | 4 `connector_*` | 400 (estimate here) |
| PRD-17 lessons | `lessons_*` | 1,500 (estimate here) |
| PRD-14 | `tests/common/` (1,470), `rule_code.rs` (220), `e2e_run.rs` (150) | 1,850 |
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
- **Hooks reach any binary built with dev-dependencies.** `cargo nextest run`, `cargo clippy --all-targets`
  and `cargo nextest run --release` all build `goalcup` with `test-hooks` on (the trap named at
  `crates/goalcup-daemon/Cargo.toml:17-19`). So in `gate full` the release-mode test step runs before the
  release build, and the last step checks the shipped binary has no hook text (section 8).

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
| `worker.rs` | fake Worker: position, chunk PUT, heartbeat, device and open routes from `docs/UPLOAD-API.md` (S5); keeps chunk bodies; a script of injected answers (status, drop the answer after commit, `Retry-After`, delay) | 250 | new; never copies Worker rules (no continuity check) |
| `recorder.rs` | helpers over `Recorder::for_tests(room)` (S1): read the collected `Calls`, change `room()` during a test | 40 | none; S1 provides the test constructor |
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

### 6. The code rule test (`crates/goalcup/tests/rule_code.rs`, about 220 lines)

Copied and cut from `crates/goalcup-protocol/tests/it/no_family_literals.rs` (378). Four rules:

1. **One system.** No string literal in `crates/goalcup/src/**` contains a connector id (the file names
   of `connectors/*.toml`, leaving out `model-hosts.toml` and `tiers.toml`, as whole tokens) or a host
   name from `connectors/model-hosts.toml` (providers, gateways such as `ai-gateway.vercel.sh` and
   `openrouter.ai`, vendor backends). Hosts and harnesses are data. Provider key shapes in `src/mask/`
   (for example `sk-`) are not host names and pass. An `ALLOWED` list holds exceptions by file and text
   with a reason, and fails on a stale entry. It starts with one file: `src/connector/builtin.rs`, the
   one `include_str!` list that embeds the connector files by id (PRD-11). `crates/goalcup-verify` is
   not scanned: its usage reader interprets per harness after the run, which is allowed.
2. **Ports.** No `bind("127.0.0.1:0")` or `bind("[::1]:0")` in any `crates/*/tests/**` file except
   `tests/common/ports.rs`.
3. **No return.** PRD-13's no-return patterns (section 6 of PRD-13, as data in this file) match nothing
   under `crates/goalcup*/src/`: the daemon and its IPC, our own caps, disk handling, signing, pins and
   deny lists, termination proof, research decoders, `pause_at(`, `_FOR_TEST`. The `test-hooks` feature
   is not a pattern (PRD-00 ruling on consistency item 86). Provenance comments `v0.7.0:` are skipped.
   PRD-13 changes the list with a night-log note.
4. **After the cut.** Once `crates/goalcup-daemon/` is gone, PRD-13's reference guard (its `git grep`
   for the old crate names, with its exclusions) prints nothing, and rule 3's patterns match nothing
   under `server/worker/src/` (not `server/worker/sql/`, the database mirror). Before the cut this rule
   checks instead that the five old crates are in the workspace `exclude` list, so it is never a skip.

### 7. The one end-to-end test (`crates/goalcup/tests/e2e_run.rs`, full tier, about 150 lines)

Replaces `m2_synthetic_vertical.rs` (881) and `m2_cli_lifecycle.rs` (1,944). Small on purpose (review B
N12): it proves the parts join; each part's depth is in its own tests and on the pad. One run of the real
`goalcup` binary (watchdog, recorder, proxy, native reader, masker, writer, uploader) in about a minute:

- `goalcup start --test --connector tests/data/connectors/fake.toml --hosts tests/data/connectors/fake-hosts.toml`
  (S16). The "harness" is `sh` waiting for a stop file; the test plays the harness: it sends traffic
  through `HTTPS_PROXY` and writes the session file. `GOALCUP_SERVER` points at the fake Worker.
- Traffic: one HTTP/1.1 SSE answer with brotli to a `provider` host; one WebSocket with deflate carrying a
  1.5 MB client message; the same call to a `gateway` host; one unlisted host (a tunnel).
- One session file that grows while the run goes on.
- One planted secret (an `sk-` shaped value) sent split across two writes.
- One `kill -9` of the recorder; `GOALCUP_TEST_ROTATE_MB=1` so the entry has two parts.
- `goalcup seal`, upload to the fake Worker, then `goalcup-verify check --json --known`.

Pass, each assertion naming its owning PRD in its failure text so a failure points somewhere: every
response the test received is byte for byte what the upstream sent (PRD-03); every request and WebSocket
message is in DATA or inside a GAP (PRD-02); the session file has a native stream whose bytes equal the
file (PRD-04); zero known-value hits and the planted value has a MASK (PRD-05); the concatenated chunks the
fake Worker holds equal the local parts (PRD-06); `status.json` `written` equals the verifier's recount and
off time equals the one `recorder_down` gap (PRD-02, PRD-08); the gateway host and the provider host have
the same `conn` shape (`decrypt`, `listed: shared`) (PRD-03, PRD-11). Verifier exit 1 with only `named`
results (the gap is planned).

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
matters only where two cargo jobs share one machine: a build box used by the gate and PRD-12's rebuild
step, or the fallback of PRD-16 S8 (builds one at a time on the conductor as a non-root user).

| Step | fast | full | Why |
|---|:-:|:-:|---|
| `cargo fmt --all --check` | yes | yes | cheap |
| `cargo clippy --locked --workspace --all-targets -- -D warnings` | yes | yes | caught Linux-only bugs before (commit 84a26baf) |
| `cargo nextest run --locked --workspace` | yes | yes | every guard in section 2 |
| `cargo test --locked --workspace --doc` | yes | yes | nextest skips doctests; one line |
| `npm --prefix server/worker test` | yes | yes | the Worker changes in the same build (PRD-07); a wire change without it is run 1's risk again |
| `python3 -m unittest` on the kept script tests, and `discover -s scripts/pad` | yes | yes | `test_gate.py`, `test_install.py`, `test_release.py`, `test_codeowners.py`, `test_third_party_licenses.py`, `test_fetch_run.py` (PRD-07), `scripts/pad/test_pad.py` (PRD-12). A named list, not `discover -s scripts`: until the cut, 13 old script tests name old crates or `cargo` and would fail with the old crates out of the workspace. A new script test is added to the list |
| `cargo nextest run --locked --workspace --run-ignored only -E 'not test(/^(by_hand\|release)_/)'` | | yes | `e2e_run`, `upload_worker` (real Worker), other slow cases |
| `cargo nextest run --release --locked -p goalcup --run-ignored only -E 'test(/^release_/)'` | | yes | `release_mask_bench` prints MB/s and start-up time for the night log; it has no floor and fails only if it crashes (PRD-05, PRD-00 "no speed number fails a gate") |
| `cargo build --release --locked -p goalcup -p goalcup-verify` | | yes | the binaries the pad copies; after the release-mode test step, so the shipped build is the last one in `target/release` |
| `grep -ac GOALCUP_TEST_ target/release/goalcup` is 0 | | yes | section 4 "Hooks reach any binary built with dev-dependencies" |

- `--workspace` holds only the new crates: PRD-16's phase 0 workspace edit moves the five old crates to
  `exclude`, so they are never built, linted or tested again, and `gate.sh` has no exclude line.
- The version check and the link check are not steps. `server/worker/scripts/check-sql.sh` is not a step
  either: no schema change in this build (ledger S3) and `stopped-runs.sql` is after the merge (PRD-07).
- Every step prints its time; the last line stays `gate <tier>: PASS in <N>s` or `gate <tier>: FAIL`.
- Test names: `by_hand_*` are `#[ignore]` and never in a tier (PRD-02 `by_hand_load`); `release_*` are
  `#[ignore]` and run only in the release-mode step; every other `#[ignore]` test is the full tier. No
  binary-name filter, so the probe and its fallback (`gate.sh:69-83`) go.
- `.config/nextest.toml` (about 25 lines): `fail-fast = false`, `retries = 0`, slow period 30 s with
  terminate after 6, JUnit on; group `heavy` (one at a time) for tests that kill processes, change limits
  or open thousands of descriptors: `binary(/^(watchdog_|e2e_)/) | binary(pipeline_restart) |
  binary(pipeline_stall) | binary(pipeline_many_streams) | binary(proxy_connect) | binary(proxy_restart) |
  binary(native_walk)`; e2e tests may run 600 s. A builder who adds such a test adds its binary here (a
  night-log note).

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

**`packaging/install.sh`** (PRD-10 interface): installs `goalcup` and `goalcup-verify`; on upgrade it deletes
a `goalcup-daemon` left by 0.7 in the prefix and the 0.7 `share/goalcup/connectors/` folder; the run guard
asks `goalcup status --json` and blocks when `status.mode` is `recording`, `losing`, `writer_slow` or
`null` (PRD-02 S4), and an answer it cannot read is `INSTALL_UNSURE` and goes on (no reader for 0.7's
signed status: no backward compatibility); one PATH line that both exports and appends to the right profile
file (H8). About 300 lines.

**`scripts/release.sh`**: the version check call goes (`:101-104`); the next-step text says "gate full, then
the pad's release check" instead of `linux-check.sh`. `tag` first runs `scripts/pad/table --release-check
<commit>` (PRD-12 section 9): exit 0 only when the core rows and the Codex kill row are PASS with a trusted
pad, built from a commit with no difference to the tag commit under `crates/`, `server/worker/src/`,
`connectors/` and `Cargo.lock`, leaving out `connectors/tiers.toml`. So the tag is the binary the pad
verified, with only the tier text newer.

**Release order** (PRD-00 question 5, PRD-16 section 14): owner merges to main; `gate full` on main in a
build box; `release.sh tag 1.0.0`; `release.yml` publishes; `install.sh` from `releases/latest` checked on
Linux and the owner's Mac; then the owner deploys the production Worker (whose `MIN_COMPETITION_VERSION`
is 1.0.0, PRD-07). The floor reaches production last, so no participant is refused while 1.0 cannot be
downloaded.

### 10. Repository files that name old paths

- `.github/CODEOWNERS`: rewritten to the new owned paths: `/connectors/`, `/crates/goalcup/src/mask/`,
  `/crates/goalcup/src/connector/`, `/crates/goalcup-format/`, `/scripts/pad/`,
  `/skills/adding-a-capture-harness/`. `scripts/test_codeowners.py` stays and keeps it honest. It lands
  once those paths exist on integration (second slice).
- `.github/PULL_REQUEST_TEMPLATE.md`: the checklist asks for the `gate fast` PASS line, and for a connector,
  the pad's `row.json` (PRD-12 contributor path). The `pin-manifests.sh` and `check-connector.sh` lines go.
- `.github/dependabot.yml`: kept; weekly update pull requests run the gate through CI.

**When PRD-14's edits land** (review B M5, PRD-00 phase 3): every edit here lands as soon as it is green,
working with the old crates still on disk and out of the workspace. The phase 3 merge needs nothing from
this PRD except deleting `check-links.py` and its test after PRD-15's last run. Rule 4 of `rule_code.rs`
switches itself on when the old crates are gone.

## Files it owns

Create: `crates/goalcup/tests/common/**` (section 5), `crates/goalcup/tests/rule_code.rs`,
`crates/goalcup/tests/e2e_run.rs`, `crates/goalcup/tests/data/connectors/fake.toml` and `fake-hosts.toml`.

Change: `scripts/gate.sh` (113 to about 75), `.config/nextest.toml` (61 to about 25),
`.github/workflows/ci.yml` (93 to about 50), `.github/workflows/release.yml` (120 to about 120),
`scripts/package-release.sh`, `scripts/smoke-release.sh`, `packaging/install.sh` (394 to about 300),
`packaging/README.md`, `scripts/release.sh`, `scripts/third-party-licenses.py`, `scripts/test_gate.py`
(226 to about 110), `scripts/test_install.py`, `scripts/test_release.py`, `.github/CODEOWNERS`,
`.github/PULL_REQUEST_TEMPLATE.md`.

Delete: in phase 0, `scripts/check-version.py`, `scripts/test_check_version.py`, `scripts/ci-scope.py`,
`scripts/test_ci_scope.py`; with `ci.yml`, `.github/workflows/linux-x86_64.yml`, `scripts/linux-check.sh`,
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
| PRD-05 | `mask_bench` becomes `release_mask_bench`, run in the full tier's release step, report only | Used there |
| PRD-06 | `common::worker` (S5); owns `upload_worker.rs` (full tier, real Worker, `fetch-run.sh --local`, equal bytes) | Both |
| PRD-07 | Gate runs `npm test` (fast and full) and `test_fetch_run.py` (fast). `check-sql.sh` is not a gate step in this build | Used here |
| PRD-08, PRD-09 | `cargo nextest` covers `crates/goalcup-verify` in the fast tier; the six PRD-08 files of section 3 and PRD-09's `usage_*`; their own `tests/common/` record builders stay in that crate; that crate has no `test-hooks` | Used here |
| PRD-10 | `install.sh` reads `status --json` `status.mode`; `cli_*.rs` use `common::worker` and `common::home`; `start --connector --hosts` (S16) loads `e2e_run`'s fake connector | Both |
| PRD-11 | `model-hosts.toml` and the connector file names are read by `rule_code.rs`; `src/connector/builtin.rs` is its one allowed file; connectors are embedded, so packaging ships none | Used here |
| PRD-12 | Shared build lock `target/.build-lock` inside a box; `test_pad.py` in the fast tier; `scripts/pad/table --release-check <commit>` for `release.sh tag`, leaving out `connectors/tiers.toml`; the pad uses no hook build | Both |
| PRD-13 | Deletes the old crates, their tests and the cut scripts. Its reference and no-return guards live in `rule_code.rs` rules 3 and 4; it does not edit `gate.sh` | Done there |
| PRD-15 | README "The gate" (two tiers, prerequisites, measured times, the guard list of section 2 as the promise list), AGENTS.md lines 68-77 and 105, `docs/OPERATING.md` "CI" (line 530), `scripts/README.md`; runs `check-links.py` on its final docs before PRD-14 deletes it | Written there |
| PRD-16 | Workspace `Cargo.toml` (old crates in `exclude`, profile changes, the version comment); `gate fast` at every merge in a build box as `ubuntu`; G1, G3, G4 in its schedule; G2, G5, G6, G7 are release steps after the merge | Used there |
| PRD-17 | `lessons_*.rs` run in the fast tier; deleting old crates waits for them | Used here |

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
| `scripts/gate.sh` version step, binary probe, `cargo test` fallback, old steps | about 40 | 6 of 16 cases in `test_gate.py` | here |
| `.config/nextest.toml` old groups and priorities | about 40 | none | here |
| Smoke `doctor`, pin and connector checks; packaged connectors; install guard's daemon-status parsing | about 120 | install guard cases rewritten to `status.mode` fixtures written inline (no protocol crate fixtures) | here |
| **Here, in all** | **about 2,000** | **about 2,100** | |

Script tests that stay: `test_gate.py`, `test_install.py`, `test_release.py`, `test_codeowners.py`,
`test_third_party_licenses.py` (here), `test_fetch_run.py` (PRD-07), `scripts/pad/test_pad.py` (PRD-12).

## Acceptance

### Real-run proofs (first)

| # | Proof | When | Pass |
|---|---|---|---|
| G1 | **Gate time, measured.** `gate fast` and `gate full` in a build box at the end of phases 0, 2 and 3, with the step times | Build | Fast under 5 minutes warm, full under 20; numbers, the `release_mask_bench` report and the three slowest tests (from JUnit) in the night log |
| G3 | **No flake.** Every `gate fast` run in a build box and every CI `check` on integration during the build | Build, read at phase 3 | No red run that a rerun with no code change turned green. Any such run is named in the night log with its JUnit output and the test is fixed; never retried away |
| G4 | **CI is the gate.** The push of the phase 3 commit to `v1.0/integration` | Build | `check` green on the 4-CPU runner; its time in the night log |
| G2 | **The guards catch the failures that matter.** Four one-line sabotages on a scratch branch, one at a time: the writer skips one GAP record; COUNT counts a record at offer instead of at write; the `url_password` rule is removed; the proxy refuses the 1,001st connection | After merge (PRD-00) | `gate fast` FAILs each time on a guard named in advance (`check_invariants`/`pipeline_guard`, `check_invariants` inv 7, `mask_rules`, `proxy_connect`). Branch deleted after. A sabotage the gate misses is a missing guard: a note to the owning PRD |
| G5 | **Every target runs the guards.** `release.yml` on the release commit | Owner's release (PRD-00) | Four green builds: Rust guards pass on macOS arm64, macOS x86_64, Linux x86_64 and Linux arm64; smoke and `install.sh` pass; `GOALCUP_TEST_` absent from every shipped `goalcup` |
| G6 | **The tag is what the pad verified.** `scripts/release.sh tag 1.0.0` on a commit that differs from the pad's core rows under `crates/` | Owner's release; also by a watcher on a scratch commit at the end of the build (PRD-16 N5) | Refused with the differing paths named; on the verified commit it tags (a differing `connectors/tiers.toml` alone does not refuse) |
| G7 | **Install from the release.** `curl .../install.sh \| sh` on Linux and on the owner's Mac, over a 0.7 install | Owner's release | Two binaries installed, `goalcup-daemon` and the old connectors folder removed, one PATH line printed, `goalcup --version` prints 1.0.0 |

### Automated tests that guard them

- `scripts/test_gate.py` (about 110 lines): fast runs fmt, clippy, nextest, doctests, Worker tests and the
  named script tests in that order and nothing from full; full adds its five steps, the release-mode test
  step before the release build and the hook grep last; root is refused unless `GOALCUP_GATE_ALLOW_ROOT=1`;
  missing prerequisites are all named on one line and no step runs; the last line format; a held build
  lock waits and a dead holder's lock is taken.
- `scripts/test_install.py`: two binaries; a 0.7 `goalcup-daemon` and connectors folder are removed on
  upgrade; each `status.mode` value blocks or not as section 9 says; unreadable output gives
  `INSTALL_UNSURE`; one PATH line. Existing rollback and checksum cases kept.
- `scripts/test_release.py`: `tag` stops when the release check fails and says why.
- `crates/goalcup/tests/rule_code.rs` (section 6) and `e2e_run.rs` (section 7).

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Too few automated tests | A regression between pad runs ships | Every merge runs the guards; the pad reruns the core set on the release commit (G6); G2 after merge shows the guards bite |
| Worker tests in the fast tier are slow | Their time is not recorded anywhere (`J-tests-gate.md:422`) | G1 measures; if over 90 s, they move to full with a night-log note and CI keeps them |
| CI on every integration push | Many runs during the build | Public repository, so free minutes; newest run only |
| CI hangs like the old suites | `ci.yml:15-16` | The `heavy` group runs one at a time; the hanging test moves to full |
| `e2e_run.rs` depends on every PRD | It cannot pass until phase 2 | Full tier only; phase 1 merges need only fast. Each assertion names its owning PRD |
| `rule_code.rs` false hits | A host name in an error message; a no-return pattern in a word | Only string literals for rule 1; `ALLOWED` with reasons; PRD-13 narrows a pattern with a night-log note |
| Rule 4 before the cut | It cannot check the reference guard while old crates are on disk | It checks the `exclude` list instead, so it never passes by skipping; the cut turns the real check on |
| A hook binary ships | `cargo nextest run --release` leaves a hooked `goalcup` in `target/release` | Release build runs after it; `gate full`'s last step and the release smoke grep the binary |
| Named script-test list goes stale | A new `scripts/test_*.py` is never run | The list is one line in `gate.sh`; `test_gate.py` reads it; a new script test adds itself there (night-log note) |
| Cutting the link check | Broken links in the participant guide (point 6) | PRD-15 runs it once on the final docs before the delete; cheap to restore later |
| Cutting the version check | A second version string slips into a doc | The release workflow checks tag and `Cargo.toml`; the smoke checks every binary's version |
| Build box lacks tools | nextest, zstd or `node_modules` missing | The gate names all missing tools on its first line; PRD-16's build snapshot holds them (phase 0 step 6) |
| macOS only at release | No Mac in the build; G5 waits for the owner's release | A macOS-only guard failure shows at G5 before publish (publish needs a green build); the owner's pad run on a Mac (PRD-12 P9) |
| Load numbers only by hand | Throughput regressions are not caught by the gate | PRD-02 R8 and PRD-05 A5 reported on the pad and in G1; never a pass line |
| `binary(/regex/)` filters in nextest groups | Unverified that a regex matching no binary is accepted (the exact form was refused, `gate.sh:73-74`) | The builder checks with `cargo nextest list` in phase 0 |
| No test reproduces run 1's exact load | The suite passed while run 1 lost 37.5% | `pipeline_stall.rs` replays its shape (20 clients, 1.5 MB bodies, a WebSocket, a stalled writer); the pad's `full` rows and A1 are the real proof |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Fast target and alarm | 3 and 5 minutes, warm 8 vCPU build box | PRD-16 after G1 |
| Full target and alarm | 15 and 20 minutes | PRD-16 after G1 |
| Worker tests | in fast | PRD-16 after G1 |
| CI | runs `gate fast` on PRs into main, pushes to main and integration | Owner (the old rule was "CI stays lean", `ci.yml:5`) |
| Release workflow | Rust guards on all four targets before the build; first run at the owner's release | Owner (may start it by hand on integration earlier for an early macOS signal) |
| Test budget | about 18,000 Rust test lines | PRD-16 |
| nextest | required, no `cargo test` fallback, no retries | PRD-14 |
| Link, version and SQL-mirror checks | out of the gate | Owner |
| Release check on tag | core set PASS on a commit with no code difference, `tiers.toml` left out | Owner |
| Root refusal | kept, with `GOALCUP_GATE_ALLOW_ROOT=1` as today | Owner |

## Builder notes

Order of work:

1. **H+0, by the integrator (PRD-00 phase 0 step 2), from this section's text.** `gate.sh`: drop the
   version step, the binary probe and the fallback, add the prerequisite check and the build lock,
   `--workspace` steps as in section 8, the named script tests. Delete `check-version.py`, `ci-scope.py`
   and their tests. This must land before any other merge: the gate is red today without it.
2. **H+0, own lane.** `tests/common/` as compiling stubs by H+1.5 (`ports`, `fds`, `home`, `pki` real;
   `upstream`, `worker`, `recorder` stubs); `.config/nextest.toml`. Measure G1 on the scaffold.
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
- nextest refuses a filter naming a binary that does not exist (`gate.sh:73-74`); the gate uses test-name
  prefixes, not binary names.
- The gate's `--offline` needed a prior `cargo fetch` (AGENTS.md:76-77); new crates add dependencies, so
  the gate uses `--locked` and lets cargo fetch.
- `test_install.py` reads status fixtures from the protocol crate (`:28`): rewrite before PRD-13 deletes it.
- `test_codeowners.py` fails the moment an owned path is deleted: CODEOWNERS lists only new paths before
  the cut.
- Tests run as `ubuntu` in build boxes and as the runner user in CI; neither is root.
- Dev-dependency features unify into every test build, `--release` included (section 4).

How a watcher proves it in a real run: run `sh scripts/gate.sh fast` in a build box and read the step
times; push and read `gh run list --workflow ci.yml --branch v1.0/integration`; after the core-set pad rows
pass, run `scripts/release.sh tag` on a scratch commit that changes a line under `crates/` and see the
refusal (G6).

## Asks (after PRD-00 revision 2)

PRD-00 revision 2 ruled on all eight asks of revision 1 (accepted, item 6 partly: the release workflow is
the owner's step) and PRD-01 accepted all three (`PartOptions`, the 998-byte worked example as files, the
converter cut). Left:

1. **PRD-16:** section 5 says "PRD-14 G2 (sabotage) and G3 (five clean runs) run here" at phase 3. PRD-00
   moved G2 and G5 after the merge; G3 is now read from the build's own gate and CI runs, at no extra
   cost. Section 1 and traps: the build box is the only place the gate runs; this PRD keeps the root
   refusal. Phase 0 workspace edit: update the `Cargo.toml` version comment (`:14-17`, "the gate fails on
   any other hard-coded copy"), which is no longer true. Interfaces row PRD-11: tiers come from
   `connectors/tiers.toml`.
2. **PRD-13:** section 5 "same merge" no longer needs PRD-14's edits (they land earlier, section 10);
   section 6's guards are kept in `rule_code.rs` rules 3 and 4, and the `test-hooks` pattern is dropped
   from the no-return list; `check-version.py`, `check-links.py`, `ci-scope.py` and `linux-check.sh` with
   their tests are PRD-14's deletes, so drop them from group D and section 7. Section 7's
   `db-local.ts` reason stands (`wrangler dev` in the full tier).
3. **PRD-07:** `check-sql.sh` is not a gate step in this build; keep it for the owner's next schema change.

## Review answers

Items from the consistency check (CC), review A and review B that concern this PRD, or where it is the
natural owner. "Accepted" means the change is in the body.

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
