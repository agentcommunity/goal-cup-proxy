# PRD-13: Cuts

Id: PRD-13. Version: goalcup 1.0. Status: plan for the overnight build, phase 3 (PRD-00 "Build order").
Date: 30 October 2026. Base: main at 4b226de2 (working tree at 7fd36019, docs only since).
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md).

## Goal

Delete every old part that 1.0 does not run: the five old crates, the separate daemon, spikes, research
and disk-full scripts, old connector files and their tests. Leave a tree where nothing old can be
built, called or brought back by accident, and say how big the code is afterwards.

## Points served

None directly. The cuts serve the "less code, cut before you add" rule (`VISION.md`, BRIEF) and make
points 1, 2 and 3 easier to keep: the hand-off that lost 37.5% of run 1's calls is deleted, not
patched. Every cut is checked against the seven points in the table "Point check" below.

## Owner decisions it carries out

- All planned cuts are in this build: research decoders, research scripts, disk-full handling, pins,
  deny lists, termination proof, signing, the separate daemon, legacy and test seams, retired versions
  (BRIEF3, 29 Oct evening). Everything in the plan's after-Edition-01 list moves in if it serves no point.
- Fresh core in this repo. Old crates are deleted in the same build, after a lessons list is extracted
  (BRIEF3, 29 Oct night). Not a new repo; not edit in place.
- Architecture B, one process. The proxy to daemon hand-off (IPC, producers, epochs, the shared queue,
  failure budgets) is removed.
- One system for every harness, no patchwork (owner, 29 Oct night; the relayed request asks the same).
- No backward compatibility before Edition 01. No v5 reader is kept.
- Disk full: no special handling (owner, phase 2 decisions).

## Today

Counted with `wc -l` on the working tree. "src" is production code: each file minus its trailing
`#[cfg(test)] mod` block (brace matched), with `src/daemon/mock.rs`, `src/participant/tests.rs` and
`src/runtime/capture/tests.rs` counted as tests. This is the method of appendix A (A.0); totals differ
from A.1 by under 2% because A split a few files by hand.

| Area | Files | All lines | src | Inline tests | `tests/**/*.rs` | Other test files (fixtures, shell) |
|---|---:|---:|---:|---:|---:|---:|
| `crates/goalcup-cli` | 211 | 111,949 | 39,267 | 8,753 | 59,959 | 3,837 |
| `crates/goalcup-daemon` | 77 | 53,606 | 18,958 | 4,087 | 30,260 | 213 |
| `crates/goalcup-platform` | 22 | 7,599 | 5,039 | 856 | 1,684 | 0 |
| `crates/goalcup-protocol` | 73 | 26,647 | 11,099 | 1,284 | 13,471 | 720 |
| `crates/goalcup-verifier` | 165 | 36,441 | 12,153 | 1,335 | 20,724 | 2,230 |
| **Old crates** | **548** | **236,242** | **86,516** | **16,315** | **126,098** | **7,000** |
| `spikes/` | 184 | 23,713 (8.4 MB) | 349 (`redact.py`) | | | 23,364 data |
| `scripts/` | 46 | 21,878 | 11,406 (code and data) | | 10,393 (`test_*.py`) | README 79 |
| `server/worker/src`, `test` | | 9,776 | 4,001 | | 5,775 | |
| `connectors/*/harness.toml` | 4 | 556 | | | | |
| `harnesses/install.sh` | 1 | 62 | 62 | | | |

The old crates hold five binaries' worth of code for what 1.0 does in two binaries (`goalcup`,
`goalcup-verify`). The separate daemon and the hand-off to it (its process, control API, IPC,
registry, capture owner, the CLI's client side and spawn gate) are 17,964 src lines (group A of
"What is cut"); termination proof adds 2,428 more (group C).

## Design

### 1. What PRD-13 does, in one sentence

After phase 2 is green, PRD-13 deletes in one branch every path in the "What is cut" tables that it
owns, lands it in the same merge as the edits other PRDs must make to stop pointing at those paths, and
proves the tree builds, gates and records with nothing old left.

### 2. Rules for every cut

1. **Delete whole, never hollow out.** An old file is either copied into a new file by its receiving PRD
   (and then deleted here) or deleted. No old crate stays as a dependency, a feature or a shim.
2. **Copy first, delete second.** A path is deleted only when every receiving PRD in the copy ledger
   (section 4) has merged its copy and its tests pass on integration. PRD-17's lessons tests must pass
   on the new core (PRD-00 phase 2 gate).
3. **The old code is never lost.** It stays at main `4b226de2` and tag `v0.7.0`. A copied part names
   its origin in one comment of the form `copied from v0.7.0:crates/goalcup-cli/src/proxy/http.rs:18-26`.
   That form is the only allowed mention of an old crate path in code (section 6 guard).
4. **A cut that would block or slow the harness is not a cut.** Each row says which point could
   suffer and why it does not (table "Point check").
5. **No cap of our own comes back.** The guard in section 6 also checks that the cut concepts are not
   rebuilt in the new crates.

### 3. Why cutting is how we get one system

The relayed request asks for one system, not a patchwork: a gateway is just another HTTPS host, and
dodgers get the light path. PRD-00 already answers that in the design. This PRD removes the patchwork
that would otherwise sit beside it:

| Patchwork today | Where | 1.0 |
|---|---|---|
| Per-connector model host rows and per-harness host readers that refuse at start | `connectors/codex-cli/harness.toml:122-130`, `connectors/amp-cli/harness.toml:111`, `crates/goalcup-cli/src/model_host.rs` (893 src) | One shared host list plus participant and config hosts (PRD-11 `model-hosts.toml`); a hint never refuses |
| The Hermes "recorder" provider that set a model base URL | `connectors/hermes-cli/harness.toml:145-152` (`[launch_prep]`) and its `configure` rows | Gone. Hermes goes through `HTTPS_PROXY` like every harness (AGENTS.md hard rule, D1) |
| A start-time provider probe per connector | `crates/goalcup-cli/src/proxy/preflight.rs` (250 src, 170 tests) | Gone. A run with no model requests says so (PRD-10) |
| Per-harness termination tables and argv guards | `[capture.termination]` in all four connectors (for example `claude-code-cli/harness.toml:150`), `hermes-cli/harness.toml:124` | Gone with termination proof |
| Dodger prototypes and plugins | `spikes/openclaw` (7,887 lines), `spikes/cursor-cli` (1,681), plugin code in `spikes/hermes` and `spikes/opencode` | Gone. Dodgers get tunnel records, session files, a named gap and the tier "not supported" (PRD-00, PRD-03) |
| Five test harness kits: matrix, linux matrix, campaign, harness smoke, connector check | `scripts/matrix.sh`, `linux-matrix.sh`, `campaign.py`, `linux-harness-smoke.sh`, `check-connector.sh` (5,842 lines plus `kit_settle.py` 302; 4,482 test lines) | One test pad, also the contributor path (PRD-12) |

