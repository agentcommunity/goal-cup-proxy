# The /goal Cup tracker — design decisions (2026-09-03)

> **Frozen: no new entries unless a rule really changes.**
>
> Citations below of STATUS, LEDGER,
> qualification records, plans and research files name documents deleted from the tree; they are in
> git history at `6494030`.

## Index

Holds on `m2/wire`? is read from each entry's own m2 note. "no m2 note" means the entry says nothing about
m2: it does not prove the entry survives where it names the hook leg.

| ID | Title | Holds on m2? | Pointer |
|---|---|---|---|
| D1 | No API proxy; the transcript is the evidence | amended: forward proxy built; base-URL proxy still forbidden | [entry](#d1-no-api-proxy-the-harnesss-own-transcript-is-the-evidence) · wire-primary §2.3, §4 |
| D2 | Two legs per harness, reconciled | amended: native + wire; hook leg removed | [entry](#d2-two-legs-per-harness-reconciled) · wire-primary §4; capture-first |
| D3 | Record raw, interpret later | no m2 note; `doctor --verify` removed by D48 | [entry](#d3-record-raw-interpret-later) |
| D4 | One daemon per machine | amended 2026-10-12: one at a time, run-scoped, no service (hook endpoint moot, D24) | [entry](#d4-one-daemon-per-machine) |
| D5 | Only tracker-created folders are seen | no m2 note (hook-config part moot) | [entry](#d5-only-sessions-in-tracker-created-folders-are-ever-seen) |
| D6 | Workspace config watched, not locked | no m2 note (hook-config part moot) | [entry](#d6-tracker-config-in-the-workspace-is-watched-not-locked) |
| D7 | Start flow: first typed message is the goal | amended | [entry](#d7-start-flow-fresh-session-first-typed-message-is-the-goal) · wire-primary §11.6; D37 |
| D8 | `keep going` is the only human message | superseded | [entry](#d8-keep-going-is-the-only-human-message-from-any-channel) · wire-primary §2.1-2.2; D37 |
| D9 | Gap policy | no m2 note | [entry](#d9-gap-policy) |
| D10 | One harness per entry | amended: detectable, kept | [entry](#d10-one-harness-per-entry) · wire-primary §11.6; D37 |
| D11 | Seal and workspace attribution | no m2 note; D29 orders V1, D40 finish | [entry](#d11-seal-and-workspace-attribution) · TERMINATION |
| D12 | Versions | amended 2026-09-24: newer is a Warn | [entry](#d12-versions) |
| D13 | Connector bar and openness | amended 2026-09-29 (open source wording) | [entry](#d13-connector-bar-and-openness) |
| D14 | Normalized format, redaction, encryption | amended by D36 | [entry](#d14-normalized-format-redaction-encryption) · D36 |
| D15 | Side channels in external services | no m2 note | [entry](#d15-side-channels-in-external-services) |
| D16 | Arming gate | superseded | [entry](#d16-m0-arming-gate-no-started_at-without-the-declared-start-signal) · wire-primary §2.4, §4 |
| D17 | Five health signals | partly superseded: signals 1, 3, 4, 5 remain | [entry](#d17-m0-five-health-signals-and-a-per-connector-hook-silence-policy) · wire-primary §4 |
| D18 | Human-message rule, two-legged | superseded | [entry](#d18-m0-the-human-message-rule-is-two-legged-with-an-exclusion-list) · wire-primary §2.1-2.2; D37 |
| D19 | Each store declares its cursor type | no m2 note | [entry](#d19-m0-every-store-declares-its-own-cursor-type) |
| D20 | Leg-1 stores partitioned at capture | no m2 note | [entry](#d20-m0-leg-1-stores-are-partitioned-at-capture-time) |
| D21 | Four named app-wide exceptions | no m2 note; D38 isolates Claude's HOME | [entry](#d21-m0-four-named-app-wide-exceptions-disclosed-on-the-public-entry) · D38 |
| D22 | Kill the process tree, not the launcher | no m2 note | [entry](#d22-m0-kill-the-process-tree-not-the-launcher) · TERMINATION |
| D23 | Measured per-harness tiers | no m2 note; m2 ships three harnesses | [entry](#d23-m0-measured-per-harness-tiers-d29-controls-v1-shipping-scope) · RULES |
| D24 | Local hook receiver is authenticated | superseded: no hook receiver | [entry](#d24-m0-the-local-hook-receiver-is-authenticated) · wire-primary §4 |
| D25 | Feedback is a status-API consumer | no m2 note | [entry](#d25-participant-feedback-is-a-consumer-of-the-status-api-never-a-daemon-feature-and-never-agent-context) |
| D26 | Token usage derived by the verifier | no m2 note | [entry](#d26-token-usage-is-derived-by-the-verifier-and-published-it-is-not-a-rule) |
| D27 | `[home_seed]` reads the participant's config | removed by D48 (V1 code; `v0.5.0` keeps it) | [entry](#d27-dogfood-home_seed-the-per-run-home-is-seeded-by-reading-the-participants-own-configuration-and-what-it-carried-is-recorded) |
| D28 | Wire and PTY capture (M2 reservation) | amended by D36: wire built | [entry](#d28-m2-reservation-wire-and-pty-capture-are-a-later-launch-mode-evidence-boundary) · D36, D1 note |
| D29 | V1 is the foundation target; V1.1 expands | V1 scope; holds on `v1/trunk` | [entry](#d29-v1-is-the-defined-foundation-qualification-target-v11-is-product-and-event-expansion) · D33 |
| D30 | `Confirmed` is bounded to the owned set | in force | [entry](#d30-confirmed-termination-is-bounded-to-the-owned-process-set-and-the-limitation-is-stated) · TERMINATION |
| D31 | Crash recovery resumes the intent | removed by D48 (V1 code; `v0.5.0` keeps it) | [entry](#d31-crash-recovery-resumes-the-interrupted-intent-it-never-quarantines-an-in-progress-run) |
| D32 | Receiver evidence loss is a verifier finding | V1 path (receiver gone on m2, D24); V1 daemon deleted by D48, verifier rule kept | [entry](#d32-receiver-evidence-loss-is-a-verifier-finding-not-a-daemon-side-refusal) |
| D33 | No ES observer, no privileged daemon, no protocol V5 | in force; note added for D37 | [entry](#d33-no-endpoint-security-observer-no-privileged-daemon-no-protocol-v5--in-v1-or-v11) · D37 |
| D34 | `3633a14` row-hardening deferred | no m2 note | [entry](#d34-3633a14-row-hardening-is-deferred-past-v1-cherry-pickable-afterward) |
| D35 | Fail is a broken rule; inference is a Flag | no m2 note | [entry](#d35-dogfood-a-fail-is-a-rule-broken-an-inference-from-incomplete-information-is-a-flag-unexplained_seal_file-is-a-flag) |
| D36 | Two-tier redaction on the wire leg | in force (m2); amended by D37 | [entry](#d36-m2-two-tier-redaction-on-the-wire-leg-and-a-harness-agnostic-recorder) · capture-first |
| D37 | Autonomous runs, research evidence, contributions | in force (m2) | [entry](#d37-m2-autonomous-runs-research-evidence-and-open-contributions) · capture-first |
| D38 | Fresh per-run HOME for Claude Code | in force (m2); cheapest-model clause superseded by D52 | [entry](#d38-m2-fresh-per-run-home-for-claude-code-cheapest-models-for-live-tests) · qualification/m2/launch-recipes |
| D39 | Headless launches, ported patterns, Codex home | in force (m2) | [entry](#d39-m2-headless-capture-launches-ported-extra-redaction-patterns-shared-isolated-codex-home) |
| D40 | Finish grace, spawn gate, release waits, DeepSeek | in force (m2) | [entry](#d40-m2-finish-keeps-the-termination-grace-the-spawn-window-is-closed-reservation-release-waits-hermes-uses-deepseek) · LEDGER H-10, H-11 |
| D41 | Hermes reference row rules (CA rule amended by D43) | in force (m2); model and thinking clause superseded by D52 | [entry](#d41-m2-hermes-reference-row-model-ca-trust-key-fingerprint-and-key-fragment-rules) · qualification/m2-hermes-reference-row |
| D42 | Open-source v0.5: V1 behind `v1-compat`, data-only harnesses, open harness PRs | in force (m2); DCO clause amended by D47; V1-path clause replaced by D48 | [entry](#d42-m2-open-source-v05-v1-path-behind-a-feature-data-only-harnesses-open-harness-prs) |
| D43 | Harness contract v1 rulings; D41 CA rule generalised | in force (m2); ruling 3 amended by D49 and D50; ruling 5 by D51; ruling 7 by D53 | [entry](#d43-m2-harness-contract-v1-owner-rulings-and-d41s-ca-rule-generalised) |
| D44 | OpenCode native = read-only own-session SQLite snapshot; public repo goal-cup-tracer | in force (m2); DCO clause amended by D47 | [entry](#d44-m2-opencodes-native-leg-is-a-read-only-snapshot-of-its-own-sessions-rows-the-public-repo-is-goal-cup-tracer) |
| D45 | Integrity exit judges the record, not the run: interrupted, undecoded and unconfirmed-stop runs stay exit 0 | in force (m2) | [entry](#d45-m2-the-integrity-exit-judges-the-record-not-the-run) |
| D46 | Capture creation binds the CLI and daemon to identical connector bytes | in force (m2) | [entry](#d46-m2-capture-creation-binds-both-local-halves-to-identical-connector-bytes) |
| D47 | No DCO sign-off; a human answers for each PR | in force (m2) | [entry](#d47-m2-no-dco-sign-off) |
| D48 | The V1 path and its `v1-compat` feature leave the tree; history keeps them | in force (m2) | [entry](#d48-m2-the-v1-path-leaves-the-tree) · LEDGER OS-30 |
| D49 | Admit `AMP_API_KEY` as a child credential | in force (m2) | [entry](#d49-m2-admit-amp_api_key-as-a-child-credential) |
| D50 | D43 ruling 3: credential + `stdout_json` via one stdout reader | in force (m2) | [entry](#d50-m2-amend-d43-ruling-3-for-stdout_json-identity) |
| D51 | A connector's argv and env are data; review is the control; credential, CA and redaction lists stay closed | in force (m2) once merged | [entry](#d51-m2-a-connectors-argv-and-env-are-data-review-is-the-control) |
| D52 | Shipped connectors set no model, effort, sandbox, permission or MCP limit | in force (m2) once merged | [entry](#d52-m2-the-shipped-connectors-set-no-model-effort-sandbox-permission-or-mcp-limit) |
| D53 | Record the harness build in v5 (`Launch.harness_build`); v0.5.0 readers refuse such traces | in force (m2) once merged | [entry](#d53-m2-record-the-harness-build-in-v5-accepting-that-v050-readers-refuse-it) |

The "Later list" at the end is V1/V1.1 scope; it is not re-decided on m2.


## V1 foundation revision (2026-09-13)

Public V1 scope was recorded in the retired `STATUS.md` (git history at `6494030`).

V1 defines one complete short-duration Codex CLI launch-mode qualification lifecycle on one exact macOS target. Once implemented, it includes interruption, recovery, manual and automatic short-deadline finish, owned-process termination, immutable workspace artifact capture, and offline verification. It does not require or claim a real 168-hour run. D29 records the complete milestone reassignment; all existing security, privacy, isolation, record-raw, and no-agent-steering decisions remain constraints.

Revised 2026-09-03 after milestone M0; see `docs/M0-FINDINGS.md` and `docs/CAPTURE-RESEARCH.md`.

Status: decision record from the 2026-09-02/03 design review, after the harness research pass (`docs/harness-research/`). These decisions reopen parts of the v0.1 plan (RESEARCH repo); v0.2 of the plan implements them. `docs/RULES.md` carries the participant-facing consequences.

Format: decision, why, what it costs, what it changes elsewhere. Passages marked **(M0)** were changed or added by the M0 revision; D16 to D24 are new in that revision. D27 is marked **(dogfood)** and was added on 2026-09-08 out of the first real-harness runs (`docs/dogfood/2026-09-07/`).

---

## D1. No API proxy. The harness's own transcript is the evidence.

> **Amended on `m2/wire`, 2026-09-24. On `v1/trunk` and the `v0.1.0` tag D1 stands as written.**
> "The tracker does not intercept model traffic" no longer holds on `m2/wire`. A **process-scoped
> forward proxy** is permitted there: a tracker-run proxy on loopback, reached only through the
> harness process's own `HTTPS_PROXY`, trusted only through a CA named in that process's
> environment, terminating TLS only for the provider hosts the connector declares and tunnelling
> every other host. Credentials are stripped under D28's rule before anything is hashed, stored or
> uploaded. **A base-URL proxy is still forbidden**: no harness's model base URL is set or changed,
> in any milestone. The "Why" below is true of a base-URL proxy and was measured false for a
> forward proxy. Subscription auth survived behind one on all five harnesses in
> `spikes/wire-pty/REPORT.md` §4(a). A Rust probe then recorded one real Claude Code session and
> one real Codex WebSocket session on `gpt-5.6-luna` end to end, and no live credential value
> appeared in either recording (`docs/superpowers/reviews/2026-09-24-proxy-probe.md`).
> Historical note: the earlier wire-primary specification is intentionally omitted from this public
> OSS cut. Current M2 proxy authority is the [public architecture](ARCHITECTURE.md#7-the-proxy),
> with first-release framing in [D37](#d37-m2-autonomous-runs-research-evidence-and-open-contributions).

> **Status note, 2026-10-02 (not a decision change).** The proxy is now built on `m2/wire`:
> `goalcup start --goal-file` runs the process-scoped forward proxy in-process
> (`crates/goalcup-cli/src/proxy/`, LEDGER T09-T16), and the last sentence of the amendment above
> is out of date. It is built and measured on short rows, not qualified for long runs
> (LEDGER T25-T28 open). No harness base URL is set by it.

**Decision.** The tracker does not intercept model traffic. Every supported harness writes a complete, model-visible transcript to local disk (Claude Code JSONL plus raw API bodies via `OTEL_LOG_RAW_API_BODIES=file:`, Codex rollout JSONL, dsh session JSONL, OpenCode/Hermes/OpenClaw SQLite). The tracker tails that store line by line, hash-chains it, and ships it.

**Why.** Participants use subscription auth (Claude Max, ChatGPT), which a base-URL proxy would break or violate. Research verified the on-disk records are complete enough for every harness except Cursor, whose transcript omits tool outputs and is captured through hooks instead.

**(M0)** Correction on Cursor: the real leg 1 is `store.db`, a content-addressed blob tree carrying full tool results and the whole system prompt; `agent-transcripts/*.jsonl` is the lossy display artefact and must not be tailed (spike 9, Unknown 2). What Cursor lacks is a human-text witness on leg 1, not tool output.

**Costs.** On-disk formats are unstable everywhere; parsers must be pinned and re-tested before T0 (see D12). Extended thinking is not fully captured on Claude Code (redacted in raw bodies, often empty in JSONL).

**(M0)** Second measured cost: "every token the agent saw" holds, "every byte a tool produced" does not. Codex truncates in band with no spill file, OpenCode's spill file is itself capped at 192 KiB, and OpenClaw persisted 64,000 of 200,001 characters with the full bytes only in a hook payload (M0-FINDINGS §2.10). `RULES.md` states this plainly.

## D2. Two legs per harness, reconciled.

> **Status note, 2026-10-02 (not a decision change; wording approved by the maintainer).** On
> `m2/wire` the two legs are the native leg (the harness's own session store, the successor of leg 1)
> and the wire leg (recorded model traffic, which replaces leg 2). The hook leg is removed
> (see the [public M2 architecture](ARCHITECTURE.md#4-the-two-legs)).
> The text below describes the V1 design on `v1/trunk` and `v0.1.0`.

**Decision.** Leg 1: the persisted transcript. Leg 2: the harness's hook or plugin stream, posted to the daemon. The verifier fails an entry's automatic verification if the legs disagree.

**Why.** Hooks are user-disableable in every harness (three hot-reload). A transcript file is editable. Forging both consistently, before the next server acknowledgement, is much harder than either alone. Hook silence while the transcript grows is itself a flag.

**(M0) Both legs are mandatory, not merely corroborating, on three harnesses.** Part of the record exists in exactly one leg: the text of a blocked prompt only in leg 2 on the Codex app (leg 1 shows a four-record empty turn with `last_agent_message: null`), the untruncated tool output only in leg 2 on OpenClaw, the only human-text witness only in leg 2 on Cursor. A connector may not declare a leg optional where the manifest says reconstruction needs it.

**(M0) Joining the legs is not always possible by tool id.** On Codex `exec-<uuid>` never equals `call_<id>`, and one rollout tool call can be several hook pairs, so the verifier joins by `turn_id` plus ordering. Each connector declares its join key.

**Consequence.** **(M0)** Cursor has both legs (`store.db` plus hooks), but only leg 2 witnesses human text, and only on a TTY; it keeps the elevated-review label and `-p` is banned for tracked sessions (D23, `RULES.md`).

## D3. Record raw, interpret later.

**Decision.** The participant-side daemon does no rule interpretation. It captures, hashes, buffers, uploads, snapshots the workspace at seal, and reports health. All parsing, normalization, rule checks, and reconciliation run server-side in the open-source verifier, which also runs offline as `goalcup-verify <trace-dir>`.

**(Corrected 2026-09-08.)** This line said `goalcup verify`, a sub-command of the participant CLI. There is no such sub-command and there is not meant to be one: verification ships as its own binary so the daemon never links the verifier, and so there is one set of flags and one set of exit codes for it. `goalcup` has eight sub-commands and its own `--help` says where verification went; `goalcup doctor --verify <dir>` relays the verdict's exit code for a participant who wants one command.
(Removed by [D48](#d48-m2-the-v1-path-leaves-the-tree), 2026-10-11: `doctor --verify` was V1-only
and is deleted, so `goalcup-verify` is the one verdict command; the `v0.5.0` tag keeps it.)

**Why.** Parsers will have bugs and formats will drift. If raw bytes are preserved, a parser fix in judging week is a re-run, not a lost entry. It also keeps the signed, hard-to-change binary small.

**Exception.** The daemon reports two capture-health signals a participant can act on: the expected transcript stopped growing while the harness process is alive, and hook posts went silent.

**(M0)** The exception list grows from two signals to five, enumerated in D17. Every addition is a comparison against a constant the manifest already carries (an expected hash, a declared start signal, a version string), not a rule check, so the principle holds: the daemon still parses nothing it has to interpret.

## D4. One daemon per machine.

**Decision.** A supervised background service owns all tailers, one local hook endpoint, the upload queue, and the signed status cache. `goalcup` CLI commands talk to it. Entries are records inside it, bound to workspace folders. Parallel entries are routed by working directory.

**Why.** Parallel entries on the same harness are allowed; per-entry processes would collide on hook endpoints and duplicate tailers. Crash recovery becomes a service restart.

**(2026-10-12) One daemon at a time, run-scoped; no service.** The daemon is no longer a supervised service (the launchd agent and its installer half are gone, closing H-19). `goalcup start` and `goalcup seal` start `goalcup-daemon --exit-when-idle` when none answers on `control.json`, detached and with an environment built from nothing, and it exits once no capture is unfinished. One per machine still holds, at a time: the registry's redb lock decides a start race. Crash recovery is `goalcup seal` (or the launcher's own finish, H-14) starting a daemon that reopens the entry for status and seal only, which is all a restarted daemon ever offered (a restart admits no producer). No contract, record or control route changed.

## D5. Only sessions in tracker-created folders are ever seen.

**Decision.** The daemon scopes every capture leg by the workspace path it created. Hook config is written folder-scoped into the workspace (Claude Code `.claude/settings.json`, Codex `.codex/hooks.json`, Cursor `.cursor/hooks.json`, OpenCode `.opencode/`, OpenClaw/Hermes project hook dirs). App-wide user settings are never modified. CLI launch mode adds process-level pins (env, flags, dedicated home or profile) on top.

**Why.** The privacy promise in RULES.md, and the participant's explicit requirement that ordinary Claude/Codex sessions are never tracked.

**(M0) Folder scoping is the rule, with four named app-wide exceptions.** Three harnesses need one app-wide write each and one takes a fourth involuntarily. The list, the disclosure text and the reversal procedure are D21. Leg 2 kept the scoping promise on every harness measured; leg 1 did not, which is D20.

**(M0) Artifact placement.** The tracker's artifact root must not be an ancestor of the workspace, nor sit anywhere a mis-launched session can enumerate it: the desktop agent ran `ls -la ..` on its first tool call and later read the tracker settings, receiver log, hook stream and raw-body path (spike 1, Surprise 3), and a Codex session mis-launched at the artifact root listed a baseline credential file (spike 2, Surprise 6). Config baselines hash credential files and never copy them.

**(M0) An empty workspace is not an empty context.** The Codex app injects the participant's memory summary and global skill listing into every thread, and OpenCode and Cursor read host skills into the system prompt; in spike 3 P3 an injected skill stopped the agent writing the requested file. v1 allows this with disclosure on the public entry rather than requiring it off, because not every harness offers a switch.

**Consequence.** The "completely empty workspace" rule gains one declared exception: the tracker's own config files, excluded from the sealed manifest and disclosed on the public entry. On Claude Code the raw-body switch can only be set at process level, so desktop attach entries run on JSONL plus hooks and carry the elevated-review label.

## D6. Tracker config in the workspace is watched, not locked.

**Decision.** No read-only or immutable flags. The daemon knows the bytes it wrote; it watches the files, reports any deviation to the server with timestamp and diff, and restores them. The server holds the expected hashes from the connector manifest.

**Attribution.** An edit matched to a traced agent tool call is a flag reviewed like any anomaly. An edit with no traced cause is a human workspace change: disqualifying after review.

**(M0) Restoring a watched file is not always free.** Codex hook trust is a hash of the hook definition, so rewriting `hooks.json` mid-event invalidates the trust and needs a re-arm; the daemon must restore and then report `arming_lost` rather than assume capture continues (spike 3 P3, constraint 3). On Cursor a hot-reload deletion of one hook event is detectable only by the pinned-hash flag, which fired on the very next hook post (spike 9, hot-reload probe).

**(M0) The workspace does not stay as the tracker wrote it.** Loading a TypeScript plugin makes OpenCode install 3,645 files and 61 MB into `.opencode/node_modules`, the Hermes shim leaves a `__pycache__` bytecode file, OpenClaw writes `AGENTS.md`, `BOOTSTRAP.md`, `IDENTITY.md`, `SOUL.md` and `USER.md` on the first turn, and in one run the agent ran `git init` in the workspace. Those paths are declared per connector and excluded from the seal (D11).

## D7. Start flow: fresh session, first typed message is the goal.

> **Later M2 revision:** D37 governs the unattended-run objective. The capture-first design's
> simplified first release retains the server-owned receipt after spawn, process handoff and
> scoped native-session binding. Record launch observations separately. Automatic goal delivery
> replaces manual initial submission; the old model-path timestamp gate below is superseded.

> **Amended on `m2/wire` (maintainer, 2026-09-25); in force as written on `v1/trunk` and `v0.1.0`.**
> The goal is given to `goalcup start --goal-file`, chained as the goal event, and passed to the
> harness as its initial-prompt argument. `started_at` is the first recorded request on the
> connector's declared model path, the moment the goal left the machine; arming needs that
> request and a transcript record inside the arming window, or the entry is refused. An
> over-length goal is recorded and flagged, never refused. Wire-primary spec §11.6.

**Decision.** `goalcup start --harness <name>` creates the empty workspace and either launches the harness fresh with pinned environment (CLI mode) or arms the folder and tells the participant to open a new conversation in it (desktop attach mode). The first prompt submitted in that session is the goal. The server-signed `started_at` is requested when the daemon observes that first prompt (hook or first transcript line), so the clock starts when the participant types, within the 24-hour window.

**(M0) Hard arming gate.** `started_at` is not stamped until the harness's declared start signal has arrived for the tracked workspace. Hooks are read at session or thread start on every harness measured, so a session opened before arming runs blind and silent for its whole life with nothing on screen to say so: on the Codex app, trust granted mid-thread produced zero posts for that thread and full capture for the next one. The signal per harness and the refusal path are D16.

**(M0) Codex desktop arming step.** Before the goal thread: run `codex -C <workspace>` once, type `/hooks`, trust the twelve entries, quit, then open a **new** thread in the app. A thread opened before trust cannot be started. Cost: twelve `trusted_hash` tables written into `~/.codex/config.toml` by Codex's own trust UI, disclosed under D21.

**Rejected.** Arming recording from a skill inside an already-running session: four of seven harnesses load hooks only at start, Claude Code's raw-body switch is launch-time only, and pre-start chat would be unlimited prompting. The skill survives as installer and guide.

**Binding rule for desktop.** A new session whose working directory equals the workspace, with start source `startup`. A second session in the same folder is a second root agent: flagged. Sessions elsewhere are ignored.

**Length check.** A blocking prompt-submit hook enforces 4,000 characters where the harness supports it; elsewhere an over-length goal is recorded and shown in status as a violation.

**(M0) Corrected per-harness list**, carried in the manifest as `blocking.capability`. Blocking verified: Claude Code CLI and Desktop, Codex CLI, Codex app (only after arming), OpenClaw (68 ms, rc=1, a real gate), Cursor (TTY only, no gate in `-p`). Best effort: OpenCode, which aborts only by throwing an undocumented defect and shows an opaque 500. Impossible: Hermes, which has no user-message rejection surface, so an over-length goal goes through in full and is recorded as a violation. A blocked prompt still persists somewhere on most harnesses and is recoverable evidence; on the Codex app it exists only in leg 2. Blocking stops model exposure, not persistence.

## D8. `keep going` is the only human message, from any channel.

> **Latest M2 product decision: D37 supersedes the count-first banner below.** Human assistance
> ends the original unassisted interval, even when it contains no model-facing message. Capture
> may continue as assisted evidence. Predeclared harness-native continuation is allowed; the
> tracker never supplies it. V1 history below is unchanged.

> **Superseded on `m2/wire` (maintainer, 2026-09-25); in force on `v1/trunk` and `v0.1.0`.**
> Human messages after the goal are counted and published, not banned: every human message
> that reaches the model is one intervention, `keep going` after a crash included and labelled;
> the goal is intervention zero. "The tracker never speaks" stays true and is strengthened: the
> tracker never relays or injects input either. Wire-primary spec §2.1-§2.2 and §11.6.

**Decision.** Content is what is checked, not the channel. Remote Control, chat channels, phone clients, gateway platforms are all allowed as long as every human message after the goal equals `keep going`. Cloud and remote execution modes remain banned because they produce no local transcript.

**(M0) The rule applies to human messages, and deciding which messages those are is D18.** Every harness measured writes its own text into the user slot, so "every prompt-hook post after T0 must equal `keep going`" fails open and its inverse fails closed. The per-connector exclusion list in D18 is a normative annex to this decision.

**(M0) Absence of `keep going` is not a violation by itself.** After a restart the OpenClaw Gateway re-runs the interrupted turn on its own, with an `internal_system` user message and no human input at all (spike 8, A5). A resumed turn with no human message is normal on that harness.

**The tracker never speaks.** No injected reminders, no auto-continue, no wrap-up nudges in v1. A participant who wants their agent to manage time writes it into the prompt or a prepared skill. The tracker ships `goalcup status --json`, a read-only MCP server, and a small skill; the participant decides whether to register them. (The MCP server and its skill were deleted on 2026-10-12; nothing shipped them.) Scheduled reminders are on the later list.

**Crash.** `goalcup resume` first catches up the tail, then relaunches the same session in resume mode; the participant then types `keep going`.

## D9. Gap policy.

**(M0) Gap classification is the primary instrument, not a fallback.** Six of eight measured harnesses leave no resume marker of any kind: Claude Desktop, the Codex app, OpenCode, Hermes, OpenClaw and Cursor. Only Codex CLI (`SessionStart.source`, gap-free `ordinal`) and Claude Code CLI (a `command` `SessionStart` with `source=resume`, which fires twice for one resume) mark a boundary. "Silence after a crash" is also not safe, because the OpenClaw Gateway restarts itself.

**(M0) New incident type: silent degrade to a new thread.** A `codex resume` that cannot take the thread-writer lock becomes a fresh thread with no parent reference and no error (spike 2, Surprise 1). The manifest declares the lock paths; the daemon treats a new `thread_source: "user"` session in a tracked home as an incident.

**Decision.** A clean gap (nothing appended to any transcript while the daemon was down) is an outage. A dirty gap (transcript grew while unwatched) is flagged; the reviewer reads exactly the lines appended in the gap. Disqualification only if the review finds a human message or an inconsistency. Late evidence is accepted when its chain links to a pre-deadline acknowledgement; otherwise reviewed as a dirty gap. Thresholds are generous: laptops close, networks drop. The daemon's job is to reconnect, catch up from the last acknowledgement, and say "back online, N events uploaded."

## D10. One harness per entry.

> **M2 clarification under D37:** the one-harness rule remains. A provider hostname or missing
> decoder alone does not establish a second harness; record unknown identity and uncertainty.
> Capture support and semantic qualification are separate contribution capabilities.

> **Made detectable on `m2/wire` (maintainer, 2026-09-25).** The tracker never stops a second
> harness the tracked one launches; it records its traffic (union host list, every known CA
> variable in the launch env) and the verifier reports model traffic under a second harness
> identity as a Fail. Same-harness subagents are allowed. Wire-primary spec §11.6.

**Decision.** Subagents inside the same harness are allowed; running another harness as a subagent is not (the event compares harnesses). The daemon still watches all installed harness stores during a run: a second harness writing a session inside the workspace is a flag, and traced shell commands invoking known harness binaries are checked by the verifier.

**(M0) Detectors proven in M0.** A second Claude Code under the same `CLAUDE_CONFIG_DIR` leaves a second slug in `projects/` and is directly detectable (spike 4, A9). Leg 2 on the Codex app caught a standalone CLI session running in the tracked workspace (spike 3 P3, A9). On OpenClaw the detector must key on `ctx.sessionKey`, because `ctx.workspaceDir` reports the Gateway's own workspace for every session and cannot filter anything.

## D11. Seal and workspace attribution.

**(M0) The attribution check was demonstrated, in both directions.** In spike 3 P3 every file present in the workspace was explained by a traced write, and the file that was absent was explained by tool calls that only read. **(M0)** The sealed manifest excludes the paths the harness itself creates, declared per connector: `.opencode/node_modules`, `__pycache__`, the OpenClaw bootstrap files, and any `.git` the agent creates. **(M0)** Termination follows the manifest's kill strategy (D22), not a kill of whatever the launcher was.

**Decision (revised 2026-09-09; superseded for V1 ordering by D29).** D29 now controls V1: one
durable `SealingIntent` records generation, reason, receipt, and source-root journal, closes
admission, and fixes the cutoff instant. The daemon drains already-admitted evidence to its
bounded point, writes the cutoff including that drain, durably binds the returned sequence/hash
to the same intent, and reconciles intent and chain before continuing after a crash. It then
terminates the managed process tree or dedicated attach app where safe, snapshots eligible files,
verifies the immutable artifact, and writes terminal finalization. Publication is strictly
post-finalization V1.1 work: it consumes that immutable artifact and cannot delay, rewrite, or
reopen a terminal artifact. D11's canonical-tree, publisher, bounded normal push, and readback
mechanics are V1.1 only; the daemon never mutates Git. An unresolved attach process is an
explicit elevated-review finding. The verifier runs a seal-time attribution check: every file in
the sealed workspace must be explained by a traced edit, write, or shell command. No continuous
workspace hashing in V1; no hourly manifests (noise while an agent builds).

## D12. Versions.

**Decision.** Each connector manifest carries a tested version range. Preflight refuses to start outside it. Auto-update is not tampered with. Mid-run version changes are recorded; parsers adapt afterwards. Rules state: do not update your harness during the hackathon. The conformance suite re-runs against pinned versions before T0.

**(M0) "Do not update" is unenforceable where the harness updates itself.** The ChatGPT app updated both itself and its embedded binary during an eleven-minute run, with the version frozen in the session header at thread creation, so the file names a binary that did not serve most of its records. The daemon samples the version on a timer, records a change as a run event, and flags rather than disqualifies; the manifest carries `version.self_updating` and `version.sample_interval`.

**(M0) Pins from M0.** Claude Code at or above 2.1.259 (the resume-after-kill leg depends on a fix in it), and `cc_version` from the billing header recorded as the finer per-request witness.

**(Amended 2026-09-24, maintainer.) Version pinning is not realistic.** Codex CLI ships about twice a week, so an exact pin refuses every participant who updated. A harness version newer than the tested range is allowed and recorded as untested: preflight and `goalcup doctor` report it as a Warn, not a Fail, and the verifier may flag it but never fails it. A floor may still refuse known-broken older versions: the range's lower bound is that floor (Codex 0.153.0, the version the V1 journeys and the 2026-09-24 probe ran on; Claude Code 2.1.259, unchanged). The manifest keeps recording what was tested, so a report can name it. This applies on `m2/wire`; `v1/trunk` keeps its qualified pin, and the recorded V1 manifests (`=0.153.0`) parse and verify as before. The "Preflight refuses to start outside it" sentence above now reads "below it".

## D13. Connector bar and openness.

**Decision.** One bar: manifest, recorded fixtures from a real session (goal, `keep going`, subagent, tool call, stop/resume), and a verifier parser passing the public conformance suite. First-party connectors pass the same suite. Verifier and tracker are open source **(Amended 2026-09-29: this said "verifier and backend"; no backend is in this repository)**; keys, production traces, and deployment config are private. Organizers may decline a harness whose capture is fundamentally incomplete.

**(M0) The tier is measured, not asserted, and it is recorded per harness in D23.** A `full` tier carries named preconditions that the manifest must declare and preflight must check: on Codex, managed or bypassed hook trust plus a launch of the native binary; on Claude Code, the `command` `SessionStart` shim plus the two-legged human rule; on OpenCode, `plugin.loaded` arming plus `session.directory` partitioning plus the SSE leg. A connector that cannot meet its preconditions is `elevated_review`, not `full`.

**Why (revised 2026-09-09).** Elevated review is an explicit supported tier when evidence is incomplete but usable, with named findings and organizer review. An open verifier lets participants self-check during the week and makes evidentiary limits visible.

## D14. Normalized format, redaction, encryption.

**Decision.** ATIF v1.8 (Harbor) as the normalized trajectory format, raw files kept and hash-linked from `extra`; optional Hugging Face session-trace export for the research release. Pattern-based secret redaction at capture with a redaction log shipped alongside so reconciliation still holds. TLS in transit, provider encryption at rest with access audit, local buffer encrypted with a per-run key (`age`). No end-to-end participant-held keys.

## D15. Side channels in external services.

**Decision.** Dedicated agent identities (Linear app actor, GitHub App token, Slack bot user) are recommended, not required. With a shared login the verifier cannot attribute writers, so those cases are review judgment based on what the agent read in the transcript.

## D16. **(M0)** Arming gate: no `started_at` without the declared start signal.

> **Superseded on `m2/wire` by the wire-primary spec (§2.4, §4); in force on `v1/trunk` and `v0.1.0`.** Replaced by: an entry arms on launch plus its first chained transcript record. There is no leg-2 start signal and no arming recipe: the manifest `start_signal` table and `arming.recipe` are gone (`8b37e91`), and the daemon's gate arms on the first transcript record unconditionally. `arming.signal` itself is still a manifest field, because V9 reads it; Codex's still reads `SessionStart`, which nothing arms on any more.

**Decision.** Every connector declares `arming.signal`, the event that proves leg 2 is live for the tracked workspace, plus `arming.timeout` and, where a human step exists, `arming.recipe`. The daemon runs a three-state machine: `armed` (config written, launch or arm done), `start_signal_seen`, `started_at` (server-signed). It refuses to stamp the third state without the second and tells the participant on screen what is missing. Signals as measured: `SessionStart` with the workspace `cwd` on both Codex modes; `plugin.loaded` on OpenCode; `goalcup.loaded` on Hermes; the shim's first post on OpenClaw; `sessionStart` on Cursor; launch plus the first transcript record on Claude Code and Claude Desktop, where `start_signal.required` is false.

**Why.** Hooks are read at session or thread start everywhere. A session opened before arming produces no posts for its whole life and shows the participant nothing. Stamping the clock for a blind session is the worst failure available: days of work on an entry that cannot be verified.

**Costs.** One more way a start can fail, and a human step on the Codex app before the first thread. Claude cannot supply a hook-derived birth at all, so a missing `SessionStart` there must never raise an anomaly, while on the Codex app the same absence is exactly the failure.

**Changes elsewhere.** D7 gains the gate; PRD R4, R5 and the manifest carry the fields; `RULES.md` carries the Codex app arming step.

## D17. **(M0)** Five health signals, and a per-connector hook-silence policy.

> **Superseded on `m2/wire` by the wire-primary spec (§4 "Dies"); in force on `v1/trunk` and `v0.1.0`.** Replaced by: signals 1, 3, 4 and 5 remain. Signal 2 (hook silence) and the per-connector `hook_silence_policy` table are gone; `HealthState::HooksSilent` stays a valid wire value that nothing produces. The wire leg's own health signal comes with the proxy. Neither shipped manifest declares `hook_silence_policy` (`8b37e91`); a recorded V1 manifest that does is accepted and the table ignored.

**Decision.** The daemon judges exactly five things: (1) the expected transcript stalled while the harness process is alive; (2) hook posts silent while leg 1 grows; (3) the arming signal absent at start; (4) a tracker config file deviating from its expected hash; (5) the harness version changed mid-run. Nothing else. Blanket "hook silence is a flag" is replaced by a per-connector table declaring, per event, `expected` | `conditional(<reason>)` | `never_silent`, plus `hot_disable: none|restart|hot(<seconds>)`.

**Why.** Signals 4 and 5 were proven live in M0: the config hash check caught a mid-chat Cursor hook deletion on the very next post, and the Codex app self-updated mid-run. A blanket silence rule would misfire constantly, because silence is normal for Claude `SessionStart`, for permission events in automatic mode, and on Codex, which fires `SessionStart` at the first prompt rather than at launch. Signal 2 is the universal fallback and the only thing either Hermes bypass leaves behind.

**Costs.** Every connector fills in the table, and a wrong entry produces either noise or a blind spot. OpenClaw hot-disables in 0.7 s, so signal 2 needs a short evaluation window there.

**Changes elsewhere.** PRD R14 health states; SYSTEM-PLAN section 3; manifest field `hook_silence_policy`.

## D18. **(M0)** The human-message rule is two-legged, with an exclusion list.

> **Latest M2 product decision: D37.** Zero detected messages is insufficient to establish
> unassisted work. Preserve unresolved authorship and capture gaps; detailed observations stay
> private unless deliberately included in a reviewed public derivative. The older M2 banner
> below records the previous design, not authorization to publish complete traces.

> **Superseded on `m2/wire` by the wire-primary spec (§2.1, §2.2, §4); in force on `v1/trunk` and `v0.1.0`.** Replaced by (as decided 2026-09-25, spec §11.6): the wire is the record of every message the model saw; a user-slot record is classified by three signals and no blocklist: typed bytes on a passive pty (human), structure from the connector's decoder (machine-authored), or Unexplained, which is published as such and is a Flag. The tracker relays nothing (`goalcup continue` is dropped). The verifier no longer applies the catalogue and V3's excluded-twin check is gone (`8fb871c`, `67f289d`), but the `[[human_rule.exclusions]]` tables are still in both shipped manifests, unread by the human rule, because the manifest validator still requires a non-empty catalogue (`crates/goalcup-protocol/src/manifest.rs:1174`); removing the requirement and the tables is the first item on `docs/superpowers/plans/2026-09-24-m2-follow-ups.md`. Until the launch channel lands, the transcript leg alone has no human-message witness: V1 holds the messages as unwitnessed, V2 flags an unwitnessed `keep going`, V3 flags each message, and on Claude Code V8 fails any run whose transcript records a tool call, because leg 2 records none. Every honest run with a tool call therefore fails verification on `m2/wire`. On Codex it is worse, as an accepted interim state: nothing now tells the rollout's `<environment_context>` header from a human message, so V1 reads it as the goal and V2 fails the real first message (wire-primary spec §11.2).

**Decision.** A message counts as human only when a leg-2 post pairs with a leg-1 record that passes the connector's positive test, is not on the connector's ordered exclusion list of harness-authored user-slot content, and satisfies the scoping clauses (`parent_id IS NULL`, `_delegate_from IS NULL`, `thread_source != subagent`, the `subagent_history_start_ordinal` cut, session directory equals the workspace). Each connector declares `human_rule.id`, `human_rule.exclusions[]`, `human_rule.byte_exact_field` and `normalisation.trailing_newline`.

The exclusion catalogue is normative and lives with the connector; the measured list per harness is M0-FINDINGS §2.3, asserted by PRD V6.

**Why.** Every harness with subagents posts harness-authored text through the same prompt hook that carries human text, and none of the single positive markers the research files named is sufficient on the versions tested.

**Costs.** The most expensive field in the manifest to get right, and the one most likely to break on a harness update. Length heuristics do not help: the Codex prefix varied from 3,735 to 13,177 characters by project. The verifier normalises exactly one trailing newline and never treats wire-versus-typed divergence as tamper evidence on its own, because Hermes and OpenClaw legitimately differ between what was typed and what the model saw.

**Changes elsewhere.** D8 annex; PRD V2, V6, V7 and 5.4; CAPTURE-RESEARCH 5.5.

## D19. **(M0)** Every store declares its own cursor type.

**Decision.** `store[].cursor.type` is one of `ordinal`, `rowid`, `sqlite_pk`, `session_seq`, `inode_offset`, with an explicit `unsafe_cursors[]` list. Per harness in SYSTEM-PLAN 4.3. Two cursors are unsafe and named as such: `rowid` on OpenClaw, because deleting a subagent generation frees rowids that later inserts reuse, and the global `seq` on OpenCode, because it is unique per aggregate only. JSONL stores are read by inode plus byte offset in append order, since timestamps inside the file are out of order.

**Why.** The stub tailer's rowid cursor missed three OpenClaw rows live, including the subagent parent link, and a global `seq` cursor on OpenCode would have dropped a child session's 42 rows.

**Costs.** No single generic SQLite tailer. A restarted tailer that re-reads from offset 0 produces a second complete chain, which is expected and must not be treated as corruption.

**Changes elsewhere.** PRD R6 and 5.4.

## D20. **(M0)** Leg-1 stores are partitioned at capture time.

**Decision.** Most leg-1 stores are home-wide or agent-wide. The daemon applies the connector's `store[].partition_key` before hashing or shipping and discards foreign rows without persisting them. Keys per harness are in SYSTEM-PLAN 4.3; the awkward one is OpenClaw, where the workspace field always reports the Gateway's own workspace and only `ctx.sessionKey` can filter. Claude Code CLI needs no key: process-scoped pins leaked nothing.

**Why.** Contamination is not marginal. In one eleven-minute Codex app run, 97.7 % of rollout bytes were foreign, plus 1,050 thread titles; on Claude Desktop 23 of 44 request bodies were foreign, one from an unrelated folder. Discarding later would mean shipping other people's sessions first.

**Costs.** The tracker must read directories it does not own (the day's `~/.codex/sessions/`, home-wide SQLite stores on OpenCode, Hermes and OpenClaw), which the privacy statement in `RULES.md` must name plainly. Partitioning bugs silently drop real evidence, so the conformance suite asserts partition behaviour on fixtures containing foreign rows.

**Changes elsewhere.** D5; PRD R6, R7, R8; `RULES.md` privacy section.

## D21. **(M0)** Four named app-wide exceptions, disclosed on the public entry.

**Decision.** These four app-wide writes are permitted, each declared in the manifest as `app_wide_exception` with the file, key, expected diff shape, reversal procedure and public disclosure string, each bracketed with before and after hashes:

| Harness | Write | Who writes it |
|---|---|---|
| Claude Desktop | `OTEL_LOG_RAW_API_BODIES` in `~/.claude/settings.json` | the tracker, reversible |
| Codex app | twelve `[hooks.state…] trusted_hash` tables in `~/.codex/config.toml` | Codex's own trust UI, after a human review |
| Hermes | one `plugins.enabled` key in `$HERMES_HOME/config.yaml` | the tracker, backed up and reversible |
| Cursor | `agent-cli-state.json`, `ai-tracking/`, `skills-cursor/**` and a login Keychain token | the harness itself, despite all three env pins; involuntary and not reversible by the tracker |

The app-wide check becomes "no change to a named `config_allowlist[]` of files that affect harness behaviour", with the app's own interface and telemetry state out of scope, and baselines taken immediately before launch with modification times recorded.

**Why.** The alternative was declining four harnesses. "Nothing app-wide changed" is not an assertion any verifier can make on a GUI app, because the app rewrites its own configuration during normal use: Claude Desktop rewrites two config files, the Codex app writes a trust level and bumps its own version key while self-updating.

**Costs.** The privacy promise gets weaker and needs plain language on the public entry. The Cursor row cannot be reversed by the tracker at all, which is part of why Cursor is `elevated_review`.

**Changes elsewhere.** D5, D23; PRD R17 and 5.4; `RULES.md` disclosure section.

## D22. **(M0)** Kill the process tree, not the launcher.

**Decision.** The manifest declares `kill.strategy` as `process_group`, `native_pid` or `wrapper_unsafe`, plus `lock_paths[]`. Seal-time termination follows it.

**Why.** `kill -9` on the Codex npm wrapper killed the wrapper only; the native binary and its host process survived, reparented to pid 1, and kept the thread-writer lock, and a `codex resume` that cannot take that lock silently becomes a new thread with no parent reference. A hard kill also leaves no `SessionEnd` on Codex or Claude Code, so absence of an end record is not evidence of a clean stop.

**Costs.** Per-OS process-group handling in the daemon, and a lock-path check before every resume.

**Changes elsewhere.** D9 incident type; D11 seal; PRD R15 and R16.

## D23. **(M0)** Measured per-harness tiers; D29 controls V1 shipping scope.

**Decision.**

| Harness | Mode | Tier | Preconditions |
|---|---|---|---|
| Claude Code CLI | launch | `full` | `command` `SessionStart` shim; version at or above 2.1.259; two-legged human rule |
| Codex CLI | launch | `full` | managed or bypassed hook trust; native-binary launch and kill |
| OpenCode | launch | `full` | `plugin.loaded` arming; `session.directory` partitioning; SSE leg |
| Claude Desktop | attach | `elevated_review` | no `SessionStart`; raw bodies only under the D21 exception |
| Codex desktop app | attach | `elevated_review` | mandatory CLI-trust arming step before the goal thread (D7, D16) |
| Hermes | launch | `elevated_review` | app-wide `plugins.enabled` key; no length gate; banned bypass flags |
| OpenClaw | launch (Gateway) | `elevated_review` | runtime pin; archive sources; 0.7 s hot-disable window |
| Cursor CLI | launch | `elevated_review` | interactive CLI only, `-p` banned; involuntary app-wide writes |
| dsh | none | **not supported in v1** | no session ever run; no folder-scoped config exists; one process serves many workspaces |

**Why.** These are the measured outcomes of the nine M0 spikes, not estimates. `elevated_review` is not a failing grade: it means an organizer reads the trace before the entry is accepted rather than an automatic seal.

**Costs.** dsh drops out of the announced starting set. It re-enters only if the spike is funded and run, a connector passes the conformance suite, and the browser surface answers A9 before T0; otherwise it stays on the later list.

**Changes elsewhere.** D13; PRD 7 and 5.4; `RULES.md` harness support section; SYSTEM-PLAN 4.3.

## D24. **(M0)** The local hook receiver is authenticated.

> **Superseded on `m2/wire` by the wire-primary spec (§4 "Dies"); in force on `v1/trunk` and `v0.1.0`.** Replaced by: nothing yet. `m2/wire` has no hook receiver, and the hook-hash pin went with it (`17e962b`, `051f485`). No manifest carries `[receiver]` or renders a receiver token any more (`8b37e91`), and `WorkspaceInfo`, `ResumeInfo` and `ResumePreviewInfo` no longer hand one to the CLI (`4039138`). The daemon still mints `RunKeys.receiver_token` internally (`crates/goalcup-daemon/src/registry.rs:1163`), unused by any connector; cutting or renaming it is a follow-up. The recording proxy's per-launch authentication is its successor (probe write-up, "Phase 0 code review"), and it is not built.

**Decision.** The daemon's hook endpoint requires a per-run token, carried in the manifest as `receiver.token` and written into the folder-scoped hook config and shims. Unauthenticated posts are dropped without persisting.

**Why.** An unauthenticated loopback receiver accepted a post from another spike's leftover shim (spike 5, Surprise 8). Loopback is not an authentication boundary on a machine that runs several harnesses.

**Costs.** The token is in a file the agent can read, so it defends against accident and cross-talk, not against a determined participant. Two-leg reconciliation remains the defence against forgery.

**Changes elsewhere.** PRD R1 and R7; 5.4.

## D25. Participant feedback is a consumer of the status API, never a daemon feature and never agent context.

**Decision.** Feedback that the tracker is armed, live, syncing, or near the deadline reaches the participant through surfaces only a human sees: a bundled static status page served by the daemon's local endpoint and opened in the browser when the workspace is armed (closable, reopened with `goalcup ui`), OS notifications at state changes, the first-prompt block reason where the harness can block, and the Claude Code status line in launch mode. The page shows arming state, time remaining, last sync, events uploaded and the health state in plain words, links to the Agent Community site, lists the sponsors, and loads nothing from the network. A native tray or menu-bar indicator is a later bolt-on over the same endpoint.

**Why.** M0 showed that a session opened before arming runs blind with nothing on screen, and that participants have no way to tell "capturing" from "silent". The status cache already exists for the CLI and the MCP tools, so the page is one route and one HTML file; the browser gives macOS, Windows and Linux for free.

**Costs.** One more local port exposed on loopback (read-only, no secrets in the page). The page is not the evidence; the signed cache is.

**Changes elsewhere.** PRD R18 and 5.2; D8 remains: nothing here is injected into the agent.

## D26. Token usage is derived by the verifier and published; it is not a rule.

**Decision.** Every harness already writes usage counts (Claude raw-body `usage`, Codex `token_count`, OpenCode message tokens, Hermes `session_model_usage`, OpenClaw and Cursor usage fields). The verifier sums them per entry and per model, cross-checks totals across legs where both exist, and the summary appears on the entry page and in the research release. Plan, credit and billing fields are stripped before publication. The daemon does not count anything, and no rule sets a token budget: participants pay their own subscriptions.

**Why.** The numbers are free, they make a good public statistic, and an unexplained jump in usage across a gap is one more reconciliation signal. Making them a rule would push the daemon into interpretation (D3) and would penalise harnesses that report differently.

**Costs.** Cross-harness totals are not comparable one to one (cache reads, per-turn versus per-call accounting), so the page shows them per harness with that caveat.

**Changes elsewhere.** PRD V11; the research-release format.

## D27. **(dogfood)** `[home_seed]`: the per-run home is seeded by READING the participant's own configuration, and what it carried is recorded.

> **Removed by [D48](#d48-m2-the-v1-path-leaves-the-tree), 2026-10-11.** The CLI code that follows
> `[home_seed]` was V1-only and is deleted; the verifier still reads recorded `HomeSeeded` events.
> The `v0.5.0` tag keeps the code.

**Decision.** A connector may declare a `[home_seed]` section, and `goalcup start` and `goalcup resume` follow it before the harness is spawned: named keys are copied out of the participant's own harness configuration file into the document at `[home_seed] to` inside that entry's harness home, and named whole files are copied out of `[home_seed] from_dir` into the same home at mode `0600`. Nothing under the participant's home is opened for writing, and the manifest has no field that could ask for one. A file the per-run home already holds is left as it is; a file the machine does not have is reported and never refused; a machine carrying none of `required` is refused with the step to take. The one value the tracker decides rather than copies is the `trust_keys` entry marking that entry's own empty workspace trusted, written only into the per-run document and only for that one path. Every seeding is chained as a raw `HomeSeeded` record naming the connector, the manifest's spelling of the source, the keys, the keys that were already identical, the keys the tracker itself wrote, and the files copied, kept or missing.

The maintainer's ruling that fixes the shape, recorded 2026-09-08 in `docs/dogfood/2026-09-07/ledger.md`: **"harness settings are the participant's decision, not ours (browser tools etc. allowed if they chose them); the tracker copies their answers and RECORDS as many of these decisions as possible."** So the seed never invents an answer and never overrides one — it carries what is there, and the trace says what it carried.

**Why.** Every entry gets a fresh harness home, and a fresh home is a first-run home. The first dogfood run of Claude Code halted on a theme picker and then "Select login method" on a machine that had been signed in for months (D1, 2026-09-07); the first Codex run stopped at the sign-in screen with an empty `CODEX_HOME`. One of the modals was worse than an obstacle: the Claude-in-Chrome offer swallowed a typed message and the harness exited 0 with the transcript unchanged — a run that looks clean and holds nothing. The alternative to seeding was asking a participant to complete a login inside a tracked session, which the rules forbid.

**The value rule, because this record is published.** A key's value is carried when it is a boolean, a number, or a string of at most 64 bytes — the shape an answer to a harness question has. An object, a list, `null` or a longer string is named and never quoted, which is why `oauthAccount` appears by name alone. A copied file is recorded by name and byte length only: never its contents, and never a digest of them, because a digest of a credential is a stable identifier for it and two public traces carrying the same one would prove the same account ran both.

**Costs.** The tracker reads a file that holds an account identity, and on Claude Code it puts `oauthAccount`, which carries the participant's email address, into the per-run home on the participant's own machine. A participant who had answered "yes" to browser tools runs their tracked entry with browser tools attached to their own Chrome; that is their decision under the ruling above, the record says the switch was carried, and the hooks capture every tool call it leads to. The record is the CLI's word about what it wrote: a participant who edits the per-run home by hand afterwards is caught by the config watch and the hooks, not by this. And a fresh home below Claude Code's fullscreen-upsell threshold still meets that one prompt, copied count and all.

**Changes elsewhere.** `RULES.md` "Tracker, auth, and privacy"; PRD 5.4 (`home_seed` in manifest schema v0); README's account on what `start` reads. D21 is unchanged and stays the list of app-wide **writes**: this is a read, and no manifest under `connectors/` declares an app-wide exception today.

---

## D28. **(M2 reservation)** Wire and PTY capture are a later launch-mode evidence boundary.

**Decision.** Wire and pseudo-terminal capture are reserved for a bounded M2 task; they are unavailable in the current V1 binary and do not alter V1's transcript/hook evidence contract. M2 may run a passive loopback forward proxy only through process-scoped `HTTPS_PROXY` and an environment-scoped trust anchor. It must not set a model base URL or alter the harness destination. Raw credentials may be transiently received and forwarded in memory; a per-run salted digest and length may be derived there, but raw values are redacted before payload hashing, disk, upload, or logging and are never retained or persisted.

**Why.** The reservation records the measured wire/PTY opportunity without representing unbuilt capture as current evidence or weakening V1's subscription-auth and privacy boundaries.

**Disposition.** The nine M2 interface reservations are scheduled for the next bounded M2 reservation task. This hardening batch does not silently accept, implement, or refuse any of them.

**Changes elsewhere.** `VISION.md`, `RULES.md`, `SYSTEM-PLAN.md`, `STATUS.md`, and `AGENTS.md` carry the same M2-only boundary.

---

## D29. V1 is the defined foundation qualification target; V1.1 is product and event expansion.

**Decision.** V1's required support matrix has one row: Codex CLI launch mode, one exact tested
harness version, on one exact tested macOS architecture. The other existing CLI connectors remain
preserved candidates and may be promoted only after the same final lifecycle suite passes.

V1's qualification definition requires one complete short-duration lifecycle: scoped workspace
creation, arming, immutable first-prompt binding, capture and upload, interruption, daemon
restart, recovery, same-session resume, manual finish, automatic signed short-deadline finish,
bounded process termination, immutable artifact capture, and offline verification. The resulting
artifact retains every eligible workspace file's path, kind, executable semantics, length, digest,
and bytes in private content-addressed storage, and the terminal chain binds it. V1 runs no
participant Git mutation and makes no public-publication claim.

**V1 finalization order.** One durable `SealingIntent` closes admission and fixes the cutoff
instant; the daemon performs the bounded drain of already-admitted evidence, writes the cutoff
including that drain, durably binds its sequence/hash to the intent, and reconciles the intent and
chain before termination, artifact capture/readback, and terminal finalization. Publication is
strictly post-finalization V1.1 work over the immutable artifact.

V1.1 owns additional CLI surfaces, Desktop, native Linux/Windows, continuous Git observations,
Git-aware eligibility and history, canonical Git-tree construction, participant-side automatic
publication, production organizer services and keys, signed/notarized self-service packages,
presentation operations, and the later 168-hour readiness exercise. Each capability is promoted
as an independently tested surface/version/OS/architecture row rather than waiting for one broad
all-or-nothing gate.

M2 owns wire and PTY capture under D28. It is independent of V1 finalization and V1.1 product
expansion except at shared protocol, launch, trace, and final-package seams.

**Why.** The 2026-09-09 release definition combined the integrity foundation with five surfaces,
three operating systems, production distribution, automatic Git publication, event operations,
and a seven-day qualification. That is a coherent broad launch, but it is not the smallest honest
foundation. Qualifying one vertical slice first exposes lifecycle and artifact failures before
multiplying them across surfaces and platforms.

**Claims.** V1 may say only that the named supported session produced the captured evidence and
immutable artifact under the named tests and limitations. It may not say that the public event is
ready, the run lasted 168 hours, production anchors were used, a public repository was submitted,
all human interference was detected, or any unqualified platform/surface is supported.

**Supersedes for milestone assignment.** D4's cross-platform service consequence, D11's
publication mechanics, D21's Desktop exceptions, D23's five-surface V1 classification, D25's
cross-platform operation, and the 2026-09-09 broad acceptance gate remain valid V1.1 designs but
are not V1 release gates. D11's V1 finalization ordering is superseded as stated above. D1-D3,
D5-D10, D12-D20, D22, D24, D26-D28 remain binding wherever applicable to the narrow V1 surface.

**Changes elsewhere.** `PRD-tracker.md`, `VISION.md`, `RULES.md`, `ARCHITECTURE.md`,
`SYSTEM-PLAN.md`, `README.md`, `STATUS.md`, and the current delivery plan carry this split.

---

## D30. `Confirmed` termination is bounded to the owned process set, and the limitation is stated.

> **[D48](#d48-m2-the-v1-path-leaves-the-tree), 2026-10-11.** `runtime/seal.rs`, cited below, was
> V1 code and is deleted (the `v0.5.0` tag keeps it); `termination_summary` is now in
> `runtime/termination.rs` and the definition stands.

**Decision.** `TerminationOutcomeV1::Confirmed` means: a process-table sweep taken after
signalling found no member of the bounded owned target set — the group leader, its group
members, and descendants walked up to the connector's `process_walk` bound — alive, re-confirmed
by one later sweep. `crates/goalcup-daemon/src/terminate.rs` already implements exactly this:
group signal → descendant walk → SIGTERM with grace (re-authenticating the root and adopting new
descendants on each poll **while the root is alive**; once the root has exited the sweep keeps
asking about every pid it already targeted and, as of `4d15fbb`, adopts nothing new — see the
2026-09-16 reopening below) → SIGKILL → a verification sweep, returning a `TerminationReport` with
`targeted`, `survived_term`, `killed`, `still_alive`, `all_gone`, `waited_ms`, `guard_degraded`,
and `harness_unverified`; 23 tests in `tests/process_kill.rs` exercise it, 21 of them on Linux (two
are macOS-gated). **Superseded 2026-09-17:** the wiring gap this paragraph described is closed.
`runtime/seal.rs:431-463` runs the sweep and reaches `TerminationOutcomeV1::Confirmed` at `:456`;
the hard-coded arm at `:390-399` is now only the `AlreadyStopped` early return. `Confirmed` is
reachable on a V4 run — whether it is *sound* is the separate question the 2026-09-16 reopening
below records.

The limitation is stated plainly rather than left implicit: a process that left the process group
and the walked descendant lineage before the first snapshot is outside the owned set and is not
claimed by `Confirmed`. `docs/VISION.md` "What the tracker must guarantee" points 1 and 3 already
say why this is the honest bound — bounded capture and reviewable tamper *evidence*, not a promise
to defeat every possible escape from a machine the participant controls.

**Why.** A definition that names only the enum variant invites the reading "nothing survived, full
stop." It means "nothing in the set the daemon could bind and watch survived, confirmed twice."
The gap between those two readings is exactly what the walked-descendant limitation records, and
leaving it unstated is how a later reader reopens a question that is already settled.

**Costs.** A daemonized child that forks and reparents before the daemon's first post-signal
snapshot is not detected by this check. That is a known, bounded blind spot, not a defect;
`docs/CHEAT-CHECKLIST.md` carries the corresponding row. **A fork by an already-targeted member
after the root has exited is a different case: under `4d15fbb` it is never adopted for the
remainder of the sweep and `Confirmed` can be returned over it
(`docs/qualification/2026-09-16-review.md` F0, a regression against `ad419b0`). That is a defect,
not an accepted cost, and it is closed before the qualifying journeys.**

**Changes elsewhere.** The 2026-09-13 spec's "V1 finalization order" step 5 and "Finish and
cutoff" section; `docs/CHEAT-CHECKLIST.md`.

**Pinned 2026-09-16.** `Confirmed` rests on `observed_gone` — the observation the definition
above already names — and on nothing else. The atomicity claim is a separate fact: `all_gone` is
`observed_gone` under an instance-bound signal, which a numeric POSIX kill on macOS never is for
a tree of more than one process, so it is recorded beside the verdict by `termination_summary`
(`crates/goalcup-daemon/src/runtime/seal.rs`) and never gates it. This is the reading `4d15fbb`
implements, the `Anchor` split in `terminate.rs`, and it is not a change to this decision: no
decision here ever made signal atomicity a *requirement* of `Confirmed`. The coupling did exist
in the code, and `docs/qualification/2026-09-15-codex-recon.md` (D2) reported it accurately as
the then-current gate `all_gone && !harness_unverified`; that was a reading of the
implementation, never a rule this document set. It is written down because the reading has now
been reopened twice.

**Reopened 2026-09-16.** Lane C (`docs/qualification/2026-09-16-review.md`) found three ways
`Confirmed` can be returned without the tree being dead — F0 (no newcomer adoption once the root
is gone, measured), F1/F1b (`is_alive` answers `false` when it cannot spawn `/bin/ps` at all, and
every sweep including the reconfirm uses it) and F6 (the group path, which reaches no shipped
connector). Recon D1 (`docs/qualification/2026-09-15-codex-recon.md` §7) is also still open:
`discover()` signals the native binary first. All four are ruled on and fixed before the
qualifying journeys; the definition above does not change.

## D31. Crash recovery resumes the interrupted intent; it never quarantines an in-progress run.

> **Removed by [D48](#d48-m2-the-v1-path-leaves-the-tree), 2026-10-11.** The V1 sealing recovery
> below is deleted; the daemon now refuses a registry whose V1 row still owes work
> (`runtime/daemon.rs`). The `v0.5.0` tag keeps the code.

**Decision.** A crash **during finalization** — a durable `SealingIntent` already exists for the
entry, in state `Sealing` or `Sealed` — resumes and completes that same intent on restart; it is
never diverted into a refused or quarantined outcome. A crash **during ordinary capture** — no
finalization intent yet exists for the entry — resumes capture through the daemon's normal
rehydration path and is not treated as a finalization event at all.
`recover_sealing_entries_for_config` (`crates/goalcup-daemon/src/runtime/seal.rs`) already gates
recovery on exactly that condition — an entry with no sealing intent is skipped by the recovery
pass and falls through to ordinary capture rehydration — and the harvested tree's extended gate
(`runtime/seal.rs:1932-1942`) keeps the same split, adding intent phase to the check.

**Why.** A tracker crash is a tracker failure, not participant misconduct, and recovery must never
cost a participant a clean result by treating an interrupted but otherwise honest run as tampered.
The cutoff instant, once durably bound, is fixed before the crash window by construction (D29's
finalization order, steps 1 and 3), so resuming the same intent cannot admit any work performed
during or after the crash as in-window evidence — there is nothing left for a quarantine policy to
protect against that the cutoff binding does not already exclude.

**Costs.** A design that instead marked every non-terminal intent nonclean on restart (proposed and
rejected; see D34's `30d7850` evidence) would have made every crash-restart during ordinary capture
look like tampering, directly contradicting the spec's own mandatory interruption/recovery test
matrix, which requires a daemon restart during capture to resume cleanly.

**Changes elsewhere.** The 2026-09-13 spec's "V1 finalization order" step 4 and "Finish and
cutoff" section.

## D32. Receiver evidence loss is a verifier finding, not a daemon-side refusal.

> **[D48](#d48-m2-the-v1-path-leaves-the-tree), 2026-10-11.** The V1 daemon path is deleted (the
> `v0.5.0` tag keeps it); the verifier rule below stays and still applies to recorded v4 traces.

**Decision.** A gap in the leg-2 receiver's declared sequence is decided in the verifier, by one
rule: per `source`, declared `receiver_meta.seq` values (`Leg2Post::receiver_meta` is `PostMeta`,
carried raw — `seq: Option<u64>`, `source`, `anomalies`, `reported_config_hash`) must be
contiguous; a hole yields finding `leg2_receiver_seq_gap`, naming the source and the expected and
observed values as `Evidence`. A post with `seq: None` takes no part in the rule — absence is not
a hole. This is the same verdict class the verifier already gives any other evidence gap (Flag,
never a silent Pass, never a Fail on its own). No daemon-side marker forces an entry into a
refused state over a receiver gap.

**Why.** This follows the standing architecture rule (D3, "record raw, interpret later" — the
daemon never depends on the verifier): the daemon records raw bytes — `chain.rs` already reads
`receiver_meta` "by no check" (`crates/goalcup-verifier/src/chain.rs:462-464`) — and every
judgement about what those bytes mean belongs in the verifier, open source and re-runnable, not
compiled into the daemon as a forced state transition.

**Costs.** None beyond the ordinary cost of any Flag finding: a receiver gap does not by itself
fail a verdict, and review judgment (automated or human) decides what a Flag means for a given
entry, same as every other gap finding.

**Changes elsewhere.** None to `goalcup-protocol` or `goalcup-daemon`; the finding lives entirely
in `goalcup-verifier`.

## D33. No Endpoint Security observer, no privileged daemon, no protocol V5 — in V1 or V1.1.

> **Status note, 2026-10-02 (not a decision change).** "No protocol V5" is scoped to V1 and V1.1.
> The M2 recorder on `m2/wire` uses a v5 chain by [D37](#d37-m2-autonomous-runs-research-evidence-and-open-contributions)
> (its 2026-09-27 review amendment: "a distinct v5 LocalFinish"). The rejections of the ES observer
> and the privileged daemon stand.

**Decision.** None of an Endpoint Security (ES) observer, a privileged daemon, or a protocol V5
ships in V1 or is scheduled for V1.1. `Confirmed` termination (D30) is reached with the
unprivileged process-table sweep the daemon already has, once wired.

**Why.** A 2026-09-15 discovery pass proposed all three to clear what it read as a `Confirmed`
blocker. The blocker turned out to be an unwired function call (D30's `runtime/seal.rs:396-400`
never calling the already-implemented `terminate.rs` sweep; since wired — see D30) rather than a
missing OS capability.
An ES observer needs an Apple entitlement and a signed system extension that an unsigned,
unnotarized bundle cannot ship (D29's packaging row), and it would also contradict
`docs/VISION.md` "What it is not": "Not device surveillance." A privileged daemon and a new
protocol version were proposed to carry the same capability and are rejected for the same reason:
the actual gap did not need them.

A detached-capsule receiver redesign, proposed separately to harden receiver evidence, was also
assessed and rejected: it multiplies durable writes per hook post roughly 2x to 7x against the
harness shim's measured 3-second budget, and its "dirty epoch" rule would make every
crash-restart during ordinary capture nonclean — directly contradicting D31 and the spec's own
mandatory interruption-and-recovery tests.

**Costs.** None; nothing was built and nothing is removed. This decision closes the proposals so a
later agent does not reopen them believing the blocker is still open.

**Changes elsewhere.** The 2026-09-13 spec's "Explicit non-goals across all three stages" list and
"M2 capture expansion" section gain this explicit rejection.

## D34. `3633a14` row-hardening is deferred past V1; cherry-pickable afterward.

**Decision.** `3633a14`'s evidence-row hardening (frame-v3 row MACs, authority-pristine markers,
reachability validation, +1282 lines in `buffer.rs`) is dropped from V1's harvested tree. It
guards evidence-row substitution by an attacker who already holds the run key; lateness detection
does not depend on it. It remains cherry-pickable after V1 on its own merits.

**Verified 2026-09-15.** Dropping it costs exactly 11 tests and 1,135 lines of `buffer.rs`
(4963 → 3828 lines).

Lateness detection is independent of it: `submission_post_cutoff_evidence`
(`crates/goalcup-verifier/src/checks/v13_binding.rs:539`) fails the verdict if any `Leg1Record` or
`Leg2Post` appears past `cutoff_seq`; late arrivals route to the private out-of-window store
rather than the chain; overflow triggers `persist_out_of_window_nonclean` →
`record_terminal_drain_validation_failure`; `submission_drain_degraded` independently fails a
degraded pre-cutoff drain. None of this depends on `3633a14`.

**`30d7850` rejection evidence.** A merged tree that included the rejected receiver-redesign
branch `30d7850` (see D33) failed three tests serially:
`live_and_recovery_tailers_filter_root_session_and_replay_each_coordinate_once` (expected `(2,0)`,
got `(1,0)`); `restart_after_termination_binding_never_reopens_an_uncaptured_workspace` (expected
phase `"terminal"`, got `"terminated"`); and
`restart_joins_the_durable_capture_owner_without_resnapshotting_changed_source`. Excluding
`30d7850` makes all three pass. This is independent confirmation that rejecting that branch — and
its "dirty epoch" quarantine policy (D31, D33) — was correct, not merely a matter of preference.

**Why.** Dropping code that guards against an attacker who already holds the run key is a small,
well-understood reduction in defense-in-depth, not a reduction in what V1 can honestly claim: the
verifier still detects lateness, gaps, and receiver loss (D32) without it.

**Costs.** An attacker who has already compromised the run key could substitute an evidence row
without the row-level MAC catching it; the daemon's other detectors (chain hash, gap findings,
attribution) are unaffected. Re-adding it post-V1 is a bounded cherry-pick.

**Changes elsewhere.** None to shipped V1 code; a note for whoever picks it back up post-V1.

---

## D35. **(dogfood)** A Fail is a rule broken; an inference from incomplete information is a Flag. `unexplained_seal_file` is a Flag.

**Decision.** The verifier's two severities are separated by what they claim, and the line is
written down here rather than decided per finding:

- **Fail** — a rule was objectively broken, or the evidence itself is broken. V2 (a human
  message that is not `keep going`), V13 (a trace not bound to a server-signed start),
  `no_session`, `chain_broken`, `store_rewritten`. No judgement is required to read one, and
  nothing an honest participant does produces one.
- **Flag** — an inference drawn from incomplete information, which an organizer or later tooling
  adjudicates. `read_only_attribution`, `bulk_attribution`, `model_unknown` — and now
  `unexplained_seal_file`, which `crate::verdict::seal_findings` raises through
  `checks::flag` instead of `checks::fail`.

The finding keeps its message and keeps its place in the review queue. It stops being a verdict.

**Why.** Absence of attribution is not evidence of wrongdoing. No verifier can parse every
command: `npm install`, `make`, `cargo build` and `python gen.py` each produce files that no
command text anywhere in the trace names, and under the old severity every one of them
disqualified the entry that ran them. That is not a theoretical objection. The real Codex journey
of 2026-09-17 (`docs/qualification/2026-09-17-journey-dogfood.md`, entry `e7029d23ed4ecdafa`)
passed all thirteen checks V1–V13, including session continuity across a live resume, and still
came out Fail — on one honestly written `hello.txt`. The defect behind that particular run is
fixed in the same change (Codex 0.153.0's code-mode envelope separates commands with a JS-escaped
newline, which `shell_attributes` did not treat as a line break while its sibling
`patch_target_paths` did). Fixing one envelope does not change the shape of the problem: the next
harness, or the next build tool, produces the next unparsed write.

`docs/RULES.md` already uses "flag" exactly this way, twice — *"Editing them is a flag"*, and
*"the tracker samples the version, records the change as an event, and an organizer looks at it.
That is a flag, not a disqualification."* The verifier was the part that was out of step.

**Where the anti-cheat claim actually rests.** Not on attribution, and
`docs/CHEAT-CHECKLIST.md`'s "copy a prepared tree into the empty workspace" row has been rewritten
to say so. It rests on three things:

1. **The baseline (R3).** The daemon manifests the workspace as it creates it, the manifest is
   chained at seq 0, and V13 decides whether it is bound tightly enough to exempt anything. The
   2026-09-17 journey recorded `file_count: 0`: the workspace is provably empty at the start, so
   every sealed file is work that appeared during the observed run.
2. **Continuous observation.** Heartbeats carry per-store positions across the whole run, so
   there is no unobserved interval in which a tree could be imported. A hole is a dirty gap, and
   V4 and V12 say so.
3. **The foreign scan.** No second session wrote into the workspace (D5, V4) — only sessions in
   tracker-created folders are ever seen, and a foreign one is chained as foreign.

Attribution corroborates those three. It is what an organizer reads to understand a workspace; it
is not the gate.

**Costs.** Stated plainly: a determined participant who smuggles a file into the workspace in a
way the baseline, the heartbeat series and the foreign scan all miss now produces a **Flag** where
they previously produced a Fail, and an organizer has to read it and decide. The verdict no longer
disqualifies them by itself. That is the price of not disqualifying every honest entry that runs a
build tool, and it is the right way round: a Fail that fires on honest work teaches organizers to
ignore Fails.

**Changes elsewhere.** `crates/goalcup-verifier/src/verdict.rs` (`seal_findings`);
`crates/goalcup-verifier/src/lib.rs` (the review queue still feeds `overall`, and genuine queue
Fails such as `no_session` still raise it); `crates/goalcup-verifier/src/attribution.rs` (the
code-mode segmentation fix and its doc); `docs/CHEAT-CHECKLIST.md`; `docs/ARCHITECTURE.md`.

---

## D36. **(M2)** Two-tier redaction on the wire leg, and a harness-agnostic recorder.

> **2026-09-25 amendment: D37 makes decoders first-class but optional for initial capture.**
> Item 4's requirement that every capture connector ship a decoder is superseded. The [M2
> contract](CONTRACTS.md#c1-limits-and-representation-t00) defines sanitized segments
> and explicit gaps on unsupported encoding.
> Item 1's optional credential fingerprint is removed too: retain neither values nor fingerprints.
> Privacy and credential-removal obligations remain; use the replacement recorder task set.

**Decision (maintainer, 2026-09-25; `m2/wire` only, amends D14 and D28 there).**

1. **The harness's credential is never stored, anywhere, ever.** The proxy strips credential
   headers by name before any byte leaves it, keeps at most a per-entry HMAC fingerprint, and
   never decrypts sign-in or token-refresh hosts. This half of D28 stays absolute.
2. **Body content is redacted best-effort locally, with a second pass on the server, and raw
   traces are never published.** The local pass is the daemon's generic pattern redactor with its
   log (D14); it is not required to be complete, because a participant's own body content (a
   file the agent read, a tool result) is theirs and goes to a private trace. D14's and D28's
   wording that redaction is finished before disk is relaxed to this for bodies only.
3. **Decoded is raw.** Transport compression (gzip, deflate, brotli, `permessage-deflate`) is a
   wrapper like TLS, not part of the message. The proxy undoes it on its tee and the decoded
   body is the record that is redacted, digested, chained and stored. Compressed bytes are not
   kept. A body the proxy cannot decode is stored as it arrived, marked undecoded in the
   redaction log, and reported by the verifier as a Flag (D35).
4. **The recorder is harness-agnostic; the decoder ships with the connector.** The proxy does
   only transport-level work: terminate the hosts a connector declares, undo transport
   encodings, strip credential headers. It never parses a harness's or provider's message
   format. Each connector ships, in the verifier, the parser for its transcript and the decoder
   for the provider protocol its wire carries (per protocol, not per harness: Codex, OpenCode,
   Hermes and OpenClaw all speak OpenAI Responses to `chatgpt.com`; Claude Code speaks Anthropic
   Messages). "Record raw, interpret later" (D3) holds, and the interpreter is shipped code, so
   the recorded data is usable for analysis without work done by hand afterwards.

**Why.** The maintainer's stated aim for M2: the most stable and usable data for analysis after
judging, and the lowest possible cost for a new harness to be recorded, so that many harnesses
can be supported. The spike measured that what remains per harness is a handful of environment
lines (`spikes/wire-pty/REPORT.md` §4(g)), and the 2026-09-24 probe measured every token value
0 times across 52 decoded files while the credential lived only in headers, so stripping headers
in the proxy is both cheap and sufficient for the credential. Compressed bodies are inert to a
regex redactor (`crates/goalcup-daemon/src/redact.rs`), so redaction has to run on the decoded
form, and storing the decoded form is what makes the data analysable.

**Costs.** An inflater defect leaves no compressed bytes to re-derive from; the transcript leg is
the fallback, and the recorded-session fixture replays the inflater in the gate. Brotli needs a
decoder crate the local registry does not hold yet; it appeared only on non-model records in the
probe, so it is a slice-2 item.

**Changes elsewhere.** Wire-primary spec §11.6; the slice-1 plan, decision 1; `docs/RULES.md:107`
keeps "credentials are never stored".

---

## D37. **(M2)** Autonomous runs, research evidence and open contributions.

**Product decisions confirmed by the maintainer, 2026-09-25; `m2/wire` only.** The written
architecture is the [public M2 architecture](ARCHITECTURE.md#1-scope).
The maintainer approved simplifying the first release.
The supporting private historical record is intentionally omitted from this public OSS cut.

1. **The primary experiment is an agent working without human assistance.** The tracker never
   supplies continuation, encouragement or input relay. Autonomous continuation belonging to
   the harness is permitted when configured before launch and recorded. Human assistance ends
   the unassisted interval; capture can continue, labelled assisted. This includes operational
   help such as approvals and manual resumes, not only messages that reach the model. Recorder
   endurance tests with deliberate interventions remain useful but do not qualify autonomy.
2. **Research traces are a primary deliverable.** Preserve observations needed for later
   comparison of harnesses, models, tool use and behaviour over time. Local capture performs
   transport/scoping, privacy, persistence and health work. Semantic interpretation runs later
   through versioned decoders/verifier code, reusable offline and on the server. Keep integrity,
   capture coverage, assistance assessment and research conclusions distinguishable.
3. **Require real recorder disk capacity before launch.** The maintainer requested actual
   reserved space, rather than merely checking free space. The first-release plan proposes a configurable
   10 GiB reservation in the actual chain file, without extending logical EOF, qualified by a
   dedicated platform gate before implementation. Account serialized bytes and retain a bounded
   terminal allowance. This is neither a duration guarantee nor protection for every metadata
   or harness-output write; unsupported allocation fails preflight.
4. **Both initial harnesses remain in scope.** Codex CLI and Claude Code CLI prove the shared
   infrastructure. New compatible harnesses should be repeatable contributions. Capture
   profiles and provider/native-transcript decoders are equally important open-source work;
   decoders can be shared across profiles. A missing decoder need not prevent capture, but
   prevents any analysis claim that requires it. Record bytes now without pretending that
   every possible format can already be interpreted completely.
5. **Keep the existing repository and qualified foundations.** Replace the M2 plan coherently;
   retain measured evidence, isolated launch, privacy and integrity boundaries. Additional
   hashing/surveys follow reliable capture and unattended qualification. No V1 measurement or
   production-readiness claim changes merely because this decision is written.

**Review amendment authorized 2026-09-27.** The first release is an installable local/private
pilot with no receipt server prerequisite. A distinct v5 LocalFinish binds the seal manifest
and actual typed process outcome; local-only completion does not fabricate a signed deadline.
Existing signed modes retain their server-owned receipt clock after handoff/session binding.
Optional signed M2 upload/anchoring and rich browser/MCP surfaces follow the pilot. Minimal
truthful CLI/API status ships with it. Preserve historical interpretation under fixture-owned
trust inputs; test current shipped pins separately rather than silently refreshing old verdicts.

**Reconciliation.** This supersedes the M2 count-first framing in D8/D18, decoder prerequisite
in D36 and §11.6's assisted run as autonomy qualification. D10's one-harness rule remains.
Use bounded sanitized segments in the existing v5 chain. Redaction precedes encoding/hash/disk;
later upload reads only the synced frontier without redb payload copies. No durable local ingest
ACK/replay or automatic proxy process restart. Ordered producer-close distinguishes clean drain
from crash. Bounded capture-only reconnect at a fresh request/message boundary is allowed:
persistent gaps remain, physical connection identity survives epochs, and no input or provider
request is replayed. This is recorder recovery, not agent assistance. Manual recovery is assisted.
Torn final lines remain preserved/non-resumable, never silently repaired or promoted to Pass.

Real allocation remains mandatory for installed pilot launch. Explicitly unreserved bounded
component experiments can run earlier while T01/T03R qualify allocation; they make no capacity
or release claim. E00/E01 use actual capture components with both real harnesses before full
lifecycle integration. E02 begins meaningful unattended learning and separate controlled capture
endurance early. Final qualification reports agent outcome and recorder endurance separately.
Research keeps source-specific errors/outcomes, timing intervals and unknowns; a new decoder
must work through the ordinary CLI registration path. Universal lineage/classification, new
workspace/process/binary surveys and public publication remain deferred. These amendments are
planning decisions, not evidence that the recorder has been built or qualified.

**Ingest trust (2026-09-29).** Loopback TCP ingest is unauthenticated toward the receiver: the sender presents its per-entry secret and then its evidence to whatever answers at the loopback address. A single-user host is assumed; a second local user is outside the pilot's threat model.

---

## D38. **(M2)** Fresh per-run HOME for Claude Code; cheapest models for live tests.

**Decision (maintainer, 2026-09-28, after E01).** Claude Code runs launched by the tracker use a
fresh per-run HOME whose only entry is a `Library/Keychains` symlink to the user's keychain
directory (subscription login), with `CLAUDE_CONFIG_DIR` pointing at the isolated config dir —
measured in `docs/qualification/m2/launch-recipes.md` §7–§8 as the only process-scoped recipe with
zero attributable app-wide writes. Live test runs use the cheapest models: Claude Code
`--model haiku --effort low`; Codex `gpt-5.6-luna` with `model_reasoning_effort=low`.

**Costs.** The agent does not see the user's dotfiles (git identity, ssh keys, shell rc); if a
later workload needs git or ssh, add a scoped identity or key deliberately rather than exposing
the real HOME.

**Evidence.** `docs/qualification/m2-early-live-capture.md`; handoff
`docs/superpowers/handoffs/2026-09-28-m2-e01-handoff.md`.

**Changes elsewhere.** `docs/qualification/m2/launch-recipes.md` §8.

---

## D39. **(M2)** Headless capture launches, ported extra redaction patterns, shared isolated Codex home.

**Decision (maintainer, 2026-09-28, wave-2 pre-flight).** Three wave-2 pre-flight questions (R7,
R8, R10 in
`.superpowers/sdd/2026-09-27-m2-e01-orchestrator-task/preflight-wave2.md`) are resolved:
- **R7 (redaction).** Legacy connector `[redaction].extra_patterns` are ported to the M2 capture
  path, not dropped. Task T04b adds an additive `StreamRedactor` constructor reusing the existing
  V1 extra-pattern type (no new schema) with the same fail-closed/bounded-hold guarantees; T14
  carries a manifest's `extra_patterns` into the capture profile unchanged and validates them with
  T04b's compiler instead of rejecting them; T15 wires them into the per-producer redactor.
- **R8 (headless launches).** M2 capture profiles lift the V1 headless bans. T14 adds
  `capture.args` — the full measured headless launch template ending in `--`, goal appended as one
  element — validated against a capture-specific deny list (base-URL/provider-key/remote flags);
  the V1 plan path and `banned_args` are untouched.
- **R10 (Codex home).** M2 uses a shared isolated `CODEX_HOME` (as in E01), not a per-entry
  `auth.json` copy.

**Security closure (maintainer, 2026-10-06).** The shared home keeps that exact D39 leaf alias in
the daemon request, local record and child environment. The CLI records and rechecks the alias and
canonical directory identities, effective-UID ownership and exact `0700` target mode before the
create request. The daemon's all-loaded connector `real_dirs` remain independently authoritative.
ACLs and same-UID replacement after the final check remain stated residuals.

**Costs.** Ported extra patterns are participant-authored regexes running inside the bounded
streaming redactor; they need an adversarial streaming review (per-pattern, at the 16 KiB hold
bound) before shipping, the same as the built-in policy (T04-C1). A shared `CODEX_HOME` means
every M2 run of one connector shares one isolated home; concurrent captures with that connector
must be serialized, particularly around auth-token refresh, to avoid the `refresh_token_reused`
failure a per-entry copy would risk instead.

**Evidence.** `.superpowers/sdd/2026-09-27-m2-e01-orchestrator-task/preflight-wave2.md` (R7/C7,
R8/C8, R10/C10); `docs/qualification/m2/launch-recipes.md` §9;
`docs/qualification/m2-early-live-capture.md` §3.2, row (a).

**Changes elsewhere.** `docs/CONTRACTS.md`
"Orchestrator amendments" § Wave 2 (2026-09-28);
`docs/superpowers/plans/2026-09-25-m2-recorder/T14-harness-profiles.md`.

---

## D40. **(M2)** Finish keeps the termination grace; the spawn window is closed; reservation release waits; Hermes uses DeepSeek.

**Decision (maintainer, 2026-10-02, after independent reviews of the Phase 1 Task 4/H-10 and
Task 5/H-11 designs and the H-11 APFS probe).**
- **Termination bound (H-10, review C2).** `goalcup seal`/finish keeps the existing documented
  termination bound: SIGTERM, the connector's grace (10 s for both shipped connectors), then
  SIGKILL. The 10 s `CAPTURE_FINISH`/`CAPTURE_DRAIN` budget is not allowed to cut the grace short;
  the CLI's finish timeout is derived from the existing constants instead of adding a new budget.
  Inside the one capture budget, a drain share and a named commit reserve are split (review C1) so
  a producer that never settles still seals with a recorded gap.
  **Reconsider if** total finish wall time (drain + grace + kill + seal) proves too long for an
  operator or for T27 unattended runs, or if a connector needs a grace far beyond 10 s. The
  alternative was capping termination by the capture budget, which made SIGKILL unreachable for a
  SIGTERM-ignoring harness.
- **Spawn-to-registration window (H-10).** Closed, not declared a residual: the tracer must be
  very stable. A capture launch must not leave a live harness the tracker never durably recorded;
  the design revision chooses the mechanism (for example, the child waits before `exec` until its
  record is durable).
- **Reservation release (H-11).** Deferred. On macOS 26.6.2 APFS only grow-then-shrink frees
  `F_ALLOCATEPERSIST` blocks, and a crash between grow and shrink leaves the sealed chain followed
  by a zero tail, which the reader rejects. Making valid evidence invalid is not acceptable, so
  capture reservations stay small (as T24 already does) and are not released. A separate-reservation
  design that can never touch the chain is to be designed before T27 or any repeated large-reserve
  endurance run.
- **Hermes provider.** The Hermes reference row uses Hermes's `deepseek` provider with the
  owner's DeepSeek API key in `DEEPSEEK_API_KEY`, passed to the child only. No DeepSeek base-URL
  variable or config `base_url` is set. The `openai-codex` OAuth login in the isolated Hermes home
  is not used. The exact model ID is pinned in the Hermes readiness record.

**Evidence.** `.superpowers/sdd/2026-10-01-m2-phase-1-local-reliability/`
`task-4-design-review.md`, `task-5-design-review.md`, `task-5-probe.md`; LEDGER H-10, H-11.

---

## D41. **(M2)** Hermes reference row: model, CA trust, key-fingerprint and key-fragment rules.

**Decision (maintainer delegated to root, 2026-10-02, after the independent Hermes seams review).**
- **Model.** `deepseek-v4-flash`, confirmed by the owner. Hermes 0.19.0 rewrites the canonical
  `deepseek-flash` to `deepseek-chat` (`hermes_cli/model_normalize.py`), while `deepseek-v4-flash`
  passes through and DeepSeek currently routes it to V4.1 Flash. Thinking is off
  (`agent.reasoning_effort: none` in the private config). The request, response and session-snapshot
  `model` values are recorded as evidence of what actually served the row.
  **Reconsider when** DeepSeek retires the alias or a newer Hermes knows `deepseek-flash`.
- **CA trust.** `HERMES_CA_BUNDLE` is admitted for the `hermes` connector family only, process-scoped,
  pointing at the tracker's proxy CA. It replaces Hermes's roots rather than extending them, which fails
  closed (Hermes then trusts only the proxy, and the proxy verifies the real upstream). `SSL_CERT_FILE`
  stays refused.
- **Key fingerprint.** The private `auth.json` (mode 0600) is pre-seeded with
  `{"version": 1, "providers": {}, "suppressed_sources": {"deepseek": ["env:DEEPSEEK_API_KEY"]}}`
  so Hermes never seeds its credential pool from the env key or writes `secret_fingerprint`. An
  empty store alone is what Hermes already assumes when the file is missing and does not suppress
  the env source (`hermes_cli/auth.py:1109-1150`, `agent/credential_pool.py:2400-2420`). Proof:
  after an offline synthetic-key run the file is byte-identical to the seed, and the synthetic key
  still reaches the outgoing request.
- **Key fragments.** On a 4xx Hermes writes `sessions/request_dump_*.json` with `key[:8]…key[-4:]`, and a
  401 body may echo a masked key. Before any Hermes run artifact is retained, a scan with the in-memory
  key (never stored) for its prefix (`key[:8]`), suffix (`key[-8:]`) and masked forms must find nothing; a hit fails the row and
  the run directory stays local and uncommitted. Masked-key forms are also added to wire redaction.
- **Isolation.** Every Hermes invocation, including version probes, runs with a private `HOME` and
  `HERMES_HOME`; nothing writes `~/.hermes`.

**Evidence.** `.superpowers/sdd/2026-10-01-m2-phase-2-hermes-reference/seams-design.md`,
`seams-design-review.md`; `https://api-docs.deepseek.com/news/news260910/`.

**Amended by D43** (CA trust: any connector, by registry mode).

---

## D42. **(M2)** Open-source v0.5: V1 path behind a feature, data-only harnesses, open harness PRs.

**Decision (maintainer, 2026-10-04, after seven readiness reviews).**
- **V1 path.** The v4/V1 transcript-and-hook product path stays in the repo but compiles only with a
  `v1-compat` cargo feature, off by default. It is not public surface for v0.5. Its upload, receipt,
  acknowledgement and sign-in pieces are reused by the server phase; the rest is deleted later.
  (Replaced by [D48](#d48-m2-the-v1-path-leaves-the-tree): the whole path is deleted, and git
  history and the `v0.5.0` tag keep it.)
- **Harness contract.** A harness is data: its connector manifest declares the native source and
  form, the session-identity source, seed files, credential variable and CA variable, under a
  `schema_version`. The v5 evidence format is frozen for v0.x (a breaking change means v6). Common
  cases need no Rust edit; a genuinely new capability (e.g. a SQLite native store) is a separate core
  PR, reviewed on its own.
- **Contributions.** Anyone may send harness PRs; connectors are trusted code, so maintainer review
  is required (CODEOWNERS) against a reviewer checklist. Contributors sign off with DCO
  (amended by [D47](#d47-m2-no-dco-sign-off): no sign-off is required).
- **Publication.** The public repo is a fresh allowlist copy, not this history. Internal plans,
  handoffs, reviews, dogfood logs and spike reports stay internal; `CONTRACTS.md` moves to `docs/`.
- **Legal and contact.** Apache-2.0, copyright AgentCommunity. Security reports via GitHub private
  vulnerability reporting or goalhack@agentcommunity.org.
- **Platforms.** macOS supported; Linux best-effort until it is tested; Windows planned (its stubs
  stay, marked not yet supported).

**Evidence.** `.superpowers/sdd/2026-10-02-oss/` (local): r1–r7 review reports.

---

## D43. **(M2)** Harness contract v1: owner rulings, and D41's CA rule generalised.

**Decision (maintainer, 2026-10-05, after the harness-contract design and its independent review).**
The harness contract of D42 is `connectors/<id>/harness.toml`, harness-contract schema 1
(`.superpowers/sdd/2026-10-02-oss/harness-contract-v1-design-v2.md`). Eight rulings:
1. **Two files.** `harness.toml` beside a V1-only `manifest.toml`; the V1 file exists only for
   codex-cli and claude-code-cli under `v1-compat`, and Hermes's is deleted.
   (By [D48](#d48-m2-the-v1-path-leaves-the-tree) `v1-compat` is gone; the two V1 manifests
   stayed for parsing only until OS-29 deleted them on 2026-10-12.)
2. **No claim fields in the contract.** `family`, `tier`, `tested_version_range` and `capture_only`
   are not harness-contract schema-1 fields; claim level lives in `docs/qualification/`. The Claude tool allowance
   becomes an id-only check.
3. **Credentials stay a closed list for v0.5** (`DEEPSEEK_API_KEY`). A credential requires a
   per-run config home and `stderr_line` identity; credential with `preassigned`, `none` or
   `stdout_json` identity is refused. Key scanning and identity are separate in the recorder. A
   shape rule plus per-run `--credential-env` consent may follow the tag. (Amended by
   [D49](#d49-m2-admit-amp_api_key-as-a-child-credential) and
   [D50](#d50-m2-amend-d43-ruling-3-for-stdout_json-identity): `AMP_API_KEY` joins the list,
   and a credential may also pair with `stdout_json` + `at_launch` identity.)
4. **Any connector may use a replace-mode CA variable** that is a `CA_REGISTRY` row (today only
   `HERMES_CA_BUNDLE`). A new CA variable in either mode is a DECISIONS entry. D41 is amended below.
5. **Server allowlist as data.** The server keeps acceptable connector directory digests as data,
   keyed by `Launch.connector_dir_sha256`; the compiled pins stay the local gate. Directory pins
   move to `crates/goalcup-protocol/pins/connector-dirs.txt`, so a harness PR has no `.rs` file.
6. **Contributions.** Maintainer team in CODEOWNERS (placeholder `@nembal` until the public repo
   exists). Stranger harness PRs are accepted once the static conformance and pin-drift tests run
   in CI from `goalcup-protocol/tests/`, the code-execution guards are validation rules (startup
   seeds, env values, env deny additions including `GIT_CONFIG_*` and `SHELL`, interpreter
   programs), and the reviewer checklist is extended.
7. **`Launch.run_public_key` and `Launch.connector_dir_sha256`** enter v5 now, optional and
   skip-if-none, so existing traces re-serialize identically. They land before any tag or staged
   bundle. The v5 freeze takes effect at the v0.5.0 tag and covers records, not the producer-to-
   daemon `FrameEvent` seam, which `CONTROL_API_VERSION` governs. A committed reader corpus holds
   the reader side: a verifier change that makes a valid v5 trace invalid needs a DECISIONS row.
8. **An unknown `protocol_hint` is capture-only**, named by `goalcup doctor`, not refused.

Notes on ruling 8 (2026-10-05, before the tag):
- **One key.** A native store names its hint `protocol_hint`, as a host does: one key for the one
  record field, `CaptureMeta.protocol_hint`. It was `hint` for a few days; after the v0.5.0 tag the
  rename would be schema 2. One grammar (`hints::valid_hint`) holds in both schemas.
- **Recorded hints are wider than declared ones.** A declared hint is held to that grammar (a
  lowercase letter first, `[a-z0-9.-]`, at most 64 bytes). The recorded field has no grammar in v5,
  only the text checks, and it freezes at the tag, so the reader can never tighten it without a
  DECISIONS row.

**Amendment to D41, "CA trust"** (replaces its first sentence; the rest stands):
- A replace-mode CA variable is admitted for any connector that declares it in `capture.ca` with
  `mode = "replace"`, provided the variable is a `capture_policy::CA_REGISTRY` row of mode
  Replace. Today the only such row is `HERMES_CA_BUNDLE`.
- It is process-scoped and points at the tracker's proxy CA. It replaces the harness's roots rather
  than extending them, which fails closed: a harness that reads it trusts only the proxy, and the
  proxy verifies the real upstream. A harness that ignores it fails its TLS handshake, visibly.
- The readiness record cites the harness source that reads the variable and shows that tool
  children do not.
- `SSL_CERT_FILE`, `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and `REQUESTS_CA_BUNDLE` stay refused, because
  they replace the roots of every tool child.

**Reconsider when** a second keyed harness needs `preassigned` or `none` identity (widen ruling 3
with a kit case), or a Go or Rust harness reads only `SSL_CERT_FILE` (a new policy row, decided on
its own).

**Evidence.** `.superpowers/sdd/2026-10-02-oss/` (local): `harness-contract-v1-design.md`,
`harness-contract-v1-review.md`, `harness-contract-v1-design-v2.md`, `r6-security.md`,
`server-phase-brief.md`.

---

## D44. **(M2)** OpenCode's native leg is a read-only snapshot of its own session's rows; the public repo is `goal-cup-tracer`.

**Decision (maintainer, 2026-10-05).**
- **OpenCode native leg (option A).** OpenCode keeps every session in one shared SQLite file. When
  OpenCode is added, the recorder opens that database read-only after the run and records only the
  rows of the run's own session, never other sessions or credential tables, and never writes to it.
  This is a new native store form, so it lands as its own core PR under D43 ruling 1 (a new
  capability is not part of a harness PR). Rejected: running `opencode export` after the run (it
  relaunches the harness and records a re-telling, not the harness's own store). Residual to state
  in CHEAT-CHECKLIST: rows edited or deleted in place during the run are not witnessed.
- **Public repository.** The open-source copy goes to `agentcommunity/goal-cup-tracer` (created
  private and empty on 2026-10-05), filled from `scripts/oss-cut.py` (deleted 2026-10-13 once
  its job was done) with no history; the existing private `agentcommunity/goal-cup` keeps the
  full history. Outside PRs require DCO sign-off,
  enforced by the GitHub DCO app (amended by [D47](#d47-m2-no-dco-sign-off): neither applies).

---

## D45. **(M2)** The integrity exit judges the record, not the run.

**Decision (maintainer, 2026-10-05).** `goalcup-verify --capture-integrity` asks whether the record
can be trusted. `Interrupted`, `DecodeError` and `Unresolved` alone do not lower exit 0 and appear
in the research detail. The bounded capture-only inventory exists only in the library
`DecodeOutcome::undecoded`; research-report schema 1 does not serialize it (H-27). Independent
accompanying loss can still lower the result. This is pinned by the v5 reader corpus and holds
after the v0.5.0 freeze.
- **A stream that ends `Interrupted` or `DecodeError`** (network drop, provider error mid-reply,
  Ctrl-C, or transport decoding failure). Already-emitted redacted capture bytes remain chained;
  coded bodies may expose only a decoded prefix, and the faithful early end is recorded.
- **A stream in `DecodeOutcome::undecoded`.** No decoder accepted its full `StreamOpen` (source,
  protocol hint and content type). Its retained capture bytes remain available to a future decoder;
  this is separate from a `DecodeError` end and is not serialized by research-report schema 1.
- **A termination recorded `Unresolved`.** It also occurs on normal runs inside the documented
  window (TERMINATION.md). It remains a termination fact, not an integrity failure, and nothing
  after the seal can change the sealed chain.

Whether any of these should count against a participant is a scoring rule for the server phase,
not an integrity verdict. Open for the server phase: use the run's final pushed Git commit (for
example a GitHub commit hash) as an extra "last known state" marker when a stop cannot be
confirmed; which branch counts is unresolved (LEDGER H-26).

---

## D46. **(M2)** Capture creation binds both local halves to identical connector bytes.

**Decision (maintainer, 2026-10-06).** Control API v5 makes the lowercase 64-hex
`connector_dir_sha256` required on `CaptureWorkspaceRequest`. The CLI sends the digest returned by
the one schema-1 connector load that also supplies its launch profile. The daemon loads its own
authoritative connector directory, pin-checks it, and compares the two digests immediately after
that load and before entry allocation or any capture write. A mismatch is the path-free
`preflight_failed` refusal. The create response does not echo the digest.

This changes `CONTROL_API_VERSION` from 4 to 5 only. It does not change the frozen v5 chain,
`M2_PROTOCOL_VERSION`, or the meaning of `Launch.connector_dir_sha256`: a successful create keeps
the daemon's authoritative digest on the entry and the daemon writes that value at launch.

---

## D47. **(M2)** No DCO sign-off.

**Decision (maintainer, 2026-10-11).** Amends the DCO clauses of D42 and D44. Goal Cup
contributions are mostly written with coding agents, so a per-commit `Signed-off-by` either
certifies a diff nobody read or fails a check the agent forgot. Contributions are accepted under
Apache-2.0 section 5. The person who opens a PR states in it that they read the whole diff and
discloses AI assistance; code-owner review on `main` stays. The `main` ruleset requires `check`
only and the DCO App is not installed. A `Signed-off-by` line is allowed but not checked.

## D48. **(M2)** The V1 path leaves the tree.

**Decision (owner, Balazs Nemethi, 2026-10-11).** Replaces D42's V1-path clause. The V1/v4
transcript-and-hook path that compiled only with the `v1-compat` cargo feature is deleted, with
its upload, receipt, acknowledgement and sign-in pieces. D42 kept those pieces for a later server
phase; git history and the `v0.5.0` tag preserve them, so a server phase starts from there and
not from dead code in the tree.

- **Gone.** The `v1-compat` features of `goalcup-cli`, `goalcup-daemon` and `goalcup-devserver`;
  every item they gated; the V1-only tests (`real_daemon_end_to_end` among them) and their
  support files; the optional `open`, `rusqlite` and `age` dependencies; the second gate block in
  README and CONTRIBUTING; the `v1-compat` checkbox in the PR template.
- **Kept.** Default-build behaviour. The hidden `auth`, `ui` and `resume` commands and the V1
  control routes still refuse, now naming D48 (all deleted on 2026-10-12, with the stub
  routes' client and mock methods). The control API types, the V1 `manifest.toml` parsing (removed 2026-10-12, OS-29) and the verifier's V1
  checks are unchanged. The V1 config fields `upload_url`, `server_public_key` and
  `receipt_binding` drive no upload but are still parsed, because `DaemonConfig` refuses unknown
  fields and would reject a config that sets them. The V1-only registry functions this entry first
  listed here (`open_resume_process_handoff`, `begin_sealing`, `restore_unfinished_sealing`,
  `in_harness_home`, `remove_if_unstarted`, `start_handoff_complete`) were removed under OS-31,
  with the other V1 items no production code reached.
- **`goalcup-devserver` stayed** at first. No shipped binary linked it and, after OS-31, no crate
  depended on it. Deleted on 2026-10-12, together with the protocol `chunk` module it alone used.

**Evidence.** LEDGER OS-30; the pull request that lands it carries the gate results.

---

## D49. **(M2)** Admit `AMP_API_KEY` as a child credential.

**Decision (owner, Balazs Nemethi, 2026-10-12; drafted 2026-10-11).** `AMP_API_KEY` joins
`capture_policy::CREDENTIAL_ENV` beside `DEEPSEEK_API_KEY`. This amends D43 ruling 3's closed list;
the list stays closed. The token is handed to the harness child only, through the existing
child-secret seam, under the shared grammar of 20 to 256 bytes of `[A-Za-z0-9_-]`. It is never a
manifest value: under `capture.env` the name is refused. D43's pairing rules are unchanged (a
credential needs a per-run config home and an identity pair D43 as amended by D50 admits). D41's
key-fragment scan applies to the token unchanged. This entry admits a name only. It admits no Amp
connector, and the owner rules separately on Amp's on-disk key fingerprint
(`.legacy-state-owner.json`).

**Reconsider when** the owner's live probe shows a token outside the shared grammar, or Amp
renames the variable.

**Evidence.** `docs/qualification/m2-amp-readiness.md` (branch `claude/v060-amp-readiness`,
merged as PR #3, `8ee9eb3`; static: Amp's help text names `AMP_API_KEY` as an access token that
takes precedence over stored credentials); [amp-core-prep](research/amp-core-prep.md), change 2.

---

## D50. **(M2)** Amend D43 ruling 3 for `stdout_json` identity.

**Decision (owner, Balazs Nemethi, 2026-10-12; drafted 2026-10-11).** Approved as part of
preparing the core for Amp's own agent (Route A in `docs/qualification/m2-amp-readiness.md`),
whose documented identity carrier is a stdout `init` event. Taken as drafted; the wider
alternative, a credential with any identity method, was not taken.

Until now D43 ruling 3 refused a credential with `stdout_json` identity and kept key scanning and
identity separate. D43 ruling 3's pairing sentence becomes: a credential requires a per-run config
home and either `stderr_line` + `deferred` or `stdout_json` + `at_launch` identity; `preassigned`
and `none` stay refused.
- **One stdout reader.** For a keyed `stdout_json` profile one reader of the child's stdout feeds
  both the key-fragment scan and the identity matcher. The identity is read as bytes arrive,
  before the scan decides whether to forward them. Output holding a fragment is withheld, sets
  the live key hit and fails the row, as D41 already requires. Key scanning and identity share
  that one reader on stdout instead of being separate.
- **A key-bearing identity is not bound.** The identity matcher holds its own key-fragment
  matcher. When the matching event's line or its decoded id holds any key-scan form, no identity
  is bound (the same unbound path as an unusable id) and a key hit is recorded. The bound id is
  written into the chain as the scope, so it can never carry a fragment of the key. Decoded
  (JSON `\u`) forms are checked for the bound id only; forwarded output is matched as raw bytes.
- **The same holds for a keyed `stderr_line` marker** (final review, 2026-10-12). A marker line
  that holds any key-scan form is not bound, whatever marker follows, and is a key hit.
- The per-run config home stays required, so the run-directory scan covers the config home.

No v5 record, control API or verifier change.

**Reconsider when** a keyed harness needs `preassigned` or `none` identity (as D43 already says).

**Evidence.** `crates/goalcup-cli/tests/credential_stdout_identity.rs` and
`capture_stderr_tee.rs` (any Unix); kit case
`synthetic-keyed-stdout-json` (macOS); [amp-core-prep](research/amp-core-prep.md), change 4.

---

## D51. **(M2)** A connector's argv and env are data; review is the control.

**Decision (drafted 2026-10-14 for the owner; taken when its PR merges).** The per-connector
rows `CAPTURE_ARG_ALLOWLIST` and `CAPTURE_ENV_ALLOWLIST` (review R7 I-1, I-2) leave
`capture_policy.rs`. A pinned connector's `capture.args`, `goal_flag` and `capture.env` are held
to generic rules that bind every connector, and a new connector, a model bump or a new variable
is a data-only PR. This amends D43 ruling 5's "a harness PR has no `.rs` file", which the
allowlists had made false.
- **Why it is safe.** At run time only pinned bytes load, and the pins are compiled in, so a
  changed argv or env already needs a merged PR. `connectors/` and `pins/` are code-owned and the
  `main` ruleset requires a code owner's approval, so an outside PR is a reviewed diff. The
  maintainer's own PRs may use the temporary single-maintainer bypass
  (`docs/REPOSITORY-GOVERNANCE.md`) and then have no second reviewer; the allowlists did not
  change that, since the maintainer wrote their rows too.
- **What became a rule.** The R7 cases each have a rule: in env, `OPENSSL_*`, `*KEYLOG*`, the
  runtime variables that name code to load (`PYTHONBREAKPOINT`, `NODE_REPL_EXTERNAL_MODULE`,
  `PERL5DB` and their siblings, by name, so `PYTHONUNBUFFERED` and `NODE_ENV` stay data) and
  server names (`*_BASE`, `*_HOST`, `*_HOSTNAME`, `*_ADDR`, `*_ADDRESS`, `*_SERVER`,
  `*_ORIGIN`, `*_REGISTRY`, beside `*_URL`); in argv, certificate-verification switches
  (including `cafile`, `capath`), `keylog`, `notify`, `command`, `hook`, an API base or host
  without a scheme, and the whole flags `--host`, `--server`, `--url`; and a `goal_flag` with a
  word that says it runs, evaluates or loads its value (`--exec`, `--script`, `--prompt-file`;
  `--execute` is a word of its own and passes). How much the harness may do (model, sandbox,
  permissions, directories) is not a capture rule and has none (owner direction, 2026-10-14).
- **What only review sees.** A dropped flag that capture needs (one headless turn, the session
  file kept) and a value that re-enables self-update (`DISABLE_AUTOUPDATER = "0"`) are
  harness-specific facts.
  They are on the reviewer checklist (`docs/CONNECTOR-CONTRIBUTING.md`, step 3), and the
  loader tests that asserted them under the allowlist were narrowed to the rule cases.
- **What stays closed, as security boundaries.** `CREDENTIAL_ENV` names the variables the
  launcher copies from the participant's shell into the harness, so a new name decides which
  of their secrets a connector may receive. `CA_REGISTRY` is a trust statement per variable
  (D41, D43 ruling 4). `EXTRA_PATTERN_SOURCES` mirrors recognizers compiled into the signed
  daemon. Each stays a core PR with a DECISIONS entry.

**Reconsider when** a connector PR is merged without code-owner review (then the rows come
back as data beside `pins/connector-dirs.txt`, not Rust).

**Evidence.** `crates/goalcup-protocol/tests/it/harness_loader.rs`
(`a_pinned_connector_needs_no_rust_row_for_its_argv_or_env`) and `harness_schema.rs`
(`a_new_connector_needs_no_rust`, `argv_that_loosens_the_run_is_refused_by_rule`).

---

## D52. **(M2)** The shipped connectors set no model, effort, sandbox, permission or MCP limit.

**Decision (owner direction relayed 2026-10-14; taken when its PR merges).** A participant's
harness runs as the participant would run it: model free, network on, nothing limited. A shipped
connector keeps only what capture needs: one headless turn (the launch mode, decided
separately), the isolated config home, the proxy and CA variables, the session file, and
self-update off (the binary is pinned). So Codex drops `-m`, `model_reasoning_effort` and
`-s workspace-write` for `-s danger-full-access` (`codex exec` alone is read-only); Claude Code
drops `--model`, `--effort`, `--strict-mcp-config`, the empty `--mcp-config` and the two MCP
variables, and uses `--permission-mode bypassPermissions` (nobody is there to answer a prompt);
Hermes drops `--model` and `agent.reasoning_effort`, and keeps `--provider deepseek` because
`DEEPSEEK_API_KEY` is its one admitted credential.
- **Kept, and why.** Codex `approval_policy="never"` (nobody answers), the per-process
  `projects` trust override (measured to stop a `config.toml` write) and `--skip-git-repo-check`
  (a non-git workspace); `check_for_update_on_startup=false` and Claude's `DISABLE_AUTOUPDATER`
  (the binary is pinned); Claude's `CLAUDE_CODE_RETRY_WATCHDOG` (a network blip does not end a
  long run); Hermes's compression and title generation off (one session id), `tirith` off (secret
  review I-2) and the model catalogue off.
- **"Model free" means the vendor default, not the participant's choice.** The launch env is
  allowlisted and argv is the connector's, so a participant cannot pick a model; Hermes's default
  is `deepseek-v4-pro`. A supported model choice is an open owner decision. Live tests now pay
  default-model prices (D38's and D41's cheapest-model clauses are superseded here).
- **Consequences.** A tool command runs with the participant's full access and network. It can
  reach the tracer's own control file, its session file, the shared config homes later runs load,
  and the real `~/.codex` and `~/.claude`; traffic to an unlisted host is tunnelled and noted, and
  a tool that ignores `HTTPS_PROXY` is not seen at all (`docs/CHEAT-CHECKLIST.md`, `docs/RULES.md`).
  The Claude tool allowance (`--capture-claude-allowed-tool`) needed `acceptEdits` and was refused
  for the shipped profile; the flag, its launch splice and the daemon's argv check were removed.
  The v5 `Launch` fields `allowed_tool_rule` and `process_handoff_digest` stay readable (and are
  now always written empty) so traces that carry them still verify. None of the three
  default-model launches is measured yet.

**Reconsider when** a measured run shows a default that breaks capture (a session file not
written, a second session id).

---

## D53. **(M2)** Record the harness build in v5, accepting that v0.5.0 readers refuse it.

**Decision (owner direction relayed by the coordinator, 2026-10-14).** The tracer records which
harness build ran instead of refusing builds it has not pinned. `Launch` gains one optional,
skip-if-none field, `harness_build`: `program_sha256` (the program file the launcher resolved and
registered; for a script or wrapper the only digest that names the harness), `image_sha256` (the
running process's executable image, read by the daemon at the launch post, which follows `exec`:
`/proc/<pid>/exe` on Linux, the path `proc_pidpath` names on macOS) and `platform`
(`<os>-<arch>`). The daemon takes both digests itself, outside the run lock, only while the pid is
the registered instance; a digest it cannot take is absent, never guessed.

**The trade, plainly.** `v0.5.0` is tagged and froze the v5 records (D43 ruling 7). Every record
refuses unknown fields, so the released v0.5.0 verifier, and any server reader built from it,
reports `record_malformed` on a trace that carries `harness_build`. This row accepts that instead
of opening protocol v6: nobody depends on a v0.5.0 verifier before the event, and the tracer and
the server ship their own. Traces without the field stay valid for every reader, old and new.

**What it is not.** The reader checks only the field's shape (canonical hex, bounded platform);
it never compares a digest with a list. Matching against known builds, and the "unlisted" note
for judges, belong to the server. Removing the pin refusal is the next PR; the recording lands
first so the tree never has neither.

**Reconsider when** the server's build list exists, or a second post-tag field is wanted (then
open v6 rather than another exception).

---

## Later list (explicitly not v1)

- Continuous workspace Merkle hashing (adds "when" to unexplained file changes).
- Scheduled time reminders or auto-`keep going` injected by the tracker.
- Sub-harness subagents with cross-harness linking.
- Native tray or menu-bar indicator over the status API (the browser status page is v1, D25).
- Rekor anchoring of the final seal.
- Desktop byte-exact capture is V1.1 work wherever a promoted surface needs it.
- **(M0, reclassified by D29)** Codex Desktop capture and bounded partitioning are V1.1 requirements; the measured four-copy amplification remains a volume-test input.
- **(M0)** A protobuf reader for Cursor's `store.db` blobs. About a quarter decode as plain JSON today; Cursor is not a V1 foundation surface.
- **(M0)** The dsh connector, and the spike that has to precede it (D23).
