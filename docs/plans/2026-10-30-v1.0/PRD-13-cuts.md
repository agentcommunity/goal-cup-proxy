# PRD-13: Cuts

Id: PRD-13. Version: goalcup 1.0. Status: plan for the build, phase 3 (PRD-00 "Build order").
Date: 30 October 2026. Revision 2: follows PRD-00 revision 2 and answers the consistency check and
reviews A and B (section "Review answers" at the end). Base: main at 4b226de2 (working tree at
7fd36019, docs only since).
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
- No backward compatibility before Edition 01. No v5 reader is kept, and the run 1 converter is cut
  (PRD-00 "Not in this build").
- Disk full: no special handling (owner, phase 2 decisions).
- Rulings of PRD-00 revision 2 that bound this PRD: history docs are kept in this build; judged-commit
  logic in the Worker is kept and untouched (PRD-00 question 3); old crates leave the workspace in
  phase 0 by PRD-16's `exclude` edit; the phase 3 merge holds only the deletes and the matching
  workspace edit.

## Today

Counted with `wc -l` on the working tree. "src" is production code: each file minus its trailing
`#[cfg(test)] mod` block (brace matched), with `src/daemon/mock.rs`, `src/participant/tests.rs` and
`src/runtime/capture/tests.rs` counted as tests. This is the method of appendix A (A.0); totals differ
from A.1 by under 2% because A split a few files by hand. PRD-14 counts the same tests a little
differently (126,112 in `tests/`, 6,939 fixture lines); the gap is under 0.1%.

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
owns, after the other owners have already landed the edits that stop pointing at those paths, and
proves the tree builds, gates and records with nothing old left.

### 2. Rules for every cut

1. **Delete whole, never hollow out.** An old file is either copied into a new file by its receiving PRD
   (and then deleted here) or deleted. No old crate stays as a dependency, a feature or a shim.
2. **Copy first, delete second.** A path is deleted only when every receiving PRD in the copy ledger
   (section 4) has merged its copy and its tests pass on integration, PRD-17's deletion gate passes
   (its four checks, PRD-00 phase 2 exit), and, for the five test kits, PRD-12's `selftest` passes.
3. **The old code is never lost.** It stays at main `4b226de2` and tag `v0.7.0`. A copied part names
   its origin in one comment of the form `copied from v0.7.0:crates/goalcup-cli/src/proxy/http.rs:18-26`.
   That form is the only allowed mention of an old crate path in code (section 6 guards).
4. **A cut that would block or slow the harness is not a cut.** Each row says which point could
   suffer and why it does not (table "Point check").
5. **No cap of our own comes back.** The no-return guard in section 6 checks that the cut concepts
   are not rebuilt in the new crates, from phase 1 on.
6. **PRD-13 edits no file it does not delete.** `scripts/gate.sh`, `Cargo.toml` and `Cargo.lock`
   belong to PRD-14 and PRD-16 (PRD-00 ownership table).

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
| Five test harness kits: matrix, linux matrix, campaign, harness smoke, connector check | `scripts/matrix.sh`, `linux-matrix.sh`, `campaign.py`, `linux-harness-smoke.sh`, `check-connector.sh` (5,842 lines plus `kit_settle.py` 302; 4,482 test lines) | One test pad, also the contributor path (PRD-12). `kit_settle.py`'s first-run answers move to PRD-11's `no_prompts` data |

### 4. The copy ledger (preconditions for each delete)

PRD-13 deletes a source only after the receiving PRD has written "copied, tests green" for it in the
night log (PRD-16's `NIGHT-LOG.md`, or the receiving lane's own log file if PRD-16 splits the log per
lane). Ranges are the owners' own citations.

| Source (old) | Copied into | By | What |
|---|---|---|---|
| `crates/goalcup-cli/src/proxy/{connect,tls,http,websocket}.rs`, `ca_bundle.rs` | `crates/goalcup/src/proxy/` | PRD-03 | About 2,250 lines: head parse, authority, ClientHello, CA and leaf, HTTP/1.1 framing, decoders, WebSocket framer and inflate |
| Proxy test bodies (PRD-03 section 7, `PRD-03:478`) | `crates/goalcup/tests/proxy_fixtures/` | PRD-03 | Anthropic brotli SSE body and the other named fixtures |
| `crates/goalcup-daemon/src/redact.rs`, `redact/stream.rs`, `crates/goalcup-cli/src/proxy/headers.rs`, `crates/goalcup-protocol/src/run_secret.rs` | `crates/goalcup/src/mask/` | PRD-05 | Rules, hold algorithm, header and query rules, run secrets (about 3,050 lines) |
| `spikes/*/fixtures/*.jsonl` except `cursor-cli` and `claude-desktop` | `crates/goalcup/tests/mask_corpus/` | PRD-05 | 86 files, about 6.2 MB |
| `crates/goalcup-daemon/src/native_capture.rs`, `crates/goalcup-platform/src/sourcecap*.rs` | `crates/goalcup/src/native/` | PRD-04 | Discovery, generations, no-follow reads (about 120 lines of these) |
| `crates/goalcup-daemon/src/runtime/capture/native_plan.rs:96-151, 224-311` | `crates/goalcup/src/connector/` | PRD-11 | Root templates, home isolation |
| `crates/goalcup-platform/src/terminal.rs:27-292` (`Shield`, `Modes`, `wait_resuming` and the constants `Shield` needs) | `crates/goalcup/src/watchdog/shield.rs` | PRD-02 | Terminal shield |
| `crates/goalcup-cli/src/commands/start/proxy.rs:33-62` | `crates/goalcup/src/watchdog/` | PRD-02 | Open-file limit raise, with the macOS clamp |
| `crates/goalcup-cli/src/subreaper.rs:73-74` and the start-time filter idea (`:11-14`) | `crates/goalcup/src/watchdog/spawn.rs` | PRD-02 | About 20 lines: `PR_SET_CHILD_SUBREAPER` on Linux, so helpers that outlive the harness stay findable (review A finding 8) |
| `crates/goalcup-cli/src/upload/**` | `crates/goalcup/src/upload/` | PRD-06 | Rewritten, about 940 lines |
| `crates/goalcup-platform/src/awake.rs`, `machine.rs:515-534`, participant text, push lines, device login | `crates/goalcup/src/cli/` | PRD-10 | About 150 lines of sleep lock; trimmed text |
| `crates/goalcup-cli/src/model_host.rs:682` onward (YAML), `launch.rs` env build, `harness/schema.rs:905-1112` (mask grammar) | `crates/goalcup/src/connector/` | PRD-11 | About 370 lines |
| `crates/goalcup-verifier/src/capture_analysis/{openai,anthropic,chat,thread}.rs`, `native/*`, `stats.rs` (model, usage, error parts) and fixtures `m2-openai`, `m2-anthropic`, `m2-native`, `connectors` | `crates/goalcup-verify/src/usage/`, `tests/usage_fixtures/` | PRD-09 | Model and usage readers; fixtures as decoded bodies |
| `crates/goalcup-verifier/src/bin/goalcup-verify.rs:264-310` | `crates/goalcup-verify/` | PRD-08 | Output file creation, parse rule |
| `crates/goalcup-cli/tests/common/**` (`m2_common/mod.rs`, `real_home_guard.rs` and others) | `crates/goalcup/tests/common/` | PRD-14 | About 1,200 lines of test support (PRD-14 section 5) |
| `crates/goalcup-cli/tests/fixtures/m2-tls/*.hex` and other lesson inputs | `crates/goalcup/tests/lessons_fixtures/` | PRD-17 | Byte inputs of the lessons cases |
| Every old crate, test and `docs/DECISIONS.md` | `docs/plans/2026-10-30-v1.0/LESSONS.md`, `crates/goalcup/tests/lessons_*.rs` | PRD-17 | Edge cases as acceptance cases |

