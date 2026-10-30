# PRD-15: Docs, AGENTS.md and decisions

Id: PRD-15. Version: goalcup 1.0. Date: 30 October 2026. Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md) (binding).

## Goal

Every doc a participant, an organizer or a contributor reads on release day says what 1.0 does, and
nothing else. About 70 Markdown files and 16,500 lines become 18 files and about 3,000 lines, and
`docs/DECISIONS.md` goes from 57 entries to 24.

## Points served

6 (lead: the participant guide, README and RULES say plainly what is recorded, what is lost and what
each harness tier means), 7 (the harness table, the gateway paragraph and the contributor path),
4 (the masking paragraph says what can still be recorded), 1, 2, 3 and 5 (README maps each point to
the tests that guard it; AGENTS.md makes those tests undeletable while the point stands).

## Owner decisions it carries out

- Replace README's P1 to P7 with the seven points (answer 1, BRIEF2).
- AGENTS.md hard rules stand. Two updates only: the P1 to P7 test rule becomes the seven points, and
  the D65 disk-freeze exception goes. The map changes because the daemon goes (BRIEF3).
- No backward compatibility before Edition 01; old crates deleted in this build; everything in the
  after-Edition-01 list that serves no point moves into this build (BRIEF3), docs included.
- Disk full: no special handling (progress doc, owner decisions of 29 Oct).
- No caps of our own; a limit guards a real cost and says so (VISION, "How we build").
- Masking option b, plus a left word boundary (BRIEF3).
- One system for every harness: a gateway is one more host on the one host list; a dodger gets
  tunnels, session files, a named gap and the tier "not supported", never a hack (BRIEF3, 29 Oct
  night; the relayed request: "isn't that also something we easily solve?" Yes, PRD-00).
- Tiers: "verified" on a named version, "not verified", and "not supported" (BRIEF3, PRD-12).
- Counts: the participant, status, heartbeat and server see only written counts and off time.
- `VISION.md` is the owner's and is not edited (PRD-00 ownership). The owner decisions that today
  live only in a plan file go into `docs/DECISIONS.md` instead.

## Today

| What | Where | Fact |
|---|---|---|
| Doc files | root 8 files, 1,223 lines; `docs/*.md` 16 files, 7,728 (DECISIONS 1,162 of them); `docs/archive` 3, 958; `docs/research` 16 `.md`, 2,545 plus 4 `.txt`, 33; `docs/research/results` 14, 1,294; dated plans 2026-10-18 to 2026-10-29: 13, 2,794 (all `wc -l`) | About 60% is history git already keeps (appendix K, lane summary) |
| AGENTS.md map | `AGENTS.md:32-50` | Names `goalcup-protocol`, `goalcup-daemon`, `goalcup-cli`, `goalcup-verifier`, `goalcup-platform`, pins, eleven commands, "v5 records". All go in 1.0 (PRD-13) |
| AGENTS.md read-first list | `AGENTS.md:24-30` | Sends agents to a 20 Oct plan, a 22 Oct status and the 18 Oct research target |
| The D65 exception | `AGENTS.md:83-86` | "on a nearly full disk Goal Cup Proxy freezes the agent" |
| The pin rule | `AGENTS.md:90-91` | Pins are cut (PRD-11, PRD-13); nothing left to act on |
| The promise-test rule | `AGENTS.md:99-101`, `README.md:276-288` | "Never delete a test in the promises list (P1 to P7)"; P5 is the control API, P2 the v5 corpus: both cut |
| README false or dead lines | `README.md:79-80` (disk pause), `:150-151` ("no header is recorded", false since header streams), `:158-160` (the daemon), `:162-168` (crate table), `:72-73`, `:102` (`goalcup mcp`), `:97-107` (eleven commands), `:109-114` (`--capture-integrity`, `--research-json`) | Every one changes with 1.0 |
| RULES contract lines | `docs/RULES.md:10-11` and `:254` (`goalcup login`), `:16` ("one known gap"), `:42-43` (0.6.1), `:306-321` (labels incl. `wire_undecoded`), `:322-325` (disk), `:341-348` (`[masked]`, `<redacted:KIND>` markers), `:385` (disk types), `:398-413` (`goalcup doctor`, four harnesses), `:485-489` (still to decide) | The contract participants agree to |
| DECISIONS | `docs/DECISIONS.md:1-50` status table, 57 entries `## D1` to `## D74` | Eleven entries are "partly superseded"; one rule (Hermes) needs seven entries (appendix K section 5.1) |
| Decision cites that survive the cut | `server/worker/src/runs.ts` (D64, D70), `server/worker/README.md` (D70), `CONTRIBUTING.md` (D47, D51), `SECURITY.md` (D39, D43, D46, D51, D53, D74), skills templates (PRD-12 rewrites them) | The other about 600 cite lines live in the old crates, which PRD-13 deletes. So the numbers can be cut hard |
| Owner decisions outside DECISIONS | `docs/plans/2026-10-29-record-everything-progress.md:80-100`; BRIEF3 (scratchpad); PRD-00 defaults | No D entry covers masking option b, disk full, no caps, or the 1.0 shape (appendix K finding 1) |
| Paths the old binary links | `crates/goalcup-cli/src/main.rs:59` (`docs/PARTICIPANT-GUIDE.md`), `commands/prep.rs:896` (`docs/HARNESS-SETUP.md`), `:558` (`docs/HARNESS-ONBOARDING.md`) | 1.0's help footer still links the guide (PRD-10 section 11). The other two are linked by 0.7 only, which the Worker refuses after the deploy (PRD-07 section 6) |
| README anchors used elsewhere | `packaging/README.md:63` (`README.md#platforms`), `scripts/README.md:4` (`README.md#the-gate`) | Keep both anchors |
| Link check | `scripts/check-links.py` (1,349 lines), run by `scripts/gate.sh:110` | PRD-13 deletes it last, after this PRD runs it once |
| Run 1 report | `docs/research/results/2026-10-30-two-hour-run.md`, only on `origin/smart/tender-clarke-qupne3` (304 lines; same as the scratchpad copy `run-report-74daaa03.md`) | Cited by PRD-00, 01, 09, 10, 11 by that path; not on main |

## Design

### 1. Rules for every doc in 1.0

1. **True at merge.** A doc changes in the same merge as the code it describes. On the integration
   branch that is phase 3, the joint merge with PRD-13's cut (section 9). The reference guard
   (PRD-13 section 6) greps `README.md` and every kept doc for old crate names, so a doc that still
   says `goalcup-daemon` fails the gate.
2. **One place per fact.** Each fact has one home doc; others link it. The homes are in section 2.
3. **Words on screen come from the code, not the PRDs.** The guide quotes three contract words
   verbatim (`Recording in chat mode`, `shows the code XXXX-XXXX.`, `PROBLEM:`, PRD-10 section 13).
   Every other on-screen line is described by what it means and checked against `src/cli/text.rs`
   once it is merged, never copied from a PRD draft.
4. **Numbers come from records.** Tiers and versions come only from the pad's tier file (section 7,
   ask 4). Run facts come from the run 1 report or `PAD-RESULTS.md`. No number is estimated in a
   participant doc.
5. **Say "model requests written", never "model calls recorded".** The count is a request count
   (PRD-01 section 9; PRD-09 ask of PRD-01 3). "Recording works" appears nowhere.
6. **Plain English, about grade 9, short sentences, no em dashes**, the house style (BRIEF.md).
7. **No history in current docs.** No "since 0.6.1", no "removed by", no fix plan numbers. Git keeps
   history. A doc may name `v0.7.0` once, where a 0.7 tester must act (section 7, appendix C).

### 2. The target doc set

