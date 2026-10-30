# PRD-05: Masking

Id: PRD-05. Version: goalcup 1.0. Date: 30 October 2026. Base: main at 4b226de2.
Index: [PRD-00](PRD-00-index.md). Format: [PRD-01](PRD-01-record-format.md) section 8.

## Goal

Every byte the recorder writes has its secrets replaced by `*` of the same length, in the recorder's
own memory, before the byte reaches a queue to disk. Masking never slows the harness and never
alters model data that is not a secret.

## Points served

- **4. Never record secrets.** This PRD leads it.
- **3. Never block or slow the harness.** Masking runs off the relay, has a bounded hold, and never
  returns an error.
- **1. Record every model call.** The left boundary stops masking from altering Codex's encrypted
  reasoning and from failing closed over the rest of a model request.
- **6. Tell the participant plainly.** This PRD gives PRD-10 and PRD-15 the exact list of what is and
  is not masked.

## Owner decisions it carries out

- Masking option b, final form (BRIEF3, 29 Oct evening): add the URL password rule, drop the email
  rule and the Codex rate-limit rules, add a left word boundary, keep every secret rule. Accept that
  a key typed right after a letter or digit may be missed.
- Masking happens before any byte reaches disk (D14, `docs/DECISIONS.md:121-124`).
- One process (option B): no IPC, no producers. The masker is a library the recorder calls.
- No caps of our own. The hold is a stated memory guard. Past it the stream over-masks and recording
  goes on; nothing is dropped and nothing is refused.
- No backward compatibility: the `<redacted:KIND>` marker goes; PRD-01 fixes `*` of the same length.
- Proven tricky parts are copied, not rewritten: the rules, the streaming hold and their test corpus
  (BRIEF3, 29 Oct night).
- Record raw, interpret later: the masker matches bytes against rules; it parses nothing else.

## Today

| What | Where | Lines | Fact |
|---|---|---:|---|
| Shared rules, whole-record redactor, selection, PEM pieces, marker | `crates/goalcup-daemon/src/redact.rs` | 1,208 src (1-1208), 71 tests | 23 literals, 19 kinds (`redact.rs:195-311`). Marker `<redacted:KIND>` changes length (`:1199-1207`); a capture-group span gets a quoted marker (`:328-351`, `:756`) |
| Streaming redactor | `crates/goalcup-daemon/src/redact/stream.rs` | 2,945 src (1-2945), 1,063 tests | One hand-written recognizer per literal (`SHAPES`, `:450-629`), hold of at most 16 KiB (`:125`), 64 live candidates (`:133`), fail closed to the next `\n` (`:68-86`, `:2220-2266`). Frames, segments, base64 and envelopes in `:2363-2945` |
| Recognizers are pinned to their regex | `stream.rs:754-770` (`policy_ready`) | | A shape that differs from its literal refuses to stream. Today that loses every producer at run time (`runtime/capture/owner.rs:298-303`) |
| Per-unit cost | `stream.rs:2022-2039`, `:2000-2011` | | Every decoded unit is offered to every rule (`for lit in 0..policy.len()`) |
| Regex passes | `stream.rs:2144-2146`, `:2209`; `redact.rs:706-772` | | Every released byte is scanned by every literal's regex; held bytes again when a match straddles the cut |
| Header masking in the proxy | `crates/goalcup-cli/src/proxy/headers.rs` | 267 src, 136 tests | Names with `key`, `token`, `secret`, `password`, `auth`, plus `cookie`, `set-cookie` (`:22-36`); `Bearer`, `Basic` in any value (`:201-214`); URL userinfo and credential-named query values (`:216-266`); bare CR line (`:104-112`); value replaced by `[masked]` (`:20`) |
| Query dropped from stream metadata | `crates/goalcup-cli/src/proxy/metadata.rs:81-106` | 26 | Path without query; the daemon drops it again (`stream.rs:2568-2607`) |
| Run's own exact secrets | `crates/goalcup-protocol/src/run_secret.rs` | 376 src, 154 tests | Proxy credential and the GitHub token from the project's key file (D71; `crates/goalcup-cli/src/commands/start/project.rs:330-346`). 11 encodings, line breaks allowed between characters. At most 2 values (`run_secret.rs:44-47`) |
| Connector rules | `crates/goalcup-protocol/src/harness/schema.rs:905-1112` | about 208 | `json_field` with `object`, `string` or `scalar`, or `header_prefix`. Rows: `connectors/amp-cli/harness.toml:100-103`, `connectors/claude-code-cli/harness.toml:156-159`, `connectors/codex-cli/harness.toml:108-117` |
| Python twin of the rules | `spikes/redact.py` | 349 | Oracle for about 800 test lines (`tests/it/redaction.rs:1305-2100`) |
| Integration tests | `crates/goalcup-daemon/tests/it/redaction.rs` 2,629, `capture_redaction.rs` 4,745, `capture_redaction_extra.rs` 1,113, `capture_redaction_run_secrets.rs` 379, `tests/common/support/redaction.rs` 321 | 9,187 tests | 106 test functions plus property shards |
| Real corpus | `spikes/*/fixtures/*.jsonl` | 113 files, 7.3 MB | Used by `the_shared_set_finds_nothing_new_in_the_committed_m0_fixtures` (`redaction.rs:615`) and `a_long_codex_reasoning_item_streams_through_intact` (`capture_redaction.rs:3430`) |

Total today: about 5,030 src lines (daemon 4,153, proxy 293, protocol about 584) and about 10,600 test
lines (inline plus integration), counted with `wc -l` and the line ranges above.

### What run 1 showed (server copy)

From the run report's server-copy review (scratchpad copy `run-report-74daaa03.md`, section "2.
Secrets"; appendix M section 9.2):

- **No secret leaked.** 0 hits for the run's OpenAI key and GitHub token in the whole 1.3 GB, segments
  joined per stream. All 378 `authorization` headers were masked.
- **Masking altered model data.** The record holds 634 `openai_key`, 777 `huggingface_token` and 1
  `github_token` spans. In the raw rollouts the same rules match 103 times; 102 of them sit in the middle
  of Codex's `encrypted_content` reasoning blobs, and the one `github_token` span is inside such a blob
  in a `response.completed` server message. None is a credential. The rules have no left boundary.
