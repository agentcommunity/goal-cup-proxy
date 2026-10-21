# Participant review (21 Oct 2026)

Three reviewers walked Goal Cup Proxy as participants, using only what a participant sees (guide,
rules, CLI text, emails): a first-timer, a participant in trouble, and eight different setups
(Codex on macOS, Claude Code on Linux, Hermes, Amp, WSL2, a cloud sandbox, a prepared power-user
setup, a harness builder). This is the merged list, most important first. File references are at
the review commit; re-check before editing.

## A. Fix now (no owner ruling needed)

| # | Problem | Fix | Size |
|---|---|---|---|
| A1 | The silence email says "Your run stopped. Run goalcup continue" after 20 min without a heartbeat. That also happens on wifi drops, sleep and server outages, where the run is still recording, and during a server problem everyone gets it at once. A worried participant may run `goalcup start`, which replaces the run. (`server/worker/sql/goal-cup-tracer-upload.sql` notice text ~2865; `server/worker/src/notices.ts` ~233) | Wording: "We have not heard from your run since X. If the computer is offline or asleep, recording goes on and catches up later. At the machine, run `goalcup status` and do what its Next line says. Do not run `goalcup start`: a new start replaces this run." Logic: no silence mail until 20 min after the server's own last healthy tick. | medium |
| A2 | `goalcup seal` asks no confirmation, stops the agent, prints jargon (anchor, trace, verify), and never says the run is still open. RULES calls it "end a run early", but it ends only one part. (`commands/seal.rs`, `RULES.md` ~103) | Confirm prompt ("This stops <harness> now. Your run stays open until <end>."), plain output ending with "To go on: goalcup continue <run>. If your work is done: goalcup upload.", and correct RULES. | medium |
| A3 | Upload advice contradicts itself: "goalcup continue sends it" (`participant.rs` ~535) vs the guide's `goalcup upload`; after a host died past the deadline `upload` says "run again once the run has ended" forever (`upload.rs` ~234); the ended email never says part of the run has not arrived or names the 24 h close. | One rule: unsent parts go with `goalcup upload`; a dead, ended run says "goalcup seal, then goalcup upload"; ended email adds "Part of your run has not arrived. Run goalcup upload before <close>." | small |
| A4 | A replacement run clones the first run's pushed work: `start` never checks the repository is empty (only prep does), which breaks the "may not seed another run" rule. (`prep.rs` ~762, `runs.ts` ~259) | Refuse or warn unless the repo is empty: "A new run needs a new empty repository: goalcup prep --repo <new>." | medium |
| A5 | `goalcup time-left` fails in the participant's own terminal, though the guide says "you can run it too". (`commands/time_left.rs`, `PARTICIPANT-GUIDE.md` ~355) | Fall back to the run state as `status` does when the env var is unset, or change the guide to "use goalcup status". | small |
| A6 | Firewall or upgrade during a run: the preflight refusal during `continue` says "start again" (leads to a replacement); status says only "Check the network"; the guide says "to upgrade, run the same command again" with no "not during your run"; a new goalcup refuses an old daemon. (`preflight.rs` ~44, `participant.rs` ~866) | "…then run: goalcup continue <id>. Your run is still open."; add "or a firewall such as Little Snitch is holding goalcup"; guide: "Do not upgrade goalcup during your run." | small |
| A7 | The disk-pause text says "free some disk space" but not where; deleting the agent's own files would break the rules. (`participant.rs` ~921, disk email) | "Free space outside your run's folder; do not delete anything the agent made." | small |
| A8 | The filesystem refusal (overlayfs, btrfs, ZFS, NFS, tmpfs, FUSE) shows up only at the practice run or start; prep does not check. Fedora's default btrfs and container sandboxes hit it. (`README.md` ~101, `prep.rs` ~16) | Prep's machine step checks the config, artifact and work folders' filesystems and says how to move them to ext4, XFS or APFS. | small |
| A9 | Words are mixed: run, entry, segment and part; "Runs on this computer" lists entry ids; leftover "tracker" in CLI messages (`continue_run.rs` ~344, `seal.rs` ~345). | Use "run" and "part" only; plain sentences ("The earlier part of this run ended without a clean seal. What was recorded is kept and uploads now. Nothing to do."). | medium |
| A10 | Guide gaps: no website address, no video format, "or CLI" for the X-post URL where no command exists; GitHub token expiry not mentioned (an expired token stops pushes mid-run); the MIT License rule and the no-side-channel rule are not in the guide; no tip to paste the prompt whole; "Recording does not need the network" (the agent does). | Add the address and format, drop "or CLI"; token: custom expiry after 16 Nov, this repo only, Contents read and write; add the two rules and the paste tip; reword the network line. | small |
| A11 | Hermes's default Quick Setup uses a browser login whose token refresh fails behind the recording proxy, so a run can die mid-week. | Guide and prep: "use an API-key provider with Hermes"; prep warns on an OAuth provider (later). | small |

