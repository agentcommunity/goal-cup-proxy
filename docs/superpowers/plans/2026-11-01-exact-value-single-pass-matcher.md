# Single-Pass Exact-Value Matcher Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Check and merge the masking lane's single-pass trie matcher for exact values (PRD-05). Task 4 (one literal tracker per stream, no change to what is masked) is deferred to after 1.0 unless the one-hour runs show the recorder falling behind.

**Architecture:** The lane's branch `v1.0/prd-05-masking-exact-trie-matcher` (head `21419832`, 4 code commits and 1 docs commit on top of `04044320`) already does the replacement. Up to 480 per-form regexes are gone. One trie over every form (`RunSecrets` in `values.rs`) does a set simulation that takes the line-break grammar in every gap. `find_all` feeds both the whole-record matcher and the release scan, through `rules::exact_candidates` and `Policy::claims`. The hold walks the same trie (`LiteralWalk`). Wave A checks this work, adds the missing edge-case tests, measures it (Tasks 1 and 2 share ONE release box) and hands it to the one train. Wave B (Task 4, deferred) would replace the per-start `Walk::Literals` candidates with one `LiteralTrack` per `Scanner`.

**Tech Stack:** Rust (crate `goalcup`), `regex` 1.x (kept only as the test oracle for exact values), cargo-nextest, Vercel Sandbox build boxes via the installed `box.mjs` (`$GC_TOOLS/box.mjs`, `/root/gc-tools` tonight; the repo copy has no `node_modules`).

**Spec:** `/root/gc-night/finish/FINISH-PLAN.md` (section 1 item 7, Wave 1 "Masking (PRD-05)", Wave 2 exit "trie merged"), `docs/plans/2026-10-30-v1.0/PRD-05-masking.md` sections 9 to 11 and the amendment "2026-11-01, single-pass exact-value matcher (owner decision)" on the branch, `/root/gc-night/lanes/integrator.log` 22:05Z owner answer.

## Global Constraints

