# Spike 8 fixtures — OpenClaw 2026.8.2 (launch, Gateway), 2026-09-03

Source run: `spikes/openclaw/out/run-20260903T122119Z` (git-ignored). Every file here went
through a **structural scrub** (oversized fields replaced by
`<scrubbed:<field>:<bytes>bytes:sha256=<hex>>`, logged in `structural-scrubs.json`) and then
through `spikes/redact.py` (log: `<file>.redactions.jsonl`). The OAuth account id
(`openai:<email>`) appears in OpenClaw's own `session_status` tool output and was caught by
the `email` pattern. The spike-local Gateway token leaked into the shim's `plugin.loaded`
`env.argv` and was redacted by hand as `gateway_token` (logged in the same file).

| File | What it is | Why it is here |
| --- | --- | --- |
| `store-transcript-events.jsonl` | leg 1, final `transcript_events` for both sessions (77 rows) | the record. Human turns are `role:"user"` with **no** `provenance`; the harness-authored ones carry `inter_session` / `internal_system` |
| `store-session-nodes.jsonl`, `store-session-windows.jsonl` | `session_nodes` / `session_windows` | note there is **no** row for the subagent session: it was deleted on completion |
| `store-session-transcript-archives.jsonl` | the one archive row | the subagent generation, `reason:"deleted"`, `encoding:"zstd"`, with OpenClaw's own `archive_sha256` |
| `store-transcript-rewrite-watermarks.jsonl` | 2 rows | proof that targeted rewrites happened during the run |
| `store-session-participants.jsonl` | participants | the CLI turn is `observation:cli` |
| `subagent-archive.jsonl` | the subagent transcript, decompressed from the `.zst` the tailer captured | A4: the child transcript is recoverable **only** from the archive, and its header has no parent link |
| `leg1-capture.jsonl` | the tailer's hash-chained leg 1 (`runtime` rows dropped, spill blob markered) | A11, including the three rows the rowid cursor missed |
| `hooks-leg2.jsonl` | the whole leg-2 hook stream, 454 posts, oversized payloads markered, 351 of 353 `plugin.tick` dropped | the hook census, the A7 block, the A9 leak, the hot-reload window |
| `hook-llm-input-full.json` | ONE `llm_input` post, unscrubbed | the 40,287-char compiled system prompt and all 54 tool schemas — this is the only shape in which they exist anywhere |
| `hook-after-tool-call-truncation.json` | the 200,001-char `exec` result post (`details.aggregated` markered) | the three-way divergence: 8,014 chars to the model, 64,000 on disk, 199,999 only here |
| `openclaw.json` | the installed launch pin, provider block included | the folder/state-dir-scoped config the tracker writes |
| `goalcup-capture-index.mjs` | the plugin shim source, sha256 `55c076324ba5e2463cf0c7c5e9a6382f085c50669a03b3cc5976d4fa7b5f414b` | the leg-2 producer |
| `CHECK.txt` | `check.py` output for the run | the mechanical verdicts the report interprets |

Companion manifest hashes (workspace, unchanged from `spikes/openclaw/config/workspace/`):

```
55c076324ba5e2463cf0c7c5e9a6382f085c50669a03b3cc5976d4fa7b5f414b  index.mjs
b04c80e995e913dbd04c88ffe0033efa0efe6c5a589ba47a76c6e02c23ae1233  openclaw.plugin.json
0198dc396f7f9b32e688ee645520aef530e5c49ca22570912c6953be1e9acda0  package.json
```

Launch pin, as rendered and as installed after the provider block was merged in:

```
340d36b5ffd89d8c3f279acae01d9ea21edb78b86308e81438bdf3e34262e7cb  openclaw.json (rendered)
67f38280421d3541a1c1fb33829138aa4a246c5197d5c982ddad5dd35c70dfc3  openclaw.json (installed, = this fixture)
```