| Reader | Doc | Lines (est.) | Home of |
|---|---|---:|---|
| Participant | `README.md` top half | 120 of 220 | What it is, install, quick start, commands |
| Participant | `docs/PARTICIPANT-GUIDE.md` (path kept: the binary links it) | 380 | Every step and message; the harness table with tiers; gateways; what is and is not recorded |
| Participant | `docs/RULES.md` | 430 | The contract |
| Participant | `SECURITY.md` (ask 1) | 110 | What runs on your machine, trust, what a check does not prove |
| Participant | `skills/goal-cup/SKILL.md` | PRD-10 | The agent's one page |
| Organizer | `docs/OPERATING.md` | 300 | Where things run, deploy, test runs and the pad, fetch and check, stopped runs, usage job, bad days, release |
| Organizer | `docs/UPLOAD-API.md` | PRD-07 | The wire contract |
| Organizer | `server/worker/README.md` | PRD-07 | Worker code, config, limits and storage cost (absorbs `docs/SERVER-DESIGN.md`) |
| Organizer | `packaging/README.md` | PRD-14 | Release steps |
| Contributor | `VISION.md` | owner | The yardstick (not edited) |
| Contributor | `AGENTS.md`, `CLAUDE.md` | 95, 16 | Map, hard rules, tests, done |
| Contributor | `CONTRIBUTING.md` (ask 1) | 95 | PR rules, the gate, adding a harness through the pad, the reviewer checklist |
| Contributor | `docs/ARCHITECTURE.md` | 160 | The one process, data flow, terms, where each point is tested |
| Contributor | `docs/RECORD-FORMAT.md` (new) | 260 | GCR1 for readers, the Python reader, the usage JSON schema |
| Contributor | `docs/CONNECTOR-SCHEMA.md` | 90 | PRD-11 section 2 as a reference page |
| Contributor | `docs/DECISIONS.md` | 260 | 24 current rules, owner decisions in force, retired numbers |
| Contributor | `docs/harnesses/*.md` | PRD-11 | One facts sheet per harness |
| Contributor | `skills/adding-a-capture-harness/**` | PRD-12 | The pad as the one way to add a harness |
| Contributor | `scripts/README.md` (ask 1) | 30 | The kept scripts and the pad |
| Contributor | `TODO.md` | 50 | Open owner questions, known issues, after-the-event list |
| Everyone | `CODE_OF_CONDUCT.md`, `LICENSE`, `NOTICE` | as is | |
| Evidence | `docs/research/results/2026-10-30-two-hour-run.md` (copied as is) | 304 | Run 1, the reason for 1.0 |

Plans stay until after the event: `docs/plans/2026-10-30-cut-first-plan.md`, `docs/plans/2026-10-30-audit/`
and `docs/plans/2026-10-30-v1.0/` (every PRD cites them).

Count: 18 docs in root, `docs/` top level and `docs/research/results/` (8 root; 8 in `docs/` with
PRD-07's UPLOAD-API; the run 1 report; `scripts/README.md`), about 2,950 lines against about 16,500 today in the same folders (UPLOAD-API and
the PRD-owned files left out of both sides).

### 3. Verdict for every doc today

| Doc | Lines | Verdict | Into or why |
|---|---:|---|---|
| `README.md` | 318 | rewrite | Section 5 |
| `AGENTS.md` | 110 | rewrite | Section 4 |
| `CLAUDE.md` | 16 | keep | Already points to AGENTS and VISION and lists the seven points |
| `TODO.md` | 350 | rewrite | Under 60 lines: PRD-00 owner questions with their defaults, asks still open after the night, known issues, after-the-event list (connector contract lock, context dedupe, native Windows, CONNECTOR-SCHEMA into the skill) |
| `SECURITY.md` | 127 | rewrite (ask 1) | Section 10 |
| `CONTRIBUTING.md` | 126 | edit (ask 1) | Section 10 |
| `CODE_OF_CONDUCT.md`, `VISION.md` | 132, 44 | keep | |
| `docs/PARTICIPANT-GUIDE.md` | 330 | rewrite | Section 7 |
| `docs/RULES.md` | 489 | edit | Section 8 |
| `docs/OPERATING.md` | 863 | rewrite | Section 9 |
| `docs/ARCHITECTURE.md` | 350 | rewrite | Section 10 |
| `docs/CONNECTOR-SCHEMA.md` | 1,002 | rewrite | One page from PRD-11 section 2; the compatibility promise, deprecation rule and pin evolution (`:94-159`, `:957-984`) go |
| `docs/DECISIONS.md` | 1,162 | rewrite | Section 6 |
| `docs/HARNESS-SETUP.md` | 468 | merge, delete | Guide section 4. Its anchors were for 0.7 (`prep.rs:896`); 1.0 links nothing there (PRD-11 cuts `setup_section`) |
| `docs/HARNESS-ONBOARDING.md` | 579 | merge, delete | The skill (PRD-12 section 11) and CONTRIBUTING's reviewer checklist |
| `docs/CONTRACTS.md` | 654 | delete | v5 envelope, control API, freeze: all cut. GCR1 is in RECORD-FORMAT, the server contract in UPLOAD-API |
| `docs/TERMINATION.md` | 262 | delete | Termination proof is cut (PRD-02, PRD-10 section 3) |
| `docs/CHEAT-CHECKLIST.md` | 305 | merge, delete | SECURITY "What a check does not prove", rewritten for PRD-08's checks and judge view |
| `docs/SERVER-DESIGN.md` | 164 | merge, delete | `server/worker/README.md` (PRD-07 interface) |
| `docs/RESEARCH-TARGET.md`, `docs/LOCAL-RESEARCH-HANDOFF.md`, `docs/README.md` | 115, 306, 83 | delete | VISION replaced the first; the second marks itself history (`:3`); the README docs map replaces the third |
| `docs/archive/*` | 958 | delete | Archived by their own first lines |
| `docs/research/*.md`, `*.txt` | 2,545, 33 | delete | Plans and handovers of 18 to 28 Oct; `m2-report-format.md` describes the research report PRD-08 cuts; the `.txt` prompts go with `campaign.py` (PRD-13 group D) |
| `docs/research/results/*` (14 files) | 1,294 | delete | Superseded by `PAD-RESULTS.md` (PRD-12) and the run 1 report |
| `docs/plans/2026-10-18-*` to `2026-10-29-*` (13 files) | 2,794 | delete | Done or overruled; the owner decisions of `progress.md:80-100` move into DECISIONS first |
| `docs/research/results/2026-10-30-two-hour-run.md` | 304 | create | Copied with `git show origin/smart/tender-clarke-qupne3:<path>`, the report only, not its evidence folder |

### 4. AGENTS.md for 1.0

