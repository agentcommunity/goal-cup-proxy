# Repository governance

How is `main` of `agentcommunity/goal-cup-tracer` protected, and how does a maintainer apply and
check those settings? For maintainers and for contributors who want to know what a PR must pass.
Updated: 2026-10-11.

**Status: applied 2026-10-11.** The read-backs are in the
[v0.5.1 evidence record](qualification/v0.5.1-public-release-cleanup.md#applied-2026-10-11),
which holds every observed value (settings, app IDs, check URLs). This guide holds the policy
and the procedure.

## Policy for `main`

- Changes reach `main` only through a pull request. No force pushes, no branch deletion.
- One required status check, bound to the app that produces it: `check` (workflow `ci`, job
  `check`), produced by GitHub Actions.
- Required checks are strict: the branch must be up to date with `main` before merging.
- One approving review. Paths in [`.github/CODEOWNERS`](../.github/CODEOWNERS) also need a code
  owner's approval. A new push dismisses earlier approvals. All review conversations are resolved
  before merging.
- GitHub never lets an author approve their own PR, so a reviewer is always someone else
  (see the single-maintainer exception below).
- No DCO sign-off and no signed (GPG or SSH) commits are required
  ([D47](DECISIONS.md#d47-m2-no-dco-sign-off)). Instead, the person who opens a PR states in it
  that they read the whole diff and discloses any AI assistance; code-owner review stays.

Since v0.5.1 the `ci` workflow has no path filter, so `check` runs on every pull request; a
docs-only PR finishes it quickly with the Rust smoke skipped. Require `check` only after it has
been observed on a PR: a required check that never reports leaves a PR waiting forever.

On a first-time contributor's fork PR, `check` may show pending until a maintainer approves the
workflow run. That is GitHub's default for Actions on fork PRs; approve it after reading the
diff for workflow or script changes.

## Code owners

Every owned path in CODEOWNERS is owned by `@nembal`, who has admin on this repository (read back
2026-10-11 in the evidence record; re-read it before changing settings). A code owner needs write
access, or GitHub ignores the line; `codeowners/errors` must return no errors. Add a team or a
second owner only when that account exists and has write access.

## Temporary single-maintainer exception

Owner decision, 2026-10-10. While `@nembal` is the only maintainer, nobody else can approve a
maintainer's PR, so the repository admin role is a bypass actor on the `main` ruleset with
bypass mode `pull_request`:

- a maintainer still opens a PR and the checks still run, but may merge it without a second
  approval or code-owner review. Bypass means GitHub does not enforce the required checks for a
  bypassing maintainer either; by policy a maintainer merges only with `check` passing;
- direct pushes, force pushes and deletion of `main` stay blocked for everyone;
- outside PRs get no exception: they need the PR, `check` and a code owner's approval.

This is an exception, not independent review. A maintainer change merged this way has had no
second reviewer; say so in the PR. GitHub lists bypassed merges in the ruleset's insights.

When a second maintainer with write access exists, remove the bypass actor (set
`bypass_actors` to `[]`), read the ruleset back, record it, and update this section.

Use either the ruleset or classic branch protection on `main`, not both. They stack and the
stricter rule wins, so leftover classic protection can override the ruleset bypass. When applying
the ruleset, remove classic protection or align it. If rulesets cannot be used, the fallback is
classic branch protection alone with `enforce_admins: false` (payload below). It is broader: an
admin can then also push to `main` directly. When a second maintainer exists, switch it to
`enforce_admins: true`.

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
   If classic protection exists, it must be removed or aligned when the ruleset is applied
   (see the single-maintainer exception above): the two must not both be active.

   ```
   gh api repos/agentcommunity/goal-cup-tracer/actions/permissions
   gh api repos/agentcommunity/goal-cup-tracer/branches/main/protection
   gh api repos/agentcommunity/goal-cup-tracer/rulesets
   gh api repos/agentcommunity/goal-cup-tracer/private-vulnerability-reporting
   gh api repos/agentcommunity/goal-cup-tracer/collaborators/nembal/permission
   ```

2. With the unfiltered `ci` workflow on `main`, open or pick a PR, set `PR_NUMBER` to its number
   (for example `PR_NUMBER=12`), and read the producing app of its `check` run:

   ```
   PR_HEAD=$(gh pr view "$PR_NUMBER" --repo agentcommunity/goal-cup-tracer \
     --json headRefOid --jq .headRefOid)
   gh api "repos/agentcommunity/goal-cup-tracer/commits/$PR_HEAD/check-runs" \
     --jq '.check_runs[] | {name,head_sha,conclusion,app_id:.app.id,app_slug:.app.slug,html_url}'
   ```

   Take `check` from the `github-actions` app. Bind the observed app ID; do not assume one.

3. Copy the payload below to a local file outside the repository, set `RULESET_PAYLOAD` to that
   path (for example `RULESET_PAYLOAD=$HOME/main-ruleset.json`), and replace every `OBSERVED_*`
   placeholder with the value read in steps 1 and 2. **Never send a payload that still contains a
   placeholder.** The placeholders are deliberately invalid JSON so that a forgotten one fails.
   `grep -n OBSERVED_ "$RULESET_PAYLOAD"` must print nothing.

4. Apply one of the two, never both: `gh api --method POST
   repos/agentcommunity/goal-cup-tracer/rulesets --input "$RULESET_PAYLOAD"`, then remove or align
   any classic protection; or, only if rulesets cannot be used, `gh api --method PUT
   repos/agentcommunity/goal-cup-tracer/branches/main/protection --input "$PROTECTION_PAYLOAD"`.
   Keep the `id` from the POST response as `RULESET_ID` (or read it from the list call).
   For the fallback, set `PROTECTION_PAYLOAD` to a filled local copy of its payload, as in step 3.

5. Read back and compare field by field with the payload. The list call returns summaries
   without `rules` or `bypass_actors`; the call by ID returns the full ruleset:

   ```
   gh api repos/agentcommunity/goal-cup-tracer/rulesets
   gh api "repos/agentcommunity/goal-cup-tracer/rulesets/$RULESET_ID"
   gh api repos/agentcommunity/goal-cup-tracer/rules/branches/main
   gh api repos/agentcommunity/goal-cup-tracer/branches/main/protection
   gh api repos/agentcommunity/goal-cup-tracer/codeowners/errors
   ```

6. Record timestamps, app IDs, PR URLs and the read-back in the evidence record, then change the
   Status line above. A PR that GitHub shows as blocked until `check` passes and a code owner
   approves is the proof that the rules work; settings alone are not.

## Ruleset payload (applied)

`OBSERVED_ADMIN_ROLE_ID` is the built-in repository admin role ID (expected `5`; confirm it in a
ruleset read-back). `OBSERVED_ACTIONS_ID` is the GitHub Actions app ID from step 2.

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
          { "context": "check", "integration_id": OBSERVED_ACTIONS_ID } ] } }
  ]
}
```

## Fallback branch-protection payload

```json
{
  "required_status_checks": { "strict": true, "contexts": [], "checks": [
    { "context": "check", "app_id": OBSERVED_ACTIONS_ID } ] },
  "enforce_admins": false,
  "required_pull_request_reviews": { "dismiss_stale_reviews": true,
    "require_code_owner_reviews": true, "required_approving_review_count": 1 },
  "restrictions": null,
  "required_conversation_resolution": true,
  "allow_force_pushes": false,
  "allow_deletions": false
}
```
