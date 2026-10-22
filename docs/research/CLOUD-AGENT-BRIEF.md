# Cloud agent brief (after the 22 Oct local test day)

You work in a cloud container on `agentcommunity/goal-cup-proxy` (`main`, v0.6.1 tagged 22 Oct). You
have no staging access, no harness sign-ins, no secrets and no owner machine. Work in code, tests
and docs only. Open one pull request per topic from a branch; never push `main`; the owner merges.

Read first: [AGENTS.md](../../AGENTS.md) (hard rules), [DECISIONS](../DECISIONS.md) (D57, D62, D66,
D67), the [test-day results](results/2026-10-22-local-test-day.md) ("For the cloud agent", "What
still exists") and the fix plans `results/2026-10-22-fix-plan*.md` (v1-v10, all merged).
Every change: one test of the promise, `sh scripts/gate.sh fast` passes (as a non-root user; add
`127.0.0.1 api.e00.localhost auth.e00.localhost` to `/etc/hosts`), connector edits regenerate pins.

## Priorities

1. **Lock the "bring your own harness" contract.** Participants must be able to add any harness
   (owner: "bring your own harness, not limited to these four"). Make adding a harness a stable,
   versioned contract that core changes do not break:
   - Write down what a connector may rely on (schema version, required and optional fields,
     launch environment, homes, native roots, capture hosts, `[participant]` fields, decoder hooks)
     in `docs/CONNECTOR-SCHEMA.md`, with a compatibility promise and a deprecation rule.
   - Add conformance tests that pin that contract (a fixture connector written against today's
     schema must keep loading and launching), and a test that adding a connector needs no core
     change (no harness names in core: already a rule).
   - Make the contributor path one page: `skills/adding-a-capture-harness`, `HARNESS-ONBOARDING.md`
     and `HARNESS-SETUP.md` agree with each other.
   - Propose the lock as a new decision (D-number) in the PR; the owner approves it.
2. **Rename step 4:** replace `goal-cup-tracer` URLs with `goal-cup-proxy` (`packaging/install.sh`
   `repo=`, README, docs, workflows), keep old links working (GitHub redirects), test the installer
   URL logic.
3. **AI Gateway (Vercel) for Claude Code and Codex:** a design PR, not merged without the owner:
   how the recording proxy learns the Gateway host as data (D43, D62), what is recorded, what the
   participant configures (D67). See [VERCEL-COMPAT](VERCEL-COMPAT.md).
4. **Review today's merged changes again** (v1-v10 and the review fixes): correctness, fail-closed
   paths, secrets, tests that test shape. Fix real defects with tests, batched per area.
5. **Decoders:** Hermes native `session_*.json`; Amp `edit_file` results; Codex user/developer
   messages without the human marker (see plan v3 L5 notes).
6. **Restarts across session limits** (Sandbox caps a session at 24 h): design and tests with fake
   sessions for a run that crosses several stops and `goalcup continue`; rules question goes to the
   owner.

## Never
Touch staging, databases, buckets, secrets or real harness configs; change harness settings for
participants (D67); set a model base URL; merge or push `main`.