The whole file, as the builder writes it (the gate paragraph is PRD-14's text):

```markdown
# Working in this repository

This repository holds Goal Cup Proxy (`goalcup`), the recorder for Goal Cup runs. One process on the
participant's machine records every model request a coding agent's harness makes, through a loopback
proxy, plus the harness's own session files, and writes every loss as a gap with a start and an end.
It uploads the record while the run goes on. `goalcup-verify` checks a record and reads usage from it
later. macOS and Linux; Windows through WSL2. Open work is in [TODO.md](TODO.md).

## The seven points

What we build toward, from [VISION.md](VISION.md). Every change is judged against these.

1. **Record every model call** the harness makes, to any provider, for the whole run.
2. **Record every subagent,** however many and however long.
3. **Never block or slow the harness.** If something cannot be recorded, note a gap and keep going.
4. **Never record secrets.**
5. **Upload everything,** so the server can rebuild the full story by ID and time.
6. **Tell the participant plainly** what was recorded.
7. **Work with the harnesses people actually use,** run the way they use them.

Less code is better. Cut before you add. Ready by Friday 6 November 2026.

Read first: [VISION.md](VISION.md), then [ARCHITECTURE](docs/ARCHITECTURE.md), then
[DECISIONS](docs/DECISIONS.md). While the 1.0 build runs, also the
[PRD index](docs/plans/2026-10-30-v1.0/PRD-00-index.md).

## Map

| Path | What it is |
|---|---|
| `crates/goalcup-format` | The record format (GCR1): records, links, part writer, reader, dump. No async, no TLS. Spec: [RECORD-FORMAT](docs/RECORD-FORMAT.md). |
| `crates/goalcup` | `goalcup`: the seven commands, the watchdog and the recorder process (proxy, session-file reader, masking, writer, uploader) and the connector loader. |
| `crates/goalcup-verify` | `goalcup-verify`: `check`, `dump`, `secrets`, `judge`, `usage`. Every judgement about the bytes lives here. |
| `connectors/` | One `<id>.toml` per harness, and `model-hosts.toml`, the one host list. Data, compiled into the binary. |
| `server/worker` | The upload server, a Cloudflare Worker. It stores bytes and interprets nothing. |
| `scripts/` | The gate, release, `fetch-run.sh` and the test pad (`scripts/pad/`): [scripts/README.md](scripts/README.md). |
| `skills/` | `goal-cup` (copied into the harness home) and `adding-a-capture-harness` (contributors). |
| `docs/` | By reader: the [docs map](README.md#docs-map). Plans: `docs/plans/`. |
| `packaging/` | Release packaging and `install.sh`. |

The dependency rule: the recorder never links the verifier, and the verifier never links the
recorder. They share only `goalcup-format`. A test checks it.

## The gate

(PRD-14's text: the two tiers and what CI runs.)

## Hard rules

- **Never set a model base URL on any harness.** The only wire path is the loopback recording
  proxy, reached through the harness's process-scoped `HTTPS_PROXY` (D1, D74).
- **Goal Cup Proxy records and never manages the agent.** No prompting, no auto "keep going", no
  finish tool. The participant types the prompt and may type only "keep going". goalcup ends the
  harness only at the deadline and on `goalcup seal`. One exception: the agent may ask
  `goalcup time-left` how much time is left (pull only, nothing is pushed) (D66).
- **No harness names in core code.** A harness is data (a connector) plus shared modules.
- **Never write into `~/.claude`, `~/.codex`, `~/.hermes` or any app-wide config.** Goal Cup Proxy
  makes isolated homes. A test that launches a harness uses a private `HOME`.
- **Never commit** anything under `target/`, `harnesses/` or `pad-out/`, a key, or a real run's
  record. A committed fixture is masked and reviewed by eye.
- **Record raw, interpret later.** The recorder captures bytes. The verifier and the usage reader
  judge them. Do not put rule logic in the recorder (D3).

## Tests

One test per behaviour you add or change. Test the promise, not the shape: no golden snapshots of
internal structures, and no assertions on doc text or message wording unless the text is the
contract. Every behaviour that serves one of the seven points has a test, and that test is not
deleted while the point stands. Where they are: [README](README.md#the-seven-points-and-their-tests).

## Done

1. `sh scripts/gate.sh fast` passes. `full` too before a release.
2. One reviewer reads the PR once.
3. `README.md`, the participant guide or `TODO.md` is updated if behaviour changed.

Rust edition 2024, stable toolchain (`rust-toolchain.toml`). Questions about rules or scope go to
the maintainer, Balazs Nemethi, not to a guess.
```

What changed against the hard rules, line by line:

| Today | 1.0 | Kind |
|---|---|---|
| D65 exception (`AGENTS.md:84-86`) | Gone | Owner rule change 2 |
| Promise-test rule (`:99-101`) | Seven points rule | Owner rule change 1 |
| Pin rule (`:90-91`) | Gone | Consequence of the pin cut (PRD-00 question 3, default: delete) |
| `spikes/*/out/`, `spikes/redact.py` (`:92-93`) | `pad-out/`, keys, real records; masked fixture | Consequence of deleting `spikes/` (PRD-13 group E). Same rule, live paths |
| "The daemon captures bytes" (`:94-95`) | "The recorder captures bytes" | PRD-00 names it |
| No sentence on ending the harness | "goalcup ends the harness only at the deadline and on `goalcup seal`" | Today's behaviour written down, because the D65 exception that listed exceptions goes (PRD-10 ask P4, owner to confirm) |
| D1, D36 | D1, D74 | D36 retires into D14 (section 6) |

### 5. README.md for 1.0

Outline, about 220 lines:

1. **Title and five lines.** What `goalcup` records (model requests through a loopback proxy, session
   files, gaps), that it uploads while the run goes on, and that it runs on the participant's machine.
   No harness list here (it is in the guide).
2. **Quick start for participants** (about 40 lines): install, `goalcup setup`, `goalcup start --practice`,
   repository, token and folder, `goalcup start --repo <you>/<repo> --prompt prompt.txt`. Same steps as
   the help footer (PRD-10 section 11). Links to the guide for each step.
3. **During the run and the end** (about 20 lines): server is the clock; `goalcup status`;
   `goalcup continue`; no server error stops a run (D64); the end screen says what was written and
   lost. **Delete** `:79-80` (disk pause) and write: "If the disk fills, the record shows the loss and
   your harness may stop. Free space, then run `goalcup continue`."
4. **Commands**: the seven of PRD-10 section 1, one line each, exit codes 0, 1, 3. Then
   `goalcup-verify` with PRD-08's five subcommands and exit codes 0 to 3.
5. **How it works**: the PRD-00 picture in ASCII (harness, `HTTPS_PROXY`, recorder, one host list,
   tunnels, session files, masker, writer, uploader, Worker). One paragraph on gateways (the owner's
   question): "A gateway is one more HTTPS host. If your harness uses Vercel AI Gateway, OpenRouter or
   Cloudflare AI Gateway, goalcup records it exactly like a direct key. A model host goalcup does not
   know is added once with `goalcup setup --record-host <host>`." One line: headers are recorded with
   credentials masked (fixes the false `:150-151`).
6. **Platforms** (anchor `#platforms` kept): macOS (owner's Mac run pending, said plainly until
   it passes), Linux x86_64 and arm64 (tested in Linux sandboxes), Windows through WSL2. The APFS, ext4
   or XFS rule goes (PRD-10 section 3 cuts the filesystem list).
7. **Add a harness**: five lines and a link to the skill and the pad (PRD-12 section 11).
8. **For developers**: build, the gate (anchor `#the-gate` kept; PRD-14's text), the dependency rule.
9. **The seven points and their tests** (replaces "The promises", `:276-288`), verbatim:

```markdown
### The seven points and their tests

Each point has tests. A test that guards a point is never deleted while the point stands; it may
move tier ([AGENTS.md](AGENTS.md#tests)). The test pad's six checks (C1 to C6,
[PAD-RESULTS](docs/plans/2026-10-30-v1.0/PAD-RESULTS.md)) prove the same points on real harnesses.

| Point | Tests |
|---|---|
| 1. Record every model call | `crates/goalcup/tests/proxy_*.rs`, `tests/lessons_*.rs`; pad C1 (wire requests match the harness's own log) |
| 2. Record every subagent | `tests/native_*.rs`, `tests/pipeline_*.rs` (no caps); pad C2 |
| 3. Never block or slow the harness | `tests/pipeline_*.rs` (memory guards write gaps, the kill test), `tests/proxy_*.rs` (relay first) |
| 4. Never record secrets | `tests/mask_*.rs` and `tests/mask_corpus/`; `goalcup-verify secrets`; pad C3 |
| 5. Upload everything | `tests/upload_*.rs`, `crates/goalcup-format/tests/recovery.rs`, `server/worker/test/`; pad C4 |
| 6. Tell the participant plainly | `tests/cli_*.rs` (screen counts equal the verifier's recount); `goalcup-verify check` count checks; pad C6 |
| 7. Work with the harnesses people use | `tests/connector_*.rs`; the pad table and tiers |
```

10. **Docs map** (anchor `#docs-map`): the reader table of section 2. `docs/README.md` goes.
11. **Support, security and license**: as today (`:309-318`).

### 6. docs/DECISIONS.md for 1.0

**Shape.** Five lines of header (a number never changes and is never reused). Then "Owner decisions
in force" (dated bullets, each pointing to its D entry). Then the 24 current entries, each one to six
lines. Then one "Retired numbers" table that maps every retired number to the entry that holds its
live part, so an old cite in git history still leads somewhere. About 260 lines.

**Owner decisions in force** (moved from `docs/plans/2026-10-29-record-everything-progress.md:80-100`,
BRIEF2, BRIEF3 and PRD-00, dated, with the D entry): less code, cut first (VISION); record everything,
never block (D9, D75); no backward compatibility before Edition 01 (D76); no caps of our own (D78);
disk full gets no special handling (D9); the file path is the session file's ID (D76); byte-for-byte
match of subagent files against disk is not required (D2); masking option b with a left boundary
(D14); no per-run size limit on the server (D78); one process (D75); GCR1 (D76); written counts
only (D77); one system for every harness (D74); tiers by the pad (D79); usage stats by an organizer
job (D80); version 1.0 and the 0.7 floor (D70). Defaults the owner has not answered are listed in
`TODO.md`, not here.

**The 24 current entries.** Existing numbers keep their number with a rewritten rule; new rules
start at D75.

| D | Rule (the whole entry, shortened here) | Built from | Cited by after 1.0 |
|---|---|---|---|
| D1 | Never set or change a harness's model base URL. Record through a loopback proxy the harness reaches by its own `HTTPS_PROXY`. The run CA is trusted only through the harness's own variables, never the system store. | D1, D36 (part), D59, D72 (the trust entry goes) | AGENTS, new code |
| D2 | Two legs: the wire and the harness's own session files. Neither erases a gap in the other. A connector may have no session rows. Session files are not compared byte for byte with the disk. | D2, D74 (part), owner O5 | |
| D3 | Record raw, interpret later. The recorder captures, masks, writes and uploads, and judges nothing. The verifier and the usage reader judge. A check judges the record, not the run. The server stores bytes and interprets nothing. | D3, D45, PRD-07 section 1 | AGENTS |
| D5 | The agent never sees its own record: `GOALCUP_HOME` and the project folder never contain each other. | D5 | |
| D9 | Every loss is a gap with a start, an end, a reason and the lost bytes when known. No loss is silent. Load never turns recording off, and nothing counts failures while the recorder answers. A full disk gets no special handling: the loss is a `write_error` gap once writing works again, and the harness may stop. | D9, D58, D65 (retired), D37 clause 3 (retired), run 1 handover items 1 to 3 | |
| D10 | One harness per entry. Its subagents are free and unlimited; a second harness is a rules matter, never stopped by goalcup. | D10 | RULES |
| D14 | Masking happens before any byte reaches disk. Every secret rule is kept; the URL password rule is added; the email and Codex rate-limit rules are dropped; a key needs a left word boundary. A masked byte becomes `*`, same length, with a MASK record. The run's own secrets and the project's `.env` values are masked by exact value. What can still be recorded is said in the guide. | D14, D36, D60, D41 (`masked_key`), owner option b, PRD-05 | PRD-05 code |
| D16 | The server fixes the clock: the run's start and deadline come from the server, and every chunk gets the server's `received_at`. Nothing is signed. | D16 without the receipt, D63 (retired) | |
| D37 | Runs are autonomous; the record is research; a new harness is data; the participant's machine is single-user. | D37 clauses 1, 2, 4, 5 | |
| D42 | Harnesses are data, compiled into the binary. Anyone may send a connector PR; review is the control. The `start` event names the connector bytes (`connector_sha256`). No pins, no deny lists, no freeze. Apache-2.0. | D42, D43 (rulings 1, 3, 5, 7), D46, D51, D53, D68 (retired) | CONTRIBUTING, SECURITY |
| D47 | No DCO sign-off; the PR author says where AI helped; code-owner review stays. | D47 | CONTRIBUTING |
| D52 | Run the harness the way people run it: chat mode, the participant's own login, no model, effort, sandbox or turn limit set by goalcup. goalcup writes the harness's no-prompt settings into its own isolated harness home after one question, never into app-wide config. | D52, D57 (chat mode), D67 reversed by PRD-10 ask P1 (owner to confirm), D38, D39 (home parts) | |
| D61 | Goal Cup holds no keys. It never reads, stores or passes a provider key. The harness keeps its own login in goalcup's isolated home. | D61, D69 | |
| D64 | No server error stops a run or its upload; only `stop` in a 200 answer or the local deadline ends it. | D64 | `server/worker/src/runs.ts` |
| D66 | Nothing is pushed to the agent. It may pull `goalcup time-left`. goalcup ends the harness only at the deadline and on `goalcup seal` (SIGTERM, then SIGKILL after 10 s); `goalcup seal` refuses inside the harness. | D25, D66, D22 (the end only), PRD-10 section 3 | AGENTS |
| D70 | The release is the list. A connector ships by release. The Worker refuses clients below `MIN_COMPETITION_VERSION` (1.0.0 at release) in every mode. | D70, PRD-07 section 6 | Worker |
| D71 | The project folder is the workspace. The agent pushes with the participant's one-repository token. The seal records the workspace's Git HEAD and a digest of uncommitted changes. | D71, D11, D56 | |
| D74 | One host list. Shared model hosts (providers, gateways, cloud model services, vendor backends), plus hosts the participant registers, plus hosts the harness config names, are decrypted and recorded. Every other connection is a tunnel recorded with host, bytes and times. Sign-in hosts are never decrypted. A client that pins or refuses the run CA falls back to a tunnel with a named gap. | D74, D62, PRD-03, PRD-11 | AGENTS, SECURITY |
| D75 | One process records everything: proxy, session-file reader, masker, writer and uploader. A small watchdog holds the listen port, starts the harness and restarts a dead recorder at once. No IPC, no producers, no failure budget. After repeated failures the recorder runs in null mode and says so. | PRD-00, PRD-02 (its ask of PRD-15), run 1 | PRD-02 code |
| D76 | The record format is GCR1: binary framing, raw bytes, one stream per WebSocket direction, one link per flush, zstd with a 2^27 window written at record time. The part file is the upload unit. The session file's path is its ID. No reader for older formats. | PRD-01, D54, D55 (retired) | goalcup-format |
| D77 | Counts are written counts. The participant, status, heartbeat and server see only counts of bytes already written, and the off time. No screen says "recording works". | PRD-01 section 9, PRD-10, run 1 finding 4 | |
| D78 | No caps of our own. Only stated memory guards, which write a gap and go on. No cap on streams, files, subagents, connections or a run's size. Each kept limit names its cost: the 32 MiB chunk (one request's retry cost), the device-flow rate limiter (the one unauthenticated route), the reader's 16 MiB record bound (a corrupt file cannot allocate wildly). | VISION, owner answers 2 and 3, PRD-02, PRD-07 section 5, PRD-13 section 7 | |
| D79 | Harness tiers come from the test pad: "verified on <version>", "not verified", or "not supported". A harness that ignores `HTTPS_PROXY` or the run CA is "not supported" and is never hacked around. A tier says how well goalcup records it, not whether a run is eligible (ask 6). | PRD-12 section 9, BRIEF3 | Guide, RULES |
| D80 | Usage stats: an organizer job reads the record and the harness's own files and writes tokens, models and calls per call, per subagent and per run as JSON in R2. Organizers see them. There is no token budget; plan and billing fields are stripped before research is published. | D26, PRD-09 | |

**Retired** (39): D4, D7, D11, D12, D22, D23, D24, D25, D26, D30, D36, D38, D39, D40, D41, D43,
D44, D45, D46, D48, D49, D50, D51, D53, D54, D55, D56, D57, D58, D59, D60, D62, D63, D65, D67, D68,
D69, D72, D73. Each row of the "Retired numbers" table says "into Dn" or "gone with <part>", for
example D4 into D75, D22 into D66 (the end) and gone with the termination proof, D24 gone with the
control API, D65 gone (disk full: no special handling), D68 gone (no backward compatibility; may
return after the event). Hermes version notes from D61 and D72 move to `docs/harnesses/hermes-cli.md`
(PRD-11 interface).

PRD-02 asked for "one process, one guard, restart by watchdog, null mode" (D75), PRD-05 for "same-length
`*` masking, the left guard, exact `.env` values" (D14). Both are in.

### 7. The participant guide for 1.0 (`docs/PARTICIPANT-GUIDE.md`)

Path kept (`main.rs:59` today, PRD-10 section 11 in 1.0). Outline, about 380 lines:

| Section | What it says | Source |
|---|---|---|
| What goalcup does for you | Five lines: records your harness's model requests and session files, uploads them, never slows your agent, hides secrets before saving, tells you exactly what it wrote and what it lost | Seven points |
| 1. What you need | Agent Community account; a harness on the list; a key or a login; macOS, Linux or WSL2; a GitHub account for the real run | `RULES.md` |
| 2. Install | One command, the PATH line, `goalcup --version`; one line on firewalls such as Little Snitch (asked once per binary, again after updates) | PRD-10 section 7, PRD-14 |
| 3. Set up | `goalcup setup`: pick the harness; sign in inside goalcup's own harness home; the no-prompt question and what "yes" changes; machine warnings; `Ready.`. How to use a gateway: configure it inside goalcup's home, nothing else to do. A model host goalcup does not know: `goalcup setup --record-host <host>`, and what is refused (an IP address, a sign-in host, a wide pattern) | PRD-10 sections 6, 7; PRD-11 |
| 4. Your harness: the list and the tiers | The three tiers in plain words, the table below, then the general setup notes from today's HARNESS-SETUP sections 1 to 7 (no prompts, push, turn limits, sign-in lifetime, subagents and their models, tools on PATH, first-run screens) and one paragraph on auto-update: goalcup turns it off where a switch exists; for Hermes, Gemini CLI and Qwen Code set it off in the harness's settings inside goalcup's home; an update matters only at the next `goalcup continue`, and the record names the version at every start | PRD-11 section 7, PRD-12 section 9 |
| 5. A practice run | `goalcup start --practice`, about 10 minutes, no GitHub, never counts | PRD-10 |
| 6. Your run | Repository, token, folder (`.gitignore`, `.env.local`), `--prompt FILE`, the approval code, the one first message on the clipboard and in `first-message.txt`, "keep going" only | PRD-10 sections 3, 8 |
| 7. During the run | The status block line by line; the recording states (on, restarting, off, losing data, behind, finished) and what each asks of you; the problem lines; `goalcup continue`; `goalcup time-left`; sleep and the sleep lock line | PRD-10 sections 4, 9, 10 |
| 8. The end | The end screen line by line (written requests by host, requests with missing parts, session files, off time, hosts that passed through unread, upload), what is removed from the computer and why; delete the token, log out or rotate keys; the videos | PRD-10 section 5, PRD-06 section 10 |
| 9. What is recorded, and what is not | PRD-05 section 12's paragraph verbatim; tunnels (host, bytes and times only); vendor backends (the harness's traffic to its vendor, not the vendor's own model calls); dodgers; HTTP/3 and QUIC never reach a proxy (PRD-03 line on ECH and HTTP/3); final database snapshots; your email address and plan and credit balance are recorded | PRD-05, PRD-03, PRD-04 |
| 10. Help | Support address; paste `goalcup status` and the run id, never a token; `goalcup.log` path | PRD-10 section 12 |
| Appendix A: start refusals | Each refusal and its fix, from `src/cli/text.rs` | PRD-10 section 13 |
| Appendix B: gap reasons in plain words | PRD-10 section 5 table | PRD-01 section 3 |
| Appendix C: if you tested 0.7 | Reinstall; `goalcup login`, `doctor`, `entries` and `mcp` are gone; remove a `goalcup mcp` line from your harness config (a 1.0 harness shows a failed MCP server and goes on); `GOALCUP_CONFIG_DIR` is now `GOALCUP_HOME`; 0.7 runs cannot be continued | PRD-10 sections 2, 11 |

**The tiers, as the guide says them:**

- **Verified on `<version>`**: the test pad ran this harness at that version, with a direct key and,
  where it has one, a gateway, and every check passed: model requests matched the harness's own log,
  every subagent was recorded, no secret was found, and the server copy was whole.
- **Not verified**: goalcup records what reaches it, the same way, but the pad has not passed on this
  harness yet. `goalcup status` tells you if model requests are missing.
- **Not supported**: the harness ignores goalcup's proxy or refuses its certificate. goalcup records
  its session files and the connections it can see, not its model calls. You can still take part
  (RULES, ask 6).

**The harness table.** One row per connector. The tier and version columns are copied from the tier
file at the end of phase 5 (ask 4); before that every row says "not verified". "Know this" is the one
fact a participant must act on, from PRD-11 section 7.

| Harness | `goalcup` id | Sign in | Gateway | Know this |
|---|---|---|---|---|
| Claude Code | `claude-code-cli` | Key or subscription | `ANTHROPIC_BASE_URL` in goalcup's home | goalcup refuses to start if `settings.json` there sets a proxy |
| Codex | `codex-cli` | Key or ChatGPT sign-in | A custom provider in goalcup's home | Do not put a proxy or CA in `$CODEX_HOME/.env` |
| Hermes | `hermes-cli` | Keys in its `.env`, or Nous sign-in | Its built-in gateway providers | goalcup stops a running Hermes gateway before the start |
| OpenCode 1 and 2 | `opencode-cli`, `opencode2-cli` | Key or OpenCode account | Its own gateway or a provider | goalcup stops OpenCode 2's background service first |
| OpenClaw | `openclaw-cli` | Keys in its `.env` | Provider plugins | Leave its own proxy setting empty; subagents unmeasured |
| Amp | `amp-cli` | Amp account | Not needed | Amp's model calls run on Amp's servers; goalcup records what Amp sends there |
| GitHub Copilot CLI | `copilot-cli` | GitHub sign-in or your own key | Your own key | `/delegate` runs in GitHub's cloud and is not allowed (RULES) |
| Gemini CLI | `gemini-cli` | Paid Gemini key | Not needed | Turn auto-update off in its settings |
| Goose | `goose-cli` | Key | A custom provider | |
| Kilo CLI | `kilo-cli` | Key or Kilo account | Vercel AI Gateway or Kilo's gateway | |
| Qwen Code | `qwen-cli` | Key | Not needed | Leave its own `proxy` setting empty; its built-in `claude-code` and `codex` subagents start another harness (RULES) |
| Crush | `crush-cli` | Key | Not needed | |
| Factory Droid | `droid-cli` | Factory account plus your key | Your own key | Cloud computers are not allowed (RULES) |
| Kimi Code | `kimi-code-cli` | Kimi sign-in or key | A provider in its config | |
| Cline CLI | `cline-cli` | Key or Cline account | Cline's provider | A Cline hub already running may bypass goalcup; quit it first |
| Cursor CLI | none | | | Not supported: no connector |

Every row is checked against PRD-11's connector and the pad row at phase 5. A "Know this" line that
the pad shows to be wrong is removed, not hedged.

### 8. RULES.md edits

| Line | Today | 1.0 |
|---|---|---|
| `:10-11`, `:254` | Two approvals; `goalcup login` | One approval per run (PRD-10 cuts login) |
| `:16` | "one known gap" | One sentence: secrets are masked by shape and by exact value; the guide lists what can still be recorded (PRD-05 section 12) |
| `:42-43` | 0.6.1 dry run | Delete |
| `:112-113` | `goalcup time-left` (D66) | Kept as is; D66 keeps its number |
| `:306-321` | Four labels including `wire_undecoded`; D74 | The judge view's labels `wire_and_native`, `wire_only`, `native_only`, `none` and its coverage line (PRD-08 section 5). Admissibility text unchanged |
| `:322-325` | Full disk is a red flag; the disk pause (D65) | "If the disk fills, the record shows a gap and the harness may stop. Judges read the gap like any other." |
| `:335-339` | Upload every 30 minutes; one hour after the stop | The cadence PRD-06 ships (default every 5 minutes); the after-stop rule from PRD-06 |
| `:341-348` | `[masked]`, `<redacted:KIND>` markers | Stars of the same length; the PRD-05 list of what is still recorded |
| `:385` | APFS, ext4 or XFS required | Delete (PRD-10 cuts the filesystem list) |
| `:398-413` | `goalcup doctor --harness all`; four connectors | The 15 harnesses and their tiers (link to guide section 4); `/delegate`, Droid cloud computers and Qwen's harness-starting subagents named under "Other agents" and cloud modes |
| `:485-489` | Still to decide | Move to `TODO.md` |

RULES is the participant's contract. Every edit that changes what a participant may do (the three
cloud modes named, the tier sentence) needs the owner's yes (ask 6). Edits that only follow cut code
do not.

