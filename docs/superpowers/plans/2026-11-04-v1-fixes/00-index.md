# goalcup 1.0 fixes and multi-version harness support: Implementation Plan (index)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Each lane file is its own plan; this index says what runs in parallel, the merge order and the shared rules.

**Goal:** Fix every confirmed finding of the 3 Nov 2026 review and make goalcup run, record and report any installed harness version, before the event (end of November / early December 2026).

**Architecture:** 12 lanes, each owning a set of files, each on its own branch `fix/<lane-id>` in its own Worktrunk worktree, cut from `v1.1/integration` (made from origin/main 70eecdfb). Lanes merge by PR into `v1.1/integration` in the order below. Connector data becomes the single source (`connectors/*.toml`, read by both binaries through `goalcup-format`), versions are parsed in one place (`connector/version.rs`), and per-version behaviour lives in `[[versions]]` rows. One code freeze and one release-row run at the end, then tag 1.0.0.

**Tech Stack:** Rust (goalcup, goalcup-verify, goalcup-format; nextest), Python 3 (scripts/pad, release scripts), TypeScript Cloudflare Worker (server/worker, npm test), shell (install.sh, gate.sh).

**Spec:** the review report and the merged multi-version plan, copied next to this file when this plan is committed: `REVIEW-REPORT.md`, `MULTI-VERSION-PLAN.md` (source: the 3 Nov review run). The multi-version plan's dates are stale: the event moved, so its "1.1" tasks are in scope now.

## Global Constraints

- The seven points (CLAUDE.md, VISION.md) are the yardstick. Less code is better; cut before you add.
- goalcup never refuses a harness because of its version. It reads the version, records it, and tells the participant one plain line. "Verified" = a list of (version, os) pairs computed from committed pad rows.
- Hermes must work for participants on 0.22.5, 0.22.6 and current (0.22.7) on Linux and macOS, with no pad-only config.
- Two videos are required: one before the run (setup/plan) and one after (demo).
- Platforms: Linux and macOS (arm64). Every lifecycle test runs on the owner's Mac through `scripts/gate.sh`.
- CI stays lean: no new jobs or suites in `.github/workflows/ci.yml` (a macOS CI job is an owner question, not a task). `release.yml` may gain checks (lane G).
- Staging only. No production Worker deploy and no website migration inside any task; the owner does those (OPERATING.md).
- Never print or commit a key. Pad keys stay in `~/.pad-keys`.
- Worktrees only through `/Users/team/dev/tools/worktrunk/bin/wt`, under `/Users/team/dev/worktrees/`. Never `git worktree add/remove` by hand.
- Edit only files your lane owns; in a shared file, only the named functions/lines (see "Shared files").
- The full gate (`scripts/gate.sh full`, PATH with `~/.cargo/bin` first) runs on the Mac once per merge into `v1.1/integration`.

## Review Focus

Cross-lane conditions no single lane's tests cover; each is pinned by a test in the named task.
1. A participant behind a company proxy runs Hermes on macOS: the proxy upstream (B.3), the corp root (B.2) and Hermes `ca_settings` (H.4) must all apply in one run. Test: H.7's Mac rows run once with `HTTPS_PROXY` set to the B.9 mitmproxy.
2. goalcup is upgraded while a run is live: the pinned exe (C.2) and the installer refusal (C.8) must both hold. Test: C.2's respawn-after-replace test.
3. A run started on 1.0.0 as `opencode2-cli` is continued and verified after the fold: I.2's `RENAMED` table, `continue_run.rs` and the verifier arm. Test: I.2 continue-of-renamed-id test plus a verifier read of a 1.0.0 opencode2 record fixture.
4. A record written by 1.0.0 (no `harness_semver`, old `version_notes`, old shapes path) is read by the new verifier without a fail. Test: D.3 and I.6 run on the committed 1.0.0 fixtures.
5. `tiers.toml`, the tier line and `--release-check` agree on the same rows (Claude gateway exemption, Hermes C3 not exercised). Test: G.2's shared "counts" rule test, plus F.4/I.5 tier-line table tests read the G-rendered fixture.

---

## Lanes

