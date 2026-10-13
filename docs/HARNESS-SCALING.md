# Scaling to 30 harnesses

How adding and keeping 30 harnesses on four platforms (macOS arm64 and x86_64, Linux x86_64 and
arm64) stays cheap. Most harnesses land before the hack; one or two may land during the 24-hour
start window, under watch. Written 2026-10-14 from review r4 of the harness kit; a plan, not a
record of what is built.

The principle: a harness is data in `connectors/<id>/`, reviewed by a code owner. The tracer's
code knows capture *mechanisms* (a proxy, a session-file reader, an identity carrier, a decoder
family), never a harness by name. A harness rarely changes how it sends tokens or writes its
transcript from one day to the next, so a new build of a known harness should cost nothing more
than a reviewed list entry.

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

## 4. Versions, pins and new connectors

Today each connector pins one build per platform by SHA-256, and `goalcup start` refuses any
other. At 30 harnesses × 4 platforms that is 120 hashes, each redone on every vendor release.
And because connector directories are pinned by digests compiled into the signed binaries, a new
connector, or any change to one, is a new tracer release that participants must install.

The proposal: keep a list of accepted builds per harness and platform; refresh it with a script
from official releases (npm, GitHub releases, PyPI); let an unknown build run with its hash and
version recorded, flagged "unlisted build" for review instead of refused. Checked against the
code, it is sound only like this:

1. **The list lives on the server, not in the connector.** Matched on (connector, platform,
   executable digest), the way the server already matches `connector_dir_sha256` (D43 ruling 5).
   In the connector directory it would change the directory's compiled pin, so every refresh
   would be a release. The local copy that `doctor` reads is a hint, not the authority.
2. **The daemon hashes what actually runs.** Today the CLI hashes the binary, checks it and drops
   the result; the trace holds only the directory digest. The CLI is the participant's own
   process, so a modified CLI can report any hash, and the same is true of today's pin. The daemon
   should hash the registered process's own executable after `exec` (`proc_pidpath` on macOS,
   `/proc/<pid>/exe` on Linux) and record it, beside the CLI's hash, as an optional skip-if-none
   v5 launch field. The version comes from the build list match, not from the connector, which
   only declares one. The gap between hash and `exec` (`docs/CHEAT-CHECKLIST.md`) remains.
3. **"Unlisted" is set where the list is.** An offline verifier reads no list, so it shows only
   the digest; the server, or a verifier given the list as an input, sets the flag. An unlisted
   build can be anything on `PATH`: a wrapper that unsets `HTTPS_PROXY` or rewrites the base URL,
   or a fake that writes a session file and makes no model call, with both legs still looking
   consistent. So a judge must see the flag in the verdict, with three outcomes: cleared
   (the build is later listed, automatically), downgraded, or disqualified. `doctor` and `start`
   warn in plain words before the run, not after 168 hours.
4. **The refresh script does not trust the registry blindly.** A hash cannot be reviewed by eye.
   Each entry records its source URL and the registry's integrity value, prefers vendor
   checksums, provenance or signatures where they exist, waits a short soak period, and can be
   delisted later.
5. **One file hash proves little for a script harness.** Hermes's pin is a console script that
   imports a Python package tree, and npm wrappers spawn a platform binary. Pin the file that
   actually runs (Codex already pins its vendor binary), or a package-tree digest (a wheel's
   `RECORD`, npm's integrity), or record "script; not proven".

The same move answers new connectors during the 24-hour window: if the server also holds the
accepted connector-directory digests, a connector reviewed and listed there can run on an
installed tracer with `--dev` semantics replaced by "unlisted connector" review. That trades the
local refusal for review, as above, and is an owner decision.

Evidence per harness × platform (120 combinations) cannot come from CI, which stays lean. Each
harness needs one measured real run per platform family before the hack, recorded in its facts
sheet; until then a platform is "pinned, not measured" (Intel and Linux today).

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
3. Remove the harness names from core: delete the Claude allowance flag and daemon path, make
   the stdout scope wait a connector field, drop the three hard-coded home folders.
4. Make the kit case derive from the connector and shrink the per-connector golden. Both are
   cheap and should land before most of the 30 connectors do.
5. Have the daemon hash the running executable and record it (condition 2).
6. Move accepted builds, and then accepted connector digests, to a server list with the refresh
   script, the "unlisted" verdict flag and the doctor warning (conditions 1, 3–5). Owner decision
   first.
7. Declare native decoder families a connector picks by hint.
8. Measure one real run per harness and platform family, recorded in its facts sheet.
9. Take row-0 items one at a time, by demand: subagent files, database snapshots, wire-only.
