# <Harness name> harness facts

Connector id: `<id>` · Facts for version: `<x.y.z>` · Written by: <name/team> · Last checked: YYYY-MM-DD
Public disclosure: <yes / partly — say what is withheld>
Status: <shipped connector | core ready, connector pending | facts only>

<!-- Delete this comment when you fill the sheet. Worked example: docs/harnesses/codex-cli.md.
Status, pick one:
- shipped connector: connectors/<id>/ is on main, or is added by the same PR as this sheet.
- core ready, connector pending: no row of the core-change table in docs/HARNESS-ONBOARDING.md needs a core change, but no connector yet.
- facts only: a core change is needed first, or a key fact is unknown; the sheet is the whole
  contribution for now.
Section 8 lives only in this sheet: the PR description links the sheet, it does not repeat it.
Rules: docs/harnesses/README.md. -->

## 1. Run one prompt, in chat mode first
Chat mode is the product (D57): the command that opens the harness's chat in a terminal (no
argument is best), how the participant types the prompt, how to quit, and the flag that reopens
the most recent session. Only if chat mode cannot work: the headless command and flags to run one
prompt with no human input, and how it exits.

## 2. Install and record a version
How to install a given version; where the binary lands; how to print the version.

## 3. Where it keeps things, per OS
| | macOS | Linux | Windows |
|---|---|---|---|
| Config dir | | | |
| Session / transcript store | | | |
| Logs / cache | | | |
| Env vars that move these | | | |
Write "unknown" rather than guess; "same as macOS" is fine when true.

## 4. Session format and identity
File type (JSONL / SQLite / other), one file per session or shared, how a session id is formed, and
where the id first appears (stdout init event, stderr line, file name, preassigned flag).

## 5. Auth
Subscription login vs API key; env var names; where credentials are stored per OS; any file derived
from the key (fingerprints, owner files). Chat mode passes no key: say how the sign-in is kept in
the harness's own config home (a login command, or the macOS Keychain). A key read only from an
environment variable does not fit chat mode.

## 6. Network
First, the CA variable the harness honours for an extra trusted certificate (for example
`NODE_EXTRA_CA_CERTS`); only `SSL_CERT_FILE` or `REQUESTS_CA_BUNDLE` means a core change. Then every
model/provider host it can reach: if the participant may pick the provider, list every provider
host, because an unlisted host is not recorded. Telemetry and update hosts; HTTPS_PROXY honoured?
SSE / WebSocket? Any base-URL override variables (Goal Cup never sets them).

## 7. Process model
Subprocesses, tools, subagents; what happens on SIGTERM / Ctrl-C; exit codes.

## 8. Things Goal Cup cannot isolate
Machine-wide or managed settings, server-side state, anything outside a per-run HOME.

## 9. Quirks and open questions