- Owner 22:05Z: "the next masking task is a single-pass Aho-Corasick matcher for the exact values (no change to what is masked), before the 2-hour runs."
- What is masked does not change. That means the same values, the same forms (`encoded_forms`), the same break grammar ("LF or CR LF, raw, or escaped with one or two backslashes before each of `r` and `n`, at most one break per gap", never an escaped break right before a backslash of the form), the same block claims, priorities and rule names (`run_secret`, `fail_closed` last).
- Exact-value masking uses sources 1 to 3 only. Values are held in memory, never written, hashed or logged. `Debug` prints a count, never a byte.
- `MIN_VALUE_BYTES = 20`, `MAX_VALUE_BYTES = 256`, `MAX_VALUES = 32`, `MAX_FORM_BYTES = 7 * 256`. A value has 11 forms, 13 with one escape kind, 15 at most.
- Same-length masking: masked bytes are `*`, one per byte, the quoted JSON and HTML-safe forms included (`mask_values.rs` `json_masked`, `a_value_with_html_bytes_is_masked_in_html_safe_json`).
- Streaming: the joined output of a stream equals `mask_whole` however the input is split, as long as no hold reaches `MAX_HELD_BYTES = 16 * 1024` and live candidates stay at or below `MAX_CANDIDATES = 64`. Past either bound the stream fails closed (`fail_closed`).
- Word guard: exact-value forms carry no guard (the old `Pattern::exact` had `guard: false`). The shared rules keep their left word boundary (OWNER-DECISIONS M1).
- Targets (PRD-05 A5, section 11): `Masker::new` under 1 s and 64 MiB with 32 values of 256 bytes, and 50 MB/s on one worker. "No speed number fails a gate" (PRD-00). A floor test may only run in a release build.
- The masker runs off the relay, so masking cost never slows the harness (point 3).
- No new dependency in `crates/goalcup/Cargo.toml`: "A PRD that needs one more names it in the night log; PRD-02 adds it."
- B2: at least two independent reviewers read each diff, one for correctness, the seven points and masking, and one for code quality and less code. The branch's code already had Reviewer A and the breaker (ledger 21:54Z, 23:17Z); the new tests and numbers get one correctness reviewer and one code-quality reviewer, and no third pass. B10: do not overbuild tests. "Less code is better. Cut before you add."
- Nothing is compiled on the conductor. Every build and test runs in a Vercel Sandbox box through `box.mjs run`, or in the one train's gate. No box per task or per fix round: the lane gets ONE box, at the end of Tasks 1 and 2 together. Reviewers are read-only. The branch merges in the one train (the conductor's one request, one gate on the combined head, bisect on failure), never alone; this lane writes no `.req`. One build per commit, at most 2 pad boxes at once, no probe boxes, smallest box, short timeouts.
- Tests use generated values only, never a real secret. No key on any command line.
- Push only `v1.0/*` branches. Merge commits only, never force. No writes to `main`, tags, production or the website database. Never edit `OWNER-DECISIONS.md`.
- No model names and no em dashes in docs. Commit trailers: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6`.

## Review Focus

1. **A value of 256 escape bytes** (a mix of `"`, `\`, `<`, `&`, giving forms near 1,792 bytes). The old per-form regex could hit a compile limit and skip such a value. The trie never skips one. Expected: counted `workspace 1, skipped 0`, masked in its nested HTML-safe form, stream equals `mask_whole`. Test `a_value_of_256_escape_bytes_is_used_and_masked` (Task 1).
2. **A push cut inside a line break.** Cuts at every byte of a wrapped value: between CR and LF, between `\\r\\` and `\\n`, right after the first character. Expected: the stream equals `mask_whole` and every value byte leaves starred. Test `a_wrapped_value_cut_at_every_byte_streams_like_mask_whole` (Task 1).
3. **One form inside another.** Value B is value A plus 10 bytes, so a trie end node has children. Also a value written twice back to back. Expected: both are masked whole, with no raw tail and no raw second copy. Test `a_value_inside_a_longer_value_and_back_to_back_values_are_masked` (Task 1).
4. **Backslash runs before values** (1 to 8 backslashes before each of 32 values, repeated to 256 KiB). Every `\u` form starts on a backslash, and a backslash both forks a `LiteralWalk` and starts escaped breaks. Expected: no `fail_closed` byte, and the stream equals `mask_whole`. Test `backslash_runs_before_values_never_fail_closed` (Task 1). Task 4 reruns it against `LiteralTrack`.
5. **Invalid UTF-8 next to a value** (a `\xff` byte right before it, right after it, and right after a second copy that a raw LF break wraps). `match_view` stands one byte for one byte, so offsets hold. Expected: the value is masked the same whole and streamed, and the invalid byte leaves raw. Test `a_value_next_to_invalid_utf8_is_masked_whole_and_streamed` (Task 1).

---

## File Structure

All paths are on branch `v1.0/prd-05-masking-exact-trie-matcher` unless a task names another.

| File | Responsibility | Tasks |
|---|---|---|
| `crates/goalcup/src/mask/values.rs` | Values, forms, the trie (`RunSecrets::new:254`, `build:289`, `find_all:450`, `spawn:482`), the hold walk (`LiteralWalk:491`, `step:503`, `matched_any:542`), and the oracle tests (`mod tests:746`, `the_trie_finds_what_the_per_form_matchers_found:902`) | 2, 4 |
| `crates/goalcup/src/mask/rules.rs` | `exact_candidates:392`: trie matches as claims | read in 1 |
| `crates/goalcup/src/mask/stream.rs` | `Policy::claims:782`, `Candidate::spawn` Literal arm `:1731`, `Walk::Literals` step `:1774` and `secret_from:1815`, `Scanner:1911`, `feed:1974`, `follow:2094`, `step_unit:2124`, `release:2195`, `finish_cut:2369`, `mask_whole:2444` | read in 1, change in 4 |
| `crates/goalcup/tests/mask_values.rs` | The public-API exact-value tests (Review Focus 1 to 5) | 1 |
| `crates/goalcup/tests/mask_bench.rs` | `by_hand_mask_bench:70` (ignored, prints) and `exact_values_mask_above_a_floor:237` (release only, floor `rate > 5.0` at `:318`) | 2 |
| `docs/plans/2026-10-30-v1.0/PRD-05-masking.md` | Section 11 "Measured again" (`:537`) and the amendment (`:1206`) | 3, 4 |