### 5. Before the cut, and the cut itself

**Before the cut (phases 0 to 2), the other owners land their edits as soon as each is green**, each
working with the old crates still on disk (PRD-00 phase 3 row, review B M5):

- PRD-16, phase 0: the five old crates move from `members` to `exclude` in `Cargo.toml`, so they are
  never built, linted or tested again, and `Cargo.lock` is regenerated without them. No rename and no
  `--exclude` line in `gate.sh` (PRD-00 ruling on consistency item 13).
- PRD-14: `scripts/gate.sh` (drop the `-p goalcup-cli --test full` steps at `:88-103` and the daemon
  comment at `:63`), `.config/nextest.toml` (filters naming `goalcup-daemon` and `goalcup-cli` at
  `:16-17, 37, 46, 56`), `.github/CODEOWNERS` (paths under folders this PRD deletes, checked by
  `scripts/test_codeowners.py`), `ci.yml:68, 80`, `release.yml`, `PULL_REQUEST_TEMPLATE.md:32`,
  `packaging/install.sh:9, 226, 312-359`, `packaging/README.md`, `scripts/package-release.sh:4, 10`,
  `scripts/test_install.py:28` (reads protocol crate fixtures), `third-party-licenses.py` and its test.
  PRD-14 also adds both guards of section 6 to `gate fast` in the form given there.
- PRD-15: `README.md`, `AGENTS.md` map and rules, `CONTRIBUTING.md`, `SECURITY.md`, `TODO.md`,
  `scripts/README.md`, and every live `docs/*.md` that names an old crate.
- PRD-11: `docs/harnesses/{claude-code-cli,codex-cli,hermes-cli}.md`.
- PRD-12: deletes `skills/adding-a-capture-harness/writing-a-decoder.md` and
  `templates/connector-harness.toml`, rewrites `templates/harness-facts.md:56`.
