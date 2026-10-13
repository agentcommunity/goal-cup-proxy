# Scaling to 30 harnesses

How adding and keeping 30 harnesses on four platforms (macOS arm64 and x86_64, Linux x86_64 and
arm64) stays cheap. Most harnesses land before the hack; one or two may land during the 24-hour
start window, under watch. Written 2026-10-14 from review r4 of the harness kit; a plan, not a
record of what is built.

The principle: a harness is data in `connectors/<id>/`, reviewed by a code owner. The tracer's
code knows capture *mechanisms* (a proxy, a session-file reader, an identity carrier, a decoder
family), never a harness by name. A harness rarely changes how it sends tokens or writes its
transcript from one day to the next, so a new build of a known harness should cost nothing: it is
recorded, not pinned.

## 1. Per-connector rows in Rust

D51 (open PR) removes the per-connector argv and env rows from `capture_policy.rs`: a new harness
or a model bump is data, held to generic rules, with review of the pinned directory as the
control. Three closed lists stay as security decisions and grow by a reviewed row now and then:
credential names (which participant secret is handed over), CA variables (trust) and redaction
regexes (code in the signed daemon).

## 2. Harness names in core

Four places still name a harness:

- **The Claude tool allowance** (`--capture-claude-allowed-tool`), checked by connector id in the
  CLI, daemon, verifier and protocol. Since D52 the shipped Claude profile bypasses permission
  prompts, so the allowance cannot apply. Delete the CLI flag and the daemon path; keep the
  protocol field and the reader's check, because v5 traces that carry it stay valid.
- **Native decoders**: the verifier reads Codex and Claude Code transcripts (`Format { Codex,
  ClaudeCode }`); any other harness's session file is recorded but not decoded. Make decoding a
  small declared set of *families* a connector picks by `protocol_hint` (it already does for the
  wire: OpenAI Responses, Anthropic Messages, Chat Completions). A new harness that writes, say,
  Chat-Completions-shaped JSONL picks that family; only a new shape is a verifier PR.
- **`CODEX_SCOPE_WAIT`** (120 s), the wait for a session id on stdout, used by every
  `stdout_json` harness: a connector field with a bounded default.
- **`native_scope.rs`** always forbids `~/.codex`, `~/.claude` and `~/.hermes`, beside the
  manifest's own `real_dirs`, which already does the job. Keep `real_dirs`, drop the three names.
  The same goes for `HARNESS_RESERVED_PREFIXES = ["HERMES_"]`, which can become a per-connector
  "reserved prefix" field or go.

## 3. The same facts typed three times

A connector's home, identity and session-file facts are written in `harness.toml`, again in the
kit case's `harness.toml`, and again in its `expect.toml`; then each connector gets a generated
golden of about 1,150 lines (`hermes-cli.json` is 1,136) that nobody reads in review.

**Changes:** the kit runner loads `connectors/<id>/harness.toml` and replaces only the id
(`kit-<id>`), executable, argv, hosts, credential and native `protocol_hint` with the
stand-in's. `expect.toml` shrinks to what the
stand-in writes (session path, first bytes, wire request). The per-connector golden becomes a
short projection of the fields a reviewer checks, or a digest of the long one.

## 4. Builds: record everything, pin nothing

Owner decision (2026-10-14): `goalcup start` never refuses a harness build. A harness rarely
changes how it sends tokens or writes its transcript between builds, and 30 harnesses × 4
platforms of exact pins, redone on every vendor release, is not maintainable. So the tracer
records which build ran, and judges read that record. What stays pinned and reviewed is the
connector directory: that is what stops a connector smuggling a base URL, a proxy or a command.

- **The daemon hashes what actually runs.** The CLI registers the parked child, releases it, and
  only after `exec` posts the launch (`start.rs`). At that post the daemon hashes the registered
  process's own executable: on Linux through `/proc/<pid>/exe`, which opens the exact file that
  was executed even if its path was replaced; on macOS through `proc_pidpath`, which names the
  path, so a same-user replacement between `exec` and the hash remains a stated residual. The
  `Launch` record gains the digest and the platform as optional skip-if-none v5 fields, the same
  pattern as `connector_dir_sha256`; old traces stay valid. The CLI's own hash is no longer a
  gate, only a launch-time consistency check (the file must not change between registration and
  release).
- **The version is the harness's own word.** A connector may name `version_args` (Codex and
  Claude Code: `--version`); the CLI runs it once with the launch's private environment, keeps
  the first line (bounded), and the `Launch` record's existing `harness_version` carries it. It is
  reported, not proven: only the digest is evidence.
