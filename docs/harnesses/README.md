# Harness facts sheets

One public page per harness, `docs/harnesses/<connector-id>.md`. It says where that harness keeps
its sessions, config and logins on each OS, how it authenticates, and which hosts it calls.

A connector describes only what the tracer does today. The sheet records the rest: what to
check when the tracer gains a platform or a new way to capture, without asking each team again.
A sheet is also a complete contribution on its own. A harness that does not fit the tracer yet
sends its sheet with status `facts only`, and the sheet shows what to build.

The sheets so far: [codex-cli](codex-cli.md) (the worked example), [claude-code-cli](claude-code-cli.md),
[hermes-cli](hermes-cli.md) and [amp-cli](amp-cli.md). The Amp sheet is a draft that Goal Cup wrote for the
Amp team to correct; its connector records a metadata log, not a transcript, as the native leg.

## What is public

Everything in a sheet is public. A sheet holds facts from your documentation, or that anyone can
measure on an installed copy: paths, variable names, file formats, host names, exit codes. It
never holds a secret, token, key, account name or personal path. Write `$HOME` and `%APPDATA%`. To
keep a fact private, leave it out, say so on the "Public disclosure" line, and email it to
goalcup@agentcommunity.org.

## How to write one

Copy [the template](../../skills/adding-a-capture-harness/templates/harness-facts.md) to
`docs/harnesses/<connector-id>.md`. Keep its headings and order, so every sheet reads the same.
Replace each one-line prompt with facts. Every fact has a source: a link to your docs, "measured
on version X", or "unknown". Write "unknown" rather than guess. "Same as macOS" is fine when true.

The `Status:` line takes one of three values:

- **shipped connector**: `connectors/<id>/` is on main, or the same PR adds it. This is source
  inclusion, not a claim that participant binaries are published or event practice is complete.
- **core ready, connector pending**: no row of the core-change table in
  [HARNESS-ONBOARDING](../HARNESS-ONBOARDING.md#do-i-need-a-core-change) needs a core change, but no
  connector exists yet.
- **facts only**: the tracer needs a core change first, or a key fact is unknown.

Section 8, "Things Goal Cup cannot isolate", lives only in the sheet. A connector PR links the
sheet in its description and does not repeat it.

## How to update one

When a release changes something a sheet says (a path, a variable, the session format, an auth
flow, a host), open a PR that edits the sheet and updates "Facts for version" and "Last checked".
If the harness has a connector, change the connector in the same PR and follow
[HARNESS-ONBOARDING](../HARNESS-ONBOARDING.md) again. A new build of a known harness needs no
connector change when the sheet's facts still hold: builds are recorded, not pinned.
