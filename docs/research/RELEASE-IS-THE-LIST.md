# The release is the list; `--dev` is for admins (proposed D70)

> **Cut down on 25 Oct 2026. D70 as accepted is smaller than this design.** Four parts of it were
> not built and are not part of D70: the latest-release nudge (2.2), known-bad harness versions
> (2.6, `bad_versions`), the practice reminder at start (half of 2.7), and the admin-only `--dev`
> rule with its migration (2.5, `DEV_NOT_ADMIN`). Any member may run `--dev` practice or test; a
> `--dev` run never competes. What stands: the release is the list, `MIN_COMPETITION_VERSION`, the
> installer guard, the frozen `CONTROL_API_VERSION`, and the shorter practice thresholds. Read
> [D70 and D71](../DECISIONS.md#d70-the-release-is-the-list-the-server-refuses-old-versions-in-the-competition---dev-never-competes). The text below is kept as written.

*Proposed 23 Oct 2026 after a discussion with the owner, owner to approve. Plan:
[`docs/plans/2026-10-23-release-is-the-list.md`](../plans/2026-10-23-release-is-the-list.md).*

## 1. The problem

A connector decides what goalcup records: which model hosts go through the recording proxy, where
the harness's session files are read, and what environment the harness gets. So a run may count
only if its connector is one somebody reviewed. Today the reviewed connectors are the ones a
release pins (`crates/goalcup-protocol/pins/connector-dirs.txt`, compiled in). Three things are
wrong around that:

1. **The 6 Nov release looks like a hard cutoff** for new harnesses, because nobody plans releases
   after it, and nothing tells a player that a newer release exists.
2. **`--dev` is open to everyone.** It admits an unpinned connector, relaxes three load rules, and
   the server records the run as a normal competition run. The start line "its results are not a
   judged entry's" is enforced nowhere.
3. **A player learns about a broken harness version too late.** Hermes 0.21.1 and later write no
   session file. Nothing checks the version, and the warning that would catch it needs ten minutes
   of model traffic, which a ten-minute practice run never reaches.

A server-side list of accepted connectors was considered and dropped: a release already is a
reviewed list, and releases are cheap (CI builds them).

## 2. The decision (D70)

1. **The release is the list.** A connector is accepted when it is merged to `main` and shipped in
   a release, as today. Adding a harness during the cup means a merge and a release. The 6 Nov
   release is not a cutoff.
2. **Players run the latest release.** `goalcup setup` and `goalcup start`, when no run is open on
   this computer, ask GitHub for the latest release and tell the player to update when theirs is
   older. The check never blocks, waits at most 3 s, and says nothing if GitHub cannot be reached
   or under `--json`. This is one request to github.com per command (PRIVACY says so).
3. **The server refuses old versions in the competition.** The Worker has a setting,
   `MIN_COMPETITION_VERSION`. A competition open from an older goalcup is refused with the update
   line. It is set to the first D70 release before the competition opens, because older goalcups
   never send `dev` (so it is also what closes `--dev` for them); later the owner raises it when an
   old version must stop competing. `continue` is not checked, so a run already going is never cut
   off. The device request is refused the same way, so nobody approves in the browser only to be
   refused at open.
4. **Never update during a run.** goalcup shows the update line only when no run is open.
   `install.sh` refuses to replace goalcup while a run is recording (`--force` overrides).
   From the D70 release to 15 Nov 2026, `CONTROL_API_VERSION` is frozen (a test pins it), so even
   a forced update leaves the player able to talk to their running daemon.
5. **`--dev` is for admins and never competes.** A `start --dev` that opens a server run (the
   player is logged in) with a competition run is refused before approval. Every other `--dev`
   open and continue sends `dev: true`. The Worker refuses a `dev` competition run, and refuses any
   `dev` run from a member who is not an admin. An admin is a member whose confirmed
   `auth.users.email` matches an email in the website's `public.admins`, after trimming spaces and
   ignoring case. This is a second admin list next to the staging `test_operators`, and a looser
   match than the website's own `is_admin` (exact): both are deliberate.
6. **Setup checks known-bad harness versions.** A connector may list harness versions that are
   known not to record fully (`[participant] bad_versions`, an optional schema-1 field, D68).
   `setup`, and `start` before approval, probe the harness's version with its `version_args` and,
   on a match, say what is wrong and the fix. It is a warning, not a refusal. The first entry is
   Hermes 0.21.1 and later. Adding it changes the Hermes connector's pin, so players see it only
   after they update to the release that carries it.
7. **Practice proves the harness works, and start reminds.** `start` of a test or competition run
   reminds the player when this computer has no finished practice run with that harness (the
   predicate setup already used before #257: a practice run, same `connector_id`, uploaded; not a
   `dev` run). Practice runs, and a `continue` of one, use shorter "no session file" thresholds
   (five requests, three minutes), so a practice run catches a silent harness.

Setup does not start the harness itself: the practice run already does that end to end, and a
short start inside setup would add sign-in prompts, a model call and timeouts for less proof.

## 3. What the server checks

| Open or continue | Result |
|---|---|
| Competition device request or open, goalcup older than `MIN_COMPETITION_VERSION` | refused: `TOO_OLD` |
| Competition, `dev` (anyone, admins too) | refused: `DEV_COMPETES` |
| Practice or test, `dev`, not an admin | refused: `DEV_NOT_ADMIN` |
| Practice or test, `dev`, admin | opens |
| Anything else | as today |

The checks run in the order of the table, after the open's lost-reply answer (a retry of a run
that already exists still gets its run) and before the GitHub repository check. `TOO_OLD` never
applies to `continue`.

