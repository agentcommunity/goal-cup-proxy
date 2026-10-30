# Upload API

The exact wire contract between goalcup 1.0 and the upload server (seam S5 of the 1.0 plan). Three
things speak it and must agree: the client (`crates/goalcup/src/upload/`, and `src/cli/` for the
device flow and the open and continue calls), the scripted fake Worker the fast tests use
(`crates/goalcup/tests/common/worker.rs`, which answers these shapes and keeps no rules of its own),
and the Worker (`server/worker/`).

A rule change touches the Worker (`server/worker/src/`), its local database (`db-local.ts`) and, if
the database must change, the SQL (the website's migration, mirrored in
`server/worker/sql/goal-cup-tracer-upload.sql`; 1.0 needs none). Then run `npm test` in
`server/worker` and the full-tier `upload_worker.rs`, which runs the real client against the Worker
under `wrangler dev` (`sh scripts/gate.sh full` runs both).

Gone since 0.7, and not coming back: the receipt, abandon and seal calls (`/receipt`, `/abandon`, `/seal`), signing and the signed `ack` in chunk answers, `last_ack`, `manifest_sha256` and `connector_dir_sha256`, the `tracer_version` alias, heartbeat `vitals`, and the per-run size limit (`413 quota_exceeded`).

## Common rules

- Base URL: the client's `GOALCUP_SERVER` (unset: `https://ingest.agentcommunity.org`, the
  production server; staging is `https://goalcup-ingest-staging.b-9a5.workers.dev`). Every path
  below is under it. `GET /v1/health` answers `200 {"status":"ok"}` and touches nothing.
- Every call except the three under "Sign in" carries `Authorization: Bearer gcr_<64 hex>`, the run
  credential. A missing, unknown or expired credential is `401`. A credential sees one run, the
  one it opened; any other run id is `404`, so a credential cannot probe for other runs.
- Request and response bodies are JSON (`Content-Type: application/json`), except the chunk body.
- Times are RFC 3339 UTC to the second, `YYYY-MM-DDTHH:MM:SSZ`.
- Identifiers: `run_id` is `r` and 16 lowercase hex digits; `entry_id` is `e` and 16 lowercase
  hex digits; hashes are 64 lowercase hex digits.
- A path the server does not have is `404 not_found`.
- An error is a status code and `{"error": "<code>", "message": "<one sentence>"}`. goalcup
  shows the message. Codes:

  | Status | `error` | Meaning | goalcup does |
  |---|---|---|---|
  | 400 | `malformed` | A field or header is missing or has the wrong shape. | backs off and retries |
  | 401 | `unauthorized` | No credential, or not one the server knows, or it has expired. | backs off and retries; `goalcup upload` gives up and says so |
  | 403 | `forbidden` | The member may not start a run now (terms not accepted, outside the start window, a goalcup older than the server accepts, a `dev` competition run, a repository another member's run uses, test mode for a non-operator), or a device request was denied. | stops the start, reports |
  | 404 | `not_found` | No such run for this credential, not this run's entry, or no such path. | backs off and retries; `goalcup upload` gives up and says so |
  | 409 | `conflict` | The request does not continue where the server is (offset, `Goalcup-Prev`, `Goalcup-First-Seq`), a final heartbeat names another end than the one committed, the credential already opened a run, or `previous_entry_id` is not the run's latest entry. | chunk: reads the position again at once and goes on from there |
  | 410 | `closed` | The run closed 24 hours after it ended; the entry is sealed; the chunk goes past what the final heartbeat committed; or the run is over and cannot be continued. A device request has expired or was already picked up. | backs off and retries; `goalcup upload` gives up and says so; a refused `continue` marks the run over |
  | 413 | `too_large` | A body over its cap. | backs off and retries (a goalcup bug) |
  | 422 | `digest_mismatch` | The body's SHA-256 is not `Goalcup-Sha256`. Nothing was stored. | hashes the range again, then retries |
  | 429 | `rate_limited` | Too many calls. `Retry-After` says when. | backs off and retries |
  | 503 | `maintenance` | The server asks every client to wait (the maintenance switch). `Retry-After` says when. | backs off and retries |
  | 503 | `awaiting_final` | The run is over (`ends_at`), and the entry's final heartbeat has not arrived. | sends the final heartbeat, then retries at once |

  During a run, no error stops the run or ends its upload (D64): only a heartbeat answer with
  `stop: true`, or goalcup's own count of the time left, stops it. After 3 refusals in a row the
  participant sees one line that recording continues and nothing is lost. Any other 5xx, and any
  network failure, is retried too. The backoff is the client's (PRD-06 section 5): it starts at
  2 s, doubles to at most 120 s, is multiplied by a random 0.5 to 1.0, never waits less than a
  `Retry-After` the server sent (taken up to 120 s), and starts again from 2 s whenever the
  server's offset moves forward. Recording never waits on any of this. Only `goalcup upload`, run by
  hand, gives up, on `401`, `404` and `410`.

## Sign in and the run credential

A participant has no password and pastes no token. goalcup asks for a code, the participant
approves it on the website in a browser, and goalcup picks up a run credential once. Only the
SHA-256 of the device code, the user code and the credential is stored on the server. The device
code is only ever in a request body, never in a URL or a log.

### `POST /v1/device`: start a request (no authentication)

```json
{"purpose": "login" | "run", "mode": "competition" | "practice" | "test",
 "connector_id": "codex-cli", "proxy_version": "<goalcup version>"}
```

All four fields are required. `proxy_version` is at most 64 characters. A request from a goalcup
older than the server's minimum is `403` in every mode (see "The version floor" under the open
call). `200`:

