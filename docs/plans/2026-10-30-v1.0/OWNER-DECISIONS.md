# Owner decisions for goalcup 1.0 (the ledger)

Written 30 Oct 2026, 02:00 UTC, from the planning conversation of 29 and 30 Oct. This is the source of truth
for what was agreed. Every PRD, the build prompt and every builder follow it. Where a PRD, the cut-first plan
or an older doc disagrees, this ledger wins. Where this ledger disagrees with VISION.md, raise it with the owner.

Status words: **locked** (decided, build it), **default** (taken by the planner, the owner did not object;
overridable), **open** (needs the owner).

## 1. What we are building

| # | Decision | Status |
|---|---|---|
| L1 | Ship a new version, **goalcup 1.0** (today 0.7.0), in one big build. Not a patch. | locked |
| L2 | Judge everything against the seven points in VISION.md. Less code. Cut before you add. | locked |
| L3 | **Fresh core in this repo.** One new recorder crate written from the new format spec. Copy in proven tricky parts on purpose (CA and TLS setup, brotli and WebSocket deflate decoding, HTTP parsing, masking rules and their test corpus, connector data, the Worker chunk store). Delete the old crates in the same build. Not a new repo; not editing the old code in place. | locked |
| L4 | **Before any old crate is deleted**, the lessons list (PRD-17) is written: every hard-won edge case in the old code, tests and DECISIONS.md, each turned into an acceptance case the fresh core must pass. | locked |
| L5 | **One process (option B).** The goalcup process holds the proxy, the session-file reader, masking, the writer and the uploader. The hand-off from the proxy to a separate daemon (IPC, producers, epochs, the shared queue, failure budgets) is gone. That hand-off is what failed in run 1. | locked |
| L6 | **All planned cuts are in:** research decoders (except the model and usage parts PRD-09 keeps), research scripts, disk-full handling, pins, deny lists, termination proof, signing, the separate daemon, legacy and test seams, retired versions. | locked |
| L7 | **No backward compatibility.** Every run so far is test data. Protocol, verifier, Worker and client change together. | locked |

## 2. Recording rules

| # | Decision | Status |
|---|---|---|
| R1 | **Record everything the harness and every subagent send and receive** to model hosts: every token requested or produced. The output is research on what agents did, so more is better. Terminal capture is out of scope. | locked |
| R2 | **Never block or slow the harness.** No backpressure on the wire. | locked |
| R3 | **The recorder never stops for good.** No failure budget. If the recorder fails, it restarts itself on the same port fast, writes a gap for the dead time and marks the failure. A tiny watchdog that only restarts the recorder is allowed. The harness is never blocked. | locked |
| R4 | **No silent loss.** Every loss writes a gap with a start, an end, a reason and lost bytes where known. | locked |
| R5 | **No caps of our own.** Only stated memory guards that never stop recording: past one, write a gap and go on. No native stream cap, no file cap, no per-run size limit on the server. | locked |
| R6 | **Counts are written counts.** A model call counts only once its bytes are in the record. The participant, status, heartbeat and server see written counts and how long recording was off. | locked |
| R7 | **Heartbeat says `failed`** when recording is off. Email about it is out of scope; the server can mail later. | locked |
| R8 | **Record raw, interpret later.** The recorder never parses for rules or stats. | locked (AGENTS.md) |

## 3. One system for every harness

| # | Decision | Status |
|---|---|---|
| H1 | **One proxy for every connection, no per-harness patchwork.** | locked |
| H2 | **One shared model-host list** for all harnesses: providers, gateways (Vercel AI Gateway, OpenRouter, Cloudflare AI Gateway), cloud model services, vendor backends (Amp, Cursor). Plus hosts the participant registers. Plus hosts the harness config names, as a hint, not the rule. These are decrypted and recorded byte for byte. | locked |
| H3 | **Every other connection** is relayed untouched and recorded as a tunnel (host, bytes, times). A long streaming tunnel to an unknown host is flagged. Do not decrypt every host: tools that do not trust the run CA would fail. | locked |
| H4 | **Gateways are just another HTTPS address.** With or without a gateway, the same system records it. | locked |
| H5 | **Proxy dodgers** (Amp, Cursor) fit the same system: record the conversation with the vendor's host, plus session files. Pinned certificates fall back to a recorded tunnel with a named gap. | locked |
| H6 | **HTTP/2 is supported** by the proxy. | locked |
| H7 | **Per harness, data only:** CA env vars to set, the harness's own proxy setting if it overrides HTTPS_PROXY, background daemons to stop or bypass before start, session-file patterns, the auto-update switch. | locked |
| H8 | **Session files** are read the same way for every harness, with no caps, and the file path is the ID. | locked |
| H9 | **Auto-updates off** where the harness allows it; the pad records the version. | locked |
| H10 | **Opening vendor hosts** (Amp, Cursor) by default under the one list. | default |

## 4. Format, compression, masking, upload, server

| # | Decision | Status |
|---|---|---|
| F1 | **New record format, the best one:** one recording per WebSocket direction, raw bytes, light framing, gaps with start and end, written counts, off spans. | locked |
| F2 | **Compression: zstd with a long window** (`--long=27`): run 1's real trace was 4 times smaller. | locked |
| M1 | **Masking:** keep every secret rule; add the URL password rule; drop the email rule and the Codex rate-limit rules; add a left word boundary (a key typed right after a letter or digit may be missed: accepted). Masking happens before any byte reaches disk. | locked |
| U1 | **Upload** often and in small chunks; retry backoff resets when the server's offset moves and is capped short. Prune a local part once the server confirms it stored it. | locked |
| S1 | **Verifier and Worker** move to the new format in the same build and deploy together with the client. | locked |
| S2 | **Usage and research stats:** model per call, tokens (input, cached, output, reasoning), status, and per subagent which models it used, with calls and tokens. Worked out after upload from the record and the harness's own session files, never by the recorder. | locked |
| S3 | **No website database schema migration** overnight. The new Worker fits today's tables; anything new goes in R2. Agents may create a separate R2 bucket for test runs. | locked |