- **The real risk is F-07.** `openai_key` and `google_key` take the whole base64url alphabet, so a match
  inside a blob runs to the blob's end. Past 16 KiB the stream fails closed and masks to the next `\n`.
  A WebSocket client message is one JSON line, so one such hit can mask the rest of a model request
  (`docs/CHEAT-CHECKLIST.md:178-181`). Whether run 1 hit it is unverified (spans were counted, not
  measured).

## Design

### 1. Where masking runs

```
proxy relay ---> offer (raw, decoded) --\
native reader -> offer (raw) ------------>  per-stream queue  -> mask worker (StreamMasker) -> writer -> part file
                                           (memory only)        OPEN meta: mask_meta
```

- Raw bytes exist only in the recorder's memory: the proxy's buffers, the queues and the masker's hold.
  Nothing raw is written, logged, put in a gap tail, a note, a count or `status.json`.
- PRD-02's mask workers call the masker (seam S2). The relay never waits on it. If masking falls
  behind, PRD-02's memory guard turns the excess into a gap: slow masking costs data, never the
  harness's speed.
- The masker is pure: no I/O, no threads, no clock, no panics by design (no `unwrap` in `src/mask/`).
  PRD-02 still wraps each call in `catch_unwind` and writes `mask_failed` on a panic.
- Every recorded stream kind (`http_req`, `http_resp`, `ws_up`, `ws_down`, `native`, `snapshot`) gets
  the same rules. `conn` streams carry no DATA; their meta goes through `mask_meta`.

### 2. The rule list

The shared list keeps today's order, so priorities and the two fixed indexes (`stream.rs:151-155`,
JSON literal 10, shell literal 11) do not move: `url_password` takes the slot `email` leaves.

| # | Literal kind | Regex (as today unless marked) | Left guard | Change |
|---:|---|---|---|---|
| 0 | `anthropic_key` | `sk-ant-[A-Za-z0-9_\-]{20,}` (`redact.rs:197`) | yes | guard |
| 1 | `openai_key` | `sk-(?:proj-)?[A-Za-z0-9_\-]{20,}` (`:198`) | yes | guard |
| 2 | `masked_key` (soft) | `(?i)sk-[A-Za-z0-9-]{1,8}(?:\.{3}\|…\|\\u2026\|\*{3,64})[A-Za-z0-9]{3,4}` (`:213-216`) | no (soft; `:209-212`) | none |
| 3 | `github_token` | `github_pat_[A-Za-z0-9_]{22,}` (`:221`) | yes | guard |
| 4 | `github_token` | `gh[pousr]_[A-Za-z0-9]{20,}` (`:222`) | yes | guard |
| 5 | `aws_access_key` | `AKIA[0-9A-Z]{16}` (`:223`) | yes | guard |
| 6 | `bearer` | `(?i)bearer\s+[A-Za-z0-9._\-]{16,}` (`:224`) | no | none |
| 7 | `jwt` | `eyJ…\.…\.…` (`:225-228`) | no | none |
| 8 | `private_key` (block) | PEM, body bounded to 15,872 bytes (`:243-246`) | no | none |
| 9 | `slack_token` | `xox[baprs]-[A-Za-z0-9\-]{10,}` (`:247`) | yes | guard |
| 10 | `generic_secret_assignment` | JSON form, value in group 1 (`:260-263`) | no | none |
| 11 | `generic_secret_assignment` | shell, TOML and form-encoded (`:276-279`) | no | none |
| 12 | `url_password` | **new**, section 5 | no | replaces `email` (`:284-287`) |
| 13 | `google_key` | `AIza[A-Za-z0-9_\-]{30,}` (`:289`) | yes | guard |
| 14 | `huggingface_token` | `hf_[A-Za-z0-9]{20,}` (`:290`) | yes | guard |
| 15 | `npm_token` | `npm_[A-Za-z0-9]{20,}` (`:291`) | yes | guard |
| 16 | `xai_key` | `xai-[A-Za-z0-9]{20,}` (`:292`) | yes | guard |
| 17 | `stripe_key` | `[sr]k_live_[A-Za-z0-9]{16,}` (`:293`) | yes | guard |
| 18 | `gitlab_token` | `glpat-[A-Za-z0-9_\-]{20,}` (`:294`) | yes | guard |
| 19 | `digitalocean_token` | `dop_v1_[a-f0-9]{32,}` (`:295`) | yes | guard |

Then the connector rules (PRD-11 data), then the exact values (section 9):

| Kind | Form | Source | Change |
|---|---|---|---|
| `amp_ws_token` | `json_field = "wsToken"`, `string` | `connectors/amp-cli/harness.toml:102` | none |
| `anthropic_billing_header` | `header_prefix = "x-anthropic-billing-"` | `connectors/claude-code-cli/harness.toml:158` | none (owner may revisit) |
| `codex_installation_id`, `codex_safety_identifier`, `codex_turn_state` | `json_field`, `scalar` | `connectors/codex-cli/harness.toml:114-116` | none (owner may revisit) |
| `codex_rate_limits`, `codex_plan_type`, `codex_credits_remaining`, `codex_credits` | | `connectors/codex-cli/harness.toml:110-113` | **dropped** (PRD-11 deletes the rows) |
| `run_secret` | exact values, 11 encodings, line breaks allowed | `redact.rs:374-477`, `run_secret.rs` | kept; sources widened (section 9) |

Removed with the thing it guarded, not dropped as a secret rule: the three `proxy_credential`
literals (`redact.rs:296-310`, shapes `stream.rs:601-628`). PRD-03 design point 5 removes the proxy
credential, so there is no `goalcup:<hex>` value left to find. If PRD-03 keeps a credential, the three
literals stay, unchanged.

`mask_rules()` returns one name per kind, in first-literal order: 18 shared kinds, then connector
kinds, then `run_secret`. A MASK record's `rule` is the position of its kind in that list (PRD-01
section 8). The start event carries the list.

### 3. Same length, and the record stays parseable