```json
{"device_code": "<64 hex>", "user_code": "BCDF-GHJK",
 "verification_uri": "<the website's approval page>",
 "verification_uri_complete": "<verification_uri>?code=BCDF-GHJK",
 "interval": 5, "expires_in": 900}
```

`user_code` is 8 letters from `BCDFGHJKLMNPQRSTVWXZ`, shown as `XXXX-XXXX`; the website accepts it in
any case, with or without the dash. `interval` is the seconds between polls, `expires_in` the
seconds a request lives, counted from its creation, whatever its state. goalcup shows the
participant `verification_uri_complete` and opens it when it can.

Both device calls are rate-limited. Starts count per client address, an IPv6 client by its /64
(production: 120 a minute). Polls count per device code (production: 30 a minute), so participants
behind one venue or campus NAT never share a poll limit; a per-address cap of 1200 polls a minute
only stops abuse. Over a limit is `429 rate_limited` with `Retry-After` set to the seconds left in
the minute (1-60). One client's normal use (one start, a poll every 5 seconds) stays far below it.

**Test runs (`"mode": "test"`).** An organizer's own test run. Only accounts on the server's
allowlist of test operators can approve one or open one (anyone else: the website refuses the
approval; an open is `403`). A test run belongs to the server's test event (production `test`,
R2 prefix `traces/test/`, never locked; staging `test-staging`), lasts that event's run hours
(168 by default), and its emails go to the operator with `[TEST]` before the subject. A test run
replaces nothing and is never replaced, so one operator may have several test runs open at once.
Only a competition run replaces anything, and only the account's open competition run. Test runs
are left out of every live number and research report, and
`server/worker/scripts/delete-test-runs.sh` (the owner's tool) deletes all of them (rows and R2
objects) in one go.

### `POST /v1/device/token`: poll a request (no authentication)

`{"device_code": "<64 hex>"}`. The answer is one of:

| Status | Body | Meaning |
|---|---|---|
| `202` | `{"status": "pending"}` | Not decided yet. Ask again after `interval` seconds. |
| `200` | `{"status": "approved", "purpose": "run", "credential": "gcr_<64 hex>", "expires_at": "..."}` | Approved for a run. The credential is minted now, returned this once, and stored hashed. |
| `200` | `{"status": "approved", "purpose": "login", "account": {"handle": "..."}}` | Approved for a login. No credential exists. |
| `403` | `forbidden` | The member denied it. Every poll answers `403` until the request expires. |
| `410` | `closed` | The request expired, never existed, or was already picked up. |

An approved request answers `200` to the first poll only; every later poll is `410`.

### The run credential

A credential stands for one member and one `mode`, the mode of the request that made it. It is
unbound until `POST /v1/runs` opens a run with it; that call binds it to the run. From then on
it may call only that run's paths, and `POST /v1/runs` with it again is the repeat described
there. goalcup keeps it in a file readable by the participant only, never in the harness's
environment, in a command line or in a URL.

A credential expires at `approved_at + 1 hour` while it is unbound, and at the run's
`deadline_at + 24 hours` once it is bound. After that it is `401`.

### `POST /v1/device/approve`: approve a request (tests and staging only)

`Authorization: Bearer <TEST_APPROVAL_KEY>`, body `{"user_code", "member_id", "approve": true|false}`;
`200 {"status": "approved"|"denied"}`. It stands in for the website's approval page, so a test needs
no browser. An unknown code is `404`, an expired request `410`, an already decided one `409`. The
route exists only when the server was given a `TEST_APPROVAL_KEY` and runs with the local database
or `STAGING=on` (unset: `404`). With a key anywhere else, every call answers `500 misconfigured`.
Production never has one.

## `POST /v1/runs`: open a run

Called by `goalcup start` after its local checks and before anything is created on disk.

