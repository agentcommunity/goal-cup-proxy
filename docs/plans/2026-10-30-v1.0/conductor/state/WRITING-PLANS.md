# Writing Plans (owner's required format)

Write implementation plans for an engineer who has not seen this codebase or this spec. Assume they write idiomatic code in the project's language once they know the exact interface and the exact test, and that they will make a reasonable choice wherever the plan leaves one open. What they cannot know is what you decided: which files, which names and signatures, which values from the spec, which tests prove each task. Document those. Give them the whole plan as bite-sized tasks. DRY. YAGNI. TDD. Frequent commits.

Save plans to: `docs/superpowers/plans/YYYY-MM-DD-<feature-name>.md`

## Scope check
One plan per independent subsystem. Each plan produces working, testable software on its own.

## File structure
Before defining tasks, map out which files will be created or modified and what each one is responsible for. Clear boundaries, one responsibility per file, files that change together live together, follow established patterns in this codebase.

## Task right-sizing
A task is the smallest unit that carries its own test cycle and is worth a fresh reviewer's gate. Fold setup, configuration, scaffolding and documentation into the task whose deliverable needs them; split only where a reviewer could reject one task while approving its neighbour. Each task ends with an independently testable deliverable.

## Step granularity
Each step is one action with a checkable result: write the failing test; run it to make sure it fails; implement the minimal code; run the tests and make sure they pass; commit.

## Plan document header (every plan MUST start with this)

```
# [Feature Name] Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** [One sentence describing what this builds]

**Architecture:** [2-3 sentences about approach]

**Tech Stack:** [Key technologies/libraries]

**Spec:** [path to the spec/design doc this plan implements]

## Global Constraints

[The spec's project-wide requirements, one line each, exact values copied verbatim from the spec. Every task implicitly includes this section.]

## Review Focus

[The five input classes or failure modes the spec implies but no task's tests exercise that are most likely to bite a person using this software, one line each, naming the input or condition and the behaviour a reasonable person would expect, most likely first. For each line, add the test that pins it to the task that owns the code.]

---
```

## Task structure

````
### Task N: [Component Name]

**Files:**
- Create: `exact/path/to/file`
- Modify: `exact/path/to/existing:123-145`
- Test: `tests/exact/path/to/test`

**Interfaces:**
- Consumes: [what this task uses from earlier tasks, exact signatures]
- Produces: [what later tasks rely on, exact names, parameter and return types]

- [ ] **Step 1: Write the failing test**
```lang
test code with the spec's exact values
```
- [ ] **Step 2: Run test to verify it fails**
Run: `exact command`
Expected: FAIL with "..."
- [ ] **Step 3: Implement `name(params) -> Return` in `exact/path`**
One line on the approach when the signature and the test leave a choice; a code block only for an algorithm they do not determine.
- [ ] **Step 4: Run test to verify it passes**
Run: `exact command`
Expected: PASS
- [ ] **Step 5: Commit**
```bash
git add ...
git commit -m "..."
```
````

## What a step contains
A step is done when the implementer can write exactly one reasonable thing from it: unambiguous, not complete.
- Test step: the test's name and its assertions, as code, with the spec's exact values.
- Code step: the exact signature, the file, the values the spec pins. The implementer writes the body. A body only for an algorithm the signature and tests do not determine, or exact copy the spec fixes.
- Verification step: the command and the output that means it passed.
- Reference to another task: that task's Interfaces block; do not repeat its code.
A plan longer than the code it describes has written the code instead. Lines that decide nothing ("TBD", "handle edge cases", "add appropriate validation", "write tests for the above", a type or function no task defines) are the opposite failure.

## Self-review (run it yourself, fix inline)
1. Spec coverage: every requirement maps to a task; add tasks for gaps.
2. Step scan: every step lets the implementer write exactly one reasonable thing; no transcripts of bodies the signature already determines.
3. Type consistency: names and signatures match across tasks.
4. Review Focus: each implied input class or failure mode has a test; the five most likely uncovered ones are listed with their tests added to the owning task.
5. Proportion: a plan several times longer than its spec is a transcript; replace bodies with signatures, test names and assertions.

## Project notes for goalcup plan writers
- In this project, code is never compiled or tested on the conductor machine. Every "Run" step names the command AND where it runs: in a Vercel Sandbox build box via `node docs/plans/2026-10-30-v1.0/conductor/box.mjs run ...` (or the existing gate via the merge queue), or a pad box for pad rows. Rust test commands use `cargo nextest run --locked -p <crate> <filter>`; pad tests use `python3.11 -m unittest discover -s scripts/pad -p test_*.py`.
- Owner rules (Global Constraints sources): VISION.md seven points, AGENTS.md, docs/plans/2026-10-30-v1.0/OWNER-DECISIONS.md, BUILD-PROMPT.md, plus the owner answers in /root/gc-night/lanes/integrator.log (Claude Code tests on Haiku only; deliberate Vercel Sandbox use: one shared build per commit, max 2 pad boxes at once, no probe boxes; exact-value masking sources 1 to 3 only, held in memory, never written; deltas-only shape hits are PASS with a stated-loss note; Hermes subagents pass on task logs with a plain note; no key on any command line; push only v1.0/* branches, merge commits only, never force; no production, main, tags or website database writes; no model names, no em dashes in docs; commit trailers Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com> and Claude-Session: https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6).
- The finishing inventory and wave plan this work comes from: /root/gc-night/finish/FINISH-PLAN.md (its "Spec" for these plans), with /root/gc-night/status/STATUS-REPORT.md and /root/gc-night/reviews/LATEST.md.

## OWNER-APPROVED EXECUTION SHAPE (2026-11-01, overrides FINISH-PLAN.md where they differ)
Box use must be smart, not small: concurrency can be 4 to 8 boxes, but the number of builds, gates and rows must fall.
1. Code first: all remaining code lanes run in parallel; lanes get NO box per fix round; reviewers are read-only; each lane slice gets ONE box at its end for its own final check.
2. Merge trains: lane branches queue and are merged together with ONE gate on the combined head; on failure bisect the train.
3. One shared build per wave head, used by every row (builds/<commit>/).
4. Composite rows: every harness gets one 10-minute row that proves start, a subagent, planted secrets (C3), upload, fetch and C4 server_equals_local together; all harnesses in parallel (up to 8 boxes), the whole wave about 15 minutes.
5. One fix round: all wave findings fixed in one batch, one gate, one build, rerun only the failed rows.
6. One-hour composite runs (not two hours) per main harness (Claude Code on Haiku, Codex, Hermes on DeepSeek): recorder kills at about minutes 10 and 30, a goalcup recover at about 45, 15 or more subagents, direct or gateway; one run proves all of these.
7. Then release (owner steps), then big tests on the released version.
Box budget target for the whole finish: about 5 builds, 5 gates, 30 rows, 50 to 80 boxes in total. Tests never run on the conductor machine.

## EXECUTION AS WORKFLOWS, MODEL PER TASK (owner, 2026-11-01)
Every plan ends with an "Execution" section: which tasks run in parallel as Workflow lanes (the lane script docs/plans/2026-10-30-v1.0/conductor/gc-lane.js, one lane per independent task group), which wait on which, and the model for each task class:
- default (most capable) model: hard design and core code (recorder, masker, proxy), the adversarial breaker, the final whole-branch reviewer, the three-hourly reviewer.
- sonnet: implementing well-specified tasks from the plan, rig and connector data, docs and reports, running pad rows and reading their results, merge-train assembly.
- haiku: mechanical work: queue ticks, keyscans and scrubs, log summaries, copying files.
Name the model next to each task in the Execution section.