### 4. The copy ledger (preconditions for each delete)

PRD-13 deletes a source only after the receiving PRD has written "copied, tests green" for it in the
night log (PRD-16's `NIGHT-LOG.md`). Ranges are the owners' own citations.

| Source (old) | Copied into | By | What |
|---|---|---|---|
| `crates/goalcup-cli/src/proxy/{connect,tls,http,websocket}.rs`, `ca_bundle.rs` | `crates/goalcup/src/proxy/` | PRD-03 | About 2,250 lines: head parse, authority, ClientHello, CA and leaf, HTTP/1.1 framing, decoders, WebSocket framer and inflate |
| `crates/goalcup-daemon/src/redact.rs`, `redact/stream.rs`, `crates/goalcup-cli/src/proxy/headers.rs`, `crates/goalcup-protocol/src/run_secret.rs` | `crates/goalcup/src/mask/` | PRD-05 | Rules, hold algorithm, header and query rules, run secrets (about 3,050 lines) |
| `spikes/*/fixtures/*.jsonl` except `cursor-cli` and `claude-desktop` | `crates/goalcup/tests/mask_corpus/` | PRD-05 | 86 files, about 6.2 MB |
| `crates/goalcup-daemon/src/native_capture.rs`, `crates/goalcup-platform/src/sourcecap*.rs` | `crates/goalcup/src/native/` | PRD-04 | Discovery, generations, no-follow reads (about 120 lines of these) |
| `crates/goalcup-daemon/src/runtime/capture/native_plan.rs:96-151, 224-311` | `crates/goalcup/src/connector/` | PRD-11 | Root templates, home isolation |
| `crates/goalcup-platform/src/terminal.rs:1-60`, `crates/goalcup-cli/src/commands/start/proxy.rs:33-62` | `crates/goalcup/src/watchdog.rs` | PRD-02 | Terminal shield, open-file limit raise |
| `crates/goalcup-cli/src/upload/**` | `crates/goalcup/src/upload/` | PRD-06 | Rewritten, about 940 lines |
| `crates/goalcup-platform/src/awake.rs`, `machine.rs:515-534`, participant text, push lines, device login | `crates/goalcup/src/cli/` | PRD-10 | About 150 lines of sleep lock; trimmed text |
| `crates/goalcup-cli/src/model_host.rs:682` onward (YAML), `launch.rs` env build, `harness/schema.rs:905-1112` (mask grammar) | `crates/goalcup/src/connector/` | PRD-11 | About 370 lines |
| `crates/goalcup-verifier/src/capture_analysis/{openai,anthropic,chat,thread}.rs`, `native/*`, `stats.rs` (model, usage, error parts) and fixtures `m2-openai`, `m2-anthropic`, `m2-native`, `connectors` | `crates/goalcup-verify/src/usage/`, its tests | PRD-09 | Model and usage readers; fixtures as decoded bodies |
| `crates/goalcup-verifier/src/bin/goalcup-verify.rs:264-310` | `crates/goalcup-verify/` | PRD-08 | Output file creation, parse rule |
| Every old crate, test and `docs/DECISIONS.md` | `docs/plans/2026-10-30-v1.0/LESSONS.md`, `crates/goalcup/tests/lessons_*.rs` | PRD-17 | Edge cases as acceptance cases |

### 5. One branch, one merge

- Branch `v1.0/prd-13-cuts` from `v1.0/integration` after the phase 2 gate (PRD-00).
- Commits, in this order, each building on its own:
  1. Workspace members: remove the five old crates from `Cargo.toml:3-9`; drop workspace
     dependencies no member uses any more (expected: `ed25519-dalek`, `redb`, `axum`; check with
     `cargo tree`). Update `Cargo.lock` with one `cargo build --offline --workspace` (not `--locked`),
     then commit it. This edit is PRD-16's file (Ask 1).
  2. `git rm -r` the five old crate folders.
  3. `git rm -r spikes/`.
  4. `git rm` the scripts, `harnesses/install.sh` and `.claude/agents/campaign-cell.md` (group D).
  5. `git rm connectors/*/harness.toml` once PRD-11's `connectors/<id>.toml` and `model-hosts.toml` are
     on integration.
- **Same merge** (the integrator merges them together, or the gate goes red): PRD-14's edits to
  `scripts/gate.sh`, `.github/workflows/*.yml`, `.config/nextest.toml`, `.github/CODEOWNERS`,
  `packaging/install.sh`, `scripts/package-release.sh`, `scripts/smoke-release.sh`; PRD-15's
  `AGENTS.md` map, `README.md`, `scripts/README.md` and docs; PRD-12's skill edits. Each names a path
  this PRD deletes (section "Interfaces").
- PRD-07's Worker cuts and PRD-15's doc cuts do not wait for phase 3. They touch no old crate.

### 6. The guards (run once at the cut, then kept in the gate by PRD-14)

Reference guard. It must print nothing:

```
git grep -nE 'goalcup[-_](cli|daemon|platform|protocol|verifier)\b|goalcup-daemon' -- . \
  ':!docs/plans/**' ':!docs/archive/**' ':!docs/DECISIONS.md' ':!CHANGELOG*' | grep -v 'v0\.7\.0:'
```

No-return guard over the new code. Each pattern must print nothing under `crates/` and `server/`:

| Concept cut | Pattern |
|---|---|
| Separate daemon and its IPC | `goalcup-daemon`, `IngestListener`, `DaemonClient`, `epoch` in `src/proxy/` |
| Our own caps | `MAX_OPEN_STREAMS`, `MAX_OPEN_CONNECTIONS`, `MAX_LIVE_PRODUCERS`, `MAX_CONNECTION_FAILURES`, `MAX_RECORDER_RESTARTS`, `MAX_TUNNEL_NOTES`, `max_files` |
| Disk handling | `SIGSTOP`, `ReserveFile`, `disk_free`, `agent_paused` (client side) |
| Signing | `ed25519`, `receipt`, `acks.jsonl` |
| Pins and deny lists | `pin-manifests`, `connector_dir_sha256` (client side), `capture_policy` |
| Termination proof | `subreaper`, `released_child`, `Confirmed` termination |
| Test seams in shipped code | `test-hooks` feature, `pause_at(`, `_FOR_TEST` |
| Research decoders | `capture_analysis`, `--research-json`, `--capture-integrity` |

A hit is either a real reintroduction (the builder removes it) or a false match (the integrator notes
it in the night log and narrows the pattern).

### 7. What is not cut, and why

| Part | Lines | Why it stays |
|---|---:|---|
| `server/worker/src/ratelimit.ts` (device-flow limiter) | 139 src, 184 tests | Guards a real cost: the device flow is the only unauthenticated route (`ratelimit.ts:1-10`). The rule "a limit must guard a real cost and say so" is met. Plan K23 is dropped |
| `server/worker/src/db-local.ts` | 701 src, 590 tests | The pad and the gate run a local Worker with `DB_BACKEND=local` (PRD-00 default "Pad server") |
| `server/worker/sql/` | 4,468 | Mirror of the website database; no migration overnight |
| `scripts/fetch-run.sh`, `test_fetch_run.py` | 349, 695 | Rebuild by ID and time (point 5); PRD-07 changes it |
| `scripts/gate.sh`, `release.sh`, `package-release.sh`, `smoke-release.sh`, `third-party-licenses.py`, `ci-scope.py` and their tests | 998 src (with `fetch-run.sh`), 1,999 tests (with `test_fetch_run.py`, `test_install.py`, `test_codeowners.py`) | Release and gate; PRD-14 changes them |
| `packaging/install.sh` | 394 | Install path; PRD-14 drops `goalcup-daemon` from it |
| `docs/plans/**`, `docs/archive/**` | | History; the guard skips them |

## Files it owns

Delete (PRD-13 runs the `git rm`):

- `crates/goalcup-cli/`, `crates/goalcup-daemon/`, `crates/goalcup-platform/`, `crates/goalcup-protocol/`,
  `crates/goalcup-verifier/` (including `crates/goalcup-protocol/providers.toml` and `pins/`).
- `spikes/` (whole).
- The scripts in group D below, `harnesses/install.sh`, `.claude/agents/campaign-cell.md`.
- `connectors/{amp-cli,claude-code-cli,codex-cli,hermes-cli}/harness.toml` (the delete is listed by
  PRD-11; PRD-13 runs it in the cut commit).

Create: nothing. Change: nothing, except the workspace members line if PRD-16 grants it (Ask 1).

Must not touch: `crates/goalcup/`, `crates/goalcup-format/`, `crates/goalcup-verify/` (the new code),
`server/worker/**` (PRD-07 cuts its own parts), `docs/**`, `README.md`, `AGENTS.md` (PRD-15),
`scripts/gate.sh`, `.github/**`, `packaging/**`, `.config/**` (PRD-14), `connectors/<id>.toml` and
`connectors/model-hosts.toml` (PRD-11), `skills/**` (PRD-10, PRD-12), `VISION.md` (the owner's).

## Interfaces with other PRDs

| PRD | Gives to PRD-13 | Gets from PRD-13 |
|---|---|---|
| PRD-02, 03, 04, 05, 06, 08, 09, 10, 11 | "copied, tests green" line per ledger row (section 4) | The delete of the source |
| PRD-17 | `LESSONS.md` merged and `tests/lessons_*.rs` green | The old code stays readable at `v0.7.0` |
| PRD-16 | Phase 2 gate green; the members edit (Ask 1); the merge of the cut with PRD-14's and PRD-15's edits | One branch, five commits |
| PRD-14 | In the same merge: `gate.sh` (drop `check-version.py` and `check-links.py` steps, the `-p goalcup-cli --test full` steps at `scripts/gate.sh:88-103`, the daemon comment at `:63`); `.config/nextest.toml` (61 lines, filters name `goalcup-daemon` and `goalcup-cli` at `:16-17, 37, 46, 56`); `.github/CODEOWNERS` (paths under deleted folders, checked by `scripts/test_codeowners.py`); `ci.yml:68, 80`; `linux-x86_64.yml:43, 63`; `packaging/install.sh:9, 226, 312-359` and `scripts/package-release.sh:4, 10` (no `goalcup-daemon`); the reference guard as a gate fast step | The list of deleted scripts and binaries |
| PRD-15 | In the same merge: the `AGENTS.md` map (`AGENTS.md:32-50`), the pin rule (`AGENTS.md:90-91`), the D65 rule (`AGENTS.md:83-86`), `scripts/README.md`, `docs/TERMINATION.md` (262), `docs/research/t25-disk-full.md`, `docs/research/m2-report-format.md`, `docs/research/LOCAL-CAMPAIGN.md`, `docs/research/replay-comparison.md`, `docs/CONTRACTS.md` v5 freeze (`:510-560`), `docs/CONNECTOR-SCHEMA.md`, `docs/RULES.md:306-321` (run labels) and `docs/CHEAT-CHECKLIST.md` bounds | Which docs describe deleted code |
| PRD-07 | Its own Worker cuts (group F) | The protocol crate's signing vectors reader is gone (`server/worker/test/vectors/signing.json:2`), so `sign.ts` and the vectors go together |
| PRD-12 | `skills/adding-a-capture-harness/writing-a-decoder.md` and `templates/*` stop citing `crates/goalcup-protocol` (`templates/connector-harness.toml:86`, `templates/harness-facts.md:56`) | The pad replaces the five kits in section 3 |
| PRD-10 | Session sniffing on `continue` is kept or dropped there (review R, K7); the answer to pasted `goalcup mcp` lines (H4) | `commands/mcp.rs`, `login.rs`, `ssh_keys.rs`, `account/github.rs` are deleted here |
| PRD-08 | `goalcup-verify check --json` reports raw bytes, compressed bytes and framing share (PRD-01 acceptance 4) | That replaces `scripts/research-measure.py` |

## What is cut

Lines are `wc -l`. "src / inline" splits each file as in "Today". "Owner" is the PRD that runs the
delete. "Replaced by" names the new path or "nothing".

### Group A: the separate daemon and the hand-off (K13, owner decision "option B")

| Path | src / inline | Tests that go | Replaced by | Owner |
|---|---|---|---|---|
| `crates/goalcup-daemon/src/{bin/,control/,ingest.rs,registry.rs,runtime/daemon.rs,runtime/mod.rs,status.rs,config.rs,fsutil.rs,lib.rs}` | 5,816 / 101 | `tests/it/control_api.rs` (1,735), `capture_ingest.rs` (3,095), `tests/health_status.rs` (151), `dir_modes.rs` (338) | Nothing: one process (PRD-02). Private files: a few lines in `src/pipeline/status.rs` | PRD-13 |
| `crates/goalcup-daemon/src/runtime/capture/` (owner, sink, writer, finish, mod, harness, native plan) | 3,915 / 2,477 (`tests.rs` 2,340) | `capture_lifecycle.rs` (3,529), `full/capture_finish_budget.rs` (1,267) | `src/pipeline/` (PRD-02), `PartWriter` (PRD-01) | PRD-13 |
| `crates/goalcup-cli/src/{daemon/,retry.rs,arming.rs,released_child.rs,subreaper.rs}` | 3,269 / 1,059 (`daemon/mock.rs` 636) | in `tests/it/start_launch.rs`, `m2_ipc_compose.rs` (1,606) | Nothing | PRD-13 |
| `crates/goalcup-cli/src/proxy/{ingest.rs,api.rs}` (IPC sender, failure budget at `ingest.rs:104`, `latch_terminal` at `:455`, count at selection `api.rs:397`) | 1,960 / 41 | `m2_capture_sender.rs` (1,991), `m2_emitter.rs` (1,517), `restarts.rs` (497) | `pipeline::Recorder` (PRD-02, seam S1) | PRD-13 |
| `crates/goalcup-protocol/src/control.rs` (the control API contract) | 1,010 / 30 | `control_shapes.rs` (689) | Nothing | PRD-13 |
| `crates/goalcup-cli/src/launch.rs` (spawn gate, parked child, D40) | 1,994 / 96 | `harness_launch_plan.rs` (782) | Watchdog spawn, about 60 lines (PRD-02); env build (PRD-11) | PRD-13 |

Run 1's cause chain lives in these files: the 8 MiB queue (`crates/goalcup-protocol/src/capture.rs:51`),
the drop of a whole producer, ten short epochs (`crates/goalcup-cli/src/proxy/ingest.rs:104`) and the
permanent latch (`:455`).

### Group B: our own caps (owner: no caps of our own)

Every cap goes with its crate. Listed so the no-return guard knows them.

| Cap | Where | Replaced by |
|---|---|---|
| 8 MiB shared capture queue | `crates/goalcup-protocol/src/capture.rs:51` | Per-stream queues with stated memory guards that write a GAP (PRD-02) |
| 128 open streams, 128 connections, 128 live producers | `capture.rs:58-66` | None |
| 32 native streams (`MAX_OPEN_STREAMS / 4`), 1,024 files | `crates/goalcup-daemon/src/native_capture.rs:279-280` | None (PRD-04) |
| 1,024 connection tasks then 503; 5 recorder restarts then an unserved port | `crates/goalcup-cli/src/proxy/mod.rs:112, 1022` | Watchdog restarts with no cap (PRD-02) |
| 10,000 tunnel rows | `crates/goalcup-cli/src/proxy/tunnels.rs:20` | Tunnels are `conn` records (PRD-01) |
| Handle budget of 32 | `crates/goalcup-platform/src/handle_budget.rs:13` | None |
| Verifier exit 2 past 128 streams | `crates/goalcup-verifier/src/capture_analysis/reader.rs:78, 1384` | PRD-08 has no cap |
| Per-run size limit, 64 GiB | `server/worker/wrangler.toml:59, 151`, `server/worker/src/context.ts:12, 61-67` | None (PRD-07, owner answer 2) |

### Group C: the rest of the old crates, by part

| Path | src / inline | Biggest tests that go | Replaced by | Owner of the copy |
|---|---|---|---|---|
| `crates/goalcup-cli/src/proxy/` other files (`mod`, `connect`, `tls`, `http`, `websocket`, `headers`, `metadata`, `outage`, `preflight`, `tunnels`) and `ca_bundle.rs` | 7,459 / 1,308 | `m2_proxy_integration.rs` (3,025), `m2_http_relay.rs` (2,249), `m2_websocket_relay.rs` (2,074), `m2_tls_connect.rs` (1,275) | `crates/goalcup/src/proxy/` (about 4,400) | PRD-03 |
| `crates/goalcup-cli/src/upload/`, `commands/upload.rs`, `vitals.rs` | 4,457 / 921 | `upload_chunks.rs` (1,736), `upload_open.rs` (1,548), `full/upload_e2e.rs` (2,518), Rust Worker stand-in `tests/common/upload_server/` (2,514), `upload_server*.rs` (2,909) | `crates/goalcup/src/upload/` (about 940) | PRD-06 |
| `crates/goalcup-cli/src/run/` (watch, pause, reserve, time left) | 2,893 / 2,085 | `full/t25_disk_full.rs` (1,173) | Time left in `src/cli/` (PRD-10); disk code: nothing | PRD-10 |
| `crates/goalcup-cli/src/commands/` (all but `upload.rs`) | 8,739 / 1,129 | `m2_cli_lifecycle.rs` (1,944), `status_views.rs` (1,869), `prep.rs` (1,600), `start_launch.rs` (1,481) | `crates/goalcup/src/cli/` and `main.rs` (about 2,960 with the next three rows) | PRD-10 |
| `crates/goalcup-cli/src/{participant/,participant.rs,push_lines.rs,host_log.rs,skill.rs,entries.rs,prep_file.rs,ssh_keys.rs}` | 2,767 / 888 | `account.rs` (1,195) | Trimmed text in `src/cli/` | PRD-10 |
| `crates/goalcup-cli/src/account/` (device flow, credential, `github.rs` 466) | 1,028 / 18 | `login.rs` (159) | Device flow only (PRD-10); `git-credential` and old token: nothing (K8) | PRD-10 |
| `crates/goalcup-cli/src/{config.rs,server.rs,main.rs,lib.rs,folder_gate.rs}` | 1,959 / 279 | `env_resolution.rs` (217), `contract_gate.rs` (782) | `src/main.rs`, `src/cli/` | PRD-10 |
| `crates/goalcup-cli/src/{model_host.rs,connector.rs,home.rs,per_run_home.rs,harness_version.rs,preflight/}` | 2,742 / 929 | `model_host_from.rs` (793), `connector_contract.rs` (994), `harness_contract_golden.rs` (975), `harness_contract_kit.rs` (805), `per_run_home.rs` (438), `shipped_connectors.rs` (411) | `crates/goalcup/src/connector/` (about 750) | PRD-11 |
| `crates/goalcup-daemon/src/redact.rs`, `redact/` | 4,152 / 1,135 | `capture_redaction.rs` (4,745), `redaction.rs` (2,629), `capture_redaction_extra.rs` (1,113) | `crates/goalcup/src/mask/` (about 3,550) | PRD-05 |
| `crates/goalcup-daemon/src/native_capture.rs` | 1,632 / 0 | `capture_native.rs` (2,664) | `crates/goalcup/src/native/` (about 870) | PRD-04 |
| `crates/goalcup-daemon/src/terminate/`, `runtime/termination.rs` (termination proof) | 2,428 / 355 | `terminate_sim.rs` (2,714), `full/process_kill.rs` (1,285), 12 shell fixtures (213) | Nothing. The seal records how the harness exited (PRD-01 section 11) | none |
| `crates/goalcup-daemon/src/workspace.rs`, `workspace_git.rs` (seal-time walk, K21) | 1,015 / 19 | `workspace_layout.rs` (643) | Two `git` calls in the seal (PRD-02 section 11) | PRD-02 |
| `crates/goalcup-platform/src/tracecap*`, `sourcecap*`, `handle_budget.rs` (K9, K15) | 3,197 / 320 | `unix_tracecap.rs` (325), `unix_sourcecap.rs` (261) | No-follow open chain, about 120 lines (PRD-04) | PRD-04 |
| `crates/goalcup-platform/src/disk.rs` (K5) | 239 / 0 | `disk.rs` (137) | Nothing | none |
| `crates/goalcup-platform/src/{awake,terminal,machine}.rs`, rest | 1,603 / 536 | `machine.rs` (376), `awake.rs` (256) | Sleep lock and browser opener (PRD-10), shield (PRD-02) | PRD-10, PRD-02 |
| `crates/goalcup-protocol/src/{envelope,chain,capture,capture_status,export,ack,receipt,seal,provenance}.rs` (v5 format, K14, K18) | 3,573 / 248 | `m2_contract.rs` (1,715), `export_roundtrip.rs` (1,477), `roundtrip.rs` (1,143), `capture_terminals.rs` (884), `capture_storage.rs` (829), `signing_vectors.rs` | `crates/goalcup-format/` (PRD-01, about 1,000) | PRD-01 |
| `crates/goalcup-protocol/src/run_secret.rs` | 376 / 154 | inline | `src/mask/values.rs` (about 330) | PRD-05 |
| `crates/goalcup-protocol/src/{harness.rs,harness/,capture_policy.rs,hints.rs,template.rs}` (schema, deny checks, trust tables, K16) | 4,655 / 106 | `harness_schema.rs` (4,451), `harness_loader.rs` (650), `harness_conformance.rs` (379) | `src/connector/schema.rs` (about 150) | PRD-11 |
| `crates/goalcup-protocol/src/{pins.rs,connector_dir.rs,manifest.rs,constant_time.rs}`, `pins/`, `providers.toml` (K17) | 811 / 432 | `connector_pins.rs` (303), `pins_build_fails.rs` (86), `connector_template.rs` (120) | `connector_sha256` in the start event (PRD-11) | PRD-11 |
| `crates/goalcup-protocol/src/process.rs` and rest | 674 / 314 | | Nothing | none |
| `crates/goalcup-verifier/src/capture_analysis/` decoders (K1) | 6,582 / 401 | `m2_anthropic_decoder.rs` (2,658), `m2_chat_decoder.rs` (2,524), `m2_openai_decoder.rs` (2,149), `m2_native_observations.rs` (1,737), `m2_decoder_v1.rs` (1,126) | Usage reader, about 1,470 (PRD-09) | PRD-09 |
| `crates/goalcup-verifier/src/capture_analysis/` reader, framework, report, stats; `bin/`, `lib.rs` | 5,571 / 934 | `m2_capture_integrity.rs` (3,737), `m2_research_report.rs` (2,456), `m2_run_stats.rs` (873), `v5_corpus.rs` (718, K26) | `crates/goalcup-verify/` (about 1,500, PRD-08) | PRD-08 |
| Test fixtures of the old crates | 7,000 lines in 165 non-`.rs` files | `harness-golden` (2,110, K4), `v5-corpus` (912), `m2-*`, `frozen-connectors`, `schema-1-connectors` | Copied only where a ledger row says so | PRD-05, PRD-09 |
| Test seams: features `test-hooks` (`crates/goalcup-daemon/Cargo.toml:20-21`, `crates/goalcup-protocol/Cargo.toml:14-15`), `test-support` (`crates/goalcup-cli/Cargo.toml:25-26`), `pause_at` hooks (for example `commands/start/phases.rs:286, 830, 856, 1009`) | in the above | | `#[cfg(test)]` only (guard, section 6) | none |

Group A plus C plus the fixtures is the whole of the five crates: 236,242 lines.

### Group D: scripts and other files (K2, K5, K17, K24, K25)

| Path | Lines | Test that goes | Replaced by |
|---|---:|---|---|
| `scripts/matrix.sh` | 2,522 | `test_matrix.py` (1,517) | Pad (PRD-12) |
| `scripts/linux-matrix.sh`, `kit_settle.py` | 876, 302 | `test_linux_matrix.py` (1,219) | Pad in Vercel Sandboxes |
| `scripts/campaign.py`, `campaign.example.json` | 1,888, 64 | `test_campaign.py` (1,335) | Pad |
| `scripts/linux-harness-smoke.sh` | 242 | none (run by `linux-x86_64.yml:63`) | Pad |
| `scripts/check-connector.sh`, `pin-manifests.sh` | 314, 274 | `test_check_connector.py` (411) | Pad; pins cut |
| `scripts/research-measure.py` | 541 | `test_research_measure.py` (365) | `goalcup-verify check --json` size numbers (PRD-08) |
| `scripts/research-replay.py` | 342 | `test_research_replay.py` (325) | `goalcup-verify check` on two entries |
| `scripts/research-sample.py` | 424 | `test_research_sample.py` (247) | `goalcup status --json` (PRD-10) |
| `scripts/research-staging.py`, `research-staging.sql` | 604, 166 | `test_research_staging.py` (426) | Nothing; read-only database access overnight |
| `scripts/research-watch.py` | 95 | `test_research_watch.py` (104) | `goalcup seal` |
| `scripts/t25-disk-full.sh` | 93 | `crates/goalcup-cli/tests/full/t25_disk_full.rs` (1,173, with the crate) | Nothing (disk full: no special handling) |
| `scripts/check-version.py` | 206 | `test_check_version.py` (128) | Nothing; the version is written once in `Cargo.toml` |
| `scripts/check-links.py` | 1,349 | `test_check_links.py` (1,672) | Nothing. Deleted last, after PRD-15 runs it once on the rewritten docs |
| `scripts/linux-check.sh` | 106 | `test_linux_check.py` (107) | Default cut: it runs old test names in Docker (`linux-check.sh:95-104`); the build is tested natively on Linux sandboxes. PRD-14 decides (Ask 2) |
| `scripts/test_early_crosscheck.py` | 0 | 538 | Goes with `spikes/m2-first-release/e02` |
| `harnesses/install.sh` | 62 | none | Pad installs each harness (PRD-12) |
| `.claude/agents/campaign-cell.md` | 31 | none | Goes with `campaign.py` |
| **Total** | **10,501** | **8,394** | |

Kept scripts: `gate.sh`, `release.sh`, `package-release.sh`, `smoke-release.sh`,
`third-party-licenses.py`, `ci-scope.py`, `fetch-run.sh` (998 lines) and their tests plus
`test_install.py` and `test_codeowners.py` (1,999 lines).

### Group E: spikes (K6, K25)

| Path | Files | Lines | Fate |
|---|---:|---:|---|
| `spikes/openclaw` | 31 | 7,887 | Deleted; 86 corpus files from all folders but `cursor-cli` and `claude-desktop` copied first (PRD-05) |
| `spikes/claude-desktop` | 17 | 4,391 | Deleted, not copied |
| `spikes/claude-code-cli` | 23 | 2,959 | Corpus copied, then deleted |
| `spikes/opencode`, `spikes/hermes`, `spikes/codex-cli`, `spikes/codex-app` | 83 | 4,882 | Corpus copied, then deleted |
| `spikes/cursor-cli` | 22 | 1,681 | Deleted, not copied (Cursor is out; the documented dodger case lives in `docs/harnesses/`, PRD-11) |
| `spikes/m2-first-release` | 7 | 1,564 | Deleted |
| `spikes/redact.py` (Python twin of the mask rules) | 1 | 349 | Deleted, not copied (PRD-05) |
| **Total** | **184** | **23,713** (8.4 MB) | |

### Group F: Worker routes and server parts (owner PRD-07)

Listed so the cut list is whole. PRD-07 runs these deletes and owns the numbers.

| Path | src | Tests that go | Replaced by |
|---|---:|---|---|
| Route `POST /v1/runs/{id}/receipt` (`server/worker/src/index.ts:43-44`, `runs.ts:352-389`) | about 75 | receipt cases (about 150, est.) | Nothing; signing is cut |
| Route `POST /v1/runs/{id}/abandon` (`index.ts:45-46`, `runs.ts:393-414`) | about 60 | abandon cases (about 120, est.) | Nothing |
| `src/sign.ts` and signer plumbing | about 135 | `sign.test.ts` (68), `test/vectors/signing.json` (26) | Nothing; `RECEIPT_SIGNING_KEY` goes |
| Seal documents and the tunnel list (`runs.ts:535-637`) | about 140, about 15 back | tunnel and seal-document cases (about 250, est.) | The seal and tunnels are records (PRD-01 section 11) |
| Per-run size limit (`chunks.ts:74, 123-144`, `context.ts:11-12, 26-27, 61-67`) | about 50 | `quota.test.ts` (114) | Nothing |
| Vitals validator and the disk-pause rule (`runs.ts:499-533`, `db-local.ts:612-622`) | about 45 | vitals cases in `notices.test.ts` | Nothing (disk full: no special handling) |
| `tracer_version` alias (`util.ts:79-91`) | about 10 | alias cases | Nothing |
| Judged commit by cron (`github.ts:173` onward, `notices.ts:68-80`) and start-time repo checks (`runs.ts:314-351`) | about 260 | `github.test.ts` (184) | **Open**: PRD-00 question 4 cuts judged-commit logic; PRD-07 keeps it (Ask 5) |

### Group G: connector fields (owner PRD-11)

`connectors/*/harness.toml` (556 lines) are replaced by `connectors/<id>.toml` and `model-hosts.toml`
(about 390 lines). Fields with no successor (PRD-11 "What is cut"): `[participant.time_left_tool]`
(`amp-cli:17`, `claude-code-cli:15`, `codex-cli:13`, `hermes-cli:21`), `[[participant.no_prompts]]`,
`[capture.identity]`, `[capture.termination]`, `[capture.argv_guard]` (`hermes-cli:124`),
`[launch_prep]` (`hermes-cli:145`), `[[capture.hosts]]` (now the shared list), `[capture.platform.*]`,
CA `mode`, `protocol_hint`, `since_run_start`, the email rule and the Codex rate-limit, plan and credit
mask rows (`codex-cli:110-113`).

### The plan's K list, mapped to 1.0

| K | What | Plan timing | In 1.0 | Group |
|---|---|---|---|---|
| K1 | Verifier research decoders | Before Friday, then moved | Cut; model and usage parts copied (PRD-09) | C |
| K2 | Scripts with no point, `check-version.py` | After | Cut | D |
| K3 | Rust Worker stand-in | After | Cut with `goalcup-cli`; one server (PRD-07) | C |
| K4 | Connector byte goldens | Before | Cut | C |
| K5 | Disk-full handling, `SIGSTOP` pause | Switch before, code after | Cut, client and Worker | C, D, F |
| K6 | Spike prototypes and data | Before | All of `spikes/` cut | E |
| K7 | Provider probe, start leftovers | After | Cut; session sniffing is PRD-10's call | C |
| K8 | `goalcup mcp` lines, `git-credential` | Part before | Cut whole | C, G |
| K9 | Platform dead code, `handle_budget.rs` | After | Cut with the crate | C |
| K10 | Uploader leftovers | After | Rewritten (PRD-06) | C |
| K11 | Proxy outage windows | After | Rewritten (PRD-03) | C |
| K12 | Daemon writer quota | Before | Cut with the daemon | A |
| K13 | Daemon merge | After | Done: option B | A |
| K14 | Back-compat, retired versions | After | Cut; no v5 reader | C |
| K15 | Platform rewrite | After | Crate cut; about 120 lines rewritten (PRD-04) | C |
| K16 | Deny checks, trust tables | After | Cut | C |
| K17 | Pins, `pin-manifests.sh`, `check-connector.sh` | After | Cut | C, D |
| K18 | Receipt, ack and status signing | After | Cut, client and Worker | C, F |
| K19 | CLI extras (`login`, SSH keys, `doctor`, `mcp`) | After | Cut; push lines trimmed (PRD-10) | C |
| K20 | Judged commit | After | Open (Ask 5) | F |
| K21 | Seal-time workspace walk | After | Cut; two `git` calls | C |
| K22 | Finish stage machine | After | Cut; PRD-02 seal | A |
| K23 | Rate limiter Durable Object | After | **Kept**: guards a real cost (section 7) | |
| K24 | Crash and failure campaign tests | Waited for N5 | Cut with the crates; the kill test (PRD-01 acceptance 3) replaces them | C |
| K25 | `check-links.py`, `spikes/redact.py` | After | Cut (links last) | D, E |
| K26 | v5 corpus test | Before | Cut | C |
| K27 | Leftovers (Worker disk vitals, owed acks, give-back, `m2_contract.rs`) | After | Cut; GCR1 golden replaces `m2_contract.rs` (PRD-01) | C, F |

Sources: plan revision 1 (`git show b19ebd44:docs/plans/2026-10-30-cut-first-plan.md`, lines 107-138),
revision 4 sections 6a and 6b (`docs/plans/2026-10-30-cut-first-plan.md:287-320`), and the review
corrections (`docs/plans/2026-10-30-audit/R-reviews.md:166-298`).

### Point check

| Cut | Point that could suffer | Why it does not |
|---|---|---|
| Separate daemon and hand-off | 1, 2, 3 | It is what failed in run 1 (no gap for 470 lost calls). One process writes a gap for every loss (PRD-02) |
| Termination proof | 2 (subagents left running) | The recorder does not end processes; it records what passes through the proxy and the session files until the seal. Subagent files are read the same way after the harness exits (PRD-04 final snapshots) |
| Disk handling | 3 | The `SIGSTOP` pause itself stopped the harness (`crates/goalcup-cli/src/run/pause.rs:5-10`). A write error is a `write_error` gap (PRD-01) |
| Signing | 5 | The hash chain stays (PRD-01 section 10); the Worker stores create-only with SHA-256 in the key |
| Research decoders | 1, 2 at judging | The usage reader (PRD-09) and the judge view (PRD-08) give model, tokens and the per-subagent view |
| Pins, deny checks | 4, 7 | Review is the control for connectors (`CODEOWNERS` comment); the start event carries `connector_sha256` |
| Provider probe | 6 | A run with no model requests says so plainly (PRD-10) |
| Scripts and spikes | none | Organizer and research tools; the pad replaces the test kits |

## Target size after 1.0

New-code numbers are the owning PRDs' estimates; "est." marks this PRD's own guess where the owner
gave none.

| Area | Today src | Today tests | 1.0 src | 1.0 tests |
|---|---:|---:|---:|---:|
| `crates/goalcup-format` (PRD-01) | | | 1,000 | 800 est. |
| `crates/goalcup` pipeline and watchdog (PRD-02) | | | 1,300 | 1,500 |
| proxy (PRD-03) | | | 4,400 | 2,500 est. |
| native (PRD-04) | | | 870 | 800 est. |
| mask (PRD-05) | | | 3,550 | 5,000 |
| upload (PRD-06) | | | 940 | 800 est. |
| CLI (PRD-10) | | | 2,960 | 1,200 |
| connector code (PRD-11) | | | 750 | 600 est. |
| `crates/goalcup-verify` check (PRD-08) | | | 1,500 | 1,200 |
| usage reader (PRD-09) | | | 1,470 | 600 |
| lessons (PRD-17) | | | 0 | 1,500 est. |
| **Rust** | **86,516** | **142,413** (+7,000 fixture lines) | **about 18,700** | **about 16,500** |
| Worker (PRD-07) | 4,001 | 5,775 | about 3,450 | about 4,950 |
| Scripts | 11,406 | 10,393 | 998 plus the pad (PRD-12, not yet sized) | 1,999 plus the pad's |
| Packaging, `harnesses/` | 456 | | 394 | |
| **Code that ships or runs** | **about 102,400** | **about 158,600** | **about 23,500 plus the pad** (minus 77%) | **about 23,400 plus the pad** (minus 85%) |
| Data: connectors | 556 | | about 390 | |
| Data: spikes | 23,713 | | 0 (86 corpus files, 6.2 MB, move under `crates/goalcup/tests/mask_corpus/`) | |

The Rust estimate (about 18,700) is above PRD-00's budget of 12,000 to 16,000 (Ask 4). The binaries
drop from three to two; `Cargo.lock` should lose at least `ed25519-dalek`, `redb` and `axum` and what
only they pull in (today 218 packages; unverified until built).

How to recount after the cut (the watcher posts the output in the night log):

```
find crates -path '*/src/*' -name '*.rs' | xargs wc -l | tail -1        # src with inline tests
find crates -path '*/tests/*' -name '*.rs' | xargs wc -l | tail -1      # test files
wc -l server/worker/src/*.ts | tail -1; wc -l server/worker/test/*.ts server/worker/test/helpers/* | tail -1
git ls-files scripts | xargs wc -l | tail -1
```

## Acceptance

Real-run proofs first. Each runs on the post-cut commit, built in a fresh Vercel Sandbox from a fresh
clone of `v1.0/integration`.

1. **The core set still passes after the cut.** The pad (PRD-12) runs Claude Code, Codex and Hermes,
   direct-key case, on binaries built from the post-cut commit. Pass: the same pass or fail row per
   harness as the last pre-cut pad run; any new failure blocks the merge of the cut.
2. **One process, two binaries.** During the Codex run, `ps -eo pid,comm` shows the `goalcup` watchdog
   and recorder and no `goalcup-daemon`; `ls target/release` after `cargo build --release --locked`
   shows `goalcup` and `goalcup-verify` and no third binary. Pass: both true.
3. **Kill test still holds.** PRD-01 acceptance 3 re-run once on the post-cut build. Pass: as there.
4. **Server copy whole.** `scripts/fetch-run.sh` plus `goalcup-verify check --server` on the Codex run
   (PRD-07, PRD-08). Pass: equal SHA-256 to the local parts.
5. **Release tarball.** `scripts/package-release.sh` and `smoke-release.sh` (PRD-14) on the post-cut
   commit. Pass: the tarball holds two binaries and the smoke passes.

Automated checks that guard them:

- `cargo build --offline --workspace` and `sh scripts/gate.sh full` green with the old crates gone
  (PRD-00 phase 3 gate).
- The reference guard (section 6) prints nothing; PRD-14 adds it to `gate fast`.
- The no-return guard (section 6) prints nothing under `crates/` and `server/`.
- `python3 -m unittest discover -s scripts` green with no test for a deleted script left behind.
- The size recount is posted, and each area is within 30% of its row in "Target size", or the owner
  of that area names the reason in the night log.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| A lesson is lost | An edge case the old code handled is not in `LESSONS.md` and the fresh core misses it | Delete only after PRD-17's tests pass; the old code stays at `v0.7.0` for any later look |
| Binary name collision before the cut | From phase 0 to phase 3 the new `goalcup` and `goalcup-verify` share names with the old binaries (`crates/goalcup-cli/Cargo.toml:129`, `crates/goalcup-verifier/Cargo.toml:44`). Cargo warns and the last build wins, so the phase 1a early real test may run the old binary and "pass" | Ask 3 |
| Dangling references break the gate | `scripts/test_codeowners.py` fails on any `CODEOWNERS` path that no longer exists; `.config/nextest.toml` names deleted packages (whether nextest errors or warns on an unmatched filter is unverified); `gate.sh:88-103` runs `-p goalcup-cli` | One merge with PRD-14's edits (section 5) |
| Dry-run participants on 0.7 | Pasted `goalcup mcp` lines now start a missing command; an old `goalcup-daemon` may still run; an old binary stays in `~/.local/bin` | PRD-10 says so in the participant text (H4); PRD-14's installer removes `goalcup-daemon` from the install folder. Whether an old daemon holds a port 1.0 needs is unverified |
| Judging loses run labels | `docs/RULES.md:306-321` and `docs/CHEAT-CHECKLIST.md` cite labels that only the cut verifier computed | PRD-08's judge view replaces them; PRD-15 rewrites the rules text |
| Docs links break | `check-links.py` goes in the same build in which PRD-15 rewrites many docs | PRD-15 runs it once on the final docs; it is deleted last |
| Linux filesystem coverage | `linux-check.sh` ran the gate on ext4 and XFS in Docker; that coverage goes | The pad runs on Linux sandboxes; macOS is the owner's run after the build (PRD-00) |
| Website database keeps dead columns | Receipt, ack and disk-pause columns stay unused; no migration overnight | Owner question 1 in PRD-00 |
| Size overrun | The new code is larger than budget | Recount at the cut; each area over its estimate by 30% names why |
| A guard pattern hits a fair use | For example a variable called `receipt` in an unrelated sense | The integrator narrows the pattern in the night log; the guard never blocks a build on its own |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| `check-links.py` | Cut, after PRD-15's last run | Owner, or PRD-15 if it needs it for the docs rewrite |
| `check-version.py` | Cut | PRD-14 |
| `linux-check.sh` and `linux-x86_64.yml` | Cut | PRD-14 |
| Device-flow rate limiter | Kept (real cost) | Owner |
| Judged commit in the Worker | Cut, following PRD-00 question 4 | Owner (Ask 5) |
| Session sniffing on `continue` | PRD-10's call | PRD-10 |
| Where the old code lives | Tag `v0.7.0` and main `4b226de2`; no new tag | PRD-16 may add `v0.7-final` |
| Size tolerance | 30% over the owner's estimate, with a reason | PRD-16 |

## Builder notes

Order of work:

1. **Phase 0 (H+0), one small commit:** settle the binary collision (Ask 3) before any early real test.
2. **Phases 1 and 2:** read only. Keep the copy ledger in the night log. Draft the five commits on
   `v1.0/prd-13-cuts` and rebase them each hour, so the cut is ready the minute phase 2 is green.
3. **Phase 3:** check every ledger row says "copied, tests green". Check PRD-17's lessons tests are
   green on integration. Apply the five commits. Run the build, the two guards, `gate full`, then hand
   the branch to PRD-16 for the joint merge with PRD-14's and PRD-15's edits.
4. **After merge:** a watcher runs acceptance 1 to 5 and posts the size recount.

Traps from the audit:

- `cargo build --locked` fails once the members change. Update `Cargo.lock` with one unlocked offline
  build, commit it, then go back to `--locked`.
- Do not delete `scripts/fetch-run.sh`, `third-party-licenses.py`, `server/worker/sql/` or
  `server/worker/src/db-local.ts`: they look old but serve point 5, the release and the pad.
- `scripts/test_early_crosscheck.py` reads `spikes/m2-first-release/e02/`; delete both together.
- The mask corpus test floor: PRD-05's copied corpus must hold at least the files its tests count
  (the old floor was 50 files, review R item 7); copy before `git rm spikes/`.
- `skills/adding-a-capture-harness/templates/connector-harness.toml:86` and `.github/PULL_REQUEST_TEMPLATE.md:32`
  cite deleted paths; the reference guard catches both.
- Cut whole folders with `git rm -r`; do not leave empty `tests/fixtures` folders that a glob in a new
  test might read.
- Never run the old crates' test suites to "check" before deleting them: the machine is shared, and
  the pre-cut gate already ran them in phase 2.

How a watcher proves it in a real run: in a fresh sandbox, clone `v1.0/integration` at the merge
commit, build release, run the pad's Codex direct-key case with the payments prompt and many
subagents, watch `ps` for a `goalcup-daemon` (there must be none), run `goalcup-verify check` on the
entry and on the fetched server copy, and post the pad row, the `ps` line, the two guard outputs and
the size recount in the night log.

## Asks of PRD-00 and PRD-01

1. **Workspace members (PRD-00 ownership).** `Cargo.toml` is PRD-16's file, but the cut cannot build
   without removing five members (`Cargo.toml:3-9`) and dead workspace dependencies. Either let PRD-13
   make that one edit in its branch with PRD-16's approval, or have PRD-16 make it in the same merge.
2. **Unowned paths (PRD-00 ownership table).** These paths have no owner but must change or go at the
   cut: `.github/CODEOWNERS`, `.github/PULL_REQUEST_TEMPLATE.md`, `.github/ISSUE_TEMPLATE/`,
   `.config/nextest.toml`, `scripts/README.md`, `scripts/linux-check.sh`, `scripts/test_linux_check.py`,
   `scripts/linux-harness-smoke.sh`, `scripts/third-party-licenses.py`, `scripts/test_third_party_licenses.py`,
   `.claude/agents/campaign-cell.md`, `harnesses/install.sh`. Proposed: all of `.github/**`,
   `.config/**`, `linux-check.sh` and `third-party-licenses.py` (with tests) to PRD-14;
   `scripts/README.md` to PRD-15; `linux-harness-smoke.sh`, `campaign-cell.md` and
   `harnesses/install.sh` to PRD-13 (delete).
3. **Binary collision while old and new coexist (PRD-00 phase 0).** Phase 0 adds the new crates beside
   the old ones, so two packages build a `goalcup` binary and two build `goalcup-verify`. Add a phase 0
   step: PRD-13 renames the old binaries to `goalcup-v07` and `goalcup-verify-v07` (two `[[bin]]`
   names plus 35 `CARGO_BIN_EXE_` uses in 24 old test files, mechanical) so `target/*/goalcup` is
   always the new one. Alternative: the watchers build with `-p goalcup` into a separate
   `CARGO_TARGET_DIR`. Without one of these, phase 1a's early real test is not trustworthy.
4. **Size budget (PRD-00 "Size budget").** The owning PRDs' own estimates add up to about 18,700 Rust
   src lines (proxy 4,400 after PRD-03 ask 6, masking 3,550, CLI 2,960, verifier 1,500, usage 1,470),
   above the 12,000 to 16,000 in PRD-00. Restate the budget, or name which area gives way.
5. **Judged commit (PRD-00 question 4 against PRD-07).** PRD-00's default cuts "judged-commit logic";
   PRD-07 keeps the Worker cron (`server/worker/src/github.ts:173`, `notices.ts:68-80`) and the start-time
   repository checks (`runs.ts:314-351`) as out of scope. Say which wins; this PRD lists the cut as open.
6. **Phase 3 gate wording (PRD-00 "Build order").** "No reference to an old crate" should mean the
   reference guard in section 6, with `docs/plans/**`, `docs/archive/**`, `docs/DECISIONS.md` and
   `v0.7.0:` provenance comments excluded. Otherwise every copied part's origin comment fails it.
7. **PRD-01 "What is cut".** It puts the ten v5 files at "about 4,900 src lines". `wc -l` gives 4,351
   lines for those files (3,949 src and 402 inline tests). It also lists `run_secret.rs` as cut, while
   PRD-05 copies about 330 lines of it into `src/mask/values.rs`. Say "copied in part (PRD-05)" there.
