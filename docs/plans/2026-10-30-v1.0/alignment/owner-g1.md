# Owner list (group 1)

Findings from the alignment reviews L1 to L7 that change a locked ledger item, add something the ledger does
not ask for, or ask the owner to confirm a reading. Agents never edit OWNER-DECISIONS.md, so every proposed
ledger line is here, not in a fix list. Each item has a recommended answer. Where a fix list already
builds the safe default, the item says so.

Count: 12 items.

| id | From | Question | Ledger item | What the fix lists do until answered | Recommended answer |
|---|---|---|---|---|---|
| OW-01 | align-L4 L4-1 | Mask beyond M1: exact bytes of credential-named values from the launch env, workspace `.env` files and the harness key files; widen the Stripe rule to test keys; add a `whsec_` rule. Today PRD-00 question 3 builds it by default. | M1 (locked) | Build M1 as written; exact-value masking only for `GH_TOKEN` as today; the pad plants no `sk_test_`, `whsec_` or canary (fix L4-1) | **Yes, add it to the ledger as M1a before H+0.** It masks only known secret bytes, so it costs research nothing, and `cat .env` is the biggest real leak path (point 4). If yes, the second slice builds it as PRD-05 already plans and fix L4-1 is skipped. |
| OW-02 | align-L4 L4-3, L4-8 | Left guard class: `[A-Za-z0-9_]` (the ledger's word boundary) or `[A-Za-z0-9_-]` (also `-`, so no guarded rule fires inside a base64url blob)? | M1 (locked: "a key typed right after a letter or digit may be missed") | `[A-Za-z0-9_]` | **`[A-Za-z0-9_]`.** A key after `-` (`token-ghp_...`, `x-sk-...`) is a real leak; a false mask inside a blob only stars a few research bytes. Point 4 wins. |
| OW-03 | align-L4 L4-4 | Unmask header names that contain `ratelimit` or `rate-limit` (today masked whole when the name holds `token`), for research? | M1 (locked: keep every secret rule) | Header name rule kept, no exemption | **No for 1.0.** Token counts come from bodies and session files (PRD-09); rate-limit headers add little. Revisit after the event. |
| OW-04 | align-L4 L4-7 | Confirm the reading of "drop the Codex rate-limit rules": the four plan, quota and credit rows (`connectors/codex-cli/harness.toml:110-113`); the three id rows stay. | M1 (wording) | That reading, noted in PRD-05 section 8 | **Yes, that reading.** |
| OW-05 | align-L1 F5, align-L2 L2-19 | The "tiny watchdog that only restarts the recorder" is the `goalcup start` or `continue` process. Beside the restart loop it holds the listen socket and run CA, runs helper stops, spawns and ends the harness and owns the deadline (about 450 lines), because those must survive a recorder crash. | R3, L5 (locked wording) | PRD-02 states its only recording duty is the restart loop; PRD-00 adds owner question 9 | **Yes.** These are the start process's existing duties; none records or touches bytes. Splitting them out would add a third process. |
| OW-06 | align-L3 L3-2 | Keep `CLAUDE_CODE_RETRY_WATCHDOG=1` in the Claude Code connector? It changes how Claude Code retries (long retries on 429 and 529). It is not one of H7's five data kinds. | H7 (locked) | Kept, with an overridable-defaults row naming it Owner | **Keep it.** It was in 0.7 (audit C kept it), and PRD-02 section 9.1 relies on it so Claude Code rides over a recorder restart (R2, R3). Without it a recorder crash can end a Claude Code run. |
| OW-07 | align-L3 L3-7 | Calls to a local model server on loopback (Ollama, LM Studio, vLLM) are named by a `local_model` note but not recorded in 1.0. Add to ledger section 8? | R1, point 1 | As the PRDs say (note only) | **Accept for 1.0; add to section 8.** Loopback traffic bypasses `HTTPS_PROXY`; recording it needs a second capture path. |
| OW-08 | align-L3 L3-8 | Session files of a harness started as another harness's subagent (Qwen Code's `claude-code` and `codex` subagents, Hermes's `copilot-acp` child) are not read; only the wire is. Add to ledger section 8? | H8, R1 | As the PRDs say (wire only) | **Accept; add to section 8** ("wire only; named in RULES as not allowed"). |
| OW-09 | align-L3 L3-9 | (a) The proxy answers `405` to a non-CONNECT request. (b) A failed upstream connect gives `502` and only a log line; add one `note` `connect_failed` `{host, port, error}` per host per minute to the record? | H3, R4 | (a) `405`; (b) log line only | **(a) Yes. (b) Yes:** a few lines of code, and the record then shows the harness tried a host it could not reach (point 6). If yes, PRD-03 line 150 and PRD-01's note list change. |
| OW-10 | align-L5 L5-13 | OpenClaw subagents need its Gateway daemon. Start a foreground Gateway with the run's env as a second recorded process? | T4, H7 | Not started; OpenClaw is "not verified: subagents need the Gateway" | **No for 1.0.** T8 lets non-core harnesses ship "not verified"; revisit in a later release. |
| OW-11 | align-L2 L2-13 | Add the age of the last heartbeat to `fetch.json` so the build's read-only checks can flag a "silent" entry (watchdog and recorder both dead) before the server's `silent` view exists? | R7 (server can mail later) | Not built | **No for this build.** R7 leaves server-side follow-up for later; `goalcup status` already says off locally. |
| OW-12 | align-L7 L7-09 | With 1.0, the unchanged cron sends the `recording` email when a heartbeat says `failed`, and the idle notices count written requests. No email code changes. Wanted? | Section 8 (email changes out of scope); R7 | A note in PRD-07 step 3 and in `MORNING.md` | **Yes, wanted.** It is what R7 intends, with no code change. Confirm before the production deploy. |

## Not sent to the owner (decided as fixes)

- Whole-build spend stop lines (L5-02/L6-09): fixed to report lines, because T6 is locked ("cost is otherwise
  not a constraint"). PRD-00's build-spend owner question becomes report-only.
- Query strings in `http_req` meta (L4-5): fixed by keeping today's drop, since M1 keeps every secret rule.
- Null-mode retry ceiling (align-L1 F6, align-L2 L2-06): fixed at 10 minutes, the reading of R3 "fast" that
  does not reset harness connections once a minute.
- Skipping the old-crate cut at H+10 (align-L1 F1): fixed to call the owner under W6, as L3 is locked.
