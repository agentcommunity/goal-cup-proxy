# PRD-05: Masking

Id: PRD-05. Version: goalcup 1.0. Revision 2, 30 October 2026 (revision 1: earlier the same day).
Base: main at 4b226de2. Index: [PRD-00](PRD-00-index.md) revision 2. Format:
[PRD-01](PRD-01-record-format.md) revision 2, section 8 (binding). Revision 2 answers the consistency
check and reviews A and B; the item list is the last section, "Review answers".

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
  goes on; nothing is dropped and nothing is refused. Over-masking is never silent: it has its own rule
  name, `fail_closed`, and its own count (PRD-01 section 8).
- Owner question 12 (PRD-00), default yes, built in the second slice: exact values from the workspace
  `.env` files, the launch environment and the harness's own key files (section 9), and the Stripe rule
  widened to test keys plus a webhook-secret rule (section 2). If the owner says no, both are left out
  and nothing else changes.
- One system for every harness and every pad case (direct key, gateway, dodger): one masker, one rule
  list. The only per-harness inputs are PRD-11 data (connector rows and `secret_files`).
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
  Nothing raw is written, logged, put in a gap tail, a note, a count or `status.json`. Memory reaching
  disk through a crash is PRD-02's rule of the shape: `PR_SET_DUMPABLE` 0, core size 0, and a panic
  hook that prints only file and line (review A finding 5).
- PRD-02's one mask worker calls the masker (seam S2). The relay never waits on it. If masking falls
  behind, PRD-02's memory guard turns the excess into a gap: slow masking costs data, never the
  harness's speed. A pool of workers comes only if PRD-02 R8 measures masking as the slow stage.
- The masker is pure: no I/O, no threads, no clock, no panics by design (no `unwrap` or `expect` in
  `src/mask/`). No error or panic text in `src/mask/` ever formats a byte, a value or a length of a
  value. PRD-02 still wraps each call in `catch_unwind` and writes GAP `mask_failed` (12) on a panic.
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
| 7 | `jwt` | `eyJ…\.…\.…` (`:225-228`) | yes | guard (section 4) |
| 8 | `private_key` (block) | PEM, body bounded to 15,872 bytes (`:243-246`) | no | none |
| 9 | `slack_token` | `xox[baprs]-[A-Za-z0-9\-]{10,}` (`:247`) | yes | guard |
| 10 | `generic_secret_assignment` | JSON form, value in group 1 (`:260-263`) | no | none |
| 11 | `generic_secret_assignment` | shell, TOML and form-encoded (`:276-279`) | no | none |
| 12 | `url_password` | **new**, section 5 | no | replaces `email` (`:284-287`) |
| 13 | `google_key` | `AIza[A-Za-z0-9_\-]{30,}` (`:289`) | yes | guard |
| 14 | `huggingface_token` | `hf_[A-Za-z0-9]{20,}` (`:290`) | yes | guard |
| 15 | `npm_token` | `npm_[A-Za-z0-9]{20,}` (`:291`) | yes | guard |
| 16 | `xai_key` | `xai-[A-Za-z0-9]{20,}` (`:292`) | yes | guard |
| 17 | `stripe_key` | `[sr]k_live_[A-Za-z0-9]{16,}` (`:293`); with owner question 12: `[sr]k_(?:live\|test)_[A-Za-z0-9]{16,}` | yes | guard; test keys (Q12) |
| 18 | `gitlab_token` | `glpat-[A-Za-z0-9_\-]{20,}` (`:294`) | yes | guard |
| 19 | `digitalocean_token` | `dop_v1_[a-f0-9]{32,}` (`:295`) | yes | guard |
| 20 | `stripe_webhook_secret` | **new with owner question 12**: `whsec_[A-Za-z0-9+/=]{24,}` | yes | added last, so no index moves |

Why the Stripe pair: the pad prompt builds a payments platform (BRIEF3, 30 Oct early), so agents write
and print Stripe test keys and webhook secrets. Today `sk_test_…` has no rule, and
`STRIPE_SECRET_KEY=sk_test_…` is not caught by the shell rule either: its name part must be followed
by `=` or `:` (`redact.rs:276-279`), and `SECRET` is followed by `_KEY` (checked in the code).
`whsec_…` after `WEBHOOK_SECRET=` is caught today; bare in code or JSON it is not.

**The rules as data (seam S12).** The shared literals (20, or 21 with owner question 12) live in one file,
`crates/goalcup/src/mask/shapes.tsv`: one line per literal, `kind`, a tab, `guard` (`1` or `0`), a
tab, the regex; `#` starts a comment. The span is group 1 when the regex has a group, else the whole
match. `rules.rs` reads it with `include_str!` and the verifier (PRD-08) does the same and ignores
the guard column. One source, so the two cannot drift. The PEM string-interior pieces and the soft rule
stay code in `rules.rs`; the verifier needs neither to find a secret.

Then the connector rules (PRD-11 data), then the exact values (section 9):

| Kind | Form | Source | Change |
|---|---|---|---|
| `amp_ws_token` | `json_field = "wsToken"`, `string` | `connectors/amp-cli/harness.toml:102` | none |
| `anthropic_billing_header` | `header_prefix = "x-anthropic-billing-"` | `connectors/claude-code-cli/harness.toml:158` | none (owner may revisit) |
| `codex_installation_id`, `codex_safety_identifier`, `codex_turn_state` | `json_field`, `scalar` | `connectors/codex-cli/harness.toml:114-116` | none (owner may revisit) |
| `codex_rate_limits`, `codex_plan_type`, `codex_credits_remaining`, `codex_credits` | | `connectors/codex-cli/harness.toml:110-113` | **dropped** (PRD-11 deletes the rows) |
| `run_secret` | exact values, 11 encodings, line breaks allowed | `redact.rs:374-477`, `run_secret.rs` | kept; sources widened (section 9) |