### 9. OPERATING.md for 1.0

Keep the runbook, drop the binary reference that 1.0 makes small. About 300 lines:

1. **Where things run** (`docs/OPERATING.md:25-42`, kept): add the `goalcup-usage` bucket (PRD-07
   section 10) and the separate R2 bucket for test runs (BRIEF3, 30 Oct early).
2. **Dates and checklist**: release on 6 Nov, the deploy order, the `EVENT_ID` switch.
3. **Deploy**: PRD-07 section 12 step by step: staging first, `MIN_COMPETITION_VERSION = "1.0.0"`, tag,
   production, then delete `RECEIPT_SIGNING_KEY`; no rollback to the 0.7 Worker; `MAINTENANCE=on`.
4. **Test runs and the pad**: replaces "The matrix" (`:154-434`, 281 lines) with ten lines that point
   to `scripts/pad/` and `PAD-RESULTS.md` (PRD-12 interface).
5. **Fetch and check a run**: `scripts/fetch-run.sh`, then `goalcup-verify check --server`, `judge`,
   `secrets` (PRD-07 section 7, PRD-08 section 1).
6. **Runs whose recording stopped**: PRD-07 section 8's query and what to do.
7. **The usage job**: how to run it and where the JSON lands (PRD-09 section 7).
8. **Storage and the manual lever**: PRD-07 section 5's cost table and the one hand write.
9. **Bad days**: kept and updated; the case "disk nearly full" (`:457`) goes.
10. **Releasing**: PRD-14's text.

