# Security policy

How do I report a vulnerability, and what does this tool do on my machine? For anyone who runs
the tracker or reviews a change to it. Updated: 2026-10-11.

## Supported versions

v0.5.0 is published as source only
([release](https://github.com/agentcommunity/goal-cup-tracer/releases/tag/v0.5.0), no attached
files). There is no stable release, downloadable binary or installer, staged bundle, registry
publication, signature, notarization or qualification.

## Reporting a vulnerability

Do not open a public issue.

1. Email goalhack@agentcommunity.org.
2. Or use GitHub private vulnerability reporting: the Security tab, "Report a vulnerability".

Say what you ran, what you expected, and what happened. Never attach a real trace, key or
harness transcript: they contain prompts and credentials. A synthetic reproduction is enough.
We aim to acknowledge a report within 7 days and will tell you what we intend to do about it.
This is a pre-release project maintained by a small team; there is no bug bounty.

For an anti-cheat recorder, a way to bypass or forge capture is a vulnerability, not just a bug.
The gaps already accepted are listed in [docs/CHEAT-CHECKLIST.md](docs/CHEAT-CHECKLIST.md): a
report of one of those is known, not new, unless it shows the stated bound does not hold.

## What the tool does on your machine

- Runs a per-user daemon only while a run needs it. `goalcup start` (or `goalcup seal`) starts
  it detached, in a session of its own, with an environment built from nothing: none of your
  shell's API keys or proxy variables reach it. It exits once no run is unsealed. Nothing is
  installed as a service and nothing starts at login.
- Listens on loopback TCP only: the control API, an ingest port, and a per-launch HTTPS proxy.
- Generates a per-launch CA that only the harness process trusts, through its own variable
  (`NODE_EXTRA_CA_CERTS`, `CODEX_CA_CERTIFICATE` or `HERMES_CA_BUNDLE`). It never touches the
  system trust store and never sets a model base URL.
  `HERMES_CA_BUNDLE` replaces that harness's trust roots rather than extending them, so a harness
  that ignores it fails its TLS handshake instead of leaving traffic uncaptured. It is the only
  replace-mode variable today; D43 governs which others a connector may use.
- Gives every run a fresh `HOME`. Codex and Claude use isolated shared `CODEX_HOME` and
  `CLAUDE_CONFIG_DIR`; Claude's run home gets the checked `~/Library/Keychains` link. Hermes uses
  a per-run `HERMES_HOME` populated only from declared seeds. It never writes app-wide harness
  configuration such as `~/.claude` or `~/.codex`.

## Assets and trust boundaries

Assets: harness subscription credentials; prompts, responses and tool output in traces; the
control bearer; the ingest and proxy secrets; trace integrity (the chain and its seal;
a local-only chain can be rewritten and re-sealed, see [CHEAT-CHECKLIST](docs/CHEAT-CHECKLIST.md)).

| Adversary | Position |
|---|---|
| Other local accounts | Blocked by loopback plus a secret on every socket, and 0600/0700 files. Artifact and config directories made by hand rather than by the installer use the default umask: create them 0700. |
| Web pages | No CORS; the bearer is required on every mutating route; a status read needs a 64-bit entry id. The control API refuses any `Host` other than its own loopback address and port, before the public/bearer split, so a DNS-rebinding page reads nothing. Only one route is public (`/v0/entries/{id}/status`); [docs/CONTRACTS.md](docs/CONTRACTS.md#the-control-apis-unauthenticated-surface) says so. |
| Same-user processes, including the agent under test | **Inside the boundary.** They can read `control.json` (mode 0600, same uid), the harness environment and run directories, and call authenticated mutators while a capture has live authority. After restart, recovered v5 captures accept status and seal only. Detection is after the fact, from captured native and wire records. A keychain-held or per-run token, or a sandbox denying config access, is future work. |
| Network | The proxy validates upstream TLS. Only declared provider hosts and paths are decrypted and retained; authentication hosts always tunnel unmodified. |
| Connector authors | **Trusted code.** See below. |
| The project server | Not used by the M2 capture path (LocalOnly). Until a production server exists, `goalcup-verify` trusts **no** server key: a receipt or acknowledgement verifies only under a key supplied to that one run (`--dev-key`), and the report then records `key_source: "supplied"` and `server_key_unpinned`. No key whose seed is in this repository is trusted, so no trace reads as `anchor: signed` on its own. |

## Connectors are trusted code

A connector (`connectors/<id>/`) chooses what program runs, with what environment, which
templates and seed files are copied into the harness home, which hosts are decrypted, and which
paths are retained. A `statusLine` command in a settings template, a `notify` or MCP entry in a
Codex config, a `capture.hosts` entry or a seeded file is a code-execution or data-exfiltration
surface. The manifest pin tables detect tampering after a build; they do not defend against a
malicious merge, because a connector PR regenerates them. Every connector change is therefore
reviewed by a code owner against the checklist in
[docs/CONNECTOR-CONTRIBUTING.md](docs/CONNECTOR-CONTRIBUTING.md#reviewing-a-connector-pr), and
an unreviewed build must not be run. The source tree contains exactly three pinned
schema-1 directories (`codex-cli`, `claude-code-cli`, `hermes-cli`). The private `m2-pilot` macOS
package allowlist selects only Codex and Claude and copies each selected directory whole, including
any seeds; a source pin does not prove package inclusion. The build has no V1 execution path
(D48) and no V1 `manifest.toml` (OS-29): a connector is its `harness.toml` and seeds.

## Guarantees the code aims to keep

- Bodies are redacted by streaming before they reach disk.
- Credentials stay out of logs, argv, errors and `Debug` output.
- The capture executable is SHA-256 pinned and re-opened with `O_NOFOLLOW|O_NONBLOCK`; after durable
  process registration the CLI requires a regular file and streams what it reads into SHA-256
  immediately before gate release. The harness environment is built from nothing.
- Control API v5 requires its version on mutating requests. Capture creation refuses before daemon
  entry, artifact and workspace allocation when CLI and daemon loaded different connector bytes
  (D46); earlier CLI run/home/CA/seed preparation can exist and is cleaned on refusal.
- A restarted unfinished v5 capture has status/seal-only authority; other authenticated mutators
  refuse before side effects, and an unfinished launched capture records one `IngestLost` gap.
- A shared capture home preserves its exact D39 leaf alias only after leaf and canonical target
  ownership, type, identity and exact `0700` mode checks, repeated before the create request.
- Undeclared hosts are never decrypted.

## Known residuals

Listed in [docs/CHEAT-CHECKLIST.md](docs/CHEAT-CHECKLIST.md): redaction (F-07, H-20), key-scan
blind spots (H-18), and the termination windows in [docs/TERMINATION.md](docs/TERMINATION.md).
Also stated here: same-user control of the daemon (above), the absent server key (above),
the macOS executable recheck is neither a coherent snapshot nor inode-bound execution (a same-UID
writer can mutate the open file from the first read, then replace its pathname before `exec`),
shared-home checks do not inspect ACLs and cannot exclude a same-UID replacement after the final
check, and connector tree loading/staging is a sequence of directory enumeration and file opens,
not an atomic namespace snapshot. A same-UID process can replace names between those operations;
the digest binds the observed sequence, not one coherent instant. The daemon `goalcup` starts is
the `goalcup-daemon` beside it or first on `PATH`; a same-user process that replaces that binary
is also inside the stated boundary.

## Out of scope

Root or same-user malware; a compromised harness binary that matches the pin; logging by the
model provider; Windows and Linux (not yet supported claims).

## For contributors

Never commit `spikes/*/out/`, `harnesses/`, real traces or keys. Put every fixture through
`spikes/redact.py` and review it by eye. CI runs no secrets on pull requests. See
[CONTRIBUTING.md](CONTRIBUTING.md).