Removed with the thing it guarded, not dropped as a secret rule: the three `proxy_credential`
literals (`redact.rs:296-310`, shapes `stream.rs:601-628`). 1.0 has no proxy credential (PRD-00 S8,
PRD-03 design point 5, PRD-11 sets `http://127.0.0.1:<port>`), so there is no `goalcup:<hex>` value
left to find.

`mask_rules()` returns one name per kind, in first-literal order: 19 shared kinds (18 without owner
question 12), then connector kinds, then `run_secret`, then `fail_closed` last. A MASK record's `rule`
is the position of its kind in that list (PRD-01 section 8). `fail_closed` names bytes masked because
the hold overflowed (section 6), never a secret the rules found. The start event carries the list.

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
- The quote cut and escape alignment apply to rule spans. A `fail_closed` span (section 6) is one run
  of `*` per DATA record, quotes included: the hold could not decide where strings start, structural
  bytes are starred anyway, so cutting at quotes would only add MASK records. A body or message with a
  `fail_closed` span is not expected to parse (acceptance A4 counts it apart).

### 4. The left guard

**Rule.** A match of a guarded literal (14 rules, 15 with owner question 12, table above) is dropped
when the byte just before it is one of `[A-Za-z0-9_-]`, unless that byte ends one of these escapes:

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
`-` or `_`, so only `AKIA`, `AIza` and `eyJ` can occur there, and 62 of 64 left neighbours are glue.

**Why `jwt` is guarded too** (new in revision 2, review A finding 3). `eyJ` is three base64url
characters, so it turns up inside a Codex blob about once in 262,144 positions. The `jwt` rule never
completes there (a blob has no `.`), which is why run 1 shows no `jwt` spans. But its candidate stays
live, holding bytes, until the blob ends. A blob tail over 16 KiB then overflows the hold and the
stream fails closed. With the guard the candidate is never spawned inside a blob. A real token follows
a quote, a space, `=`, `:` or `Bearer `, none of which is glue. `bearer`, `masked_key`, the PEM rule and
the assignment rules need a space, a dot run, dashes or a quote that a base64url blob cannot hold, or
are bounded, so they stay unguarded.

**Never a plain `\b`.** It misses keys after `\n` in JSON text and after ANSI codes (appendix M 9.2).

