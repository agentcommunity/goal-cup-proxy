# Draft: report to the Hermes team (not sent)

A draft for the owner to read and, if they agree, post on the Hermes repository
(`NousResearch/hermes-agent`). Nothing has been sent. Two findings, both seen on Hermes 0.22.5
in our test runs, from the recorded requests (no participant data). Post them as two separate
issues if the team prefers.

---

**Title:** Tool-call ids change when the conversation history is sent again

**What we see.** When Hermes sends a request, it resends the earlier turns as history. For turns
with parallel tool calls, the `tool_call` ids in that resent history are not the ids the model
returned. They are renamed. In one 20-minute run this happened on 43 turns. In one more turn, a
single call that carried several tool calls came back split into separate calls.

**Why it matters.** Anything that follows a conversation across requests by tool-call id loses the
thread: tracing and recording tools, cost tools, and the provider's own prompt cache, which only
helps when the resent prefix is the same bytes. Most agents resend the ids exactly as the model
returned them.

**Expected.** Each tool call keeps the id the model gave it, in every later request.

**Version.** Hermes 0.22.5, DeepSeek provider, both direct and through an OpenAI-compatible
gateway. We can share the request shape (ids and roles only, no content) if useful.

---

**Title:** Subagent requests carry no link to the parent session outside `state.db`

**What we see.** When Hermes delegates work to subagents, each subagent's model requests carry
nothing that ties them to the parent conversation: no header and no field in the request. The only
place the parent link exists is the session tables in `state.db`. The delegation `manifest.json`
and the `task-<n>.log` files show that subagents ran, but not which session started them.

**Why it matters.** Tools that observe an agent from outside (cost per task, tracing, recording)
cannot tell which subagent belongs to which parent without reading Hermes's private database.

**Suggestion.** Write the parent session id into the delegation manifest or task log, or send it as
a request header (for example the session id and parent session id that several other agent CLIs
send).

---

Our own fix for the first finding is in our verifier: it matches calls by content as well as by id.
So this report helps other tools; it does not block us.
