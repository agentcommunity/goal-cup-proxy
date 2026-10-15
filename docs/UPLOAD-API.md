# Upload API, version 1

The exact wire contract between the tracer (`goalcup`) and the upload server. How the server is
built, and why, is in [SERVER-DESIGN](SERVER-DESIGN.md). Three things speak this contract and must
agree byte for byte: the tracer (`crates/goalcup-cli/src/upload/`), the stand-in server the tests run
(`crates/goalcup-cli/tests/common/upload_server/`), and the Worker (`server/worker/`).

A rule change touches all of them: the Worker (`server/worker/src/`), its local database
(`db-local.ts`), the SQL (the website's migration, copied to `server/worker/sql/goal-cup-tracer-upload.sql`)
and the Rust stand-in. Then run the suite that holds the Worker to the real contract:
`cargo nextest run --offline -p goalcup-cli --test full --run-ignored ignored-only -E 'test(/upload_worker/)'`
(it needs `npm ci` in `server/worker`; `sh scripts/gate.sh full` runs it too, and says so if that is
missing). It is not in CI.

## Common rules

- Base URL: what `goalcup login --server` stored (default `https://ingest.agentcommunity.org`,
  the production server; staging is `https://goalcup-ingest-staging.b-9a5.workers.dev`). Every
  path below is under it. `GET /v1/health` answers `200 {"status":"ok"}` and touches nothing.
- Every call except the three under "Sign in" carries `Authorization: Bearer gcr_<64 hex>`, the run
  credential. A missing, unknown or expired credential is `401`. A credential sees one run, the
  one it opened; any other run id is `404`, so a credential cannot probe for other runs.
- Request and response bodies are JSON (`Content-Type: application/json`), except the chunk body.
- Times are RFC 3339 UTC to the second, `YYYY-MM-DDTHH:MM:SSZ`.
- Identifiers: `run_id` is `r` and 16 lowercase hex digits; `entry_id` is `e` and 16 lowercase
  hex digits (the shape the daemon mints for a local-only run); hashes are 64 lowercase hex
  digits.
- An error is a status code and `{"error": "<code>", "message": "<one sentence>"}`. The tracer
  shows the message. Codes:

  | Status | `error` | Meaning | Tracer does |
  |---|---|---|---|
  | 400 | `malformed` | A field or header is missing or has the wrong shape. | stops, reports |
  | 401 | `unauthorized` | No credential, or not one the server knows, or it has expired. | stops: `goalcup login` again |
  | 403 | `forbidden` | The member may not start a run now (terms not accepted, outside the start window), or a device request was denied. | stops, reports |
  | 404 | `not_found` | No such run for this credential, or not this run's entry. | stops, reports |
  | 409 | `conflict` | The request does not continue where the server is (offset, `prev`, seq), the credential already opened a run, `previous_entry_id` is not the run's latest entry, or a seal comes before the final heartbeat. | chunk: re-reads the position (`GET`), then continues |
  | 410 | `closed` | The run is abandoned, or closed 24 hours after it ended; the entry is sealed; the request goes past what the final heartbeat committed; or the run is over and cannot be continued. A device request has expired or was already picked up. | stops, reports |
  | 413 | `too_large` | A body over its cap. | stops, reports (a tracer bug) |
  | 422 | `digest_mismatch` | The body's SHA-256 is not `Goalcup-Sha256`. Nothing was stored. | retries with backoff |
  | 429 | `rate_limited` | Too many calls. `Retry-After` says when. | retries with backoff |
  | 503 | `maintenance` | The server asks every tracer to wait. `Retry-After` says when. | retries with backoff |
  | 503 | `awaiting_final` | The run is over (`ends_at`), and the entry's final heartbeat has not arrived. | sends it, then retries |

  Any other 5xx, and any network failure, is retried with backoff. Backoff starts at 60 s,
  doubles to at most 15 min, with up to 25 % random jitter, and never waits less than a
  `Retry-After` (seconds) the server sent. Recording never waits on any of this.

## Sign in and the run credential

A participant has no password and pastes no token. The tracer asks for a code, the participant
approves it on the website in a browser, and the tracer picks up a run credential once. Only the
SHA-256 of the device code, the user code and the credential is stored on the server. The device
code is only ever in a request body, never in a URL or a log.

### `POST /v1/device` — start a request (no authentication)

```json
{"purpose": "login" | "run", "mode": "competition" | "practice" | "test",
 "connector_id": "codex-cli", "tracer_version": "<tracer version>"}
```

All four fields are required. `200`:

```json
{"device_code": "<64 hex>", "user_code": "BCDF-GHJK",
 "verification_uri": "<the website's approval page>",
 "verification_uri_complete": "<verification_uri>?code=BCDF-GHJK",
 "interval": 5, "expires_in": 900}
```

`user_code` is 8 letters from `BCDFGHJKLMNPQRSTVWXZ`, shown as `XXXX-XXXX`; the website accepts it in
any case, with or without the dash. `interval` is the seconds between polls, `expires_in` the
seconds a request lives, counted from its creation, whatever its state. The tracer shows the
participant `verification_uri_complete` and opens it when it can.

Both device calls are rate-limited. Starts count per client address, an IPv6 client by its /64
(production: 120 a minute). Polls count per device code (production: 30 a minute), so participants
behind one venue or campus NAT never share a poll limit; a per-address cap of 1200 polls a minute
only stops abuse. Over a limit is `429 rate_limited` with `Retry-After` set to the seconds left in
the minute (1-60). One tracer's normal use (one start, a poll every 5 seconds) stays far below it.

**Test runs (`"mode": "test"`).** An organizer's own test run. Only accounts on the server's
allowlist of test operators can approve one or open one (anyone else: the website refuses the
approval; an open is `403`). A test run belongs to the server's test event (production `test`,
R2 prefix `traces/test/`, never locked; staging `test-staging`), lasts that event's run hours
(168 by default), and its emails go to the operator with `[TEST]` before the subject. The same mode
replaces the same mode: a new test run replaces the operator's open test run, never a competition
run, and a competition run never replaces a test run. Test runs are left out of every live number and
research report, and `server/worker/scripts/delete-test-runs.sh` deletes all of them (rows and
R2 objects) in one go.

