# TODO

The open list for goalcup 1.0. Decisions live in [DECISIONS](docs/DECISIONS.md). Done means the fast
gate passes, two independent reviewers read each diff during the 1.0 build, and README, the
participant guide or this file is updated if behaviour changed ([AGENTS.md](AGENTS.md#done)).

## Open owner questions

The twelve questions are in [PRD-00](docs/plans/2026-10-30-v1.0/PRD-00-index.md#owner-questions).
None is answered yet. The build uses each default below until the owner answers.

1. Server and data. Default: no website database change; usage stats in R2, for organizers only.
2. Participant-facing changes. Default: yes to all five, including the whole environment for the
   harness minus proxy and CA variables.
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

- O1 ([ledger section 9](docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md#9-open)), staging and the
  website database. Default: shared; no migrations, read-only use.
- O2, Copilot CLI and a subscription. Default: try the key; "not verified" if it fails.

D61's "forwards no key" clause stays open with question 2, whose whole-environment default touches it.

## Known issues

- macOS is not tested in this build; Linux arm64 is built but not tested (ledger T9 and section 8,
  [OWNER-DECISIONS](docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md)).
- Swap on Linux is the one way raw bytes can still reach disk
  ([PRD-00](docs/plans/2026-10-30-v1.0/PRD-00-index.md#architecture)).
- The `zstd` command line tool may stop short on a live entry; read one with `goalcup-verify dump`
  ([RECORD-FORMAT](docs/RECORD-FORMAT.md#compression-chunks-and-crash-recovery)).
- H-28: usage objects in the record carry plan, credit and billing data. They are stripped before
  any research export, which comes after the event (D26).
- H-29: a key the model echoes split across stream pieces is written unmasked
  ([PRD-05](docs/plans/2026-10-30-v1.0/PRD-05-masking.md#risks-and-blind-spots)).
- F-07: a guarded key prefix inside a long base64url run masks the rest of the run
  ([PRD-05](docs/plans/2026-10-30-v1.0/PRD-05-masking.md#risks-and-blind-spots), question 10).

## After the merge to main

- The history purge: `docs/archive/`, `docs/research/`, the dated plans, `docs/RESEARCH-TARGET.md`
  and `docs/LOCAL-RESEARCH-HANDOFF.md` (PRD-15).
- The DECISIONS rewrite to 24 entries with a retired-numbers table (PRD-15). Each "This supersedes"
  line names every old entry its status rows point to (D3, D9, D37, D40, D43, D45, D46, D52, D53
  and D71 are not named today); D75, D76, D80, D81 and D84 go back to short prose.
- Hermes notes from D61 and D72 move into [its facts sheet](docs/harnesses/hermes-cli.md) (PRD-11).
- PRD-15's contributor run and organizer run. The judge view (PRD-08).
- PRD-14 G2 (sabotage) and G5 (the release workflow on four targets); PRD-12 P8 (contributor dry run).
- `stopped-runs.sql`, the organizer query for stopped runs (PRD-07).
- HTTP/2 PUSH_PROMISE, WebSocket over HTTP/2 (RFC 8441) and the gRPC branch (PRD-03).
- The mask worker pool, only if PRD-02's throughput test shows masking is the slow stage (PRD-02).
