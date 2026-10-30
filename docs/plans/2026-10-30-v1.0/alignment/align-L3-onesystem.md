# Alignment review L3: one system for every harness (ledger H1 to H10)

Scope: OWNER-DECISIONS.md section 3 (H1 to H10) against PRD-03 (proxy), PRD-04 (native capture), PRD-11
(connectors), PRD-12 (test pad), and the core (PRD-00, PRD-02, PRD-10 where it touches connectors, PRD-13
patchwork table, PRD-14 code rule test). I looked for per-harness code paths, patchwork, and harness names in
core code. Line numbers are from the files in docs/plans/2026-10-30-v1.0/ on branch
smart/intelligent-newton-0avrpi.

Counts: 0 blocker, 4 major, 7 minor.

## Findings

| id | file and section (lines) | what is wrong | breaks | severity | exact fix |
|---|---|---|---|---|---|
| L3-1 | PRD-11 section 7 "What auto-update off can and cannot do" (685-690); PRD-11 schema row `update_off` (157); PRD-10 setup step 3 (405-411); PRD-10 asks row C1 (770) | Where the switch is only a setting (Hermes, Gemini CLI, Qwen Code), auto-update is turned off only if the participant runs `goalcup setup` and answers yes to one question that also turns on bypass-permission mode. A participant who skips setup, or says no to unattended approvals, keeps auto-update on. PRD-10 also says `no_prompts` holds the update switch, while PRD-11 has a separate `update_off` key. | H9 (locked: "Auto-updates off where the harness allows it") | major | PRD-11 line 157, replace "Written by `goalcup setup` into the isolated home in the same step as `no_prompts` (owner question 8)" with "Written by `connector::plan` into the isolated home at every `goalcup start` and `continue`, before launch, whatever the setup answer. It is not part of the `no_prompts` question." PRD-11 lines 686-689: replace "`goalcup setup` writes it into goalcup's own isolated home ... may still update." with "`plan` writes it into goalcup's own isolated home at every start (never the participant's real one). A harness that rewrites the file mid-run may still update; the `start` event records the version at every start." PRD-10 lines 405-411: drop "and auto-update" and "which include the auto-update switch where it is a setting (Hermes, Gemini CLI, Qwen Code)" and "and to not update itself during the run". PRD-10 line 770: replace "`no_prompts` also holds the settings-form auto-update switch (Hermes, Gemini CLI, Qwen Code)" with "`update_off` is a separate key that `plan` applies at every start (PRD-11)". |
| L3-2 | PRD-11 section 6, Claude Code connector `env` (386); overridable defaults (859) | `CLAUDE_CODE_RETRY_WATCHDOG = "1"` changes how Claude Code retries (long retries on 429 and 529, D-harnesses.md:347, :400). It is not one of the five kinds of per-harness data the ledger allows, and unlike `CLAUDE_CODE_DISABLE_AGENT_VIEW` it has no owner row. It changes the run being measured. (`CLAUDE_CODE_DISABLE_AGENT_VIEW` is aligned: it bypasses a background supervisor, an H7 kind.) | H7 (locked: per harness only CA vars, proxy setting, daemons, session patterns, auto-update switch); AGENTS.md "records and never manages the agent"; point 7 | major | PRD-11 line 386: change the `env` table to `{ DISABLE_AUTOUPDATER = "1", DISABLE_UPDATES = "1", CLAUDE_CODE_DISABLE_AGENT_VIEW = "1" }`. Add to OWNER-DECISIONS section 9: "O3: keep `CLAUDE_CODE_RETRY_WATCHDOG=1` on Claude Code (it changes how the harness retries)? Default if no answer: no." If the owner says yes, add a row to PRD-11 overridable defaults: "`CLAUDE_CODE_RETRY_WATCHDOG=1` / on / Owner (it changes how the harness retries)". |
| L3-3 | PRD-00 Definition of done (601-628); PRD-00 third slice (551); PRD-12 section 9 `--release-check` (437-441); PRD-03 acceptance 3 (554-558) | HTTP/2 lands in the third slice and no done item or release check requires PRD-03 acceptance 3. If the third slice slips, 1.0 ships without HTTP/2 and nothing fails. | H6 (locked: "HTTP/2 is supported by the proxy") | major | PRD-00 after done item 2 add: "2a. **HTTP/2 works:** PRD-03 acceptance 3 passes in a pad box (an `h2`-only client to a listed host: one `conn` with `alpn: "h2"`, four decoded request and response pairs, zero `h2_unsupported`), and `tests/proxy_h2.rs` is green in `gate full`." PRD-12 line 438: after "and the Codex kill row are PASS" add "and the `h2-parallel.sh` exec row of PRD-03 acceptance 3 passed". |
| L3-4 | PRD-04 section 8 step 5 (336-343); PRD-02 section 11 step 4 (seal) | Every seal, including a plain harness exit, stops reading session files after 180 s and writes `cut_at_seal`. After a harness exit the files are static on disk, so this is a time cap of our own that drops bytes we could read. R5 allows only memory guards. | H8 (locked: session files read "with no caps"); R5 | major | PRD-04 line 336, replace "5. **Time guard: 180 s** (today's finish budget, `control.rs:708`)." with "5. **Time guard: 180 s, only when the seal reason is `deadline`, `server_stop` or `signal`** (today's finish budget, `control.rs:708`). On `harness_exit` and `seal_command`, finish reads every file to its end with no time guard; status shows `sealing: reading session files` meanwhile." Make the matching change in PRD-02 section 11 step 4 and in PRD-04 overridable defaults row "Finish time guard". |
| L3-5 | PRD-00 "One system" (98-102); PRD-11 shape table, Dodger row (62); PRD-13 section 3 last-but-one row (103) | The ledger calls Amp and Cursor "proxy dodgers" that fit the same system (vendor host recorded, plus session files). The plan uses "dodger" only for a client that ignores the proxy, pins or refuses the CA, and gives that the tier "not supported"; Amp is a "vendor backend". Both readings are internally consistent with T3, but a builder reading the ledger's "dodgers (Amp, Cursor)" next to PRD-13's "Dodgers get ... the tier not supported" could mark Amp "not supported". | H5, H10 (wording) | minor | Add to PRD-00 after line 102: "Words: the ledger's 'proxy dodgers (Amp, Cursor)' (H5) are the plan's vendor backends: listed, decrypted, recorded, tier 'verified' or 'not verified'. Only a client that ignores `HTTPS_PROXY`, pins, or refuses the CA on its model host gets 'not supported' (T3)." PRD-13 line 103: replace "and the tier \"not supported\"" with "and, when the harness's own model host is affected, the tier \"not supported\" (T3); Amp and Cursor are vendor backends, not this case". |
| L3-6 | PRD-10 setup step 4 (414-415) | "the model hosts of the chosen connector reachable" assumes per-connector model hosts. In 1.0 connectors hold no hosts; there is one shared list. A builder would have to invent a per-harness host set. | H2 (one shared list) | minor | Replace "the model hosts of the chosen connector reachable (TCP, a warning only: F3)" with "the hosts that the connector's `host_hints` and `hint_env` name, if any, reachable (TCP, a warning only: F3)", or cut the probe (less code). |
| L3-7 | PRD-11 "A model server on this computer" (76-79), risk row (844) | Calls to a local model server (Ollama, LM Studio, vLLM on loopback) are never recorded, only named by a `local_model` note. That is a model call not recorded; the ledger's out-of-scope list (section 8) does not name it. | R1, point 1 (scope not agreed) | minor | Add to OWNER-DECISIONS section 9: "O4: local model servers on loopback are named (`local_model`) but not recorded in 1.0. Default if no answer: accepted, listed in section 8." |
| L3-8 | PRD-11 section 8 "Cross-harness subagents: wire only" (692-701), risk row (841) | Session files of a harness started as another harness's subagent (Qwen Code's `claude-code` and `codex` subagents, Hermes's `copilot-acp` child) are not read; only the wire is. The ledger does not list this limit. | H8, R1 (scope not agreed) | minor | Add to OWNER-DECISIONS section 8: "Session files of a harness started as another harness's subagent (wire only, named in RULES as not allowed)." |
| L3-9 | PRD-03 section 1 rule 3 (147-151), overridable defaults (650) | A non-CONNECT request gets `405`, and an unreachable upstream gets `502` with "no record". H3 says every other connection is relayed untouched and recorded. The 405 is flagged "Owner" in PRD-03 but is not in the ledger's open list; the 502 attempt leaves no trace in the record. | H3, R4 | minor | Add to OWNER-DECISIONS section 9: "O5: answer `405` to a non-CONNECT request on the proxy. Default: yes." PRD-03 line 150: replace "one log line, no record, since no connection exists" with "one log line and one `note` `connect_failed` `{host, port, error}` per host per minute, no stream". Add `connect_failed` to PRD-01's note list. |
| L3-10 | PRD-00 third slice row (551) | Says unsupported HTTP/2 frames are "`h2_unsupported` and a tunnel". PRD-03 section 6 and 5a say a decrypted connection cannot become a tunnel: parsing stops, the connection keeps relaying, and a `conn` GAP covers the rest. | H6 (clarity) | minor | PRD-00 line 551: replace "anything else is `h2_unsupported` and a tunnel" with "anything else stops parsing for that connection with GAP `h2_unsupported` while it keeps relaying (PRD-03 section 5a)". |
| L3-11 | PRD-11 schema `vendor` key (159), Amp connector (510), PRD-10 reads it (PRD-11 746) | The connector bool `vendor = true` repeats a fact the one host list already holds (`kind: vendor` on `ampcode.com`, written into every `conn` meta). Two sources for one fact; a gateway or vendor fact per harness is the patchwork H1 removes. | H1, H10, L2 (less code) | minor | PRD-11: delete the `vendor` row (159), `vendor = true` (510), and `vendor` from the field lists at 353 and 746. PRD-10 keys its "vendor backend" line off `conn` meta `kind: "vendor"` in the record, as PRD-03 section 12 (453) already does. |