**One predicate, two callers.** `glued(before: &[u8]) -> bool` reads at most the last 24 bytes before
a match. The whole-record matcher calls it with the bytes before each guarded match (a post-filter in
`candidates`, `redact.rs:706-772`; a dropped match ends before any byte that could start a real one,
because no guarded class holds `\`, `%` or ESC). The stream calls it at candidate spawn with a 24-byte
ring of the raw bytes before the unit (`Candidate::spawn` takes it where `prev_local` sits today,
`stream.rs:1603`). The ring holds raw bytes in memory only; it is cleared at `gap()` and `boundary()`.

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
16 KiB. A secret split across two pushes is one secret (`stream.rs:61-66`). For a WebSocket direction
"whole stream" means each message on its own: the output equals `mask_whole` of each message, in order.

| Boundary | What the masker does |
|---|---|
| Push boundaries (DATA pieces, socket reads, decoder output, file reads) | Invisible. Held bytes carry over |
| WebSocket message end (`fin`, review A finding 3) | PRD-02 calls `boundary()` after the message's last piece. Held bytes are decided as end of input (as `finish_complete`), a fail-closed window ends there, the guard ring is cleared, and the next message starts like a new stream. Frames inside one message are invisible (PRD-03 joins them and inflates before the offer). Cost: a secret split across two messages is not one secret; no harness splits a JSON value across messages |
| SSE events, HTTP/2 DATA frames, chunked encoding | Invisible: PRD-03 offers the decoded body bytes |
| A gap on the stream (memory guard, decode failure, source change) | PRD-02 calls `gap()` first. Held bytes that already look like secret material are masked to the cut (today's `finish_cut`, `stream.rs:2310-2344`; `secret_from`, `:1720-1752`); the rest are decided as end of input. The guard ring is cleared |
| END `complete` | `finish()`: held bytes decided as end of input (`finish_complete`, `:2286-2308`) |
| END with any other reason | PRD-02 calls `gap()`, then `finish()` |
| Masker panic | PRD-02 drops the piece, replaces the stream's masker with a fresh one and writes GAP `mask_failed` (12). The fresh masker starts like a new stream |
| Recorder restart | The hold dies with the process. Wire streams of that boot are cut by the `recorder_down` gap. Native streams resume at the written offset (PRD-02 section 9.3), so held bytes are read again and a secret across the restart is seen whole |
| Part rotation, upload chunks, links | Come after masking. Nothing to do |

**The hold is a stated memory guard, not a cap.** At most 16 KiB of undecided bytes per stream plus a
3-byte UTF-8 tail, and at most 64 live candidates. It guards memory per open stream (16 KiB times open
streams). Past it the stream fails closed: bytes from the earliest undecided candidate are masked
(rule `fail_closed`) until the first of: a `\n` that no live candidate reaches (`stream.rs:68-86`), a
WebSocket message end, a gap, or the stream's end. Recording goes on. The hold holds bytes only while
a candidate is live, so a quiet JSONL file is fully released at each `\n`.

**Fail closed is never silent.** Today the copied code runs a fail-closed window "to the stream's
`End` or `Gap`" when no `\n` comes (`stream.rs:76-80`, checked). In 0.7 a WebSocket message was its own
stream; in 1.0 a direction is one stream for the connection's life and Codex's compact JSON has no raw
`\n`, so without `boundary()` one overflow could star out hours of messages while every check stayed
green. Now the window ends at the message, the span is named `fail_closed`, PRD-02's writer adds every
`fail_closed` span's length to COUNT `mask_failclosed_bytes` (PRD-01 section 9), PRD-08 reports it,
and status may show it (an ask of PRD-02 and PRD-10, last section). A1 and A2 report the number for
every run.

**Idle cost per stream.** A `StreamMasker` with nothing held keeps no buffer: the hold is allocated
when a candidate goes live and freed when the hold empties. Its fixed state (counters, the 24-byte
guard ring, an `Arc` of the policy) stays under 256 bytes, so 20,000 open native streams (PRD-04 risk
row) cost about 5 MB. `mask_stream.rs` pins the struct size.

**The returned piece.** `push`, `boundary`, `gap` and `finish` return `Masked { bytes, spans }`. `bytes` is the next
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

### 9. Exact values: the run's own secrets, the workspace `.env` and the harness's key files

Built in the second slice under owner question 12 (PRD-00), default yes. The core slice ships the
`GH_TOKEN` value only, as today.

The `run_secret` rule masks known values by exact bytes in 11 encodings (raw, percent upper and lower,
`\u` lower and upper, base64 standard and URL-safe at three alignments), with a line break allowed
between any two characters (`run_secret.rs:1-31`). It is copied whole. Its inputs change. PRD-05 owns
the one rule for which values are used (PRD-00 "Smaller interface rulings"); PRD-11 only collects
candidates.

| Order | Source | Today | 1.0 | Count key in `mask_values` |
|---:|---|---|---|---|
| | Proxy credential | yes (`commands/start/phases.rs:385`) | gone with the credential | |
| 1 | `GH_TOKEN=` in the project's key file (D71) | yes (`project.rs:330-346`) | yes, always first | `workspace` |
| 2 | Credential-named variables of the environment goalcup builds for the harness (PRD-11 `LaunchPlan.env`; the whole participant environment if owner question 13 stays yes) | no | yes | `env` |
| 3 | Other credential-named values in the workspace key files (`.env`, `.env.*`, `project.rs:330-346` reads them already) | no | yes | `workspace` |
| 4 | The harness's own key files, PRD-11 `secret_files` (`auth.json`, `oauth_creds.json`, `secrets.yaml`, `settings.json` and so on) | no | yes | `harness` |

**One gate for every source.** A value is used when all of these hold:

- its name is credential-named: the variable name, the dotenv name, or the key path in a JSON, TOML or
  YAML file (`tokens.access_token`, `providers.deepseek.apiKey`) contains, ignoring case, `key`,
  `token`, `secret`, `password`, `pass`, `pwd`, `auth` or `credential`;
- it is 20 to 256 bytes of printable ASCII without a space (`run_secret.rs:39-42`); shorter values
  match ordinary text too often;
- it is not a URL (holds `://`) and not a path (starts with `/`, `~` or `.`). A base URL under an
  `auth_url` key would otherwise be starred in every request.

Why the name gate also covers `secret_files` (new in revision 2): PRD-11 collects "every string value of
16 bytes or more" from those files. Harness settings files hold model names, base URLs and paths
beside the keys (`settings.json`, `config.json`, `providers.json`). Masking those by exact value would
star the model name in every request and break PRD-09's usage reader. PRD-11 therefore passes each
candidate with its key path (ask, last section) and PRD-05 applies the gate.

**Order, duplicates and the guard.** Values are taken in the order of the table, duplicates dropped
(the same key in the environment and in `.env` counts once), up to 32 values. Past 32, the rest are
left out and counted in `skipped`, with the values the gate refused. 32 is a stated guard, not a cap of
records: each value adds 11 compiled forms and one candidate check per matching first byte, and the
guard is start-up time and memory, measured by A5. The provider keys come first because the
environment is source 2.

**Never written.** Values are read once, at `goalcup start` and `continue`, in memory only, and reach
the recorder on PRD-02's first control line. They are never written, never hashed, never logged.
`Debug` prints a count (`redact.rs:407-411`). The start event carries counts only:
`{"mask_values": {"workspace": n, "env": n, "harness": n, "skipped": n}}` (PRD-01 section 8). No value
is added during a run, so the `note` `mask_values` that PRD-01 allows is never written by 1.0.

Today `.env` files are listed by name and size and never read for hashing
(`daemon/src/workspace.rs:413-418`). This reads values for masking only, as `project.rs:330-346`
already does for `GH_TOKEN`.

**Why the default is yes.** Appendix G finding G.4 calls `cat .env` the biggest real leak.
`AWS_SECRET_ACCESS_KEY=`, `ASIA…` keys, `ENCRYPTION_KEY=` values and gateway keys with no known prefix
have no shape a rule can catch, and RULES invites product keys into `.env` (`docs/RULES.md:196-200`).
Masking the participant's own values by exact bytes adds no heuristic and no false hit on other text.
It is the same rule fed more values, so no new rule kind.

**Not built: values added during the run** (review A finding 12, partly). A key the participant or
the agent adds to `.env` mid-week is protected by the shape rules only, until the next `continue`. A
live update would need a file watcher in the recorder and a policy swap inside open streams. Shape
rules, including the Stripe pair, cover the pad's payments keys; the participant text says it.

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
| A key with no known prefix and no name (`gsk_…`, `tvly-…`, `sgamp_…` bare) | not masked, unless it is a workspace, environment or harness-file value |
| A gateway key: `AI_GATEWAY_API_KEY=…` in `.env`, OpenRouter `sk-or-v1-…`, a Cloudflare `cf-aig-authorization` header | masked: the shell rule (`API_KEY=`) and the exact value; `sk-or-` by `openai_key`; the header by its name (`auth`). Same path as a direct key: one system |
| `STRIPE_SECRET_KEY=sk_test_…`, `whsec_…` (the payments pad) | masked with owner question 12 (section 2); without it, `sk_test_` only by exact value from `.env` |
| A token used as a URL user with no password (`https://<token>@host`) | masked only if the token has a shape (`ghp_…` after `//` is not glued) or is an exact value. The dropped `email` rule used to catch it (PRD-17 K12) |
| A key the agent adds to `.env` during the run | shape rules only, until the next `continue` (section 9) |
| A key after hours of one WebSocket connection whose hold overflowed | the overflowing message is starred to its end (`fail_closed`, counted); the next message is masked normally |
| Hermes `state.db` (binary) | best effort: a value split across an overflow page is missed; masking a WAL frame breaks its checksum (PRD-01 risks, PRD-04) |

### 11. Cost per byte

Today, per decoded unit, every rule's spawn check runs (`stream.rs:2025-2036`), then each live candidate
steps. Every released byte is then scanned by every literal's regex (`stream.rs:2209`). With 1.0's
rules that is 20 or 21 shared literals, up to 5 connector rules and up to 352 exact-value forms (32
values times 11). What keeps it cheap, in build order (review B finding B2: measure before adding):

1. **Off the relay.** PRD-02's one mask worker runs at low priority; the relay never waits. Built.
2. **Exact-value regexes only after a hit.** The exact-value forms run their regex only on a hold in
   which a `Literal` candidate completed. Their recognizer is exact, so this changes no result. Built
   with the exact values (second slice), since 352 forms on every byte is the one cost known to be large.
3. **First-byte dispatch, only if A5 is short.** `Policy` builds a 256-entry table from each rule's
   spawn set (its first byte, folded for the `(?i)` rules, plus `ſ` and the Kelvin sign for those), and
   `step_unit` offers a unit only to the rules in its entry. Built only when A5 measures the copied
   code under the target, with a night-log line.
4. **The worker pool** is PRD-02's last step, only if R8 still names masking as the slow stage.

Memory: per open stream at most 16 KiB held plus up to 64 candidates (a JSON-key candidate keeps up to
2,008 key bytes, `stream.rs:141-144`). Typical: zero held between lines, under 256 bytes of fixed state
(section 6). Worst case about 150 KiB per stream while a pathological input keeps candidates live;
PRD-02's guard counts the held bytes.

Target, measured on the sandbox (acceptance A5), reported and never a gate failure (PRD-00: no speed
number fails a gate): 50 MB/s of masking on one worker on run-1-like input, because PRD-02 R8 wants 50
MB/s through mask and writer with one worker. Run 1's heaviest burst was about 30 MB (appendix M section
3), so even at 30 MB/s it clears in about a second; below the target a burst costs a memory-guard gap,
not the harness's speed. Today's throughput is unmeasured.

