# goalcup 1.1: open list

The open list for goalcup 1.1. Decisions live in [DECISIONS](docs/DECISIONS.md). Done is defined in
[AGENTS.md](AGENTS.md#done).

## Open owner questions

The twelve questions are in [PRD-00](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/PRD-00-index.md#owner-questions).
The build uses each default below until the owner answers.

1. Server and data. Default: no website database change; usage stats in R2, for organizers only.
2. Participant-facing changes. Default: yes to all five, including the whole environment for the
   harness minus proxy and CA variables; this touches D61's "forwards no key" clause.
3. Masking beyond option b. Default: no; only the run's `GH_TOKEN` is masked by exact value.
4. Rules and judging. Default: yes to all three; a tier does not decide eligibility.
5. Release order. Default: publish 1.0.0, then deploy production with the 1.0.0 floor.
6. Run 1's prompt. Default: not committed; without the file, A1 runs the pad's `full` prompt.
7. Hermes and per-call matching. Default: per call when `state.db` has usable times, else per session.
8. Short answers and the build budget. Default: the readings and report lines PRD-00 gives.
9. The watchdog's jobs. Default: the `start` or `continue` process, as PRD-00 lists.
10. Left guard class. Default: `[A-Za-z0-9_]`.
11. Rate-limit headers. Default: no exemption from the name rule.
12. One test-only build feature. Default: keep it, never in a release build.

- O1 ([ledger section 9](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md#9-open)), staging and the
  website database. Default: shared; no migrations, read-only use.
- O2, Copilot CLI and a subscription. Default: try the key; "not verified" if it fails.

## Still to decide

- Judging and result dates.
- Event capacity limits.
- Final judging rubric, shortlist threshold and appeal process.

## Known issues

- Linux x86_64 and macOS arm64 (owner's Mac, 3 Nov) are tested. Linux arm64 is built by the
  release workflow, not tested. macOS Intel is not supported. Windows (WSL2) is untested.
- Unmasked memory (held bytes, the run CA key) can still reach disk through swap on Linux
  ([PRD-00](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/PRD-00-index.md#architecture)).
- The `zstd` command line tool may stop short on a live entry; read one with `goalcup-verify dump`
  ([RECORD-FORMAT](docs/RECORD-FORMAT.md#compression-chunks-and-crash-recovery)).
- H-28: usage objects in the record carry plan, credit and billing data. They must be stripped
  before any research export, which comes after the event (D26); nothing does it yet.
- H-29: a key the model echoes split across stream pieces is written unmasked
  ([PRD-05](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/PRD-05-masking.md#risks-and-blind-spots)).
- F-07: a guarded key prefix right after a `-` inside a base64url run masks the rest of the run,
  failing closed past 16 KiB, up to the end of the message
  ([PRD-05](https://github.com/agentcommunity/goal-cup-proxy/blob/archive/docs/plans/2026-10-30-v1.0/PRD-05-masking.md#risks-and-blind-spots), question 10).

## Owner, before 5 Dec

- Production Worker deploy ([OPERATING](docs/OPERATING.md#deploy)).
- Website purge migration.
- Canary on.
- Little Snitch back on, with one rule for `goalcup` to any server.
- In `server/worker/wrangler.toml` `[vars]`: raise `DEVICE_TOKEN_PER_MINUTE` and
  `DEVICE_START_PER_MINUTE` if the event needs more (1200 and 120 today), and set `EVENT_ID` to the
  cup event on the day before the start window opens, not earlier.

## After 1.0

- A one-hour run that passes (best so far misses on harness subagent counts, not loss).
- macOS CI job for the lifecycle tests (owner decision).
- A test run cut off on macOS can leave `goalcup recorder` orphans that keep respawning (found 4 Nov: 6 from a 3 Nov run, still alive); tests should end their process group, and check that the respawn cap (C.3) stops an orphan with no watchdog.
- Recorder failure domain: keep the relay alive across recorder restarts.
- Full proxy drain before the recorder exits.
- `run` and `ctl` leaf modules (arch-4).
- One include point for connector data (5 `include_str!` sites: both `model-hosts.toml`, both `shapes.tsv`, `tiers.toml`).
- Signed releases.
- Windows (WSL2) tested.
- Multi-version matrix for Claude Code and Codex.
- A newcomer reads the docs through.
- Restated passwords stay raw (OpenClaw): server-side scrub.
- Copilot and Droid subagent readers; Cline and Factory Droid rows.

## Left from the 1.0 plans

- Repoint the harness sheets in `docs/harnesses/` from [HARNESS-SETUP](docs/HARNESS-SETUP.md) to
  the [participant's guide](docs/PARTICIPANT-GUIDE.md), then delete HARNESS-SETUP (PRD-11).
  Until then the two duplicate each other: change both.

- The DECISIONS rewrite to 24 entries with a retired-numbers table (PRD-15). Each "This supersedes"
  line names every old entry its status rows point to (D3, D9, D37, D40, D43, D45, D46, D52, D53
  and D71 are not named today); D75, D76, D80, D81 and D84 go back to short prose.
- Hermes notes from D61 and D72 move into [its facts sheet](docs/harnesses/hermes-cli.md) (PRD-11).
- PRD-15's contributor run and organizer run.
- The judge view (PRD-08).
- PRD-14 G2 (sabotage) and G5 (the release workflow on three targets); PRD-12 P8 (contributor dry run).
- `stopped-runs.sql`, the organizer query for stopped runs (PRD-07).
- Following HTTP/2 PUSH_PROMISE, WebSocket over HTTP/2 (RFC 8441) (PRD-03); today each is a GAP
  (`h2_unsupported`), never silent. Compressed gRPC bodies are a GAP too (`unsupported_encoding`).
- The mask worker pool, only if PRD-02's throughput test shows masking is the slow stage (PRD-02).

## After the event

- One copy of `own_server`/`OWN_SERVER` and the `shapes.tsv` parsing, in
  `crates/goalcup-format/src/shared.rs`.
- A seam S1-S18 table in [ARCHITECTURE](docs/ARCHITECTURE.md).
- Cut `scripts/check-links.py` to about 80 lines.
- `tomllib` in the pad, when the box has Python 3.11.
- Pad `version` and `release_versions` checked against the connector `checked` and `[[versions]]`.