```json
{"connector_id": "codex-cli", "connector_sha256": "<64 hex>",
 "workspace": "/abs/path/run-.../workspace", "session_id": "<uuid>",
 "harness_version": "0.153.0", "proxy_version": "<goalcup version>",
 "repo_url": "https://github.com/<owner>/<repo>",
 "machine": {"os": "macOS", "cpus": 8, "ac_power": true}}
```

`connector_sha256` is the SHA-256 of the connector file the client ran (64 lowercase hex).
`proxy_version` is the goalcup version, at most 64 characters, and the only version field the server
reads.

`201`: `{"run_id", "entry_id", "official_start", "deadline_at", "seconds_left"}`. The server mints
both ids and stamps `official_start` from its own clock. `entry_id` is the run's first entry,
segment 0.

`seconds_left` is the time left, on the server's clock: whole seconds from the server's now to
`deadline_at`, rounded up, and `0` once `deadline_at` has come (never negative). The open, the
continue and every heartbeat answer carry it. **The server is the clock:** goalcup counts the
run down from the last `seconds_left` it was given. It never reads the machine's own date to decide
when to stop the agent (see "When a run ends"). It counts to `deadline_at`, not `ends_at`: a run that
a later start replaced learns it from `stop`.

`deadline_at` = `official_start` + the run length: the event's run hours for a competition run,
`PRACTICE_RUN_MINUTES` (10) for a practice run. The credential's `mode` decides which: a `practice`
credential opens a practice run, in the practice event.

`dev` is an optional boolean on the open and the continue call (missing is `false`; anything else,
`null` included, is `400`). A `dev` run never enters the competition: `403` when the run's mode is
`competition` ("A --dev run never enters the competition. Start without --dev."). In practice and
test mode `dev` is open to every member (those runs count for nothing).

**The version floor.** The Worker's `MIN_COMPETITION_VERSION` (the name stays, but it applies to
every mode) refuses a device request and an open, in competition, practice and test mode alike,
from a goalcup older than it, or of a version with no `x.y.z` in it, with `403`:

```
This goalcup (<sent>) is older than this server accepts (<min>). Update it: <install line>
```

`<sent>` is the `proxy_version` as sent, `<min>` the server's minimum, `<install line>` the one-line
installer. `continue` is not checked (D70), so a run that opened can always go on. A retried open of
a run that exists is answered before the check. An empty minimum checks nothing. A minimum with no
`x.y.z` in it is malformed (parsing is lenient: the first `x.y.z` counts) and is
`500 misconfigured` for those two calls only. `wrangler.toml` sets `1.0.0` for staging and for
production; production carries it from the owner's 1.0 deploy.

`machine` is optional: facts about the machine, a flat JSON object whose values are strings,
numbers, booleans or `null`, at most 4 KiB serialized (else `400`; `null` means absent). The server
keeps it on the run and never shows it. It also keeps the connection's IP address and country (from
Cloudflare) on the run: the open call's pair, which stays, and the latest pair, which each `continue`
and each heartbeat moves (a call without one keeps the last). They are never in an answer, a key or
a log.

`repo_url` is `https://github.com/<owner>/<repo>` exactly: the owner is ASCII letters, digits and
`-`; the repository is ASCII letters, digits, `-`, `_` and `.`, and is never `.` or `..`. Anything
else is `400`.

