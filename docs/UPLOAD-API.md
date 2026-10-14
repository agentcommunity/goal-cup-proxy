# Upload API, version 1

The exact wire contract between the tracer (`goalcup`) and the upload server. It implements
[SERVER-DESIGN.md](SERVER-DESIGN.md) section 4 for the calls built so far; where it differs from
that section, this file says so and why. Three things speak it and must agree byte for byte: the
tracer (`crates/goalcup-cli/src/upload/`), the local stand-in server the tests run
(`crates/goalcup-cli/tests/common/upload_server/`), and the Worker
(`server/worker/`).

## Common rules

- Base URL: what `goalcup login --server` stored. Every path below is under it.
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
  | 409 | `conflict` | The request does not continue where the server is (offset, `prev`, seq), the account already has an open run, or the credential already opened a run. | chunk: re-reads the position (`GET`), then continues |
  | 410 | `closed` | The run is sealed or abandoned, or (after `deadline_at`) the request goes past what the final heartbeat committed. A device request has expired or was already picked up. | stops, reports |
  | 413 | `too_large` | A body over its cap. | stops, reports (a tracer bug) |
  | 422 | `digest_mismatch` | The body's SHA-256 is not `Goalcup-Sha256`. Nothing was stored. | retries with backoff |
  | 429 | `rate_limited` | Too many calls. `Retry-After` says when. | retries with backoff |
  | 503 | `maintenance` | The server asks every tracer to wait. `Retry-After` says when. | retries with backoff |
  | 503 | `awaiting_final` | Past `deadline_at`, and the final heartbeat has not arrived. | sends it, then retries |

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
{"purpose": "login" | "run", "mode": "competition" | "practice",
 "connector_id": "codex-cli", "tracer_version": "0.1.0"}
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
 "harness_version": "0.153.0", "tracer_version": "0.1.0",
 "repo_url": "https://github.com/<owner>/<repo>"}
```

`201`: `{"run_id", "entry_id", "official_start", "deadline_at"}`. The server mints both ids,
stamps `official_start` from its own clock, and sets `deadline_at` = `official_start` + the
event's run hours.

`repo_url` is `https://github.com/<owner>/<repo>` exactly: the owner is ASCII letters, digits and
`-`; the repository is ASCII letters, digits, `-`, `_` and `.`, and is never `.` or `..`. Anything
else is `400`.

A credential opens one run. A repeat of the same `session_id` on a credential that already opened a
run returns that run's answer with `200`, whatever the run's state, so a reply lost in transit is
not a second run. Another `session_id` on it is `409`: this credential already opened a run, and
the participant runs `goalcup continue`. A credential that has not opened a run yet is still held
to the account's one open run: another open run on the account is `409`.

**Differs from section 4:** `chain_head_at_create` is not sent here. The chain's first link hashes
the `entry_id`, and the server issues the `entry_id` in this answer, so the head cannot exist yet.
It is sent to the receipt call below.

## `POST /v1/runs/{run_id}/receipt` — the signed start receipt

Called once the daemon has created the entry under the server's `entry_id` and chained its
baseline.

```json
{"entry_id": "e…", "chain_head": "<hex of the baseline link>"}
```

`200`: `{"start_receipt": <StartReceipt>}`, in the verifier's existing format
(`crates/goalcup-protocol/src/receipt.rs`), signed with the server's Ed25519 key. Its claims are
the run's own fields: `entry_id`, `connector_id`, `workspace`, `session_id`,
`started_at` = `official_start`, `deadline_at`, `manifest_sha256`, `chain_head` (from this
request), `connector_dir_sha256`. A repeat with the same `chain_head` returns the stored receipt;
a different `chain_head` once a receipt exists is `409`.

## `POST /v1/runs/{run_id}/abandon` — give back a run that never started

`{"reason": "<text, at most 500 characters>"}`. Accepted only while no chunk has been stored:
`200 {"status": "abandoned"}`, and `200` again for a repeat. A run with stored chunks is `409`; a
sealed run is `410`. The tracer sends a fixed sentence, never a local path. Used when `goalcup start` fails
after the run opened and before the harness launched, so the account is not left with an open run
that holds nothing.

**Not in section 4.** Without it a failed start (a daemon refusal, a full disk) would leave the
one open run per account taken.

## `GET /v1/runs/{run_id}` — where the server is

`200`:

```json
{"run_id": "r…", "entry_id": "e…", "status": "open", "official_start": "…", "deadline_at": "…",
 "offset": 123456, "next_seq": 812, "head": "<hex>", "last_ack": <AckCheckpoint or null>,
 "stop": false, "final": {"seq": 812, "head": "<hex>"} or null}
```

`stop` is `true` once the server's clock is at or past `deadline_at`; `final` is the committed end
(below) or `null`.

`offset` is how many bytes of the entry's `chain.jsonl` the server holds, `next_seq` the sequence
the next chunk starts at (0 before the first chunk: a chain's first record, the baseline, has
`seq` 0), `head` the chain hash of the last stored record (the 64-zero genesis before any), `last_ack` the acknowledgement of the last stored chunk. `status` is `open`,
`sealed` or `abandoned`.

## `PUT /v1/runs/{run_id}/chunks/{offset}` — store bytes of the chain

The body is one zstd frame holding bytes `[offset, offset + length)` of the entry's
`chain.jsonl`: whole lines only, at most 32 MiB compressed (`Content-Length` ≤ 33554432).
`Content-Type: application/zstd`. Headers:

| Header | Value |
|---|---|
| `Goalcup-Entry-Id` | the entry the bytes belong to |
| `Goalcup-Length` | the uncompressed length in bytes |
| `Goalcup-Sha256` | SHA-256 of the request body (the compressed bytes) |
| `Goalcup-First-Seq` | the `seq` of the first record |
| `Goalcup-Last-Seq` | the `seq` of the last record |
| `Goalcup-Prev` | the `link.prev` of the first record |
| `Goalcup-Head` | the `link.hash` of the last record |

Before the body is read the server refuses a bad token, a run that is not open, a
`Content-Length` over the cap and a missing or malformed header. It then requires
`offset` = its `offset`, `Goalcup-Prev` = its `head` and `Goalcup-First-Seq` = its `next_seq`
(`409` otherwise), streams the body into storage while hashing it, and keeps it only if the hash
is `Goalcup-Sha256` (`422` otherwise, nothing kept). Storage is create-only.

`200`: `{"ack": <AckCheckpoint>, "offset": <new offset>, "next_seq": <last_seq + 1>, "head": "<hex>"}`.
The acknowledgement is the verifier's existing format (`crates/goalcup-protocol/src/ack.rs`):
`entry_id`, `seq` = `Goalcup-Last-Seq`, `hash` = `Goalcup-Head`, `acked_at` = the server's clock,
signed with the same key as the receipt.

**A retry is safe.** A chunk at an `offset` the server already holds, with the same
`Goalcup-Length` and `Goalcup-Head`, is answered `200` with the acknowledgement stored for it,
without reading the body. The same `offset` with a different length or head is `409`.

The server checks only these headers. Whether the bytes really are the chain they claim to be is
checked later by the verify job, which runs `goalcup-verify` over the reassembled copy.

## `POST /v1/runs/{run_id}/seal` — the run is over

```json
{"entry_id": "e…", "seal_json": "<seal.json, verbatim>", "baseline_json": "<baseline.json, verbatim>"}
```

Each document at most 16 MiB. Sent after the last chunk. `200 {"status": "sealed"}`; a repeat with
the same documents is `200` again. Different documents once sealed are `409`.

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
tracer sees its own recording) and `"model_requests": <u64>` (model calls seen so far, cumulative).
A heartbeat without them is as before. The server checks their shape and records nothing else
about them yet.

`200`: `{"stop": <bool>, "deadline_at": "…", "reason": null}`. `reason` says why `stop` is true when
the deadline is not the only reason (`"deadline"` or `"replaced"`; a later change fills it, today it
is `null`). `stop` is `true` once the server's clock is at or past `deadline_at`: **the run is over**. The tracer then stops the agent at once, seals, and sends
the final heartbeat.

**The final heartbeat commits the end of the run.** Its `(seq, head)` is the last record the
server will ever accept for the entry. A repeat with the same `(seq, head)` is `200`; a different
one is `409`. A final heartbeat is accepted before `deadline_at` (the agent ended early) or after
it, until the run is abandoned.

## The deadline

At `deadline_at` the run is over, in whatever state the agent is in. There is no grace period and
no late window.

- **The server says stop.** Every heartbeat answered at or after `deadline_at` carries
  `stop: true`. The tracer stops recording at once. Without a network it stops on its own clock:
  `deadline_at − official_start` after the run opened, counted on a clock that keeps running
  while the machine sleeps.
- **The wrap-up upload has no time limit, but it can only complete the chain up to the committed
  end.** After `deadline_at`, a chunk is accepted only once a final heartbeat has committed
  `(seq, head)`, and only if its `Goalcup-Last-Seq` is at most that `seq` (and the chunk ending at
  that `seq` has that `head`); anything else is `410 closed`. Before the final heartbeat arrives,
  a chunk after `deadline_at` is `503 awaiting_final` with `Retry-After`, so a tracer still
  uploading asks again after it has sent its final heartbeat. The seal after `deadline_at` is
  likewise `503 awaiting_final` before the final heartbeat, and `409` until the server holds the
  chain up to the committed end.
- **Before `deadline_at`** a run that has a committed end accepts nothing past it either, and its
  seal only once the server holds the chain up to it.
- **Abandoned.** A run with no seal 24 hours after `deadline_at` is abandoned: it accepts
  nothing more (`410`), and it no longer counts as the account's open run.
- Which records count is the verify job's: a record stamped after `deadline_at` does not, however
  it arrived.

## `POST /v1/runs/{run_id}/continue` — one more segment of the same run

**Not built yet: the Worker's next task implements it.** The contract is fixed here so the tracer
can code against it.

```json
{"previous_entry_id": "e…", "connector_id": "…", "manifest_sha256": "<hex>",
 "connector_dir_sha256": "<hex>", "workspace": "…", "session_id": "…",
 "harness_version": "…", "tracer_version": "…"}
```

`201 {"entry_id", "segment", "official_start", "deadline_at"}`. The same `session_id` again is `200`
with the same answer. A `previous_entry_id` that is not the run's latest entry is `409`. At or after
the run's end (`ends_at = min(deadline_at, replaced_at)`) it is `410`. The receipt, chunks,
heartbeat and seal calls then address the named `entry_id`, and `GET /v1/runs/{run_id}?entry_id=`
gives that entry's position.

## Not built yet

Outage windows, live totals in the heartbeat, and the rest of `continue` (section 4) come in later
slices.