### `POST /v1/device/token` — poll a request (no authentication)

`{"device_code": "<64 hex>"}`. The answer is one of:

| Status | Body | Meaning |
|---|---|---|
| `202` | `{"status": "pending"}` | Not decided yet. Ask again after `interval` seconds. |
| `200` | `{"status": "approved", "purpose": "run", "credential": "gcr_<64 hex>", "expires_at": "…"}` | Approved for a run. The credential is minted now, returned this once, and stored hashed. |
| `200` | `{"status": "approved", "purpose": "login", "account": {"handle": "…"}}` | Approved for a login. No credential exists. |
| `403` | `forbidden` | The member denied it. Every poll answers `403` until the request expires. |
| `410` | `closed` | The request expired, never existed, or was already picked up. |

An approved request answers `200` to the first poll only; every later poll is `410`.

### The run credential

A credential stands for one member and one `mode`, the mode of the request that made it. It is
unbound until `POST /v1/runs` opens a run with it; that call binds it to the run. From then on
it may call only that run's paths, and `POST /v1/runs` with it again is the repeat described
there. The tracer keeps it in a file readable by the participant only, never in the harness's
environment, in a command line or in a URL.

A credential expires at `approved_at + 1 hour` while it is unbound, and at the run's
`deadline_at + 24 hours` once it is bound. After that it is `401`.

### `POST /v1/device/approve` — approve a request (tests and staging only)

`Authorization: Bearer <TEST_APPROVAL_KEY>`, body `{"user_code", "member_id", "approve": true|false}`;
`200 {"status": "approved"|"denied"}`. It stands in for the website's approval page, so a test needs
no browser. An unknown code is `404`, an expired request `410`, an already decided one `409`. The
route exists only when the server was given a `TEST_APPROVAL_KEY` and runs with the local database
or `STAGING=on` (unset: `404`). With a key anywhere else, every call answers `500 misconfigured`.
Production never has one.

## `POST /v1/runs` — open a run

Called by `goalcup start` after its local checks and before anything is created on disk.

```json
{"connector_id": "codex-cli", "manifest_sha256": "<hex>", "connector_dir_sha256": "<hex>",
 "workspace": "/abs/path/run-.../workspace", "session_id": "<uuid>",
 "harness_version": "0.153.0", "tracer_version": "<tracer version>",
 "repo_url": "https://github.com/<owner>/<repo>",
 "machine": {"os": "macOS", "cpus": 8, "ac_power": true}}
```

