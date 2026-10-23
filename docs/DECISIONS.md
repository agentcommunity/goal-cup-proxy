# Decisions

Current rules and historical decisions. Code and docs cite them as `D<number>`. A number never
changes and is never reused. Add or change an entry only when a rule changes. Entries for rules
that no code or doc cites any more are removed; git history keeps them. An entry marked
superseded or retired is retained to explain history and references; follow the named replacement.

Entries are short on purpose. The code and its tests are the detail. D1 to D26 are older decisions
retained for references; some have been superseded or retired.

## Status

Where each entry stands on 22 Oct 2026. "Current" means follow it as written. "Partly superseded"
means some clauses are replaced; the entry names which. Read the replacing entry for those clauses.
Update this table in the same PR as any entry it lists.

| D | Status | Replaced by |
|---|---|---|
| D1 to D5 | current | |
| D7 | superseded | D37, D57 |
| D9, D10, D11 | current | D11's clone case is D56 |
| D12 | superseded | D53 |
| D14, D16, D22 | current | D14's rule detail is D36 |
| D23 | retired | `connectors/` and RULES |
| D24 | current | |
| D25 | partly superseded | D66 (the agent may ask for the time left) |
| D26, D30, D36, D37 | current | |
| D38 | partly superseded | D52 (the cheapest-model clause), D69 (the agent's git login) |
| D39 | partly superseded | D57 (chat mode; the headless launch clause is removed) |
| D40 | partly superseded | D61 (the Hermes clause), D56 (the 180 s budget) |
| D41 | partly superseded | D43 (CA rule), D52 (model clause), D61 (seed and key scan) |
| D42 | current | |
| D43 | partly superseded | D61 (ruling 2); ruling 3 amended by WP12 |
| D44 to D48 | current | |
| D49 | superseded | D60, D61 |
| D50 | superseded | D61 |
| D51 to D60 | current | |
| D61 | partly superseded | D62 (model traffic for listed providers only) |
| D62, D63 | current | |
| D64 | partly superseded | D65 (the disk watch pauses, never stops) |
| D65, D66, D67 | current | |
| D69 | current | |

## D1. No API proxy. The harness's own transcript is the evidence.

Goal Cup Proxy never sets or changes a harness's model base URL, in any harness or milestone.
Subscription auth would break, and the harness must talk to its real provider.

A **process-scoped forward proxy** is allowed. It listens on loopback and is reached only through
the harness process's own `HTTPS_PROXY`. It is trusted only through a CA named in that process's
environment. It decrypts only the provider hosts the connector declares and tunnels every other
host. Credentials are stripped before anything is hashed, stored or uploaded (D36). The code that
enforces the no-base-URL rule is `capture_policy.rs`; tests cite D1 for it.

## D2. Two legs per harness: native and wire.

The native leg is the harness's own session files. The wire leg is the recorded model traffic.
Neither leg erases a gap in the other. The older hook leg is gone (D48).

## D3. Record raw, interpret later.

The daemon captures, redacts, hashes and uploads. It parses and judges nothing. Every judgement is
made in the verifier, which is open source and re-runnable. The verifier is its own binary
(`goalcup-verify`); there is no `goalcup verify`. The daemon never links the verifier. This keeps
the signed, hard-to-change daemon small.

## D4. One daemon at a time, run-scoped. No service.

`goalcup start` and `goalcup seal` start `goalcup-daemon --exit-when-idle` when none answers on
`control.json`. It runs detached with an environment built from nothing and exits once no capture
is unfinished. The registry's redb lock decides a start race. Crash recovery is `goalcup seal`
starting a daemon that reopens the entry for status and seal only.

One workspace holds one entry. The exception: a continued run's next segment takes over the
workspace of its last entry once that entry is sealed or the daemon could not reopen it. An entry
whose seal write failed keeps the workspace. Residual: the set of entries that failed to reopen
lives only in that daemon's memory, so a later daemon may reopen the old entry beside the new one.
It is recovered (status and seal only) and routes nothing.

## D5. The agent must never see its own evidence.

Capture is scoped to folders Goal Cup Proxy created. `artifact_root` (the traces) and
`workspace_parent` may not contain each other, so an agent that lists its workspace's parent never
sees the run's evidence. `config.rs` checks this twice: on the spelled paths and on the resolved ones.

## D7. Start flow.

Superseded by D37 and D57. `goalcup start` creates the empty workspace and launches the harness
fresh. The participant types the first prompt in chat mode. The headless launch and its
`--goal-file` flag are removed.

## D9. Gap policy.

Loss is stated, never hidden. A gap is a chained record with a reason. A trace with a gap is
incomplete and never clean. Goal Cup Proxy's job is to keep recording and to say what it lost.

## D10. One harness per entry.

Subagents of the same harness are allowed. Running another harness as a subagent is not: the event
compares harnesses. The rule is checked by reviewing the recorded traffic. Goal Cup Proxy never stops a
second harness and never reads one as an error. A provider hostname or a missing decoder alone does
not prove a second harness: record the unknown identity and say it is unknown.

## D11. The seal records the workspace.

The seal manifest (`seal.json`, digest bound into the chain) lists the workspace at the end. For a
clone of the participant's repository it records a commit and an uncommitted digest instead (D56).

## D12. Versions.

Superseded by D53. Goal Cup Proxy records the harness build that ran and refuses none.

## D14. Redaction happens at capture.

Redaction runs before bytes are hashed or reach disk. Each `Segment` carries redaction evidence
beside the redacted bytes. See D36 for the current two-tier rule.

## D16. The server fixes the clock.

The server's signed start receipt gives `started_at` and `deadline_at`. The chain's seq-0
`Baseline` is the head the receipt signs. There is no separate arming signal: launch plus the first
chained record is the start.

## D22. Kill the process tree, not the launcher.

A connector declares how its harness is ended: `process_group` or `native_pid` (the npm wrapper of
Codex hides the native binary), plus a grace. Seal-time termination follows it. See D30 and
[TERMINATION](TERMINATION.md).

## D23. Harness tiers.

Retired. Which harnesses a build ships is in `connectors/` and in [RULES](RULES.md).

## D24. The control API is authenticated.

Every mutating route needs the daemon's bearer, kept in `control.json` at mode 0600. A hook
receiver no longer exists. The bearer is readable by the participant's own processes, including
the agent: that is inside the stated boundary ([SECURITY](../SECURITY.md)).

## D25. Status is for humans, never agent context.

Feedback on the run's state goes to the participant through `goalcup status` and the signed status
document. Nothing is injected into the agent.

*Narrowed by D66: the agent may ask `goalcup time-left`; nothing is pushed to it.*

## D26. Token usage is derived by the verifier, not enforced.

The verifier may sum usage counts. No rule sets a token budget. Plan, credit and billing fields
must be stripped before anything is published (open: [TODO](../TODO.md#known-issues), H-28).

## D30. `Confirmed` termination is bounded to the owned process set.

`Confirmed` means: a process-table sweep after signalling found no member of the owned set alive,
confirmed by a second sweep. The owned set is the root and what the daemon could reach by walking
its descendants. A process that left the walk before the first snapshot is outside it. The limit is
stated, not hidden. Full contract and per-platform windows: [TERMINATION](TERMINATION.md).

## D36. Two-tier redaction on the wire leg, and a harness-agnostic recorder.

1. The harness's credential is never stored. The recording proxy strips credential headers by name and never
   decrypts sign-in or token-refresh hosts.
2. Body content is redacted best effort, locally, before disk. A participant's own file contents
   and tool results go to a private trace. Raw traces are never published.
3. Decoded is raw. Transport compression is undone on the tee, and the decoded body is what is
   redacted, hashed and stored. A body that cannot be decoded is stored as it arrived and gapped.
4. The recorder is harness-agnostic. The recording proxy does transport work only: terminate declared hosts,
   undo encodings, strip credentials. It never parses a message format. Decoders live in the
   verifier and are per provider protocol, not per harness. A missing decoder does not stop capture.

## D37. Autonomous runs, research evidence and open contributions.

1. The experiment is an agent working without human help. Goal Cup Proxy never supplies continuation,
   encouragement or input. A harness's own autonomous continuation is allowed when set before
   launch and recorded. Human help ends the unassisted interval; capture continues, labelled.
2. Research traces are a primary output. Local capture does transport, scoping, privacy,
   persistence and health. Semantic interpretation runs later in versioned decoders.
3. Real recorder disk capacity is required before launch, not a free-space check (see D40).
4. New harnesses are repeatable contributions: a capture profile (data) and, separately, a
   decoder. A missing decoder never prevents capture.
5. Loopback ingest is unauthenticated toward the receiver beyond a per-entry secret. A single-user
   host is assumed. A second local user is outside the threat model.

A distinct v5 `LocalFinish` binds the seal and the typed process outcome, so a local-only run
does not fake a signed deadline. A torn final line is kept and is never repaired or promoted.

## D38. **(M2)** Fresh per-run HOME for Claude Code; cheapest models for live tests.

Claude Code runs with a fresh per-run `HOME` whose only entry is a link to the user's
`Library/Keychains` (subscription login) and `CLAUDE_CONFIG_DIR` pointing at the isolated config
home. This is the only recipe measured with zero app-wide writes. The agent does not see the
user's dotfiles; add a scoped identity on purpose if a workload needs git or ssh. The
cheapest-model clause is superseded by D52.

## D39. **(M2)** Headless capture launches, ported extra redaction patterns, shared isolated Codex home.

- A connector's `[redaction].extra_patterns` apply on the capture path, with the same fail-closed
  bounded hold as the built-in rules.
- `capture.args` is the full launch template; the goal is one trailing argument. It is checked
  against capture-specific deny rules (D51).
- Codex uses one shared isolated `CODEX_HOME` per connector, never a per-entry copy. Runs on it
  are serialized by a lock, particularly for token refresh.
- The shared home keeps its exact leaf alias in the daemon request, the record and the child
  environment. The CLI checks and rechecks the alias and target identity, owner and `0700` mode
  before create. The daemon refuses every loaded connector's `real_dirs`. ACLs and a same-UID
  replacement after the last check are stated residuals.

## D40. **(M2)** Finish keeps the termination grace; the spawn window is closed; reservation release waits; Hermes uses DeepSeek.

- **Termination bound.** Finish keeps SIGTERM, the connector's grace, then SIGKILL. The capture
  budget never cuts the grace short. Inside the capture budget a drain share and a named commit
  reserve are split, so a producer that never settles still seals with a gap. The budget is 180 s
  (D56); the reserve is 2 s. The daemon terminates under the entry's run lock before draining
  ingest or stopping the native owner, keeping both capture legs live during termination.
  Both capture budget boundaries move by actual termination time. This amends C6's former
  drain-first order: a manual seal must let the launcher observe harness exit and close its
  proxy producer, rather than spend the drain share waiting on a producer whose harness is live.
  Stalled producers still expire with explicit loss; D22 and D30's authority limits are unchanged.
- **Spawn gate.** A capture launch must not leave a live harness that Goal Cup Proxy never durably
  recorded. The child waits before `exec` until the daemon has registered it.
- **Disk reservation.** One 3 GiB reserve file for each run (`RUN_RESERVE_BYTES`), held by the run host, reused all week
  and released after the last upload. A chain is quota-only (`CHAIN_QUOTA_BYTES`, 1 TiB logical).
  It preallocates nothing, so no grow-then-shrink release exists to fail.
- **Hermes** uses its `deepseek` provider with `DEEPSEEK_API_KEY` passed to the child only. No
  DeepSeek base URL is set.

*Superseded in part by D61: no key is passed, and Hermes uses the provider the participant set up
in its home.*

## D41. **(M2)** Hermes reference row: model, CA trust, key-fingerprint and key-fragment rules.

- **CA trust.** `HERMES_CA_BUNDLE` replaces Hermes's roots with the recording proxy's CA. That fails
  closed: Hermes trusts only the recording proxy, and the recording proxy verifies the real upstream. `SSL_CERT_FILE`
  stays refused. D43 generalises this to any connector.
- **Key fingerprint.** The private `auth.json` is pre-seeded so that Hermes never seeds its
  credential pool from the env key or writes `secret_fingerprint`.
- **Key fragments.** Before any Hermes run artifact is kept, a scan with the in-memory key (never
  stored) looks for its prefix (`key[:8]`), suffix (`key[-8:]`) and masked forms. A hit fails the
  row and the run folder stays local. Masked-key forms are also redacted on the wire (`masked_key`,
  a soft rule that only adds redaction).
- **Isolation.** Every Hermes call, version probes included, uses a private `HOME` and
  `HERMES_HOME`. The model clause is superseded by D52.

*Superseded in part by D61: Hermes's home is the participant's own and is not seeded, and
Goal Cup Proxy holds no key, so no run is scanned for one. The `masked_key` rule stays.*

## D42. Open source: data-only harnesses, open harness PRs.

A harness is data: a connector manifest under a `schema_version`. Common cases need no Rust edit.
A genuinely new capability (a new native-store form, a new credential) is a separate reviewed core
PR. Anyone may send a harness PR; connectors are trusted code, so a code owner reviews each one.
The public repository is `agentcommunity/goal-cup-tracer`, Apache-2.0, copyright AgentCommunity.
The protocol freezes at v5 for v0.x. macOS is the measured platform, Linux next, Windows later.

## D43. Harness contract v1: owner rulings, and D41's CA rule generalised.

The contract is `connectors/<id>/harness.toml`, schema 1.

1. One file per connector. No claim fields (`family`, `tier`, `tested_version_range`) exist.
2. Credentials are a closed list (`CREDENTIAL_ENV`): `DEEPSEEK_API_KEY`, `AMP_API_KEY` (D49). A
   credential needs a per-run config home and either `stderr_line` + `deferred` or
   `stdout_json` + `at_launch` identity (D50). `preassigned` and `none` stay refused.
   *Superseded in part by D61: the list is gone, and `[capture.credential]` is refused.*
3. *(Amended by WP12.)* A CA variable's name is connector data, held to a closed shape: a name
   ending `_CA_CERTIFICATE`, `_CA_CERTIFICATES`, `_CA_CERT`, `_CA_CERTS`, `_CA_BUNDLE` or
   `_CA_FILE`, under the shared env-name rules, or one of the two shared rows `SSL_CERT_FILE` and
   `REQUESTS_CA_BUNDLE`. A harness's own variable is handed the recording proxy CA alone, in either mode;
   a replace-mode one means the harness trusts only the recording proxy, which fails closed (D41). The
   shared rows are read by every tool child, so they are handed the system's roots plus the recording proxy
   CA, built for each run from the OS trust store, and take `mode = "extend"` only. That gives up
   failing closed for that harness (CHEAT-CHECKLIST), so a connector names a shared row only when
   the harness has no CA variable of its own. `SSL_CERT_DIR`, `CURL_CA_BUNDLE` and
   `NODE_TLS_REJECT_UNAUTHORIZED` stay refused. A new mode, shared row or name shape is a
   DECISIONS entry. This narrows D41's 'SSL_CERT_FILE stays refused' to connectors that have a CA
   variable of their own; D41's proxy-only trust for a replace-mode variable is unchanged.
4. The server keeps acceptable connector directory digests as data. The compiled pins in
   `pins/connector-dirs.txt` stay the local gate. A harness PR has no `.rs` file (D51).
5. Stranger harness PRs are accepted once the conformance and pin tests run in CI, the
   code-execution guards are validation rules, and the reviewer checklist covers the rest.
6. `Launch.run_public_key` and `Launch.connector_dir_sha256` are optional, skip-if-none fields in
   v5. The v5 freeze covers records, not the producer-to-daemon `FrameEvent`, which
   `CONTROL_API_VERSION` governs. A verifier change that makes a valid v5 trace invalid needs a
   DECISIONS entry (the reader corpus guards this).
7. An unknown `protocol_hint` is capture-only: `goalcup doctor` names it, and nothing is refused.
   A native store names its hint `protocol_hint`, like a host. A declared hint is a lowercase
   letter first, then `[a-z0-9.-]`, at most 64 bytes. The recorded field has no grammar and the
   reader may never tighten it.

## D44. OpenCode's native leg is a read-only snapshot of its own session's rows.

When OpenCode is added, the recorder opens its shared SQLite file read-only after the run and
records only the run's own session rows. That is a new native-store form and so a separate core PR
(D43). Rows edited in place during the run are not witnessed.

## D45. The integrity exit judges the record, not the run.

`goalcup-verify --capture-integrity` asks whether the record can be trusted. A stream that ends
`Interrupted` or `DecodeError`, a stream no decoder accepts, and a termination recorded
`Unresolved` do not lower exit 0. They appear in the research detail. Independent loss still
lowers it. Whether any of this counts against a participant is a scoring rule for the server, not
an integrity verdict. The v5 reader corpus pins this.

## D46. Capture creation binds both local halves to identical connector bytes.

`connector_dir_sha256` is required on the create request. The CLI sends the digest of the one
connector load that also gives its launch profile. The daemon loads and pin-checks its own copy and
compares before it allocates anything. A mismatch is the path-free `preflight_failed` refusal.

## D47. No DCO sign-off.

Contributions are accepted under Apache-2.0 section 5. The person who opens a PR says they read the
whole diff and discloses AI assistance. Code-owner review on `main` stays. A `Signed-off-by` line
is allowed and not checked.

## D48. The V1 path leaves the tree.

The V1 transcript-and-hook product path, its `v1-compat` feature, its upload, receipt and sign-in
code, `goalcup-devserver` and the V1 `manifest.toml` are deleted. Git history keeps them. The
daemon refuses a registry whose V1 row still owes work. The daemon config first kept parsing
`upload_url`, `server_public_key` and `receipt_binding`, and they drove nothing. Those keys and
the V1 cadence keys were deleted before release, and `DaemonConfig` refuses them as unknown fields.

## D49. **(M2)** Admit `AMP_API_KEY` as a child credential. *(Superseded by D61/D60.)*

`AMP_API_KEY` joins `CREDENTIAL_ENV`. It goes to the harness child only, through the child-secret
seam, under the shared grammar (20 to 256 bytes of `[A-Za-z0-9_-]`). It is never a manifest value.
D41's key-fragment scan applies. This admits a name only; it admits no Amp connector.

*Superseded by D61/D60: the child-credential list and seam are gone (D61). The one secret
Goal Cup Proxy hands a harness is the recording proxy's credential (D60).*

## D50. **(M2)** Amend D43 ruling 3 for `stdout_json` identity. *(Superseded by D61.)*

A credential may pair with `stdout_json` + `at_launch` identity. One reader of the child's stdout
feeds both the key-fragment scan and the identity matcher, and the identity is read as bytes
arrive. Output holding a fragment is withheld and fails the row. A key-bearing identity line (or a
key-bearing `stderr_line` marker) binds no identity. The per-run config home stays required.

*Superseded by D61: no connector passes a credential, so there is no key scan. A `stdout_json`
identity is read by its own stdout tee.*

## D51. A connector's argv and env are data; review is the control.

A pinned connector's `capture.args`, `goal_flag` and `capture.env` are held to generic rules, not
per-connector allowlist rows. A new connector, a model bump or a new variable is a data-only PR.

- **Rules.** In env: no `OPENSSL_*`, `*KEYLOG*`, code-loading runtime variables (`PYTHONBREAKPOINT`,
  `NODE_REPL_EXTERNAL_MODULE`, `PERL5DB` and siblings) or server names (`*_BASE`, `*_HOST`,
  `*_ADDR`, `*_SERVER`, `*_ORIGIN`, `*_REGISTRY`, `*_URL`). In argv: no certificate-verification
  switches, `keylog`, `notify`, `command`, `hook`, an API base or host without a scheme, and no
  `--host`, `--server`, `--url`. A `goal_flag` may not say it runs, evaluates or loads its value.
  How much the harness may do (model, sandbox, permissions) is not a capture rule.
- **Review sees** what a rule cannot: a dropped flag that capture needs, a value that re-enables
  self-update. These are on the reviewer checklist ([HARNESS-ONBOARDING](HARNESS-ONBOARDING.md)).
- **Closed lists stay core PRs with a DECISIONS entry:** the CA modes and
  CA-name shape (`capture_policy::ca_env_problem`), the shared provider host table
  (`providers.toml`), and the redaction rule forms (`harness::RedactionRule`). The field name or
  header prefix a form takes, and a CA variable's name, are connector data (WP12).

Reconsider if a connector PR is ever merged without code-owner review.

## D52. The shipped connectors set no model, effort, sandbox, permission or MCP limit.

A participant's harness runs as they would run it: model free, network on, nothing limited. A
shipped connector keeps only what capture needs: the isolated config home, the recording proxy and CA
variables, the session file, and self-update off. "Model free" means the vendor default, not a
choice the participant can make through the connector.

A tool command therefore runs with the participant's full access. It can reach Goal Cup Proxy's control
file, its own session file, the shared config homes and the real `~/.codex` and `~/.claude`, and
use a tool that ignores `HTTPS_PROXY`. Traffic to an unlisted host is tunnelled and noted
([CHEAT-CHECKLIST](CHEAT-CHECKLIST.md)). D57 adds that connectors run chat mode and set no
approval or permission option. The v5 `Launch` fields `allowed_tool_rule` and
`process_handoff_digest` stay readable and are written empty. The verifier names no harness for
them: both present and well-formed, or both absent.

## D53. Record the harness build in v5.

`Launch` has one optional field, `harness_build`: `program_sha256` (the program file the launcher
resolved), `image_sha256` (the running executable image, read by the daemon at the launch post) and
`platform`. Goal Cup Proxy pins no build. `[capture.executable]` names only `program` and optionally
`version_args`. `Launch.harness_version` is what the harness printed, or `unreported`. The
connector-directory pin stays.

Trade: v0.5.0 froze v5, and every record denies unknown fields, so a v0.5.0 verifier reports
`record_malformed` for a trace with `harness_build`. Goal Cup Proxy and the server ship their own
verifier. Matching builds against a list belongs to the server. Digests cover the registered pid,
not a native binary a wrapper spawns. Any further post-tag field means v6.

## D54. The protocol carries v5 only.

`goalcup-protocol` and `goalcup-verifier` carry no pre-v5 versions, bodies, writers, Git-provenance
or publication types. v5 bytes and every v5 trace's verdict are frozen. A line declaring version 1
to 4 is `capture_envelope_invalid`; any other non-v5 version is `protocol_version_unsupported`. A
removed body in a v5 line is `record_malformed`.

*Amended 21 Oct 2026:* the signed-mode `cutoff_reached_v1` and `finalization_v1` bodies, their
Git and publisher vocabulary and their test-only writers are removed; no product writer ever
produced them. A v5 chain carrying one is still rejected (exit 2), now as `record_malformed` at
that line. The frozen `signed-receipt-*` corpus bytes stay; only `signed-receipt-ack-unkeyed`
changes, by this rule alone (findings and validated prefix, same exit).

## D55. The capture chain rotates into part files.

The first part is `chain.jsonl`; later parts are `chain.<start>.jsonl`, where `<start>` is the
part's first byte in the whole chain, in 20 decimal digits. Parts are contiguous, and their
concatenation is the single file the same records would make. Records and hashes are unchanged. The
writer closes a part at 256 MiB or 30 minutes, only at a durable frontier. Acknowledgements stay in
one `acks.jsonl`.

Acknowledged parts may be deleted locally. Resume then starts at the first surviving record. The
verifier checks links, terminal and acknowledgements, reports `chain_pruned_locally` (incomplete,
exit 1: verify the server copy) and reports a missing middle part as `chain_part_missing`. The
writer never starts a fresh chain over any part file.

## D56. The seal of a clone records its commit and an uncommitted digest.

A participant works in a clone of their own GitHub repository. The workspace is admitted if it is
the root of its own clone (`<workspace>/.git` is a directory and no work tree lies above).
Otherwise the old rule holds: a directory inside a work tree, or holding a `.git` file, or one that
is not empty, is refused.

- **Manifest.** `SealManifest` gains optional `git` (`commit`, `dirty_sha256`, or
  `{"unreadable": "<why>"}`) and, on the baseline only, `home` (name, size and SHA-256 of each file
  of the connector's config home before the run; bounded to 10 000 files, files over 64 MiB size
  only). With `git`, `files` is empty.
- **A clone is never walked, and always seals.** Whether the workspace is a clone is decided at
  create and read back from `baseline.json`. If Git cannot be read, the manifest says
  `unreadable`, the finish still seals, and the verifier makes the trace incomplete (exit 1,
  `seal_git_unreadable` and similar). An agent that breaks its `.git` can only make its own trace
  incomplete.
- **The digest** covers paths that differ from `HEAD` and untracked paths, sorted, with a content
  hash for each. Only `.gitignore` files hide a file. A clean clone digests to
  `sha256(b"diff\0untracked\0")`. Algorithm: `workspace_git.rs`.
- **No program the agent can configure runs.** The agent writes `.git/config`, `.gitattributes` and
  hooks, so the seal runs `git` with a cleared environment, empty `HOME`, no system or global
  config, no transport, `core.fsmonitor=false`, `core.hooksPath=/dev/null`, and only `rev-parse`,
  `ls-tree`, `ls-files`, `hash-object --no-filters` and `cat-file --batch`. Output is capped at
  64 MiB and one deadline covers every call.
- **Time.** `LOCAL_CAPTURE_FINISH_BUDGET` is 180 s; the commit reserve stays 2 s. For a clone the
  drain share (the daemon's ingest drain and the launcher's proxy stop) ends 30 s before the
  deadline, so Git always gets time. A real Codex run showed the recording proxy stop had not: it waited for
  a connection the harness held open until the reserve, and the Git read ran out of time. The
  Hermes marker wait (CONTRACTS C6 para 3) ends one second before the reserve and keeps no floor.

## D57. Connectors run the harness in chat mode and record every session file since the start.

Shipped connectors open the harness in its normal chat mode in the participant's terminal. The
participant types the prompt. Identity is every session file new or written since the run started,
which is safe because runs on a shared home are serialized.

- **Schema (additive, defaults keep old behaviour).** `[capture] interactive = true` (no
  `goal_flag`, no credential, identity `none`); `[capture] resume_args` (appended on continue, same
  checks as `args`); `[[capture.native]] since_run_start = true`; `[capture.home] skills_dir`.
- **Run start.** The create request may carry `native_since` (Unix seconds). A `since_run_start`
  source admits a file once `max(mtime, ctime)`, truncated to seconds, is at or after it. Change
  time is used because an agent can set mtime back but not ctime. Once admitted, a file is read
  whole from offset 0. Without `native_since` the floor is the entry's create time. A value more
  than 300 s ahead of the daemon's clock is refused.
- **Roots are created.** A missing `{config_home}` native root is created at entry create, mode
  0700, through the no-follow directory capability. A symlink or non-directory refuses the create.
- **Goal stream.** The launcher records the fixed marker `CHAT_MODE_GOAL` as the goal stream's
  bytes; the prompt is the first user message in the session file.
- **Walker bound.** `max_dir_entries` is 8192. Discovery rotates, so every entry is examined within
  `ceil(n / max_dir_entries)` polls. A file kept out for good (past `max_files`, or refused) is a
  `LimitExceeded` gap.

Reconsider when two runs share a config home at once.

## D58. A week of stalls never exhausts the producer IDs.

Every recovered wire epoch spends one fresh producer ID and no ID is reused. `MAX_PRODUCERS` (IDs
per entry, ever) is 65 536. `MAX_LIVE_PRODUCERS` (128) bounds producers live at once: the receiver
refuses a handshake while 128 are unsettled, and the sink answers a fresh epoch `Pending` at the
same count. Goal-stream bounds and the native adapter's retry budget stay at 128. A Recovery
naming a predecessor the chain never recorded supersedes every wire producer of the entry, so
lineage walks stay linear. The recording proxy's `MAX_CONNECTION_FAILURES` (10 in a row) still ends the wire
leg. A verifier built before this entry flags a trace with more than 128 producers.

## D59. `NODE_USE_ENV_PROXY` and a shared run home.

- `capture.env` admits `NODE_USE_ENV_PROXY = "1"` and no other `*_PROXY` name or value. A Node
  harness needs it to use the launch's `HTTPS_PROXY`. It names no endpoint (D1).
- `[capture.home] run_home = "shared"` makes the run `HOME` one empty directory per connector,
  kept across runs, for a harness with no home variable. It follows the shared-home rules: runs
  are serialized (D39); the daemon refuses it at the user's `HOME`, under any `real_dirs`, or
  anywhere but directly under `<config_dir>/capture-homes`; the baseline lists it (D56); a
  `{run_home}` native root with identity `none` needs `since_run_start`; a missing root is created
  at create; a credential is refused. It is refused together with `capture.home.env`. A launcher
  that does not know the value refuses it at start.

## D60. The daemon redacts the run's own secret by exact value.

16 Oct 2026. Goal Cup Proxy hands a harness one secret of its own: the per-run proxy credential in
`HTTPS_PROXY` (H8). It never sees, forwards or stores a participant's provider key (D61); the
harness keeps that in its own home. The launcher makes the credential before the capture create and
passes it to the daemon in `run_secrets` (control API v9), in memory only: never a file, a log,
argv, the registry or the chain. Both redactors add every form of it
(`goalcup_protocol::run_secret::encoded_forms`: the value, every byte percent-encoded or
`\u`-escaped in either case, base64 at all three alignments in both alphabets) as literal rules
after the shared and connector rules, compiled once per run. A form matches with one line break
between any two characters: LF or CR LF, raw or escaped with one or two backslashes, because
`base64` wraps at 76 columns and a tool's output reaches a model inside a JSON string, or JSON
inside a JSON string. Each literal is a block (it absorbs the claims it overlaps), and the stream
holds it from its first byte, so a split across pushes is one value and a cut releases none of it;
the longest wrapped form (10 746 bytes) fits `MAX_HELD_BYTES`. The marker is
`<redacted:run_secret>`. The pattern rule `proxy_credential` stays for a reader without the secret
(`spikes/redact.py`). There is no upload-side check: the daemon is the one place a body is redacted,
and the secret is loopback-only and dead once the run ends.

## D61. Hermes runs in chat mode from its own home; Goal Cup Proxy forwards no key.

16 Oct 2026, owner ruling. Goal Cup Proxy never sees, forwards or stores a participant's provider key.
Hermes runs however the participant set it up, as Codex does (D39, D57).

- **Chat mode.** `hermes chat` opens Hermes's classic REPL (`--tui` is not passed; a
  participant's own `display.interface: tui` writes the same session file). The connector has
  `interactive = true` and `resume_args = ["--continue"]`, measured to resume the most recent
  session. No `-Q`, `goal_flag`, `--provider` or `--model`.
- **Its own home.** `config_home = "shared"` (`<cfg>/capture-homes/hermes-cli`), never seeded:
  the seed files and `[capture.credential]` are gone. `goalcup prep` opens Hermes there. With no
  provider it asks `Run setup now? [Y/n]` and runs its own wizard, which keeps the provider and
  model in `config.yaml` and the key in `.env`. Hermes loads `.env` itself; no launch passes it.
- **Identity.** `none`, with the native source `sessions/session_*.json` and `since_run_start`:
  the only identity an interactive profile takes (D57). The banner prints `Session: <id>` on the
  terminal, which Goal Cup Proxy does not read. Runs on the shared home are serialized, so every
  snapshot new or written since the start is the run's. A `request_dump_*` file is never matched.
- **`capture.configure` (schema 1, additive).** A chat-mode connector may list up to 4 commands
  of its own program (each 1 to 16 arguments of at most 256 bytes, held to the `args` rules, no
  template). `goalcup prep` runs them in the harness home after the participant's setup exits 0;
  `goalcup start` and `goalcup continue` run them again before each launch. They get the harness's
  environment without a proxy or CA variable, stdin `/dev/null`, a process group of their own and
  60 seconds each; their output goes to `<cfg>/prep.log` or the run's `host.log` only. A failure
  leaves the prep step `fail` (the home is not ready) and refuses the start before any launch.
- **What configure may change (carve-out from "Hermes runs as the participant set it up").** Only
  how the harness stores its session: capture and storage settings. Never how the agent behaves:
  not the model, the provider, the tools, the prompts, or whether compression is on. Core refuses a
  configure argument that has `model`, `provider`, `base_url`, `api_key`, `key`, `token` or
  `secret` as a `.`- or `=`-separated part (`CONFIGURE_DENIED_KEYS`, any case); review is the
  control for the rest.
- **Hermes's two commands** (Hermes 0.19.0; neither setting has an environment variable):
  - `config set sessions.write_json_snapshots true` (default off): without it there is no
    session file to record.
  - `config set compression.in_place false` (default on). An in-place compaction keeps the
    session id with fewer messages, and Hermes never rewrites a snapshot that holds more
    (`run_agent.py:2754`), so the file stops changing and the native leg loses the rest of the run
    with no gap. With it off, a compaction starts a new session id
    (`agent/conversation_compression.py:1247`) and so a new `session_<id>.json`
    (`run_agent.py:2723`), which the native source reads from the run's start. Whether and when
    Hermes compacts stays the participant's.
- **What it gives up.** No key, so no key scan (D41) for Hermes. The daemon's shape rules still
  remove a key printed raw or JSON-escaped; a key the agent encodes (`base64`, hex) is not
  matched, because Goal Cup Proxy never knows its value (CHEAT-CHECKLIST). Hermes's own defaults now
  apply where the seed turned them off: title generation (an auxiliary model call), compression
  (each compaction now starts a new snapshot file, above), the `tirith` scanner download and the
  remote model catalogue. Model traffic is recorded for the `providers` rows only (DeepSeek) while
  `HERMES_CA_BUNDLE` replaces Hermes's roots, so another service the participant picks fails its
  TLS handshake until the connector reads the model host from Hermes's own config.
- **Baseline.** The home's `.env` and `auth.json` are listed in the uploaded baseline by name,
  size and SHA-256 only (D56), as Codex's `auth.json` is: their bytes are hashed, never copied.

- **No credential mechanism.** The schema refuses `[capture.credential]` (in the base profile
  and in a platform override) with "a connector may not pass a provider key; the harness keeps
  its own login in its home". The launcher reads no provider key from its own environment and
  hands none to a harness; the child's environment is built from nothing (D36), so a key the
  participant's shell exported never reaches it. With no key in hand there is no key-fragment
  scan (`key-scan.txt`), no secret-aware tee and no keyed-run upload rule: every run uploads
  under the same rules. Redaction of what the harness itself sends (D36, D60, `masked_key`) is
  unchanged.

This supersedes D40's Hermes clause, D41's seed and key-scan clauses, D43 ruling 2, D49 and D50.

## D62. Goal Cup Proxy records the model service the harness's own config names.

16 Oct 2026, owner ruling, for Hermes and every later harness: no provider list, and no tracking
of what a harness offers. The participant picks the model service in the harness; Goal Cup Proxy
records that service and nothing else.

- **The field.** `capture.model_host_from = { file = "<path in config_home>", key = "<dotted
  key>" }`, schema 1, additive. `goalcup start` and each `goalcup continue` read that one value,
  before anything is created. It must be an `https` URL; its host joins the intercepted hosts for
  that segment only (with its port for the start probe), and its path is the base the recorded
  paths sit under (`<base>/chat/completions`, `/messages` and `/responses`, also under
  `<base>/v1`, carry their wire hints; the rest of the base is recorded with none). A host the
  connector lists in `capture.hosts` keeps its reviewed row. With the field, `capture.hosts` may be
  empty. It needs `config_home = "shared"`, the home the participant's own setup writes.
- **Everything else is a tunnel.** Git, package registries, web fetches and update checks pass as
  opaque tunnels, counted in `proxy admission:` and never opened or recorded, as before.
- **The reader.** One regular file (no link, at most 1 MiB), one key. YAML by a small
  block-mapping lookup in `crates/goalcup-cli/src/model_host.rs` (no YAML crate); JSON and TOML by
  `serde_json` and `toml`, which the binary already links. Only `host:port` is ever logged
  (`model service: <host>:<port> (<key> in <file>)`); a userinfo, the path and the query are not.
  The file's name and the key may not name `auth`, `credential(s)`, `secret(s)`, `token(s)`,
  `key(s)`, `api_key` or `env`: Goal Cup Proxy reads an address, never a key file or a key.
- **Refused at start.** A missing value, a non-URL, plain `http` (only `HTTPS_PROXY` is set, so it
  would bypass the recorder), a loopback name, an address literal, a single-label name, a Goal Cup
  host or an authentication host: "`<display name>` is set to use `<scheme>://<host>:<port>`,
  which Goal Cup cannot record: `<reason>`. Goal Cup records only an https model service that is not on
  this computer. Run goalcup prep --harness `<id>`, choose such a service there, then run the
  command again." `goalcup prep` makes the same check: while the value cannot be recorded the
  home is not ready, and prep says why and opens the harness again. What is checked is the
  address the config names, so a run never starts with that address tunnelled unrecorded. It is
  not a check of every host the harness reaches: a login the harness refreshes at another host
  (an OAuth provider's token endpoint) tunnels, and under a replace-mode CA such as Hermes's that
  refresh fails (`docs/harnesses/hermes-cli.md`, known gaps; TODO Known issues).
- **A switch mid-run.** The file is not read during a run. A service switched to inside a session
  (Hermes's `/model`) is a tunnel until the next segment: the participant exits the harness and
  runs `goalcup continue`, which reads the config again (PARTICIPANT-GUIDE, section 8).
- **Hermes.** `{ file = "config.yaml", key = "model.base_url" }`, which Hermes's setup and
  `/model` write. API-key services are what this delivers; an OAuth-login service may fail its
  token refresh (above), pending a test-round run. `providers = ["deepseek"]` is removed: with `HERMES_CA_BUNDLE` in replace mode
  any service the participant picks other than a listed one failed its TLS handshake, and a listed
  one the participant does not use was only probed at start. `[capture.ca]` stays in replace
  mode: the approved host is decrypted by the recording proxy, so Hermes trusts it through the recording proxy CA.
- **The default.** The connector template and HARNESS-ONBOARDING make the field the default for
  any harness whose model service is the participant's choice; the facts sheet asks where the
  harness saves it. `providers.toml` stays for a harness that can reach one fixed service.

This supersedes D61's "model traffic is recorded for the `providers` rows only" for Hermes.


## D63. A signed receipt must describe the Launch it anchors.

17 Oct 2026, Review 13 C-02: the capture reader compares each Launch with its verified receipt.
`connector_id` must equal `profile_id`, and `manifest_sha256` must equal `profile_digest`.
When the receipt signs `connector_dir_sha256`, Launch must carry that same digest. Older receipts
without a directory claim stay valid, including when Launch carries a digest. This binds existing
claims to each other; it does not independently hash a connector, add a pin check to integrity,
change receipt head binding, or change the existing optional run-key rule.

A contradiction is invalid with `receipt_launch_mismatch`, before the Launch reaches research
callbacks. This deliberately tightens the v5 reader under the format-freeze exception in
CONTRACTS: previously accepted contradictory receipt/Launch pairs now fail. The historical corpus
bytes are preserved. `signed-receipt-ack-finalization` and `signed-receipt-key-agrees` both sign
placeholder `11…11` profiles but launch `cc…cc`; their expected verdicts become invalid. The
run-key mismatch case keeps its earlier `record_field_invalid` precedence. Dynamic signed tests
use coherent profile claims and cover matching tuples, each contradiction and legacy absence.
No record schema, canonical bytes, receipt signature domain, server API or local-only verdict
changes.

## D64. No server error stops a run or ends its upload.

21 Oct 2026, owner ruling (event readiness plan, WP-A, invariant I1): "the server should REALLY not
fail; the run should continue and we hotfix the server." Only two things stop a run: a heartbeat
answer that says `stop` (its deadline, or `reason: "replaced"` for a run a newer start replaced),
and the run's own count to its deadline (`LocalDeadline`, WP11). The participant can still end the
agent, and the disk watch still stops it on a nearly full disk with the run left open, as before.

- **Every other answer is asked again.** Any status (`401`, `404`, `410`, `413`, any `5xx`), a
  timeout or no connection: the run keeps recording on the participant's machine, and the
  uploader and the heartbeats ask again with the existing backoff (60 s doubling to 15 min, with
  jitter, never under `Retry-After`). The Worker's error code is not read for this, and a route a
  bad deploy lost answers `404` like any other refusal. So does a local chain that does not
  continue the server's copy, and an acknowledgement that does not verify: the server's position
  is read again before the next try, since a fixed server may say otherwise.
- **The wrap-up** after the stop asks the same way until it runs out (`Policy::wrap_up`); what is
  left is sent later by `goalcup upload`.
- **`goalcup upload`**, run by hand with a person waiting, still gives up on `401`, `404` and `410`
  and on those two disagreements, and says why (`ServerError::ends_manual_upload`).
- **Said, not silent.** Three refusals in a row of one status (a `4xx` other than `429`) on the
  upload's calls or on the heartbeats are said once in the terminal and kept as a `PROBLEM:` line
  in `goalcup status`: the server refuses the run's uploads, recording continues on this computer,
  nothing is lost. An answer that works clears it.
- **Measured.** `host.log` gets one line per stored chunk with its chain bytes and compressed
  bytes; never content or keys.

This supersedes the earlier behaviour, which no decision recorded: a heartbeat answered `401` or
`410` stopped the run ("the server closed this run"), and a `401`, `404` or `410` to any upload
call ended the run's upload for good. A server that must end a run says `stop` in a `200` answer.

*Superseded in part by D65: the disk watch no longer stops a run; it pauses the agent.*

## D65. On a nearly full disk Goal Cup Proxy pauses the agent and resumes it by itself.

21 Oct 2026, owner ruling (event readiness plan, WP-C): when the disk is about to fill, Goal Cup Proxy
pauses the agent so that nothing happens unrecorded, and resumes it by itself when space returns.
This is an exception to the hard rule "Goal Cup Proxy records and never manages the agent"
(`AGENTS.md`); D66 is the other.

- **Freeze, not prompt.** The watch tick (every 30 s) sends `SIGSTOP` to the harness when less
  than 1 GiB is free beyond the run's reservation, and `SIGCONT` once more than 3 GiB is free
  again. Nothing is typed to the agent, nothing is killed, and the participant does nothing but
  free space. The terminal says once: "The disk is almost full, so your agent is paused. Free
  some disk space; it resumes by itself. If your agent seems stuck afterwards, type keep going."
  `goalcup status` keeps it as a `PROBLEM:` line, and the run's `state.json` has
  `disk.disk_paused`, until the agent resumes.
- **Best effort.** The processes paused are the ones the termination code reaches: the root process
  Goal Cup Proxy spawned and its descendants in a `ps` walk (twice, so a child forked during the first
  pass is caught), and the harness's own process group when it has one. A process that left the
  tree (detached, daemonized, or reparented to `goalcup start` by the Linux subreaper) may escape
  and keep running.
- **Never paused at the end.** Before anything ends the harness (the deadline, a newer run
  replacing it, the harness's exit, the end of `goalcup start`) the agent is continued and never
  paused again, because a stopped process does not act on the termination signal. A chat-mode
  harness stopped by this pause is not continued by the host's wait, which still continues a
  harness that suspends itself.
- **The disk stop verdict is removed.** A run no longer stops when the reserve is drawn down, the
  disk is below the floor and nothing was acknowledged for 30 minutes. A run never stops for the
  disk; the pause and, if the disk still fills, the `StorageFull` gap are the record. This
  supersedes the disk-stop clause of D64.
- **Not covered.** A `goalcup seal` typed in another terminal while the agent is paused reaches
  the daemon, not the host's pause: the agent gets the termination signal frozen and is ended at
  the end of its grace. The host continues what is left once the root has exited.

## D66. The agent may ask how much time is left; nothing is pushed to it.

21 Oct 2026, owner ruling: the agent may ask how much time its run has left. Goal Cup Proxy never
pushes anything to the agent. This narrows D25 for one value, and it is the second exception
to the hard rule "Goal Cup Proxy records and never manages the agent" (`AGENTS.md`), after D65:
the agent pulls, and nothing is sent to it.

- **Pull only, the same for every harness.** Every recording launch, whatever the connector, gets
  `GOALCUP_TIME_LEFT_FILE`: a file at `<config dir>/time-left/<run_id>`, in a private (0700) folder
  outside the workspace and the evidence and beside neither the run's credential nor
  `control.json`. No connector names it (`GOALCUP_*` is refused in `capture.env`); there is no
  per-harness code.
- **Only these values.** The watch tick (every 30 s) writes `seconds_left` from the server's count
  kept in `state.json` on the continuous clock, `deadline_at` (the server's value, parsed as a date
  and written again), `as_of` (when it was written), and `practice=1` for a practice run (from
  the run's own record). No other string from the server is written. When the count says nothing
  (after a reboot) the file is removed.
- **The agent asks.** `goalcup time-left` reads the file through the variable and prints one line
  ("About 41 h 12 min left (the run stops at 2026-11-14 00:00 UTC). Push your work often; work not
  pushed when the run stops is lost."; a practice run, whose clone cannot push, gets "This is a
  practice run: its clone cannot push, so there is nothing to push." instead) and exits 0; a
  missing file, or one written more than 5 minutes ago, is "unknown right now" and exit 1. The goal-cup skill tells the agent the command
  exists. The skill reaches only harnesses with a `skills_dir` (Codex, Claude Code); Hermes and Amp
  need a skills folder measured locally before their connectors get one.
- **The same answer as a tool** ([fix plan v9](research/results/2026-10-22-fix-plan-v9.md)).
  `goalcup mcp` is a read-only MCP server on stdin and stdout whose one tool, `time_left`, returns
  what `goalcup time-left` prints, so the agent can ask whatever its `PATH`. The participant adds
  it to the harness's MCP servers (D67: `goalcup prep` prints the lines from the connector's
  `participant.time_left_tool`, and nothing writes them). The skill names the tool first.
- **`goalcup seal` refuses inside the harness.** Every recording launch also sets
  `GOALCUP_IN_HARNESS=1`, and `goalcup seal` refuses under it and names another terminal, so the
  agent cannot end its own segment by accident. A seal from another terminal, and the host's own
  seal at the end of a run, are unchanged.

## D67. Goal Cup recommends running without permission prompts; the participant sets it.

22 Oct 2026, owner ruling: "it should really tell users how to set up their harness, so we can
recommend how to achieve it but they do it, we dont change harness settings". Answering a
permission prompt is typing, and the participant may type only "keep going" (RULES), so a harness
that stops to ask for approval stalls until its deadline (lx-codex and lx-claude, 22 Oct:
[results](research/results/2026-10-22-local-test-day.md),
[plan v4](research/results/2026-10-22-fix-plan-v4.md)).

- **Goal Cup recommends, the participant configures.** Each harness doc
  (`docs/harnesses/<id>.md`, "Run without permission prompts") names the exact setting and where it
  lives. A connector may list those settings as `[[participant.no_prompts]]` (a file in the shared
  config home, a key, the accepted values; read like `capture.model_host_from`, D62). `goalcup
  prep` reads them and, when one is not set, adds one recommendation with the doc's link to the
  harness home step. It never changes the setting and never fails or holds back a step for it.
- **One setup guide for participants** (fix plan v8). [HARNESS-SETUP](HARNESS-SETUP.md) gathers
  every setting a 168-hour run needs, per harness. Prep's recommendation links to the harness's
  section there, named by the connector's `participant.setup_section`, and prep's harness step
  always points to it ("Before a long run, read ...").
- **Goal Cup never writes a participant's harness permission settings.** No connector field,
  configure command or launch option sets them (D52, D57 stand).
- **The test kit's own cells are the organiser's.** `scripts/kit_settle.py` sets them in the kit's
  tracker homes, like the cheapest-model rule: those are not participant homes.

## D69. Goal Cup Proxy holds no keys; the agent pushes with the participant's own git login.

23 Oct 2026, owner ruling ([plan](plans/2026-10-23-simple-setup.md)): "goalcup observes, it
should not store any key and should not pass keys anywhere. The agent can push to any repo, that's
ok." Goal Cup Proxy reads, stores, logs and passes no key, GitHub token included.

- **`goalcup setup`** (once `goalcup prep`, still a hidden alias) has four steps: this computer
  (it runs `goalcup login` when needed), the harness, signing in to it, and this machine. It needs
  no GitHub. It makes `<work folder>/.env.local`, empty but for a comment, mode 0600, when it is
  missing, for keys the participant gives their agent, and never reads it.
- **GitHub at `goalcup start`, for a real run only.** Before the approval, start asks the name of a
  new, empty, public repository (or takes `--repo`), finds the participant's GitHub user from
  their own login (`ssh -T git@github.com`, then `gh api user`; with neither, it prints how to set
  one up and looks again), opens `github.com/new` with the name filled in when it does not exist,
  and checks it: public and empty by GitHub's open API with no login (a rate limit is said and
  passed on: the server checks public at open, the clone checks empty), pushable by a dry-run push
  with the agent's own git, in the environment the launch builds (a scratch `HOME` and the
  variables below), so a login only the participant's shell has fails here and not at the agent's
  first push. Each real run gets its own repository; `prep.json` keeps the last name only to
  suggest the next. A Ctrl-C at any of its questions or checks ends the start with nothing
  created.
- **The clone is the participant's own git.** Over SSH when their SSH login works, otherwise
  HTTPS. No credential helper and no token is written to it. `.env*` goes in its
  `.git/info/exclude`. A practice run clones nothing: its folder is a new local repository with
  no remote, and the server is told the organisers' practice repository as its `repo_url`.
- **The environment is still built from nothing (D1, D36, D38, D61), plus exactly five ways to
  the participant's git login**, as paths or sockets and only when set or present, and never for
  a practice run: `SSH_AUTH_SOCK`; `GIT_CONFIG_GLOBAL`, their own or their `~/.gitconfig` (else
  `~/.config/git/config`; with both, start says the second is not used); `GH_CONFIG_DIR`, their
  own or `~/.config/gh`; and `XDG_RUNTIME_DIR` and `DBUS_SESSION_BUS_ADDRESS`, which a Linux
  keyring helper needs. A token variable such as `GH_TOKEN` is not carried.
- **What it gives up.** The agent can push to any repository the participant's login can (the
  owner accepts this). A `git config --global` the agent runs writes the participant's real git
  config, which `GIT_CONFIG_GLOBAL` names. A push over SSH is not seen by the recording proxy; the judged commit is
  read from GitHub's API, not from recorded traffic. A dry-run push is git's own check of the push
  connection, made with no key of Goal Cup's; whether GitHub refuses it without write access was
  not measured against GitHub here.
- **Old clones.** The hidden `goalcup git-credential` still answers from `github.json` for clones
  made before this, and nothing writes that file any more. It can be removed after the event.
