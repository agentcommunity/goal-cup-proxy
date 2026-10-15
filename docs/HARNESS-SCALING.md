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

`openai` and `anthropic` may appear in core when they name an API wire format or a provider
(decoder family ids, secret shapes, provider prefixes); the ban is on harness and product names
(codex, claude-code, hermes, …). `crates/goalcup-protocol/tests/it/no_family_literals.rs`
enforces it on every source file of the protocol, daemon and verifier crates, with each allowed
literal listed by file and reason. The closed security lists in `capture_policy.rs` (CA rows,
refused launch variables such as the `CLAUDE_*` ones) and the decoder evidence notes in
`registry.rs` name harness variables and builds by design, each listed in `ALLOWED`.

One decoder path still names a harness:

- **Native decoders**: the verifier reads two transcript shapes (`Format { Codex, ClaudeCode }`
  in `capture_analysis/native.rs`), chosen by the connector's `protocol_hint` (`codex-jsonl`,
  `claude-code-jsonl`); any other harness's session file is recorded but not decoded. Name them
  as *families* instead, as the wire already does (OpenAI Responses, Anthropic Messages, Chat
  Completions). A new harness that writes, say, Chat-Completions-shaped JSONL picks that family;
  only a new shape is a verifier PR.

Already data or gone:

- **The tool allowance** (`allowed_tool_rule`): no CLI flag and no daemon path write it since D52.
  The protocol and verifier hold it to shape only (both fields present, bounded text and a
  SHA-256 hex digest, or both absent) for any `profile_id`: the protocol keeps the fields; the
  verifier holds them to shape only, so v5 traces that carry them stay valid (D52, amended
  2026-10-15).
- **The identity wait**: `capture.identity.wait_seconds`, a connector field defaulting to
  `DEFAULT_STDOUT_WAIT_SECONDS` (120 s).
- **Home directories**: each connector's `real_dirs` names its own (`{user_home}/.codex` and so
  on); no core code lists them.
- **Reserved variable prefixes**: `capture.reserved_env_prefixes`, a connector field (Hermes sets
  `HERMES_`).

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

- **Three digests, because one is not enough.** The CLI hashes the program file it resolved
  (for a script or npm/pip wrapper, the only digest that names the harness). The daemon, at the
  launch post, which comes after `exec`, hashes the registered process's running image (on Linux
  through `/proc/<pid>/exe`, the executed file itself; on macOS through `proc_pidpath`, a name
  that a replacement can change) and, when the connector declares a `native_binary`, that
  process's image too. For an interpreted harness the image is `node` or `python`, which says
  nothing about the harness. The daemon hashes outside the run lock: the files are 100–200 MB.
- **Recording them in the trace needs a DECISIONS row.** `v0.5.0` is tagged and the v5 records
  are frozen (D43 ruling 7): every record refuses unknown fields, so a v0.5.0 verifier or server
  rejects a trace that carries new `Launch` fields. Either the row accepts that (the server and
  verifier upgrade first), or the build goes into protocol v6. Owner decision.
- **The version is the harness's own word.** A connector may name `version_args` (Codex and
  Claude Code: `--version`). The CLI runs it once with a minimal environment, no credential and
  a timeout, keeps the first line, and sends it as `harness_version`; without `version_args`
  the field says "unreported". It is reported, not proven: only the digests are evidence. Today
  the field holds the connector's pinned version, so its meaning changes; that belongs in the
  same DECISIONS row.
- **A known-builds list is advisory and lives on the server**, matched on (connector, platform,
  digest), the way the server matches `connector_dir_sha256` (D43 ruling 5), filled by a script
  from official releases with each source URL and the registry's integrity value. "Unlisted" is a
  note for judges, never a refusal; an offline verifier shows only the digests.
- **Recording is enough only with a cross-leg check.** An exact pin never stopped a determined
  participant, but it stopped a cheap one: a three-line wrapper on `PATH` that unsets
  `HTTPS_PROXY` now gets a recorded run, and the verifier has no check that the session file's
  model turns match model calls on the wire. Before refusal goes, the verifier must note "native
  turns with no wire model call" (or, until it does, a CHEAT-CHECKLIST row tells judges to look).
