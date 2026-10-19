# Security policy

How do I report a vulnerability, and what does this tool do on my machine? For anyone who runs the
tracer or reviews a change to it.

## Supported versions

The latest tagged release and `main`. Release tarballs are unsigned and not notarized
([packaging/README.md](packaging/README.md)).

## Reporting a vulnerability

Do not open a public issue.

1. Email goalcup@agentcommunity.org.
2. Or use GitHub private vulnerability reporting: the Security tab, "Report a vulnerability".

Say what you ran, what you expected and what happened. Never attach a real trace, key or harness
transcript: they hold prompts and credentials. A synthetic reproduction is enough. We aim to
acknowledge a report within 7 days and will say what we plan to do. A small team maintains this
project. There is no bug bounty.

For an anti-cheat recorder, a way to bypass or forge capture is a vulnerability, not just a bug.
The gaps already accepted are in [docs/CHEAT-CHECKLIST.md](docs/CHEAT-CHECKLIST.md). A report of
one of those is known, not new, unless it shows that the stated bound does not hold.

## What the tool does on your machine

- Runs a per-user daemon only while a run needs it. `goalcup start` (or `goalcup seal`) starts it
  detached, in a session of its own, with an environment built from nothing: none of your shell's
  API keys or proxy variables reach it. It exits once no run is unsealed. Nothing is installed as a
  service and nothing starts at login.
- Listens on loopback TCP only: the control API, an ingest port and a per-launch HTTPS proxy.
- Generates a per-launch CA that only the harness process trusts, through that harness's own
  variable (`NODE_EXTRA_CA_CERTS`, `CODEX_CA_CERTIFICATE` or `HERMES_CA_BUNDLE`). It never touches
  the system trust store and never sets a model base URL. `HERMES_CA_BUNDLE` replaces that
  harness's trust roots rather than extending them, so a harness that ignores it fails its TLS
  handshake instead of leaving traffic uncaptured. D43 governs which other variables a connector
  may use.
- Uses isolated harness homes rather than the user's normal stores. Codex, Claude Code and
  Hermes keep shared config homes (`CODEX_HOME`, `CLAUDE_CONFIG_DIR`, `HERMES_HOME`); Amp keeps
  an isolated shared `HOME`. Runs sharing a home are serialized. Claude Code's per-run home
  links the macOS Keychains directory. Hermes's shared home is never seeded: its own setup
  keeps its provider and login there. The tracer passes no provider key and never writes
  app-wide harness configuration such as `~/.claude`, `~/.codex` or `~/.hermes`.
- With `goalcup login`, talks to the Goal Cup server (`https://ingest.agentcommunity.org`) and to
  GitHub for the participant's repository. A run's credential is stored at mode 0600 under the
  configuration directory and is never put in the harness's environment or argv.

## Assets and trust boundaries

Assets: harness subscription credentials; prompts, responses and tool output in traces; the control
bearer; the ingest and proxy secrets; the run credential; trace integrity (the chain and its seal).
A trace that never reached the server can be rewritten and re-sealed
([CHEAT-CHECKLIST](docs/CHEAT-CHECKLIST.md)).

| Adversary | Position |
|---|---|
| Other local accounts | Blocked by loopback plus a secret on every socket, and 0600/0700 files. Create artifact and config directories made by hand with mode 0700. |
| Web pages | No CORS. The bearer is required on every mutating route. A status read needs a 64-bit entry id. The control API refuses any `Host` other than its own loopback address and port before the public/bearer split, so a DNS-rebinding page reads nothing. One route is public: `/v0/entries/{id}/status` ([CONTRACTS](docs/CONTRACTS.md#the-control-apis-unauthenticated-surface)). |
| Same-user processes, including the agent under test | **Inside the boundary.** They can read `control.json` (mode 0600, same uid), the harness environment and run directories, and call authenticated mutators while a capture has live authority. After a restart, a recovered capture accepts status and seal only. Detection is after the fact, from the captured native and wire records. |
| Network | The proxy validates upstream TLS. Only declared provider hosts and paths are decrypted and retained. Authentication hosts always tunnel unmodified. |
| The Goal Cup server | Signs start receipts and acknowledgements with an Ed25519 key. The verifier accepts only the key built into it (`SERVER_PUBLIC_KEYS`, the production Worker's). Any other key verifies only when passed with `--dev-key`, and the report then says `key_source: "supplied"` and `server_key_unpinned`. No key whose seed is in this repository is trusted. |
| Connector authors | **Trusted code.** See below. |

## Connectors are trusted code

A connector (`connectors/<id>/`) chooses what program runs, with what environment, which templates
and seed files are copied into the harness home, which hosts are decrypted and which paths are
retained. A `statusLine` command in a settings template, a `notify` or MCP entry in a Codex config,
a `capture.hosts` entry or a seeded file is a code-execution or data-exfiltration surface. The
directory pins detect tampering after a build. They do not defend against a malicious merge,
because a connector PR regenerates them. Every connector change is reviewed by a code owner against
the [checklist](docs/HARNESS-ONBOARDING.md#reviewing-a-connector-pr), and you must not run an
unreviewed build (D51). Every committed [connector directory](connectors) is packaged for release;
its [facts sheet](docs/harnesses/README.md) records measured support and gaps. Packaging inclusion
is not proof of a published binary or a successful real harness run.

## Guarantees the code aims to keep

- Bodies are redacted by streaming before they reach disk.
- Credentials stay out of logs, argv, errors and `Debug` output.
- The harness environment is built from nothing. After the daemon durably registers the harness
  process, the CLI re-opens the program path with `O_NOFOLLOW|O_NONBLOCK` and requires a regular
  file before it releases the harness. It reads none of the file's bytes. The daemon records the
  program and image digests in `Launch.harness_build` (D53).
- The control API requires its version on mutating requests. Capture creation refuses, before
  daemon entry, artifact and workspace allocation, when CLI and daemon loaded different connector
  bytes (D46). Earlier CLI run, home, CA and seed preparation can exist, and is cleaned on refusal.
- A restarted unfinished capture has status and seal authority only. Other authenticated mutators
  refuse before side effects. A launched capture records one `IngestLost` gap.
- A shared capture home keeps its exact leaf alias only after leaf and canonical-target ownership,
  type, identity and `0700` mode checks, repeated before the create request (D39).
- Undeclared hosts are never decrypted.

## Known residuals

Listed in [docs/CHEAT-CHECKLIST.md](docs/CHEAT-CHECKLIST.md) (redaction, key-scan blind spots) and
[docs/TERMINATION.md](docs/TERMINATION.md) (termination windows). Also:

- Same-user control of the daemon (above).
- The launch is not inode-bound execution. A same-UID writer or an updater can replace the program's
  pathname before `exec`. That is a different build, and the daemon records it.
- Shared-home checks do not inspect ACLs and cannot exclude a same-UID replacement after the final
  check.
- Connector loading is a sequence of directory reads and file opens, not an atomic snapshot. A
  same-UID process can replace names between them. The digest binds the observed sequence, not one
  instant.
- The daemon `goalcup` starts is the `goalcup-daemon` beside it, or first on `PATH`. A same-user
  process that replaces that binary is inside the stated boundary.

## Out of scope

Root or same-user malware; a compromised harness binary; logging by the model provider; native
Windows (use WSL2).

## For contributors

Never commit `spikes/*/out/`, `harnesses/`, real traces or keys. Put every fixture through
`spikes/redact.py` and review it by eye. CI runs no secrets on pull requests. See
[CONTRIBUTING.md](CONTRIBUTING.md).
