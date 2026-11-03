# Appendix G: Secrets (Lane G)

Repo: /home/user/goal-cup-proxy, commit 4b226de2 (same as origin/main). Read-only audit of 29 Oct 2026.
Nothing was built, run or tested. This appendix merges two parts: G-code (every masking rule, its code,
its tests and its verdict under the owner's option b) and G-surface (every place a secret could enter the
record, and whether today's code covers it). Main point: 4 (never record secrets). Point 3 (never block)
and point 6 (tell the participant) are checked too.

**Lane summary.** Secret masking is large but mostly sound. About 4,150 src lines in the daemon
(`redact.rs` and `redact/stream.rs`), 267 in the proxy (`headers.rs`) and about 580 in the protocol do the
work, with about 10,100 test lines behind them. The order is right: redact, then hash, then write, and raw
proxy bytes never touch disk. Redaction cannot block the harness: at its limits it over-masks or drops a
producer with a gap. Owner option b is small: add a URL password rule, then drop the email rule and four
Codex rate-limit rows. Net, that is about 40 fewer src lines and about 76 fewer test lines (the G-surface
estimate puts the URL and email swap at close to zero). The bigger risks are outside option b: a
participant's `.env` printed by the agent leaks several common shapes (`AWS_SECRET_ACCESS_KEY=`, `ASIA`
keys, `*_KEY=`, short passwords), a key the model echoes across stream deltas is stored raw, the secret
scan RULES promises does not exist, Hermes's binary `state.db` defeats text masking, and RULES.md tells
participants there is "one known gap" when there are several. The biggest code savings in this area
(the Python oracle, about 1,150 lines; the unused `object` form; the exact-value run-secret machinery,
about 400 src lines) all need an owner call.

The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on main or on
any of the 89 remote branches. Nothing here relies on it. CPU and queue effects below are not measured.

## Top findings

The two parts ranked differently. G-code put the URL password rule first, because it is the step that
must land before the email drop (it is option b work). G-surface put the env-file gaps first, because they
are the largest real leak (but fixing them is beyond option b). Both are listed.

1. **Add `url_password` before dropping `email`** (point 4, option b). Today the email rule is the only
   body rule that masks `https://user:TOKEN@host`, and only by accident. `postgres://app:PASS@localhost:5432`
   leaks today (no dot TLD), so the new rule is strictly better. Land both in one commit. (Finding G.1.)
2. **Env-file gaps in tool output** (point 4, high). `cat .env` leaks `AWS_SECRET_ACCESS_KEY=`, `ASIA...`
   temporary keys, `ENCRYPTION_KEY=` style names, passwords under 16 characters and `curl` Basic auth.
   Owner call: one upper-snake env-line rule plus `A[KS]IA` (about 50 src lines), or say it in RULES.
   (Finding G.4.)
3. **Dropping the Codex `object` rows breaks a test** (`stream.rs:3540-3546` demands a shipped `object`
   rule). Edit one line, or cut the form. (Findings G.2, G.3.)
4. **The promised verifier secret scan does not exist** (points 4, 6). Fix the RULES wording now; build the
   scan at export, after the event. (Finding G.8.)
5. **Participant text is wrong** (point 6). `docs/RULES.md:16` says "one known gap". Replace with a plain
   list; fix stale "no header is recorded" lines. (Finding G.11.)
6. **Hermes `state.db`** (points 4, 2). A binary copy defeats text masking, and one match corrupts the
   file. Owner: keep and disclose, or drop the row. (Finding G.9.)
7. **Point 3 holds.** No path leads from redaction back to the harness. Watch `QueueFull` gaps in long runs
   with many subagents. (Finding G.10.)
8. **Owner calls for later:** rate-limit headers masked by name, unused `object` form, duplicate Python
   oracle (about 1,150 lines), exact-value run-secret machinery (about 400 src lines).

## G.1 Where masking lives (sizes)

| Part | Path | Src lines | Test lines | What it does | Points |
|---|---|---|---|---|---|
| Shared body rules, whole-record redactor, select | `crates/goalcup-daemon/src/redact.rs` | 1,208 (1-1208) | 71 inline (1209-1279) | 23 shared body rules, select, marker (`redact.rs:195-311`, `1201-1207`) | 4 |
| Streaming redactor (hold, recognizers, fail closed) | `crates/goalcup-daemon/src/redact/stream.rs` | 2,945 (1-2945) | 1,063 inline (2946-4008) | One bounded recognizer per rule, hold up to 16 KiB, fail closed (`stream.rs:1-120`, `125`, `440-560`); also scrubs metadata strings (`stream.rs:2567-2604`) | 4, 3 |
| Integration tests | `crates/goalcup-daemon/tests/it/redaction.rs`, `capture_redaction.rs`, `capture_redaction_extra.rs`, `capture_redaction_run_secrets.rs` | 0 | 2,629 + 4,745 + 1,113 + 379 = 8,866 | Rule, streaming and oracle tests | 4 |
| Test support | `crates/goalcup-daemon/tests/it/support/redaction.rs` | 0 | 321 | Helpers (G-surface counts it; with it, `tests/` holds 9,187 redaction test lines) | 4 |
| Run secret forms | `crates/goalcup-protocol/src/run_secret.rs` | 376 (1-376; 530 file) | 154 inline | Exact-value forms of the run's own secrets: proxy credential, server-run GitHub token (`run_secret.rs:1-47`) | 4 |
| Connector rule schema (`ExtraPattern`) | `crates/goalcup-protocol/src/harness/schema.rs:905-1112` | about 208 | (in protocol tests) | Schema for `[redaction] extra_patterns` | 4 |
| Connector `[redaction]` rows | `connectors/amp-cli/harness.toml:100-103`, `connectors/claude-code-cli/harness.toml:154-159`, `connectors/codex-cli/harness.toml:108-117` | 4 to 9 data lines each | goldens | Amp `wsToken`, Claude billing header prefix, 7 Codex rows | 4 (Amp), privacy (rest) |
| Header masking in the proxy (Task 7) | `crates/goalcup-cli/src/proxy/headers.rs` | 267 (1-267) | 136 inline (268-403) | Masks header values by header name, Bearer/Basic in any header, URL userinfo, credential-named query values, before bytes leave the proxy (`headers.rs:20-36`, `85-152`, `219-268`) | 4 |
| Metadata allowlist, query stripped | `crates/goalcup-cli/src/proxy/metadata.rs` (407 lines total; rule at 81-106, about 26 lines) | 26 | test at `metadata.rs:261` | Path without query or fragment, no header value (`metadata.rs:1-22`, `75-104`) | 4, 5 |
| Harness env built, not inherited | `crates/goalcup-cli/src/launch.rs` (part) | | | `env_clear`, then an allowlist and a fresh `HOME` (`launch.rs:371-388`, `392-407`, `1440-1474`) | 4 |
| Workspace walk | `crates/goalcup-daemon/src/workspace.rs` (part) | | | `.env*` and `.git-credentials` listed by size, never read or hashed (`workspace.rs:415-417`) | 4 |
| Python oracle the tests compare against | `spikes/redact.py` | 349 | (used by `redaction.rs:1293-2100`) | Python twin of the rules (`spikes/redact.py:53-75`) | none directly |
| Uploader | `crates/goalcup-cli/src/upload/http.rs:106` | `Debug` hides token only | none | Uploads already redacted bytes | 4 |
| Verifier | `crates/goalcup-verifier/src/capture_analysis/wire.rs:730`, `anthropic.rs:383-387` | | | Reports redaction kinds, masks nothing | 6 |
| Worker | `server/worker/src/*.ts` | no masking (grep for `redact` or `mask` finds nothing) | | | |

Total masking code today: about 4,150 src lines in the daemon, 267 in the proxy, about 580 in the
protocol, and about 10,100 test lines (daemon inline plus integration plus headers).

The order is right and should stay: redact, then hash, then write (`redact.rs:10-12`, D14). Raw proxy
bytes never touch disk: the proxy hands frames to the daemon over loopback and the daemon redacts per
producer (`crates/goalcup-daemon/src/runtime/capture/owner.rs:296-313`). Every native file and every wire
stream goes through the same `StreamRedactor`.

## G.2 Every rule, with its verdict under option b

Owner option b, as written in `docs/plans/2026-10-29-record-everything-progress.md:94-95`: add the URL
password rule first, then drop the email rule and the Codex rate-limit rules, keep every secret rule,
nothing more.

### G.2a Shared body rules (daemon, applied to every recorded byte: wire bodies, header streams, native files)

