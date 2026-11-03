# Lane K: docs and decisions

Appendix to the 30 Oct audit. Repo `/home/user/goal-cup-proxy`, commit 4b226de2 (origin/main). Read-only. Nothing
was built or run. Line counts for docs are `wc -l` of Markdown; code line counts say src or tests. This file merges
two parts: the entry-by-entry audit of `docs/DECISIONS.md` and the audit of every other doc.

## Lane summary

The docs are 82 Markdown files and 17,472 lines, plus `docs/DECISIONS.md` (1,162 lines, 57 entries, D1 to D74 with
gaps). About 60% of the doc lines are history (dated plans, test-day handovers, fix plans v1 to v10, status notes
from 21 and 22 Oct) that git already keeps and that no participant, operator or contributor needs on 6 Nov. The
target is 31 files and about 7,000 to 7,500 lines, and DECISIONS shrinks from 57 entries to about 20. The bigger
problem is not size but truth: the docs we keep still promise things the owner decided on 29 Oct to drop (the disk
pause, the v5 freeze, the connector compatibility promise, the 32-stream and 1024-file native caps, the email
masking rule), and three kept docs say things that are already false on main. DECISIONS has the same conflicts at
the rule level (D37 clause 3, D40 reserve, D42, D43 ruling 6, D54, D57 cap clause, D58, D63, D65, D68, D69, D73),
and the owner decisions themselves live only in a plan file. Fix the false lines now, change each conflicting line
in the same PR as the code it describes, then delete history in one PR and merge overlapping docs in another.

## Top findings

1. **The owner decisions of 29 Oct live only in a plan file** (`docs/plans/2026-10-29-record-everything-progress.md:80-100`).
   No D entry covers masking option b, the 64 GiB `RUN_MAX_COMPRESSED_BYTES` limit, or "no byte-for-byte match".
   Move them into VISION (a short "Decided" list) and DECISIONS before any plan is deleted.
2. **Disk pause is promised in kept docs and in DECISIONS.** D65 (SIGSTOP on a nearly full disk) and the D40 3 GiB
   reserve conflict with O4 ("Disk full: no special handling") and point 3. The pause is promised to participants in
   6 kept docs (`AGENTS.md:84-86`, `README.md:79-80`, `docs/RULES.md:322-325`, `docs/PARTICIPANT-GUIDE.md:242-243,324-325`,
   `docs/OPERATING.md:457`, `docs/UPLOAD-API.md:380,388,399-407,554,586,595`). Code at stake: about 340 src and about
   2,150 test lines (unverified total).
3. **"Protocol v5 is frozen" fills whole sections** of kept docs (`docs/CONTRACTS.md:510-571`,
   `docs/CONNECTOR-SCHEMA.md:94-159,957-984`, `docs/ARCHITECTURE.md:191-194`) and five decisions (D42, D43 ruling 6,
   D54, D63, D68). O2 says it no longer applies. The v5 corpus (99 fixture files, 564 KiB; `v5_corpus.rs` 718 test
   lines) exists to hold it.
4. **Caps of our own are written up as normal**: 32 native streams and 1024 tracked files
   (`docs/harnesses/claude-code-cli.md:163-170`, `docs/HARNESS-ONBOARDING.md:198-202`, D57), 128 live producers and
   streams (D58, `crates/goalcup-protocol/src/capture.rs:58-66`), 10,000 tunnel rows and 8 MiB (D74), and five
   recorder restarts, after which calls are refused (stated in no kept doc, so a participant is never told).
5. **Three kept docs are already false on main**: "no header is recorded" (`README.md:150-151`,
   `docs/ARCHITECTURE.md:212-213`; header streams exist since Task 7), "automatic recording proxy restart: not built"
   (`docs/ARCHITECTURE.md:253-256`; built in Task 10), and "Hermes 0.21.1 and later: model calls only"
   (`docs/HARNESS-SETUP.md:194-195`; `state.db` is copied at finish since Task 6).
6. **DECISIONS breaks its own removal rule.** D44 and D50 are cited by no file outside DECISIONS; D49 has one test
   cite; eleven entries are "partly superseded", so one rule (Hermes) needs seven entries (D40, D41, D61, D62, D72,
   D73, D74). D68 is "proposed" yet a shipped connector leans on it (`connectors/claude-code-cli/harness.toml:107`).
7. **Stale decision cites in code**: D12 (`crates/goalcup-cli/tests/common/support/mod.rs:15,334`), D7 on a likely
   dead V1 `AttachOnly` (`crates/goalcup-protocol/src/manifest.rs:28-31`), D23 (`crates/goalcup-cli/src/launch.rs:13`),
   "D7" and "D1" reused as test labels, and D48 history comments in 25 files.
8. **Masking option b is not reflected anywhere yet**: the email rule (`crates/goalcup-daemon/src/redact/stream.rs:148,386,541`)
   has no D number; the Codex rate-limit rows cite D26 and D39 (`connectors/codex-cli/harness.toml:102-110`);
   `docs/CONNECTOR-SCHEMA.md:886` lists email in the shared set and uses `codex_plan_type` as its example (`:205`, `:899`).
9. **Docs are pinned in place** by the binary (`crates/goalcup-cli/src/main.rs:59`, `commands/prep.rs:896`), by a
   compiled-in skill (`crates/goalcup-cli/src/skill.rs:18`), by about 90 code-comment cites, and by
   `scripts/check-links.py` in CI. Every delete or merge must fix links and comment cites in the same PR.