### 12. Participant text (for PRD-10 and PRD-15)

> Goal Cup Proxy hides known secret shapes in memory, before anything is saved: API keys with a known
> prefix, login headers, cookies, passwords in URLs, private keys, login tokens, values named like
> `api_key`, `secret`, `token` or `password`, and the key values in your project's `.env` files and
> your harness's login files when the run starts. A hidden value is replaced by stars of the same
> length. It cannot catch every secret. These can still be recorded: a key the model repeats back in
> its reply, a key written right after a letter or a digit, a short password, a key you add during the
> run, a value with no known shape that is not in those files, and Hermes's database. Your email
> address and your plan and credit balance are recorded. Raw records stay private and are cleaned
> before research use. Use keys you can revoke, and rotate every key after the run.

If the owner answers no to question 12, PRD-10 and PRD-15 drop "and the key values in your project's
`.env` files and your harness's login files when the run starts" and "a key you add during the run",
and say "a value with no known shape".

## Files it owns

Create (all new, from the copies named in "What is cut"):

| Path | What | Lines (estimate) |
|---|---|---:|
| `crates/goalcup/src/mask/mod.rs` | `Masker`, `StreamMasker`, `Masked`, `MaskSpan`, `MaskError`, `ValueCounts`, `mask_rules()`, `mask_whole()` | 160 |
| `crates/goalcup/src/mask/shapes.tsv` | Seam S12: the shared literals as data (kind, guard, regex), read by `rules.rs` and by the verifier with `include_str!` | data, about 30 |
| `crates/goalcup/src/mask/rules.rs` | Parse `shapes.tsv`, compile, `match_view`, `candidates`, `select`, PEM pieces, `glued`, quote cut, escape alignment, `*` fill | 800 |
| `crates/goalcup/src/mask/stream.rs` | Units, predicates, `SHAPES`, `Policy`, recognizers (with `UrlPassword`), `Scanner`, `boundary`, `fail_closed` naming. The dispatch table only if A5 asks for it (about 60 more) | 1,950 |
| `crates/goalcup/src/mask/values.rs` | Exact-value forms and wrapping (from `run_secret.rs`), workspace key-file and environment readers, the one gate, order and dedupe for all four sources | 440 |
| `crates/goalcup/src/mask/meta.rs` | `mask_meta`: header names, schemes, URLs, queries, string values | 180 |
| `crates/goalcup/tests/mask_rules.rs`, `mask_stream.rs`, `mask_meta.rs`, `mask_values.rs`, `mask_corpus.rs`, `mask_bench.rs` (ignored by default) | Tests | about 5,000 |
| `crates/goalcup/tests/mask_corpus/` | Real fixtures copied from `spikes/*/fixtures/*.jsonl` except `cursor-cli` and `claude-desktop` (86 files, about 6.2 MB), plus four synthetic files: a planted `.env`, a 1.5 MB Codex-shaped request with `encrypted_content` blobs (some holding `eyJ`, `sk-`, `hf_`), a PEM-heavy SSE stream, and a harness settings file with keys, a model name and a base URL | data |

Source total about 3,530 lines, against about 5,030 today and PRD-00's 3,550 line for masking. If the
copy runs over 3,800, the builder says why in the night log.

Change: nothing outside `src/mask/` and `tests/mask_*`.

