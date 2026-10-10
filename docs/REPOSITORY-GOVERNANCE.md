# Repository governance

How is `main` of `agentcommunity/goal-cup-tracer` protected, and how does a maintainer apply and
check those settings? For maintainers and for contributors who want to know what a PR must pass.
Updated: 2026-10-10.

**Status: proposed, not yet applied; read-back pending.** Nothing below is installed until its
read-back is recorded in the
[v0.5.1 evidence record](qualification/v0.5.1-public-release-cleanup.md), which holds every
observed value (settings, app IDs, check URLs). This guide holds the policy and the procedure.

## Policy for `main`

- Changes reach `main` only through a pull request. No force pushes, no branch deletion.
- Two required status checks, each bound to the app that produces it:
  - `check`: workflow `ci`, job `check`, produced by GitHub Actions;
  - the DCO App's check, expected context `DCO`, produced by the official
    [DCO App](https://github.com/apps/dco).
- Required checks are strict: the branch must be up to date with `main` before merging.
- One approving review. Paths in [`.github/CODEOWNERS`](../.github/CODEOWNERS) also need a code
  owner's approval. A new push dismisses earlier approvals. All review conversations are resolved
  before merging.
- GitHub never lets an author approve their own PR, so a reviewer is always someone else.
- Signed (GPG or SSH) commits are not required. That is a different feature from DCO sign-off.

`check` can only be required once the `ci` workflow on `main` runs for every PR, docs-only ones
included. A path-filtered workflow leaves a docs-only PR waiting on an "Expected" check forever.
Require `check` only after the unfiltered workflow is on `main` and has been observed on a PR.

## Code owners

Every owned path in CODEOWNERS is owned by `@nembal`. Read from the plan baseline of 2026-10-10:
`@nembal` has admin on this repository (this session could not read the permission API; re-read
it with the commands below before applying settings). A code owner needs write access, or GitHub
ignores the line; `codeowners/errors` must return no errors. Add a team or a second owner only
when that account exists and has write access.

## Temporary single-maintainer exception

Owner decision, 2026-10-10. While `@nembal` is the only maintainer, nobody else can approve a
maintainer's PR, so the repository admin role is a bypass actor on the `main` ruleset with
bypass mode `pull_request`:

- a maintainer still opens a PR and the checks still run, but may merge it without a second
  approval or code-owner review. The bypass also covers required checks, so a maintainer merges
  only with `check` and `DCO` passing;
- direct pushes, force pushes and deletion of `main` stay blocked for everyone;
- outside PRs get no exception: they need the PR, `check`, `DCO` and a code owner's approval.

This is an exception, not independent review. A maintainer change merged this way has had no
second reviewer; say so in the PR. GitHub lists bypassed merges in the ruleset's insights.

When a second maintainer with write access exists, remove the bypass actor (set
`bypass_actors` to `[]`), read the ruleset back, record it, and update this section.

If rulesets cannot be used, the fallback is classic branch protection with
`enforce_admins: false` (payload below). It is broader: an admin can then also push to `main`
directly. When a second maintainer exists, switch it to `enforce_admins: true`.

## DCO App

- Use the official DCO App installed for this repository only. There is no custom DCO workflow
  and no `.github/dco.yml`, so the app runs with its defaults.
- It checks every commit in a PR for a `Signed-off-by:` line matching the commit author. A failing
  check's details page tells the author how to fix it; CONTRIBUTING repeats the short version.
- Exemptions in the app's documented defaults: merge commits and commits authored by bot
  accounts are not checked. Remediation commits are off. Confirm these against the installed
  version and record what was observed.
- A user with write access can mark a failing DCO check as passed. That override is never
  ordinary remediation. Use it only when the app misreads a sign-off that is present and correct
  for the author, and leave a PR comment saying why. Never use it to accept a missing human
  sign-off; the author fixes the commit instead.
- Bind the observed context name and app ID from a real PR. Do not assume `DCO` or an ID.

## Private vulnerability reporting

[SECURITY.md](../SECURITY.md) tells reporters to use GitHub private vulnerability reporting. Enable
it, then read it back and expect `enabled:true`:

```
gh api --method PUT repos/agentcommunity/goal-cup-tracer/private-vulnerability-reporting
gh api repos/agentcommunity/goal-cup-tracer/private-vulnerability-reporting
```

## Applying the settings

1. Read the current state (read-only). A 404 for protection means absent, not passing. Keep any
   stricter setting that already exists; stop and ask if another maintainer's policy conflicts.

   ```
   gh api repos/agentcommunity/goal-cup-tracer/actions/permissions
   gh api repos/agentcommunity/goal-cup-tracer/branches/main/protection
   gh api repos/agentcommunity/goal-cup-tracer/rulesets
   gh api repos/agentcommunity/goal-cup-tracer/private-vulnerability-reporting
   gh api repos/agentcommunity/goal-cup-tracer/collaborators/nembal/permission
   ```

2. With the unfiltered `ci` workflow on `main` and the DCO App installed, open or pick a PR and
   read the producing app of each check:

   ```
   PR_HEAD=$(gh pr view "$PR_NUMBER" --repo agentcommunity/goal-cup-tracer \
     --json headRefOid --jq .headRefOid)
   gh api "repos/agentcommunity/goal-cup-tracer/commits/$PR_HEAD/check-runs" \
     --jq '.check_runs[] | {name,head_sha,conclusion,app_id:.app.id,app_slug:.app.slug,html_url}'
   ```

   Take `check` from the `github-actions` app and the DCO context from the `dco` app.

3. Copy the payload below to a local file outside the repository and replace every `OBSERVED_*`
   placeholder with the value read in steps 1 and 2. **Never send a payload that still contains a
   placeholder.** The placeholders are deliberately invalid JSON so that a forgotten one fails.
   `grep -n OBSERVED_ "$RULESET_PAYLOAD"` must print nothing.

4. Apply: `gh api --method POST repos/agentcommunity/goal-cup-tracer/rulesets --input
   "$RULESET_PAYLOAD"` (fallback: `gh api --method PUT
   repos/agentcommunity/goal-cup-tracer/branches/main/protection --input "$PROTECTION_PAYLOAD"`).

5. Read back and compare field by field with the payload:

   ```
   gh api repos/agentcommunity/goal-cup-tracer/rulesets
   gh api repos/agentcommunity/goal-cup-tracer/rules/branches/main
   gh api repos/agentcommunity/goal-cup-tracer/branches/main/protection
   gh api repos/agentcommunity/goal-cup-tracer/codeowners/errors
   ```

6. Record timestamps, app IDs, PR URLs and the read-back in the evidence record, then change the
   Status line above. A blocked unsigned PR and a passing signed-off PR are the proof that the
   rules work; settings alone are not.

## Proposed ruleset payload

`OBSERVED_ADMIN_ROLE_ID` is the built-in repository admin role ID (expected `5`; confirm it in a
ruleset read-back). `OBSERVED_ACTIONS_ID` and `OBSERVED_DCO_ID` are the app IDs from step 2;
`OBSERVED_DCO_CONTEXT` is the DCO check's observed name (expected `DCO`).

```json
{
  "name": "main",
  "target": "branch",
  "enforcement": "active",
  "conditions": { "ref_name": { "include": ["refs/heads/main"], "exclude": [] } },
  "bypass_actors": [
    { "actor_type": "RepositoryRole", "actor_id": OBSERVED_ADMIN_ROLE_ID,
      "bypass_mode": "pull_request" }
  ],
  "rules": [
    { "type": "deletion" },
    { "type": "non_fast_forward" },
    { "type": "pull_request", "parameters": {
        "required_approving_review_count": 1,
        "dismiss_stale_reviews_on_push": true,
        "require_code_owner_review": true,
        "require_last_push_approval": false,
        "required_review_thread_resolution": true } },
    { "type": "required_status_checks", "parameters": {
        "strict_required_status_checks_policy": true,
        "required_status_checks": [
          { "context": "check", "integration_id": OBSERVED_ACTIONS_ID },
          { "context": OBSERVED_DCO_CONTEXT, "integration_id": OBSERVED_DCO_ID } ] } }
  ]
}
```

## Fallback branch-protection payload

```json
{
  "required_status_checks": { "strict": true, "contexts": [], "checks": [
    { "context": "check", "app_id": OBSERVED_ACTIONS_ID },
    { "context": OBSERVED_DCO_CONTEXT, "app_id": OBSERVED_DCO_ID } ] },
  "enforce_admins": false,
  "required_pull_request_reviews": { "dismiss_stale_reviews": true,
    "require_code_owner_reviews": true, "required_approving_review_count": 1 },
  "restrictions": null,
  "required_conversation_resolution": true,
  "allow_force_pushes": false,
  "allow_deletions": false
}
```

`OBSERVED_DCO_CONTEXT` becomes a quoted JSON string, for example `"DCO"`, once observed.
