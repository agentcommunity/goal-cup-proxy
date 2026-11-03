# One-hour test from scratch on the Mac (plan)

A participant's path, end to end, on staging: install the package, approve on agentcommunity.org, set up
Hermes, run one hour with many subagents, one recorder kill and one watchdog kill with `goalcup recover`,
seal, fetch the server copy and check it. Driven by Claude and its subagents through tmux; the owner only
approves on the website.

## Choices (defaults, owner to confirm)

| Choice | Default | Why |
|---|---|---|
| Harness | Hermes 0.22.6 (the owner's install, read-only) with the documented `ssl_ca_cert` config line in goalcup's private Hermes home | The version a new participant gets today; it also tests the version note and the workaround text |
| Model | DeepSeek (`deepseek-flash`), key written into the private home's `.env` by script, never printed | Funded; cheapest |
| Run mode | `goalcup start --test` against staging, approved by the owner on agentcommunity.org | The owner's member is the staging test member; no repo needed. Fallback: a competition run on `goalcup-dryrun-staging` with a new empty public repo |
| Home | fresh `GOALCUP_HOME=~/gc-test/home`, project `~/gc-test/proj-<time>` | Nothing from earlier runs |
| Prompt | `scripts/pad/prompts/paynest.txt` without its pad-only setup line | Many subagents in parallel (X2 target: 15, 10 at once) |
| Faults | recorder `kill -9` at about minute 20; watchdog `kill -9` and `goalcup recover` at about minute 40 | Point 3 on macOS, never run before |
| Keep going | typed when the agent stops, at most every 5 minutes | The one message participants may send |
| Cap | stop at 5 USD estimated spend | Hermes one-hour runs cost 2 to 2.1 USD on Linux |

## Steps

1. Build the release archive as `release.yml` does, write `SHA256SUMS`, install with
   `GOALCUP_RELEASE_BASE=file://… sh packaging/install.sh` into `~/.local/bin`. Check `goalcup --version`.
2. `goalcup setup --harness hermes-cli` with the fresh home: Hermes opens in its private home and installs its
   packages; provider DeepSeek; key written by script into the private `.env`; add the `providers.goalcup-ca`
   line to its `config.yaml`; quit.
3. `goalcup start --test --harness hermes-cli --prompt <paynest>` in an empty project folder, in tmux.
   Record what the start screen says (version, version note, tier line).
4. Owner opens the printed `agentcommunity.org/goalcup/approve?code=…` link and approves.
5. Paste the first message. Watch every minute: model calls, subagents, gaps, spend.
6. Minute 20: recorder kill. Minute 40: watchdog kill, then `goalcup recover` in a second shell.
7. Minute 60: `goalcup seal --yes`; wait for "server holds everything".
8. Fetch with `scripts/fetch-run.sh` (read-only), then `goalcup-verify check --server --local` and
   `goalcup-verify usage`. Keyscan the local folders.
9. Write the result into MAC-LEARNINGS.md and TEST-COVERAGE.md; clean up `~/gc-test`.

## PASS

- Install, website approval and setup work as the participant guide says.
- C1 no missed call; C2 subagents linked to the parent (by task logs for Hermes), 15 or more, 10 at once;
  C3 no secret beyond stated losses; C4 server copy equals local; C5 usage; C6 counts on screen match.
- Both kills leave a named gap of a few seconds; `goalcup recover` brings recording back; Hermes never stops.

## Small fixes before the run

- `packaging/install.sh:352` says "Next: goalcup login", a command 1.0 does not have.
- `docs/HARNESS-SETUP.md` still names `capture-homes/`.
- `goalcup setup` reads the harness version before Hermes installs its packages, so setup shows no version
  note on a fresh home; `start` should (to be shown by step 3).
