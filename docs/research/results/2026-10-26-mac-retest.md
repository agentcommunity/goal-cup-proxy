# Mac re-test, 26 Oct 2026 UTC (results)

A re-test on the owner's Mac (arm64) against staging, after the 24 Oct real-machine round
([results](2026-10-24-real-machine-round.md)). Builds: goalcup `main` at `c389b2dc` and later
(0.7 work: #268, #269, #270), staging Worker `d7762cab`. Harnesses: Hermes v0.22.5 ("latest") and
0.19.0, Codex (GPT-5.6 Luna, low effort), Amp. Raw logs, captures and traces stay on the Mac,
outside the repository. No secrets or tokens are in this file.

**Verdict: the Hermes recording fault is fixed, and one security claim was wrong.** Hermes on the
latest release now records, and the new status line catches a run that records nothing. A real run
with a repository works end to end. But the claim that the agent can reach only the run's
repository is not true on a normal Mac with an SSH key (finding 5). The docs are corrected in this
change.

## What was tested

1. Hermes v0.22.5 before the fix: why model calls were missing or failing (D72).
2. Practice runs after the fix, on four harnesses.
3. The R2 before-launch checks on staging.
4. A real test run with a repository (D71): the "Push to" block, the token, the pushed commit.
5. What the agent can reach when a player has an SSH key.
6. The setup and website screens a player meets.

## Results

| Case | Result | Notes |
|---|---|---|
| Hermes v0.22.5, before the fix, gateway left by `goalcup setup` | FAIL | Every model call went around the recording proxy. 5 KB recorded, status "Problems: none". |
| Hermes v0.22.5, before the fix, fresh gateway in the run | FAIL | Every call failed with `"api.deepseek.com" certificate is not trusted`. |
| Hermes latest, after #268 and #269, from a deliberately stale gateway | PASS | 14.4 MB recorded, no errors, no false warnings, "Problems: none". |
| Hermes 0.19.0, after the fix | PASS | 330 KB recorded, session file captured. `gateway stop -y` exits 2 on 0.19.0; logged as designed. |
| Codex (GPT-5.6 Luna, low) | PASS | 1.4 MB recorded, session file captured, "Problems: none". |
| Amp | PASS | 585 KB recorded, thread log captured, "Problems: none". |
| R2 before-launch checks on staging | PASS | Already recorded in `server/worker/README.md` (#270). |
| Real test run with a repository, "Push to" block pasted | PASS | Hermes pushed `hello.py` over HTTPS with the credential helper. The token was in no file, in `.git` or in any git setting. The server recorded the final repository head equal to the pushed commit. |
| Same, token with Contents read-only | FAIL (as expected) | GitHub answers 403 "Permission ... denied". The token needs Contents read and write. |
| Real test run without the block | FINDING | The agent used the player's own SSH key (finding 5). Nothing was pushed. |

## Findings

**1. Hermes 0.22 sends model calls around the proxy, or fails them (fixed in #268, #269).**
Hermes 0.22 runs every model call in a gateway process, one per `HERMES_HOME`. The gateway keeps
the environment of whoever started it. `goalcup setup` opens Hermes, which starts a gateway with no
proxy variables, and a later run attaches to it. So the run recorded almost nothing (5 KB), yet
status said "Problems: none". A fresh gateway inside the run fixed that but failed every call: on
macOS, Hermes 0.22 ignores `HERMES_CA_BUNDLE` and trusts only a provider entry's `ssl_ca_cert`.
The fix is two parts. #268 adds the `No model calls recorded yet` status line, for every harness.
#269 adds `[launch_prep]` (D72): goalcup writes the run's CA to a file, adds a `recorder` provider
entry for the model URL, and stops the old gateway before the launch.

**2. After the fix, all four harnesses record cleanly.** See the table. Hermes latest recorded
14.4 MB from a gateway that was left stale on purpose, with no errors and no false warnings. Hermes
0.19.0 exits 2 on `gateway stop -y`, which goalcup logs and ignores.

**3. The R2 before-launch checks pass on staging.** Recorded in `server/worker/README.md` by #270.

**4. A real run with a repository works when the "Push to" block is pasted (D71).** With the block
printed by `goalcup start` pasted at the start of the prompt, Hermes ran `git init`, set the
credential helper and pushed over HTTPS. The token stayed in `.env.local`. The server's record of
the final repository head equals the pushed commit. A fine-grained token with Contents read-only
fails with 403: the token needs Contents read and write.

**5. Known limit: the agent can use the player's own SSH key.** In an earlier test run without the
block, the agent (Hermes with deepseek-flash) did it unprompted, in about two
minutes: it overrode goalcup's `GIT_SSH_COMMAND` and used the player's key
(`~/.ssh/id_ed25519`, through `~/.ssh/config`). A dry-run push to any of the player's repositories
would have succeeded, including a private one the token cannot reach. OpenSSH finds `~/.ssh` from
the account, not from `HOME`, and the agent runs as the player's user. Nothing was pushed.
Therefore "the agent can reach only the run's repository" is **not true** on a Mac with an SSH
key. What stays true: the token reaches one repository, the token stays in its file, and goalcup
passes the agent no git login, SSH agent or `gh` setting. goalcup does not stop the agent from
using what the player's account can already do.
Mitigations today: protect the SSH key with a passphrase or a per-use confirmation (for example
the 1Password SSH agent with approval), or run in the Docker cell. Planned: a start-time warning
(0.7.x) and a macOS sandbox for the agent (later). The docs now say this in
[RULES](../../RULES.md), [PARTICIPANT-GUIDE](../../PARTICIPANT-GUIDE.md) and D71.

**6. Smaller findings.**
- Setup did not say clearly that the player must quit the agent to finish setup. Fixed in #270.
- The goal-cup skill said "168 hours" in practice runs. Fixed in #270.
- Prompts typed in the Claude app's Terminal panel sometimes needed a second Enter. This is
  tooling, not goalcup.
- The website approval page failed once on a first click, before hydration (`Origin: null`). Fixed
  on the website (nembal/agentcommunity_page#343).
- Codex's warm-up call counts as a model call, so a Codex run where nobody typed shows no "No model
  calls" line. Accepted.
- A staging trace verifies only with `--dev-key` from its `receipt.json`. Expected.

## Follow-ups

Added to [TODO](../../../TODO.md#after-the-26-oct-mac-re-test):

- A start-time warning when the player has an SSH key the agent can use (0.7.x).
- A macOS sandbox for the agent (later).
- Make the "Push to" block harder to miss, for example copy it to the clipboard.
- A simpler setup screen.
- Read Hermes's `state.db` (the native record on 0.21.1 and later).
- Hermes with Gemini is untested.
- Hermes's native Anthropic client does not record on 0.21.6 and later (D72).
