# Record everything we safely can (spec)

29 Oct 2026. Owner decisions from the 28 Oct audit, measurement and design council
(council papers and measurements: [ledger](2026-10-28-one-capture-system-ledger.md) L27 to L29
and the results file). The implementation plan is
[2026-10-29-record-everything-plan.md](2026-10-29-record-everything-plan.md).

## Why

Goal Cup Proxy is a research recorder. The traces are worth what they hold: every AI call and
answer, main agent and subagents, in full, from 10 to 15 harnesses and the custom ones
participants bring. The 28 Oct audit and measurement found that we still lose data on the way:
other paths on an opened host pass unrecorded (and a crafted base path hides every call),
headers are dropped, Claude's subagent transcripts are not captured (60% of its chat bytes),
Codex API-key users go unrecorded, and a gateway user can be refused for a host they never use.
Nothing has been measured past 2 hours.

## The rules (every part of the system follows these)

1. **Never set a model address, never handle keys.** The harness reads its own settings and keys;
   Goal Cup only records. A key the participant gives the harness goes in the harness's own
   settings in its Goal Cup home.
2. **Never block the harness.** Refuse only when recording is impossible: a plain-http or
   loopback model address. Everything else warns and records.
3. **Record everything we safely can.** On an opened host: every request and response, any path
   and method, full bodies, and all headers except credentials. Credentials are masked by name
   and by value (anything that looks like a key or token), in headers and in bodies
   (`access_token`, `refresh_token`, `client_secret`, `x-api-key`, ...).
4. **Opened hosts** are every model host the harness can use (listed in its connector) plus the
   one address its own settings name. The agent's own apps (code it writes that calls an AI API)
   stay sealed: they do not trust Goal Cup's certificate.
5. **Everything else passes sealed and is listed**: host, port, bytes each way, times, reason.
   Sign-in hosts always stay sealed.
6. **Chat files are the backup.** Every per-run session file the harness writes is captured:
   main session, every subagent session and its metadata, tool-output files. A harness database
   in Goal Cup's own home is copied at each segment end. One last read after the harness exits.
   A harness with no chat file is fine.
7. **Subagents are first-class.** Every subagent's AI calls are on the wire (same proxy) and
   every subagent transcript the harness writes is in the chain. A measured run proves the
   number of subagents equals the number of distinct conversations recorded.
8. **Never break the chain.** One sealed hash chain, every loss marked as a gap, protocol v5 for
   the event. New data (headers) goes in as new streams that older verifiers skip. The
   sealed-connection list stays a side file, read safely, until v6; raw bytes instead of base64
   also wait for v6 after the event.
9. **Store everything raw; optimise later on the server**: remove repeated history, clean for
   research, v6.
10. **Flag, do not judge, on the laptop.** The evidence label is a hint; suspicious things are
    flagged on the server afterwards.
11. **Each harness is data**: a connector file, a facts sheet and a measured run. Custom
    harnesses use the same shape.
12. **Prove it before the run.** The practice run says plainly that model calls were recorded,
    or what is wrong.
13. **Every message says what to do.**
14. **The server keeps harness, harness version and models used per run.** Harness id and
    version are already stored per entry (`entries.connector_id`, `entries.harness_version`); the
    models used come from the research report's per-decoder model counts.
15. **Smart transfer.** Compressed chunks stream during the run (zstd today); a server problem
    never stops the run.
16. **The proxy restarts itself** on the same port if it dies; `goalcup continue` after a machine
    stop; `goalcup status` always says the state and what to type.
17. **A harness ships only after a measured run shows its calls recorded**; during every run the
    "no model calls recorded" alarm catches the rest. Participants are told in RULES what a trace
    holds (everything the agent sent and saw, file contents and paths on their machine).

## Test keys and models (owner, 29 Oct)

| Key | For | Model, main and subagents |
|---|---|---|
| `OPENAI_API_KEY` (small budget) | Codex with an API key | GPT-6.1 Luma, low effort |
| `DEEPSEEK_API_KEY` | most harnesses | the latest DeepSeek flash (`-1017`), little thinking |
| `ANTHROPIC_API_KEY` | Claude Code with an API key | Haiku, low thinking |
| `AI_GATEWAY_API_KEY` | quick small tests only | free gateway models |

Keys live only in the cloud environment's variables, are copied into a sandbox as 0600 files,
and go into each harness's own settings there; never in argv, output or a commit. The 168-hour
rehearsal runs on Tenki with a funded key, restarting with `goalcup continue` if the box stops.

## Out of scope for 6 Nov

v6 (raw bytes), the server flag engine, the heartbeat-head check, the stitcher, smart transfer
beyond today's zstd, terminal recording, decrypting non-AI traffic.
