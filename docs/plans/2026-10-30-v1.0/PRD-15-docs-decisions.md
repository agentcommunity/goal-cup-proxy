# PRD-15: Docs, AGENTS.md and decisions

Id: PRD-15. Version: goalcup 1.0. Revision 3, 30 October 2026 (the alignment pass against the ledger;
revision 2: earlier the same day; revision 1: 29 October). Base: main at 4b226de2. Index:
[PRD-00](PRD-00-index.md) revision 4. Format: [PRD-01](PRD-01-record-format.md) revision 2 (binding).
The ledger, [OWNER-DECISIONS.md](OWNER-DECISIONS.md), wins over this file where they differ.

Revision 2 answered the three reviews of 30 October (the consistency check, review A, review B) in
"Review answers", now history. Revision 3 answers the alignment fix lists in "Alignment answers", the
last section.

## Goal

Every doc a participant, an organizer or a contributor reads on release day says what 1.0 does, and
nothing else. In this build: about 20 live docs rewritten or written, five dead docs moved to
`docs/archive/`, and twelve new DECISIONS entries. The history purge and the DECISIONS rewrite from
57 entries to 24 wait until after the merge to main (PRD-00 "Not in this build").

## Points served

6 (lead: the participant guide, README and RULES say plainly what is recorded, what is lost and what
each harness tier means), 7 (the link to the one harness table, the gateway paragraph and the
contributor path),
4 (the masking paragraph says what can still be recorded), 1, 2, 3 and 5 (README maps each point to
the tests that guard it; AGENTS.md makes those tests undeletable while the point stands).

## Owner decisions it carries out

- Replace README's P1 to P7 with the seven points (answer 1, BRIEF2).
- AGENTS.md hard rules stand, with the changes ledger section 7 lists: the P1 to P7 rule becomes the
  seven points, D65 goes, the pin rule goes with the pins, and "daemon" becomes "recorder". The map
  changes because the daemon goes (BRIEF3, ledger section 7).
- No backward compatibility before Edition 01; old crates deleted in this build (ledger L3, L7).
- Disk full: no special handling (progress doc, owner decisions of 29 Oct; ledger section 7).
- No caps of our own; a limit guards a real cost and says so (VISION, ledger R5).
- Masking option b, plus a left word boundary (ledger M1). Exact-value masking beyond today's
  `GH_TOKEN`, and test keys and webhook secrets, only with the owner's yes (PRD-00 question 3, default
  no).
- One system for every harness: a gateway is one more host on the one host list; a dodger gets
  tunnels, session files, a named gap and, if it ignores the proxy, the tier "not supported", never a
  hack (ledger H1 to H5). The relayed question "isn't that also something we easily solve?" is
  answered yes in README and the guide (section 5, PRD-11 "The owner's question").
- Tiers: "verified" on a named version, "not verified", and "not supported" (ledger T3, PRD-12).
- Counts: the participant, status, heartbeat and server see only written counts and off time (R6).
- `VISION.md` is the owner's and is not edited. Owner decisions live in `docs/DECISIONS.md`
  ("Owner decisions in force").
- PRD-00 rulings (made in revision 2, still in force): history docs are kept in this build; PRD-15's
  docs are in the third slice and land as soon as each is green, not in the phase 3 merge; only the
  newcomer acceptance run happens in this build; owner questions live in PRD-00 alone (this PRD cites
  their numbers).
- Question numbers are PRD-00 revision 4's (twelve questions; PRD-00 maps the old numbers 1 to 19 to
  them). "Review answers" at the end still uses the old numbers.

## Today

| What | Where | Fact |
|---|---|---|
| Doc files | root 8 files, 1,223 lines; `docs/*.md` 16 files, 7,728 (DECISIONS 1,162 of them); `docs/archive` 3, 958; `docs/research` 16 `.md`, 2,545 plus 4 `.txt`, 33; `docs/research/results` 14, 1,294; dated plans 2026-10-18 to 2026-10-29: 13, 2,794 (all `wc -l`) | About 60% is history git already keeps (appendix K, lane summary) |
| Kept Markdown that names an old crate | `git grep -c` with PRD-13's guard pattern and exclusions: `README.md` 8, `AGENTS.md` 5, `CONTRIBUTING.md` 4, `SECURITY.md` 2, `TODO.md` 3, `docs/ARCHITECTURE.md` 51, `docs/CONNECTOR-SCHEMA.md` 13, `docs/OPERATING.md` 18, `docs/CONTRACTS.md` 23, `docs/HARNESS-ONBOARDING.md` 11, `docs/CHEAT-CHECKLIST.md` 3, `docs/TERMINATION.md` 3, `docs/SERVER-DESIGN.md` 2 (also PRD-07, PRD-11, PRD-12, PRD-14 files) | Every one must be rewritten or moved before the reference guard runs after the cut (PRD-13 section 6) |
| AGENTS.md map | `AGENTS.md:32-50` | Names `goalcup-protocol`, `goalcup-daemon`, `goalcup-cli`, `goalcup-verifier`, `goalcup-platform`, pins, eleven commands, "v5 records". All go in 1.0 (PRD-13) |
| AGENTS.md read-first list | `AGENTS.md:24-30`, `:60-67` | Sends agents to a 20 Oct plan, a 22 Oct status, the 18 Oct research target, CONTRACTS and TERMINATION |
| The D65 exception | `AGENTS.md:83-86` | "on a nearly full disk Goal Cup Proxy freezes the agent" |
| The pin rule | `AGENTS.md:90-91` | Pins are cut (PRD-11, PRD-13); nothing left to act on |
| The promise-test rule | `AGENTS.md:99-101`, `README.md:276-288` | "Never delete a test in the promises list (P1 to P7)"; P5 is the control API, P2 the v5 corpus: both cut |
| README false or dead lines | `README.md:79-80` (disk pause), `:150-151` ("no header is recorded", false since header streams), `:158-160` (the daemon), `:162-168` (crate table), `:72-73`, `:102` (`goalcup mcp`), `:97-107` (eleven commands), `:109-114` (`--capture-integrity`, `--research-json`) | Every one changes with 1.0 |
| RULES contract lines | `docs/RULES.md:10-11` and `:254` (`goalcup login`), `:16` ("one known gap"), `:42-43` (0.6.1), `:306-321` (labels incl. `wire_undecoded`), `:322-325` (disk), `:332-335` (upload), `:336-348` (`[masked]`, `<redacted:KIND>` markers), `:385` (disk types), `:398-413` (`goalcup doctor`, four harnesses), `:485-489` (still to decide) | The contract participants agree to |
| DECISIONS | `docs/DECISIONS.md:1-50` status table, 57 entries `## D1` to `## D74` | The file's own rule (`:3-6`, `:15`): add an entry when a rule changes and update the status table in the same change. No D entry covers masking option b, disk full, no caps, or the 1.0 shape (appendix K finding 1) |
| Owner decisions outside DECISIONS | `docs/plans/2026-10-29-record-everything-progress.md:80-100`; the ledger `OWNER-DECISIONS.md`; PRD-00 owner questions | Plan files, not the decision record |
| Paths the old binary links | `crates/goalcup-cli/src/main.rs:59` (`docs/PARTICIPANT-GUIDE.md`), `commands/prep.rs:896` (`docs/HARNESS-SETUP.md`), `:558` (`docs/HARNESS-ONBOARDING.md`) | 1.0's help footer still links the guide (PRD-10 section 11). The other two are linked by 0.7 only, which the Worker refuses after the floor is set (PRD-00 question 5) |
| README anchors used elsewhere | `packaging/README.md:63` (`README.md#platforms`), `scripts/README.md:4` (`README.md#the-gate`) | Keep both anchors |
| Link check | `scripts/check-links.py` (1,349 lines), run by `scripts/gate.sh:110` in the full tier only. It checks every tracked `.md` (`check-links.py:1-19`), history docs included, and prints `file:line: target (reason)` (`:1343`) | After the cut, history docs that link into deleted crates break by the hundreds. A run over the whole tree can never pass; only the lines from the files this PRD writes count (Acceptance) |
| Run 1 report | `docs/research/results/2026-10-30-two-hour-run.md`, only on `origin/smart/tender-clarke-qupne3` (304 lines; same as the scratchpad copy `run-report-74daaa03.md`) | Cited by PRD-00, 01, 09, 10, 11 by that path; not on main. The reference guard excludes `docs/research/**` (PRD-13 section 6) |

## Design

### 1. Rules for every doc in 1.0

1. **True when it merges, and as soon as it can be.** A doc lands on the integration branch in the
   first merge after the code it describes is there, as soon as it is green (PRD-00 phase 3 row). No
   PRD-15 file waits for the phase 3 merge, which holds only PRD-13's deletes. A doc that still names
   an old crate fails the reference guard after the cut (PRD-13 section 6).
2. **One place per fact.** Each fact has one home doc; others link it. The homes are in section 2.
3. **Words on screen come from the code, not the PRDs.** The guide quotes three contract words
   verbatim (`Recording in chat mode`, `shows the code XXXX-XXXX.`, `PROBLEM:`, PRD-10). Every other
   on-screen line is described by what it means and checked against `src/cli/text.rs` once it is
   merged, never copied from a PRD draft.
4. **Numbers come from records.** Tiers and versions come only from `connectors/tiers.toml` (PRD-12
   section 9). Run facts come from the run 1 report or `PAD-RESULTS.md`. Limits come from the code
   that holds them. No number is estimated in a participant doc.
5. **Say "model requests written", never "model calls recorded".** The count is a request count
   (PRD-01 section 9). "Recording works" appears nowhere.
6. **Plain English, about grade 9, short sentences, no em dashes**, the house style (BRIEF.md).
7. **No history in current docs.** No "since 0.6.1", no "removed by", no fix plan numbers. Git keeps
   history. No current doc has a section for 0.7 testers (ledger L7); one line in the 1.0.0 release
   notes covers them (section 9 item 10).
8. **Words for files.** On every participant page, one `start` or `continue` is a "part" and the
   compressed files are "record files" (PRD-10 section 1). Contributor pages say "part file" for a
   `rec-*.gcr.zst` file and "entry" for a part (PRD-01). The ARCHITECTURE glossary maps both.

### 2. The target doc set

| Reader | Doc | Lines (est.) | Home of |
|---|---|---:|---|
| Participant | `README.md` top half | 120 of 220 | What it is, install, quick start, commands |
| Participant | `docs/PARTICIPANT-GUIDE.md` (path kept: the binary links it) | 400 | Every step and message; a link to `docs/harnesses/README.md`; gateways; what is and is not recorded |
| Participant | `docs/RULES.md` | 430 | The contract |
| Participant | `SECURITY.md` | 110 | What runs on your machine, trust, what a check does not prove |
| Participant | `skills/goal-cup/SKILL.md` | PRD-10 | The agent's one page |
| Organizer | `docs/OPERATING.md` | 280 | Where things run, deploy, test runs and the pad, fetch and check, the usage job, bad days, release |
| Organizer | `docs/UPLOAD-API.md` | PRD-07 | The wire contract |
| Organizer | `server/worker/README.md` | PRD-07 | Worker code, config, limits and storage cost (absorbs `docs/SERVER-DESIGN.md`) |
| Organizer | `packaging/README.md` | PRD-14 | Release steps |
| Contributor | `VISION.md` | owner | The yardstick (not edited) |
| Contributor | `AGENTS.md`, `CLAUDE.md` | 95, 16 | Map, hard rules, tests, done |
| Contributor | `CONTRIBUTING.md` | 95 | PR rules, the gate, adding a harness through the pad, the reviewer checklist |
| Contributor | `docs/ARCHITECTURE.md` | 160 | The one process, data flow, terms, where each point is tested |
| Contributor | `docs/RECORD-FORMAT.md` (new) | 280 | GCR1 for readers, the Python reader, the `check --json` report, the usage JSON |
| Contributor | `docs/CONNECTOR-SCHEMA.md` | 90 | PRD-11 section 2 as a reference page |
| Contributor | `docs/DECISIONS.md` | 1,162 plus about 130 | Today's entries unchanged, the status table updated, owner decisions in force, D75 to D86 |
| Participant, contributor | `docs/harnesses/README.md`, `docs/harnesses/*.md` | PRD-11 | The one harness table (tier columns generated from `connectors/tiers.toml`); one facts sheet per harness |
| Contributor | `skills/adding-a-capture-harness/**` | PRD-12 | The pad as the one way to add a harness |
| Contributor | `docs/HARNESS-ONBOARDING.md` | 5 | A pointer to the skill (PRD-12 section 11) |
| Contributor | `scripts/README.md` | 30 | The kept scripts and the pad |
| Contributor | `TODO.md` | 60 | Open owner questions by PRD-00 number, known issues, the after-the-merge list |
| Everyone | `CODE_OF_CONDUCT.md`, `LICENSE`, `NOTICE` | as is | |
| Evidence | `docs/research/results/2026-10-30-two-hour-run.md` (copied as is) | 304 | Run 1, the reason for 1.0 |

Kept as history in this build, untouched: `docs/archive/**`, `docs/research/**`, the dated plans
before 30 Oct, `docs/RESEARCH-TARGET.md`, `docs/LOCAL-RESEARCH-HANDOFF.md`, the 30 Oct plan, audit and
this PRD set. PRD-09, PRD-12, PRD-16 and PRD-17 cite three of them as live evidence
(`docs/research/results/2026-10-vercel-gateway.md`, `docs/research/VERCEL-COMPAT.md`,
`docs/research/m2-report-format.md`). The reference guard skips all of them.

### 3. Verdict for every doc today