Each shared rule has a regex in `redact.rs` SHARED (195-311) and a matching streaming shape in `stream.rs`
SHAPES (450-626). `policy_ready` (`stream.rs:754-772`) refuses to stream if the two lists differ.

| # | Kind | Rule (path:line) | What it masks | A test that covers it | Verdict | Why |
|---|---|---|---|---|---|---|
| 1 | `anthropic_key` | `redact.rs:197` | `sk-ant-` keys | `redaction.rs:165` `every_pattern_the_script_carries_still_fires` | keep | secret |
| 2 | `openai_key` | `redact.rs:198` | `sk-` and `sk-proj-` keys | `redaction.rs:165` | keep | secret |
| 3 | `masked_key` (soft) | `redact.rs:212-215` | a key echoed half-masked (`sk-ab***wxyz`) | `redaction.rs:998` `a_masked_key_in_a_401_body_is_redacted`, `:1231` | keep | partial secret |
| 4 | `github_token` fine-grained | `redact.rs:221` | `github_pat_` | `redaction.rs:2531` | keep | secret |
| 5 | `github_token` classic | `redact.rs:222` | `ghp_`, `gho_` and kin | `redaction.rs:165` | keep | secret |
| 6 | `aws_access_key` | `redact.rs:223` | `AKIA...` | `redaction.rs:165` | keep | secret |
| 7 | `bearer` | `redact.rs:224` | `Bearer <16+ chars>` anywhere in a body | `redaction.rs:165`, `:1016` | keep | secret |
| 8 | `jwt` | `redact.rs:225-228` | three-part `eyJ` tokens | `redaction.rs:165` | keep | secret |
| 9 | `private_key` (block) | `redact.rs:241-245` | PEM private key, bounded to 15,872 bytes | `capture_redaction.rs:1757` `a_pem_block_is_never_shadowed_at_any_split_point`; `redaction.rs:2146-2445` | keep | secret |
| 10 | `slack_token` | `redact.rs:247` | `xox[baprs]-` | `redaction.rs:165` | keep | secret |
| 11 | `generic_secret_assignment` JSON form | `redact.rs:259-262` | value of a JSON key named like `api_key`, `secret`, `token`, `password` | `redaction.rs:445`, `:517`, `:531`; `capture_redaction.rs:3544` | keep | secret |
| 12 | `generic_secret_assignment` shell and form-encoded | `redact.rs:275-278` | `token = ...`, and form-encoded `refresh_token=1%2F%2F...` or `0.AXoA...` (Task 1 fix) | `redaction.rs:541` `a_percent_encoded_form_refresh_token_is_masked`, `:551`, `:562`; `capture_redaction.rs:2272` | keep | secret |
| 13 | `email` | `redact.rs:281-288` (comment 280-283, literal 284-287) | any `local@domain.tld`; also, by accident, the password in `https://user:PASS@host` | `redaction.rs:186` (row), `:323`; `capture_redaction.rs:3580` | **cut, after rule 25 lands** | not a secret; breaks transcript bytes (3 emails, 15 bytes, in the measured Claude run, progress doc line 72) |
| 14-20 | `google_key`, `huggingface_token`, `npm_token`, `xai_key`, `stripe_key`, `gitlab_token`, `digitalocean_token` | `redact.rs:289-295` | fixed vendor prefixes | `redaction.rs:165` (seven rows) | keep | secret |
| 21-23 | `proxy_credential` x3 | `redact.rs:303`, `:305`, `:310` | our own proxy password: plain, percent-encoded, Basic base64 | `redaction.rs:233`; `capture_redaction.rs:142` | keep | secret |
| 24 | `run_secret` (exact values) | `redact.rs:376-470`; forms in `run_secret.rs:117-147` | the run's proxy credential and the GitHub token (D71) in 11 encodings, line-wrapped | `capture_redaction_run_secrets.rs:117`, `:152`, `:364` | keep | secret |
| 25 | `url_password` | **does not exist** | `scheme://user:PASS@host` | none | **add** | owner b; today only the email rule covers it, and only for a narrow character class |

G-surface cites some of these with slightly different ranges (for example `email` at `redact.rs:284-287`,
JSON rule at `261-263`, shell rule at `276-279`, GitHub at `220-221`, proxy credential at `296-310`). The
ranges overlap; G-code's include the comment lines.

### G.2b Connector rules (`[redaction] extra_patterns` in `harness.toml`)

| Kind | Rule (path:line) | What it masks | Test | Verdict | Why |
|---|---|---|---|---|---|
| `amp_ws_token` | `connectors/amp-cli/harness.toml:102` | Amp's `wsToken` JWT | indirect only: `stream.rs:3473` `shipped_extra_patterns` feeds `stream.rs:3524` and `:3791`; the shared `jwt` rule also matches it | keep | secret |
| `anthropic_billing_header` | `connectors/claude-code-cli/harness.toml:158` | `x-anthropic-billing-*` text | `redaction.rs:352` | keep ("nothing more") | not a secret; owner may revisit later |
| `codex_rate_limits` | `connectors/codex-cli/harness.toml:110` | the `rate_limits` object | `redaction.rs:935` | **cut** | option b |
| `codex_plan_type` | `harness.toml:111` | `plan_type` string | `redaction.rs:755` | **cut** | option b |
| `codex_credits_remaining` | `harness.toml:112` | credit balance | `redaction.rs:755` | **cut** | option b |
| `codex_credits` | `harness.toml:113` | `credits` object | `redaction.rs:797` (case row) | **cut** | option b |
| `codex_installation_id` | `harness.toml:114` | a stable install id | `redaction.rs:797` (case row) | keep ("nothing more") | an id, not a secret; phase 2 notes list it as "not decided" (`phase2-notes.md:425`) |
| `codex_safety_identifier` | `harness.toml:115` | a stable user id | `redaction.rs:797` | keep ("nothing more") | same |
| `codex_turn_state` | `harness.toml:116` | opaque server routing token | `redaction.rs:797` | keep ("nothing more") | same |

"The Codex rate-limit rules" is read as the four plan, quota and credit rows, as the phase 2 notes do
(`phase2-notes.md:424`). The three id rows stay. Owner should confirm.

### G.2c Header masking in the proxy (Task 7), runs before the daemon rules

All in `crates/goalcup-cli/src/proxy/headers.rs`. The header stream then also goes through every daemon
rule above (`headers.rs:8-9`; `http.rs:1310-1324`).

| Rule | Path:line | What it masks | Test | Verdict |
|---|---|---|---|---|
| Cookie names | `headers.rs:23-24`, `:33-36` | `cookie`, `set-cookie` values | `headers.rs:276` `a_credential_header_value_is_masked` | keep |
| Name substrings | `headers.rs:26-27`, `:33-36` | any header whose name contains `key`, `token`, `secret`, `password`, `auth` | `headers.rs:276`; `m2_proxy_integration.rs:648` | keep |
| Scheme in any header | `headers.rs:29-30`, `:167-176`, `:202-213` | `Bearer x`, `Basic x` | `headers.rs:342` | keep |
| Bare CR line | `headers.rs:101-109` | a whole line with a lone CR | `headers.rs:384` | keep |
| Folded line after a masked header | `headers.rs:116-124` | continuation of a masked value | **no direct test found** | keep, add one small test |
| URL userinfo | `headers.rs:219-242` | `user:pass@` in the request target and in any header URL | `headers.rs:364` `userinfo_in_an_absolute_target_is_masked` | keep |
| Query values (query-string masking) | `headers.rs:39-43`, `:247-266` | values of query keys matching the name rule, `sig`, or `*signature*`; names percent-decoded first | `headers.rs:314`, `:326`, `:354`, `:373`; `m2_proxy_integration.rs:720` | keep |

### G.2d Metadata (stream labels and paths)

| Rule | Path:line | What it does | Test | Verdict |
|---|---|---|---|---|
| Proxy allowlist, query dropped | `crates/goalcup-cli/src/proxy/metadata.rs:81-106` | path keeps no query or fragment; no header value reaches metadata | `metadata.rs:261` `query_and_fragment_are_removed_entirely` | keep |
| Daemon sanitize | `stream.rs:2568-2607` | drops query again, runs every rule over each metadata string | `capture_redaction.rs:2162`; `capture_redaction_extra.rs:1087`; `capture_redaction_run_secrets.rs:364` | keep (second line of defence, about 40 lines) |

### G.2e Uploader, verifier, Worker, other

- Uploader: no masking of content. It uploads chain bytes that are already redacted.
  `upload/http.rs:106` hides its token in `Debug`. Keep.
- `Debug` redaction of tokens elsewhere: `daemon/mod.rs:190-199`, `account/credential.rs:73`,
  `account/github.rs:62`, `config.rs:346-359`, `proxy/connect.rs:109`. A few lines each. Keep.