`201`: `{"run_id", "entry_id", "official_start", "deadline_at", "seconds_left"}`. The server mints
both ids and stamps `official_start` from its own clock. `entry_id` is the run's first entry,
segment 0.

`seconds_left` is the time left, on the server's clock: whole seconds from the server's now to
`deadline_at`, rounded up, and `0` once `deadline_at` has come (never negative). The open, the
continue and every heartbeat answer carry it. **The server is the clock:** the tracer counts the
run down from the last `seconds_left` it was given. It never reads the machine's own date to decide
when to stop the agent (see "When a run ends"). It counts to `deadline_at`, not `ends_at`: a run that a later start
replaced learns it from `stop`.

`deadline_at` = `official_start` + the run length: the event's run hours for a competition run,
`PRACTICE_RUN_MINUTES` (10) for a practice run. The credential's `mode` decides which: a `practice`
credential opens a practice run, in the practice event.

`machine` is optional: facts about the machine, a flat JSON object whose values are strings,
numbers, booleans or `null`, at most 4 KiB serialized (else `400`; `null` means absent). The server
keeps it on the run and never shows it. It also keeps the connection's IP address and country (from
Cloudflare) on the run: the open call's pair, which stays, and the latest pair, which each `continue`
and each heartbeat moves (a call without one keeps the last). They are never in an answer, a key or
a log.

`repo_url` is `https://github.com/<owner>/<repo>` exactly: the owner is ASCII letters, digits and
`-`; the repository is ASCII letters, digits, `-`, `_` and `.`, and is never `.` or `..`. Anything
else is `400`.

**One main run per account.** A competition start by the account replaces its other open competition
run: that run gets `replaced_at` = this run's `official_start`, in the same step, and ends then
(`ends_at`, below). A replaced run keeps its credential and goes on taking its wrap-up (below). A
practice run replaces nothing and is never replaced; an account may have any number of practice runs.

**The start window.** A competition run can be started only inside the event's start window (its
edges are inside). Before it opens the answer is `403` ("The start window has not opened yet.");
after it closes, `403` with the message "after the start window you can only `goalcup continue` your
run". Practice has no window.

A credential opens one run. A repeat of the same `session_id` (the one the open call sent, whatever
`continue` has added since) on a credential that already opened a run returns that run's answer
with `200`, whatever the run's state, so a reply lost in transit is not a second run. That includes
a retry that raced the original call. Another `session_id` on it is `409`: this credential already opened a run, and
the participant runs `goalcup continue`. The credential is bound to the new run in the same atomic
step that creates the run, so a credential that cannot bind (it is unknown, or was bound to another
run in the meantime) is the same `409` and creates nothing.

`chain_head_at_create` is not sent here. The chain's first link hashes the `entry_id`, and the
server issues the `entry_id` in this answer, so the head cannot exist yet. It goes to the receipt
call below.

## `POST /v1/runs/{run_id}/receipt` — the signed start receipt

Called once the daemon has created the entry under the server's `entry_id` and chained its
baseline.

```json
{"entry_id": "e…", "chain_head": "<hex of the baseline link>"}
```

`200`: `{"start_receipt": <StartReceipt>}`, in the verifier's existing format
(`crates/goalcup-protocol/src/receipt.rs`), signed with the server's Ed25519 key. Each entry has its
own receipt. Its claims are that entry's own fields: `entry_id`, `connector_id`, `workspace`,
`session_id`, `manifest_sha256`, `connector_dir_sha256` (what the open or continue call for that
entry sent), `chain_head` (from this request), and the run's `started_at` = `official_start` and
`deadline_at`. A repeat with the same `chain_head` returns the stored receipt; a different
`chain_head` once the entry has a receipt is `409`.

## `POST /v1/runs/{run_id}/abandon` — give back a run that never started

`{"reason": "<text, at most 500 characters>"}`. Accepted only while no chunk has been stored:
`200 {"status": "abandoned"}`, and `200` again for a repeat. A run with stored chunks is `409`; a
sealed run is `410`. The tracer sends a fixed sentence, never a local path. Used when `goalcup start` fails
after the run opened and before the harness launched (a daemon refusal, a full disk), so no run is
left open holding nothing.

## `GET /v1/runs/{run_id}` — where the server is

