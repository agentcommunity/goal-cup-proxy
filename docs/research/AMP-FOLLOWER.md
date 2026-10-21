# Amp thread follower: do we need it?

Plain-English note, 21 Oct 2026. Question: does Goal Cup Proxy need the "JSONL thread follower" an Amp
engineer offered (Balazs's 8-point spec, WIP orb thread `T-44eaf3a8-...`, contact @nicolay)?
The orb thread is private (it asks for an Amp sign-in), so this note is built from our own
measurements in [amp-cli.md](../harnesses/amp-cli.md).

**Answer: not for the event. Nice to have after it.** Amp can ship on 7 Nov with what we have.

## 1. What we get today, without it

- **Wire leg (the content).** Goal Cup Proxy records Amp's thread WebSocket. Decoder
  `amp-thread-jsonrpc` v1 recovers assistant text, tool calls with input, tool results the model
  saw, and model and usage. That is the same content the follower would write.
- **Native leg (counts only).** Amp's own log `~/.cache/amp/logs/threads/T-*.log` has transport state,
  message types, frame sizes and tool runs, but almost no conversation text. We capture it, label it
  "metadata, no transcript", and leave the cross-leg check unchecked for Amp.
- **Not seen today:**
  - the provider request and system prompt (Amp's server builds them);
  - `/api/internal` calls (`getThreadTail` history on a continue, the orb goal text);
  - a possible second user-level WebSocket (not measured);
  - subagent threads (`Task`, `oracle`, `librarian`, `finder`): we do not know yet if they use the
    same WebSocket.
- **What this proves:** what the agent said and did, as Amp's server sent it. **What it cannot
  prove:** that a second source agrees. For Claude Code and Codex we compare the wire with a native
  transcript. For Amp we cannot.

## 2. What the follower would add

| Gain | Event (7 Nov) | Research later |
|---|---|---|
| A native transcript to cross-check the wire, like Claude and Codex | Nice, not blocking | Yes |
| Subagent and child thread coverage, if the wire misses it | Only if the wire misses it (unknown) | Yes |
| Survives a change to Amp's WebSocket format | Low risk inside 2 weeks with pinned versions | Yes |
| Simpler decoding than the snapshot decoder | No | Yes |

The only gain that could matter for the event is the cross-check. It is anti-cheat value, not
something a run needs to be recorded or judged.

## 3. Which spec items we can drop

| # | Item | Keep or drop |
|---|---|---|
| 1 | One file per thread, `T-<uuid>.jsonl` | Keep. Must sit inside the run's harness home so our native pattern catches it |
| 2 | Append-only, flushed per final message | Keep |
| 3 | Final user, assistant, tool call, tool result, model, usage, time | Keep |
| 4 | `thread_id`, `schema_version` | Keep. Server `seq`: nice, not needed |
| 5 | Same file for `--execute` and `-ox` orbs | Drop orbs. Orbs run in Amp's cloud, which the rules already ban. Local CLI only |
| 6 | Reconnect from last `seq`, gap lines | Drop for now. Our own chain records gaps and the wire still has the stream |
| 7 | Child threads in the parent file, tagged | Keep the tag. Separate child files are fine too, a `T-*.jsonl` glob catches them |
| 8 | No credentials, no system prompt | Keep |

The minimal ask: items 1, 2, 3, 4 (without `seq`), 7 and 8, local CLI only.

## 4. Recommendation

- **Ship Amp for 7 Nov as it is:** wire decoder plus metadata log, honestly labelled "no native
  transcript, cross-check unchecked".
- **Adopt the follower before the event only if** it is in a released Amp build by about 1 Nov.
  Then it is a manifest change (native pattern), regenerated pins and one measured run, all before
  the 6 Nov 12:00 UTC connector cut-off. Anything later waits until after the event. A rushed swap
  risks a broken Amp connector on the day, which is worse than a missing cross-check.
- **Ask now, cheap and useful either way:** do subagents and child threads go over the same thread
  WebSocket? If not, Amp runs that use them have a blind spot, and RULES should say so.
- **RULES:** name Amp orbs (`-ox`, `--executor orb`) and Amp's Claude Code mode as not allowed. The
  first is vendor-cloud execution, the second is a sub-harness (D10).

Blind spots in this note: we have one macOS sample of Amp, the orb thread is unread, and subagent
traffic is unmeasured. If @nicolay says subagents leave the WebSocket, the "not needed" answer
weakens for runs that use them.

## Draft reply to @nicolay

> Thanks a lot for this, and thanks to Balazs for the spec. We checked, and we can run Amp at Goal
> Cup on 7 Nov without the follower, because we already decode the thread WebSocket. After the
> event we would love a small version: local CLI only, one append-only JSONL per thread, final
> messages with `thread_id` and `schema_version`, child threads tagged, no credentials. We do not
> need orbs, server `seq` or gap lines. One quick question that matters now: do subagents (`Task`,
> `oracle`, `librarian`) and child threads stream over the same thread WebSocket as the parent?
> No rush on the rest.
