# Scaling to 30 harnesses

How adding and keeping 30 harnesses on four platforms (macOS arm64 and x86_64, Linux x86_64 and
arm64) stays cheap. Most harnesses land before the hack; one or two may land during the 24-hour
start window, under watch. Written 2026-10-14 from review r4 of the harness kit; the plan, not a
record of what is built. Each section ends with what changes.

The principle: a harness is data in `connectors/<id>/`, reviewed by a code owner. The tracer's
code knows capture *mechanisms* (a proxy, a session-file reader, an identity carrier, a decoder
family), never a harness by name. A harness rarely changes how it sends tokens or writes its
transcript from one day to the next, so a new build of a known harness should cost nothing.

## 1. Per-connector rows in Rust

Until D51 every connector's exact argv and env were rows in `capture_policy.rs`, so a new harness
or a model bump was a Rust PR. D51 removes them: argv and env are held to generic rules (no
endpoint, proxy, credential, trust or code-loading override), and review of the pinned directory
is the control. Three closed lists stay, because each is a security decision: credential names
(which participant secret the tracer hands over), CA variables (a trust statement) and redaction
regexes (code in the signed daemon). Those are expected to grow by a row now and then, each a
small core PR with a DECISIONS entry.

**Changes:** D51 (open). Nothing else here.

## 2. Harness names in core

Four places still name a harness:

- **The Claude tool allowance** (`--capture-claude-allowed-tool`), checked by connector id in the
  CLI, daemon, verifier and protocol. Since D52 the shipped Claude profile bypasses permission
  prompts, so the allowance cannot apply. Delete it; do not generalise it.
- **Native decoders**: the verifier reads Codex and Claude Code transcripts (`Format { Codex,
  ClaudeCode }`); any other harness's session file is recorded but not decoded. Make decoding a
  small declared set of *families* a connector picks by `protocol_hint` (it already does for the
  wire: OpenAI Responses, Anthropic Messages, Chat Completions). A new harness that writes, say,
  Chat-Completions-shaped JSONL picks that family; only a new shape is a verifier PR.
- **`CODEX_SCOPE_WAIT`** (120 s), the identity wait for every harness: a connector field with a
  bounded default.
- **`native_scope.rs`** always forbids `~/.codex`, `~/.claude` and `~/.hermes`, beside the
  manifest's own `real_dirs`, which already does the job. Keep `real_dirs`, drop the three names.
  The same goes for `HARNESS_RESERVED_PREFIXES = ["HERMES_"]`, which can become a per-connector
  "reserved prefix" field or go.

## 3. The same facts typed three times

A connector's home, identity and session-file facts are written in `harness.toml`, again in the
kit case's `harness.toml`, and again in its `expect.toml`; then each connector gets a generated
golden of about 1,150 lines (`hermes-cli.json` is 1,136) that nobody reads in review.

**Changes:** the kit runner loads `connectors/<id>/harness.toml` and replaces only the
executable, argv, hosts and credential with the stand-in's. `expect.toml` shrinks to what the
stand-in writes (session path, first bytes, wire request). The per-connector golden becomes a
short projection of the fields a reviewer checks, or a digest of the long one.

## 4. Versions and pins

Today each connector pins one build per platform by SHA-256, and `goalcup start` refuses any
other. At 30 harnesses × 4 platforms that is 120 hashes, each re-done on every release.

The proposal: each connector keeps a list of accepted builds per platform. A script refreshes
it from official releases (npm, GitHub releases, PyPI), downloads each artifact, hashes the file
the CLI will run and writes the list; a reviewer reads the diff. An unknown build still runs: its
hash and version are recorded, and the verifier flags "unlisted build" for review instead of
refusing.

Checked against the code, this is sound with four conditions. Without them it is unsafe.

1. **The digest must be in the trace.** Today it is not: the CLI hashes the binary, checks it and
   throws the result away; the trace holds only the connector directory's digest. Add the
   executable's SHA-256, its platform and declared version to the launch record, as an optional
   skip-if-none v5 field (the same pattern as `connector_dir_sha256`), written by the daemon.
2. **"Unlisted" must be visible in the verdict**, not just in a log. An unlisted build can be
   anything the participant put on `PATH`: a wrapper that unsets `HTTPS_PROXY` or rewrites the
   base URL, or a fake that writes a session file and makes no model call. The wire and native
   legs would still look internally consistent. A pin is what tells a judge "this is the vendor's
   program"; without one, the judge needs the flag to know they must look.
3. **Warn before the run, not after 168 hours.** `goalcup doctor` and `start` should say "this
   build is unlisted; it will be reviewed" in plain words, so a participant can install a listed
   build first.
4. **A script entry point proves little.** Hermes's pin is a 372-byte console script whose
   shebang names an interpreter path that differs per install, and npm wrappers spawn a platform
   binary. For those, the list should name the file that actually runs (Codex already pins its
   vendor binary), or the record says "script; not proven".

The list lives in the connector directory, so it is covered by the directory pin and code-owner
review. A new listed build is then a reviewed data PR made by a script, and an unlisted build is a
review item rather than a lost run. Intel pins (PR E) are the same data in today's shape.

## 5. What cannot be captured yet

Row 0 of the kit (`docs/HARNESS-ONBOARDING.md`) stops these harnesses in minute one. What each
would take:

| Kind | What it would take |
|---|---|
| No local session file (server-side sessions) | A wire-only claim level: schema allows zero native rows, the verifier rates it lower |
| Database store (SQLite, LevelDB) | A read-only snapshot native form for the run's own rows (D44 plans this for OpenCode) |
| HTTP/2-only or gRPC | An `h2` proxy path, plus a decoder for its framing; the largest item here |
| Local or loopback model | Recording plain HTTP to loopback; a different evidence claim, since no vendor sees the call |
| Desktop or IDE app | V1.1 scope: launch and isolation of an app, not a CLI |
| Subagent transcripts in own files | A `children` pattern beside the one native row |
| Self-update mid-run | Re-hash the binary when the run ends and record a mismatch |
| More than 16 hosts or wildcards | Raise the bound; wildcard hosts need a reviewed suffix rule |

## Order of work

1. Land D51 (argv and env are data) and the optional `home.env`, then the kit docs with row 0.
2. Land D52 (no model, sandbox or permission limits in the shipped connectors) and the Intel pins
   in today's shape, so every release target can capture.
3. Record the executable's digest, platform and version in the trace (condition 1).
4. Replace the single pin with an accepted-builds list per platform, plus the refresh script and
   the "unlisted build" verdict flag and doctor warning (conditions 2–4).
5. Remove the harness names from core: delete the Claude allowance, make the scope wait a
   connector field, drop the three hard-coded home folders.
6. Make the kit case derive from the connector and shrink the per-connector golden.
7. Declare native decoder families a connector picks by hint.
8. Take row-0 items one at a time, by demand: subagent files, database snapshots, wire-only.