- **A known-builds list is advisory and lives on the server**, matched on (connector, platform,
  digest), the way the server matches `connector_dir_sha256` (D43 ruling 5). A script fills it
  from official releases (npm, GitHub releases, PyPI), recording each source URL and the
  registry's integrity value. "Unlisted" is a note for judges, never a refusal; an offline
  verifier shows only the digest. A build listed later clears the note.
- **What an unlisted build can be, and why recording is enough here.** Anything on `PATH`: a
  wrapper that unsets `HTTPS_PROXY`, or a fake that writes a session file and calls no model.
  An exact pin never stopped a determined participant either (they control the machine and the
  CLI); what catches these is the evidence itself: a wire leg with no model calls, legs that
  disagree, a digest no vendor ever published. The digest makes that review possible.
- **Script harnesses.** A console script or npm wrapper's digest says little about the code it
  loads. Record it anyway; the build list can later add a package-tree digest (a wheel's
  `RECORD`, npm's integrity) where the vendor publishes one.

New connectors during the 24-hour window still need a tracer release, because connector
directories are pinned by digests compiled into the binaries. Moving accepted connector digests
to the server too would remove that, at the cost of trusting a downloaded list; owner decision.

Evidence per harness × platform cannot come from CI, which stays lean. Each harness needs one
measured real run per platform family before the hack, recorded in its facts sheet.

## 5. What cannot be captured yet

Row 0 of the kit (`docs/HARNESS-ONBOARDING.md`) stops these harnesses in minute one. What each
would take:

| Kind | What it would take |
|---|---|
| No local session file (server-side sessions) | A wire-only claim level: schema allows zero native rows, the verifier rates it lower |
| Database store (SQLite, LevelDB) | A read-only snapshot native form for the run's own rows (D44 plans this for OpenCode) |
| HTTP/2-only or gRPC | An `h2` proxy path, plus a decoder for its framing; the largest item here |
| Local or loopback model | A loopback relay the tracer records (below) |
| Desktop or IDE app | V1.1 scope: launch and isolation of an app, not a CLI |
| Subagent transcripts in own files | A `children` pattern beside the one native row |
| Self-update mid-run | Re-hash the binary when the run ends and record a mismatch |
| More than 16 hosts or wildcards | Raise the bound; wildcard hosts need a reviewed suffix rule |

### Local models

A participant may run their own model (Ollama, LM Studio) on loopback; the owner accepts it.
Today the launch exempts loopback from the proxy (`NO_PROXY=127.0.0.1,localhost`), so that
traffic is not recorded, and a pinned connector may not name a loopback host.

The plan: `goalcup start --local-model http://127.0.0.1:11434` names the participant's server
(loopback only). The tracer starts a recording relay on another loopback port, plain HTTP, that
forwards to it and records each request and response as a wire stream, exactly as the proxy does
for a vendor host. The connector points the harness at the relay through a launch token,
`{local_model_url}`, which the tracer renders and a connector cannot spell. This sets a base URL,
and that is fine here: the "never set a model base URL" rule protects vendor endpoints, and no
vendor endpoint is replaced; the harness talks to the participant's own server either way.

The evidence claim is weaker and says so: "local model, self-hosted". The record proves what the
harness sent and received, in order and in time, but no third party witnessed the model call, so
it cannot rule out a "model" that is a person typing through a fake server. The relay also
records the server's own model listing (Ollama's `/api/tags` digests) as reported identity.
Judges weigh it; the verdict names the claim level.

## Order of work

1. Landing now: D51 (argv and env are data), the optional `home.env`, the kit docs with row 0, D52 (no
   model, sandbox or permission limits). The Intel pins of #72 become moot under step 2.
2. Record the build, then stop pinning: the daemon hashes the running executable into `Launch`,
   connectors may name `version_args`; then `executable.sha256` and the per-platform pins and
   their refusal path go, leaving `[capture.platform.<key>]` for env overrides only.
3. No harness names in core: delete the Claude allowance flag and daemon path (keep the v5 field
   and the reader check), make the stdout scope wait a connector field, drop the three
   hard-coded home folders and the `HERMES_` reserved prefix.
4. Declare native decoder families a connector picks by `protocol_hint`.
5. Make the kit case derive from the connector and shrink the per-connector golden.
6. The server-side known-builds list, its refresh script and the "unlisted" note.
7. The local-model relay.
8. Measure one real run per harness and platform family, recorded in its facts sheet.
9. The other row-0 items, by demand: subagent files, database snapshots, wire-only.