- PRD-07: `server/worker/src/sign.ts`, `test/sign.test.ts`, `test/vectors/signing.json` (they name the
  protocol crate's signing vectors) go with its signing cut.

During phases 1 and 2, PRD-13 runs the reference guard by hand each hour with the to-be-deleted paths
and `Cargo.toml` excluded, and posts any hit in the night log with the owner's PRD. So the cut finds no
surprise.

**The cut (phase 3).** Branch `v1.0/prd-13-cuts` from `v1.0/integration` after the phase 2 gate.
Commits, in this order, each building on its own:

1. `git rm -r` the five old crate folders.
2. `git rm -r spikes/` and `scripts/test_early_crosscheck.py` (it reads `spikes/m2-first-release/e02/`).
3. `git rm` the scripts and tests of group D, `harnesses/install.sh` and `.claude/agents/campaign-cell.md`.
4. `git rm connectors/*/harness.toml` once PRD-11's `connectors/<id>.toml` and `model-hosts.toml` are on
   integration (the delete is listed by PRD-11; PRD-13 runs it here).

The phase 3 merge holds these four commits plus two small commits by their owners: PRD-16's workspace
edit (drop the five `exclude` entries and every workspace dependency only the old crates used, checked
with `cargo tree`; expected `ed25519-dalek`, `redb`, `axum`), and PRD-14's delete of `check-links.py`
and its test after PRD-15's last run. Nothing else.

PRD-07's Worker cuts and PRD-15's doc rewrite do not wait for phase 3. They touch no old crate.

If PRD-17's deletion gate is not green by the time PRD-16 sets (H+9 today), the cut does not happen:
the old crates stay on disk, out of the workspace, and the end-of-build report says so (PRD-16
section 5). Nothing breaks, because nothing builds them.

### 6. The guards (kept in the gate by PRD-14)

**Reference guard.** It must print nothing. PRD-14 adds it to `gate fast` in phase 0 or 1 in a form
that skips, with the one line `reference guard: skipped, old crates on disk`, while
`crates/goalcup-cli/` exists. After the cut it runs with no edit to `gate.sh`.

```
git grep -nE 'goalcup[-_](cli|daemon|platform|protocol|verifier)\b|goalcup-daemon' -- . \
  ':!docs/plans/**' ':!docs/archive/**' ':!docs/research/**' ':!docs/DECISIONS.md' ':!CHANGELOG*' \
  | grep -v 'v0\.7\.0:'
```

The excluded paths are the history PRD-00 keeps in this build. `docs/research/**` includes run 1's
report once PRD-15 copies it to `docs/research/results/2026-10-30-two-hour-run.md` (consistency
item 27). A top-level `docs/*.md` that PRD-15 keeps as history but does not rewrite moves under
`docs/archive/`, or PRD-15 adds it here by name with a night-log line.

**No-return guard.** It runs from phase 1, because it only reads the new code. Every line it prints
must also survive `grep -v 'v0\.7\.0:'` (a provenance comment such as
`copied from v0.7.0:crates/goalcup-verifier/src/capture_analysis/openai.rs` is not a hit).

New Rust (`crates/goalcup/`, `crates/goalcup-format/`, `crates/goalcup-verify/`; after the cut,
all of `crates/`). Each pattern must print nothing:

| Concept cut | Pattern |
|---|---|
| Separate daemon and its IPC | `goalcup-daemon`, `IngestListener`, `DaemonClient`; `epoch` under `src/proxy/` only |
| Our own caps | `MAX_OPEN_STREAMS`, `MAX_OPEN_CONNECTIONS`, `MAX_LIVE_PRODUCERS`, `MAX_CONNECTION_FAILURES`, `MAX_RECORDER_RESTARTS`, `MAX_TUNNEL_NOTES`, `max_files` |
| Disk handling | `AgentPause`, `DiskPolicy`, `ReserveFile`, `reserve_path`, `disk_free`, `agent_paused` |
| Signing | `ed25519`, `receipt`, `acks.jsonl` |
| Pins and deny lists | `pin-manifests`, `connector_dir_sha256`, `capture_policy` |
| Termination proof | `TerminationOutcome`, `released_child`, `observed_gone` |
| Test seams | `test-support` feature, `pause_at(`, `_FOR_TEST`; a `test-hooks` feature in any `Cargo.toml` other than `crates/goalcup/Cargo.toml` and `crates/goalcup-verify/Cargo.toml` |
| Research decoders | `capture_analysis`, `--research-json`, `--capture-integrity` |

The `test-hooks` feature itself is allowed in those two crates, off by default (PRD-00 ruling on
consistency item 86). That the shipped binary carries no hook is PRD-14's release smoke: no
`GOALCUP_TEST_` text in the release binaries. The Linux subreaper is allowed: PRD-02 copies it on
purpose (review A finding 8), so `subreaper` is not a pattern.

Worker (`server/worker/src/` and `server/worker/wrangler.toml` only). Patterns: `receipt`,
`agent_paused`, `RECEIPT_SIGNING_KEY`, `RUN_MAX_COMPRESSED_BYTES`, and the file `src/sign.ts` must not
exist. `server/worker/sql/` (the database mirror keeps `receipt`, `agent_paused`, `disk_free` and
`connector_dir_sha256` columns, no migration this build), `scripts/check-sql.sh`, `test/` and
`README.md` are not scanned (PRD-07 interfaces row PRD-13).

A hit is either a real reintroduction (the builder removes it) or a false match (the integrator notes
it in the night log and narrows the pattern).

### 7. What is not cut, and why

| Part | Lines | Why it stays |
|---|---:|---|
| `server/worker/src/ratelimit.ts` (device-flow limiter) | 139 src, 184 tests | Guards a real cost: the device flow is the only unauthenticated route (`ratelimit.ts:1-10`). The rule "a limit must guard a real cost and say so" is met. Plan K23 is dropped |
| `server/worker/src/db-local.ts` | 701 src, 590 tests | The Worker's tests and `npm run dev` run with `DB_BACKEND:local` (`server/worker/package.json:11`, `wrangler.toml:60`); PRD-07 keeps it and cuts only its disk-pause twin |
| `server/worker/sql/` | 4,468 | Mirror of the website database; no migration in this build |
| `scripts/fetch-run.sh`, `test_fetch_run.py` | 349, 695 | Rebuild by ID and time (point 5); PRD-07 owns and changes it |
| `scripts/gate.sh`, `release.sh`, `package-release.sh`, `smoke-release.sh`, `third-party-licenses.py` and their tests, `test_install.py`, `test_codeowners.py` | 559 src, 1,170 tests | Release and gate; PRD-14 owns them |
| `scripts/check-version.py`, `check-links.py`, `ci-scope.py`, `linux-check.sh`, `.github/workflows/linux-x86_64.yml` | 1,751 src (plus 63 workflow lines), 2,041 tests | Not PRD-13's to cut. PRD-14 owns them and decides (PRD-00 ownership table); its default deletes all five, `check-links.py` last, after PRD-15's run |
| `packaging/install.sh` | 394 | Install path; PRD-14 drops `goalcup-daemon` from it |
| `docs/plans/**`, `docs/archive/**`, `docs/research/**`, `docs/DECISIONS.md` history | | Kept in this build (PRD-00 "Not in this build"); PRD-15 owns them; the reference guard skips them |

## Files it owns

Delete (PRD-13 runs the `git rm`):

- `crates/goalcup-cli/`, `crates/goalcup-daemon/`, `crates/goalcup-platform/`, `crates/goalcup-protocol/`,
  `crates/goalcup-verifier/` (including `crates/goalcup-protocol/providers.toml` and `pins/`).
- `spikes/` (whole).
- The scripts and tests of group D, `harnesses/install.sh`, `.claude/agents/campaign-cell.md`
  (PRD-00 ownership table).
- `connectors/{amp-cli,claude-code-cli,codex-cli,hermes-cli}/harness.toml` (the delete is listed by
  PRD-11; PRD-13 runs it in the cut commit).

Create: nothing. Change: nothing.

Must not touch: `crates/goalcup/`, `crates/goalcup-format/`, `crates/goalcup-verify/` (the new code),
`Cargo.toml`, `Cargo.lock` (PRD-16), `server/worker/**` (PRD-07 cuts its own parts), `docs/**`,
`README.md`, `AGENTS.md` (PRD-15), `scripts/gate.sh` and every other PRD-14 script, `.github/**`,
`packaging/**`, `.config/**` (PRD-14), `connectors/<id>.toml`, `connectors/model-hosts.toml`,
`docs/harnesses/**` (PRD-11), `skills/**` (PRD-10, PRD-12), `VISION.md` (the owner's).

## Interfaces with other PRDs

| PRD | Gives to PRD-13 | Gets from PRD-13 |
|---|---|---|
| PRD-02, 03, 04, 05, 06, 08, 09, 10, 11, 14, 17 | "copied, tests green" line per ledger row (section 4) | The delete of the source |
| PRD-17 | `LESSONS.md` merged and its four deletion-gate checks green (phase 2 exit) | The old code stays readable at `v0.7.0` |
| PRD-12 | `selftest` passes on integration (precondition for deleting the five kits); its two skill files deleted by PRD-12 itself (PRD-00 ruling; PRD-12's "What is cut" lists them as PRD-13's and should follow PRD-00) | The pad replaces the five kits in section 3 |
| PRD-16 | Phase 0 `exclude` edit and lock; phase 2 gate green; the phase 3 workspace edit in the same merge; the H+9 rule for skipping the cut | Four delete commits on one branch |
| PRD-14 | Its gate, CI, CODEOWNERS, nextest and packaging edits before the cut; both guards in `gate fast` in the forms of section 6; `check-links.py` delete in the phase 3 merge | The list of deleted scripts and binaries. PRD-13 edits no `gate.sh` line (PRD-14's interfaces row PRD-13 and section 4 still mention an exclude line PRD-13 deletes; PRD-00 ruled it away) |
| PRD-15 | Live docs rewritten before the cut: the `AGENTS.md` map (`AGENTS.md:32-50`), pin rule (`:90-91`), D65 rule (`:83-86`), `scripts/README.md`, and the live docs that describe deleted code (`docs/TERMINATION.md`, `docs/CONTRACTS.md` v5 freeze `:510-560`, `docs/CONNECTOR-SCHEMA.md`, `docs/RULES.md:306-321` run labels, `docs/CHEAT-CHECKLIST.md` bounds, `docs/ARCHITECTURE.md`, `docs/OPERATING.md`, `docs/SERVER-DESIGN.md`, `docs/HARNESS-ONBOARDING.md`); run 1's report copied under `docs/research/results/`; `check-links.py` run once | Which docs describe deleted code; the reference guard's history exclusions |
| PRD-11 | `connectors/<id>.toml` and `model-hosts.toml` on integration; `docs/harnesses/*` without old crate names | The delete of `connectors/*/harness.toml` |
| PRD-07 | Its own Worker cuts (group F), including `sign.ts` and the signing vectors | The no-return guard scans only `server/worker/src/` and `wrangler.toml` |
| PRD-10 | Session sniffing on `continue` replaced by its written-count resume rule; pasted `goalcup mcp` lines get no compatibility, one guide line (PRD-10 section 3.1, review row "PRD-13 rows") | `commands/mcp.rs`, `login.rs`, `ssh_keys.rs`, `account/github.rs` are deleted here |
| PRD-08 | `goalcup-verify check --json` reports raw, compressed and framing bytes (S6 `bytes`); S6 `label` and the coverage line replace the cut verifier's run labels | That replaces `scripts/research-measure.py` and the old labels |

## What is cut

Lines are `wc -l`. "src / inline" splits each file as in "Today". "Owner" is the PRD that holds the
copy (the delete itself is always run here). "Replaced by" names the new path or "nothing".

### Group A: the separate daemon and the hand-off (K13, owner decision "option B")

| Path | src / inline | Tests that go | Replaced by | Owner |
|---|---|---|---|---|
| `crates/goalcup-daemon/src/{bin/,control/,ingest.rs,registry.rs,runtime/daemon.rs,runtime/mod.rs,status.rs,config.rs,fsutil.rs,lib.rs}` | 5,816 / 101 | `tests/it/control_api.rs` (1,735), `capture_ingest.rs` (3,095), `tests/health_status.rs` (151), `dir_modes.rs` (338) | Nothing: one process (PRD-02). Private files: a few lines in `src/pipeline/status.rs` | PRD-13 |
| `crates/goalcup-daemon/src/runtime/capture/` (owner, sink, writer, finish, mod, harness, native plan) | 3,915 / 2,477 (`tests.rs` 2,340) | `capture_lifecycle.rs` (3,529), `full/capture_finish_budget.rs` (1,267) | `src/pipeline/` (PRD-02), `PartWriter` (PRD-01) | PRD-13 |
| `crates/goalcup-cli/src/{daemon/,retry.rs,arming.rs,released_child.rs,subreaper.rs}` | 3,269 / 1,059 (`daemon/mock.rs` 636) | in `tests/it/start_launch.rs`, `m2_ipc_compose.rs` (1,606) | Nothing, except about 20 lines of `subreaper.rs` copied by PRD-02 | PRD-13 |
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
| 8 MiB shared capture queue | `crates/goalcup-protocol/src/capture.rs:51` | One stated memory guard that drops DATA of one stream and writes a GAP (PRD-02) |
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
| `crates/goalcup-daemon/src/terminate/`, `runtime/termination.rs` (termination proof) | 2,428 / 355 | `terminate_sim.rs` (2,714), `full/process_kill.rs` (1,285), 12 shell fixtures (213) | Nothing. The watchdog ends the harness tree at the deadline and on seal with no proof (PRD-02 section 8); the seal records how the harness exited (PRD-01 section 11) | none |
| `crates/goalcup-daemon/src/workspace.rs`, `workspace_git.rs` (seal-time walk, K21) | 1,015 / 19 | `workspace_layout.rs` (643) | Two `git` calls in the seal (PRD-02 section 11) | PRD-02 |
| `crates/goalcup-platform/src/tracecap*`, `sourcecap*`, `handle_budget.rs` (K9, K15) | 3,197 / 320 | `unix_tracecap.rs` (325), `unix_sourcecap.rs` (261) | No-follow open chain, about 120 lines (PRD-04) | PRD-04 |
| `crates/goalcup-platform/src/disk.rs` (K5) | 239 / 0 | `disk.rs` (137) | Nothing | none |
| `crates/goalcup-platform/src/{awake,terminal,machine}.rs`, rest | 1,603 / 536 | `machine.rs` (376), `awake.rs` (256) | Sleep lock and browser opener (PRD-10), shield (PRD-02) | PRD-10, PRD-02 |
| `crates/goalcup-protocol/src/{envelope,chain,capture,capture_status,export,ack,receipt,seal,provenance}.rs` (v5 format, K14, K18) | 3,573 / 248 | `m2_contract.rs` (1,715), `export_roundtrip.rs` (1,477), `roundtrip.rs` (1,143), `capture_terminals.rs` (884), `capture_storage.rs` (829), `signing_vectors.rs` | `crates/goalcup-format/` (PRD-01, about 1,000) | PRD-01 |
| `crates/goalcup-protocol/src/run_secret.rs` | 376 / 154 | inline | `src/mask/values.rs` (about 330, copied in part) | PRD-05 |
| `crates/goalcup-protocol/src/{harness.rs,harness/,capture_policy.rs,hints.rs,template.rs}` (schema, deny checks, trust tables, K16) | 4,655 / 106 | `harness_schema.rs` (4,451), `harness_loader.rs` (650), `harness_conformance.rs` (379) | `src/connector/schema.rs` (about 150) | PRD-11 |
| `crates/goalcup-protocol/src/{pins.rs,connector_dir.rs,manifest.rs,constant_time.rs}`, `pins/`, `providers.toml` (K17) | 811 / 432 | `connector_pins.rs` (303), `pins_build_fails.rs` (86), `connector_template.rs` (120) | `connector_sha256` in the start event (PRD-11) | PRD-11 |
| `crates/goalcup-protocol/src/process.rs` and rest | 674 / 314 | | Nothing | none |
| `crates/goalcup-verifier/src/capture_analysis/` decoders (K1) | 6,582 / 401 | `m2_anthropic_decoder.rs` (2,658), `m2_chat_decoder.rs` (2,524), `m2_openai_decoder.rs` (2,149), `m2_native_observations.rs` (1,737), `m2_decoder_v1.rs` (1,126) | Usage reader, about 1,470 (PRD-09) | PRD-09 |
| `crates/goalcup-verifier/src/capture_analysis/` reader, framework, report, stats; `bin/`, `lib.rs` | 5,571 / 934 | `m2_capture_integrity.rs` (3,737), `m2_research_report.rs` (2,456), `m2_run_stats.rs` (873), `v5_corpus.rs` (718, K26) | `crates/goalcup-verify/` (about 1,500, PRD-08) | PRD-08 |
| Test fixtures of the old crates | 7,000 lines in 165 non-`.rs` files | `harness-golden` (2,110, K4), `v5-corpus` (912), `m2-*`, `frozen-connectors`, `schema-1-connectors` | Copied only where a ledger row says so | PRD-03, PRD-05, PRD-09, PRD-17 |
| Test seams: features `test-hooks` (`crates/goalcup-daemon/Cargo.toml:20-21`, `crates/goalcup-protocol/Cargo.toml:14-15`), `test-support` (`crates/goalcup-cli/Cargo.toml:25-26`), `pause_at` hooks (for example `commands/start/phases.rs:286, 830, 856, 1009`) | in the above | | One `test-hooks` feature in `crates/goalcup` (PRD-02 scaffold, PRD-14 registry), off by default; none in the release binary | PRD-14 |

Group A plus C plus the fixtures is the whole of the five crates: 236,242 lines.

### Group D: scripts and other files (K2, K5, K17, K24, K25)

| Path | Lines | Test that goes | Replaced by |
|---|---:|---|---|
| `scripts/matrix.sh` | 2,522 | `test_matrix.py` (1,517) | Pad (PRD-12) |
| `scripts/linux-matrix.sh`, `kit_settle.py` | 876, 302 | `test_linux_matrix.py` (1,219) | Pad in Vercel Sandboxes; first-run answers to PRD-11 `no_prompts` |
| `scripts/campaign.py`, `campaign.example.json` | 1,888, 64 | `test_campaign.py` (1,335) | Pad |
| `scripts/linux-harness-smoke.sh` | 242 | none (run by `linux-x86_64.yml:63`, which PRD-14 deletes) | Pad |
| `scripts/check-connector.sh`, `pin-manifests.sh` | 314, 274 | `test_check_connector.py` (411) | Pad; pins cut |
| `scripts/research-measure.py` | 541 | `test_research_measure.py` (365) | `goalcup-verify check --json` S6 `bytes` (PRD-08) |
| `scripts/research-replay.py` | 342 | `test_research_replay.py` (325) | `goalcup-verify check` on two entries |
| `scripts/research-sample.py` | 424 | `test_research_sample.py` (247) | `goalcup status --json` (PRD-10) |
| `scripts/research-staging.py`, `research-staging.sql` | 604, 166 | `test_research_staging.py` (426) | Nothing; read-only database access on the conductor |
| `scripts/research-watch.py` | 95 | `test_research_watch.py` (104) | `goalcup seal` |
| `scripts/t25-disk-full.sh` | 93 | `crates/goalcup-cli/tests/full/t25_disk_full.rs` (1,173, with the crate) | Nothing (disk full: no special handling) |
| `scripts/test_early_crosscheck.py` | 0 | 538 | Goes with `spikes/m2-first-release/e02` |
| `harnesses/install.sh` | 62 | none | Pad installs each harness (PRD-12) |
| `.claude/agents/campaign-cell.md` | 31 | none | Goes with `campaign.py` |
| **Total** | **8,840** | **6,487** | |

Not in this group (PRD-14's, section 7): `check-version.py`, `check-links.py`, `ci-scope.py`,
`linux-check.sh` and their tests. The `.txt` campaign prompts in `docs/research/` are PRD-15's history.

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

Listed so the cut list is whole. PRD-07 runs these deletes; its "What is cut" table is the source of
the numbers.

| Path | src | Tests that go | Replaced by |
|---|---:|---|---|
| `src/sign.ts` and signer plumbing (`context.ts:133-145, 168-170`, `chunks.ts:82`) | about 135 | `sign.test.ts` (68), `test/vectors/signing.json` (26) | Nothing; `RECEIPT_SIGNING_KEY` goes |
| Route `POST /v1/runs/{id}/receipt` (`runs.ts:352-389`, `db-*.ts`) | about 75 | receipt cases (about 150, est.) | Nothing; signing is cut |
| Route `POST /v1/runs/{id}/abandon` (`runs.ts:393-414`, `db-*.ts`) | about 60 | abandon cases (about 120, est.) | Nothing |
| Seal documents and the tunnel list (`runs.ts:25-29, 57-85, 535-637`) | about 140, about 20 back | tunnel and seal-document cases (about 250, est.) | The seal and tunnels are records (PRD-01 section 11) |
| Per-run size limit (`chunks.ts:74, 123-144`, `context.ts:11-12, 26-27, 61-67, 126`) | about 50 | `quota.test.ts` (114), `config.test.ts:237` | Nothing |
| Vitals validator (`runs.ts:499-533`), `Vitals` fields, local disk-pause twin (`db-local.ts:612-623`) | about 57, about 12 back | vitals and disk-pause cases | Nothing (disk full: no special handling) |
| `tracer_version` alias (`util.ts:79-91`) | about 10 | alias cases | Nothing |
| Judged commit by cron (`github.ts:173` onward, `notices.ts:68-80`) and start-time repo checks (`runs.ts:314-351`) | | | **Not cut.** Kept and untouched in 1.0 (PRD-00 question 3); it is off the record path |

### Group G: connector fields (owner PRD-11)

`connectors/*/harness.toml` (556 lines) are replaced by `connectors/<id>.toml` and `model-hosts.toml`.
Fields with no successor (PRD-11 "What is cut"): `[participant.time_left_tool]`
(`amp-cli:17`, `claude-code-cli:15`, `codex-cli:13`, `hermes-cli:21`), `[capture.identity]`,
`[capture.termination]`, `[capture.argv_guard]` (`hermes-cli:124`), `[launch_prep]` (`hermes-cli:145`),
`[[capture.hosts]]` (now the shared list), `[capture.platform.*]`, CA `mode`, `protocol_hint`,
`since_run_start`, the email rule and the Codex rate-limit, plan and credit mask rows
(`codex-cli:110-113`). Kept in the new schema by PRD-00's ruling: `no_prompts`, `first_screen`,
`warn_missing` and a `vendor` flag (seam S3).

### The plan's K list, mapped to 1.0

| K | What | Plan timing | In 1.0 | Group |
|---|---|---|---|---|
| K1 | Verifier research decoders | Before Friday, then moved | Cut; model and usage parts copied (PRD-09) | C |
| K2 | Scripts with no point, `check-version.py` | After | Cut; `check-version.py` by PRD-14 | D |
| K3 | Rust Worker stand-in | After | Cut with `goalcup-cli`; one server (PRD-07) | C |
| K4 | Connector byte goldens | Before | Cut | C |
| K5 | Disk-full handling, `SIGSTOP` pause | Switch before, code after | Cut, client and Worker | C, D, F |
| K6 | Spike prototypes and data | Before | All of `spikes/` cut | E |
| K7 | Provider probe, start leftovers | After | Cut; session sniffing replaced by PRD-10's resume rule | C |
| K8 | `goalcup mcp` lines, `git-credential` | Part before | Cut whole | C, G |
| K9 | Platform dead code, `handle_budget.rs` | After | Cut with the crate | C |
| K10 | Uploader leftovers | After | Rewritten (PRD-06) | C |
| K11 | Proxy outage windows | After | Rewritten (PRD-03) | C |
| K12 | Daemon writer quota | Before | Cut with the daemon | A |
| K13 | Daemon merge | After | Done: option B | A |
| K14 | Back-compat, retired versions | After | Cut; no v5 reader, no run 1 converter | C |
| K15 | Platform rewrite | After | Crate cut; about 120 lines rewritten (PRD-04) | C |
| K16 | Deny checks, trust tables | After | Cut | C |
| K17 | Pins, `pin-manifests.sh`, `check-connector.sh` | After | Cut | C, D |
| K18 | Receipt, ack and status signing | After | Cut, client and Worker | C, F |
| K19 | CLI extras (`login`, SSH keys, `doctor`, `mcp`) | After | Cut; push lines trimmed (PRD-10) | C |
| K20 | Judged commit | After | Kept, untouched (PRD-00 question 3) | F |
| K21 | Seal-time workspace walk | After | Cut; two `git` calls | C |
| K22 | Finish stage machine | After | Cut; PRD-02 seal | A |
| K23 | Rate limiter Durable Object | After | **Kept**: guards a real cost (section 7) | |
| K24 | Crash and failure campaign tests | Waited for N5 | Cut with the crates; the kill test (PRD-01 acceptance 3, PRD-02 R1) replaces them | C |
| K25 | `check-links.py`, `spikes/redact.py` | After | Cut (links last, by PRD-14) | D, E |
| K26 | v5 corpus test | Before | Cut | C |
| K27 | Leftovers (Worker disk vitals, owed acks, give-back, `m2_contract.rs`) | After | Cut; GCR1 golden replaces `m2_contract.rs` (PRD-01) | C, F |

Sources: plan revision 1 (`git show b19ebd44:docs/plans/2026-10-30-cut-first-plan.md`, lines 107-138),
revision 4 sections 6a and 6b (`docs/plans/2026-10-30-cut-first-plan.md:287-320`), and the review
corrections (`docs/plans/2026-10-30-audit/R-reviews.md:166-298`).

### Point check

| Cut | Point that could suffer | Why it does not |
|---|---|---|
| Separate daemon and hand-off | 1, 2, 3 | It is what failed in run 1 (no gap for 470 lost calls). One process writes a gap for every loss (PRD-02) |
| Termination proof | 2 (subagents and helpers left running) | The watchdog ends the harness tree at the deadline and on seal (PRD-02 section 8). A Linux subreaper keeps helpers that outlive the harness findable, an `outlived` note names them, and bytes relayed after the stop are an `after_stop` GAP (PRD-02, review A finding 8). Subagent files are read the same way after the harness exits (PRD-04 final snapshots) |
| Disk handling | 3 | The `SIGSTOP` pause itself stopped the harness (`crates/goalcup-cli/src/run/pause.rs:5-10`). A write error ends the recorder (exit 74); the watchdog restarts it and the down time is a `recorder_down` GAP whose `cause` names `write_error` (PRD-02 section 5). GAP reason 8 `write_error` is reserved and never written (PRD-01 section 3) |
| Signing | 5 | The hash chain stays (PRD-01 section 10); the Worker stores create-only with SHA-256 in the key |
| Research decoders | 1, 2 at judging | The usage reader (PRD-09) and S6 `label` with the coverage line (PRD-08) give model, tokens, the per-subagent view and the evidence label |
| Pins, deny checks | 4, 7 | Review is the control for connectors (`CODEOWNERS` comment); the start event carries `connector_sha256` |
| Provider probe | 6 | A run with no model requests says so plainly (PRD-10) |
| Scripts and spikes | none | Organizer and research tools; the pad replaces the test kits |

## Target size after 1.0

New-code numbers are the owning PRDs' estimates: src from each PRD, tests from PRD-14's test budget
(PRD-14 section 3, the one source for tests, PRD-00 "Size budget").

| Area | Today src | Today tests | 1.0 src | 1.0 tests |
|---|---:|---:|---:|---:|
| `crates/goalcup-format` (PRD-01) | | | 1,000 | 800 |
| `crates/goalcup` pipeline and watchdog (PRD-02) | | | 1,350 | 1,500 |
| proxy (PRD-03) | | | 4,400 | 2,000 |
| native (PRD-04) | | | 870 | 1,000 |
| mask (PRD-05) | | | 3,550 | 5,000 |
| upload (PRD-06) | | | 940 | 1,000 |
| CLI (PRD-10) | | | 2,960 | 1,200 |
| connector code (PRD-11) | | | 750 | 400 |
| `crates/goalcup-verify` check (PRD-08) | | | 1,500 | 1,200 |
| usage reader (PRD-09) | | | 1,470 | 600 |
| lessons (PRD-17) | | | 0 | 1,500 |
| test support, `rule_code`, `e2e_run` (PRD-14) | | | 0 | 1,800 |
| **Rust** | **86,516** | **142,413** (+7,000 fixture lines) | **about 18,800** | **about 18,000** |
| Worker (PRD-07) | 4,001 | 5,775 | about 3,440 | about 4,950 |
| Scripts | 11,406 | 10,393 | 908 (gate, release, packaging, licences, fetch-run) plus the pad (PRD-12; about 1,350 by review B, unverified) | 1,865 plus the pad's |
| Packaging, `harnesses/` | 456 | | 394 | |
| **Code that ships or runs** | **about 102,400** | **about 158,600** | **about 23,550 plus the pad** (minus 77%) | **about 24,800 plus the pad's** (minus 84%) |
| Data: connectors | 556 | | about 390 | |
| Data: spikes | 23,713 | | 0 (86 corpus files, 6.2 MB, move under `crates/goalcup/tests/mask_corpus/`) | |

The Rust src sum is 18,790; PRD-00 rounds the same table to about 18,700 because it took PRD-02 at
1,300. The judge view and the run 1 converter are out of this build (PRD-00 "Not in this build"), so
PRD-08 may land lower. The binaries drop from three to two; `Cargo.lock` already loses
`ed25519-dalek`, `redb`, `axum` and what only they pull in when PRD-16 regenerates it in phase 0 with
the old crates excluded (today 218 packages; unverified until built).

How to recount after the cut (the watcher posts the output in the night log):

```
find crates -path '*/src/*' -name '*.rs' | xargs wc -l | tail -1        # src with inline tests
find crates -path '*/tests/*' -name '*.rs' | xargs wc -l | tail -1      # test files
wc -l server/worker/src/*.ts | tail -1; wc -l server/worker/test/*.ts server/worker/test/helpers/* | tail -1
git ls-files scripts | xargs wc -l | tail -1
```

## Acceptance

Real-run proofs first. Each runs on the post-cut commit, built in a fresh Vercel Sandbox build box from
a fresh clone of `v1.0/integration`, against the staging Worker, with `GOALCUP_KEEP_PARTS=1`.

1. **The core set still passes after the cut.** The pad (PRD-12) runs Claude Code, Codex and Hermes,
   direct-key case, on binaries built from the post-cut commit. Pass: the same pass or fail row per
   harness as the last pre-cut pad run; any new failure blocks the merge of the cut.
2. **One process, two binaries.** During the Codex run, `ps -eo pid,comm` shows the `goalcup` watchdog
   and recorder and no `goalcup-daemon`; `ls target/release` after `cargo build --release --locked`
   shows `goalcup` and `goalcup-verify` and no third binary. Pass: both true.
3. **Kill test still holds.** PRD-01 acceptance 3 (PRD-02 R1) re-run once on the post-cut build. Pass:
   as there.
4. **Server copy whole.** `scripts/fetch-run.sh` plus `goalcup-verify check --server` on the Codex run
   (PRD-07, PRD-08). Pass: equal SHA-256 to the kept local parts.
5. **Release tarball.** `scripts/package-release.sh` and `smoke-release.sh` (PRD-14) on the post-cut
   commit. Pass: the tarball holds two binaries, the smoke passes, and neither binary contains the text
   `GOALCUP_TEST_`.

Automated checks that guard them:

- `cargo build --locked --workspace` and `sh scripts/gate.sh full` green with the old crates gone
  (PRD-00 phase 3 gate), in a build box as `ubuntu`.
- The reference guard (section 6) prints nothing in `gate fast` after the cut, with no `gate.sh` edit.
- The no-return guard (section 6) prints nothing over the new crates from phase 1, and over `crates/`
  and the Worker scope after the cut.
- `python3 -m unittest discover -s scripts` green with no test for a deleted script left behind.
- The size recount is posted, and each area is within 30% of its row in "Target size", or the owner
  of that area names the reason in the night log.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| A lesson is lost | An edge case the old code handled is not in `LESSONS.md` and the fresh core misses it | Delete only after PRD-17's deletion gate passes; the old code stays at `v0.7.0` for any later look |
| The cut is skipped | PRD-17's gate is not green by PRD-16's cut-off | The old crates stay on disk, out of the workspace; nothing builds them; the end-of-build report says so and the cut lands in a later merge |
| A reference hides until the cut | The reference guard skips while old crates are on disk, so a live file that names an old crate is not caught by the gate | PRD-13's hourly hand-run with the deleted paths excluded (section 5) names each hit and its owner in the night log |
| Dangling references break the gate | `scripts/test_codeowners.py` fails on any `CODEOWNERS` path that no longer exists; `.config/nextest.toml` names deleted packages; `gate.sh:88-103` runs `-p goalcup-cli` | PRD-14 lands these edits in phase 0 or 1, when the old crates leave the workspace; they no longer wait for the cut |
| Dry-run participants on 0.7 | Pasted `goalcup mcp` lines now start a missing command; an old `goalcup-daemon` may still run; an old binary stays in `~/.local/bin` | PRD-10: no compatibility, one guide line. PRD-14's installer removes `goalcup-daemon` from the install folder. Whether an old daemon holds a port 1.0 needs is unverified |
| Judging loses run labels | `docs/RULES.md:306-321` and `docs/CHEAT-CHECKLIST.md` cite labels that only the cut verifier computed | PRD-08 keeps S6 `label` and the coverage line; PRD-15 rewrites the rules text |
| Docs links break | `check-links.py` goes in the same build in which PRD-15 rewrites many docs | PRD-15 runs it once on the final docs; PRD-14 deletes it last |
| Linux filesystem coverage | `linux-check.sh` ran the gate on ext4 and XFS in Docker; if PRD-14 cuts it, that coverage goes | The pad runs on Linux sandboxes; macOS is the owner's run after the build (PRD-00) |
| Website database keeps dead columns | Receipt, ack and disk-pause columns stay unused; no migration in this build | Owner question in PRD-00; the no-return guard skips `server/worker/sql/` |
| Size overrun | The new code is larger than budget | Recount at the cut; each area over its estimate by 30% names why |
| A guard pattern hits a fair use | For example a variable called `receipt` in an unrelated sense, or a copied test comment | The integrator narrows the pattern in the night log; provenance comments are filtered; the guard never blocks a build on its own |
| The subreaper comes back as termination proof | The copied 20 lines grow into the old sweep and refusal | PRD-02 copies only `PR_SET_CHILD_SUBREAPER` and the start-time filter; `released_child`, `TerminationOutcome` and `observed_gone` stay guarded |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Device-flow rate limiter | Kept (real cost) | Owner |
| Judged commit in the Worker | Kept, untouched (PRD-00 question 3) | Owner |
| Where the old code lives | Tag `v0.7.0` and main `4b226de2`; no new tag | PRD-16 may add `v0.7-final` |
| History docs in the reference guard | Excluded: `docs/plans/**`, `docs/archive/**`, `docs/research/**`, `docs/DECISIONS.md`, `CHANGELOG*` | PRD-15, by a night-log line |
| Size tolerance | 30% over the owner's estimate, with a reason | PRD-16 |

## Builder notes

Order of work:

1. **Phase 0 (H+0):** nothing to commit. The binary collision is settled by PRD-16's `exclude` edit.
   Start the copy ledger in the night log.
2. **Phases 1 and 2 (read only):** keep the ledger. Each hour, run the reference guard by hand with the
   to-be-deleted paths and `Cargo.toml` excluded, and post hits with their owner. Draft the four
   commits on `v1.0/prd-13-cuts` and rebase them each hour, so the cut is ready the minute phase 2 is
   green.
3. **Phase 3:** check every ledger row says "copied, tests green", PRD-17's deletion gate is green and
   PRD-12's `selftest` passed. Apply the four commits. Run the build, the two guards and `gate full` in
   a build box, then hand the branch to PRD-16 for the merge with its workspace edit and PRD-14's
   `check-links.py` delete.
4. **After merge:** a watcher runs acceptance 1 to 5 and posts the size recount.

Traps from the audit:

- The lock: with the old crates out of the workspace since phase 0, `Cargo.lock` should not change at
  the cut. If `cargo build --locked` fails after PRD-16's workspace edit, the integrator refreshes the
  lock once (`cargo generate-lockfile` in a build box) and commits it; PRD-13 never does.
- Do not delete `scripts/fetch-run.sh`, `third-party-licenses.py`, `server/worker/sql/` or
  `server/worker/src/db-local.ts`: they look old but serve point 5, the release and the Worker tests.
- `scripts/test_early_crosscheck.py` reads `spikes/m2-first-release/e02/`; delete both together.
- The mask corpus test floor: PRD-05's copied corpus must hold at least the files its tests count
  (the old floor was 50 files, review R item 7); copy before `git rm spikes/`.
- Cut whole folders with `git rm -r`; do not leave empty `tests/fixtures` folders that a glob in a new
  test might read.
- Never run the old crates' test suites to "check" before deleting them: they are out of the
  workspace since phase 0, and their behaviour is what 1.0 replaces.
- A copied test that keeps a comment naming `SIGSTOP` or an old type is fine if the comment has the
  `v0.7.0:` provenance form; otherwise reword it.

How a watcher proves it in a real run: in a fresh sandbox, clone `v1.0/integration` at the merge
commit, build release, run the pad's Codex direct-key case with the payments prompt and many
subagents against staging, watch `ps` for a `goalcup-daemon` (there must be none), run
`goalcup-verify check` on the entry and on the fetched server copy, and post the pad row, the `ps`
line, the two guard outputs and the size recount in the night log.

## Asks of PRD-00 and PRD-01

All seven asks of revision 1 are answered (PRD-00 "Review answers", PRD-01 CC 89). None is open.

| Ask | Answer |
|---|---|
| 1 Workspace members | PRD-16's edit: `exclude` in phase 0, removed in the phase 3 merge |
| 2 Unowned paths | PRD-00 ownership table: `.github/**`, `.config/**`, `linux-check.sh`, `third-party-licenses.py` to PRD-14; `scripts/README.md` to PRD-15; `linux-harness-smoke.sh`, `campaign-cell.md`, `harnesses/install.sh` to PRD-13 |
| 3 Binary collision | Settled by the phase 0 `exclude`; no rename |
| 4 Size budget | PRD-00 takes the owners' numbers |
| 5 Judged commit | Kept, untouched |
| 6 Phase 3 gate wording | The reference guard of section 6, history and provenance excluded |
| 7 PRD-01 v5 size and `run_secret.rs` | PRD-01 now says 4,351 lines and "copied in part (PRD-05)" |

## Review answers

Sources: `prd-consistency.md` (CC), `prd-review-A.md` (RA), `prd-review-B.md` (RB). Code checked where
an item was disputed.

| Item | What | Answer |
|---|---|---|
| CC 86, RB C15 | No-return guard bans the `test-hooks` feature other PRDs need; `receipt` and `agent_paused` hit the SQL mirror | **Accepted.** `test-hooks` allowed in `crates/goalcup` and `crates/goalcup-verify` only; the release smoke checks no `GOALCUP_TEST_` text. Worker scan narrowed to `src/` and `wrangler.toml`; checked: `receipt`, `agent_paused`, `disk_free` and `connector_dir_sha256` today sit in `sql/`, `scripts/check-sql.sh`, tests and `db-*.ts` (git grep). PRD-07 removes them from `src/` |
| CC 87 | Point check says a write error is a `write_error` gap | **Accepted.** Reworded: exit 74, `recorder_down` GAP with `cause` `write_error`; reason 8 reserved (PRD-01 line 212) |
| CC 88 | Ledger `terminal.rs:1-60` is the module comment | **Accepted, checked.** Lines 1-25 are the module comment and `Shield` starts at 53. Ledger now uses PRD-02's `:27-292` into `src/watchdog/shield.rs` |
| CC 89, ask 7 | v5 size and `run_secret.rs` in PRD-01 | **Closed.** PRD-01 took the numbers and the wording |
| CC 90 | Test numbers differ from PRD-14 | **Accepted.** Target size copies PRD-14 section 3 (about 18,000) |
| CC 91, CC 13, RB C17, RB M9 | Asks 1 and 3; the `gate.sh` exclude line | **Accepted.** PRD-16's phase 0 `exclude`; no rename; PRD-13 edits no `gate.sh` line (rule 6). PRD-14's lines about an exclude line PRD-13 deletes are stale against PRD-00 |
| CC 92 | `db-local.ts` reason; `ci-scope.py` row | **Accepted, checked.** `db-local.ts` stays because the Worker's tests and `npm run dev` use `DB_BACKEND:local` (`server/worker/package.json:11`); the "pad's local Worker" reason is gone (staging). `ci-scope.py` is PRD-14's |
| CC 9 | Two leftover files to PRD-13 | **Accepted.** `campaign-cell.md` and `harnesses/install.sh` stay in Files it owns |
| CC 10, RB C21 | `check-version.py`, `check-links.py`, `ci-scope.py` have two deleters | **Accepted.** Removed from group D; PRD-14 owns them (section 7), `check-links.py` deleted in the phase 3 merge after PRD-15's run. Group D recounted: 8,840 lines, 6,487 test lines (`wc -l`). `linux-check.sh` also moved to PRD-14 per PRD-00 |
| CC 10, RB C20, CC 102 | Keep or delete history docs | **Accepted as ruled by PRD-00: kept.** Section 7 says PRD-15 owns them; the reference guard excludes `docs/research/**` too, since every research doc names old crates (checked with the guard command) |
| CC 20, RB C22 | Judged commit open (ask 5) | **Accepted.** Kept, untouched; group F row and K20 changed |
| CC 24 | Size budget | **Closed.** PRD-00 took the owners' numbers; this PRD shows 18,790 with PRD-02 at 1,350 |
| CC 27 | Run report copy hits the guard | **Accepted.** Excluded through `docs/research/**` |
| CC 82 | `agent_paused` in Worker code | **Accepted.** See CC 86; PRD-07 stores `counts` nested so `src/` never names it |
| CC 107 | PRD-13 starts read only at H+0 | **Accepted.** PRD-00 phase 0 step 10 says so; builder notes match |
| RB M5 | One big phase 3 merge | **Accepted.** Section 5 rewritten: other owners land their edits before the cut; the phase 3 merge holds four delete commits, PRD-16's workspace edit and PRD-14's `check-links.py` delete. Reference guard skips while old crates exist, so no `gate.sh` edit at the cut |
| RB M6 | One night-log writer | **Accepted.** Ledger lines may sit in the receiving lane's own log file |
| RB "Too big" (verdict 2) | 18,700 src and 16,500 test lines | **Partly.** Numbers restated from owners; cutting scope is PRD-00's job (slices and "Not in this build"). This PRD adds no code |
| RB section 10 | Smaller plan | **No change needed.** The cut is phase 3 in every version of the plan |
| RA 8 | Processes that outlive the harness; the subreaper is deleted | **Accepted for this PRD's part.** `subreaper` removed from the no-return guard; ledger row for PRD-02's 20-line copy; point check row cites PRD-02's after-stop GAP and `outlived` note. Termination proof stays guarded by `released_child`, `TerminationOutcome`, `observed_gone` (checked in `runtime/termination.rs`) |
| Own check | `SIGSTOP` as a guard pattern | **Changed.** The copied shield's tests mention `SIGSTOP` (`terminal.rs:413`) and the pad freezes trees with it; the disk row now names the cut types (`AgentPause`, `DiskPolicy`, `ReserveFile`, `reserve_path`, checked with git grep) |
| Own check | Provenance comments in the no-return guard | **Changed.** Copies from `capture_analysis` carry that name in their `v0.7.0:` comment; the guard filters that form as the reference guard does |
| PRD-10 rows | Session sniffing, `goalcup mcp` | **Closed.** PRD-10 decided both; the overridable default row is gone |
| PRD-12 What is cut | Lists `linux-check.sh` and its skill files as PRD-13's | **Rejected for this PRD.** PRD-00 gives `linux-check.sh` to PRD-14 and the skill files to PRD-12; noted in the interfaces row |
| CC 112 | Style | Checked: no em dash, no model names |