| Lane | File | Tasks | Wave | PAD tasks | Model hint |
|---|---|---|---|---|---|
| A cli-ux | [A-cli-ux.md](A-cli-ux.md) | 16 (15 S, 1 M) | 1 | none | sonnet |
| B net | [B-net.md](B-net.md) | 9 (3 M, 6 S) | 1 | B.9 | opus |
| C lifecycle | [C-lifecycle.md](C-lifecycle.md) | 11 (2 M, 9 S) | 1 (C.11 last) | none | opus |
| D verify | [D-verify.md](D-verify.md) | 10 (5 S, 5 M) | 1 | none | sonnet |
| E connector-data | [E-connector-data.md](E-connector-data.md) | 4 (2 S, 2 M) | 1 | none | opus |
| F version-core | [F-version-core.md](F-version-core.md) | 5 S | 1 | F.5 acceptance | opus |
| G pad-release | [G-pad-release.md](G-pad-release.md) | 19 | 1 (G.1-G.14), 2 (G.20-G.24) | G.14, G.21, G.23 | sonnet |
| H hermes | [H-hermes.md](H-hermes.md) | 7 | 1 (H.1-H.2), 2 (H.3-H.7) | H.2, H.7 | opus |
| I version-overlays | [I-version-overlays.md](I-version-overlays.md) | 8 (2 M, 6 S) | 2 | none | opus |
| J server | [J-server.md](J-server.md) | 12 (7 S, 4 M, 1 L) | 1 | none | sonnet |
| K oss | [K-oss.md](K-oss.md) | 7 | 1 (K.1-K.5), 3 (K.6-K.7) | none | sonnet |
| L docs | [L-docs.md](L-docs.md) | 8 | 1 (L.1-L.7), 3 (L.8) | none | sonnet |

## Waves and merge order

**Wave 1: 11 lanes start at once** (A, B, C, D, E, F, G wave 1, H.1-H.2, J, K wave 1, L wave 1). They merge into `v1.1/integration` in this order; a lane merges when green, waiting only for its predecessors in the chain:
1. D.2 (moves `shapes.tsv` to `connectors/`, adds `goalcup_format::shared`).
2. E (glob in `crates/goalcup-format/build.rs`, `goalcup_format::connectors::CONNECTORS`, `[usage]`).
3. F (`connector/version.rs`, `harness_version.py`).
4. Any order: A, B, rest of D, G.1-G.14 (G.1-G.5 in order), J, K.1-K.5, L.1-L.7, C except C.11, H.1.
5. C.11 last (deletes single lines in files A, E, F edit).

**Wave 2: starts once E, F and C are merged.**
1. H.3 (`ca_settings` key) → H.5 (Hermes routes, drops `version_notes`, adds `checked = "0.22.5"`).
2. I.1 (`checked`, `[[versions]]`, removes `version`/`version_notes`/VersionMismatch) → I.2 (OpenCode fold) → I.3, I.4, I.6, I.7, I.8 (parallel) → I.5 after G.22.
3. G.20-G.23 (version matrix, `here_path`) in parallel with I.
4. H.4, H.6, then H.7 (PAD) after G.20/G.23.

**Wave 3: after everything above merged.**
1. L.8 docs sync (grep checklist; no `OWNER-Q` markers left).
2. K.7 gate lines, then K.6 (moves `docs/plans`, `docs/research`, `docs/superpowers`, `docs/archive` to the `archive` branch; this plan folder goes with it).
3. Freeze: full gate on the Mac, then G.14 release rows, then `table --release-check`, then tag 1.0.0 (`scripts/release.sh tag 1.0.0`).

## Shared files (edit only your part)

| File | Lanes and parts |
|---|---|
| `crates/goalcup/src/cli/start.rs` | A: `check()`, `hand_over()`, first-message print; B: one Egress line after ~292; F/I.5: `tier_line` ~442; C.11: delete `build`/`server_time` lines |
| `crates/goalcup/src/cli/text.rs` | A: non-version texts; F: version/tier texts ~176-182, ~1158 |
| `crates/goalcup/src/connector/{schema,mod}.rs` | E (wave 1), A.4 id lookup ~156-204, F ~242; wave 2: H.3 then I.1 |
| `crates/goalcup/src/cli/setup.rs` | A: tty/sign-in check; B: one item in the array ~238-243; F: version probe |
| `crates/goalcup/src/cli/{approve,continue_run}.rs` | A owns; J: one field + one call each (notes); I: continue_run connector-id check ~141-150 |
| `crates/goalcup/src/pipeline/mod.rs` | C owns; B.7: one field in `ProxyCtx` ~846-850; I.4: version-read hunk |
| `crates/goalcup-format/src/lib.rs` | D.2 then E: one line each |
| `scripts/gate.sh` | G: ~55; K.7: link check + personal-path grep; J: SQL step; C: none |
| `scripts/pad/check.py` | G owns; add `connectors/` and `crates/goalcup-format` to the `trusted_at` paths (~917) for E |
| `docs/*` | L owns; other lanes edit only the lines their file names; L.8 reconciles |

