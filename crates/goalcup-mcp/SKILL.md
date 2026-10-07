---
name: goalcup-status
description: Read-only status for a /goal Cup run — how long is left, whether the tracker is still recording, and the round's own dates. Use when the agent needs the time or the state of the run it is inside. It changes nothing.
---

# /goal Cup status (read-only)

The `goalcup` MCP server answers three questions about the run this session is part of. It
is **read-only**: it reads the tracker's signed status cache off the disk, opens no socket,
and can change nothing about the run, the workspace or the tracker. There is no tool here
that starts, stops, seals, resumes or configures anything, and asking for one will not
produce one.

It is shipped with the tracker and **never** registered automatically. A participant who
wants their agent to have these tools registers this binary themselves.

For an M2 capture entry, `goalcup_run_status` and each `goalcup_entries` row report only
`entry_id`, `connector_id`, `m2_health: "unavailable"` and a `detail`: run `goalcup status`
for capture health. The m2-pilot bundle does not include this server.

## What the answers are worth

Every answer comes from a status document the tracker signed with this run's own key, and
the server checks each one against the key this machine pinned when the run was created —
**never** the key written inside the document. A document that does not check out comes back
as an error saying so, and none of its contents are reported. So:

- An answer you get is a fact the tracker signed.
- An error is not a fact about the run. It means this machine could not vouch for the
  document it found, and the number you would have read may have been written by something
  else.

## The three tools

```json
[
  {
    "description": "The round's timeline as the last signed status document carries it: t0, when the start window closes, when publication closes, and the rules version. `source` says where those dates came from — `config` means an organiser configured them on this machine, `fallback` means no round was configured and the fields are this entry's own timeline standing in for one. A `fallback` answer is not a deadline anybody set. Read-only.",
    "inputSchema": {
      "additionalProperties": false,
      "properties": {},
      "type": "object"
    },
    "name": "goalcup_event_status",
    "title": "Round status"
  },
  {
    "description": "How the current run is doing, from the signed status cache: arming state, health, seconds remaining, when the tracker last synced, and whether that document is stale (`offline`). `arming: \"refused\"` means the gate will never arm and waiting for it is waiting for nothing. Read-only.",
    "inputSchema": {
      "additionalProperties": false,
      "properties": {
        "entry_id": {
          "description": "Which run. Defaults to the most recently started one.",
          "type": "string"
        }
      },
      "type": "object"
    },
    "name": "goalcup_run_status",
    "title": "This run's status"
  },
  {
    "description": "Every run this machine holds a verified status document for, with its arming state, health and time remaining. `unverified` counts the cached documents that did NOT check out against a pinned run key and are therefore not in the list. Read-only.",
    "inputSchema": {
      "additionalProperties": false,
      "properties": {},
      "type": "object"
    },
    "name": "goalcup_entries",
    "title": "Runs on this machine"
  }
]
```

### `goalcup_event_status` — the round

`t0`, `start_window_closes_at`, `publication_closes_at`, `rules_version`, plus `source`,
`published_at`, `offline` and `freshness`.

**Read `source` before you use any of the dates.** `config` means an organiser configured
the round on this machine and the dates are the round's. `fallback` means no round was
configured, and the four fields are **this entry's own timeline standing in for one** — its
creation time and its own deadline. A `fallback` answer is not a deadline anybody set, and a
plan built on it is a plan built on a date nobody chose. Say so rather than presenting it as
the round's schedule.

### `goalcup_run_status` — this run

`entry_id`, `connector_id`, `arming`, `health`, `detail`, `started_at`, `deadline_at`,
`seconds_remaining`, `events_uploaded`, `events_pending`, `last_sync_at`, `published_at`,
`stale_seconds`, `offline` and `freshness`. Takes an optional `entry_id`; without one it
answers about the most recently started run.

- `arming: "refused"` means the gate will **never** arm — no start signal arrived, the wait
  timed out, or the harness home was already in use. It is not one more pending state, and
  waiting for it to clear is waiting for a thing that will not happen. `detail` says which
  refusal it was, in words meant for the participant.
- `offline: true` means nothing has published a status for this run recently, and
  `stale_seconds` says how long it has been. What you are reading is then the last document
  there was — `last_sync_at` and `published_at` are how old it is. It does not mean the run
  ended.
- `freshness` is that same judgement in a sentence: `published 30s ago`, or
  `published 4000s ago (stale)`, or **`published in the future by 45s (clock skew)`** when
  the document is dated after this machine's clock. A clock-skew answer means the daemon's
  clock and this machine's disagree, so the document's age is **not known**: `stale_seconds`
  is `null`, `offline` is `true`, and neither the age nor the countdown in that document can
  be placed in time. Say so rather than treating a skewed document as a fresh one. The same
  `stale_seconds: null` with `published at a time this server could not read` means the
  timestamp itself did not parse, and reads the same way.

### `goalcup_entries` — the runs on this machine

`entries` (one row per run: `entry_id`, `connector_id`, `arming`, `health`, `started_at`,
`deadline_at`, `seconds_remaining`, `last_sync_at`, `published_at`, `stale_seconds`,
`offline`, `freshness`), `count`, and `unverified` — how many cached documents did **not**
check out and are therefore not in the list. A non-zero `unverified` is worth mentioning to
the participant.

## When to call these

- The user asks how much time is left, or when the deadline is.
- You are planning work against a deadline and want to know whether the plan fits.
- Something looks wrong with the recording and you want to say what the tracker reports.

## When NOT to call these

- **Not on a schedule.** These are three files on a disk, not a clock; polling them in a
  loop wastes the run's time and tells you nothing new between the tracker's own writes.
- **Not to decide whether the run is compliant.** These tools report state, not judgement.
  What counts as a valid entry is decided by the verifier after the run, from the sealed
  evidence, and nothing here is that verdict.
- **Not as a source of a deadline when `source` is `fallback`.** See above: those dates are
  the entry's own timeline, not the round's.
- **Not for anything about the participant's machine.** There is no workspace path, no
  session id and no token in any answer, on purpose, and there is no tool that will produce
  one.
- **Not to act on an error.** A refusal here is a reason to tell the participant that the
  status could not be vouched for, and never a reason to try another route to the same
  number.