Must not touch: `src/pipeline/` (PRD-02), `src/proxy/` (PRD-03), `src/native/` (PRD-04),
`src/connector/` and `connectors/**` (PRD-11; the Codex rate-limit rows go in PRD-11's first commit),
`src/cli/` (PRD-10), `crates/goalcup-verify/` (PRD-08), the old crates and `spikes/` (PRD-13),
`docs/**` (PRD-15).

## Interfaces with other PRDs

| PRD | What crosses | Direction |
|---|---|---|
| PRD-02 | **S2, as in PRD-00.** `Masker::new(rules: &[connector::MaskRule], values: Vec<SecretValue>) -> Result<Masker, MaskError>`; `Masker::stream(&self, kind) -> StreamMasker`; `mask_meta(&self, &mut Value)`; `mask_rules(&self) -> Vec<String>` (kinds, `fail_closed` last); `value_counts(&self) -> ValueCounts`; `StreamMasker::{push(&[u8]), boundary(), gap(), finish(self)}`, each returning `Masked { bytes, spans }`, spans relative to `bytes`. `Masker` is `Send + Sync` (shared `Arc` policy); `StreamMasker` is `Send`. PRD-02's one mask worker calls `boundary()` after every `fin` piece, `gap()` before any GAP and before `finish()` on an END that is not `complete`, and `mask_meta` on OPEN meta, END tails and upstream-text events. The writer adds the length of every span whose rule is `fail_closed` to COUNT `mask_failclosed_bytes`. `SecretValue` serializes for the first control line and never prints its bytes | Used there |
| PRD-03 | Offers decoded, de-chunked, inflated, unmasked-WebSocket bytes, never encoded bytes (a secret inside compressed bytes is invisible to rules), and marks each message end with `fin`. Puts raw headers (wire order, pseudo-headers included) and the raw path into meta; joins folded header lines into the value. Copies no masking. No proxy credential, so no credential rule | Used here |
| PRD-04 | Native and snapshot streams use the same S2 path. Resume at the written offset (not the read offset), so held bytes are read again. An idle `StreamMasker` costs under 256 bytes (section 6) | Used here |
| PRD-08 | **S12.** Includes `src/mask/shapes.tsv` with `include_str!` and compiles it **without** the guard, plus the pad's planted and real values, joined across DATA records per stream. Invariant 5 (every MASK covers bytes that are all `*`) holds because quotes are kept outside rule spans and a `fail_closed` span stars everything. Only `fail_closed` spans may exceed 16 KiB: PRD-08 reports their bytes and does not fail on their length; any other span over 16 KiB is a fail | Used there |
| PRD-09 | Masks never touch a byte no rule matched; usage fields stay readable. Rate-limit headers stay readable (section 7). Codex `rate_limits` bodies are no longer masked. The exact-value gate skips model names and base URLs in harness files (section 9) | Used there |
| PRD-10 | Calls `mask::workspace_values(project)`, `mask::env_values(&plan.env)` and `mask::harness_values(&plan.secret_values)` at `start` and `continue` and hands them to the watchdog (PRD-02 sends them on the first control line; the recorder calls `Masker::new`, which orders, dedupes and applies the 32-value guard). A `MaskError` fails `start` before the harness launches, with fixed text. Shows the section 12 text. May show `mask_failclosed_bytes` in status when it is not zero | Used there |
| PRD-11 | Owns the `MaskRule` type in S3: `{kind, JsonField {field, value: String or Scalar} or HeaderPrefix(prefix)}`, same grammar as `schema.rs:995-1040`; `value = "object"` is refused. Deletes `connectors/codex-cli/harness.toml:110-113` in its first commit and fixes the stale comment at `:102-107` ("no header is ever recorded"). Exposes `LaunchPlan.env` and `secret_values` as candidates with their key path (`Vec<(String, Vec<u8>)>`); the 16-byte floor in PRD-11's defaults is replaced by PRD-05's gate | Both |
| PRD-12 | The pad plants a fake `.env` (one value per shape in section 10, plus a 40-byte AWS-style secret with no shape, plus, with owner question 12, a `sk_test_` and a `whsec_` value) and a prompt that has the agent print it, run `git remote -v` on a URL with a password, and run `env`. The gateway case keeps its key in `.env` (`AI_GATEWAY_API_KEY`). Real keys stay in 0600 files and are scanned for by value. Optional, review A finding 12: write a fresh `sk_test_` canary to `.env` at minute 5 and expect no hit (it tests the shape rule, not a mid-run reader) | Used there |
| PRD-13 | Deletes the old code and `spikes/` only after `tests/mask_corpus/` holds the copied fixtures and `mask_*` tests pass | Used there |
| PRD-14 | `mask_bench.rs` is `#[ignore]` and runs in `gate full` in release mode. It reports MB/s and start-up time; it has no floor and never fails the gate (review B T3, PRD-00) | Used there |
| PRD-15 | Section 12 text into README and the participant guide; `docs/RULES.md:16` ("one known gap") replaced; a DECISIONS entry: same-length `*` masking, the left guard (with `jwt`), `fail_closed` named and counted, exact values from four sources | Used there |
| PRD-17 | Lessons that are masking acceptance cases: form-encoded refresh tokens (`redaction.rs:541`), a key after `\n` in JSON text, a PEM key printed by the Read tool (`capture_redaction.rs:3637`), Codex reasoning blobs intact (`:3430`), K12 (token as URL user with no password: a stated loss), and review A finding 3's case: a 20 KiB undecided candidate in message 1 of a `ws_up` stream, then 10 plain messages; messages 2 to 11 come through unmasked | Used there |

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

Totals (estimates): source about 5,030 to about 3,530 (minus about 1,500). Tests about 10,600 to about
5,000 (minus about 5,600), most of it the Python oracle and the frame protocol.

## Acceptance

Real-run proofs first. Each runs on the pad (PRD-12) and is checked with PRD-08's tools on the local
parts and on the server copy.

| # | Proof | Pass |
|---|---|---|
| A1 | **No secret, three cases.** Claude Code, Codex and Hermes, each direct key, gateway and dodger, with the planted `.env` and prompt (PRD-12) | The secret scan finds 0 planted values and 0 real keys in local parts and server copy: raw, base64, percent-encoded, joined across the DATA records of each stream. Every planted value with a shape in section 10 has a MASK record. The gateway key is found nowhere. `mask_failclosed_bytes` is reported |
| A2 | **Codex blobs intact.** The Codex direct-key run with one subagent | 0 MASK records inside an `encrypted_content` JSON string. Every MASK record of a guarded rule has a previous byte that passes `glued() == false`. MASK counts per rule and `mask_failclosed_bytes` are reported; any MASK that is not a planted value is named in the night log. A non-zero `fail_closed` count is a night-log line with the stream and the message, not a fail (the bytes are counted, not lost silently) |
| A3 | **Headers masked.** Same runs | Every `authorization`, `x-api-key`, `x-goog-api-key`, `cookie` and gateway key header value in meta is all `*` and listed in `meta.masked`. Rate-limit headers and Codex's `thread-id` are readable |
| A4 | **The record still parses.** Same runs | Every `ws_up` text message and every `application/json` body that ended `complete` with no gap and no `fail_closed` span parses as JSON after masking, and every JSONL line of every native stream parses. Messages and bodies with a `fail_closed` span are counted and named apart |
| A5 | **Cost, reported.** `mask_bench` on the sandbox, release build, one thread: the planted 1.5 MB Codex request pushed in 16 KiB pieces, a Claude Code transcript from the corpus, and start-up with 32 values of 256 bytes | Reported in the night log, never a gate failure: MB/s per input (target 50), `Masker::new` time and memory (target under 1 s and 64 MiB). Below a target, section 11 says what comes next (dispatch table, then PRD-02's pool) |
| A6 | **Early test (phase 1a).** The first Claude Code and Codex direct-key runs, with the core-slice masker (option b, the left guard, `boundary()`, `GH_TOKEN` only) | A1 and A2 pass before the full build ends |

Automated tests that guard them (few, each named for its promise):

- `mask_stream.rs`: for every split point and seeded random chunking of the corpus, the stream's bytes
  and spans equal `mask_whole` (copied); a secret is never released raw at a gap, a boundary or past
  the hold (copied `never_less` and fail-closed reproducers); output length always equals input length.
  New: `a_fail_closed_window_ends_at_the_message` (review A finding 3: a 20 KiB undecided candidate in
  message 1, then 10 plain messages; messages 2 to 11 are byte for byte, the overflow is `fail_closed`);
  the idle `StreamMasker` size is pinned under 256 bytes.
- `mask_rules.rs`: `shapes.tsv` and `mask_rules()` order pinned (`fail_closed` last); `url_password`
  cases from section 5; guard cases from section 4 (each allowed left context masks, each glue byte does
  not, `jwt` included); a 20 KiB base64url run with `sk-`, `hf_`, `ghp_`, `AIza`, `eyJ` inside comes
  through byte for byte and leaves no candidate held; an email and a Codex `rate_limits` object come
  through byte for byte; no rule span holds an unescaped quote or splits an escape; with owner question
  12, `sk_test_` and `whsec_` mask; `Masker::new` refuses a shape that differs from its literal (today's
  `policy_ready` becomes an error at start, not a silent run-time loss).
- `mask_meta.rs`: the header, scheme, URL and query table, rewritten from `headers.rs:276-402`.
- `mask_values.rs` (second slice): each value is masked in all 11 forms, wrapped; the gate refuses a
  value under 20 bytes, with a space, under a non-credential name, a URL or a path; the synthetic
  settings file gives its keys and not its model name or base URL; order and dedupe across the four
  sources; past 32 the rest are `skipped`; nothing value-derived appears in `Debug` or an error.
- `mask_corpus.rs`: over the copied fixtures, new spans only where listed (today's known-catches rule,
  `redaction.rs:615-720`), and the Codex rollout with `encrypted_content` passes byte for byte.

## Risks and blind spots

| Risk | What could go wrong | What we do |
|---|---|---|
| Guard misses glued keys | `xsk-…`, `KEY_sk-…` recorded raw | Owner accepted. The scan (PRD-08) runs without the guard and flags them |
| Model echo | A key split across deltas is stored raw | Unchanged from today. Scan flags; participant told to rotate |
| Fail closed stars real model data | One overflowing hold masks the rest of that message, JSONL line or body (a 1.5 MB Codex HTTP request at worst) | Named `fail_closed`, counted in COUNT, reported by PRD-08 and A2; ends at the message (`boundary()`). `jwt` guarded so blobs no longer hold. Frequency on real Codex traffic is unverified; A2 measures it |
| Values read only at start | Keys the agent adds later, short passwords, keys outside the workspace and the harness's key files are not masked by value | Participant text says so. `continue` reads the files again. A mid-run reader is not built (section 9) |
| Exact values over-mask | A credential-named value that also appears as ordinary text is masked there too | The gate (name, 20 bytes, no URL, no path); accepted (it is the participant's own secret) |
| Exact values over 32 | A participant with a large environment and big harness key files | Provider keys come first (environment is source 2); `skipped` in the start event says how many were left out |
| Token as URL user, no password | `https://<token>@host` with a shapeless token is recorded raw (PRD-17 K12) | Stated loss of dropping `email`; caught when the token has a shape or is an exact value |
| Copy is large for one day | About 1,950 lines of recognizers moved under time pressure | Own lane from H+0; copied, not rewritten; the property tests come with it and stay green before any rule change and before PRD-13 deletes the old code |
| Throughput unknown | Masking below the inflow turns into memory-guard gaps (point 1) | A5 measures in phase 1a; dispatch table, then PRD-02's pool, only if short |
| Held bytes reach disk late | A live candidate at a quiet stream's end waits for more bytes; a lone PEM `BEGIN` holds up to 15,872 bytes | Wire bytes lost only inside a `recorder_down` gap; native bytes are read again |
| Raw bytes in a crash image | The hold, queues and the CA key are in memory | PRD-02: non-dumpable, core size 0, panic hook with file and line only. The masker formats no value into any message |
| Length is visible | A masked value shows its length | Accepted in PRD-01 |
| A real `*` run looks masked | Text that already has stars | MASK records say what was masked; readers use them |
| Privacy | Emails, plan type and credit balance are recorded | Not secrets. Participant told; stripped at export (D26) |
| Binary SQLite | Split values missed; masked WAL frames ignored by SQLite | PRD-04's call; the wire is the primary record |
| Bare scalar | A `scalar` rule over a number breaks that JSON value | No shipped row does it |
| `shapes.tsv` read by two crates | A path change breaks the verifier build | The verifier's `include_str!` path is in PRD-08's builder notes; a test in each crate parses the file |

## Overridable defaults

| Default | Value | Who may change it |
|---|---|---|
| Guard class | `[A-Za-z0-9_-]` (owner said letter or digit; `_` and `-` cover base64url) | Owner |
| Guarded rules | The 13 vendor-prefix literals, `jwt`, and `stripe_webhook_secret` with question 12 | Owner |
| Stripe test keys and `whsec_` | yes (owner question 12), second slice | Owner |
| `url_password` bounds | user 256, password 1,024, escaped slashes allowed | PRD-05 |
| Hold | 16 KiB per stream, 64 candidates; a fail-closed window ends at `\n`, a message end, a gap or the end | PRD-05 after A5 |
| Exact-value sources | `GH_TOKEN`, then credential-named launch environment, workspace key-file and harness key-file values (owner question 12, second slice) | Owner |
| Exact-value gate and guard | credential name, 20 to 256 bytes, no URL, no path; at most 32 values | PRD-05 after A5 |
| Mid-run value reader | not built | Owner |
| Rate-limit header exemption | names with `ratelimit` or `rate-limit` | Owner |
| `object` form | cut | Owner |
| Kept as is | `anthropic_billing_header`, the three Codex id rows | Owner may revisit |
| Not added | `ASIA` beside `AKIA`, an env-line rule, prefixes `pplx-`, `gsk_`, `tvly-`, `re_`, `vcp_`, `sgamp_` (appendix G, G.4 and G.5) | Owner |

## Builder notes

PRD-05 is its own lane from H+0 (PRD-00 phase 0 item 10, review B B2 and O4). It needs only the S2
stub. Order of work:

1. Publish the S2 stub (pass-through, with `boundary()`) in phase 0 so PRD-02 can build. Write
   `shapes.tsv` from `redact.rs:195-311` as it is today and publish it for PRD-08 (S12, by H+3).
2. Copy `redact.rs` into `rules.rs` reading `shapes.tsv`; copy `stream.rs` lines 1-2361 into
   `stream.rs`; replace `Piece` with plain byte runs and spans; drop time marks. Copy the split-point
   properties and get them green before any rule change.
3. Replace the marker with `*` fill, the quote cut and escape alignment; keep `mask_whole` as the oracle.
   Name fail-closed spans `fail_closed`; add `boundary()`.
4. Remove `email`, `object` and `proxy_credential` (shapes and literals together; the indexes 10 and 11
   must not move). Add `url_password` in slot 12.
5. Add `glued()` and the guard in both callers, `jwt` included. Then `meta.rs` and `mask_bench`. This
   ends the core slice (phase 1a runs on it).
6. Second slice: `values.rs` (four sources, the gate), the Stripe pair (owner question 12). Run A5;
   add the dispatch table only if it is short.

Traps from the audit:

- A literal and its shape must change in one commit. A mismatch is now a start error; it must also be a
  failing test (`shapes_follow_the_shared_list`, `stream.rs` test 4).
- `JSON_LITERAL` and `SHELL_LITERAL` are fixed indexes (`stream.rs:151-155`). Removing `email` shifts
  nothing only because `url_password` takes slot 12; `stripe_webhook_secret` goes last.
- `stream.rs:3540-3546` demands that some shipped rule uses the `object` form. Remove `"object"` there.
- `prev_local` exists only for email (`stream.rs:1603`, `:1874`). Reuse its slot for the guard ring.
- `PEM_BODY_MAX + 512 <= MAX_HELD_BYTES` (`stream.rs:904`) and the wrapped-form assert (`:129`) must hold.
- Never use `\b` for the guard. Never test the guard on masked output: it reads raw bytes.
- `boundary()` is not `finish()`: the `StreamMasker` lives on. Do not call it on HTTP bodies.
- The copied fixtures carry `<redacted:…>` markers from the old script. They are data; leave them.
- Never log a value: keep the `Debug` wrappers (`redact.rs:88-98`, `:407-411`) and `wipe` on drop.
  No `format!` of a byte or value in any error or panic text.

How a watcher proves it in a real run: after each pad run, run PRD-08's secret scan on the entry folder
and on the fetched server copy (A1); run `goalcup-verify dump`, take every MASK record with the 24 bytes
of its stream before it, and fail on any guarded-rule span with a glue byte before it or any span
inside an `encrypted_content` string (A2); parse every complete JSON body and message without a
`fail_closed` span (A4). Report MASK counts per rule and `mask_failclosed_bytes` in the night log.

## Open asks of other PRDs

The seven asks of revision 1 (S2 and S3 shapes, the PRD table row, PRD-01 section 8 wording,
`meta.masked`, `mask_values`, GAP numbers) are closed by PRD-00 and PRD-01 revision 2. Left:

1. **PRD-11.** Pass `secret_values` with their key paths (`Vec<(String, Vec<u8>)>`) and drop the 16-byte
   floor from its defaults table; PRD-05's gate decides (section 9).
2. **PRD-10.** Add the `mask::harness_values(&plan.secret_values)` call beside the two it has; show
   `mask_failclosed_bytes` in status when it is not zero.
3. **PRD-02.** The writer adds `fail_closed` span lengths to COUNT `mask_failclosed_bytes`; `status.json`
   carries the number.
4. **PRD-08.** Read `shapes.tsv` (S12); `fail_closed` spans are reported, not failed on length.
5. **PRD-14.** `release_mask_bench` reports, with no floor.
6. **PRD-12 and PRD-17.** The planted `sk_test_` and `whsec_` values and the gateway key in `.env`;
   the boundary lessons case.

## Review answers

Sources: the consistency check (CC), review A (A), review B (B). Decisions already ruled in PRD-00
revision 2 are followed, not reopened.

| Item | What it asked | Answer and what changed here |
|---|---|---|
| CC 4, B (S2 sketch) | S2 lacks `new`, `gap`, `Masked`, `mask_rules`, `value_counts` | Accepted. Interfaces row PRD-02 now matches PRD-00's S2 word for word, `boundary()` included |
| CC 5 (S3 `MaskRule`) | S3 lacks `MaskRule` | Accepted. Row PRD-11 uses PRD-11's `LaunchPlan` fields (`env`, `secret_values`, `mask_rules`) |
| CC 8 (PRD table row) | Row omits exact values and the credential rule | Accepted; PRD-00 fixed it. Nothing more here |
| CC 28, B C2 (GAP 12 twice) | `mask_failed` and `upstream_untrusted` both 12 | Accepted as ruled: `mask_failed` 12. Section 1 and section 6 use it; ask 7 closed |
| CC 37, 40 (start keys, section 8 wording) | `mask_values`, rule indexes kinds, `meta.masked`, no quote in a span | Accepted; PRD-01 revision 2 holds them. Section 9 now names the sources `workspace`, `env`, `harness`, `skipped` |
| CC 41, 66; B C4 (proxy credential) | Delete the "if PRD-03 keeps a credential" hedge | Accepted. The three literals are removed, no condition (section 2, Risks row dropped) |
| CC 63, B C18 (exact-value inputs) | PRD-11 sends `secret_files` values of 16 bytes, PRD-05 says 20 and 32 | Accepted, extended. `secret_files` is source 4; one gate for all sources (20 bytes, credential name, no URL, no path); order `GH_TOKEN`, env, `.env`, harness files; dedupe; `skipped` counted. Checked in the plan: PRD-11 takes every string of 16 bytes or more from settings files that also hold model names and base URLs, so a name gate is needed; PRD-11 passes key paths (ask 1) |
| CC 64, CC 23; B A5 (beyond "nothing more", defer) | Ask the owner; B: defer exact values | Partly. As PRD-00 ruled: owner question 12, default yes, built in the second slice, not deferred. The core slice keeps `GH_TOKEN` only, as today. Section 12 has the text for a "no" |
| CC 65, B C26 (rules as data) | A data file the verifier includes | Accepted. `src/mask/shapes.tsv` (S12): kind, guard, regex; read by `rules.rs` and PRD-08 with `include_str!` |
| CC 89 (`run_secret.rs` cut or copied) | PRD-01 numbers | No change here: about 330 lines copied into `values.rs`, as before |
| A 3 (fail closed stars a WebSocket direction) | Message boundary, separate count, lessons case | Accepted. Checked in the code: the window runs to `End` or `Gap` when no `\n` comes (`stream.rs:76-80`). `boundary()` ends it at each message; spans named `fail_closed`; counted in `mask_failclosed_bytes`; reported in A1, A2, A4; lessons case in `mask_stream.rs` and PRD-17. Added the `jwt` guard, since `eyJ` inside a blob holds bytes to the blob's end. The "above 1 MiB, status says so" line is PRD-10's call: status shows any non-zero count |
| A 5 (core dump, panic text) | Non-dumpable, core 0, panic hook | Accepted for PRD-02, which owns it (PRD-00 rules of the shape). Here: section 1 points to it, and no text in `src/mask/` formats a value |
| A 9 (one mask worker for a large snapshot) | Spread or exempt | Not this PRD's: routed to PRD-02 and PRD-04. A5 reports MB/s so they can decide |
| A 12 (mid-run secrets, Stripe test keys) | Watch `.env`, widen Stripe, add `whsec_`, a canary check | Partly. Stripe test keys and `whsec_` added under owner question 12 (left-guarded, `whsec_` last so no index moves). Checked in the code: `STRIPE_SECRET_KEY=sk_test_…` escapes the shell rule today. Mid-run watching rejected for 1.0: it needs a file watcher in the recorder and a policy swap inside open streams; stated in sections 9, 10, 12 and Risks. The canary is offered to PRD-12 as a shape-rule check |
| B B2, O4 (own lane, order, dispatch only if measured) | Own lane at H+0; order of work; dispatch table only if A5 is short | Accepted. Builder notes now follow that order; section 11 builds the dispatch table only after A5 |
| B N7 (one mask worker) | No pool before measuring | Accepted as PRD-02 ruled. Sections 1 and 11 say one worker; target moved to 50 MB/s on one worker to match PRD-02 R8 |
| B T3 (speed floors fail the gate) | Measure and log only | Accepted. A5 is reported, never a pass line; ask 5 to PRD-14 |
| B M8 (Codex rows in PRD-11's files) | Row removal in PRD-11's first commit | Accepted. "Must not touch" and row PRD-11 say so |
| B section 11 blind spot (deferring `.env` values leaks) | | Agreed; one more reason not to defer (see CC 64) |
| Owner, 30 Oct ("one system, not a patchwork"; gateways) | | No change in design: one masker and one rule list for direct, gateway and dodger cases. Section 10 now shows gateway keys (`AI_GATEWAY_API_KEY`, `sk-or-`, `cf-aig-authorization`) on the same path |
