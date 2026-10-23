# Local review brief (after the 22-23 Oct cloud session)

For the owner's local session with subagents. It covers three goals: review the project A to Z,
build a frame for adding many harnesses, and test fast on Linux, macOS and Windows. Read
[AGENTS.md](../../AGENTS.md) first, then this.

## 0. Where things stand

- **Open PRs from the cloud session.** Every PR body has an "Owner questions" list.
  - Merge first: #252 (Hermes 0.19.0), #249 (dry run guide), #255 (docs match the CLI), #242
    (false decode gap).
  - Wait for the dry run: #243 (connector contract, D68), #244, #240, #245, #246, #250, #251.
  - Park: #247 (event pick by error text), #248 (kit secrets), #254 (GitHub live numbers),
    #239 and #241 (designs).
- **Simple setup** (plan: `docs/plans/2026-10-23-simple-setup.md`, on branch
  `smart/upbeat-mendel-uvoxtr-setup` when it lands):
  - `goalcup setup` has 4 steps and no GitHub.
  - Each real run asks for its own repo at `start`.
  - goalcup holds no keys: the agent pushes with the player's own git login, and keys go in
    `~/goalcup-work/.env.local`.
  - This is the flow to test first on a real Mac.

## 1. A to Z review (one subagent per area, each read-only, each reports defects with a path)

| Agent | Area | Start from |
|---|---|---|
| A | Player path, from install to seal, as a first-timer. Time it. | README, PARTICIPANT-GUIDE |
| B | Recording and redaction: what can leak into an upload | ARCHITECTURE, CHEAT-CHECKLIST, `goalcup-daemon` |
| C | Ending a run: deadline, seal, kill, restart | TERMINATION, HOW-A-RUN-ENDS |
| D | Upload server and DB: refusals, the cron, emails | server/worker, UPLOAD-API, SERVER-DESIGN |
| E | Verifier and judging inputs | `goalcup-verifier`, RULES "Judging" |
| F | Docs: length, contradictions, what a player can skip | all player docs (now about 16,000 words) |

Ask each one for its top 5 findings only, ranked. Then a separate agent checks each finding
before anything is fixed.

## 2. The harness-adding frame

What exists today:
- A connector is one folder `connectors/<id>/` with a `harness.toml` (schema 1, see
  CONNECTOR-SCHEMA).
- Its pins are compiled in (`sh scripts/pin-manifests.sh`).
- There is a template test, and #243 adds frozen schema-1 fixtures and the contract.
- HARNESS-ONBOARDING says how a new one is measured.

What a "massive" frame needs, in this order:
1. **A candidate list with one row per harness:** OpenCode, Cursor CLI, Gemini CLI, Copilot CLI,
   Aider, Goose, OpenClaw, Cline CLI, Kilo. For each one, record:
   - how it signs in and where it keeps that sign-in;
   - whether it honours `HTTPS_PROXY`;
   - where its session file lives;
   - how it runs without prompts;
   - its side channels (RULES already lists flags for OpenCode, Cursor and OpenClaw).
2. **A scaffold:** `connectors/_template/` plus a checklist that a person or an agent fills in.
   No new code is needed for this.
