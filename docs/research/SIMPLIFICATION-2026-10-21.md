# Simplification plan (21 Oct 2026)

Five reviews: architecture, tests, protocol and connectors, server, scripts and docs. Merged here
and split by timing. The feature stop is 4 Nov and the event starts 7 Nov, so anything that moves
lots of code waits until after the event. Sizes are rough.

## The honest numbers

- Product code: about 94k lines of Rust, 3.8k TypeScript. Tests: about 125k lines of Rust plus the
  Worker and script tests.
- **Over-testing is smaller than it looks.** The test review found only about 6k lines (5%) that can
  go without losing a promise; most tests are real behaviour tests. The real cost is **time**: the
  fast gate takes about 5.5 minutes, and 12 redaction tests use over half of it.
- **The biggest code wins are dead paths**: headless mode (no shipped harness uses it, about 1,500
  lines) and the old signed-finalization code (no writer, about 800 lines). Both should go after
  the event, when nothing depends on the verifier staying fixed.

## Before the event (low risk)

| # | Change | Why | Size |
|---|---|---|---|
| B1 | Merge the five pending migrations into one file; test it on a throwaway Postgres first | One review, one transaction, one apply on the shared database; no SQL is executed by any test today | S |
| B2 | Fix the Rust upload stand-in where it drifted (test runs still replace; no 413 size limit) | Tests must match the real server | S |
| B3 | Speed up the gate: split the slow P1 redaction loops into separate tests so they run in parallel; optimise the daemon in test builds; run each kit case once, not twice; shrink one slow serial test | Fast gate about 335 s to about 190 s; slowest test 99 s to about 10 s; no promise test deleted | S to M |
| B4 | Shrink goldens: keep only non-zero rows of `decoder-selection.json`; drop manifest echoes from per-connector goldens | About 5,400 fixture lines, fewer regenerations per connector PR | S |
| B5 | Delete small dead code and dead tests: two ignored tests that assert old behaviour, unused helpers (`validate_questionnaire_version`, `readcap`), three copies of one `say` helper; move the T25 measurement test to `scripts/`; define the ingest handshake once | Less to read | S |
| B6 | Docs map: `docs/README.md` (by audience) and `scripts/README.md`; fix AGENTS.md pointing new agents at the 18 Oct handoff; fix stale text; move finished TODO sections to `docs/archive/`; a status table at the top of DECISIONS (current, superseded, retired) | A new person or agent finds the right doc in one step | S |
| B7 | Harness builders: say "chat mode only" in the template and onboarding (headless moves to a core appendix); a one-screen "what a chat connector writes" table at the top of CONNECTOR-SCHEMA | Adding harness 5 to 15 gets much simpler; about 15 of 60 fields are unused by every shipped connector | S |
| B8 | One glossary: "run" and "part" for participants, "segment" inside the code, "chain file" for rotated files | One concept, one name | S |
| B9 | Move the GitHub code out of the cron file into `github.ts` | The cron file has four jobs | S |

## After the event

| # | Change | Size saved |
|---|---|---|
| A1 | Delete headless mode code (launch, start, harness checks, daemon deferred binding, kit cases) | about 1,500 lines |
| A2 | Delete the signed-finalization and legacy-seal paths (needs a D54 note; after judging) | about 800 lines |
| A3 | Status document and control API v10: drop about 12 always-constant fields | about 250 lines |
| A4 | Registry: drop the V1 legacy variants (after every event run has closed) | about 250 lines |
| A5 | Split the big files: `start.rs` (4,100 lines) into six modules, `uploader.rs` into five, `harness.rs` and `participant.rs` by section | navigation, about 350 lines |
| A6 | Shared test kit crate (10 copies of `copy_tree`, 15 of `wait_until`, two fake GitHubs, three drifted copies of connector fixtures) | about 1,200 test lines |
| A7 | One contract table of server rules run against the Worker, the local database and Postgres; shrink the Rust stand-in to fault injection | about 1,800 test lines and one copy of the rules |
| A8 | SQL mirror as a schema snapshot instead of 15 migrations with functions defined up to 6 times | about 2,000 lines, a stronger parity test |
| A9 | Connector schema 2: chat-mode defaults, optional identity, pins computed at build time, goldens as "matches the pin" | about 10 lines per harness, fewer merge conflicts |
| A10 | Anthropic decoder on the shared tracker; table-driven decoder tests | about 1,000 lines |
| A11 | OS code (file modes, signals, time zone) into `goalcup-platform` (groundwork for native Windows) | about 300 lines |
| A12 | Let the database pick the event by its window and drop the manual `EVENT_ID` switch | removes a risky manual step |
| A13 | Docs tree by audience; one `docs/TESTING.md` from the three local-run docs; research scripts as one tool | easier to find things |

## Do not simplify

Closed trust tables and their deny rows (D43); compiled pins and the folder/id/digest check; the
redaction design (hold and fail closed, the whole-record oracle, exact-value run secret); the spawn
gate; TLS and CONNECT handling; the deliberate orphan rule in four places; the phase replay and
no-follow file access; the process-handoff state machine; `ProcessOs` and termination authority;
the exit ladders; the v5 corpus; every P1 to P7 promise test.
