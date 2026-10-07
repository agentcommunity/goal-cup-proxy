# Hermes reference row 1: DeepSeek V4.1 Flash, 2026-10-02

Result: **PASS, sealed complete LocalOnly capture.** First live third-harness row. Owner-run per D41
(the agent never opened the key file); post-run checks by an agent and spot-verified by root.

## Identity

| Item | Value |
|---|---|
| Source | `m2/wire` `8e1d834443d39e1770480114585eb1c70b39d32b`, clean tree |
| Binaries (release, macOS 26.6.2 arm64) | `goalcup` `7646b91e…98e0b2`, `goalcup-daemon` `8688a932…20c2ba`, `goalcup-verify` `634b60ae…b4bcf` |
| Connector | `hermes-cli` manifest pin `6c478f79…cf82bc`, directory pin `6f5eb45f…9d7a2b` |
| Harness | `hermes-agent==0.19.0`, CPython 3.13.13; readiness inventory identical before and after the run |
| Provider / model | `--provider deepseek --model deepseek-v4-flash`, thinking disabled (D41) |
| Credential | `DEEPSEEK_API_KEY` loaded by `scripts/with-deepseek-key.sh` (`/bin/sh -p`) from the owner's multi-variable 0600 `.env`, child-only |
| Goal | create `sum.py` printing the sum of 1..10, run it, report the number |

## Results

| Check | Result |
|---|---|
| Launcher | exit 0 |
| Key scan (`key-scan.txt`) | `clean`; independent scan: no `sk-` token, masked form or `Bearer` in the run dir or trace (34 raw `sk-` substrings are all inside "risk-"/"Task-") |
| `goalcup-verify --capture-integrity` | exit 0: valid, sealed `local_only`, no torn tail, 0 gaps, `coverage_incomplete` false, 0 findings; 45 records, 392,595 bytes, 3 producers all closed, terminal `local_finish` |
| Provider evidence | 3 × `POST /v1/chat/completions` to api.deepseek.com, HTTP/1.1, 200, SSE (`write_file`, `terminal`, final reply) |
| Served model | request `model` = `deepseek-v4-flash`; all 121 response chunks `model` = `deepseek-flash` (DeepSeek's V4.1 Flash ID); snapshot `deepseek-v4-flash` |
| Native evidence | one Hermes session snapshot, complete and byte-identical to disk, bound to stderr session ID `20261002_125737_dc0697`; no SourceChanged or deferred-scope gap |
| Output, independent | `sum.py` = `print(sum(range(1, 11)))`, prints `55` |
| Credential residue | private `auth.json` byte-identical to the D41 seed, no `credential_pool`; 0 `request_dump_*`; no tirith; `~/.hermes` absent |
| Hygiene | candidate daemon stopped; only the three pre-existing H-13 daemons remain |

## Notes and limits

- The served model ID is DeepSeek's undated `deepseek-flash`; the row claims only what the response field says.
- Four connections, three exchanges: one connection opened and closed (`interrupted`) in ~0.13 s with no payload or host. models.dev (accepted, D41/H-18) left no chain record; the proxy did not count it as loss. Neither is attributable from the chain.
- Response stream 2 of 3 ended `interrupted` after `finish_reason=tool_calls`, usage and `[DONE]`; the verifier counts no gap.
- Semantic decoding now exists: the `openai-chat-completions` decoder (v1, 2026-10-05) reads this wire format, tested on synthetic bytes only. Claim level: **decoder available, live row not re-verified.** The row was captured before the decoder and its trace has not been re-decoded or compared with the native snapshot, the served model fields or the independent `sum.py` output, so none of its observations is a measured fact yet. Until that is done the row's claim stays capture-only; the connector stays `capture_only = true` (V1 still refuses it).
- Under D45 the decoder reports the stream-2 `interrupted` end (after `[DONE]`) as one `unknown` with `done_seen: true` and does not mark its summary incomplete; it still decodes everything that arrived.
- One row. No repeated, long, large-reserve or cross-version qualification; H-10 4c and T25–T28 remain open.

Raw run directory, trace and private log stay local under `/private/tmp/goalcup-hermes-live-8e1d8344/` (not committed).
