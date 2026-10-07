# Is there one capture layer that works for every harness?

Research date: 2026-09-10. Status: research answer, not a decision. Nothing here is implemented. The four included public research notes with sources and Verified / Inferred / Unknown labels are in `universal-capture/` (starting with [`wire-proxy.md`](universal-capture/wire-proxy.md)): `wire-proxy.md`, `pty-and-os-layers.md`, `client-protocols.md`, `standards-and-witnesses.md`. The historical Spike W report and raw output are omitted from this public cut; the measured synthesis below is the record available here, not a claim that the private rerun bundle is shipped.

## Measured on 2026-09-10

Spike W ran both layers on five launch-mode CLI harnesses: Claude Code CLI 2.1.259,
Codex CLI 0.153.0, OpenCode 1.18.27, Hermes 0.19.0 and OpenClaw 2026.8.2, each through a
process-scoped mitmproxy and a tracker-owned pty, with no hook leg anywhere.

Subscription authentication survived on all five, with exactly one credential hash per
run on the model path and no refresh, no re-login and no token endpoint touched. The
wire was a strict superset of every harness's own store, catching calls no transcript
records at all: Claude Haiku classifier and quota probes, a Codex title-generation call
on a thread with no rollout file, OpenCode's title generator, Hermes' title and
security-approval calls, and OpenClaw's deleted subagent and background "dreaming"
sessions. Where a comparison exists the wire is the only byte-faithful record, and
Claude Code's own `OTEL_LOG_RAW_API_BODIES` is not: 0 of 23 responses matched strictly,
it redacts thinking, adds SDK defaults, blanks the billing checksum and in one case
rewrote a Bash command before logging it. The pty paired every human turn with typed
bytes plus Enter on all five and left every harness-authored user-slot record unpaired,
with one named partial (OpenCode's subagent prompt, which is exactly the record the leg
exists to catch). A Seatbelt profile denying all egress except the proxy port worked on
Claude Code and Codex, and kill plus resume survived on all five.

The measured limits preserved in this record are that the Codex WebSocket is stateful, tool
subprocesses fail TLS silently unless the generic CA variables are also set, and non-model
traffic dominates the capture on two harnesses. The historical evidence identity is
`spikes/wire-pty/REPORT.md` (omitted, with its raw output); no private report or evidence path
is required or claimed present. For overlapping M0 harness facts, the included
[CAPTURE-RESEARCH synthesis](../CAPTURE-RESEARCH.md) and its linked redacted fixture inventories
are the direct public record; they do not recreate the omitted Spike W wire/pty run.

## The question

The M0 design tails nine private on-disk formats and installs nine hook dialects, and the manifest grew fifteen fields to cope with them (CAPTURE-RESEARCH 5.6). Is there a layer below or above the harness that would give one capture path for all of them?

## The short answer

Not one layer for all nine. Two layers for the six launch-mode CLI harnesses, with almost no per-harness code, and the three that cannot be reached by any universal layer are exactly the three that are already `elevated_review` for other reasons.

The two layers are the ones the harness cannot avoid: the **wire** (the HTTPS request it sends to the model provider) and the **terminal** (the pty the human types into). Both are process-scoped, both need no hook, no plugin, no transcript parser, and both are things the current design deliberately did not look at. D1 rejected "an API proxy" because a base-URL proxy breaks subscription auth. A forward proxy set through `HTTPS_PROXY` leaves the base URL alone, so the auth objection does not apply to it, and both Anthropic and OpenAI document this exact mechanism as supported for their subscription clients.

## What each candidate layer gives, honestly

### 1. The wire: process-scoped `HTTPS_PROXY` plus a per-runtime CA env