Box jobs are bash files in the lane's scratch folder `/root/gc-night/jobs/`, never in the repo. `<wt>` is the lane worktree `/root/gc-wt/builder-exact-trie`. If that worktree is gone, the lane script recreates it from `origin/v1.0/prd-05-masking-exact-trie-matcher`. Never work in `/home/user/goal-cup-proxy`.

---

## Wave A: verify, measure and merge the lane's branch

### Task 1: Review the branch and pin the edge cases

**Files:**
- Test: `crates/goalcup/tests/mask_values.rs` (append after `thirty_two_values_stream_escaped_text_without_failing_closed`)
- Read: `values.rs`, `rules.rs:392-416`, `stream.rs` hunks of `git diff 04044320...21419832`

**Interfaces:**
- Consumes: `Masker::new(&[MaskRule], Vec<SecretValue>) -> Result<Masker, MaskError>`, `Masker::stream`, `Masker::mask_whole(&[u8]) -> Masked`, `Masker::value_counts() -> ValueCounts`, `Masker::mask_rules() -> Vec<String>`, and from `common`: `Rng`, `ALNUM`, `starred(&Masked, Range<usize>) -> bool`, `stream_in(&Masker, StreamKind, &[u8], &[usize]) -> Masked`
- Produces: five tests that run on both the old matcher and the new one

- [ ] **Step 1: Two reviewers read the diff `04044320...21419832`** (B2). The correctness reviewer confirms each line below and writes a finding for any that fails:
  - `exact_candidates` builds the same `Claim` as the old `candidates` did for `Pattern::exact`: `priority = base + form`, `start`, `end = char_end(view, end)`, `whole_end = end`, `block: true`, `soft/cut/bounded: false`, `name_end = start`.
  - `base = self.len() - self.secrets.len()` in `Policy::claims` equals the old `lit` of form 0.
  - `RunSecrets::next` takes the same language as the test oracle's `wrapped_regex`, in particular `(Break::None, b'\\')` only when some child is not `\`, with `no_backslash` set when one is.
  - `find_all` keeps, per form, the leftmost matches that do not overlap.
  - `Policy::starts` still offers `Form::Literal { form: 0 }` only.
  - The quality reviewer lists any code that the trie made dead: `Progress`, `wrapped_regex` and `Advance` outside tests; the `MaskError` "compile limit" path for values.