- Verifier: masks nothing. It reports the kinds it saw (`wire.rs:730`, `anthropic.rs:383-387`) and caps
  the list at 16 (`anthropic.rs:106`, `native/mod.rs:105`). Fewer kinds after option b, so the cap matters
  less. Out of scope here.
- Worker: no masking code (`server/worker/src`, grep for `redact` and `mask` returns nothing). Keep as is.

## G.3 Surface map: where a secret can enter the record

"Least masking" is the smallest rule set that still meets point 4. The verdict is for the code that covers
the surface. "Probe" means: the 23 shared literals were copied out of
`crates/goalcup-daemon/src/redact.rs:195-311` into a small Python script (`re`, same leftmost-first
semantics for these literals) and run on fake samples. It is not the Rust build. Script:
`/tmp/claude-0/-home-user/c3485515-d491-51f1-8b7f-6278b4ef8e37/scratchpad/g/probe.py` (scratch, not in the
repo).

| # | Surface | Covered today (path:line) | Least masking for point 4 | Verdict | Risk if wrong |
|---|---|---|---|---|---|
| S1 | Request headers: `Authorization`, `x-api-key`, `Proxy-Authorization`, `x-goog-api-key`, `x-amz-security-token`, `api-key` | Yes. Name contains `key`, `token`, `secret`, `password` or `auth`, or is `cookie`/`set-cookie` (`headers.rs:24-36`, `125-131`). Bearer/Basic masked in any header (`headers.rs:167-178`, `203-216`). Then the daemon rules run again on the header stream (`http.rs:1310-1324`) | What exists. Do not add the opaque-value header check from phase 2 notes section 2 (option b says nothing more) | keep | Low. A credential under an unlisted header name with no vendor prefix stays raw. No such header is known for the four harnesses |
| S2 | Response headers: `set-cookie`, `www-authenticate` | Yes, same rule (`headers.rs:24`) | Same | keep | Low |
| S3 | Rate-limit headers (`x-ratelimit-limit-tokens`, `anthropic-ratelimit-*-tokens-*`) | Over-masked: the substring `token` hits `tokens` (`headers.rs:27`, `33-36`) | Not secrets. Least masking would leave them visible | change (small, optional) | Loss of rate-limit research data only. Fix is one exemption line, or leave as is |
| S4 | URL and query in the request line and in header values | Yes. Userinfo masked; credential-named query values masked, plus `sig` and `*signature*` (`headers.rs:154-165`, `219-268`). Stream metadata keeps the path with the whole query cut (`metadata.rs:9-11`, `97`) and scrubs it again (`stream.rs:2590-2596`) | What exists | keep | A secret inside a path segment (Slack webhook style) stays. None of the four model hosts use that |
| S5 | Request bodies: OAuth refresh and token exchange | Sign-in hosts are never decrypted (`connectors/claude-code-cli/harness.toml:34`; `docs/RULES.md:323-325`). If one is: JSON `refresh_token`/`access_token` by the JSON rule (`redact.rs:261-263`), form `refresh_token=` by the shell rule (`redact.rs:276-279`), `sk-ant-oat01`/`sk-ant-ort01` by `anthropic_key` (`redact.rs:197`), Codex tokens are JWTs (`redact.rs:225-228`) | What exists | keep | Low. `code=` and `code_verifier=` (login exchange) are not matched by name; they only occur in setup, on sign-in hosts |
| S6 | Response bodies: token responses, `/api/oauth/profile`, usage | Token fields: as S5. Profile responses carry the account email: masked today only by the `email` rule (`redact.rs:284-287`), which option b drops | Not a secret. After the drop the email is recorded | change (drop email rule, tell participant) | Privacy, not point 4. Must be said plainly (point 6) |
| S7 | WebSocket frames (Amp thread, any WS on a model host) | Upgrade head masked like S1 (`http.rs:368`, `529`, `581`); each message is a stream through the daemon rules (`websocket.rs:10-23`); Amp `wsToken` row (`connectors/amp-cli/harness.toml:100-103`) plus the `jwt` rule | What exists | keep | Same echo gap as S10 for secrets split across messages |
| S8 | Harness session files: Claude Code JSONL, subagent JSONL, meta, `tool-results/*`; Codex rollouts; Hermes `session_*.json`; Amp thread logs | All through the per-producer redactor (`owner.rs:296-313`). Patterns: `connectors/claude-code-cli/harness.toml:92-148`, `connectors/codex-cli/harness.toml:87-93`, `connectors/hermes-cli/harness.toml:103-109`, `connectors/amp-cli/harness.toml:84-90` | What exists, minus email and Codex rate-limit rows | keep | Same as S11 (tool output) |
| S9 | Login files: Codex `auth.json`, Hermes `.env`/`auth.json`, Amp `secrets.json`, Claude credential store | Never captured by a native pattern (patterns above do not match them). Baseline lists the harness home by name, size and SHA-256; `.env` by name and size only (`docs/harnesses/hermes-cli.md:67`, D71) | What exists | keep | The agent can still read and print them: then S11 applies |
| S10 | Model reply streams (SSE deltas, WS deltas): a key the model echoes | Not covered. Rules need one contiguous run; deltas put JSON framing between pieces (`TODO.md:302-312`, `docs/CHEAT-CHECKLIST.md:190-210`, H-29) | No capture-time fix for Friday. Tell the participant; rotate | keep code, change docs | High: an echoed key is stored raw in R2 under the 180-day lock |
| S11 | Tool output inside transcripts and request history (`cat .env`, `env`, `git remote -v`, `cat ~/.codex/auth.json`) | Partly. Vendor prefixes, JSON key rule, shell `*api_key=`/`*secret=`/`*token=`/`*password=` (16+ chars), JWT, PEM, run secrets. Gaps found by probe, see G.4 | Option b plus an owner call on two gaps (G.4) | change | High: the workspace `.env` is part of the starting kit (`workspace.rs:419-425`) and RULES tells people to put product keys there (`docs/RULES.md:196-200`) |
| S12 | Env dumps | Harness env is cleared and rebuilt (`launch.rs:1473`), `HOME` is fresh (`launch.rs:381`), so the participant's shell exports never reach the agent. What remains: `HTTPS_PROXY` with the run credential, masked by exact value in every encoding (`run_secret.rs:12-29`) and by `proxy_credential` (`redact.rs:296-310`) | What exists | keep | Low. Codex's login-shell snapshot reads rc files of the fresh `HOME`, not the user's (unverified on Linux) |
| S13 | Hermes `state.db`, `state.db-wal`, `state.db-shm` (binary SQLite) | Captured whole once after exit (`connectors/hermes-cli/harness.toml:111-120`) and run through the same text rules | Text rules do not fit a binary page file: a value split across an overflow page has a 4-byte page number in the middle and is missed; freed pages and old WAL frames keep old rows; a masked span changes length (`redact.rs:1201-1207`), so the copy no longer opens as SQLite | change (owner call) | Medium. Same text the wire already has, plus history and split values. See G.9 |
| S14 | Config files copied into isolated homes | Nothing is copied: no connector has seed files (`ls connectors/*` shows only `harness.toml`); Amp is the only `per_run` home (`connectors/amp-cli/harness.toml:58`). Claude on macOS links the Keychain folder; not read by goalcup | None | keep | None found |
| S15 | Local logs: `host.log`, `daemon.log`, `prep.log` | Mode 0600, local only, not uploaded (`crates/goalcup-cli/src/host_log.rs:1-24`, `crates/goalcup-cli/src/daemon/spawn.rs:56-57`, `crates/goalcup-cli/src/config.rs:19-27`). `prep.log` holds what `capture.configure` commands printed | None | keep | Low. A configure command that prints a key would leave it in a local 0600 file only |
| S16 | Error messages and gaps in the record | Gap reasons are a closed enum (`crates/goalcup-protocol/src/capture.rs:191-202`). Free text: `LocalFinish.reason`, `Assistance.action` (`capture.rs:332-338`), produced by goalcup, not by the harness (unverified that no upstream text flows in) | None | keep | Low |
| S17 | Crash dumps | No `RLIMIT_CORE` or dumpable setting (grep found none). A core dump of the proxy would hold raw headers, but it stays on the machine | None for upload. Optional: set `RLIMIT_CORE=0` in the proxy process | keep | Low, local only |
| S18 | Upload metadata (chunk headers, heartbeats, vitals) | Vitals are disk free and goalcup's own process usage (`crates/goalcup-cli/src/vitals.rs:1-40`). The upload bearer credential is stored as SHA-256 only (`server/worker/src/auth.ts:1`, `25`) | None | keep | None found |
| S19 | Worker logs | Invocation logs off, so no request headers are kept (`server/worker/wrangler.toml:95-105`, staging `124-128`). Own lines name run ids, keys of R2 objects, Supabase function names, error text cut to 200 chars (`server/worker/src/db-supabase.ts:139-147`, `175-205`; `src/chunks.ts:128-225`) | None | keep | Low. `unhandled: e.stack` (`server/worker/src/index.ts:99`) prints a stack, no secret in normal paths |

