# Capture research: what we tested, and what it changed

> **A dated measurement, not a status.** Everything here was measured on 2026-09-03, and it is correct as that. The build has moved since; `STATUS.md` is where it stands, and `docs/PRD-tracker.md` is what it is required to do. Section 8 remains the errata for `docs/harness-research/`.

Date of the measurements: 2026-09-03. Every number below comes from a spike that ran on one
macOS 26.5.2 (arm64) machine against isolated installs of each harness. This public cut retains
the measured synthesis, the included redaction utility and the redacted fixture trees that are
linked below. The per-spike `REPORT.md`/`NOTES.md` files, private per-harness rerun scripts and
configs, and raw outputs remain omitted historical records; their identities remain named where
they support a finding, but they are not required or claimed to be present in this cut. The
shipped Codex App generator is the narrow fixture-only exception. Its `--self-check` mode
deterministically validates the committed A9 fixture/sidecar and scrub wiring without launching a
harness; full regeneration requires an explicitly supplied historical artifact root plus local
Codex session/config/workspace inputs and writes fixture outputs. Those private inputs are not
shipped, and the generator does not recreate the private harness run.

This document exists because the tracker makes a strong claim, and a reader deciding whether to
run it, or whether to believe someone else's entry, deserves the evidence the design rests on
rather than the design's own confidence.