Launch the harness with `HTTPS_PROXY=http://127.0.0.1:<port>` and the CA-trust variable its runtime honours (`NODE_EXTRA_CA_CERTS` for Claude Code, OpenCode, OpenClaw, Gemini; `CODEX_CA_CERTIFICATE` plus `SSL_CERT_FILE` for Codex; `HERMES_CA_BUNDLE` for Hermes, which measurement showed wins over all four standard names and is guarded by a start-up check that aborts on a bad bundle). **Measured correction:** the runtime-specific variable covers the harness and nothing else. On Claude Code, OpenCode and OpenClaw the agent's own `curl` inherited the proxy, reached CONNECT and then failed the handshake, leaving *no record at all*, because `NODE_EXTRA_CA_CERTS` is Node-only. The generic names (`SSL_CERT_FILE`, `CURL_CA_BUNDLE`, `REQUESTS_CA_BUNDLE`) have to be set too, and TLS is better terminated only for the provider hosts with everything else tunnelled. A tracker-owned proxy terminates TLS and records raw request and response bytes, including SSE streams.

| Harness | Works today? | Notes |
|---|---|---|
| Claude Code CLI | Yes, vendor-documented | Proxy and CA env are in Anthropic's network-config docs. Model calls are HTTP/1.1 SSE. Anthropic's proxy-marker code keys on `ANTHROPIC_BASE_URL`, which stays unset. Third-party MITM tools report Max auth working. |
| Codex CLI | Yes, measured | ChatGPT auth streams over a WebSocket to `chatgpt.com/backend-api/codex/responses`. Measured: 2,686 frames over 5 connections captured whole in both directions, **no HTTPS fallback and no mid-run transport switch**, and the connection is *stateful* (see below). Forcing HTTP works only through a new, differently-named `CODEX_HOME`-scoped provider with `supports_websockets=false`; the built-in `openai` provider is rejected outright, and the `responses_websockets` feature flag reads `removed`. |
| OpenCode | Yes, measured | Bun honours the env; `NO_PROXY=localhost,127.0.0.1` is required and sufficient. 29 of 29 model requests 200 on the unchanged credential, HTTP/1.1 SSE with no `Content-Type` header, so SSE must be sniffed from the body. |
| Hermes | Yes, measured | httpx honours the env and `HERMES_CA_BUNDLE` is the highest-priority CA variable. Nothing broke behind the proxy. Its Claude path reuses Claude Code's OAuth token, which is the pattern Anthropic bans regardless of how we capture. Separately, the wire caught the isolated home spending a host-level GitHub token nine times per session. |
| OpenClaw | Yes, measured, and only with `openclaw gateway` in the foreground | The default launchd Gateway inherits no env. Measured on a ChatGPT-subscription OAuth profile: no provider path bypassed the proxy, every persisted turn had a wire request, and the transport is a WebSocket rather than SSE. The `openai-completions` path that builds its own fetch was never taken on that route and remains untested. |
| Gemini CLI | Yes | One more envelope for the verifier: the OAuth path wraps `generateContent` in an internal `cloudcode-pa` request. |
| Cursor CLI and IDE | **No, and no method can** | The client speaks Cursor's protobuf protocol to Cursor's backend, which builds the prompt and calls the model. The model-visible bytes never exist on the participant's machine. |
| Claude Desktop | No | Chromium needs a keychain CA (system-wide, forbidden); the Code-tab subprocess drops the env. |
| Codex desktop app | No | LaunchServices passes no shell env; the Swift side follows system proxy; the app ignores `config.toml`. |

What the wire gives that nothing else does:

- The complete model input, exactly as the model received it: system prompt, full history, tool definitions, tool results. This is the strongest possible form of the "every token the agent saw" claim, and it closes the truncation asymmetry from M0 (D1): what the model saw is by definition what is on the wire. **Measured correction: "on every call" is true on HTTP and false on a WebSocket.** Codex and OpenClaw talk to `chatgpt.com/backend-api/codex/responses` over a stateful WebSocket where the *first* `response.create` on a connection carries the system prompt, the tool namespace and the whole history, and every later call on that connection carries only the new item, typically a 2 KB tool result with no system prompt and no tools (24 of 35 calls in the measured run). A wire capture of such a session must replay the connection from its first frame, or force the stateless HTTP shape at roughly 10x the request bytes.
- The model id on every request, and the provider's `request-id` / `x-request-id` on every response. Those ids are what both vendors use for support lookups, which makes them the only latent third-party witness that exists for consumer subscriptions (see [`standards-and-witnesses.md`](universal-capture/standards-and-witnesses.md) section 3d). No precedent exists for a vendor confirming them for a competition; it would be a sponsor conversation.
- Auxiliary calls (title generators, suggestion agents) that M0 found invisible to every transcript. They stop being a whitelist field and become captured traffic.
- Usage counts in one shape per provider instead of six harness shapes (D26).
- A mechanical `started_at`: the first outbound request whose user content is the goal. No arming signal, no read-at-start hook problem.
- Parsers per **provider** wire format instead of per harness: Anthropic Messages, OpenAI Responses (HTTP and WS), Gemini's envelope, OpenAI-compatible chat. These are public, versioned contracts; the on-disk formats are private and drift weekly ([CAPTURE-RESEARCH §6.1](../CAPTURE-RESEARCH.md#61-the-version-pin-caveat)).

What it does not give:

- Raw chain of thought. Not a proxy limit: Anthropic returns summaries or an empty block with a signature, Codex and OpenCode use `store:false` with encrypted reasoning, Gemini returns thought summaries. Same limit as today (SYSTEM-PLAN 10).
- Enforcement by itself. An env var can be unset. See the Seatbelt note below.

Costs and risks:

- The captured headers carry the participant's OAuth bearer token. The tracker must strip `Authorization`, `x-api-key` and cookies before hashing or uploading, or the organizers become the "collect or store credentials" case in Anthropic's policy. This is a hard requirement, not a nicety.
- Terms. A passive proxy on your own machine, with an unmodified binary and an unchanged base URL, is the mechanism both vendors document for corporate TLS inspection, and it is distinct from the 2026 enforcement against consumer OAuth tokens in third-party harnesses. Still, VISION.md says "Not a proxy" and [D1](../DECISIONS.md#d1-no-api-proxy-the-harnesss-own-transcript-is-the-evidence) says the same, so this is a reversal to make deliberately, and worth a written yes from Anthropic and OpenAI as sponsors.
- Nobody has published a 168-hour run of a subscription client through a local TLS-terminating proxy. The two concrete unknowns: whether Codex's WS-over-CONNECT stays on one transport for a week, and whether the proxy's upstream TLS fingerprint (mitmproxy or Go versus Bun or rustls) or Anthropic's client-integrity checks ever reject a long session. The transport question is now answered for a short run (no fallback, no mid-run switch); the week-long and fingerprint questions are not, and a 168-hour soak is still the follow-up. Bun's `fetch` CONNECT body-corruption bug (oven-sh/bun#30381) did not reproduce: the two Bun harnesses measured, Claude Code and OpenCode, reconciled byte-exactly against their own stores, and the only Claude Code mismatches were rewrites the harness itself made before logging.

### 2. The terminal: a tracker-owned pty as the human-input witness

The tracker opens a pseudo-terminal, runs the harness inside it, forwards bytes verbatim in both directions, forwards `SIGWINCH`, and records raw input bytes with timestamps (asciicast v3 `i` events). It does not interpret TUI semantics live; reconciliation happens offline.

- Every harness that runs in a terminal already runs daily inside nested ptys (tmux, VS Code, ssh, asciinema). Reported breakages come from wrappers that interpret the stream (tmux mouse capture, zellij alt-screen), not from transparent byte proxies.
- Under the Cup rule the classifier is trivial. Any user turn that is not the goal or `keep going` is a violation regardless of the pty. A `keep going` with no matching typed bytes plus Enter is injected from elsewhere: flag. Typed bytes with no matching turn means the human typed into something nested (Claude Code `!` shell mode, Ctrl-G editor): flag or ban.
- This is exactly the distinction M0 could not make with hooks: harness-authored user-slot text has no pty bytes at all, so the exclusion catalogue in D18 mostly stops being needed.
- The screen-output stream is a cheap third leg. It uniquely shows what the human saw: `/rc active`, `← telegram …`, `› Message from @…`, permission prompts. Keep it compressed, index it only for triage.
- Bypasses exist and every one of them still lands in the harness's own session store as a user turn, so "turn present, no pty bytes" stays mechanical: Claude Code Remote Control, Channels, cross-session messaging (a raw Unix-socket injection API, on by default since 2.1.224, refusable folder-scoped with `crossSessionInbound: refuse`), background sessions, `--resume --force`; Codex desktop app or a second `codex resume` between turns; OpenCode's always-on HTTP server; Hermes gateway rebinding; OpenClaw, where the Gateway is the agent and the TUI is a client. Argv inspection and a process-tree check (tmux, gateways, supervisors, reparenting to pid 1) cover the launch-time ones.
- Desktop apps have no pty. Transcript-only witness there, as today.

Windows: ConPTY gives the same chokepoint by construction, with Claude Code's own ConPTY rendering bugs to document as not ours.

### 3. Making the wire mandatory: Seatbelt

`sandbox-exec` is deprecated in the man page but shipped, root-free, SIP-compatible and process-scoped, and both Claude Code's sandbox runtime and Codex's sandbox run on it. A tracker profile that denies all network except the loopback proxy port turns "the harness was asked to use the proxy" into "nothing left this process except through the recorder", plus a per-host egress log that helps D15 side-channel review. Measured on Claude Code and Codex: a full session completes inside the profile on the existing subscription token, with MCP servers booting and the agent's own direct `curl` refused while the proxied one returns 200. Two gotchas the spike found: `(allow network* (local ip "localhost:*"))` silently opens the entire internet, so the inbound-side operations must be named explicitly after a leading `(deny network*)`, and allowing `(remote unix-socket)` is what keeps DNS working through mDNSResponder while TCP stays blocked. Still open: the browser launch for OAuth from inside the profile, and keychain access some harnesses need. Linux equivalents are bubblewrap or Landlock; `pf`, Network Extensions, Endpoint Security and `tmutil` all need root or Apple entitlements and are out.

### 4. Tracker as the client (ACP, stream-json, app-server)

The participant's UX need is tiny, so the tracker could be the client and the harness the server.

- Claude Code `-p --input-format stream-json --output-format stream-json` and a tracker-spawned `codex app-server` over stdio are vendor-owned, complete (thinking summaries, full tool I/O, subagents, model, usage, compaction markers, permission events) and a genuine single-stdin chokepoint. Max auth works in `-p` and still draws from plan limits.
- ACP alone cannot cover the field: no model id or system prompt fields, raw tool output optional, subagents a draft, and Claude Code and Codex speak it only through community adapters that drop hooks and agent teams.
- OpenCode, OpenClaw and Hermes are multi-client servers by design, so the tracker is not a chokepoint there. Cursor headless suppresses thinking and reports no usage.
- Behaviour risk: Anthropic says `--bare` "will become the default for `-p` in a future release", which would silently strip hooks, skills, CLAUDE.md and subagents.
- Cost: the tracker has to ship its own UI, and the entry no longer runs the harness people actually use in the way they use it.

Verdict: a strong option for a strict tier on the two most-used harnesses, not the universal answer. The pty keeps the real UI and needs less code.

### 5. Tracker-built container or VM

Realistic for Claude Code and Codex on consumer plans (`claude setup-token` or paste-code login; `codex login --device-auth`, unless a workspace admin disabled it), on OrbStack or Lima with the Anthropic devcontainer firewall pattern. It closes the messaging, tmux and background-session holes by construction. It costs the GUI harnesses, macOS-native tooling, Docker-in-Docker friction and a week of VM uptime risk. Apple `container` is too young. Offer it as a strict tier, never the only tier.

### 6. Things that are not the answer

- **OpenTelemetry as the one leg.** Two harnesses push near-full content over OTLP (Claude Code, Gemini), OpenCode experimentally, Codex prompts only, OpenClaw bounded and redacted, Hermes via a third-party plugin, Cursor nothing. Push-only with no local spool, and entirely participant-editable: a self-report, not a witness.
- **A cross-vendor hook standard.** None exists. Claude Code's JSON-on-stdin dialect is the de facto shape (Codex copied it, Cursor and Copilot import it, Gemini shares the envelope), so one receiver plus a ten-line manifest covers six harnesses, with OpenCode and OpenClaw needing a small plugin. But hooks carry no assistant text, no usage, no raw bodies, they load at start, and they hot-disable. They are the layer causing the pain.
- **Provider-side witnesses.** claude.ai and ChatGPT exports omit local CLI sessions; local sessions are not synced; the Analytics API is org-admin only. Nothing a consumer subscription can hand over after the event.
- **Existing cross-harness recorders.** None converged. The one that captures raw bodies for fifteen harnesses (claude-tap) uses three mechanisms, including a forward proxy with a local CA for OpenCode, Hermes and Gemini, and a base-URL reverse proxy for the rest.

## What this would do to the design

If the wire and the pty become the two mandatory legs in launch mode, with Seatbelt making the wire unavoidable:

Fields that stop existing or shrink to one line: `arming.signal`, `arming.timeout`, `arming.recipe`, `start_signal.required`, `store[].cursor.type`, `unsafe_cursors[]`, `store[].partition_key`, most of `human_rule.exclusions[]`, `human_rule.byte_exact_field`, `normalisation.trailing_newline`, `hook_silence_policy`, `hot_disable`, `receiver.token`, `aux_calls[]`, `archive_sources[]`.

Fields that survive: the launch recipe and env pins, `kill.strategy` and `lock_paths[]`, version range, `workspace_exclusions[]`, the banned-flags list (now including `--bg`, `--channels`, `--remote`, multiplexers), and a process-tree policy.

The transcript tailer and the hook leg survive as optional corroboration on launch mode, and as the only legs for Claude Desktop, the Codex app and Cursor, which stay `elevated_review` or get dropped from v1. That is the honest shape of "works for all": the universal layers cover the tier that is already `full` plus Hermes, OpenClaw in foreground and Gemini, and bespoke code shrinks to the three entries that were already the weakest.

The verifier changes from nine harness parsers to four provider parsers plus one pty reconciler, and the conformance fixture for a new harness becomes: a launch recipe, one recorded session through the proxy and the pty, and a check that the provider format is one the verifier already knows.

## What it cost to find out

**Done on 2026-09-10, on five harnesses rather than two.** The measured facts are preserved in this record. Historical result identity: `spikes/wire-pty/REPORT.md` (omitted, with its raw output); no private result file is required or claimed present. The plan below is historical procedure, not a runnable public script: the referenced M0 script, pty recorder setup and Seatbelt profile are omitted.

1. Launch Claude Code CLI and Codex CLI inside a pty recorder (asciinema 3 with input capture, or `script -r`) with `HTTPS_PROXY` and the CA env pointed at mitmproxy. Assert: Max and ChatGPT auth survive; every request from the M0 script appears, including subagent and title-generation calls; Codex WS frames are captured or HTTP fallback is forced and stable; Bun's CONNECT body is byte-correct; every human turn in the transcript pairs with typed bytes plus Enter, and every harness-authored user slot has none.
2. Wrap the same launch in a Seatbelt profile that denies non-loopback egress and confirm login, MCP and tool network calls still work through the proxy.
3. Soak for as many hours as the day allows and watch for a transport flip or a provider rejection. A 168-hour soak is the follow-up if the day passes.

Two decisions only the organizers can make before building on it: reversing the "not a proxy" line in VISION.md and D1, and whether the desktop attach modes are worth keeping in v1 once they are the only thing the old machinery exists for.