- [ ] **Step 2: Write the five failing-if-changed tests** in `mask_values.rs`:
```rust
#[test]
fn a_value_of_256_escape_bytes_is_used_and_masked() -> Res {
    let mut rng = Rng(0x0E5C_0256);
    let value = rng.chars(b"\"\\<&ABCabc012", 256);
    let m = masker(&value)?;
    let c = m.value_counts();
    assert_eq!((c.workspace, c.skipped), (1, 0));
    // the longest form: HTML-safe JSON-escaped twice (a JSON string nested in one)
    let twice = html_safe(&html_safe(&value));
    let input = format!("{{\"t\":\"{twice}\"}}\n").into_bytes();
    let whole = m.mask_whole(&input);
    assert!(starred(&whole, 6..6 + twice.len()));
    let cuts: Vec<usize> = (1..input.len()).step_by(97).collect();
    assert_eq!(stream_in(&m, StreamKind::Native, &input, &cuts), whole);
    Ok(())
}

#[test]
fn a_wrapped_value_cut_at_every_byte_streams_like_mask_whole() -> Res {
    let v = token();
    let m = masker(&v)?;
    for brk in ["\r\n", "\\r\\n", "\\\\r\\\\n", "\\r\\\\n"] {
        let input = format!("x {}{brk}{} y\n", &v[..10], &v[10..]).into_bytes();
        let whole = m.mask_whole(&input);
        assert!(starred(&whole, 2..2 + 10) && !holds(&whole.bytes, &v[10..]));
        for cut in 1..input.len() {
            assert_eq!(stream_in(&m, StreamKind::Native, &input, &[cut]), whole, "cut {cut}");
        }
    }
    Ok(())
}

#[test]
fn a_value_inside_a_longer_value_and_back_to_back_values_are_masked() -> Res {
    let mut rng = Rng(0x1B51_DE00);
    let a = rng.chars(ALNUM, 24);
    let b = format!("{a}{}", rng.chars(ALNUM, 10));
    let m = Masker::new(&[], [&a, &b].iter().map(|v| SecretValue::new(ValueSource::Workspace, v.as_bytes().to_vec())).collect())?;
    let input = format!("k={b} {a}{a} end\n").into_bytes();
    let whole = m.mask_whole(&input);
    let starts = [2, 2 + b.len() + 1];
    assert!(starred(&whole, starts[0]..starts[0] + b.len()));
    assert!(starred(&whole, starts[1]..starts[1] + 2 * a.len()));
    let cuts: Vec<usize> = (1..input.len()).collect();
    assert_eq!(stream_in(&m, StreamKind::Native, &input, &cuts), whole);
    Ok(())
}

#[test]
fn backslash_runs_before_values_never_fail_closed() -> Res {
    let mut rng = Rng(0xBA5E_0032);
    let values: Vec<String> = (0..32).map(|_| rng.chars(ALNUM, 256)).collect();
    let m = Masker::new(&[], values.iter().map(|v| SecretValue::new(ValueSource::Workspace, v.clone().into_bytes())).collect())?;
    let mut input = String::new();
    let mut i = 0;
    while input.len() < 256 * 1024 {
        input.push_str(&"\\".repeat(1 + i % 8));
        input.push_str(&values[i % 32]);
        input.push_str(" \\u0041\\n\n");
        i += 1;
    }
    let input = input.into_bytes();
    let whole = m.mask_whole(&input);
    assert!(values.iter().all(|v| !holds(&whole.bytes, v)));
    let cuts: Vec<usize> = (1..input.len() / 4096).map(|k| k * 4096).collect();
    let got = stream_in(&m, StreamKind::Native, &input, &cuts);
    assert_eq!(got, whole);
    let fail_closed = m.mask_rules().len() - 1;
    assert!(got.spans.iter().all(|s| s.rule != fail_closed));
    Ok(())
}

#[test]
fn a_value_next_to_invalid_utf8_is_masked_whole_and_streamed() -> Res {
    let v = token();
    let m = masker(&v)?;
    let mut input = b"a\xff".to_vec();
    input.extend_from_slice(v.as_bytes());
    input.extend_from_slice(b"\xff b ");
    input.extend_from_slice(&v.as_bytes()[..8]);
    input.extend_from_slice(b"\n");
    input.extend_from_slice(&v.as_bytes()[8..]);
    input.extend_from_slice(b"\xff\n");
    let whole = m.mask_whole(&input);
    assert!(starred(&whole, 2..2 + v.len()));
    assert_eq!(whole.bytes[1], 0xff);
    let cuts: Vec<usize> = (1..input.len()).collect();
    assert_eq!(stream_in(&m, StreamKind::Native, &input, &cuts), whole);
    Ok(())
}
```
- [ ] **Step 3: Write the Task 1 half of the shared box job** `/root/gc-night/jobs/trie-box.sh` (no box is run here; Task 2 Step 3 runs it). The job starts with `cargo fmt --all --check` (two of Sunday's three masking gates failed on format or a missing fixture, not on logic). The half runs `cargo nextest run --locked -p goalcup --test mask_values` on the branch head. Then it checks out `04044320` with this test file copied over and runs the same command (the old regex matcher is the oracle).
Expected once the shared box has run: both runs PASS, so the output is unchanged. A test that passes on one matcher and fails on the other is a finding. A test that fails on both is a gap that was already there (for example, first-wins selection leaving the tail of value B raw where value A claimed its start). Report it to the conductor as a PRD-05 finding. Do not change selection in this slice, because what is masked must not change. Mark the test `#[ignore = "pre-existing: <finding id>"]`. Exception: on `04044320`, a FAIL of `a_value_of_256_escape_bytes_is_used_and_masked` only on `skipped` is the expected compile-limit difference. Name it in the amendment (Task 3) as the one stated change: such a value is now masked where it was skipped.
- [ ] **Step 4: Fix all reviewer findings and shared-box test failures in one batch** in the owning file (`values.rs`, `rules.rs` or `stream.rs`). No second box: the fix is checked by the one train's gate.
Expected: the gate PASSes the combined head.
- [ ] **Step 5: Commit**
```bash
git add crates/goalcup/tests/mask_values.rs
git commit -m "test(mask): pin the trie matcher's edge cases against the regex matcher"
```

### Task 2: Measure the matcher alone and the stream's share

**Files:**
- Modify: `crates/goalcup/src/mask/values.rs` (`mod tests`, after `walks_agree`)
- Test: same file; `crates/goalcup/tests/mask_bench.rs` is run unchanged

**Interfaces:**
- Consumes: `RunSecrets::new(&[SecretValue]) -> (RunSecrets, ValueCounts)`, `RunSecrets::find_all(&[u8]) -> Vec<(usize, usize, usize)>`, test helpers `Rng`, `text(&mut Rng, &[Vec<u8>]) -> Vec<u8>`
- Produces: three numbers for Task 3 and the Task 4 decision: `F` (`find_all` MB/s), `S0` and `S32` (one stream of the generated Codex request, with no values and with 32 values, from `by_hand_mask_bench`), and `R = S32 / S0`

- [ ] **Step 1: Write the release-only test**
```rust
#[test]
fn find_all_alone_is_far_above_50_mb_s() {
    if cfg!(debug_assertions) {
        println!("find_all_alone_is_far_above_50_mb_s: skipped: debug build");
        return;
    }
    let mut rng = Rng(0xA11_0256);
    const SET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
    let values: Vec<SecretValue> = (0..32)
        .map(|_| SecretValue::new(ValueSource::Workspace, (0..256).map(|_| rng.pick(SET)).collect()))
        .collect();
    let (secrets, counts) = RunSecrets::new(&values);
    assert_eq!(counts.workspace, 32);
    let forms: Vec<Vec<u8>> = (0..secrets.len()).map(|i| secrets.form(i).to_vec()).collect();
    let mut view = Vec::new();
    while view.len() < 4_000_000 {
        view.extend_from_slice(&text(&mut rng, &forms));
    }
    let mut best = std::time::Duration::MAX;
    for _ in 0..3 {
        let t = std::time::Instant::now();
        std::hint::black_box(secrets.find_all(&view));
        best = best.min(t.elapsed());
    }
    let rate = view.len() as f64 / 1e6 / best.as_secs_f64();
    println!("find_all_alone_is_far_above_50_mb_s: {rate:.1} MB/s");
    assert!(rate > 50.0, "find_all below 50 MB/s");
}
```
- [ ] **Step 2: Check that it fails before the matcher exists.** No box is needed: on `04044320`, `find_all` does not exist, so the test does not compile. Note this in the commit body instead of spending a box.
- [ ] **Step 3: Run Tasks 1 and 2 in ONE release box.** Add the Task 2 half to `/root/gc-night/jobs/trie-box.sh` after the Task 1 half:
  - `cargo nextest run --locked --release -p goalcup --no-capture -E 'test(find_all_alone_is_far_above_50_mb_s) | test(exact_values_mask_above_a_floor)'`
  - `cargo test --locked --release -p goalcup --test mask_bench -- --ignored --nocapture by_hand_mask_bench`

Run: `node "$GC_TOOLS/box.mjs" run --repo <wt> --ref v1.0/prd-05-masking-exact-trie-matcher --script /root/gc-night/jobs/trie-box.sh --minutes 30 --vcpus 4`
Expected: `cargo fmt` clean, PASS, both halves. The output prints `find_all_alone_is_far_above_50_mb_s: <F> MB/s` with F > 50, `Masker::new with 32 values of 256 bytes: <ms>` under 1000, a peak `VmHWM` under 65536 kB, and the generated Codex request lines with no values (S0) and with 32 values (S32).
- [ ] **Step 4: Record F, S0, S32, R, the `Masker::new` ms and the peak kB** in `/root/gc-night/jobs/trie-t2.numbers`, as numbers only, never a value byte. R does not start Task 4. It is the baseline for Task 4's trigger.
- [ ] **Step 5: Commit**
```bash
git add crates/goalcup/src/mask/values.rs
git commit -m "test(mask): a release-only floor for the trie matcher alone"
```

### Task 3: Write the measured cost into PRD-05 and merge

**Files:**
- Modify: `docs/plans/2026-10-30-v1.0/PRD-05-masking.md:537-546` (section 11 "Measured again") and the amendment `:1206-1248`

**Interfaces:**
- Consumes: the Task 2 numbers and the Task 1 stated change, if there is one
- Produces: the trie matcher merged into `v1.0/integration` through the one train (the FINISH-PLAN Wave 2 exit "trie merged"), so it is in H2, the build the one-hour runs use. The composite 10-minute wave does not wait for it (owner 22:05Z: "before the 2-hour runs").

- [ ] **Step 1: Edit the two PRD-05 passages.** Add F next to the stream rate. Say that the matcher alone is above 50 MB/s, and that the 50 MB/s stream target is held back by the shared rules (14.6 MB/s alone) and by the hold (R). Name the 256-escape-byte value change if Task 1 found it. Name Task 4 as deferred to after 1.0, with its trigger (Wave B). Do not edit PRD-02: the core docs lane owns the cost line there (FINISH-PLAN Wave 1, "Core docs"), and it gets these numbers from `/root/gc-night/jobs/trie-t2.numbers`. Use no model names and no em dashes.
- [ ] **Step 2: Check the docs text without a box:** `grep -n $'\xe2\x80\x94' docs/plans/2026-10-30-v1.0/PRD-05-masking.md`
Expected: no output.
- [ ] **Step 3: Commit and push the branch**
```bash
git add docs/plans/2026-10-30-v1.0/PRD-05-masking.md
git commit -m "docs(prd-05): measured cost of the single-pass matcher"
git push origin v1.0/prd-05-masking-exact-trie-matcher
```
- [ ] **Step 4: Ready for the one train.** Ledger `ready for the train`; no `.req`. The conductor diffs the branch against `v1.0/integration`, reads its note and names it in its ONE train request (mini plan Task 11 Step 3: `branch` is the comma-joined list, so `mergeq.sh` runs `merge.sh` once and the train runs ONE gate (`sh scripts/gate.sh fast`, the floor tests skip themselves in that debug build) on the combined head). On failure the conductor bisects the train by requeueing a comma-joined subset.
Expected: the train's `<id>.res` reads PASS, and `git merge-base --is-ancestor origin/v1.0/prd-05-masking-exact-trie-matcher origin/v1.0/integration` exits 0. The merge is a merge commit.

---

## Wave B: one literal tracker per stream (deferred to after 1.0)

**Trigger:** run Task 4 only if the one-hour composite runs show the recorder falling behind (the masking backlog grows through the hour, or a hold or candidate bound trips `fail_closed`) and R from Task 2 is below 0.8. Otherwise it is a 1.0.1 candidate and nothing here runs.

### Task 4: `LiteralTrack` replaces per-start literal candidates

Fresh branch `v1.0/prd-05-masking-literal-hold` from `v1.0/integration` after Task 3.

**Files:**
- Modify: `crates/goalcup/src/mask/values.rs`. Add `LiteralTrack` and move `LiteralWalk` into `mod tests` as its oracle.
- Modify: `crates/goalcup/src/mask/stream.rs`:
  - `Scanner` gets a field `literals: LiteralTrack`, and `Redacting` gets one too.
  - Remove `Walk::Literals` and the `Form::Literal { form: 0 }` spawn arm. `Extra` entries for the forms stay, for `rule_of` and the priority base.
  - `step_unit`, `follow`, `release` (`cut`), `finish_cut`/`secret_from` and the idle reset in `feed` read the tracker.
  - The bound becomes `self.candidates.len() + self.literals.len() > MAX_CANDIDATES`.
- Test: `values.rs` `mod tests`; `stream.rs` `held_state_stays_bounded` (adds `s.literals.len()` to the bound it checks); `mask_values.rs` and `mask_stream.rs` unchanged

**Interfaces:**
- Consumes: `RunSecrets` (Task 3 head), with `RunSecrets::next(State, u8) -> ([Option<State>; 2], Option<usize>)` kept private to `values.rs`
- Produces:
```rust
/// Every match in progress of the exact-value forms in one stream: (state, start offset), at most
/// one per distinct state, the earliest start kept.
#[derive(Debug, Default, Clone)]
pub(crate) struct LiteralTrack { live: Vec<(State, u64)> }
impl LiteralTrack {
    /// Step every live match over the unit at `off` (`b` is None for a non-ASCII unit: all die),
    /// then start one at `off` when a form starts with `b`. True when a live or completing
    /// match includes this unit (the fail-closed window's "included").
    pub(crate) fn step(&mut self, secrets: &RunSecrets, b: Option<u8>, off: u64) -> bool;
    /// The earliest start of a live match: the hold must keep bytes from there.
    pub(crate) fn hold_from(&self) -> Option<u64>;
    /// The earliest start of a live match that has matched at least one character.
    pub(crate) fn secret_from(&self) -> Option<u64>;
    /// Distinct live starts, counted against MAX_CANDIDATES.
    pub(crate) fn len(&self) -> usize;
    pub(crate) fn clear(&mut self);
}
```

- [ ] **Step 1: Write the failing test** in `values.rs` `mod tests`. It reuses `value`, `text` and `Rng`:
```rust
#[test]
fn literal_track_holds_what_the_walks_held() {
    let mut rng = Rng(0x7AC4_0001);
    for case in 0..80 {
        let values: Vec<SecretValue> = (0..1 + rng.below(8))
            .map(|_| SecretValue::new(ValueSource::Workspace, value(&mut rng)))
            .collect();
        let (secrets, _) = RunSecrets::new(&values);
        let forms: Vec<Vec<u8>> = (0..secrets.len()).map(|i| secrets.form(i).to_vec()).collect();
        let text = text(&mut rng, &forms);
        let mut track = LiteralTrack::default();
        let mut walks: Vec<(u64, LiteralWalk)> = Vec::new();
        for (at, &b) in text.iter().enumerate() {
            let at = at as u64;
            let b = b.is_ascii().then_some(b);
            walks.retain_mut(|(_, w)| b.is_some_and(|b| w.step(&secrets, b) == Advance::Live));
            if let Some(mut w) = b.and_then(|b| secrets.spawn(b)) {
                if w.step(&secrets, b.unwrap_or(0)) == Advance::Live { walks.push((at, w)); }
            }
            track.step(&secrets, b, at);
            assert_eq!(track.hold_from(), walks.iter().map(|w| w.0).min(), "case {case} at {at}");
            assert_eq!(
                track.secret_from(),
                walks.iter().filter(|w| w.1.matched_any()).map(|w| w.0).min(),
                "case {case} at {at}"
            );
        }
    }
}
```
- [ ] **Step 2: Run it to verify it fails**
Run: no box. Commit the test first; the Step 4 job (the task's ONE box) runs on that commit, then on the implementation commit, with `cargo nextest run --locked -p goalcup literal_track_holds_what_the_walks_held`.
Expected on the test commit: FAIL to compile with "cannot find type `LiteralTrack`".
- [ ] **Step 3: Implement `LiteralTrack` in `values.rs`** using the Produces signatures. `step` runs one iteration of `find_all`'s inner loop over `live`, plus a root start at `off`. It keeps `live` sorted by start, with the earliest start per equal `State`. Then wire it into `stream.rs` as listed under Files. In `release`, the cut is `min(candidates' starts, literals.hold_from(), end)`. In `finish_cut`, a discontinuity uses `literals.secret_from()` beside the candidates' `secret_from`. Delete `LiteralWalk` from non-test code, and `Walks` with it.
- [ ] **Step 4: Run the masking suites, then the speed check, in one box**
Run: `node "$GC_TOOLS/box.mjs" run --repo <wt> --ref v1.0/prd-05-masking-literal-hold --script /root/gc-night/jobs/trie-t4.sh --minutes 30 --vcpus 4`, with the job running `cargo fmt --all --check` and `cargo nextest run --locked -p goalcup mask` (every masking unit and integration test, including Task 1's five and `the_trie_finds_what_the_per_form_matchers_found`). Then it runs Task 2's release job.
Expected: PASS. `by_hand_mask_bench` prints S32/S0 >= 0.8 on the generated Codex request. `exact_values_mask_above_a_floor` and `backslash_runs_before_values_never_fail_closed` show no `fail_closed` byte. If R stays below 0.8, keep the change only if it is shorter than the code it removes (less code). Otherwise drop the branch and write the reason into the PRD-05 amendment.
- [ ] **Step 5: Amend PRD-05** section 11 and the amendment with the new S32 and R, and state that what is masked is unchanged. The one difference is that `fail_closed` can fire no earlier than before, because equal literal states from different starts now count once.
- [ ] **Step 6: Commit, review, push, queue**
```bash
git add crates/goalcup/src/mask/values.rs crates/goalcup/src/mask/stream.rs docs/plans/2026-10-30-v1.0/PRD-05-masking.md
git commit -m "perf(mask): one literal tracker per stream instead of a candidate per start"
git push origin v1.0/prd-05-masking-literal-hold
```
Two reviewers read the diff (B2). Then the conductor names it in a train request, as in Task 3 Step 4. Expected: the train's `<id>.res` PASS, merged with a merge commit.

---

## Open decisions (for the conductor, and the owner where marked)

1. **(Owner) The per-stream "far above 50 MB/s" target is not reachable in PRD-05's exact-value scope.** One stream of the shared rules alone measures 14.6 MB/s. This plan holds the matcher alone to more than 50 MB/s (Task 2), and the stream with 32 values to at least 0.8 of the no-value rate (Task 4). The 50 MB/s per stream stays with PRD-05 section 11 items 3 (dispatch) and 4 (PRD-02's pool).
2. **(Owner) "Aho-Corasick" became the lane's own trie with a set simulation, and no crate was added.** A literal multi-pattern matcher cannot take a line break inside a form, and the manifest is PRD-02's. `aho-corasick 1.1.5` is already in `Cargo.lock` (pulled in by `regex`) if the owner wants the crate instead. Confirm the trie.
3. **The floor tests run in every tier, while PRD-05's PRD-14 row says `mask_bench.rs` has "no floor and never fails the gate".** Both floor tests skip themselves in the debug builds that `gate.sh fast` and `full` run, so they never fail the gate. Keep them, or mark them `#[ignore]`.
4. **Task 4 is deferred** to after 1.0 unless the one-hour runs show the recorder falling behind (see Wave B). The one-hour runs wait only for the trie (Task 3, in the one train).

---

## Self-review

1. Spec coverage: owner 22:05Z single-pass matcher checked (Task 1), measured alone and in the stream (Task 2), written into PRD-05 and merged in the one train before the one-hour runs (Task 3); Wave B deferred with its trigger (Task 4).
2. Box and merge shape (owner execution shape and the 1 Nov reviews): one release box at `--vcpus 4` through the installed `box.mjs`, starting with `cargo fmt --all --check`; no gate of its own; no lane request, the conductor's one train request names the branch; the `04044320` oracle checkout works in the box because that commit is an ancestor in the bundle.
3. Reviews: the new diff gets B2's two reviewers (one most capable, one mid-size) and no third pass, since the branch's code already had two.
4. Type consistency: `RunSecrets::new`, `find_all`, `Rng`, `text`, `F`, `S0`, `S32`, `R` match across Tasks 1, 2 and 4.
5. Global Constraints: the quote-cut sentence now matches the passing quoted-form tests.

---

## Execution

Lane script: `docs/plans/2026-10-30-v1.0/conductor/gc-lane.js`. One lane, the existing branch `v1.0/prd-05-masking-exact-trie-matcher`, running in parallel with the other code-wave lanes. Tasks 1 to 3 are sequential inside the lane. The lane gets no box per task and none per fix round: ONE release box runs Tasks 1 and 2 together, at the end of the slice. Reviewers are read-only. Task 3 ends when the branch is in the one train (one request, one gate on the combined head, bisect on failure). Task 4 is not scheduled; it starts only on the Wave B trigger, after 1.0, as its own lane after Task 3 is merged.

| Task | Work | Model |
|---|---|---|
| 1 | Read-only reviewer on the new tests and numbers: correctness and masking | most capable |
| 1 | Read-only reviewer: code quality and less code | mid-size |
| 1 | Write the five tests, the shared box job, apply the batched fixes | mid-size |
| 2 | Write the release-only test, run the shared box, read and record the numbers | mid-size |
| 2 | Keyscan of `trie-t2.numbers` before it is shared | smallest |
| 3 | PRD-05 edits, push, ready for the one train | mid-size |
| 4 (deferred) | `LiteralTrack`, `stream.rs` wiring, its two reviewers | most capable |
| 4 (deferred) | Tests run, PRD-05 amendment | mid-size |

Boxes: 1 release box (2 with its fix round), no gate or build of its own. The trie reaches the one-hour runs through H2, the one train's head the mini-test plan builds once.
