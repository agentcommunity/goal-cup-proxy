# Accepted connectors on the server (proposed D70)

*Proposed 23 Oct 2026, owner to approve. Plan:
[`docs/plans/2026-10-23-server-accept-list.md`](../plans/2026-10-23-server-accept-list.md).*

Owner: "connectors should not be pinned", and "`--dev` should need an admin login, so it is
not usable for non-admins". This design does both and keeps the one thing a pin protects.

## 1. The problem

A connector decides what goalcup records: which model hosts go through the recording proxy, where
the harness's session files are read, and what environment the harness gets. A connector nobody
reviewed could leave a host out of the recording, or send the model traffic to a fake model on the
player's own machine. So a run counts only if its connector is one somebody reviewed.

Today "reviewed" means "pinned": the four connector digests are compiled into each release
(`crates/goalcup-protocol/pins/connector-dirs.txt`). So:

- a new harness needs a merged PR **and** a new release, and the last release for the cup is
  6 Nov 2026 12:00 UTC (D68, "Open: the pin and the cutoff");
- `--dev` admits any unpinned connector, relaxes three load rules, and the server records the run
  as a normal competition run, because the server checks no list. D43 ruling 4 ("The server keeps
  acceptable connector directory digests as data") was decided and never built;
- `--dev` is open to every player, and the start line "its results are not a judged entry's" is
  enforced nowhere.

## 2. The decision (D70)

1. **The server holds the list.** A new table, `goal_cup_tracer.accepted_connectors`, holds the
   connector directory digests that may enter the competition, each with its `connector_id`.
   This builds D43 ruling 4 and amends its "the compiled pins stay the local gate". Accepting a
   new harness is one reviewed row, at any time, with no release, as long as the connector uses
   only what the release already knows: the closed lists of provider hosts, CA modes and redaction
   forms (D51) are compiled in, so a harness that needs a new entry there still needs a release.
2. **The competition needs an accepted connector.** The server refuses a competition open, and a
   competition continue, whose `(connector_dir_sha256, connector_id)` pair is not in the table.
   The check is in the database function, so every Worker, old or new, applies it.
3. **Strict rules for every run without `--dev`.** The CLI validates an unlisted connector with
   the same strict rules as a shipped one (`Origin::Pinned` validation: no interpreter `program`,
   the argument path rules, no loopback or single-label model host). It no longer refuses an
   unlisted connector itself; it says the server decides whether the run may count. The daemon
   keeps loading unlisted folders with its development rules (`harness.rs`); that is safe because
   it runs only the bytes the CLI already validated (D46), so the daemon does not change.
4. **`--dev` is for admins only, and never competes.** The CLI sends `dev: true` on open and
   continue. The server refuses every `dev` run in the competition, admins included, and refuses
   any other `dev` run unless the member is an admin. An admin is a member whose confirmed
   `auth.users.email` matches an email in the website's `public.admins` table, compared after
   trimming spaces and ignoring case (the website's own `is_admin` compares exactly).
5. **Practice and test runs take any strict connector.** A non-admin may practise or test with an
   unlisted connector loaded with the strict rules. This is how a contributor gets the evidence
   their connector PR needs. Those runs never count.
6. **The compiled pins stay as a local default, not a gate.** They still mark the shipped four as
   known, and a changed copy of a shipped connector under its shipped id is still refused (it
   must take its own id). `goalcup doctor` uses the same gate as `start`, so it also stops
   failing an unlisted connector. The offline verifier cannot see the server's list: its
   `Dev/unpinned` label is renamed to say "not in this build's list", and judges read the entry's
   `connector_accepted` and `dev` columns on the server. `goalcup status` shows a `dev` run as one.

## 3. What the server checks, in one table

The refusals are short forms here; the plan has the exact copy (`NOT_ACCEPTED`, `DEV_COMPETES`,
`DEV_NOT_ADMIN`). The checks run in this order: `dev` in the competition, then `dev` by a
non-admin, then an unaccepted connector in the competition.

| Run | Connector accepted | Connector not accepted |
|---|---|---|
| Competition, no `dev` | opens | refused: not accepted for the cup yet (`NOT_ACCEPTED`) |
| Competition, `dev` (anyone) | refused: a `--dev` run never competes (`DEV_COMPETES`) | same refusal |
| Practice or test, no `dev` | opens | opens |
| Practice or test, `dev`, admin | opens | opens |
| Practice or test, `dev`, not admin | refused: `--dev` is for admins (`DEV_NOT_ADMIN`) | same refusal |

`continue` applies the same table to the connector of the new part, after its `closed` and
`conflict` answers, so a run cannot open with an accepted connector and then continue with an
unlisted one, and a closed run still answers "closed".

## 4. Where trust sits

- The digest check stops an **unmodified** goalcup from entering an unreviewed connector in the
  competition. The digest the server sees is what the client reports: D46 binds the CLI and the
  daemon on one machine, and the signed receipt binds the server's digest to the trace, but
  nothing binds either to the bytes that really ran. A **modified** goalcup can claim any digest,
  exactly as it could with the compiled pins. This design does not make that worse; the trace
  review (CHEAT-CHECKLIST) is the control for it.
- The CLI's `dev` flag is honest-client policy in the same way. A changed client can send
  `dev: false`; it still cannot open a competition run with an unlisted digest unless it lies
  about the digest, which is the case above.
- A changed client in a practice or test run gains nothing: those runs never count.
- The admin check reads only `auth.users.email` and `public.admins`, inside a `SECURITY DEFINER`
  function the Worker already calls. The Worker never sees the admin list.
- Accepting a connector is an operator write to production (staging and production share one
  database). It is a reviewed row, like a pin was a reviewed line.

## 5. Rollout and compatibility

- **Seed.** The migration inserts the four digests that v0.6.1 and today's `main` pin (they are the
  same). If a release changes a connector, its new digest is added before that release ships.
  v0.6.0's digests are not seeded: a v0.6.0 player is told to upgrade.
- **Old Workers and old CLIs.** The old SQL signatures stay as wrappers that pass `dev = false`, so
  the live Worker keeps working and the competition check applies to it at once. The new
  overloads take `p_dev` with **no default**: a default would make the old Worker's call match two
  overloads, and PostgREST would refuse every open (the pending `20261021100000` file avoids
  defaults for the same reason). An old CLI never sends `dev`; the Worker reads a missing `dev`
  as `false`.
- **The dry run.** The production dry-run window is open now (23 Oct to 6 Nov). Dry-run players on
  v0.6.1 use seeded digests, so their opens are not refused.
- **Open runs** are not re-checked at open, but their next `continue` is. A competition run opened
  with an unlisted connector (only possible with `--dev` before the migration) cannot continue
  after it. The old Worker turns the new `forbidden` answer on continue into a `502`, which
  goalcup retries forever, so: count those runs and tell their players before applying, and deploy
  the new Worker (which answers `403` with the message) straight after the migration.
- **Order.** Migration (website repo, applied once), then the staging Worker and the production
  Worker at once, then the CLI in the 6 Nov release.

## 6. Where a player's own connector lives

No new search path for this cup. A player copies the shipped `connectors/` folder, adds their own
connector folder next to the four, and sets `GOALCUP_CONNECTORS_DIR` to the copy. What that means:

- The shipped four keep their folder name, id and digest, so they still load as themselves. A
  stray file in any folder (a `.DS_Store`, an editor backup) is refused.
- Every folder in the directory is validated with the strict rules on a start without `--dev`, so
  one folder that only passes the development rules blocks every start from that directory.
- The daemon starts with an empty environment and gets the directory through `daemon.toml`, which
  the CLI rewrites. A running daemon refuses a switch to another directory until 30 s after its
  last run is sealed: a player with an open competition run cannot start a practice run from
  another folder meanwhile.
- `continue`, `seal` and `status` load the connector by id from the current directory, so the
  variable must stay set for the whole run.

## 7. Not in this design

- Per-event lists (one global list is enough for one cup).
- A web page to accept connectors (an operator runs one SQL insert; OPERATING says how).
- Renaming `Origin::Pinned` to a name that says "strict" (after the cup).
- Re-checking open runs when a row is removed (removing a row stops new opens and continues only).

## 8. The DECISIONS entry

Add to `docs/DECISIONS.md` once D68 (#243) and D69 (#257) are on `main`, and replace D68's
"Open: the pin and the cutoff" bullet with a pointer to it:

> ## D70. The server holds the accepted connectors; `--dev` is for admins and never competes.
>
> *Owner, 23 Oct 2026.* Builds D43 ruling 4, amends its "the compiled pins stay the local gate",
> and settles D68's open pin question. A competition run opens or continues only with a
> `(connector_dir_sha256, connector_id)` pair in `goal_cup_tracer.accepted_connectors`; accepting
> a harness is one reviewed row, with no release when it uses only the compiled closed lists (D51).
> Every run without `--dev` loads its connector with the strict rules. `--dev` sends `dev: true`;
> the server refuses it for a member who is not in `public.admins`, and refuses every `dev` run in
> the competition. Practice and test runs take any strict connector. The compiled pins stay as
> the local default and keep refusing a changed copy under a shipped id; the server's record is
> the authority for judging. See [ACCEPT-LIST-DESIGN](research/ACCEPT-LIST-DESIGN.md).
