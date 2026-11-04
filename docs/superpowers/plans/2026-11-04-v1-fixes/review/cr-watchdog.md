# cr-watchdog notes (in progress)
Scope: crates/goalcup/src/{watchdog,cli,native,connector}
## Findings (draft)
- important: tests/watchdog_{end,null,recover,restart,rlimit,shield}.rs, cli_recover.rs, cli_start.rs, native_safety.rs are all #![cfg(target_os="linux")]: macOS paths (spawn.rs:277 ps-based procs/end_tree, cli/mod.rs:446 proc_pidinfo start_id, no subreaper) have zero automated tests, CI is ubuntu only.
- important: recover leaves the harness home unlocked: cli/recover.rs:195 takes HomeLock only inside rebuild(), dropped on return; watchdog/recover.rs:136 holds none. A new `goalcup start` of the same harness then gets the lock, writes settings into the live home and its native reader takes the recovered harness's session files (cross-run contamination). Fix: recovered watchdog takes HomeLock for its life.
- important (usability/versions overlap): connectors/tiers.toml ships all 16 rows "not verified" version "" and is include_str'd (cli/start.rs:26,298): every participant sees "<harness>: not verified" at start of the release build.
- minor: shield.rs:30 hangup_seen() is dead; after a closed window cli/start.rs:138-143 println! on a hung-up tty panics (exit 101 instead of harness code). Recorder unaffected.
- minor: connector/mod.rs:236-247 a pin (`version`) is skipped silently when `--version` fails/takes >5 s (harness_version None) -> pinned connector runs on any version.
- important: watchdog/mod.rs:569-580 recorder respawn `loop` retries forever with no harness look / SIGUSR1 / exit check: a persistent spawn failure (binary replaced/unlinked mid-run on Linux -> current_exe "(deleted)", EMFILE) wedges the watchdog; harness exit never reaped, no seal, shell never returns.
- minor: cli/start.rs:375,382 + pipeline/mod.rs:706,712 `server_time` and `build` always None -> start event always "build":null,"server_time":null (dead fields; clock-skew anchor missing).
- minor: watchdog/mod.rs:295-307 failure paths after write_pid leave rec/watchdog.pid (harmless: start_id guards).
- note: SIGTERM to the watchdog is not caught (shield catches INT/QUIT/TSTP/HUP); recorder self-seals on control EOF, so OK by design.
## Re other lanes
## Answers to inbox
- to cr-scripts: yes. install.sh:326-328 renames old binary into $bin_stage/goalcup/old then rm -rf $bin_stage (install.sh:208). Linux: /proc/self/exe -> ".../old (deleted)" -> Boot::start spawn ENOENT forever (watchdog/mod.rs:569-580): watchdog wedged, listener backlog accepts harness connects that never get served (harness hangs), harness exit never reaped. macOS: current_exe is the original path -> NEW binary spawned as recorder (version skew in StartFacts/control lines). Agree important; INSTALL_UNSURE should refuse.
- to arch (null-mode Restart drain): no drain on the watchdog side. Restarts::tick (watchdog/mod.rs:794-808) sends Restart as soon as the wait ends, never looks at open connections, and SIGKILLs 10 s later. The watchdog only keeps the listener (fd 3); in-flight streams live in the recorder and die with it. Agree important; fix belongs in the recorder (stop accept, finish open relays, then exit) plus the watchdog raising RESTART_KILL_NS / deferring Restart.
- to cr-proxy (participant proxy/CA env): confirmed nothing carries them. connector/launch.rs:89-105 removes REMOVED (incl. HTTPS_PROXY/ALL_PROXY/NODE_EXTRA_CA_CERTS/SSL_CERT_FILE/REQUESTS_CA_BUNDLE) and finish_env (launch.rs:110-128) overwrites with the run CA/bundle; grep finds no other reader of the originals and cli/start.rs check() (163-237) prints no warning. Agree important; cheapest step is a start-time warning when any of those were set.
## Re other lanes
- arch-7 agree: recovered watchdog is dispatched through the recorder subcommand (watchdog/recover.rs:112 spawn::recorder + ToRecorder::Recover first line). Low risk but confusing in ps/status.
- arch-10 agree (see answer above).
- versions V3/V7: agree from connector side: connector/mod.rs:236 runs --version only when the lock is free, launch.rs:148-171 5 s cap; a None version silently skips the `version` pin refusal (mod.rs:242-247) -> a pinned connector runs on any build when the probe fails.
- cr-scripts-2 agree (answered in inbox).
- usability-1 agree: start.rs check() never tests that plan.program resolves (resolve_program falls back to bare name, launch.rs:143) before approve::begin opens the run; watchdog then logs harness_failed and seals an empty part.
- minor (extra): Linux subreaper adopts orphans (spawn.rs:156) but nothing reaps them until end_tree at the deadline/seal -> defunct children of `goalcup start` pile up over a long run.
- minor (extra): the_start_line_fits (watchdog/mod.rs:1084) uses 120-byte values and the built-in host list only; a large --hosts file (practice/test) can push the Start line past MAX_LINE -> recorder exits no_start_line -> generic "no_ready".
## Verdict: With fixes. Lifecycle design (restart loop, null mode, recover, shield, start ids, CA key handling) is careful and well tested on Linux; fix the respawn wedge and the recover home lock, add macOS coverage.

## Answers
- Inbox (cr-scripts, arch, cr-proxy) answered above under "Answers to inbox"; all three agreed as important after checking code.
- Correction: my earlier "arch-7 agree" line about the recovered watchdog posing as the recorder subcommand is arch-5, not arch-7.

## Re other lanes (round 2)
- arch-7 / usability-12: duplicate of cr-watchdog-4 (tiers.toml: 16 rows, all 16 `tier = "not verified"`, include_str'd at cli/start.rs:26).
- versions-1 agree, severity-down to minor for 1.0: start.rs:442-443 keeps only words starting with a digit, so "Hermes Agent v0.22.6" / "opencode v2.2.2" give None and text.rs:1167 prints plain "verified". Real bug, but zero effect while every tiers.toml row is "not verified"; becomes important the day a row flips.
- versions-2 agree with extra evidence: connector/mod.rs alone has two matchers (`held` 237-240 prefix-of-word, `exact` 251-256 no-digit-after), plus start.rs:442 tier parser that does not strip `v`. One `version_word(line) -> Option<&str>` would fix versions-1 too.
- usability-3 agree: text.rs:281-286 says "goalcup start again opens a new run; the run stays open"; continue_run.rs:324-325 reads first-message.txt that the opened-then-stopped path never wrote.
- versions-7 (Hermes fresh-home unread) plausible from code: launch.rs:148-171 returns None on non-zero exit or >5 s, and the note lookup (mod.rs:258-261) needs Some -> silent on first run. Did not run hermes.
- arch-1 agree (answered in inbox), cr-proxy-2 agree (answered in inbox), cr-scripts-2 agree, usability-1 agree.