## G.4 Findings

Each finding gives the point, what the code does today, the verdict, lines, risk and blind spots. Source
labels in brackets: G-code finding numbers (G-1 to G-9) and G-surface finding numbers (G1 to G9).

### G.1 Add the URL password rule, then drop the email rule (points 4, 1, 3, 5) [G-code G-1, G-2; G-surface G1]

**Today.** No URL password rule exists. The email rule (`redact.rs:281-288`) catches
`https://user:TOKEN@host` only because `TOKEN@host.tld` looks like an address. Its own comment says so
(`redact.rs:280-283`: "a URL credential ... is a local part here"). The test `capture_redaction.rs:3580`
`long_secret_keys_and_url_credentials_are_redacted_whole` relies on that (its doc says so at 3575-3578).
The header path already masks userinfo (`headers.rs:219-242`); bodies and native files do not. The probe
shows `postgresql://postgres:PASS@db.x.supabase.co` and `rediss://default:PASS@host.upstash.io` are
masked only by `email`, which also masks the host name. `postgres://app:PASS@localhost:5432` leaks today
(no dot TLD), so a URL rule is strictly better.

Email code today: rule `redact.rs:281-288` (literal 284-287); streaming code in `stream.rs`:
`EMAIL_LOCAL_MAX` 148-149, `local_part` and `domain` 284-293, `Form::Email` 386, shape 540-544 (G-surface:
539-544), `enum Email` 967-978, `impl Email` 1538-1576 (G-surface: 1538-1572), `Walk::Email` 1103, `spawn`
arms 1615-1618 and 1648, `step` arm 1687-1694, and the `prev_local` field that exists only for email
(1874, 1889, 2017, 2037, 2283, plus the `spawn` parameter); test logic `stream.rs:3100`, `3145-3146`; twin
in `spikes/redact.py:69`. Tests mention email 17 times in `capture_redaction.rs` and 16 times in
`redaction.rs`.

**Verdict.** Change: add `url_password`, then cut `email`. Land the URL rule before, or in the same commit
as, the drop.

**The rule shape: the parts disagree.**

| | G-code | G-surface |
|---|---|---|
| Regex | `//[^\s/?#@:"'\\<>]{0,256}:[^\s/?#@"'\\<>]{1,1024}@` | `(?i)[a-z][a-z0-9+.\-]{1,15}://[^\s/?#@:"'<>\\]{0,256}:([^\s/?#@"'<>\\]{1,256})@` |
| Span | whole match, no capture group. Output keeps `https:` and the host: `https:<redacted:url_password>host/...` | value in group 1, so the URL keeps its shape (user and `@` stay) |
| Password cap | 1,024 characters | 256 characters (hold about 530 bytes) |
| Why | A group span that is not between JSON quotes gets a quoted marker (`redact.rs:343-351`, `:756`), which would put a raw `"` inside a JSON string and break the record. The leading `//` keeps `npm:react@18` style text unmasked; `ssh://git@github.com` (no colon) is left alone | Keeps the URL readable; scheme prefix limits false hits |

The G-code reason was checked in this merge: `span_is_a_json_string_interior` (`redact.rs:340-351`) is
asked only of capture-group spans, and a group span inside a URL inside a JSON string is not bounded by
quotes. So the G-surface group form needs a check that its marker does not break JSON before it is
chosen. The whole-match form avoids that question. Owner or implementer picks; the whole-match form is the
safer default.

**What the change needs.** A SHARED literal, a SHAPES entry with a bounded recognizer, the Python twin
line, and the pinned count and kind lists in `tests/it/redaction.rs` (`:144`, `:1324`, `:1488`, `:1616`).
G-code sizes the streaming part at one `Form`, one `Walk` arm, one shape row, a small state machine like
`Email` at 1538-1576, a `spawn` arm and a `secret_from` arm.

**Lines: the parts disagree.**

| | G-code | G-surface |
|---|---|---|
| Add `url_password` | +61 src (about 6 in `redact.rs`, about 55 in `stream.rs`), +1 `redact.py`, about +40 tests | about +70 src, +80 tests |
| Drop `email` | about -100 src (about 8 `redact.rs`, about 92 `stream.rs`), -1 `redact.py`, about -10 tests net | about -75 src, -150 tests |
| Net | about -40 src | close to zero |

**Tests that go or change (from G-code).**
- Re-point `capture_redaction.rs:3580` (its two URL cases now name `url_password`). Add cases for: a
  percent-encoded password, a password with `!` and `:` (email missed these), `ssh://git@github.com` left
  alone, an email address left alone, and a JSON-escaped URL. About 40 test lines.
- `redaction.rs:186` (the `someone@example.com` row): delete.
- `redaction.rs:323-351` `reverse_ordered_high_match_input_keeps_first_pattern_wins_and_source_order`:
  uses 2,000 emails; rewrite with another kind (net 0).
- `redaction.rs:2239` (expects `<redacted:email>` after a PEM block): edit one line.
- Kind lists `redaction.rs:144`, `:1324`, `:1488`, `:1616`: swap `email` for `url_password`.
- `stream.rs:3383` and 3397 (`local-part` body of `held_state_stays_bounded`) and `stream.rs:3145` (email
  exception): delete about 4 lines.
- `spikes/redact.py:69` and the documented-difference tables in `redaction.rs:1519-2100`: edit.
- Verifier tests that use `"email"` only as a sample kind name (`wire.rs:1098-1205`,
  `m2_chat_decoder.rs:1612`, `m2_native_observations.rs:711-740`) and the committed spike fixtures that
  already carry `<redacted:email>`: no change needed; they are data.

**Point 3 effect.** The email candidate is live on every letter-and-digit run, up to 1,024 units
(`stream.rs:1618`, `:1687-1694`). It is the rule that most often keeps bytes held, which makes `release`
run every regex over the held bytes twice per push (`stream.rs:2141-2209`). Dropping it should cut CPU in
the owner thread. Not measured. The URL rule only starts on `/` after `:`, so it is far cheaper.

**Risks and blind spots.**
- Misses `https:\/\/user:pass@` (JSON with escaped slashes, rare) and passwords longer than the cap.
  G-code: with a 1,024 cap the candidate ends but the regex still matches below the 16 KiB hold, so the
  stream and the oracle agree.
- A token used as the user part with no password (`https://TOKEN@host`, Sentry DSN style) is not masked.
  Sentry DSN keys are meant to be public in client code, so that is fine. A GitHub token as user is still
  caught by `github_token`.
- The streaming recognizer must be pinned like the others (`recognizers_hold_every_prefix_of_a_match`,
  `stream.rs:3103`), or `policy_ready` refuses to stream and every producer is lost
  (`owner.rs:298-303`). That is a gap, not a block, but it would lose a whole run. Run the full redaction
  suite before merge.
- The email rule also hid account emails in profile responses (S6), prompts, git logs and transcripts.
  After the drop they are recorded. That is privacy, not a secret. Docs must say so (point 6):
  `docs/RULES.md:344` describes body masking in general terms; add one line that email addresses are
  recorded (see G.11).

### G.2 Drop the four Codex rate-limit rows (points 4, 1, 5) [G-code G-3; G-surface G2]

**Today.** `connectors/codex-cli/harness.toml:108-117` has 7 rows; the four to drop are at 110-113, with
the comment at 102-107. `rate_limits`, `plan_type`, `credits_remaining`, `credits` are plan and quota
data. D26 says such fields are stripped "before anything is published", not at capture
(`docs/DECISIONS.md:155-158`). These are data rows; the code that runs them (the JSON field recognizer)
stays, because `amp_ws_token` and the three Codex id rows still use it. G-surface notes a run report showed
5 masked `codex_rate_limits` spans; which report that was is not named in the part, and the 2-hour report
does not exist (unverified).

**Verdict.** Cut 4 TOML rows; edit the comment.

**Tests that go or change.**
- `redaction.rs:752-766` `the_codex_harness_strips_the_credit_balance_it_names` (15 lines): delete.
- `redaction.rs:930-953` `stripping_rate_limits_leaves_the_token_count_record_readable` (24 lines):
  delete.