## Checked and aligned

- H1 one proxy: PRD-03 Goal (11-12) and rules 1 to 4 (138-154) give one code path for every connection. The
  per-harness evidence paths (`connectors/codex-cli/harness.toml:121-130`) and the Host-mismatch fallback are
  cut (144-145). PRD-13 section 3 (97-104) removes the old patchwork rows. PRD-02 boot steps (340-385) and seal
  (520-540) have no harness branch; `before` and `after` are connector data.
- No harness names in core code: PRD-14 section 6 rule 1 (221-227) fails the gate on any connector id or
  listed host string in `crates/goalcup/src/**`, with one allowed file (`connector/builtin.rs`). The verifier's
  per-harness usage readers (PRD-09 `native.rs`, `sqlite.rs`) are outside the recorder and allowed by the
  ledger's section 7 wording ("the verifier and the usage reader judge them"). PRD-11 states "no harness-name
  checks" in the loader (165).
- H2 one shared list: PRD-11 section 4 (236-308) holds providers, gateways (Vercel, OpenRouter, Cloudflare,
  Kilo, Cline, OpenCode Zen), cloud model services (Bedrock, Vertex, Azure), vendor backends (Amp, Copilot,
  Code Assist, Cursor), plus participant (`record-hosts.txt`) and config hints, one `hosts::check` for every
  source. PRD-03 section 3 (199-217) consumes it and keeps no copy.
