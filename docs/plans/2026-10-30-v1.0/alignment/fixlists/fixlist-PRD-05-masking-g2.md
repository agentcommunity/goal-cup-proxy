# Fix list for PRD-05-masking.md (triage group 2)

Source reviews: align-X1-run1, align-X2-scope, align-X3-builder, align-PRD-00 to align-PRD-06.
The ledger (OWNER-DECISIONS.md) wins. An id shared by several fix lists carries the same change text in each;
make in this file only the part that names this file. Items are ordered by severity, then id.

Counts: 3 blocker, 7 major, 15 minor, 25 in all.

## G2-01 (blocker)
- Sources: X3-1, P05-7
- Section and lines here: Builder notes step 1 (610-611)
- Same id also in: PRD-02-recorder-core, PRD-16-orchestration-acceptance
- Ledger item or rule: Ledger M1 (masking before any byte reaches disk), point 4
- Change:
  PRD-02 builder note step 2: replace "that unlocks phase 1a" with "that unlocks phase 1a only once PRD-05's core-slice masker (option b, left guard, `boundary()`) is merged; the stub masker is for fake-upstream tests only".
  PRD-05 builder note step 1: "Publish the S2 stub, which stars every byte as `fail_closed` (same length, with `boundary()`), so no build ever writes a raw byte. Tests that need clear bytes use a test-only masker under `#[cfg(test)]`."
  PRD-16 section 3 Phase 1a, add: "Precondition: PRD-05's core slice is merged on the commit under test, checked by a `mask_rules` length above zero in the `start` event. No real key enters a pad box before that."

## G2-34 (blocker)
- Sources: X1-5, P05-2, P05-14
- Section and lines here: 'Owner decisions it carries out' (26-28); section 4 rule and why (193-194, 205-209), cost (229-230); Overridable defaults 'Guard class' (592); section 12 participant text (458-459)
- Ledger item or rule: Ledger M1 (locked: accepted loss is 'after a letter or digit'); B2a; also owner item O-1
- Change:
  In "Owner decisions" add: "Guard class `[A-Za-z0-9_-]`: `_` is part of a word boundary; `-` is an addition for base64url blobs that needs the owner's yes, recorded in the ledger under M1 (O-1). If the owner says no, the class is `[A-Za-z0-9_]` and `mask_rules.rs` keeps a case showing the in-blob `-sk-` hit it lets through." Line 592: Value cell gets "(not yet agreed)". Section 12 participant text: "a key written right after a letter, a digit or `_`" plus "or `-`" only if the owner says yes.