`GET /v1/runs/{run_id}` answers the run's current entry (the highest segment);
`GET /v1/runs/{run_id}?entry_id=e…` answers that entry (`404` if it is not this run's). `200`:

```json
{"run_id": "r…", "entry_id": "e…", "segment": 0, "status": "open",
 "official_start": "…", "deadline_at": "…", "ends_at": "…",
 "offset": 123456, "next_seq": 812, "head": "<hex>", "last_ack": <AckCheckpoint or null>,
 "stop": false, "final": {"seq": 812, "head": "<hex>"} or null}
```

`ends_at` is when the run is over: `deadline_at`, or `replaced_at` if a later start replaced the run
first. `stop` is `true` once the server's clock is at or past `ends_at`; `final` is the entry's
committed end (below) or `null`.

`offset` is how many bytes of the entry's `chain.jsonl` the server holds, `next_seq` the sequence
the next chunk starts at (0 before the first chunk: a chain's first record, the baseline, has
`seq` 0), `head` the chain hash of the last stored record (the 64-zero genesis before any), `last_ack` the acknowledgement of the last stored chunk. `status` is the run's: `open`,
`sealed` (the run is over and its current entry is sealed), `incomplete` (the run closed without it)
or `abandoned`.

## `PUT /v1/runs/{run_id}/chunks/{offset}` — store bytes of the chain

The body is one zstd frame holding bytes `[offset, offset + length)` of the entry's
`chain.jsonl`: whole lines only, at most 32 MiB compressed (`Content-Length` ≤ 33554432).
`Content-Type: application/zstd`. Headers:

| Header | Value |
|---|---|
| `Goalcup-Entry-Id` | the entry the bytes belong to (any entry of the run: an older segment keeps uploading its wrap-up) |
| `Goalcup-Length` | the uncompressed length in bytes |
| `Goalcup-Sha256` | SHA-256 of the request body (the compressed bytes) |
| `Goalcup-First-Seq` | the `seq` of the first record |
| `Goalcup-Last-Seq` | the `seq` of the last record |
| `Goalcup-Prev` | the `link.prev` of the first record |
| `Goalcup-Head` | the `link.hash` of the last record |

Before the body is read the server refuses a bad token, an entry that is not this run's (`404`), a
run that is closed, an entry that is sealed, a `Content-Length` over the cap and a missing or
malformed header. It then requires
`offset` = its `offset`, `Goalcup-Prev` = its `head` and `Goalcup-First-Seq` = its `next_seq`
(`409` otherwise), streams the body into storage while hashing it, and keeps it only if the hash
is `Goalcup-Sha256` (`422` otherwise, nothing kept). Storage is create-only.

`200`: `{"ack": <AckCheckpoint>, "offset": <new offset>, "next_seq": <last_seq + 1>, "head": "<hex>"}`.
The acknowledgement is the verifier's existing format (`crates/goalcup-protocol/src/ack.rs`):
`entry_id`, `seq` = `Goalcup-Last-Seq`, `hash` = `Goalcup-Head`, `acked_at` = the server's clock,
signed with the same key as the receipt.

**A retry is safe.** A chunk at an `offset` the server already holds, with the same
`Goalcup-Length` and `Goalcup-Head`, is answered `200` with the acknowledgement stored for it,
without reading the body, before anything about the run's state is looked at. The same `offset` with
a different length or head is `409`.

The server checks only these headers. Whether the bytes really are the chain they claim to be is
checked later by the verify job, which runs `goalcup-verify` over the reassembled copy.

## `POST /v1/runs/{run_id}/seal` — an entry is over

```json
{"entry_id": "e…", "seal_json": "<seal.json, verbatim>", "baseline_json": "<baseline.json, verbatim>"}
```

Each document at most 16 MiB. Sent after the entry's final heartbeat and its last chunk: a seal
needs the entry's committed end, and the server holding the chain up to it, checked in the step that
seals. Without a final heartbeat the answer is `409` ("send the final heartbeat first"), or `503
awaiting_final` once the run is over; a chain the server does not yet hold up to the end is `409`.

`200 {"status": "sealed", "run_status": "open" | "sealed"}`. The entry is sealed. The run is `sealed`
only when this is its current entry and the run is over (`ends_at` has passed); a segment sealed
before the end leaves the run `open`, so it can be continued. A repeat with the same documents is
`200` again, with the run's status then (`open`, `sealed` or `incomplete`). Different documents once sealed are `409`.

## `POST /v1/runs/{run_id}/heartbeat` — still running, and where the chain is

Every 5 minutes while the run records, and once more, `final`, when it has ended:

```json
{"entry_id": "e…", "seq": 812, "head": "<hex>", "offset_local": 123456, "final": false,
 "repo_head": null}
```

`seq` and `head` are the last whole record of the local `chain.jsonl` and `offset_local` the bytes
up to the end of it. The server stamps its own receive time on them, which commits the tracer to
that chain position before the bytes arrive. `repo_head` is the 40-hex commit the run's workspace
repository had checked out when the run ended (final heartbeat only; `null` when there is none).

Two optional fields may be added: `"recording": "ok" | "gaps" | "failed" | "unknown"` (how the
tracer sees its own recording) and `"model_requests": <u64>` (model calls seen so far, cumulative,
at most 2^53 - 1). A heartbeat without them is as before, and `null` means absent, as for
`repo_head`. The first `gaps` or `failed` flags the run, once. `model_requests` is counted per
entry, because a continued process counts from zero: the first value an entry reports, and any
value above the highest that entry has reported, moves the run's last model request to this
heartbeat; a lower or equal one changes nothing. Every heartbeat also sets the run's latest
heartbeat time (whichever entry sent it), and the server keeps the connection's IP address and
country on the heartbeat and as the run's latest pair, as for the open call. The notices that follow
from these are under "What the server does on its own".

`200`: `{"stop": <bool>, "deadline_at": "…", "seconds_left": <u64>, "reason": "deadline" | "replaced" | null}`.
`seconds_left` is the time left (see the open call). `stop` is
`true` once the server's clock is at or past `ends_at`: **the run is over**. The tracer then stops
the agent at once, seals, and sends the final heartbeat. `reason` says why: `"replaced"` when a
later start replaced the run, `"deadline"` when `deadline_at` came, `null` while `stop` is `false`.

**The final heartbeat commits the end of the entry.** Its `(seq, head)` is the last record the
server will ever accept for that entry. A repeat with the same `(seq, head)` is `200`; a different
one is `409`. Any entry may send its own final heartbeat at any time: before `ends_at` (the agent
ended early) or after it, until the run is abandoned or closed.

## When a run ends

A run ends at `ends_at` = `min(deadline_at, replaced_at)`: at its deadline, or when the account's
later competition start replaced it. At `ends_at` the run is over, in whatever state the agent is
in. There is no grace period and no late window.

- **The server says stop.** Every heartbeat answered at or after `ends_at` carries `stop: true` and
  the `reason`. The tracer stops recording at once. Without a network it stops on its own count:
  the `seconds_left` of the last answer it had (the open, the continue or a heartbeat), counted
  from just before that call on a clock that keeps running while the machine sleeps (so it ends
  early by the call's round trip, and never more than a second late: the server counts its clock in
  whole seconds). Each answer
  sets the count again, and the server's answer wins. The machine's date is never read for it, so
  changing the date does not move the stop. (A replaced run learns of it from its next
  heartbeat.) The count decides only the stop. For now, `goalcup status`, `goalcup continue` and the
  disk reserve still read the machine's date to decide that a run is over. An answer from an older server has no `seconds_left`; the tracer then counts
  `deadline_at − official_start` from when the run opened, and for a continued segment
  `deadline_at` less the machine's date.
- **The wrap-up upload has no time limit, but it can only complete each entry's chain up to its
  committed end.** After `ends_at`, a chunk is accepted only once the entry's final heartbeat has
  committed `(seq, head)`, and only if its `Goalcup-Last-Seq` is at most that `seq` (and the chunk
  ending at that `seq` has that `head`); anything else is `410 closed`. Before the final heartbeat
  arrives, a chunk after `ends_at` is `503 awaiting_final` with `Retry-After`, so a tracer still
  uploading asks again after it has sent its final heartbeat. The seal needs the committed end too
  (see the seal call). Every entry of the run, not only the current one, is accepted, so an older
  segment finishes its wrap-up.
- **Before `ends_at`** an entry that has a committed end accepts nothing past it either.
- **Closed.** 24 hours after `ends_at` the server closes the run (the cron, or the first call that
  names it): `sealed` if its current entry is sealed, else `incomplete` (the last piece is missing;
  the run is judged on what arrived). It accepts nothing more (`410`), including a repeat of the
  final heartbeat; only a chunk it already stored and a seal it already answered are answered again
  (`200`). A run that ended at its deadline closes when its credential expires, so for it the
  participant meets `401`, not `410`. `abandoned` is only the participant's own `abandon` call
  before any chunk.
- Which records count is the verify job's: a record stamped after `deadline_at` does not, however
  it arrived.

## `POST /v1/runs/{run_id}/continue` — one more segment of the same run

Called by `goalcup continue` on the machine that started the run, with that run's credential. A run
is one or more entries (segments): segment 0 is made by `POST /v1/runs`, each `continue` adds the
next. Every entry has its own chain, receipt and seal, and the receipt, chunk, heartbeat and seal
calls address the entry they name.

```json
{"previous_entry_id": "e…", "connector_id": "…", "manifest_sha256": "<hex>",
 "connector_dir_sha256": "<hex>", "workspace": "…", "session_id": "…",
 "harness_version": "…", "tracer_version": "…"}
```

`201 {"entry_id", "segment", "official_start", "deadline_at", "seconds_left"}`: the new entry, and
the run's own `official_start` and `deadline_at` (a segment is not a new run, and the deadline does
not move). `seconds_left` is what is left of the run now, on the server's clock (see the open call);
the continued segment counts down from it.
The previous entry is not sealed or closed by this; it may still upload its wrap-up.