## 5. Harnesses and the test pad

| # | Decision | Status |
|---|---|---|
| T1 | **15 harnesses:** Claude Code, Codex, Hermes, OpenCode, OpenClaw, Amp, GitHub Copilot CLI, Gemini CLI, Goose, Kilo CLI, Qwen Code, Crush, Factory Droid, Kimi Code, Cline CLI. Cursor and Aider are out; Cursor stays a documented dodger case. | locked |
| T2 | **Keys:** Claude Code uses the Claude key. Codex uses the OpenAI key. Every bring-your-own-key harness uses DeepSeek, or the cheapest current OpenAI model to spread rate limits. Gemini CLI gets one funded Google key. Copilot CLI: try the DeepSeek key. No keys this cycle for OpenRouter, Amp or Factory: those ship "not verified", built from their docs. | locked |
| T3 | **Tiers:** "verified" (passed the pad on a named version) and "not verified". A harness that ignores HTTPS_PROXY or the CA is "not supported", never hacked around. | locked |
| T4 | **Test pad:** one tool, three cases (direct key, through a gateway, a dodger). Checks: wire calls match the harness's own log, every subagent recorded, no secret in the record, server copy whole, usage stats present. One pass or fail table. It is also how contributors add a harness. | locked |
| T5 | **Pad workload:** about 20 minutes of real work. The prompt asks the agent to build a payments platform (like PayPal, under a made-up name) with admin and user accounts, an admin frontend and a user frontend, and to use many subagents in parallel. Subagents are asked for in the prompt, never forced by config. | locked |
| T6 | **Spend cap:** 10 USD per run. Cost is otherwise not a constraint; the owner funds the keys. | locked |
| T7 | **Implementers double-check the vendor's current docs** before building any harness-specific part. | locked |
| T8 | **Release blockers:** Claude Code, Codex and Hermes must pass the pad. The others may ship "not verified". | default |
| T9 | **macOS** is tested later by the owner. This build is tested on Linux. | locked |

## 6. How the build runs

| # | Decision | Status |
|---|---|---|
| B1 | **Builds, compiles, tests and real runs happen in Vercel Sandboxes.** The cloud coding environment is for code, review and orchestration only. | locked |
| B2 | **Several workflows run in parallel** in the same session (one per lane), to get past the per-workflow agent limit. Heavy subagent-driven development. Per PRD: a planner writes the task plan from the PRD and the code; builders work in parallel worktrees inside the PRD's owned files; **at least two independent reviewers** read each diff (one for correctness, the seven points and masking; one for code quality and less code); fix; merge. Separate cloud sessions are a fallback only. | locked |
| B2a | **Replan after each PRD.** When a PRD merges, an improvement agent reads what was learned and updates the remaining PRDs and the build plan, so later work uses real findings. A change to a locked decision in this ledger still needs the owner. | locked |
| B3 | **Short tests during the build:** prove recording works; one short real test after each new core feature, starting as soon as the core records end to end. **Mass testing of all harnesses comes at the end**, with watcher agents that fix and rerun (stop after three failed fixes on one cause). | locked |
| B4 | **Staging only.** All runs go to the staging Worker, approved with the staging test approval key and the owner's member id. Never add an approval bypass to production. | locked |
| B5 | Agents may **deploy the Worker to staging** on their own. Production deploy and the merge to main are the owner's. | locked |
| B6 | **Integration branch only.** Nothing merges to main without the owner. | locked |
| B7 | The build **starts in the morning** after the owner wakes and the keys are in place. | locked |
| B8 | **Autonomy:** agents decide anything inside the seven points, AGENTS.md and this ledger. They stop and leave a note for a hard-rule change, the website database, production, or spend past a cap. | default |

## 7. AGENTS.md hard rules for 1.0

All hard rules in AGENTS.md stand, with these changes the owner agreed:

| Rule today | In 1.0 |
|---|---|
| Never set a model base URL on any harness | **Stands.** The test pad configures a harness the way a participant would (for example pointing it at DeepSeek); goalcup itself never does. |
| Records and never manages the agent; exceptions D65 and D66 | **Stands without D65.** No disk-full freeze (disk full: no special handling). D66 (`goalcup time-left`, pull only) stays. |
| No harness names in core code | **Stands.** |
| Never write into `~/.claude`, `~/.codex`, `~/.hermes` or app-wide config | **Stands.** |
| Connector edits regenerate the pins | **Goes with the pins** (L6). Connectors are embedded in the binary instead. |
| Never commit `spikes/*/out/`, `harnesses/`, `target/` | **Stands.** |
| Record raw, interpret later (the daemon) | **Stands,** reworded for the recorder: the recorder captures bytes; the verifier and the usage reader judge them. |
| Never delete a test in the promises list P1 to P7 | **Replaced** by the seven points: a test stays if it proves one of them. |

## 8. Out of scope for 1.0

Email changes, terminal capture, macOS testing in this build, production deploys by agents, website database
schema changes, keys for OpenRouter, Amp and Factory, Cursor and Aider support.

## 9. Open

| # | Question | Default if no answer |
|---|---|---|
| O1 | Does staging share the website's real database? | Assume yes: no migrations, read-only use. |
| O2 | Does Copilot CLI need a Copilot subscription even with its own key? | Try the key; "not verified" if it fails. |