## P05-1 (blocker)
- Sources: P05-1
- Section and lines here: Section 4 escape table rows 1, 2, 4 (198-201); 'One predicate' (222-227); `mask_rules.rs` guard cases
- Ledger item or rule: Point 4; ledger M1 (the accepted loss is only 'after a letter or digit', not after a newline)
- Change:
  Row 1: "an odd run of backslashes, then one letter or digit" becomes "a run of one or more backslashes, then one letter or digit". Row 2: "odd backslashes, `u` and 4 hex, or `x` and 2 hex" becomes "one or more backslashes, `u` and 4 hex, or `x` and 2 hex". Row 4: allow the ESC spellings after one or more backslashes. Add to the example column `\\nsk-…` and `\\\\nghp_…` (keys after a newline in JSON nested in JSON). `mask_rules.rs` guard cases: "a key after `\\n` and after `\\\\n` masks". (Checked: today's row 1 says "odd run"; with 2 backslashes the `n` counts as glue and the key is dropped.)

## G2-06 (major)
- Sources: X3-6, P00-11
- Section and lines here: Builder notes step 1 (610)
- Same id also in: PRD-00-index, PRD-02-recorder-core, PRD-11-connectors-harnesses
- Ledger item or rule: PRD-00 merge rule 1 and file ownership
- Change:
  PRD-00 phase 0 step 5: "PRD-02: the scaffold: `Cargo.toml` with every dependency the PRDs named, `lib.rs` with the full module list, S1 (with `for_tests` and `reserve`), S4 and S8 as compiling stubs, and an empty placeholder `mod.rs` in every other module folder of `crates/goalcup/src/`. This is the one ownership exception; each owner replaces its own placeholder in its first merge." Add: "PRD-05: the S2 stub in `src/mask/` by H+1 (it stars every byte, G2-01). PRD-11: the S3 stub in `src/connector/` by H+1, before the host list."
  PRD-02 builder note 1: add "placeholder module roots for every other folder (PRD-00 phase 0 step 5)".
  PRD-05 note 1 and PRD-11 note 1: "replace the scaffold's placeholder with the real S2 (S3) stub by H+1".

## G2-18 (major)
- Sources: P00-6, P00-23, P05-4
- Section and lines here: 'Owner decisions it carries out' (34-37); section 2 rows 17 and 20 (129, 132); section 9 top (334); Overridable defaults (594, 597)
- Same id also in: PRD-00-index
- Ledger item or rule: Ledger M1 (locked masking scope; owner said 'nothing more'); VISION 'a limit says what it guards'; also owner item O-2
- Change:
  PRD-00 owner question 3: change "Default: yes." to "Not in the ledger. Built in the second slice only once the ledger records the owner's yes; without it, PRD-05 builder step 6 is skipped and MORNING.md lists it as not built." Smaller rulings: add "The 32-value limit guards `Masker::new` start time and memory (PRD-05 A5); the count of values past it is shown to the participant."
  PRD-05: at the top of section 9 and in "Owner decisions" add "Not yet in the ledger (PRD-00 owner question 3). Built in the second slice only after the owner's yes; until then builder step 6 waits." Mark rows 17 and 20 and the Stripe pair the same.

## P05-3 (major)
- Sources: P05-3
- Section and lines here: Section 7 `mask_meta` rule 2 (307-313); section 2 row `codex_turn_state` (153); A3; `mask_meta.rs`
- Ledger item or rule: Ledger M1 ('keep every secret rule'); point 4
- Change:
  Section 7 rule 2, add: "or equals, lower-cased, the field name of a connector `json_field` rule (so `x-codex-turn-state` and `x-codex-installation-id` are masked as headers too)." A3 Pass: "`x-codex-turn-state` header values are all `*`". `mask_meta.rs`: "a header named like a connector `json_field` is masked".

## P05-5 (major)
- Sources: P05-5
- Section and lines here: Acceptance A6 (541), A1 (536)
- Ledger item or rule: Ledger B3 (short real test after each core feature); provable acceptance
- Change:
  A6 Pass: "With the core-slice masker: 0 hits of the real keys and the GitHub token (raw, base64, percent, joined per stream) in the part files, found by a value grep since the secret scan is not built yet; every shaped planted value has a MASK; A2 passes. The no-shape value and the Stripe pair are checked only after the second slice, in A1."

## P05-6 (major)
- Sources: P05-6
- Section and lines here: Section 6 table row 'Masker panic' (267); Interfaces row PRD-02 (496)
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Ledger R4 (lost bytes where known); PRD-01 invariant 4
- Change:
  Row "Masker panic" and PRD-02 section 4: "PRD-02 drops the piece and the held bytes, replaces the masker and writes GAP `mask_failed` (12) from the last released offset to the end of the dropped piece (held bytes included; the worker knows both offsets)."

## P05-8 (major)
- Sources: P05-8
- Section and lines here: Acceptance A2 (537); 'How a watcher proves it' (639-643)
- Ledger item or rule: Acceptance provable without misreading
- Change:
  A2 Pass: "0 MASK records of a rule other than `fail_closed` inside an `encrypted_content` JSON string." Watcher paragraph: "any span other than `fail_closed` inside an `encrypted_content` string".

## X2-02 (major)
- Sources: X2-02
- Section and lines here: Files it owns (479, after 482); 'What is cut' (517-526)
- Same id also in: PRD-14-tests-gate
- Ledger item or rule: Ledger L3 (copy the corpus), L2; VISION 'Less code'
- Change:
  PRD-05 line 479: "about 5,000" becomes "about 2,000". After line 482 add: "Tests copy the corpus as data, the split-at-every-byte property test run over the whole corpus, and only the old cases that a PRD-17 lesson names or that pin a rule's edge (PEM pieces, escapes, the left guard, `url_password`, fail-closed per message). Other old cases are not ported; the property test over the corpus covers them." Line 526: "to about 2,000".
  PRD-14 line 114: "2,000 (PRD-05)"; line 122 total "about 15,000"; line 124-125 keeps "Masking is the largest share" only if still true, else drop it.

## G2-12 (minor)
- Sources: X3-17, P00-14, P05-9, P02-16, 06-12
- Section and lines here: Lines 34, 129, 132, 140, 162, 163, 193, 336, 349, 417, 458, 463, 556, 594, 597, 621
- Same id also in: PRD-16-orchestration-acceptance, PRD-02-recorder-core, PRD-00-index, PRD-06-uploader-compression
- Ledger item or rule: Clarity; ledger is the source of truth
- Change:
  Replace revision 2 owner question numbers with PRD-00's current eight. Map: old 1, 2, 4, 6, 7 are question 1; old 8, 9, 10, 11, 13 are question 2; old 12 is 3; old 3, 14, 15 are 4; old 5 is 5; old 16 is 6; old 19 is 7; old 17 and 18 are 8. Write "owner question N (old M)" where a reader may meet the old number.
  PRD-00 line 681: "owner question 2". PRD-00 review answers: write each as "old N (now M)".
  PRD-02: "owner question 2" (line 37); line 120: "This is PRD-00's S1 block (revision 3, rulings R3-1 and R3-2)"; header: "Index: PRD-00 revision 3".
  PRD-06: every "owner question 6" and "owner question 2" becomes "PRD-00 owner question 1"; in "Owner decisions it carries out" tag each line with its ledger id: F2 (zstd), U1 (prune, small chunks, capped backoff that resets on progress), R5 (no caps), R6 and R7 (written counts, `failed`), L5 (one process), B4 (staging).

## G2-35 (minor)
- Sources: X2-06, P05-13 (rename superseded)
- Section and lines here: Files it owns (479); Interfaces PRD-14 (505); A5 (540); ask 5 (657)
- Same id also in: PRD-14-tests-gate
- Ledger item or rule: Ledger L2; ledger section 7 test rule (a test stays if it proves a point)
- Change:
  PRD-14: delete the release-mode test row at 294; G1 drops "the `release_mask_bench` report".
  PRD-05: `mask_bench.rs` holds one `#[ignore]` test named `by_hand_mask_bench`, in no gate tier, run once by hand for PRD-02 R8 and A5, its number in the night log. Same words at 479, 505, 540 and ask 5.

## G2-36 (minor)
- Sources: P05-18, P04-25 (part)
- Section and lines here: Acceptance A4 (539)
- Ledger item or rule: Acceptance provable from a real run
- Change:
  "Every complete JSONL line of every native stream, joined across the streams of one `path` and `gen` by offset, with no gap and no `fail_closed` span in it, parses."

## P05-10 (minor)
- Sources: P05-10
- Section and lines here: Section 4 escape table row 2 (199)
- Ledger item or rule: Clarity
- Change:
  Change the example to `\u0041sk-…`, `\x41ghp_…`.

## P05-11 (minor)
- Sources: P05-11
- Section and lines here: Section 3 last bullet (186-189); Open asks
- Same id also in: PRD-01-record-format
- Ledger item or rule: Interface agreement
- Change:
  PRD-01 section 8: add "except a `fail_closed` span, which may cover quotes; such a body is not expected to parse". PRD-05 Open asks lists it.

## P05-12 (minor)
- Sources: P05-12
- Section and lines here: Section 3 (182-183)
- Ledger item or rule: Interface agreement (PRD-01 section 8, PRD-02 section 2 item 4)
- Change:
  Add: "When PRD-02 splits a returned piece into several DATA records, it splits each span at the DATA edges (PRD-01 section 8)."

## P05-15 (minor)
- Sources: P05-15 (applies only if exact values are built, G2-18)
- Section and lines here: Section 9 'Order, duplicates and the guard' (369-374); start event (379)
- Same id also in: PRD-10-cli-participant
- Ledger item or rule: Points 4 and 6
- Change:
  PRD-05: count only the values past 32 in `skipped`; drop gate refusals from it. PRD-10: when `skipped` is above 0, the start screen says "N key values were not added to masking."

## P05-16 (minor)
- Sources: P05-16
- Section and lines here: Section 9 gate, first bullet (355-357); `mask_values.rs`
- Ledger item or rule: Ledger S2 (usage stats readable)
- Change:
  "its name is credential-named: the last segment of the key path, or the variable or dotenv name, contains …". Add a `mask_values.rs` case for `auth.model`.

## P05-17 (minor)
- Sources: P05-17
- Section and lines here: Risks row 'Privacy' (583)
- Ledger item or rule: Point 6 (only true claims)
- Change:
  "Not secrets. Participant told. PRD-09's usage output never copies plan, credits or rate limits (PRD-09:211); RULES says raw records are cleaned before anyone else gets them."

## P05-19 (minor)
- Sources: P05-19
- Section and lines here: Section 6 'Fail closed is never silent' (283-284)
- Ledger item or rule: Point 6; ledger R6
- Change:
  Line 284: "status shows it when it is not zero (PRD-10)". Add: "a `fail_closed` span is a loss of model data, recorded by its MASK records (offset and length) and COUNT, not by a GAP".

## P05-20 (minor)
- Sources: P05-20
- Section and lines here: Builder notes step 2 (612)
- Ledger item or rule: Clarity for the builder
- Change:
  "Set `MAX_HELD_BYTES` to the literal 16 KiB; `MAX_SEGMENT_BYTES` goes with segments."

## P05-21 (minor)
- Sources: P05-21
- Section and lines here: Interfaces PRD-03 (497); PRD-11 row (502); 'Must not touch' (488)
- Same id also in: PRD-03-proxy
- Ledger item or rule: Less code; clarity
- Change:
  PRD-03 row: "PRD-05 copies the header rules into `src/mask/meta.rs`; PRD-03 copies none." PRD-05 lines 488 and 502: "PRD-11's new Codex connector file has no rate-limit, plan or credit rows and its comment says headers are recorded".

## P05-22 (minor)
- Sources: P05 'also small' notes
- Section and lines here: Section 7 rule 1; `mask_rules.rs`
- Ledger item or rule: Clarity; PRD-17 K12
- Change:
  Section 7 rule 1: "all rules above" becomes "all shared, connector and exact-value rules". `mask_rules.rs` names PRD-17 K12's two old URL credential cases (81 and 190 characters) that must stay masked.

## X2-11 (minor)
- Sources: X2-11
- Section and lines here: 'Review answers' section and fully ruled 'Asks' sections
- Same id also in: PRD-00-index, PRD-01-record-format, PRD-02-recorder-core, PRD-03-proxy, PRD-04-native-capture, PRD-06-uploader-compression, PRD-07-worker-server, PRD-08-verifier-checks, PRD-09-usage-research-stats, PRD-10-cli-participant, PRD-11-connectors-harnesses, PRD-12-test-pad, PRD-13-cuts, PRD-14-tests-gate, PRD-15-docs-decisions, PRD-16-orchestration-acceptance, PRD-17-lessons-old-code
- Ledger item or rule: Ledger L2; point 6 (plain, true text). Apply last, after every other fix, so line numbers in the fix lists stay valid
- Change:
  Move every "Review answers" section and every fully ruled "Asks" section into one file `docs/plans/2026-10-30-v1.0/REVIEW-HISTORY.md`, verbatim, one heading per PRD. Before moving, copy any ruling still binding into the PRD body. Each PRD keeps one line at its end: "Review history: REVIEW-HISTORY.md, section PRD-NN." The replan keeps new asks in the Amendments note only.

## X3-22 (minor)
- Sources: X3-22, P05 'also small' (MaskError from a participant value)
- Section and lines here: Interfaces PRD-10 row (501); section 9
- Same id also in: PRD-02-recorder-core
- Ledger item or rule: Seam gap; point 6
- Change:
  PRD-02 section 8: "A recorder that cannot build its `Masker` from built-in shapes exits 78 before `ready`; the watchdog passes 78 to PRD-10, which shows PRD-05's fixed text."
  PRD-05: "A `MaskError` from a built-in shape fails `start` (exit 78). A `MaskError` from a participant value (for example a compile limit) skips and counts that value; it never fails `start`."