- `previous_entry_id` must be the run's latest entry, else `409`.
- At or after the run's end (`ends_at`) the answer is `410`, as it is for an abandoned or closed run.
- The same call again (the latest entry was made by a `continue` with this `session_id` after this
  `previous_entry_id`) is `200` with the same answer, so a lost reply is not a second segment.

## What the server does on its own

A cron runs every 5 minutes. It does four things, in this order, and a step that fails is logged
and does not stop the next: it closes runs that are over (so a run's status is final without any
call naming it), it records the GitHub commit at each deadline, it queues the run emails in an
outbox, and it sends them. A tracer sees none of this. Each outbox row is sent once through Resend
(`server/worker/src/mail.ts`); the database writes the email, picks the member's address and checks
suppression. A staging Worker is never given an address by the database and sends every email to
its test address. (While the maintenance switch is on the
cron does nothing, since no tracer can send a heartbeat.)

**The commit at the deadline.** For a competition run that reached its deadline (not a replaced run,
not practice), the server asks GitHub for the head commit of `repo_url`
(`GET /repos/{owner}/{repo}/commits/HEAD`, 8 seconds, at most 50 runs per tick, oldest first) and
keeps the sha, with the time it was asked (`final_commit_at`). It is asked on the first tick at or
after `ends_at`, not at that instant, so it may include pushes made in the minutes after the
deadline; the final heartbeat's `repo_head` is the tracer's own account. GitHub saying the
repository does not exist or is empty (`404`, `409`) is kept as "no commit". Any other answer, or
none, keeps nothing and the next tick asks again. Runs that ended more than 48 hours ago and are
closed are not asked.