Contents: [1. The claim](#1-the-claim-we-have-to-prove) ·
[2. Method](#2-method) · [3. Results per harness](#3-results-per-harness) ·
[4. What we learned](#4-what-we-learned-across-harnesses) ·
[5. Why the tracker has this shape](#5-why-the-tracker-has-this-shape) ·
[6. Limits](#6-limits-and-what-we-did-not-test) · [7. Reproducing](#7-reproducing-this) ·
[8. Corrections](#8-corrections-to-our-own-research)

---

## 1. The claim we have to prove

The /goal Cup rule that makes the event mean anything is that one person types one prompt and
then stops helping. After 168 autonomous hours we have to be able to show, from a record
collected on the participant's own machine, that no human instruction reached the agent after
that prompt except the exact phrase `keep going`. Participants authenticate their harness with
an ordinary consumer subscription, so we cannot put a proxy in front of the model: changing
the base URL would break the auth those plans use and would put us in the middle of traffic we
have no business reading. That single constraint decides most of the architecture. It means
the evidence has to be whatever the harness already writes to local disk plus whatever the
harness's own extension surface will tell us, and it means we had to find out, by running
them, exactly what each harness writes and what its hooks actually do.

---

## 2. Method

### 2.1 Nine spikes, one per harness and mode

We ran one capture spike per harness, and one per mode where the modes differ enough to have
separate answers. Claude Code and Codex each got two, because a desktop app and a CLI are not
the same harness from a capture point of view.

| # | Spike folder | Harness and mode |
|---|---|---|
| 1 | `spikes/claude-desktop/` | Claude Desktop, attach |
| 2 | `spikes/codex-cli/` | Codex CLI, launch |
| 3 | `spikes/codex-app/` | Codex desktop app, attach |
| 4 | `spikes/claude-code-cli/` | Claude Code CLI, launch |
| 5 | `spikes/opencode/` | OpenCode, launch |
| 6 | `spikes/dsh/` | DeepSeek Harness, parked (prep only) |
| 7 | `spikes/hermes/` | Hermes, launch |
| 8 | `spikes/openclaw/` | OpenClaw, launch via Gateway |
| 9 | `spikes/cursor-cli/` | Cursor CLI, launch |

Order followed risk. Claude Desktop went first because it carried the riskiest assumption in the
plan, that an attach-mode desktop app honours folder-scoped hook config, and Cursor went last
because we expected its research file to be the least wrong. Both expectations were backwards.

### 2.2 The same script every time

Each spike ran the same nine-step script, so a difference between two reports is a difference
between two harnesses rather than between two people improvising:

1. A goal that forces one file write and one shell command.
2. Wait for the turn to complete.
3. Ask for a subagent, where the harness has one.
4. Let it stop, then type `keep going`.
5. Kill the process, relaunch in resume mode, type `keep going` again.
6. Switch model mid-session, then ask one trivial question.
7. Paste an over-length message of 4,100 characters and see whether a blocking hook refuses it.
8. Type a message that is neither the goal nor `keep going`. This is the negative fixture.
9. Stop.

A parallel session in a second folder ran during each spike, to test whether capture leaks
across folders.

### 2.3 The eleven assertions

Every report answers the same eleven questions. Abbreviated:

| # | Assertion |
|---|---|
| A1 | The goal text is recoverable byte-exactly from leg 1 and appears in leg 2 with the same bytes. |
| A2 | Every human message appears exactly once in the derived human-message list, and no harness-authored user-slot text is classified as human. |
| A3 | Tool call and tool result pair up, the result the model saw is present, and large output spills where documented. |
| A4 | A subagent transcript exists where documented, with a parent link. |
| A5 | Stop, kill and resume: the resumed session appends to or links to the same store, fresh start is distinguishable from resume, hooks fire again. |
| A6 | Model identity is present per message or per turn, and a mid-run switch is visible. |
| A7 | The over-length message is blocked, or recorded as a violation if the harness cannot block. |
| A8 | The disallowed message is present and classifiable. |
| A9 | Hooks fired only for the session in the tracked folder; a parallel session elsewhere produced no post. |
| A10 | Nothing app-wide was modified. |
| A11 | Tailing kept up: no lines missing against the final store, hash chain continuous. |

### 2.4 Two legs, always

Every spike captured two independent streams. Leg 1 is whatever the harness persists on its
own: JSONL for Claude Code, Codex and dsh, SQLite for OpenCode, Hermes, OpenClaw and Cursor,
plus raw API request and response bodies on Claude Code, where the harness offers a first-party
switch for them. A historical stub tailer (`spikes/stub-tailer.py`, omitted from this public
cut) copied every appended line or row into a local file with a receive time and a running
SHA-256 chain, so the report could say whether the tail kept up and whether anything was
rewritten behind it. Leg 2 is the harness's own hook or plugin surface, configured
folder-scoped inside the tracked workspace, posting JSON to a historical stub receiver on
loopback (`spikes/stub-receiver.py`, also omitted) that appended every POST body as one line
with receive time, path and headers.

The point of two legs is not redundancy. Leg 1 is a file the participant owns and can edit, and
leg 2 is a stream the participant can turn off, so neither is trustworthy alone. Forging both
consistently, in real time, before the next server acknowledgement, is a much larger job than
editing either. The spikes then found three harnesses where the legs are jointly necessary
rather than merely corroborating, because part of the record exists in only one of them.

### 2.5 Isolation, redaction, fixtures

Each harness was installed under `harnesses/` with its own home directory, pinned through the
harness's documented environment variable (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`, the `XDG_*` set
for OpenCode, `HERMES_HOME`, `OPENCLAW_STATE_DIR`, `CURSOR_CONFIG_DIR` plus two more for
Cursor). Claude Desktop and the Codex desktop app have no such pin and were run as documented
exceptions, bracketed with before-and-after hashes of the app-wide files they touch.

Raw spike output landed in `spikes/<harness>/out/`, which stays local, not in git, because
transcripts contain tokens and personal context. Anything committed as a fixture goes through
`spikes/redact.py` (provider key prefixes, bearer tokens, private key blocks, cloud account
ids, emails), then a structural scrub that replaces system prompts, tool schemas and account
identifiers with placeholders stating their length, then an eye review. For harnesses with
committed fixtures, the included `fixtures/README.md` lists the per-file hashes, the redaction
log and every structural edit.

### 2.6 Human-typed versus orchestrator-driven, and when they are the same thing

Four spikes had a person at the keyboard. Five did not, because the harness's interface is a
terminal UI an automated runner cannot type into. The distinction matters and every report
states which it was.

| Spike | Driven by | Equivalent to a human at the keyboard? |
|---|---|---|
| 1 Claude Desktop | human, GUI | yes, this is the participant path |
| 2 Codex CLI | human, TUI | yes |
| 3 Codex app | human, GUI | yes |
| 4 Claude Code CLI | human, TUI (a script killed the pid) | yes |
| 5 OpenCode | orchestrator over the HTTP API | yes, by construction |
| 7 Hermes | orchestrator writing to the REPL's stdin | yes, same code path |
| 8 OpenClaw | orchestrator running `openclaw agent --session-key` | yes, same Gateway path |
| 9 Cursor CLI | orchestrator with `-p`, plus a PTY probe | **no**, and that is the finding |

On OpenCode the equivalence is not an argument, it is the harness's own limitation: a message
carries no client marker at all, the TUI and the HTTP API both call the same prompt path, and
so an API-submitted message is indistinguishable from a typed one in both legs. On Hermes the
classic REPL reads the same line stream whether stdin is a terminal or a pipe. On OpenClaw the
CLI and the TUI both arrive as `channel: "webchat"` with `trigger: "user"` and neither stamps
provenance, so they are indistinguishable by construction.

Cursor is the counter-example, and it is why the distinction is worth stating rather than
waving away. Three of the eighteen hooks fire only on a real terminal, including the one that
carries the human's text, so a print-mode run answers a different question from a terminal run.
That spike added a PTY probe and labels every result by mode. The four terminal UIs we could not
drive still owe us a pass with a person at the keyboard, and that is an open item rather than a
detail.

---

## 3. Results per harness

### 3.1 The table

Pass counts are the mechanical checker's; each report's own reading is in the notes below.

| # | Harness | Version tested | Mode | Tier | A pass / partial / fail | What worked | What failed |
|---|---|---|---|---|---|---|---|
| 1 | Claude Desktop | app 1.44121.4, bundled Claude Code 2.1.258 | attach | `elevated_review` | 7 / 3 / 1 | folder-scoped hooks for 16 of 17 events; byte-exact goal on three sources; blocking works; tail byte-perfect | no `SessionStart` and no resume marker; raw bodies reachable only through app-wide config, which then captured 23 foreign request bodies; A10 unpassable |
| 2 | Codex CLI | 0.153.0 | launch | `full` | 11 / 0 / 0 | append-only rollout with gap-free `ordinal` across a resume; hooks fire inside subagents with an `agent_id`; prompt blocked end to end | untrusted hooks are skipped in silence; the npm wrapper is not the session, so `kill` did not end it |
| 3 | Codex desktop app | ChatGPT app 26.825.31414, embedded codex-cli 0.150.0-alpha.12.2 | attach | `elevated_review` plus mandatory arming | main run 3 / 4 / 4; probe P3 re-scored A1 A2 A3 A7 to pass | after CLI-granted trust: `SessionStart`, `UserPromptSubmit` byte-identical to leg 1, tool pairs, blocking | ships no way to trust a hook, so out of the box leg 2 is empty; no resume marker; leg 1 is 97.7 % foreign bytes; the app self-updated mid-run |
| 4 | Claude Code CLI | 2.1.259 | launch | `full` | 10 / 1 / 0 | complete raw request and response bodies that survive SIGKILL; process-scoped pins leaked nothing; `--session-id` pins filename, records, hook payloads and body metadata | `SessionStart` never dispatches to an `http` hook, silently; `UserPromptSubmit` also carries harness-authored prompts |
| 5 | OpenCode | 1.18.27 | launch (`serve`, HTTP-driven) | `full`, conditional | 11 / 0 / 0 | plugin auto-loads from one file in the workspace with no global config; `event` table gap-free across `kill -9`; transform hooks give a lossless view of the model input | store is home-wide; the length block works only by throwing an undocumented defect; spill file is itself capped |
| 6 | dsh | 0.1.1-rc.2 installed | not run | none | not run | nothing measured | see 3.10 |
| 7 | Hermes | 0.19.0, schema v22, 23 hooks | launch (REPL over stdin) | `elevated_review` | 11 / 0 / 0, A7 recorded rather than blocked | byte-exact conversation array on leg 2; `api_content` sidecar exposes the harness's own injections; both a foreign key and a JSON marker for the subagent link | the plugin allow-list key cannot be folder-scoped; `--safe-mode` is a one-flag capture bypass; no user-message rejection surface at all |
| 8 | OpenClaw | 2026.8.2 | launch (Gateway) | `elevated_review` | 8 / 1 / 2 | a real input gate that refused a prompt in 68 ms; system prompt visible on two hooks; skill discovery genuinely isolated | hooks hot-disable in 0.7 s with no restart; subagent transcripts are deleted to an archive; SQLite rowid reuse defeated our tailing cursor |
| 9 | Cursor CLI | 2026.09.02-c22c1a3 | launch (`-p` plus PTY probe) | `elevated_review` | 5 / 3 / 3 | the chat store is a complete content-addressed Merkle tree with full tool results and the whole system prompt; real per-directory partitioning; a working input gate on a TTY | `beforeSubmitPrompt` does not fire in print mode, so print mode has no human witness and no length gate; three app-wide writes survive all three env pins |

Tier summary as measured: `full` for Codex CLI, Claude Code CLI and OpenCode; `elevated_review`
for Claude Desktop, the Codex app, Hermes, OpenClaw and Cursor; unrated for dsh.

`elevated_review` is not a failing grade. It means an organizer reads the trace before the
entry is accepted, rather than an automatic seal.

**Public-cut evidence notation.** A reference such as `spikes/<harness>/REPORT.md` below is the
identity of an omitted historical report, not a link to a required file. The measured finding is
preserved here. Where a redacted fixture inventory is included in the public cut, it is linked
directly; otherwise the relevant public section is the evidence available in this document.

### 3.2 Claude Desktop (spike 1)

The riskiest assumption held: the desktop app does honour a `.claude/settings.json` written
into the opened folder, and it proves it in the transcript, because each dispatch leaves a
record naming the URL it called. Sixteen of seventeen configured events fired. The one that
never fired was `SessionStart`, and it did not fire on resume either, so the session boundary
is invisible: the transcript goes from the pre-quit record to the post-reopen record with a gap
and nothing in between. The strongest evidence source here is the raw API bodies, and getting
them requires editing app-wide settings that then apply to every Claude Code process on the
machine, which in a six-minute run swept in request bodies from three other sessions, one in an
unrelated folder. Two side findings shaped the design more than the assertions did: the agent
read the tracker's own settings, receiver log and hook stream out of the parent directory, and
five subagent-stop posts turned out to be the interface's follow-up-suggestion generator,
carrying text that reads exactly like a human turn and pointing at transcript files that never
exist. Evidence: omitted historical report `spikes/claude-desktop/REPORT.md`, the included
[redacted fixture inventory](../spikes/claude-desktop/fixtures/README.md), A5, A9, A10 and
Surprises 1 and 3.

### 3.3 Codex CLI (spike 2)

The strongest harness we measured, on a version three minors ahead of the research file, and
the version difference mattered: the record type that file named as the human-input marker does
not exist on 0.153. What works is unusually clean. The rollout is append-only with a gap-free
ordinal that stayed contiguous across a kill and a resume, which is a free gap detector; hooks
are discovered from the working directory rather than the git root, so a tracked workspace does
not need to be a git repository; and a blocking prompt hook stopped the over-length paste from
reaching the model or the rollout. Both failures are operational. Untrusted hooks are skipped
with no prompt, no warning, no stderr and exit code 0, so a session can run with the tracker
blind and nothing on screen says so. And `codex` on npm is a wrapper: killing it left the native
binary and a helper alive, holding the thread writer lock, and the resume that could not take
that lock silently became a brand-new thread with no reference back to the original. Evidence:
omitted historical report `spikes/codex-cli/REPORT.md`, the included
[redacted fixture inventory](../spikes/codex-cli/fixtures/README.md), A5, Unknowns 2 and
Surprise 1.

### 3.4 Codex desktop app (spike 3), and the trust detour

This is the spike that most deserves reading in full, because its conclusion reversed.

The main run produced a zero-byte hook file. Eleven minutes, seven prompts, eight tool calls, a
subagent, a quit and reopen, a model switch, and not one hook post. The cause was neither a
capture bug nor the trust gate refusing us. The app discovers, loads and dispatches
folder-scoped hooks correctly, but on this build it ships no way to review or trust one: no
hooks settings page routed in the interface, no composer command. Codex skips untrusted hooks
in silence, which spike 2 established on the CLI, so the app ran blind. The trust store in
`~/.codex/config.toml` gained zero entries across the whole run, which is the independent
confirmation. The first draft of that report recommended declining the harness.

Probe P3 overturned it. Grant the trust once from the standalone Codex CLI in the same folder,
using its `/hooks` review, quit, and then open a new thread in the app. The next app thread
fired thirteen posts across six event types, blocked the over-length prompt end to end, and
reconciled byte-for-byte with leg 1. The trust written by CLI 0.153.0 was honoured by the app's
embedded 0.151.0-alpha.7.1.

Three constraints came with the recipe, and each is now a design requirement. Hooks are read at
thread start: the already-open thread was given `keep going` after trust was granted and
produced zero posts, so the goal thread has to be opened after arming. The trust is a hash of
the hook definition, so rewriting the hook file mid-event invalidates it and requires a re-arm.
And arming writes twelve `trusted_hash` tables into the participant's real
`~/.codex/config.toml`, written by Codex's own trust interface after a human review and
containing only hashes of our own file. That is still an app-wide write and has to be declared.

The detour is a worked example of why the spikes existed. The vendor's own page says hooks are
supported in the desktop app, which is true of dispatch and false of trust, and the difference
between those two words is the difference between a supported harness and a declined one.
Evidence: omitted historical report `spikes/codex-app/REPORT.md`, the included
[redacted fixture inventory](../spikes/codex-app/fixtures/README.md), the Headline section and
Probe P3.

### 3.5 Claude Code CLI (spike 4)

The best-evidenced harness we have. Raw request and response bodies are complete, carry the
whole system prompt and every tool schema, and survive a hard kill cleanly: the in-flight
request file was complete valid JSON a second before `kill -9`, its response file simply absent,
and none of the body files was truncated. A billing header inside each request supplies the
linkage the research file assumed did not exist, tying a body to its transcript turn, to the
previous response, and to whether the call was a subagent's. Process-scoped pins did what
app-wide settings could not: a second run in another folder under the same config directory
produced zero hook posts and zero body files, and stays detectable because it leaves a second
project slug in that directory. Two findings changed rules rather than code. `SessionStart`
never dispatched to an `http` handler from either configuration source, and the failure is
completely silent, while a `command` hook on the same event from the same file fires normally,
at startup and on resume, though one resume fires it twice. And the prompt hook carries
harness-authored prompts: a subagent-completion notice arrived through `UserPromptSubmit` with
no meta flag on its transcript record. Evidence: omitted historical report
`spikes/claude-code-cli/REPORT.md`, the included
[redacted fixture inventory](../spikes/claude-code-cli/fixtures/README.md), the SessionStart
proof, A2 and A9.

### 3.6 OpenCode (spike 5)

The cleanest arming story of any harness. A single TypeScript file in
`<workspace>/.opencode/plugin/` loaded with no global configuration of any kind, and the
app-wide check passed with an empty diff, which no Claude harness manages. The durable event
table stayed gap-free across a `kill -9` and a process change, with the sequence advancing by
exactly one over a forty-one second hole. Three qualifications. The database is home-wide, so
leg 1 must be partitioned by session directory at capture time. The length block works by
throwing from a hook that is not documented as refusable, which aborts the turn and returns an
opaque 500 to the participant, so the connector has to verify the absence of the row rather
than trust the block. And the spill file for large tool output is itself capped, so the
untruncated bytes exist nowhere. The spike also found a real side channel: a documented shell
endpoint runs a command inside the tracked session, writes the full output to leg 1, and fires
no hook at all. Evidence: omitted historical report `spikes/opencode/REPORT.md`, the included
[redacted fixture inventory](../spikes/opencode/fixtures/README.md), A3, A9 and Surprise 3.

### 3.7 Hermes (spike 7)

Everything about Hermes is `full` grade except one key. Two genuinely independent legs, a
byte-exact conversation array on leg 2, a sidecar column that exposes the harness's own
injections into the human's message, both a foreign key and a JSON marker for the subagent
link, per-call model and billing identity, and a resume that appends to the same session row.
The blocker is that arming needs the plugin listed in an allow-list key that exists only in the
home-level config file; there is no project-level config on this version, so that key cannot be
folder-scoped, and every alternative visible in the source is worse. Two further findings are
rulebook material: `--safe-mode` skips plugin discovery entirely, so leg 2 goes dark while
leg 1 keeps filling, and the environment variable alone is enough without the flag; and Hermes
has no user-message rejection surface at all, so the length rule can only be recorded, never
enforced. The isolation finding is the one worth repeating outside this repository: Hermes
seeded a credential into its isolated home by reading a host-level CLI's token with no user
action, and the same code path reads two further credential files in the home directory.
Evidence: omitted historical report `spikes/hermes/REPORT.md`, the included
[redacted fixture inventory](../spikes/hermes/fixtures/README.md), Unknowns 1 and 4, A7,
Surprise 1.

### 3.8 OpenClaw (spike 8)

The input gate is the best of any harness measured: a hook that refuses a turn before any model
call, in 68 ms, with a non-zero exit code and a message the participant can read. The system
prompt is visible on two hooks, and skill discovery is genuinely isolated, which no other
harness with an isolated home managed. Three properties keep it out of `full`. Flipping one
config key to `false` hot-applied in 0.7 seconds with no restart and the same process id, and
killed both the hook stream and the input gate, while the shim's own timer kept ticking, so
plugin liveness is not proof the hooks are armed. The store is agent-wide and the hook context
reports the Gateway's own workspace for every session, so the workspace cannot filter foreign
sessions. And the record is lossy by design in two places: completed subagent generations are
deleted from the live table into a compressed archive, and large tool results are truncated on
the way to disk with the untruncated bytes existing only in a hook payload. The spike also
produced the finding that most complicates the `keep going` rule: after a crash the Gateway
restarted the interrupted turn by itself, with no human input, recording the restart as an
`internal_system` user message. Silence after a crash is not a safe assumption. Evidence:
omitted historical report `spikes/openclaw/REPORT.md`, the included
[redacted fixture inventory](../spikes/openclaw/fixtures/README.md), A4, A5, A9, A11 and
Unknowns 4.

### 3.9 Cursor CLI (spike 9)

The research file was more wrong about Cursor than about any other harness, in both directions.
Better than expected: the chat store is a content-addressed Merkle tree whose blobs carry full
tool results including shell output, and the whole system prompt. It is the best leg-1 substrate
we measured, at the cost of needing a protobuf reader for three quarters of the rows, and its
partitioning is real, one directory per working directory with a literal path recorded beside
it. Worse than expected: the JSONL transcript everyone points at is a display artefact with no
identifiers, no timestamps and zero tool results, so it must not be tailed; three of the
eighteen hooks fire only on a terminal, and one of those is both the only place a human message
is visible as human and the only rejection surface, so print mode has neither; and isolation
needs three environment variables rather than the one Cursor documents, yet the first real agent
turn still wrote three app-wide paths in the real config directory and put the credential in the
login keychain. The spike revoked its own earlier isolation confirmation on that measurement. A
deliberate probe then deleted one hook from the config mid-chat: it stopped firing on the very
next turn with no restart and no warning, and restoring the file brought it back, detected only
by our shim's pinned-hash check. Evidence: omitted historical report
`spikes/cursor-cli/REPORT.md`, the included
[redacted fixture inventory](../spikes/cursor-cli/fixtures/README.md), A7, A10, Unknown 2 and
the hot-reload probe.

### 3.10 dsh, not run, and why

The DeepSeek Harness spike is parked at preparation only. No session was run and no model was
called. The omitted historical preparation record `spikes/dsh/NOTES.md` records what was read
out of the installed package, and three
findings from that reading explain the parking:

- Headless mode has no resume. The runner submits one task and exits with no follow-up surface,
  so the crash-and-continue leg of the script cannot run there at all. Resume exists only in the
  browser surface.
- There is no folder-scoped configuration mechanism: no project settings file, no per-repo hook
  config the harness discovers on its own. The closest thing available is a relative config path
  resolved from the launching directory, which is a launch-time property rather than a discovery
  one.
- Folder scoping fails in the other direction too. The hook config is process-wide and the
  browser surface supports several workspaces in one process, so a second session in another
  folder inside the tracked process will post. The only separator is a per-post working
  directory field.

The preparation also produced two corrections to our own research file, listed in section 8.
dsh is the only unmeasured harness and needs a real run before it can be offered at any tier.

---

## 4. What we learned across harnesses

Ten lessons. Each one is stated as what we saw, where we saw it, where the evidence is, and
what it changed.

### 4.1 A prompt hook is not a human witness

**Observation.** Every harness with subagents delivers harness-authored text through the same
prompt hook that carries the human's typing. On Claude Code a subagent-completion notice
arrived through `UserPromptSubmit` with no meta flag on its transcript record, so a meta-only
filter misclassifies it. OpenCode's task tool writes the subagent's prompt as an unmarked user
message that fires its own hook, Hermes' delegate prompt gets its own pre-call hook, and Codex
subagent rollouts replay the parent's human turns verbatim. OpenClaw persists a hook-blocked
message as an unstamped user row, and restarts an interrupted turn after a crash with an
`internal_system` user message.

**Harnesses and evidence.** All five with subagents, and OpenClaw again on restart: spike 4 A2; spike 5 A2; spike 7 A2; spike 2 A2; spike 3 A2; spike 8 A2, A5, A7.

**Consequence.** The rule "every prompt-hook post after the goal must equal `keep going`" fails
open on a legitimate run. Its inverse, "every user-slot row is human", fails closed. So the
human-message rule is two-legged on every harness: a hook post counts as human only when it
pairs with a leg-1 record that passes that harness's positive human test and the root and
workspace scoping clauses.

### 4.2 Session start is unreliable, and hooks are read at start

**Observation.** Session birth is often not observable: a project-scoped `SessionStart` over
HTTP never fired on Claude Desktop while sixteen other project hooks did, the same event over
HTTP never fired on the Claude Code CLI while fifteen other events did, and both failures were
completely silent. A `command` hook on the same event from the same file fires normally, but one
resume fires it twice. Yet where the event does exist it is decisive, because hooks are read at
session or thread start. On the Codex app, trust granted mid-session did nothing for the
already-open thread and everything for the next new one. Hermes reads its config and plugin
gates at process start, Codex CLI loads hooks per launch, and dsh reads its hook config once at
startup, running no hooks if it is unreadable while the agent starts anyway.

**Harnesses and evidence.** Claude Code CLI and Desktop for the unreliability, Codex app, Codex
CLI, Hermes and dsh for the read-at-start behaviour: spike 1 Unknowns; spike 4 "The SessionStart proof" and A5; spike 3 P3 steps 4 and
5; spike 7 Unknowns 1; spike 2 Unknowns 1; omitted historical preparation record
`spikes/dsh/NOTES.md`, "Leg 2". The measured conclusion is preserved in this subsection; dsh
has no included redacted fixture inventory because no session ran.

**Consequence.** Two changes, in opposite directions. A missing `SessionStart` must not raise
an anomaly on Claude, where session start is derived from the launch the daemon performed plus
the first transcript record, with the hook as corroboration and deduplication by session and
source. On the Codex app the same event is the required proof that arming worked, so the daemon
refuses to stamp `started_at` until it arrives.

### 4.3 Hooks can be silenced while the agent runs

**Observation.** On OpenClaw one config key flipped to `false` was hot-applied in 0.7 seconds
with no restart and the same process id, and the next full turn produced zero posts and no input
gate, while the shim's own timer kept ticking. On Cursor, deleting one event from the hook file
mid-chat stopped that event firing on the very next turn, with no restart and no warning. On
Hermes a single flag or environment variable skips plugin discovery completely, leaving a full
session row and zero posts. On Codex the trust is a hash of the hook definition, so editing the
file invalidates it.

**Harnesses and evidence.** OpenClaw, Cursor and Claude Code hot-reload; Codex, Hermes and dsh
reload on restart, and a restart is exactly what `keep going` after a crash looks like. See
spike 8 Unknowns 4; spike 9 hot-reload probe; spike 7 Unknowns 4; spike 3 P3
constraint 3.

**Consequence.** The transcript leg, not the hook leg, is the integrity anchor, and hook silence
while leg 1 keeps growing is the universal detector. But a blanket "hook silence is a flag" is
wrong, because silence is normal for Claude's `SessionStart`, for permission events in automatic
mode, and for Codex, which fires `SessionStart` at the first prompt rather than at launch. So
each connector declares, per event, whether it is always expected, conditionally silent or never
silent, and whether the harness can hot-disable it and how fast.

### 4.4 The tracker's artifacts must not sit above the workspace

**Observation.** On Claude Desktop the artifact root was the parent of the opened folder. The
agent's first tool call listed the parent directory, and two turns later it read the tracker's
settings file, the receiver log, the hook stream and the raw-body path, unprompted. On Codex CLI
a session launched one directory wrong searched the artifact root and listed a baseline copy of
the isolated home's OAuth token. Pointing the artifact root away from the workspace is not
enough either: the Codex app's agent and Cursor's agent both read globally installed skills out
of the home directory on their first tool call.

**Harnesses and evidence.** Claude Desktop, Codex CLI, Codex app, Cursor: spike 1 Surprise 3; spike 2 Surprise 6; spike 3 Surprise 4; spike 9 A10.

**Consequence.** Artifacts live in a tracker-owned directory that is not an ancestor of the
workspace and is not adjacent to the home directory. The configuration baseline hashes
credential files rather than copying them. An agent reading the tracker's own configuration is
a capture risk before it is a privacy one, because everything needed to disable or poison the
recorder is in those files.

### 4.5 Kill the process tree, not the launcher

**Observation.** A `kill -9` on the npm wrapper for Codex killed the wrapper only. The native
binary and a helper process survived, reparented, and kept holding the thread writer lock. The
resume that could not take that lock did not error and did not fork. It silently became a new
thread with its own identifier, no parent reference, and an empty conversation. Nothing on disk
links the two halves of that run. Separately, a hard kill leaves no session-end event on Codex
or on Claude Code, so process death has to be inferred.

**Harnesses and evidence.** Codex CLI directly, and the lesson generalises to any harness
distributed through a script wrapper: spike 2 A5 and Surprise 1; spike 4 A5.

**Consequence.** Each connector declares a kill strategy, by process group or native process
id, and the lock paths that reveal a degraded resume. A new user-sourced session appearing in a
tracked home during a run is an incident, not noise.

### 4.6 An isolated home is not a credential or configuration boundary

**Observation.** Hermes seeded a credential into its isolated home by reading a host-level CLI's
token with no user action, and the same code path reads two further credential files in the home
directory; it also downloaded and ran a 32.8 MB binary mid-session. More than half of OpenCode's
system prompt was a skill catalogue read from two directories no environment pin covers, and the
agent loaded one of those skills mid-run. The Codex desktop app injects the user's memory
summary and their whole global skill listing into every thread. Cursor reads the same global
skill directories and puts its credential in the login keychain, which no pin moves.

**Harnesses.** Hermes, OpenCode, Codex app, Cursor, Claude Desktop. Clean counter-examples:
Claude Code's config-directory pin produced a genuinely clean tier, and OpenClaw's skill
discovery resolved entirely inside its isolated install.

**Evidence.** spike 7 Surprise 1 and A10; spike 5 A10; spike 3 Surprise 3 and 4; spike 9 A10;
spike 1 A9; spike 4 Surprise 8; spike 8 Surprise 7.

**Consequence.** The connector pins provider and model on every launch and records the harness's
own credential listing, because a mid-run credential swap changes who paid without changing
anything in the message store. And an empty workspace is not an empty context: pre-injected
memory and host-discovered skills must be disclosed with the entry or turned off for the
duration. That is not cosmetic. In one probe the injected skill carried a design-approval gate
that stopped the agent from writing the file it had been asked for.

### 4.7 Every store needs its own cursor rule

**Observation.** The obvious cursor is wrong on at least two harnesses. On OpenClaw, deleting a
completed subagent generation freed SQLite row ids that later inserts reused, so a monotonic
row-id cursor that had already passed those numbers missed three rows of the main session,
including the row that carries the subagent parent link. On OpenCode the sequence column is
per-aggregate rather than global, so a global sequence cursor would have dropped a child
session's forty-two rows in silence. Codex's per-file ordinal is gap-free and contiguous across
a resume. Claude Code and dsh need inode plus byte offset in append order, because timestamps
inside the file are out of order. Hermes' autoincrement primary key is a true insert cursor,
with the caveat that rows can be mutated after insert. Cursor's real store is a blob table read
by row id, while the JSONL beside it is decoration.

**Harnesses and evidence.** All of them, differently: spike 8 A11; spike 5 A11 and Unknowns 3; spike 2 A5 and A11; spike 4 A11; spike 7
A11; spike 9 Unknown 2.

**Consequence.** The connector manifest declares a cursor type per store and an explicit list
of cursors that are unsafe on that harness. The tailer bug that produced this lesson was
deliberately left unfixed in the spike so the artifact would show the failure rather than a
patched-over pass.

### 4.8 Harnesses write their own text into the user slot, so catalogue it

**Observation.** Beyond subagent prompts, each harness injects its own text where the human's
text lives. The catalogue as measured:

| Harness | Harness-authored content that lands in the user slot or the prompt hook |
|---|---|
| Claude Code | subagent-completion notice; a synthetic continuation on resume; the three records the model picker injects for a slash command; the sidechain prompt; suggestion-agent output arriving as a subagent-stop payload with an empty agent type and a transcript path that never exists |
| Codex | a recommended-plugins block with the environment context concatenated into the same message, whose length varies by project; an instructions-file header; replayed parent turns inside subagent rollouts |
| OpenCode | the task tool's subagent prompt, unmarked; a synthetic message from the shell endpoint; the title generator firing the transform hooks |
| Hermes | the delegate prompt; cron and compaction prefixes; a model-switch note prepended into the wire copy of the human's message |
| OpenClaw | a subagent announcement; a restart-recovery message; the notice persisted when a hook blocks a message |
| Cursor | the subagent task prompt in the same wrapper shape as a human turn; a follow-up message on the stop hook |

**Harnesses and evidence.** All six measured with subagents or an interface layer: spikes 1 and 4 A2 and A4; spike 2 A2; spike 3 A2; spike 5 A2; spike 7 A2 and
Surprise 2; spike 8 A2 and A7; spike 9 A2.

**Consequence.** A single positive marker is never enough. Each connector carries an ordered
exclusion list next to its positive test: prefixes, provenance kinds, synthetic flags, blocked-
notice markers, and the scoping clauses that keep subagent and foreign-folder sessions out.

### 4.9 Length blocking works where a blocking hook exists, and only there

**Observation.** The 4,100-character paste was refused end to end wherever a blocking hook
exists, and nowhere else. Hermes has no user-message rejection surface at all.

| Harness | Length gate | Where the blocked text survives |
|---|---|---|
| Claude Code CLI | blocked | an informational record, the paste cache, the config-dir history |
| Claude Desktop | blocked | a queue record and an informational record |
| Codex CLI | blocked | the home-level history file |
| Codex app | blocked only after arming | leg 2 only; leg 1 shows a four-record empty turn with a null last message |
| OpenCode | aborted by throwing an undocumented defect | leg 2 only; the participant sees an opaque 500 |
| OpenClaw | blocked, 68 ms, non-zero exit | leg 2, plus an unstamped notice row |
| Cursor | blocked on a terminal, no gate in print mode | nothing persisted |
| Hermes | impossible, recorded only | the message goes through and is stored in full |

**Evidence.** spike 4 A7; spike 1 A7; spike 2 A7; spike 3 P3 A7; spike 5 A7; spike 8 A7; spike
9 A7; spike 7 A7.

**Consequence.** The rule states the length limit and the tracker enforces it where the harness
allows enforcement, records it as a violation everywhere else. The useful side effect is that a
blocked prompt is almost always still recoverable as evidence, which is good for the record and
worth telling participants: blocking stops model exposure, not persistence.

### 4.10 Tool output on disk is often truncated, so the hook leg is evidence and not a copy

**Observation.** The disk record of a tool result is frequently not the full output, and
sometimes is not even what the model saw. OpenClaw produced three divergent copies of one large
output: what the model got at execution time, a larger copy persisted to disk, and the full
bytes existing only in a hook payload.

| Harness | Fact |
|---|---|
| Codex CLI | truncated in band, no spill file, untruncated bytes exist nowhere |
| Codex app | every tool output stored four times, so leg 1 is roughly four times larger than the model-visible content needs |
| OpenCode | spills to a file capped at 192 KiB; the hook sees post-truncation output |
| OpenClaw | three divergent copies; the full bytes only in a hook payload; result details dropped for every tool at persistence |
| Cursor | full output on the hook leg and in the blob store; zero tool results in the JSONL |
| Claude Code | spill path never exercised in either spike |
| Hermes | spill path never exercised |

**Evidence.** spike 2 A3; spike 3 A3 and Surprise 5; spike 5 A3 and Unknowns 2; spike 8 A3;
spike 9 Unknown 2; spikes 1 and 4 A3; spike 7 A3.

**Consequence.** Two legs are mandatory rather than merely corroborating on Codex app, OpenClaw
and Cursor, because part of the record exists in only one of them. And the honesty statement
that belongs in the rules: "every token the agent saw" is true, "every byte a tool produced" is
not.

---

## 5. Why the tracker has this shape

Each design decision below maps to the lessons that justify it. We mark each one **confirmed**
where the spikes agreed with what we had already decided, and **sharpened** where the spikes
changed it. Nothing here was decided after the fact to match the evidence; the decisions
predate the spikes and are recorded in `docs/DECISIONS.md` with their original dates.

### 5.1 Record raw, interpret later

The daemon captures bytes and health; every judgment about what the bytes mean runs afterwards,
in the open-source verifier.

**Confirmed**, for a reason we did not anticipate. Three of the seven research files turned out
to be wrong about the record type that identifies a human message, on the versions we tested. If
the daemon had shipped that rule, those entries would have been mis-verified with no way back.
Because raw bytes are preserved, a parser mistake during judging week is a re-run.

**Sharpened** in one place. The exception list, the small set of things the daemon does judge,
grew from two signals to five: the transcript stalled while the process lives; hook posts
silent while leg 1 grows; the arming signal absent at start; a tracker config file deviating
from its expected hash; and the harness version changing mid-run. Every addition is something a
participant can act on and nobody else can detect later. See 4.2, 4.3 and the self-update in
spike 3.

### 5.2 Two legs, reconciled

**Sharpened, substantially.** We designed two legs as mutual corroboration, on the argument that
forging both is harder than forging either. That still holds. What the spikes added is that on
three harnesses reconstruction *requires* both legs: a blocked prompt's text exists only in
leg 2 on the Codex app, the untruncated tool output only in leg 2 on OpenClaw, and the only
human-text witness is leg 2 on Cursor (4.10). They also added a warning about the join. On Codex
the legs use tool identifier spaces that never match, and one leg-1 tool call can be several
hook pairs, so the join goes through turn identity and ordering rather than tool id.

### 5.3 Folder-scoped configuration only

Hook and plugin configuration is written inside the tracker-created workspace. App-wide harness
settings are never modified.

**Confirmed as the rule, sharpened into a rule with named exceptions.** Folder scoping works,
and it works better than we expected on OpenCode, where a single file in the workspace loads
with no global configuration at all, and on Codex, where hook discovery follows the working
directory rather than the git root, so a tracked workspace does not need to be a git repository.

But it cannot be absolute. Three harnesses need one named app-wide write each, and one gets a
fourth involuntarily:

| Harness | The app-wide write | Who writes it |
|---|---|---|
| Claude Desktop | the raw-body switch in the user settings file | the tracker, as a declared exception |
| Codex app | twelve hook-trust hash tables in the user config file | Codex's own trust UI, after a human review |
| Hermes | one plugin allow-list key in the home config file | the tracker, backed up and reversible |
| Cursor | a state file, a telemetry database, a skills re-sync, a keychain credential | the harness itself, despite all three pins |

Each is disclosed on the public entry, bracketed with before-and-after hashes, and reversible
where it can be. The alternative was declining four harnesses.

The app-wide check itself also had to change. On a GUI harness "nothing app-wide changed" is not
an assertion any verifier can make, because the app rewrites its own configuration during normal
use. The check became "no change to a named allowlist of files that affect harness behaviour",
with the app's own interface and telemetry state out of scope, and baselines taken immediately
before launch with modification times recorded. See 4.6 and spikes 1, 3 and 4 A10.

### 5.4 Arming before start

`goalcup start` creates the workspace and either launches the harness with pinned environment or
arms the folder and waits. The clock starts on the first prompt.

**Sharpened into a hard gate.** Because hooks are read at session start, a session opened before
arming runs blind and silent for its whole life, and nothing on screen says so. So the daemon
now refuses to stamp `started_at` until the harness's declared start signal arrives for the
tracked workspace, and the connector manifest names that signal per harness. Where the signal
does not exist, as on Claude, start is derived from the launch the daemon performed plus the
first transcript record, and a missing hook must not raise an anomaly. See 4.2.

We had already rejected the idea of arming recording from inside an already-running session.
Spike 3 turned that from a judgment call into a measurement: trust granted mid-thread produced
zero posts for that thread and full capture for the next one.

### 5.5 A per-harness human-message rule, with exclusion lists

**Sharpened, and this is the biggest change the spikes made.** We knew the human test was
harness-specific. We did not know that a single positive marker is insufficient on every
harness we measured. The rule for each connector is now a positive test, plus an ordered
exclusion list of harness-authored content, plus scoping clauses that keep out subagent sessions
and sessions in other folders, plus corroboration across both legs.

| Harness | Positive test | Required extra clauses |
|---|---|---|
| Claude Code | `origin.kind=human` on a main-session user record | corroborated by a prompt-hook post; subagent-stop payloads never feed the list |
| Codex | a user-role message in the file whose first session header is the tracked thread | harness prefixes excluded; subagent rollouts replay parent turns and must be cut at the declared ordinal |
| OpenCode | a non-synthetic user message | session directory equals the workspace, and the session has no parent |
| Hermes | an active user row with no cron or compaction prefix | no parent session, no delegate marker, session directory equals the workspace |
| OpenClaw | provenance absent | no transport or sender id, no blocked-notice marker, corroborated by a hook post with a user trigger, partitioned by session key |
| Cursor | the user-query wrapper in the blob store | no subagent info on the chat, chat directory equals the workspace |

See 4.1 and 4.8.

### 5.6 Connector manifest fields

**Sharpened.** The PRD's manifest was already declarative: detection, launch recipe, stores,
hook install recipe, human-rule id, tier. The spikes proved fifteen further fields necessary,
each traceable to a measurement:

| Field | Why, from the spikes |
|---|---|
| `arming.signal`, `arming.timeout`, `arming.recipe` | the event that proves leg 2 is live, plus the human arming step where one exists (4.2) |
| `start_signal.required` | whether `started_at` may be stamped without it: false on Claude, true on the Codex app |
| `store[].cursor.type` and `unsafe_cursors[]` | row-id reuse and per-aggregate sequences (4.7) |
| `store[].partition_key` | home-wide stores must be partitioned at capture time (5.7) |
| `human_rule.exclusions[]` | the catalogue in 4.8 |
| `human_rule.byte_exact_field`, `normalisation.trailing_newline` | one harness posts a timestamp-prefixed prompt on one hook and the exact bytes on another; one desktop app appends a newline to every message |
| `kill.strategy`, `lock_paths[]` | the wrapper kill and the silent degrade to a new thread (4.5) |
| `config_allowlist[]` | so the app-wide check is checkable on GUI harnesses (5.3) |
| `hook_silence_policy`, `hot_disable` | per event, and how fast the harness can silence it (4.3) |
| `app_wide_exception` | the declared file, key, expected diff, reversal and disclosure string |
| `version.self_updating`, `version.sample_interval` | one app updated itself mid-run with the version frozen in the session header |
| `aux_calls[]` | auxiliary model calls invisible to the transcript, on four harnesses out of four — **Removed from schema v0 in the M1a fix wave (2026-09-04):** four rows written across three connectors and no reader anywhere, and M2 freezes the schema. Held absent by `aux_calls_is_gone_from_the_schema_and_from_every_manifest` in `crates/goalcup-protocol/tests/manifest_load.rs`. The requirement below is the measurement that motivated it and stands as written. |
| `workspace_exclusions[]` | paths the harness itself creates in the workspace |
| `archive_sources[]` | stores where records leave the primary table |
| `receiver.token` | an unauthenticated loopback receiver accepted a post from another spike's leftover shim |

### 5.7 Capture-time partitioning and scope

The tracker sees only sessions in the folder it created.

**Sharpened from a scoping promise into a capture-time requirement.** Leg 2 keeps the promise
everywhere we measured: a parallel session in another folder produced zero posts on every
harness. Leg 1 does not, because most of these stores are home-wide or agent-wide by design.

| Harness | Partition key | Contamination measured in one run |
|---|---|---|
| Codex app | first session header's working directory | 20 rollout files, 2 of them ours, 97.7 % foreign bytes, plus 1,050 thread titles |
| Codex CLI | first session header | 95 of 295 rollout lines foreign |
| OpenCode | session directory | 28 of 458 event rows foreign |
| Hermes | session working directory | 1 session and 2 messages foreign |
| OpenClaw | session key; the workspace field cannot filter | a foreign session in the same store |
| Claude Desktop, raw bodies | session id inside the request metadata | 23 of 44 request bodies foreign, one from another folder |
| Cursor | the chat directory's own recorded working directory | a foreign chat in the same config directory |
| Claude Code CLI | not needed | process-scoped pins leaked nothing at all |

So the daemon partitions at capture time and discards foreign rows before hashing or shipping,
and the privacy statement in the rules has to say plainly which directories the tracker reads.
The Claude Code CLI row is the one that shows this is a property of the pinning mechanism rather
than an inevitability: process-scoped pins do not leak, app-wide settings do.

### 5.8 Health signals

**Confirmed and extended.** The daemon reports states a participant can act on, and the spikes
proved two detectors live: the tracker config hash check caught a mid-chat hook deletion on
Cursor on the very next hook, and "leg 2 silent while leg 1 grows" is the only signal either
Hermes bypass leaves.

### 5.9 Seal by workspace manifest

**Confirmed, and the attribution check was demonstrated.** At the deadline the daemon records the
workspace manifest and that is the seal. In one probe every file present in the workspace was
explained by a traced write, and the file that was *absent* was explained by traced tool calls
that only read. That is the attribution check passing in both directions.

**Sharpened** on exclusions. The workspace does not stay as the tracker wrote it: OpenCode
installs a 61 MB dependency tree to load a TypeScript plugin, a Python shim leaves a bytecode
cache, OpenClaw writes five bootstrap files on the first turn, and in one run the agent
initialised a git repository. Those paths are named in the manifest and excluded from the seal.

### 5.10 An open verifier, and one connector bar

**Confirmed.** Every connector, first-party or contributed, passes the same public conformance
suite: a manifest, recorded fixtures from a real session, and a parser. The spikes are the
reason that is realistic rather than aspirational, because each one produced exactly the
fixtures a parser needs.

The nine reports are also the argument against a review-only tier. Reading raw transcripts of an
unfamiliar harness during a live event is the work these reports show is slow, and an organizer
would be doing it on a participant's private data under time pressure. An open verifier lets a
participant self-check during the week, which removes most appeals before they happen.

---

## 6. Limits, and what we did not test

We would rather publish this list than have someone find it.

| Not tested | Where it stands |
|---|---|
| Spill paths | Claude Code's documented tool-result spill directory was never created in either Claude spike, and Hermes' hook-output spill captured no files, because no output was large enough. Both remain source-reading claims |
| Compaction, undo, rewind | never triggered on any harness. The "row mutated after insert" detector runs and has never seen a positive. Compaction is the likeliest thing to break a naive parser over 168 hours |
| The permission and approval leg | zero events on four harnesses: Claude Code came up in automatic mode on its own, OpenCode's merged config asked for nothing, no Hermes tool tripped the dangerous-command classifier, and Cursor ran with approvals forced |
| Mid-stream kill on Claude Code | the kill landed while a request was in flight, not while a response was streaming. The in-flight case is proven clean; the streaming case is untested |
| Cursor hooks in print mode | measured absent rather than untested, which is worse. The exit-code block route was not exercised and the MCP hooks were untested with no server configured |
| dsh | everything. No session was ever run. See 3.10 |
| Remote and cloud modes | not tested and not eligible. They leave no local trace, so they are banned rather than unsupported: Claude Code on the web, Codex cloud, Cursor cloud and background agents, OpenCode's CI integrations, remote Hermes and OpenClaw servers, dsh remote sandboxes |
| Terminal UI behaviour | four spikes were orchestrator-driven and cannot say what a participant sees on screen when a message is refused, or whether a model switch typed into a terminal UI persists differently from one made over an API |
| Long-run behaviour | every spike ran for minutes. Codex rollouts have been observed in the hundreds of megabytes elsewhere, the Codex app stores every tool output four times, and OpenClaw prunes sessions on a schedule we have not watched run. Sustained tailing, disk growth and pruning over days are untested |

### 6.1 The version-pin caveat

Every on-disk format here is unstable, and several of these harnesses ship daily. This is not a
hedge, it is the single most likely way this document goes stale.

The evidence: the Codex CLI record type our research file named as the human-input marker does
not exist on 0.153, one minor after the version the file was written against. The Hermes schema
is five versions behind what the docs describe, and its hook list is shorter. The Codex desktop
app updated itself from one build to another during an eleven-minute run, with the old version
frozen into the session header at thread creation, so the file claims a binary that did not
serve most of its records. Cursor's transcript format is both lossier and richer than documented,
in different places.

What was actually tested, on 2026-09-03:

| Harness | Version tested |
|---|---|
| Claude Desktop | app 1.44121.4, bundled Claude Code 2.1.258 |
| Claude Code CLI | 2.1.259 (pin at or above this; the resume-after-kill leg depends on a fix in it) |
| Codex CLI | 0.153.0 |
| Codex desktop app | ChatGPT app 26.825.31414, embedded codex-cli 0.150.0-alpha.12.2, self-updated mid-run |
| OpenCode | 1.18.27 |
| Hermes | 0.19.0 (schema v22, 23 hooks) |
| OpenClaw | 2026.8.2 |
| Cursor CLI | 2026.09.02-c22c1a3 |
| dsh | 0.1.1-rc.2 installed, not run |

Each connector manifest carries a tested version range and preflight refuses to start outside
it. The conformance suite re-runs against pinned versions before the event opens. If you are
reading this more than a few weeks after the date at the top, assume at least one row has moved
and re-run the spike before trusting it.

---

## 7. Reproducing this

This public cut is a measured synthesis, not a complete rerun bundle. It retains the included
redaction utility, the redacted fixture trees and one fixture-only generator listed below.
Historical per-spike reports, dsh preparation notes, private per-harness rerun scripts/configs
and raw outputs remain omitted; the generator's `--self-check` validates committed fixture/scrub
wiring without launching a harness, while full regeneration requires an explicitly supplied
historical artifact root plus local Codex session/config/workspace inputs. Those private inputs
are not shipped, the generator does not recreate the private harness run, and none of those
omitted artifacts is needed to read the findings here or claimed as present.

### 7.1 Where things live

| Path | What |
|---|---|
| `spikes/redact.py` | included conservative pattern redaction utility used when preparing fixtures |
| `spikes/codex-app/make_fixtures.py` | shipped fixture-only generator; `--self-check` deterministically validates the committed A9 fixture/sidecar and scrub wiring without launching a harness. Full regeneration requires an explicitly supplied historical artifact root plus local Codex session/config/workspace inputs and writes fixture outputs; those private inputs are not shipped and it does not rerun the private harness |
| `spikes/{claude-desktop,codex-cli,codex-app,claude-code-cli,opencode,hermes,openclaw,cursor-cli}/fixtures/README.md` | included redacted fixture inventories with per-file hashes and scrub notes |
| `docs/harness-research/` | pre-spike source notes; Section 8 above remains the measurement errata |

Raw output stays outside the public cut, local, not in git, in `spikes/<harness>/out/` directories
and is not evidence linked by this document.

### 7.2 Running the same spike for a new harness

The sequence below preserves the historical method; it is not a self-contained public rerun
command. The receiver, tailer, per-harness protocol/config files, private launch scripts and
raw-output directories used by that method are omitted from this cut. The shipped
`spikes/codex-app/make_fixtures.py` is a narrow fixture-only exception: `--self-check`
deterministically validates the committed A9 fixture/sidecar and scrub wiring without launching
a harness, while full regeneration requires an explicitly supplied historical artifact root plus
local Codex session/config/workspace inputs and writes fixture outputs. Those private inputs are
not shipped and the generator does not recreate the private harness run. A future private rerun
must supply equivalent tooling and should treat this document and the included fixtures as the
public record, not as a claim that the omitted harness packet is shipped.

1. Install the harness under `harnesses/` with its own home directory. Never point a spike at a
   real user configuration directory unless the harness has no isolated mode; if it has none,
   say so in the report and bracket the run with before-and-after hashes.
2. Read the harness's documentation for its transcript store and its hook or plugin surface, and
   write down what you expect before running anything. Half the value of these reports is the
   difference between the expectation and the measurement.
3. Create an empty workspace outside any git worktree, with the artifact root neither an
   ancestor of it nor adjacent to the home directory. See 4.4.
4. Write the folder-scoped configuration the manifest would write, pointing the hooks at the
   stub receiver, and record its hash.
5. Start the receiver and the tailer before the harness, and confirm the receiver is live with a
   ping that leaves a line, so its liveness is provable from the artifacts alone. One spike
   could not prove it and had to argue around the gap.
6. Launch or arm with the pins, run the nine-step script, and open a parallel session in another
   folder during the run for A9.
7. Collect everything into `out/`: leg 1 copies, the hook stream, spill directories, the
   harness's own store files copied read-only at the end, exit codes, timestamps.
8. Check the eleven assertions mechanically where you can. Keep the checker's output even where
   you disagree with its verdict, and explain the disagreement in the report.
9. Redact, scrub structurally, review by eye, commit fixtures, then write the report.

### 7.3 What goes in a report

The historical template was in `spikes/README.md` (omitted from this public cut). The reports
that turned out most useful had four
properties beyond it:

- **Version first, and the exact one.** Not the version you installed, the version the store's
  own records name.
- **Deviations stated up front.** Every spike deviated from the script somewhere. The ones that
  said so in the first hundred lines are the ones we could still reason about afterwards. Two
  reports record mistakes made during the run that produced better findings than the intended
  step.
- **Failures kept as artifacts.** One spike left a tailer bug unfixed so the artifact would show
  the loss. One committed a zero-byte hook file as the evidence. A passing report with the
  interesting failure smoothed away is worth less than a failing one.
- **Surprises separated from assertions.** The assertion list is what we knew to ask. The
  surprises section is where most of section 4 came from.

---

## 8. Corrections to our own research

The pre-spike research files in `docs/harness-research/` were written from documentation and
source reading. The spikes overturned parts of them. The support matrix in
`docs/harness-research/README.md` now carries per-cell verification notes, but the per-harness
files still need rewriting, and until they are, these are the claims not to trust.

| Where | What the research file said | What the spike measured |
|---|---|---|
| `codex-and-cursor.md` | a human input is an `event_msg/user_message` line | that record type does not exist on 0.153 with paginated history. A typed prompt is a user-role message with the same shape harness context uses (spike 2, A2) |
| `codex-and-cursor.md` | subagent threads are linked by an inter-agent metadata line | that line exists and its entire payload is a boolean. The parent link is in the subagent's session header (spike 2, A4) |
| `codex-and-cursor.md` | hooks are supported across the desktop app, CLI and IDE extension | true of dispatch, false of trust. The desktop app runs folder-scoped hooks correctly and ships no way to trust one, so out of the box it runs blind (spike 3, Headline and P3) |
| `codex-and-cursor.md` | hooks are read at session start, so the risk is a restart | the trust store is what actually gates them, and it is per hook definition hash (spike 2, Unknowns 2) |
| `codex-and-cursor.md` (Cursor) | the hook leg is the only source of tool results | the chat store carries full tool results and the whole system prompt. The JSONL is the lossy one and must not be tailed (spike 9, Unknown 2) |
| `claude-code.md` | the session-start hook payload has no model field | it does carry one, as a variant id that never equals the per-message model value, and the suffix is not stable within a session (spike 4, A6) |
| `claude-code.md` | request and response bodies are linkable only indirectly | a billing header supplies the prompt id, the previous response id, a subagent flag and a finer build version (spike 4, cross-check c) |
| `claude-code.md` §4 | the desktop app section says nothing about the bundled binary, the missing session-start event, the automatic permission mode or the auxiliary agents | all four are measured in spike 1 and the section should be rewritten from it |
| `hermes.md` | schema version 27, with a separate system-prompts table and a system-prompt hash column | version 22 on 0.19.0, with neither. The per-session snapshot is the only byte-exact copy (spike 7, Unknowns 2) |
| `hermes.md` | the documented hook set is 26 or more names, possibly around 75 | 23 names in the valid-hooks list on 0.19.0. Several the design assumed do not exist, including the one that would have made slash commands visible on leg 2 (spike 7) |
| `hermes.md` | the pre-request hook's message array includes the system prompt | not on the API mode we tested. The sanitised sibling field is truncated to the sanitiser's floor, so leg 2 cannot recover the system prompt at all there (spike 7, Unknowns 2) |
| `openclaw.md` | plain CLI or terminal-UI input may be stamped with an external-user provenance kind | it is left unstamped entirely. Only harness-authored text is stamped, so the rule inverts: provenance absent plus three negative clauses plus hook corroboration (spike 8, Unknowns 2) |
| `openclaw.md` | the enabled flag needs a restart, per the schema help text | it hot-applies in 0.7 seconds with the same process id, and takes the input gate down with it (spike 8, Unknowns 4) |
| `deepseek-harness.md` §5 | a model-selection event carries selection intent | there is no such event in 0.1.1-rc.2. Model identity comes from the assistant message source and the request context and header records (dsh notes) |
| `deepseek-harness.md` §5 | four session lifecycle events are durable log types | they are framework events, not durable log types (dsh notes) |
| `README.md` matrix, OpenCode row | the harness-authored marker is sufficient to identify human input | literally true and practically insufficient. The subagent's own prompt is an unmarked user message with its own hook post (spike 5, A2) |
| `README.md` matrix, Cursor row | predates spike 9 | transcript lossiness, store completeness, the print-mode hook table and the isolation failure are all newer than the row |
| `README.md` matrix, dsh row | names 0.1.2-alpha | the installed harness is 0.1.1-rc.2 |

Two of these were caught only because a spike ran a newer version than the research file, which
is the argument for re-running the whole set shortly before the event opens.