- H3 tunnels: PRD-03 section 9 (384-411): every undecrypted connection is a `conn` record with host, bytes and
  times, written at open, no row cap; the per-host `long_tunnel` flag. Decryption limited to the list (58-63).
- H4 gateways: PRD-03 (45-56), PRD-11 (44-83), PRD-00 (69-97): gateway and direct key are one code path; PRD-11
  acceptance 2 (780-783) and PRD-12 P4 (640) require "no connector has a gateway-specific line".
- H5 dodgers: pinned and refused clients fall back to a recorded tunnel with a named GAP (`pinned`,
  `tls_refused`, `upstream_untrusted`), PRD-03 section 1 (121-133) and section 4 refusal learning (259-278);
  session files read the same way (PRD-04 section 10). Wording only: see L3-5.
- H6 HTTP/2: PRD-03 section 6 (316-341) and ALPN rule (240-251) design it; only the release gate is missing
  (L3-3).
- H7 per-harness data: PRD-11 sections 2 and 6 put CA variables (`ca_env`), proxy overrides (`unset`,
  `refuse_if_set`), daemons (`before`, `after`, `--no-daemon`, `--standalone`, `CLINE_SESSION_BACKEND_MODE`,
  `CLAUDE_CODE_DISABLE_AGENT_VIEW`), session rows (`native`) and update switches in TOML; defaults in code are
  the same for every harness (section 3, 174-196). Extra keys (`no_prompts`, `host_hints`, `mask`,
  `secret_files`, `first_screen`, `skills_dir`) trace to other ledger items (owner question 8, H2, M1, point 4,
  point 6, D66). Exceptions: L3-2 and L3-11.
- H8 session files: PRD-04 sections 2 to 7: one reader for every harness, rows are data, `**` patterns, no row,
  slot or file cap, path is the ID (`<row>:<relative path>`, 302-307), the file is the buffer. Exception: the
  seal time guard (L3-4).
- H9 auto-update: env switches set at every start (PRD-11 section 6); version recorded in every `start` event
  (PRD-11 120-124, PRD-12 row `harness_version`). Exception: settings-only switches (L3-1).
- H10 vendor hosts open by default: PRD-11 `vendor` list (256-259), Amp sign-in host in `never` (260).
- T3 "never hacked around": PRD-12 section 13 rule (555-556); PRD-11 removes the Hermes `providers.recorder`
  base URL (762, 896-897).
- PRD-12 per-harness pad files (section 6, 273-298) are test data only, goalcup never reads them; launch facts
  stay in PRD-11. The simulated dodger case (153-158, 679) is justified by T2 (no Amp key, Cursor out).
