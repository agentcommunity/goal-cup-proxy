# Windsurf Cascade capture research — current-surface preflight

Status: official documentation and release research only, 2026-09-10. No
installer, application, account, model, hook, transcript, keychain or local
Windsurf state was used.

## Answer first

A current local Cascade experiment is **not feasible**. The official stable
[Devin Desktop changelog] identifies **v3.9.19**, released 2026-09-08, and says
“Cascade has been removed.” Devin Local is now the only agent in Devin Desktop;
the release directs existing conversations to a migration flow.

Current official documentation still publishes Cascade pages. Those pages are
useful evidence about the removed surface and a possible future reintroduction,
but they do not override the stable release note. No assertion was run and this
preflight assigns no support tier.

## Product and evidence boundary

The former target was **Cascade inside the local Devin Desktop editor**, which
was previously branded Windsurf. It was an editor panel with Code and Chat
modes, tool calls and editor/terminal context ([Cascade overview]). It was not a
standalone Cascade CLI.

The following are separate products or surfaces and cannot substitute for it:

- **Devin Local** is the current primary local agent in Devin Desktop and shares
  its harness with Devin CLI ([Devin Local]). Its sessions, hooks and subagents
  would need their own research.
- **Devin CLI** is the CLI harness shared by Devin Local, not a Cascade CLI.
- **Agent Command Center**, remote/cloud Devin sessions and SDK/API clients are
  distinct orchestration surfaces.
- **Windsurf Plugins**, including the JetBrains Cascade pages, are editor
  extensions with different host and state boundaries.

Evidence classes remain separate:

- **Stable release metadata:** the changelog is the authority for the current
  v3.9.19 application and removal of Cascade.
- **Mutable documentation:** the still-published Cascade pages describe the
  legacy feature. Several pages now call it the “legacy Cascade agent.”
- **Public runtime source:** none was identified in the reviewed official
  material. Documentation is not an implementation-level wire contract.
- **Executable evidence:** none; v3.9.19 was not downloaded or run.

There is therefore no current bundled Cascade engine version to record. The
official docs describe Devin Local as a server-fetched bundled agent, but that
is a different harness and is not evidence about Cascade.

## What the legacy documentation establishes

This section records candidate behavior only. Every point remains unmeasured on
an executable, and none makes the removed agent available in current stable.

The [Cascade hooks] page describes callbacks merged from system, user,
workspace and optional cloud configuration. Pre-hooks could block; post-hooks
were asynchronous. Restricted Mode disabled hooks and agents. Common identity
included `trajectory_id`, `execution_id` and mutable `model_name`, but no
canonical workspace.

Its transcript callback supplied sensitive, full-history JSONL under app-wide
`~/.windsurf/transcripts/`, using mode `0600`, a changeable schema and a
100-file cap. The docs do not define relocation, append/replacement behavior,
partial recovery, compaction, exact resume, stable model IDs or workspace
authorization. Both output forms derived from trajectory data. Completeness,
ownership, independent legs and foreign-conversation exclusion remain unproved.

The [Cascade overview] separately describes editor/terminal context, a
background planning agent, up to 20 tool calls per prompt and continuation.
It does not provide durable child lineage or byte-preserving resume semantics.
[Cascade Arena Mode] created separate model sessions and worktrees, rather than
a documented same-session subagent relationship.

Current setup documentation requires a Devin account and offers manual API-key
login ([getting started]). It names `~/.codeium/windsurf` as application config,
but no reviewed page provides an isolated profile, credential relocation,
Cascade-only subscription mode or dedicated Cascade process tree. The macOS
editor terminal also loaded the user's zsh configuration ([terminal]). These
are concrete unknowns, not permission to inspect real user state.

## Measurement decision

There is no bounded current-stable experiment to run. Installing an old build
would measure an unsupported historical release rather than the current
product, and substituting Devin Local, Devin CLI, a plugin or a remote agent
would measure another harness.

The current-stable removal already falsifies feasibility. Reconsideration needs
an official supported stable release that exposes Cascade again. Before any
model request, its official controls would also have to support all of these:

1. an isolated application profile/state and credential boundary that does not
   adopt or mutate real user state;
2. an authenticated folder/process-scoped hook that binds one canonical
   workspace and exact trajectory before any transcript read;
3. an observable dedicated process lifetime that the spike can terminate and
   wait without killing unrelated editor work; and
4. a local raw surface whose retention, replacement and resume behavior can be
   measured without scanning an app-wide store.

Absence of any one control is an early refusal, before login or model traffic.
If all become available, a new version-pinned preflight can define the smallest
isolated eleven-assertion run. This document does not authorize that run.

## Official sources

Accessed 2026-09-10:

- [Devin Desktop changelog] — current stable version and Cascade removal.
- [getting started] — current product name, application launch, login and
  config-directory information.
- [Devin Local] — current default/primary local agent and its distinction from
  Cascade and Devin CLI.
- [Cascade overview] — removed editor-local Cascade behavior, UI context,
  model selection and continuation limits.
- [Cascade hooks] — legacy callback scopes, payloads and transcript file.
- [Cascade worktrees] — legacy app-wide worktree location and cleanup.
- [Cascade Arena Mode] — separate-session parallel model behavior.
- [terminal] — editor terminal and macOS zsh behavior.

[Devin Desktop changelog]: https://docs.devin.ai/desktop/changelog
[getting started]: https://docs.devin.ai/desktop/getting-started
[Devin Local]: https://docs.devin.ai/desktop/devin-local
[Cascade overview]: https://docs.devin.ai/desktop/cascade/cascade
[Cascade hooks]: https://docs.devin.ai/desktop/cascade/hooks
[Cascade worktrees]: https://docs.devin.ai/desktop/cascade/worktrees
[Cascade Arena Mode]: https://docs.devin.ai/desktop/cascade/arena
[terminal]: https://docs.devin.ai/desktop/terminal