- Every masked byte becomes `*` (0x2A). Offsets stay equal to source offsets (PRD-01 section 8).
- **A span never holds an unescaped `"`.** Before filling, the masker cuts each claim at every quote
  that an even run of backslashes precedes, and keeps the quote. So a JSON string boundary never moves
  and a JSON line stays a JSON line. A `scalar` connector rule over `"abc"` masks `abc` and keeps both
  quotes. This replaces today's quoted-marker logic (`redact.rs:328-351`, `:599-608`, `:756`).
- **A span never splits an escape.** A span whose first byte follows an odd run of backslashes starts
  one byte earlier (the backslash is masked too). A span whose last byte is an escaping backslash ends
  one byte later. So `\n`, `\"` and `\\` are masked whole or not at all.
- The PEM block keeps today's string-interior pieces (`redact.rs:915-964`) and the 8 KiB cut
  (`:774-779`), so a key streamed as many SSE string deltas loses every delta's text and keeps the JSON
  between them.
- A span over a multi-byte character covers all its bytes (today's `char_end`, `redact.rs:986-1006`), so
  the result is valid UTF-8.
- One MASK record per masked run inside one returned piece (section 6): spans do not cross the bytes
  they are returned with. A long run across two pieces is two MASK records.
- Known limit: a `scalar` rule over a bare number turns it into `****`, which is not JSON. No shipped
  row does that once the Codex credit rows are gone.

### 4. The left guard

**Rule.** A match of a guarded literal (13 rules, table above) is dropped when the byte just before it
is one of `[A-Za-z0-9_-]`, unless that byte ends one of these escapes:

| Escape | Before the match | Example that is still masked |
|---|---|---|
| A backslash escape | an odd run of backslashes, then one letter or digit | `\nsk-…`, `\tghp_…`, `\\\nAKIA…` |
| `\uXXXX`, `\xHH` | odd backslashes, `u` and 4 hex, or `x` and 2 hex | `"sk-…` |
| Percent escape | `%` and 2 hex | `%3Dsk-…`, `%20ghp_…` |
| ANSI colour code | ESC (raw 0x1B, `\u001b`, `\x1b`, `\033` or `\e`), `[`, up to 16 of `[0-9;]`, then a letter | `\u001b[32msk-…` |

At the start of a stream, after a gap, and at a native resume, nothing is before the match: it is kept.

**Why `[A-Za-z0-9_-]` and not only letters and digits.** Codex's `encrypted_content` is base64url,
whose alphabet is exactly `[A-Za-z0-9_-]` (plus `=` padding). With `_` and `-` in the guard, no
guarded prefix can match inside such a blob except at its first byte, where it follows the quote.
Without them, about 2 in 64 of the in-blob hits would remain. Standard base64 (`+`, `/`) cannot hold
`-` or `_`, so only `AKIA` and `AIza` can occur there, and 62 of 64 left neighbours are glue.

**Never a plain `\b`.** It misses keys after `\n` in JSON text and after ANSI codes (appendix M 9.2).

**One predicate, two callers.** `glued(before: &[u8]) -> bool` reads at most the last 24 bytes before
a match. The whole-record matcher calls it with the bytes before each guarded match (a post-filter in
`candidates`, `redact.rs:706-772`; a dropped match ends before any byte that could start a real one,
because no guarded class holds `\`, `%` or ESC). The stream calls it at candidate spawn with a 24-byte
ring of the raw bytes before the unit (`Candidate::spawn` takes it where `prev_local` sits today,
`stream.rs:1603`). The ring holds raw bytes in memory only; it is cleared at `gap()`.

**What it costs.** A key glued after a letter, digit, `_` or `-` is not masked: `xsk-…`, `KEY_sk-…`,
`token-ghp_…`. The owner accepted this. The assignment rules (10, 11) are not guarded, so
`OPENAI_API_KEY=sk-…` is still masked twice over: `=` is not glue, and the shell rule matches the name.

### 5. The URL password rule

```
(?:\\{0,3}/){2}[^\s/?#@:"'<>\\]{0,256}:([^\s/?#@"'<>\\]{1,1024})@
```

- The span is group 1, the password. `postgres://app:hunter2@localhost:5432` becomes
  `postgres://app:*******@localhost:5432`; the scheme, user, `@` and host stay readable. With same-length
  `*` there is no marker to break JSON, so the group form G-code avoided (appendix G, G.1) is safe now.
- It starts at `//`, not at the scheme, so the stream spawns a candidate only on `/` or `\`, never on
  every letter. `(?:\\{0,3}/){2}` also takes JSON-escaped slashes (`https:\/\/u:p@h`), which appendix G
  lists as missed.
- Left alone: `ssh://git@github.com` (no password), `https://host:443/path` (no `@` before `/`),
  `npm:react@18` (no `//`), an email address.
- Over-masked, accepted: `//127.0.0.1:8080@x` masks `8080`; a code comment `//a:b@c` masks `b`.
- Its recognizer is one new `Form` with a small state machine (slashes, user, colon, password, `@`),
  bounded at 3 + 6 + 256 + 1 + 1,024 + 1 units, shaped like today's `Email` (`stream.rs:1538-1576`).
  `secret_from` returns the password start once the colon after the user is seen.

### 6. Streaming: pieces, messages, gaps, restarts, chunks

Copied as is (`stream.rs:1-108`, `1755-2361`), minus frames and segments: for one stream, the masked
bytes and spans equal `mask_whole(whole stream)` however the bytes were split, while no hold reaches
16 KiB. A secret split across two pushes is one secret (`stream.rs:61-66`).

| Boundary | What the masker does |
|---|---|
| Push boundaries (DATA pieces, socket reads, decoder output, file reads) | Invisible. Held bytes carry over |
| WebSocket messages | The masker sees one direction's message payloads back to back (PRD-01 section 4). A match may cross a message boundary, as in the whole stream. Inflated and unmasked before the offer (PRD-03) |
| SSE events, HTTP/2 DATA frames, chunked encoding | Invisible: PRD-03 offers the decoded body bytes |
| A gap on the stream (memory guard, decode failure, source change) | PRD-02 calls `gap()` first. Held bytes that already look like secret material are masked to the cut (today's `finish_cut`, `stream.rs:2310-2344`; `secret_from`, `:1720-1752`); the rest are decided as end of input. The guard ring is cleared |
| END `complete` | `finish()`: held bytes decided as end of input (`finish_complete`, `:2286-2308`) |
| END with any other reason | PRD-02 calls `gap()`, then `finish()` |
| Recorder restart | The hold dies with the process. Wire streams of that boot are cut by the `recorder_down` gap. Native streams resume at the written offset (PRD-02 section 9.3), so held bytes are read again and a secret across the restart is seen whole |
| Part rotation, upload chunks, links | Come after masking. Nothing to do |

**The hold is a stated memory guard, not a cap.** At most 16 KiB of undecided bytes per stream plus a
3-byte UTF-8 tail, and at most 64 live candidates. It guards memory per open stream (16 KiB times open
streams). Past it the stream fails closed: bytes from the earliest undecided candidate are masked until a
`\n` that no live candidate reaches (`stream.rs:68-86`). Recording goes on. The hold holds bytes only
while a candidate is live, so a quiet JSONL file is fully released at each `\n`.

**The returned piece.** `push`, `gap` and `finish` return `Masked { bytes, spans }`. `bytes` is the next
run of released bytes of this stream, contiguous with the previous return, possibly empty, possibly
larger than the push (held bytes released with it). Its length always equals the source bytes it
stands for. Each span is `(start, len, rule)` with `start` relative to `bytes`. PRD-02 adds the stream
offset and times the DATA by the earliest input piece it holds. The masker keeps no times; today's time
marks go (`stream.rs:1902-1925`).

### 7. Metadata: `mask_meta`

`mask_meta(&mut serde_json::Value)` masks a JSON metadata value in place, same length, before it is
written. PRD-02 calls it on OPEN meta, on END JSON tails that carry upstream text (`trailers`), and on
EVENT bodies that carry upstream text (the `note` for an informational response). It applies, in order:

1. **Every string value** goes through the whole-record masker (all rules above, guard included).
   This is today's second line of defence (`stream.rs:2568-2607`).
2. **Header pairs** (`headers`, `trailers`: arrays of `[name, value]`, HTTP/2 pseudo-headers included):
   the whole value becomes `*` when the name, lower-cased, is `cookie` or `set-cookie`, contains `key`,
   `token`, `secret`, `password` or `auth`, or starts with a connector `header_prefix`
   (`headers.rs:22-36`). Exception: a name that contains `ratelimit` or `rate-limit` is not masked by
   the name rule (rule 1 still runs on its value), so rate-limit headers stay readable for research,
   in line with dropping the Codex rate-limit rules. A value that holds a CR or LF is masked whole (the
   bare-CR rule, `headers.rs:104-112`).
3. **In any other header value**: the credential after `Bearer` or `Basic` (`headers.rs:201-214`), and
   in every URL the userinfo and the values of query keys that match the name rule, `sig`, or contain
   `signature`, names percent-decoded first (`headers.rs:38-44`, `216-266`).
4. **`path`** (and `:path`): URL userinfo and credential query values as in 3. The query is kept, masked
   (PRD-01 `http_req` meta), so today's query drop (`metadata.rs:81-106`) goes.
5. `meta.masked` lists the name of every header with any masked byte.

Header names, the method, status, host, port and ALPN are never masked. `mask_meta` never fails: a
value it cannot read as text is masked whole.

### 8. What is dropped, and what that records

| Dropped | Lines today | Now recorded | Point 6 text |
|---|---|---|---|
| `email` | `redact.rs:280-287`; `stream.rs:148-149, 284-293, 386, 540-544, 967-978, 1103, 1538-1576, 1615-1618, 1648, 1687-1694`, `prev_local` (`1874, 1889, 2017, 2037, 2283`) | Email addresses in prompts, git logs, profile responses | "Email addresses are recorded" |
| Codex `rate_limits`, `plan_type`, `credits_remaining`, `credits` rows | `connectors/codex-cli/harness.toml:110-113` | Plan type, quota, credit balance | "Your plan and credit balance are recorded" |
| `object` value form (unused once the two `object` rows go) | `schema.rs:972, 981-987`; `stream.rs:344-347, 410-411, 712, 1155-1158, 1172-1180` | nothing new | none |
| `proxy_credential` x3 | `redact.rs:296-310`; `stream.rs:601-628` | nothing (no credential exists) | none |
| `[masked]` and `<redacted:KIND>` markers, quoted markers | `headers.rs:20`; `redact.rs:328-351, 1199-1207` | lengths of masked values | "A masked value keeps its length" |

### 9. Exact values: the run's own secrets and the workspace `.env`

The `run_secret` rule masks known values by exact bytes in 11 encodings (raw, percent upper and lower,
`\u` lower and upper, base64 standard and URL-safe at three alignments), with a line break allowed
between any two characters (`run_secret.rs:1-31`). It is copied whole. Its inputs change:

| Source | Today | 1.0 |
|---|---|---|
| Proxy credential | yes (`commands/start/phases.rs:385`) | gone with the credential (PRD-03) |
| `GH_TOKEN=` in the project's key file (D71) | yes (`project.rs:330-346`) | yes |
| Other values in the workspace's key files (`.env`, `.env.*`) whose name contains `key`, `token`, `secret`, `password`, `pass`, `pwd`, `auth` or `credential` | no | **yes (default, overridable)** |
| Values of credential-named variables in the environment goalcup builds for the harness (PRD-11's `LaunchPlan`) | no | **yes (default, overridable)** |

Why this default: appendix G finding G.4 calls `cat .env` the biggest real leak. `AWS_SECRET_ACCESS_KEY=`,
`ASIA…` keys and `ENCRYPTION_KEY=` values have no shape a rule can catch, and RULES invites product
keys into `.env` (`docs/RULES.md:196-200`). Masking the participant's own values by exact bytes adds no
heuristic and no false hit on other text. It is the same rule fed more values, so no new rule kind.
The environment source covers a gateway or provider key with an unknown shape (a gateway key the
harness reads from its environment) when the agent runs `env`.

Limits, each a stated guard:

- 20 to 256 bytes of printable ASCII without a space (`run_secret.rs:39-42`, `RunSecret::is_usable`).
  Shorter values match ordinary text too often; they are not masked by value.
- At most 32 values. Each value adds about 11 compiled forms and one candidate check per matching first
  byte. The guard is start-up time and memory, both unmeasured (builder measures, section Acceptance).
  Past 32, the first 32 in file order are used and the start event says how many were left out.
- Values are read once, at `goalcup start` and `continue`, in memory only. They are never written, never
  hashed, never logged. `Debug` prints a count (`redact.rs:407-411`). The start event carries counts
  only: `{"mask_values": {"workspace": n, "env": n, "skipped": n}}` (an ask of PRD-01).
- Today `.env` files are listed by name and size and never read for hashing (`daemon/src/workspace.rs:413-418`).
  This reads values for masking only, as `project.rs:330-346` already does for `GH_TOKEN`.

### 10. Tool output: what is caught

The agent prints files and commands into its tool results; those land in request bodies, WebSocket
messages and session files, JSON-escaped up to three levels (the rules take up to 7 backslashes,
`redact.rs:248-263`).

| The agent prints | 1.0 |
|---|---|
| `cat .env` with `OPENAI_API_KEY=sk-…`, `GITHUB_TOKEN=ghp_…`, `GOOGLE_API_KEY=AIza…` | masked (prefix rules, and the exact value) |
| `AWS_SECRET_ACCESS_KEY=…`, `ASIA…`, `ENCRYPTION_KEY=…` from the workspace `.env` | masked by exact value (section 9). From a file outside the workspace: not masked |
| `DB_PASSWORD=hunter2hunter2` (under 20 bytes, under 16 for the shell rule) | not masked |
| `git remote -v` with `https://user:TOKEN@host`, `postgres://u:PASS@db` | masked (`url_password`, and `github_token` for a GitHub token) |
| `env` | credential-named values masked by exact value; shaped keys by their rules |
| `cat ~/.codex/auth.json` in the isolated home | JWTs (`jwt`) and `"refresh_token": "…"` (JSON rule) masked |
| A PEM private key, printed with line numbers or escaped | masked, body up to 15,872 bytes (`redact.rs:111-125`) |
| `curl -H "Authorization: Basic …"` inside a command line | not masked in a body (no Basic rule there). Masked in a real request header |
| A key the model repeats across SSE or WebSocket deltas | not masked: JSON framing sits between the pieces (`TODO.md:302-312`). PRD-08's scan joins deltas and flags it |
| A key glued after a letter, digit, `_` or `-` | not masked (section 4) |
| A key with no known prefix and no name (`gsk_…`, `tvly-…`, `sgamp_…` bare) | not masked, unless it is a workspace or environment value |
| Hermes `state.db` (binary) | best effort: a value split across an overflow page is missed; masking a WAL frame breaks its checksum (PRD-01 risks, PRD-04) |

### 11. Cost per byte

Today, per decoded unit, every rule's spawn check runs (`stream.rs:2025-2036`), then each live candidate
steps. Every released byte is then scanned by every literal's regex (`stream.rs:2209`). With 1.0's
rules that is 20 shared literals, up to 5 connector rules and up to 352 exact-value forms (32 values
times 11). Three changes keep that cheap:

1. **First-byte dispatch.** `Policy` builds a 256-entry table from each rule's spawn set (its first
   byte, folded for the `(?i)` rules, plus `ſ` and the Kelvin sign for those). `step_unit` offers a unit
   only to the rules in its entry. Most bytes reach two or three rules instead of all.
2. **Exact-value regexes only after a hit.** The exact-value forms run their regex only on a hold in
   which a `Literal` candidate completed. Their recognizer is exact, so this changes no result.
3. **Off the relay.** PRD-02 runs one to four mask workers at low priority; the relay never waits.

Memory: per open stream at most 16 KiB held plus up to 64 candidates (a JSON-key candidate keeps up to
2,008 key bytes, `stream.rs:141-144`). Typical: zero held between lines. Worst case about 150 KiB per
stream while a pathological input keeps candidates live; PRD-02's guard counts the held bytes.

Target, measured on the sandbox (acceptance A5): at least 30 MB/s of masking per worker on run-1-like
input. With two workers on 4 vCPU that is above PRD-02's 50 MB/s pipeline target (PRD-02 R8). Run 1's
heaviest burst was about 30 MB (appendix M section 3), so at the target it clears in about a second.
Today's throughput is unmeasured.

### 12. Participant text (for PRD-10 and PRD-15)

> Goal Cup Proxy hides known secret shapes in memory, before anything is saved: API keys with a known
> prefix, login headers, cookies, passwords in URLs, private keys, login tokens, values named like
> `api_key`, `secret`, `token` or `password`, and the values in your project's `.env` files. A hidden
> value is replaced by stars of the same length. It cannot catch every secret. These can still be
> recorded: a key the model repeats back in its reply, a key written right after a letter or a digit,
> a short password, a value with no known shape that is not in your `.env`, and Hermes's database.
> Your email address and your plan and credit balance are recorded. Raw records stay private and are
> cleaned before research use. Use keys you can revoke, and rotate every key after the run.

## Files it owns

Create (all new, from the copies named in "What is cut"):

| Path | What | Lines (estimate) |
|---|---|---:|
| `crates/goalcup/src/mask/mod.rs` | `Masker`, `StreamMasker`, `Masked`, `MaskSpan`, `MaskError`, `mask_rules()`, `mask_whole()` | 150 |
| `crates/goalcup/src/mask/rules.rs` | Shared literals, compile, `match_view`, `candidates`, `select`, PEM pieces, `glued`, quote cut, escape alignment, `*` fill | 800 |
| `crates/goalcup/src/mask/stream.rs` | Units, predicates, `SHAPES`, `Policy` with dispatch table, recognizers (with `UrlPassword`), `Scanner` | 2,000 |
| `crates/goalcup/src/mask/values.rs` | Exact-value forms and wrapping (from `run_secret.rs`), key-file and environment readers, name gate | 420 |
| `crates/goalcup/src/mask/meta.rs` | `mask_meta`: header names, schemes, URLs, queries, string values | 180 |
| `crates/goalcup/tests/mask_rules.rs`, `mask_stream.rs`, `mask_meta.rs`, `mask_values.rs`, `mask_corpus.rs`, `mask_bench.rs` (ignored by default) | Tests | about 5,000 |
| `crates/goalcup/tests/mask_corpus/` | Real fixtures copied from `spikes/*/fixtures/*.jsonl` except `cursor-cli` and `claude-desktop` (86 files, about 6.2 MB), plus three synthetic files: a planted `.env`, a 1.5 MB Codex-shaped request with `encrypted_content` blobs, a PEM-heavy SSE stream | data |

Source total about 3,550 lines, against about 5,030 today and PRD-00's 3,500 budget. If the copy runs
over 3,800, the builder says why in the night log.

Change: nothing outside `src/mask/` and `tests/mask_*`.

Must not touch: `src/pipeline/` (PRD-02), `src/proxy/` (PRD-03), `src/native/` (PRD-04),
`src/connector/` and `connectors/**` (PRD-11), `src/cli/` (PRD-10), the old crates and `spikes/`
(PRD-13), `docs/**` (PRD-15).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-02 | **S2.** `Masker::new(rules: &[connector::MaskRule], values: Vec<SecretValue>) -> Result<Masker, MaskError>`; `Masker::stream(kind: StreamKind) -> StreamMasker`; `StreamMasker::{push(&[u8]) -> Masked, gap() -> Masked, finish() -> Masked}`; `Masker::mask_meta(&mut Value)`; `Masker::mask_rules() -> Vec<String>`; `Masker::value_counts()`. `Masker` is `Send + Sync` (shared `Arc` policy); `StreamMasker` is `Send`. Spans are relative to `Masked.bytes`. PRD-02 calls `gap()` before `finish()` on any END that is not `complete`, and `mask_meta` on OPEN meta, END tails and upstream-text events | Used there |
| PRD-03 | Offers decoded, de-chunked, inflated, unmasked-WebSocket bytes, never encoded bytes (a secret inside compressed bytes is invisible to rules). Puts raw headers (wire order, pseudo-headers included) and the raw path into meta; joins folded header lines into the value. Copies no masking. If it keeps a proxy credential, the `proxy_credential` literals return | Used here |
| PRD-04 | Native and snapshot streams use the same S2 path. Resume at the written offset (not the read offset), so held bytes are read again | Used here |
| PRD-08 | The secret scan reports, never masks. It uses these rules **without** the guard, plus the pad's planted and real values, joined across DATA records per stream. Invariant 5 (every MASK covers bytes that are all `*`) holds because quotes are kept outside spans | Used there |
| PRD-09 | Masks never touch a byte no rule matched; usage fields stay readable. Rate-limit headers stay readable (section 7). Codex `rate_limits` bodies are no longer masked | Used there |
| PRD-10 | Calls `mask::workspace_values(project)` and `mask::env_values(&launch_plan.env)` at `start` and `continue`, passes them to `Masker::new`. A `MaskError` fails `start` before the harness launches, with fixed text. Shows the section 12 text | Used there |
| PRD-11 | Owns the `MaskRule` type in S3: `{kind, JsonField {field, value: String or Scalar} or HeaderPrefix(prefix)}`, same grammar as `schema.rs:995-1040`; `value = "object"` is refused. Deletes `connectors/codex-cli/harness.toml:110-113` and fixes the stale comment at `:102-107` ("no header is ever recorded"). Exposes `LaunchPlan.env` | Both |
| PRD-12 | The pad plants a fake `.env` (one value per shape in section 10, plus a 40-byte AWS-style secret with no shape) and a prompt that has the agent print it, run `git remote -v` on a URL with a password, and run `env`. Real keys stay in 0600 files and are scanned for by value | Used there |
| PRD-13 | Deletes the old code and `spikes/` only after `tests/mask_corpus/` holds the copied fixtures and `mask_*` tests pass | Used there |
| PRD-14 | `mask_bench.rs` is `#[ignore]` and runs in `gate full` in release mode | Used there |
| PRD-15 | Section 12 text into README and the participant guide; `docs/RULES.md:16` ("one known gap") replaced; a DECISIONS entry: same-length `*` masking, the left guard, exact `.env` values | Used there |
| PRD-17 | Lessons that are masking acceptance cases: form-encoded refresh tokens (`redaction.rs:541`), a key after `\n` in JSON text, a PEM key printed by the Read tool (`capture_redaction.rs:3637`), Codex reasoning blobs intact (`:3430`) | Used there |

## What is cut

| Path | Lines today | 1.0 | Tests that go |
|---|---:|---|---|
| `crates/goalcup-daemon/src/redact.rs` | 1,208 src, 71 tests | About 750 copied into `rules.rs` (literals, `match_view`, `candidates`, `select`, PEM pieces). Cut: `Redactor` struct API (the oracle becomes `mask_whole`), marker and quoted marker (`:328-351`, `:599-608`, `:1199-1207`), `email` (`:280-287`), `proxy_credential` (`:296-310`), `RunSecrets` (moves to `values.rs`) | Inline test kept (it tests `select`) |
| `crates/goalcup-daemon/src/redact/stream.rs` | 2,945 src, 1,063 tests | About 1,850 copied into `stream.rs`. Cut: frames, segments, base64, envelope budget, `StreamRedactor`, `admit`, `interrupt`, `sanitize` (`:2363-2945`, about 580); time marks (`:1902-1925`); `Email`; `object` form; `proxy_credential` shapes | `a_loss_flushes_decided_bytes_then_records_a_streamless_gap`, `proxy_credential_predicates_equal_their_regex_classes`, the `local_part` body of `held_state_stays_bounded` (`:3383`), the `object` cases of `extra_held_state_stays_bounded`; `"object"` removed from `every_rule_form_streams_with_its_span` (`:3540-3546`) |
| `crates/goalcup-cli/src/proxy/headers.rs` | 267 src, 136 tests | About 120 copied into `meta.rs` (name rule, schemes, URL userinfo, query, percent decode). Head-line parsing (`masked_head`, `:83-150`) goes: PRD-03 parses heads | 9 inline tests rewritten against `mask_meta` (about 100 lines) |
| `crates/goalcup-cli/src/proxy/metadata.rs:81-106` | 26 | Cut (query kept, masked) | `query_and_fragment_are_removed_entirely` (`metadata.rs:261`) |
| `crates/goalcup-protocol/src/run_secret.rs` | 376 src, 154 tests | About 330 copied into `values.rs`. Cut: the control-request wiring (`control.rs:567`) | Inline tests copied |
| `crates/goalcup-protocol/src/harness/schema.rs:905-1112` | about 208 | PRD-11 copies about 150 minus `object` | object-form schema cases |
| `spikes/redact.py` | 349 | Not copied (duplicate of the rules, G.12) | `spikes_redact_py_*` and `all_three_agree` (`redaction.rs:1305-2100`, about 800; `capture_redaction.rs:3708-3770`) |
| `tests/it/redaction.rs` | 2,629 | About 1,000 copied into `mask_rules.rs` | `the_codex_harness_strips_the_credit_balance_it_names` (`:752-766`), `stripping_rate_limits_leaves_the_token_count_record_readable` (`:930-953`), the four Codex case rows (`:812-847`), `the_proxy_credential_is_removed_in_every_form_a_harness_holds` (`:233`), `the_marker_carries_only_the_public_kind` (`:421`); email rows (`:186`, `:323-351` rewritten with another kind, `:2239`) |
| `tests/it/capture_redaction.rs` | 4,745 | About 2,200 copied into `mask_stream.rs`: split-point and chunking properties, `never_less`, fail-closed reproducers, PEM cases, SSE deltas, masked-key bounds, long Codex reasoning | Frame and envelope tests: `unknown_stream_rejected`, `ended_stream_rejects_bytes`, `stream_limit_bounds_memory`, `many_short_matches_fit_envelope_budget`, `envelope_fits`, `merged_observations_keep_time_interval`, `recovery_discards_old_secret_suffix`, `sparse_tick_flushes_safe_prefix`, `interrupt_never_flushes_secret_prefix`, `connection_controls_never_create_byte_parts`, `recovery_cannot_clear_previous_gap`, `gap_frames_keep_source_coordinates`, `flush_takes_a_valid_tick_and_consumes_no_sequence`, `a_proxy_credential_split_at_any_byte_is_never_persisted`, `metadata_is_redacted_without_retaining_query` (about 1,700) |
| `tests/it/capture_redaction_extra.rs` | 1,113 | About 600 into `mask_stream.rs` | Four Codex kinds from `EXTRA_KINDS` (`:42-45`); `:1111` re-pointed at `codex_turn_state` |
| `tests/it/capture_redaction_run_secrets.rs` | 379 | About 300 into `mask_values.rs` | Proxy-credential cases |
| `tests/common/support/redaction.rs` | 321 | About 200 into the shared test helpers | Frame builders |

Totals (estimates): source about 5,030 to about 3,550 (minus about 1,480). Tests about 10,600 to about
5,000 (minus about 5,600), most of it the Python oracle and the frame protocol.

## Acceptance

Real-run proofs first. Each runs on the pad (PRD-12) and is checked with PRD-08's tools on the local
parts and on the server copy.

| # | Proof | Pass |
|---|---|---|
| A1 | **No secret, three cases.** Claude Code, Codex and Hermes, each direct key, gateway and dodger, with the planted `.env` and prompt (PRD-12) | The secret scan finds 0 planted values and 0 real keys in local parts and server copy: raw, base64, percent-encoded, joined across the DATA records of each stream. Every planted value with a shape in section 10 has a MASK record |
| A2 | **Codex blobs intact.** The Codex direct-key run with one subagent | 0 MASK records inside an `encrypted_content` JSON string. Every MASK record's previous byte passes `glued() == false`. MASK counts per rule are reported; any MASK that is not a planted value is named in the night log |
| A3 | **Headers masked.** Same runs | Every `authorization`, `x-api-key`, `x-goog-api-key`, `cookie` and gateway key header value in meta is all `*` and listed in `meta.masked`. Rate-limit headers are readable |
| A4 | **The record still parses.** Same runs | Every `ws_up` text message and every `application/json` body that ended `complete` with no gap parses as JSON after masking, and every JSONL line of every native stream parses |
| A5 | **Cost.** `mask_bench` on the sandbox, release build, one thread: the planted 1.5 MB Codex request pushed in 16 KiB pieces, and a Claude Code transcript from the corpus; also start-up with 32 values of 256 bytes | At least 30 MB/s per worker. `Masker::new` under 1 s and under 64 MiB with 32 values. Below either, the night log names the stage; PRD-02 R8 decides |
| A6 | **Early test (phase 1a).** The first Claude Code and Codex direct-key runs | A1 and A2 pass before the full build ends |

Automated tests that guard them (few, each named for its promise):

- `mask_stream.rs`: for every split point and seeded random chunking of the corpus, the stream's bytes
  and spans equal `mask_whole` (copied); a secret is never released raw at a gap or past the hold
  (copied `never_less` and fail-closed reproducers); output length always equals input length.
- `mask_rules.rs`: the rule table and `mask_rules()` order pinned; `url_password` cases from section 5;
  guard cases from section 4 (each allowed left context masks, each glue byte does not); a 20 KiB
  base64url run with `sk-`, `hf_`, `ghp_`, `AIza` inside comes through byte for byte; an email and a
  Codex `rate_limits` object come through byte for byte; no span holds an unescaped quote or splits an
  escape; `Masker::new` refuses a shape that differs from its literal (today's `policy_ready` becomes an
  error at start, not a silent run-time loss).
- `mask_meta.rs`: the header, scheme, URL and query table, rewritten from `headers.rs:276-402`.
- `mask_values.rs`: each workspace value is masked in all 11 forms, wrapped; a value under 20 bytes,
  with a space, or under a non-credential name is not used; nothing value-derived appears in `Debug`.
- `mask_corpus.rs`: over the copied fixtures, new spans only where listed (today's known-catches rule,
  `redaction.rs:615-720`), and the Codex rollout with `encrypted_content` passes byte for byte.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Guard misses glued keys | `xsk-…`, `KEY_sk-…` recorded raw | Owner accepted. The scan (PRD-08) runs without the guard and flags them |
| Model echo | A key split across deltas is stored raw | Unchanged from today. Scan flags; participant told to rotate |
| `.env` read only at start | Keys the agent adds later, short passwords, keys outside the workspace or in the harness's own config files are not masked by value | Participant text says so. `continue` reads the files again |
| Exact values over-mask | A credential-named variable whose value also appears as ordinary text is masked there too | Name gate and 20-byte floor; accepted (it is the participant's own secret) |
| Copy is large for one night | 2,000 lines of recognizers moved under time pressure | Copied, not rewritten; the property tests come with it and must stay green before PRD-13 deletes the old code |
| Throughput unknown | Masking below the inflow turns into memory-guard gaps (point 1) | A5 measures in phase 1a; dispatch table first; PRD-02 scales workers |
| Held bytes reach disk late | A live candidate at a quiet stream's end waits for more bytes; a lone PEM `BEGIN` holds up to 15,872 bytes | Wire bytes lost only inside a `recorder_down` gap; native bytes are read again |
| Length is visible | A masked value shows its length | Accepted in PRD-01 |
| A real `*` run looks masked | Text that already has stars | MASK records say what was masked; readers use them |
| Privacy | Emails, plan type and credit balance are recorded | Not secrets. Participant told; stripped at export (D26) |
| Binary SQLite | Split values missed; masked WAL frames ignored by SQLite | PRD-04's call; the wire is the primary record |
| Bare scalar | A `scalar` rule over a number breaks that JSON value | No shipped row does it |
| Removing `proxy_credential` | A stale build that still has a credential | PRD-03 removes the credential in the same build |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Guard class | `[A-Za-z0-9_-]` (owner said letter or digit; `_` and `-` cover base64url) | Owner |
| Guarded rules | The 13 vendor-prefix literals | Owner |
| `url_password` bounds | user 256, password 1,024, escaped slashes allowed | PRD-05 |
| Hold | 16 KiB per stream, 64 candidates | PRD-05 after A5 |
| Exact-value sources | `GH_TOKEN`, credential-named workspace key-file values, credential-named launch environment values | Owner (beyond "nothing more"; reason in section 9) |
| Exact-value limits | 20 to 256 bytes, at most 32 values | PRD-05 after A5 |
| Rate-limit header exemption | names with `ratelimit` or `rate-limit` | Owner |
| `object` form | cut | Owner |
| Kept as is | `anthropic_billing_header`, the three Codex id rows | Owner may revisit |
| Not added | `ASIA` beside `AKIA`, an env-line rule, prefixes `pplx-`, `gsk_`, `tvly-`, `re_`, `vcp_`, `sgamp_` (appendix G, G.4 and G.5) | Owner |

## Builder notes

Order of work:

1. Copy `redact.rs` into `rules.rs`; replace the marker with `*` fill, the quote cut and escape
   alignment; keep `mask_whole` as the oracle. Copy the rule tests. Publish the S2 stub (pass-through)
   in phase 0 so PRD-02 can build.
2. Copy `stream.rs` lines 1-2361 into `stream.rs`; replace `Piece` with plain byte runs and spans; drop
   time marks. Copy the split-point properties and get them green before any rule change.
3. Remove `email`, `object` and `proxy_credential` (shapes and literals together; the indexes 10 and 11
   must not move). Add `url_password` in slot 12.
4. Add `glued()` and the guard in both callers; add the dispatch table.
5. `values.rs`, then `meta.rs`, then the S2 wrapper and `mask_bench`.

Traps from the audit:

- A literal and its shape must change in one commit. A mismatch is now a start error; it must also be a
  failing test (`shapes_follow_the_shared_list`, `stream.rs` test 4).
- `JSON_LITERAL` and `SHELL_LITERAL` are fixed indexes (`stream.rs:151-155`). Removing `email` shifts
  nothing only because `url_password` takes slot 12.
- `stream.rs:3540-3546` demands that some shipped rule uses the `object` form. Remove `"object"` there.
- `prev_local` exists only for email (`stream.rs:1603`, `:1874`). Reuse its slot for the guard ring.
- `PEM_BODY_MAX + 512 <= MAX_HELD_BYTES` (`stream.rs:904`) and the wrapped-form assert (`:129`) must hold.
- Never use `\b` for the guard. Never test the guard on masked output: it reads raw bytes.
- The copied fixtures carry `<redacted:…>` markers from the old script. They are data; leave them.
- Never log a value: keep the `Debug` wrappers (`redact.rs:88-98`, `:407-411`) and `wipe` on drop.

How a watcher proves it in a real run: after each pad run, run PRD-08's secret scan on the entry folder
and on the fetched server copy (A1); run `goalcup-verify dump`, take every MASK record with the 24 bytes
of its stream before it, and fail on any span with a glue byte before it or inside an
`encrypted_content` string (A2); parse every complete JSON body and message (A4). Report MASK counts
per rule in the night log.

## Asks of PRD-00 and PRD-01

1. **PRD-00, seam S2.** Record the constructor and the return shape: `Masker::new(rules, values) ->
   Result<Masker, MaskError>`, `StreamMasker::gap()` (PRD-02 asked too), `Masked { bytes, spans }` with
   spans relative to `bytes`, and `mask_rules()` as one name per kind.
2. **PRD-00, seam S3.** List `connector::MaskRule` (string and scalar fields, header prefix; no object)
   and `LaunchPlan.env` as things PRD-05 reads.
3. **PRD-00, PRD table.** The PRD-05 line should add "exact `.env` values" and "proxy-credential rule
   removed with the credential", so readers do not take either as a surprise.
4. **PRD-01 section 8.** Say that `rule` numbers index a list of kinds (two literals of one kind share a
   number), that a masked run may be several MASK records across DATA records, and that an unescaped
   `"` is never inside a MASK span (it keeps invariant 5 true for JSON).
5. **PRD-01 section 8.** Say that `meta.masked` lists every header with any masked byte, not only those
   masked by name.
6. **PRD-01 section 11, start event.** Add `mask_values` with counts only: `{"workspace": n, "env": n,
   "skipped": n}`. Never a value, a hash or a length of a value.
7. **PRD-01 section 3, GAP reasons.** PRD-02 asks for `mask_failed` as 12 and PRD-03 asks for
   `upstream_untrusted` as 12. Please give them different numbers.
