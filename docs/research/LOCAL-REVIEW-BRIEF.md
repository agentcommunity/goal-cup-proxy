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

## 6. Real-key test plan (owner's machines)

The cloud session could not test these: they need real sign-ins, real GitHub and a real desktop.
Run them with **#257 (simple setup)** built locally, against **staging** only. Each test has a pass
line written before it runs. Write down every stop and every confusing line; each one is a fix.

### 6.1 Zero to started run (time each one)

| # | Machine | Harness | GitHub login | Pass line |
|---|---|---|---|---|
| Z1 | Fresh macOS user account | Claude Code | SSH key (no passphrase) | setup under 5 min of your own time; `start` creates the repo and the agent's first `git push` lands |
| Z2 | Same Mac, another fresh user | Codex | HTTPS, macOS keychain only (no `gh`, no SSH) | `start` asks `<user>/<name>`, the push check passes, the agent's push lands |
| Z3 | Same Mac | Claude Code | `gh auth login` (HTTPS, "Authenticate Git" = Yes) | the agent's push lands through `gh` under its scratch HOME |
| Z4 | Same Mac | any | none at all, no GitHub account | the guide is clear: sign up, install `gh`, log in, press Enter, done; time it |
| Z5 | Linux desktop (Docker is not enough for keyrings) | Hermes 0.19.0 | HTTPS with libsecret or `gh` keyring | push works through `XDG_RUNTIME_DIR`/`DBUS_SESSION_BUS_ADDRESS` |
| Z6 | WSL2 | Amp | SSH key with a passphrase in `ssh-agent` | push works through `SSH_AUTH_SOCK` |

### 6.2 Things only a real machine can answer

1. **Does the push check fail without write access?** Make a repo you can read but not write (a
   friend's public repo). `goalcup start --repo friend/repo` must refuse before the approval.
2. **Practice with no GitHub at all:** `goalcup start --practice` on Z4 before any GitHub step. The
   agent must not be able to push (try asking it to add a remote and push).
3. **Keys file:** put a test key in `~/goalcup-work/.env.local` and prompt the agent to use it.
   Check:
   - the agent finds it with only the path in the prompt;
   - `git status` in the clone never shows `.env.local` (the `.git/info/exclude` entry);
   - the uploaded record has the key redacted, or note exactly where it shows.
4. **API-key sign-in instead of a subscription:** for each harness, sign in with an API key in step 3.
   The run must record and the key must not appear in the upload.
5. **Claude Code over a long run:** a 24 h cell (dry run) to see whether the sign-in survives
   ("not measured" in HARNESS-SETUP section 4).
6. **Repo turned private mid-run:** make the dry-run repo private for 10 minutes, then public. Note
   what the player sees (today: nothing until the end email). This decides whether to build the
   "repo went private" email.
7. **Ctrl-C** at "Name it" and at "Press Enter": nothing is created on the server.
8. **Rate limit:** run `start` behind a busy shared network (or 61 quick checks); the start must
   still go on.

### 6.3 Keys and accounts for the test

- Use throwaway GitHub repos and fine-grained tokens with access to one repo only. Revoke them
  after.
- Use cheap models (LOCAL-CAMPAIGN rules: Codex `gpt-5.6-luna` low, Claude `haiku`).
- Never use a production model key or a production DB.
- Staging only. Production refuses test runs by design.
