## PRD-12 rig, TOP PRIORITY for the next pad slice (from /root/gc-night/c4/REPORT.txt)
- P0a The pad's conductor-half fetch-run uses psql on the conductor, which cannot reach the database (HTTPS proxy only). Owner-approved route (2026-11-01): run the read-only index query in a separate short-lived box that holds only DATABASE_URL (0600 via put-env, removed after; never the harness's pad box, never a build box), copy the index back, then fetch-run --index FILE on the conductor (R2 over HTTPS works here). Proven by hand: /root/gc-night/dbfetch/{install.sh,idx.sh,query.sql} (query.sql = fetch-run.sh's SQL block).
- P0b step_cleanup (scripts/pad/pad:677-689) removes the pad box even when fetch failed, so C4's server_equals_local can never be finished later. Keep the box (or copy the entry's rec/ out, scrubbed) until fetch and check are done; on a failed fetch, copy rec/ out so a refetch can complete C4.

# Queued for the next PRD-12 and PRD-08 slices (from rung 1x, /root/gc-night/rung1x/REPORT.txt)
## PRD-08 verifier
- V1 honesty.rs:363-383 tunnel_end fails a listed tunnel with no GAP even when 0 bytes moved; the recorder names no GAP when nothing moved (proxy/tunnel.rs:62-65). Skip zero-byte tunnels. (Copilot C1/C4 FAIL)
- V2 OpenClaw history_complete: answers orphaned though every response is recorded; suspect tool-call id rewrite or history matcher on zstd-decoded tool calls (capture_analysis/thread.rs, chat.rs).
## PRD-12 rig
- P1 scripts/pad/image.id snapshot only has Claude Code, Codex, Hermes: rebuild the pad image with every pinned harness.
- P2 check.py:437 C3 'no MASK for a planted value' fails when the harness or model redacted first (raw value never on the wire): count it as masked-upstream, not FAIL. Headerless PEM body lines (pad-run:222, 295-309).
- P3 check.py:420 counts the pad's own pane copies in the echo scan.
- P4 harnesses/hermes-cli.toml: ready_text, Enter past telemetry screen, wait (local fix in /root/gc-night/rung1x/padfix/harnesses/hermes-cli.toml).
- P5 harnesses/kimi-code-cli.toml: folder-trust screen; key_line config needs models.<id>.protocol.
- P6 harnesses/direct/cline-cli-config.json: Cline ignores providers.json and shows onboarding; find the real config.
## PRD-10 CLI (wording)
- W1 end screen 'the program refused the recorder' for a host pinned by the host list.
- W2 end screen counts startup calls to a listed host as model requests (kimi); 'session files 0' when a database file was captured (opencode).
## PRD-04
- N1 OpenClaw sqlite: 3-4 source_changed GAPs per run.
## PRD-12 rig, from rung 7 (/root/gc-night/rung7/REPORT.txt, local patch local-rung7.patch)
- G1 pad-run:416-417 always sends top-level pre_keys; read pre_keys from the case section first; claude-code-cli.toml [gateway] pre_keys = ['\r','\r','\033[B\r','\033[B\r'] (no API-key screen, an auto-mode screen after trust). Also handle the auto-mode screen on the direct case if it appears.
- G3 rung 4 kill timing: pad-run:603-604 counts any new written request as work in flight, so the kill lands after the task is done; make rung 4 kill@0 for the smoke prompt (proven on both harnesses) or use a longer prompt; pad-run:620-621 count requests at the seal as recovery evidence only if the harness still had work.
## PRD-12 rig, from rung 5 (/root/gc-night/rung5/REPORT.txt, local-rung5.patch L1-L3)
- R5-1 (conductor decision) adopt L3 as the rung-5 setup: the launch shell outlives goalcup start (exec sleep), modelling a participant whose terminal stays open; document in PRD-12 rung 5. Add L2 exec/kill-both-seal.sh as the rung-5b case.
- R5-2 check.py:584 'harness lost its terminal' matches any 'goalcup continue' text (ab.chatgpt.com status line); match the specific phrase only.
- R5-3 check.py:254 'harness did no work' hides 'harness lost its terminal'; report the real reason first.
- R5-4 codex-cli.toml:13 '/quit\r' does not submit in Codex 0.165; fix the exit keys.
- L1 (same as P0b) copy entry folders out (no *.key, *.pem, server.json) before the box is removed.
## From rung 3b (/root/gc-night/rung3b/REPORT.txt)
- H1 PRD-12:646 says C1 wants history_complete pass or skip; check.py:314-317 lets warn pass. Settle one rule (spec or rig), with PRD-09 on why Claude Code's resent text does not match a response.
- H2 PRD-11: Codex opens a long tunnel to unlisted ab.chatgpt.com; decide list it or name it as expected.
## Still open after 98873c1d (verified: no check.py change merged)
- C3U check.py:465-469 'no MASK' FAIL when the harness or model redacted the planted value first: count as masked upstream.
- H1 history_complete warn rule vs PRD-12:646; revisit after the PRD-08 ends-key fix merges.
## After the masking package merges (one PRD-12 slice)
- D1 OWNER ANSWER: deltas-only shape hits with 0 known hits = C3 PASS with note 'stated loss: shape in streamed reply' (check.py:461 area). Known hits still FAIL.
- D2 pad.toml rates: price gateway Haiku at the <100k-input rate (0.1 in / 0.5 out), see /root/gc-night/gw3/REPORT.txt; add deepseek/deepseek-v4.1-flash (0.3 / 0.007 cached / 1.2) and anthropic/claude-haiku-5.5 from /root/gc-night/gw3/model.patch if not already in.
- D3 hermes-cli.toml:7-9 comment: the gateway preset has now been run (gw3).
