# cr-scripts (scripts/, packaging/install.sh, .github/workflows) -- notes, top first
- IMPORTANT cr-scripts-1 pad: case="all" + goalcup_home runs direct+dodger IN PARALLEL in one signed-in GOALCUP_HOME (scripts/pad/pad:1071-1074, pad-run:232). goalcup's HomeLock refuses the 2nd -> rig FAIL. Evidence: pad-out/claude-code-cli/dodger/20261103T210247Z-pad-e9261e "setup FAIL goalcup setup exit 1, 0 home paths printed", same second as the direct row. runs/core-*-signin.toml all use case=all. Fix: max_workers=1 when goalcup_home set (or refuse case=all with it).
- IMPORTANT cr-scripts-2 install.sh: INSTALL_UNSURE still installs (packaging/install.sh:157-176). status >10s / unreadable -> binary replaced under a live run. With cr-watchdog's respawn finding (current_exe "(deleted)" on Linux) a live run can wedge. Fix: unsure = refuse unless --force.
- IMPORTANT cr-scripts-3 table --release-check (scripts/pad/table:128-175) is the release gate in release.sh tag and has NO test (no test_* references release_check). Fix: unit tests with fixture rows (PASS, C3-not-exercised relax, masked_upstream, commit diff under GUARDED, missing exec verdict).
- MINOR cr-scripts-4 release.yml publishes any v* tag matching Cargo version; never checks the tag commit is on origin/main nor that the pad release check ran (.github/workflows/release.yml:19-21,56-63). Fix: `git merge-base --is-ancestor $GITHUB_SHA origin/main` + run table --release-check in the version job.
- MINOR cr-scripts-5 install.sh wget branch: no https-only redirect / TLS floor (install.sh:67-68). Fix: `wget --https-only --secure-protocol=TLSv1_2`.
- MINOR cr-scripts-6 install.sh path_line: fish (and nushell) users get a POSIX `export` line into ~/.profile which fish never reads (install.sh:222-229).
- MINOR cr-scripts-7 pad extract(): tar data filter only "where Python has it"; 3.11.0-3.11.3 extract box-made tars unfiltered on the conductor (scripts/pad/pad:153-156). Refuse instead.
- MINOR cr-scripts-8 queue.json written in place under queue.json.lock, read under a different lock (pad:239 vs 256-262): torn read -> {} -> deepseek downgrade lost. os.replace.
- MINOR cr-scripts-9 gate.sh silently `sudo -n dnf install python3.11` (scripts/gate.sh:55): a test gate installing system packages; OSS contributors on Fedora get a surprise.
- MINOR cr-scripts-10 release.sh prepare: a failing `git commit` (hook/gpg) or awk under set -e leaves the user detached/on release branch with edits; restore only covers cargo (scripts/release.sh:96-113).
- MINOR cr-scripts-11 fetch-run.sh curl -K config interpolates dest path and token unescaped in "..." (fetch-run.sh:202-206): an <outdir> with `"` or `\` breaks the config.
- MINOR cr-scripts-12 pad push-dir copies all of scripts/pad (untracked files too) into a box where an AI agent runs (pad:684).

## Strengths
- install.sh: whole script in main(), checksum before touching prefix, run-before-promote, staged rename + rollback, recording guard; tests for each.
- Actions pinned by SHA, persist-credentials false, no pull_request_target, read-only token; release job only on push.
- Pad secret hygiene: names not values, sbx put-env, organizer secrets stripped from every child env, run-ca.key excluded from rec-kept, C3 scans before rec copy.
- fetch-run: DB URL via 0600 service file, token via curl -K stdin, BEGIN READ ONLY..ROLLBACK, key regex and path checks.

## Re other lanes
- oss-1: origin remote of the checkout IS git@github.com:agentcommunity/goal-cup-proxy.git, so install.sh/Cargo links match the real repo; only the local folder name differs. Dispute "checkout is goal-cup-tracer" as a defect.
- cr-watchdog respawn-forever: install.sh is the main way the binary gets replaced mid-run; INSTALL_UNSURE proceeds (cr-scripts-2) -> extra evidence for severity.
- versions: pad side of pinning: scripts/pad/image:43-45 installs every harness at the pinned `version` and fails the image if not exact; pad-run:315 now accepts any installed version and records it (owner 3 Nov); scripts/pad/table:82-84 refuses "verified" unless all case rows share ONE harness_version; tiers.toml gets that one string (table:31). Multi-version needs: image per version set, table keyed (connector, version), tiers.toml version list.
- cr-proxy / C3: scripts/pad/check.py:242-256 + 556 turn known planted values in SSE deltas into "stated loss" PASS (owner 2 Nov); committed rows show "9 known hits" PASS (qwen, opencode2). Point 4 gap is accepted, not fixed.

## Answers (round 2)
- inbox-cr-scripts.md holds only a notice from versions (their answer is in versions.md); no open question for cr-scripts.

## Re other lanes (round 2, checked)
- arch-7 / cr-watchdog-4 AGREE + extra: connectors/tiers.toml is include_str! (start.rs:26) and still the 5dc9adf5 phase-0 seed (all "not verified"). release_check (scripts/pad/table:128-175) never compares the committed tiers.toml with render_tiers(rows), and GUARDED excludes tiers.toml (table:17-18). So `release.sh tag` can pass with green rows and still ship "not verified" for every harness. -> new cr-scripts-x1.
- arch-9 AGREE, numbers check: check-links.py 1,349 + test_check_links.py 1,672 = 3,021; gate.sh:85-86 runs only test_install, test_fetch_run and scripts/pad tests; ci.yml runs no python. Only docs/archive and a research plan mention it.
- oss-3 PARTIAL DISPUTE: docs/plans is not all history; the release gate reads docs/plans/2026-10-30-v1.0/pad-rows.jsonl (table:13 ROWS_REL) via `git show <sha>:...`. Deleting docs/plans breaks `release.sh tag`. Move rows to e.g. scripts/pad/rows/ first. (Also: origin/main 70eecdfb has no committed rows yet, so tag v1.0.0 is currently refused, as designed.)
- oss-4 SEVERITY-DOWN to minor: the pad's staging URL (pad:30) is a public staging endpoint, not a credential; pad refuses production by design. Personal handle cleanup is fine but not important.
- cr-upload-server-12 AGREE on the advice: delete-test-runs.sh:118 tells the operator to rerun "without --older-than", which widens the delete to every test run; should say rerun with the same args.
- cr-scripts-4 + release.sh: release.sh tag does run the release check locally, but release.yml (version job) never re-runs it, so a hand-pushed v* tag skips the pad gate entirely. Keeps cr-scripts-4 at minor (maintainer-only path).

## New (round 2)
- IMPORTANT cr-scripts-x1 release gate does not check tiers.toml matches the rows (scripts/pad/table:128, :17). Fix: in release_check, fail unless `git show sha:connectors/tiers.toml` == render_tiers(tiers from rows).