- `redaction.rs:812-847`: the four `Case` rows for `codex_rate_limits`, `codex_plan_type`,
  `codex_credits_remaining`, `codex_credits` (about 34 lines): delete.
- `capture_redaction_extra.rs:42-45` (`EXTRA_KINDS`): drop four names. `:1111`
  (`metadata_strings_go_through_the_connector_rules` asserts `codex_plan_type`): point it at another rule.
  The corpus strings at 178-221 and generators at 277-340 still run (the fields are just no longer masked,
  and stream and oracle must still agree), so they can stay.
- `crates/goalcup-cli/tests/fixtures/harness-golden/codex-cli.json:76-91` (16 lines): regenerate. Also the
  pin and golden updates G-surface names (`pins/connector-dirs.txt`, harness goldens).
- Protocol fixtures `crates/goalcup-protocol/tests/fixtures/harness/codex-cli.toml`,
  `codex-cli.chat-mode.toml`, `frozen-connectors/codex-cli/harness.toml`: update or leave (they are
  fixtures of the schema, not of the shipped connector). Unverified whether any test compares them to the
  shipped file.
- **Hidden break:** `stream.rs:3540-3546` asserts that some shipped rule uses each of the `object`,
  `string` and `scalar` forms (checked in this merge). Only `codex_rate_limits` and `codex_credits` use
  `object`. Dropping them fails this test. Fix: remove `"object"` from that list (1 line), or cut the form
  (G.3).

**Lines.** About 4 TOML lines and about 90 test lines go, plus 16 golden JSON lines.

**Risk.** Plan type, quota and credit balance now enter the chain and the uploaded run. They are account
facts, not secrets. Raw records are private; strip at export (D26). Point 6 copy should say so.

**Owner question, not in option b.** `x-codex-installation-id`, `safety_identifier`,
`x-codex-turn-state` (Codex) and `anthropic_billing_header`
(`connectors/claude-code-cli/harness.toml:154-159`) are ids and billing tags, not secrets. Least masking
would drop them too. Option b says "nothing more", so keep them unless the owner says otherwise.

**A possible inconsistency, unresolved.** G-surface: if `x-codex-turn-state` also arrives as a real HTTP
response header (likely, given its name; unverified), the header stream records it raw, since the header
rule has no `state` part (`headers.rs:27`) and the connector row only matches the JSON field. The connector
comment says the opposite: these are JSON fields in `codex.response.metadata`, "not an HTTP header"
(`connectors/codex-cli/harness.toml:105-107`). A live Codex run would settle it. Either way it is an id,
not a secret.

### G.3 The `object` value form becomes unused (point 4, less code), owner call [G-code G-4; G-surface G2]

**Today.** `FieldValue::Object` in `schema.rs:969-970`, `:983`; `stream.rs:344-347` (`object_char`),
`:410-411`, `:1155-1158`, `:1172-1180` (G-surface: `1154-1162`); `with_extra` mapping `stream.rs:712`.
After G.2 no connector uses it.

**Verdict.** Option b says "nothing more", so the minimum is to keep it and edit the test (G.2). Cutting
it saves about 25 src lines (G-code) or about 30 to 50 lines (G-surface), plus the synthetic object cases.
It is a schema change. Not needed for Friday. Flag only.

**Risk if cut.** A later connector that needs to mask a JSON object cannot. Low for Friday.

### G.4 Env-file gaps in tool output (`cat .env`): the biggest real leak (point 4) [G-surface G3]

**Today, by probe on fake values.**

| Line the agent prints | Today |
|---|---|
| `AWS_SECRET_ACCESS_KEY=wJalr.../K7MDENG/...` | leaks: keyword must sit right before `=`, and `key` alone is not a keyword (`redact.rs:278`) |
| `AWS_ACCESS_KEY_ID=ASIA...` (temporary key) | leaks: only `AKIA` is a rule (`redact.rs:223`) |
| `ENCRYPTION_KEY=...`, `PRIVATE_KEY=0x...`, `SESSION_KEY=...` | leaks (`*_KEY=` other than `API_KEY`) |
| `DB_PASSWORD=hunter2hunter2` (under 16 chars) | leaks (floor of 16, `redact.rs:278`) |
| `curl -H "Authorization: Basic dXNl..."` in a command | leaks: `bearer` rule only (`redact.rs:224`) |
| bare `gsk_...`, `tvly-...`, `re_...`, `vcp_...`, `sgamp_...` | leak: no prefix rule. With a `*_API_KEY=` name in front they are caught |
| `postgres://app:PASS@localhost` | leaks (G.1 fixes) |
| `OPENAI_API_KEY=sk-proj-...`, `GROQ_API_KEY=gsk_...`, `STRIPE_WEBHOOK_SECRET=...`, `GITHUB_TOKEN=ghp_...`, JSON `"aws_secret_access_key": "..."` | masked |

**Why it matters.** `.env` is the one file the starting kit allows (`workspace.rs:419-425`,
`crates/goalcup-cli/src/commands/start/project.rs:147-149`), and RULES invites product keys there
(`docs/RULES.md:196-200`). An agent working for a week will print it.

**The parts differ on what to do.** G-code (G-5) reads option b strictly: keep every secret rule and add
nothing beyond `url_password`. G-surface says option b alone leaves this leak and asks the owner for one
more rule. Both agree that, without the rule, participants must be told.

**Least masking that closes most of it (owner question, beyond option b).** One env-line rule: "an
UPPER_SNAKE name whose last segment is `KEY`, `SECRET`, `TOKEN`, `PASSWORD` or `PASS`, then `=`, then a
value of 8 or more of `[A-Za-z0-9_./+=-]`". Matching any name that merely contains `KEY` would be too wide.
Upper-case only keeps it off source code like `let key = x`. Plus `ASIA` beside `AKIA` (one character
class: `A[KS]IA`). Cost: one literal and one recognizer (about 50 src, 60 test lines), and the
`aws_access_key` regex edit (1 line in each of three places: `redact.rs`, `stream.rs`, `spikes/redact.py`).

**Without it.** Tell the participant plainly (G.11), and the post-event scan (G.8) must catch these shapes.

**Risk.** Of adding: over-masking a few config lines in transcripts. Of not adding: an AWS secret or a
service key from the participant's own product is uploaded raw.

### G.5 Keep every other secret rule; do not add opaque-token value masking (point 4) [G-code G-5; G-surface S1]

**Today.** Rules 1-12 and 14-24 in G.2a; header rules in G.2c; metadata in G.2d.

**Verdict.** Keep. The phase 2 notes propose an opaque-token check for header values
(`phase2-notes.md:427-433`). Option b says nothing more, so do not build it.

**Risk.** An opaque credential under a header name that has none of the five words is recorded raw.
`docs/RULES.md:343` already tells participants this. No such header is known for the four harnesses.

### G.6 Header name rule over-masks rate-limit headers (points 4, 6) [G-code G-6; G-surface G4, S3]

**Today.** `headers.rs:26-27` (and `33-36`) masks any header whose name contains `token`. Provider headers
such as `anthropic-ratelimit-tokens-remaining` or `x-ratelimit-remaining-tokens` therefore show `[masked]`
(inferred from the rule; not checked on a live run). It also over-masks `www-authenticate`. It is
fail-closed, which is right for headers.

**Verdict.** Keep under option b. It is over-masking, not a leak. It sits oddly next to G.2, which
unmasks Codex rate limits in bodies. Optional one-line change (G-surface): never mask a name containing
`ratelimit`. Do not move to whole-segment matching (phase 2 notes, section 4): Supabase-style `apikey` and
`x-apikey` would then need their own entries, and a miss here leaks the harness login itself.

**Risk.** None for secrets either way. Small loss of non-secret facts.

### G.7 Run secrets by exact value: keep for now (point 4) [G-surface G5; G-code rule 24]

**Today.** `run_secret.rs` (376 src) plus the run-secret recognizer in `stream.rs` mask the proxy
credential and a server run's GitHub token in every encoding, wrapped or not (`run_secret.rs:12-29`). The
`proxy_credential` pattern rules (`redact.rs:296-310`) cover the plain, percent and Basic forms anyway.

**Verdict.** Keep (owner: "keep every secret rule"). Note for later: the proxy credential is
loopback-only and dead after the run (D60, `docs/DECISIONS.md:510-527`), and GitHub tokens have prefix
rules (`redact.rs:220-221`). Cutting the exact-value machinery would save about 400 src and 379 test
lines, and lose only base64 or `\u`-escaped forms. Not for Friday.

**Risk of a cut.** `env | base64` output of a classic token would leak.