- **No launch-time hash or re-check**: the CLI reads none of the harness binary. A file swapped
  between resolve and release is a different build, which the daemon records in
  `Launch.harness_build`.

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

### Which row-0 items matter most

A survey of the top coding agents by use (October 2026; vendor and community pages, so hints to
measure before building) ranks Claude Code, GitHub Copilot, Codex, Cursor, Antigravity
(`agy`, which replaced Gemini CLI), OpenCode, Hermes, Kilo Code, Windsurf/Devin, Warp, Cline,
Pi, Mistral Vibe, Continue and OpenHands, with Goose next. Against row 0:

- **A SQLite session snapshot is the biggest unlock**: OpenCode, Goose, Cline's CLI, Cursor's
  CLI and part of Hermes keep their sessions in SQLite. A read-only snapshot of the run's own
  rows (D44's design for OpenCode) is one native form for all of them.
- **A wire-only claim level** covers agents whose sessions live on a server (Amp, Warp, cloud
  Devin): the wire leg alone, rated lower, rather than no fit.
- **HTTP/2** matters for Cursor (Connect over protobuf). First check whether its HTTP/1.1
  fallback can be forced per process; only then build an `h2` path.
- **Desktop and IDE agents** are steered to their CLI where one exists (Copilot CLI,
  `cursor-agent`, `agy`); no desktop capture is planned.
- **A local-model recorder** (below) serves about eleven of the fifteen, which accept a local or
  OpenAI-compatible endpoint.

### Local models

A participant may run their own model (Ollama, LM Studio) on loopback; the owner accepts it.
Today the launch exempts loopback from the proxy (`NO_PROXY=127.0.0.1,localhost`), so that
traffic is not recorded, and a pinned connector may not name a loopback host.

The plan: `goalcup start --local-model http://127.0.0.1:11434` names the participant's server
(loopback only). The tracer starts a recording relay on another loopback port, plain HTTP, that
forwards to it and records each request and response as a wire stream, as the proxy does for a
vendor host. A local-model connector (its own connector, beside the vendor one) names a new
launch token, `{local_model_url}`, which only the tracer renders; the connector never writes the
URL. Today's rules refuse that: the template tokens are `workspace` and `session_id`, and any
`*_URL` or `*_BASE` name is refused. So the change is a new token plus one narrow exemption: a
server-named variable or argument is admitted when its whole value is that token.

This sets a base URL. The "never set a model base URL" rule (AGENTS.md, D1) exists to protect
vendor endpoints, and none is replaced here; but the rule is written as absolute, so the
exemption needs its own DECISIONS amendment.

The evidence claim is weaker and says so: "participant-operated endpoint". The record shows what
the harness sent and received, in order and in time, not what answered: a loopback port can be
a local model, a tunnel to a vendor API, or a person typing. Judges weigh it; the verdict names
the claim level. Anything the tracer asks the server about itself (Ollama's `/api/tags`) is the
server's own claim.

## Order of work

1. Landing now: D51 (argv and env are data), the optional `home.env`, the kit docs with row 0,
   D52 (no model, sandbox or permission limits). The Intel pins of #72 become moot under step 2.
2. Record the build, then stop pinning, in this order: the DECISIONS row for the v5 freeze (or
   v6); the three digests and `version_args`; the verifier's cross-leg note (or the
   CHEAT-CHECKLIST row); then `executable.sha256`, the per-platform pins and their refusal path
   go, leaving `[capture.platform.<key>]` for env overrides only.
3. No harness names in core: delete the Claude allowance flag and daemon path (keep the v5 field
   and the reader check), make the stdout scope wait a connector field, drop the three
   hard-coded home folders and the `HERMES_` reserved prefix.
4. Declare native decoder families a connector picks by `protocol_hint`.
5. Make the kit case derive from the connector and shrink the per-connector golden.
6. The server-side known-builds list, its refresh script and the "unlisted" note.
7. The local-model relay, with its DECISIONS amendment to D1.
8. Measure one real run per harness and platform family, recorded in its facts sheet.
9. Row-0 items by the survey above: the SQLite snapshot form first (OpenCode, Goose, Cline,
   Cursor CLI), then the wire-only claim level (Amp, Warp), then a forced-HTTP/1.1 check and
   only if needed an `h2` path (Cursor), then subagent files.
