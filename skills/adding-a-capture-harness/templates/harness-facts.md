# <Harness name> harness facts

Connector id: `<id>` · Facts for version: `<x.y.z>` · Written by: <name/team> · Last checked: YYYY-MM-DD
Public disclosure: <yes / partly — say what is withheld>
Status: <shipped connector | core ready, connector pending | facts only>

## 1. Run one prompt, headless
The exact command and flags to run one prompt with no human input, and how it exits.

## 2. Install and pin a version
How to install an exact version; where the binary lands; how to print the version.

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
from the key (fingerprints, owner files).

## 6. Network
Model/provider hosts; telemetry and update hosts; HTTPS_PROXY honoured? custom CA honoured (which env
var)? SSE / WebSocket? Any base-URL override variables (Goal Cup never sets them).

## 7. Process model
Subprocesses, tools, subagents; what happens on SIGTERM / Ctrl-C; exit codes.

## 8. Things Goal Cup cannot isolate
Machine-wide or managed settings, server-side state, anything outside a per-run HOME.

## 9. Quirks and open questions