Cut: "Running the daemon" (`:571-610`), "Running a capture with the CLI" (`:611-655`, now the README
quick start), "Running the verifier" (`:656-730`, now PRD-08's five subcommands in README),
"Pinning connectors" (`:731-746`) and "Finding codes" (`:747-863`, replaced by PRD-08's
`check --json` report, documented in RECORD-FORMAT's last section).

### 10. Contributor docs

- **`docs/ARCHITECTURE.md`**, about 160 lines: (1) what it is for (the seven points); (2) the three
  crates and the dependency rule; (3) the one process, with PRD-00's diagram and the rules of the
  shape (relay first, mask before disk, one writer, the watchdog); (4) the one host list, tunnels,
  gateways and dodgers (PRD-00 "One system"); (5) session files; (6) masking; (7) the record and the
  upload, pointing to RECORD-FORMAT and UPLOAD-API; (8) terms: run, entry, boot, part (the participant
  word), part file, stream, gap, off span, link, seal, written count, tier; (9) where each point is
  tested (the README table); (10) the lessons list (`docs/plans/2026-10-30-v1.0/LESSONS.md`, PRD-17).
- **`docs/RECORD-FORMAT.md`** (new), about 260 lines, written from PRD-01 once its asks settle
  (ask of PRD-01 2): files on disk, framing, records, stream kinds, IDs, time, gaps and off spans,
  masks, counts, links, the seal, compression and fetch, invariants, the worked example, and a
  Python reader of about 60 lines that reads the decompressed stream from standard input
  (`cat rec-*.gcr.zst | zstd -dc --long=27 | python3 read.py`), so it needs only the standard
  library. Last two sections: the `goalcup-verify check --json` report (PRD-08 S6) and the usage JSON
  schema (PRD-09 section 7, its ask 2: "or a section of `docs/RECORD-FORMAT.md`").
- **`docs/CONNECTOR-SCHEMA.md`**: PRD-11 section 2's table, the host list rules of its section 4, one
  example connector, and how `--connector` and `--hosts` load a file. About 90 lines.
- **`CONTRIBUTING.md`** (ask 1): the gate paragraph from PRD-14; "Adding a harness" (`:76-117`)
  becomes the pad path (PRD-12 section 11) in ten lines plus a short reviewer checklist moved from
  `HARNESS-ONBOARDING.md`: the connector sets no base URL, no proxy or CA variable outside
  `ca_env`, `env` holds no key, every host added to `model-hosts.toml` has a vendor doc link, the
  pad row is attached. D51 becomes D42. `spikes/redact.py` goes.
- **`SECURITY.md`** (ask 1), about 110 lines: supported version (1.0 only); what runs on your machine
  (one recorder process and its watchdog, a loopback proxy, a run CA trusted only through the
  harness's own variables, isolated harness homes, no keys held, the run credential at 0600, the
  upload); trust boundaries (no control API; same-user processes are inside the boundary; nothing is
  signed, so the organizers trust the server copy and its `received_at` times); connectors are trusted
  data compiled into the binary, and a file connector is named in the record; "What a check does not
  prove" (from `docs/CHEAT-CHECKLIST.md`, only the rows still true: masking gaps, length visible,
  tunnels unread, vendor backends, the harness's own log read only for harnesses with a reader, local
  parts can be changed before upload within one flush interval).
- **`scripts/README.md`** (ask 1): the kept scripts (PRD-13 "Kept scripts") and `scripts/pad/`.
- **`TODO.md`**: under 60 lines (section 3).

## Files it owns

Change: `README.md`, `AGENTS.md`, `TODO.md`, `docs/PARTICIPANT-GUIDE.md`, `docs/RULES.md`,
`docs/OPERATING.md`, `docs/ARCHITECTURE.md`, `docs/CONNECTOR-SCHEMA.md`, `docs/DECISIONS.md`.
With ask 1: `SECURITY.md`, `CONTRIBUTING.md`, `scripts/README.md`.

Create: `docs/RECORD-FORMAT.md`; `docs/research/results/2026-10-30-two-hour-run.md` (copied as is).

Delete (with ask 1 for the folders outside `docs/*.md`): `docs/README.md`, `docs/CONTRACTS.md`,
`docs/TERMINATION.md`, `docs/CHEAT-CHECKLIST.md`, `docs/SERVER-DESIGN.md` (after PRD-07 confirms its
facts are in the Worker README), `docs/HARNESS-ONBOARDING.md` (after PRD-12's skill lands),
`docs/HARNESS-SETUP.md`, `docs/RESEARCH-TARGET.md`, `docs/LOCAL-RESEARCH-HANDOFF.md`,
`docs/archive/` (3 files), `docs/research/` (16 `.md`, 4 `.txt`) and `docs/research/results/`
(14 files), `docs/plans/2026-10-1*.md` and `docs/plans/2026-10-2*.md` (13 files).

No change: `CLAUDE.md`, `VISION.md` (the owner's), `CODE_OF_CONDUCT.md`, `LICENSE`, `NOTICE`.

Must not touch: `docs/UPLOAD-API.md`, `server/**` (PRD-07); `connectors/**`, `docs/harnesses/**`
(PRD-11); `skills/goal-cup/**` (PRD-10); `skills/adding-a-capture-harness/**`, `scripts/pad/**`,
`PAD-RESULTS.md` (PRD-12); `packaging/**`, `.github/**`, `scripts/gate.sh` (PRD-14); every crate;
`docs/plans/2026-10-30-cut-first-plan.md`, `docs/plans/2026-10-30-audit/**` and every other PRD file;
`LESSONS.md` (PRD-17); `NIGHT-LOG.md` (PRD-16).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-00 | Ownership asks; owner questions 3 and the new ones (asks); definition of done item 6 | Asks |
| PRD-01 | `docs/RECORD-FORMAT.md` is written from it; the term "part" | Used here; asks |
| PRD-02 | D75 text; `status.json` modes for the guide's state table; memory guard names and values for D78 | Used here |
| PRD-03 | Host list behaviour, tunnel notes, the `never` list, the HTTP/3 and ECH line | Used here |
| PRD-04 | Session file rows, final snapshots, the SQLite WAL note, "files cut at the seal" | Used here |
| PRD-05 | Section 12 paragraph into the guide and README; `RULES.md:16`; D14 | Used here |
| PRD-06 | Upload lines (section 10) and cadence for RULES and the guide | Used here |
| PRD-07 | `server/worker/README.md` absorbs SERVER-DESIGN (they confirm before the delete); deploy order, stopped-runs query and storage cost for OPERATING; PRD-07's list of stale docs (OPERATING, SERVER-DESIGN, CHEAT-CHECKLIST) is done here | Both |
| PRD-08 | Commands, exit codes, the judge view labels for RULES; the `check --json` report for RECORD-FORMAT; the CHEAT-CHECKLIST rewrite in SECURITY | Used here |
| PRD-09 | Usage JSON schema as a RECORD-FORMAT section; the organizer job for OPERATING | Used here |
| PRD-10 | Guide sections 3 to 10 of its design, the message list, the 0.7 leftover line, the firewall line, the guide path in the help footer, D66 wording (its ask P4), D52 (its ask P1) | Used here |
| PRD-11 | CONNECTOR-SCHEMA from its section 2; harness facts from its section 7; Hermes version notes from D61 and D72 into `docs/harnesses/hermes-cli.md` | Both |
| PRD-12 | The skill replaces HARNESS-ONBOARDING; the tier file and `PAD-RESULTS.md` feed the guide table and README's test table; OPERATING points to the pad | Used here |
| PRD-13 | Joint merge in phase 3; `check-links.py` run here once before PRD-13 deletes it; the reference guard; its list of docs that describe deleted code | Both |
| PRD-14 | Gate text for README, AGENTS and CONTRIBUTING; release text for OPERATING; CODEOWNERS entries for docs | Used here |
| PRD-16 | Merge timing (phase 3 and the tier update after phase 5); the night log | Used here |
| PRD-17 | `LESSONS.md` linked from ARCHITECTURE | Used here |

## What is cut

| Path | Lines | Note |
|---|---:|---|
| Nine `docs/*.md` files (section 3) | 2,936 | `CONTRACTS` 654, `HARNESS-ONBOARDING` 579, `HARNESS-SETUP` 468, `LOCAL-RESEARCH-HANDOFF` 306, `CHEAT-CHECKLIST` 305, `TERMINATION` 262, `SERVER-DESIGN` 164, `RESEARCH-TARGET` 115, `README` 83 |
| `docs/archive/` | 958 | 3 files |
| `docs/research/` (not `results/`) | 2,578 | 16 `.md`, 4 `.txt` |
| `docs/research/results/` | 1,294 | 14 files |
| Dated plans 18 to 29 Oct | 2,794 | 13 files |
| Shrunk in place | about 3,000 | DECISIONS 1,162 to 260, CONNECTOR-SCHEMA 1,002 to 90, OPERATING 863 to 300, TODO 350 to 50, ARCHITECTURE 350 to 160, README 318 to 220, RULES 489 to 430, AGENTS 110 to 95 |
| **Total** | about **13,600 out, 600 in** | The guide grows by about 50 lines (it absorbs HARNESS-SETUP and the tiers); RECORD-FORMAT and the run 1 report are new |

Tests that go: none in this PRD. `scripts/check-links.py` and `scripts/check-version.py` with their
tests are PRD-13's (group D).

## Acceptance

Real-run proofs first. Each runs on the integration branch after phase 2, on a Linux sandbox, by a
watcher that has not read the PRDs.

1. **A newcomer completes a practice run from the docs alone.** Given only `README.md` and
   `docs/PARTICIPANT-GUIDE.md`, the watcher installs the 1.0 build, runs `goalcup setup` and a
   practice run against the staging Worker, and writes down every step it had to guess and every line
   on screen the guide does not explain. Pass: zero guesses; zero unexplained lines; the three
   contract words match byte for byte.
2. **Every line of the core-set end screens is explained.** For the Claude Code, Codex and Hermes pad
   rows, each line of the saved end screen and each recording state seen in `status.jsonl` maps to a
   guide line or the gap reasons table. Pass: zero unmapped lines.
3. **A contributor adds a harness from the docs alone.** Given only `CONTRIBUTING.md`, the skill and
   `docs/CONNECTOR-SCHEMA.md`, a second watcher writes a connector for one non-core harness from the
   vendor's docs and gets a pad `smoke` row. Pass: `row.json` exists, the connector loads, and the
   watcher read no Rust.
4. **An organizer follows OPERATING.** A watcher fetches a staging pad run and runs every command in
   OPERATING sections 5 to 7 as written. Pass: each command runs unchanged and exits as the doc says.
5. **The format doc reads a real record.** The Python reader in `docs/RECORD-FORMAT.md` runs on a
   core-set pad entry. Pass: its record count per kind equals `goalcup-verify dump`'s.
6. **AGENTS.md briefs a fresh agent.** An agent that reads only `AGENTS.md` answers: which crate
   masks; may the recorder link the verifier; may a test that guards point 4 be deleted; what goalcup
   does on a full disk; where the host list lives; may goalcup set a base URL. Pass: six of six.

Automated, run once at the phase 3 merge (no new gate code):

- `python3 scripts/check-links.py --root .` passes on the final docs, before PRD-13 deletes it.
- PRD-13's reference guard prints nothing.
- A stale-term grep over the kept docs prints nothing outside `docs/plans/` and DECISIONS'
  "Retired numbers" table: `goalcup-daemon`, `goalcup-cli`, `goalcup-protocol`, `goalcup-verifier`,
  `goalcup-platform`, `goalcup login`, `goalcup doctor`, `goalcup mcp`, `goalcup entries`, `prep.json`,
  `--capture-integrity`, `--research-json`, `pin-manifests`, `receipt`, `SIGSTOP`, `paused`,
  `Recording works`, `protocol v5`, `P1 to P7`, `D65`.
- Every `goalcup <command>` in README, the guide, RULES and OPERATING is one of PRD-10's seven, and
  every `goalcup-verify <subcommand>` one of PRD-08's five.
- Every `D<number>` cited in surviving code, connectors, server, skills and docs is a current entry.
- A grep for the em dash character (U+2014) over the files this PRD writes prints nothing.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Docs written from PRDs that change overnight | The guide quotes a line PRD-10 changed at 3 a.m. | Participant text is written last (phase 2), from merged `src/cli/text.rs`; only the contract words are quoted |
| A tier printed before the pad ran | A participant trusts "verified" for an untested harness | Tiers only from the tier file; default "not verified"; acceptance 2 |
| Deleting history | A later agent misses why a rule exists | Git keeps it; DECISIONS maps every retired number; the run 1 report and the audit stay |
| 0.7 links break | 0.7 testers clicking `HARNESS-SETUP.md` or `HARNESS-ONBOARDING.md` get 404 | 0.7 is refused after the deploy; guide appendix C tells testers to reinstall |
| No link check after the build | Links rot after PRD-13 deletes `check-links.py` | Accepted (PRD-13); the run here is the last one |
| RULES is a contract | Changing eligibility wording without the owner changes the deal participants signed | Only rule-following edits go in by default; eligibility and cloud-mode lines wait for the owner (ask 6) |
| The guide grows with 15 harnesses | A long table nobody reads | One line per harness; detail stays in `docs/harnesses/` |
| "Part" means two things | A participant reads "Parts: 2" and a contributor reads "part file" | Glossary in ARCHITECTURE; ask of PRD-01 1 |
| PRD-10 and PRD-11 disagree on connector fields | The guide's setup section describes a no-prompt step that does not exist | Ask 7; the guide follows what merges |
| Owner questions unanswered | Docs show defaults as facts | Each default is named as a default in TODO; DECISIONS holds only answered decisions |
| macOS untested in this build | README says macOS works | README says "macOS: the owner's Mac run is pending" until it passes |
| Copying the run 1 report | It names the old crates, so the reference guard would flag it | It lives under `docs/research/results/`; ask PRD-13 to exclude that one file, or keep it only on its branch (ask 9) |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| History docs | Deleted (git keeps them) | Owner (ask 2) |
| HARNESS-SETUP | Merged into the guide | PRD-15 |
| CHEAT-CHECKLIST | Merged into SECURITY | PRD-15 with PRD-08 |
| Usage schema | A section of RECORD-FORMAT, not its own file | PRD-15 with PRD-09 |
| Decision numbers | Old numbers kept where the rule lives on; new ones from D75 | PRD-15 |
| Tiers and eligibility | A tier is not an eligibility rule | Owner (ask 6) |
| Run 1 report | Copied to main as one file, no evidence folder | Owner (ask 9) |
| TODO size | Under 60 lines | PRD-15 |

## Asks of PRD-00 and PRD-01

PRD-00:

1. **Ownership gaps.** `SECURITY.md`, `CONTRIBUTING.md` and `scripts/README.md` have no owner; give them
   to PRD-15 (PRD-13 ask 2 agrees on `scripts/README.md`). Say that PRD-15's `docs/*.md` includes
   `docs/archive/**`, `docs/research/**` and the dated plans before 30 Oct. `CODE_OF_CONDUCT.md`,
   `LICENSE` and `NOTICE` are not edited.
2. **History docs.** PRD-13 section 7 keeps `docs/plans/**` and `docs/archive/**` "as history". This
   PRD deletes `docs/archive/` and the dated plans before 30 Oct. Default: delete. The guard skips them
   either way, so this is only about what the next agent reads.
3. **Merge timing.** Add to the build order: PRD-15's rewrite merges in the joint phase 3 merge with
   PRD-13 (the reference guard needs it), and the tier columns land in a second small merge after
   phase 5.
4. **One tier file.** PRD-12 ask 5 leaves the tier source open (PRD-11 copies `pad-out/tiers.json`).
   Name one file, for example `connectors/tiers.toml`, as the single source for the start line, the
   guide table and README. The guide copies from it; it never edits tiers by hand.
5. **Owner decisions go to DECISIONS, not VISION.** Appendix K proposed a "Decided" list in VISION;
   PRD-00 says VISION is not edited. Confirm DECISIONS' "Owner decisions in force" is the home.
6. **New owner question.** Does a tier decide eligibility? Default: no. A "not verified" or "not
   supported" harness may take part; RULES' existing label rule decides admissibility. Also confirm
   naming Copilot `/delegate`, Droid cloud computers and Qwen's harness-starting subagents as not
   allowed in RULES.
7. **PRD-10 and PRD-11 disagree.** PRD-10 reads `no_prompts`, `first_screen`, `warn_missing`, a
   vendor-backend flag and a tier from connector data (its interfaces row PRD-11); PRD-11 lists
   `no_prompts`, `first_screen` and `warn_missing` as "fields with no successor" and its schema has
   none of them. PRD-12 ask 3 also needs `no_prompts`. Rule which wins; the guide's setup section
   depends on it. Default: PRD-10's needs win.
8. **File connectors in the competition.** D70 said `--dev` never competes. PRD-11 replaces it with
   `--connector <file>` and `--hosts <file>` and no dev mode. Rule whether a competition run may use
   them. Default: refused in competition mode, allowed in practice and test runs; RULES and D70 say so.
9. **The run 1 report on main.** Five PRDs cite `docs/research/results/2026-10-30-two-hour-run.md`,
   which is only on `origin/smart/tender-clarke-qupne3`. Default: PRD-15 copies the report file to
   main; PRD-13 adds that path to the reference guard's exclusions, since the report names old crates.
10. **Question 2 has an answer.** BRIEF3 (30 Oct early) lets agents deploy the Worker to staging;
    PRD-00 question 2 and PRD-07 section 12 step 1 still say nothing is deployed overnight. OPERATING
    needs one deploy story.
11. **Command list.** PRD-00's PRD table and `src/cli/` line still name `login`; PRD-10 cuts it (its
    ask A4). The docs follow PRD-10's seven commands.

PRD-01:

1. **"Part" means two things.** PRD-01 calls a `rec-*.gcr.zst` file a part; PRD-10's status and today's
   README call one `start` or `continue` a part (`README.md:91-92`, PRD-10 section 4 "Parts: 2").
   Default here: "part" is the participant's word for an entry; PRD-01's prose says "part file". Please
   use "part file" in sections 1 and 12.
2. **A settled revision.** `docs/RECORD-FORMAT.md` is written from PRD-01 plus its accepted asks (PRD-08
   asks 1 to 8, PRD-09 asks 1 to 4, PRD-10 asks A1 to A3, PRD-11 and PRD-12 asks on the `start` event).
   Write a night-log line when they are settled (target H+4), so the doc is not written against a
   moving spec.
3. **The Python reader.** Section 2 promises a 60-line reader. Confirm it reads the decompressed stream
   from standard input, so it needs no third-party zstd package.
4. **Gap reason 12.** PRD-02 wants `mask_failed`, PRD-03 wants `upstream_untrusted` (PRD-08 ask of
   PRD-01 8). The guide's plain-words table needs the final numbers.

## Builder notes

Order of work:

1. **H+1.5, from the PRDs:** `docs/DECISIONS.md` (section 6), `AGENTS.md` (section 4), README's seven
   points table (section 5). These depend only on decisions already taken.
2. **H+4, after PRD-01's asks settle:** `docs/RECORD-FORMAT.md`, `docs/ARCHITECTURE.md`,
   `docs/CONNECTOR-SCHEMA.md` (after PRD-11's schema merges).
3. **Phase 2, from merged code:** the guide, RULES, README's commands, OPERATING, SECURITY,
   CONTRIBUTING, `scripts/README.md`, TODO. Read `src/cli/text.rs`, `goalcup --help` and
   `goalcup-verify --help` from the build; never from a PRD draft.
4. **Before the phase 3 merge:** the deletes, then `check-links.py` once, the reference guard, the
   stale-term grep and the D-cite check. Merge jointly with PRD-13 (and PRD-14's gate edits).
5. **After phase 5:** copy tiers and versions from the tier file into the guide table; one small merge.

Traps from the audit:

- Keep `docs/PARTICIPANT-GUIDE.md` and the README anchors `#platforms` and `#the-gate` (section Today).
- Never name an old crate in a kept doc; the reference guard fails the gate (PRD-13 section 6).
- Never write "model calls recorded"; write "model requests written" (rule 5).
- The run 1 report says what happened in 0.7; do not quote its numbers as 1.0 numbers.
- The 2-hour run of 1.0 has not happened. No doc cites its results until its report exists.
- Never write a key, a key file path or a token in a doc; name secrets only (`docs/OPERATING.md:9-10`).
- No em dashes. Check with the grep in Acceptance.

How a watcher proves it in a real run: run acceptance 1 on the core set's first green build, and
acceptance 2 on the three core-set pad rows. Write every unexplained line into the night log with the
screen it came from; the fix is a guide edit, or a `text.rs` edit by PRD-10 when the screen is wrong.
