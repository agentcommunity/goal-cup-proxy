# Upload API, version 1

The exact wire contract between the tracer (`goalcup`) and the upload server. It implements
[SERVER-DESIGN.md](SERVER-DESIGN.md) section 4 for the calls built so far; where it differs from
that section, this file says so and why. Three things speak it and must agree byte for byte: the
tracer (`crates/goalcup-cli/src/upload/`), the local stand-in server the tests run
(`crates/goalcup-cli/tests/common/upload_server/`), and the Worker
(`server/worker/`).

## Common rules

- Base URL: what `goalcup login --server` stored. Every path below is under it.
- Every call carries `Authorization: Bearer <token>`. A missing or unknown token is `401`; a run
  that belongs to another member is `404`, so a token cannot probe for other members' runs.
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
  | 401 | `unauthorized` | No token, or not a token the server knows. | stops: `goalcup login` again |
  | 403 | `forbidden` | The member may not start a run now (terms not accepted, outside the start window). | stops, reports |
  | 404 | `not_found` | No such run for this token, or not this run's entry. | stops, reports |
  | 409 | `conflict` | The request does not continue where the server is (offset, `prev`, seq), or the account already has an open run. | chunk: re-reads the position (`GET`), then continues |
  | 410 | `closed` | The run is sealed or abandoned, or past its grace period. | stops, reports |
  | 413 | `too_large` | A body over its cap. | stops, reports (a tracer bug) |
  | 422 | `digest_mismatch` | The body's SHA-256 is not `Goalcup-Sha256`. Nothing was stored. | retries with backoff |
  | 429 | `rate_limited` | Too many calls. `Retry-After` says when. | retries with backoff |
  | 503 | `maintenance` | The server asks every tracer to wait. `Retry-After` says when. | retries with backoff |

  Any other 5xx, and any network failure, is retried with backoff. Backoff starts at 60 s,
  doubles to at most 15 min, with up to 25 % random jitter, and never waits less than a
  `Retry-After` (seconds) the server sent. Recording never waits on any of this.

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

A repeat of the same `session_id` by the same member while that run is open and has no receipt
returns the same answer with `200`, so a reply lost in transit is not a second run. Another open
run on the account is `409`.

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
 "offset": 123456, "next_seq": 812, "head": "<hex>", "last_ack": <AckCheckpoint or null>}
```

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

## Not built yet

Heartbeats, outage windows and `continue` (section 4) come in later slices.
