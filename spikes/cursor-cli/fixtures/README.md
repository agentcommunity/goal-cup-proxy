# Fixtures — spike 9, cursor-cli (launch), run 2026-09-03

Redacted copies of the artefacts from spike run `run-20260903T125448Z`
(cursor-agent **2026.09.02-c22c1a3**, isolated install under `harnesses/cursor/`,
`CURSOR_CONFIG_DIR` + `CURSOR_DATA_DIR` + `NODE_COMPILE_CACHE` pinned under
`harnesses/home/`, macOS 26.5.2 arm64, subscription auth). Tracked chat
`f0fb67b1-ae44-4b4f-bff7-17d0d3fc502f`, subagent chat
`cdfaa7dd-8f5f-4f2e-9f16-b437481ca5cc`, A9 chat
`01570840-9801-4b6a-9fc9-3f3307945a53`.

## Files

| file | sha256 (first 16) | bytes | what it is |
|---|---|---|---|
| `leg1-store-blobs-main.jsonl` | `7f9166356ef1c6a1` | 632,415 | leg 1, all **196** `blobs` rows of the tracked chat's `store.db`, content-addressed by sha256 `id`. 52 decode as plain JSON (`role: system\|user\|assistant\|tool`) and are emitted decoded; 144 are protobuf-framed and kept as base64 with `"encoding":"protobuf-b64"`. The `role:"tool"` rows carry **full tool results including shell stdout** — this is the file that proves leg 1 is *not* lossy at the store layer |
| `hooks.jsonl` | `8bb90755e2faa4ea` | 165,865 | leg 2, all **85** posts from the folder-scoped shim, as the stub receiver logged them, `_goalcup` chain fields included. Contains the single `beforeSubmitPrompt` that was blocked, the 10 `afterAgentThought`, the 5 `config_mismatch` posts from the hot-reload probe, and the 5 subagent-conversation posts |
| `leg1-store-blobs-subagent.jsonl` | `34d54bf121e3f23aec109257ba2ced659af35693160727594548d519b91041a5` | 59,460 | the same for the subagent chat (19 rows) |
| `tui-screen-transcript.txt` | `226c5a323e5f89c6` | 57,219 | the three PTY probes' screen bytes, ANSI stripped and `x` runs elided. **Line 360 is `prompt exceeds 4000 characters`** — everything the human sees when the tracker blocks a prompt. Also shows the "Workspace Trust Required" modal that `-p --force` never displays |
| `transcript-main.jsonl` | `44782b51431cb547` | 47,689 | leg 1, the **lossy** `agent-transcripts` JSONL for the tracked chat: 35 message rows + one `turn_ended`, `role` and `message.content` only, 14 `tool_use` entries with **inputs only**, zero `tool_result` blocks, no ids/timestamps/model/tokens |
| `goalcup-hook.py` | `b51759df3507411f` | 7,725 | the folder-scoped shim, **unmodified**. Full sha256 `b51759df3507411fd22d30fddb806e18f270234ee94b9e143ca34f61310277dc` — identical to `config-hashes.txt` |
| `hooks.json` | `330a45ec81104109` | 2,596 | the folder-scoped config, **unmodified**, all 18 non-IDE events. Full sha256 `330a45ec81104109d48f6fb74c9c2b76ebb4c7c18ea879cf7be91b67f916f543`; the workspace held only this and the shim at launch, and it hashed identically again at collection |
| `stream-turn-06-model-switch.ndjson` | `a6d07357b487c4af` | 2,420 | the whole `-p --output-format stream-json` stdout for the model-switch turn: `system/init` with `model: "GPT-5.4 Mini Medium"`, the `user` echo, thinking deltas, `result` with `session_id`, `request_id` and token usage |
| `leg1-store-meta.jsonl` | `5f8c5981216de1fc` | 1,486 | the `meta` rows of all three `store.db` files, hex-decoded. The subagent row carries **`subagentInfo.{parentAgentId, rootParentAgentId, toolCallId, typeName}`** — the only parent link that exists on this harness (A4). The A9 chat's row is kept deliberately: it is the evidence that a foreign session lands in the same config dir under a different `chats/<hash-of-cwd>/` directory |
| `_structural-scrub.json` | `8c6f12f0b332dda5` | 1,462 | the structural-scrub log (12 entries), see below |
| `transcript-subagent.jsonl` | `03bff1adef9d3930` | 1,455 | the subagent's own lossy JSONL (4 rows). Its first row is a `role:user` `<user_query>` — **byte-identical in shape to a human message**, which is the A2 trap |

Total 988 KB. `spikes/redact.py` also wrote one `*.redactions.jsonl` per file.

## Redaction