# Lane C: module audits

Appendix C of the 2026-10-30 audit. Repo `/home/user/goal-cup-proxy` at 4b226de2 (same commit as origin/main).
Read only: nothing was built, run or edited. Line counts are `wc -l`. "src" means product code; inline
`#[cfg(test)]` modules are counted as tests unless a section says otherwise. Every "lines saved" figure is an
estimate from the line ranges named, not a measured patch. The 2-hour run report
(`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any of the 89 remote branches, so no figure
in this appendix comes from it.

## Lane summary

Lane C audited eleven areas, part by part, against the seven points. The recording core holds up: the hash link,
the v5 capture records, the HTTP and WebSocket relays with their decoders, the native reader, the redactor, the
chunk uploader and the Worker's chunk store all serve points 1 to 5 and stay. The weight sits around that core:
deny lists and pins that duplicate code review, disk-full handling the owner has dropped, legacy and V1 paths kept
for a compatibility the owner no longer wants, a second copy of the server rules for tests, research tools in the
product tree, and caps of our own (128 open streams, 32 native streams, 1,024 files, 128 connections, 10,000 tunnel
notes, 10 ingest failures, 5 proxy restarts) that turn load into loss. If every proposed cut were taken before
Friday, about 21,700 src lines and about 36,000 test lines would go (overlaps removed as the notes under the summary
table explain; many need an owner call). The biggest structural cut, merging the daemon into `goalcup start`, is about
8,500 src and 15,000 test lines in the daemon alone, but every area that looked at it says after Edition 01. The
changes that matter most for the points are not cuts: lift the stream and connection caps in protocol, daemon,
proxy and verifier in one release (with the native poll cost fixed first), make the 129th stream a per-stream loss
instead of the end of wire capture, chain every tunnel and loss as it happens, raise the open-file limit, and set
`MIN_COMPETITION_VERSION` to the Friday release.

## Summary table

"Before Friday" is what the area's audit says could go before 6 November if every proposed cut is taken (several
are owner calls). "After Edition 01" is what it defers. Figures are src / tests, estimates.

| # | Area | src lines | test lines | Could go before Friday (src / tests) | Could go after Edition 01 (src / tests) | Top risk |
|---|---|---:|---:|---|---|---|
| 1 | Protocol (`crates/goalcup-protocol`) | 12,383 (11,013 prod + 1,370 inline) | 13,471 (+ about 1,000 fixture lines) | ~3,380 / ~7,800 | ~280 receipt and ack signing; ~200 parts and pruning (disputed, section 13) / ~600 | Control API and record changes must all land in the Friday release (D70); cutting deny lists leaves review as the only connector guard |
| 2 | Daemon (`crates/goalcup-daemon`) | 18,977 | 34,541 | ~680 / ~650 (plus ~800 / ~600 if the owner drops the workspace walk) | ~8,500 / ~15,000 (merge into `goalcup start`, includes termination ~2,300 / ~4,500); finish ~400 / ~900 | Raising caps without the native poll CPU fix, or without the verifier in the same release |
| 3 | Recording proxy (`crates/goalcup-cli/src/proxy/`) | 9,212 (+ 437 caller glue) | 17,507 (1,349 inline + 12,131 integration + 4,027 support) | ~860 / ~925 | ~1,300 / ~3,500 (proxy next to the chain writer; overlaps 2) | The 129th open stream disables wire capture for the rest of the launch |
| 4 | Start path (`goalcup start`, setup, launch) | ~10,300 | ~13,900 | ~1,000 / ~1,400 | ~3,500 to 4,000 one-process recorder (overlaps 2 and 3); retry counted in 5 | Cutting the spawn gate opens a window where a harness runs before the daemon knows it (no subreaper on macOS) |
| 5 | Other CLI commands, `run/`, `participant/`, `account/` | 14,137 | 10,491 (4,650 inline + 5,841) | ~4,150 / ~4,350, plus disk files 744 / 572 and t25 1,173 (counted here once) | ~250 / ~450 (one retry schedule) | Cutting `seal`, `entries`, `login`, `doctor` breaks `matrix.sh` and the install guard |
| 6 | Uploader (`crates/goalcup-cli/src/upload/`) | 3,956 | 12,468 (787 inline + 11,681) | ~470 / ~4,150 | tunnel notes into the chain (not estimated) | Cutting the stand-in's rule tests leaves only `upload_worker.rs`, which is ignored by default and needs Node |
| 7 | Platform (`crates/goalcup-platform`) | 4,926 | 2,653 (969 inline + 1,684) | ~1,400 / ~860 | ~2,400 / ~2,050 (std rewrite of tracecap and sourcecap; fresh crate ~1,500 / ~600) | With no reserve, a real ENOSPC poisons the chain; whether the proxy keeps forwarding is unmeasured |
| 8 | Verifier (`crates/goalcup-verifier`) | 13,488 file lines (~12,150 product + 1,335 inline) | 20,724 (+ 2,182 fixture lines) | ~1,270 / ~3,980 | ~800 to 1,000 / ~1,000 (decoder framework) | Leaving the cap checks in the reader makes every large run exit 2 ("invalid") |
| 9 | Upload Worker (`server/worker`) | 4,001 | 5,255 (+ 520 helpers and vectors) | ~280 / ~450 | ~1,030 / ~1,100 (`db-local.ts` ~700 / 590; judged commit ~330 / 510) | Cutting `mail.ts` without keeping the `claimNotices` call stops every run email; `RESEND_API_KEY` in production is unverified |
| 10 | Connectors (`connectors/`, pins, connector scripts) | 556 data; 588 script src; schema 3,370 (counted in 1) | 411 script tests; ~11,000 connector tests (mostly counted in 1 and 8) | ~670 / ~3,670, plus ~400 to 480 data and ~1,000 docs | none beyond | Embedding connectors touches packaging, `GOALCUP_CONNECTORS_DIR` and the kit; the `**` walk cost is unmeasured |
| 11 | Scripts, gate, workflows, packaging | ~12,300 | ~11,000 | ~7,900 / ~8,300 | `matrix.sh` rewrite into a smaller live driver (most of the remaining ~1,000 test lines) | Cutting `campaign.py` removes the only automated network-cut drill; `matrix.sh` parses CLI wording other areas change |
| | **Lane total, overlaps removed** | not summed (area sizes overlap, see notes) | not summed | **~21,700 / ~36,000** | not summed (sections 2, 3 and 4 overlap) | |

Notes on the totals. Removed from the before-Friday sum because two areas count the same lines: `check-connector.sh`
(10 and 11, ~255 src), `test_check_connector.py` (10 and 11, 411 tests), the `goalcup doctor` decoder check
(8 inside 5's `doctor` cut, ~71 / ~55). Up to ~270 more src may overlap between 1's pin cut (~650) and 10's
`pin-manifests.sh` (274): section 1 lists the script among what goes but does not split its estimate. Connector
ceremony fields (10, ~550 / ~1,400) sit inside 1's `check.rs` cut and are not added. Section 10's test figure
leaves out what 1 (~600 frozen fixtures and guards) and 8 (540 JSON golden lines) already count. The area src and
test sizes overlap too: 4 and 5 both name `home.rs`, `per_run_home.rs`, `folder_gate.rs`, `harness_version.rs` and
`retry.rs`, and 2's merge estimate includes CLI files that 3 and 4 also count.

## Line counts spot-checked

| Claim in a part | `wc -l` now | Result |
|---|---|---|
| `crates/goalcup-protocol/src/chain.rs` 45 | 45 | correct |
| `export.rs` 1,872 prod + 145 inline | 2,017 | correct |
| `harness/check.rs` 1,580 | 1,580 | correct |
| Proxy total 10,561 (`crates/goalcup-cli/src/proxy/*.rs`) | 10,561 | correct |
| Worker src 4,001 and tests 5,255 | 4,001 and 5,255 | correct |
| Upload module 3,956 src + 787 inline | 4,743 | correct |
| `stats.rs` 1,031 + 84 inline | 1,115 | correct |
| `matrix.sh` 2,522, `campaign.py` 1,888, `awake.rs` 269, daemon `ingest.rs` 1,356 | same | correct |
| `launch.rs` 1,994 + 96, `mcp.rs` 273 + 201 | 2,090, 474 | correct |
| `retry.rs` "995 src, 170 inline" (start path part) | 1,165 total; test modules start at line 473, 846 and 996 | **wrong**: about 474 src and 691 tests (the other-CLI part had it right). Fixed in section 4 |
| `run/reserve.rs` "about 283 src / 673 tests" (platform part) | 956 total; `mod tests` starts at line 535 | **wrong**: about 533 src and 423 tests. Fixed in section 7 |
| `MAX_CONNECTION_TASKS` at `proxy/mod.rs:108` (proxy part) | line 112 | fixed to 112 in section 3 |
| `OPEN_FILES_WANTED` at `start/proxy.rs:26` (proxy part) | line 27 | fixed to 27 in section 3 |
| Worker part summary "430 test lines" | its own table sums to ~450 | fixed to ~450 in section 9 |

`GOALCUP_CONNECTORS_DIR` is cited at `config.rs:132` (section 1) and `config.rs:147` (section 10): both are right,
132 is the doc comment and 147 the function.

## 1. Protocol (`crates/goalcup-protocol`)

"Prod" means a src file up to its first `#[cfg(test)]`; "inline" is the test module inside that src file; "tests" is `crates/goalcup-protocol/tests/`.

### Size

| Area | Lines |
|---|---|
| `src/` total | 12,383 (11,013 prod, 1,370 inline tests) |
| `tests/` .rs files | 13,471 (plus about 1,000 lines of TOML and JSON fixtures) |
| Biggest prod files | `export.rs` 1,872; `harness/check.rs` 1,580; `harness/schema.rs` 1,146; `capture_policy.rs` 1,077; `control.rs` 1,010; `capture.rs` 744; `pins.rs` 520; `process.rs` 497 |
| Biggest test files | `tests/it/harness_schema.rs` 4,451 (114 tests); `tests/it/m2_contract.rs` 1,715; `tests/export_roundtrip.rs` 1,477; `tests/it/roundtrip.rs` 1,143; `tests/it/capture_terminals.rs` 884; `tests/it/capture_storage.rs` 829 |

### The main question: sealed hash chain, export format, schema checks, or a plain append-only record?

**Short answer.** Keep the hash link. Cut most of what is built around it. Cut most of the schema checks.

1. **The link itself is cheap.** `chain.rs` is 45 lines (`crates/goalcup-protocol/src/chain.rs:1-45`) and
   `Envelope::chain_bytes` is about 20 more (`src/envelope.rs:49-70`). A plain append-only record with per-stream IDs
   and timestamps would still need `seq`, `entry_id`, `recorded_at` and the stream key, which the envelope already
   has. Dropping `prev` and `hash` saves under 100 protocol lines.
2. **But the link is load-bearing across four parts.** The Worker refuses a chunk whose `Goalcup-Prev` is not the head
   it holds and whose first seq does not follow (`server/worker/src/chunks.ts:69-70`). It signs acks and receipts over
   the head (`server/worker/src/sign.ts:8-106`). The uploader and the verifier read the same fields
   (`crates/goalcup-verifier/src/capture_analysis/reader.rs:437,782,954`). Removing the hash means changing the Worker
   (already deployed by the owner on 29 Oct), the uploader and the verifier together, 8 days before Friday, to save
   under 100 lines. Not worth it.
3. **The hash is not what makes the record trustworthy.** `docs/CHEAT-CHECKLIST.md:15-22` says it plainly: the chain
   is SHA-256 with no secret, so a participant can rewrite and re-hash anything not yet uploaded. The real anchor is
   the server's own copy: create-only R2 objects named by their SHA-256 (`server/worker/src/chunks.ts:155-157`) and the
   time the server stored them. The Worker never recomputes a link hash; it treats `prev` and `head` as opaque tokens
   (`chunks.ts:184-193`). So the hash adds accident detection and offline checks, not point 5. That is fine for 65
   lines. It does not justify the 1,872-line writer around it.
4. **The export format is mostly guard rails that never fire in a real run.** The chain quota is 1 TiB
   (`src/capture.rs:54`, sent by the CLI as `chain_quota: CHAIN_QUOTA_BYTES`, `crates/goalcup-cli/src/run/reserve.rs:75`).
   The terminal allowance, the legacy seal, retired versions 1 to 4, test-only V1 writers, resume seams and a resume
   that re-hashes every byte of every part are all inside `export.rs`. These serve none of the seven points.
5. **Schema checks: most are a second review.** `harness/check.rs` and `capture_policy.rs` (2,657 prod lines together)
   refuse spellings a malicious connector might use: interpreters, startup files, injection env names, seed words.
   D51 already says "review is the control", D70 says only a merged release runs without `--dev`, and `--dev` never
   competes. Keep the checks that stop a typo or a broken run (unknown keys, template tokens, glob grammar, host shape,
   never Goal Cup's own host). Cut the deny lists.

### Verdicts by part

Format: Part (path, src lines, test lines) | What it does | Points served | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go.

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Hash link (`src/chain.rs`, 45 / `tests/it/chain_golden.rs` 102) | `hex(sha256(prev ‖ bytes))` from 64 zeros | 5 (order, continuity for upload) | Keep | 45 lines; the Worker, uploader, acks, receipts and verifier all use it | 0 / 0 | None | None |
| Envelope (`src/envelope.rs`, 129 / in `roundtrip.rs`) | One record shape: entry, seq, time, link, body | 5 | Change | Drop the `Seal` body (`envelope.rs:119-123`, no daemon writes it, D48) and the pre-v5 `redaction` field (`envelope.rs:24-28`), which a v5 line must not set anyway | ~45 / ~120 | The verifier reader still handles `TerminalKind::LegacySeal` (`crates/goalcup-verifier/src/capture_analysis/reader.rs`); change both together | `roundtrip.rs` redaction-field tests (`:521-585`), legacy seal cases in `capture_terminals.rs` |
| v5 capture records (`src/capture.rs`, 744 / 103 inline + `m2_contract.rs` 1,715) | Frames, records, gap reasons, ingest framing and bounds | 1, 2, 3 (gaps), 5 | Keep, change caps | Records are the product. Cut `allowed_tool_rule` and `process_handoff_digest` from `Launch` (`capture.rs:309-316`, no writer since D52), the `capture_status` re-export shim (`:15-22`), `CHAIN_QUOTA_BYTES` and `TERMINAL_RESERVE_BYTES` (`:54-56`). Raise `MAX_OPEN_STREAMS` (`:58`) and `MAX_OPEN_CONNECTIONS` (`:60`); see "Caps" below | ~30 / ~1,000 (freeze goldens) | Raising caps raises worst-case daemon memory; say the number | Most of `m2_contract.rs` byte freezes; keep one golden per record type |
| Status projection (`src/capture_status.rs` 87, `src/events.rs` 26) | What `goalcup status` shows | 6 | Keep | Small, feeds the participant line | 0 / 0 | None | None |
| Control API (`src/control.rs`, 1,010 / 30 inline + `control_shapes.rs` 689 + `control_consumer.rs` 121) | CLI to daemon routes and bodies | 3, 6 (plumbing for all) | Change | `StatusPayload` carries six V1 fields every writer fills with 0 or false: `polls_capped`, `cursor_restore_failures`, `skipped_replayed`, `buffer_gap_at`, `files_capped` (`control.rs:283-318`; written as zero at `crates/goalcup-daemon/src/status.rs:86-90`). Cut them. Optionally cut status signing (`SignedStatus::sign/verify_with`, `control.rs:353-430`): the route is loopback only and the Worker never sees `run_public_key` | ~70, or ~160 with signing / ~250 | `CONTROL_API_VERSION` must bump; D70 pins it at 12 from 0.7.0 to 15 Nov, so this can only ship in the Friday release, never after | `control_shapes.rs` cases for the dead fields and signing |
| Trace writer (`src/export.rs`, 1,872 / 145 inline + `export_roundtrip.rs` 1,477 + `capture_storage.rs` 829 + `capture_terminals.rs` 884) | Appends envelopes, stamps the link, writes baseline and finish, rotates parts, resumes | 5 | Change | Cut: quota and terminal allowance (`:426-445`, `:1062-1095`, `:1207-1235`, `:1298-1310`, never binds at 1 TiB; owner: "disk full: no special handling"); legacy seal phase and test-only V1 writers (`:1425-1428`, `:1511-1565`, `TerminalKind::LegacySeal`); retired version checks (`:345-373`); resume seams (`:1686-1727`). Change resume to read only the last line of the last part for the head instead of re-hashing the whole chain (`:920-967`, `ChainScan :1730-1857`): a week-long chain of tens of GB is re-hashed on every `goalcup continue` | ~550 / ~1,500 | A resume that trusts earlier bytes will not catch local corruption; the verifier and server copy still do | Most of `capture_storage.rs` (quota), legacy seal cases in `capture_terminals.rs`, resume-validation cases in `export_roundtrip.rs` |
| Chain parts and pruning (`export.rs:52-128`, `:1262-1296`; CLI `upload/prune.rs` 421) | 256 MiB / 30 min parts; acked parts deleted locally (D55) | none directly (disk) | Cut, but only with the uploader lane | It exists so a week of traffic fits the disk. The owner ruled disk full needs no special handling. The uploader reads by offset and does not need parts | ~200 protocol + 421 CLI + verifier part code / ~600 | Heavy runs keep every byte on disk all week. Unverified how big a 168-hour run gets: the 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) is not on any branch yet | `roundtrip.rs:768-1100` part tests, prune tests in CLI |
| Seal and baseline manifest (`src/seal.rs`, 140 / in `roundtrip.rs`) | Workspace file list or Git commit and dirty digest; harness home listing | none of the seven; RULES (fresh product, judges) | Keep | Small; judges need the product record | 0 / 0 | None | None |
| Start receipt (`src/receipt.rs`, 186) and ack (`src/ack.rs`, 95) / `signing_vectors.rs` 54, parts of `roundtrip.rs` | Server-signed start time, deadline and chain head; signed acks | none of the seven; "rules held" | Keep for Friday | The Worker that is live signs them. The deadline the client obeys comes from here. After Edition 01, cut: the server's own DB and R2 copy already hold start time and bytes, so signatures only help an offline verifier | 0 now (~280 later) / 0 now | Cutting later touches Worker `sign.ts`, uploader, verifier together | Later: `signing_vectors.rs`, receipt layout tests (`roundtrip.rs:107-356`) |
| Harness schema types and loader (`src/harness.rs` 72, `harness/schema.rs` 1,146, `harness/load.rs` 325, `harness/parse.rs` 228 / `harness_loader.rs` 650, `harness_conformance.rs` 379) | `harness.toml` data model and its one loader | 7, 1 (hosts), 2 (native rows) | Keep, trim | Almost every field is used by a shipped connector. Unused by all four: seeds (`seed`, `files/`), `dirs`, `per_run_dir`. Cut seed support (83 seed mentions in harness and policy code) | ~250 / ~300 | A future harness that needs a seeded file needs a PR | Seed cases in `harness_loader.rs` and `harness_schema.rs` |
| Harness checks (`harness/check.rs`, 1,580 / `harness_schema.rs` 4,451) | 114 refusal rules run in a fixed order | 7 (a typo is refused), otherwise none | Change | Keep: unknown keys (serde `deny_unknown_fields`), template tokens (`:29-70`), native row grammar and duplicates (`:360-464`, `:1068-1112`), host rules (`:1231-1365`), a few cross-field rules. Cut the executable, args, env, home, CA name shape, argv guard, configure, launch-prep and delivery-agnostic deny checks (`:671-1048`, `:1113-1230`, `:1365-1412`) | ~1,000 / ~3,400 | A careless connector PR that sets `LD_PRELOAD` or a base URL passes the loader; review must catch it (D51 already relies on review) | About 90 of the 114 tests in `harness_schema.rs` |
| `MAX_NATIVE_ROWS = 8` (`harness/check.rs:26`) | Caps session-file rows per connector | none | Change | A cap of our own. Claude already uses six rows. Raise to 32 or drop | 0 / 0 | The daemon's `MAX_NATIVE_SOURCES` equals it; change together | None |
| Trust tables (`src/capture_policy.rs`, 1,077 / 29 inline) | Deny lists for env names, interpreters, startup files, argv words, seed words; CA name shapes; host rules | 4 and 1 only for `is_tracker_host` (`:128`) and host shape (`:1001-1077`); 7 for the CA env handling | Change | Keep tracker host, capture host, path and CA rules (~300 lines). Cut `INJECTION_ENV` and friends (`:151-580`), `arg_path_denied` (`:580-646`), interpreters and startup files (`:647-815`), seed words (`:816-889`), argv denial (`:890-1000`). Keep one runtime use: `chat_passthrough` calls `env_key_denied` for `LC_*` names (`crates/goalcup-cli/src/launch.rs:392-404`); replace with a plain `LC_` prefix check | ~780 / ~20 | Same as the row above: review becomes the only guard | Inline policy tests and their `harness_schema.rs` rows |
| Connector pins and directory digest (`src/pins.rs` 520 / 230 inline, `src/connector_dir.rs` 215 / 188 inline, `pins/connector-dirs.txt` / `connector_pins.rs` 303, `pins_build_fails.rs` 86) | Build-time table of connector folder SHA-256s; digest walk; CLI and daemon compare digests (D46) | none of the seven; stops a participant editing the rulebook on disk | Change | Simpler: embed the four `harness.toml` files in the binary with `include_str!` and record the SHA-256 of the embedded bytes in `Launch.connector_dir_sha256` and the Worker run row. `--dev` keeps reading from disk, unpinned. Then pins, the const parser, the directory walk, `scripts/pin-manifests.sh` and the re-pin step on every connector edit go | ~650 / ~800 | Release packaging (`scripts/package-release.sh`), the matrix kit and `GOALCUP_CONNECTORS_DIR` (`crates/goalcup-cli/src/config.rs:132`) assume a folder; change in one PR or skip until after Friday | `connector_pins.rs`, `pins_build_fails.rs`, inline pin and digest tests |
| `providers.toml` (25) | Shared provider host rows, "Frozen (D74): no new rows" | 1 | Change | The freeze stops point 1 ("any provider"). Unfreeze; a row is data | 0 / 0 | None | None |
| Run secret forms (`src/run_secret.rs`, 376 / 154 inline) | Every encoded form of the run's own secret for the redactor (D60) | 4 | Keep | Point 4 | 0 / 0 | None | None |
| Redaction audit (`src/redaction.rs`, 23) | Span evidence beside redacted bytes | 4 | Keep | Small | 0 / 0 | None | None |
| Process discovery (`src/process.rs`, 497 / 251 inline, `ps_env.rs` 24) | `ps` walk, PID-reuse guard, handle digest | 3 (stop at deadline cleanly) | Keep | Needed to end a 168-hour run | 0 / 0 | None | None |
| Clock (`src/clock.rs`, 60 / 63 inline) | Monotonic clock that counts through sleep | 5 (correct time across lid-closed nights) | Keep | Small, real bug it fixes | 0 / 0 | None | None |
| Small helpers (`template.rs` 114, `hints.rs` 27, `manifest.rs` 49, `constant_time.rs` 27, `provenance.rs` 275, `lib.rs` 68) | Template rendering, hint grammar, kill vocabulary, secret compare, digest and time types | 7, 1, 3, 5 | Keep | Shared by CLI and daemon; small | 0 / 0 | None | None |
| Contributor guards (`no_family_literals.rs` 378, `connector_template.rs` 120, `fixtures/frozen-connectors`) | No harness name in core code; template loads; frozen schema-1 fixtures (D68, which is only "proposed") | none | Cut | They guard open contribution and schema-1 compatibility. The owner ruled out backward compatibility before Edition 01 | 0 / ~600 | A core change could again name a harness in a literal | These files |

### Caps in this crate (owner: "no caps of our own")

| Constant | Where | Enforced in | Guards | Proposal |
|---|---|---|---|---|
| `MAX_OPEN_STREAMS = 128` per entry | `src/capture.rs:58` | daemon `runtime/capture/sink.rs:425`, `ingest.rs:1335`, `redact/stream.rs:2769`; CLI `proxy/api.rs:451`; native budget `MAX_OPEN_STREAMS / 4 = 32` at `crates/goalcup-daemon/src/native_capture.rs:279` | Redactor memory: each open stream may hold up to `MAX_HELD_BYTES` = 16 KiB (`redact/stream.rs:125`) | Raise to 4,096 and drop the `/4` split. Worst case held memory is then 64 MiB; say so beside the constant. A native open stream holds no file handle (`OpenStream`, `native_capture.rs:1077-1084`), so macOS's 256 handles is not the limit here (verify the wire side in lane B) |
| `MAX_OPEN_CONNECTIONS = 128` | `src/capture.rs:60` | CLI `proxy/mod.rs:1463`, `proxy/api.rs:599` | Per-connection table | Raise to 1,024, the proxy's own `MAX_CONNECTION_TASKS` (`crates/goalcup-cli/src/proxy/mod.rs:112`). Past the cap the proxy fails open and that traffic is unrecorded, which breaks point 1 for many parallel subagents |
| `MAX_PRODUCERS = 65,536`, `MAX_LIVE_PRODUCERS = 128` | `src/capture.rs:63-66` | ingest receiver | Producer IDs are per proxy epoch, not per subagent (D58) | Keep; add a one-line "guards receiver memory" note |
| `CHAIN_QUOTA_BYTES = 1 TiB`, `TERMINAL_RESERVE_BYTES` | `src/capture.rs:54-56` | `export.rs` quota | Nothing real | Cut with the quota code |
| `MAX_SEGMENT_BYTES` 64 KiB, `MAX_ENVELOPE_BYTES` 128 KiB, `MAX_INGEST_FRAME_BYTES` 512 KiB, `MAX_CAPTURE_QUEUE_BYTES` 8 MiB | `src/capture.rs:42-52` | everywhere | Per-message memory; not a total cap | Keep; they bound one message, not how much is recorded |
| `MAX_NATIVE_ROWS = 8` | `harness/check.rs:26` | loader, daemon | Nothing real | Raise or drop |

### Decisions that justify this crate, and whether they still hold

| D | What it says | Still holds under the seven points and owner decisions? |
|---|---|---|
| D3 | Record raw, judge later; daemon links no verifier | Holds. Keeps the protocol free of rule logic |
| D9 | Loss is a chained gap with a reason | Holds; it is point 3 |
| D14, D36 | Redact at capture, audit spans in the segment | Holds; point 4 |
| D16 | Server signs start time and deadline over the baseline head | Holds for Friday. The signature part is optional once offline verification is dropped |
| D42, D51 | Harness is data; review is the control | Holds, and argues for cutting the deny lists |
| D43 | Trust tables, CA shapes, pins as local gate | Weakened. Its rulings duplicate review (D51) and add ~2,600 lines of checks |
| D46 | CLI and daemon compare connector digests | Holds only while connectors live on disk; moot if embedded |
| D48 | V1 path leaves the tree | Holds, but V1 leftovers remain: legacy `Seal`, test-only receipt and seal writers, V1 status fields |
| D52, D53 | No tool rule; record harness build | D53 holds (cheap). D52 means the two `Launch` fields can go |
| D54 | v5 only; "v5 bytes and verdicts are frozen" | The freeze no longer applies (owner). "v5 only" holds; drop the retired-version paths |
| D55 | Parts at 256 MiB or 30 min; local pruning | Weakened by "disk full: no special handling" |
| D56, D71 | Seal records Git commit and dirty digest | Holds (RULES, judges) |
| D58 | Producer ID budget | Holds |
| D60 | Redact the run's own secret in every form | Holds; point 4 |
| D63 | Receipt must match Launch | Holds while receipts stay |
| D68 | Schema 1 locked, frozen fixtures | Only "proposed". Conflicts with "no backward compatibility". Drop |
| D70 | Release is the list; `CONTROL_API_VERSION` stays 12 until 15 Nov | Holds; it forces every control API change into the Friday release |
| D74 | `providers.toml` frozen | Conflicts with point 1 ("to any provider"). Unfreeze |

### Hidden couplings (what else breaks if a part goes)

- **Link and seq** are read by the Worker (`chunks.ts:69-70`, headers `Goalcup-Prev`, `Goalcup-First-Seq`, `Goalcup-Head`), the
  uploader and the verifier. Keep them stable.
- **`MAX_OPEN_STREAMS`** is one constant enforced in five places in two crates (table above). Raising it is one line here
  plus tests that loop to the cap (`crates/goalcup-daemon/src/runtime/capture/tests.rs:189-277,2236`).
- **The finish rule "a stream never restarts mid-file"** lives in the verifier (`reader.rs:1726-1733`; phase 2 notes
  line 260). If the daemon ever closes an idle native stream and reopens it, the verifier must accept a second `Open`
  of the same path at an offset above 0. The label already carries the path and a generation
  (`native_capture.rs:376`, `<source>:<path>#g<n>`), so "the path is the ID" needs no new field. Not needed if the
  caps are simply raised.
- **Record changes** (removing `Launch` fields, the `Seal` body) need the verifier's `v5-corpus`
  (`crates/goalcup-verifier/tests/fixtures/v5-corpus`, part of 1.1 MB of fixtures) regenerated, and the Worker's
  `MIN_COMPETITION_VERSION` set so an old client cannot open a competition run.
- **Control API changes** need `CONTROL_API_VERSION` bumped, the CLI mock (`crates/goalcup-cli/src/daemon/mock.rs:157-168`)
  and daemon status writer (`crates/goalcup-daemon/src/status.rs:83-90`) edited in the same PR, and must ship before
  0.7.x goes out for Friday (D70).
- **Quota removal** touches `CaptureWorkspaceRequest.reserve_bytes` (`control.rs:548`), the daemon's quota set
  (`crates/goalcup-daemon/src/runtime/capture/mod.rs:449`) and the CLI disk reserve (`crates/goalcup-cli/src/run/reserve.rs:59-75`).
- **Embedding connectors** touches release packaging, the matrix kit, `GOALCUP_CONNECTORS_DIR`, and every connector edit
  (today each edit also needs `sh scripts/pin-manifests.sh`). The owner's masking change (drop `codex_rate_limits`,
  `connectors/codex-cli/harness.toml:110`) is such an edit and needs a re-pin today.
- **Env passthrough (cross-lane note).** A launch hands the harness only `PATH`, `TERM`, `COLORTERM`, `LANG`, `SHELL`
  and `LC_*` (`crates/goalcup-cli/src/launch.rs:333`). Tools that need other variables (a cloud login, a proxy the
  participant uses) will not see them. That is a point 7 risk owned by lane B, not this crate.

### Must change together

| Change | Protocol | Verifier | Worker | Client (CLI, daemon) |
|---|---|---|---|---|
| Raise stream and connection caps | constants | none | none | tests that loop to the cap |
| Drop dead `Launch` fields and `Seal` body | records | reader, report, `v5-corpus` | `MIN_COMPETITION_VERSION` | daemon `Launch` writer |
| Drop quota and terminal allowance | `export.rs`, `control.rs` | none | none | daemon create, CLI reserve |
| Drop dead status fields or status signing | `control.rs`, `CONTROL_API_VERSION` | none | none | daemon `status.rs`, CLI mock and status |
| Drop parts and pruning | `export.rs` | part and prune findings | none | CLI `upload/prune.rs`, `run/watch.rs`, daemon writer |
| Embed connectors | drop `pins.rs`, `connector_dir.rs` | D63 check keeps working on the new digest | run row keeps a `connector_dir_sha256` value | CLI config, packaging, matrix |
| Cut trust tables | `capture_policy.rs`, `check.rs` | none | none | `launch.rs` `LC_*` filter |

### Smallest version that still meets the points this crate serves

- **Records (points 1, 2, 3, 5):** one envelope `{protocol_version, entry_id, source, recorded_at, seq, prev, hash, body}`;
  bodies `Baseline` and `Capture{Open, Segment, End, Gap, Connection, Recovery, ProducerClosed, Launch, Assistance,
  LocalFinish}`. Stream identity is producer plus stream number; a native stream's label is its path. No quota.
- **Writer (point 5):** append a line, stamp the link, flush on a timer, write `seal.json` at the finish. Resume reads
  the last line for the head. One file, or parts only if the uploader lane keeps pruning.
- **Control API (points 3, 6):** create, goal, launch, process, seal, abandon, health, status. Status unsigned or
  signed, but no dead fields.
- **Harness schema (point 7):** types plus serde `deny_unknown_fields`, template tokens, native glob grammar, host
  shape, never Goal Cup's host, CA env present. Four embedded connectors. `--dev` reads a folder.
- **Kept as is:** `run_secret.rs` (point 4), `redaction.rs`, `clock.rs`, `process.rs`, `template.rs`, `hints.rs`, `seal.rs`,
  `receipt.rs` and `ack.rs` (for the live Worker).
- **Estimated size:** about 7,000 to 7,500 prod lines (from 11,013) and about 5,000 to 6,000 test lines (from about
  14,800 including inline tests).

### Blind spots

- Lines saved are estimates from line ranges, not from a written patch.
- I did not measure the daemon memory per open stream beyond `MAX_HELD_BYTES`; the ingest receiver and sink also keep
  per-stream state (unverified size).
- I found no server-side job that re-runs `goalcup-verify` over stored chains, although `docs/CHEAT-CHECKLIST.md:27-28`
  says one exists; I searched `server/` and `scripts/` only (unverified elsewhere).
- The 2-hour run report is not on any branch, so the real chain size of a long run, and so the case for parts and
  pruning, is unverified.
- Cutting deny lists moves all connector safety onto review. A malicious or careless connector PR is then caught only
  by a person.

### Short list

**Top cuts (lines saved, estimated, src / tests)**
1. Harness deny checks in `harness/check.rs` and trust tables in `capture_policy.rs`: ~1,780 / ~3,400.
2. `export.rs` quota, terminal allowance, legacy seal, retired versions, resume seams, full-chain resume re-hash: ~550 / ~1,500.
3. Connector pins and directory digest, replaced by embedded connectors: ~650 / ~800.
4. Frozen byte goldens (`m2_contract.rs`), contributor guards (`no_family_literals.rs`, `connector_template.rs`, frozen connectors): 0 / ~1,600.
5. Chain parts and local pruning, with the uploader lane: ~200 here plus 421 in the CLI / ~600.
6. Seed support, dead status fields, dead `Launch` fields, `Seal` body: ~400 / ~500.

**Top changes**
1. Raise `MAX_OPEN_STREAMS` to 4,096 and drop the native `/4` split; raise `MAX_OPEN_CONNECTIONS` to 1,024; raise or drop `MAX_NATIVE_ROWS`. Write the memory each guards beside it.
2. Resume reads the head from the last line instead of re-hashing the whole chain.
3. Unfreeze `providers.toml` (D74) so any provider can be listed.
4. Land every control API and record change before the Friday release, with `CONTROL_API_VERSION` and `MIN_COMPETITION_VERSION` set to match.

**Must add for the seven points**
1. Point 1: calls to a model host that is not on the approved list are tunnelled and leave no chain record
   (phase 2 notes lines 585-596). The protocol needs either a way to open every host except auth hosts, or at least
   a chained record per unapproved connection (host, bytes, times) instead of only the side tunnel list. The proxy work is lane B.
2. Point 2: the cap changes above, so no subagent transcript is left without a stream.
3. Point 3: a gap reason for connections refused during the proxy's rebind window, which the progress doc left open because "v5 is frozen".

Overlap: the pin cut (~650 / ~800) lists `scripts/pin-manifests.sh` among what goes, and section 10 counts that script (274) too; up to 274 src may be double. The frozen fixtures and guards (~600 tests) are part of section 10's ~1,090. Parts and pruning (~200 here plus 421 CLI) is the same code as section 6's prune row and section 5's Pruner cut, and the views differ (section 13). The ceremony fields in section 10 sit inside this section's `check.rs` cut.

## 2. Daemon (`crates/goalcup-daemon`)

"src" below means product code with inline `mod tests` blocks removed; inline tests are counted as tests.

### 2.1 The crate in numbers

| Area | src lines | test lines (inline + tests/) |
|---|---:|---:|
| Whole crate | 18,977 | 34,541 (4,068 inline + 30,473 in tests/, incl. 213 lines of shell fixtures) |
| Redaction (lane G owns it) | 4,153 (redact.rs 1,208, redact/stream.rs 2,945) | about 10,320 (inline 1,134; capture_redaction 4,745, extra 1,113, run_secrets 379, redaction 2,629, support/redaction 321) |
| Everything else | 14,824 | about 24,200 |

Code comments are long. A large share of every src count is doc comment, so real logic is smaller than the numbers suggest.

### 2.2 Should the daemon be a separate process at all?

**Short answer: no, not for the seven points. But do not merge it before Friday.**

Why it is separate today:

- D4 (docs/DECISIONS.md:75-86): one run-scoped daemon, started detached with `setsid`, so a closed terminal or Ctrl-C does not reach the chain writer (crates/goalcup-cli/src/daemon/spawn.rs:1-45).
- D3 (docs/DECISIONS.md:68-74): "keeps the signed, hard-to-change daemon small". This reason no longer holds: the binaries are unsigned and not notarized (packaging/README.md:22).
- `goalcup seal` and `goalcup status` from another terminal, and crash reopen (status and seal only).

What the split costs. Each item below exists only because the recorder and the launcher are two processes:

| Cost of the split | Where | src lines |
|---|---|---:|
| Loopback HTTP control API with bearer auth (D24, promise P5) | control/ | 1,714 |
| Authenticated TCP ingest, handshake, per-producer ledger | ingest.rs | 1,356 |
| redb registry holding run keys and a legacy state machine | registry.rs | 1,009 |
| Process lifetime, idle exit, control.json, config file | bin/, config.rs, runtime/daemon.rs, runtime/mod.rs | 1,252 |
| Signed status document (run key signs only this) | status.rs, capture/mod.rs:1082-1102 | about 170 |
| Termination with pid-identity guards, because the daemon is not the harness's parent and a pid can be reused | terminate/, runtime/termination.rs | 2,428 |
| Both halves load the connector and compare digests (D46) | runtime/capture/harness.rs | 71 |
| CLI side: daemon spawn, HTTP client, mock | crates/goalcup-cli/src/daemon/ | 2,080 |
| CLI side: ingest sender over TCP | crates/goalcup-cli/src/proxy/ingest.rs | 1,177 |
| Protocol: control API types | crates/goalcup-protocol/src/control.rs | 1,040 |
| The orphan `Recovery` rule kept in four layers (receiver, sink, writer, reader) | ingest.rs, sink.rs, writer.rs, verifier reader.rs | spread |

In one process, `goalcup start` is the harness's parent. It can kill its own child's process group with no pid-reuse risk, hand frames to the sink by a function call, and write status to the `state.json` it already keeps (crates/goalcup-cli/src/run/mod.rs:210-213). If `goalcup start` is killed, the chain has a torn tail, and an offline `goalcup seal` appends the terminal record. The verifier already treats an unsealed chain as incomplete.

**Recommendation.** For Friday, keep the split and make the targeted cuts in section 5. The merge is the right "built fresh" answer: about 8,000 to 9,000 src lines leave the daemon crate, about 3,000 to 4,000 leave the CLI and protocol, and about 15,000 test lines go. But it touches 22 CLI files that use the daemon (`grep -rln 'daemon::\|DaemonClient' crates/goalcup-cli/src`), and it changes P5 (a promise test may not be deleted without the owner). Do it after Edition 01, or before only if the owner accepts the risk and a measured 2-hour run passes on the merged build. The 2-hour run report (docs/research/results/2026-10-30-two-hour-run.md) does not exist yet on any branch, so no live evidence covers either version.

What could go wrong with the merge: a terminal close or Ctrl-C now reaches the recorder (it must ignore SIGHUP and SIGINT and seal itself); a `goalcup seal` from a second terminal needs a new path (a signal to the start process, or a file it watches); crash recovery becomes an offline seal that has not been tested.

### 2.3 File handles and the open-stream limits

**Finding: a native open stream holds no file descriptor.** The native reader opens a file inside one poll and drops it before the next file (crates/goalcup-daemon/src/native_capture.rs:76-81, 1415-1460, 1597-1631). Only one root directory per source stays open (at most 8 sources, `MAX_NATIVE_ROWS`). So raising the 32 native streams, the 128 per entry, or the 1024 files changes the daemon's descriptor count by zero.

Descriptors the daemon holds for one run (read from the code, not measured):

| Holder | Count |
|---|---|
| redb registry | 1 to 2 |
| control listener, ingest listener | 2 |
| ingest sockets (one per live proxy epoch; normally 1 or 2) | up to 128 live + 16 pending handshakes in theory (ingest.rs:145, protocol capture.rs:66) |
| chain part file | 1 to 2 |
| native retained roots (Claude has 6 rows) | up to 8 |
| one native poll in flight: directory chain plus one file | up to 9, then closed |
| stdout/stderr to daemon.log, async runtime | about 5 |
| finish only: git pipes, workspace walk (32-handle budget, crates/goalcup-platform/src/handle_budget.rs:13) | up to 35, then closed |

Normal total: about 20 to 40, far under the macOS default of 256. The only path toward 256 is 128 live proxy epochs at once, which the single-lineage proxy does not produce (unverified, CLI lane).

The descriptor-heavy process is the CLI's recording proxy: two per relayed connection. It already raises its own soft limit to 2,304 (crates/goalcup-cli/src/commands/start/proxy.rs:25-61). That is below macOS `OPEN_MAX` (10,240), so `setrlimit` should not fail with EINVAL (background: https://developer.apple.com/forums/thread/735798, https://gitlab.freedesktop.org/dbus/dbus/-/merge_requests/178). The harness inherits the raised limit.

**The real cost of raising the limits is CPU, not handles.** Every poll processes every tracked file: it re-opens each parent directory and stats the file (native_capture.rs:1376-1389, 1415-1430), and polls run every 100 ms when idle (runtime/capture/owner.rs:16). Completed snapshot files (subagent `.meta.json`) are re-checked too. For Claude's subagent rows (`*/*/subagents/agent-*.jsonl`, connectors/claude-code-cli/harness.toml:108-114) that is about 4 or 5 system calls per file. At 1,024 files that is about 50,000 calls a second; at 20,000 files about 1,000,000 a second, likely a full CPU core (estimate, unverified). That breaks point 3.

Fix in the same change: poll every 1 s when idle (`NATIVE_EVERY`; a poll with deferred bytes already repeats at once, owner.rs:173-186), and open each parent directory once per pass instead of once per file. Then measure CPU with 5,000 and 20,000 synthetic subagent files before Friday.

Memory with no caps: each tracked file is a small struct; each open stream holds at most 16 KiB of undecided redaction bytes (redact/stream.rs:125), and is usually empty once a line ends. The sink queue is a fixed 8 MiB whatever the stream count (sink.rs:7-13). 20,000 open streams is roughly tens of MiB in the worst case (estimate).

### 2.4 Verdicts by part

Format from the brief. Lines are src / tests.

| Part (path, src, tests) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| bin/goalcup-daemon.rs (259 / 32 binary_surface) | Foreground binary, `--exit-when-idle`, signals | none directly (enables 1, 5 via the split) | Keep for Friday; cut when merged | Exists only for the process split | 0 now; 259 / 32 merged | Low | it/binary_surface.rs |
| config.rs (236 / 160 config_load) | daemon.toml roots and control port | none | Keep for Friday; cut when merged | Second config file only because of the split | 0; 236 / 160 | Low | it/config_load.rs |
| control/ (1,714 / about 2,400: control_api 1,735, support/control 499, health_status 151) | Loopback HTTP API, bearer auth (D24), control.json | none directly | Keep for Friday, trim dead routes; cut when merged | The bare `seal` route only answers an error for v5 (protocol control.rs:951-954); the API exists for the split | about 100 / 150 now; 1,714 / 2,400 merged | Medium: P5 is a promise; owner must agree before its tests go | control_api.rs, health_status.rs, support/control.rs |
| registry.rs (1,009 / 652 registry_roundtrip + 105 table_lock) | redb rows, run keys, a legacy launch and process-handoff state machine | none | Change now: strip legacy | `EntryState::Armed/StartSignalSeen/Sealing`, `ExpectedProcessHandoff::LegacyUnknown`, `ProcessHandoffPhase` legacy phases, `last_ack_seq`, `terminal_drain_validation_failed` and many `serde(default)` upgrade paths (registry.rs:36-120, 185-265) serve old rows; no backward compatibility is needed; `v1_row_owes_work` (runtime/daemon.rs:518-535) is V1 | about 400 / 300 now (estimate: a reader must confirm which phases `continue` still uses); 1,009 / 757 merged | Medium: the handoff phases guard the spawn gate on `continue` | parts of registry_roundtrip.rs; table_lock tests when merged |
| status.rs + run key (149 + about 50 / in lifecycle tests) | Signs a status document with a per-run Ed25519 key (D25) | 6 | Keep for Friday; cut when merged | The key signs only this document, which the CLI itself reads; the CLI already has state.json | 0; about 200 / about 300 merged | Low. Launch.run_public_key and the verifier's receipt binding (D63) must change with it; the Worker does not reference `run_public_key` (grep of server/worker/src, unverified for SQL) | status tests in capture_lifecycle.rs |
| runtime/daemon.rs, runtime/mod.rs (757 / inline 0) | Daemon start, stop, idle exit, configured roots | none | Change now (drop V1 helpers); cut when merged | Glue for the split | about 60 / 0 now; about 600 merged | Low | dir_modes.rs (part), start_race.rs |
| ingest.rs (1,356 / 3,095 capture_ingest) | TCP receiver for proxy frames, auth, seq and offset checks, outcome latch, Recovery supersession | 1, 5 (transport) | Keep for Friday, raise stream limit; cut when merged | In one process the sink is a function call; the orphan Recovery rule shrinks to "one producer, gaps for loss" | 0; 1,356 / 3,095 merged | Medium: the Recovery rule is also in the verifier and the proxy, so all three change together | capture_ingest.rs |
| runtime/capture/mod.rs (1,346 / part of 2,340 capture/tests.rs) | Create, goal, launch, reopen, status projection | 1, 2, 5 | Keep; trim reopen and abandon paths when merged | Core wiring of one entry | about 0 now; about 500 merged | Medium | some capture/tests.rs |
| runtime/capture/sink.rs (556) | Bounded queue, epoch admission, per-stream checks | 3 (never blocks: answers Pending or Disabled, not wait) | Keep; raise `MAX_OPEN_STREAMS` | The bounded non-blocking queue is what keeps point 3 | 0 | Low | none |
| runtime/capture/owner.rs (388) | Owner thread: one redactor per producer, flush 200 ms, sync 800 ms, native poll | 1, 2, 4 | Keep; change `NATIVE_EVERY` to 1 s | Poll cost (section 3) | 0 | Low: idle files show up up to 1 s later | none |
| runtime/capture/writer.rs (391) | Chain append, rotation (D55), quota and terminal allowance, gaps on loss | 1, 5 | Change: drop the quota half | `CHAIN_QUOTA_BYTES` is 1 TiB logical (protocol capture.rs:53-54) and is never reached; owner decided no special disk-full handling. A real ENOSPC latches `failure` and every later record is dropped for that segment (writer.rs:123-157, 221-223); status then says Failed (capture/mod.rs:1310) but only when asked | about 120 / about 200 (plus protocol `configure_capture_quota`, export.rs, lane A) | Low. Gap: after a disk-full, the participant is not told unless they run `goalcup status` (point 6) | storage-full tests in capture/tests.rs |
| runtime/capture/finish.rs (763 / 1,267 capture_finish_budget + part of tests.rs) | 180 s budget, 7 refusable stages, sticky expired-finish latch, Git floor (D40, D56) | 5 | Change after Friday; keep for Friday | Fresh version: stop harness, final native pass, drain with a timeout, write terminal. The stage machine exists for a cross-process seal that can time out | about 400 / about 900 when simplified | Medium: finish timing bugs lose the seal | most of capture_finish_budget.rs |
| runtime/capture/harness.rs, harness_build.rs (160 / 137 inline + 449 capture_harness_load + 463 golden) | Load connector (D46); hash harness binary and image (D53) | 7 | Keep both for Friday; harness.rs goes when merged | D53 build record is research evidence; D46 binding only matters with two processes | 0; 71 / 449 merged | Low | capture_harness_load.rs when merged |
| runtime/capture/native_plan.rs (311) | Turns `[[capture.native]]` rows into sources, roots, since_run_start (D57) | 1, 2, 7 | Keep | Data-driven, small | 0 | Low | none |
| native_capture.rs (1,632 / 2,664 capture_native + part of tests.rs) | Native file reader: no-follow, generations, snapshots, discovery bound, loss reports | 1, 2, 7 | Keep; change limits and poll cost | Holds no fd per stream; caps are our own | about 40 removed (cap checks, `MAX_NATIVE_FILES`), about 40 added (dir cache) / about 250 rewritten | Medium: unmeasured CPU at 20,000 files | `a_file_past_max_files_is_one_loss_until_another_arrives`, `stream_file_and_discovery_limits_are_explicit` (part), capture/tests.rs `a_native_file_past_max_files_is_one_streamless_gap_until_the_count_grows`, `an_append_only_file_past_the_default_stream_budget_is_one_gap_at_finish` |
| terminate/ + runtime/termination.rs (2,428 / about 4,550: terminate_sim 2,714, process_kill 1,285, inline 355, shell fixtures 213) | SIGTERM, grace, SIGKILL over the owned tree, pid identity guards, macOS audit-token path, `Confirmed` claim (D22, D30) | none of the seven (serves the 168-hour rule) | Keep for Friday; cut to about 100 lines when merged | The parent can kill its own process group with no pid-reuse risk. The CLI also has its own sweep (crates/goalcup-cli/src/subreaper.rs, 617), so two killers exist today | 0; about 2,300 / about 4,500 merged | Medium: a harness that leaves its process group (Codex npm wrapper, D22 `native_pid`) may outlive the deadline; the server deadline still stops uploads | terminate_sim.rs, process_kill.rs, wrapper fixtures |
| D65 freeze on disk full | Not in the daemon. It is crates/goalcup-cli/src/run/pause.rs and run/reserve.rs (CLI lane) | none | Cut (CLI lane) | Owner: disk full gets no special handling; D65 should be retired | 0 here | Low | CLI pause tests |
| workspace.rs + workspace_git.rs (1,015 / 643 workspace_layout + 19 inline + parts of lifecycle) | Workspace availability check (D5), seal walk or Git digest (D11, D56, D71), baseline home listing | none of the seven | Change: owner decision | The repository is public on GitHub, which already records its history. The walk and Git digest add a 64 MiB output cap, a 50,000-file cap and a 10,000-file home cap. Keep `check_workspace_available` (D5, about 100 lines) | about 800 / about 600 | Medium: lose a local end-state digest; verifier `seal_git_*` findings and protocol `SealManifest.git/home` change with it | most of workspace_layout.rs |
| fsutil.rs (234 / 83 inline) | Atomic private writes | 5 | Keep | Small and shared | 0 | Low | none |
| lib.rs (120) | Error type, time helpers | n/a | Keep | | 0 | Low | none |
| redact.rs, redact/stream.rs (4,153 / about 10,320) | Streaming redaction per producer | 4 | Lane G | Coupling below | n/a | n/a | n/a |

#### Redaction: size and coupling only (lane G audits it)

- Size: 4,153 src, about 10,320 test lines. That is 22% of the crate's src and 30% of its tests.
- Couplings: one `StreamRedactor` per producer, made in owner.rs:296-305; `MAX_HELD_BYTES = MAX_SEGMENT_BYTES / 4` (redact/stream.rs:125) and a compile-time assert that the run secret's wrapped form fits it (stream.rs:129); its own open-stream check at stream.rs:2769 (must move with `MAX_OPEN_STREAMS`); run secrets reach it through the control API create request (D60); connector `extra_patterns` through the schema. The owner decision (add URL password, drop email and Codex rate-limit rules) is redaction-only and does not touch the rest of the daemon.

### 2.5 Decisions behind the daemon, checked against the seven points

| D | Still holds? | Note |
|---|---|---|
| D3 record raw | Yes for "raw, interpret later". The "signed daemon" reason is void: binaries are unsigned (packaging/README.md:22) | The CLI-never-links-daemon rule exists only to keep the split |
| D4 one run-scoped daemon | Holds only while the split stays | Retire if merged |
| D5 agent never sees its evidence | Yes | Keep the check, about 100 lines |
| D9 gap policy | Yes, it is point 3 | |
| D11, D56, D71 seal manifest | Not required by any point | Owner decision (section 4) |
| D14, D36, D60 redaction at capture | Yes, point 4 | Lane G |
| D22, D30 termination | Not a point; serves the 168-hour rule | Simplify when merged |
| D24 control auth | Only because of the split | P5 rests on it |
| D25 status | Point 6, but state.json already serves it | |
| D40 finish budget and spawn gate | Spawn gate serves point 1 (no unrecorded start). The 180 s staged budget is heavier than needed | |
| D46 both halves agree on connector bytes | Only because of the split | |
| D53 harness build | Yes, research and point 7 | |
| D54 v5 only, frozen | Freeze no longer applies (owner) | Lets the stream limit and Recovery change |
| D55 chain rotation | Yes, point 5 (upload in parts) | |
| D57 since_run_start, walker bound 8,192 | Yes, point 2. The walker bound is per poll, not a cap on what is found | |
| D58 producer IDs | Yes. `MAX_PRODUCERS` 65,536 guards ledger memory and is reached only by a stall every 9 s for a week | Keep, it states its cost |
| D65 disk pause | No: owner says no special disk-full handling | Retire; code is in the CLI |

Other limits in the daemon and what they guard:

| Limit | Where | Guards | Verdict |
|---|---|---|---|
| `MAX_OPEN_STREAMS` 128 per entry | protocol capture.rs:58 | nothing real: native streams hold no fd | Raise to about 1,048,576 or remove |
| native `max_open_streams` 32 | native_capture.rs:279 | nothing real | Set equal to the entry limit |
| native `max_files` 1,024, ceiling 20,000 | native_capture.rs:187, 280 | poll CPU (section 3) | Remove, fix poll cost instead |
| `max_dir_entries` 8,192 per poll | native_capture.rs:281 | poll time; rotates, so nothing is lost | Keep, say so |
| `NATIVE_FINISH_BYTES` 64 MiB | owner.rs:13 | finish time | Keep, say so |
| `MAX_TARGETED` 4,096 processes | terminate/mod.rs:94 | signal loop size | Goes with terminate/ when merged |
| `WORKSPACE_FILE_LIMIT` 50,000, `HOME_FILE_LIMIT` 10,000 | workspace.rs:123, 204 | seal time | Goes if the walk goes |
| `UNLAUNCHED_LIMIT` 15 min | runtime/daemon.rs:103 | stale unlaunched entries | Keep |

### 2.6 What must change together

- **Raising `MAX_OPEN_STREAMS`** (one constant, protocol capture.rs:58) changes, in one release: daemon sink.rs:425, ingest.rs:1335, redact/stream.rs:2769, native_capture.rs:279 and 293; CLI proxy/api.rs:451 (wire streams; with one header stream per exchange from Task 7 the wire side can hit 128 with many parallel subagent calls, so this raise helps point 1 too); verifier reader.rs:1694-1696, where more open streams is `open_stream_limit_exceeded`, an invalid trace (exit 2), and decoders tracker.rs:173, session.rs:644 and 761, native/mod.rs:291, anthropic.rs:152. An old verifier will call a new trace invalid, so whoever verifies uploaded runs needs the new verifier (D70: the release is the list).
- **Removing `max_files`**: daemon only, plus docs (docs/harnesses/claude-code-cli.md, docs/HARNESS-ONBOARDING.md) and the four tests above. No connector pin changes.
- **Dropping the writer quota**: protocol export.rs (`configure_capture_quota`, `CaptureStorageFull`) and CLI reserve code (run/reserve.rs) change with it.
- **Merging the daemon into the CLI**: protocol control.rs and ingest handshake types; verifier Recovery and orphan rule, `Launch.run_public_key` and D63 receipt binding; CLI daemon/, proxy/ingest.rs, commands/seal.rs, status.rs, doctor.rs, entries.rs, continue_run.rs; packaging (one binary fewer); docs ARCHITECTURE, OPERATING, TERMINATION, CONTRACTS; promise P5 in README.md:286. The Worker is not affected (unverified for SQL).

### 2.7 The smallest version that still meets the points this area serves

Built fresh, inside `goalcup start` (one process):

1. A bounded, non-blocking sink: on overflow, a `Loss` gap, never a wait (point 3).
2. One redactor per producer before hashing (point 4).
3. The chain writer with rotation and an 800 ms sync (points 1, 2, 5).
4. The native reader: no fd held between polls, no caps of our own, 1 s idle poll, one directory open per pass, the file path as the stream label (`<source>:<relative path>#g<n>`, already the label today) (points 2, 7).
5. Finish: SIGTERM to the child's process group, grace, SIGKILL, one last native pass, `LocalFinish` (point 5).
6. Status lines written to state.json, including "recording stopped" on a writer failure (point 6).
7. An offline `goalcup seal` for a killed launcher.

Estimate: about 3,000 src lines without redaction, against 14,824 today (estimate, not a trial build).

### 2.8 Short list

**Top cuts (src / tests, estimates)**

1. Merge the daemon into `goalcup start` (after Friday unless the owner accepts the risk): about 8,500 / 15,000 in this crate, plus about 3,000 to 4,000 src in the CLI and protocol.
2. Termination down to a process-group kill when merged: about 2,300 / 4,500 (included in 1).
3. Workspace walk and Git digest, if the owner agrees: about 800 / 600.
4. Legacy registry state machine and V1 helpers: about 460 / 300. Safe before Friday once a reader confirms what `continue` uses.
5. Finish stage machine simplified: about 400 / 900 (after Friday).
6. Writer quota half: about 120 / 200.

**Top changes (before Friday)**

1. Raise `MAX_OPEN_STREAMS` to about 1,048,576 or remove it, set native `max_open_streams` to the same and remove `max_files`, in one release with the verifier. No file-handle impact.
2. Cut native poll cost: idle poll 1 s, one directory open per pass. Measure CPU at 5,000 and 20,000 files.
3. Strip registry and daemon legacy compatibility (no backward compatibility before Edition 01).
4. Retire D65 (CLI lane) and the writer quota; disk full is a writer failure with a gap.

**Must add for the seven points**

1. A CPU measurement of the native poll with thousands of subagent files (point 3). Without it, raising the caps may slow the harness.
2. A test, end to end through the verifier, with more than 128 open native streams and more than 1,024 files (point 2).
3. A plain line in the terminal when the chain writer stops recording (disk full or I/O error), not only a `Failed` state in `goalcup status` (point 6).

**What could go wrong**

- Higher caps without the poll fix: CPU load slows the harness (point 3).
- Raising the constant without the verifier in the same release: traces marked invalid.
- Merging the daemon too close to Friday: an untested finish or seal path loses whole runs. The 2-hour run report does not exist yet, so neither design has live evidence for long runs.

Overlap: the merge estimate's "3,000 to 4,000 src in the CLI and protocol" contains section 3's fresh-build ~1,300 and most of section 4's 3,500 to 4,000. Termination (~2,300 / ~4,500) is inside the merge figure. The CLI half of D65 is counted in section 5; the protocol half of the quota in section 1.

## 3. Recording proxy (`crates/goalcup-cli/src/proxy/`)

Scope: `crates/goalcup-cli/src/proxy/` (all twelve files), the host lists that feed it (`commands/start/proxy.rs`, `model_host.rs` where it builds a host rule), and the tests `m2_proxy_integration`, `m2_http_relay`, `m2_websocket_relay`, `m2_tls_connect`, `m2_capture_sender`, `m2_emitter` with their support code.

### 3.1 Short answer: the least proxy that meets the seven points

The proxy core is mostly needed. The over-engineering sits at its edges: counters, a restart
supervisor, outage windows, a side list of tunnels, and caps that turn load into loss or refusal.
The decoding code looks large but point 4 (never record secrets) needs it.

The least proxy that still meets points 1, 3, 4 and 5 is:

1. A loopback CONNECT proxy with a per-launch credential (keep `connect.rs` core).
2. For each listed host: validate upstream TLS, then terminate the harness's TLS with a leaf from a
   per-launch, name-constrained CA (keep `tls.rs`).
3. Forward every byte unchanged first, tee second (keep `http.rs` and `websocket.rs` relays).
4. Undo only what redaction must see through: chunked framing, gzip, deflate, brotli, zstd,
   WebSocket client masking and permessage-deflate. Mask credential headers by name (keep
   `headers.rs`, `metadata.rs`).
5. Chain every uncaptured connection as a record with host, port, reason, time and bytes, at the
   moment it happens. No side list, no top-five summary, no 10,000 cap.
6. No cap of its own on connections, streams or failures. Only the memory queue to the daemon is
   bounded, and it says so.
7. Cut: outage windows, the restart supervisor and its run-wide count, about 30 diagnostic
   counters, the 503 refusal over 1024 connections, the dedupe of loss records, the "10 failures in
   a row" end of the wire leg.

On the four hard cases the question names:

| Case | Today | Least version |
|---|---|---|
| Streaming (SSE) | Forwarded before capture; `sse_prefix_reaches_client_before_eof` (`m2_http_relay.rs:162`) | Keep as is |
| WebSockets | Full RFC 6455/7692 relay with per-message streams (`websocket.rs:1-1019`). Codex uses one WebSocket with permessage-deflate (`docs/harnesses/codex-cli.md:149`) | Keep. Client frames are masked, so the raw bytes hide secrets from the redactor; unmasking and inflating is a point 4 need |
| HTTP/2 | No HTTP/2 path. The harness-facing leaf offers only `http/1.1` (`tls.rs:24-28`, `tls.rs:196`). An `h2`-only client fails its handshake (`connect.rs:629-640`) and gets no service | Do not build HTTP/2 now. Add a fallback: read ALPN from the ClientHello already peeked (`connect.rs:1042-1170`); when it offers `h2` but not `http/1.1`, tunnel it and chain a named loss. About 25 added lines. The shipped harnesses all negotiated `http/1.1` in measured runs (Claude, Codex, Hermes, Amp captured). Whether Bun or a Go harness ever sends `h2`-only through a proxy: unverified |
| Hosts not in a list | Opaque tunnel, counted, noted in the side list at finish (`connect.rs:530-567`, `tunnels.rs`) | Cannot be recorded without decrypting, and decrypting a host the client does not trust breaks it (point 3). Tools the agent runs (curl, git, pip) do not read `NODE_EXTRA_CA_CERTS`. So keep the list (connector defaults plus the config's model address, D74) and make every unlisted tunnel a chained record with host and bytes, so the server and the participant see it (points 5 and 6). Owner decision if more is wanted: see section 7 |

### 3.2 Line counts (wc -l)

`src` is the part of each file above its last `#[cfg(test)]`; `inline tests` is the rest.
`code` is src without comment and blank lines (grep), to show how much is doc text.

| File | total | src | inline tests | code |
|---|---|---|---|---|
| api.rs | 824 | 824 | 0 | 570 |
| connect.rs | 1565 | 1281 | 284 | 974 |
| headers.rs | 403 | 267 | 136 | 205 |
| http.rs | 2257 | 2052 | 205 | 1706 |
| ingest.rs | 1177 | 1136 | 41 | 833 |
| metadata.rs | 407 | 254 | 153 | 162 |
| mod.rs | 1532 | 1498 | 34 | 1043 |
| outage.rs | 316 | 144 | 172 | 95 |
| preflight.rs | 420 | 250 | 170 | 183 |
| tls.rs | 295 | 295 | 0 | 207 |
| tunnels.rs | 285 | 192 | 93 | 147 |
| websocket.rs | 1080 | 1019 | 61 | 821 |
| **proxy total** | **10561** | **9212** | **1349** | **6946** |

Integration tests (all tests): `m2_proxy_integration.rs` 3025 (37 tests), `m2_http_relay.rs` 2249
(32), `m2_websocket_relay.rs` 2074 (27), `m2_tls_connect.rs` 1275 (16), `m2_capture_sender.rs`
1991 (22), `m2_emitter.rs` 1517 (28): 12,131 lines. Support (tests): `m2_proxy_support` 1300,
`m2_ws_support` 783, `m2_sender_support` 693, `m2_tls_support` 525, `m2_http_support` 486,
`m2_common` 240: 4,027 lines. Caller glue (src): `commands/start/proxy.rs` 437.

### 3.3 Verdicts

Format: Part (path, src lines, test lines) | What it does | Points | Verdict | Reason | Lines saved
(src / tests) | Risk | Tests that go.

| Part | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Lifecycle: `start`, `stop`, `stop_within`, drain and cut (`mod.rs:219-560`, about 340 src; covered by `clean_producer_close_follows_connection_drain`, `stop_with_active_stream_and_blocked_sink_meets_one_deadline` and others) | Binds, mints CA, admits producer, serves; stops inside one deadline and closes the producer only when every task ended | 1, 3, 5 | keep | A clean producer close is how the server knows the tail is complete. The single deadline keeps the finish bounded | 0 / 0 | none | none |
| Restart supervisor (`mod.rs:1022-1124`, `ProxyHandle` kill hook `mod.rs:291,310,363,455`; plumbing in `run/mod.rs:275,557,636,731,777`, `run/watch.rs:314-339`, `commands/start/phases.rs:520,709,1060,1073,1508`, `forked.rs:74`, `proxy.rs:89,136`) | Restarts a dead accept loop on the same port, at most 5 times per run, carried across `continue` | 3 (in theory) | cut (or change to a 10-line loop with no cap) | The accept loop only ends on stop, abort or panic. Accept errors already pause and retry (`mod.rs:1011-1014`); a connection task panic is caught by the `JoinSet` (`mod.rs:965-969`). The failure that does happen, the `goalcup start` process dying, is not covered by this. The cap of 5 is a cap of our own with no cost behind it. Spec rule 16 asked for it (`docs/plans/2026-10-29-record-everything-spec.md:59`) | about 150 / about 120 | A real accept-loop death (none known) would leave the harness with a dead proxy. If the owner keeps rule 16, keep a plain loop with no cap and no carry | `a_dead_accept_loop_is_restarted_on_the_same_port`, `the_recorder_is_restarted_at_most_five_times`, `a_continued_launch_keeps_the_runs_restart_count` (`m2_proxy_integration.rs:2920-3025`), `phases.rs:1613-1624` unit |
| Connection cap and 503 (`MAX_CONNECTION_TASKS` `mod.rs:112`, refusal `mod.rs:991-996`, `OPEN_FILES_WANTED` `commands/start/proxy.rs:27`) | Refuses the 1025th live connection with a 503 | 3 (the opposite) | change | A refusal blocks the harness. With thousands of subagents and keep-alive pools, plus tool children that inherit `HTTPS_PROXY` (npm, pip as tunnels), 1024 is reachable. Raise the soft open-file limit to the hard limit and drop the cap: an `EMFILE` accept error already pauses 10 ms and retries, which slows instead of refusing | about 25 / about 70 | Running out of file handles also starves the sink's socket to the daemon. Check the macOS hard limit first (owner decision note) | `excess_connections_are_refused_predictably` (`m2_proxy_integration.rs:1846-1900`), `start_accepts_more_tasks_than_capture_connections` (`:1981-1993`) |
| Capture table (`CloseGuard` `mod.rs:1445-1497`, `MAX_OPEN_CONNECTIONS = 128` `goalcup-protocol/src/capture.rs:60`) | Captures at most 128 intercepted connections at once; the rest are relayed uncaptured with a loss | 1, 2, 3 | change | Idle keep-alive connections hold a place too, so 128 is reached well before 128 live calls. Raise the protocol constant (for example to 65,536) or remove it. The fail-open behaviour itself is right | about 0 / about 0 (test changes) | Memory per tracked connection in the emitter, daemon and verifier is small; a recovery re-observes every open connection (`api.rs:700-800`), so a huge table makes each `Recovery` large (`CONTROL_RESERVE_BYTES` 512 KiB, `ingest.rs:115`) | `intercepted_connections_past_the_capture_table_fail_open` (`m2_proxy_integration.rs:1997`) rewritten |
| Open-stream limit that ends capture for good (`api.rs:451-453`) | The 129th open payload stream of a producer sets the epoch `Disabled(LimitExceeded)`, which `try_recover` never leaves (`api.rs:711`) | 1, 2 (broken) | change | One burst of 128 parallel streaming replies ends wire capture for the rest of the launch. Make it a per-stream refusal plus a chained loss, and raise `MAX_OPEN_STREAMS` (`capture.rs:58`), which the daemon also shares with native files entry-wide (`goalcup-daemon/src/runtime/capture/sink.rs:425`) | about 0 / about 0 (test change) | Must move with the daemon sink, the daemon redactor (`redact/stream.rs:2769`) and the verifier reader (`reader.rs:1694`) | `local_stream_limit_refusal_is_counted` (`m2_proxy_integration.rs:2077`), `stream_and_connection_limits_disable_capture_instead_of_growing` (`m2_emitter.rs:911`) rewritten |
| Health counters (`ProxyHealth` `mod.rs:683-777`, `AdmissionFailureCounts` `mod.rs:647-680`, `Counters` and snapshot `mod.rs:781-920`, the admission class match `mod.rs:1176-1224`, `ConnectFailureKind` `connect.rs:402-426`, `proxy_admission_line` `commands/start/proxy.rs:232-290`) | About 45 counters; 15 admission failure classes; 6 connect subtypes; 4 fail-open subtypes | 6 (weakly) | change | Keep about ten: accepted, intercepted, tunnels by reason, failed, `upstream_connect`, `client_tls` (the common CA trust fault), `alpn_incompatible`, model and recorded request counts. The rest is diagnosis nobody reads in a run | about 170 / about 60 | Less detail when helping a participant whose firewall blocks a host | `refused_loopback_upstream_keeps_a_fixed_connect_failure_kind` (`m2_tls_connect.rs:273-303`); assertions trimmed elsewhere |
| Test-only stop report judgements (`ProxyStopReport::is_clean_drain`, `capture_loss`, `transport_failures`, `mod.rs:591-640`) | Summaries used only by tests | none | keep (test-support only) | Not shipped code; cutting them means rewriting many assertions for little gain | 0 / 0 | none | none |
| Outage windows (`outage.rs` 144 src; `record_upstream_reach` `mod.rs:1241-1270`; `is_provider_host` `connect.rs:274-281`; `finish.rs:445`) | Coalesces failed upstream connects into windows, printed at finish | 6 (weakly) | cut | Not a recording gap. The harness shows its own network errors, and the server sees the silent stretch by time. Its integration test is the known flake (`progress.md:51`) | about 200 / about 230 | The finish text no longer says "the provider was unreachable from 10:02 to 10:07" | `outage.rs` inline tests (172), `an_upstream_outage_is_one_window_closed_by_the_next_reach` (`m2_proxy_integration.rs:2857-2916`) |
| CONNECT admission core (`connect.rs:428-760` head parse, credential, `refuse`, `write_established`) | Authenticates the loopback client, parses the target, decides tunnel or intercept | 1, 3, 4 | keep | Needed; the credential is never forwarded | 0 / 0 | none | none |
| ClientHello peek and SNI checks (`connect.rs:1042-1280`; SniMismatch on other authorities `connect.rs:552-559`) | Decides by SNI; a tunnel to another address that names a listed host is a fail-open loss | 1, 3 | keep, add ALPN read | Small and it stops listed traffic leaving unrecorded through an IP literal. Add the `h2`-only fallback here | -25 (added) / 0 | A new tunnel reason must be known to the verifier and Worker | none |
| Upstream connect retry (`connect.rs:884-1040`) | Retries a provider host's connect inside the admission deadline (5/6 of 30 s) | 3 | keep | A short outage costs a slower answer, not a 502 | 0 / 0 | Can hold a harness call up to about 25 s; the harness's own retry would have done the same | none |
| Per-launch CA and leaves (`tls.rs:93-210`) | Fresh CA key in memory only, name-constrained to listed hosts, 0600 cert file, leaf per host | 1, 4 | keep | Small; the name constraint limits a leaked key. `CA_VALIDITY` 8 days (`tls.rs:54`) fits 168 h because each launch mints a new CA | 0 / 0 | none | none |
| ALPN policy (`tls.rs:24-28,196,270,276-278`, `connect.rs:625-640`) | HTTP/1.1 only on both legs; `h2`-only is refused | 3 (broken for `h2`-only) | change | Refusal blocks; fall back to a tunnel plus a named loss instead | -25 / 0 | none known for the four shipped harnesses | `http1_alpn_policy_is_explicit` (`m2_tls_connect.rs:726`) rewritten |
| HTTP/1.1 relay (`http.rs:116-1310`, about 1,000 code lines) | Forwards first, parses framing (keep-alive, chunked, pipelining, 1xx, upgrade), opens one request and one response body stream per exchange, host-mismatch fallback | 1, 3, 5 | keep | Every exchange on a listed host is captured whatever its path (`http.rs:410-445`), which is what point 1 needs. Framing must be followed to find each exchange | 0 / 0 | none | none |
| Body decoders (`http.rs:1514-2050`: gzip, deflate, brotli, zstd) | Decode transport coding on the capture side only | 4, 1 | keep | The redactor needs plaintext (D36.3). Every Anthropic reply is brotli when `br` is offered (`docs/research/results/2026-10-22-fix-plan-v2.md:15-22`). The alternative, rewriting `Accept-Encoding` to identity, changes harness traffic and still needs a decoder for a server that ignores it | 0 / 0 | none | none |
| Header streams (`headers.rs` 267 src) | One masked head stream per request and response | 4, 5 | keep (optional change later) | Masks credential headers by name before the daemon sees them. Folding the head into the body stream would halve open streams, but it is a protocol and verifier change not worth doing before Friday | 0 / 0 | none | none |
| Metadata allowlist (`metadata.rs` 254 src) | Builds method, host, path without query, status, content type, hint | 4 | keep | Small; keeps query strings and header values out of metadata | 0 / 0 | none | none |
| WebSocket relay (`websocket.rs` 1019 src) | Forwards at once, then unmasks and inflates a bounded view, one stream per message, message index | 1, 4, 7 | keep | Codex's transport. Client frames are masked and compressed, so the redactor cannot see secrets without this | 0 / 0 | none | none |
| Emitter core (`api.rs:296-650`: `open`, `emit`, `connection_event`) | Orders frames, allocates streams and sequence numbers, splits big segments | 1, 5 | keep | The chain needs ordered frames | 0 / 0 | none | none |
| Loss dedupe (`lost_reasons`, `api.rs:205,672-684`) | Sends each loss reason once per emitter, for the whole launch | 5, 6 (against) | change | The server sees one `Loss` for a thousand lost connections, with no time, host or connection. Send one per event and let the protocol carry the connection ID and host | about 15 / about 55 | More small frames; a loop of fail-open tunnels adds chain volume (small) | `a_loss_is_a_stream_zero_control_sent_once_per_reason` (`m2_emitter.rs:1255`) rewritten |
| Epochs, recovery, re-observation (`api.rs:700-824`, `ingest.rs` 1136 src) | Loopback IPC to the daemon with no acks; on queue full or socket loss a fresh producer ID, `Recovery` and re-observation of open connections | 1, 3 | keep for Friday; change in a fresh build | Needed while the proxy lives in `goalcup start` and the chain lives in the daemon (D4, `docs/ARCHITECTURE.md:132`). Fresh build: put the proxy where the chain writer is, or write per-connection spool files the daemon tails like native files (the owner's "the file path is the ID"). Either removes producer IDs, epochs and recovery (about 1,300 src, about 3,500 tests) | 0 now / 0 now | A spool file holds unredacted bodies on disk briefly, which D14 forbids today; the in-daemon proxy grows the signed daemon (D3) | none now |
| Ingest give-up rule (`MAX_CONNECTION_FAILURES = 10`, `STABLE_EPOCH`, `ingest.rs:51-56,98-108,466`) | After 10 failed connects or losses in a row the wire leg is off for the rest of the launch | 1 (against) | change | A daemon stall of about 80 s (1+2+5+10x7) ends recording for a week-long run. Retry every 10 s forever; it costs one connect attempt | about 30 / about 160 | A receiver that always refuses the credential is retried forever, cheaply | `an_epoch_that_takes_one_item_and_closes_at_once_still_runs_out_at_ten` (`m2_capture_sender.rs:1123-1157`), `a_successful_connection_resets_the_failure_count` (`:1161-1237`); `recovery_budget_exhaustion_stays_incomplete` (`:1241-1288`) rewritten |
| Capture queue bound (`MAX_CAPTURE_QUEUE_BYTES` 8 MiB `capture.rs:51`, lanes `ingest.rs:110-125`) | Bounds memory between relay and daemon; overflow latches the epoch lost | 3 | keep, consider raising | A real cost (memory). With many parallel streams and a slow daemon, an overflow loses every in-flight stream until the next request. 64 MiB is cheap on a laptop | 0 / 0 | Memory use goes up; the decode bound `MAX_DECODED_PER_READ` (`http.rs:97`) is tied to it | none |
| Sealed-connection side list (`tunnels.rs` 192 src, `OpenTunnel` `mod.rs:1323-1364`, `finish.rs:450-453`, `upload/api.rs:272`, `upload/uploader/finish.rs:241-249`, Worker `server/worker/src/runs.ts`) | Notes each tunnel; written once at finish; 10,000 rows; top five hosts in the summary; sent as `tunnels_jsonl` with the seal | 5, 6 | change | Written only at finish, so a crash loses it; capped; outside the chain. Chain each tunnel as a stream-zero record when it opens and ends (host, port, reason, bytes). Then cut the file, the seal field, the Worker store and the cap | about 230 client (plus Worker, other lane) / about 200 | Protocol, verifier, daemon and Worker change together; the Worker deploy is an owner step | `tunnels.rs` inline (93), `upload/uploader/finish.rs` tunnel tests, test upload server tunnel handlers |
| Seal-time copy of loss counters (`ProxyCounters` `goalcup-protocol/src/control.rs:683-699`, `finish.rs:457-462`, daemon `runtime/capture/writer.rs:191-194`) | Repeats loss counters in the seal so the daemon adds stream-less gaps | 5 | cut | Since H-16 each loss is chained when it happens; the protocol doc calls this a "confirmatory second copy" | about 40 (across crates) / about 30 | A loss whose frame was refused leaves only an unclosed producer, which already reads as incomplete | daemon `capture/tests.rs:1515` area |
| Runtime with one worker thread (`commands/start/proxy.rs:96-100`) | All TLS, relays, decoders and the sender share one thread | 3 (against) | change | A brotli or zstd decode of a large read, or TLS for hundreds of subagent connections, delays every other connection. Use the default worker count | 0 / 0 | Emitter mutex contention; order is still kept by the mutex | none |
| Start probe (`preflight.rs` 250 src) | One TCP connect per provider host before launch; names the binary for firewall prompts | 6, 7 | keep | Tells the participant plainly before a dead run | 0 / 0 | none | none |
| Host rule assembly (`approved_hosts` `commands/start/proxy.rs:150-185`; `ModelHost::approved` `model_host.rs:79-107`) | Connector hosts plus the one config address (D74); hints by path | 1, 7 | keep | Small; the list is the honest limit of a decrypting proxy | 0 / 0 | A subagent on another provider is unrecorded (section 7) | none |

### 3.4 Decisions that justify the area, and whether they hold

| Decision | What it says for the proxy | Holds under the seven points and owner decisions? |
|---|---|---|
| D1 (`DECISIONS.md:52-60`) | Process-scoped forward proxy; decrypt only declared provider hosts; tunnel the rest | Holds. It is the reason "any provider" stops at the list. Point 7 is served: no base URL change |
| D3 (`:68-73`) | Record raw, interpret later | Holds; the proxy parses framing only |
| D9 (`:100-103`) | Loss is stated, never hidden | Holds, but the once-per-reason loss dedupe and the finish-only tunnel list under-state it. Change both |
| D14, D36 (`:121-124`, `:167-178`) | Redact before disk; decode transport coding on the tee; strip credential headers by name | Holds; it is why decoders and WebSocket unmasking stay. D14 blocks the "spool file" fresh-build option unless the owner relaxes it |
| D58 (`:488-496`) | 65,536 producer IDs; `MAX_CONNECTION_FAILURES` ends the wire leg | The ID budget holds. The give-up rule conflicts with "record everything, never block, runs last a week". Change |
| D59 (`:498-509`) | `NODE_USE_ENV_PROXY` so Node uses the proxy | Holds (point 7) |
| D62, D74 (`:592-640`, `:1105-1162`) | No vendor list; connector defaults plus the config's address; every sealed tunnel listed in a side file with a cap of 10,000 | The host rule holds (owner ruling). The side file and its cap conflict with "no caps of our own" and point 5; move tunnels into the chain |
| H-16 (`api.rs:25-30`, `control.rs:685-689`) | Every proxy loss is a stream-zero `Loss` when it happens | Holds; it makes the seal-time counter copy redundant |
| Spec rule 16 (`record-everything-spec.md:59`) and Task 10 | The proxy restarts itself; cap 5 kept by owner | Weak. It guards a failure not seen; the cap is our own. Owner to confirm the cut |
| "Protocol v5 is frozen" (cited in `outage.rs:13-15`, `mod.rs:39`, `tunnels.rs:3-4`) | Why outages and tunnels live outside the chain | No longer applies (owner decision). The reason for the side channels is gone |

### 3.5 Hidden couplings (what else breaks if a part goes)

- `ProxyHealth` fields feed `commands/start/proxy.rs:232-290` (`proxy admission:` line),
  `commands/start/finish.rs:440-462` (finish lines and the seal's `ProxyCounters`), and the run
  heartbeat through `ProxyHandle::model_requests`, `recorded_requests`, `generation_requests`,
  `recorder_restarts`, `recorder_lost`, `client_tls_failures` (`mod.rs:384-440`; read in
  `run/watch.rs:314-339` and `commands/start/phases.rs:1073`). Keep the request counts: the "no
  model calls recorded" alarm and the evidence label use them.
- The restart count is stored in run state (`run/mod.rs:275,457,557`) and shown by `goalcup status`.
  Cutting the supervisor means removing the status problem line and the carry in `phases.rs`.
- `tunnels_jsonl` is a seal request field (`upload/api.rs:272`), a Worker store
  (`server/worker/src/runs.ts`), a test upload server field (`tests/common/upload_server/handlers.rs:1128-1185`)
  and a D74 promise. Moving tunnels into the chain touches all of them and the verifier report.
- The `test-support` seams (`connect.rs:924-945` DNS override, `tls.rs:240-265` extra roots,
  `ingest.rs:185-200` stable-epoch override, `mod.rs:291,455` kill hook) are used by tests in other
  lanes (`tests/full/*`). Cutting the kill hook breaks only the three restart tests.
- `MAX_OPEN_STREAMS` is entry-wide in the daemon and shared with native files (32 append-only,
  `daemon/src/native_capture.rs:279` per phase 2 notes line 605). Raising the wire side alone
  does not help; the daemon lane must raise the same constant.
- `MAX_DECODED_PER_READ = MAX_CAPTURE_QUEUE_BYTES` (`http.rs:97`): raising the queue also raises
  the decode bound per read.
- The `CutUnsettled` loss is chained as `ShutdownDeadline` (`api.rs:41-46`), and so is a restart
  cut. Cutting the restart supervisor removes the second meaning.

### 3.6 What must change together

| Change | Proxy | Protocol | Daemon | Verifier | Worker | Client glue |
|---|---|---|---|---|---|---|
| Raise or remove `MAX_OPEN_STREAMS`, `MAX_OPEN_CONNECTIONS` | `api.rs:451`, `mod.rs:1470` | `capture.rs:58,60` | `runtime/capture/sink.rs:425`, `redact/stream.rs:2769` | `reader.rs:1694` and decoders that cite it | none | none |
| Per-event `Loss` with connection and host | `api.rs:652-690` | `FrameEvent::Loss` shape | ingest validation, writer | gap reading | report fields if shown | none |
| Tunnels as chained records | `mod.rs:1149-1237`, delete `tunnels.rs` | new `ConnectionEvent` or `Loss` fields | chain writer | report: unrecorded hosts | drop `tunnels_jsonl` store | `finish.rs`, `upload/api.rs`, `uploader/finish.rs` |
| New `h2`-only fail-open reason | `connect.rs` | tunnel reason word | none | count it | none | admission line |
| Drop seal-time `ProxyCounters` | none | `control.rs:683-699` | `writer.rs:191-194` | none | none | `finish.rs:455-462` |
| Ingest retries forever | `ingest.rs` | none | none | none | none | none |
| Cut restart supervisor | `mod.rs` | none | none | none | none | `run/mod.rs`, `watch.rs`, `phases.rs`, `forked.rs`, `proxy.rs` |

### 3.7 Blind spots and owner questions

- **Unlisted providers.** A subagent that calls a provider the config does not name is tunnelled
  and not recorded. Chaining the tunnel makes it visible, not recorded. Options beyond that all
  cost something: reading every endpoint in the harness config (phase 2 notes, lines 295-298),
  a fixed list of well-known model hosts (against D62 and D74, an owner ruling), or decrypting all
  hosts with a CA handed to every tool (breaks tools that ignore the CA variables; Go on macOS and
  Rust tools with built-in roots likely ignore `SSL_CERT_FILE`, unverified).
- **Plain HTTP and loopback models.** Only `HTTPS_PROXY` is set, so `http://` traffic and a local
  model server never reach the proxy. D62 refuses such a model address at start, so this is
  stated, not silent.
- **Proxy overrides inside the harness.** OpenClaw's own proxy setting wins over `HTTPS_PROXY`
  (`origin/opencode-openclaw:docs/harnesses/openclaw-cli.md:74-75,103-104`). The proxy cannot see
  that traffic at all; only the native leg and the "no model calls recorded" alarm catch it.
- **Process death.** If `goalcup start` dies, the harness keeps an `HTTPS_PROXY` that points at a
  dead port and every model call fails. The restart supervisor does not cover this. Nothing in this
  lane can; it is a launcher question (does the harness die with its launcher?).
- **HTTP/2 claims.** That every shipped harness negotiates `http/1.1` through the proxy rests on
  measured runs of Claude Code, Codex, Hermes and Amp in this repo's research results. Whether Bun
  `fetch` (OpenCode) or a Go client ever sends `h2` only is unverified.
- **The 2-hour run report** (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on
  any branch yet, so no load figure (peak open streams, peak connections) backs the limits above.
  Measure the wire peak on that run before picking new constants.
- **Raising the queue and stream limits** moves pressure to the daemon redactor (per-stream state)
  and the verifier (per-stream memory). Those lanes must confirm memory per stream.

### 3.8 Smallest version of this area

Keep: `connect.rs` core, `tls.rs`, `http.rs`, `websocket.rs`, `headers.rs`, `metadata.rs`,
`api.rs` emitter core, `ingest.rs` (for Friday), `preflight.rs`, the host rule assembly.
Cut: `outage.rs`, `tunnels.rs` (moved into the chain), the restart supervisor, about 30 counters,
the connection cap, the loss dedupe, the ingest give-up rule, the seal-time loss counter copy.
Change: no limit that ends capture for good, `h2`-only falls back to a tunnel, more worker threads.
That is about 9,200 src lines down to about 8,400 for Friday, and about 7,100 in a fresh build that
moves the proxy next to the chain writer and drops epochs and recovery.

### 3.9 Lists

**Top cuts (src / tests, estimates from the spans cited above)**

1. Tunnel side list into the chain, then delete `tunnels.rs`, `OpenTunnel`, `tunnels_jsonl` and the
   Worker store: about 230 / 200 (client side only).
2. Outage windows (`outage.rs`, `record_upstream_reach`, finish line): about 200 / 230.
3. Diagnostic counters and connect subtypes down to about ten: about 170 / 60.
4. Restart supervisor, its cap and its run-wide carry: about 150 / 120.
5. Ingest give-up after 10 failures: about 30 / 160.
6. Seal-time `ProxyCounters` copy: about 40 / 30.
7. 503 connection cap: about 25 / 70. Loss dedupe: about 15 / 55.

Total: about 860 src and 925 test lines, before the fresh-build option (about 1,300 src and 3,500
test lines more).

**Top changes**

1. The 129th open stream must not end capture for good (`api.rs:451-453`); refuse that stream with
   a chained loss, and raise `MAX_OPEN_STREAMS` and `MAX_OPEN_CONNECTIONS` with the daemon and
   verifier.
2. Chain every loss and every tunnel as it happens, with connection ID, host and time.
3. Never refuse a connection (drop the 503), never give up on the daemon (retry every 10 s).
4. `h2`-only clients fall back to a tunnel plus a named loss instead of failing.
5. Use more than one runtime worker thread for the proxy.
6. Consider a larger capture queue (64 MiB) after the 2-hour run gives a measured peak.

**Must add for the seven points**

- Point 1 and 2: the open-stream and connection limits raised or removed, and no terminal disable
  on overflow. Without this, a burst of parallel subagents ends wire recording for the launch.
- Point 3: the `h2`-only fallback and no 503 refusal.
- Point 5 and 6: tunnel and loss records in the chain with host and time, and one plain finish
  line per unrecorded host ("passed through unread: host:port, n connections, KiB"), not a top
  five.

Overlap: the tunnel cut (~230 / ~200) includes `upload/uploader/finish.rs` tunnel code that section 6 keeps for Friday (section 13). The fresh-build option overlaps section 2's merge.

## 4. Start path (`goalcup start`, `goalcup setup`, launch)

Audit of the start path in `crates/goalcup-cli`. "src" is the part of a file above its `#[cfg(test)]` module, "inline tests" is the rest. Paths are relative to `crates/goalcup-cli/` unless they start with `docs/` or `connectors/`. Correction: `retry.rs` is 474 src and 691 inline test lines (`wc -l` 1,165; test modules start at lines 473, 846 and 996); this section first said 995 and 170, which the figures below now fix.

### 4.1 What is in scope, and how big it is

| Part | src | inline tests | Notes |
|---|---:|---:|---|
| `src/commands/start/` (mod, phases, server, project, proxy, refusals, forked, finish) | 4,418 | 404 | phases.rs alone is 1,511 src |
| `src/commands/prep.rs` (`goalcup setup`, alias `prep`) | 1,301 | 63 | |
| `src/model_host.rs` | 894 | 646 | |
| `src/preflight/` (mod, gates, probe) | 277 | 0 | mod.rs is docs only |
| `src/launch.rs` | 1,994 | 96 | about 320 lines are the spawn gate |
| `src/daemon/mod.rs` + `spawn.rs` (CLI side client) | 1,434 | 10 | |
| `src/daemon/mock.rs` | 636 | | test-support only, counted as test code |
| Supporting modules the start path calls (`home.rs`, `per_run_home.rs`, `folder_gate.rs`, `harness_version.rs`, `retry.rs`, `commands/run_repo.rs`) | 2,749 | 534 | named where they matter; other lanes may own them |

Integration tests that test this path (all test lines):

| Test file | Lines | Tests |
|---|---:|---:|
| `tests/m2_cli_lifecycle.rs` | 1,944 | 34 |
| `tests/it/start_launch.rs` | 1,481 | 32 |
| `tests/it/prep.rs` | 1,600 | 32 |
| `tests/it/daemon_client.rs` | 1,248 | 33 |
| `tests/it/m2_ipc_compose.rs` | 1,606 | 4 |
| `tests/it/model_host_from.rs` | 793 | 10 |
| `tests/it/harness_launch_plan.rs` | 782 | 18 |
| `tests/it/capture_gate.rs` | 519 | 16 |
| `tests/it/preflight_doctor.rs` | 478 | 13 |
| `tests/it/per_run_home.rs` | 438 | |
| `tests/it/process_handoff.rs` | 147 | 2 |
| `tests/common/m2_cli_support/mod.rs` (rig) | 1,673 | |

So about 10,300 src lines and about 13,900 test lines (inline plus integration plus rig plus mock)
sit on the road from `goalcup start` to the harness's first byte.

### 4.2 Every check and refusal before a harness starts, in order

"Refuse" stops the start with exit 1. "Warn" prints a line and goes on. "Point" is the VISION
point it serves (1 to 7), or "rules" when it serves the contest rules (RULES.md) but none of the
seven points, or "none".

| # | Check | Where | Kind | Point | Verdict |
|---|---|---|---|---|---|
| 1 | Connector id exists, loads, is declared once | `src/main.rs:544-548`, `src/connector.rs` `load_harness` | refuse | 7 | keep |
| 2 | Same load again inside `capture` | `src/commands/start/mod.rs:443` | refuse | none | cut the second load (pass the first) |
| 3 | Child subreaper armed (Linux) | `src/main.rs:553-559` | warn | 3 | keep |
| 4 | WSL1 refused; WSL2 Windows drive refused, for 4 folders | `start/refusals.rs:111-140`, `src/folder_gate.rs` | refuse | 7 | keep |
| 5 | Folder filesystem not APFS, ext4 or XFS | same | warn at start, **fail** in setup | none (reserve file) | change: warn in setup too, or drop with the reserve |
| 6 | Stale reserve released | `start/mod.rs:442` | side effect | none | cut with the reserve (owner: disk full has no special handling) |
| 7 | Model service in the harness config: missing (when `absent = "refuse"`), unreadable file, `via` not a plain name, not https, loopback or local name, IP literal, not a DNS name, Goal Cup's own host, sign-in host, unclean path | `src/model_host.rs:339-497` | refuse | 1 | keep (a service the proxy cannot open would be a silent hole in point 1) |
| 8 | `--practice` or `--test` without `goalcup login` | `start/server.rs:35-42` | refuse | none | keep (2 lines) |
| 9 | `--dev` on a competition start | `start/server.rs:44-46` | refuse | rules (D70) | keep; the Worker refuses too |
| 10 | `goalcup setup` has run (`prep.json` exists) | `start/server.rs:48`, `src/prep_file.rs:50-64` | refuse | 7 | keep |
| 11 | Project folder: no open run uses it | `start/project.rs:44-50` | refuse | 6 | keep |
| 12 | Project folder not in, around or beside goalcup's folders | `start/project.rs:187-205` | refuse | rules (D5); daemon refuses too | keep (cheap, saves a wasted approval) |
| 13 | Project folder not inside a Git work tree | `start/project.rs:52-59` | refuse | rules | keep |
| 14 | Project folder not in a synced folder (iCloud, Dropbox) | `src/per_run_home.rs:318-489` | refuse | none | cut, or one warning line |
| 15 | Project folder empty but for `.gitignore`, `.DS_Store`, `.env*` | `start/project.rs:153-185` | refuse | rules (fresh product) | keep |
| 16 | Every `.env*` file covered by `.gitignore` (own gitignore matcher) | `start/project.rs:62-72, 225-330` | refuse | none (protects the token on GitHub, not the record) | change: accept a small fixed set of lines (`.env*`, `.env`, `.env.local`), cut the glob matcher |
| 17 | Real run: one plain `GH_TOKEN=` line | `start/project.rs:98-145` | refuse | rules (D71) | keep |
| 18 | Free disk room for the 3 GiB reserve | `start/phases.rs:80-84`, `src/run/reserve.rs:251-300` | refuse | none | cut (owner decision on disk full) |
| 19 | Connector directory pin (`manifest_pin`), `--dev` door | `start/phases.rs:95-105`, `src/preflight/gates.rs:117-171` | refuse | rules | keep (small) |
| 20 | stdin is a foreground terminal and stdout a terminal | `start/refusals.rs:76-87` | refuse | 7 | keep |
| 21 | Harness program on `PATH`; `#!/usr/bin/env` interpreter on the harness `PATH`; symlink; not a regular file; not UTF-8 | `src/launch.rs:444-574` | refuse | 7 | keep; the symlink check never fires (the path is canonicalised first, `launch.rs:458`) |
| 22 | Model service name resolves only to this computer (DNS) | `start/phases.rs:125-133`, `src/model_host.rs:507-540`, `src/proxy/preflight.rs:137-200` | refuse | none clear | cut |
| 23 | Hard hosts answer a TCP connect (model service, else the first default host) | `start/phases.rs:107-142`, `start/refusals.rs:172-216` | refuse | none (setup already checks) | cut at start; keep in setup |
| 24 | Default hosts answer, when a service is named | same | warn | none | cut with 23 |
| 25 | Per-run home: run folder not in a Git work tree, not synced | `start/phases.rs:144-160` | refuse | none | keep the Git check, cut the sync check |
| 26 | Repository named, public and empty on GitHub | `src/commands/run_repo.rs:66-168` | refuse | rules (D71); Worker checks again | keep (it is how the repo is asked for) |
| 27 | Browser approval: denied, expired, Ctrl-C, server away | `start/server.rs:120-190`, `src/account/device.rs` | refuse | rules | keep |
| 28 | Project folder checks 11 to 17 again, after the approval | `start/server.rs:174-180` | refuse | as above | keep (one call) |
| 29 | Ctrl-C seen since the start | `start/mod.rs:495-497` | refuse | 3 | keep |
| 30 | Shared harness home locked by another run | `src/home.rs:91-110` | refuse | 2 (session files of two runs would mix, D57) | keep |
| 31 | Shared home owner, mode 0700, alias identity; checked at claim, then rechecked twice before create (`pause_at` point) | `start/phases.rs:234-245, 282-290`, `src/home.rs:30-90` | refuse | none (same-user swap is a stated residual in D39) | change: check once at claim, cut the two rechecks |
| 32 | Per-run home made from seeds | `src/per_run_home.rs:64-120` | refuse on error | 7 | keep |
| 33 | Harness `--version` probe in a scratch home | `start/phases.rs:271-275`, `src/harness_version.rs` | never refuses | 5 (research) | keep |
| 34 | Continue only: does the harness have a session to resume (native files, Hermes `state.db` mtime, earlier model calls, last resume failed) | `start/phases.rs:296-361, 745-772`, `src/launch.rs:664-746` | decides resume | 7 | change: resume when earlier parts recorded a model call, else new; keep the "failed resume" fallback; cut file and database sniffing |
| 35 | Server opens the run (`POST /v1/runs`): `TOO_OLD`, `DEV_COMPETES`, repo not empty | `start/server.rs:280-380` | refuse | rules | keep |
| 36 | Proxy credential accepted as a run secret | `start/phases.rs:384-396` | refuse (internal) | 4 | keep |
| 37 | Daemon creates the capture workspace (its own refusals) | `start/phases.rs:409-417` | refuse | 1, 5 | keep |
| 38 | Reserve file taken | `start/phases.rs:496-499` | refuse | none | cut |
| 39 | Server start receipt bound | `start/phases.rs:500-507` | refuse | 5 | keep |
| 40 | Open-file soft limit raised to 2,304 | `start/proxy.rs:33-63` | warn | 2, 3 | change: raise to the hard limit (see section 7) |
| 41 | Proxy runtime, listener and IPC sender start | `start/proxy.rs:81-150` | refuse | 1 | keep |
| 42 | CA bundle file written | `src/ca_bundle.rs` | refuse on error | 1, 7 | keep |
| 43 | Daemon shows the proxy's producer admitted within 5 s | `start/proxy.rs:192-226` | refuse | 1 | keep |
| 44 | Goal Cup skill installed in the harness home | `start/phases.rs:537-540`, `src/skill.rs` | refuse on error | none (helps the agent find `time-left`, D66) | keep, but make an error a warning |
| 45 | Connector `configure` commands (D61) and launch prep (D72, Hermes trust entry) | `start/phases.rs:541-600`, `src/launch.rs:1004-1170` | refuse | 1, 7 | keep |
| 46 | Workspace path has no quote, backslash or control byte | `src/launch.rs:783-799` | refuse | 7 | keep |
| 47 | Run homes, CA, bundle, time-left file are absolute UTF-8 | `src/launch.rs:801-814, 1285-1305` | refuse | none | keep (tiny) |
| 48 | Proxy URL is loopback (checks a URL this process just made) | `src/launch.rs:812, 1306-1349` | refuse | none | change to a debug assertion |
| 49 | Every rendered argument passes `harness_arg_refusal` (D51) | `src/launch.rs:816-838, 1461-1468` | refuse | 1 (D1, no base URL) | keep |
| 50 | Termination strategy known | `src/launch.rs:886-896` | refuse | none (unreachable) | keep (5 lines) |
| 51 | argv plus env fit in `ARG_MAX` | `src/launch.rs:912-920, 1350-1390` | refuse | none (exec reports E2BIG anyway) | cut |
| 52 | Fixed goal sentence recorded (`CHAT_MODE_GOAL`), asked once | `start/phases.rs:71-73, 612-618` | refuse | none (v5 shape only) | cut with a protocol change |
| 53 | Ctrl-C seen | `start/phases.rs:619-621` | refuse | 3 | keep |
| 54 | Push block copied to the clipboard | `start/phases.rs:627-630` | never refuses | 6 | keep or cut (owner); small |
| 55 | Child forked and parked before exec (spawn gate, D40) | `src/launch.rs:1676-1994` | refuse | none of the seven | change: plain spawn, then register, kill on failure |
| 56 | Parked child named (`gated_handle`) | `start/phases.rs:837-843` | refuse | none | goes with 55 |
| 57 | Process registered with the daemon | `start/phases.rs:845-856` | refuse, child never runs | 3 (the seal can end it) | keep the registration, drop the gate around it |
| 58 | Ctrl-C seen (third time) | `start/phases.rs:860-863` | refuse | 3 | keep |
| 59 | Terminal still held (second time) | `start/phases.rs:870-879` | refuse | 7 | keep |
| 60 | Terminal shield taken (`ChatTerminal::take`) | `start/phases.rs:921-934` | refuse | 3 | keep |
| 61 | Gate released, child acknowledges within 5 s | `start/phases.rs:936-942` | refuse | none | goes with 55 |
| 62 | After release: entry not sealed; launch recorded | `start/phases.rs:997-1030` | warn (never stops the harness) | 1, 5 | keep |

That is 62 named steps. About 40 of them can refuse a start. My count of steps that serve one of
the seven points directly is 26; 14 serve the contest rules only; the rest serve none.

### 4.3 Verdicts by part

| Part (path, src lines, test lines) | What it does | Points served | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Start orchestration (`src/commands/start/mod.rs`, 591, 0; `forked.rs` 316/18; `finish.rs` 545/64) | Order of phases, abandon window, drains of earlier segments, wrap-up | 1, 3, 5 | keep | The abandon window and drains carry D64 (no server error stops a run). `pause_at` and `panic_while_quiet_for_test` are test seams worth keeping. | ~10 (second `load_harness`) / 0 | low | none |
| Phases (`start/phases.rs`, 1,511, 202 inline) | `prepare`, shared home, `create_and_gate`, register and release, `run_harness` | 1, 2, 3, 5, 7 | change | Cut: reserve calls, start probe block, two shared-home rechecks, session sniffing on continue, goal record, gate plumbing. Keep the rest. | ~250 / ~150 inline | medium: this is the most tested file; each cut needs its test rewired | `resumes_without_a_file` and `session_db_written_since` inline tests |
| Refusals (`start/refusals.rs`, 229, 0) | Sentences, folder gate, terminal check, probe and seams | 3, 7 | change | Cut `reachable`, `stated_answer_for_test` (`GOALCUP_TEST_PROBE_ANSWERS`), `offline_upstreams_for_test`, `default_host_warning`. Keep terminal and folder checks. | ~90 / 0 | low | probe tests below |
| Server (`start/server.rs`, 546, 0) | Prep checks, repo ask, approval, open and continue on the server | rules, 5 | keep | It is the contract with the Worker; the stale-segment walk in `open_segment` handles a real failure (D73). | 0 | | |
| Project folder (`start/project.rs`, 363, 0) | The D71 folder rule and token reader | rules, 4 (token redacted by value) | change | Replace the hand-written gitignore matcher (`ignored`, `glob`, `class`, `trim_unescaped_spaces`, about 100 lines) with "one of these exact lines". Keep the token reader: it feeds the redactor (D71 two run secrets). | ~100 / 0 | medium: a participant with an unusual but valid `.gitignore` is refused; the sentence must say the exact line to add | none (its tests are elsewhere; `start_launch` folder tests keep working with `.env*`) |
| Proxy start (`start/proxy.rs`, 317, 120 inline) | rlimit raise, `Proxied::start`, admission wait, admission line | 1, 2, 3 | change | Raise the soft limit to the hard limit (clamped for macOS) instead of 2,304. | 0 / 0 | low | none |
| Setup (`src/commands/prep.rs`, 1,301, 63 inline) | Four steps: login, harness, harness home, machine | 6, 7 | keep, with three cuts | Cut the old `github.json` notice (no backward compatibility, `prep.rs:436-445`), the hidden `prep` alias, and turn the filesystem `[fail]` into `[warn]` (start already only warns, and setup saves `prep.json` before the machine step, so the fail never gated anything, `prep.rs:413,416`). Fedora's btrfs and container sandboxes hit it (`docs/research/PARTICIPANT-REVIEW-2026-10-21.md:20`); the Vercel Sandbox filesystem type is unverified (`docs/research/VERCEL-COMPAT.md:39`). | ~20 / ~50 | low | `the_old_name_prep_runs_setup`, `setup_names_an_old_github_token_file_and_never_prints_the_token`; `prep_fails_a_folder_on_an_unsupported_filesystem...` changes to a warn test |
| Model host reader (`src/model_host.rs`, 894, 646 inline) | Reads the one model URL from the harness config; refuses what the proxy cannot record; host split for probes | 1, 7 | keep the reader, cut the DNS check and the start half of the split | The reader is how a gateway user is recorded at all (D62, D74). The DNS loopback check is a point-in-time lookup with no integration test (`start/phases.rs:125-129` says so). `Caller` and `provider_hosts` exist to split start from setup; with no probe at start, setup needs a 10-line rule. | ~110 / ~170 inline | low for the cuts. Separate gap: Codex profiles and Hermes extra providers are not read (phase 2 notes, design 3), so a subagent on another provider tunnels unrecorded. | `refuse_if_on_this_computer_uses_the_name_and_reason`, `a_name_on_this_computer_from_a_file_says_which_file_to_edit`, `start_never_probes_sign_in_hosts_and_setup_does`, `the_default_host_warning_names_the_command_that_is_run_again` |
| Preflight (`src/preflight/`, 277, 0) | Check type, pin gate, `--dev` door, PATH walk, V1 probe-file sweep | rules, 7 | keep, cut the sweep | `sweep_stale_probes` (`preflight/probe.rs:7-29`) cleans files a V1 build could leave. No backward compatibility now. mod.rs is 36 lines of history. | ~50 / 0 | none | none |
| Launch (`src/launch.rs`, 1,994, 96 inline) | Builds argv and env from nothing, CA vars, configure, launch prep, after-exit, spawn gate | 1, 4, 7 | change | Keep the builder and the allowlist env (it is what stops a `*_BASE_URL` in the shell from sending calls around the recorder, point 1). Cut: spawn gate (~320), `exec_footprint`/`exec_limit` (~45), `is_loopback_proxy_url` (~40, make it a debug assertion), `has_native_session` (~85, with change 34), dead `LaunchMode::Server` and `AttachOnly` text. Long V1 history comments can go too. | ~500 / ~200 | medium: the gate cut changes D40 (see section 5) | `capture_gate.rs` gate tests (lines 128-280, 483-519); `capture_registration_refused_never_execs` becomes "a refused registration kills the child" |
| Daemon client (`src/daemon/mod.rs`, 724; `spawn.rs` 710/10) | One HTTP client to the loopback daemon; starts the daemon when none answers | 1, 3, 5 | keep for Friday; add the fd raise | Removing the daemon process is the biggest possible cut on this path (client, spawn, mock, `retry.rs` 474 (corrected from 995), control routes, `daemon_client.rs`, `m2_ipc_compose.rs`), but it is a rewrite, not a cut. Not before 6 Nov. | 0 now | | |
| Daemon mock (`src/daemon/mock.rs`, 636 test-support) | In-process fake for command tests | none (tests) | keep | Needed while the daemon exists. | 0 | | |
| Retry policy (`src/retry.rs`, 474, 691 inline; corrected from 995 and 170) | Three busy schedules plus the seal retry | 3 | change later | Three schedules for one `busy` answer is more than the seven points need; one bounded retry and the seal loop would do. Not urgent. | ~250 later / ~450 (corrected to section 5's estimate; the first figure, ~400 / ~100, assumed 995 src lines; counted once, in section 5) | medium | some `retry.rs` inline tests |
| Synced-folder checks (`src/per_run_home.rs:318-489`, ~170; tests `it/per_run_home.rs:311-438`) | Refuses a project or run folder iCloud or Dropbox would upload | none | cut, or one warning | It protects the participant's token in their own cloud, which is not recording. | ~170 / ~130 | low: a synced folder can also evict files under the harness; a warning keeps that said | `a_goal_directory_in_a_synced_folder_is_refused`, `icloud_desktop_and_documents_are_refused_only_when_the_signal_is_present`, `a_folder_that_is_the_icloud_drive_folder_is_synced` |
| Shared-home rechecks (`src/home.rs:30-90`, `start/phases.rs:234-245, 282-290`) | Alias, owner and mode checked three times before create | none | change: check once | D39 already names a same-user swap after the last check as a residual, so the extra checks buy little. | ~60 / ~160 | low | `m2_cli_lifecycle.rs:1628-1788` (3 to 4 tests) |
| Reserve on the start path (`run/reserve.rs` callers in start) | 3 GiB reserve file, room check, stale release | none | cut | Owner: disk full has no special handling. The file lives in another lane; the start-side calls and tests go with it. | ~20 here / ~310 | low | `a_run_that_cannot_take_its_reserve_is_refused_before_anything_is_created`, `a_run_holds_one_reserve_file_and_releases_it_at_the_end`, `a_full_disk_with_no_upload_never_ends_the_run`, `a_stale_reserve_is_released_by_the_next_start` |
| Legacy tests | Prove removed flags stay gone | none | cut | No backward compatibility before Edition 01. | 0 / ~150 | none | `capture_reserve_bytes_is_no_longer_a_flag`, `the_goal_file_flag_is_gone`, `the_git_credential_command_is_hidden_and_prep_is_a_hidden_alias_of_setup` (rewrite to a 5-line help test if wanted) |
| `m2_ipc_compose.rs` (1,606 tests, 4 tests) | Real proxy to real sender to real receiver, with recovery epochs | 1, 3 | keep for Friday | The only test of producer recovery over real TCP. It goes only if the daemon process goes. | 0 | | |

### 4.4 The `GOALCUP_TEST_PROBE_ANSWERS` seam

- Defined at `src/commands/start/refusals.rs:172-215`, exported at `start/mod.rs:85-87`, compiled only
  under the `test-support` feature (`Cargo.toml:25-26`). In a shipped build `stated_answer_for_test`
  returns `None` and every host is probed for real.
- It reads the `env` map `goalcup start` was handed, which is the whole process environment
  (`src/main.rs:560`). So in a test-support binary (the ones `scripts/matrix.sh` and the kit build),
  anyone who sets the variable can make start skip a probe or fail one on purpose. That is fine for
  tests, but it means a test-support binary must never be handed to a participant. Nothing enforces
  that except the release build having no features.
- Only three tests use it, all in `tests/it/start_launch.rs:1026-1110`. No doc names it except the
  progress log ("needs owner awareness", `docs/plans/2026-10-29-record-everything-progress.md:58`).
- It sits beside a second seam, `GOALCUP_TEST_OFFLINE_UPSTREAMS` (`refusals.rs:150-170`), which skips
  the whole probe block. Because the DNS loopback check (row 22) lives inside that block, no
  integration test reaches it (`start/phases.rs:125-129` says so).
- The seam exists only because start probes hosts. Setup already probes the same hosts with its own
  door (`GOALCUP_TEST_PREP`, `hosts=` and `defaults=`, `prep.rs:1180-1300`). If the start probe is
  cut (row 23), both start seams, `default_host_warning`, `Caller::Start`, and the three tests go.
  Start-path seams today: 17 `GOALCUP_TEST_*` variables in `src/`.

### 4.5 Decisions behind this path, and whether they still hold

| Decision | What it justifies here | Holds under the seven points and owner decisions? |
|---|---|---|
| D1, D36 (no base URL; process-scoped proxy) | The env allowlist, `harness_arg_refusal`, loopback proxy URL | Yes. The allowlist is what keeps a shell `OPENAI_BASE_URL` from routing calls around the recorder (point 1). The loopback self-check can be an assertion. |
| D4 (one run-scoped daemon, started on demand) | `daemon/spawn.rs`, `LazyDaemon` in `main.rs:780-860` | Yes for Friday. A fresh build would likely keep the recorder in one process; see section 8. |
| D5 (agent never sees its evidence) | Row 12, the beside-goalcup check | Yes, and the daemon enforces it too. |
| D22, D30 (kill the tree, bounded) | Process registration, handle with argv and env keys | Yes: point 3 needs the seal to end the harness at the deadline. |
| D39 (shared home, lock, recheck alias, owner, mode) | Rows 30 and 31 | The lock holds (point 2: two runs' session files would mix). The triple recheck does not earn its lines; D39 itself calls the same-user swap a residual. |
| D40 spawn gate ("never leave a live harness Goal Cup never durably recorded") | Rows 55, 56, 61; ~320 lines of fork and park code | Weak. The proxy is up before the spawn, so every model call is recorded whether or not the registration lands. A refused registration can kill the child instead. Owner call. |
| D40 disk reservation, D65 pause | Rows 6, 18, 38 | No. Owner: disk full has no special handling. |
| D51 (argv and env are data; review is the control) | Row 49 | Yes. |
| D53 (record the build, do not pin it) | Row 33 | Yes. |
| D57 (chat mode; fixed goal sentence) | Rows 20, 52, 59 | Chat mode holds (point 7). The fixed goal sentence only exists because the v5 Launch record needs a goal stream. With protocol changes allowed, drop it. |
| D60, D71 (two run secrets by value) | Row 36, the token reader in `project.rs` | Yes (point 4). |
| D61 (no key forwarded; configure before each launch) | Row 45 | Yes, with a point 7 cost: a participant's exported `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` never reaches the harness; they must sign in inside the isolated home during setup. That is how the docs tell them to do it. |
| D62, D74 (model host from the harness config, defaults first, refusals) | Rows 7, 22, 23, 24 | The reader and its main refusals hold (point 1). The DNS loopback check and the start probe do not serve a point. Phase 2 notes (design 3) show Codex profiles and Hermes extra providers are not read: a gap against point 1 and point 2. |
| D66 (time-left file) | `time_left_file` env, skill install | Yes, small. |
| D67 (recommend no prompts) | Setup advice | Yes: a harness waiting on an approval prompt for a week records nothing (point 7). |
| D70 (`--dev` never competes; old versions refused) | Rows 9, 19, 35 | Yes, and the Worker enforces both. |
| D71 (project folder is the workspace; folder rule; token) | Rows 11 to 17, 26 | The rule holds for the contest. The hand-written gitignore matcher and the sync check are more than the rule needs. |
| D72, D73 (launch prep; resume on continue; push lines never lost) | Rows 34, 45, 54 | Launch prep holds (Hermes records only through it). Resume sniffing can be simplified: resume when the run has model calls, fall back once if it fails. |

### 4.6 Hidden couplings (what else breaks if a part goes)

- **Start probe (rows 23, 24).** `model_host::provider_hosts` and `Caller` are shared with setup's
  machine step (`prep.rs:1013-1060`); keep a setup-only version. Tests in `start_launch.rs:1026-1110`
  and `m2_cli_lifecycle.rs:679-713` (`an_unreachable_provider_host_refuses_the_start_before_anything_is_created`).
  Many tests set `GOALCUP_TEST_OFFLINE_UPSTREAMS` through the rigs (`tests/common/m2_cli_support`,
  `tests/common/support`, `tests/full/*`); removing the seam means removing it from the rigs in the
  same change. `continue_preflight` (`refusals.rs:40-67`) also calls the DNS check.
- **Spawn gate (rows 55, 56, 61).** `register_and_release` and `Forked::unlaunched` assume a parked
  child; `forked.rs` `sealed_launch` and `released_child.rs` (`end_released_child`) still matter for a
  released child on a sealed entry. `tests/full/capture_crash_campaign.rs` (1,465 lines) drives real
  binaries through the gate and may assert the parked state; not read in full here (unverified).
  The `process_start` identity is taken at fork today; a plain spawn takes it right after exec, which
  `handle()` already does for `prep_launch`.
- **Goal stream (row 52).** Daemon (`record_capture_goal`, Launch written only after a goal), protocol
  (`CaptureGoalRequest`, `SUFFIX_CAPTURE_GOAL`, the Launch record's goal stream field), verifier
  (Launch reader), the mock (`MOCK_GOAL_PRODUCER`, `daemon/mock.rs:60`), `retry` grep test
  (`start_launch.rs:27-62`), `a_lost_goal_answer_never_spawns_and_is_never_retried`
  (`m2_cli_lifecycle.rs:837-868`). The Worker and the receipt: unverified whether they read the goal
  stream (D63 binds receipt and Launch on connector fields only).
- **Reserve.** `run::watch` (disk pause), `goalcup status` disk lines, `run/reserve.rs` (956 lines),
  `tests/full/t25_disk_full.rs` (1,173 lines), `scripts/t25-disk-full.sh`. Another lane owns these.
- **Filesystem fail in setup.** `folder_gate.rs` is shared by setup, start and continue. Its reason is
  not written in DECISIONS; my reading is that it exists for the reserve file and file-change times
  (unverified). If the reserve goes, recheck whether any filesystem rule is still needed.
- **Session sniffing on continue (row 34).** `participant::Opens` drives the launch block wording and
  `shows_push_block`; `tests/it/start_launch.rs:1217-1261` tests the "no session opens the normal
  way" case.
- **Old `prep` alias and git-credential.** `main.rs` hidden commands, `account/github.rs`
  (`github.json` loader kept "until after 15 Nov", D71). With no backward compatibility both can go.

### 4.7 What must change together with the protocol, verifier, Worker or client

1. **Drop the chat goal stream.** Protocol: remove `CaptureGoalRequest` and the goal-stream field on
   Launch. Daemon: record Launch without a goal. Verifier: stop requiring the goal stream. Client:
   remove `CHAT_MODE_GOAL` and the goal call. All four in one change.
2. **Open-file limits for thousands of subagents (owner decision).** The CLI raises its own soft
   limit only to 2,304 (`start/proxy.rs:30`, `MAX_CONNECTION_TASKS = 1024`, `src/proxy/mod.rs:112`), and
   it does so **after** the daemon was started: `LazyDaemon` starts the daemon on the first call,
   `create_capture_workspace` at `start/phases.rs:409`, and the raise is at `start/phases.rs:516`. The
   daemon never raises its own limit (no `setrlimit` anywhere outside `start/proxy.rs`). So on macOS,
   where a shell's soft limit is often 256, the daemon that holds the native file streams and chain
   files runs at 256. Raising the native caps (32 open, 128 per entry, 1,024 files) without this will
   hit EMFILE. The daemon must raise its own soft limit at start (to the hard limit, clamped to
   `kern.maxfilesperproc` on macOS, since `setrlimit` above `OPEN_MAX` fails there), and the CLI should
   raise to the hard limit too. Daemon and client change together.
3. **More than one model host per run (point 1 and 2 gap).** Reading every provider table the config
   names (Codex profiles, Hermes `providers:`) needs `model_host_from` to return a list: schema
   (`goalcup-protocol` harness schema), `model_host.rs`, `start/proxy.rs` `approved_hosts`, and the
   verifier's host expectations. Phase 2 notes, design 3, is the plan; the owner has not decided it.
4. **Spawn gate removal.** D40 text changes; the daemon's registration route stays as it is; the
   crash campaign tests change. No protocol or Worker change.

### 4.8 The smallest start path that still meets the points it serves

For Friday, keeping the daemon process:

1. Load the connector once. Refuse WSL1 and a Windows drive.
2. Read the model service from the harness config; refuse http, loopback, IP literal, Goal Cup's own
   host, sign-in host.
3. With a server: `prep.json` exists; the folder rule (open run, beside goalcup, Git work tree,
   empty kit, `.env*` covered by one known line, token line for a real run); `--dev` not in the
   competition; ask the repo; browser approval; folder rule again.
4. Pin gate; terminal held; harness on `PATH`.
5. Lock the shared home (one owner and mode check); make the run folder; open the run on the server;
   create the capture entry; bind the receipt.
6. Raise the open-file limit to the hard limit; start the proxy and sender; wait for admission;
   configure and launch prep; build the plan.
7. Spawn; register; on a refused registration, kill the child and seal. Hand over the terminal.

That drops about 1,000 src lines and about 1,300 test lines from this lane (estimate, section 10),
and roughly 25 of the 62 steps. Every remaining refusal either protects a point (1, 2, 3, 7) or is a
contest rule the Worker also checks, where the local check saves the participant a wasted approval.

A fresh build would go further, after Friday: put the recorder in the `goalcup` process (no daemon,
no control API, no client, no mock, no busy retries, no IPC sender), which removes about 3,500 to 4,000 src
lines on this path alone (`daemon/` 1,434, `retry.rs` 474 (corrected from 995), `daemon/mock.rs` 636, control types, the
sender) and `daemon_client.rs`, `m2_ipc_compose.rs` and much of the rig. Risk: D3 and D4 kept the
daemon small and separate so that a crash of the CLI leaves a sealable chain; one process loses
that, and the crash recovery path (`goalcup seal` reopening an entry) would need a new design. Not
for 6 Nov.

### 4.9 Blind spots and what could go wrong

- **Cutting the start probe.** A network that worked at setup and fails at start now shows as the
  harness's own connection errors and an upstream-outage gap, not a clear refusal. Setup still
  names a firewall such as Little Snitch, from the same binary.
- **Cutting the DNS loopback check.** A participant could point a name at a local relay through
  `/etc/hosts`. The host name is still in the record, and the relay's traffic to its provider is not
  recorded. Judges would have to spot it.
- **Cutting the spawn gate.** A harness can run a few milliseconds before the daemon knows its pid.
  If the daemon then refuses, the child is killed; if the CLI dies in that window, the harness is
  orphaned with its proxy gone (it cannot reach the model). The subreaper on Linux and the
  terminal hangup cover most of it; macOS has no subreaper.
- **Gitignore lines.** A fixed set may refuse a valid `.gitignore` written by a template. The refusal
  must name the one line to add.
- **The env allowlist (point 7).** Keys exported in the shell do not reach the harness. This is
  deliberate (D61), but it is a "not the way people use it" cost. Passing more of the environment
  would risk a `*_BASE_URL` routing calls around the recorder, so it is an owner call, not a cut.
- **The Vercel Sandbox run.** Its filesystem type is not documented
  ([Vercel Sandbox concepts](https://vercel.com/docs/sandbox/concepts),
  [system specifications](https://vercel.com/docs/sandbox/system-specifications)); if it is overlayfs,
  setup prints `[fail]` today while start only warns. A chat harness needs a TTY: use
  `sandbox exec --interactive --tty` and tmux (`docs/research/VERCEL-COMPAT.md:43`). The 2-hour run
  report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any branch yet, so
  none of this is measured.
- **What I did not read in full:** `finish.rs`, `forked.rs`, `account/device.rs`,
  `tests/full/capture_crash_campaign.rs`, the daemon side of `create_capture_workspace`. Verdicts on
  them are from headers and call sites.

### 4.10 Short list

**Top cuts (estimated src / test lines)**

1. Spawn gate (fork, park, release; plain spawn, register, kill on refusal): ~340 / ~230.
2. Start-time host probe, DNS loopback check, both start seams (`GOALCUP_TEST_PROBE_ANSWERS`,
   `GOALCUP_TEST_OFFLINE_UPSTREAMS`) and the start half of the host split: ~230 / ~300.
3. Reserve calls and room check on the start path (with the owner's disk-full decision): ~20 here,
   plus the reserve module in its own lane / ~310 here.
4. Synced-folder refusal: ~170 / ~130.
5. Hand-written gitignore matcher: ~100 / small.
6. Session sniffing on continue (`has_native_session`, `state.db` mtime): ~130 / ~60.
7. Shared-home double recheck: ~60 / ~160.
8. Legacy and V1 leftovers (old `github.json` notice, `prep` alias test, removed-flag tests,
   `sweep_stale_probes`, ARG_MAX check, loopback self-check): ~120 / ~240.

Total for this lane: about 1,000 src and 1,400 test lines, with the goal-stream cut below adding a
little more across crates.

**Top changes**

1. Raise the open-file limit to the hard limit in both the CLI and the daemon, before the daemon
   opens anything (section 7, item 2).
2. Setup's filesystem check: `[warn]`, not `[fail]`, or remove it with the reserve.
3. Drop the fixed chat goal stream across protocol, daemon, verifier and client.
4. Skill install and other "nice" steps warn on error instead of refusing the start (point 3).
5. Later: one busy retry instead of three schedules (`retry.rs`).

**Must add for the seven points**

1. Daemon `RLIMIT_NOFILE` raise at its own start (points 2 and 3): without it, raising the native
   stream caps fails on macOS.
2. A plain "not recorded" line at finish for every tunnelled host that was not approved, uncapped
   (point 6; today only the top five by bytes show, per the phase 2 notes, `src/proxy/tunnels.rs:23`).
3. Owner decision, then code: read every model endpoint the harness config names, not only the
   selected one (points 1 and 2; Codex profiles and Hermes providers, phase 2 notes design 3).

Overlap: the reserve module and `t25_disk_full.rs` are counted in section 5; `retry.rs` is counted once, in section 5. The legacy `prep` alias and `github.json` notice are different lines from section 5's `git-credential` cut. The one-process recorder estimate overlaps sections 2 and 3.

## 5. Other CLI commands, `run/`, `participant/`, `account/`

"src" means product lines with inline `#[cfg(test)] mod tests` blocks removed; inline tests are counted as tests. About 30 to 40 percent of every src count is doc comment, so real logic is smaller.

### 5.1 The area in numbers

| Area | src | inline tests | integration tests (crates/goalcup-cli/tests) |
|---|---:|---:|---:|
| commands other than start and prep (continue_run, status, time_left, mcp, entries, seal, doctor, login, upload, run_repo, mod) | 3,363 | 745 | status_views 1,869, seal_scan 206, preflight_doctor 478, login 159, restarts 497, native_silence 152 |
| run/mod.rs, run/watch.rs | 1,855 | 1,377 | (driven through lifecycle suites) |
| participant.rs + participant/ | 1,788 | 702 (participant/tests.rs) | |
| account/ | 1,018 | 18 | account 1,195 |
| top-level src files (not proxy, model_host, launch) | 6,113 | 1,808 | per_run_home 438, ca_bundle 88, harness_version_probe 195, process_handoff 147, env_resolution 217 |
| **Total in scope** | **14,137** | **4,650** | **5,841** |

Not in scope but coupled and named below: run/pause.rs (211 src, 149 tests), run/reserve.rs (533 src, 423 tests), run/time_left.rs (260 src, 170 tests), preflight/ (277 src), daemon/ (section 2), commands/start and prep.

### 5.2 Eleven commands: which does a participant need?

The tree is `setup` (alias `prep`), `login`, `start`, `status`, `time-left`, `mcp`, `continue`, `upload`, `entries`, `seal`, `doctor`, plus the hidden `git-credential` (crates/goalcup-cli/src/main.rs:82-249).

| Command | A participant needs it? | Why |
|---|---|---|
| setup | yes | prepares harness, key and machine (prep lane) |
| start | yes | the run (start lane) |
| status | yes | point 6 while the harness owns the terminal |
| continue | yes | a week-long run outlives a closed window, a crash or a reboot (points 1, 2) |
| upload | yes | point 5 when the end-of-run upload could not finish |
| login | no | `start` asks for a browser approval anyway; login is a second approval that only stores a URL and a handle. Worse, with no `server.json` a competition start records locally and never uploads (crates/goalcup-cli/src/commands/start/server.rs:35-41), which works against point 5 |
| seal | no as a command | the participant ends a part by quitting the harness. The one real job, sealing a part whose host died, can move into `upload` and `continue` (continue already does it: crates/goalcup-cli/src/commands/continue_run.rs:345-406) |
| entries | no | a debugging list; `status` and `continue` default to the newest run |
| doctor | no | overlaps setup; setup already calls its one useful check (`executable_check`, crates/goalcup-cli/src/commands/prep.rs:566, 611, 688) |
| time-left | owner call | serves none of the seven points. D66 is a current owner ruling. A deadline line in the pasted push block gives the agent the same fact with no runtime code |
| mcp | owner call, cut first | a 273-line MCP server for one value, plus a connector schema field and setup advice |
| git-credential (hidden) | no | answers git for clones made by 0.6.1; "no backward compatibility before Edition 01" retires it now |

Answer: five commands (setup, start, status, continue, upload). Six if the owner keeps `time-left`.

### 5.3 Verdicts by part

Format: Part (path, src, tests) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go.

#### 5.3.1 Commands

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| `continue` (commands/continue_run.rs, 417, 77 inline + restarts.rs 497) | Starts the next part of an open run on the same machine with the saved run credential; seals or closes earlier parts first; resumes the harness session (D73 resume rule) | 1, 2, 3, 7 | keep | A week of agent work will hit closed windows and reboots | ~30 / ~0 (drop 0.7.0 compat wording) | low | none |
| `status` (commands/status.rs, 764, 50 inline + status_views 1,869) | Asks the daemon for a signed status document, checks it against a run key pinned in entries/<id>.json, refuses to render if it fails, prints a participant block from state.json, then a technical "Details" block from the signed document | 6 | change | Keep the participant block (state.json). Drop the signature check, the entry-id cross-check, the daemon call, the technical block and the stale/clock-skew logic: they defend against a rogue local process on the control port (D24, D25), which none of the seven points asks for. Add plain counts of what was recorded (section 7) | ~410 / ~1,050 | medium: packaging/install.sh parses `status --json` (`health`, `capture.run`, packaging/install.sh:100-160) to refuse an update over a live run; it must read the new JSON. scripts/test_install.py pins the old shape | status_views: the 9 signed-document and entries tests (lines 43-482, 603-811) shrink to 2 or 3 on state.json; test_install.py fixtures |
| `upload` (commands/upload.rs, 343, 83 inline) | Sends what is not yet acknowledged from the server's offset; `send_leftovers` sends finished parts for 3 s before most commands (main.rs:355-384) | 5 | keep, small change | Point 5 needs a manual retry. Absorb the "seal a part no host holds" job from `seal` (+~40 src) | -40 / 0 | low | none |
| `seal` (commands/seal.rs, 586, 198 inline + seal_scan 206; participant/seal.rs 79) | Asks y/n, asks the daemon to finish a part, terminates the harness through the daemon, prints the verify line; refuses inside the harness (D66) | 3 indirectly | change: cut the command, keep `finish_locally`, `seal_locally(_until)`, `report_local_finish`, `render_termination` (used by start/finish.rs:467-480 and continue_run.rs:383), move them to start/finish | Quitting the harness ends a part; continue and upload close parts with no host | ~330 / ~350 | medium: scripts/matrix.sh:167-729 uses `seal --yes` and `entries`; the kit must end the harness the way a person does (quit or signal), which is what the brief asks for anyway. A participant who wants to stop the agent from another terminal must kill it | seal_scan.rs; seal_for_person/confirm/PartFacts inline tests; matrix.sh paths |
| `entries` (commands/entries.rs, 82, 0) | Lists daemon registry rows with run ids | none | cut | Debug list; status and continue default to newest | ~105 (with main and words) / ~75 | low: kit reads `entries --json` (matrix.sh:1424-1462); read run.json instead | status_views `entries_lists_one_line_per_entry...` |
| `doctor` (commands/doctor.rs, 471, 85 inline + preflight_doctor 478) | Per connector: pin gate, executable on PATH, decoders; daemon reachability; SSH key warning | 7 weakly | cut, move `executable_check` (~25) into prep | Setup asks the same questions and is the one step participants run | ~500 (incl. main 30, preflight report bits) / ~460 | low: scripts/smoke-release.sh:39-45 and linux-harness-smoke.sh:204 call `doctor`; switch to `setup --json`. D68's deprecation rule names "a doctor warning"; moot under no backward compatibility | preflight_doctor.rs except the pin-gate cases (keep ~100 lines in prep tests); doctor inline tests |
| `login` (commands/login.rs, 84; server.rs 212; device.rs Purpose::Login) | Device-flow approval that stores server URL and account handle in server.json | none directly; its absence breaks 5 | cut | One approval per run is enough. Make the server a constant with an organizer override variable, so every run uploads | ~380 (login 84, server ~170, device login branch ~40, prep step ~40, words ~30, main ~15) / ~370 | medium: the Worker's login purpose and the kit's login step (linux-matrix.sh:237-257, matrix.sh:1825-1831) change together. Removes local-only runs, so offline organizer tests must use the loopback stand-in server | login.rs; account.rs login tests (773-984) |
| `time-left` (commands/time_left.rs, 117, 51) + run/time_left.rs file part (~130 of 260) | Reads GOALCUP_TIME_LEFT_FILE written by the watch tick, or the newest run's state | none (D66) | owner call; recommend cut and put "The run stops at <UTC>" in the prompt block | Seven points do not need it; the agent can run `date` against a pasted deadline | ~330 with launch env, watch call, main / ~220 | low to medium: a continued run in a new conversation has no pasted block after the first prompt (D73 says type only "keep going"), so the agent may lose the deadline | time_left inline; run/time_left file tests; contract_gate and launch-plan goldens that list GOALCUP_TIME_LEFT_FILE |
| `mcp` (commands/mcp.rs, 273, 201) | JSON-RPC MCP server with one tool | none (D66 fix plan v9) | cut | Largest code per value in this area; also costs `participant.time_left_tool` in the protocol schema and prep advice | ~320 (incl. prep advice ~40) / ~200 + protocol schema tests | low | mcp inline tests; schema1-optional fixture field; prep advice tests |
| `git-credential` (hidden) + github.rs legacy (~240) | Answers git from github.json for 0.6.1 clones | none | cut | D69 and D71 keep it "until after 15 Nov"; the owner's "no backward compatibility" overrides | ~250 / ~360 | low: any 0.6.1 dry run still open loses its push helper; all are test data | account.rs 358-705 (helper, old clone, old setup) |
| `run_repo` (commands/run_repo.rs, 197) | Asks the real run's repo name, checks public and empty | none of the seven; rules | keep for Friday (start lane decides) | Needed by the cup rules, not by recording | 0 | | |

#### 5.3.2 run/

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| run/mod.rs (1,137, 610) | RunRecord (run.json), RunState (state.json), problems(), advice, RunSignals shared with heartbeat, StateKeeper | 3, 5, 6 | keep, change | Core local record. Drop DiskState pause and low fields, `UploadState.stopped`, DISK_STOP_REASON reads, and the ~33 `serde(default)` "older build" markers | ~150 / ~150 | low | disk and old-state cases in inline tests; status_views `status_does_not_call_an_old_state...`, `a_run_json_without_the_test_field_still_reads` |
| run/watch.rs (718, 767) | 30 s tick: Pruner deletes acknowledged chain parts, measures disk, resizes reserve, pauses agent (D65), asks daemon recording state, writes state.json and time-left file, says native-silent and no-model-calls once | 3, 6 | change | Owner: "Disk full: no special handling." Cut Pruner, Disk trait, Tick, look_at_disk, follow_disk and the pause plumbing (watch.rs:471-718 and parts of Ticker). Keep the beat: recording state, native silence, no model calls, model count | ~400 / ~500 | medium: without in-run pruning a week of traces stays on disk until the run is sealed and sent; a full disk stops the harness (owner accepts). Deleting local evidence mid-run also stops, which is safer for point 5 | Pruner and disk tick inline tests; m2_cli_lifecycle reserve rows (start lane); full/t25_disk_full.rs 1,173 |
| run/pause.rs, run/reserve.rs (out of scope, 744 src, 572 tests) | D65 pause, 3 GiB reserve file | none after owner decision | cut, same decision as watch | Coupled: status calls `reserve::release_stale` (status.rs:96-100), seal and continue call reserve helpers | 744 / 572 (+t25 1,173) | as above | count once, by whichever lane owns them |

#### 5.3.3 participant/ (1,788 src, 702 tests)

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| participant.rs + approval, ending, launch, refusals, status, times, upload | All participant words in one place; constants read by scripts (READY_MARK by matrix.sh and linux-harness-smoke.sh) | 6 | keep, shrink with the cuts | One place for wording is good design for point 6 | ~350 (seal.rs 79, login words, disk lines, entries intro, local-run words, time-left words) / ~250 | low; keep the `pub const` marks scripts wait for | matching cases in participant/tests.rs |

#### 5.3.4 account/ (1,018 src)

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| account/device.rs (371, 18) | Device-code approval for a run or a login | 5 (the run credential) | keep, drop login branch | Run approval stays | ~40 / ~200 (in account.rs) | low | login cases in account.rs |
| account/credential.rs (176) | Run credential at 0600 in run dir | 4, 5 | keep | | 0 | | |
| account/github.rs (466) | Public-and-empty repo check (keep), legacy token helper (cut) | rules, none | change | See git-credential | ~240 / ~360 | low | account.rs 358-705 |

#### 5.3.5 Top-level src files

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk |
|---|---|---|---|---|---|---|
| main.rs (953, 21) | clap tree, exit ladder, daemon wiring, test-support doors | all | keep, shrink | Loses 5 or 6 commands and their wiring | ~200 / 0 | low |
| lib.rs (203, 9) | CliError and exit codes | | keep | | 0 | |
| config.rs (426, 190) | Config dir, control.json reader, secret-file writer, D5 checks | 4, 5 | keep | Drop github.json, grant.json, server.json rows | ~30 / ~20 | low |
| connector.rs (133, 34) | Finds a connector by declared id | 7 | keep | | 0 | |
| entries.rs (208) | entries/<id>.json: pinned run key, workspace, connector, pid, dev | none beyond status verify | change: merge the few fields seal/continue need into run.json EntryRef | Two local records for one run; the pinned key exists only for the status signature check | ~150 / ~60 | low; continue_run.rs and start/phases.rs, start/proxy.rs read it |
| home.rs (700, 295) | Shared capture homes, locks, run dir layout | 7 | keep, trim | Drop `old_home_lock_*` (home.rs:108-195, pre-fix-plan-v6 compat); the symlink TOCTOU re-checks (home.rs:396-483) are hardening against a local attacker, not a point | ~120 / ~100 | low to medium (TOCTOU checks guard the agent swapping its own home; owner call) |
| per_run_home.rs (489) | Fresh home per run (Amp uses it, connectors/amp-cli/harness.toml:58) | 7 | keep | | 0 | |
| host_log.rs (253) | Quiet terminal while chat-mode harness draws | 3, 7 | keep | | 0 | |
| ca_bundle.rs (207) | System roots plus proxy CA for SSL_CERT_FILE-style variables | 1, 7 | keep | | 0 | |
| harness_version.rs (203) | Records harness self-reported version (D53) | 5, 7 | keep | Small and useful research | 0 | |
| arming.rs (175) | Ctrl-C flag during approval | 3 | keep, trim the V1 history docs | | ~50 / 0 | none |
| folder_gate.rs (156, 68) | WSL1 refusal, Windows-drive refusal, filesystem allowlist (APFS, ext4, XFS only) | 7 | change: warn only for filesystem type | Setup fails its machine step on btrfs (Fedora and openSUSE default), ZFS, tmpfs and overlayfs (most containers) (crates/goalcup-cli/src/commands/prep.rs:1105-1111, crates/goalcup-platform/src/machine.rs:352-387). That blocks people "run the way they use them" | ~20 / ~20 | low; Vercel Sandbox root filesystem type is unverified ([Vercel Sandbox concepts](https://vercel.com/docs/sandbox/concepts)) |
| prep_file.rs (88) | prep.json | 7 | keep | Drop `work_root` compat read | ~5 | none |
| push_lines.rs (152, 61) | Writes push-lines.txt, copies to clipboard (pbcopy, wl-copy, xclip, clip.exe) | rules, 6 | change: keep file and print, cut clipboard (push_lines.rs:56-213) | Clipboard tooling is comfort, four platforms of it | ~95 / ~30 | low |
| released_child.rs (291) | Ends a child released after a seal raced in | 3 | keep for Friday; cut later with the spawn gate | The race comes mainly from a second-terminal `goalcup seal`; with that command gone the race may vanish (unverified) | 0 now | medium if cut early |
| retry.rs (474, 691) | Four busy-retry schedules for the daemon | 3 | change after the daemon decision: one schedule | Four schedules, each with its own test set | ~250 / ~450 | medium: seal under load could hit a shorter budget |
| server.rs (212) | server.json read and write | 5 | cut to a constant plus override | See login | counted in login | |
| skill.rs (86, 76) + skills/goal-cup/SKILL.md (34) | Installs a skill into Codex and Claude Code skills folders | none (D57, D66) | owner call; cut with time-left | The skill mostly explains time-left; writes into the harness home | ~86 / ~76 | low; agents lose a hint about "keep going" |
| ssh_keys.rs (192, 49) | Warns about passphrase-less SSH keys (D73) | none | cut; say it once in the guide | Not a recording concern | ~192 / ~49 | low to medium: weaker warning on a real security issue (D71 correction) |
| subreaper.rs (354, 263) | Linux: become child subreaper, sweep orphans before seal | none (D30) | cut, owner call | Nothing after the seal is recorded either way; Unresolved termination does not lower integrity (D45) | ~366 (with main) / ~263 | low; orphans keep running on Linux after a run |
| vitals.rs (158, 51) | Heartbeat RSS, CPU, open files, disk free | 3, 5 (diagnosis) | keep, drop disk_short and agent_paused | `open_files` is exactly the signal for the fd-limit risk the owner named | ~15 / ~10 | low |

### 5.4 Decisions behind this area, and whether they still hold

| D | Used by | Still holds under the seven points and owner decisions? |
|---|---|---|
| D4 (docs/DECISIONS.md:75-86) run-scoped daemon, seal starts one for crash recovery | seal, status, entries, continue | The recovery part holds; the "seal from another terminal" route does not need its own command |
| D24, D25 (DECISIONS.md:142-153) control API auth; status for humans | status signature check, entries.rs pinned key | D25's "nothing injected into the agent" holds. The forged-status threat model behind the pinned key is not a point; drop it |
| D30, D22 (132-165) termination | subreaper, released_child, seal | Not a point. D45 already says Unresolved termination does not lower integrity, so cutting subreaper costs no verdict |
| D53 (388-400) record the harness build | harness_version, doctor | Holds (cheap, research value) |
| D57 chat mode, skill | skill.rs, host_log | Chat mode holds (point 7). The skill clause is optional |
| D64 (660-690) no server error stops a run; manual upload gives up on 401/404/410 | upload, run/mod problems | Holds (point 3). The manual give-up (upload/policy.rs:257) can stay |
| D65 (692-724) pause on full disk | watch, pause, reserve, status disk lines, vitals | **Superseded by owner decision 29 Oct: "Disk full: no special handling."** Not yet written as a D entry |
| D66 (726-758) time-left, mcp, GOALCUP_IN_HARNESS seal refusal | time_left, mcp, run/time_left, skill, seal | Owner ruling still current; not a point. Ask the owner |
| D67 (760-782) prep prints mcp lines | prep advice for mcp | Falls with mcp |
| D68 (784-811) deprecation needs a doctor warning | doctor | Moot under "no backward compatibility"; amend |
| D69, D71 (813-973) git-credential stays until 15 Nov | github.rs, main GitCredential | **Overridden by "no backward compatibility before Edition 01"** |
| D70 (867-896) MIN_COMPETITION_VERSION on device request, continue not checked | device, continue | Holds; with no old runs to protect, `continue` could be checked too |
| D73 (1051-1103) push lines file, clipboard, resume rule, ssh warning | push_lines, continue, status, ssh_keys | File and resume rule hold; clipboard and ssh warning are comfort |
| D5 (88-93) agent never sees evidence | config.rs checks | Not a point, but cheap and protects the record; keep |

No D entry covers: the local-only run mode (no server.json), the four retry schedules, or the filesystem allowlist failing setup.

### 5.5 Hidden couplings

1. `login` decides whether a run uploads at all: no server.json means a competition start records only locally (crates/goalcup-cli/src/commands/start/server.rs:35-41; also start/mod.rs:420-428 picks the run folder by it). Cutting login without changing start would make every run local.
2. `status --json` is an installer interface: packaging/install.sh:100-160 greps `"health"` and `capture.run` out of the signed document to refuse an update over a live run (D70). scripts/test_install.py pins it.
3. The status signature check needs entries/<id>.json's `run_public_key` (crates/goalcup-cli/src/entries.rs:44-75), which needs the daemon's signed status route and protocol `SignedStatus`.
4. `status` mutates: it releases stale reserves and prunes finished runs before printing (crates/goalcup-cli/src/commands/status.rs:96-102). Cutting reserve or prune touches status.
5. The kit drives `seal --yes`, `entries --json`, `login` and `doctor` (scripts/matrix.sh:167-729, 1424-1462, 1825-1831; linux-matrix.sh:237-257; linux-harness-smoke.sh:204; smoke-release.sh:39-45).
6. time-left reaches six places: launch env GOALCUP_TIME_LEFT_FILE (launch.rs), the watch tick (watch.rs keep_file call), prep's MCP advice (prep.rs:722-760), the protocol schema field `participant.time_left_tool` and its check (goalcup-protocol/src/harness/schema.rs:323-332, check.rs:533-537), the connectors' harness.toml, and the harness-golden launch fixtures. Changing a harness.toml changes connector pins.
7. `send_leftovers` runs before most commands and skips inside the harness by the in-harness marker; `mcp` is exempted because a harness may hide the marker (main.rs:355-378).
8. prep calls `doctor::executable_check` (prep.rs:566, 611, 688); preflight/ is half doctor's.
9. `HeartbeatVitals.disk_short` and `agent_paused` (upload/api.rs:217-235) are stored by the Worker (server/worker/src/runs.ts, db*.ts).
10. `participant::READY_MARK` and other `pub const` words are read by scripts.

### 5.6 What must change together

| Change | Client | Protocol | Daemon | Verifier | Worker | Scripts and docs |
|---|---|---|---|---|---|---|
| Cut login, always upload | start, prep, server.rs | | | | drop the login purpose of the device flow; keep version check | matrix.sh, linux-matrix.sh, PARTICIPANT-GUIDE, D69 |
| Status from state.json | status, entries.rs merge | `SignedStatus` may become unsigned (daemon lane) | status route | | | install.sh guard, test_install.py |
| Cut seal command | main, seal.rs move, upload seals orphans | | | | | matrix.sh, guide, D66 seal clause |
| Cut time-left and mcp | launch, watch, prep, skill | remove `participant.time_left_tool` | | | | connectors, pins, goldens, HARNESS-SETUP, D66, D67 |
| Cut disk pause and prune | watch, run/mod, status, vitals | | D55 rotation unchanged | | heartbeat columns optional | D65 retired |
| Cut git-credential | main, github.rs, config | | | | | D69, D71 |

### 5.7 The smallest version that still meets the points this area serves

- Five commands: `setup`, `start`, `status`, `continue`, `upload` (plus `time-left` only if the owner keeps D66).
- `status` reads run.json and state.json only, never the daemon. It prints: run, mode, ends at and time left (server count), agent running or exited, upload state, recorded so far, problems with fixes, push lines while open, one next step. `--json` prints state.json plus problems.
- `continue` as today, minus 0.7.0 compat text.
- `upload` as today, plus: seal a part whose host is gone, then send it.
- run/: run.json (with the few EntryRecord fields folded in), state.json, a watch tick that only writes the beat (recording state, model calls, session files, gaps, native silence, no model calls, upload, heartbeat).
- participant/: the same single-place wording, with the cut commands' words gone.
- account: device approval for a run, run credential, public-and-empty repo check.
- Top level: config, connector, home (no old lock), per_run_home, host_log, ca_bundle, harness_version, arming, folder_gate (WSL1 refusal only, filesystem warn), prep_file, push_lines (file only), released_child, retry (one schedule after Friday), vitals.

Points served: 3 (nothing here stops the harness), 5 (every run uploads, leftovers and manual upload), 6 (status with counts), 7 (no filesystem failure, chat mode, continue).

### 5.8 Short list

#### Top cuts (src / tests)

1. status signature check, technical block, and entries.rs merge into run.json: ~560 / ~1,110
2. Disk pause, reserve sizing and in-run pruning in watch and run/mod: ~550 / ~650 (plus pause.rs and reserve.rs 744 / 572 and t25_disk_full 1,173 if counted here)
3. `doctor`: ~500 / ~460
4. `time-left` + `mcp` + skill (owner call on D66): ~740 / ~500
5. `login` and server.json, so every run uploads: ~380 / ~370
6. `seal` command (logic moves to start/finish and upload): ~290 net / ~350
7. subreaper: ~366 / ~263
8. git-credential legacy: ~250 / ~360
9. retry schedules to one (after Friday): ~250 / ~450
10. ssh_keys, clipboard, entries command, old lock, compat fields: ~560 / ~330

Sum: about 4,400 of 14,137 src lines and about 4,800 of 10,491 test lines in scope, before the out-of-scope disk files.

#### Top changes

- `status` reads local files only; `install.sh` reads the new JSON.
- `start` always uses the Goal Cup server (constant plus organizer override); no local-only competition runs.
- `upload` also seals a part no host holds.
- folder_gate: filesystem type is a warning in setup too.
- Put "The run stops at <UTC>" in the prompt block if time-left goes.

#### Must add for the seven points

- Point 6: `status` and the end-of-run block say plainly what was recorded so far: model calls (state.json already has `model_calls`), session files taken (main and subagents; the tick already hears `native_files`, watch.rs:48-57), gaps with reasons, bytes sent. Today the participant block has none of these counts; they sit only in the technical block (status.rs:634-680). About 40 src lines.
- Point 5: no run without upload (see login).
- Point 2 support: show `open_files` from vitals as a PROBLEM line when near the process limit, since the owner wants the open-stream caps raised and macOS allows 256 per process by default. Raising RLIMIT_NOFILE itself belongs to the start and daemon lanes.

### 5.9 What could go wrong

- Cutting `seal` removes the only way to stop the agent from another terminal without killing processes by hand. A participant on a shared screen may want that.
- Status without the signature check trusts local files; a participant could edit state.json to fool themselves only, since the server and verifier do not read it.
- If the owner keeps D66, the time-left cuts shrink to mcp only (~320 / ~200).
- The install guard falls back to INSTALL_UNSURE and proceeds when it cannot parse status; a wrong parse after the change could let an update replace goalcup during a run.
- Removing local-only runs removes the offline path organizers may use on a plane or in CI without the stand-in server.
- Line savings are estimates; doc comments are a large share of every number.

Overlap: `doctor` (~500 / ~460) contains section 8's decoder check (~71 / ~55). `run/pause.rs`, `run/reserve.rs` and `t25_disk_full.rs` are counted here once, not in sections 4 or 7. The status signature cut is the client half of section 1's optional `SignedStatus` cut (different files, one change). `retry.rs` (~250 / ~450, after Friday) is counted here, not in section 4.

## 6. Uploader (`crates/goalcup-cli/src/upload/`)

"src" excludes the `#[cfg(test)]` blocks inside the same files, which are counted as tests.

### 6.1 Scope and size

| Part | src lines | test lines |
|---|---|---|
| `crates/goalcup-cli/src/upload/mod.rs` (open, continue, receipt, abandon, give-back, keep, `wait_stopping`) | 523 | 0 |
| `upload/api.rs` (wire documents) | 291 | 51 |
| `upload/http.rs` (one HTTPS client, TLS, error kinds) | 372 | 0 |
| `upload/policy.rs` (cadence, backoff, wrap-up, test seams) | 292 | 93 |
| `upload/prune.rs` (which acknowledged parts may be deleted) | 173 | 248 |
| `upload/uploader.rs` (module doc, re-exports) | 82 | 0 |
| `upload/uploader/chunk.rs` (build one zstd chunk) | 227 | 0 |
| `upload/uploader/session.rs` (one entry's uploader, acks) | 511 | 0 |
| `upload/uploader/finish.rs` (final heartbeat, rest, seal, tunnel list) | 354 | 121 |
| `upload/uploader/heartbeat.rs` (heartbeats, `LocalDeadline`) | 308 | 109 |
| `upload/uploader/lifecycle.rs` (threads, finish, drain of an older segment) | 577 | 84 |
| `upload/uploader/local.rs` (lock, local chain end, repo head, `.sent`) | 246 | 81 |
| **Module total** | **3,956** | **787** |
| `tests/it/upload_open.rs` (28 tests; most are start admission, not upload) | | 1,548 |
| `tests/it/upload_chunks.rs` (33 tests; the uploader proper) | | 1,736 |
| `tests/it/upload_server.rs`, `upload_server_segments.rs`, `upload_server_device.rs` (65 tests of the stand-in server itself) | | 2,909 |
| `tests/full/upload_e2e.rs` (38 tests, real `goalcup start` against the stand-in) | | 2,518 |
| `tests/full/upload_worker.rs` (4 e2e flows against the real Worker under `wrangler dev`, ignored by default) | | 456 |
| `tests/common/upload_server/` (Rust stand-in of the Worker: `mod.rs` 603, `handlers.rs` 1,506, `state.rs` 405) | | 2,514 |
| **Tests total** | | **12,468** |

Compared (read only): `docs/UPLOAD-API.md` (596), `server/worker/src/chunks.ts` (228), `server/worker/src/runs.ts` (637).
The Worker has its own TypeScript tests, 5,255 lines in `server/worker/test/` (the owner's deploy note
says 276 of 276 passed, `docs/plans/2026-10-29-record-everything-progress.md:68`).

### 6.2 How it works today (short)

1. `open_run` (`mod.rs:149-175`): `POST /v1/runs`, three tries 3 s apart; the server mints `run_id`,
   `entry_id`, `official_start`, `deadline_at`, `seconds_left`.
2. The daemon creates the entry; `fetch_receipt` (`mod.rs:258-307`) gets a signed start receipt over
   the baseline head and checks every claim and the signature before `keep_run` writes it
   (`mod.rs:310-320`).
3. Beside the run, two threads (`lifecycle.rs:93-180`): heartbeats every 5 min plus the run's own
   count to the deadline (`heartbeat.rs:157-198`), and the uploader. The uploader sends whole lines
   from the server's `(offset, next_seq, head)` as one zstd frame of at most 32 MiB
   (`chunk.rs:12-215`), at once, then every 30 min (first interval random) or once 256 MiB waits
   (`policy.rs:85-98`). Each signed ack is checked and appended to `acks.jsonl`
   (`session.rs:454-481`).
4. Every failure is retried with 60 s to 15 min backoff and jitter, never ending the run's upload
   (D64; `policy.rs:280-288`, `session.rs:258-266`). A `409` re-reads the position.
5. At the stop: final heartbeat commits `(seq, head)` (`finish.rs:139-210`), the rest of the chain,
   then the seal with `seal.json`, `baseline.json` and the tunnel list (`finish.rs:110-133`,
   `finish.rs:245-333`), within a one-hour wrap-up on a sleep-proof clock (`policy.rs:203-245`).
   What is left goes with `goalcup upload`.
6. Acknowledged closed chain parts are deleted locally after 2 h, or at once under disk pressure
   (`prune.rs:47-82`, driven from `run/watch.rs:528-618`).

### 6.3 What the server needs to rebuild the story by ID and time (point 5)

The server needs only four things per entry, and it has all four today:

| Need | Where it is | Status |
|---|---|---|
| The chain bytes in order | R2 `traces/{event}/{run}/{entry}/{offset:016}-{sha256}.zst` (`chunks.ts:150-153`) plus the DB chunk index (offset, length, seq range, head, `received_at`) (`chunks.ts:72-86`) | Complete. Create-only, hash in the key. |
| Which entry belongs to which run, in which order | `segment` on each entry (`runs.ts:286-293`, `runs.ts:428-444`) | Complete. |
| Time | Each chain record's own time stamp, the server's `received_at` on each chunk and `acked_at` on each ack, the heartbeat receive time, `official_start` and `deadline_at` | Complete. Heartbeats commit the position every 5 min before the bytes arrive. |
| The end and the seal | Final heartbeat's committed `(seq, head)`, `seal-<sha>.json`, `baseline-<sha>.json`, optional `tunnels.jsonl` (`runs.ts:536-597`) | Complete. |

Subagents need nothing extra from the uploader: every native file and wire stream is a record in
the chain, with its path or stream id inside it. "The file path is the ID; the server orders by
ID and time" (owner decision, progress plan line 89) is satisfied by the chain plus the chunk index.
The uploader never looks inside records, so raising the subagent caps in the daemon changes only
the volume it carries.

The rebuild tool exists: `scripts/fetch-run.sh` joins chunks in offset order, checks each hash and
length, and writes `chain.jsonl`, `acks.jsonl`, `receipt.json`, `seal.json`, `baseline.json`,
`tunnels.jsonl` per entry (`scripts/fetch-run.sh:1-30`). The automatic "verify job" that runs
`goalcup-verify` over the server copy is not built (`docs/OPERATING.md:310`,
`docs/SERVER-DESIGN.md:159`). Until it is, nobody learns that a server copy is broken unless an
organizer runs `fetch-run.sh` and the verifier by hand.

The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any
branch yet, so the upload of a long run with many subagents is not measured. Everything about
volume below is from design targets, not from a run.

### 6.4 Verdicts per part

Format: Part (path, src, tests) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go.

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Chunk builder (`uploader/chunk.rs`, 227, covered in `upload_chunks.rs`) | Whole lines from the server's position, link-checked, one zstd frame, at most 32 MiB, streamed from a spool file | 1, 2, 5 | Keep | Small, streaming, never holds a chunk in memory. The 32 MiB cap guards a real limit: Cloudflare caps a request body at 100 MB on Free and Pro and a Worker isolate at 128 MB (https://developers.cloudflare.com/workers/platform/limits/). Say that in the code comment. | 0 / 0 | None new | None |
| Uploader session (`uploader/session.rs`, 511) | Resume from server position, send chunks, re-read on 409, check and keep acks | 5, 3 | Change | Cut the "owed ack" path (`session.rs:33-45`, `108`, `247-251`, `280-283`, `327-347`, `483-496`): when `acks.jsonl` cannot be written, log it and go on. The server keeps every ack and `fetch-run.sh` rebuilds `acks.jsonl`. Also drop `adopt_server_ack` (`session.rs:228-253`): a lost ack only weakens the local copy. | ~70 / ~95 | A local trace may miss an ack, so a local verify is weaker. The server copy is whole. | `an_acknowledgement_that_cannot_be_written_is_retried_and_its_chunk_is_not_sent_again`, `an_acknowledgement_still_owed_is_not_an_idle_upload`, `a_servers_acknowledgement_that_cannot_be_written_on_resume_is_held` (`upload_chunks.rs`) |
| Ack check (`session.rs:454-481`) | Entry, seq, head and signature under the receipt key; order kept | 5 | Keep | Cheap. It is what makes `acked_offset` trustworthy, which prune relies on. | 0 / 0 | | |
| `acked_offset` and `adopt_verified_offset` (`session.rs:196-220`, shared atomics in `lifecycle.rs`) | Tells the disk watch how far it may delete | 5 (via prune) | Keep while prune stays | Exists only for prune. If prune is cut, all of it goes (about 60 src lines). | 0 / 0 | | |
| Lifecycle (`uploader/lifecycle.rs`, 577 / 84) | Heartbeat thread, upload thread, finish, drain of an older segment | 3, 5 | Change | Five near-copies of "call, backoff, say, sleep, check the wrap-up" (`lifecycle.rs:270-311`, `lifecycle.rs:524-571`, `finish.rs:62-86`, `finish.rs:100-132`, `finish.rs:139-210`) and two copies of the "wait for the last word" loop (`lifecycle.rs:228-245`, `lifecycle.rs:442-457`). One `retry_until(wrap_up, label, call)` helper and one `wait_report` helper. | ~80 / 0 | A refactor of code that works, days before the event. Do it only with the existing tests green. | None |
| Final heartbeat, rest, seal (`uploader/finish.rs`, 354 / 121) | Commit the end, drain, seal with documents read first | 5 | Keep, dedupe as above | The order (documents read before the drain, end committed before the seal) is what lets the server seal a complete copy. | (in the row above) | | |
| Tunnel list in the seal (`finish.rs:216-333`) | Sends `tunnels-<entry>.jsonl`, checked: name, no-follow open, 8 MiB, first line | 6, 1 (shows what was not recorded) | Keep for Friday | Merged on 28 Oct as Task 3, live PASS. The Worker re-checks size and first line (`runs.ts:70-84`), so the client's first-line parse (`finish.rs:302-319`) is a duplicate, but it costs 20 lines and avoids a 400 loop. After Edition 01, consider writing tunnel notes as chain records: then the seal field, the side object, both 8 MiB caps and both first-line checks go. | 0 / 0 now | The proxy's `MAX_TUNNEL_NOTES = 10_000` (`proxy/tunnels.rs:20`, other lane) is a cap of our own; a week with thousands of subagents can pass it. It keeps a dropped count, so it is not silent. | |
| Heartbeat and `LocalDeadline` (`uploader/heartbeat.rs`, 308 / 109) | Heartbeat every 5 min with chain end, `recording`, `model_requests`, vitals; the run's own count from `seconds_left` | 3, 5, 6 | Change | Cut the "older server" fallbacks: `LocalDeadline::of`, `until`, `until_at` and the `None` arms of `at_start` (`heartbeat.rs:231-280`). Make `seconds_left` required in `OpenRun`, `ContinueRun` and `HeartbeatAnswer` (`api.rs:115-139`, `api.rs:247-262`). The deployed Worker always sends it (`runs.ts:276-282`, `runs.ts:136-141`). | ~40 / ~45 | An answer without `seconds_left` becomes a malformed answer, which the start already refuses cleanly. | `until_on_a_skewed_date_counts_what_the_date_says_on_the_continuous_clock` and part of `the_first_count_is_the_servers_time_left_and_never_the_date` (`heartbeat.rs` inline) |
| Heartbeat vitals `agent_paused`, `disk_short` (`api.rs:208-217`) | Tells the server about the D65 disk pause | none (D65) | Cut if the run lane removes the D65 pause | Owner: "Disk full: no special handling." If the pause goes, these two fields, the Worker's disk-pause tracking and the `disk_full` email go with it. Keep `rss_bytes`, `cpu_ms`, `open_files`: with the stream caps raised, `open_files` is the one live signal of handle exhaustion (macOS default 256 per process). | ~10 / ~20 | Worker and SQL keep accepting the fields, so a client-only cut needs no deploy. | `the_heartbeat_carries_the_vitals_it_could_read` shrinks |
| `repo_head` (`uploader/local.rs:132-246`, call sites in `finish.rs:146`, `commands/start/phases.rs`, `commands/continue_run.rs`) | Reads `.git/HEAD` and refs by hand, notes the commit when the run stops, sends it in the final heartbeat | none | Cut | Redundant: `seal.json` already carries `git.commit` and an uncommitted digest (`crates/goalcup-protocol/src/seal.rs:17-19`, `:70-73`, D56 as amended by D71), and the Worker records the judged commit from GitHub's own push times at the deadline (`docs/UPLOAD-API.md:500-522`). Send `repo_head: null`; the Worker already accepts null (`runs.ts:461-462`), so no deploy. | ~115 / ~210 | An unsealed run (killed host) loses the client's own view of the commit. The judged commit never used it. | `a_commit_noted_as_a_segment_ended_is_not_replaced_by_a_later_one`, `a_commit_that_cannot_be_noted_is_said` (`local.rs` inline), `the_repo_head_is_read_from_the_git_files` (`upload_chunks.rs`), `the_final_heartbeat_with_the_commit_reaches_the_server_before_the_server_seal`, `the_old_segments_final_heartbeat_carries_the_commit_it_ended_with` (`upload_e2e.rs`; keep the seal-order part of the first as a smaller test) |
| Lock, local end, `.sent` (`local.rs:14-131`, `184-191`) | One uploader per entry; last whole record; "everything sent" marker | 5 | Keep | Small and load-bearing (start versus a manual `goalcup upload`; `continue` reads `.sent`). | 0 / 0 | | |
| Open, continue, receipt, abandon (`mod.rs:96-307`) | Server calls with short retries and checked answers | 5 (and the contest rules) | Keep | The server must mint ids and the start time; the receipt key verifies every ack. Checking the receipt claims enforces D63 before the trace is written. | 0 / 0 | A server that is down refuses a start. That is by design, not a breach of point 3, which is about a running harness. | |
| Give-back of a failed start (`mod.rs:333-492`) | `GiveBack` abandons a run whose start failed; `pending-open.json` plus the saved credential let the next start give back a run a killed start left open | none | Change | Keep the one-shot `GiveBack::give` (abandon once, best effort). Cut the pending note, `undo_open`, `give_back_pending` and `forget_credential` (`mod.rs:338-349`, `395-492` mostly). A competition start replaces the account's open competition run anyway (`docs/UPLOAD-API.md:206-210`), and the server closes the orphan 24 h after its deadline. | ~110 / ~200 (plus ~30 src in `commands/start/server.rs`, start lane) | An orphan left by a killed start may send a `started` email and, until the next start replaces it, a `silence` email. If the window has closed, the orphan sits open with nothing in it until it closes as `incomplete`. Ask the owner. | `a_run_that_could_not_be_given_back_is_given_back_by_the_next_start`, `a_note_for_another_server_or_an_ended_credential_is_dropped_with_its_credential`, `a_failed_open_keeps_its_note_and_credential_until_the_server_takes_the_run_back` (`upload_open.rs`), `a_run_left_open_by_a_killed_start_is_given_back_by_the_next` (`upload_e2e.rs`) |
| `wait_stopping` (`mod.rs:487-523`) | Waits for the harness; the stop channel asks the daemon to seal (D22, D57) | 3 | Keep, move | It is not upload code; it belongs in `commands/start`. No lines saved, one less surprise. | 0 / 0 | | |
| Wire documents (`api.rs`, 291 / 51) | Serde types, field for field as the contract | 5 | Change | Cut `ProxyVersion`'s dual name (`api.rs:14-69`): send only `proxy_version`. "No backward compatibility before Edition 01." Same in the Worker (`util.ts:84-91`), the stand-in (`handlers.rs:345-417`) and `account/device.rs`. Also drop "an older server leaves it out" options (see heartbeat row) and `UploadRecord.tunnels_file`'s "an older record" default. | ~55 / ~75 (+ ~40 stand-in, ~6 Worker) | A client older than the Worker change cannot open a run. Every run so far is test data, so that is fine; release the client and the Worker together. The DB column stays named `tracer_version` (`sql/goal-cup-tracer-upload.sql:83`, `:143`); do not rename it this week. | `the_version_is_sent_under_both_names`, `the_version_reads_from_either_name` (`api.rs` inline), `an_open_reads_the_version_under_either_name` (`upload_server_segments.rs`) |
| HTTP client (`http.rs`, 372) | ureq with this crate's rustls, no redirects, no env proxy, bounded body, error kinds | 4 (no token to a redirect or a proxy), 5 | Keep | Turning on ureq's own TLS feature would save the 60-line connector but risks a second rustls version in the tree. The bounded read and the redirect ban are security, not caps. | 0 / 0 | | |
| Policy (`policy.rs`, 292 / 93) | Interval, eager size, backoff, wrap-up, test seams | 3, 5 | Change (constants) | Shorten `interval` from 30 min to 5 min, the heartbeat's pace: less of the record sits only on one disk, and the server's copy lags less. Cost: about 1,500 runs x 12 puts an hour x 168 h, about 3 million R2 writes per edition (price per million not fetched here; unverified). The test seams (`from_test_spec`, `TEST_LOCAL_WRAP_UP`, `WrapUpSpec`) could be one env var. | ~30 / 0 | More PUTs and more objects per run. The first-interval spread (`policy.rs:157-197`) still matters for 1,500 runs that start together: keep it. | None |
| Prune (`prune.rs`, 173 / 248; `run/watch.rs` `Pruner`, other lane) | Deletes acknowledged closed parts 2 h after the ack, or at once under disk pressure | 3 (keeps a week-long run off a full disk), 5 | Change, do not cut | The design target is 1 GB/h of chain (`docs/plans/2026-10-20-event-readiness.md:95`); a 168-hour run is then about 168 GB raw on the participant's disk without prune. One rule is enough: delete a closed part as soon as a signed ack covers it. That drops `AckAges`, the 2-hour clock and the pressure branch, which is the "special handling" the owner declined. | ~50 / ~110 | No local safety copy if the server loses a stored chunk before an organizer fetches it. R2 is create-only and the ack is signed, so the window is small. If the owner wants the 2-hour copy, keep it and drop only the pressure branch. | `only_parts_acknowledged_two_hours_ago_are_deleted`, `pressure_deletes_young_acknowledged_parts`, `a_part_keeps_its_clock_when_the_ones_before_it_are_gone`, `ages_are_kept_for_each_entry_and_forgotten_with_the_part` (`prune.rs` inline), the age tests in `run/watch.rs` |
| Stand-in server (`tests/common/upload_server/`, 2,514) | A Rust copy of the Worker's rules so Rust tests run without Node | test support | Change | Keep it as a fake for client tests, but stop maintaining it as a second implementation of every rule. Remove what only its own tests use: device approval knobs, practice and test-mode windows, replacement rules, machine-fact validation, UTF-16 caps, the legacy member token. Estimate, not measured: ~500 lines. TODO item 8 already plans "drop one copy of the rules" (`TODO.md:242`). | 0 / ~500 | Drift: a client test can pass against a fake that disagrees with the Worker. `upload_worker.rs` runs the real Worker in `gate full`; keep that, and add one or two flows to it (see adds). | |
| Stand-in's own rule tests (`tests/it/upload_server.rs` 1,419, `upload_server_segments.rs` 961, `upload_server_device.rs` 529) | 65 tests of the fake, not of the product | none | Cut | They test the stand-in. The Worker's own 5,255 lines of tests own the rules. | 0 / 2,909 | Lose the "cheap way to find out" when the fake and the contract differ. Covered by `upload_worker.rs` against the real Worker. | All 65 |
| E2E (`tests/full/upload_e2e.rs`, 2,518) | Real `goalcup start` and `continue` against the stand-in | 3, 5, 6 | Keep, trim | They test the promises: uploaded while recording, server away mid-run gets everything once, lost reply no duplicate, finish by `goalcup upload`, stop at deadline with or without the server, continue, rotated parts through a crash. Trim only the give-back and repo-head ones listed above. | 0 / ~180 | | See rows above |
| `upload_open.rs` (1,548) | 28 tests; most are start admission (folder, repo, key file) | lane of `start` | Keep (other lane) | Only the three pending-note tests are upload code. | 0 / ~146 | | See give-back row |
| `upload_chunks.rs` (1,736) | The uploader's promises | 3, 5 | Keep, trim | Trim the owed-ack and repo-head tests only. | 0 / ~125 | | See rows above |

### 6.5 Decisions behind the area, and whether they hold

| D | What it says here | Holds under the seven points and owner decisions? |
|---|---|---|
| D64 | No server answer stops a run or ends its upload; everything is retried with backoff | Yes. It is point 3 in server form. Keep. |
| D65 | Disk-full pause, cited in `api.rs:210-211` and the prune pressure path | No, per owner ("Disk full: no special handling"). Removing it removes `agent_paused`, `disk_short`, the prune pressure branch and the Worker's `disk_full` email. Decided in the run lane. |
| D55 | Chain rotates into parts; acknowledged parts may be deleted; verifier reports `chain_pruned_locally` | Yes, because of volume (section 4, prune row). Note: `uploader.rs:3` cites D56 for this; it should cite D55. |
| D74 | Tunnel list written at finish and sent with the seal, at most 8 MiB, 10,000 rows | Mostly. The 10,000-row cap is a cap of our own; raise it or move the notes into the chain after the event. The 8 MiB server cap guards Worker memory (128 MB isolate), a real cost; say so. |
| D63 | A receipt must describe the Launch it anchors | Yes. `fetch_receipt` checks the claims first. |
| D22, D57 | The daemon is the only terminator; chat-mode wait resumes a self-stopped harness | Yes; `wait_stopping` uses them. |
| WP11 | The server is the clock (`seconds_left`) | Yes; and it makes the "older server" fallbacks dead code. |
| D56 / D71 | The seal records the workspace commit | Yes; and it makes `repo_head` redundant. |
| D70 | `--dev` never competes | Yes; one boolean. |
| OD7 | Empty-repo check at open | Worker only; no client code. |

### 6.6 Hidden couplings (what else breaks if a part goes)

- **Prune** is read by `run/watch.rs` (`Pruner`), `commands/status.rs`, `commands/upload.rs` and
  `commands/start/finish.rs` (`clean_finished_runs`). It is why `acked_offset`,
  `adopt_verified_offset`, `DrainHandles`, `local_link_before`'s part search, the "first bytes were
  already deleted" refusal in `build_chunk` (`chunk.rs:110-117`) and the verifier's
  `chain_pruned_locally` exist. Cut prune and all of those go too; part rotation (D55) then loses
  its main reason, which the daemon lane should weigh.
- **`.sent` marker** is read by `goalcup continue` (do not send an older segment again) and by
  `status`. Keep it with any change to `finish`.
- **`EndedGate`** couples the heartbeat thread and the final heartbeat: no ordinary heartbeat may
  follow the final one (`heartbeat.rs:15-20`). A retry-helper refactor must keep the lock held across
  the final call.
- **Receipt key** in `UploadRecord` flows into every ack check. Abandoning receipts would make acks
  unverifiable locally.
- **`ServerClient::bearer`** is also used by `account/github.rs` for the GitHub check, and
  `ProxyVersion` by `account/device.rs`.
- **`with_retries` 3 x 3 s** on open and continue: a person waits at the terminal; changing it
  changes the start's failure time, which `upload_open.rs` asserts.
- **Server quota (`RUN_MAX_COMPRESSED_BYTES`, 64 GiB)**: on `413 quota_exceeded` the client retries
  forever (D64) and prune can delete nothing unacknowledged, so the local disk grows until the
  owner raises the limit or the disk fills. At the 1 GB/h target and about 10x zstd, a full week is
  about 17 GB compressed, well under 64 GiB. Keep it as a stated storage-cost guard.

### 6.7 What must change together

| Change | Client | Worker | Stand-in | Verifier / protocol | SQL |
|---|---|---|---|---|---|
| Chunk cap 32 MiB | `chunk.rs:12` | `chunks.ts:21` | yes | no | no |
| Seal document caps (16 MiB, 8 MiB list) | `finish.rs:261-264` | `runs.ts:25-28` | yes | verifier's `seal.json` bound | no |
| Chunk headers, ack format, receipt | `session.rs:427-447`, `api.rs` | `chunks.ts`, `sign.ts` | yes | `goalcup-protocol` `ack.rs`, `receipt.rs`; `scripts/fetch-run.sh` | chunk index |
| Drop `tracer_version` | `api.rs`, `account/device.rs` | `util.ts:84-91`, `device.ts` | yes | no | no (keep column name) |
| `seconds_left` required | `api.rs`, `heartbeat.rs` | already sent | already sent | no | no |
| `repo_head` always null | `local.rs`, `finish.rs`, start and continue | none (accepts null) | none | no | none |
| Tunnel notes into the chain (later) | proxy, `finish.rs` | `runs.ts` seal | yes | daemon record type, verifier decoder | none |
| Remove D65 vitals | `api.rs`, `run/` | `runs.ts:497-522`, notices | yes | no | disk-pause columns may stay |

### 6.8 The smallest version that still meets the points it serves

Per entry: open (or continue) and a signed receipt; a background thread that sends whole lines
from the server's position as zstd chunks of at most 32 MiB, every 5 minutes and at the end,
checking each signed ack and appending it to `acks.jsonl` when it can; a heartbeat every 5
minutes that commits the chain end, reports `recording`, `model_requests` and a few vitals, and
carries the server's `seconds_left`; one retry rule for everything (backoff 60 s to 15 min,
never stop, D64); at the stop: final heartbeat, rest of the chain, seal with the two documents and
the tunnel list, within a one-hour wrap-up, and `goalcup upload` for anything left; delete a closed
local part once a signed ack covers it. Abandon once if the start fails before launch.

What that leaves out: dual version names, older-server fallbacks, the local repo head, the
pending-open note, the owed-ack retry, the two-hour age clock and disk-pressure branch, and the
stand-in's own rule tests. Estimated module src after cuts: about 3,450 of 3,956 lines.

### 6.9 Blind spots

- No long-run measurement exists. The 2-hour run report is not on any branch. A run with thousands
  of subagents has never been uploaded; chunk rate, R2 object count and wrap-up length at that
  scale are unverified.
- The server copy is never verified automatically (no verify job). A bad chunk that passes its
  hash but is not a valid chain is found only by an organizer running `fetch-run.sh` and the
  verifier.
- The R2 digest-mismatch case is recognised by error text (`chunks.ts:212-218`). A wording change
  turns it into `502`, which the client retries forever. Not wrong under D64, but it would hide a
  real damaged-body loop. Logged as `r2-put-unrecognised-error`.
- If the wrap-up runs out and the participant never runs `goalcup upload` within 24 h of the end,
  the tail is lost for good (`410` after close). The `ended` email says so; the terminal line says
  so. That is point 6 working, not a fix.
- Cutting the stand-in's rule tests leans on `upload_worker.rs`, which is `#[ignore]` and needs Node.
  If `gate full` is skipped, nothing checks the client against the real Worker.

### 6.10 Short list

**Top cuts (src / tests):**
1. Stand-in's own rule tests (`tests/it/upload_server*.rs`): 0 / 2,909. Plus trim the stand-in to a
   plain fake: 0 / ~500 (estimate).
2. `repo_head` (redundant with `seal.json` `git.commit`): ~115 / ~210. Client only, no deploy.
3. Pending-open give-back on the next start: ~110 (+~30 in start) / ~200. Owner to confirm the
   orphan-email trade.
4. Owed-ack retry and `adopt_server_ack`: ~70 / ~95.
5. Dual `proxy_version` / `tracer_version` names: ~55 / ~75 (+~40 stand-in, ~6 Worker).
6. Older-server `seconds_left` fallbacks: ~40 / ~45.
7. Prune age clock and pressure branch: ~50 / ~110.

Total: about 470 src lines and about 4,150 test lines.

**Top changes:**
1. One retry helper and one wait helper across `finish.rs` and `lifecycle.rs` (~80 src lines).
2. Upload interval 30 min to 5 min (one constant), so less of the record sits on one disk.
3. Prune: delete a closed part once a signed ack covers it; nothing else.
4. Fix the citation `D56` to `D55` at `uploader.rs:3`; move `wait_stopping` to `commands/start`.
5. Comment the two real limits with their reason: 32 MiB chunk (Cloudflare body and isolate
   memory limits) and 64 GiB per run (R2 storage cost).

**Must add for the seven points:**
1. Point 5: after the 2-hour run, rebuild it with `scripts/fetch-run.sh` and run `goalcup-verify`
   on the server copy, and compare with the local copy. That is the only proof the server can
   rebuild the story. No code needed for Friday.
2. Point 2 and 6: the proxy's 10,000-row tunnel-note cap (other lane) is a cap of our own; raise it
   to what fits under the 8 MiB server bound, or say plainly at finish when rows were dropped.
3. Nothing else in this module is needed for the seven points.

Overlap: the Worker side of the `tracer_version` cut (~6 src) is the same lines as section 9's ~10 / ~25; the totals use section 9's figure. Prune (~50 / ~110) is the same file as section 1's parts and pruning row; count one of them.

## 7. Platform (`crates/goalcup-platform`)

No branch of the 89 remotes changes this crate (`git diff origin/main...<branch> -- crates/goalcup-platform` is empty for all). The phase 2 notes say disk growth and the sleep lock "need a check (not measured here)" (docs/plans/2026-10-29-record-everything-phase2-notes.md:905-906). Correction: `crates/goalcup-cli/src/run/reserve.rs` is about 533 src and 423 test lines (`wc -l` 956, `mod tests` from line 535); this section first said about 283 and 673, which the figures below now fix.

### Size

Measured with `wc -l`. "Inline tests" are the `#[cfg(test)] mod tests` blocks inside src files.

| File | Total | Src | Inline tests |
|---|---|---|---|
| src/lib.rs (config dir) | 234 | 83 | 151 |
| src/awake.rs (sleep lock) | 269 | 269 | 0 |
| src/clone.rs | 164 | 68 | 96 |
| src/disk.rs (reserve) | 239 | 239 | 0 |
| src/errno.rs | 42 | 42 | 0 |
| src/handle_budget.rs | 79 | 79 | 0 |
| src/machine.rs | 908 | 810 | 98 |
| src/process_image.rs | 40 | 40 | 0 |
| src/terminal.rs | 482 | 292 | 190 |
| src/sourcecap.rs + sourcecap/unix.rs | 616 + 669 | 522 + 655 | 94 + 14 |
| src/tracecap.rs + tracecap/unix.rs | 1020 + 1133 | 843 + 984 | 177 + 149 |
| **src/ total** | **5895** | **4926** | **969** |
| tests/it/*.rs (8 files) | 1684 | 0 | 1684 |

So the crate is about 4.9k src lines and 2.65k test lines. Two parts, tracecap and sourcecap,
are 3.0k of the 4.9k src lines.

### Who uses what (outside the crate)

| Module | Users (src) |
|---|---|
| disk | goalcup-cli `run/reserve.rs:14`, `vitals.rs:29`, `commands/prep.rs:184` |
| sourcecap, handle_budget | goalcup-daemon `native_capture.rs:160-164, 576, 604` only |
| tracecap | goalcup-protocol `export.rs:41,68` (the chain writer), goalcup-verifier `capture_analysis/reader.rs:41`, goalcup-daemon `registry.rs:19`, `runtime/capture/mod.rs:97`, `native_plan.rs:101`, `runtime/daemon.rs:7`, goalcup-cli `upload/prune.rs:354`; `scripts/research-measure.py:25` copies the part-name grammar |
| awake | goalcup-cli `main.rs:306,614`, `commands/start/mod.rs:151`, `continue_run.rs:65`, `prep.rs:199` |
| machine | goalcup-cli `upload/api.rs:84` and `start/server.rs:322` (facts sent at start), `vitals.rs:33` (heartbeat), `prep.rs:180` (battery), `folder_gate.rs` (WSL and filesystem gate), `ssh_keys.rs:77`, `push_lines.rs:62,200`, `main.rs:468` (open URL) |
| terminal | goalcup-cli `launch.rs:979-992`, `start/forked.rs:96-111`, `upload/mod.rs:517`, `host_log.rs:141`, `prep.rs:175`, `start/refusals.rs:77`, `start/server.rs:87` |
| clone | goalcup-daemon `workspace.rs:119`, goalcup-cli `start/project.rs:54` |
| process_image | goalcup-daemon `runtime/capture/harness_build.rs:52,120` (D53 harness build digest) |
| errno | only sourcecap and tracecap (`sourcecap/unix.rs:346-350`, `tracecap/unix.rs:771-777`) |
| config_dir_with, Platform | goalcup-cli `config.rs:33,111` |

Dead in the product (used only by this crate's own tests), found by grep across `crates/`:

- `SourceDir::read_symlink_into_matching` and its native code (`sourcecap.rs:281-305`,
  `sourcecap/unix.rs:254-312`, `sourcecap/unix.rs:597-654`).
- `SourceIdentity::journal_token` (`sourcecap.rs:20-26`; the internal `imp::journal_token` is still
  used by `snapshot_facts`).
- `TraceDir::replace_descriptor` and the whole `trace.toml` replace path
  (`tracecap.rs:344-352`, `tracecap/unix.rs:607-739`). Nothing writes `trace.toml` any more; the
  verifier says so (`goalcup-verifier/src/capture_analysis/reader.rs:7`). Yet the writer still
  sweeps its temporaries (`export.rs:811`, `export.rs:984` call `sweep_descriptor_temps`,
  `tracecap.rs:239-253`, `tracecap.rs:777-803`, `tracecap/unix.rs:812-822`).
- `TraceDir::child_names` (`tracecap.rs:186-210`).
- `EntryArtifactRoot::create` and `EntryArtifactRoot::open` (`tracecap.rs:546-563`); the daemon uses
  `ArtifactRoot::create_entry/open_entry`.
- `FixedDocument::Manifest` (`tracecap.rs:30,39`).
- Windows-era refusal kinds never built on Unix: tracecap `ReparsePoint`, `OfflineOrRecall`,
  `NotDiskFile` (`tracecap.rs:48-52`); sourcecap `UnsupportedFileSystem`, `ReparsePoint`,
  `OfflineOrRecall` (`sourcecap.rs:461-468`).
- `terminal::EchoOff` (`terminal.rs:155-245`): no user anywhere in the workspace.
- The Windows arm of `config_dir_with` (`lib.rs:63-71`): the crate refuses to build on non-Unix
  (`lib.rs:11-14`), so it only runs in tests.

### Verdicts

Format from the brief. "Lines saved" counts only this crate unless marked "coupled".

| Part (path, src, tests) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Disk reserve (`src/disk.rs`, 239 src; `tests/it/disk.rs`, 137) | `ReserveFile`: a physically allocated 3 GiB file (`fallocate`, `F_PREALLOCATE`, zero-fill fallback) that gives blocks back as the chain grows; `free_bytes` via `statvfs` | none now (it served D40 and D65) | **cut** | Owner: "Disk full: no special handling. The harness stops and the user cleans up." The reserve exists only to hold space for the recording on a full disk (`disk.rs:1-4`, `run/reserve.rs:1-9`). `free_bytes` duplicates `machine::disk_space` (`machine.rs:659-675`), which already uses `statvfs` | 239 / 137 here. Coupled: `cli/src/run/reserve.rs` about 533 src / 423 tests (corrected from about 283 / 673), `run/pause.rs` 211 / 149, the disk branch of `run/watch.rs`, `tests/full/t25_disk_full.rs` 1173 test lines, Worker disk-pause email | Low here. Real ENOSPC now reaches the chain writer: a failed append or flush poisons it for good (`tracecap.rs:361-383`, `673-689`) and the daemon latches the failure (`daemon/src/runtime/capture/writer.rs:206-222`). That matches "the harness stops", but it must not stall the proxy (point 3) | all 8 in `tests/it/disk.rs` |
| Filesystem gate (`machine.rs:352-413`, about 60 src; `tests/it/machine.rs:163-192`, 30) | Names btrfs, ZFS, NFS, SMB, tmpfs, overlayfs, FUSE as "cannot keep records"; `goalcup setup` fails, `start`/`continue` warn | none now (7 is hurt by it) | **cut** | The rule was there for the reserve ("APFS, ext4, or XFS with Goal Cup Proxy's required disk reservation", `docs/LOCAL-RESEARCH-HANDOFF.md:54`). With the reserve gone it only turns people away: Fedora's default btrfs and container sandboxes (`docs/research/PARTICIPANT-REVIEW-2026-10-21.md:20`) and possibly the Vercel Sandbox, whose filesystem type is undocumented (`docs/research/VERCEL-COMPAT.md:39`) | about 60 / 30 here. Coupled: `cli/src/folder_gate.rs` filesystem half, `participant/refusals.rs:201-215`, prep step, README:184, RULES:385 | NFS or SMB may have weak `fsync` or locking for the redb registry; tmpfs loses local parts on reboot (the server copy remains). Could keep one warning line for NFS and SMB only | `the_filesystems_a_run_cannot_keep_records_on_...` |
| Sourcecap core (`sourcecap.rs` + `sourcecap/unix.rs`, about 700 src after cuts below; `tests/it/unix_sourcecap.rs` 261, `unix_snapshot_primitives.rs` 88) | Handle-rooted, no-follow reads of the harness's session files: `openat(O_NOFOLLOW)`, `fstatat(AT_SYMLINK_NOFOLLOW)`, refuses FIFOs and devices, reads at most the length seen at open | 1, 2, 3, 4 | **keep, shrink now; rewrite later** | No-follow stops the recorder from copying and uploading a file a symlink points at (point 4; for example a link to `~/.aws/credentials` inside the session store). `O_NONBLOCK` plus the kind check stop a FIFO named `*.jsonl` from hanging a poll (point 3, `sourcecap/unix.rs:214-246`). It is the only way subagent transcripts are read (point 2). The facts-matching dance (observe, open, compare every stat field) is anti-race hardening against a same-UID agent, which can `cat` the same secret into its transcript anyway; that is a point-4 job for redaction, not for this layer | see rows below | Rewriting it means rewriting the native walker in the daemon too | none for the core |
| Sourcecap symlink reading (`sourcecap.rs:281-305`, `sourcecap/unix.rs:254-312, 597-654`, about 144 src) | Reads a symlink's target text through a descriptor (`O_PATH` + `readlinkat` on Linux, `dlsym("freadlink")` on macOS) | none | **cut** | No product caller; the native walker skips symlinks (`native_capture.rs:1250`) | about 144 / about 35 | None | `symlink_payload_is_descriptor_bound_and_a_full_buffer_is_refused`, the symlink half of `mount_mismatch_refuses_...` |
| Sourcecap mount binding (`sourcecap/unix.rs:47-53, 475-596` plus call sites, about 160 src) | Refuses to cross a mount below a source root: Linux `statx(STATX_MNT_ID)`, macOS `fstatfs` fsid plus `fgetattrlist` mount status | none (it can hurt 1, 2, 7) | **cut** | The agent cannot mount anything without root, and patterns have no recursive wildcard, so a crossing cannot widen the scope much. On Linux a kernel without `STATX_MNT_ID` (added in Linux 5.8, per the statx(2) man page, https://man7.org/linux/man-pages/man2/statx.2.html) refuses the root with `UnsupportedMountBinding` (`sourcecap/unix.rs:476-506`), which loses the whole native leg, every subagent transcript included. The Vercel Sandbox kernel version is unverified | about 160 / about 50 | A bind mount under a session store could make the walker read another filesystem; it still reads only files that match the pattern | `mount_mismatch_refuses_directory_regular_and_symlink_acquisition` (`sourcecap.rs:587`), the `FORCE_CROSS_MOUNT` seam |
| Handle budget (`src/handle_budget.rs`, 79 src, plus about 25 lines of permit plumbing in sourcecap) | A 32-handle counter per native capture; each open dir, file or enumeration takes a permit | 3 (in theory) | **cut** | The use is bounded by structure already: at most 8 sources (`protocol/src/harness/check.rs:26`), each keeps one root, and a poll holds one chain of at most 8 directories plus their enumeration and one file (`native_capture.rs:76-81, 185`). That is about 25 descriptors at most, whatever the number of subagents. The constant's name (`MAX_GIT_LIVE_HANDLES_V1`) is left over from a removed Git capture | about 104 / about 35 | A future walker bug that leaks descriptors would no longer be stopped early. Low | `fixed_budget_refuses_the_thirty_third_live_handle_...`, `failed_native_open_releases_its_reserved_permit` |
| Sourcecap leftovers (`journal_token` method, Windows refusal kinds, `SourceIdentity` if unused after the cuts) | Persistence token and Windows variants | none | **cut** | Dead | about 20 / 12 | None | `source_root_journal_token_is_versioned_native_identity_not_debug_output` |
| Tracecap core (`tracecap.rs` + `tracecap/unix.rs`, about 1550 src after cuts below; `unix_tracecap.rs` 325, `artifact_root_unix.rs` 224) | Handle-rooted writes of the chain parts, `acks.jsonl` and the fixed documents; link-count and identity checks on every append and flush; poisons a stream for good after any failure; the verifier reads traces through it too | 5 (durable local chain to upload and resume) | **keep for Friday; change after** | The writer needs append, `fsync` of file and directory, create-once fixed files, a part list and reads. That serves point 5. The rest (link-count checks, identity re-checks of the artifact root on every call, contained recursive removal) defends against a same-UID process swapping paths. None of the seven points asks for that: the hash chain plus server acknowledgements is the real record. A fresh build would be a std-only module of about 250 lines | Now: see the next row. Later: about 1300 src / about 550 tests | A rewrite touches `protocol/src/export.rs`, the verifier reader, daemon registry and capture: too wide 8 days before Friday unless lane B rewrites `export.rs` anyway | (later) most of `unix_tracecap.rs`, `artifact_root_unix.rs` |
| Tracecap dead parts (descriptor replace and sweep, `child_names`, `EntryArtifactRoot::create/open`, `FixedDocument::Manifest`, Windows kinds) | See "Dead in the product" | none | **cut** | Dead | about 245 / about 195 | Protocol must drop the two `sweep_descriptor_temps` calls and `DESCRIPTOR_FILE` tests (`protocol/tests/export_roundtrip.rs:199-603`, about 10 tests) at the same time | unix.rs `descriptor_*` (3), tracecap.rs `descriptor_temp_grammar_...`, `unix_tracecap.rs` `descriptor_*` (2), `sweep_*` (2), `bounded_child_listing_...`; `artifact_root_unix.rs` tests that use `EntryArtifactRoot::open` switch to `ArtifactRoot::open_entry` |
| Tracecap enumeration cap (`tracecap.rs:19-20, 735-775`, about 40 src) | Refuses to list a trace directory past 4096 entries or 1 MiB of names | none | **cut** | A cap of our own that guards no cost: parts rotate at 256 MiB or 30 minutes (D55), so a week is about 340 to 600 parts | about 40 / 14 | None | `enumeration_budget_counts_unmatched_names_before_deletion` |
| Tracecap poison-for-good (`tracecap.rs:361-383, 673-689`) | After any failed write or flush the stream refuses every later call | 5 (no false durability) vs 1, 3 | **change (with daemon and protocol)** | Correct about durability, but the daemon then stops the whole entry's chain (`writer.rs:206-222`) while the harness may keep running. On a transient EIO or a disk the user just cleaned, nothing more is recorded until `goalcup continue`. Under "never block, note a gap and keep going" the writer should start a fresh part after a stated gap | 0 here | Needs a verifier rule for a gap at a part boundary; must change with protocol and verifier | none |
| Sleep lock (`src/awake.rs`, 269 src; `tests/it/awake.rs`, 256) | Holds `caffeinate -i -s -w <pid>` (macOS) or `systemd-inhibit ... sh -c 'echo ready; exec sleep infinity'` (Linux) from a dedicated thread, with a ready-line proof, `PR_SET_PDEATHSIG`, own process group | 7 (the run goes on for a week on a laptop) | **change: shrink** | A sleeping machine also pauses the harness, so nothing goes unrecorded; the lock serves the participant's 168 hours, not the record. A week on a laptop is how people will run it, so a small lock is worth keeping. The proof protocol, thread and settle wait are hardening. Smallest: spawn `caffeinate -i -w <pid>` or `systemd-inhibit --what=sleep:idle sh -c 'while kill -0 <pid>; do sleep 30; done'`, kill on drop | about 210 / about 215 | Linux loses proof that the lock was taken; prep can then only say the tool exists. On a Vercel Sandbox there is no sleep and it does not matter (`VERCEL-COMPAT.md:43`) | keep 1 or 2 of the 7 (`a_sleep_lock_dies_with_its_value`, `no_inhibitor_is_an_err_not_a_panic`) |
| Machine facts (`machine.rs:16-56, 562-675`) | OS, version, arch, CPU count, memory, disk total and free, WSL, container, AC power; sent once at start (`start/server.rs:322`) | none of the seven directly (research); 4 by holding no identity | **keep, shrink** | Small and useful to read runs. The Worker accepts any flat object up to 4 KiB (`server/worker/src/runs.ts:191-201`), so no Worker change. Cut `ac_power` (`machine.rs:415-466`, spawns `pmset`; prep's battery warning can be one line of advice) and `in_container` (`machine.rs:541-561`) | about 75 / about 36 | Less research detail on power and containers | `ac_power_parses_pmset`, `systemd_container_marker_ignores_wsl` |
| Process usage (`machine.rs:677-810`) | RSS, CPU time and open files of goalcup's own processes for the heartbeat (`vitals.rs:130-150`) | 3 (watch our own cost) | **keep** | `open_files` is exactly the signal the owner needs once stream caps go (open handles, macOS 256 default) | 0 | None | none |
| WSL checks (`machine.rs:89-150, 151-290, 309-356`) | Refuse WSL1; refuse folders on a Windows drive (drvfs, 9p, virtiofs) by mount type | 7 (Windows users run WSL2) | **keep** | Clear refusal for a setup that cannot work (file modes, speed). About 200 src is heavy for two checks, but cutting it buys little for Friday | 0 | | none |
| Host helpers (`account_home`, `open_url`, `open_file`, `url_allowed`, `machine.rs:62-88, 467-540`) | Browser open for login; home lookup for the SSH key warning | 6 (login and setup) | **keep** | Small; owned by the setup lane | 0 | | none |
| Terminal shield, modes, wait (`terminal.rs:1-153, 247-292`) | Catches Ctrl-C, Ctrl-\, Ctrl-Z and hangup in the host while the chat-mode harness gets them; restores terminal modes; continues a harness that stops itself | 3, 5, 7 (chat mode in the user's own terminal; the host lives to seal and upload) | **keep** | Needed for D57 chat mode | about 5 when D65 pause goes (the `on_stopped` "leave it frozen" branch, `terminal.rs:247-256`) | | `a_stop_the_caller_keeps_is_not_continued` once pause goes |
| `terminal::EchoOff` (`terminal.rs:155-245`) | Hides a typed secret and restores echo on signals | none | **cut** | No user in the workspace | about 90 / about 56 | None | `echo_is_left_alone_...`, `a_signal_while_the_echo_is_off_...` |
| Config dir (`lib.rs:1-83`) | Resolves `GOALCUP_CONFIG_DIR`, macOS Application Support, XDG | 6, 7 | **keep, trim** | Drop the Windows arm and `Platform::Windows` (Unix-only crate) | about 12 / about 45 | `cli/src/config.rs:33` picks `Platform`; it must change too | 3 `windows_*` tests |
| Clone rule (`src/clone.rs`, 68 src, 96 tests) | Says whether a folder is the root of its own Git clone | 5 (seal records the workspace, D11, D56, D71) | **keep** | Small, shared by daemon and CLI; whether the seal still needs it is the seal lane's call | 0 | | none |
| Process image (`src/process_image.rs`, 40) | Path of a running pid's executable, for the harness build digest | 1, 7 (what ran, D53) | **keep** | Tiny | 0 | macOS reads a path, a stated residual | none |
| errno (`src/errno.rs`, 42) | `readdir` end-or-error test | serves sourcecap, tracecap | **keep until a std rewrite** | Goes away if both walkers move to `std::fs::read_dir` | 0 now; 42 later | | none |

### Decisions that justify this crate, and whether they still hold

| D | What it says here | Holds under the seven points and owner decisions? |
|---|---|---|
| D40 (disk reservation clause, DECISIONS.md:229-231) | One 3 GiB reserve per run | **No.** Owner: no special handling for a full disk. Supersede the clause. |
| D64 (disk-stop clause) and D65 (DECISIONS.md:692-724) | Pause the agent with SIGSTOP under 1 GiB free, resume over 3 GiB | **No.** "Disk full: no special handling." Supersede D65 with a new D entry; also lifts one of the two exceptions to "records and never manages the agent". |
| D42 (DECISIONS.md:255-262) | Unix only now; Windows later; "protocol freezes at v5" | Platform clause holds. The freeze clause no longer applies (owner: no backward compatibility). |
| D57 (DECISIONS.md:464-486) | Chat mode in the user's terminal; native roots created through the no-follow capability; walker bound 8192 | Holds (point 7). The no-follow root creation is a point-4 guard and stays. |
| D55 | Chain parts and their names (`part_file_name`, `parse_part_name`, `tracecap.rs:614-636`) | Holds. Names must stay in step with `scripts/research-measure.py:25`, `cli/src/upload/prune.rs:354`, the verifier. |
| D53 | Record the harness build (uses `process_image`) | Holds. |
| D4, D56, D71 | Clone rule | Holds as far as this crate goes. |
| D5 | Evidence outside the workspace | Holds, but it is enforced in `cli/src/config.rs`, not here. The handle-rooted tracecap is not what keeps evidence from the agent. |
| D58 | Native adapter retry budget 128, producer IDs | Not in this crate; the fixed handle budget that sat beside it is redundant (see above). |

No decision justifies the dead parts listed above (trace.toml descriptor, symlink payload reads,
EchoOff, the Windows arms).

### Hidden couplings: what else breaks if a part goes

- **Disk reserve and pause.** `cli/src/run/reserve.rs` (956 lines), `run/pause.rs` (360),
  `run/watch.rs` disk branch, `commands/prep.rs` low-disk warning, `participant/status.rs` and
  `goalcup status` `PROBLEM:` lines, `state.json` `disk.disk_paused`, heartbeat `Vitals.agent_paused`
  and `disk_short` (`server/worker/src/db.ts:237-248`), Worker disk-pause email
  (`server/worker/src/notices.ts`, `db-local.ts:616-622`, `sql/goal-cup-tracer-upload.sql`,
  `test/notices.test.ts`), `terminal::wait_resuming`'s keep-frozen branch, OPERATING.md drill 4,
  README, PARTICIPANT-GUIDE, VERCEL-COMPAT row 3, `tests/full/t25_disk_full.rs` (1173 lines).
  `disk_free_bytes` in the heartbeat can stay (it comes from `machine::disk_space` or a one-line
  `statvfs`).
- **Filesystem gate.** `cli/src/folder_gate.rs` `check_filesystems`/`warn_filesystems`,
  `participant/refusals.rs:201-215`, the prep test door's `fs` facts, README:184, RULES.md:385,
  LOCAL-RESEARCH-HANDOFF.md:54, VERCEL-COMPAT.md:23,39.
- **Sourcecap mount binding.** Only `native_capture.rs` sees `RefusalKind::CrossMount` and
  `UnsupportedMountBinding` (as generic errors). Removing the variants changes the
  `SourceCapError` enum the daemon matches on (`native_capture.rs:1366-1369, 1458-1462` match only
  `MetadataChanged`, so they keep compiling).
- **Handle budget.** `native_capture.rs:160, 576, 604` construct and pass it; `SourceDir::open_selected`
  takes it as an argument, so its signature changes.
- **Tracecap descriptor sweep.** `protocol/src/export.rs:60, 811, 984`, about 10 tests in
  `protocol/tests/export_roundtrip.rs`.
- **Tracecap poison and part names.** The verifier reads parts via `TraceDir::open_selected` and
  `read_regular` (`verifier/src/capture_analysis/reader.rs:41`). Any change to how a failed part is
  handled needs a verifier rule.
- **Sleep lock.** `SleepLockFn` type in `start/mod.rs:151`, `continue_run.rs:65`, `RunState::launched`
  sleep state (`start/phases.rs:976-1050`), prep's sleep step (`prep.rs:1116-1141`), test support
  (`cli/tests/common/support/mod.rs:428`).
- **Machine facts.** Prep's battery line uses `on_ac_power` (`prep.rs:180`). The Worker stores the
  facts as an opaque flat object, so dropping fields needs no Worker change.
- **Config dir.** `cli/src/config.rs:33,111` passes a `Platform`.
- **errno.** Shared by both walkers; the doc comment also names the verifier's trace reader.

### What must change together with protocol, verifier, Worker or client

1. **Remove disk handling** (client and Worker together): drop `agent_paused: "disk"` and
   `disk_short` from the heartbeat and the Worker's disk-pause email and columns, or the Worker
   keeps a dead branch. The UPLOAD-API heartbeat doc changes too.
2. **Remove the descriptor sweep** (platform and protocol together): `export.rs:811, 984`.
3. **Poison then fresh part** (platform, protocol writer, daemon writer, verifier together), if the
   owner wants recording to continue after a transient write failure: the writer starts a new part
   after a gap; the verifier accepts a part that starts after a stated failure. Optional for Friday.
4. **Any tracecap rewrite** (platform, protocol `export.rs`, daemon registry and capture, verifier
   reader, `upload/prune.rs`, `scripts/research-measure.py` together).

### The smallest version that still meets the points it serves

| Area | Points | Smallest version |
|---|---|---|
| Session-file reading | 1, 2, 3, 4 | std `read_dir` per pattern level; open each match with `custom_flags(O_NOFOLLOW \| O_NONBLOCK)`; `fstat` after open and skip anything not a regular file; read up to the length seen. About 150 to 200 lines in place of about 1300 (sourcecap, handle budget, errno). Keeps the symlink and FIFO guards; drops same-UID race hardening. |
| Trace directory | 5 | std `create_dir_all`; `OpenOptions` append with `O_NOFOLLOW`; `sync_all` on file and directory; `create_new` for receipt, baseline and seal; part names and list; plain reads. About 250 lines in place of about 1800. |
| Disk | none | Nothing. One `statvfs` helper for the heartbeat's free-disk figure (already in `machine.rs`). |
| Sleep | 7 | About 50 lines: one child process tied to our pid, killed on drop. |
| Machine facts and vitals | 3, research | `facts()` without power and container; `process_usage` as is. |
| Terminal | 3, 5, 7 | Shield, Modes, `wait_resuming` (always continue). |
| WSL, config dir, clone, process image | 6, 7, 5 | As is, minus Windows arms. |

Fresh-build estimate for the crate: about 1500 src lines and about 600 test lines, against 4926
and 2653 today. For Friday the safe step is the cuts that do not touch the writer's design:
about 1400 src and about 850 test lines (list below).

### Blind spots and what could go wrong

- **Real ENOSPC behaviour is unmeasured.** With no reserve, the first failed append poisons the
  chain and the daemon latches the failure. I did not check the proxy keeps forwarding when the
  daemon's sink is failed; that is the proxy and daemon lanes' job (point 3). Unverified.
- **Open files with thousands of subagents.** In the native leg, descriptors do not grow with the
  number of tracked files: streams keep no open descriptor between polls
  (`native_capture.rs:78-81`, files reopened per read at `native_capture.rs:1456`). So raising
  the 32 native stream cap and the 1024-file cap costs no descriptors here. The wire leg (one
  socket per harness connection through the proxy) is not in this crate and is the likelier
  place to hit macOS's default soft limit of 256 (owner's figure; unverified on a current macOS).
- **Dropping the mount check** could let a walk follow a bind mount placed under a session store.
  Patterns have no recursive wildcard, so the reach is the pattern's own depth.
- **Shrinking the sleep lock** loses the Linux "ready" proof; a participant might believe the
  machine stays awake when the inhibitor was denied. Prep can still say the tool exists.
- **Cutting the filesystem gate** removes a warning some NFS or SMB users might have needed.
- **The 2-hour run report is missing**, so the sleep lock and disk growth were never measured on a
  real long run. The Vercel Sandbox filesystem type and kernel version are unknown
  (`VERCEL-COMPAT.md:23,39`), which matters for both the filesystem gate and `STATX_MNT_ID`.
- Line counts for coupled CLI files are from `wc -l` and a `#[cfg(test)]` split by eye; they are
  approximate.

### Short list

**Top cuts (this crate; src / tests)**

1. `disk.rs` reserve: 239 / 137. Coupled CLI: reserve, pause, watch branch, t25 (about 750 src plus the watch branch,
   about 1,750 test lines in the CLI; corrected from about 500 / 2,000; counted in section 5) plus the Worker disk-pause email.
2. Sleep lock to a 50-line version: about 210 / about 215.
3. Tracecap dead parts (trace.toml replace and sweep, `child_names`, unused constructors,
   Manifest, Windows kinds) and the 4096-entry enumeration cap: about 285 / about 210.
4. Sourcecap mount binding: about 160 / about 50 (also removes the Linux 5.8 dependency).
5. Sourcecap symlink reading and leftovers: about 165 / about 47.
6. Handle budget: about 104 / about 35.
7. Machine: filesystem gate, AC power, container: about 135 / about 66.
8. `terminal::EchoOff`: about 90 / about 56. Windows config arm: about 12 / about 45.

Friday total: about 1400 src and about 860 test lines out of 4926 and 2653. A later std rewrite
of tracecap and sourcecap saves about 2400 more src lines.

**Top changes**

1. Supersede D65 and the D40 reserve clause with one entry: "no special handling for a full
   disk; the harness stops; the user cleans up; `goalcup continue` resumes".
2. After a failed write, start a fresh part behind a stated gap instead of ending the entry's
   chain (protocol, daemon and verifier together), so a transient failure does not end
   recording for the rest of a week.
3. Rename or remove `MAX_GIT_LIVE_HANDLES_V1` and the "V1" names if anything of the budget stays.
4. Plan the std-only rewrite of tracecap and sourcecap together with any rewrite of
   `protocol/src/export.rs`, not alone.

**Must add for the seven points**

1. Point 2: raise the soft open-file limit to the hard limit (capped at the system maximum on
   macOS) in goalcup and the daemon at start, and put it back before exec of the harness so the
   harness runs as the user runs it (point 7). About 15 lines. Check `open_files` in the heartbeat
   during the next long run to confirm.
2. Point 3: one test that fills a small volume and shows the proxy keeps forwarding and the run
   records a `StorageFull` gap, now that the reserve is gone.
3. Point 7: before the Vercel run, check `findmnt -no FSTYPE -T "$HOME"` and `uname -r` there; if
   the mount check stays, a kernel older than 5.8 loses the native leg.

Overlap: the coupled CLI disk files are counted in section 5, not here. Only `disk.rs` and the crate's own lines are in this section's total.

## 8. Verifier (`crates/goalcup-verifier`)

### 8.1 Size and shape

| Area | Files | Lines (wc -l) |
|---|---|---|
| src, all files | 21 `.rs` | 13,488 file lines. Of these, 1,335 are inline `#[cfg(test)]` modules, so about 12,150 are product code. About 1,825 lines are comments. |
| tests, Rust | 19 `.rs` | 20,724 |
| tests, fixtures | v5 corpus 912 lines (191 KB); other fixtures 1,270 lines | 2,182 |

Split of src (product lines / inline test lines, from the first `#[cfg(test)]` line of each file):

| File | Product | Inline tests |
|---|---|---|
| capture_analysis/reader.rs | 1,889 | 181 |
| capture_analysis/lineage.rs | 101 | 41 |
| capture_analysis/wire.rs | 991 | 348 |
| capture_analysis/api.rs | 313 | 0 |
| capture_analysis/registry.rs | 213 | 0 |
| capture_analysis/session.rs | 878 | 62 |
| capture_analysis/operations.rs | 265 | 83 |
| capture_analysis/tracker.rs | 600 | 0 |
| capture_analysis/anthropic.rs | 1,585 | 53 |
| capture_analysis/openai.rs | 664 | 0 |
| capture_analysis/chat.rs | 1,080 | 0 |
| capture_analysis/thread.rs | 383 | 0 |
| capture_analysis/native/mod.rs, claude_code.rs, codex.rs | 617 + 219 + 443 = 1,279 | 0 |
| capture_analysis/report.rs | 463 | 483 |
| capture_analysis/stats.rs | 1,031 | 84 |
| capture_analysis/mod.rs, lib.rs | 39 + 53 | 0 |
| bin/goalcup-verify.rs | 326 | 0 |

### 8.2 Who uses the verifier today

This is the key fact for the audit: **nothing on the recording path uses the verifier.**

| Consumer | What it uses | Evidence |
|---|---|---|
| Daemon | Nothing. D3: the daemon never links the verifier. | docs/DECISIONS.md:68-73 |
| CLI, product code | Only `goalcup doctor`'s "decoders" check, which asks the decoder registry whether a hint has a decoder. It never affects recording. | crates/goalcup-cli/src/commands/doctor.rs:246-316; crates/goalcup-cli/Cargo.toml:31-33 |
| CLI, tests | Integrity reader (`scan_capture_trace`) and decoders, in about 10 test files. | crates/goalcup-cli/tests/it/harness_contract_kit.rs:20-22, tests/it/restarts.rs:337-359, tests/full/upload_e2e.rs:7,65,351, tests/it/connector_contract.rs:36-37, src/model_host.rs:1218-1270 (a `#[test]`) |
| Participant | `goalcup seal` prints a `goalcup-verify --capture-integrity` line. The participant guide does not mention the verifier. What the participant is told (model calls recorded, problems) comes from the CLI's proxy count, not from the verifier. | crates/goalcup-cli/src/commands/seal.rs:15-17; crates/goalcup-cli/src/participant/status.rs:456-505; `grep goalcup-verify docs/PARTICIPANT-GUIDE.md` finds nothing |
| Worker (server) | Nothing. It checks chunk headers only and signs receipts and acks in the verifier's formats. No verify job exists. | server/worker/src (no reference except a comment in sign.ts:3-13); docs/SERVER-DESIGN.md:86-87, 157-160; crates/goalcup-verifier/src/lib.rs:1-2 |
| Organiser | `scripts/fetch-run.sh` rebuilds an entry from R2 and the database, then the organiser runs `goalcup-verify --capture-integrity` on it. | scripts/fetch-run.sh:1-22, 349 |
| Test kit | `scripts/matrix.sh` runs `--capture-integrity` (check 1) and `--research-json` (check 3, "the agent worked": a usage observation with output tokens, or an assistant message). `research-replay.py` runs both. `linux-harness-smoke.sh` runs integrity. | scripts/matrix.sh:27-36, 2054-2105; scripts/research-replay.py:104-106; scripts/linux-harness-smoke.sh:231 |
| Judges (later) | RULES says the verifier's evidence label is a hint for judges, and every label but `none` counts. | docs/RULES.md:306-319 |
| `--stats` | No script, no Worker code, no current participant doc. Only docs/OPERATING.md:661-670 and docs/CONTRACTS.md:273-279 describe it; the old participant guide in docs/archive mentions it. | `grep -rn -- "--stats" scripts server` finds nothing |

So for points 5 and 6 on Friday:

- **Point 5 (server rebuilds the story):** the server stores chunks. The rebuild is `fetch-run.sh`. The integrity reader is how an organiser checks that rebuilt copy. That is the only verifier part point 5 leans on, and it runs offline, after upload.
- **Point 6 (tell the participant):** served by the CLI, not the verifier. The verifier is an optional self-check the seal output points to.
- **Decoders, research report and stats** read the uploaded raw bytes. D3, D36.4 and D37.2 say this is "interpret later". None of it is needed before the start window. Judging dates are not set (docs/RULES.md:54).

### 8.3 Verdicts per part

Format from the brief. "Lines saved" counts file lines from wc -l. "Freeze" means keep the code, make no change before Friday, and take it off every path that ships to participants or runs during a run.

| Part (path, src lines, test lines) | What it does | Points served | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Integrity reader (capture_analysis/reader.rs, 1,889 + 181 inline; tests/it/m2_capture_integrity.rs 3,737) | Streams a v5 trace: line bounds, canonical bytes, hash links, phase rules, producer and stream rules, receipt and ack signatures, seal and baseline binding, Git state, deadline facts, local pruning. Exit 0/1/2/3. | 5 (judging the server copy), 6 (optional self-check), 3 and 9 indirectly (gaps make a trace incomplete, never invalid) | **Keep, change** | It is the one tool that says whether an uploaded record is whole and untouched. But it enforces the protocol's caps as violations: more than 128 open streams is `open_stream_limit_exceeded`, more than 128 open connections is `connection_limit_exceeded`, more than 128 goal streams fails too (reader.rs:1384, 1566-1568, 1694-1696). The test pins 129 open streams as exit 2 (tests/it/m2_capture_integrity.rs:1972-1984). If the daemon lifts the caps (owner decision), every large run reads "invalid evidence". These checks must go when the writer's caps go. Also drop shape checks for Launch fields the protocol lane cuts (`allowed_tool_rule`, `process_handoff_digest`, `run_public_key`; reader.rs:1818-1889). | about 40 src, about 150 tests (cap and dead-field cases) | Removing a cap check also removes a memory bound in an offline tool. Acceptable: the reader runs on the organiser's machine, and per-stream state is a few flags. | The cap cases in m2_capture_integrity.rs (around 1972-1984 and 2840-2860); Launch allowance cases if the fields go |
| v5 reader corpus (tests/it/v5_corpus.rs 718; tests/fixtures/v5-corpus 912 lines, 191 KB) | Runs the real binary over 30 frozen traces and compares exits and finding codes with expected.toml. Exists to enforce the v5 freeze (D43 ruling 6, D54). | none now (it guarded a freeze the owner lifted) | **Cut** | "Protocol v5 is frozen no longer applies" (owner decisions). The corpus would have to be regenerated for every protocol change before Friday. m2_capture_integrity.rs already covers the same rules from the library. Keep at most 3 small binary smoke cases (valid sealed, torn tail, unreadable) inside m2_capture_integrity.rs if a test of the binary's exit codes is wanted. | 0 / 1,630 (718 + 912) | Loses a check of the binary's exit codes and stderr. Mitigation: 3 cases, about 60 lines. | v5_corpus.rs, fixtures/v5-corpus/, including `legacy-pre-v5-trace` (D54 says no pre-v5 code anyway) |
| Lineage (capture_analysis/lineage.rs, 101 + 41) | One recovery-supersession rule for producers, shared by reader and decode session. Bound `MAX_PRODUCERS` 65,536. | 5 | **Keep** | Small, and both reader and session need one rule. | 0 | none | none |
| bin/goalcup-verify.rs (326) | Three modes: `--capture-integrity`, `--research-json`, `--stats`; `--dev-key`; safe output file creation. | 5, 6 | **Keep, change** | Drop `--stats` (see stats). Keep integrity and research. | about 70 src | none | none |
| lib.rs (53) | Exports; pinned server key `SERVER_PUBLIC_KEYS`; `VerifyOptions`. | 5 | **Keep, change** | `allow_descriptor_key` "has no effect" by its own doc (lib.rs:38-42); delete it and its echo in the research summary (report.rs:250-253). | about 10 src | Research report JSON changes shape (no consumer reads that field; unverified for private notebooks). | none |
| Decoder framework (api.rs 313, registry.rs 213, session.rs 878 + 62, operations.rs 265 + 83; tests/it/m2_decoder_v1.rs 1,126; tests/m2_decoder_v1_quiet_panic.rs 127) | Stable external decoder contract: per-decoder `catch_unwind` isolation, a quiet panic hook, identity checks, thread safety, routing by hint, bounded operational facts, record counts and the model-request stream count used by the evidence label. | none on Friday; research later. operations.rs's request count feeds the RULES label. | **Freeze now; simplify after the event** | It runs on uploaded bytes only. The "external crate can implement DecoderV1" promise, the isolation and the quiet panic hook serve outside decoder authors, which Goal Cup does not have yet. After the event, an in-tree enum of decoders with plain error returns could replace most of api/registry/session. Also: session routes at most `MAX_OPEN_STREAMS` streams per decoder and fails a decoder with `too_many_open_streams` past it (session.rs:755-766), so a run with many live subagent files makes the report `partial`. Raise with the protocol constant. | Friday 0. After the event, estimated 800 to 1,000 src and about 1,000 tests (estimate, not measured) | A refactor before Friday could break the kit's "agent worked" check; hence freeze. | After the event: most of m2_decoder_v1.rs, quiet_panic test |
| Wire plumbing (wire.rs, 991 + 348; tracker.rs 600) | Segment assembler, SSE framer, redaction audit projection, bounded field helpers; tracker shared by the two OpenAI decoders. The reader uses only `SegmentView` (about 20 lines, wire.rs:77-97; reader.rs:36, 1468). | none on Friday; research later | **Freeze** | Decoder support code. Redaction audit kinds are open tokens (wire.rs:133-148), so masking option b (add URL password, drop email and Codex rate-limit rules) needs no verifier change. | 0 | none | none |
| Anthropic Messages decoder (anthropic.rs 1,585 + 53; tests/it/m2_anthropic_decoder.rs 2,658; fixtures/m2-anthropic 331) | Decodes `/messages` requests and SSE replies into model_request, model_response, usage, tool_call observations. Claude Code's wire leg. | 1 (measures it in tests), research | **Freeze** | It is the meter the measured runs use ("decoded model calls", progress plan table) and what matrix.sh check 3 reads. Not needed by recording. | 0 | none if frozen | none |
| OpenAI Responses decoder (openai.rs 664; tests/it/m2_openai_decoder.rs 2,149; fixtures/m2-openai 741) | Codex HTTP and WebSocket `response.create` and replies. | 1 (test meter), research | **Freeze** | As above for Codex. | 0 | none | none |
| Chat Completions decoder (chat.rs 1,080; tests/it/m2_chat_decoder.rs 2,524) | `/chat/completions` (Hermes, DeepSeek, gateways). | 1 (test meter), research | **Freeze** | As above for Hermes and any gateway. | 0 | none | none |
| Hermes capture-only test (tests/it/m2_hermes_capture_only.rs 395) | Pins that a Hermes chat stream is read by the chat decoder and an unhinted snapshot by none. Its own header says the file name is historical (lines 1-7). | none | **Cut** | Covered by m2_chat_decoder.rs and connector_samples.rs. | 0 / 395 | Small loss of an end-to-end Hermes row through the report. | m2_hermes_capture_only.rs |
| Amp thread decoder (thread.rs 383; tests/it/m2_thread_decoder.rs 625) | Amp's thread WebSocket snapshots, including subagent `parentToolUseId`. | research; 2 for Amp analysis | **Freeze** | Amp is still a shipped connector (docs/RULES.md:408) and has no native decoder; without this, Amp's label is `wire_undecoded`, which still counts. Small. | 0 | none | none |
| Native decoders (native/mod.rs 617, claude_code.rs 219, codex.rs 443; tests/it/m2_native_observations.rs 1,737; fixtures/m2-native 19) | Read Claude Code and Codex session JSONL: model, usage, tool calls, human-message markers, Codex child sessions (`receiver_thread_ids`). | research; 2 (subagent attribution); RULES label (`native` model activity) | **Freeze; raise buffer bounds after the event** | Needed for the `wire_and_native` and `native_only` labels and the cross-leg bypass note, at judging time. One shared 8 MiB budget across all open native streams (native/mod.rs:99-102) and a 128-stream tracking cap (native/mod.rs:291) will mark records `oversize` or untracked when hundreds of subagent files are open. Fine for Friday; fix before judging. | 0 | Analysis of week-long runs with thousands of subagents is degraded until the bounds rise. Recording is not affected. | none |
| Connector samples (tests/it/connector_samples.rs 239; fixtures/connectors 114) | Per-connector replay of real samples as data. | research | **Keep** | Cheap and data-driven; the way a new harness adds a decoder sample. | 0 | none | none |
| Harness contract golden (tests/it/harness_contract_golden.rs 487; plus goalcup-cli/tests/fixtures/harness-golden/decoder-selection.json 391 and launch-allowance-reader.json 149) | Golden of decoder selection per hint, and of the reader's tool-allowance shape check. | none | **Cut** | The decoder half repeats connector_samples and m2_decoder_v1 selection tests. The allowance half pins a Launch field the protocol lane should drop. | 0 / about 1,027 (487 + 391 + 149) | Golden files are written by a regen flag in a different crate's fixture dir: deleting one half without the other breaks the CLI fixture check. Delete together. | harness_contract_golden.rs and both JSON goldens |
| Research report (report.rs, 463 + 483 inline; tests/it/m2_research_report.rs 2,456; fixtures/m2-report-bytes 53) | Streams every observation to a private NDJSON, then one summary: integrity, delivery, assistance markers, decoders, models, the RULES evidence label, cross-leg note. | 6 indirectly (judges), 1 (matrix check 3), research | **Keep, freeze** | It holds the evidence label RULES promises judges (docs/RULES.md:306-319) and the bypass note the cheat checklist relies on (docs/CHEAT-CHECKLIST.md:44-47). Off the Friday path, but needed before judging. | 0 (the `allow_descriptor_key` echo only) | none | none |
| Run stats (stats.rs 1,031 + 84 inline; tests/it/m2_run_stats.rs 873) | `--stats`: numbers-only summary across a run's traces, with de-dup of ids (up to 1,000,000), token source choice, model and error code lists. | none now (D26 says the verifier *may* sum usage) | **Cut** | No consumer on main. It also caps a run at 128 traces (stats.rs:39), a cap of our own. Token totals can be added to the report later, from the same observations. | 1,115 + about 70 in bin + 3 lines in lib.rs / 873 | A researcher loses one ready summary. Recover from git history after the event. | m2_run_stats.rs; stats.rs inline tests |
| tests/common (846: capture_records 523, decode 58, mod 81, records 31, wire 153) | Shared trace builders and decode helpers. | test support | **Keep, shrink** | Shrinks with the cuts above (unused helpers fail clippy's dead-code lint). | about 50 tests (estimate) | none | helpers only used by cut files |
| CLI coupling (goalcup-cli: doctor.rs:246-316 decoder check; model_host.rs:1218-1270 test; Cargo.toml:31-33 library dependency) | `goalcup doctor` warns when a declared hint has no decoder. | none (D36.4 and D43 ruling 7: a missing decoder never matters for capture) | **Cut** (lane: CLI, listed here because it couples) | The warning tells a participant about something that does not affect their run. Removing it lets the CLI drop its product dependency on the verifier (keep it as a dev-dependency for tests), which shrinks the shipped `goalcup` binary. | about 71 CLI src + about 55 CLI tests | doctor's output and its tests change; check any kit parser reading the "decoders" line (unverified that none does). | doctor decoder check tests; model_host.rs `every_hinted_path_is_read_by_its_decoder` |

### 8.4 Decisions that justify the verifier, and whether they still hold

| D | What it says for the verifier | Still holds under the seven points and owner decisions? |
|---|---|---|
| D3 (docs/DECISIONS.md:68-73) | Record raw, interpret later; verifier is its own binary; daemon never links it. | **Holds.** It is the reason decoders can be frozen and moved off the Friday path. |
| D9 (100-103) | A gap is loss, stated; a gapped trace is incomplete, never clean. | **Holds** and matches point 3. The reader implements it (exit 1, not 2). |
| D16 (126-130) | The server's signed receipt fixes the clock. | **Holds.** The reader verifies the receipt and reports deadline facts. |
| D26 (155-158) | The verifier may sum token usage. | **Holds as "may".** It does not require `--stats`. Cut stats. |
| D36.4 (167-177) | Decoders live in the verifier, per protocol; a missing decoder never stops capture. | **Holds.** Also makes doctor's decoder warning pointless. |
| D37.2, D37.4 (179-191) | Semantic interpretation runs later in versioned decoders. | **Holds.** Supports freezing decoders. |
| D43 ruling 6 (290-294) | A verifier change that makes a valid v5 trace invalid needs a DECISIONS entry; the corpus guards it. | **No longer holds.** No backward compatibility before Edition 01. Retire the ruling and the corpus. |
| D43 ruling 7 (295-298) | Unknown hint is capture-only; doctor names it. | Capture-only part **holds**. "Doctor names it" should go with the doctor check. |
| D45 (306-312) | Integrity judges the record, not the run; undecoded or interrupted streams do not lower exit 0. | **Holds.** Fits point 3. |
| D53 (388-400) | `harness_build` recorded, never judged; reader checks shape. | **Holds.** The "v0.5.0 verifier reports record_malformed" trade no longer matters (no old verifiers to serve). |
| D54 (403-412) | v5 only; "every v5 trace's verdict is frozen". | No-pre-v5 part **holds**. Freeze clause **no longer holds**. |
| D55 (414-425) | Part files; local pruning reported as incomplete. | **Holds.** Long runs prune locally, so the server copy (fetch-run) is what gets verified. |
| D56, D71 | Seal records Git state or a capped walk; reader reports `git_unreadable`, `seal_capped`. | **Holds**, small. |
| D58 (488-496) | 65,536 producers; 128 live. "A verifier built before this entry flags more than 128 producers." | Producer bound **holds**. The historical sentence can go. The reader does not check live producers. |
| D63 (641-657) | Signed receipt must match the Launch it anchors. | **Holds**, small (a handful of comparisons). |
| D74 and RULES "What a label is worth" | Evidence label in the research summary. | **Holds**, but it is a judging hint, needed at judging, not on Friday. |

Owner decisions that touch this crate:

- "Raise or remove the open-stream limits (128 per entry, 32 native)": **forces a reader change** (section 6).
- "Byte-for-byte match of subagent files is no longer required": no verifier change; that check lives in the kit.
- "Masking option b": no verifier change (redaction kinds are open tokens).
- "RUN_MAX_COMPRESSED_BYTES": no verifier change.
- "The file path is the ID; the server orders by ID and time": no verifier change. The reader already orders by chain sequence and gives capture time per segment.

### 8.5 Hidden couplings

1. **Protocol caps are verdicts in the reader.** `MAX_OPEN_STREAMS` and `MAX_OPEN_CONNECTIONS` come from goalcup-protocol (crates/goalcup-protocol/src/capture.rs:57-60) and the reader fails a trace that exceeds them (reader.rs:1566-1568, 1694-1696). `MAX_GOAL_STREAMS` is a separate hard-coded 128 (reader.rs:76-78, 1384). The same constant also bounds decoder routing (session.rs:644, 761), the tracker (tracker.rs:173), the Anthropic decoder (anthropic.rs:152) and native tracking (native/mod.rs:291). Raising the constant in the protocol raises all of them at once. Deleting the cap in the writer without deleting these checks makes large runs invalid.
2. **Pinned server key.** `SERVER_PUBLIC_KEYS` (lib.rs:34-35) must equal the production Worker's signing key. A Worker redeploy that rotates the key needs a new verifier release (docs/SERVER-DESIGN.md:91-95). The 29 Oct redeploy is reported not to have needed SQL; whether the key stayed the same is unverified.
3. **Byte layouts of receipts and acks** are shared by the Worker (server/worker/src/sign.ts:3-13) and goalcup-protocol (`receipt.rs`, `ack.rs`), and checked by the reader. Change all three together.
4. **The CLI links the verifier for one doctor warning**, which pulls the whole decoder set into the shipped `goalcup` binary (crates/goalcup-cli/Cargo.toml:31-33).
5. **Golden files cross crates.** harness_contract_golden.rs writes into goalcup-cli/tests/fixtures/harness-golden/ (its lines 15-17).
6. **The test kit depends on the research report's JSON shape** (scripts/matrix.sh:2054-2105 reads `type`, `observation.kind`, `fields.role` and output token keys). A change to observation kinds breaks matrix check 3.
7. **Docs claim a server verify job that does not exist.** docs/CHEAT-CHECKLIST.md:26-28 says "a verify job re-runs goalcup-verify over the stored chain every hour and at seal". docs/SERVER-DESIGN.md:157-160 and lib.rs:1-2 say it is not built. docs/UPLOAD-API.md:320 and 462 also lean on it. The cheat checklist's bound for "the agent can read the run credential" therefore does not hold today.
8. **A documented point 4 control is not in the code.** RULES says "a trace in which the verifier finds secret-shaped text ... is flagged" (docs/RULES.md:294-296). There is no such scan in the verifier (`grep -i secret` in src finds none). TODO.md:117-118 moved it to the post-event export step.
9. **SERVER-DESIGN is stale on fetching.** It says "nothing downloads the stored chain" (docs/SERVER-DESIGN.md:159-160), but scripts/fetch-run.sh does exactly that.

### 8.6 What must change together

| Change | Protocol | Verifier | Worker | Client (CLI, daemon) |
|---|---|---|---|---|
| Lift open-stream and connection caps (owner) | raise or remove `MAX_OPEN_STREAMS`, `MAX_OPEN_CONNECTIONS` | drop `open_stream_limit_exceeded`, `connection_limit_exceeded` and the goal-stream cap, or make them follow the constant; raise decoder routing bounds; update m2_capture_integrity cases | none | daemon writer limits and its tests (crates/goalcup-daemon/src/runtime/capture/tests.rs:189-277) |
| Drop dead Launch fields (protocol lane) | remove fields | remove shape checks (reader.rs:1840-1872) and allowance golden | none | stop writing them |
| Rotate the signing key | none | new `SERVER_PUBLIC_KEYS`, new release | new secret | ships the new verifier |
| Drop doctor decoder check | none | none | none | CLI doctor, Cargo dev-dependency |
| Cut `--stats` | none | stats.rs, bin, lib | none | docs/OPERATING.md, docs/CONTRACTS.md |
| Cut v5 corpus | none | tests only | none | docs/CONTRACTS.md "format freeze", D43 r6, D54 freeze clause |

### 8.7 The smallest verifier that meets the points it serves

The verifier serves point 5 (an organiser can check the rebuilt server copy) and, weakly, point 6 (a self-check line at seal). For Friday the smallest version is:

- reader.rs, lineage.rs, `SegmentView` from wire.rs, lib.rs, and `--capture-integrity` in the binary. About 2,100 product lines.
- A model-request count could move into the integrity report (count hinted POST wire streams, as operations.rs:173-185 does), so the kit and the organiser can see "calls were recorded" without the research report. About 40 lines. Optional.

Everything else reads uploaded raw bytes and can run later (D3). Two ways to treat it:

- **Recommended for Friday: freeze.** Keep decoders, session and report unchanged, off the participant path (CLI does not link them). Reason: the kit's measured runs and matrix check 3 use them as the meter for point 1, and the 2-hour Vercel Sandbox run should not lose its meter four days before the start. The 2-hour run report (docs/research/results/2026-10-30-two-hour-run.md) does not exist on any branch yet, so I cannot say which numbers it will need.
- **Bolder: delete decoders now, restore from a git tag after the event.** Saves about 9,500 src lines and about 14,500 test lines plus about 1,250 fixture lines (sum of the decoder, session, report, wire and tracker files above and their tests; estimate). What could go wrong: matrix check 3 and the measured-run "decoded model calls" disappear, the RULES evidence label and the cheat checklist's cross-leg note have no code until restored, and the restore lands under time pressure before judging. I do not recommend this before Friday.

### 8.8 Blind spots

- I did not build or run anything. Line counts are wc -l. Estimates are marked.
- Whether any private notebook or Supabase job reads `--stats` output or `options.allow_descriptor_key` is unverified; nothing in this repo does.
- Whether the 29 Oct Worker redeploy kept the same signing key is unverified.
- The phase 2 notes propose a header decoder and a reconcile script (docs/plans/2026-10-29-record-everything-phase2-notes.md:673-686). The owner decisions win; I did not count them as needed.
- The verifier's memory with caps removed: the reader keeps per-stream flags only, so thousands of open streams cost little. Decoders hold up to 1 MiB per open stream, bounded by a shared 8 MiB; that bound is what degrades analysis, not memory.

### 8.9 Short list

**Top cuts (lines saved, src / tests)**

1. `--stats` and stats.rs: about 1,185 src (1,115 file lines + about 70 in the binary) / 873 tests.
2. v5 reader corpus and its freeze rule: 0 / 1,630 (718 Rust + 912 fixture lines).
3. Harness contract golden (decoder selection and tool allowance): 0 / about 1,027 (487 + 391 + 149 JSON in goalcup-cli).
4. Hermes capture-only test: 0 / 395.
5. `goalcup doctor` decoder check and the CLI's product dependency on the verifier: about 71 CLI src / about 55 CLI tests.
6. `allow_descriptor_key`: about 10 src.
Total for Friday: about 1,270 src and about 3,980 test and fixture lines. After the event, simplifying the decoder framework could save an estimated 800 to 1,000 src more.

**Top changes**

1. Reader: remove or follow the open-stream, connection and goal-stream caps in the same PR that lifts them in the daemon and protocol. Otherwise every big run is exit 2.
2. Raise decoder routing and native buffer bounds with the same constant, so the research report is not `partial` on runs with many subagents (before judging).
3. Freeze decoders, session and report; no work on them before Friday.
4. Fix the docs: the cheat checklist's hourly verify job (it does not exist), SERVER-DESIGN's "nothing downloads the chain", and D43 ruling 6 and D54's freeze clause.

**Must add for the seven points**

1. A reader test with more than 128 simultaneously open streams and connections that exits 0 (point 2). Small.
2. Before any research export (not Friday): the secret-shaped text scan RULES already promises, or remove the promise from RULES (point 4).
3. Nothing else in this crate is needed for Friday. Recording, gaps, masking and upload all live outside it.

Overlap: the harness contract golden JSON (391 + 149) is part of section 10's connector goldens; it is counted here. The doctor decoder check is inside section 5's `doctor` cut and is counted there.

## 9. Upload Worker (`server/worker`)

"src" means `server/worker/src`, "tests" means `server/worker/test`.

### Summary in one paragraph

The Worker is the leanest part of Goal Cup Proxy. It never parses the chain: it stores opaque zstd
chunks by offset, signs a receipt and one ack per chunk, and keeps a small index in Supabase. So
points 1 and 2 (every call, every subagent) need **no Worker change**: whatever the client puts in
the chain is uploaded, and the server can rebuild it by run id, entry id, offset and time. The
over-building is around the edges: a mail sender that production no longer uses, the disk-pause
emails the owner has dropped, idle emails that are not about recording, two backward-compatibility
shims, and a second copy of every database rule (`db-local.ts`) kept only for tests. For Friday the
safe cuts are about **280 src lines and 450 test lines** (the table below sums to ~450). The big structural cut (the rule copies)
is real but belongs after Edition 01.

### Size

| Area | Path | Lines |
|---|---|---|
| Worker source | `server/worker/src/*.ts` (17 files) | 4,001 src |
| Worker tests | `server/worker/test/*.test.ts` (15 files) | 5,255 tests |
| Test helpers and vectors | `server/worker/test/helpers/*.ts`, `test/vectors/signing.json` | 494 + 26 tests |
| SQL mirror (reference copy of the website's migrations) | `server/worker/sql/goal-cup-tracer-upload.sql` | 4,468 (not run by the Worker) |
| Scripts | `scripts/check-sql.sh` 299, `scripts/delete-test-runs.sh` 95 | 394 |
| Config | `wrangler.toml` 164, `package.json` 21 | 185 |
| Worker README | `server/worker/README.md` | 471 |
| Rust stand-in of the same server (lane outside C, a coupling) | `crates/goalcup-cli/tests/common/upload_server/*.rs` | 2,514 tests |

Source files: `db-local.ts` 701, `runs.ts` 637, `db.ts` 485, `db-supabase.ts` 433, `chunks.ts` 228,
`github.ts` 223, `notices.ts` 220, `context.ts` 171, `util.ts` 148, `device.ts` 146,
`ratelimit.ts` 139, `sign.ts` 116, `index.ts` 108, `env.ts` 96, `mail.ts` 70, `errors.ts` 51,
`auth.ts` 29.

### What the server needs to rebuild the story by ID and time (point 5)

| Need | Where it is today | Verdict |
|---|---|---|
| Who and which run | `run_id` (`r` + 16 hex), member, mode, event (`src/util.ts:7-8`, `src/runs.ts:244-258`) | have |
| Order of process restarts | entries with `segment` 0, 1, 2 ... (`src/db.ts:104-126`, `continue` route) | have |
| Order of bytes | chunk index: `entry_id`, `offset`, `length`, `first_seq`, `last_seq`, `prev`, `head`, `r2_key` (`src/db.ts:134-146`) | have |
| Server time | signed receipt `started_at`/`deadline_at` (D16, `src/runs.ts:366-376`); signed ack `acked_at` per chunk (`src/chunks.ts:81-82`); `received_at` per chunk | have |
| The bytes | R2 `traces/{event}/{run}/{entry}/{offset:016}-{sha256}.zst`, create-only, R2 checks the hash (`src/chunks.ts:155-158`, `202-228`) | have |
| The end | final heartbeat `(seq, head, repo_head)`; seal and baseline docs; tunnel list (`src/runs.ts:540-637`) | have |
| A timeline of upload health | one `heartbeats` row per beat (`sql/goal-cup-tracer-upload.sql:164`) | have |
| Subagents, model calls | inside the chain. The Worker never looks. "The file path is the ID" is a verifier rule, not a Worker rule. | no change |
| A tool that rebuilds a run | `scripts/fetch-run.sh` (349 lines, lane outside C) reads the DB chunk index and R2 | have; `docs/SERVER-DESIGN.md` "Not built" still says nothing downloads the chain (stale) |

One chain record is at most 128 KiB (`crates/goalcup-protocol/src/capture.rs:47`), so a 32 MiB chunk
always fits at least one record. A week with thousands of subagents only means more chunks; no
Worker limit counts subagents, calls or streams.

### Limits and quotas

| Limit | Value | Where | Guards a real cost? | Verdict |
|---|---|---|---|---|
| `RUN_MAX_COMPRESSED_BYTES` | 64 GiB per run | `wrangler.toml:56-59`, `src/context.ts:11-12,61-67`, `src/chunks.ts:123-144` | Yes: R2 storage. At $0.015 per GB-month ([R2 pricing](https://developers.cloudflare.com/r2/pricing/)) a full run is about $1 a month, about $6 over the 180-day bucket lock (`wrangler.toml:31`), and locked bytes cannot be deleted. It stops a stuck or hostile client. | **Keep at 64 GiB** and write the cost reason next to it. Estimate of real use: the largest measured 2-hour run was 73.5 MB uncompressed (`docs/research/coverage-matrix.md:64`), about 6 GB for 168 h, about 0.6 GB after zstd at the ~10x the client notes (`crates/goalcup-cli/src/upload/uploader/chunk.rs:17-18`). Even 50 times that fits. This is an estimate from pre-record-everything runs: the 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any branch yet, so record-everything sizes are unmeasured. Owner decides. |
| `MAX_CHUNK_BYTES` | 32 MiB compressed | `src/chunks.ts:22`; client `chunk.rs:13` | Yes: one request's size and retry cost. | Keep. Must match the client. |
| Seal and baseline doc | 16 MiB each | `src/runs.ts:26`; client `finish.rs:254`; verifier `reader.rs:145` | Worker memory (128 MB). The daemon caps the workspace walk. | Keep. Three copies must match. |
| Tunnel list | 8 MiB, first line `goalcup-tunnels-v1` | `src/runs.ts:27-28,73-85` | Memory. | Keep. |
| JSON body | 64 KiB; device body 4 KiB; machine facts 4 KiB | `src/runs.ts:25,174`; `src/device.ts:22` | Memory and DB row size. | Keep. |
| Close after the end | 24 h (`CLOSE_AFTER_SECONDS`) | `src/db.ts:17` | Not a cost: it fixes when judging can start. After it, uploads are `410` forever. | **Change (owner):** say why in the comment. A machine that is off for a day after the deadline loses its tail (point 5). Raising it to 72 h costs nothing in storage. |
| Credential life | unbound 1 h, bound deadline + 24 h | `src/db.ts:55-57` | Security. | Keep. |
| Device request life | 900 s, poll every 5 s | `src/device.ts:20-21` | Security. | Keep. |
| Device rate limits | 120 starts/min per address, 30 polls/min per code, 1200 polls/min per address | `src/ratelimit.ts:29`, `wrangler.toml:76-78` | Yes: the only calls without a credential; each costs a DB call. | Keep. |
| Cron GitHub batch | 50 runs/tick, 5 activity pages | `src/notices.ts:36`, `src/github.ts:28-32` | Subrequest limits per invocation. | Keep while the judged commit stays. |
| Mail batch | 20 per tick (`NOTICES_PER_TICK`) | `src/mail.ts:18` | None any more (see mail). | Cut with mail. |
| Concurrency gap in the quota | up to one 32 MiB chunk per concurrent entry over the limit | `src/chunks.ts:117-122` | n/a | Keep as is (harmless). |

### MIN_COMPETITION_VERSION and the "change together" rule

`MIN_COMPETITION_VERSION` (D70) refuses a competition device request and a competition open from an
older goalcup (`src/runs.ts:138-169`, `src/device.ts:55`, `src/context.ts:37-42`). It is empty today
(`wrangler.toml:55`). Under "no backward compatibility, protocol, verifier, Worker and client change
together" it matters **more**, not less: the Worker accepts any chain bytes, so only this check stops
a Friday-minus-one client from entering the cup with a trace the new verifier may reject.

- Keep it (about 30 lines). Set it to the Friday release before the window opens. The README step is
  at `server/worker/README.md:319`.
- Change: it checks competition only. Practice and test from an old client still open. That is fine
  for test runs, but a participant practising on an old client learns nothing true. Owner choice:
  apply it to practice too (one line in `src/runs.ts:228` and `src/device.ts:55`).
- `continue` is not checked on purpose (D70). Keep.

### Verdicts, part by part

Format: Part (path, src lines, test lines) | What it does | Points served | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go.

| Part | What it does | Points | Verdict | Reason | Saved src / tests | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| Routing and maintenance (`src/index.ts`, 108; covered in `test/integration.test.ts`) | Eleven routes plus health; `MAINTENANCE=on` answers 503 before any DB call (`index.ts:73-79`) | 5, 3 | keep | Small, no framework. Maintenance lets a bad deploy be held without the client losing data (D64). | 0 / 0 | none | none |
| Auth (`src/auth.ts`, 29) | Bearer `gcr_` credential, stored as sha256 only | 4, 5 | keep | Minimal. | 0 / 0 | none | none |
| Device flow (`src/device.ts`, 146; `test/device.test.ts` 306, `test/device-unit.test.ts` 87) | Code, user code, poll, test-only approve route | 5 | keep | It is how a credential exists. The approve route is 404 unless `TEST_APPROVAL_KEY` (staging and tests only, `src/context.ts:98-103`). | 0 / 0 | none | none |
| Rate limiter (`src/ratelimit.ts`, 139; `test/ratelimit.test.ts` 184) | Durable Object counter per HMAC of address or code | none directly; guards a real cost | keep | The only unauthenticated calls. The old `[[ratelimits]]` binding never tripped (`ratelimit.ts:11-15`). | 0 / 0 | Cutting it lets anyone flood Supabase. | none |
| Open, continue, position, receipt, abandon (`src/runs.ts` parts, about 300) | Server-minted ids and clock; one run per credential; segments | 5, 3 | keep | Core. Abandon is used by the client when a start fails before launch (`crates/goalcup-cli/src/upload/mod.rs:380,406,477`). | 0 / 0 | none | none |
| GitHub public and empty checks at open (`src/runs.ts:233-236,313-348`) | Repo must be public and have no commits (OD7, D71) | none (contest rule in VISION) | keep | 35 lines; it is the "fresh product" rule at the cheapest moment, before the harness starts. | 0 / 0 | A GitHub outage gives 502 at open; the client retries. | none |
| Chunks (`src/chunks.ts`, 228; `test/integration.test.ts`, `test/quota.test.ts` 114) | Stream to R2 create-only with sha256; one atomic DB commit; signed ack | 5 | keep | The heart of point 5. | 0 / 0 | none | none |
| Signing (`src/sign.ts`, 116; `test/sign.test.ts` 68 + vectors 26) | Ed25519 receipt and ack, bytes fixed by `crates/goalcup-protocol/src/receipt.rs` and `ack.rs` | 5 | keep | Server time is the trusted time (D16; verifier `reader.rs:169-172`). | 0 / 0 | Any change must change the protocol crate and vectors in the same PR. | none |
| Heartbeat core (`src/runs.ts:444-491`) | seq, head, offset, final, repo_head, recording; answers stop and seconds left | 3, 5, 6 | keep | The only server signal that stops a run is `stop` at the deadline (D64). | 0 / 0 | none | none |
| Heartbeat `vitals` validator (`src/runs.ts:499-533`) | Checks six optional fields, drops bad ones, logs | 3 (recorder health) | change | With the disk pause gone, store `vitals` as an opaque JSON object of at most 1 KiB. | ~25 / ~60 | A huge or odd object: the 1 KiB cap covers it. | `test/notices.test.ts` 897-968 shrink to one case |
| Disk pause and `disk_full` email (`src/runs.ts` agent_paused, `src/db.ts:242-252`, `src/db-local.ts:616-628`, `src/notices.ts:153`, SQL section 3 at `sql/...:2806-2943`) | Heartbeat vitals start a pause; cron mails once per 6 h | none (owner: "Disk full: no special handling") | **cut** | Owner decision after phase 2. Client side D65 goes in another lane; both must go together. | ~45 / ~120 | An old client still sending `agent_paused`: ignored once vitals are opaque. | `notices.test.ts` 969-1060; `supabase.test.ts` 314-340 |
| Idle emails and `model_requests` (`src/notices.ts:28,163-171`, `src/db.ts` fields, `src/db-local.ts:597-612`) | Mails at 3 h and 6 h without model-call growth | none | **cut** | Not about recording; it nudges the participant about the agent. The verifier counts calls from the chain. | ~35 / ~120 | Participants lose a reminder. RULES promises it (`docs/RULES.md:175-178`): change RULES too. | `notices.test.ts` 259-312, 360-418; `local-db.test.ts` 530-542 (part) |
| Started, recording, silence, ended emails (`src/notices.ts`) | Queue per run, kind and key into the outbox | 6 (recording, ended says whether all is uploaded), 5 (silence says uploads stopped) | keep | The only way to tell someone away from a dead machine during a 168 h run. | 0 / 0 | none | none |
| Health marker (`src/notices.ts:90-131`) | Mails silence only when the server itself was well | 6 | keep | Stops false silence emails during our own outages. Goes only if silence goes. | 0 / 0 | none | none |
| Mail sender (`src/mail.ts`, 70; `test/mail.test.ts` 108) | Claims notices and posts them to Resend | 6 in name only | **change (cut the sender)** | Production `goal_cup_upload_claim_notices` hands every notice to the website's `email_queue` and **returns `[]`** since website migration `20261017150200` (`sql/goal-cup-tracer-upload.sql:2366-2439`; same file in `/home/user/agentcommunity_page/supabase/migrations/`). So the Resend POST never runs. But the handoff happens **inside** that call, and `mail.ts:23-24` makes the call only when `RESEND_API_KEY` is set. Replace the file with one call in the tick: `await ctx.db.claimNotices(NOTICES_LIMIT, at)`. | ~165 / ~110 | **Whether `RESEND_API_KEY` is set in production is unverified.** If it is not, no run email has ever left. Check `wrangler secret list` now. | `mail.test.ts` all; `local-db` claim/finish parts |
| GitHub judged commit (`src/github.ts`, 223; `src/notices.ts` recordFinalCommits; `test/github.test.ts` 184; about 330 lines of `test/notices.test.ts`) | Reads GitHub's activity API after each deadline; flags late and force pushes; flagged fallbacks | none of the seven (a published contest rule, `docs/RULES.md:158,419-422`) | keep for Friday | It must run near the deadline: a repo made private or deleted later loses the submission. Deployed and tested. After Edition 01, consider an organizer script. | 0 / 0 now (~330 / ~510 if moved later) | none now | none now |
| Three modes and three event ids (`src/context.ts:105-131`) | competition, practice (10 min), test (operator) | 7 (practice proves a harness), 5 | keep | Practice is how a participant checks their harness. Test runs are the organizers' path (the Vercel runs need a test operator, progress log 29 Oct). | 0 / 0 | none | none |
| Machine facts, client IP and country (`src/runs.ts:191-209`) | Stored on the run, never shown | none | keep (owner may cut client IP) | Machine facts help research (OS, WSL2). Client IP serves no point and is personal data. | ~10 / ~30 if IP cut | none | `lifecycle.test.ts` 435-466 (part) |
| `tracer_version` alias (`src/util.ts:79-91`) | Accepts the old field name | none | **cut** | No backward compatibility. Client sends both (`crates/goalcup-cli/src/account/device.rs:386-387`); cut both sides together. | ~10 / ~25 | An old client gets 400: it must be refused anyway. | `lifecycle.test.ts` 481-505 |
| Old SQL overloads (`src/db-supabase.ts:357-359,400-401`) | 13 and 14 argument heartbeat; 4 and 9 argument final commit, "for the live Worker" | none | **cut** (website migration) | The live Worker is from main since 29 Oct. | ~3 / ~15 | Needs a website migration applied to the shared DB. | `supabase.test.ts` 300-312 |
| Supabase backend (`src/db-supabase.ts`, 433; `test/supabase.test.ts` 567) | One RPC per method; checks every returned row's shape | 5 | keep | The shape checks catch SQL drift between two repos. | 0 / 0 | none | none |
| Local backend (`src/db-local.ts`, 701; `test/local-db.test.ts` 590) | Second copy of every DB rule in R2 JSON, for tests and `wrangler dev` | none (tests only) | keep for Friday; cut after | It is the store under all 276 Worker tests. A rewrite now is the riskiest change in this lane. Later: run the same tests against Postgres loaded with the real SQL (`scripts/check-sql.sh` already starts one; TODO item 8). | 0 / 0 now (~700 / ~590 later) | Drift: it already does not model the one-repo rule (`db-local.ts` openRun comment). | none now |
| Config, env, errors, util (`src/context.ts` 171, `src/env.ts` 96, `src/errors.ts` 51, `src/util.ts` 148; `test/config.test.ts` 298) | Parse vars; refuse test-only vars outside tests | 5 | keep, trim | Drops `RESEND_API_KEY`, `NOTICE_FROM`, `NOTICE_RECIPIENT_OVERRIDE` with mail. | ~15 / ~0 | none | none |
| SQL mirror (`sql/goal-cup-tracer-upload.sql`, 4,468) | Copy of the website migrations, for reading and `check-sql.sh` | 5 | keep | Not code the Worker runs. The authority is the website repo. | 0 / 0 | none | none |
| `wrangler.toml` | Prod and staging, cron, DO, R2, logs without invocation logs | 4, 5 | keep, edit | Edit the vars that go; comment the quota's cost reason. `test/config.test.ts:237` asserts 64 GiB in the file. | ~6 / ~0 | none | none |
| Scripts (`scripts/check-sql.sh` 299, `scripts/delete-test-runs.sh` 95) | Try a migration on a throwaway Postgres; delete test runs | 5 | keep | Ops tools, not run in the request path. | 0 / 0 | none | none |

### Decisions that justify the Worker, and whether they hold

| D | What it says for the Worker | Still holds under the seven points and the owner's decisions? |
|---|---|---|
| D16 | The server fixes the clock: signed receipt start and deadline | Yes. Point 5 "by time" needs one trusted clock. |
| D63 | Receipt must describe the Launch it anchors | Yes, verifier side; the Worker only signs `connector_dir_sha256` (`src/sign.ts`). |
| D64 | No server error stops a run; the client retries everything | Yes. It is why the Worker's 16 error codes matter little to the run: only `stop` in a 200 ends it. Point 3. |
| D65 | Disk pause, partly via heartbeat vitals and the `disk_full` email | **No.** Owner: "Disk full: no special handling." Cut the Worker half with the client half. |
| D66 | Time left pulled by the agent, from the heartbeat's `seconds_left` | Yes; the Worker field is tiny (`src/runs.ts:126-136`). |
| D70 | `MIN_COMPETITION_VERSION`; `--dev` never competes | Yes, and more needed now (see above). |
| D71 / OD7 | Repo public and empty at open | Yes, as a contest rule. |
| WP-D (no D number; `docs/SERVER-DESIGN.md:120-134`) | 64 GiB per-run limit | Yes, as a storage-cost guard. Say so in the config. |

No D number covers the idle emails, the mail sender, or `db-local.ts`. They are not backed by a current rule.

### Hidden couplings (what must change together)

1. **Four copies of the server rules.** The Worker handlers (`runs.ts`, `chunks.ts`), `db-local.ts`,
   the SQL in the website repo (applied by hand to the shared Supabase), and the Rust stand-in
   `crates/goalcup-cli/tests/common/upload_server/` (2,514 test lines). A rule change (for example the
   close time) touches all four, plus `sql/` regeneration and `scripts/check-sql.sh`.
2. **The website repo owns the database.** Any `Db` change needs a migration in
   `agentcommunity_page/supabase/migrations/`, applied to production before the Worker deploy.
   Removing notice kinds also needs the `notices` CHECK constraint and `notice_text` changed
   (`test/supabase.test.ts:558` checks this).
3. **Emails cross three systems.** Worker cron, the DB function that writes the text and queues it,
   and the website's email gateway (template `goalcup_run_notice`). The Worker's `RESEND_API_KEY` is now
   only an on-switch. Docs still say the Worker sends through Resend (`docs/UPLOAD-API.md:494-496`,
   `docs/OPERATING.md:488`): both are stale.
4. **Signed bytes** in `src/sign.ts` match `crates/goalcup-protocol/src/receipt.rs` and `ack.rs`
   byte for byte (`test/sign.test.ts`). Change only together.
5. **Size limits** live in three places each: chunk (Worker, client), seal docs (Worker, client,
   verifier, `prune.rs:103`).
6. **The tunnel list format line** `goalcup-tunnels-v1` is checked by the Worker and written by the
   client (`crates/goalcup-cli/src/proxy/tunnels.rs`).
7. **`EVENT_ID` is in every R2 key**, and the 180-day bucket lock covers only
   `traces/goalcup-cup-2026/`. The switch is a manual deploy between 6 Nov 00:00 and 7 Nov 00:00 UTC
   (`wrangler.toml:37-42`). Create-only under the lock is still unproven (`README.md:400-404`).
8. **`final_commit` reaches the member's entry page** through `goal_cup_my_entry`
   (`sql/...:3479,3543`). Moving the judged commit out of the cron leaves that field empty.
9. **The version field names** `proxy_version` and `tracer_version` are sent by the client and read by
   the Worker; the DB column is still `tracer_version`.

### The smallest Worker that still meets the points it serves

Keep: routing and maintenance; auth; device flow and its limiter; open, continue, receipt, position,
abandon; heartbeat with seq, head, offset, final, repo_head, recording and an opaque vitals blob;
chunks; seal with tunnel list; signing; the Supabase backend; the cron steps close runs, judged
commit, queue the four useful emails (started, recording, silence, ended) and one handoff call.
That is about 3,700 src lines today, and about 3,000 once `db-local.ts` is replaced by tests against
real Postgres after the event.

Nothing must be **added** to the Worker for points 1 and 2. Recommendation for every lane: put new
data (headers, subagent files, database snapshots) in the chain, not in new routes or side files, so
the Worker stays unchanged and protocol-agnostic.

### Blind spots and what could go wrong

- Cutting `mail.ts` without keeping the claim call stops every run email. The replacement must still
  call `claimNotices` every tick.
- Any DB-side cut needs a website migration on the shared production database before Friday. That is
  owner work and a deploy-order risk: deploy the Worker that no longer reads a column only after the
  migration, or keep the column.
- Raising the close time weakens nothing in the record (acks still time each chunk), but it delays
  the final status of a run by the same amount.
- The 64 GiB estimate is from runs before record-everything. Header streams and uncapped subagents
  may grow traces by an unknown factor. Measure on the 2-hour run (report not yet written).
- `upload_worker` tests in the full gate (`scripts/gate.sh:93-108`) run the real Worker; any cut must
  keep them green, and the Rust stand-in must change with it.

### Short list

**Top cuts (Friday-safe)**

| Cut | Src lines | Test lines |
|---|---|---|
| Resend sender in `mail.ts`, plus claim/finish plumbing in `db.ts`, `db-local.ts`, `db-supabase.ts`, `env.ts` (keep one handoff call) | ~165 | ~110 |
| Disk pause and `disk_full` email (owner decision), with the client's D65 | ~45 | ~120 |
| Idle emails and `model_requests` | ~35 | ~120 |
| `vitals` validator to an opaque 1 KiB object | ~25 | ~60 |
| `tracer_version` alias and old SQL overloads | ~13 | ~40 |
| **Total** | **~280** | **~450** |

After Edition 01: replace `db-local.ts` with tests on real Postgres (~700 src, ~590 tests), drop the
Rust stand-in's rule copy, and consider moving the judged commit to an organizer script (~330 src,
~510 tests).

**Top changes**

1. Check now that `RESEND_API_KEY` is set on the production Worker. If not, no run email goes out.
2. Set `MIN_COMPETITION_VERSION` to the Friday release before 7 Nov 00:00 UTC; owner to decide
   whether practice is checked too.
3. Keep `RUN_MAX_COMPRESSED_BYTES` at 64 GiB and write its cost reason in `wrangler.toml`; owner may
   raise it once the 2-hour run gives real sizes.
4. Owner: state the reason for the 24-hour close, or raise it to 72 hours.
5. Fix stale docs: `docs/UPLOAD-API.md:494-496`, `docs/OPERATING.md:488`, the "Not built" item in
   `docs/SERVER-DESIGN.md`, and the email list in `docs/RULES.md:175-178`.
6. Owner steps already on the list: the `EVENT_ID` switch, and proving create-only under the bucket lock.

**Must add for the seven points**

Nothing in the Worker. Points 1, 2 and 5 are met as long as the client keeps every new kind of data
inside the chain.

Overlap: the Rust stand-in (2,514 test lines) belongs to section 6, which counts its trim. The `tracer_version` cut is counted here, not in section 6.

## 10. Connectors (`connectors/`, the schema as data, pins and connector scripts)

The OpenCode and OpenClaw facts were read with `git show` from `origin/opencode-openclaw` (commit bf15fe85) and not edited. This area overlaps section 1, which already scored the schema module (`crates/goalcup-protocol/src/harness/*`), the trust tables and the pin code. Where a cut is already counted there, this section says "counted in section 1" and does not add it again.

### 10.1 What is in the lane

| Part | Lines | Kind |
|---|---|---|
| `connectors/claude-code-cli/harness.toml` | 159 (100 config, 59 comment) | data |
| `connectors/codex-cli/harness.toml` | 130 (78 config) | data |
| `connectors/hermes-cli/harness.toml` | 152 (70 config) | data |
| `connectors/amp-cli/harness.toml` | 115 (58 config) | data |
| Seed files (`connectors/<id>/files/`) | 0: no connector has one (`git ls-files connectors` lists only the four `harness.toml`) | data |
| `crates/goalcup-protocol/src/harness.rs` + `harness/{schema,parse,load,check}.rs` | 72 + 1,146 + 228 + 344 + 1,580 = 3,370 src (`load.rs` has about 20 inline test lines) | src |
| `crates/goalcup-protocol/providers.toml` | 25 (2 rows, `anthropic` and `deepseek`) | data |
| `scripts/pin-manifests.sh` | 274 (109 comment) | src |
| `scripts/check-connector.sh` | 314 (64 comment) | src |
| `scripts/test_check_connector.py` | 411 | tests |
| `crates/goalcup-protocol/pins/connector-dirs.txt` | 7 | data |
| `skills/adding-a-capture-harness/` | 599 (SKILL 39, writing-a-decoder 277, templates 209 + 74) | docs |
| `docs/CONNECTOR-SCHEMA.md` | 1,002 | docs |
| `docs/HARNESS-ONBOARDING.md` | 579 | docs |
| `docs/harnesses/*.md` | 911 (README 51, amp 231, claude 170, codex 242, hermes 217) | docs |
| OpenCode and OpenClaw facts (branch only) | 126 + 121, results 145, handover 302 | docs |

Connector tests (all tests, not src), from `wc -l`:

| Test file | Lines |
|---|---|
| `crates/goalcup-protocol/tests/it/harness_schema.rs` (114 tests) | 4,451 |
| `crates/goalcup-protocol/tests/it/harness_loader.rs` | 650 |
| `crates/goalcup-protocol/tests/it/harness_conformance.rs` | 379 |
| `crates/goalcup-protocol/tests/it/connector_pins.rs` | 303 |
| `crates/goalcup-protocol/tests/it/pins_build_fails.rs` | 86 |
| `crates/goalcup-protocol/tests/it/no_family_literals.rs` | 378 |
| `crates/goalcup-protocol/tests/it/connector_template.rs` | 120 |
| `crates/goalcup-cli/tests/it/connector_contract.rs` | 994 |
| `crates/goalcup-cli/tests/it/harness_contract_golden.rs` | 975 |
| `crates/goalcup-cli/tests/it/harness_contract_kit.rs` | 805 |
| `crates/goalcup-cli/tests/full/connector_contract.rs` | 463 |
| `crates/goalcup-cli/tests/shipped_connectors.rs` | 411 |
| Fixtures: `harness-golden` 2,110, `schema-1-connectors` 354, `frozen-connectors` 237 | 2,701 |

### 10.2 Every connector field, scored against the seven points

"Used by" is the code outside the schema module that reads the field (grep of `\.field\b` in
`crates/*/src`). A field that no code outside the schema reads, or that must always hold one value,
is ceremony.

| Field | Used by | Shipped connectors | Point | Verdict |
|---|---|---|---|---|
| `schema_version` | parse only | all, always `1` | none | Cut. No backward compatibility before Edition 01. |
| `id` | everywhere; Worker `connector_id` (`server/worker/src/db-supabase.ts:97`) | all | 5 | Keep. Could be the folder name. |
| `participant.display_name` | 11 CLI files | all | 6 | Keep. |
| `participant.first_screen` | `participant/launch.rs`, `start/phases.rs` | claude, amp, hermes | 7 | Keep (one line each). |
| `participant.setup_section` | `prep.rs:898-906`, `participant/status.rs` | all | 6 | Cut. Derive the anchor from `id`. |
| `participant.no_prompts` | `prep.rs:911`, `model_host.rs:548` (D67) | claude, codex, hermes | 7 (a prompt stalls a 168 h run: lx-codex and lx-claude, 22 Oct) | Keep. Measured failure. |
| `participant.time_left_tool` | `prep.rs:739`, `mcp.rs:194` (D66) | all | none | Owner call. Serves no point. |
| `capture.interactive` | parse only; must be `true` (`parse.rs:185-206`) | all | none | Cut. |
| `capture.continuation` | `start/phases.rs:1015` to the daemon and the `Launch` record; same sentence in all four | all | none | Cut, with the `Launch` field (section 1). |
| `capture.args` | `launch.rs` | codex, hermes | 7 | Keep. |
| `capture.resume_args` | `launch.rs` | all | 3, 7 (`goalcup continue`) | Keep. |
| `capture.configure` | `prep.rs`, `start/phases.rs` | hermes | 2 for Hermes 0.19 only (no snapshot file in 0.21.1+, D72) | Change: keep the mechanism; drop the two rows if Hermes 0.22 is the cup version. |
| `capture.auth_hosts` | `proxy/connect.rs:529`, `model_host.rs:464`, setup probe | claude, codex, amp | 4 | Keep. It becomes more important if the proxy ever opens more hosts (section 8). |
| `capture.env` | `launch.rs` | all | 1 (`NODE_USE_ENV_PROXY`, D59), 3 (`CLAUDE_CODE_RETRY_WATCHDOG`, self-update off) | Keep. |
| `capture.platform.<key>.env` | `launch.rs` | all; only `SHELL = "/bin/bash"` on Linux | none | Cut. Pick `SHELL` in code: the participant's, else `/bin/zsh` on macOS, `/bin/bash` on Linux. |
| `capture.reserved_env_prefixes` | check only | hermes | none: it guards the connector's own `env` against itself | Cut. |
| `capture.executable.program` | `launch.rs`, doctor, prep | all | 7 | Keep. |
| `capture.executable.version_args` | prep, start | all, always `["--version"]` | 5 (`Launch.harness_version`) | Keep the probe; cut the field (default `--version`). |
| `capture.home.run_home`, `config_home`, `env` | launch, home, daemon | all | 7 (keychain login, D38), 2 (native roots) | Keep. |
| `capture.home.real_dirs` | daemon refusal (`native_plan.rs:289-297`, `harness.rs:38`) | all, 1 to 3 paths | none: goalcup picks every home path itself | Cut. Keep one check that the config dir is not inside a known harness folder, if any. |
| `capture.home.skills_dir` | `start/phases.rs:538` | claude, codex | none (agent guidance; "no one to ask") | Keep, tiny. |
| `capture.home.per_run_dir`, `dirs`, `seed` | `per_run_home.rs`, `connector.rs` | none | none | Cut (counted in section 1). |
| `capture.ca.env`, `mode` | `launch.rs`, `ca_bundle.rs` | all | 1 | Keep. |
| `capture.identity.source` | must be `"none"` | all | none | Cut. |
| `capture.native.name`, `root`, `pattern`, `form` | daemon native capture | all | 2, 5 | Keep. Add `**` (section 4). |
| `capture.native.protocol_hint` | verifier decoders | claude, codex, amp | 6 (research labels) | Keep. |
| `capture.native.since_run_start` | `launch.rs`, `native_plan.rs` | every row is `true`; identity `none` needs it (D59) | 2 | Cut the field; always on. |
| `capture.native.warn_missing` | `start/phases.rs` status line | false on 7 of 10 rows | 6 | Keep. Consider default `false`. |
| `capture.argv_guard` | `launch.rs:1461` | hermes; guards `["chat"]` and `["--continue"]`, which the connector itself wrote | none | Cut. |
| `capture.termination.native_binary` | daemon terminate | codex (D22) | 3 (a clean stop so the seal happens) | Keep. |
| `capture.termination.strategy`, `grace_seconds` | daemon | all set the defaults (`process_group`, 10) except codex's strategy | none | Cut the field; strategy follows from `native_binary`. |
| `capture.hosts[].host` | proxy | codex, amp | 1 | Keep, as a plain list of names. |
| `capture.hosts[].evidence_paths`, `prefix_paths`, `protocol_hint` | decoder hint only: since Task 2 every path on an opened host is captured (`proxy/http.rs:11-13`, `metadata.rs:138-146`) | codex, amp | none for recording | Change: one central path-suffix table picks the hint (`/responses`, `/messages`, `/chat/completions`, as `model_host.rs` already does for D62). Amp keeps one host-level hint. |
| `capture.providers` | `schema.rs:846-905` resolves names from `providers.toml` | claude (`anthropic`) | 1 | Change: write `api.anthropic.com` in the connector; delete `providers.toml` and the resolver. |
| `capture.model_host_from` (+ `via`, `absent`) | `model_host.rs` (1,540 src) | claude, codex, hermes | 1 | Keep. |
| `[launch_prep]` | `launch.rs`, prep, start | hermes | 1 (Hermes 0.22 gateway, D72) | Keep. |
| `[redaction].extra_patterns` | daemon redactor | all | 4 | Change per owner option b: drop the four Codex rate-limit rows (`codex_rate_limits`, `codex_plan_type`, `codex_credits_remaining`, `codex_credits`; `connectors/codex-cli/harness.toml:110-113`). Keep the rest. |

Count: 13 fields are ceremony (always one value, a default, or a self-guard), 3 are unused,
2 are owner calls (`time_left_tool`, Hermes `configure`), and the rest serve a point.

### 10.3 Verdicts per part

| Part (path, src lines, test lines) | What it does | Points served | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| `connectors/claude-code-cli/harness.toml` (159 data, 0) | Claude Code recipe: keychain home, `CLAUDE_CONFIG_DIR`, 6 native rows, anthropic host, `ANTHROPIC_BASE_URL` reader, billing-header rule | 1, 2, 4, 7 | Change | 6 native rows collapse to 2 with `**`; ceremony fields go; comments shrink to one line per decision | about 130 data | A `**` row could match a new file type in `projects/` that holds something unexpected. Same root, same since-start rule, so low. | Goldens regenerate (`harness-golden/claude-code-cli.json`) |
| `connectors/codex-cli/harness.toml` (130, 0) | Codex recipe: `CODEX_HOME`, trust arg, rollout row, `native_pid`, 2 hosts, 7 redaction rows | 1, 2, 3, 4, 7 | Change | Drop 4 rate-limit rows (owner); host rows lose paths and hints; ceremony goes | about 95 data | Dropping the rate-limit rows records plan and credit balance. Owner chose this (option b). | `capture_redaction_extra.rs` and `redaction.rs` cases that name the Codex kinds; fixtures in `tests/fixtures/harness/codex-cli*.toml` |
| `connectors/hermes-cli/harness.toml` (152, 0) | Hermes recipe: `HERMES_HOME`, replace-mode CA, `config.yaml` model host, 2 configure commands, snapshot and `state.db` rows, argv guard, launch prep | 1, 2, 7 | Change | Cut `argv_guard`, `reserved_env_prefixes`. If 0.22 is the cup version, cut `configure` and the `session` row (0.21.1+ writes no session file, `harness.toml:99-102`) | about 80 data | If participants run 0.19, dropping `configure` loses the snapshot (but `state.db` still comes in at the seal). Owner must pick a version (progress doc, decision 2). | Hermes golden; `m2_hermes_capture_only.rs` (section 8 already cuts it) |
| `connectors/amp-cli/harness.toml` (115, 0) | Amp recipe: shared run home, metadata log row, WebSocket host, `wsToken` rule | 1, 4, 7 | Change | Ceremony goes; host row keeps one hint | about 75 data | None new. | Amp golden |
| Ceremony fields in schema and check (in `harness/*.rs`) | Parse, validate and default the fields marked "Cut" above: `interactive`, `identity`, `continuation`, `platform`, `argv_guard` (`check.rs:1113-1157`, `schema.rs:686-748`), `reserved_env_prefixes`, `real_dirs`, `providers` (`schema.rs:846-905`), retired-key refusals (`parse.rs:98-228`) | none | Cut | Each is one value, a default, a self-guard or a refusal of an old shape. No backward compatibility now. | about 550 / about 1,400 (overlaps section 1's check.rs cut; do not add) | A typo in a removed field is still refused by `deny_unknown_fields`. | `harness_schema.rs` cases: platform (41 mentions), providers (29), identity/headless (34), credential (40), argv_guard (7), reserved prefixes (4) |
| `crates/goalcup-protocol/providers.toml` (25, 0) | Two shared host rows | 1 | Cut | One user (Claude). D74 froze it; the owner lifted the freeze | 25 + resolver above | None. | provider cases in `harness_schema.rs` |
| `scripts/pin-manifests.sh` (274, 0) | Writes the directory digest table the build reads | none (anti-tamper of on-disk connectors) | Cut | Embed the four files with `include_str!` and hash the embedded bytes (section 1 row). Then no re-pin step on each of the 12 October connector commits | 274 / 0 | Release packaging (`scripts/package-release.sh:38`) and `GOALCUP_CONNECTORS_DIR` (`crates/goalcup-cli/src/config.rs:147`) assume a folder. `--dev` must still read one. | `connector_pins.rs` 303, `pins_build_fails.rs` 86 (counted in section 1) |
| `scripts/check-connector.sh` (314, 0) and `scripts/test_check_connector.py` (0, 411) | Runs pins, schema tests, goldens and kit for one connector | 7 (contributor check) | Cut | With embedding and no goldens it is one `cargo test connector_contract` line. Wired into CI (`.github/workflows/ci.yml:13`), PR template, CONTRIBUTING, AGENTS | 314 / 411 | A contributor loses the one-command report. A doc line replaces it. | `test_check_connector.py` |
| Connector goldens (`harness_contract_golden.rs` 975 CLI, fixtures 2,110) | Byte goldens of each connector's launch plan and daemon plan | none | Cut | Every connector edit forces a regen; `connector_contract.rs` already runs every connector | 0 / about 3,000 (section 8 cuts the verifier half) | Less early warning of an argv change. The live run still shows it. | these files |
| Frozen schema-1 fixtures and contributor guards (`schema-1-connectors` 354, `frozen-connectors` 237, `no_family_literals.rs` 378, `connector_template.rs` 120) | Hold the D68 compatibility promise | none | Cut | D68 is only "proposed" and contradicts "no backward compatibility" | 0 / about 1,090 (section 1 counts 600 of it) | A core change may name a harness again. Review catches it. | these files |
| `connector_contract.rs` (994) and `full/connector_contract.rs` (463), `shipped_connectors.rs` (411) | Static rules and one stand-in run per connector | 7 | Keep, trim | This is the real check that a connector launches and records | 0 / about 400 (pin and digest cases) | none | pin cases |
| `skills/adding-a-capture-harness/` (599 docs) | Contributor skill, templates, decoder guide | 7 | Change | Template shrinks with the schema; writing-a-decoder stays | about 200 docs | none | `connector_template.rs` |
| `docs/CONNECTOR-SCHEMA.md` (1,002 docs) | Field reference, compatibility promise, deprecation rule | 7 | Change | Drop the contract, compatibility and deprecation sections (lines 7-183) and cut-field tables | about 600 docs | none | none |
| `docs/HARNESS-ONBOARDING.md` (579 docs) | Procedure, stop rules, reviewer checklist | 7 | Change | Fix the SQLite stop rule (section 6); drop pin and golden steps | about 200 docs | none | none |
| `docs/harnesses/*.md` (911 docs) | Measured facts per harness | 7, research | Keep, fix stale lines | `hermes-cli.md:61` says `state.db` "is not captured"; Task 6 (#297) captures it as `final_snapshot`. `claude-code-cli.md:163-170` describe the 32-stream cap the owner is removing | 0 | none | none |

Totals for this lane alone (not counted elsewhere): about 400 data lines in connectors, 588 src
lines of scripts, 411 test lines of script tests, about 1,000 docs lines. The schema, pin and
golden cuts are counted in sections 1 and 8.

### 10.4 Can a connector be 10 lines?

Close. A Claude Code connector that keeps only fields that serve a point, after the cuts above
(flat keys, defaults for ceremony, `**` in native patterns, plain host names):

```toml
id = "claude-code-cli"
display_name = "Claude Code"
first_screen = "Claude Code may ask whether you trust this folder. Choose Yes."
program = "claude"
home = { env = "CLAUDE_CONFIG_DIR", run_home = "keychain_link" }
ca = { env = "NODE_EXTRA_CA_CERTS", mode = "extend" }
env = { NODE_USE_ENV_PROXY = "1", DISABLE_AUTOUPDATER = "1", CLAUDE_CODE_RETRY_WATCHDOG = "1" }
resume_args = ["--continue"]
hosts = ["api.anthropic.com"]
auth_hosts = ["platform.claude.com"]
model_host_from = { file = "settings.json", key = "env.ANTHROPIC_BASE_URL" }
native = [
  { root = "{config_home}/projects", pattern = "**/*.jsonl", hint = "claude-code-jsonl" },
  { root = "{config_home}/projects", pattern = "**/*", form = "snapshot" },
]
redact = [{ kind = "anthropic_billing_header", header_prefix = "x-anthropic-billing-" }]
no_prompts = [{ file = "settings.json", key = "permissions.defaultMode", values = ["bypassPermissions"] },
              { file = "settings.json", key = "skipDangerousModePermissionPrompt", values = ["true"] }]
```

That is 16 lines against 100 config lines today. Amp fits in about 11. Codex needs about 16
(its trust argument, `native_binary`, two hosts, three redaction rows). Hermes needs about 20
(`configure`, `launch_prep`, a `final_snapshot` row). So: about 10 to 20 lines, with one comment
line per non-obvious choice.

The `**` change matters for point 2, not only for size. Today Claude's subagent rows match fixed
depths (`*/*/subagents/agent-*.jsonl`, `*/*/subagents/workflows/*/agent-*.jsonl`,
`connectors/claude-code-cli/harness.toml:92-131`). The matcher handles one component per glob
part (`check.rs:322-351`), so a subagent written one level deeper than these shapes, by a new
Claude Code version, is silently not matched. A recursive pattern under the same root records it.
It also removes the need for `MAX_NATIVE_ROWS = 8` (`check.rs:26`), which equals the daemon's
`MAX_NATIVE_SOURCES` (`crates/goalcup-daemon/src/native_capture.rs:183`).

Risk: a recursive walk over a large `projects/` tree costs more per poll. The walker already
rotates through 8,192 entries per poll (D57), so cost is bounded, but a week-long run with
thousands of subagents is not measured. The new cap work (lane A2/B) must size it.

### 10.5 What OpenCode and OpenClaw need

Both were proven with unpinned `--dev` connectors in a Vercel Sandbox on 28 Oct: every trace
verified valid, model calls recorded through the Gateway and DeepSeek
(`origin/opencode-openclaw:docs/research/results/2026-10-opencode-openclaw.md`, "Runs"). The proof
connectors are 60 and 57 lines (handover, "Proof connectors"). Both facts sheets end with
"Status: facts only" because both keep transcripts only in SQLite.

**The SQLite stop rule is already obsolete.** Hermes ships a `final_snapshot` row that copies
`state.db`, `-wal` and `-shm` once after the harness exits (`connectors/hermes-cli/harness.toml:110-121`,
Task 6, #297). Hermes writes `state.db` while the run goes, yet the onboarding guide says such a
store "is refused" (`docs/HARNESS-ONBOARDING.md:188`). The guide and the shipped Hermes connector
disagree. The same row shape works for both new harnesses. D44 (a read-only snapshot of the run's
own rows) should be replaced by this.

| Need | OpenCode 1.19.4 | OpenClaw 2026.10.8 |
|---|---|---|
| Transcript (point 2) | `final_snapshot` row: root `{run_home}/.local/share/opencode`, pattern `opencode.db*` | `final_snapshot` row: root `{run_home}/.openclaw/agents`, pattern `*/agent/openclaw-agent.sqlite*` |
| Live leg while running | its log `opencode.log` as an append-only metadata row (proof) | its log, only after the participant sets `logging.file` into the home (default `/tmp/openclaw/`, outside `HOME`) |
| Home | `run_home = "shared"`, `config_home = "per_run"` (login and database follow `XDG_DATA_HOME`, which a connector may not set) | same shape in the proof; `OPENCLAW_HOME` as `home.env` is untested |
| CA | `NODE_EXTRA_CA_CERTS`, extend (Bun) | `NODE_EXTRA_CA_CERTS`, extend |
| Proxy | `HTTPS_PROXY` honoured; no `NODE_USE_ENV_PROXY` | `NODE_USE_ENV_PROXY = "1"`; but `proxy.proxyUrl` in `openclaw.json` or `OPENCLAW_PROXY_URL` in `~/.openclaw/.env` win over `HTTPS_PROXY` and send model traffic past the recorder (measured) |
| Model host (point 1) | No URL in its config by default (catalog), so `model_host_from` reads nothing. List default hosts: `ai-gateway.vercel.sh`, `api.deepseek.com`. Optional: `provider.*.options.baseURL` when set | Same: `ai-gateway.vercel.sh` (`/v1/messages`), `api.deepseek.com`. Optional `models.providers.*.baseUrl` |
| Decoder | Gateway wire is `/v3/ai/language-model` (AI SDK protocol): no decoder, so label `wire_undecoded`, which counts (D74). DeepSeek `/chat/completions` decodes | Gateway `/v1/messages` is Anthropic shape; DeepSeek `/chat/completions`; both decode |
| Self-update off | `OPENCODE_DISABLE_AUTOUPDATE = "1"` | `OPENCLAW_NO_AUTO_UPDATE = "1"` |
| Executable | native ELF | `#!/usr/bin/env node` script. The facts sheet says a pinned connector refuses it (row 10). The code refuses only an interpreter *name* (`capture_policy.rs:710-721`, `check.rs:687`), so `program = "openclaw"` should pass, like Codex's npm script. Unverified by a pinned load |
| Stop | Escape, Ctrl-C twice | `/exit` refused during retries; `goalcup seal --yes` ended it as `Unresolved` |
| Subagents | not measured | not measured |
| macOS | paths unknown | paths unknown |
| Participant docs | unknown model id falls back to a paid model silently; set `small_model`; `"permission": "allow"` | key in `~/.openclaw/.env`; Node 24.22.1+ |

What must still happen before either ships for Friday:

1. Measure subagents on both (point 2). Neither sheet has a subagent run.
2. Measure on macOS, the measured platform (D42). Every macOS cell is "unknown".
3. OpenClaw proxy override (point 1). Smallest fix: a `configure` command that unsets
   `proxy.proxyUrl` before each launch (the Hermes mechanism, D61). The `.env` form cannot be
   checked without reading a key file (D62 forbids it), so say it in the setup guide and rely on
   the evidence label: a full bypass shows `none`; a partial bypass is invisible.
4. A release that ships the connectors (D70: `--dev` never competes).
5. The host-list problem (section 8): both harnesses reach dozens of providers through a catalog,
   and only the listed hosts are recorded. The Gateway run of OpenCode shows
   `intercepted=1 tunneled_not_approved=18`; those 18 are expected to be the catalog and npm
   hosts, but "goalcup does not log the names of tunnelled hosts" (results file), so it is unverified.

### 10.6 Decisions that justify this lane, and whether they hold

| Decision | What it says | Holds under the seven points and owner decisions? |
|---|---|---|
| D22 | Kill the process tree; `native_pid` for npm wrappers | Yes (point 3: a clean stop seals the run). |
| D38 | Fresh per-run `HOME` with a Keychain link for Claude Code | Yes for login. But see blind spots: isolation means the participant's own settings, skills and MCP servers are not there (point 7 cost). |
| D39 | Extra redaction patterns; shared isolated homes, serialized; daemon refuses `real_dirs` | Patterns: yes, trimmed by option b. `real_dirs` refusal: no point served. |
| D41 / D43.3 | Replace-mode CA fails closed; CA name shapes are a closed list | Mode: yes (point 1). Closed name list: ceremony (section 1). |
| D42 | Harnesses are data; open harness PRs; "protocol freezes at v5" | Data: yes. Freeze: no longer applies. |
| D43.1, .4, .5 | One file; pins as the local gate; stranger PRs once CI runs | Pins: replaced by embedding. Stranger PRs: not a point. |
| D44 | OpenCode needs a read-only row snapshot form | Superseded in practice by `final_snapshot` (Task 6). Rewrite. |
| D46 | CLI and daemon bind to identical connector bytes | Moot with embedding: hash the embedded bytes once, keep `connector_dir_sha256` for the Worker. |
| D48 | No `manifest.toml`; loader refuses one | The refusal is backward compatibility; cut it. |
| D51 | Argv and env are data; review is the control | Yes, and it argues for cutting the deny lists and self-guards. |
| D52 | No model, sandbox, permission or MCP limit | Yes (point 7). |
| D53 | `version_args` only; no build pin | Yes. Make `--version` the default. |
| D57 | Chat mode; every session file since the start | Yes. It makes `interactive`, `identity` and `since_run_start` always one value: ceremony. |
| D59 | `NODE_USE_ENV_PROXY`; shared run home | Yes (point 1, point 7). |
| D61, D62, D72, D74 | Model host from the harness's own config; launch prep for Hermes 0.22; `providers.toml` frozen | Host reading and launch prep: yes (point 1). The freeze: lifted by the owner. D74's one-key reader leaves gaps (phase 2 notes 297-375). |
| D66 | Time-left tool | Owner ruling; serves no point. |
| D67 | Prep recommends no-prompt settings, never writes them | Yes. Tied to a measured stall. |
| D68 (proposed) | Schema 1 locked; compatibility and deprecation rules; frozen fixtures | No. Contradicts "no backward compatibility before Edition 01". Withdraw. |
| D70 | The release is the list | Yes. Simpler still with embedded connectors. |

### 10.7 Hidden couplings

- `MAX_NATIVE_ROWS` (`check.rs:26`) is the daemon's `MAX_NATIVE_SOURCES` (`native_capture.rs:183`).
  Change both or neither.
- Every connector edit today needs `sh scripts/pin-manifests.sh` (AGENTS.md:90), a golden regen
  and the PR checklist (`.github/PULL_REQUEST_TEMPLATE.md:30,45`). The owner's masking change
  to `connectors/codex-cli/harness.toml` is such an edit: re-pin, regen `harness-golden/codex-cli.json`,
  and update the Codex cases in `crates/goalcup-daemon/tests/it/{capture_redaction_extra,redaction,capture_harness_load}.rs`
  and three protocol fixtures.
- `protocol_hint` strings are a three-way contract: the connector writes them, the proxy stamps
  them on streams, the verifier picks a decoder by them (`crates/goalcup-protocol/src/hints.rs`,
  `crates/goalcup-verifier/src/capture_analysis/*`). Moving hints to a central path table keeps the
  recorded field, so the verifier is untouched.
- The Worker stores `connector_id`, `manifest_sha256` and `connector_dir_sha256` per run
  (`server/worker/src/db-supabase.ts:97-99,264-269`, a `hex64` column). Embedding must still send a
  64-hex value, or the SQL changes too.
- `scripts/package-release.sh:38` ships `connectors/` from `git archive`; `config.rs:147` finds them
  beside the binary or by `GOALCUP_CONNECTORS_DIR`. The matrix and kit scripts
  (`scripts/matrix.sh`, `kit_settle.py`, `linux-matrix.sh`) name connector ids and folders.
- `real_dirs` is read by the daemon at every create from every loaded connector
  (`crates/goalcup-daemon/src/runtime/capture/harness.rs:38`), so cutting it touches the daemon.
- `continuation` crosses the control API (`control/handlers.rs:391`) and the `Launch` record:
  cutting it is a protocol change, with `CONTROL_API_VERSION`.
- `SHELL`: a connector entry wins over the participant's (`launch.rs:331-333`). Cutting
  `platform` moves the choice into `launch.rs`.

### 10.8 What must change together

| Change | Protocol | Verifier | Worker | Client |
|---|---|---|---|---|
| Embed connectors, drop pins | drop `pins.rs`, `connector_dir.rs`, `pin-manifests.sh` | none | keeps a `connector_dir_sha256` value | `config.rs`, `package-release.sh`, matrix scripts; `--dev` keeps a folder |
| Cut ceremony fields | schema, parse, check | none | none | `launch.rs` (`SHELL`, argv guard), daemon (`real_dirs`, `continuation`), control API version |
| `**` in native patterns | `glob_matches` and pattern grammar | none | none | daemon walker; Claude connector |
| Central hint table | drop host-row paths and hints | none (field unchanged) | none | `commands/start/proxy.rs:153-187`, `proxy/metadata.rs` |
| Drop Codex rate-limit rows | none | the Codex fixture `native-cli-0.153.0.json` if it expects masks | none | connector, daemon redaction tests |
| Raise native caps (owner) | `MAX_NATIVE_ROWS` | none | `RUN_MAX_COMPRESSED_BYTES` owner call | daemon `NativeCaptureLimits` |

### 10.9 The smallest version of this area that still meets its points

- Four connectors (plus OpenCode and OpenClaw when measured), each 10 to 20 lines, embedded in
  the binary. `--dev` reads a folder for a new harness.
- Fields: `id`, `display_name`, `first_screen`, `no_prompts`, `program`, `args`, `resume_args`,
  `home` (`env`, `run_home`, `config_home`), `ca`, `env`, `hosts`, `auth_hosts`,
  `model_host_from`, `native` rows (with `**`), `native_binary`, `configure`, `launch_prep`,
  `redact`, `skills_dir`. Everything else is a default in code.
- One loader with `deny_unknown_fields`, template-token checks, glob grammar, host shape, "never
  Goal Cup's host", CA present (as section 1 proposes).
- One test that loads and launches every connector with the stand-in (`connector_contract`).
- Docs: one field table, one onboarding page, the facts sheets.

### 10.10 Blind spots

- Lines saved are estimates from field counts and line ranges, not a written patch.
- The isolated homes (D38, D39, D59) mean a participant's own Claude Code settings, skills, plugins
  and MCP servers are absent unless they add them again in Goal Cup's home. That is a real point 7
  cost, not scored as a cut here: running in the real home would make "every file since the run
  started" pick up the participant's other sessions, and Codex's `auth.json` refresh. Owner call
  after Friday.
- The `**` walk cost over a week with thousands of subagents is not measured.
- Whether `program = "openclaw"` loads pinned is read from code, not run.
- The names of the 18 tunnelled OpenCode hosts are unverified.
- The owner has not said which Hermes version the cup uses; two `configure` rows depend on it.

### 10.11 Short list

**Top cuts (lines saved, src / tests; data and docs noted)**

1. Pins and directory digest, replaced by embedded connectors: `pin-manifests.sh` 274 src,
   `check-connector.sh` 314 src, `test_check_connector.py` 411 tests (plus about 650 / 800 counted in section 1).
2. Ceremony fields (`schema_version`, `interactive`, `identity`, `continuation`, `platform`,
   `argv_guard`, `reserved_env_prefixes`, `real_dirs`, `since_run_start`, `version_args`,
   `grace_seconds`, `strategy`, `setup_section`) and retired-key refusals: about 550 src / about 1,400 tests,
   overlapping section 1's check.rs cut.
3. Connector goldens and frozen schema-1 fixtures: 0 / about 4,000 (part counted in sections 1 and 8).
4. `providers.toml` and its resolver: about 85 src.
5. Connector files: 556 to about 80 data lines; docs (schema, onboarding, skill): about 1,000 lines.

**Top changes**

1. Allow `**` in native patterns; Claude's six rows become two and catch any subagent depth.
2. Drop the four Codex rate-limit redaction rows (owner option b), with the test and fixture edits in one PR.
3. Pick the decoder hint from one central path table; host rows become plain host names.
4. Fix the SQLite stop rule in HARNESS-ONBOARDING and rewrite D44: `final_snapshot` already copies a database at the seal.
5. Withdraw D68; strip CONNECTOR-SCHEMA's compatibility and deprecation sections.

**Must add for the seven points**

1. Point 1: a record or a plain finish line for every tunnelled host, uncapped by the top five
   (`proxy/tunnels.rs:23`), so a provider missing from a connector's list is visible. This
   matters most for OpenCode and OpenClaw, whose providers come from a catalog. Proxy work is lane B.
2. Point 2: subagent runs for OpenCode and OpenClaw before they ship, and a Hermes version whose
   `delegate_task` works.
3. Point 1: OpenClaw's proxy override needs a `configure` unset of `proxy.proxyUrl` and a setup
   guide line for `OPENCLAW_PROXY_URL` in `.env`.
4. Point 6: update `docs/harnesses/hermes-cli.md:61` and `claude-code-cli.md:163-170` so participants read what is actually recorded.

Overlap: `check-connector.sh` and `test_check_connector.py` are also in section 11; the lane totals count them once. Of the ~4,000 golden and fixture test lines, ~600 are counted in section 1 and 540 in section 8, so about 2,860 are this section's own.

## 11. Scripts, gate, workflows and packaging

Lane J (`J-tests-gate.md` in this folder) covers the gate tiers, timings and flakes in depth; this section agrees with it and adds the per-script verdicts. "src" means the script itself; "tests" means its `test_*.py`.

### 11.1 The area in numbers

| Group | src lines | test lines | Python test methods |
|---|---|---|---|
| `scripts/` all files | 11,485 (incl. README 79, campaign.example.json 64, research-staging.sql 166) | 10,393 in 20 `test_*.py` | 577 |
| `.github/workflows/` | 276 (ci 93, linux-x86_64 63, release 120) | 0 | 0 |
| `packaging/` | 394 install.sh + 101 README | 614 (`scripts/test_install.py`) | 27 |
| `harnesses/install.sh` | 62 (tracked, though AGENTS.md calls `harnesses/` git-ignored) | 0 | 0 |

The three biggest files are test-kit tools, not product: `matrix.sh` 2,522, `campaign.py` 1,888,
`check-links.py` 1,349. Their tests are bigger still: 1,517, 1,335 and 1,672 lines.

Key fact for this lane: the live checks that proved Tasks 1 to 12 ("live PASS" in
`docs/plans/2026-10-29-record-everything-progress.md:24-35`) used a kit kept outside the repo
(`scratchpad/kit/before.md`, cited in `docs/plans/2026-10-29-record-everything-status.md:178-179`). No script in
`scripts/` changed for those tasks except `fetch-run.sh` (commits a71583a6, e64cf4c6, 5c5f61a8). The in-repo kit
(`matrix.sh`, `linux-matrix.sh`, `campaign.py`, `kit_settle.py`) last changed on 26 Oct. Whether it still passes on
main is unverified.

The 2-hour run report (`docs/research/results/2026-10-30-two-hour-run.md`) does not exist on any branch. Nothing
below relies on its content.

### 11.2 Verdicts, per part

Format: Part (path, src, tests) | What it does | Points | Verdict | Reason | Lines saved (src / tests) | Risk | Tests that go.

#### 11.2.1 Gate and CI checks

| Part | What it does | Points | Verdict | Reason | Saved | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| `scripts/gate.sh` (113 / `test_gate.py` 226) | `fast` and `full` tiers (gate.sh:1-21, 61-111) | all, indirectly (it runs the Rust and Worker tests) | **keep, change** | The one command that proves a change. Change: drop the `unittest discover` and `check-links` steps from `full` (gate.sh:107-111) once those scripts are cut; keep the root refusal (gate.sh:32-39) since the sandbox may run as root and needs `GOALCUP_GATE_ALLOW_ROOT=1` (unverified). Cut `test_gate.py`: 226 test lines guard a 113-line script that runs on every commit, and the script already warns itself when its filter names a missing binary (gate.sh:76-83). | 4 / 226 | A broken nextest filter could run all tests (slow, not wrong) or skip some (the gate's own warning catches renames only). | `test_gate.py` (16) |
| `scripts/check-version.py` (206 / 128) | Fails when the version is written anywhere but `Cargo.toml` | none (release hygiene) | **cut** | `release.yml:55-62` already refuses a tag that differs from `Cargo.toml`, and `smoke-release.sh:31-37` checks each binary prints it. Rules 4 to 6 only scan docs for version strings. Its `ALLOWED` list (check-version.py:45-48) exists for the "v5 frozen at v0.5.0" wording, which the owner withdrew. | 206 / 128 | A stale version number in a doc. Harmless for the event. Also edit `release.sh:101-104`, which calls it. | `test_check_version.py` (13) |
| `scripts/ci-scope.py` (90 / 134) | Skips the Rust CI smoke for docs-only PRs | none | **cut** | Saves runner minutes only. Run the smoke on every PR. Same call as lane J. | 90 / 134 | A few minutes of CI on docs PRs. | `test_ci_scope.py` (9) |
| `scripts/check-links.py` (1,349 / 1,672) | Relative Markdown link and anchor check, own GFM parser | none (doc hygiene) | **change: replace** | 3,021 lines for doc links. Replace with one CI step running lychee with `--offline --include-fragments` (both options exist: https://lychee.cli.rs/guides/cli/), or drop it until after the event. | 1,349 / 1,672 | lychee's slug rules may differ from GitHub's in edge cases (unverified); a pinned third-party action is a supply-chain input, so pin it by SHA as ci.yml does today. | `test_check_links.py` (116) |
| `scripts/test_codeowners.py` (0 / 56) | Every CODEOWNERS path exists | none | **cut** | GitHub flags bad CODEOWNERS lines on the file's own page. CODEOWNERS must change anyway when the protocol lane removes pin files. | 0 / 56 | A stale owner line goes unnoticed. | itself (3) |
| `.github/workflows/ci.yml` (93) | Light Linux CI: scope, 4 script tests, link check, build, fmt, clippy, tests of protocol, verifier, platform | 5 (the protocol and verifier tests) | **keep, change** | Remove the scope step (ci.yml:60-75) and the four `unittest` lines (ci.yml:76-79); swap the link check (ci.yml:80) for lychee or nothing. Drop `GOALCUP_TRACECAP_REQUIRE_QUALIFIED` (ci.yml:17-19, 90-93) if the disk reservation is cut (lane A1 cuts `run/reserve.rs`). | about 25 / 0 | CI never ran daemon or CLI tests (ci.yml:15-16), so it was never the proof; nothing is lost there. | none |

#### 11.2.2 Linux checks

| Part | What it does | Points | Verdict | Reason | Saved | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| `scripts/linux-check.sh` (106 / 107) | Fast gate plus slow CLI tests in Docker, with ext4 and XFS loop volumes (linux-check.sh:84-104) | 7 (Linux participants, the Vercel Sandbox) | **keep, change** | Linux is where the 2-hour run happens. The loop volumes exist for the disk reservation and the filesystem gate; if those go (owner: "Disk full: no special handling"), drop the volumes, `--privileged`, e2fsprogs and xfsprogs. About 60 lines remain. Cut `test_linux_check.py`. | about 45 / 107 | Without ext4 TMPDIR, any test still tied to the filesystem gate fails on overlayfs. Cut the volumes in the same PR that cuts the gate. | `test_linux_check.py` (4) |
| `scripts/linux-harness-smoke.sh` (242 / 0) | No-login launch of fixed Codex 0.153.0 and Claude Code 2.1.283 builds, by npm tarball digest (linux-harness-smoke.sh:1-40) | 7 | **cut** | The pinned builds go stale as harnesses ship (D53 says Goal Cup pins no build). A logged-in run in the Vercel Sandbox proves more. Only the manual workflow runs it. | 242 / 0 | Lose a cheap, key-free Linux launch signal on x86_64. | none |
| `.github/workflows/linux-x86_64.yml` (63) | Manual: linux-check and the harness smoke | 7 | **change** | Keep the `linux-check` job, drop the `harness-smoke` job (linux-x86_64.yml:45-63). | about 22 / 0 | none beyond the line above | none |

#### 11.2.3 Connectors and pins

| Part | What it does | Points | Verdict | Reason | Saved | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| `scripts/pin-manifests.sh` (274 / tested from Rust: `goalcup-protocol/tests/it/connector_pins.rs:140-291`, `goalcup-cli/tests/shipped_connectors.rs:206`) | Writes `pins/connector-dirs.txt`; its byte layout is written twice, here and in `connector_dir.rs` | none of the seven (trust, D43, D46) | **keep for now; cut with embedded connectors** | Section 1 proposes embedding connectors in the binary, which removes pins (section 1: the pins verdict row, its must-change-together table and its short list). If taken, this script and its Rust tests go together. Until then every connector edit, including the owner's masking change to `connectors/codex-cli/harness.toml`, needs a re-pin or every start refuses. | 274 / (Rust side counted by section 1) if embedded | Cutting it alone breaks `connector_pins.rs`. | none now |
| `scripts/check-connector.sh` (314 / 411) | Wraps five checks for one connector (files, pins, schema, goldens, kit) | 7 (adding harnesses), 1 (its Node proxy rule) | **change: shrink** | Most of it is a wrapper around cargo commands whose names other lanes are renaming or cutting (harness_contract_golden, harness_contract_kit, connector_contract). Keep a 30 to 60 line list of commands. The one real rule, "a Node harness needs `NODE_USE_ENV_PROXY=1` or no model traffic is recorded" (check-connector.sh:159-205, D59), belongs in `goalcup doctor` or `setup`, where a participant sees it (point 1). | about 255 / 411 | A contributor loses one-line PASS/FAIL per step. If the Node rule is not moved to Rust first, a new Node connector can ship recording nothing. | `test_check_connector.py` (27) |

#### 11.2.4 Release and install

| Part | What it does | Points | Verdict | Reason | Saved | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| `.github/workflows/release.yml` (120) | Builds 4 targets, packages, smoke-tests, pipes install.sh, publishes | 7 (people install it) | **keep** | D70: releases continue through the cup and the Worker refuses old versions, so the release path is live during the event. | 0 | none | none |
| `scripts/release.sh` (115 / 180) | `prepare` opens the version PR; `tag` tags merged `origin/main` | release | **keep script, cut test** | Small and run by a person who reads its output; release.yml's tag check backs it. | 0 / 180 | A broken helper is found when used; fallback is three git commands. | `test_release.py` (9) |
| `scripts/package-release.sh` (45) | One tarball per target, connectors from `git archive` | 7 | **keep** | Small. Changes if connectors are embedded (package-release.sh:38). | 0 | none | none |
| `scripts/smoke-release.sh` (55) | Runs the 3 binaries and `goalcup doctor --json` from the tarball | 7 | **keep, change with pins** | Its Python block reads `checks[].id == "manifest_pin"` (smoke-release.sh:50-53). That check goes if pins go. | about 10 / 0 if pins go | none | none |
| `scripts/third-party-licenses.py` (231 / 94) | Licence texts for the binaries' crates, with an allow list | release (licence duty for a binary release) | **keep** | Legal requirement for shipping binaries; small. | 0 | none | none |
| `packaging/install.sh` (394 / `test_install.py` 614) | Download, checksum, try-run, refuse during a recording run, stage, roll back | 7, 3 (never break a running run during a point release) | **keep, change** | The "do not replace goalcup while a run records" guard (install.sh:104-195) matters because releases continue through the cup (D70). Cut its stderr wording match (install.sh:168-183): it parses three CLI error sentences, a hidden coupling; on failure it can just say "unsure" and go on. Simplify the rollback (install.sh:197-241, 351-366) to "stage, then mv". Trim tests to the promises: checksum refusal, run-guard block, no partial install. | about 70 / about 350 | Fewer rollback paths: a kill mid-install could leave a mix of old and new binaries (already true for SIGKILL, install.sh:351-353). | about 15 of 27 in `test_install.py` |
| `packaging/README.md` (101) | Release layout and steps | 7 | **keep** | Docs. Update if connectors are embedded. | 0 | none | none |
| `harnesses/install.sh` (62) | Installs Codex, OpenCode, DeepSeek dsh, OpenClaw and Hermes unpinned into `harnesses/` | none | **cut** | Spike-era research script. Points to `docs/harness-research/`, which no longer exists (harnesses/install.sh:15). AGENTS.md says `harnesses/` is git-ignored, yet this file is tracked. Two harness sheets cite it (`docs/harnesses/codex-cli.md:107`, `docs/harnesses/hermes-cli.md:36`); edit those lines. | 62 / 0 | Someone who used it to set up spike harnesses must install by hand. | none |

#### 11.2.5 Live runs and the server copy

| Part | What it does | Points | Verdict | Reason | Saved | Risk | Tests that go |
|---|---|---|---|---|---|---|---|
| `scripts/fetch-run.sh` (349 / 695) | Rebuilds a trace from the DB chunk index and R2, checks SHA-256, length and offsets, writes chain, acks, receipt, seal, baseline, tunnels (fetch-run.sh:1-40) | 5 (the server can rebuild the story by ID) | **keep** | It is the only proof that the server copy rebuilds into something the verifier accepts. TODO.md:245 calls it "needed before judging". Must change with any protocol or Worker change to the file set or chunk index. | 0 | none | none |
| `scripts/matrix.sh` (2,522 / 1,517) | One real run of one connector against one server: login, start under a pty, prompt, seal, local verify, server-copy compare, "agent worked" check; `--test --duration`, `--refusals`, `--check-copy`, a fake connector | 7, 5, partly 1 | **keep for Friday, change; rewrite after the event** | The only in-repo driver of a real harness against a real server, and the natural tool for the 2-hour Vercel Sandbox run (`--test --duration 7200`, host mode, no Docker). Cut for Friday: `--refusals` (matrix.sh:1555-1692, the Rust tests cover refusals), `--check-copy` mode, the `m2-fake-tui` connector (matrix.sh:2448-2522). It does not check points 2 or 4 at all: no subagent count, no scan of the chain for secrets (grep finds neither). Its token push-leak scan (matrix.sh:1140-1320) checks the agent's GitHub pushes, not the recorder. | about 400 / about 500 | It parses CLI wording that other lanes may change: "shows the code" (`crates/goalcup-cli/src/participant/approval.rs:45`), `READY_MARK` "Recording in chat mode" (`participant.rs:76`, matrix.sh:1762), "uploaded: ... seal" (matrix.sh:1960-1979), "Nothing was created." (matrix.sh:1687). Unverified that it passes on main today. | the refusal and check-copy cases in `test_matrix.py` |
| `scripts/linux-matrix.sh` (876 / 1,219) | `matrix.sh` in Docker per connector, with images, volumes, login and GitHub sign-in | 7 | **cut** | The Vercel Sandbox is the Linux test machine now (owner decision), and it runs matrix.sh on the host. Docker images per harness version add upkeep. | 876 / 1,219 | Lose repeatable Linux cells on a Mac. The Vercel run must be set up by hand once. | `test_linux_matrix.py` (87) |
| `scripts/campaign.py` (1,888 / 1,335) + `campaign.example.json` (64) | Many long cells in parallel, drills (`network_cut`, `pause`), sampler, reports (campaign.py:1-60, 98-112) | 3, 5 (the drills) | **cut, or move out of the repo** | Research tooling for long soak runs on Tenki, which the progress doc puts after Friday (progress doc:4-5). Its one point-3 value, the network-cut drill, can be done by hand once in the 2-hour run. | 1,952 / 1,335 | No automated proof that recording continues through a network cut and the upload catches up (D64). Rust's `capture_failure_campaign` covers part of it. Do the manual drill. | `test_campaign.py` (73) |
| `scripts/kit_settle.py` (302 / covered inside `test_linux_matrix.py`, `test_campaign.py`) | Writes the cheapest-model and no-prompt settings into the tracker home; checks the run used them | none of the seven (test cost) | **cut** | Only campaign.py and linux-matrix.sh call it; matrix.sh does not. In the sandbox, set the model once by hand, as a participant would (point 7, D67). It hard-codes model names. | 302 / 0 | A run on a dearer model costs more money. Check the model in the harness settings before the 2-hour run. | with the two files above |
| `scripts/research-sample.py` (424 / 247) | CSV of soak samples per interval | none | **cut** | Only campaign.py uses it. | 424 / 247 | none for the event | `test_research_sample.py` (8) |
| `scripts/research-measure.py` (541 / 365) | Sizes, gaps, compression of chain parts | none (research) | **cut or move** | Research, not recording. Parses the D55 part grammar, which the protocol lane may change. | 541 / 365 | Researchers lose a sizing tool; it lives in git history. | `test_research_measure.py` (14) |
| `scripts/research-watch.py` (95 / 104) | Seals one entry after at most 1,800 s (research-watch.py:24-30) | none | **cut** | `matrix.sh --test --duration` does this. | 95 / 104 | none | `test_research_watch.py` (7) |
| `scripts/research-replay.py` (342 / 325) | Compares two traces with one verifier | none (research) | **cut or move** | Decoder research. | 342 / 325 | none for the event | `test_research_replay.py` (17) |
| `scripts/research-staging.py` (604 / 426) + `research-staging.sql` (166) | Read-only event aggregates: chunk sizes, ACK lag, heartbeat health, plus R2 listing | 5 (organizer sees uploads arrive during the event) | **change: keep the SQL, cut the Python** | The SQL alone is an event dashboard in `psql` (research-staging.sql:1-20). The Python adds R2 sizes and formatting. | 604 / 426 | No compressed sizes. The SQL must change with the Worker schema (it names columns of `server/worker/sql/goal-cup-tracer-upload.sql`). | `test_research_staging.py` (27) |
| `scripts/t25-disk-full.sh` (93 / 0) | Measures a run on a literally full disk, root and loop image | none (owner: no disk-full handling) | **cut** | D65 no longer holds. Goes with `crates/goalcup-cli/tests/full/t25_disk_full.rs` (1,173, lane A1) and `docs/research/t25-disk-full.md` (247). | 93 / 0 | none | none here |
| `scripts/test_early_crosscheck.py` (0 / 538) | Tests a spike script, `spikes/m2-first-release/e02/early-crosscheck.py` | none | **cut** | Tests research code in `spikes/`, not the product. | 0 / 538 | none | itself (21) |
| `scripts/README.md` (79) | One line per script | 6 for maintainers | **keep, rewrite** | Shrinks with the cuts; drop the TODO at README.md:79. | about 40 / 0 | none | none |

#### 11.2.6 Other `.github` files (outside the brief's list, noted only)

`CODEOWNERS` (57) names pin and harness-contract files that the protocol lane may remove; edit together.
`dependabot.yml` (17), issue templates (137), PR template (46): no verdict, cheap.

### 11.3 Decisions behind this area

| D | What it gives this area | Still holds under the seven points and owner decisions? |
|---|---|---|
| D3 | Verifier is its own binary; fetch-run and matrix call `goalcup-verify` | Yes |
| D42 | Open harness PRs, code-owner review: CODEOWNERS, check-connector | Yes, but review is the control, not the wrapper script |
| D43, D46 | Pins as a local gate; CLI and daemon compare digests: pin-manifests, smoke-release pin check | Weakened. Moot if connectors are embedded (section 1) |
| D53 | No build pin | Yes. `linux-harness-smoke.sh` pins builds by digest anyway, against its spirit |
| D55 | Chain part files and local pruning: research-measure, research-sample, matrix's long-run fetch path | Weakened by "disk full: no special handling"; changes with the protocol lane |
| D57 | Chat mode: matrix's pty prompt | Yes |
| D59 | `NODE_USE_ENV_PROXY`: check-connector's Node rule | Yes, and it serves point 1, so move it into `goalcup doctor` |
| D64 | No server error stops a run: campaign's network-cut drill | Yes (point 3). Test by hand once if campaign goes |
| D65 | Disk pause: t25-disk-full.sh, linux-check's ext4 and XFS volumes, ci.yml's TRACECAP variable | **No.** Owner dropped disk handling |
| D67 | Participant sets no-prompt settings: kit_settle sets them for test cells | Yes; the kit acts as the participant. Not needed if kit_settle goes |
| D69, D71 | Test run repo and `GH_TOKEN`: matrix `--test` token handling and push-leak scan | Yes for the product. The leak scan is a kit safety net, not a recorder check |
| D70 | Releases through the cup, old versions refused: release.yml, install guard | Yes. This is why the install guard stays |
| (none) | `check-version.py`'s v0.5.0 allowance (frozen v5) | **No.** Owner: "protocol v5 is frozen" no longer applies |

### 11.4 Hidden couplings (change these together)

| Script | Coupled to | Breaks when |
|---|---|---|
| `fetch-run.sh` | Worker DB chunk index and R2 key layout; the trace file set the verifier reads (chain parts, `acks.jsonl`, `receipt.json`, `seal.json`, `baseline.json`, `tunnels.jsonl`) | protocol, Worker or verifier change the file set, ack format or chunk table |
| `research-staging.sql` | Worker SQL tables `events`, `runs`, `entries`, `heartbeats`, `chunks`, `notices` (research-staging.sql:20-30) | Worker schema changes (section 9 cuts the disk-pause columns) |
| `matrix.sh` | CLI wording (section 2.5), `GET /v1/runs/<id>`, verifier flags `--capture-integrity`, `--research-json`, `--dev-key`, verdict `chain_pruned_locally` | CLI messages, Worker API or verifier output change. Two CLI tests pin the wording for it (`participant/tests.rs:76-86`) |
| `packaging/install.sh` | `goalcup status --json` keys `health` and `capture.run`; three stderr sentences (install.sh:177-180); tarball layout | the status payload changes (section 1 changes `StatusPayload`) or the connectors are embedded |
| `smoke-release.sh`, `package-release.sh`, `release.yml:79-86` | `share/goalcup/connectors` layout and doctor's `manifest_pin` check | connectors embedded or pins removed |
| `pin-manifests.sh` | Byte layout duplicated in `goalcup-protocol/src/connector_dir.rs`; run by Rust tests | pins removed (remove both sides) |
| `check-connector.sh`, `gate.sh` | Test target and module names: `harness_conformance`, `connector_pins`, `harness_contract_golden`, `harness_contract_kit`, `connector_contract`, binaries `full`, `m2_cli_lifecycle`, modules `m2_synthetic_vertical`, `capture_failure_campaign`, `upload_worker` (gate.sh:75, 86-93) | other lanes rename or cut those tests |
| `ci.yml` | `GOALCUP_TRACECAP_REQUIRE_QUALIFIED` and the platform crate's reservation tests | the reservation is cut |
| `linux-check.sh` | ext4 and XFS TMPDIR for the filesystem gate | the gate is cut |
| `CODEOWNERS` | pin and contract file paths | those files move or go |
| crates comments | `main.rs:444, 956`, `participant.rs:75`, `approval.rs:45`, `t25_disk_full.rs:15, 120-125, 731-1111` name scripts | the scripts are cut (edit comments; t25 test goes with A1) |

### 11.5 Smallest version of this area that still meets its points

| Keep | src | tests | Serves |
|---|---|---|---|
| `gate.sh` | about 110 | 0 | all (runs the tests) |
| `linux-check.sh`, slimmed | about 60 | 0 | 7 |
| `fetch-run.sh` | 349 | 695 | 5 |
| `matrix.sh`, slimmed | about 2,100 | about 1,000 | 7, 5 |
| `live-check.py` (new, section 6) | about 200 | about 100 | 1, 2, 4, 6 |
| `release.sh`, `package-release.sh`, `smoke-release.sh`, `third-party-licenses.py` | 446 | 94 | 7, release |
| `packaging/install.sh`, trimmed | about 320 | about 260 | 7, 3 |
| `pin-manifests.sh` (until connectors are embedded) | 274 | 0 here | trust |
| `check-connector.sh`, as a command list | about 60 | 0 | 7 |
| `research-staging.sql` | 166 | 0 | 5 (event dashboard) |
| workflows: ci (about 65), linux-x86_64 (about 40), release (120) | about 225 | 0 | 5, 7 |
| **Total** | **about 4,300** | **about 2,150** | |

Today the same area is about 12,300 src (scripts 11,485 + workflows 276 + install 394 + harnesses 62 + packaging README
101) and about 11,000 test lines (10,393 + 614). After the event, rewriting matrix.sh into one smaller live-run driver
is the next big cut (it would also take most of the remaining 1,000 test lines).

### 11.6 What must be added for the seven points

Nothing in `scripts/` today checks points 2 or 4 on a real run, and point 1 only as "at least one model exchange with
output" (matrix.sh:26-31, 2054-2114). The out-of-repo kit did some of this; its own subagent counter read 0 when the
chain held the subagent (progress doc:59). Add one small, read-only check, as the phase 2 notes proposed
(`docs/plans/2026-10-29-record-everything-phase2-notes.md:673-676`), and call it from matrix.sh after the seal:

1. **Point 1:** wire model requests (from `goalcup-verify --research-json`) next to the harness's own count (Claude
   assistant messages by `requestId`, Codex turns, Hermes `messages` rows). Print both and the difference; label
   warm-ups and helper calls. No byte match (owner decision).
2. **Point 2:** subagent files on disk under the harness home against native streams in the chain, matched by path
   (the path is the ID). Count only.
3. **Point 4:** search the local chain and the fetched server copy for the run's own secret values (provider key,
   `GH_TOKEN`), raw, base64 and percent-encoded. Reuse the matcher matrix.sh already has for git objects
   (matrix.sh:1267-1320). Read the values from 0600 files, never print them.
4. **Point 5:** for the 2-hour run, always rebuild the server copy with `fetch-run.sh` and verify it, not only for
   pruned runs.
5. **Point 6:** compare the finish lines the participant saw with the counts above.

Also, for the 2-hour Vercel run, no code: a prompt that asks for many subagents (workload text file), one manual
network cut (replaces the campaign drill, D64), and a note of `ulimit -n` (the owner asked to check open file handles
before raising stream caps).

What could go wrong: the reconcile can look broken when helper calls or retries differ by design (phase 2 notes,
Part D risk 5); labels must be explicit. A secret scan that only searches raw bytes misses encoded forms; that is why
the encoded forms are listed.

### 11.7 Short list

**Top cuts (src / tests):**
1. `campaign.py` + example + `test_campaign.py`: 1,952 / 1,335.
2. `check-links.py` + test (replace with lychee or nothing): 1,349 / 1,672.
3. `linux-matrix.sh` + `kit_settle.py` + `test_linux_matrix.py`: 1,178 / 1,219.
4. Research tools (`research-measure`, `-sample`, `-watch`, `-replay`, `-staging.py`) and their tests: 2,006 / 1,467.
5. Script-of-script tests with no product value (`test_gate`, `test_release`, `test_linux_check`, `test_codeowners`,
   `test_early_crosscheck`): 0 / 1,107.
6. `check-version.py`, `ci-scope.py`, `linux-harness-smoke.sh`, `t25-disk-full.sh`, `harnesses/install.sh`: 693 / 262.
7. Trims: check-connector (about 255 / 411), install.sh (about 70 / 350), matrix.sh modes (about 400 / 500).

In all about 7,900 src and 8,300 test lines, about 16,000 of the area's 23,300.

**Top changes:**
1. Keep matrix.sh as the one live driver through Friday; drop its refusals, check-copy and fake-connector modes.
2. Move the D59 Node proxy rule from check-connector.sh into `goalcup doctor` (point 1).
3. Install guard: keep the "never replace during a run" check, drop its stderr wording match.
4. ci.yml: no scope step, no script tests, no TRACECAP variable once the reservation goes.
5. Change `fetch-run.sh`, `research-staging.sql`, `smoke-release.sh`, `install.sh` and `pin-manifests.sh` in the same
   PRs as the protocol, Worker and connector changes they depend on (section 4).

**Must add:** one read-only `live-check` step (about 200 lines) for points 1, 2, 4 and 6, run after every live run,
plus `fetch-run.sh` and a server-copy verify on the 2-hour run.

**Blind spots:** I did not run any script, so I cannot say matrix.sh still passes on main. Vercel Sandbox user (root
or not) and filesystem are unverified, which decides whether the gate's root refusal and the ext4/XFS rules bite.
Cutting campaign.py removes the only automated network-cut drill. If the protocol lane does not embed connectors,
pin-manifests.sh and its couplings stay.

Overlap: `check-connector.sh` (~255 / 411) is also cut in section 10; the lane totals count it once. `pin-manifests.sh` is not in this section's total.

## 12. Cross-area couplings: what must change together

Each row is one change that more than one area named. "Areas" uses the section numbers above. All citations are
from the sections named; read them there for detail.

### 12.1 `MAX_OPEN_STREAMS` and `MAX_OPEN_CONNECTIONS` (one release, all four parts)

| Where | What reads the constant | Areas |
|---|---|---|
| Protocol | `crates/goalcup-protocol/src/capture.rs:58` (`MAX_OPEN_STREAMS = 128`) and `:60` (`MAX_OPEN_CONNECTIONS = 128`) | 1 |
| Daemon | `runtime/capture/sink.rs:425`, `ingest.rs:1335`, `redact/stream.rs:2769`, native split `native_capture.rs:279` (`MAX_OPEN_STREAMS / 4 = 32`) and `:293`; native `max_files` 1,024 at `native_capture.rs:187, 280`; tests that loop to the cap `runtime/capture/tests.rs:189-277, 2236` | 2 |
| Proxy | `proxy/api.rs:451-453` (the 129th stream sets the epoch `Disabled(LimitExceeded)`, never left, `api.rs:711`), `proxy/mod.rs:1463, 1470`, `api.rs:599`; the 503 cap `MAX_CONNECTION_TASKS = 1024` at `proxy/mod.rs:112` | 3 |
| Verifier | reader `reader.rs:1566-1568` (`connection_limit_exceeded`) and `:1694-1696` (`open_stream_limit_exceeded`, exit 2); separate `MAX_GOAL_STREAMS` 128 at `reader.rs:76-78, 1384`; decoders `session.rs:644, 755-766`, `tracker.rs:173`, `anthropic.rs:152`, `native/mod.rs:291` (and a shared 8 MiB native budget, `native/mod.rs:99-102`); test pinning 129 streams as exit 2 `tests/it/m2_capture_integrity.rs:1972-1984` | 8 |
| Worker | none: no Worker limit counts subagents, calls or streams | 9 |
| Connectors | `MAX_NATIVE_ROWS = 8` (`harness/check.rs:26`) equals the daemon's `MAX_NATIVE_SOURCES` (`native_capture.rs:183`); the handle budget's "about 25 descriptors" bound in section 7 assumes at most 8 sources | 1, 7, 10 |

Rules that follow. Raise the protocol constant, the daemon's native split and file cap, the proxy's terminal
disable, and the verifier's reader and decoder bounds in the same release, or large runs read "invalid evidence"
(2, 3, 8). Fix the native poll cost (1 s idle poll, one directory open per pass) in the same change (2). The values
proposed differ (section 13). An old verifier will call a new trace invalid, so whoever verifies uploaded runs needs
the new verifier (D70).

### 12.2 Control API version (D70)

`CONTROL_API_VERSION` is pinned at 12 from 0.7.0 to 15 Nov (D70), so every change below can ship only in the Friday
release, never in a point release after it (1). Each also needs the CLI mock (`crates/goalcup-cli/src/daemon/mock.rs:157-168`)
and the daemon status writer (`crates/goalcup-daemon/src/status.rs:83-90`) edited in the same PR (1).

| Change | Proposed by | Also touches |
|---|---|---|
| Drop six dead V1 fields in `StatusPayload` (`control.rs:283-318`) | 1 | daemon `status.rs:86-90`; `packaging/install.sh` reads `status --json` (11) |
| Drop status signing (`SignedStatus`, `control.rs:353-430`) and the CLI's signature check | 1 (optional), 5 (now), 2 (only when merged) | `entries.rs` pinned run key, `Launch.run_public_key`, verifier D63 receipt binding (2, 5) |
| Drop `continuation` (connector field and `Launch` field) | 10 | `control/handlers.rs:391`, `Launch` record (10) |
| Drop the chat goal stream (`CaptureGoalRequest`, `CHAT_MODE_GOAL`) | 4 | daemon `record_capture_goal`, verifier Launch reader and `MAX_GOAL_STREAMS`, mock `MOCK_GOAL_PRODUCER` (4, 8) |
| Drop the quota (`CaptureWorkspaceRequest.reserve_bytes`, `control.rs:548`) | 1, 2 | daemon quota set `runtime/capture/mod.rs:449`, CLI `run/reserve.rs:59-75` (1) |
| Drop seal-time `ProxyCounters` (`control.rs:683-699`) | 3 | daemon `writer.rs:191-194`, `finish.rs:455-462` (3) |

### 12.3 `MIN_COMPETITION_VERSION` (Worker)

It is empty today (`server/worker/wrangler.toml:55`) and checks competition only (`src/runs.ts:138-169`,
`src/device.ts:55`). Because the Worker accepts any chain bytes, it is the only thing that stops an older client
entering the cup with a trace the new verifier may reject (9). Every record or reader change in this appendix leans
on it: dropping `Launch` fields and the `Seal` body (1), lifting the reader caps (8), dropping the goal stream (4),
dropping `tracer_version` (6, 9). Set it to the Friday release before 7 Nov 00:00 UTC; the owner decides whether
practice is checked too (one line at `src/runs.ts:228` and `src/device.ts:55`). `continue` stays unchecked on
purpose (D70), though section 5 notes it could be checked now that there are no old runs to protect.

### 12.4 Signed receipts and acks, and the hash link

| Piece | Where | Areas |
|---|---|---|
| Byte layout | Worker `src/sign.ts:3-13, 8-106`; protocol `receipt.rs` (186), `ack.rs` (95); vectors `test/vectors/signing.json` and `signing_vectors.rs` | 1, 9 |
| Checks | uploader `session.rs:454-481` (ack check, which `acked_offset` and prune rely on); `fetch_receipt` (`upload/mod.rs:258-307`); verifier reader | 6, 8 |
| Pinned key | verifier `SERVER_PUBLIC_KEYS` (`lib.rs:34-35`) must equal the Worker's key; whether the 29 Oct redeploy kept it is unverified | 8 |
| Chain link | Worker refuses a chunk whose `Goalcup-Prev` is not its head (`chunks.ts:69-70`); headers `Goalcup-Prev`, `Goalcup-First-Seq`, `Goalcup-Head` | 1, 6, 9 |
| Rebuild | `scripts/fetch-run.sh` writes `chain.jsonl`, `acks.jsonl`, `receipt.json`, `seal.json`, `baseline.json`, `tunnels.jsonl` | 6, 11 |

All areas keep these for Friday. Cutting receipt signing after Edition 01 (~280 src, 1) touches `sign.ts`, the
uploader and the verifier together, and makes acks unverifiable locally (6).

### 12.5 Disk-full handling (D40 reserve clause, D65 pause)

The owner ruled "Disk full: no special handling". The code that goes is spread over seven areas and must go
together, or the Worker keeps a dead branch:

| Area | Lines |
|---|---|
| 7 | `disk.rs` reserve 239 / 137; filesystem gate ~60 / 30 |
| 5 | `run/reserve.rs` ~533 / ~423, `run/pause.rs` 211 / 149, watch disk branch and Pruner, `status.rs:96-102` (status releases stale reserves), vitals `disk_short`, `agent_paused`, `tests/full/t25_disk_full.rs` 1,173 |
| 4 | start-path reserve calls ~20 / ~310; setup filesystem `[fail]` |
| 2 | writer quota ~120 / ~200 |
| 1 | `export.rs` quota and terminal allowance, `CHAIN_QUOTA_BYTES`, `reserve_bytes` |
| 6 | heartbeat vitals `agent_paused`, `disk_short` (`api.rs:208-217`) ~10 / ~20; prune pressure branch |
| 9 | disk pause and `disk_full` email ~45 / ~120; `vitals` validator to an opaque 1 KiB object; SQL section 3 needs a website migration |
| 11 | `t25-disk-full.sh` 93; `linux-check.sh` ext4 and XFS volumes; `ci.yml` `GOALCUP_TRACECAP_REQUIRE_QUALIFIED` |

Write one new D entry that supersedes D65 and the D40 reserve clause (7). With no reserve, a real ENOSPC poisons
the chain writer for good (`tracecap.rs:361-383, 673-689`) and the daemon latches the failure
(`writer.rs:206-222`); the participant learns it only from `goalcup status` (2). Add a plain terminal line (2) and a
test that fills a small volume and shows the proxy keeps forwarding (7).

### 12.6 Tunnels and losses into the chain

Today unlisted-host tunnels are a side list written once at finish, capped at 10,000 rows
(`proxy/tunnels.rs:20`) with a top-five summary (`tunnels.rs:23`), and sent as `tunnels_jsonl` with the seal. Moving
them into the chain touches the proxy (`mod.rs:1149-1237`, delete `tunnels.rs`), a protocol record shape, the
daemon writer, the verifier report, the Worker store (`server/worker/src/runs.ts`, 8 MiB cap and
`goalcup-tunnels-v1` first line at `runs.ts:27-28, 73-85`), the uploader (`finish.rs:216-333`, `upload/api.rs:272`),
the Rust stand-in (`handlers.rs:1128-1185`) and `fetch-run.sh` (3, 6, 9, 11). The per-event `Loss` with connection
and host (3) is the same kind of change. Sections 3, 6 and 9 disagree on timing (section 13).

### 12.7 Embedding connectors and dropping pins

Protocol `pins.rs`, `connector_dir.rs`; `scripts/pin-manifests.sh` (its byte layout is written twice, here and in
`connector_dir.rs`), `check-connector.sh`; `scripts/package-release.sh:38`, `smoke-release.sh:50-53`
(`manifest_pin`), `release.yml:79-86`; CLI `config.rs:132-147` (`GOALCUP_CONNECTORS_DIR`); doctor's pin check; the
Worker's `connector_dir_sha256` column (a `hex64`, `db-supabase.ts:97-99, 264-269`), so embedding must still send a
64-hex value; `CODEOWNERS`; the kit scripts (1, 10, 11). Until it is done, every connector edit needs a re-pin, a
golden regen and the PR checklist, including the owner's masking change to `connectors/codex-cli/harness.toml:110-113`
(1, 10, 11).

### 12.8 Open-file limits

The proxy raises its own soft limit to 2,304 (`start/proxy.rs:27`, `OPEN_FILES_WANTED = 2 * MAX_CONNECTION_TASKS + 256`),
after the daemon is already started (`start/phases.rs:409` versus `:516`); the daemon never calls `setrlimit` (4).
Proposals in 3, 4, 5 and 7 all touch the same code: raise to the hard limit (clamped on macOS), in the CLI and
perhaps the daemon, and show `open_files` from the heartbeat vitals (`machine.rs:677-810`, `vitals.rs`) as a
problem line near the limit. Whether the daemon needs the raise at all is disputed (section 13).

### 12.9 CLI commands and wording that scripts parse

`matrix.sh` drives `seal --yes`, `entries --json`, `login`, `doctor` (`matrix.sh:167-729, 1424-1462, 1825-1831`) and
waits on wording (`READY_MARK` "Recording in chat mode", `participant.rs:76`; "shows the code",
`approval.rs:45`; "uploaded: ... seal"; "Nothing was created."). `packaging/install.sh:100-160` greps `status --json`
for `health` and `capture.run`, and matches three stderr sentences (`install.sh:168-183`). `smoke-release.sh:39-45`
and `linux-harness-smoke.sh:204` call `doctor`. Cutting or changing those commands (5) and the status payload (1)
must update the scripts in the same PR (11).

### 12.10 Four copies of the server rules

Worker handlers (`runs.ts`, `chunks.ts`), `db-local.ts` (701 src), the SQL in the website repo
(`agentcommunity_page/supabase/migrations/`, mirrored in `server/worker/sql/goal-cup-tracer-upload.sql`), and the
Rust stand-in `crates/goalcup-cli/tests/common/upload_server/` (2,514 test lines) (6, 9). Size limits also live in
several places: chunk 32 MiB (`chunks.ts:21-22`, client `chunk.rs:12-13`), seal documents 16 MiB (Worker
`runs.ts:26`, client `finish.rs:254-264`, verifier `reader.rs:145`, `prune.rs:103`), tunnel list 8 MiB. The version
field names `proxy_version` and `tracer_version` cross the client (`api.rs:14-69`, `account/device.rs:386-387`), the
Worker (`util.ts:79-91`) and the stand-in (`handlers.rs:345-417`); keep the DB column name `tracer_version`.

### 12.11 Merging the daemon into `goalcup start` (after Edition 01)

Sections 2, 3 and 4 each estimate part of the same change: protocol `control.rs` and ingest handshake types; CLI
`daemon/` (1,444), `retry.rs` (474 src), `daemon/mock.rs` (636), `proxy/ingest.rs` (1,136 src), `commands/seal.rs`,
`status.rs`, `doctor.rs`, `entries.rs`, `continue_run.rs`; the orphan `Recovery` rule kept in four layers (proxy
emitter, daemon receiver, writer, verifier reader); `Launch.run_public_key` and D63 receipt binding; packaging (one
binary fewer); promise P5 (`README.md:286`), whose tests may not be deleted without the owner. Do not add the three
estimates: 2's "3,000 to 4,000 src in the CLI and protocol" contains most of 3's ~1,300 and 4's ~3,500 to 4,000.

### 12.12 Docs that are stale in more than one area

`docs/CHEAT-CHECKLIST.md:26-28` describes an hourly server verify job that does not exist (1, 6, 8; also
`docs/UPLOAD-API.md:320, 462`, `docs/SERVER-DESIGN.md:157-160`). `docs/SERVER-DESIGN.md:159-160` says nothing downloads
the chain, but `fetch-run.sh` does (8, 9). `docs/UPLOAD-API.md:494-496` and `docs/OPERATING.md:488` say the Worker
sends mail through Resend (9). `docs/RULES.md:294-296` promises a secret-shaped text scan the verifier does not have
(8). `docs/HARNESS-ONBOARDING.md:188` refuses a live SQLite store that the shipped Hermes connector already copies
(10). `docs/harnesses/claude-code-cli.md:163-170` describes the 32-stream cap (2, 10).

## 13. Where areas disagree

Each row gives both views. None is settled here; most need an owner call or a measurement from the 2-hour run.

| Topic | One view | Other view |
|---|---|---|
| New value for `MAX_OPEN_STREAMS` | Section 1: raise to 4,096, drop the native `/4` split; worst-case held redactor memory 64 MiB, say so beside the constant | Section 2: raise to about 1,048,576 or remove it; native streams hold no descriptor, memory is tens of MiB at 20,000 streams. Section 3: raise, and above all make the 129th stream a per-stream loss, not the end of wire capture |
| New value for `MAX_OPEN_CONNECTIONS` | Section 1: 1,024, the proxy's own `MAX_CONNECTION_TASKS` | Section 3: about 65,536 or remove it, and drop the 1,024 connection cap (503) too, since a refusal blocks the harness |
| Does the daemon need its own `RLIMIT_NOFILE` raise? | Section 4: yes. The daemon runs at the shell's soft limit (often 256 on macOS); raising the native caps without a daemon raise will hit EMFILE | Sections 2 and 7: native streams hold no descriptor between polls (`native_capture.rs:76-81, 1415-1460`); the daemon holds about 20 to 40 descriptors whatever the subagent count. The pressure is the proxy's sockets. Section 7 still lists "raise in goalcup and the daemon" as a must-add |
| Should the harness inherit the raised limit? | Section 2: the harness inherits the raised limit (said as a fact, no change asked) | Section 7: put the limit back before exec so the harness runs as the user runs it (point 7) |
| Chain parts and local pruning | Section 1: cut parts and pruning (with the uploader lane); the uploader reads by offset; the owner ruled disk full needs no handling. Section 5: cut the in-run Pruner; keeping local evidence is safer for point 5 | Section 6: change, do not cut; at the 1 GB/h design target a 168-hour run is about 168 GB raw on the participant's disk. Keep one rule: delete a closed part once a signed ack covers it. Sections 2 and 7: D55 rotation holds |
| Size of a week-long run | Section 9: the largest measured 2-hour run was 73.5 MB, about 0.6 GB compressed for 168 h | Section 6: the design target is 1 GB/h, about 17 GB compressed for a week. Both agree 64 GiB per run is enough and that record-everything sizes are unmeasured |
| Tunnel list timing | Section 3: move tunnels into the chain now and delete `tunnels.rs`, the seal field and the Worker store (~230 / ~200 client side) | Sections 6 and 9: keep the tunnel list for Friday (Task 3 merged 28 Oct, live PASS); move it into the chain after Edition 01 |
| Spawn gate (D40) | Section 4: cut it (~340 / ~230). The proxy is up before the spawn, so every model call is recorded anyway; kill the child if registration is refused | Section 2: the spawn gate serves point 1 ("no unrecorded start"); only the 180 s staged finish budget is heavier than needed |
| Workspace walk and Git digest in the seal | Section 2: change, owner decision; cut about 800 / 600, keep only `check_workspace_available`. GitHub already records the history | Section 1: keep `seal.rs`; judges need the product record. Section 6 cuts `repo_head` because `seal.json` already carries `git.commit`; that reason falls if the Git digest goes. Section 7 leaves `clone.rs` to "the seal lane's call" |
| Status signing | Section 5: drop the signature check and the daemon call now (~560 / ~1,110 with `entries.rs`) | Section 2: keep for Friday, cut when the daemon is merged. Section 1: optional cut, Friday release only (D70) |
| `providers.toml` | Section 1: keep the file, unfreeze it (D74) so any provider can be listed | Section 10: delete the file and its resolver (~85 src); write `api.anthropic.com` in the connector |
| `check-connector.sh` | Section 10: cut it (314 / 411); one `cargo test connector_contract` line replaces it | Section 11: shrink it to a 30 to 60 line command list (~255 / 411) and move the D59 Node proxy rule into `goalcup doctor` or `setup` first |
| `goalcup doctor` | Section 5: cut it; move `executable_check` into setup | Section 11 proposes moving the D59 Node rule into `doctor` (or `setup`); `smoke-release.sh` keeps reading doctor's `manifest_pin`. If doctor goes, the Node rule must go into setup |
| `goalcup login` | Section 5: cut it; make the server a constant with an organizer override so every run uploads | Section 4 keeps row 8 (refuse `--practice` or `--test` without `goalcup login`). Cutting login without changing `start/server.rs:35-41` would make every run local-only |
| `retry.rs` | Section 4: change later, ~400 / ~100 (based on a wrong 995-line src count) | Section 5: one schedule after Friday, ~250 / ~450. Counted once, in section 5 |
| Filesystem gate | Section 7: cut it with the reserve (keep one NFS and SMB warning at most) | Sections 4 and 5: keep it as a warning in setup (today setup fails while start only warns) |
| Writer failure after a write error | Section 7: change, start a fresh part after a stated gap so a transient failure does not end recording for the week (protocol, daemon, verifier together) | Section 2: the latch is low risk; the gap is that the participant is not told, so add a plain terminal line |
| Subreaper and termination | Section 5: cut `subreaper.rs` now (owner call); D45 says Unresolved termination does not lower integrity | Section 2: keep `terminate/` for Friday (a Codex npm wrapper may outlive the deadline); notes two killers exist today |
| Running `matrix.sh` for the 2-hour run | Section 11: keep `matrix.sh` as the one live driver through Friday | Section 5's command cuts (`seal`, `entries`, `login`, `doctor`) break it unless it is changed in the same PR |
| Deny lists and harness checks | Sections 1 and 10: cut them (~1,780 / ~3,400); D51 already says review is the control | Section 4 keeps `harness_arg_refusal` (D51, row 49) as the guard that no argument sends calls around the recorder. Not a direct conflict, but the launch-time check stays while the load-time deny lists go |
