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
Chat mode is the only mode (D57): the command that opens the harness's chat in a terminal (no
argument is best), how the participant types the prompt, how to quit, and the flag that reopens
the most recent session. A harness with no chat mode in a terminal cannot be captured: say so here
and set `Status: facts only`.

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
environment variable does not fit chat mode. Say yes or no: does the harness have a variable that
moves its config and login (like `CODEX_HOME`)? Yes: the connector shares that config home. No:
the connector keeps one shared private `HOME` (`run_home = "shared"`) so the sign-in lasts.

## 6. Network
First, the CA variable the harness honours, with its source or measurement, and whether it adds
to the harness's roots (`NODE_EXTRA_CA_CERTS`: mode `extend`) or replaces them (a harness's own
`*_CA_BUNDLE`: mode `replace`). Only if it has no variable of its own: `SSL_CERT_FILE` or
`REQUESTS_CA_BUNDLE` (mode `extend`, and a direct connection then goes unrecorded). Then where
the harness saves the model service the participant picks: the config file (relative to its
config home) and the key that holds the service's URL, with a source or a measurement, and
whether a switch inside a session (a `/model` command) rewrites it. That is
`capture.model_host_from` (D62), and no provider list is needed. Only for a harness that can
reach one fixed service: its host, and whether the shared provider table
(`crates/goalcup-protocol/providers.toml`) has it. Say yes or no: is the harness Node (npm package,
`node` script or Node/Bun binary)? Say yes or no: does it honour `HTTPS_PROXY` on its own, and if
not, which variable turns that on (Node: `NODE_USE_ENV_PROXY=1`)? Without it the run records no
model traffic. Telemetry and update hosts; SSE / WebSocket? Any base-URL override variables (Goal
Cup never sets them).

## 7. Process model
Subprocesses, tools, subagents; what happens on SIGTERM / Ctrl-C; exit codes.

## 8. Things Goal Cup cannot isolate
Machine-wide or managed settings, server-side state, anything outside a per-run HOME.

## 9. Quirks and open questions

## Run without permission prompts
The setting in the harness's own config that stops its permission prompts, the file and key it
lives in, and the values that mean so (source or measurement). It becomes
`[[participant.no_prompts]]` and the harness's section in docs/HARNESS-SETUP.md. Goal Cup
recommends it and never sets it (D67). "None needed" is a fact too: say how you know.