## 4. Where trust sits

- The pins stop an **unmodified** goalcup from running an unreviewed connector without `--dev`.
  A **modified** goalcup can report any digest and any version and leave out `dev`. That is true
  today and stays true; the trace review (CHEAT-CHECKLIST) is the control for it.
- The `dev` and admin rules close the honest path for D70 releases: a player who types `--dev`
  can no longer enter a relaxed connector in the competition, and a non-admin cannot use `--dev`
  at all. Older goalcups never send `dev`, so for them `MIN_COMPETITION_VERSION` is what closes
  it; until it is set, an old goalcup's `--dev` run still enters the competition.
- The admin check is one new `SECURITY DEFINER` function the Worker calls only when `dev` is true.
  The Worker never sees the admin list.

## 5. Rollout and compatibility

- **Database:** one additive migration that adds one function,
  `goal_cup_upload_member_is_admin`. No table, and no change to the run-open functions, so the live
  Worker is not affected.
- **Worker:** an old CLI never sends `dev`; a missing `dev` is `false`. `MIN_COMPETITION_VERSION`
  is a `wrangler.toml` var (a dashboard value is overwritten by the next deploy). It ships empty,
  then is set to the D70 release's version once that release is out, before 6 Nov 2026 12:00 UTC.
  A malformed value refuses competition opens and competition device requests (500
  `misconfigured`); heartbeats and uploads of running runs are never affected. A `dev` value that
  is not a boolean (including `null`) is refused as malformed.
- **CLI:** ships in the next release. Older CLIs simply do not nudge, do not warn about bad
  versions, and do not send `dev`; the Worker still refuses a non-admin `dev` run from a new CLI,
  and `MIN_COMPETITION_VERSION` can stop old CLIs from competing.
- **Installer:** the guard asks the goalcup it is about to replace (the prefix's copy first, then
  the one on `PATH`) for `goalcup status --json`. A run is recording when the payload's health
  is `online` and the capture's run state is `starting`, `running` or `exited`; `sealed` does not
  block. When no daemon answers (it exits about 30 s after the last seal) nothing is recording, so
  the update goes on quietly, as it does for "no run on record". Anything else it cannot read gives
  the unsure line and the update goes on. `status` looks only at the newest run on record, so a newer finished
  practice run can hide an older recording one; that case is accepted for this cup.
- **Staging:** staging test members are synthetic (no `auth.users` row), so they are never admins.
  A `--dev` run against staging needs a real admin account.

## 6. Not in this design

- A server-side list of connectors, or reading `main` from the Worker.
- Fetching new connectors outside a release.
- Starting the harness inside `setup`.
- Renaming `Origin::Pinned` (after the cup).

## 7. The DECISIONS entry

Add to `docs/DECISIONS.md` once D68 (#243) and D69 (#257) are on `main`, and replace D68's
"Open: the pin and the cutoff" bullet with "Settled by D70":

> ## D70. The release is the list; players run the latest one; `--dev` is for admins.
>
> *Owner, 23 Oct 2026.* Settles D68's open pin question. A connector is accepted by being merged
> and shipped in a release; releases may continue through the cup, and the 6 Nov release is not
> a cutoff. goalcup tells a player when a newer release exists, never while a run is open, and
> `install.sh` refuses to update over a recording run. The Worker's `MIN_COMPETITION_VERSION`
> refuses competition opens from older goalcups. During the cup week a release keeps
> `CONTROL_API_VERSION`. `--dev` never enters the competition, and the Worker refuses it for a
> member who is not in `public.admins`. A connector may list known-bad harness versions, which
> `setup` and `start` warn about. D43 ruling 4 (a server list of digests) is withdrawn. See
> RELEASE-IS-THE-LIST (in `DECISIONS.md`, link it as `research/RELEASE-IS-THE-LIST.md`).