10. **The 2-hour run report** (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any of the 89
    remote branches. No decision or doc was checked against it. Do not cite it until it exists.

## Where the two parts overlap or disagree

| Topic | Decisions part | Docs part | Resolution here |
|---|---|---|---|
| OpenCode facts sheet and D44 | `docs/harnesses/opencode-cli.md:5-6,55` (branch `origin/opencode-openclaw`) still names D44 as the stop rule; that stop is gone. | "No conflict lines found" in `opencode-cli.md` and `openclaw-cli.md`. | Both stated. The docs part looked for the owner-decision conflicts; the decisions part found a D44 reference. The sheet is branch-only and must not be edited by this audit. |
| `CONTROL_API_VERSION` freeze to 15 Nov | D70: restart the pin at the Friday release; keeping it from Friday through the cup is fine, since a forced update over a live run is a real cost. | `docs/research/RELEASE-IS-THE-LIST.md:8,52` "frozen ... to 15 Nov" conflicts with no backward compatibility; `TODO.md:246` waits for "the next `CONTROL_API_VERSION`". | Both stated. They agree that the 0.7.0 to 15 Nov freeze goes; the decisions part keeps a new freeze from the Friday release to the end of the cup. |
| Headless or one-shot harness modes | D39: delete the headless clause; D57: keep chat mode in the merged D52. | `docs/HARNESS-ONBOARDING.md:191` "Not planned" may conflict with point 7 if people run `codex exec` or `claude -p`. Owner question, unverified. | Both stated. Owner to decide before the D52 merge locks chat mode in. |
| Schema-1 fixture count | D68 text says five frozen fixtures; the folder holds six (`crates/goalcup-cli/tests/fixtures/schema-1-connectors/`). | `docs/CONNECTOR-SCHEMA.md:118-127` says six frozen fixtures. | Six on disk. Both go if D68 retires. |
| Disk reserve and 5 GiB warning | Retire the D40 3 GiB reserve. | Lines on the reserve and warning (`docs/PARTICIPANT-GUIDE.md:324`, `docs/OPERATING.md:691`, `docs/ARCHITECTURE.md:146,245-249`) are "conditional" on the code lane (lane F-os recommends cutting the reserve). | Agree in direction; the doc change follows the code PR. |
| Target file count | n/a | Summary says 31 files: 8 root, 10 in `docs/`, 5 facts-sheet files, 5 skill files, 3 sub-READMEs. Section 3 counts "facts sheets (4 on main, 2 more on the branch)" and "3 skills". | Both counts kept. The 5 and 5 read as README plus 4 sheets, and 3 SKILL.md files plus `writing-a-decoder.md` and the template. |
| Tunnel list caps | D74: either upload the list as a normal chunk or say what cost the cap guards. | `docs/CONTRACTS.md:543-544`, `docs/CHEAT-CHECKLIST.md:125`, `docs/UPLOAD-API.md:332,337`: say the cost, or raise. | Same verdict. |
| Termination docs | Merge D30 and D40's grace clause into D22. | Merge `TERMINATION.md` into `ARCHITECTURE.md` (about 60 lines). | Consistent: one decision, one doc section. |

## 1. Method

- **Decisions.** Cites counted with `rg -w D<n>` over `crates/`, `server/` and `connectors/` (code, tests, fixtures,
  connector data). "Docs" is the count of other files in `docs/`, `README.md`, `AGENTS.md`, `SECURITY.md`, `TODO.md`
  and `scripts/` that cite the number. Raw per-file list:
  `/tmp/claude-0/-home-user/c3485515-d491-51f1-8b7f-6278b4ef8e37/scratchpad/k_dcites.txt`.
- **Docs.** 82 Markdown files, 17,472 lines: root 1,223; `docs/*.md` without DECISIONS 6,566; `docs/archive` 958;
  `docs/research` 2,545; `docs/research/results` 1,294; `docs/plans` 2,794; `docs/harnesses` 911; `skills` 530;
  the three sub-READMEs 651.
- **Yardsticks.** The seven points (VISION.md) and the owner decisions after phase 2
  (`docs/plans/2026-10-29-record-everything-progress.md`, last section), quoted below as O1 to O7:
  - O1: "We over-engineered. Less code is better: fewer places for things to break. Simplify first."
  - O2: "No backward compatibility. Every run so far is test data. ... "Protocol v5 is frozen" no longer applies."
  - O3: "Subagents: record whatever is running, with no caps of our own. Simplest path: raise or remove the open-stream limits (128 per entry, 32 for native files) and the 1024-file cap"
  - O4: "Disk full: no special handling. The harness stops and the user cleans up."
  - O5: "A byte-for-byte match of subagent files against disk is no longer a requirement."
  - O6: "Masking, option b: add the URL password rule first, then drop the email rule and the Codex rate-limit rules; keep every secret rule. Nothing more."
  - O7 (VISION, How we build): "No caps of our own on subagents, calls or bytes. If a limit is needed, it guards a real cost, and it says so."

## 2. Target doc set

The fewest docs each reader needs.

| Reader | Docs (target) | Notes |
|---|---|---|
| Participant | `README.md` (top half), `docs/PARTICIPANT-GUIDE.md`, `docs/HARNESS-SETUP.md`, `docs/RULES.md`, `SECURITY.md`, `skills/goal-cup-setup/SKILL.md` | The guide absorbs the message table from `research/NOTIFICATIONS.md` (point 6) and the "what to do at a 24-hour sandbox stop" part of `research/RESTARTS-DESIGN.md`. |
| Operator (organizer) | `docs/OPERATING.md`, `docs/UPLOAD-API.md`, `server/worker/README.md`, `packaging/README.md` | OPERATING takes the runbook parts that today repeat in `server/worker/README.md` (deploy, maintenance, test runs, min version, EVENT_ID switch) and one "Measured runs" table that replaces `research/coverage-matrix.md` and the results folder. `server/worker/README.md` keeps code, config and tests, and absorbs `SERVER-DESIGN.md`. |
| Contributor | `VISION.md`, `AGENTS.md` (+ `CLAUDE.md` pointer), `CONTRIBUTING.md`, `docs/ARCHITECTURE.md`, `docs/HARNESS-ONBOARDING.md`, `docs/CONNECTOR-SCHEMA.md`, `docs/harnesses/*.md`, `docs/REPORT-FORMAT.md` (moved from `research/m2-report-format.md`), `docs/DECISIONS.md`, `TODO.md`, `scripts/README.md`, `skills/adding-a-capture-harness/**` | ARCHITECTURE absorbs the parts of `CONTRACTS.md` and `TERMINATION.md` that are still true after the cut. `SECURITY.md` "Known residuals" absorbs `CHEAT-CHECKLIST.md`. |
| Everyone | `CODE_OF_CONDUCT.md` | Standard file, keep as is. |

Count: **31 files, about 7,000 to 7,500 lines**: 8 root files, 10 in `docs/` (DECISIONS included), the facts
sheets (4 on main plus their README; 2 more on `origin/opencode-openclaw`), 3 skills (5 skill files), 3
sub-READMEs. Delete 45 files, merge 7, move 1. `docs/README.md` goes: the "Docs map" table in `README.md:297-307`
already does its job.

After the event (not before Friday, because of the code cites): merge `CONNECTOR-SCHEMA.md` into
`HARNESS-ONBOARDING.md` as its reference half, and `UPLOAD-API.md` into `server/worker/README.md`.

## 3. Decisions to keep: the short list

From 57 entries to about 20. Keep the numbers (code cites them), delete retired entries, and fix the cites in the
same PR. Each line is the whole rule.

| Keep as | Built from | One-line rule |
|---|---|---|
| D1 | D1, D59 (NODE_USE_ENV_PROXY), D72 (trust entry is not a base URL) | Never change the harness's model URL; record through a loopback proxy reached by the process's own `HTTPS_PROXY`. |
| D2 | D2, D74 (zero native rows) | Two legs, wire and native; a connector may have no native row. |
| D3 | D3 | The recorder captures, redacts, hashes and uploads; it judges nothing. The verifier is separate. |
| D4 | D4, D24 | One run-scoped daemon; one entry per workspace; the control API needs the bearer. |
| D5 | D5 | The agent never sees the evidence folder. |
| D9 | D9, D45 | Loss is a chained gap with a reason; integrity judges the record, not the run. |
| D10 | D10 | One harness per entry; subagents of that harness are free; checked by review, never stopped. |
| D14 | D14, D36, D60, D41's `masked_key`, O6 | Secrets: credentials stripped by name; bodies redacted before disk; the run's own secrets by exact value; the shipped rule list is named here (O6). |
| D16 | D16, D63 | The server's signed receipt fixes start and deadline; it must match the Launch it anchors. |
| D22 | D22, D30, D40 (grace, budget, spawn gate) | How a harness is ended, what "confirmed" means, the finish budget. |
| D25 | D25, D66 | Nothing is pushed to the agent; it may pull the time left; `seal` refuses inside the harness. |
| D26 | D26 (minus capture rule) | No token budget; plan and billing fields are stripped at publication. |
| D37 | D37 (clauses 1, 2, 4, 5) | Autonomous runs; traces are research; a new harness is data; single-user host. |
| D42 | D42, D43 (1, 3, 5, 7), D51 | Connectors are reviewed data under generic rules; closed lists change by a DECISIONS entry. No freeze. |
| D52 | D52, D57 (chat mode, since-start identity), D67 | Run the harness as people run it: chat mode, no limits set, settings recommended, never written. |
| D55 | D55 | The chain rotates into parts. |
| D61 | D61 (no-key core), D69 (holds no keys) | Goal Cup never reads, stores or passes a provider key or git login. |
| D64 | D64 | No server error stops a run or its upload. |
| D70 | D70, D46 | The release is the list; minimum version; `--dev` never competes. |
| D71 | D71, D56, D11 | The project folder is the workspace; how the seal records it, with any limit stated as a time guard. |
| D74 | D74, D62, D72's `[launch_prep]` rule | Which hosts are recorded, and every sealed tunnel is listed. |

Retire outright: D7, D12, D23, D38, D39 (fold the shared-home rule into D52 or D59), D40's reserve, D41, D44, D48,
D49, D50, D53's trade paragraph (keep the field in D3 or D52), D54, D58 (replace with "no live caps of our own; any
bound states its cost"), D65, D68, D69, D73's 0.7.0 branches (keep the resume rule inside D52).

New lines to add (owner decisions with no D entry yet):

- **O6 masking option b.** The shared `email` rule lives in `crates/goalcup-daemon/src/redact/stream.rs:148,386,541`
  with no D number. The Codex rate-limit rows cite D26 and D39 (`connectors/codex-cli/harness.toml:102-110`). The URL
  password rule is new. The merged "secrets" entry (D14) should list the rule set in a few lines.
- **O5 (no byte-for-byte match of subagent files).** No D entry promises it; it is in spec rule 3 and the phase-2
  notes, not in DECISIONS. Nothing to retire here.
- **Server `RUN_MAX_COMPRESSED_BYTES`** (64 GiB, `server/worker/wrangler.toml:59,151`) has no D entry. If the owner
  keeps it, it needs one line saying it guards storage cost (O7). D64 already says a `413` is retried, never a stop
  (`server/worker/src/errors.ts:13`).

## 4. Constraints that decide how far the docs can be cut

Check these before deleting or renaming.

| Constraint | Evidence | Effect |
|---|---|---|
| The binary prints a link to the guide | `crates/goalcup-cli/src/main.rs:59` (`blob/main/docs/PARTICIPANT-GUIDE.md`) | Keep this path and file name. |
| The binary prints a link to the harness setup page | `crates/goalcup-cli/src/commands/prep.rs:896` (`docs/HARNESS-SETUP.md`), plus `participant.setup_section` anchors in connectors (`skills/adding-a-capture-harness/SKILL.md:23-24`) | Keep the path and every `### <Harness>` anchor. |
| The binary links the onboarding page | one `blob/main/docs/HARNESS-ONBOARDING.md` in `crates/*/src` | Keep the path, or change the code in the same PR. |
| `skills/goal-cup/SKILL.md` is compiled in | `crates/goalcup-cli/src/skill.rs:18` (`include_str!`) | Any edit ships in the next release only. |
| Code comments cite docs | 24 cites of UPLOAD-API, 21 CONNECTOR-SCHEMA, 13 HARNESS-ONBOARDING, 11 TERMINATION, 9 CONTRACTS, 8 `research/m2-report-format.md`, 1 `research/RESTARTS-DESIGN.md` (`crates/goalcup-cli/tests/it/restarts.rs:1`), 1 `research/NOTIFICATIONS.md` (`server/worker/sql/goal-cup-tracer-upload.sql:2515`), 1 each event-readiness and release-is-the-list plans (`scripts/campaign.py:101`, `scripts/test_install.py:30`) | A merge or delete must update these comments in the same PR, or they rot. Comments are not checked by `check-links.py`. |
| Code comments cite decisions | About 600 decision cite lines in crates, server and connectors (raw list above) | Merging entries but keeping numbers means cites point at merged entries; deleted numbers must be re-cited in the same PR. |
| Already dead cites in code | `docs/qualification/...` (5 fixtures and tests), `docs/plans/2026-10-21-goal-cup-signup-data-flows.md` and `docs/specs/...` (SQL comments): none of these files exist | Not new, but a doc clean-up should fix or drop them. |
| The link check runs in CI and the full gate | `scripts/check-links.py`, `scripts/gate.sh:110`, `scripts/README.md:16` | Every delete needs its inbound Markdown links removed in the same PR, or CI fails. |
| The version check skips history folders | `scripts/check-version.py:72` (`docs/harnesses/`, `docs/research/`, `docs/archive/`) | Moving `m2-report-format.md` out of `docs/research/` puts it under the version check. It may then flag version strings. Unverified whether it holds any. |
| A Claude Code agent file links the campaign doc | `.claude/agents/campaign-cell.md:3` | Goes with `LOCAL-CAMPAIGN.md` and `campaign.py`. |
| Workload prompts live in `docs/research/*.txt` | `scripts/campaign.example.json:10` | Not Markdown, outside this lane's list, but they go with the campaign kit. |

## 5. docs/DECISIONS.md

Source: `docs/DECISIONS.md` (1162 lines, 57 entries, D1 to D74 with gaps) on commit 4b226de2.

### 5.1 The status table itself (docs/DECISIONS.md:11-50)

The table is dated 28 Oct 2026. It is mostly accurate about which entry replaces which, but:

| Problem | Evidence |
|---|---|
| It does not know about the owner decisions of 29 Oct. Every entry it marks "current" that freezes v5, keeps old clients working, pauses on disk or caps files is now in conflict (see 5.3). | progress doc, owner section |
| The doc's own rule says "Entries for rules that no code or doc cites any more are removed". D44 and D50 are cited by no file at all outside DECISIONS (D44 only by the OpenCode facts sheet on branch `origin/opencode-openclaw`). D49 has one test cite. | `rg -w D44`, `rg -w D50` over the repo: no hits |
| D68 is "proposed, owner to approve" but a shipped connector already leans on it. | `connectors/claude-code-cli/harness.toml:107` |
| "Five frozen schema-1 fixtures" (D68) but the folder holds six. The text "D74 added `schema1-wire-only`" can be read either way. | `crates/goalcup-cli/tests/fixtures/schema-1-connectors/` (6 dirs) |
| Eleven entries are "partly superseded" with clause-level notes (D38, D39, D40, D41, D43, D56, D61, D62, D64, D69, D72). A reader has to stitch 3 to 4 entries to learn one rule (for example Hermes: D40, D41, D61, D62, D72, D73, D74). | DECISIONS.md:30-46 |

### 5.2 Stale or misleading code citations

| Cite | Where | Problem |
|---|---|---|
| "D12's version gate" | `crates/goalcup-cli/tests/common/support/mod.rs:15,334` | D12 is superseded by D53, which says "Goal Cup Proxy pins no build". The comment describes a gate the decision says is gone. Unverified whether a warn-only probe still exists in `src/harness_version.rs`; the comment is at least stale. |
| "(D7)" on `AttachOnly` | `crates/goalcup-protocol/src/manifest.rs:28-31`, used only at `crates/goalcup-cli/src/launch.rs:86,1406` | D7 is superseded; "Desktop attach mode" was V1 (D48 removed V1). The enum variant looks like dead V1 code. |
| "D7" and "D1" as measurement labels | `crates/goalcup-daemon/src/terminate/authority.rs:425`, `crates/goalcup-daemon/tests/it/terminate_sim.rs:1187,2044` | These are smoke or scenario labels, not decision numbers. Same spelling, different meaning. Rename to avoid false traces. |
| "D23" | `crates/goalcup-cli/src/launch.rs:13` | D23 is retired ("Harness tiers"); the heading "environment is built from nothing (D1, D23, D38)" should cite D36 or D61. |
| "D48" in 25 files | for example `crates/goalcup-daemon/src/runtime/daemon.rs:143,514`, `crates/goalcup-cli/src/lib.rs:25,27`, `crates/goalcup-cli/src/config.rs:30` | Mostly "removed by D48" history comments. Noise for a reader; git keeps history. Real code behind it: the daemon still refuses a registry whose V1 row owes work (`runtime/daemon.rs:143`, `:514`, test `crates/goalcup-daemon/tests/it/capture_lifecycle.rs:2270`), and V1 cadences are written as zero (`crates/goalcup-daemon/src/runtime/mod.rs:30`). |

### 5.3 Entry by entry

Columns: D (lines in DECISIONS.md) | What it says | Status (table / real) | Verdict | Conflict or reason | Code cites (crates/server/connectors) and where.

| D | What it says | Status (table / real) | Verdict | Conflict or reason | Code cites |
|---|---|---|---|---|---|
| D1 (52-61) | Never change the harness's model base URL; a loopback forward proxy reached only via the process's `HTTPS_PROXY` decrypts declared hosts. | current / current | **keep** | Core of point 1 and point 7 (subscription logins keep working). | 45 lines, 9 files. Enforced in `crates/goalcup-protocol/src/capture_policy.rs` (22 cites, e.g. :424, :492, :818-821, :924-931); `crates/goalcup-cli/src/launch.rs:13,37,1295`; goldens `crates/goalcup-cli/tests/fixtures/harness-golden/policy.json` (12). One false hit: `terminate_sim.rs:1187`. |
| D2 (63-66) | Two legs: native session files and wire traffic; neither erases a gap in the other. | current / current | **keep** (merge D74's "zero native rows allowed") | Points 1, 2. | 2: `crates/goalcup-cli/src/run/mod.rs:303`, `tests/it/native_silence.rs:4`. |
| D3 (68-73) | Daemon captures, redacts, hashes, uploads; it judges nothing; verifier is its own binary. | current / current | **keep** | Keeps the recorder small (O1). | 13 lines, 12 files (protocol 6, daemon 4, cli 3), e.g. `crates/goalcup-cli/tests/it/dependency_bans.rs`. |
| D4 (75-86) | One run-scoped daemon at a time, started on demand; one workspace per entry; continue takes over the last entry's workspace. | current / current | **keep**, trim the "Residual" paragraph | Simple and needed. The residual note is detail, not a rule. | 17 lines, 8 files: `crates/goalcup-daemon/src/registry.rs:5,453,458,482`, `runtime/daemon.rs:32,86,248,325`, `crates/goalcup-cli/src/daemon/spawn.rs:29,345`. |
| D5 (88-92) | The agent must not see the evidence folder; `artifact_root` and `workspace_parent` may not contain each other. | current / current | **keep** | Protects the record. | 2: `crates/goalcup-daemon/src/config.rs:128`, `tests/it/config_load.rs:83`. |
| D7 (94-98) | Old start flow; now: start makes the workspace, launches the harness in chat mode. | superseded / superseded | **retire** (delete) | Replaced by D57. | 3, of which 2 are a different "D7" label (`terminate/authority.rs:425`, `terminate_sim.rs:2044`); 1 real: `crates/goalcup-protocol/src/manifest.rs:30` (V1 `AttachOnly`, likely dead). |
| D9 (100-103) | Loss is stated, never hidden: a gap is a chained record with a reason. | current / current | **keep** | This is point 3's "note a gap and keep going". | 3: `crates/goalcup-daemon/src/bin/goalcup-daemon.rs:31`, `runtime/daemon.rs:447`, `crates/goalcup-protocol/src/export.rs:890`. |
| D10 (105-110) | One harness per entry; subagents of the same harness allowed; a second harness is a rules matter, never stopped. | current / current | **keep** (it is a rule, not code) | Matches VISION "one root agent in one harness". | 0 in code; 5 docs. |
| D11 (112-115) | The seal lists the workspace at the end. | current / current | **merge** into one seal decision with D56 and D71 | Three entries describe one seal. | 1: `crates/goalcup-protocol/src/seal.rs:1`. |
| D12 (117-119) | Old version pinning; now: record the build, refuse none. | superseded / superseded | **retire** (delete) | Replaced by D53. | 2, both stale: `crates/goalcup-cli/tests/common/support/mod.rs:15,334`. |
| D14 (121-124) | Redaction runs before bytes are hashed or written. | current / current | **merge** into one secrets decision (D14 + D36 + D60 + the O6 rule list) | Point 4. | 2: `crates/goalcup-daemon/src/redact.rs:1`, `crates/goalcup-protocol/src/redaction.rs:1`. |
| D16 (126-130) | The server's signed start receipt fixes `started_at` and `deadline_at`. | current / current | **keep** | Point 5 (server rebuilds by ID and time). | 2: `crates/goalcup-cli/src/arming.rs:1`, `crates/goalcup-protocol/src/receipt.rs:1`. |
| D22 (132-136) | A connector says how its harness is killed (`process_group` or `native_pid`) plus a grace. | current / current | **keep**, merge D30 and D40's grace clause into it | Needed so a run ends cleanly. | 22 lines, 18 files: `crates/goalcup-daemon/src/terminate/mod.rs`, `crates/goalcup-protocol/src/process.rs` (2), `connectors/codex-cli/harness.toml`. |
| D23 (138-140) | Harness tiers: retired. | retired / retired | **retire** (delete) | Nothing to follow. | 1 stale: `crates/goalcup-cli/src/launch.rs:13`. |
| D24 (142-146) | Every mutating control route needs the daemon's bearer in `control.json` (0600). | current / current | **keep** | Small, protects the record. | 4: `crates/goalcup-daemon/src/control/file.rs`, `control/mod.rs`, 2 tests. |
| D25 (148-153) | Status is for humans; nothing is injected into the agent. Narrowed by D66. | partly superseded / partly | **merge** with D66 into one line: "nothing is pushed to the agent" | Two entries for one rule. | 8 lines, 6 files: `crates/goalcup-daemon/src/status.rs` (2), `crates/goalcup-protocol/src/control.rs` (2). |
| D26 (155-158) | Verifier may sum token usage; no token budget; plan, credit and billing fields stripped before publishing. | current / current | **change**: keep "no budget" and "strip at publication"; drop the capture-time rate-limit redaction that cites it | O6: "drop the email rule and the Codex rate-limit rules". The Codex rows cite D26 at `connectors/codex-cli/harness.toml:102-110`; Claude at `connectors/claude-code-cli/harness.toml:155`. Test `crates/goalcup-daemon/tests/it/redaction.rs:92`. | 4: 2 connectors, `crates/goalcup-daemon/src/redact.rs:542`, 1 test. |
| D30 (160-165) | "Confirmed" termination is bounded to the owned process set, checked by two sweeps. | current / current | **merge** into D22 | Same topic. | 8: `crates/goalcup-daemon/src/terminate/mod.rs`, `terminate_sim.rs` (5), `crates/goalcup-cli/src/released_child.rs`. |
| D36 (167-177) | Wire-leg redaction: credentials never stored, bodies redacted best effort before disk, decoded is raw, recorder never parses formats. | current / current | **keep** (as the base of the merged secrets decision) | Points 1 and 4. O6 changes only the rule list, not this entry. | 2: `crates/goalcup-protocol/src/capture_policy.rs:4`, `crates/goalcup-cli/tests/it/connector_contract.rs:473`. 7 docs. |
| D37 (179-193) | Autonomous runs; research traces are primary; disk capacity required before launch; new harness = data; loopback ingest trusts one user; `LocalFinish`. | current / current | **change**: keep clauses 1, 2, 4, 5; drop clause 3 | Clause 3 "Real recorder disk capacity is required before launch" conflicts with O4 "Disk full: no special handling." | 3: `crates/goalcup-daemon/src/redact.rs:389`, `redact/stream.rs:648`, `crates/goalcup-protocol/src/run_secret.rs:33`. |
| D38 (195-201) | Claude Code gets a fresh per-run HOME with a Keychains link and `CLAUDE_CONFIG_DIR`. | partly superseded / partly | **change**: fold the one live rule (how Claude Code's home is built) into the home rules with D59; delete the rest | Mostly replaced by D52, D69, D71. | 15 lines, 10 files, mostly connector data and fixtures: `connectors/claude-code-cli/harness.toml` (2), `crates/goalcup-cli/src/home.rs`, `launch.rs` (3). |
| D39 (203-214) | Connector extra redaction patterns; `capture.args` template; one shared Codex home serialized by a lock; alias and mode checks. | partly superseded / partly | **change**: keep "extra patterns are data" and "shared home, serialized"; delete the headless clause | O6 trims which patterns ship. The serialization lock means a second run on the same machine waits; that is one-run-per-machine, not a slowdown of the harness. | 40 lines, 22 files: `crates/goalcup-cli/src/home.rs` (6), `commands/start/phases.rs` (3), all 4 connectors (10). |
| D40 (216-236) | Finish keeps termination grace; 180 s budget, 2 s reserve; spawn gate; one 3 GiB disk reserve per run; Hermes uses DeepSeek (superseded). | partly superseded / partly | **change**: keep grace, budget and spawn gate (into D22); **retire** the 3 GiB reserve | O4 "Disk full: no special handling." The reserve exists only for disk handling (`crates/goalcup-cli/src/run/reserve.rs`, 128 src lines + 827 inline test lines). Spawn gate makes the child wait before `exec` until registered: a one-time start delay, which serves point 1 (no unrecorded start). | 22 lines, 13 files: `crates/goalcup-cli/src/launch.rs` (6), `commands/start/phases.rs` (4). |
| D41 (238-253) | Hermes CA trust, pre-seeded `auth.json`, key-fragment scan, isolation; `masked_key` rule stays. | partly superseded / mostly dead | **retire**; move the `masked_key` rule into the secrets rule list | D61 removed the seed and the key scan; D43 generalised CA trust. Only `masked_key` survives (a secret rule, which O6 says to keep). | 18 lines, 14 files: `crates/goalcup-daemon/src/redact.rs`, `crates/goalcup-cli/src/ca_bundle.rs`, `connectors/hermes-cli/harness.toml` (2). |
| D42 (255-262) | Harnesses are data; anyone may send a harness PR; Apache-2.0; "The protocol freezes at v5 for v0.x"; macOS first. | current / partly in conflict | **change**: keep data-only harnesses and licence; delete the freeze sentence | O2: "Protocol v5 is frozen" no longer applies. Quote: "The protocol freezes at v5 for v0.x." (DECISIONS.md:261). | 7: three `harness_contract_golden.rs` (cli, daemon, verifier), `crates/goalcup-protocol/src/harness/schema.rs:155`, `crates/goalcup-platform/src/lib.rs:6`. |
| D43 (264-298) | Harness contract v1: one file per connector, credential list (gone), CA name shape, server digest list (withdrawn), stranger PRs, v5 freeze of optional Launch fields, unknown `protocol_hint` is capture-only. | partly superseded / partly in conflict | **change**: keep rulings 1, 3, 5, 7; delete 2, 4, 6 | Ruling 6: "A verifier change that makes a valid v5 trace invalid needs a DECISIONS entry (the reader corpus guards this)." conflicts with O2. | 16 lines, 12 files: `crates/goalcup-protocol/src/capture_policy.rs` (3), `pins.rs`, `hints.rs`, `connector_dir.rs`; `crates/goalcup-verifier/tests/it/v5_corpus.rs`. |
| D44 (300-304) | When OpenCode is added, read its SQLite store read-only after the run. | current / never built | **retire** (or move to the harness backlog) | No code or doc cites it. D74 now allows zero native rows, so OpenCode could ship wire-only without a SQLite reader (point 7). The facts sheet on `origin/opencode-openclaw` (`docs/harnesses/opencode-cli.md:5-6,55`) still names D44 as the stop rule; that stop is gone. Unverified: whether OpenCode's wire traffic decodes. | 0. |
| D45 (306-312) | `--capture-integrity` judges whether the record can be trusted, not how the run went. | current / current | **keep**, drop "The v5 reader corpus pins this" | Point 6 (say plainly what was recorded). | 10 lines, 7 files: `crates/goalcup-verifier/src/capture_analysis/{anthropic,chat,tracker,wire}.rs`. |
| D46 (314-318) | Create request carries `connector_dir_sha256`; CLI and daemon must agree. | current / current | **keep** (one line), or merge into D70 | Small safety check between two local halves. | 0 in code (rule lives in code without a cite); 2 docs (ARCHITECTURE, SECURITY). |
| D47 (320-324) | No DCO sign-off; PR author discloses AI help; code-owner review stays. | current / current | **keep** | Process only, no code. | 0 in code; `CONTRIBUTING.md`. |
| D48 (326-332) | The V1 product path is deleted; the daemon refuses a registry whose V1 row still owes work. | current / done | **retire**, and cut the V1-row refusal and the "removed by D48" comments | O2: "No backward compatibility. Every run so far is test data." The V1-row check guards data that no longer matters. | 35 lines, 25 files, mostly history comments; live code at `crates/goalcup-daemon/src/runtime/daemon.rs:143,514`, test `crates/goalcup-daemon/tests/it/capture_lifecycle.rs:2270`. |
| D49 (334-341) | Admit `AMP_API_KEY` as a child credential. | superseded / dead | **retire** (delete) | Replaced by D60, D61. | 1: `crates/goalcup-protocol/tests/it/harness_schema.rs:731`. |
| D50 (343-351) | `stdout_json` identity with a credential. | superseded / dead | **retire** (delete) | Replaced by D61. No cite anywhere. | 0. |
| D51 (353-371) | Connector argv and env are data held to generic deny rules; review is the control; closed lists need a DECISIONS entry. | current / current | **keep** | Point 7 (new harness = data PR) and D1 enforcement. | 15 lines, 5 files: `crates/goalcup-protocol/src/capture_policy.rs` (5), `harness_loader.rs` (4), `harness_schema.rs` (4). |
| D52 (373-386) | Shipped connectors set no model, effort, sandbox, permission or MCP limit; the two v5 tool fields stay readable and are written empty. | current / partly in conflict | **change**: keep the rule; delete the two always-empty v5 fields | The first paragraph is point 7 and "never force by config". The "stay readable and are written empty" clause exists only for old traces: O2. | 3: `crates/goalcup-daemon/src/runtime/capture/mod.rs:1269`, `crates/goalcup-protocol/src/capture.rs:310`, verifier golden. |
| D53 (388-399) | `Launch.harness_build` records program and image digests; no build is pinned. | current / current | **keep** the field; delete the "Trade" paragraph | Trade paragraph ("a v0.5.0 verifier reports `record_malformed`... Any further post-tag field means v6") is backward-compat reasoning: O2. | 39 lines, 28 files: `crates/goalcup-daemon/src/runtime/capture/harness_build.rs` (179 lines), `crates/goalcup-cli/src/harness_version.rs` (203 lines), `commands/doctor.rs`. |
| D54 (401-412) | The protocol carries v5 only; "v5 bytes and every v5 trace's verdict are frozen"; old version numbers get set error codes. | current / in conflict | **retire** the freeze; keep one line "the protocol carries one version" | O2. Quote: "v5 bytes and every v5 trace's verdict are frozen." (DECISIONS.md:405). The frozen corpus (`crates/goalcup-verifier/tests/fixtures/v5-corpus/`, 99 files, 564 KiB; `tests/it/v5_corpus.rs` 718 lines) exists to hold this. | 15 lines, 13 files: `crates/goalcup-protocol/src/{capture,control,envelope,export,lib,provenance}.rs`, `crates/goalcup-verifier/src/capture_analysis/reader.rs` (2). |
| D55 (414-425) | The chain rotates into part files at 256 MiB or 30 min; acknowledged parts may be deleted locally. | current / current | **keep** | Needed for a week-long run and for upload (points 1, 5). | 20 lines, 11 files: `crates/goalcup-protocol/src/export.rs` (9), `crates/goalcup-daemon/src/runtime/capture/writer.rs`. |
| D56 (427-462) | Seal of a clone records commit plus an uncommitted digest; git run with a locked-down environment; home baseline (10,000 files, 64 MiB); 180 s budget. | partly superseded / partly | **merge** with D11 and D71 into one seal decision; question the home baseline caps | Caps "bounded to 10 000 files, files over 64 MiB size only" and "Output is capped at 64 MiB" are caps of our own (O7); they guard finish time, which is a real cost, so they may stay if the merged entry says so. | 43 lines, 21 files: `crates/goalcup-daemon/src/workspace.rs` (5), `workspace_git.rs` (549 lines), `runtime/capture/finish.rs` (3), `crates/goalcup-protocol/src/seal.rs` (5). |
| D57 (464-486) | Connectors run chat mode; native identity = every session file new or written since run start; walker bound 8192 entries; past `max_files` is a `LimitExceeded` gap. | current / partly in conflict | **change**: keep chat mode and since-start identity; remove the file cap clause | O3: "raise or remove ... the 1024-file cap". Quote: "A file kept out for good (past `max_files`, or refused) is a `LimitExceeded` gap." (DECISIONS.md:483-484). The cap is `max_files: 1024` at `crates/goalcup-daemon/src/native_capture.rs:280` (ceiling `MAX_NATIVE_FILES` 20,000 at :187). | 102 lines, 50 files, most of any entry: `crates/goalcup-cli` (46), daemon (21), connectors (17), protocol (16). |
| D58 (488-496) | Each recovered wire epoch spends a fresh producer ID (65,536 ever); at most 128 producers live at once; 10 connection failures in a row end the wire leg. | current / in conflict | **change**: raise or remove `MAX_LIVE_PRODUCERS` (128) and the 128 goal-stream bound; restate as a cost guard if any number stays | O3: "raise or remove the open-stream limits (128 per entry, 32 for native files)". Constants: `crates/goalcup-protocol/src/capture.rs:58` (`MAX_OPEN_STREAMS` 128), :60 (`MAX_OPEN_CONNECTIONS` 128), :63, :66; native share is `MAX_OPEN_STREAMS / 4` = 32 at `crates/goalcup-daemon/src/native_capture.rs:279`. Also "A verifier built before this entry flags a trace with more than 128 producers" is O2 noise. Also question "`MAX_CONNECTION_FAILURES` (10 in a row) still ends the wire leg" (`crates/goalcup-cli/src/proxy/ingest.rs:104`) against point 1. | 14 lines, 7 files: `crates/goalcup-verifier/tests/it/m2_capture_integrity.rs` (5), `crates/goalcup-daemon/tests/it/capture_ingest.rs` (2). |
| D59 (498-508) | `NODE_USE_ENV_PROXY=1` is admitted; `run_home = "shared"` for harnesses with no home variable. | current / current | **keep** | Point 7 (Node harnesses, Amp). | 42 lines, 20 files: `crates/goalcup-cli/src/home.rs` (6), `crates/goalcup-protocol/src/capture_policy.rs` (4). |
| D60 (510-527) | The daemon redacts the run's own proxy secret by exact value in every common encoding. | current / current | **keep** (inside the merged secrets decision) | Point 4. | 20 lines, 12 files: `crates/goalcup-daemon/src/redact/stream.rs` (6), `redact.rs` (2). |
| D61 (529-590) | Hermes runs chat mode from its own home; Goal Cup forwards no key; `capture.configure` commands; no credential mechanism. | partly superseded / partly | **keep** the no-key rule and `capture.configure`; move the Hermes-specific detail to `docs/harnesses/hermes-cli.md` | Point 4 and O1: the "no key held" rule is what lets the key scan, seed and keyed upload rules go. | 44 lines, 24 files: `crates/goalcup-cli/src/commands/prep.rs` (2), `crates/goalcup-protocol/src/harness/check.rs` (3), `parse.rs` (2), `connectors/hermes-cli/harness.toml` (4). |
| D62 (592-639) | `capture.model_host_from`: read one https URL from the harness's own config; refuse unsafe ones. | partly superseded / partly | **merge** into D74 | D74 rewrites its refusal and deny words. Two entries for one field. | 28 lines, 19 files: `crates/goalcup-cli/src/model_host.rs` (1540 lines incl. tests), `commands/prep.rs` (3). |
| D63 (641-658) | A signed receipt must match the Launch it anchors; older receipts without a directory claim stay valid; corpus verdicts flipped. | current / partly in conflict | **change**: keep the match rule; require the directory claim; drop corpus history | O2. Quote: "Older receipts without a directory claim stay valid, including when Launch carries a digest." | 4: `crates/goalcup-verifier/src/capture_analysis/reader.rs:848`, `tests/fixtures/v5-corpus/expected.toml:523,569`, `tests/it/v5_corpus.rs:194`. |
| D64 (660-690) | No server error stops a run or its upload; only `stop` in a 200 answer or the local deadline ends it; retries with backoff. | partly superseded / current | **keep** | Point 3 exactly. Drop the disk-watch sentence (D65 goes). | 21 lines, 12 files: `crates/goalcup-cli/src/upload/uploader/lifecycle.rs` (4), `session.rs` (3), `upload/policy.rs` (2); `server/worker/src/runs.ts:504`. |
| D65 (692-724) | On a nearly full disk Goal Cup sends SIGSTOP to the agent and SIGCONT when space returns. | current / in conflict | **retire** | O4: "Disk full: no special handling. The harness stops and the user cleans up." Also point 3: "Never block or slow the harness." A SIGSTOP is a block. Code: `crates/goalcup-cli/src/run/pause.rs` (211 src + 149 inline test lines), disk logic in `run/watch.rs`, `run/reserve.rs`; tests `crates/goalcup-cli/tests/full/t25_disk_full.rs` (1173 test lines). | 34 lines, 16 files: `crates/goalcup-cli/src/commands/start/phases.rs` (5), `run/reserve.rs` (5), `run/watch.rs` (4), `crates/goalcup-platform/src/terminal.rs` (2). |
| D66 (726-758) | The agent may pull time left (`goalcup time-left` and an MCP server `goalcup mcp`); `goalcup seal` refuses inside the harness. | current / current | **change** (owner to decide): keep `time-left` and the in-harness seal refusal; consider cutting `goalcup mcp` | Not one of the seven points. RULES promises the command (`docs/RULES.md:112`); README promises the MCP tool (`README.md:72,102`). `crates/goalcup-cli/src/commands/mcp.rs` is 273 src + 201 test lines. The seal refusal protects point 1 (agent cannot end its own recording by accident). | 16 lines, 10 files: `crates/goalcup-cli/src/run/time_left.rs` (4), `launch.rs` (3), `commands/mcp.rs`. |
| D67 (760-782) | Goal Cup recommends no-prompt settings per harness; the participant sets them; Goal Cup never writes them. | current / current | **keep** | Point 7 and "never force by config". | 16 lines, 11 files: `crates/goalcup-cli/src/commands/prep.rs` (4), `crates/goalcup-protocol/src/harness/{check,schema}.rs` (2 each), 3 connectors. |
| D68 (784-811) | Connector contract locked at schema 1: only grows; 14-day deprecation notice; frozen fixtures; `--dev` rules. | proposed / in conflict | **retire** before Edition 01 (keep only the `--dev` paragraph, which D70 already holds) | O2. Quote: "While schema 1 is read, a connector that loads today keeps loading and launching with the same meaning." and "A field is retired by a DECISIONS entry, at least one release and 14 days of notice". That is a backward-compat promise. It could return after the cup. | 1: `connectors/claude-code-cli/harness.toml:107`. Fixtures `crates/goalcup-cli/tests/fixtures/schema-1-connectors/` (6 dirs), `crates/goalcup-protocol/tests/fixtures/frozen-connectors/` (3). |
| D69 (813-865) | Goal Cup holds no keys; four-step setup; the old git login and clone flow (mostly replaced by D71); hidden `git-credential` for old clones. | partly superseded / mostly dead | **retire**; move "holds no keys" and "four-step setup, no GitHub" into D71; cut the old-clone helper | O2. Quote: "The hidden `goalcup git-credential` still answers from `github.json` for clones made before this ... It can be removed after the event." Code: `crates/goalcup-cli/src/account/github.rs` (466 src lines, no inline tests), `crates/goalcup-cli/src/main.rs:79,233`, `crates/goalcup-cli/src/commands/prep.rs:436`, `crates/goalcup-cli/src/config.rs:20`. Unverified how much of `github.rs` is only the old helper. | 22 lines, 13 files: `crates/goalcup-cli/src/account/github.rs` (6), `prep_file.rs` (2), `tests/it/account.rs` (3). |
| D70 (867-896) | A connector ships by release; the Worker's `MIN_COMPETITION_VERSION` refuses older clients; `--dev` never competes; `install.sh` refuses to replace a recording goalcup; `CONTROL_API_VERSION` 12 kept to 15 Nov. | current / partly in conflict | **keep** the release-is-the-list, `--dev` and minimum-version rules; **change** the compat clauses | O2 for the 0.6.1 and 0.7.0 notes. The `CONTROL_API_VERSION` pin "From 0.7.0 to 15 Nov 2026" should restart at the Friday release: before Edition 01 it may change. Keeping it from the Friday release through the cup is fine, since a forced update over a live run is real. | 6: `server/worker/src/runs.ts:138`, `server/worker/test/device.test.ts:274`, `lifecycle.test.ts:535`, `server/worker/README.md:322,356`, `crates/goalcup-cli/tests/full/upload_worker.rs`. Var at `server/worker/wrangler.toml:55,150`. |
| D71 (898-973) | The project folder is the workspace; folder rules; push with a one-repo `GH_TOKEN`; git env locked down; seal decides clone at finish (50,000-file walk cap); two run secrets. | current / current | **keep**, trim | Points 4, 5, 7. Trim the 0.6.1 clause: "The hidden `goalcup git-credential` and the `github.json` loader stay until after 15 Nov for runs started on 0.6.1" (O2). The 50,000-file walk cap guards finish time; it records "capped", never an error, which matches point 3. | 31 lines, 12 files: `crates/goalcup-daemon/src/workspace.rs` (7), `runtime/capture/finish.rs` (4), `runtime/capture/mod.rs` (4), `crates/goalcup-protocol/src/seal.rs` (3). |
| D72 (975-1049) | Hermes 0.22 records only through a `providers.recorder` trust entry and a fresh gateway; `[launch_prep]` (`ca_file`, `commands`, `after_exit`); `warn_missing`. | partly superseded / current | **keep** the `[launch_prep]` schema rule; move Hermes version notes to the Hermes harness doc | Point 7 (Hermes records at all). 75 lines of mostly version-specific notes. | 44 lines, 16 files: `crates/goalcup-cli/src/commands/start/phases.rs` (6), `crates/goalcup-protocol/src/harness/schema.rs` (6), `launch.rs` (4). |
| D73 (1051-1103) | Push lines saved to `push-lines.txt` and the clipboard; `continue` resumes a harness with no session file when calls were recorded; SSH-key warning. | current / partly in conflict | **change**: keep the resume rule; cut every "run started on 0.7.0" branch; consider cutting the clipboard copy | O2. Quote: "A run started on 0.7.0 has no count and keeps that rule." and "A run with no count (started on 0.7.0) keeps the 0.7.0 text on a resume". O1: clipboard tools (`pbcopy`, `wl-copy`, `xclip`, `clip.exe`) are four platform paths for a convenience. Code: `crates/goalcup-cli/src/push_lines.rs` (152 src + 61 test lines), `ssh_keys.rs` (192 src + 48 test). | 44 lines, 16 files, all in `crates/goalcup-cli`: `commands/start/phases.rs` (16), `participant/launch.rs` (6). |
| D74 (1105-1162) | Model hosts = connector defaults plus the one address the harness config names (`via`, `absent`); `env` no longer a deny word; zero native rows allowed; every sealed tunnel listed; evidence labels; `providers.toml` frozen. | current / partly in conflict | **keep** (absorb D62); **change** the tunnel list caps | Points 1, 6, 7. O7 caps of our own: "at most 10,000 rows plus a dropped count" (`crates/goalcup-cli/src/proxy/tunnels.rs:20`) and "at most 8 MiB" (`crates/goalcup-cli/src/upload/uploader/finish.rs:257`). A week with thousands of subagents could pass 10,000 tunnels (unverified). Either upload the list as a normal chunk or say what cost the cap guards. Also O5 does not touch this entry. | 12 lines, 12 files: `crates/goalcup-cli/src/proxy/{mod,tunnels}.rs`, `upload/uploader/finish.rs`, 2 connectors, `crates/goalcup-protocol/providers.toml`. |

### 5.4 Code and tests at stake (only where the decision is the reason the code exists)

| Decision change | Main code | Src lines | Test lines | Note |
|---|---|---|---|---|
| Retire D65 and the D40 reserve | `crates/goalcup-cli/src/run/pause.rs`, `run/reserve.rs`, disk parts of `run/watch.rs`, status lines | about 340 (pause 211, reserve 128) plus unmeasured parts of `watch.rs`, `phases.rs`, `participant/status.rs` | about 2150 (pause 149, reserve 827, `tests/full/t25_disk_full.rs` 1173) | unverified total; owned by another lane |
| D66: cut `goalcup mcp` (if owner agrees) | `crates/goalcup-cli/src/commands/mcp.rs` | 273 | 201 | README and HARNESS-SETUP lines go too |
| D69: cut old-clone helper | `crates/goalcup-cli/src/account/github.rs`, `main.rs:233` | up to 466 (unverified share) | part of `tests/it/account.rs` | after checking what else uses `github.rs` |
| D54/D63/D43.6: drop the v5 freeze | `crates/goalcup-verifier/tests/fixtures/v5-corpus/` (99 files), `tests/it/v5_corpus.rs` | 0 | 718 plus fixtures | corpus can be regenerated, not deleted, if the verifier still needs goldens |
| D68: drop the schema-1 lock | `tests/fixtures/schema-1-connectors/` (6), `crates/goalcup-protocol/tests/fixtures/frozen-connectors/` (3) | 0 | fixtures and their tests | the conformance tests for shipped connectors stay |
| D48: drop the V1-row refusal | `crates/goalcup-daemon/src/runtime/daemon.rs:143,514` | small (tens) | `capture_lifecycle.rs:2270` block | plus about 30 history comments |
| D52: drop the two empty v5 fields | `crates/goalcup-protocol/src/capture.rs:310`, verifier reader | small | verifier golden | needs a protocol change, allowed by O2 |
| D73: drop 0.7.0 branches and clipboard | `crates/goalcup-cli/src/push_lines.rs`, `participant/launch.rs` | up to 152 | up to 61 | owner to decide on the clipboard |

The same changes in the brief's verdict format (numbers only from the table above):

| Part (path, src, tests) | What it does | Points served | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| `run/pause.rs` + `run/reserve.rs` (211 + 128 src; 149 + 827 inline tests) | Disk pause (D65) and 3 GiB reserve (D40) | none (pause works against 3) | cut | O4 | about 340 / about 2150 (unverified) | tail of a run lost on full disk (accepted, O4) | `t25_disk_full.rs`, inline tests |
| `commands/mcp.rs` (273 src, 201 tests) | MCP time-left server (D66) | none | cut if owner agrees | not in the seven points | 273 / 201 | agents without `goalcup` on `PATH` lose time-left | its 201 lines |
| `account/github.rs` (466 src) | old-clone `git-credential` helper (D69) | none | cut (share unverified) | O2 | up to 466 / part of `tests/it/account.rs` | 0.6.1 test clones cannot push | part of `account.rs` |
| `v5-corpus/` + `v5_corpus.rs` (0 src, 718 tests + 99 fixtures) | holds the v5 freeze (D54, D63, D43.6) | none | change (regenerate with the protocol) | O2 | 0 / up to 718 | clients, verifier and Worker must ship together | v5 freeze cases |
| schema-1 and frozen-connector fixtures (6 + 3 dirs) | hold the D68 lock | none | cut | O2 | 0 / fixtures | outside authors lose the promise (fine before Edition 01) | frozen-fixture tests |
| V1-row refusal (`runtime/daemon.rs:143,514`) | D48 guard | none | cut | O2 | tens / `capture_lifecycle.rs:2270` block | none found | that block |
| two empty v5 tool fields (`capture.rs:310`) | D52 compat | none | cut | O2 | small / verifier golden | protocol change | golden update |
| `push_lines.rs` 0.7.0 branches and clipboard (152 src, 61 tests) | D73 | 6 (push lines shown) | change | O1, O2 | up to 152 / up to 61 | owner may want the clipboard | clipboard tests |

## 6. Verdict for every doc

Verdicts: **keep**, **rewrite** (keep the path, change much of the text), **merge** (into a named doc, then
delete), **delete** (git history is enough). "Lines" is the file today.

### 6.1 Root files

| Doc | Lines | Verdict | Into | Reason |
|---|---|---|---|---|
| `VISION.md` | 44 | keep | | The yardstick. Add the owner decisions of 29 Oct as one short "Decided" list, so they stop living only in a plan file (`docs/plans/2026-10-29-record-everything-progress.md:80-100`). |
| `AGENTS.md` | 110 | rewrite | | Read-first list points at stale plans (`:24-30`), Hard rules keep D65 (`:84-86`). Map table is good; keep it. |
| `CLAUDE.md` | 16 | keep | | Pointer only. It repeats the seven points, which is fine for an agent entry file. |
| `README.md` | 318 | rewrite | | Keep install, quick start, commands, how it works, gate. Fix `:79-80` (disk pause), `:150-151` (no header is recorded: false), `:283` (v5 corpus). Shorten "For developers" by pointing to CONTRIBUTING. |
| `TODO.md` | 350 | rewrite | | Mostly dated status (21 and 22 Oct), done work packages and post-event clean-up lists. Replace with under 60 lines: the cut list for Friday, known issues still true, owner questions still open. |
| `CONTRIBUTING.md` | 126 | keep | | Accurate. Only edit if the gate or CI change. |
| `SECURITY.md` | 127 | rewrite | | Absorb `CHEAT-CHECKLIST.md` into "Known residuals" (`:102-117`) as a short table. |
| `CODE_OF_CONDUCT.md` | 132 | keep | | Standard text. |

### 6.2 `docs/*.md` (top level; DECISIONS is in section 5)

| Doc | Lines | Verdict | Into | Reason |
|---|---|---|---|---|
| `PARTICIPANT-GUIDE.md` | 330 | rewrite | | Path is baked into the binary. Drop 0.6.1 notes (`:93-100`), the disk pause (`:242-243`, `:324-325`). Add the email table from NOTIFICATIONS (point 6). |
| `HARNESS-SETUP.md` | 468 | rewrite | | Path and anchors baked in. Fix Hermes "no session file, calls only" (`:194-195`) now that `state.db` is copied once at finish (Task 6, #297). Fix Hermes subagent claim (`:424` says `delegate_task` started two subagents; `progress.md:74` says it fails in 0.19.0). Keep sections 1 to 7. |
| `RULES.md` | 489 | rewrite | | The contract. Fix `:322-325` (disk), `:40-43` (0.6.1). Make "what a trace holds" (`:336-348`) match masking option b: say email addresses and Codex plan and credit fields are kept (they already say plan and credit data is in raw traces at `:370-372`). Move "Still to decide" (`:485-489`) to TODO. |
| `OPERATING.md` | 863 | rewrite | | Keep Part 1 (where things run, dates, deploy, test runs, edge cases, bad days) and the CLI and verifier reference in Part 2. Cut "The matrix" (`:154-434`, 281 lines) to one short how-to once the scripts lane decides `matrix.sh`. Fix `:457` (case 4, disk), `:634-636` (outages not in the trace because "v5 is frozen"). Take in the runbook parts now repeated in `server/worker/README.md:129-384`. |
| `UPLOAD-API.md` | 596 | keep, edit | | The wire contract, 24 code cites. Remove `agent_paused` and `disk_full` (`:380`, `:388`, `:399-407`, `:554`, `:586`, `:595`) when the CLI pause goes. |
| `SERVER-DESIGN.md` | 164 | merge | `server/worker/README.md` | Second description of the same Worker. Limits (`:109-133`), notices (`:135-147`), storage and signing fit the Worker README. Remove the `disk_full` notice (`:135-147`). |
| `ARCHITECTURE.md` | 350 | rewrite | | Keep crates, parts, legs, lifecycle, terms, test map. Remove "the v5 freeze" (`:191-194`), fix `:212-213` (header streams exist: `crates/goalcup-cli/src/proxy/headers.rs`), `:253-256` (proxy restart exists: `crates/goalcup-cli/src/proxy/mod.rs:1022`), `:14-17` (links to retired research docs). Take in a one-page termination contract from TERMINATION and the limits table from CONTRACTS. |
| `CONTRACTS.md` | 654 | merge | `ARCHITECTURE.md` (limits, ingest and redaction rules), `UPLOAD-API.md` (tunnel side file) | Built around a backward-compat freeze the owner dropped: `:510-571` (freeze, exceptions, reader rule), `:603-608`, `:638-654` (control API version history). 9 code cites to update. |
| `TERMINATION.md` | 262 | merge | `ARCHITECTURE.md` (one section, about 60 lines) | Precise but deep. The invariant text is quoted in a test (`tests/it/terminate_sim.rs`, per `:165`); keep the test as the source and link it. 11 code cites. Matches the D22 + D30 + D40 merge in section 3. |
| `CHEAT-CHECKLIST.md` | 305 | merge | `SECURITY.md` "Known residuals" | Same purpose as SECURITY's residuals list. Keep only rows still true after the cut (lane L decides which cheats must stay visible). 4 code cites. |
| `CONNECTOR-SCHEMA.md` | 1,002 | rewrite | | Field reference, 21 code cites, keep the path. Cut "The compatibility promise" (`:94-135`), "The deprecation rule" (`:136-159`), "Pin evolution" (`:957-984`): all are backward-compat machinery (the doc side of D68). Fix `:110-111` and `:704` ("providers table is frozen"), `:886` (email in shared set), `:205` and `:899` (example uses `codex_plan_type`, a rule being dropped). Target about 650 lines. |
| `HARNESS-ONBOARDING.md` | 579 | rewrite | | The one contributor procedure. Fix `:198-202` (native ceiling 32 / 1024), `:537` (reviewer asks for "plan, billing, quota" redaction coverage). Ask the owner about `:191` (headless and one-shot harness modes "not planned"), see 7.6. |
| `RESEARCH-TARGET.md` | 115 | delete | `VISION.md` | VISION replaced it. It is dated 18 Oct and steers by phases that are done. AGENTS and TODO link it. |
| `LOCAL-RESEARCH-HANDOFF.md` | 306 | delete | | Marks itself "history (18 Oct 2026)" at `:3`. |
| `README.md` (docs map) | 83 | delete | `README.md` "Docs map" | Two maps drift. It lists plans and test-day notes as organizer reading (`:30-37`). |

### 6.3 `docs/harnesses/`

| Doc | Lines | Verdict | Reason |
|---|---|---|---|
| `README.md` | 51 | keep | Rules for writing a sheet. Drop the link to the coverage matrix (`:15-16`) when that goes. |
| `codex-cli.md` | 242 | keep, edit | Worked example. Fix `:135` ("Goal Cup redacts it": the rate-limit rules are being dropped). Trim the 18 Oct experiment narrative (`:14-90`) to its facts. |
| `claude-code-cli.md` | 170 | keep, edit | Section 11 (`:163-170`) documents the 32 / 1024 caps as normal. Rewrite after the cap change. |
| `hermes-cli.md` | 217 | keep | Facts. Check the state.db wording against Task 6. Receives the Hermes-specific detail from D61 and D72. |
| `amp-cli.md` | 231 | keep | Facts. |
| `opencode-cli.md`, `openclaw-cli.md` | 126, 121 | (branch only) | Only on `origin/opencode-openclaw`. Not edited. The docs part found no owner-decision conflict lines; the decisions part found that `opencode-cli.md:5-6,55` names D44 as the stop rule, which is gone if D44 retires. Subagents "unmeasured" (`opencode-cli.md:102`, `openclaw-cli.md:99`). |

### 6.4 `docs/archive/`

| Doc | Lines | Verdict | Reason |
|---|---|---|---|
| `2026-10-23-simple-setup.md` | 157 | delete | Says "Archived ... partly overtaken by D71" at `:3`. |
| `PARTICIPANT-GUIDE-0.6.md` | 549 | delete | Says "not current" at `:3-6`. Still holds the old disk text (`:330`, `:376`). |
| `TODO-2026-10.md` | 252 | delete | Finished work. Ticket IDs it keeps alive (F-04, H-29) are also defined in TODO and CHEAT-CHECKLIST; carry the few still-open ones into SECURITY and TODO. |

### 6.5 `docs/research/` and `docs/research/results/`

| Doc | Lines | Verdict | Into | Reason |
|---|---|---|---|---|
| `AMP-FOLLOWER.md` | 86 | delete | | A 21 Oct yes/no note. Facts are in `harnesses/amp-cli.md`. |
| `HOW-A-RUN-ENDS.md` | 76 | delete | `RULES.md` already | Plan; RULES "Submission" and the guide "The end" hold the result. |
| `LAPTOP-LOGIN-RUNS.md` | 115 | delete after the runs | `OPERATING.md` one paragraph | Owner test guide of 28 Oct. If still needed for Friday, keep until then. |
| `LOCAL-CAMPAIGN.md` | 275 | delete | | Tied to `campaign.py`. Pass line for disk drills is D65 (`:202-208`). |
| `LOCAL-NEXT-STEPS.md` | 235 | delete | | Status of 22 Oct. AGENTS says "read first" (`AGENTS.md:26`). Disk pause at `:20`, `:96`, `:144-148`. |
| `LOCAL-TEST-DAY.md` | 170 | delete | | One-day handover for 22 Oct. D65 at `:124`. |
| `NOTIFICATIONS.md` | 150 | merge | `PARTICIPANT-GUIDE.md` (table), `UPLOAD-API.md` (rules already there) | Point 6. Drop the `disk_full` row (`:55`) and `:28`, `:126`, `:142`. SQL comment cite to update. |
| `PARTICIPANT-REVIEW-2026-10-21.md` | 62 | delete | | Review list; open items belong in TODO if still open. |
| `RELEASE-IS-THE-LIST.md` | 148 | delete | | Says "cut down on 25 Oct" at `:3`. Also "`CONTROL_API_VERSION` is frozen ... to 15 Nov" (`:8`, `:52`), which conflicts with no backward compat (see the D70 row for the decisions part's view). |
| `RESTARTS-DESIGN.md` | 150 | merge | `PARTICIPANT-GUIDE.md` (sandbox stops) | Cited by `crates/goalcup-cli/tests/it/restarts.rs:1`. |
| `SIMPLIFICATION-2026-10-21.md` | 63 | delete | | Replaced by this audit. |
| `VERCEL-COMPAT.md` | 180 | delete | `OPERATING.md` (one line on Vercel Sandbox) | Says "Superseded by D74" at `:3`. The 2-hour run is in a Vercel Sandbox, so keep one line on sandbox limits. Disk pause at `:40`. |
| `coverage-matrix.md` | 164 | rewrite | `OPERATING.md` "Measured runs" | One table of what ran, on which harness and OS, and the result. Add the 2-hour run when its report exists (it does not exist on any branch yet). D65 at `:63`. |
| `m2-report-format.md` | 376 | keep, move | `docs/REPORT-FORMAT.md` | Verifier output format, 8 code cites. Not research history. |
| `replay-comparison.md` | 48 | delete or merge | `scripts/README.md` | Follows the scripts lane verdict on `research-replay.py`. |
| `t25-disk-full.md` | 247 | delete | | Measures the disk path the owner decided not to handle. Its key fact (a full disk latches the writer, `:158-164`) goes into ARCHITECTURE in one line. |
| `results/2026-10-22-fix-plan*.md` (10 files) | 523 | delete | | Merged fix plans. |
| `results/2026-10-22-local-test-day.md` | 234 | delete | coverage table | |
| `results/2026-10-24-real-machine-round.md` | 149 | delete | coverage table | |
| `results/2026-10-26-mac-retest.md` | 99 | delete | coverage table | Linked from `RULES.md:158` and `PARTICIPANT-GUIDE.md:131`: replace the links. |
| `results/2026-10-vercel-gateway.md` | 289 | delete | coverage table | Linked from `HARNESS-SETUP.md:113`, `:169`. |

### 6.6 `docs/plans/`

The rule: a plan lives until its work merges, then goes. Keep only the plan for the work to Friday.

| Doc | Lines | Verdict | Reason |
|---|---|---|---|
| `2026-10-18-shared-manual-seal.md` | 46 | delete | Done. |
| `2026-10-19-research-readiness.md` | 110 | delete | Done or dropped. |
| `2026-10-20-event-readiness.md` | 221 | delete | AGENTS calls it the plan to 6 Nov (`AGENTS.md:25`); the owner decisions of 29 Oct replace it. WP-C disk (`:103-111`), StorageFull rule (`:207`). Keep its event-week ops lines (on call, deploy freeze, `:188-200`) by moving them to OPERATING. |
| `2026-10-23-release-is-the-list.md` | 198 | delete | Partly cancelled (`:3`). Freezes the control API (`:120`, `:140`) and the schema-1 fixtures (`:171`). |
| `2026-10-28-one-capture-system.md` | 368 | delete | Merged. "v5 chain frozen" (`:27-29`). |
| `2026-10-28-one-capture-system-spec.md` | 73 | delete | Merged. "Frozen, not deleted" (`:57-58`). |
| `2026-10-28-one-capture-system-ledger.md` | 41 | delete | L33 caps (`:41`). Any ledger entry still binding moves to DECISIONS. |
| `2026-10-29-next-prompts.md` | 79 | delete after use | Prompts for this audit and the 2-hour run. |
| `2026-10-29-record-everything-spec.md` | 81 | delete | Rule 8 "protocol v5 for the event" (`:41-44`) is overruled. |
| `2026-10-29-record-everything-plan.md` | 328 | delete | Tasks 1 to 12 merged. "Protocol v5 is frozen (D54)" (`:25-27`); byte-for-byte pass line (`:307`). |
| `2026-10-29-record-everything-status.md` | 213 | delete | Status at build end; its open items are now owner decisions. |
| `2026-10-29-record-everything-phase2-notes.md` | 936 | delete after the cut plan lands | Input only; "where they propose more code than these decisions, the decisions win" (`progress.md:99-100`). Several sections propose more code than decided (7.3). |
| `2026-10-29-record-everything-progress.md` | 100 | keep until Friday, then delete | The live tracker. Move the owner decisions (`:80-100`) to VISION first. Its own `:9` ("Protocol v5 stays frozen") contradicts its `:85`. |

### 6.7 Skills and sub-READMEs

| Doc | Lines | Verdict | Reason |
|---|---|---|---|
| `skills/goal-cup/SKILL.md` | 34 | keep | Compiled in. Accurate. Short and plain (point 6 for the agent). |
| `skills/goal-cup-setup/SKILL.md` | 106 | keep | Accurate. |
| `skills/adding-a-capture-harness/SKILL.md` | 39 | keep | Pointer to HARNESS-ONBOARDING. |
| `skills/adding-a-capture-harness/writing-a-decoder.md` | 277 | keep | Verifier contributor guide. `:220` asks to scan committed fixtures for e-mail addresses: that is fixture hygiene for a public repo, not runtime masking, so no conflict. |
| `skills/adding-a-capture-harness/templates/harness-facts.md` | 74 | keep | Template. |
| `server/worker/README.md` | 471 | rewrite | Absorb SERVER-DESIGN; move the operator runbook (`:129-384`) to OPERATING so it lives once. `:81` and `:119`: `RUN_MAX_COMPRESSED_BYTES` 64 GiB is an open owner decision; when kept, the text should say the cost it guards (VISION "a limit ... says so"). Also cites D70 at `:322,356`. |
| `packaging/README.md` | 101 | keep | Release steps. |
| `scripts/README.md` | 79 | keep, edit | Follows the scripts lane cuts (campaign, t25, research-*). Drop `:31` (`t25-disk-full.sh`) and `:79` (post-event TODO). |

## 7. Every doc line that conflicts

Grouped by the decision it breaks. Lines in docs marked "delete" above go with the file; they are listed so a
reviewer can see nothing binding is lost. Lines in kept docs must change by Friday. The matching DECISIONS
entries are named in each heading.

### 7.1 Disk full: "no special handling" (owner) versus D65 pause, D40 reserve, D37 clause 3 and the disk email

Kept docs (must change):

| Line | What it says |
|---|---|
| `AGENTS.md:84-86` | Hard rule exception: D65 freezes the agent on a nearly full disk. Also conflicts with point 3 (it stops the harness). |
| `README.md:79-80` | Under 1 GiB free the agent is paused until space returns. |
| `docs/RULES.md:322-323` | A full disk is a red flag, `StorageFull` gap. Keep the "a gap is reviewed" idea, drop the special status. |
| `docs/RULES.md:324-325` | A nearly full disk pauses the agent (D65). |
| `docs/PARTICIPANT-GUIDE.md:242-243` | `goalcup` pauses the agent under 1 GiB. |
| `docs/PARTICIPANT-GUIDE.md:324` | The 5 GiB warning line. Conditional: goes if the reserve and warning go (lane F-os recommends cutting the reserve). |
| `docs/PARTICIPANT-GUIDE.md:325` | "The disk is almost full, so your agent is paused" and "goalcup froze the agent". |
| `docs/OPERATING.md:457` | Edge case 4, "Disk nearly full", expects the pause. |
| `docs/OPERATING.md:691` | Prune at once when the 3 GiB reservation or 5 GiB free is passed. Conditional on the code lane. |
| `docs/ARCHITECTURE.md:146` | "one disk reservation per run". Conditional. |
| `docs/ARCHITECTURE.md:245-249` | Pruning at once under disk pressure. Conditional. |
| `docs/UPLOAD-API.md:380`, `:388`, `:399-407`, `:554`, `:586`, `:595` | Heartbeat `agent_paused`, the disk pause, the `disk_full` email. |
| `docs/SERVER-DESIGN.md:137-147` | `disk_full` notice and `disk_paused_at`. |
| `TODO.md:124-128`, `:142-144` | WP-C small version and D65 drill results presented as current. |

History docs (go with the file): `docs/plans/2026-10-20-event-readiness.md:103-111`, `:188`, `:207`;
`docs/research/LOCAL-NEXT-STEPS.md:20`, `:96`, `:144-148`, `:233`; `docs/research/NOTIFICATIONS.md:28`, `:55`, `:126`, `:142`;
`docs/research/LOCAL-TEST-DAY.md:124`; `docs/research/LOCAL-CAMPAIGN.md:202`, `:206-208`;
`docs/research/VERCEL-COMPAT.md:40`; `docs/research/coverage-matrix.md:63`;
`docs/research/results/2026-10-22-local-test-day.md:87`; `docs/research/results/2026-10-24-real-machine-round.md:113`;
`docs/research/t25-disk-full.md` (whole file); `docs/archive/PARTICIPANT-GUIDE-0.6.md:330`, `:376`.

What to write instead (one line each, in README, RULES and the guide): "If the disk fills, recording stops with a
gap and the harness may stop. Free space and run `goalcup continue`." Lane C-daemon also asks for a plain terminal
line when the chain writer stops (point 6); the guide's message table should list that line once it exists.

### 7.2 "Protocol v5 is frozen" and other backward compatibility (owner: no longer applies; D42, D43.6, D52, D53, D54, D63, D68, D69, D70, D71, D73)

Kept docs (must change):

| Line | What it says |
|---|---|
| `AGENTS.md:36` | "v5 records" in the map. Minor: write "chain records". |
| `README.md:278`, `:283` | P2 tests include the "v5 corpus"; "a test that guards one of these is never deleted". When the protocol changes, the corpus is regenerated, so the rule needs "or replaced with the protocol". `AGENTS.md:101` repeats the never-delete rule. |
| `docs/ARCHITECTURE.md:191-194` | "The v5 freeze": any new field is v6; a verifier change that invalidates a v5 trace needs a DECISIONS row. |
| `docs/CONTRACTS.md:3`, `:268` | Scope stated as "protocol v5" only. |
| `docs/CONTRACTS.md:510-571` | The whole freeze section, its two exceptions and the reader rule. |
| `docs/CONTRACTS.md:603`, `:608` | "frozen v5 records". |
| `docs/CONTRACTS.md:638-654` | Control API versions 5 to 12 with what each added or removed: change history kept for compatibility. |
| `docs/CONNECTOR-SCHEMA.md:94-135` | "The compatibility promise" for schema 1, held by six frozen fixtures (`:118-127`). |
| `docs/CONNECTOR-SCHEMA.md:110-111`, `:704` | `providers` table frozen (D74): "no row is added either". |
| `docs/CONNECTOR-SCHEMA.md:136-159` | The deprecation rule (notice and a warning before refusing). |
| `docs/CONNECTOR-SCHEMA.md:957-984` | Pin evolution and schema 2 rules. |
| `docs/OPERATING.md:634-636` | Provider outages are not in the trace because "the frozen v5 chain has no record kind for an outage". Point 5 wants it in the record; the reason no longer holds. |
| `TODO.md:246` | Clean-ups wait for "the next `CONTROL_API_VERSION`". |
| `TODO.md:262` | Keep as is: "the v5 corpus (add cases, never regenerate to pass)". |

History docs: `docs/plans/2026-10-29-record-everything-progress.md:9` (contradicts `:85` in the same file);
`docs/plans/2026-10-29-record-everything-plan.md:25-29`; `docs/plans/2026-10-29-record-everything-spec.md:41-44`;
`docs/plans/2026-10-29-record-everything-status.md:52`, `:106`; `docs/plans/2026-10-28-one-capture-system.md:27-29`;
`docs/plans/2026-10-28-one-capture-system-spec.md:57-58`; `docs/plans/2026-10-29-record-everything-phase2-notes.md:273`, `:371`, `:617`;
`docs/plans/2026-10-23-release-is-the-list.md:45`, `:120`, `:140`, `:171`; `docs/research/RELEASE-IS-THE-LIST.md:8`, `:52`;
`docs/research/results/2026-10-vercel-gateway.md:32`.

### 7.3 Caps of our own on subagents, streams, files and connections (D56, D57, D58, D74)

Owner: raise or remove the 128-per-entry and 32-native stream limits and the 1024-file cap; no caps of our own; a
limit must guard a real cost and say so.

| Line | What it says | Note |
|---|---|---|
| `docs/harnesses/claude-code-cli.md:163-170` | 32 open streams, 1024 tracked files, the 33rd append-only file waits for the whole run. | Kept doc. Rewrite after the code change. |
| `docs/HARNESS-ONBOARDING.md:198-202` | "A per-run native ceiling. All native rows of one run share 32 open streams and 1024 tracked files." | Kept doc. |
| `docs/CONTRACTS.md:28` | `MAX_OPEN_STREAMS` 128 per entry. | Code: `crates/goalcup-protocol/src/capture.rs:58`. |
| `docs/CONTRACTS.md:29`, `:207-210` | 128 captured connections; past that a connection is relayed uncaptured (`capture_table_full`, a point 1 loss). Past 1024 connection tasks it is answered `503` (`refused_over_capacity`), which blocks the harness (point 3). | With thousands of subagents these are reachable. Each should state the cost it guards (memory, file handles) or go. |
| `docs/CONTRACTS.md:543-544`, `docs/CHEAT-CHECKLIST.md:125`, `docs/UPLOAD-API.md:332`, `:337` | Sealed-connection list: at most 10,000 rows plus a dropped count, at most 8 MiB. | A cap on what is uploaded (point 5). Say the cost, or raise. Same verdict as D74. |
| `docs/plans/2026-10-29-record-everything-status.md:54-55`, `:100-101`; `progress.md:33`, `:63` | After five recorder restarts the recorder refuses calls for the rest of the run. | Point 3: a refused call blocks the harness. No kept doc states it, so a participant is never told (point 6). Related: D58's `MAX_CONNECTION_FAILURES` (10 in a row) ends the wire leg (`crates/goalcup-cli/src/proxy/ingest.rs:104`). |
| `docs/CONNECTOR-SCHEMA.md:624`, `docs/HARNESS-ONBOARDING.md:458` | Zero to eight native rows per connector. | A schema bound, not a recording cap. Low priority. |

History docs: `docs/plans/2026-10-28-one-capture-system-ledger.md:41` (L33);
`docs/plans/2026-10-29-record-everything-status.md:40-42`, `:137-139`; `progress.md:28`, `:60`, `:87`;
`docs/plans/2026-10-29-record-everything-phase2-notes.md:255-279` (proposes eviction and reopen plus a 4096 cap: more
code than "raise or remove"), `:479`, `:604-605`, `:758-759` (a prompt rule of "at most three subagents alive at
once" for the 2-hour run: a cap of our own, and against "ask for subagents in the prompt, never force them"),
`:769`, `:884`, `:921`.

### 7.4 Byte-for-byte match of subagent files (owner: no longer required)

No kept doc and no D entry requires it. History: `docs/plans/2026-10-29-record-everything-plan.md:307` (Task 13 pass
line); `docs/plans/2026-10-29-record-everything-status.md:43-45`, `:152-153`;
`docs/plans/2026-10-29-record-everything-progress.md:72`, `:78`;
`docs/plans/2026-10-29-record-everything-phase2-notes.md:270-275` ("the Task 13 byte check"), `:436-440`.

Not a conflict (different meaning of "byte for byte"): `docs/UPLOAD-API.md:5` (client, stand-in and Worker agree on
the wire), `docs/OPERATING.md:315` (local part versus server copy), `CONTRACTS.md` canonical bytes. Keep those.

### 7.5 Masking option b: add URL password rule, drop email and Codex rate-limit rules (D14, D26, D39)

| Line | What it says | Change |
|---|---|---|
| `docs/CONNECTOR-SCHEMA.md:886` | "The shared set already covers ... e-mail addresses". | Replace with "URL passwords". |
| `docs/CONNECTOR-SCHEMA.md:205`, `:899` | Example `extra_patterns` row `codex_plan_type`. | Use a secret-shaped example (a real key field), or the Amp `wsToken` row (`docs/harnesses/amp-cli.md:200`). |
| `docs/HARNESS-ONBOARDING.md:537` | Reviewer checks "plan, billing, quota and key-fragment coverage". | "key-fragment coverage" only. |
| `docs/harnesses/codex-cli.md:135` | Rate-limit block "(measured; Goal Cup redacts it)". | "kept in the private trace". |
| `docs/RULES.md:336-348` | Lists what is masked. URL userinfo masking is stated for URLs (`:341`); bodies "matched by pattern". | Add one plain sentence: e-mail addresses and plan, credit and quota fields are kept in the private trace. Point 6. |
| `docs/PARTICIPANT-GUIDE.md:313-316` | Same summary for participants. | Same sentence. |
| `TODO.md:237` | Clean-up names `BILLING_PREFIX` and billing rules. | Drop with the TODO rewrite. |

History: `docs/plans/2026-10-29-record-everything-phase2-notes.md:412-447` (the design; its caveat at `:440` is the
reason for "URL password rule first"); `status.md:43-45`, `:152`; `progress.md:78`, `:94-95`. Keep as is:
`TODO.md:248` and `:321-323` (strip plan and billing data in the research export after the event). That is a
cleaning step on the server side, not recorder masking, and matches D26's "strip at publication".

### 7.6 Other conflicts with the seven points

| Line | Point | Issue |
|---|---|---|
| `docs/HARNESS-ONBOARDING.md:191` | 7 | Harnesses with no terminal chat mode ("one-shot or headless only") are "Not planned" (D57). If people run long jobs headless (for example `codex exec`, `claude -p`), point 7 says support it. Owner question, unverified how many participants do this. |
| `docs/RULES.md:216-225` | 7 | Bans flags that switch capture off. Fine. Lists OpenCode, OpenClaw and Cursor flags while main ships no such connectors (`docs/RULES.md:403-408`). Keep only for shipped harnesses, or say "if a connector ships". |
| `AGENTS.md:24-30` | none | "Read first" sends agents to a 20 Oct plan, a 22 Oct status and the 18 Oct research target. VISION should be first. |
| `docs/ARCHITECTURE.md:14-17` | none | Same stale pointers. |
| `README.md:150-151`, `docs/ARCHITECTURE.md:212-213` | 6 | "No header is recorded." False since Task 7 (#293): header streams per exchange, credentials masked (`crates/goalcup-cli/src/proxy/headers.rs`). The participant is told less than is recorded. |
| `docs/ARCHITECTURE.md:253-256` | none | "Deliberately not built: ... automatic recording proxy restart". Built in Task 10 (`crates/goalcup-cli/src/proxy/mod.rs:1022`, `MAX_RECORDER_RESTARTS = 5`). |
| `docs/HARNESS-SETUP.md:194-195` | 6 | "Hermes 0.21.1 and later write no session file, so Goal Cup records the model calls only." Task 6 (#297) copies `state.db` once at finish. Unverified for every Hermes version. |
| `docs/HARNESS-SETUP.md:424` vs `docs/plans/2026-10-29-record-everything-progress.md:74` | 2 | One says Hermes `delegate_task` started two subagents; the other says it fails in 0.19.0. Different versions perhaps; the guide should name the version. |
| `TODO.md:213` | 1 | "Read Hermes's `state.db` (not in 0.7.0)": done on main by Task 6. |
| `docs/RULES.md:40-43`, `docs/PARTICIPANT-GUIDE.md:93-100` | none | 0.6.1 dry-run advice. Stale once the Friday release is the minimum. |
| `README.md:53-54` | none | "A dry run is open from 23 October". Stale after 6 Nov. |
| `docs/RULES.md:112`, `README.md:72,102` | none | Promise `goalcup time-left` and the `goalcup mcp` tool (D66). If the owner cuts `goalcup mcp`, the README lines go; RULES holds with the command alone. |

## 8. Order of work

1. **Before the code PRs merge, in one docs PR:** VISION gains the owner decisions; AGENTS read-first and Hard rules
   fixed; the three false statements fixed (headers, proxy restart, Hermes). These are true today and need no code
   change.
2. **With each code PR (same PR):** disk lines (7.1) and the D65, D40 reserve and D37 clause 3 retirements with the
   CLI pause removal; cap lines (7.3) and D57, D58, D74 with the native limit change; masking lines (7.5) and the
   merged D14 rule list with the redactor change; v5 and compat lines (7.2) and D42, D43, D52, D53, D54, D63, D68
   with the protocol change. This keeps each doc and decision true at every merge.
3. **One DECISIONS PR:** cut to the short list in section 3; delete D7, D12, D23, D44, D49, D50 and the other retired
   entries; fix the stale cites in 5.2 (D12, D7 `AttachOnly`, D23, the "D7" and "D1" labels, the D48 comments) and
   re-cite merged numbers across the about 600 cite lines.
4. **One delete PR:** archive, research (except `m2-report-format.md`, moved), results, done plans,
   `RESEARCH-TARGET.md`, `LOCAL-RESEARCH-HANDOFF.md`, `docs/README.md`. Fix inbound links so `check-links.py`
   passes; update the code-comment cites in section 4.
5. **One merge PR:** CONTRACTS and TERMINATION into ARCHITECTURE; CHEAT-CHECKLIST into SECURITY; SERVER-DESIGN into
   the Worker README; runbook duplicates into OPERATING; NOTIFICATIONS and the restarts guidance into the guide.
   Update the 9 + 11 + 4 code cites.
6. **Rewrite TODO.md** to the Friday list.
7. **After the event:** CONNECTOR-SCHEMA into HARNESS-ONBOARDING; UPLOAD-API into the Worker README. D68 (connector
   contract lock) could come back then.

## 9. What could go wrong

| Recommendation or step | Risk | Mitigation |
|---|---|---|
| Retire D65 (disk pause) | On a full disk the harness's own writes fail and the record ends in a `StorageFull` gap; the tail of the run is lost. The owner accepted this (O4). | The participant message must say it plainly (point 6). |
| Retire the D40 3 GiB reserve | The chain and the harness share free space; the recorder may fail first. Same acceptance as above. | Check that nothing else reads `RUN_RESERVE_BYTES` (for example the 180 s finish or upload). |
| Raise or remove D58's live caps | Open file handles: macOS defaults to 256 per process (owner note). Each live stream may hold a descriptor; raising to thousands without raising `RLIMIT_NOFILE` turns a cap into an `EMFILE` crash. | Check handles per stream first. |
| Remove D57's 1024-file cap | A shared home with thousands of old sessions: `since_run_start` keeps old files out, but memory per tracked file grows with a week of subagents. Unverified how much state each tracked file holds. | Measure state per tracked file before removing. |
| Drop the v5 freeze (D42, D43.6, D54, D63) | Every client, verifier and Worker must ship together on Friday. A participant on an older test build gets refused or misread. | `MIN_COMPETITION_VERSION` (D70) is the guard; set it to the Friday release. |
| Retire D68 | Outside harness authors lose the promise that their connector keeps loading. | Fine before Edition 01; it could come back after the cup. |
| Cut `goalcup mcp` | Agents whose `PATH` lacks `goalcup` cannot ask the time left. | RULES (`docs/RULES.md:112`) still holds with the command alone. |
| Cut the old-clone `git-credential` helper | A 0.6.1 test clone can no longer push. | All runs so far are test data (O2). |
| Merging entries but keeping numbers | Code cites point to merged entries; a reader lands on a deleted heading. About 600 cite lines exist in crates, server and connectors. | Update cites in the same PR (raw list in section 1). |
| Removing the D74 tunnel caps | A hostile or broken harness could make a large tunnel list. | It is metadata only, uploaded once at seal; the 64 GiB run limit (if kept) still bounds storage. |
| D44 and OpenCode wire-only | Unverified that OpenCode honours `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS` for its model calls in the way the facts sheet suggests; a wire-only run with an undecoded wire would label `wire_undecoded`. | Measure one OpenCode run before shipping its connector. |
| A deleted doc held the only copy of a rule still in force | Owner decisions and ledger entries live in plan files (`progress.md:80-100`, ledger L1 to L33). | Move binding lines to VISION or DECISIONS before deleting. |
| Released binaries link to docs on `main` | `main.rs:59`, `prep.rs:896`. | Keep those paths and anchors. A participant on an older release still lands on a live page. |
| Code comments cite deleted docs | Section 4 counts; comments are not link-checked. | Grep for the path in the same PR; there are already dead cites to `docs/qualification/`. |
| CI fails on dangling links | `check-links.py` in CI and full gate. | Delete and relink in one PR; run the link check locally. |
| Docs run ahead of code | Writing "no disk pause" before the pause is removed tells participants something false. | Change disk, cap and masking text in the same PR as the code (step 2). |
| CHEAT-CHECKLIST merged too small | Judges and lane L rely on it to know what a Pass does not prove. | Keep every row lane L marks "must still show"; only drop rows the cut makes moot. |
| Losing measured facts | Results files hold the only record of some runs (for example D65 drill timings, Claude 2.1.299 brotli fix). | One "Measured runs" table keeps harness, version, OS, date and result. Raw detail stays in git history. |
| Moving `m2-report-format.md` trips the version check | `scripts/check-version.py:72` skips `docs/research/` only. | Run the version check after the move; unverified whether the file names a version. |
| OPERATING grows too big after the merges | It is already 863 lines. | Cut the matrix section (281 lines) first; the runbook duplicate removal more than pays for SERVER-DESIGN. |

## 10. Not verified

- The 2-hour run report `docs/research/results/2026-10-30-two-hour-run.md` is not on any of the 89 remote branches,
  so no decision or doc was checked against it. Do not cite it anywhere until it exists.
- Whether a warn-only version probe still exists in `crates/goalcup-cli/src/harness_version.rs` (D12 comment).
- How much of `crates/goalcup-cli/src/account/github.rs` (466 src lines) is only the old-clone helper.
- Whether OpenCode's wire traffic decodes, and whether it honours `HTTPS_PROXY` and `NODE_EXTRA_CA_CERTS`.
- Whether a week with thousands of subagents passes 10,000 tunnels, and how much state each tracked native file holds.
- The total src and test lines behind the disk pause and reserve (about 340 src and 2,150 test lines, plus unmeasured parts of `watch.rs`, `phases.rs`, `participant/status.rs`).
- Whether `m2-report-format.md` holds version strings that `check-version.py` would flag after the move.
- Whether `state.db` copy at finish holds for every Hermes version, and how many participants run harnesses headless.
