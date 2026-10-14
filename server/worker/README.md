# Goal Cup upload Worker

The server end of `goalcup`'s trace upload: a Cloudflare Worker (TypeScript, no framework) that
checks a participant's token, streams zstd chunks of `chain.jsonl` into R2, records progress in
Supabase (Postgres), and returns Ed25519-signed receipts and acknowledgements in the formats the
Rust verifier already checks.

- **Wire contract (the authority):** `docs/UPLOAD-API.md`. Paths, headers, status codes, error
  bodies and idempotency rules are implemented exactly as written there.
- **Design:** `docs/SERVER-DESIGN.md`, sections 2 to 7, 11, 14 and 16.

Nothing in this folder has been deployed, and nothing here touches a Cloudflare or Supabase
account. Local runs and tests use `wrangler dev`, which simulates R2 on disk.

## Run it locally

Needs Node 22.18 or newer (the tests run TypeScript directly) and network access for
`npm install` only.

```sh
cd server/worker
npm install
cp .dev.vars.example .dev.vars     # test values only; .dev.vars is git-ignored
npm run dev                        # http://localhost:8787, DB_BACKEND=local, R2 simulated
npm test                           # type-check, then unit and end-to-end tests
```

`npm test` starts its own `wrangler dev` processes (on free ports, with temporary state, and with
wrangler's own config directory pointed at a temporary folder), so it needs no setup and leaves
nothing behind. It takes about 15 seconds.

Try it by hand against `npm run dev` (tokens come from `.dev.vars`):

```sh
curl -s -XPOST localhost:8787/v1/runs -H 'authorization: Bearer dev-token-1' \
  -H 'content-type: application/json' -d '{"connector_id":"codex-cli", ... }'
```

## Layout

| Path | What |
|---|---|
| `src/index.ts` | Routing, maintenance switch, authentication, error mapping |
| `src/runs.ts` | Open, receipt, abandon, position, seal |
| `src/chunks.ts` | The chunk upload (early refusals, retry rule, streaming to R2, atomic commit) |
| `src/sign.ts` | Ed25519 receipts and acks (byte-exact with the Rust formats) |
| `src/db.ts` | The `Db` interface: one method per step |
| `src/db-supabase.ts` | Production: one Postgres function per step, via PostgREST RPC |
| `src/db-local.ts` | `wrangler dev` and tests only: JSON state in the R2 bucket under `state/`, atomic by etag |
| `sql/proposed-goalcup-functions.sql` | **Proposal** for the schema and functions SupabaseDb calls |
| `test/` | `sign.test.ts` (Rust vectors), `config.test.ts`, `github.test.ts`, `supabase.test.ts` (RPC names and arguments, stubbed fetch), `local-db.test.ts`, `integration.test.ts` (HTTP against `wrangler dev`) |

## Configuration

Variables (in `wrangler.toml` `[vars]`, or `--var` / `.dev.vars` locally):

| Name | Default | Meaning |
|---|---|---|
| `EVENT_ID` | none (required) | Event that new runs belong to, e.g. `practice`. Lowercase letters, digits, dashes; it becomes part of every R2 key. One deployment serves one event for now. |
| `RUN_HOURS` | `168` | `deadline_at` = `official_start` + this many hours. |
| `GRACE_HOURS` | `48` | After `deadline_at` plus this, chunks, receipts and seals get `410`. Which records count as late is the verify job's business. |
| `DB_BACKEND` | `supabase` | `supabase` (production) or `local`. `local` is for `wrangler dev` and tests; it refuses to work without `DEV_TOKENS`. Never set it in the deployed `wrangler.toml`. |
| `DEV_TOKENS` | none | Local backend only: `token1:member1,token2:member2`. |
| `GITHUB_CHECK` | off | `on` makes `POST /v1/runs` ask the GitHub API that `repo_url` is a public repository (`wrangler.toml` sets `on`; `npm run dev` and the tests turn it off so they need no network). A missing or private repository is `400`; GitHub being unreachable or rate-limiting is `502`, which the tracer retries. |
| `MAINTENANCE` | off | `on` makes every call answer `503 maintenance` with `Retry-After`, before authentication and before any body is read. |
| `MAINTENANCE_RETRY_AFTER` | `300` | Seconds for that header. |

Secrets (`wrangler secret put <NAME>`; locally in `.dev.vars`):

| Name | Meaning |
|---|---|
| `RECEIPT_SIGNING_KEY` | The Ed25519 seed, 32 bytes as 64 hex characters, generated offline. Its public key is what the tracer pins. |
| `SUPABASE_URL` | `https://<project>.supabase.co` (supabase backend). |
| `SUPABASE_KEY` | Key of a dedicated role limited to schema `goalcup` (see the SQL header). Not the community-wide service key. |
| `GITHUB_TOKEN` | Optional. Read-only; only raises the GitHub API rate limit. |

## What is checked where

- **Before a chunk body is read:** token, run exists and is this member's, run is open and inside
  its grace period, `Content-Length` present and at most 33,554,432, `Content-Type` and all seven
  `Goalcup-*` headers well formed, entry id matches, maintenance switch.
- **Then:** a chunk at an offset the server already holds with the same `Goalcup-Length` and
  `Goalcup-Head` is answered `200` with the stored ack (body unread); otherwise offset, `prev` and
  first sequence must continue exactly where the run stands (`409`).
- **Storage:** the body is piped (never buffered) into R2 with a create-only put and R2 checks
  the SHA-256 (`422 digest_mismatch`, nothing stored). Key:
  `traces/{event}/{run_id}/{entry_id}/{offset:016}-{sha256}.zst`. The hash is part of the name, so
  a name can only ever hold those bytes; if the name already exists (an earlier attempt stored it
  but its database step failed) the upload continues from there.
- **Then one atomic database step** advances offset, sequence and head and records the chunk and
  its ack, only if they still match what was checked. A request that loses a race is answered
  from the stored chunk if it was a retry, otherwise `409`.
- **Not checked here, by design:** whether the bytes really are a valid chain. The verify job
  does that over the reassembled copy.

## Before launch: things only real services can prove

- **R2 checksum failure.** The Worker maps a failed `sha256` put to `422 digest_mismatch` by
  recognising the error text (`10037`, "checksum", "did not match"). That wording is proven only
  in the local simulator. On a deployed preview, upload a chunk with a wrong `Goalcup-Sha256`
  against real R2 and confirm `422` and nothing stored. If it comes back `502`, the Worker log has
  a line starting `r2-put-unrecognised-error` with the real wording: add it to the pattern in
  `src/chunks.ts`. (A recognised failure logs `r2-put-digest-mismatch`.)
- **Create-only puts** (`onlyIf: { etagDoesNotMatch: "*" }`) must refuse to overwrite on real R2,
  including under the 180-day bucket lock.
- **The proposed SQL** has only been run on a local PostgreSQL, never on Supabase/PostgREST.

## What the owner must create before deploying

Nothing below exists yet. Nothing was created, logged into, or deployed while writing this code.

1. **Cloudflare R2.** A private bucket for traces (name it in `wrangler.toml`, replacing
   `replace-me-goalcup-traces`). Turn on a **bucket lock of 180 days** and **Data Access Logs**.
   Check that a locked bucket still allows the create-only puts this Worker makes (it never
   overwrites or deletes). The Worker does not need a second bucket in production; the
   `state/` objects exist only for the local backend.
2. **The Worker and its address.** On the existing paid Cloudflare account: deploy this Worker
   and attach a custom domain such as `ingest.agentcommunity.org` (uncomment and edit `routes` in
   `wrangler.toml`). `workers_dev` is off, so there is no public `*.workers.dev` address.
3. **Secrets** (`wrangler secret put`):
   - `RECEIPT_SIGNING_KEY`, made offline (for example 32 random bytes as hex on a machine that is
     not the build machine). Keep the seed in the owner's password manager, never in a repo. The public key is then
     pinned in the tracer and verifier.
   - `SUPABASE_URL` and `SUPABASE_KEY`: the key of a dedicated Postgres role (`goalcup_worker` in
     the proposed SQL) that can execute the `goalcup.upload_*` functions and nothing else.
   - Optionally `GITHUB_TOKEN` (read access to public repositories).
4. **Supabase, in the website repo** (the website repo owns the schema): migrations for schema
   `goalcup` with row-level security, and the eight `upload_*` functions. `sql/proposed-goalcup-functions.sql`
   is a proposal to start from. It was run only against a throwaway local PostgreSQL, never against Supabase.
   The owner also has to decide or provide:
   - the `events` rows (start window, `run_hours`, `grace_hours`) and the `EVENT_ID` the Worker
     is deployed with; the proposal checks that the event exists and that now is inside its start
     window, while `RUN_HOURS` and `GRACE_HOURS` come from the Worker's variables;
   - where "accepted the current terms" lives (the proposal assumes `goalcup.consents`);
   - token minting (only `sha256(token)` is stored);
   - exposing schema `goalcup` to PostgREST (RPC needs it) while revoking everything from `anon`
     and `authenticated`, as the proposal does.
5. **Not built yet** (SERVER-DESIGN sections 4, 11): per-token rate limiting (`429`; do it with a
   Cloudflare rate-limiting rule on the route), the per-run storage quota, heartbeats, outages,
   `continue`, the verify job, `live.json`, monitoring. The organizer pause on new starts is not a
   separate switch; `MAINTENANCE=on` stops everything.

## Decisions where the contract is silent

- The first record's `seq` is **0** (the baseline link of a v5 chain; this follows the contract), so
  a fresh run reports `next_seq: 0` and `head` = 64 zeros.
- `POST /v1/runs` repeated with the same `session_id` returns the existing run only while it has
  no receipt; after the receipt it is `409` (the account has an open run), as the contract says.
- A repeated `abandon` of an already abandoned run is `200`. Abandoning a sealed run is `410`.
- A retried chunk's `200` body repeats what the original answered: that chunk's ack, the offset
  just after it, `last_seq + 1`, and its head (not the run's current position).
- `Content-Length` missing, malformed or zero is `400 malformed` (the contract's table has no 411).
  `Content-Type` other than `application/zstd` is `400`.
- A `repo_url` that GitHub says is private or missing is `400 malformed`.
- Receipt, seal and chunk calls on a run past `deadline_at` + `GRACE_HOURS` are `410 closed`;
  `GET /v1/runs/{id}` always answers, whatever the status.
- Malformed run ids in a path (not `r` + 16 hex) are `404`; an unparseable chunk offset is `400`.
- A wrong HTTP method on a known path is `405 method_not_allowed` (not in the contract's table).
- The seal documents are stored in R2 as `seal-{sha256}.json` and `baseline-{sha256}.json` beside
  the chunks (create-only, named by hash), so a repeat is cheap and different documents can never
  overwrite them.
- Database or storage failures are `502 upstream_error` and misconfiguration is
  `500 misconfigured` (the same generic sentence for every cause: the detail, which may hold
  configuration, goes to the Worker's log only); the tracer retries any 5xx.
- An unsealed run more than 24 hours past its `deadline_at` is abandoned when its member next opens
  a run, which frees the member's one open slot.
