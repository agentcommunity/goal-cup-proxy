# Harness facts sheets

One public page per harness, `docs/harnesses/<connector-id>.md`, saying where that harness keeps
its sessions, config and logins on every OS, how it authenticates and what it talks to.
Updated: 2026-10-12.

## Why they exist

A connector only describes what the tracer does today: macOS, one way of capturing. Goal Cup
will add Linux and Windows, and the way it captures will change. When that happens we need to
know, for every harness, where its session files land on each OS, which variables move them,
where its login is stored and which hosts it calls. Without a sheet we would have to ask each
team again and wait for answers. With one, we can plan the change ourselves and only ask about
what the sheet marks "unknown".

A sheet is also a complete contribution on its own. A harness that does not fit the tracer yet
sends its sheet first (status "facts only"); the sheet then shows us what to build.

## What is public

Everything in a sheet is public, in this repository. It holds facts that are already in your
documentation or that anyone could measure on an installed copy: paths, variable names, file
formats, host names, exit codes. It never holds secrets, tokens, keys, account names or personal
paths: write `$HOME`, `%APPDATA%` and so on. If you prefer to keep a fact private, leave it out,
say so on the "Public disclosure" line, and email it to goalhack@agentcommunity.org.

## How to write one

Copy [the template](../../skills/adding-a-capture-harness/templates/harness-facts.md) to
`docs/harnesses/<connector-id>.md` and keep its headings and order, so every sheet reads the
same. Replace each section's one-line prompt with the facts. Every fact has a source: a link to
your docs, "measured on version X", or "unknown". Write "unknown" rather than guess; "same as
macOS" is fine when it is true.

The worked example is [codex-cli.md](codex-cli.md). The other sheets so far are
[claude-code-cli.md](claude-code-cli.md), [hermes-cli.md](hermes-cli.md) and
[amp.md](amp.md); Amp's is a draft written by Goal Cup for the Amp team to correct, and is
named after the harness because no connector id exists yet.

The `Status:` line takes one of three values:

- **shipped connector**: `connectors/<id>/` is on main, or is added by the same PR as the sheet.
- **core ready, connector pending**: no row of the
  [decision table](../../skills/adding-a-capture-harness/decision-table.md) needs a core PR,
  but no connector exists yet.
- **facts only**: the tracer needs a core change first, or a key fact is unknown. The sheet is
  the whole contribution for now.

Section 8, "Things Goal Cup cannot isolate", lives only in the sheet. A connector PR links the
sheet in its description rather than repeating it.

## How to update one

When a release changes something a sheet says (a path, a variable, the session format, an auth
flow, a host), open a PR that edits the sheet and bumps "Facts for version" and "Last checked".
If the harness has a connector, change the connector in the same PR and follow the
[skill](../../skills/adding-a-capture-harness/SKILL.md) again; a connector pins one exact binary,
so a new version always needs a connector change. If nothing in the sheet changed, a one-line PR
bumping "Last checked" is still welcome.