## B. Needs an owner ruling

| # | Question | Context |
|---|---|---|
| B1 | **Approval prompts.** By default Claude Code and Codex ask before running commands or pushing. Answering is "approving actions", which RULES forbids, so a run can stall for days on its first tool call. Should the guide tell participants to set "never ask" modes in prep (Claude `bypassPermissions`, Codex `approval_policy=never` with full access), and should prep warn when they are missing? | All three reviewers ranked this first or second. Connectors set no permission option (D52). |
| B2 | **Start-up dialogs.** Only Claude's folder-trust dialog is allowed after the start. Amp's first screen asks for a login and Hermes's for setup. Allow a listed set of start-up dialogs, or have prep guarantee none appear? | The connector hint text currently tells participants to do what RULES forbids. |
| B3 | **Mid-run keystrokes and quota.** The guide accepts a mid-run `/model` switch, against "only keep going" and frozen settings. Also undefined: `/exit`, Ctrl-C, `/compact`, quota or spend-limit top-ups. | Publish an allowed-keystrokes list and a quota rule. |
| B4 | **Who is told about the deadline.** The goal-cup skill (with `goalcup time-left`) reaches only Codex and Claude Code, and only if the model loads it. Measure a skills folder for Hermes and Amp, tell participants to put it in the prompt, or drop the skill until every harness has it? | Fairness across harnesses. |
| B5 | **Remote machines and sandboxes.** RULES says remote servers are not eligible; the guide gives SSH advice; a provided cloud sandbox is planned. | Suggested: "a machine or VM you control, running goalcup locally, is eligible". |
| B6 | **Amp features that are a second harness or cloud execution.** Amp's Claude Code mode starts the local `claude`; its remote executors and orbs run in the cloud. | Ban them by name. |
| B7 | **Prepared setups.** The workspace must start empty, so where do AGENTS.md, CLAUDE.md or skills go, how are they disclosed, and what about account connectors and parent-folder files that load automatically (Claude account connectors, Amp skills up to `/`)? MCP tools that call other models (D10 grey zone). | Write one section per harness. |
| B8 | **Support status table.** Show which harness and platform pairs are measured, practice-only or unmeasured, and treat capture gaps on unmeasured pairs as organizer-caused? | Claude on Linux, Amp on Linux, Intel Mac and WSL2 on a real Windows machine are unmeasured. |
| B9 | **Connector cutoff and dry runs.** The last release (6 Nov) is after the dry-run window closes, so a late harness's users get no dry run; "dry run" is not defined in the guide; a late dry run could block a competition start. | Earlier connector cutoff; define dry run. |

## What already works well

Install output; login's two approvals; prep's `[ok]/[todo]` steps; the deadline messages; crash
and closed-terminal advice (terminal, status, guide and email agree on `goalcup continue`); reboot
detection; refusal of a double start while the host is alive; the disk-pause wording across
terminal, status and email.