| Doc | Lines | Verdict in this build | Into or why |
|---|---:|---|---|
| `README.md` | 318 | rewrite | Section 5 |
| `AGENTS.md` | 110 | rewrite | Section 4 |
| `CLAUDE.md` | 16 | keep | Already points to AGENTS and VISION and lists the seven points |
| `TODO.md` | 350 | rewrite | Under 60 lines: the PRD-00 owner questions still open (number and default only), known issues, the after-the-merge list (section 9 of this PRD's builder notes) |
| `SECURITY.md` | 127 | rewrite | Section 10 |
| `CONTRIBUTING.md` | 126 | edit | Section 10 |
| `CODE_OF_CONDUCT.md`, `VISION.md` | 132, 44 | keep | |
| `docs/PARTICIPANT-GUIDE.md` | 330 | rewrite | Section 7 |
| `docs/RULES.md` | 489 | edit | Section 8 |
| `docs/OPERATING.md` | 863 | rewrite | Section 9 |
| `docs/ARCHITECTURE.md` | 350 | rewrite | Section 10 |
| `docs/CONNECTOR-SCHEMA.md` | 1,002 | rewrite | One page from PRD-11 section 2; the compatibility promise, deprecation rule and pin evolution (`:94-159`, `:957-984`) go |
| `docs/DECISIONS.md` | 1,162 | append | Section 6. The rewrite to 24 entries is after the merge |
| `docs/HARNESS-SETUP.md` | 468 | merge, then move to `docs/archive/` | Guide section 4. 1.0 links nothing there (PRD-11 cuts `setup_section`) |
| `docs/HARNESS-ONBOARDING.md` | 579 | shrink to a 5-line pointer | To the skill (PRD-12 section 11); the reviewer checklist moves to CONTRIBUTING |
| `docs/CONTRACTS.md` | 654 | move to `docs/archive/` | v5 envelope, control API, freeze: all cut. GCR1 is in RECORD-FORMAT, the server contract in UPLOAD-API |
| `docs/TERMINATION.md` | 262 | move to `docs/archive/` | Termination proof is cut (PRD-02, PRD-10) |
| `docs/CHEAT-CHECKLIST.md` | 305 | merge, then move to `docs/archive/` | SECURITY "What a check does not prove", rewritten for PRD-08's checks, the S6 `label` and the coverage line |
| `docs/SERVER-DESIGN.md` | 164 | merge, then move to `docs/archive/` | `server/worker/README.md`, after PRD-07 confirms its facts are there |
| `docs/README.md` | 83 | delete | README's docs map replaces it; its links point at moved files |
| `docs/RESEARCH-TARGET.md`, `docs/LOCAL-RESEARCH-HANDOFF.md` | 115, 306 | keep (history) | Neither names an old crate; purge after the merge |
| `docs/archive/*`, `docs/research/**`, dated plans before 30 Oct | 958, 2,578, 1,294, 2,794 | keep (history) | PRD-00 "Not in this build"; purge after the merge |
| `docs/research/results/2026-10-30-two-hour-run.md` | 304 | create | Copied with `git show origin/smart/tender-clarke-qupne3:<path>`, the report only, not its evidence folder |

Moving a dead doc to `docs/archive/` (`git mv`, no edit) keeps it readable as history and outside the
reference guard, without the purge PRD-00 defers. Its first line is not edited; the folder says it
is history.

### 4. AGENTS.md for 1.0

The whole file, as the builder writes it (the gate paragraph is PRD-14's text):

```markdown
# Working in this repository

This repository holds Goal Cup Proxy (`goalcup`), the recorder for Goal Cup runs. One process on the
participant's machine records every model request a coding agent's harness makes, through a loopback
proxy, plus the harness's own session files, and writes every loss as a gap with a start and an end.
It uploads the record while the run goes on. `goalcup-verify` checks a record and reads usage from it
later. Linux; macOS pending the owner's Mac run; Windows through WSL2. Open work is in
[TODO.md](TODO.md).

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
[DECISIONS](docs/DECISIONS.md) from "Owner decisions in force". While the 1.0 build runs, also the
[ledger](docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md), which wins over every PRD, then the
[PRD index](docs/plans/2026-10-30-v1.0/PRD-00-index.md).

## Map

| Path | What it is |
|---|---|
| `crates/goalcup-format` | The record format (GCR1): records, links, part writer, reader, dump. No async, no TLS. Spec: [RECORD-FORMAT](docs/RECORD-FORMAT.md). |
| `crates/goalcup` | `goalcup`: the seven commands, the watchdog, the recorder process (proxy, session-file reader, masking, writer, uploader) and the connector loader. |
| `crates/goalcup-verify` | `goalcup-verify`: `check`, `dump`, `secrets`, `usage`. Every judgement about the bytes lives here. |
| `connectors/` | One `<id>.toml` per harness, `model-hosts.toml` (the one host list) and `tiers.toml` (written by the test pad). Data, compiled into the binary. |
| `server/worker` | The upload server, a Cloudflare Worker. It stores bytes and interprets nothing. |
| `scripts/` | The gate, release, `fetch-run.sh` and the test pad (`scripts/pad/`): [scripts/README.md](scripts/README.md). |
| `skills/` | `goal-cup` (copied into the harness home) and `adding-a-capture-harness` (contributors). |
| `docs/` | By reader: the [docs map](README.md#docs-map). Plans: `docs/plans/`. History: `docs/archive/`, `docs/research/`. |
| `packaging/` | Release packaging and `install.sh`. |

The dependency rule: the recorder never links the verifier, and the verifier never links the
recorder. They share only `goalcup-format`. A test checks it.

Core code is `crates/goalcup` and `crates/goalcup-format`. The usage reader in `crates/goalcup-verify`
reads harness session formats after the run, chosen by connector id.

## The gate

(PRD-14's text: the two tiers and what CI runs.)

## Hard rules

- **Never set a model base URL on any harness.** The only wire path is the loopback recording
  proxy, reached through the harness's process-scoped `HTTPS_PROXY` (D1, D81). The test pad
  configures a harness the way a participant would (for example pointing it at DeepSeek or a gateway,
  in goalcup's isolated home); goalcup's own launch code never does.
- **Goal Cup Proxy records and never manages the agent.** No prompting, no auto "keep going", no
  finish tool. The participant types the prompt and may type only "keep going". One exception: D66,
  the agent may ask `goalcup time-left` how much time is left (pull only, nothing is pushed). A full
  disk gets no special handling: the recorder restarts, then runs in null mode, and the harness keeps
  working (D78).
- **No harness names in core code.** A harness is data (a connector) plus shared modules.
- **Never write into `~/.claude`, `~/.codex`, `~/.hermes` or any app-wide config.** Goal Cup Proxy
  makes isolated homes. A test that launches a harness uses a private `HOME`.
- **Never commit** anything under the repository root's `/pad-out/` or `/harnesses/`, or any
  `target/`, a key, or a real run's record. A committed fixture is masked and reviewed by eye.
- **Record raw, interpret later.** The recorder captures bytes. The verifier and the usage reader
  judge them. Do not put rule logic in the recorder (D3).

## Tests

One test per behaviour you add or change. Test the promise, not the shape: no golden snapshots of
internal structures, and no assertions on doc text or message wording unless the text is the
contract. Every behaviour that serves one of the seven points has a test, and that test is not
deleted while the point stands, or is replaced in the same change by a test with the same name prefix
that asserts the same promise. Where they are: [README](README.md#the-seven-points-and-their-tests).

## Done

1. `sh scripts/gate.sh fast` passes. `full` too before a release.
2. At least two independent reviewers read each diff during the 1.0 build (ledger B2); after it, one
   reviewer reads the PR once.
3. `README.md`, the participant guide or `TODO.md` is updated if behaviour changed.

Rust edition 2024, stable toolchain (`rust-toolchain.toml`). Questions about rules or scope go to
the maintainer, Balazs Nemethi, not to a guess.
```

What changed against the hard rules, line by line:

| Today | 1.0 | Kind |
|---|---|---|
| D65 exception (`AGENTS.md:84-86`) | Gone; "Two exceptions" becomes "One exception"; one sentence says a full disk gets no special handling (D78) | Owner rule change 2 |
| Promise-test rule (`:99-101`) | Seven points rule, in PRD-00's words | Owner rule change 1 |
| Pin rule (`:90-91`) | Gone | Consequence of the pin cut (ledger section 7, L6; default taken in PRD-00) |
| `spikes/*/out/`, `spikes/redact.py` (`:92-93`) | Root `/pad-out/` beside root `/harnesses/` and any `target/` (root-anchored, so `docs/harnesses/` stays committable); keys and real records named; masked fixture | Consequence of deleting `spikes/` (PRD-13 phase 3 step 2). Same rule, live paths. The wording is on the owner list (G3-O4, default yes) |
| "The daemon captures bytes" (`:94-95`) | "The recorder captures bytes"; the usage reader judges too | Ledger section 7 wording |
| D1, D36 | D1, D81; one sentence says the pad configures a harness as a participant would and goalcup never does | D81 is the one host list (section 6); D36's text stays as history. The sentence is ledger section 7 row 1 |

Ending the harness at the deadline and on `goalcup seal` is today's behaviour and stays (PRD-00
question 2, old 11). It is written in D85 and the guide, not in the hard-rule text, because ledger
section 7 changes only the rows it lists and changes no other hard rule.

### 5. README.md for 1.0

Outline, about 220 lines:

1. **Title and five lines.** What `goalcup` records (model requests through a loopback proxy, session
   files, gaps), that it uploads while the run goes on, and that it runs on the participant's machine.
   No harness list here (it is in `docs/harnesses/README.md`, PRD-11).
2. **Quick start for participants** (about 40 lines): install, `goalcup setup`, `goalcup start --practice`,
   repository, token and folder, `goalcup start --repo <you>/<repo> --prompt prompt.txt`. Same steps as
   the help footer (PRD-10 section 11). Links to the guide for each step.
3. **During the run and the end** (about 20 lines): server is the clock; `goalcup status`;
   `goalcup continue`; no server error stops a run (D64); the end screen says what was written and
   lost. **Delete** `:79-80` (disk pause) and write: "If the disk fills, goalcup cannot write. Your
   agent keeps working, `goalcup status` says recording is off, and goalcup retries by itself. Free
   space and recording starts again. The time it was off is a gap in the record." (PRD-02 section 5.)
4. **Commands**: the seven of PRD-10 section 1, one line each, exit codes 0, 1, 3. Then
   `goalcup-verify` with PRD-08's four subcommands (`check`, `dump`, `secrets`, `usage`) and exit
   codes 0 to 3.
5. **How it works**: the PRD-00 picture in ASCII (harness, `HTTPS_PROXY`, recorder, one host list,
   tunnels, session files, masker, writer, uploader, Worker). One line: headers are recorded with
   credentials masked (fixes the false `:150-151`). Then the gateway paragraph (the owner's question):

   > A gateway is one more HTTPS address. If your harness uses Vercel AI Gateway, OpenRouter or
   > Cloudflare AI Gateway, goalcup records it exactly like a direct key: same proxy, same record. A
   > model host goalcup does not know is added once with `goalcup setup --record-host <host>`. Some
   > harnesses talk only to their vendor (Amp, Copilot with GitHub's models): goalcup records what
   > the harness sends to the vendor; the vendor's own model calls happen on its servers. A harness
   > that ignores goalcup's proxy is "not supported": goalcup records its session files only, not its
   > model calls, and says so.

6. **Platforms** (anchor `#platforms` kept): macOS (the owner's Mac run is pending, said plainly until
   it passes), Linux x86_64 (tested in sandboxes), Linux arm64 built, not yet tested, Windows through
   WSL2. The APFS, ext4 or XFS rule goes (PRD-10 cuts the filesystem list).
7. **Add a harness**: five lines and a link to the skill and the pad (PRD-12 section 11).
8. **For developers**: build, the gate (anchor `#the-gate` kept; PRD-14's text with its measured
   times), the dependency rule.
9. **The seven points and their tests** (replaces "The promises", `:276-288`):

```markdown
### The seven points and their tests

Each point has tests. A test that guards a point is never deleted while the point stands; it may
move tier ([AGENTS.md](AGENTS.md#tests)). The test pad's six checks (C1 to C6,
[PAD-RESULTS](docs/plans/2026-10-30-v1.0/PAD-RESULTS.md)) prove the same points on real harnesses.

| Point | Real-run proof | Guard tests |
|---|---|---|
(rows: PRD-14 section 2, one row per point plus "Format" and "All together")
```

   The rows are PRD-14 section 2's guard list, copied when the guards merge, with every file name
   checked against the tree (`ls crates/*/tests/`). A name that does not exist is fixed with its
   owner, never left in the table.
10. **Docs map** (anchor `#docs-map`): the reader table of section 2.
11. **Support, security and license**: as today (`:309-318`).

### 6. docs/DECISIONS.md in this build

The file keeps every entry as it stands today. Four changes, all appended or one-word table edits,
so nothing conflicts with other lanes and every old cite still resolves:

1. **Header** (`:1-9`): one sentence: "From goalcup 1.0, read 'Owner decisions in force' and D75 to
   D86 first; they replace the clauses the status table names."
2. **Owner decisions in force**, a new section after the header, dated bullets, each pointing to its
   entry: 29 Oct (progress doc `:80-100`): less code, cut first (VISION); record everything, never
   block (D78); no caps of our own (D79); disk full gets no special handling (D78); the file path is
   a session file's ID and session files are not compared byte for byte with the disk (D76); masking
   option b (D80). 29 Oct night (ledger): goalcup 1.0, fresh core, old crates deleted, no backward
   compatibility (D84); one process (D75); GCR1 (D76); written counts (D77); one system for every
   harness, gateways included (D81); tiers by the pad (D82); usage stats after upload (D83). Answered
   or defaulted PRD-00 owner questions enter only with their answer: ending the harness (question 2,
   D85), no-prompt settings (question 2, D86), cut login and file connectors (questions 2 and 4,
   D84), tier and eligibility (question 4, D82), exact-value masking (question 3, D80), the
   watchdog's jobs (question 9, D75), the left guard class (question 10, D80), the test-only build
   feature (question 12, D84). A default the owner has not confirmed is marked "(default, PRD-00
   question N)". The open list is in `TODO.md`.
3. **D75 to D86**, appended, each one to eight lines:

| D | Rule (the entry, shortened here) | Status table rows it changes | Asked by |
|---|---|---|---|
| D75 | One goalcup process records everything: proxy, session-file reader, masker, writer, uploader. The `start` or `continue` process launches and ends the harness (D85) and is also the watchdog: its only recording job is to restart a dead recorder at once on the same port with the same CA (ledger R3, R3a). Beside that it holds the listen port, mints the run CA once per entry and runs the connector's helper stops; it never touches a recorded byte (default, PRD-00 question 9). No producers, no shared queue, no failure budget. Only a crash loop (3 crashes within 10 minutes, each within 60 s of its start) puts the recorder in null mode: it relays, records nothing, says so and retries a normal boot after 30 s, 1, 2, then every 5 minutes, never ending; the count resets once a recorder stays up 60 s (ledger R3a). If the watchdog itself died, `goalcup recover` restarts only the recorder on the same port with the same CA and writes a gap for the time it was down; the harness never restarts. For that, the run CA key is kept in a 0600 file during the entry and removed at the seal (ledger R3b). Core dumps are off and a panic writes only file, line and fixed text. Every timer runs on boot time | D4 superseded; D58 superseded | PRD-02, PRD-00 done item 6 |
| D76 | The record format is GCR1: binary framing, raw bytes, one stream per WebSocket direction, one link per flush, zstd with a 2^27 window written at record time. The part file is the upload unit. A session file's path is its ID. Session files are not compared byte for byte with the disk. No reader for older formats. Upload: a chunk every 5 minutes or when 16 MiB waits; retry backoff resets when the server's offset moves and is capped short; a local part file is pruned once the server confirms it (PRD-06) | D53, D54, D55 superseded; D2 partly (the byte-for-byte clause); D64 partly (its backoff clause) | PRD-01, PRD-00 done item 6 |
| D77 | Counts are written counts. The participant, status, heartbeat and server see only counts of bytes already written, and how long recording was off. No screen says "recording works". The heartbeat says `recording: failed` while recording is off; mail about it is out of scope | none | PRD-01 section 9, PRD-10 |
| D78 | Every loss is a gap with a start, an end, a reason and the lost bytes when known. No loss is silent. Load never turns recording off, and nothing counts failures while the recorder answers. A full disk gets no special handling: a write error ends the recorder, the watchdog restarts it, after repeated failures it runs in null mode, status says recording is off, and the next good boot's `recorder_down` gap covers the time. The harness keeps working | D65 superseded; D9 partly; D64 (its D65 clause) | Run 1 handover items 1 to 4; PRD-02 section 5 |
| D79 | No caps of our own. No cap on streams, files, subagents, connections or a run's size. A kept limit guards a real cost and names it where it is set (PRD-02 memory guards, PRD-07 section 5). A memory guard never stops recording: past it, a gap, and recording goes on | none (D58 is in D75) | VISION, owner answers 2 and 3 |
| D80 | Masking happens before any byte reaches disk. Every secret rule stays; the URL password rule is added; the email and Codex rate-limit rules go; a key needs a left word boundary (with `jwt`): a guarded rule is dropped when the byte before it is a letter, a digit or `_` (PRD-00 question 10; `-` is added only with the owner's yes). A masked byte becomes `*`, same length, with a MASK record. A fail-closed span is named and counted. The run's `GH_TOKEN` is masked by exact value, as today. Exact-value masking of the other sources of PRD-05 section 9, and widening the Stripe rule to test keys and webhook secrets, are built only with the owner's yes (PRD-00 question 3, default no); with a yes this entry says so. What can still be recorded is in the guide | D14, D36, D60 partly; D41 (its key-fragment rule) | PRD-05 |
| D81 | One host list for every harness. Shared model hosts (providers, gateways, cloud model services, vendor backends), hosts the participant registers and hosts the harness config names are decrypted and recorded. Every other connection is a tunnel recorded with host, bytes and times, and a long or large tunnel to one host is flagged. Sign-in hosts are never decrypted. A client that pins or refuses the run CA falls back to a tunnel with a named gap. A harness that ignores `HTTPS_PROXY` or the run CA is never hacked around. A model server on loopback is not recorded and is named (`local_model`). The run CA is trusted only through the harness's own variables | D1 partly (the "hosts the connector declares" clause); D62, D74 partly | PRD-03, PRD-11 |
| D82 | Harness tiers come from the test pad: "verified on `<version>`", "not verified" with a reason, or "not supported". `connectors/tiers.toml` is the one source. A tier says how well goalcup records a harness, not whether a run is eligible (default, PRD-00 question 4) | none (D23 already retired) | PRD-12 |
| D83 | Usage stats are read after upload: `goalcup-verify usage` reads the record and the harness's own session files and writes tokens, models and calls per call, per subagent and per run. An organizer job stores them as JSON in R2. Organizers see them (default, PRD-00 question 1). The recorder never parses for stats | D26 partly | PRD-09 |
| D84 | What 1.0 removes: signing and receipts, pins, deny lists, the connector contract freeze, the termination proof, the control API, `goalcup login`, `doctor`, `mcp` and `--dev`. No backward compatibility before Edition 01. Connectors are compiled in; `--connector` and `--hosts` load a file in practice and test runs only (default, PRD-00 question 4). The Worker refuses clients below `MIN_COMPETITION_VERSION` in every mode; production gets 1.0.0 after the 1.0.0 release is published (default, PRD-00 question 5). Also removed: research decoders except the model and usage parts PRD-09 keeps, research scripts, the separate daemon (D75), disk-full handling (D78), legacy and test seams (one test-only build feature stays, never in a release build: default, PRD-00 question 12), retired versions | D16 partly (the receipt); D24, D30, D63, D68 superseded; D42, D43, D70 partly | PRD-00 "Not in this build", PRD-07, PRD-10, PRD-13 |
| D85 | goalcup ends the harness only at the deadline, on `goalcup seal` and on the server's `stop`: SIGTERM to the harness tree, SIGKILL after 10 s. `goalcup seal` refuses inside the harness. There is no termination proof (default, PRD-00 question 2) | D22 partly; D66 unchanged | PRD-02 section 8, PRD-10 |
| D86 | `goalcup setup` always writes the connector's auto-update switch into goalcup's own isolated harness home (ledger H9), and after one question writes the no-prompt values there too, never into app-wide config (default, PRD-00 question 2). If the owner answers no, the no-prompt clause is not written and D67 stands for it | D67 superseded | PRD-10, PRD-11 |

4. **Status table** (`:17-50`): one row per entry in the column "Status table rows it changes",
   naming the new entry. The builder then reads every entry once and marks any other clause 1.0
   changes the same way. No entry text is edited.

After the merge to main (TODO): the rewrite to 24 current entries with a "Retired numbers" table that
maps every old number to its live entry, as revision 1 of this PRD planned. Hermes version notes from
D61 and D72 move to `docs/harnesses/hermes-cli.md` then (PRD-11).

### 7. The participant guide for 1.0 (`docs/PARTICIPANT-GUIDE.md`)

Path kept (`main.rs:59` today, PRD-10 section 11 in 1.0). Outline, about 400 lines:

| Section | What it says | Source |
|---|---|---|
| What goalcup does for you | Five lines: records your harness's model requests and session files, uploads them, never slows your agent, hides secrets before saving, tells you exactly what it wrote and what it lost | Seven points |
| 1. What you need | Agent Community account; a harness on the list; a key or a login; macOS, Linux or WSL2; a GitHub account for the real run | `RULES.md` |
| 2. Install | One command, the PATH line, `goalcup --version`; one line on firewalls such as Little Snitch (asked once per binary, again after updates) | PRD-10 section 7, PRD-14 |
| 3. Set up | `goalcup setup`: pick the harness; sign in inside goalcup's own harness home; the auto-update switch goalcup always writes there; the one question and what "yes" writes there (the no-prompt values); machine warnings; `Ready.`. How to use a gateway: configure it inside goalcup's home, nothing else to do. A model host goalcup does not know: `goalcup setup --record-host <host>`, and what is refused (an IP address, a sign-in host, a wide pattern) | PRD-10 sections 1, 7; PRD-11 sections 4, 7 |
| 4. Your harness: the list and the tiers | The three tiers in plain words, a link to the one harness table `docs/harnesses/README.md` (PRD-11; never copied into the guide), then the general setup notes from today's HARNESS-SETUP sections 1 to 7 (no prompts, push, turn limits, sign-in lifetime, subagents and their models, tools on PATH, first-run screens) and one paragraph on auto-update: goalcup turns it off by an env switch where one exists; for Hermes, Gemini CLI and Qwen Code, `goalcup setup` writes the switch into goalcup's home; an update matters only at the next `goalcup continue`, and the record names the version at every start | PRD-11 section 7, PRD-12 section 9 |
| 5. A practice run | `goalcup start --practice`, about 10 minutes, no GitHub, never counts | PRD-10 |
| 6. Your run | Repository, token, folder (`.gitignore`, `.env.local`), `--prompt FILE`, the approval code, the one first message on the clipboard and in `first-message.txt`, "keep going" only | PRD-10 sections 3, 8 |
| 7. During the run | The status block line by line; the recording states (on, restarting, off, losing data, behind, finished) and what each asks of you, including "recording off: disk write failed" (free space, goalcup retries by itself); each `PROBLEM:` line and its fix, including "goalcup is not running" while the harness still runs (`goalcup recover`, ledger R3b) and the line for a host that streamed and was not recorded (`--record-host`, then `goalcup seal` and `goalcup continue`: the host is recorded from the next part); `goalcup continue`; `goalcup time-left`; sleep and the sleep lock line | PRD-10 sections 4, 9, 10; PRD-02 section 5 |
| 8. The end | The end screen line by line (written requests by host, requests with missing parts, session files, off time, hosts that passed through unread, upload); if upload has not finished, the line "Run `goalcup upload` before <time>" and why (the server takes uploads until 24 hours after the run ends); what is removed from the computer and why; delete the token, log out or rotate keys; the videos | PRD-10 section 5, PRD-06 section 10, PRD-00 question 1 |
| 9. What is recorded, and what is not | PRD-05 section 12's paragraph verbatim as merged. Then one line each: tunnels (host, bytes and times only); vendor backends (the harness's traffic to its vendor, not the vendor's own model calls); harnesses that ignore the proxy ("not supported"); a harness that starts another harness as a subagent ("wire only": its calls are recorded, its session files are not); a model server on this computer (Ollama, LM Studio: not recorded, named in status); HTTP/3, QUIC and ECH never reach a proxy; a network that re-signs TLS and needs its own proxy is not supported; a token used as the user name of a URL with no password may be recorded; SQLite harnesses' databases are copied at the seal; on Linux, memory goalcup holds can reach unencrypted swap | PRD-05, PRD-03, PRD-04, PRD-11 section 8, PRD-17 T9 and K12, review A 5 and 7 |
| 10. Help | Support address; paste `goalcup status` and the run id, never a token; `goalcup.log` path | PRD-10 section 12 |
| Appendix A: start refusals | Each refusal and its fix, from `src/cli/text.rs`, including `--connector` and `--hosts` refused in a competition run | PRD-10 sections 1, 13 |
| Appendix B: gap reasons in plain words | PRD-01 section 3's table as merged (reasons 1 to 15; 8 and 9 reserved and not listed), in PRD-10's words | PRD-01, PRD-10 section 5 |

**The tiers, as the guide says them** (rules from PRD-12 section 9):

- **Verified on `<version>`**: the test pad ran this harness at that version with a direct key, and
  also through a gateway where it has one and in the "cannot read" case, and every check passed on
  goalcup's own record: no unexplained gap and no missed model call, every subagent was recorded, no secret was found, the server
  copy was whole, usage was readable, and the counts on screen were true.
- **Not verified**: goalcup records what reaches it, the same way, but the pad has not passed on this
  harness yet. The table says why (for example "no key"). `goalcup status` tells
  you if model requests seem to be missing.
- **Not supported**: the harness ignores goalcup's proxy or refuses its certificate. goalcup records
  its session files and, if it refused the certificate, the connection as a tunnel; its model calls
  are not recorded. A tier does not decide whether you may take part (RULES; PRD-00 question 4).

**The harness table is not in the guide.** The one harness table is `docs/harnesses/README.md`
(PRD-11); its tier columns are generated from `connectors/tiers.toml` (PRD-12), and the guide, README
and RULES link to it (rule 2). The cells below are PRD-15's draft of the participant columns, handed to
PRD-11 to check and own (ask 5): one row per harness (OpenCode 1 and 2 share a row); "Know this" is the
one fact a participant must act on, from PRD-11 section 7.

| Harness | `goalcup` id | Sign in | Gateway | Know this |
|---|---|---|---|---|
| Claude Code | `claude-code-cli` | Key or subscription | You set `ANTHROPIC_BASE_URL` in the `settings.json` of goalcup's Claude Code home; goalcup never sets it | goalcup refuses to start if `settings.json` there sets a proxy |
| Codex | `codex-cli` | Key or ChatGPT sign-in | A custom provider in goalcup's home | Do not put a proxy or CA in `$CODEX_HOME/.env` |
| Hermes | `hermes-cli` | Keys in its `.env`, or Nous sign-in | Its built-in providers, or `model.base_url` | goalcup stops a running Hermes gateway before the start; its `copilot-acp` child is wire only |
| OpenCode 1 and 2 | `opencode-cli`, `opencode2-cli` | Key or OpenCode account | Its own gateway or a provider | goalcup stops OpenCode 2's background service first |
| OpenClaw | `openclaw-cli` | Keys in its `.env` | Provider plugins | Leave its own proxy setting empty; subagents unmeasured |
| Amp | `amp-cli` | Amp account | Not needed | Amp's servers call the model; goalcup records what Amp sends there |
| GitHub Copilot CLI | `copilot-cli` | GitHub sign-in (GitHub's models) or your own key | Your own key | `/delegate` runs in GitHub's cloud and is not allowed (RULES) |
| Gemini CLI | `gemini-cli` | Gemini key, or Google sign-in (Google's servers call the model) | Not needed | Setup turns auto-update off in goalcup's home |
| Goose | `goose-cli` | Key | A custom provider | |
| Kilo CLI | `kilo-cli` | Key or Kilo account | Vercel AI Gateway or Kilo's gateway | |
| Qwen Code | `qwen-cli` | Key | Not needed | Leave its own `proxy` setting empty; its built-in `claude-code` and `codex` subagents start another harness and are not allowed (RULES) |
| Crush | `crush-cli` | Key | Not needed | |
| Factory Droid | `droid-cli` | Factory account plus your key | Your own key | Cloud computers are not allowed (RULES) |
| Kimi Code | `kimi-code-cli` | Kimi sign-in or a provider key | A provider in its config | |
| Cline CLI | `cline-cli` | Key or Cline account | Cline's provider | A Cline hub already running may bypass goalcup; quit it first |
| Cursor CLI | none | | | Out of 1.0: no connector. Documented as the dodger case in `docs/harnesses/` |

PRD-11 checks every row against its connector and the pad row at phase 5. A "Know this" line that
the pad shows to be wrong is removed, not hedged.

### 8. RULES.md edits

| Line | Today | 1.0 |
|---|---|---|
| `:10-11`, `:254` | Two approvals; `goalcup login` | One approval per run (PRD-00 question 2) |
| `:16` | "one known gap" | One sentence: secrets are masked by shape and by exact value; the guide lists what can still be recorded (PRD-05 section 12) |
| `:42-43` | 0.6.1 dry run | Delete |
| `:112-113` | `goalcup time-left` (D66) | Kept as is; D66 keeps its number |
| `:306-321` | Four labels including `wire_undecoded`; D74 | The `label` of `goalcup-verify check --json`: `wire_and_native`, `wire_only`, `native_only`, `none`, and the coverage line of the text output (PRD-08 section 5). A label is a hint for judges, not a judgement. Admissibility text unchanged |
| `:322-325` | Full disk is a red flag; the disk pause (D65) | "If the disk fills, recording goes off until there is space again, and the record shows the gap. Judges read it like any other gap." (D78) |
| `:332-335` | Upload every 30 minutes; one hour after the stop | The cadence and windows PRD-06 and PRD-07 ship, read from the merged code (default: every 5 minutes; up to 60 minutes after the seal; the server takes uploads until 24 hours after the run ends) |
| `:336-348` | `[masked]`, `<redacted:KIND>` markers | Stars of the same length; the PRD-05 list of what is still recorded |
| `:385` | APFS, ext4 or XFS required | Delete (PRD-10 cuts the filesystem list) |
| `:398-413` | `goalcup doctor --harness all`; four connectors | The 15 harnesses and their tiers (link to `docs/harnesses/README.md`); `/delegate`, Droid cloud computers and Qwen Code's harness-starting subagents named under "Other agents" and cloud modes (PRD-00 question 4); `--connector` and `--hosts` refused in a competition run (PRD-00 question 4) |
| `:485-489` | Still to decide | Move to `TODO.md` |

RULES is the participant's contract. The lines that change what a participant may do (the three
cloud modes, the tier sentence, file connectors) carry PRD-00's default for question 4 and are
marked in `TODO.md` as defaults until the owner answers. Edits that only follow cut code do not
need the owner.

### 9. OPERATING.md for 1.0

Keep the runbook, drop the binary reference that 1.0 makes small. About 280 lines:

1. **Where things run** (`docs/OPERATING.md:25-42`, kept): add the `goalcup-usage` bucket (PRD-07
   section 10) and the test bucket PRD-07 names, if any (ledger S3).
2. **Dates and checklist**: release on 6 Nov, the deploy order, the `EVENT_ID` switch.
3. **Deploy**: PRD-07 section 12 step by step. Staging: agents may deploy it (ledger B5); it carries
   the 1.0.0 floor from its first deploy (PRD-00 defaults). Production: the owner deploys after the
   merge, publishes the 1.0.0 release, then sets `MIN_COMPETITION_VERSION = "1.0.0"` (PRD-00
   question 5), then deletes the signing key secret; no rollback to the 0.7 Worker; `MAINTENANCE=on`.
4. **Test runs and the pad**: replaces "The matrix" (`:154-434`, 281 lines) with ten lines that point
   to `scripts/pad/` and `PAD-RESULTS.md` (PRD-12 interface).
5. **Fetch and check a run**: `scripts/fetch-run.sh`, then `goalcup-verify check --server`, `secrets`,
   `usage` (PRD-07 section 7, PRD-08 section 1).
6. **Runs whose recording is off**: a heartbeat with `recording: "failed"` marks one (PRD-06, PRD-07).
   The organizer query `stopped-runs.sql` comes after the merge (TODO).
7. **The usage job**: how to run it and where the JSON lands (PRD-09 section 7; third slice).
8. **Storage and the manual lever**: PRD-07 section 5's cost table and the one hand write.
9. **Bad days** (`:468-491`): kept and updated. "Edge cases by hand" (`:435-467`): row 4, disk nearly
   full (`:457`), goes; the rest moves to the pad or goes.
10. **Releasing**: PRD-14's text, plus the one line for 0.7 testers that the 1.0.0 release notes carry:
   "If you tested 0.7, install 1.0 fresh; 0.7 runs cannot be continued." (PRD-10 section 11).

Cut: "Running the daemon" (`:571-610`), "Running a capture with the CLI" (`:611-655`, now the README
quick start), "Running the verifier" (`:656-730`, now PRD-08's four subcommands in README),
"Pinning connectors" (`:731-746`) and "Finding codes" (`:747-863`, replaced by PRD-08's
`check --json` report, documented in RECORD-FORMAT's last sections). "Building and verifying" becomes
the README developer section plus PRD-14's CI text (`:530`); "Installing the participant binaries"
becomes one link to `packaging/README.md`.

### 10. Contributor docs

- **`docs/ARCHITECTURE.md`**, about 160 lines: (1) what it is for (the seven points); (2) the three
  crates and the dependency rule; (3) the one process, with PRD-00's diagram and the rules of the
  shape (relay first, mask before disk, no raw bytes leave memory, one writer, the watchdog, null
  mode, boot-time timers); (4) the one host list, tunnels, gateways, vendor backends, dodgers and
  local model servers (PRD-11 "The owner's question"); (5) session files; (6) masking; (7) the record
  and the upload, pointing to RECORD-FORMAT and UPLOAD-API; (8) terms: run, entry (a "part" on
  screen), boot, part file (a "record file" on screen), stream, gap, off span, link, seal, written
  count, tier; (9) where each point is tested (the README table); (10) the lessons list
  (`docs/plans/2026-10-30-v1.0/LESSONS.md`, PRD-17).
- **`docs/RECORD-FORMAT.md`** (new), about 280 lines, written from PRD-01 revision 2 (its asks are
  closed): files on disk, framing, records, stream kinds, IDs, time, gaps (the final reason table)
  and off spans, masks, counts, links, the seal, compression and fetch, invariants, the worked
  example, and a Python reader of about 60 lines that reads the decompressed stream from standard
  input (`cat rec-*.gcr.zst | zstd -dc --long=27 | python3 read.py`), so it needs only the standard
  library. Last two sections: the `goalcup-verify check --json` report (PRD-08 S6, with `label`) and
  the usage JSON (PRD-09 section 7: the schema, its token table and its hint table).
- **`docs/CONNECTOR-SCHEMA.md`**: PRD-11 section 2's table (including `no_prompts`, `first_screen`,
  `warn_missing`, `vendor`, `update_off`, `skills_dir`), the host list rules of its section 4, one
  example connector, and how `--connector` and `--hosts` load a file (practice and test runs only).
  About 90 lines.
- **`CONTRIBUTING.md`**: the gate paragraph from PRD-14; "Adding a harness" (`:76-117`) becomes the
  pad path (PRD-12 section 11) in ten lines plus a short reviewer checklist moved from
  `HARNESS-ONBOARDING.md`: the connector sets no base URL, no proxy or CA variable outside `ca_env`,
  `env` holds no key, every host added to `model-hosts.toml` has a vendor doc link, the pad row is
  attached. D51 becomes D42 and D84. `spikes/redact.py` goes.
- **`SECURITY.md`**, about 110 lines: supported version (1.0 only); what runs on your machine (one
  recorder process and its watchdog, a loopback proxy, a run CA trusted only through the harness's own
  variables, isolated harness homes, no keys held, the run credential at 0600, the run CA key in a 0600
  file from the start of an entry to its seal so `goalcup recover` can reuse it (ledger R3b), the upload); how raw
  bytes are kept off disk (masked before writing, core dumps off, panic text fixed) and the one place
  they can still land: memory paged to unencrypted swap on Linux (macOS encrypts swap; PRD-02); trust
  boundaries (no control API; same-user processes are inside the boundary; nothing is signed, so the
  organizers trust the server copy and its `received_at` times); connectors are trusted data compiled
  into the binary, and a file connector is named in the record; "What a check does not prove" (from
  `docs/CHEAT-CHECKLIST.md`, only the rows still true: masking gaps, length visible, tunnels unread,
  vendor backends, the harness's own log read only for harnesses with a reader, the label is a hint,
  local record files can be changed before upload within one flush interval).
- **`docs/HARNESS-ONBOARDING.md`**: five lines: the pad is the one way to add a harness; links to the
  skill, CONTRIBUTING's reviewer checklist and CONNECTOR-SCHEMA.
- **`scripts/README.md`**: the kept scripts (PRD-13 "Kept scripts", PRD-14 section 9) and
  `scripts/pad/`.
- **`TODO.md`**: under 60 lines (section 3).

## Files it owns

Change: `README.md`, `AGENTS.md`, `TODO.md`, `SECURITY.md`, `CONTRIBUTING.md`, `scripts/README.md`,
`docs/PARTICIPANT-GUIDE.md`, `docs/RULES.md`, `docs/OPERATING.md`, `docs/ARCHITECTURE.md`,
`docs/CONNECTOR-SCHEMA.md`, `docs/HARNESS-ONBOARDING.md`, `docs/DECISIONS.md` (append and status
table only). Ownership per PRD-00's table, which gives PRD-15 `docs/**` except `docs/UPLOAD-API.md`,
`docs/harnesses/**` and `docs/plans/2026-10-30-v1.0/**`.

Create: `docs/RECORD-FORMAT.md`; `docs/research/results/2026-10-30-two-hour-run.md` (copied as is).

Move to `docs/archive/` (`git mv`, no edit): `docs/CONTRACTS.md`, `docs/TERMINATION.md`,
`docs/CHEAT-CHECKLIST.md`, `docs/HARNESS-SETUP.md`, and `docs/SERVER-DESIGN.md` after PRD-07
confirms its facts are in the Worker README.

Delete: `docs/README.md`.

No change in this build: `CLAUDE.md`, `VISION.md` (the owner's), `CODE_OF_CONDUCT.md`, `LICENSE`,
`NOTICE`, and the history: `docs/archive/**` (other than the moves in), `docs/research/**` (other than
the one copy in), the dated plans, `docs/RESEARCH-TARGET.md`, `docs/LOCAL-RESEARCH-HANDOFF.md`.

Must not touch: `docs/UPLOAD-API.md`, `server/**` (PRD-07); `connectors/**`, `docs/harnesses/**`
(PRD-11; `connectors/tiers.toml` is PRD-12's); `skills/goal-cup/**` (PRD-10);
`skills/adding-a-capture-harness/**`, `scripts/pad/**`, `PAD-RESULTS.md` (PRD-12); `packaging/**`,
`.github/**`, `scripts/gate.sh`, `scripts/check-links.py` (PRD-14); every crate;
`docs/plans/2026-10-30-cut-first-plan.md`, `docs/plans/2026-10-30-audit/**` and every file in
`docs/plans/2026-10-30-v1.0/` other than this one; `LESSONS.md` (PRD-17); the night log (PRD-16).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-00 | Ownership; owner questions 1, 2, 3, 4, 5, 9, 10, 12 (revision 4 numbers), cited by number; definition of done item 6; "Not in this build" (history, DECISIONS rewrite, acceptance runs) | Used here |
| PRD-01 | `docs/RECORD-FORMAT.md` is written from revision 2; the final GAP reason table for guide appendix B; "part file" in prose | Used here |
| PRD-02 | D75 text (one process, guard, watchdog restart, null mode, no core dumps, boot-time timers); `status.json` modes for the guide's state table; disk full as exit 74 and null mode (D78, README, RULES); the swap residual for SECURITY | Used here |
| PRD-03 | Host list behaviour, tunnel notes, the per-host long-tunnel flag, the `never` list, the HTTP/3 and ECH line | Used here |
| PRD-04 | Session file rows, final snapshots, the SQLite note, "files cut at the seal" | Used here |
| PRD-05 | Section 12 paragraph into the guide and README; `RULES.md:16`; D80 | Used here |
| PRD-06 | Upload lines (section 10), cadence and the wrap-up window for RULES and the guide; the "upload before" line | Used here |
| PRD-07 | `server/worker/README.md` absorbs SERVER-DESIGN (they confirm before the move); deploy order, floor, test bucket and storage cost for OPERATING; PRD-07's list of stale docs (OPERATING, SERVER-DESIGN, CHEAT-CHECKLIST) is done here | Both |
| PRD-08 | Four subcommands and exit codes for README and AGENTS; the S6 `label` and the coverage line for RULES and SECURITY; the `check --json` report for RECORD-FORMAT; the judge view is after the merge | Used here |
| PRD-09 | Usage JSON schema, token and hint tables as a RECORD-FORMAT section; the organizer job for OPERATING | Used here |
| PRD-10 | Guide sections 3 to 10, the message list, "part" and "record file", the 0.7 release-notes line (OPERATING "Releasing"), the firewall line, the guide path in the help footer, the `local_model` and long-tunnel lines, the file-connector refusal | Used here |
| PRD-11 | CONNECTOR-SCHEMA from its section 2; harness facts from its section 7; "wire only" (section 8) and `local_model` in the guide; the gateway answer for README; `docs/harnesses/README.md`, the one harness table the guide, README and RULES link (PRD-15 hands it the draft participant cells of section 7) | Both |
| PRD-12 | The skill replaces HARNESS-ONBOARDING (shrunk to a pointer); `connectors/tiers.toml` feeds the tier columns of `docs/harnesses/README.md` (PRD-11), which the guide and README link; `PAD-RESULTS.md` feeds README's seven-points table; tier rules for the guide; OPERATING points to the pad | Used here |
| PRD-13 | The reference guard and its exclusions (`docs/archive/**`, `docs/research/**`, `docs/plans/**`, `docs/DECISIONS.md`); its list of live docs that name old crates | Used here |
| PRD-14 | Gate text for README, AGENTS and CONTRIBUTING with measured times; the guard list (section 2) for README's seven-points table; release text for OPERATING; `check-links.py` stays until PRD-15's run, then PRD-14 deletes it in the phase 3 merge | Both |
| PRD-16 | Merge timing (as soon as green); the night log; the newcomer run's slot (ask 4) | Both |
| PRD-17 | `LESSONS.md` linked from ARCHITECTURE; T9 and K12 into the guide's known limits | Used here |

## What is cut

In this build:

| Path | Lines | Note |
|---|---:|---|
| `docs/CONTRACTS.md`, `TERMINATION.md`, `CHEAT-CHECKLIST.md`, `HARNESS-SETUP.md`, `SERVER-DESIGN.md` | 1,853 | Moved to `docs/archive/`, not edited (654, 262, 305, 468, 164) |
| `docs/HARNESS-ONBOARDING.md` | 574 | 579 to 5 |
| `docs/README.md` | 83 | Deleted |
| Shrunk in place | about 1,900 | CONNECTOR-SCHEMA 1,002 to 90, OPERATING 863 to 280, TODO 350 to 60, ARCHITECTURE 350 to 160, README 318 to 220, RULES 489 to 430, AGENTS 110 to 95 |
| **Total** | about **4,400 out of the live docs, 600 in** | The guide grows by about 70 lines; RECORD-FORMAT, the run 1 report and DECISIONS' new entries are new |

After the merge to main (TODO, PRD-00 "Not in this build"): the history purge (`docs/archive/`,
`docs/research/`, the dated plans, RESEARCH-TARGET, LOCAL-RESEARCH-HANDOFF; about 8,000 lines) and the
DECISIONS rewrite (1,162 to about 260).

Tests that go: none in this PRD. `scripts/check-links.py` and `scripts/check-version.py` with their
tests are PRD-14's (PRD-00 ownership).

## Acceptance

Real-run proof first, on the integration branch after the third slice lands the guide, in a Vercel
Sandbox pad box (ledger B1), by a watcher that has not read the PRDs:

1. **A newcomer completes a practice run from the docs alone.** Given only `README.md` and
   `docs/PARTICIPANT-GUIDE.md`, the watcher installs the 1.0 build in a Vercel Sandbox pad box, runs
   `goalcup setup` and a practice run against the staging Worker. The watcher is given only
   `GOALCUP_SERVER` for staging; the conductor approves the run with the staging test approval key and
   the owner's member id (ledger B4). These two inputs are not counted as guesses. Everything else
   comes from README and the guide. The watcher writes down every step it had to guess and every line
   on screen the guide does not explain. Pass: zero guesses; zero unexplained lines; the three
   contract words match byte for byte. The box is removed when the run ends (ledger B3: every run
   cleans up after itself).

Checks on saved output, no extra harness run (the PRD-15 lane does them):

2. **Every line of the core-set end screens is explained.** For the Claude Code, Codex and Hermes pad
   rows, each line of the saved end screen and each recording state seen in `status.jsonl` maps to a
   guide line or the gap reasons table. Pass: zero unmapped lines.
3. **The format doc reads a real record.** The Python reader in `docs/RECORD-FORMAT.md` runs on a
   kept core-set pad entry. Pass: its record count per kind equals `goalcup-verify dump`'s.
4. **Review checklist for AGENTS.md** (replaces revision 1's fresh-agent run): the reviewer confirms
   AGENTS.md alone answers which crate masks; whether the recorder may link the verifier; whether a
   test that guards point 4 may be deleted; what goalcup does on a full disk (it points to D78); where
   the host list lives; whether goalcup may set a base URL.

After the merge (TODO): the contributor run (a harness added from CONTRIBUTING, the skill and
CONNECTOR-SCHEMA alone) and the organizer run (every OPERATING command as written).

Automated, run once before the last PRD-15 merge ahead of the phase 3 merge (no new gate code), in a
build box (ledger B1):

- Link check, only for the files this PRD writes: `python3 scripts/check-links.py --root .` (or, if
  PRD-14 has already removed it, `git show 4b226de2:scripts/check-links.py > <tmp>/check-links.py`
  and run that copy; it is stdlib only), filtered to lines whose file is one of "Files it owns".
  Pass: no line. Broken links inside history docs are expected after the cut and not counted.
- PRD-13's reference guard, with the old crates excluded by hand before the cut, prints nothing for
  the files this PRD writes.
- A case-sensitive stale-term grep over the files this PRD writes prints nothing: `goalcup-daemon`,
  `goalcup-cli`, `goalcup-protocol`, `goalcup-verifier`, `goalcup-platform`, `goalcup login`,
  `goalcup doctor`, `goalcup mcp`, `goalcup entries`, `prep.json`, `--capture-integrity`,
  `--research-json`, `pin-manifests`, `receipt`, `SIGSTOP`, `paused`, `Recording works`,
  `protocol v5`, `P1 to P7`, `D65`, `goalcup-verify judge`. Allowed: the D entries and status rows in
  DECISIONS.
- Every `goalcup <command>` in README, the guide, RULES and OPERATING is one of PRD-10's seven, and every `goalcup-verify <subcommand>` one of PRD-08's four.
- Every `D<number>` cited in surviving code, connectors, server, skills and docs is an entry in
  DECISIONS; a cite in new code is a current entry or D75 to D86, not a superseded one.
- A grep for the em dash character (U+2014) over the files this PRD writes prints nothing.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Docs written from PRDs that change during the build | The guide quotes a line PRD-10 changed late | Participant text is written in the third slice from merged `src/cli/text.rs`; only the contract words are quoted |
| A tier printed before the pad ran | A participant trusts "verified" for an untested harness | Tiers only from `connectors/tiers.toml`, shown only in `docs/harnesses/README.md` (PRD-11); default "not verified"; no PRD-15 doc copies a tier |
| DECISIONS grows instead of shrinking | 57 entries plus 12 new, with old text that contradicts 1.0 | The header and status table send readers to D75 to D86 first; the rewrite to 24 is on the after-merge list. Accepted for one week |
| History kept | A later agent reads a dead doc as current | Dead live docs sit in `docs/archive/`; AGENTS' read-first list names only current docs; the guard keeps old crate names out of every current doc |
| Link check over the whole tree | After the cut, history docs link into deleted crates, so a plain run fails | The run is filtered to this PRD's files; history links are accepted as broken |
| No link check after the build | Links rot after PRD-14 deletes `check-links.py` | Accepted (PRD-14); the run here is the last one, and main's copy stays runnable by hand |
| 0.7 links break | 0.7 testers clicking `HARNESS-SETUP.md` get a 404 | 0.7 is refused once the floor is set; one line in the 1.0.0 release notes covers testers (ledger L7); HARNESS-ONBOARDING stays as a pointer |
| RULES is a contract | Changing eligibility wording without the owner changes the deal participants signed | Lines that change what a participant may do carry PRD-00's default (question 4) and are marked as defaults in TODO |
| The guide grows with 15 harnesses | A long table nobody reads | The guide links the one table in `docs/harnesses/README.md`; detail stays in `docs/harnesses/` |
| "Part" means two things | A participant reads "Parts: 2" and a contributor reads "part file" | Rule 8; glossary in ARCHITECTURE |
| Owner questions unanswered | Docs show defaults as facts | Each default is named as a default in TODO and in its D entry |
| macOS and Linux arm64 untested in this build | README says they work | README says "macOS: the owner's Mac run is pending" and "Linux arm64 built, not yet tested" until each passes (ledger T9) |
| The run 1 report names old crates | The reference guard would flag it | It lives under `docs/research/results/`, which the guard excludes (PRD-13 section 6) |
| Swap is the one raw-bytes leak left | A participant on Linux with unencrypted swap | SECURITY and the guide say so plainly; nothing is built for it |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| History docs | Kept in this build; purged after the merge | Owner (PRD-00) |
| Dead live docs | Moved to `docs/archive/`, not deleted | PRD-15 |
| HARNESS-SETUP | Merged into the guide | PRD-15 |
| HARNESS-ONBOARDING | A 5-line pointer | PRD-15 with PRD-12 |
| CHEAT-CHECKLIST | Merged into SECURITY | PRD-15 with PRD-08 |
| Usage schema | A section of RECORD-FORMAT, not its own file | PRD-15 with PRD-09 |
| Decision numbers | Old entries untouched; new rules D75 to D86; rewrite after the merge | PRD-15 |
| Run 1 report | Copied to main as one file, no evidence folder | Owner |
| TODO size | Under 60 lines | PRD-15 |

## Asks of other PRDs

The eleven asks of PRD-00 and four of PRD-01 in revision 1 are answered (PRD-00 and PRD-01
revision 2, "Review answers"). Rulings this revision follows: ownership as asked; history kept (ask 2
rejected); docs land as soon as green (ask 3 changed); one tier file, `connectors/tiers.toml`
(ask 4); DECISIONS is the home for owner decisions (ask 5); tier and eligibility and file connectors
are PRD-00 question 4, old 14 and 15 (asks 6, 8); the connector fields PRD-10 needs are kept (ask 7); the
run 1 report is copied here and excluded from the guard (ask 9); staging is deployed by agents
(ask 10); `login` is cut (ask 11, question 2, old 9). PRD-01: "part file" in prose, revision 2 is settled,
the reader reads standard input, reason 12 `mask_failed` and 13 `upstream_untrusted`.

Left, small:

1. **PRD-07.** Name the test bucket, if one is created, so OPERATING lists it; confirm in the night
   log when `server/worker/README.md` holds SERVER-DESIGN's facts, so the move can happen.
2. **PRD-14.** Leave `scripts/check-links.py` in the tree until PRD-15's night-log line "link check
   run". If it goes earlier, PRD-15 runs main's copy (Acceptance); nothing blocks.
3. **PRD-08.** The rewritten CHEAT-CHECKLIST text lives in SECURITY "What a check does not prove";
   point there, not at `docs/CHEAT-CHECKLIST.md`.
4. **PRD-16.** Schedule the newcomer run: one practice run in a pad box on staging, after the third
   slice lands the guide, by a watcher given no PRD; about 15 minutes, `smoke` budget.
5. **PRD-11.** `docs/harnesses/README.md` carries, per harness, the sign-in, gateway and "Know this"
   cells a participant needs; PRD-15's draft is in section 7. PRD-11 checks and owns them; the guide,
   README and RULES only link the table.

## Builder notes

Order of work (PRD-00 puts PRD-15's docs in the third slice; parts that need no code go earlier):

1. **Any time from H+0, from decisions only:** DECISIONS (section 6: header line, owner decisions in
   force, D75 to D86, status rows); AGENTS.md hard rules (the two changes, the pin line, the commit
   paths). Merge when green. Copy the run 1 report.
2. **After the core slice merges:** AGENTS.md map, `docs/ARCHITECTURE.md`, `docs/RECORD-FORMAT.md`
   (PRD-01 revision 2 is settled), `docs/CONNECTOR-SCHEMA.md` (after PRD-11's schema merges).
3. **Third slice, from merged code:** the guide, README, RULES, OPERATING, SECURITY, CONTRIBUTING,
   HARNESS-ONBOARDING, `scripts/README.md`, TODO; the moves to `docs/archive/` and the delete of
   `docs/README.md` in the same merge, since the new docs replace their links. Read `src/cli/text.rs`
   from the merged code, and `goalcup --help` and `goalcup-verify --help` from the build's output in a
   build box; never from a PRD draft.
4. **Before the phase 3 merge:** the automated checks of Acceptance; night-log line "link check run".
   Nothing of PRD-15 goes into the phase 3 merge.
5. **After phase 5:** confirm the guide, README and RULES link `docs/harnesses/README.md` and copy no
   tier (PRD-11 generates its tier columns from `connectors/tiers.toml`). Then checks 2 and 3 on the
   core-set pad rows.

Traps:

- Keep `docs/PARTICIPANT-GUIDE.md` and the README anchors `#platforms` and `#the-gate`.
- Never name an old crate in a current doc; PRD-13's reference guard flags it at the cut (a grep by
  hand, not a gate test, ledger B10).
- Never write "model calls recorded"; write "model requests written" (rule 5).
- `goalcup-verify` has four subcommands in this build; the judge view is after the merge.
- Disk full is exit 74 and null mode, never a `write_error` gap (reason 8 is reserved).
- The run 1 report says what happened in 0.7; do not quote its numbers as 1.0 numbers.
- The 2-hour run of 1.0 has not happened. No doc cites its results until its report exists.
- Never write a key, a key file path or a token in a doc; name secrets only (`docs/OPERATING.md:9-10`).
- No em dashes. Check with the grep in Acceptance.

The after-merge list for `TODO.md`: the history purge; the DECISIONS rewrite to 24 entries with the
retired-numbers table; Hermes notes from D61 and D72 into its facts sheet; PRD-15's contributor and
organizer runs; and the items other PRDs defer (PRD-00 "Not in this build").

How a watcher proves it in a real run: run acceptance 1 (ask 4) on the first green build that has the guide,
then checks 2 and 3 on the three core-set pad rows. Write every unexplained line into the night log
with the screen it came from; the fix is a guide edit, or a `text.rs` edit by PRD-10 when the screen
is wrong.

## Review answers

History of revision 2. It uses PRD-00 revision 2's owner question numbers (PRD-00 maps them to
revision 4's). The body above, the ledger and "Alignment answers" win over it.

CC is the consistency check, A is review A, B is review B.

| Item | Answer |
|---|---|
| CC 10, B C20, CC 102, B N13 History docs | Accepted. History is kept in this build (PRD-00); the purge is on the after-merge list. The three docs other PRDs cite stay. Dead live docs that name old crates are moved to `docs/archive/`, not deleted, so the guard passes without a purge |
| CC 10, B C21, CC 103 `check-links.py` owner and timing | Accepted. PRD-14 owns and deletes it in the phase 3 merge after PRD-15's run. Checked in code: it scans every tracked `.md` (`check-links.py:1-19`, `:1343`), so after the cut history docs always fail; the run is filtered to this PRD's files, and main's copy is the fallback, so there is no timing dependency |
| CC 23 PRD-15 asks 6 and 8 as owner questions | Accepted. Now PRD-00 questions 14 and 15; RULES, D82 and D84 carry their defaults, marked as defaults |
| CC 26 Pin rule | Accepted. Default taken; the AGENTS line goes as a consequence of the pin cut |
| CC 27 Run report path | Accepted. Copied to `docs/research/results/`, which the guard already excludes |
| CC 73, B C14 (ask 7) Connector fields | Closed by PRD-00 S3 and PRD-11: fields kept. The guide's setup section describes the one question (question 8) and CONNECTOR-SCHEMA lists the fields |
| CC 75, B C12 File connectors | Accepted. Refused in competition (question 15) in RULES, D84 and guide appendix A |
| CC 87, CC 101 D9 `write_error` | Accepted. Checked against PRD-02 section 5: a write error is exit 74, then restarts, then null mode, and the `recorder_down` gap names the cause. The rule is now D78. README and RULES also said "run `goalcup continue`" after freeing space, which was wrong: goalcup retries by itself |
| CC 98 Tier file | Accepted. `connectors/tiers.toml` (PRD-12) feeds the guide and README; nothing is copied by hand |
| CC 105 `failed` only in null mode | No change needed here. The guide's state table uses PRD-10's words |
| CC 40, PRD-01 ask 1 "Part" | Accepted. Rule 8: "part" and "record file" for participants, "part file" for contributors, glossary in ARCHITECTURE |
| CC 112 Style | Checked: no em dash in this file |
| A 5 Core dumps; swap residual | Accepted. SECURITY says how raw bytes stay off disk and that Linux swap is the one place left; the guide has one line; D75 records "core dumps off" |
| A 12 Keys added during the run; test payment keys | Accepted as text. PRD-05's paragraph, quoted verbatim, says a key added during the run may be recorded; the rule change itself is PRD-05's (question 12) |
| A 7 Local model server | Accepted. Guide section 9 and D81 name it; status shows the `local_model` line |
| A 6 Unlisted model host | Partly. The guide explains the per-host `PROBLEM:` line and `--record-host`; detection is PRD-03's and PRD-10's |
| A 15 Tail lost after an outage | Accepted. Guide section 8 explains the "Run `goalcup upload` before <time>" line (question 2) |
| A 16 Auto-update; cross-harness subagents | Accepted as text. Setup writes the update switch (question 8); "wire only" is in the guide and the table |
| A 1 to 4, 8 to 11, 13, 14 | Not this PRD's. Docs follow whatever their owners merge |
| B B5 Too big for one day | Partly. The purge and the 24-entry rewrite wait. This build still rewrites every current doc that names an old crate (14 kept Markdown files hit the guard pattern, checked with `git grep`), RULES (the contract is false on release day) and OPERATING (deploy), and adds twelve DECISIONS entries, not two, because the owner decisions otherwise have no home outside plan files |
| B T8 Four extra agent runs | Accepted. One run (the newcomer); checks 2 and 3 read saved pad output; the contributor and organizer runs are after the merge |
| B W11 AGENTS-only agent run | Accepted. Now a review checklist item (acceptance 4) |
| B M5 One big phase 3 merge | Accepted. PRD-15's files land as soon as green; none is in the phase 3 merge |
| B A3 Owner questions in three places | Accepted. This PRD lists none of its own; it cites PRD-00's numbers |
| B A10 `AMENDMENTS.md` | Not this PRD's file; no change |
| PRD-08 subcommands (found while checking interfaces) | Fixed. Four subcommands (`check`, `dump`, `secrets`, `usage`); the judge view is after the merge; RULES uses the S6 `label` and the coverage line |
| PRD-12 HARNESS-ONBOARDING | Accepted. Shrunk to a pointer, not deleted |
| PRD-17 T9 and K12 | Accepted. Both in the guide's known limits |
| Owner's relayed question (a gateway is an HTTPS proxy with another address; strong pads for proxy and gateway, dodgers use the same pipe) | Answered yes in README section 5 and the guide: direct key and gateway are one path, vendor backends and dodgers are named, nothing else is built |

## Alignment answers

Revision 3 answers the three alignment fix lists for this file (groups 1 to 3). The ledger wins.
"Applied" means the body above now says it; "partly" says what was taken; "rejected" says why. Only
this file's part of a shared fix is made here.

| Id | Answer |
|---|---|
| L4-1 | Applied. D80: only the run's `GH_TOKEN` is masked by exact value, as today; the other PRD-05 section 9 sources and the wider Stripe rule only with the owner's yes (PRD-00 question 3, default no). "Owner decisions it carries out" says the same |
| L4-8 | Applied. D80 names the guard class: a letter, a digit or `_`; `-` only with the owner's yes (PRD-00 question 10, owner list OW-02) |
| L7-05 | Applied. AGENTS.md map paragraph names the core code and the usage reader in `crates/goalcup-verify`; the hard-rule line is unchanged |
| L7-07 | Applied, merged with G3-15g. The section 4 note cites ledger section 7 and PRD-00 question 2, old 11 |
| L7-08 | Applied in the draft cells of section 7, which now go to PRD-11's one table (X2-09) |
| L7-11 | Applied. The "Never commit" rule names root `/pad-out/` and `/harnesses/`, so `docs/harnesses/` stays committable; any `target/`, keys and real records. The change table row says the wording is on the owner list (G3-O4, default yes) |
| X2-08 | Applied. Guide appendix C, rule 7's exception and both acceptance exceptions are gone; one release-notes line for 0.7 testers is in OPERATING "Releasing" (PRD-10 section 11 points here); the risk row says 0.7 is refused once the floor is set |
| X2-09 | Applied. `docs/harnesses/README.md` (PRD-11) is the one harness table; the guide, README and RULES link it. Section 7's table is kept only as draft cells handed to PRD-11 (new ask 5), so the facts are not lost; builder step 5, interfaces and risks follow |
| X2-11 | Partly, as PRD-00 did. This pass may edit only this file, so nothing moves to `REVIEW-HISTORY.md`. "Review answers" now says it is history, uses the old question numbers, and that the body, the ledger and these answers win over it. The move may run once, after every alignment pass, by one agent |
| G3-15a | Applied, with revision 4's numbers, not revision 3's: PRD-00 is now revision 4 with twelve questions (1 to 8 as revision 3, 9 to 12 new). Header, owner decisions, D entries, guide, RULES, interfaces, risks and asks use them; "Review answers" keeps the old ones as history |
| G3-15b | Partly. The sentence is added to the existing "records and never manages" rule, not as a new bullet, because ledger section 7 lists no new hard rule; acceptance 4 is answered by it |
| G3-15c | Applied |
| G3-15d | Applied |
| G3-15e | Applied to D76; status rows add "D64 partly (its backoff clause)". No numbers for the backoff: they come from PRD-06's merged code (rule 4) |
| G3-15f | Applied |
| G3-X31 | Applied as ask 4 to PRD-16; builder notes point to it |
| G3-X17 | Applied. Checked against PRD-10's `long_tunnel` lines |
| G3-15g | Applied, merged with L7-07 |
| G3-X19 | Applied. D75 cites ledger R3 and PRD-00 question 9 (default yes) |
| G3-15h | Applied |
| G3-15i | Applied, with the test seams clause naming the one test-only build feature (PRD-00 question 12) |
| G3-15j | Rejected as a default. It conflicts with L4-1 (a blocker) and PRD-00 question 3, default no: ledger M1 is the masking list. D80 names the Stripe widening as built only with the owner's yes |
| G3-15k | Applied. Checked in the tree: RULES upload is `:332-335`, masking `:336-348`; OPERATING "Edge cases by hand" `:435-467`, "Bad days" `:468-491` (the fix list said `:493`; corrected) |
| G3-15l | Applied in section 9 after the cut list |
| G3-15m | Applied (ledger T9); the macOS risk row also names Linux arm64 |
| G3-15n | Applied in README and the guide's tier text |
| G3-15o | Applied in the draft cells (ledger T1, H5) |
| G3-15p | Applied: acceptance 1 in a Vercel Sandbox pad box; the automated checks and builder step 3 in a build box (ledger B1) |
| G3-15q | Applied. Checked: PRD-14 section 2 has the same-name-prefix clause |
| Re-read (no id) | Fixed one clear break: D86, guide sections 3 and 4 and the Gemini cell said the auto-update switch is written only on "yes". PRD-00 question 2 (revision 4) and ledger H9 say `update_off` is always written; only the no-prompt values follow the question |
| B3, B10 sweep (30 Oct) | Applied. The newcomer run's box is removed when it ends; the reference guard is a grep by hand at the cut, not a gate test (PRD-13 section 6) |

## Amendments

### 2026-10-30, replan after PRD-15 slice 1 (merge 98bbb7e9)

Slice 1 merged builder step 1: the DECISIONS header line, "Owner decisions in force", D75 to D86 and
the status rows; the AGENTS.md read-first paragraph, hard rules, Tests and Done; the run 1 report copy.
What it changes for the slices still to come:

- **The last link check leaves out the run 1 report by name** (reviews A1, B-3). The copy keeps 21
  relative links into `2026-10-30-two-hour-run/`, which is not copied (section 3, "copied as is").
  Acceptance filters to "Files it owns", and the report is in the Create list, so the check as written
  lists them. Filter out `docs/research/results/2026-10-30-two-hour-run.md` by name, and the night-log
  line "link check run" says it was left out and why. Do not edit the report. The stale-term grep
  leaves it out the same way (it is history and names 0.7 terms on 7 lines). The em dash grep keeps it
  (it has none). PRD-13's reference guard already skips `docs/research/**`.
- **`scripts/check-links.py` needs Python 3.10 or later**, and the box snapshot's `python3` is 3.9.
  Slice 1 ran it with `python3.11` in the box (lane log 10:46Z). The last run does the same.
- **Slice 2 (the AGENTS.md map) drops every pin mention** (review A4): "Pinned by digest" in the
  `connectors/<id>/` row (`AGENTS.md:37` on integration), "pins" in the `scripts/` row (`:39`) and
  "connector pins" in the `goalcup-protocol` row (`:32`). Until the cut, the old crates still refuse an
  unpinned `connectors/<id>/harness.toml` edit while the hard rules no longer say so; accepted for the
  window.
- **The Tests pointer comes back with the README section it points to** (review B-4): "Where they
  are: [README](README.md#the-seven-points-and-their-tests)" (section 4) lands in the same merge that
  gives README that section, since the anchor does not exist before it and the link check would flag
  it. Likewise the read-first paragraph gains "then [ARCHITECTURE](docs/ARCHITECTURE.md)" in the merge
  that rewrites ARCHITECTURE.
- **Tests sentence wording** (review A5): the next AGENTS.md slice writes "that test is not deleted
  while the point stands, unless it is replaced in the same change by a test with the same name prefix
  that asserts the same promise". The "or" in section 4 reads as either/or. Same rule as ledger
  section 7 ("a test stays if it proves one of them"); wording only.
- **The never-commit line keeps `spikes/*/out/` while `spikes/` is on disk** (lane log 10:43Z);
  section 4's text drops it. Nothing of PRD-15 goes into the phase 3 merge, so the first PRD-15 merge
  after PRD-13 deletes `spikes/` drops that clause.
- **Status-table marks for the read-through of section 6 item 4** (review A3), in a later slice that
  touches DECISIONS, status-table cells only: D71's row also names D84 for its 0.6.1 compatibility
  clause (the hidden git-credential helper and the `github.json` loader) and its `prep.json`
  read-and-ignore clause. D4's row says "partly" for D75 unless its one-workspace, one-entry paragraph
  is really gone in 1.0 (D75 does not restate it; check PRD-02 and PRD-10 first). D61's "forwards no
  key" clause is touched by PRD-00 question 2's whole-environment default (review N15), which has no
  entry among D75 to D86: its row reads "partly, PRD-00 question 2 (default)" until the owner answers,
  and the item stays on the `TODO.md` open list.
- **Left for the after-merge DECISIONS rewrite** (already on the TODO list; reviews A2, B-1): each
  replacing entry's "This supersedes" line names every old entry whose row points to it (D3, D9, D37,
  D40, D43, D45, D46, D52, D53 and D71 are not named today), and D75, D76, D80, D81 and D84 go back to
  the one-to-eight-line prose of section 6, without the "This supersedes" paragraph that repeats the
  status rows (about 30 to 40 lines).
- **Drop one sentence at the first owner answer at the latest** (review B-2): "None was answered when
  these entries were written, so each default below holds until the owner answers" in "Owner
  decisions in force". The per-bullet "(default, PRD-00 question N)" markers already say it, and the
  sentence turns false at the first answer. It is this build's own text, so dropping it edits no old
  entry.

### 2026-10-30, replan after PRD-15 slice 2 (merge 21c0fd75)

Slice 2 merged `docs/RECORD-FORMAT.md` (324 lines, the reader 51 of them), the D4, D61 and D71
status marks with the stale "None was answered" sentence dropped, and the AGENTS.md Tests "unless"
wording. `gate fast` passed in 77 s. Two nits came after the merge; per the earlier ruling they go to
the next slice that touches `docs/RECORD-FORMAT.md` (the one adding the `check --json` and usage
JSON sections), not a separate merge request:

- **Invariant 4 gets PRD-01's two missing clauses** (review A-N5). Today it reads "DATA offsets are
  contiguous from `meta.from` (or 0), except where a GAP of that stream fills the hole". Add, from
  PRD-01 section 13 invariant 4: a GAP with unknown loss (`lost` 0) must sit at the hole, and the
  hole's size is then reported, not checked; `conn` streams have no DATA, and their GAPs (`off` 0)
  are exempt. Without it a contributor who recounts from the page flags a valid unknown-loss hole as
  a break. Doc accuracy only; the code and PRD-08's checks follow PRD-01 already.
- **Two repeats go** (review B5): (a) in "The seal and events", the recovered-seal sentence (COUNT,
  `recorder_down` GAP, seal, LINK, frame end, `R` line) becomes "a recovered seal is the `seal` event
  inside the recovery frame (see Recovery)", so the Recovery bullet is the one source; (b) in "A
  Python reader", "On a live entry its counts are a lower bound" becomes only "(see the `zstd` note
  above)", since the note says it more exactly (the CLI "may" stop short, also on an entry not yet
  recovered). About 3 lines saved.
- **Size against PRD-15's own estimate** (review B5). Builder notes say "about 280" for the page; it
  is 324 before the two last sections, so the finished page lands about 30% over. PRD-00 "Size
  budget" has no docs row, so this is not a ledger L2 ceiling. If the page is still well past 280
  after those sections, the builder writes a replan `decision` line naming what the extra lines serve
  (the record tables PRD-01 requires, the worked example, the reader) rather than trimming required
  content. The reader is 51 lines, under PRD-01's 60.
- **What waits for PRD-02's merge** (builder decisions, lane log 11:48Z). The page leaves out the
  numbers no merged code holds: the 256 KiB DATA piece, the 1 s link cadence, the 60 s COUNT and the
  141 MiB compressor memory; `cursors.json` and `counts.jsonl` are named only as local recorder
  files. The slice after the core merge (the one with the AGENTS.md map) adds these to the page from
  `crates/goalcup` (never from a PRD draft) and rechecks the Recovery bullet against PRD-02's recovery code. GAP
  reasons 8 and 9 stay "reserved, shown as `other(n)`" (PRD-00 line on reserved reasons; the format
  crate has no names for them). The `PartOptions` field is `rotate_after_ns`, not PRD-01's
  `rotate_after`; the page follows the code.
- **The worked-example dump is cited as `crates/goalcup-format/src/dump.rs`** while `goalcup-verify`
  is not on integration. When PRD-08 merges, the page's "made by" line names `goalcup-verify dump` if
  the binary is what produced the committed `worked-example.dump.jsonl`; the file stays where it is.
- **The `zstd` CLI stops short on a live open frame** (review A, lane log 11:56Z): the reader then
  loses the last flush interval. The page already says so and sends live entries to
  `goalcup-verify dump`. The guide (third slice) keeps that: a participant reading a live entry by
  hand uses `dump`, not the CLI pipe.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-07 slice 2 (merge 2f4bc6a0)

- **`docs/SERVER-DESIGN.md` can move to `docs/archive/` now.** `server/worker/README.md` holds its
  still-true facts on integration at 2f4bc6a0 (PRD-07 seam line 11:46Z, the log line the builder notes wait for). Links
  to repoint in the same slice: `README.md:306`, `docs/ARCHITECTURE.md:68` and `:251`,
  `docs/OPERATING.md:6`, `docs/README.md:22`. `docs/ARCHITECTURE.md:68` also says the upload server
  "signs receipts and acknowledgements", false since 2f4bc6a0. The two `crates/goalcup-cli/src/upload/`
  comments that cite SERVER-DESIGN go with the cut, not here.
- **`docs/UPLOAD-API.md` line 15 is now true** (signing, `ack`, `last_ack`, receipt, abandon and the
  alias are gone from the Worker); no doc edit follows from it.

### 2026-10-30, replan after PRD-15 slice 3 (merge 4e0ca793)

Slice 3 merged `TODO.md` for 1.0 (59 lines, was 350) and repointed one README link. `gate fast`
passed in 98 s. One nit came after the merge (review A7); it goes to the third slice, not a separate
merge request:

- **The after-merge purge must break no live link** (review A7). `README.md:180` now links
  `docs/research/results/2026-10-22-local-test-day.md`, which the TODO purge bullet deletes (only
  four files in `docs/research/` are kept). The keep list does not grow: the third-slice README
  rewrite builds its platform table from merged code and the TODO known issue ("only Linux x86_64 is
  built and tested in this build"), so the Linux arm64 "measurements" link goes. The same rule holds
  for every live doc the third slice rewrites (guide, RULES, OPERATING, SECURITY, CONTRIBUTING,
  `scripts/README.md`): link into `docs/research/` or `docs/archive/` only the four kept files and
  run 1's report; a still-true fact cites those or stands without a link. Live links into purged
  paths on integration at 4e0ca793: `README.md:180`, `README.md:307` (RESEARCH-TARGET and
  `research/coverage-matrix.md`), `docs/PARTICIPANT-GUIDE.md:131`, `docs/RULES.md:158`,
  `docs/OPERATING.md:187` (26 Oct results), `scripts/README.md:28` and `:37` (`LOCAL-CAMPAIGN.md`,
  `replay-comparison.md`; `campaign.py` is cut by PRD-13). `docs/HARNESS-SETUP.md` and
  `docs/CHEAT-CHECKLIST.md` go to `docs/archive/` and need nothing. `docs/DECISIONS.md` lines 820,
  836, 837 and 1017 link research results; it is history in this build, and the after-merge
  DECISIONS rewrite drops those links. The harness pages are PRD-11's (PRD-11 note).
- **The purge itself runs `check-links.py` with the purged files gone** and fixes any live link it
  lists in the same change. That is an after-merge step on the TODO list already; no TODO edit is
  needed for it now.
- **The removed TODO anchors.** PRD-11 dropped the three harness-page links to `#wp0-measure` and
  `#research-recorder-validation` on `v1.0/prd-11-connectors-harness-page-todo-links` (0c0a18d5,
  harnesses log 12:22Z; not yet on integration). `docs/RESEARCH-TARGET.md:48` (`#wp0-measure`) and
  `docs/research/results/2026-10-26-mac-retest.md:91` stay broken: history, untouched, purged after
  the merge. The last link check filters both by name with the run 1 report and says so in the
  night-log line.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-07 slice 3 (merge 323b1a0f)

- **`docs/OPERATING.md` "Test runs" is out of date since 323b1a0f** (review RA-N1). Lines 135-138
  still say the script "touches only `goalcup-traces/traces/test/` and
  `goalcup-traces-staging/traces/test-staging/`" and ask for "R2 read and write". The script now also
  empties `goalcup-usage/usage/test/` and `goalcup-usage-staging/usage/test-staging/`, lists R2 before
  the database changes, and skips a usage bucket the token cannot reach with one line. An owner who
  scopes the token from OPERATING alone leaves the test stats behind, which breaks PRD-07 section 10's
  "leaves no orphan stats". The third-slice OPERATING rewrite (section 9, item 4 "Test runs and the
  pad", and item 1 "Where things run") says, in plain words: the four bucket and prefix pairs; the
  token needs R2 read and write on both traces buckets and on each usage bucket that exists; a usage
  bucket skipped by a run with `--older-than` keeps its test objects until a run without
  `--older-than` (PRD-07 note, RA-N2). Copy the facts from `server/worker/README.md` "Clean up test
  runs" and the script header as merged, not from this note. If the rewrite is not reached in this
  build, the two-line fix to lines 135-138 goes in alone; it is a PRD-15 file.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 4 (merge 7be8bf98)

Slice 4 moved `docs/SERVER-DESIGN.md` to `docs/archive/` byte for byte, repointed its five live
links and dropped the signing claim from ARCHITECTURE section 10 (fix bdcea297, review B1).
`gate fast` passed in 100 s. What follows goes to later slices, not a separate merge request:

- **The ARCHITECTURE rewrite drops every server-signing claim** (review A-N3). Still on integration at
  7be8bf98: `docs/ARCHITECTURE.md:31` ("server's signed receipt and acknowledgements"), the glossary
  at `:276` ("the server's signed start receipt") and `:277` ("the server's signed note that it stored
  the chain"); the `Signed` anchor at `:183` and `:303` describes the old verifier and goes with it.
  `server/worker/README.md` says the server signs nothing. The section 10 rewrite (after the PRD-02
  core merge) builds its terms list from merged code, so none of these survive; the reviewer of that
  merge greps the new page for `signed receipt`, `signed note` and `acknowledg` and expects none
  that claims the server signs.
- **The same claim in the other third-slice pages.** `SECURITY.md:69` (the server "signs start
  receipts and acknowledgements with an Ed25519 key") is false today; `README.md:130` and `:155`,
  `AGENTS.md:52` and `README.md:171` (glossary words receipt, ack) and the receipt, ack and
  `RECEIPT_SIGNING_KEY` lines in `docs/OPERATING.md` (107, 303 to 312, 333, 489, 675 to 692, 711,
  751 to 777) describe the old client or server. The builder left them on purpose for the README,
  SECURITY, AGENTS map and OPERATING rewrites. Each rewrite drops them, citing merged code; none is
  fixed alone unless that rewrite is not reached in this build, in which case `SECURITY.md:69` gets a
  one-line fix first because it is a false statement about the server today.
- **Files moved into `docs/archive/` keep their old relative links** (reviews A-N1 and B2, skipped by
  the fixer as plan text). `docs/archive/SERVER-DESIGN.md` has 8 broken relative links, 8 of the 40
  the box link check listed at bdcea297. The last link check leaves moved archive files out by name,
  as it does run 1's report, and says so in the night-log line. Later moves (CONTRACTS, TERMINATION,
  HARNESS-SETUP, CHEAT-CHECKLIST) follow the same rule: move byte for byte, fix only the live links
  into them.
- **Still waiting, unchanged:** the HARNESS-ONBOARDING pointer (after PRD-12's skill rewrite), the
  RECORD-FORMAT nits (check --json and usage slice), the AGENTS.md map, ARCHITECTURE and
  RECORD-FORMAT numbers (PRD-02 core merge), CONNECTOR-SCHEMA (PRD-11 schema), and the third slice.
  The two `crates/goalcup-cli/src/upload/` comments that cite SERVER-DESIGN go with the cut (PRD-13).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-07 slice 4 (merge c4ada44a)

- **For the RA-N1 fix to `docs/OPERATING.md` "Test runs"** (lines 135-138, still unchanged at
  c4ada44a): copy the usage-bucket caveat from `server/worker/README.md` "Clean up test runs" (around
  line 394) or the script's skip line, not from the script header. The header (lines 14-15) does not
  yet say that a skipped usage bucket keeps its test objects until a run without `--older-than`
  (PRD-07 note, review A-N1). The usage skip rule is now plain `403` or `404`.

### 2026-10-30, replan after PRD-11 slice 2 (merge b9fa50f8)

- **`docs/CONNECTOR-SCHEMA.md`** can be written from `crates/goalcup/src/connector/schema.rs` after
  PRD-11's next merge, which changes three rules: `no_prompts` and `update_off` files under `{home}`
  only (F1b); a `.jsonc` file read as JSON with comments; `"true"` and `"false"` written as booleans.
  Also from the PRD-11 note of b9fa50f8: a pattern ending in `**` is refused (PRD-04's check); a load
  error names the file and the line or key, never a value; built-in connectors are one TOML file plus
  one line in `builtin.rs`.

### 2026-10-30, replan after PRD-14 slice 1 (merge 22fe5926)

- **Docs still name deleted scripts.** PRD-14 deleted `scripts/linux-check.sh`,
  `scripts/test_linux_check.py` and `.github/workflows/linux-x86_64.yml` (and earlier
  `check-version.py`, `ci-scope.py` and their tests). On integration at 22fe5926 these still name
  them: `README.md:292` and `:294` (the release steps: now `gate full`, then
  `scripts/pad/table --release-check <commit>`, then `release.sh tag`), `docs/OPERATING.md:50`, `:145`,
  `:534` and `:540-541` (CI is one job that runs `gate fast`; there is no scope step and no Linux
  check), `scripts/README.md:6`, `:14`, `:15`, `:17`, `:61`, `:62` and `:68`, and
  `docs/research/LOCAL-NEXT-STEPS.md:27`, `:31` and `:172`. `packaging/README.md` was fixed in the
  slice. `docs/CONNECTOR-SCHEMA.md` and the skill template name the platform `linux-x86_64`, not the
  workflow; they are not stale for this reason.
- `README.md` "The gate": the first measured `gate fast` with the script steps is 87 s on 22fe5926 in
  a build box. CI has not yet finished a run on integration (PRD-14 note), so there is no CI time to
  quote.

### 2026-10-30, replan after PRD-15 slice 5 (merge ea0a0fec)

Slice 5 rewrote `docs/CONNECTOR-SCHEMA.md` for 1.0 from the merged `crates/goalcup/src/connector/`
code: 113 lines against the "about 90" of the docs table (the 21-row key table, the mask grammar and
eight **(next slice)** marks carry the extra lines). `gate fast` passed in 83 s. Two review nits stay
open; both go in the next PRD-15 slice that touches these files, not a separate merge request:

- **A10, `continue` and the file flags.** `CONNECTOR-SCHEMA.md` lines 97-100 say `start` and
  `continue` both take `--connector` and `--hosts`, then give only the start rule. `continue` has no
  `--practice` or `--test`; its rule (PRD-10 section 1 command table, and the `main.rs` help "The
  connector file of the run's first part, if it used one") is that the two file flags match the run's
  first part. Add that clause, marked **(next slice)** while `continue` is still `not_built`, and drop
  the mark when PRD-10 merges `continue`.
- **B12, two links to a page without the fact.** `docs/ARCHITECTURE.md:118` (the run home's `.profile`
  that exports `PATH`) and `docs/HARNESS-ONBOARDING.md:562` ("pinned, or `--dev` only") now link the
  bare CONNECTOR-SCHEMA page, which says neither; it says "Nothing is pinned". Delete the two
  parenthetical links. The `.profile` fact is 1.0 behaviour that arrives with PRD-11's launch slice
  (`launch.rs`, PRD-17 P11); the ARCHITECTURE rewrite states it from that merged code, not from
  CONNECTOR-SCHEMA. The pin and `--dev` text goes with the HARNESS-ONBOARDING rewrite.
- **The (next slice) marks.** When PRD-11's next slice (`hosts.rs`, `hints.rs`, `launch.rs`) merges,
  PRD-15 drops the eight marks and their "today" clauses and checks each rule against the merged
  files (reviews A7, B8; the fixer's seam line at 13:21Z). If that slice is not reached in this build,
  the marks stay; they are true of the code.
- **Known broken link:** `docs/CONTRACTS.md:615` points at the removed `#pin-evolution` anchor. It is
  left on purpose: CONTRACTS moves byte for byte to `docs/archive/` in the third slice, and the last
  link check names it in its filter list if the move has not landed.
- **Still waiting, unchanged:** the RECORD-FORMAT `check --json` and usage sections (PRD-08, PRD-09),
  the AGENTS.md map, ARCHITECTURE and RECORD-FORMAT numbers (PRD-02 core merge), the
  HARNESS-ONBOARDING pointer (PRD-12's skill rewrite), and the third slice.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 1 (merge d10feef7)

- **RECORD-FORMAT's `check --json` section can start.** The S6 shape for the integrity ids is on
  integration (`crates/goalcup-verify/src/report.rs`, field list in the PRD-08 note); the later ids
  are in the report as `skip` "not built in this slice" and the page marks them so. `usage` waits for
  PRD-09.
- **The worked-example "made by" line** may name `goalcup-verify dump`: it uses
  `goalcup_format::dump::Dumper` and its output on the worked example equals the committed file as
  JSON (judge 12:35Z box).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 2 (merge 56495dd6)

- `skills/goal-cup/SKILL.md` has no MCP paragraph now. PRD-10 deletes `skills/goal-cup-setup/` only
  after nothing links it, because CI runs check-links. Integration links it from four places:
  `README.md:20`, `docs/PARTICIPANT-GUIDE.md:9`, `docs/README.md:14` and `AGENTS.md:40` (the
  `skills/` map row). PRD-15's rewrites of these pages drop the links. The merge note of the slice
  that removes the last one says so, so PRD-10's delete can follow.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 6 (merge 9e89a22d)

Slice 6 closed A10 (the `continue` file-flag rule in `docs/CONNECTOR-SCHEMA.md`, marked
**(next slice, ruled in PRD-10)** until `continue` merges) and B12 (the two bare CONNECTOR-SCHEMA
links). It added the `check --json` report section to `docs/RECORD-FORMAT.md` from the merged S6 code
(`crates/goalcup-verify/src/report.rs`), closed the slice 2 nits A-N5 and B5, and named
`goalcup-verify dump` (on the `zstd -dc` copy) as the worked example's maker.

- **Open nit B-N4, the reader intro.** `docs/RECORD-FORMAT.md` lines 263-265 end "and a total (see
  the `zstd` note above)"; the pointer no longer says why the note matters. The next slice that
  touches the page (the usage section, or the PRD-02 numbers) changes it, with no new line, to "and a
  total, a lower bound on a live entry (see the `zstd` note above)", or drops the parenthetical.
  Not a separate merge request.
- **The `check --json` section follows the code.** It lists ten check ids as always `skip` "not built
  in this slice" and `server`, `harness_log`, `provider_errors`, `history` and `threads` as not
  filled. Each later PRD-08 slice that builds one of them names it in its merge note (PRD-08 note),
  and the next PRD-15 slice moves it out of the skip list or fills its row, rechecked in a box against
  `check --json` output as slice 6 was (key sets, check ids, `inv` numbers, skip list). If a check is
  not built in this build, its line stays; it is true of the code.
- **The `continue` mark** goes when PRD-10 merges `continue` (PRD-10 note); until then it stays.
- **Size:** the page is 374 lines against the docs table's "about 280" (134%). The builder's decision
  line (14:36Z) names what the lines serve: the PRD-01 record tables, the worked example, the 51-line
  reader and the S6 key tables. The usage section adds to it; that slice states its own count and
  cuts repetition before adding. Docs have no PRD-00 size row, so this is not a ledger L2 ceiling.
- Review items A5, A6 and N3 were declined by the fixer (14:48Z) and stay closed.
- **Still waiting, unchanged:** the usage section (PRD-09 core), the AGENTS.md map, ARCHITECTURE and
  RECORD-FORMAT numbers (PRD-02 core merge), the HARNESS-ONBOARDING pointer (PRD-12's skill rewrite),
  and the third slice.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-15 slice 7 (merge 938f6733)

Slice 7 dropped the four live links to `skills/goal-cup-setup/` (`README.md`, `docs/PARTICIPANT-GUIDE.md`,
`docs/README.md`, the `skills/` row of the `AGENTS.md` map). The cross-lane ask in the PRD-10 slice 2
note is closed; PRD-10's delete of the skill can follow (PRD-10 note).

- **One link stays, as history (A-N1).** `docs/archive/PARTICIPANT-GUIDE-0.6.md:13` still links
  `../../skills/goal-cup-setup/SKILL.md` and breaks when PRD-10 deletes the skill. It is not edited:
  the archive is history. PRD-15's last `check-links.py` run (before PRD-14 deletes the script) names
  it in its filter list next to `docs/CONTRACTS.md` to `#pin-evolution`, and acceptance lists both as
  known broken links in history docs.
- **Correction to the PRD-10 slice 2 note.** Its premise "because CI runs check-links" is not true of
  integration: neither `scripts/gate.sh` nor `.github/workflows/ci.yml` runs `scripts/check-links.py`
  (PRD-14 took it out of the gate in phase 0). Only PRD-15's own runs look at links. Nothing waits on
  link checks in CI.
- **The `AGENTS.md` map row** now reads "`goal-cup` (copied into the harness home) and
  `adding-a-capture-harness` (contributors)". The map rewrite (after the PRD-02 core merge) keeps it
  true of PRD-12's skill rewrite.
- **Still waiting, unchanged:** the usage section (PRD-09 core), the AGENTS.md map, ARCHITECTURE and
  RECORD-FORMAT numbers (PRD-02 core merge), the HARNESS-ONBOARDING pointer (PRD-12's skill rewrite),
  the `continue` mark (PRD-10 `continue`), the CONNECTOR-SCHEMA (next slice) marks, nit B-N4, and the
  third slice.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 2 (merge 669f5d92)

- For `docs/RECORD-FORMAT.md` (the `check --json` section), from PRD-08's merge note. Under
  `--server`, the three ids `server_chunks_whole`, `server_equals_local` and `server_counts_match`
  are built and no longer "not built in this slice". The top-level `server` object
  `{chunks, whole, equals_local, db_match}` is filled. New exit rules: a missing `fetch.json` under
  `--server` gives all three `fail`; `final` null gives `server_chunks_whole` `named` (exit 1); a
  `--local` path that never held a record exits 3; a seal with no `reason` or `totals` is reported,
  never exit 3. The seven local ids stay `skip` "not built in this slice" until their slices.
  PRD-15's next slice updates the page; no code change is asked of PRD-08.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 3 (merge c14d63b8)

- PRD-11's merge c14d63b8 makes real every rule `docs/CONNECTOR-SCHEMA.md` marks **(next slice)**:
  the version probe (first non-empty line of `program --version`, 5 s, a prefix the line must
  contain), `refuse_if_set` (`false` is a value; null, `""`, `[]` and `{}` are unset),
  `host_hints` and `hint_env` (https URLs only, userinfo dropped, a loopback value is a
  `local_model` note), a pattern ending in `**` refused by PRD-04's `pattern::parse`, the settings
  files (every `.json` and `.jsonc` read with comments and trailing commas; booleans written typed;
  dotenv not writable), host decisions and the full host check. Also state: a `file` template's last
  component is a pattern, and every file that fits is read. Drop the marks and check each rule against
  merged `hosts.rs`, `hints.rs`, `launch.rs` and `mod.rs` (PRD-11 note lists the facts).

### 2026-10-30, replan after PRD-10 slice 3 (merge f66ce4a5)

`goalcup start` now runs the folder checks of PRD-10 section 3 step 2 (`crates/goalcup/src/cli/folder.rs`,
messages in `cli/text.rs`). `docs/PARTICIPANT-GUIDE.md` (section 3 step 3 and the appendix "if
`goalcup start` stops") differs from the merged code here; the next PRD-15 slice checks each line
against `folder.rs` and fixes the guide, without quoting message wording the code may change:
- A synced folder is a **warning**, not a stop (PRD-10 section 3); the appendix lists it among the
  rows that stop the start. It is found by name on the resolved path (Dropbox, OneDrive, Google
  Drive, iCloud, `Mobile Documents`, `Library/CloudStorage`), so a Desktop or Documents folder that
  iCloud syncs without one of those names in its path gets no warning. The advice "do not use Desktop,
  Documents" stays true as advice.
- A folder on NFS or SMB (the project or `GOALCUP_HOME`) is a warning; the guide does not say so.
- Not in the appendix yet: inside a Git work tree; the project and goalcup's own folder hold each
  other or sit side by side (fix: another folder, or `GOALCUP_HOME` elsewhere); the `--prompt` file
  inside the project folder or missing.
- The token line is checked in `.env.local` only, and refuses two `GH_TOKEN` lines or a CRLF ending
  there; the guide's "any second `GH_TOKEN` line in any `.env` file" says more than the check.
- The missing-`.gitignore` case: the code asks for the line `.env*`; it does not print the two lines
  `.env` and `.env.*` the guide names.
- "The folder is checked once more right after you approve": not built and not in section 3. PRD-10's
  start slice says in its merge note whether it adds it; until then the sentence is not true.
- "Every row but the outage row ... says Nothing was created": true for the folder refusal. A refusal
  from the connector plan (home in use, harness version) may come after the isolated home was made
  (PRD-10 note), so that wording waits for the start slice.
- WSL1 or a Windows drive under WSL2 is not checked yet (PRD-10 note); the guide's advice stays.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-10 slice 4 (merge a5d7ddc3)

- **`skills/goal-cup-setup/` is deleted.** The only link left is `docs/archive/PARTICIPANT-GUIDE-0.6.md:13`,
  which this PRD keeps as history (the PRD-10 note after PRD-15 slice 7); `check-links.py` now counts it as broken
  (39 to 40 in the slice's box), and no CI step runs that script. The fixer's 16:18Z line on it needs
  no owner answer.
- **The guide's sleep row does not quote the CLI.** `docs/PARTICIPANT-GUIDE.md` line 324 reads
  `this computer may go to sleep (…)`; `text::no_sleep_lock` prints "This computer may sleep during
  the run (no sleep lock: <why>). Keep it on mains power ...", with `<why>` one of "caffeinate is
  missing", "set Windows sleep to Never" or "no systemd-inhibit with a running systemd". The next
  PRD-15 touch of the message table quotes the start of the printed line.
- **`README.md` line 237** says `goalcup setup --harness claude-code-cli` signs in to the harness. On
  integration, setup without `--yes` asks, opens and writes nothing (the interactive setup is
  PRD-10's next setup slice). The line stays; if that slice has not merged by PRD-15's last pass,
  the line says what setup does then.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-11 slice 4, part 1 (merge a96d9f71)

- **`docs/CONNECTOR-SCHEMA.md` line 24 is now wrong.** The `version` key is no longer text the line
  must contain: a word of the first non-empty line of `program --version`, with a leading `v`
  stripped, must start with it (`mod.rs` `plan`, `held`). The next PRD-15 touch rewrites that cell
  from merged `mod.rs`.
- **`.profile` is merged** (`launch.rs` `write_profile`, called from `mod.rs` `plan`): written only
  where `home` sets `HOME`, replaced only when goalcup wrote it, otherwise kept with a log row.
  CONNECTOR-SCHEMA's `home` row or the ARCHITECTURE rewrite states it (earlier note).
- **`docs/harnesses/README.md` holds the one 15-row harness table** (PRD-11 owned). PRD-15 pages
  that list harnesses (README, guide, HARNESS-ONBOARDING) link to it and copy no rows; tiers come
  from `connectors/tiers.toml` (PRD-12). The five per-harness sheets stay in the old shape until
  PRD-11's last slice; a PRD-15 page that quotes a sheet quotes nothing that slice will rewrite.
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-30, replan after PRD-08 slice 3 (merge 7a995b6e)

- **For `docs/RECORD-FORMAT.md`** (PRD-08 note): `--known` is now read (its line at :335 says it is
  not). With it, `check` adds a `secrets` check (`inv` null) to every entry, result `pass`, `warn` or
  `fail`, with facts `known_hits`, `shape_hits`, `glued_shape`, `masks`, `mask_bytes` and
  `big_masks`. The `secrets` subcommand writes one JSON report on stdout (keys and exit rules in the
  PRD-08 note) and has no `--json` flag. Document both in PRD-15's next slice, by names, not by
  copying the PRD text. The participant guide's line on what can still be recorded keeps "the scan
  flags a key glued to a word" (`glued_shape`, a `warn`).
- No locked ledger item is touched and no owner question comes from this slice.

### 2026-10-31, replan after PRD-15 slice 9 (merge d56216f4)

Slice 9 brought `docs/RECORD-FORMAT.md` to PRD-08 slice 3 and `docs/CONNECTOR-SCHEMA.md` to PRD-11 slice
4; rewrote `docs/OPERATING.md` (863 to 233 lines, under the 280 estimate) from the merged Worker, pad,
gate and release code; rewrote `scripts/README.md` (39 lines) for the PRD-13 pre-cut; and moved
`docs/CONTRACTS.md` and `docs/TERMINATION.md` to `docs/archive/` with no edit (100% renames).

- **Next RECORD-FORMAT slice (A2-N1).** PRD-08 slice 4 (1fdaf2c3) is on integration and builds
  `listed_hosts_decrypted`, `conn_bytes` and `mask_failclosed` (`honesty.rs`). `docs/RECORD-FORMAT.md`
  lines 365-366 still say those three are "Always `skip` with not built in this slice". The next slice
  describes the three honesty checks by id and cuts that skip list to the four ids `report.rs` still
  holds as LATER (`history_complete`, `wire_keeps_up`, `threads_match`, `subagents_present`). Re-read
  `report.rs` first: PRD-08's remaining slices may shorten it again.
- **OPERATING nit B-R1.** The last sentence of the recording-off section ("A query that lists such
  runs across the event is not in this build.", `docs/OPERATING.md:169`) is build-state text in a
  current doc (rule 7). Delete it in the next slice that touches OPERATING; the sentence before it
  already points at `heartbeats` in `fetch.json`.
- **RECORD-FORMAT size (B-R2), decision.** 419 lines against the 280 estimate (150%). What the extra
  lines serve: the `check --json` report section with every check id and its facts (point 5: the
  server rebuilds the story and an organizer reads this report), the `secrets` check and subcommand
  (point 4), and the S6 key sets the Worker and the verifier both read. All three are reference tables a
  reader looks up by id; none repeats PRD text. No behaviour-keeping cut was found in review. The
  ceiling for this page is 430 lines. The A2-N1 edit should come out line-neutral or shorter (it
  replaces a 3-line skip list with a shorter one and adds a short honesty paragraph); if it needs more
  than 430 in total, cut the per-check fact lists to the ids and point at `report.rs`.
- **Two build-progress marks stay until `continue` merges (B-R3).** `docs/CONNECTOR-SCHEMA.md` line 9
  (the intro: `start` and `continue` not built) and the bold mark near line 100 ("next slice, ruled in
  PRD-10: `continue` is not built yet"). `start` is still `not_built` on integration (`cli/start.rs`
  prints it) and `continue`, `recover`, `seal`, `upload` are `not_built` in `main.rs`. Delete both marks
  in the final link and wording pass, when PRD-10 has merged `start` and `continue`. They wait in the
  same slice as the usage section.
- **Usage section is unblocked on the code side.** PRD-09 slice 4 (`out.rs`, `usage::run`, c262ef5a) is
  on integration. The OPERATING usage job (left out of the slice 9 rewrite) and the S7 summary and
  calls files can be written from the merged code in the next slice, after PRD-09's own replan note.
  Rule: read `usage/out.rs` for the file names and the CLI flags; do not copy PRD-09 text.
- **Known broken links for the filter list** (history docs, not edited): `OPERATING.md#the-matrix` in
  `docs/LOCAL-RESEARCH-HANDOFF.md:135`, `docs/research/LOCAL-CAMPAIGN.md` and `LOCAL-TEST-DAY.md`;
  `docs/CONTRACTS.md` to `#pin-evolution` (now `docs/archive/CONTRACTS.md:615`); and
  `../../skills/goal-cup-setup/SKILL.md` in `docs/archive/PARTICIPANT-GUIDE-0.6.md:13`. Links to the
  moved files from `docs/CHEAT-CHECKLIST.md:91,:305` (moves unedited) and `docs/DECISIONS.md:206,:235`
  (D22 and D35: a path-only fix is the one allowed edit to the append-only file, done in the DECISIONS
  slice) are also known. Acceptance lists these as known broken links in history docs.
- **Still waiting, unchanged:** the AGENTS.md map and README rewrite (PRD-02 recorder `run`, PRD-10
  `start`), ARCHITECTURE and RECORD-FORMAT numbers (PRD-02 core), the HARNESS-ONBOARDING pointer
  (PRD-12's skill rewrite; `skills/adding-a-capture-harness/writing-a-decoder.md` is still there), the
  guide, RULES, SECURITY, TODO and the DECISIONS entries, and the third slice with the newcomer run.
- No locked ledger item is touched and no owner question comes from this slice.