### G.8 The model echo gap (H-29) and the promised scan that does not exist (points 4, 6) [G-surface G6]

**Today.** No rule sees a key split across SSE or WS deltas (`TODO.md:302-312`). RULES says "A trace in
which the verifier finds secret-shaped text is not rejected: it is flagged" (`docs/RULES.md:294-296`). No
such scan exists in `crates/goalcup-verifier/src` (grep for secret scanning finds only the redaction audit
reader, `capture_analysis/wire.rs:161-286`). `TODO.md:117-118` moved it to the post-event export.

**Verdict.** Change the words, not the code, for Friday. RULES should say "is scanned before anyone else
gets it" only once the scan exists; today say "will be scanned at export". Build the scan after the event,
before any export: join decoded reply text per response, run the same shared rules plus gitleaks-style
shapes, flag by kind and count.

**Realistic limit.** A stream-aware redactor that joins deltas before matching is the only capture-time
fix. It is large (a decoder per wire format inside the daemon) and against "the daemon parses nothing"
(D36). Not for Friday.

**Risk.** The raw reply is in R2 under a 180-day lock; only rotation closes it.

### G.9 Hermes `state.db`: text masking on a binary database (points 4, 2, 5) [G-surface G7]

**Today.** `connectors/hermes-cli/harness.toml:111-120` copies `state.db*` whole after exit. Hermes keeps
messages and tool output there (`messages` table in Hermes `hermes_state_common.py:531`, local copy of
Hermes source, unverified against the pinned version). SQLite facts: an overflow page starts with a 4-byte
next-page number, so a long value is cut into pieces with binary bytes between them
([SQLite file format](https://www.sqlite.org/fileformat2.html)); the WAL keeps several versions of a page
and old frames may stay on disk (same page); freed pages are not cleared unless `secure_delete` is on
([PRAGMA secure_delete](https://www.sqlite.org/pragma.html#pragma_secure_delete)). The marker changes
length (`redact.rs:1201-1207`), so one match makes the copy unreadable as SQLite.

**Least masking that still meets point 4.** Export rows as text (JSON lines) at seal and redact those, or
do not copy the file. Both are owner calls. The wire already records every Hermes model call (point 1).
The file is the only native record of Hermes delegation (A4 notes this).

**Verdict.** Change, owner call. Cheapest safe option for Friday: keep it, and say in RULES that Hermes's
database is uploaded whole and masking there is best effort. Cheapest strict option: drop the row (8 data
lines) and accept wire-only for Hermes.

**Risk.** Keep: split values and old rows leak. Drop: no native subagent tree for Hermes.

### G.10 Streaming redaction cannot block the harness (point 3) [G-code G-8]

- Proxy side: `CaptureEmitter` and the IPC sink are synchronous, take one short lock, never wait and never
  do I/O; a refused frame stops capture, never the relay (`proxy/api.rs:5-8`, `proxy/ingest.rs:20-27`).
  `masked_head` runs inline on the relay task (`http.rs:1311-1325`) but is one pass over one bounded head.
- Daemon side: redaction runs in the entry owner thread (`runtime/capture/owner.rs:283-312`), after
  `drain` has released the sink lock (`sink.rs:142-162`). The sink never waits on disk or network
  (`sink.rs:115-117`). A full queue (8 MiB, `capture.rs:51`) answers `QueueFull` and the producer is lost
  with a gap (`sink.rs:274`); the proxy keeps relaying.
- The redactor itself does no I/O and has no waits (`stream.rs:107-112`). At its bounds it fails closed
  (over-masks to the end of a line) instead of waiting: 16 KiB hold (`stream.rs:125`), 64 live candidates
  (`stream.rs:133`).

**Verdict.** Keep. Point 3 holds by design. G-surface agrees: the fail-closed streaming hold
(`stream.rs:70-90`) is large, but a simpler line-based redactor would lose "a secret split across two
pushes is one secret" (`stream.rs:61-66`). Lanes A2 and C-daemon flag `stream.rs` as the largest file;
this lane's view: do not rewrite it before Friday.

**Risks.**
- CPU: one owner thread per entry redacts every producer. Each byte is checked against every rule
  (`stream.rs:2022-2039`), about 23 shared plus 7 to 10 connector plus up to 22 run-secret forms; and each
  push that leaves a live candidate rescans the held bytes with every regex, twice
  (`stream.rs:2141-2209`). With thousands of subagents, if the thread falls behind, the queue fills and
  frames are lost as gaps. That hurts point 1, not point 3. Dropping email (G.1) helps. Unmeasured; the
  long run should report `QueueFull` gaps.
- Memory if open-stream limits are raised: 16 KiB held per open stream. At 4,096 streams that is 64 MiB
  worst case. Fine, but the comment at `stream.rs:122-124` ("128 open streams hold at most 2 MiB") becomes
  stale.
- Over-masking: a base64 run of 16 KiB or more (Codex `encrypted_content`) can blank the rest of its line
  (F-07, `docs/CHEAT-CHECKLIST.md:178-181`). Data loss of unreadable bytes, not a block.
- A redactor that cannot be built loses its producer (`owner.rs:298-303`). A broken new shape from G.1
  would lose every producer. Tests catch it; run them.

### G.11 Stale comments and the participant text (point 6) [G-code G-7; G-surface G9]

**Stale lines.** Since Task 7, header streams are recorded (`http.rs:1310-1324`) and pass through these
rules. These lines still say otherwise:
- `connectors/codex-cli/harness.toml:107`: "no header is ever recorded".
- `crates/goalcup-protocol/src/harness/schema.rs:960-963`: "No header is recorded".
- `docs/CHEAT-CHECKLIST.md:204-205`: "headers are skipped unread" (`http.rs:726`).

Verdict: change the comments (0 code). Risk if left: a reviewer believes header-prefix rules never see real
headers.

**RULES.md is wrong.** `docs/RULES.md:16` says secrets are removed "with one known gap". That is not true
today: G.4, G.8 and G.9 list more. After option b, emails, plan type and credit balance are also recorded.
Suggested text for RULES and the start screen, grade 9:

> Goal Cup Proxy removes known secret shapes before anything is saved: API keys with a known prefix,
> login headers, cookies, tokens in URLs, private keys, JWTs, and `NAME_API_KEY=`, `NAME_SECRET=`,
> `NAME_TOKEN=`, `NAME_PASSWORD=` lines. It cannot catch every secret. These can still be recorded:
> a key the model repeats in its reply, a value with no known shape (for example a short password or a
> name like `ENCRYPTION_KEY=`), and Hermes's database. Your account email and plan details are recorded.
> Raw records stay private and are cleaned before research use. So: use keys you can revoke, give the
> agent only the keys its product needs, and rotate every key in your project folder and your harness
> login after the run.

If the owner adds the env-line rule (G.4), drop "a name like `ENCRYPTION_KEY=`" from the list. If the
owner drops the Hermes row (G.9), drop "Hermes's database". Also add one line near `docs/RULES.md:344`
that email addresses are recorded.

### G.12 Test oracle duplication (none of the seven points), outside option b [G-code G-9]

**Today.** `spikes/redact.py` (349 lines) is a second copy of the rules, and `redaction.rs:1293-2100`
(about 800 test lines) checks that the two agree, with documented differences. Every rule change (G.1,
G.4) must be made twice.

**Verdict.** Outside option b; flag for the test-cut lane. Cutting it would save about 1,150 lines and make
G.1 cheaper.

**Risk.** The committed spike fixtures were redacted by that script (`AGENTS.md:93`); the fixtures stay
valid as data without it.

### G.13 What stays good and should not be touched [G-surface G8]

- `env_clear` and the allowlist (`launch.rs:1440-1474`, `371-407`): this removes the whole "participant's
  shell secrets" surface. Keep.
- `.env` never read or hashed in the workspace walk (`workspace.rs:415-417`). Keep.
- Metadata allowlist and scrub (`metadata.rs:1-22`, `stream.rs:2567-2604`). Keep.
- Worker: invocation logs off, credential stored as hash (`wrangler.toml:95-105`, `auth.ts:1-28`). Keep.
- Fail-closed streaming hold (`stream.rs:70-90`). See G.10.

## G.5 Token formats checked (web)

| Format | Source | Covered today |
|---|---|---|
| Anthropic API `sk-ant-api03-...`, admin `sk-ant-admin01-...` | [gitleaks default rules](https://github.com/gitleaks/gitleaks/blob/master/config/gitleaks.toml) | yes, `redact.rs:197` |
| Claude login `sk-ant-oat01-` (access), `sk-ant-ort01-` (refresh) | community docs, not official: [guide](https://apoorva87.github.io/RevEng101/UsageDashboard/Claude/docs/claude_oauth_guide.html) | yes, `redact.rs:197` |
| OpenAI `sk-proj-`, `sk-svcacct-`, `sk-admin-` | [gitleaks](https://github.com/gitleaks/gitleaks/blob/master/config/gitleaks.toml) | yes, `redact.rs:198` (class takes `-`) |
| OpenRouter `sk-or-v1-` | not in gitleaks; seen in practice (unverified) | yes, via `openai_key` (probe) |
| GitHub `ghp_ gho_ ghu_ ghs_ ghr_`, `github_pat_` | [GitHub blog](https://github.blog/2021-04-05-behind-githubs-new-authentication-token-formats) | yes, `redact.rs:220-221` |
| AWS `AKIA` long-term, `ASIA` temporary (also `ABIA`, `ACCA`) | [AWS STS GetAccessKeyInfo](https://docs.aws.amazon.com/STS/latest/APIReference/API_GetAccessKeyInfo.html), [hackingthe.cloud](https://hackingthe.cloud/aws/general-knowledge/iam-key-identifiers/) | `AKIA` only, `redact.rs:223`; secret access key has no shape and leaks in env form (G.4) |
| Google `AIza` | [gitleaks](https://github.com/gitleaks/gitleaks/blob/master/config/gitleaks.toml) | yes, `redact.rs:289` |
| Hugging Face `hf_`, npm `npm_`, GitLab `glpat-`, Slack `xox[baprs]-`, Stripe `sk_live_`/`rk_live_` | [gitleaks](https://github.com/gitleaks/gitleaks/blob/master/config/gitleaks.toml) | yes, `redact.rs:231-295`. Stripe `sk_test_` not covered (test keys, low value) |
| Perplexity `pplx-` | [gitleaks](https://github.com/gitleaks/gitleaks/blob/master/config/gitleaks.toml) | no prefix rule; caught only with a `*_API_KEY=` name |
| Amp `sgamp_` | [amp-sdk on PyPI](https://pypi.org/project/amp-sdk/0.1.4/) (third party) | no prefix rule. Sent as a header (masked); in `secrets.json` the JSON key `apiKey@https://...` does not match the JSON rule (probe) |
| Groq `gsk_`, Tavily `tvly-`, Resend `re_`, Vercel `vcp_` | not in gitleaks default (checked above link) | no; caught only with a `*_API_KEY=` or `*_TOKEN=` name |
| `curl` Basic auth | [gitleaks `curl-auth-header`](https://github.com/gitleaks/gitleaks/blob/master/config/gitleaks.toml) | headers yes (`headers.rs:30`); in bodies no |

Each missing prefix is one `prefixed(...)` entry in `stream.rs` and one literal in `redact.rs` and
`spikes/redact.py` (about 10 lines each plus pinned-count test edits). Option b says nothing more, so they
are owner questions, not work.

## G.6 Tool output: what is realistic

- Realistic at capture: shape rules on contiguous text. They cover vendor prefixes, named assignments in
  JSON and env form, JWT, PEM, URL passwords (after G.1), and the run's own secrets by value.
- Not realistic before Friday: opaque values with no name and no prefix, short passwords, model echoes
  split across deltas, binary files. Entropy rules give many false hits on code, hashes and base64 images
  (phase 2 notes, section 4 say the same).
- Realistic after the event: a scan of the joined, decoded text at export (G.8), with gitleaks-like rules,
  flagging by kind and count. That is where wider shapes belong, because a false hit there costs nothing
  at capture.
- What to tell the participant: the G.11 text. The key advice is rotation, because a raw record cannot be
  rewritten.

## G.7 Totals under option b (estimates from the line ranges above)

G-code estimate:

| Change | Src lines | Test and fixture lines |
|---|---|---|
| G.1 add URL password rule | +61 (6 `redact.rs`, 55 `stream.rs`), +1 `redact.py` | +40 |
| G.1 drop email | -100 (8 `redact.rs`, 92 `stream.rs`), -1 `redact.py` | about -10 net (deletes and edits; one test rewritten) |
| G.2 drop Codex rate-limit rules | -4 TOML rows | about -90 (two tests, four cases, kinds list) and -16 golden JSON |
| G.2 forced test edit (`stream.rs:3540`) | 0 | 1 line edited |
| G.11 comments | 0 (2 comment edits) | 0 |
| **Net** | **about -43** | **about -76** |

G-surface estimate for the URL and email swap: about +70 src and +80 tests added, about -75 src and -150
tests removed, net close to zero src. Both agree option b is small: the masking code stays about 4,100 src
lines. The big savings in this area are outside option b (G.3, G.7, G.12) and need an owner decision.

Tests that go entirely: `the_codex_harness_strips_the_credit_balance_it_names` (`redaction.rs:755`),
`stripping_rate_limits_leaves_the_token_count_record_readable` (`redaction.rs:935`).

Tests that change: `every_pattern_the_script_carries_still_fires`,
`reverse_ordered_high_match_input_keeps_first_pattern_wins_and_source_order`,
`the_shared_set_kinds_are_in_the_documented_order`,
`a_connector_pattern_leaves_the_record_valid_json_with_its_key_intact`,
`a_claim_across_a_pem_edge_is_merged_into_the_block`,
`long_secret_keys_and_url_credentials_are_redacted_whole`,
`the_shipped_rules_are_the_ones_these_tests_exercise`,
`metadata_strings_go_through_the_connector_rules`, `every_rule_form_streams_with_its_span`,
`held_state_stays_bounded`, `recognizers_hold_every_prefix_of_a_match`, and the `spikes_redact_py_*`
tests.

## G.8 Verdict table (brief format)

| Part (path, src, tests) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| `redact.rs` (1,208 / 71) | Shared rules, select, marker | 4 | change | add `url_password`, drop `email` | about -2 / see `stream.rs` | broken shape loses every producer | none whole; row `redaction.rs:186` |
| `redact/stream.rs` (2,945 / 1,063) | Streaming twin, hold, fail closed | 4, 3 | change | same swap; keep the hold | about -37 / about -4 | CPU on one owner thread with thousands of subagents | lines at `stream.rs:3145`, `3383`, `3397` |
| Integration tests (0 / 8,866 + 321 support) | Rule, stream and oracle tests | 4 | change | follow the rule changes | 0 / about -76 net | none | `redaction.rs:752-766`, `930-953`, case rows `812-847` |
| `run_secret.rs` (376 / 154) | Exact-value run secrets | 4 | keep (later: owner call) | "keep every secret rule" | 0 now; about 400 / 379 if cut later | base64 token forms leak if cut | none |
| `schema.rs:905-1112` (about 208) | Connector rule schema | 4 | keep; `object` form owner call | unused after G.2 | 0 now; about 25 to 50 if cut | later connector cannot mask an object | object cases if cut |
| Codex `[redaction]` rows | 7 JSON field rules | privacy | change | drop 4 rate-limit rows | 4 TOML / about 106 test and golden | plan and credit data recorded | two tests, four cases |
| Amp and Claude `[redaction]` rows | `wsToken`, billing header | 4, privacy | keep | option b: nothing more | 0 | none | none |
| `headers.rs` (267 / 136) | Header, userinfo, query masking | 4 | keep (add one test) | fail-closed, correct | 0 | rate-limit headers over-masked | none |
| `metadata.rs:81-106` (26) and `stream.rs:2568-2607` | Metadata allowlist and scrub | 4, 5 | keep | two cheap layers | 0 | none | none |
| `launch.rs` env build, `workspace.rs:415-417` | Env cleared, `.env` never read | 4 | keep | removes a whole surface | 0 | none | none |
| `spikes/redact.py` (349) and `redaction.rs:1293-2100` (about 800) | Python oracle and comparison | none | cut (owner call, test-cut lane) | duplicate of the rules | 349 / about 800 | fixtures stay as data | `spikes_redact_py_*` |
| Hermes `state.db*` row (`hermes-cli/harness.toml:111-120`) | Binary SQLite copy | 2, 4 | change (owner call) | text masking misses split values, corrupts file | 8 data lines if dropped | no native Hermes subagent tree if dropped | Hermes goldens (unverified) |
| Verifier secret scan | Does not exist | 4, 6 | change docs now; build at export | RULES promises it | +0 for Friday | echoed keys stay raw | none |
| Worker (`server/worker/src`) | No masking; logs off; hashed credential | 4 | keep | correct | 0 | none | none |

## G.9 Final rule list

### Rules to keep

| Rule | Where (path:line) | Tests that cover it |
|---|---|---|
| `anthropic_key` | `redact.rs:197` | `redaction.rs:165` |
| `openai_key` | `redact.rs:198` | `redaction.rs:165` |
| `masked_key` | `redact.rs:212-215` | `redaction.rs:998`, `:1231` |
| `github_token` fine-grained and classic | `redact.rs:221`, `:222` | `redaction.rs:2531`, `:165` |
| `aws_access_key` (`AKIA`) | `redact.rs:223` | `redaction.rs:165` |
| `bearer` | `redact.rs:224` | `redaction.rs:165`, `:1016` |
| `jwt` | `redact.rs:225-228` | `redaction.rs:165` |
| `private_key` | `redact.rs:241-245` | `capture_redaction.rs:1757`; `redaction.rs:2146-2445` |
| `slack_token` | `redact.rs:247` | `redaction.rs:165` |
| `generic_secret_assignment` JSON | `redact.rs:259-262` | `redaction.rs:445`, `:517`, `:531`; `capture_redaction.rs:3544` |
| `generic_secret_assignment` shell and form | `redact.rs:275-278` | `redaction.rs:541`, `:551`, `:562`; `capture_redaction.rs:2272` |
| Seven vendor prefixes (`google_key` to `digitalocean_token`) | `redact.rs:289-295` | `redaction.rs:165` |
| `proxy_credential` x3 | `redact.rs:303`, `:305`, `:310` | `redaction.rs:233`; `capture_redaction.rs:142` |
| `run_secret` exact values | `redact.rs:376-470`; `run_secret.rs:117-147` | `capture_redaction_run_secrets.rs:117`, `:152`, `:364` |
| `amp_ws_token` | `connectors/amp-cli/harness.toml:102` | indirect: `stream.rs:3473`, `:3524`, `:3791` |
| `anthropic_billing_header` (owner may revisit) | `connectors/claude-code-cli/harness.toml:158` | `redaction.rs:352` |
| `codex_installation_id`, `codex_safety_identifier`, `codex_turn_state` (owner may revisit) | `connectors/codex-cli/harness.toml:114-116` | `redaction.rs:797` |
| Header: cookie names | `headers.rs:23-24`, `:33-36` | `headers.rs:276` |
| Header: name substrings (`key`, `token`, `secret`, `password`, `auth`) | `headers.rs:26-27`, `:33-36` | `headers.rs:276`; `m2_proxy_integration.rs:648` |
| Header: Bearer/Basic in any header | `headers.rs:29-30`, `:167-176`, `:202-213` | `headers.rs:342` |
| Header: bare CR line | `headers.rs:101-109` | `headers.rs:384` |
| Header: folded line after a masked header | `headers.rs:116-124` | none today; add one (below) |
| Header: URL userinfo | `headers.rs:219-242` | `headers.rs:364` |
| Header: query values | `headers.rs:39-43`, `:247-266` | `headers.rs:314`, `:326`, `:354`, `:373`; `m2_proxy_integration.rs:720` |
| Metadata: proxy allowlist, query dropped | `metadata.rs:81-106` | `metadata.rs:261` |
| Metadata: daemon sanitize | `stream.rs:2568-2607` | `capture_redaction.rs:2162`; `capture_redaction_extra.rs:1087`; `capture_redaction_run_secrets.rs:364` |
| Harness env cleared and rebuilt | `launch.rs:371-407`, `1440-1474` | not checked in this lane |
| `.env*`, `.git-credentials` never read | `workspace.rs:415-417` | not checked in this lane |
| `Debug` hides tokens | `upload/http.rs:106`, `daemon/mod.rs:190-199`, `account/credential.rs:73`, `account/github.rs:62`, `config.rs:346-359`, `proxy/connect.rs:109` | not checked in this lane |

### Rules to add

| Rule | Where | Lines | Tests |
|---|---|---|---|
| `url_password` (option b; must land with or before the email drop) | SHARED literal in `redact.rs` (about 6 lines), SHAPES entry and recognizer in `stream.rs` (about 55), `spikes/redact.py` (1) | +61 (G-code) or about +70 (G-surface) | re-point `capture_redaction.rs:3580`; add percent-encoded, `!` and `:`, `ssh://git@github.com` left alone, email left alone, JSON-escaped URL; kind lists `redaction.rs:144`, `:1324`, `:1488`, `:1616`; must pass `recognizers_hold_every_prefix_of_a_match` (`stream.rs:3103`). About +40 to +80 test lines |
| Test only: folded header line after a masked header | `headers.rs:116-124` | 0 src | one small inline test in `headers.rs` |
| Owner call (beyond option b): env-line rule, UPPER_SNAKE name ending in `KEY`, `SECRET`, `TOKEN`, `PASSWORD` or `PASS`, then `=`, value of 8+ of `[A-Za-z0-9_./+=-]` | `redact.rs`, `stream.rs`, `spikes/redact.py` | about +50 src | about +60 test lines, plus pinned count and kind lists |
| Owner call: `A[KS]IA` in `aws_access_key` | `redact.rs:223` and its twins in `stream.rs` and `spikes/redact.py` | 1 line in each of three places | add an `ASIA` row next to `redaction.rs:165` |
| Owner call: prefix rules `pplx-`, `gsk_`, `tvly-`, `re_`, `vcp_`, `sgamp_` | one `prefixed(...)` in `stream.rs`, one literal in `redact.rs` and `spikes/redact.py` each | about 10 lines each | pinned-count edits plus one row each |
| Optional: header `ratelimit` exemption | `headers.rs:26-27` | 1 line | one inline test |
| Optional: `RLIMIT_CORE=0` in the proxy | proxy start-up | a few lines | none needed |

### Rules to drop

| Rule | Where | Lines | Tests that go or change |
|---|---|---|---|
| `email` (after `url_password`) | `redact.rs:281-288`; `stream.rs` 148-149, 284-293, 386, 540-544, 967-978, 1103, 1538-1576, 1615-1618, 1648, 1687-1694, `prev_local` (1874, 1889, 2017, 2037, 2283); `spikes/redact.py:69` | about -100 src (G-code) or about -75 (G-surface) | delete `redaction.rs:186`; rewrite `redaction.rs:323-351`; edit `redaction.rs:2239`; kind lists `:144`, `:1324`, `:1488`, `:1616`; delete `stream.rs:3145`, `3383`, `3397`; edit `redaction.rs:1519-2100` tables |
| `codex_rate_limits` | `connectors/codex-cli/harness.toml:110` | 1 | delete `redaction.rs:930-953`; its case row in `redaction.rs:812-847` |
| `codex_plan_type` | `harness.toml:111` | 1 | delete `redaction.rs:752-766`; case row; re-point `capture_redaction_extra.rs:1111` |
| `codex_credits_remaining` | `harness.toml:112` | 1 | same test as above; case row |
| `codex_credits` | `harness.toml:113` | 1 | case row; `capture_redaction_extra.rs:42-45` drops four names; regenerate `harness-golden/codex-cli.json:76-91`; edit `stream.rs:3540-3546` (remove `"object"`) |
| Owner call: `object` value form | `schema.rs:969-970`, `:983`; `stream.rs:344-347`, `:410-411`, `:712`, `:1155-1158`, `:1172-1180` | about -25 to -50 | synthetic object cases; the `"object"` entry in `stream.rs:3540-3546` |
| Owner call: Hermes `state.db*` capture row | `connectors/hermes-cli/harness.toml:111-120` | -8 data lines | Hermes goldens (unverified) |
| Owner call, test-cut lane: Python oracle | `spikes/redact.py`; `redaction.rs:1293-2100` | -349 | about -800; `spikes_redact_py_*` |
| Owner call, later (not Friday): exact-value run-secret machinery | `run_secret.rs`; run-secret recognizer in `stream.rs` | about -400 | -379 (`capture_redaction_run_secrets.rs`) |
| Owner call, not option b: Codex id rows and Claude billing row | `codex-cli/harness.toml:114-116`; `claude-code-cli/harness.toml:154-159` | -3 to -9 data lines | `redaction.rs:797`, `:352` |

### Docs to change with the rules (point 6)

- `docs/RULES.md:16`: replace "one known gap" with the G.11 text.
- `docs/RULES.md:294-296`: say the secret scan "will be run at export", until it exists.
- `docs/RULES.md:344`: add that email addresses, plan type and credit balance are recorded.
- `connectors/codex-cli/harness.toml:102-107` and `:107`, `schema.rs:960-963`,
  `docs/CHEAT-CHECKLIST.md:204-205`: fix the stale "no header is recorded" lines.
- `stream.rs:122-124`: update the "128 open streams hold at most 2 MiB" comment if stream limits rise.
