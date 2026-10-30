# Owner list (triage group 2)

Findings from align-X1, X2, X3 and align-PRD-00 to PRD-06 that change a locked ledger item or add something the
ledger does not ask for. Each has a recommended answer. The fix lists already make the PRD text say "pending the
owner" for these, so nothing is built against the ledger in the meantime. Best asked when the owner wakes,
before H+0 (ledger B7).

## O-1. Left guard class: may `-` (and `_`) count as glue? (G2-34; X1-5, P05-2, P05-14)
- Ledger M1 (locked): "add a left word boundary (a key typed right after a letter or digit may be missed:
  accepted)". PRD-05 uses the class `[A-Za-z0-9_-]`.
- `_` is a word character in the usual meaning of a word boundary, so it fits M1 as written. `-` does not.
- Why the plan wants `-`: Codex's `encrypted_content` is base64url (`[A-Za-z0-9_-]`). With `-` and `_` in the
  class, no guarded prefix can match inside such a blob, which is the run 1 alteration the boundary exists to fix.
  Without them about 2 in 64 in-blob hits remain.
- Cost: a key written right after `-` or `_` is not masked (`token-ghp_...`, `KEY_sk-...`).
- **Recommended answer: yes to `[A-Za-z0-9_-]`.** Record it in the ledger under M1. If no, the class is
  `[A-Za-z0-9_]` and a test shows the in-blob `-sk-` hit it lets through.

## O-2. Exact-value masking and the wider Stripe rule (G2-18; P00-6, P00-23, P05-4, P05-15)
- Ledger M1 lists the masking scope; the owner's words were "option b ... nothing more". PRD-00 owner question 3
  adds, with default yes: mask by exact bytes the credential-named values from workspace `.env` files, the launch
  environment and the harness's key files (at most 32 values, 20 to 256 bytes), and widen the Stripe rule to test
  keys and webhook secrets (the pad prompt is a payments platform).
- It is not in the ledger's Open table, so a builder cannot tell it was agreed.
- **Recommended answer: yes to both.** `cat .env` is the biggest real leak path (point 4), and the pad plants a
  no-shape value that only exact matching catches. Built in the second slice. The 32-value limit guards start
  time and memory; values past it are counted and shown to the participant.

## O-3. One `test-hooks` feature against L6 "test seams cut" (G2-19; P00-7, P02-13, 06-21)
- Ledger L6 (locked) cuts "legacy and test seams". The old seams were the `test-hooks` and `test-support`
  features and `GOALCUP_TEST_*` hooks. PRD-00 and PRD-02 re-add a `test-hooks` feature and new `GOALCUP_TEST_*`
  hooks with no owner line.
- **Recommended answer: allow one narrow seam.** One compile-time `test-hooks` feature in `crates/goalcup`
  only, off by default, never in a release build (PRD-14 greps the shipped binary for `GOALCUP_TEST_`). Restart,
  guard and rotation cannot be proven from a real run in the gate's time. No hooks in `goalcup-verify` unless
  PRD-08 names one. Pure functions take their clock as an argument instead of a hook (the uploader's backoff).

## O-4. The watchdog does more than restart the recorder (G2-17; P00-3, P02-9)
- Ledger R3 (locked): "A tiny watchdog that only restarts the recorder is allowed." The planned watchdog also
  binds the port, mints and holds the CA, spawns, shields and ends the harness, runs helper stops, owns the
  deadline and null mode, and raises the file limit (about 450 lines).
- **Recommended answer: accept this reading.** The watchdog is the `goalcup start` process the participant
  already runs; spawning and ending the harness and the helper stops are launcher duties it had anyway. It holds
  the socket and the CA only so a restart keeps the same port and trust ("on the same port fast"). It never reads,
  masks, writes or uploads a recorded byte, so L5 (one recording process) holds. If no, the launcher work moves
  into a separate small parent and the restart loop stays alone, at the cost of one more process.

## O-5. Keep or cut the sleep lock (X2-03 part a)
- Not asked for by any ledger item or point. About 150 lines copied from `goalcup-platform/src/awake.rs` on three
  systems; PRD-10 already marks it "can be dropped if the day runs out". A sleeping machine pauses the harness
  too, so nothing goes unrecorded; the run only loses wall time.
- **Recommended answer: cut it from 1.0.** The launch block and the guide say once: keep the computer awake and
  on mains power. A suspend is written as a `suspend` event (PRD-01). PRD-10's CLI total then falls to about
  2,600 lines.

## O-6. For your macOS run (low priority; from X1-8)
- Ledger T9 leaves macOS testing to the owner. If the open-file raise fails on a soft limit of 256, about 96
  connections fit and long-lived WebSockets may keep the rest waiting.
- **Recommended answer: in the macOS run, once ask the agent for at least 100 subagents** and read the
  `fd_limit` note and the status line. No build work needed.

## Dropped or superseded findings (for the record)

| Finding | Why |
|---|---|
| X1-2 (record a coded body as it came when the decoder guard is full) | Dropped. The masker cannot read brotli or zstd bytes, so coded bytes would reach disk unmasked: breaks ledger M1 and point 4. G2-23 (decoder windows inside the one guard through `reserve`) and G2-26 (`lost` 0 with the wire size in the tail) cover the loss honestly |
| P01-10 (delete the seal's `dirty` digest) | Dropped. It is PRD-00 owner question 4, default yes: "the seal records the workspace git HEAD and a digest of uncommitted changes". P02-22 (`--no-optional-locks`) stays as a fix |
| P02-17 (drop `Recorder::reserve`) | Superseded by G2-23: PRD-03 now uses `reserve`, so it has a caller and decoder memory sits in the one guard |
| P02-4 fix text (use `open_conns()` for the grace) | Superseded by G2-24: one accessor, `quiet_since()` |
| X2-04 "keep only `idle()`" | Superseded by G2-24: `idle()` means "no connection open", the wrong signal for the grace |
| P01-3 "no COUNT event in a recovery frame" | Superseded by G2-21 (P02-1's `counts.jsonl`): the recovery frame carries the dead boot's COUNT, so totals stay exact recounts |
| P05-13 (rename the bench to `release_mask_bench`) | Superseded by G2-35: the release-mode gate step is cut and the bench runs by hand |
| P02-8 and X1-3 "5 minutes, no doubling" | Merged in G2-16 with L1 F6's "30 s, then every 60 s", the reading closest to R3's "restarts fast" |
| X1-3, X1-5, X1-11 ("same finding as ...") | Merged into G2-16, G2-34 and G2-31 |
| X2 "Also found by other alignment parts" list | Pointers to ids triaged with the L files; only those that X1, X2, X3 or a PRD-00 to PRD-06 review also raised are in these lists |
