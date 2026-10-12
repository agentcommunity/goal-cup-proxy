# Vision

> **Scope (2026-10-02).** V1 on `v1/trunk` is one short Codex CLI/macOS lifecycle; V1.1 expands it;
> the 168-hour event is untested. On `m2/wire` the recorder (native leg plus proxy wire leg) is built
> and measured on short rows, not qualified. Current: [TODO](../TODO.md), [ARCHITECTURE](ARCHITECTURE.md).

## The event

**One Prompt. One Agent. 168 Hours.**

One person gives one coding agent one prompt, then leaves it alone for a week. The agent may plan, write skills, spawn subagents, buy compute, deploy, and surprise us. At the end there is a fresh, public, MIT-licensed product, or there is not. Judges score the product. Nobody scores the trace.

The Cup is two things at once: a competition, and a global snapshot of what autonomous coding agents can actually do in Q3 2026, across every harness people really use. Prompts, products, failures, and lessons are published afterwards so the agent community learns from the whole field, not from the winner alone.

## Why a tracker exists

The single rule that makes the experiment meaningful is that no human helps after the prompt. That rule is also the easiest one to break quietly: a nudge in the chat, a file edited by hand, a ticket written into a board the agent reads. If the rule cannot be checked, the results mean nothing.

So participants run an open-source tracker. It records the declared evidence surfaces for the
bound entry and ships that bounded record privately to the organizers. After the event,
compliance is decided from that record, with automated checks first and a human making every
disqualification decision. Traces are never public.

## What the tracker must guarantee

1. **Bounded capture.** The declared transcript and hook evidence for the bound entry is
   captured, including the tool calls, results, subagent activity, and model identity those
   surfaces expose. Coverage is bounded by the supported harness and its declared evidence;
   unavailable or incomplete evidence is a limitation or finding, not an inferred record.
2. **Provenance of human input.** Every message a human typed is identifiable as such, so the check "only the original prompt, then only `keep going`" is mechanical.
3. **Tamper evidence.** A participant controls their own machine. We cannot stop them editing a
   file or promise to observe every edit; declared evidence, artifact attribution, gaps, and
   capture failures make detected changes reviewable through the hash chain and acknowledgements.
4. **Scope.** The tracker captures the bound session and workspace. It may read declared shared
   harness stores only to identify the bound partition and discard foreign records before
   persistence; it does not retain ordinary sessions, other terminals, browser history, or
   unrelated machine activity.
5. **Silence.** The tracker never speaks to the agent. No reminders, no nudges. The agent may ask how much time is left; that is the builder's problem to solve in their prompt.
6. **Expandability.** Nine harnesses were measured and three CLI connectors exist in the M1 code. V1 defines Codex CLI as the reference qualification surface without deleting the other candidates. V1.1 promotes additional CLI and Desktop surfaces against the same lifecycle and artifact contracts; M2 may later make launch-mode additions smaller through wire/PTY evidence.

## What it is not

Not an API proxy that changes where the harness talks or what it sends. Not a sandbox. Not device
surveillance. Not a judge. Current V1 uses bounded transcript/hook evidence; a future M2 passive
loopback path, if implemented, remains process-scoped and disclosed. It is a recorder with a
public verifier, and the honesty of the whole event rests on stating its evidence boundaries
plainly.

## Principle that decides most arguments

**Record raw, interpret later.** The tracker captures the declared raw evidence bytes within each
harness's bounded capability. Every judgment about what those bytes mean is made afterwards, in
open-source code anyone can run, where a mistake is a re-run instead of a lost entry.