**The repository must be empty when the run opens.** With the GitHub check on (`GITHUB_CHECK=on`,
which production and staging set), the open call asks GitHub twice, after the version and `dev`
checks and before anything is created, with the same headers (the Worker's `GITHUB_TOKEN`, if set,
only raises the rate limit). First `GET /repos/<owner>/<repo>`: the repository must exist and be
public (`400` if not). Then `GET /repos/<owner>/<repo>/commits?per_page=1`: `409` is an empty
repository and the open goes on; `200` means the repository already has a commit, and the open is
`400 malformed` ("The repository must be empty when the run opens. Create a new one with no README,
.gitignore or license, and start again."). Any other answer, or no answer, is `502 upstream_error`,
which goalcup retries, so a GitHub outage never refuses a player; nothing is created in any of these
cases. The check applies to competition and test runs, and to the first open only: a retried open
of a run that exists is answered before GitHub is asked, and `continue` never asks. A practice run
skips both GitHub checks, because it judges nothing.

**One competition repository, one member.** A competition open is `403` ("Another member's run in
this event already uses this repository. Create a new empty public repository and start again.")
when its `repo_url` is already the repository of another member's competition run in the same event,
in any state. URLs compare in lower case with a trailing `.git` removed (a trailing `/` is already
`400`). The same member may name it again, and a retried open of a run that exists is answered as
before. Practice and test runs are not checked. An open the sign-up gate refuses gets the sign-up
message first. The database decides it (website migration `20261027100000`, on top of the sign-up
gate `20261021120200`).

**One main run per account.** A competition start by the account replaces its other open competition
run: that run gets `replaced_at` = this run's `official_start`, in the same step, and ends then
(`ends_at`, below). A replaced run keeps its credential and goes on taking its wrap-up (below). A
practice run or a test run replaces nothing and is never replaced; an account may have any number of
practice runs, and an operator any number of test runs.

**The start window.** A competition run can be started only inside the event's start window (its
edges are inside). Before it opens the answer is `403` ("The start window has not opened yet.");
after it closes, `403` with the message "after the start window you can only `goalcup continue` your
run". Practice has no window.

A credential opens one run. A repeat of the same `session_id` (the one the open call sent, whatever
`continue` has added since) on a credential that already opened a run returns that run's answer
with `200`, whatever the run's state, so a reply lost in transit is not a second run. That includes
a retry that raced the original call. Another `session_id` on it is `409`: this credential already
opened a run, and the participant runs `goalcup continue`. The credential is bound to the new run in
the same atomic step that creates the run, so a credential that cannot bind (it is unknown, or was
bound to another run in the meantime) is the same `409` and creates nothing.

## `GET /v1/runs/{run_id}`: where the server is

`GET /v1/runs/{run_id}` answers the run's current entry (the highest segment);
`GET /v1/runs/{run_id}?entry_id=e...` answers that entry (`404` if it is not this run's). `200`:

```json
{"run_id": "r...", "entry_id": "e...", "segment": 0, "status": "open",
 "official_start": "...", "deadline_at": "...", "ends_at": "...",
 "offset": 123456, "next_seq": 812, "head": "<64 hex>", "sealed": false,
 "stop": false, "final": {"seq": 811, "head": "<64 hex>"}}
```

`final` is the entry's committed end (below), or `null` until the final heartbeat commits it.

`ends_at` is when the run is over: `deadline_at`, or `replaced_at` if a later start replaced the run
first. `stop` is `true` once the server's clock is at or past `ends_at`.

`offset` is the raw GCR1 stream offset the server holds the entry up to (the sum of the
`Goalcup-Length` of its stored chunks); `next_seq` is the LINK sequence number the next chunk starts
at (0 before the first chunk); `head` is the link hash at the end of the last stored chunk (64 zero
hex digits, the genesis, before any). `sealed` is `true` once this entry is sealed (see "When a run
ends"). `status` is the run's: `open`, `sealed` (the run is over and its current entry is sealed) or
`incomplete` (the run closed without it).

## `PUT /v1/runs/{run_id}/chunks/{offset}`: store bytes of the record

The body is a byte range of one compressed part file of the entry (PRD-01 section 12): from one
index line, or the file's start, to a later index line, never across two part files. It continues
its part file's zstd frame, so a chunk is **not** decodable on its own; the server never decodes
it. At most 32 MiB (`Content-Length` at most 33554432). `Content-Type: application/zstd`. Headers:

| Header | Value |
|---|---|
| `Goalcup-Entry-Id` | the entry the bytes belong to (any entry of the run: an older segment keeps uploading its wrap-up) |
| `Goalcup-Length` | the raw GCR1 bytes the chunk covers (always above 0) |
| `Goalcup-Sha256` | SHA-256 of the request body (the compressed bytes) |
| `Goalcup-First-Seq` | the first LINK sequence number in the chunk (the server's `next_seq`) |
| `Goalcup-Last-Seq` | the sequence number of the LINK at the chunk's end |
| `Goalcup-Prev` | the link hash at the chunk's start (the server's `head`; 64 zero hex digits for an entry's first chunk) |
| `Goalcup-Head` | the link hash of the LINK at the chunk's end |

The path `offset` is the raw GCR1 stream offset of the chunk's first byte; an entry's first chunk is
at offset 0 with `Goalcup-First-Seq` 0.

Before the body is read the server refuses a bad credential, an entry that is not this run's
(`404`), a run that is closed, an entry that is sealed, a `Content-Length` over the cap and a missing
or malformed header. It then requires `offset` = its `offset`, `Goalcup-Prev` = its `head` and
`Goalcup-First-Seq` = its `next_seq` (`409` otherwise). Then it streams the body into storage while
hashing it, and keeps it only if the hash is `Goalcup-Sha256` (`422` otherwise, nothing kept).
Storage is create-only, at the R2 key `traces/<event>/<run>/<entry>/<offset, 16 digits>-<sha256>.zst`.
There is no per-run size limit.

`200`:

```json
{"offset": 123456, "next_seq": 812, "head": "<64 hex>", "received_at": "...", "sealed": false}
```

`offset` is the new offset, `next_seq` is `Goalcup-Last-Seq` + 1, `head` is `Goalcup-Head`,
`received_at` the server's clock when it stored the chunk, and `sealed` whether the entry is sealed
after this chunk. **The chunk that ends at a committed end seals the entry:** when the entry's final
heartbeat has committed `(seq, head)` and this chunk's `Goalcup-Last-Seq` and `Goalcup-Head` are
that `seq` and `head`, the server seals the entry in the same call and answers `sealed: true`.

**A retry is safe.** A chunk at an `offset` the server already holds, with the same
`Goalcup-Length` and `Goalcup-Head`, is answered `200` from the stored row (with `sealed` from the
entry as it stands), without reading the body, before anything about the run's state is looked at.
The same `offset` with a different length or head is `409`.

The server checks only these headers. Whether the bytes really are the record they claim to be is
checked later, on the fetched copy (`scripts/fetch-run.sh`, then `goalcup-verify check --server`).

## `POST /v1/runs/{run_id}/heartbeat`: still running, and where the record is

The client sends one every 5 minutes while the entry records, at once when its recording state
changes, and the final one once the entry is sealed on disk (PRD-06 section 6):

```json
{"entry_id": "e0123456789abcdef", "seq": 3601, "head": "<64 hex>", "offset_local": 726000000,
 "final": false, "repo_head": null, "recording": "gaps", "model_requests": 905,
 "counts": {"v": 1, "boot": 2, "mode": "recording", "off_ms": 2300, "off_spans": 1,
            "off_since": null, "gaps": 3, "lost_bytes": 1500000, "lost_unknown": 0,
            "http_post": 243, "ws_up_msgs": 662, "conns": 120, "tunnels": 118,
            "wire_bytes": 674000000, "native_files": 43, "native_bytes": 48200000,
            "waiting_bytes": 1048576}}
```

| Field | Meaning |
|---|---|
| `entry_id` | the entry it speaks for (any entry of the run) |
| `seq`, `head`, `offset_local` | the **durable point**: the last LINK on the client's disk, its link hash, and the raw offset just past it. The final heartbeat names the seal LINK |
| `final` | `true` for the final heartbeat (below) |
| `repo_head` | the 40-hex commit the workspace repository had checked out when the entry ended; final heartbeat only, else `null` |
| `recording` | the client's recording state (table below) |
| `model_requests` | model requests written (`http_post + ws_up_msgs`, PRD-01 section 9), entry totals over boots, as of the durable point, at most 2^53 - 1. A request count, not a call count |
| `counts` | the written counts and off time at the durable point (below) |

The server stamps its own receive time on each heartbeat, which commits goalcup to that position
before the bytes arrive. Every heartbeat row is kept, so the off spans can be read in time order.

**`recording`.** The client decides it from `counts.mode` and the counts. PRD-06 section 6 owns this
rule; if this table ever differs, PRD-06 wins.

| `recording` | Means | The client sends it when |
|---|---|---|
| `ok` | recording, nothing lost | `mode` is `recording` (or `sealed`), and `gaps`, `lost_bytes`, `lost_unknown`, `off_ms` and `streams_in_gap` are all 0 |
| `gaps` | recording now (or sealed), something was lost | any of those is above 0, or `mode` is `losing` or `writer_slow`. A recovered entry is `gaps` |
| `failed` | **recording is off now** | `mode` is null, or the writer has stalled past 60 s while records wait; and only then |

`streams_in_gap` is the recorder's live count of streams in a gap (PRD-02 section 3), which the
client reads for this decision; it is not a `counts` key, so the server cannot recompute the state
from the stored counts alone. `failed` is a state, not a latch: the next heartbeat after a good boot says `gaps`. 1.0 never sends
`unknown` (no heartbeat goes before the first durable point); the server still accepts it.

**`counts`.** A flat JSON object whose values are whole numbers, strings of at most 64 bytes,
booleans or `null`, at most 2 KiB serialized. The Worker fixes no key list and interprets nothing
in it. The client's keys (PRD-06 section 6 names them; this list copies it):

- every whole-number counter of the COUNT event (PRD-01 section 9) as entry totals at the durable
  LINK: `up_ms`, `http_req`, `http_post`, `ws_up_msgs`, `ws_down_msgs`, `req_whole`, `req_holes`,
  `conns`, `tunnels`, `tunnel_up`, `tunnel_down`, `wire_bytes`, `native_files`, `native_bytes`,
  `snapshots`, `gaps`, `lost_bytes`, `lost_unknown`, `off_ms`, `mask_failclosed_bytes`, and
  `held_peak` (the largest over boots). The maps `hosts` and `tunnel_hosts` and the gauges
  `open_streams` and `held_bytes` are left out, so the object stays flat;
- `v` (1), `boot` (the boot number at the durable point);
- `mode`: `"recording"`, `"losing"`, `"writer_slow"`, `null` (null mode) or `"sealed"` (the final
  heartbeat sends `"sealed"`);
- `off_ms` and `off_spans`: the off time and the number of off spans, including the open span while
  in null mode; `off_since`: the start of the open off span (RFC 3339), else `null`;
- `waiting_bytes`: compressed bytes on the client's disk not yet confirmed by the server.

**Optional facts, and a bad `counts`.** The client always sends `recording`, `model_requests` and
`counts`, also from `goalcup upload` (there they are the seal's totals). The server treats all three
as optional: a missing one, or `null`, is stored as `null`. A bad `recording` or `model_requests` is
`400 malformed`, as are the heartbeat's own fields. A `counts` that is not a flat object within the
bounds above is **dropped, never refused**: the heartbeat is kept and answered `200`, its row stores
`{"counts_dropped": <byte length of the serialized counts>}`, and the Worker logs
`counts-dropped <run_id>` without the value. A refused heartbeat would be a heartbeat lost (D64).

**Storage.** Counts land in the database column `heartbeats.vitals` (and as the run's latest in
`runs.last_vitals`) as `{"counts": ...}`; readers use `vitals -> 'counts'`. The column keeps the name
`vitals` because a rename is a migration.

`model_requests` is kept per entry, because a continued process counts from zero: the first value an
entry reports, and any value above the highest that entry has reported, moves the run's last model
request to this heartbeat; a lower or equal one changes nothing. The first `gaps` or `failed` flags
the run, once (`runs.flag_reason` keeps the first; read the heartbeat rows for the latest state).
Every heartbeat also sets the run's latest heartbeat time (whichever entry sent it), and the server
keeps the connection's IP address and country on the heartbeat and as the run's latest pair, as for
the open call. The notices that follow are under "What the server does on its own".

`200`:

```json
{"stop": false, "deadline_at": "...", "seconds_left": 561540, "reason": null, "sealed": false}
```

`seconds_left` is the time left (see the open call). `stop` is `true` once the server's clock is at
or past `ends_at`: **the run is over**. goalcup then stops the agent at once, seals, and sends the
final heartbeat. `reason` says why: `"replaced"` when a later start replaced the run, `"deadline"`
when `deadline_at` came, `null` while `stop` is `false`. `sealed` is whether the named entry is
sealed after this call.

**The final heartbeat commits the end of the entry.** Its `(seq, head)` is the seal LINK, the last
link the server will ever accept for that entry. If the server already holds the chain up to it, the
server seals the entry in the same call and answers `sealed: true`. A repeat with the same
`(seq, head)` is `200` and adds no row; a different one, or one that does not match the chain the
server already holds, is `409`. Any entry may send its own final heartbeat at any time: before
`ends_at` (the agent ended early) or after it, until the run is closed.

## When a run ends

A run ends at `ends_at` = `min(deadline_at, replaced_at)`: at its deadline, or when the account's
later competition start replaced it. At `ends_at` the run is over, in whatever state the agent is
in. There is no grace period and no late window.

- **Only two things stop a run:** a heartbeat answer with `stop: true`, or goalcup's own count
  below. No error answer does (D64).
- **The server says stop.** Every heartbeat answered at or after `ends_at` carries `stop: true` and
  the `reason`. goalcup stops recording at once. Without a network it stops on its own count: the
  `seconds_left` of the last answer it had (the open, the continue or a heartbeat), counted from
  just before that call on a clock that keeps running while the machine sleeps. Each answer sets the
  count again, and the server's answer wins. The machine's date is never read for it, so changing
  the date does not move the stop. (A replaced run learns of it from its next heartbeat.)
- **The seal.** There is no seal call. An entry is sealed by the server once two facts hold: its
  final heartbeat has committed `(seq, head)`, and the server holds the chain up to that link.
  Whichever arrives second seals it: the final heartbeat, or the chunk that ends at the committed
  end. The client sends the final heartbeat (naming the seal LINK, `counts.mode` `"sealed"`),
  uploads the rest, and repeats the final heartbeat until a heartbeat, chunk or position answer says
  `sealed: true`. A repeat is free: it adds no row. The repeat covers the one race neither call can
  see (the final heartbeat commits while the last chunk is in flight).
- **The wrap-up upload has no time limit of its own, but it can only complete each entry's chain up
  to its committed end.** After `ends_at`, a chunk is accepted only once the entry's final heartbeat
  has committed `(seq, head)`, and only if its `Goalcup-Last-Seq` is at most that `seq` (and the
  chunk ending at that `seq` has that `head`); anything else is `410 closed`. Before the final
  heartbeat arrives, a chunk after `ends_at` is `503 awaiting_final` with `Retry-After`, so a client
  still uploading sends its final heartbeat and asks again. Every entry of the run, not only the
  current one, is accepted, so an older segment finishes its wrap-up.
- **Before `ends_at`** an entry that has a committed end accepts nothing past it either.
- **The run's status.** The run is `sealed` once it is over and its current entry is sealed. An
  entry sealed before the end leaves the run `open`, so it can be continued.
- **Closed.** 24 hours after `ends_at` the server closes the run (the cron, or the first call that
  names it): `sealed` if its current entry is sealed, else `incomplete` (the last piece is missing;
  the run is judged on what arrived). It accepts nothing more (`410`), including a repeat of the
  final heartbeat; only a chunk it already stored is answered again (`200`). A credential lives until
  `deadline_at` plus 24 hours, so a run that ended at its deadline closes when its credential
  expires, and a late upload meets `401`, not `410`.
- The server judges no record. Which records count is decided later, on the fetched copy.

## `POST /v1/runs/{run_id}/continue`: one more segment of the same run

Called by `goalcup continue` on the machine that started the run, with that run's credential. A run
is one or more entries (segments): segment 0 is made by `POST /v1/runs`, each `continue` adds the
next. Every entry has its own chain and its own seal, and the chunk, heartbeat and position calls
address the entry they name.

```json
{"previous_entry_id": "e...", "connector_id": "...", "connector_sha256": "<64 hex>",
 "workspace": "...", "session_id": "...", "harness_version": "...", "proxy_version": "..."}
```

The fields mean what they mean in the open call. The version floor does not apply here (D70).

`201 {"entry_id", "segment", "official_start", "deadline_at", "seconds_left"}`: the new entry, and
the run's own `official_start` and `deadline_at` (a segment is not a new run, and the deadline does
not move). `seconds_left` is what is left of the run now, on the server's clock (see the open call);
the continued segment counts down from it. The previous entry is not sealed or closed by this; it
may still upload its wrap-up.

- `previous_entry_id` must be the run's latest entry, else `409`.
- At or after the run's end (`ends_at`) the answer is `410`, as it is for a closed run.
- The same call again (the latest entry was made by a `continue` with this `session_id` after this
  `previous_entry_id`) is `200` with the same answer, so a lost reply is not a second segment.

## What the server does on its own

A cron runs every 5 minutes. It does four things, in this order, and a step that fails is logged
and does not stop the next: it closes runs that are over (so a run's status is final without any
call naming it), it records the GitHub commit at each deadline, it queues the run emails in an
outbox, and it sends them. A client sees none of this. Each outbox row is sent once through Resend
(`server/worker/src/mail.ts`); the database writes the email, picks the member's address and checks
suppression. A staging Worker is never given an address by the database and sends every email to
its test address. (While the maintenance switch is on the cron does nothing but delete its health
marker, since no client can send a heartbeat.)

**The judged commit.** For a competition run that reached its deadline (not a replaced run, not
practice), the judged version is the last push to the repository's default branch that GitHub
itself timestamped at or before the run's `ends_at` (owner, 21 Oct 2026). Pushes after that do not
count; commit dates are ignored. From the first tick at or after `ends_at` + 5 minutes (so a push
GitHub received just before the deadline has reached its activity list), at most 50 runs per tick
(oldest first; when more are due, each tick starts where the tick before stopped, so runs GitHub
keeps failing on cannot hold up the others), 8 seconds per call, the server reads:

1. `GET /repos/{owner}/{repo}`: `default_branch`.
2. `GET /repos/{owner}/{repo}/activity?ref=refs/heads/<branch>&per_page=100`, newest first, following
   the `Link` header's `rel="next"` back to the run's `official_start` (at most 5 pages). Every
   activity type that moves the branch counts as a push (`push`, `force_push`, `pr_merge`,
   `merge_queue_merge`, `branch_creation`); each item's `after` is the sha and `timestamp` the time
   GitHub received it.

It keeps the `after` of the newest activity with `timestamp <= ends_at` (`final_commit`), with that
timestamp (`final_pushed_at`), the branch (`final_branch`), the time it was decided
(`final_commit_at`) and two flags: `final_late_push` (GitHub listed a push after `ends_at` when asked)
and `final_force_push` (a `force_push` between `official_start` and `ends_at`). If no activity at or
before `ends_at` is listed but later ones are, and the listing reached back to `official_start`, the
branch stood at the oldest later push's `before`. If the 5-page limit stopped the listing first, the
update that set the branch at the deadline was not read: the server asks again each tick for an hour
after `ends_at`, then uses the fallback.

**The fallback, and why its rows need an organizer.** If the branch has no activity at all, the
activity API keeps failing for an hour after `ends_at`, or the listing stays incomplete as above,
the server keeps the newest commit on the branch whose commit date is at or before `ends_at`
(`GET /repos/{owner}/{repo}/commits?sha=<branch>&until=<ends_at>&per_page=1`), else the head GitHub
has when asked (`GET /repos/{owner}/{repo}/commits/HEAD`), and sets `final_head_fallback`. Neither is
the rule above: commit dates are the pusher's to set, and the head may be a push after the deadline.
So every row with `final_head_fallback` set needs manual review by an organizer before it is judged.

A `404` for the repository is retried each tick for an hour after `ends_at` (a repository briefly
private must not lose the submission), then kept as "no commit" (`none`), as is an empty repository
(`commits/HEAD` `409`). Any other error status (a `403` rate limit, `451`, a `5xx` that does not go
away) is retried for the same hour, then kept as `none` with `final_head_fallback` set, so it too goes
to an organizer. Every GitHub answer that is not a `200` is a Worker log line `github-status <status>
<path> ratelimit-remaining=<n>`, and a GitHub that cannot be reached at all is `github-unreachable`;
an unreachable GitHub keeps nothing and the next tick asks again. Runs that ended more than 48 hours
ago and are closed are not asked. The final heartbeat's `repo_head` is goalcup's own account of the
clone.

**The emails.** Each is queued at most once per run, kind and key. None is queued for a practice
run, and none after the run's end except `ended`. A test run mails its operator, with `[TEST]` in
the subject.

| Kind | Queued when | Once per |
|---|---|---|
| `started` | a competition run is open (before its end) | run |
| `silence` | the run's last heartbeat (or its start, if none yet) is more than 20 minutes old, and the server itself has been well for more than 20 minutes (see "The server's health") | last heartbeat time |
| `idle_3h` | the run is beating (last heartbeat within 20 minutes), has reported `model_requests` at least once, and its last model request is more than 3 hours old | last model request |
| `idle_6h` | the same, 6 hours | last model request |
| `recording` | a heartbeat has flagged the run (`gaps` or `failed`) | run |
| `ended` | the run reached its deadline (a replaced run is not the main run and has none) and the judged commit was recorded, or an hour has passed without it. Its payload says whether the run is sealed on the server (`uploaded`) and when it closes (`closes_at`, the end + 24 hours); if it is not sealed, the email says to run `goalcup upload` before then | run |
| `disk_full` | never from a 1.0 client: no heartbeat starts a disk pause any more (the cron still holds the rule) | start of that pause |

The last model request is unset until a heartbeat first reports `model_requests`, so a client that
never reports gets no idle email; once set, it moves whenever any entry's count first appears or
grows (a continue's new process reporting 2 after the old one reached 50 counts). Since 1.0 the
count is model requests written, so a run whose recording is off gets an idle email. A run that
comes back and goes quiet again gets a new `silence`; a new model request starts a new idle period.
The `ended` payload holds the end time, the commit (or none), the repository, `uploaded` and
`closes_at`.

**The server's health.** A silence the server's own trouble may explain is never mailed. The cron
keeps a small marker in the bucket (`cron/health.json`: the last tick, and since when the server
has been well). A tick is well when the tick before it ran at most 11 minutes earlier (so at most
one 5-minute tick was missed) and some run's heartbeat, of any mode, reached the server in the last
10 minutes. Anything else starts the well stretch again at that tick: two ticks in a row that were
missed or that the database failed (the marker is written only after the database answered), the
maintenance switch (it deletes the marker), no heartbeat from any run, or a marker that cannot be
read or written. A `silence` is queued only once both the run's last heartbeat and the start of the
well stretch are more than 20 minutes old. So during our trouble no one is mailed, and after it
every run has 20 minutes to come back (goalcup's retries are at most 2 minutes apart). The key is
still the last heartbeat, so a silence that outlasts our trouble is mailed once, after it.

**Known gap: a lone run never gets a `silence` email.** The well stretch needs some run's heartbeat
in the last 10 minutes. When only one run is open and it goes quiet, no heartbeat reaches the server,
every tick starts the stretch again, and its silence is never mailed, however long it lasts. From
the server a lone quiet run looks the same as a server nobody can reach. This matters most at the
start of the event and for a test run on its own. The owner accepted this (21 Oct 2026): a lone
run's record still uploads normally within the run's length plus the 24-hour close window.

The words of each email are in the database (`goal_cup_tracer.notice_text`, owned by the website);
`docs/research/NOTIFICATIONS.md` lists every message a participant gets, by email and in the
terminal, with its trigger and the one thing to do. `silence` never says the run stopped (the
server cannot know) and never advises `goalcup start` (a new start replaces the run).

**What is stored, and only on the server.** The outbox row holds the run, the member, the kind, the
key and a small payload (times, hours, the commit, the repository). The run holds the machine facts
from the open call, the IP address and country of the open call, the latest pair (moved by each
`continue` and heartbeat) and the latest heartbeat's counts. None of it is in any answer, key or
log, and none of it is public.