**The six emails.** Each is queued at most once per run, kind and key. None is queued for a practice
run, and none after the run's end except `ended`. A test run mails its operator, with `[TEST]` in
the subject.

| Kind | Queued when | Once per |
|---|---|---|
| `started` | a competition run is open (before its end) | run |
| `silence` | the run's last heartbeat (or its start, if none yet) is more than 20 minutes old | last heartbeat time |
| `idle_3h` | the run is beating (last heartbeat within 20 minutes), has reported `model_requests` at least once, and its last model request is more than 3 hours old | last model request |
| `idle_6h` | the same, 6 hours | last model request |
| `recording` | a heartbeat has flagged the run (`gaps` or `failed`) | run |
| `ended` | the run reached its deadline (a replaced run is not the main run and has none) and the commit was recorded, or an hour has passed without it | run |

The last model request is unset until a heartbeat first reports `model_requests`, so a tracer that
never reports gets no idle email; once set, it moves whenever any entry's count first appears or
grows (a continue's new process reporting 2 after the old one reached 50 counts). A run that comes back
and goes quiet again gets a new `silence`; a new model request starts a new idle period. The
`ended` payload holds the end time, the commit (or none) and the repository.

**What is stored, and only on the server.** The outbox row holds the run, the member, the kind, the
key and a small payload (times, hours, the commit, the repository). The run holds the machine facts
from the open call, the IP address and country of the open call, and the latest pair (moved by each
`continue` and heartbeat). None of it is in any answer, key or log, and none of it is public.