3. **One conformance test per connector, with no real sign-in:**
   - The harness talks to a local fake model server through the recording proxy.
   - The test checks four things: a request is recorded, the session file appears, the version
     probe works, and the "native silence" warning (#251) fires when it should.
   - This is what makes adding 10 harnesses cheap. Build it once and reuse it.
4. **A real measurement per harness:** one short cell on a real machine (LOCAL-CAMPAIGN), done
   only after the conformance test passes.
5. Pins and a release. A connector the player adds after 6 Nov 12:00 UTC is not in the cup
   build (pins are compiled in). Say so in the "add your harness" link.

## 3. Testing fast on three systems

- **The repo is public, so GitHub Actions is free, including macOS runners.**
  - PR CI today runs on Ubuntu only. macOS runs only at release time.
  - Cheapest big win: add `macos-14` to the PR `check` job (fast tier).
  - Add a weekly full tier on both systems.
- **Windows today means WSL2** (D-notes, RULES). A `windows-latest` runner can run WSL2 with an
  Ubuntu image (for example the `Vampire/setup-wsl` action). Run the fast tier inside it on
  `main` only, so it does not slow PRs.
- **Real harnesses cannot run in CI**, because they need sign-ins. Use:
  - the conformance test above (fake model server) in CI, on every connector;
  - the local campaign kit for real cells: the Mac itself, Docker on the Mac for Linux, a WSL2 box;
  - a fresh macOS user account (or a Tart or UTM VM) for "zero to started run" timing.
- **Time budget idea:** PR CI under 10 minutes on Linux and macOS. Full tier and Windows/WSL2
  nightly.

## 4. Full native Windows (not WSL2)? A recommendation

**Not for this cup. Revisit after 8 Nov.** Native Windows needs new work in every layer that
touches the OS:
- process groups and killing (Job Objects, not signals);
- the terminal (ConPTY);
- keeping the machine awake (`SetThreadExecutionState`);
- the CA trust store and per-process proxy behaviour for each harness;
- credential stores (Windows Credential Manager);
- paths and file locking;
- each harness's own Windows build.

The termination contract (TERMINATION.md) would need a Windows section and its own tests. WSL2
covers Windows players today at a fraction of that cost.

## 5. Open questions to settle locally

- Does `goalcup start` really work without a practice run? The code suggests yes; confirm on a
  real machine.
- Claude Code's sign-in over a 7-day run: "not measured" (HARNESS-SETUP section 4).
- The `--dev` (unpinned connector) path: refuse it for competition, or let judges reject it?

## 6. What to test: the new setup and process (fast, real GitHub, staging)

This says **what** to test and what must be true. How to run it, and how to make it fast, is up to
the local session.

**Scope:** the simple setup (#257) and short runs only. No long runs yet. Each case below stands
alone, so it can be one subagent in parallel with the others. Every real run uses a throwaway
public repo, staging, and a cheap model. Where a case says "DB", check the staging `goal_cup_tracer`
tables (`runs`, `heartbeats`, `entries`/`chunks`, `notices`).

### A. Setup

| # | Case | Must be true |
|---|---|---|
| A1 | `goalcup setup` on a machine that is not logged in | Login runs inline; 4 steps; no GitHub question; no token asked |
| A2 | `goalcup prep` (old name) | Same as `setup` |
| A3 | Setup twice | Second run asks nothing already done; the keys file is not recreated or reopened |
| A4 | Keys file | `<work root>/.env.local` exists, empty plus comments, mode 0600; goalcup never reads it |
| A5 | Harness list | Installed ones first, then "also supported", then "your own harness" link |
| A6 | Sign-in with an API key instead of a subscription, each harness | The run later records; the key is not in any upload |

### B. GitHub at start (real runs only)

| # | Case | Must be true |
|---|---|---|
| B1 | New repo through the prefilled GitHub link | Repo is public and empty; start goes on; approval comes after the repo check |
| B2 | Repo made by hand, given with `--repo owner/name` | Same as B1, no question asked |
| B3 | Repo not empty (has a README) | Refused before approval, with the fix in one line; nothing on the server |
| B4 | Repo private | Refused before approval; nothing on the server |
| B5 | Repo you can read but not write | Push check refuses before approval |
| B6 | No GitHub login on the machine | Guide shown; after `gh auth login` and Enter, start goes on |
| B7 | Login only in the macOS keychain (no `gh`, no SSH) | Typing `<user>/<name>` works; the agent's push lands |
| B8 | SSH key, with and without passphrase | Agent's push lands |
| B9 | Second real run | Suggests `<name>-2`; the used repo is refused |
| B10 | Ctrl-C at each question | Nothing created on the server, no clone, no harness |
| B11 | GitHub API rate-limited | Start still goes on; the server still refuses a private repo |

### C. A mini run, end to end

Prompt the agent to do a tiny task fast (create a file, commit, push, stop).

| # | Case | Must be true |
|---|---|---|
| C1 | Practice run | No GitHub at all; the agent cannot push even if asked to add a remote; DB: a practice run, heartbeats, entries uploaded, sealed |
| C2 | Test or dry run | DB: run row with the right `repo_url` and event; heartbeats while it runs; entries arrive; the agent's push is on GitHub |
| C3 | Same run, after the end | Sealed; "Upload done."; DB: judged commit is the agent's last push before the deadline |
| C4 | `.env.local` with a test key, path named in the prompt | Agent uses it; `.env.local` is never committed; the key is redacted in the upload (or note exactly where it shows) |
| C5 | `goalcup status` during the run | Shows recording, upload state, no false problems |

### D. Edge cases (short runs)

| # | Case | Must be true |
|---|---|---|
| D1 | Harness writes no session file (wrong Hermes version) | The "no session file" warning shows (#243 bundle) |
| D2 | Agent exits early | `goalcup continue` resumes the same run; DB: same run, a new part |
| D3 | Clone folder deleted, then `continue` | Clear refusal (`clone_missing`), no new run |
| D4 | Network off for a few minutes mid-run | Recording goes on; upload catches up; DB shows no lost entries |
| D5 | Terminal closed mid-run | Daemon keeps recording; `status` from a new terminal shows it |
| D6 | `goalcup seal` mid-run | Asks first; with `--yes` it seals; upload finishes |
| D7 | Repo made private mid-run, then public again | What the player sees (today nothing); DB judged commit still found after |
| D8 | Agent force-pushes | DB/judging flags it |
| D9 | Disk nearly full (test override) | Agent paused, then resumed; no data lost |
| D10 | A second start of the same harness while one records | Refused; the first run is untouched |

### Not in this round
Long runs (24 h and 168 h), sign-in expiry over days, restarts after a platform cap, native Windows.
