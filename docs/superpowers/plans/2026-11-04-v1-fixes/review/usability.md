# usability lane (running notes)
Top findings (file:line, one line each), release/v1.0-rows checkout:
1. start never checks the harness program resolves before the browser approval + server open (cli/start.rs check(), line 163-237); setup does (setup.rs:76). Not on PATH => competition run opened, clock + replace-main-run already happened, then "X did not start" with no reason.
2. --harness takes only exact ids (claude-code-cli); `--harness claude` => "no built-in connector `claude`" and no list (connector/mod.rs:156, setup.rs:73, start.rs:181); --help lists no ids.
3. opened_then_stopped (text.rs:281) tells the user to run `goalcup start` again; correct next step is `goalcup continue`, and RULES say a new start REPLACES the open run (text says "stays open"). Also continue then has no first-message.txt (never written) so push block lost; `status` on that run says "write to support" (status.rs:331).
4. first message recovery: first-message.txt path shown once before the TUI paints; no command/status line reprints it (grep: only start.rs/continue_run.rs).
5. README.md:41,44 `--prompt prompt.txt` (relative => always inside project folder => refused/missing).
6. repo_not_empty (text.rs:308) does not say "to resume use goalcup continue"; restart-by-habit lands here.
7. server window/closed refusals come AFTER the browser approval (approve.rs:124 then :139), so early start = approve, then refused, approve again; messages end ".." (said ends with '.') and AFTER_WINDOW is a lowercase fragment (db-local.ts:116).
8. PROBLEM: lines with no action: Null (text.rs state_line Null), no_request_yet (text.rs:620).
9. Mac: no lid/battery warning when caffeinate exists (awake.rs missing()); `-s` is AC-only.
10. setup prints Ready. with no tty and no --yes after writing nothing (setup.rs:79-95); Ready never checks sign-in.
11. OpenCode 1./2. split: both ids listed as installed (setup.rs installed()), wrong pick => "does not show 1." (text.rs:176) says install that version, not "use opencode2-cli".
12. tier line "not verified" for every harness (connectors/tiers.toml) with no what-to-do; clap parse errors drop tips (main.rs:145); jargon (seal/part/chat mode/pinned).
13. time-left outside harness says unknown between parts although status knows the deadline (time_left.rs:43-58).
14. continue does no workspace check (continue_run.rs:45) - wrong cwd launches harness elsewhere.
## Re other lanes
(pending)
- versions: agree tiers.toml ships all "not verified" (start screen line has no what-to-do); extra: opencode wrong-id pick gives "install that one" instead of naming the sibling connector.
- cr-scripts-2 (INSTALL_UNSURE installs anyway): agree, participant-side symptom is a silent wedge mid-run; refuse unless --force.
- docs: README.md:41,44 `--prompt prompt.txt` bug is new (usability-5), not in docs-*. docs-6 agrees with the worker message "Start without --dev" on a CLI that has no --dev.

## Answers
- inbox-usability.md does not exist (no lane asked me anything).

## Round 2 responses
- versions-1 agree (checked start.rs:438-445): the version filter keeps only tokens starting with a digit, so "v2.1.3" gives None and text.rs:1167 then prints "verified with goalcup on X" with no version compared. Participant reads a green claim for an unchecked build.
- arch-7 / cr-watchdog-4 / usability-12: duplicates of one root (tiers.toml all "not verified", written by pad, include_str at start.rs:26). Participant impact: every harness prints the same line with no action (text.rs:1171).
- cr-proxy-2 agree important (launch.rs:19-21 strips HTTPS_PROXY/https_proxy then sets own at 120). Corporate-network participant sees a stalled harness with no PROBLEM line pointing at the proxy.
- cr-proxy-1 agree important, same symptom class; needs a participant-facing line naming "corporate proxy/CA" as the cause.
- cr-upload-server-2 agree; extends usability-6: GitHub outage turns into a refusal after browser approval with raw text.
- cr-watchdog-6 agree minor; closed terminal panic also loses the next-step text.
- versions-7 duplicate of usability-11 (same text.rs:179 downgrade message).