## Coordinator rulings (override the lane files where they differ)

1. **Hermes CA:** the macOS problem hits every Hermes since 0.21.6 (H). Fix = base key `ca_settings` (H.3/H.4). No `ca_env` and no `ca_settings` inside `[[versions]]` rows (I.1 tests it). **Drop G.24**; G.5 keeps the `pad_only` mechanism but carries no Hermes 0.22.6 data.
2. **Mac rows per version:** G adds the pad key `here_path` (a PATH prefix per run, for installs under `/Users/team/dev/harness-installs/`) to G.20; H.7 consumes it.
3. **Pinned exe:** C.2 puts `<entry>/rec/goalcup`; G's pad must skip `rec/goalcup` when copying run folders.
4. **Ownership gaps:** `cli/status.rs` → A (A.7). `packaging/README.md:32-34` → C (C.8). `skills/adding-a-capture-harness/SKILL.md:20` (`spikes/*/out/`) → E.4. `crates/goalcup-verify/tests/usage_fixtures` scrub → K.3. `crates/goalcup/tests/connector_launch.rs:150-185` → F.3.
5. **Tiers depth:** keep the last 5 verified builds per (harness, os) until the owner says otherwise.
6. **Findings not planned now (post-event):** arch-4 (`run`/`ctl` leaf modules), the recorder failure domain (keep the relay alive across recorder restarts), the full proxy drain beyond C.7, signed releases (oss-7). Refuted, no work: cr-upload-server-2, oss-5, docs-5, docs-x1, docs-x2. arch-6 is covered by I.2.

## PAD queue (serial on the owner's Mac)

Real harness runs share one signed-in home and key budgets, so they run one at a time, in this order:
1. F.5 acceptance: fresh Hermes home `goalcup setup`, one `core-hermes` micro row.
2. H.2 spike: Hermes 0.22.5 on Linux and Mac micro rows without the pad CA block.
3. B.9: one Claude Code practice run behind mitmproxy (needs mitmproxy installed: owner question).
4. G.21: one box run of the version-matrix image.
5. H.7: 12 Hermes full rows (Linux 0.22.5; Mac 0.22.5, 0.22.6, 0.22.7; direct, gateway, dodger).
6. G.23: version-matrix rows for the other CORE harnesses.
7. G.14 at the freeze: all CORE release rows on the frozen head, then `--release-check`.

## Owner questions (answer before the lane that needs it starts)

Blocking wave 1:
1. Event dates: start window date, when the setup video is due and where it is handed in (L.1-L.3).
2. Hermes trust entry: AGENTS.md:60 says goalcup never sets a model base URL; H.3's entry names the real provider address, never the proxy. Allow the exception? (recommended: yes)
3. `PAD_STAGING_URL` as an env var with no default in the repo (G)?
4. Installer: also refuse when a 1.0+ binary answers with JSON it cannot read (C.8)?
5. Drop the start-event `build` field (C.11), or fill it with the git commit from E's build.rs?

Blocking wave 2 or 3:
6. Hermes routes: ship all 9, or only the 2 the pad proved (H.5)?
7. Install three Hermes builds under `/Users/team/dev/harness-installs/` on the Mac (H.7), and mitmproxy for B.9?
8. Server: `ALERT_EMAIL`, `PRACTICE_MAX_BYTES` (1 GiB suggested), `STORAGE_ALERT_BYTES` (200 GiB suggested); who applies the purge migration in the website repo (J.7)?
9. Push a public `archive` branch for the old plans (K.6)?
10. Release must require Hermes 0.22.6 verified, or 0.22.5 is enough (G)?

Can wait: macOS system proxy/PAC and login-keychain roots (B); macOS orphans not ended at seal (C); "watchdog gone" status line (C); a recovered segment's null `version_row` (I); Worker refusal of a competition device request outside the window (A); macOS Intel support level (L); `docs/research/NOTIFICATIONS.md` still live? (K).

## Execution

Per lane: create the worktree with `wt` from `v1.1/integration`, run the lane file with superpowers:subagent-driven-development, open a PR into `v1.1/integration`, run the full gate on the Mac, merge in the order above. Lanes in the same wave run at the same time; each lane's tasks marked PARALLEL-OK can run at the same time inside the lane.
