# Harness Mini-Test Wave Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give every runnable harness one 10-minute composite pad row on staging (start, a subagent, planted secrets, upload, fetch and C4 in one row) tonight, all at once on one shared build of the current integration head (B0), as diagnostic rows beside the code wave; fold the fixes into the one train; rerun only the failed rows on its head (H2).

**Architecture:** Two parts. Part A (Tasks 1 to 8) is one PRD-12 rig slice on branch `v1.0/prd-12-mini-rig`: per-row pad overrides, a fixture prompt variant, Kimi's key file kept beside goalcup setup's row, `{home}`/`{workspace}` in provider snippets (OpenClaw workspace), list-form exit keys plus the Codex gateway pin, the Crush permission file, and the Cline route search; Python and data under `scripts/pad/`, one slice box, merged in the one train. Part B (Tasks 9 to 12): the conductor builds `04044320` once (B0, tonight); the canary pair (Claude Code and Codex) runs on B0 from a worktree at `04044320` at once; the other rows run on B0 from the rig worktree as soon as the rig slice box passes, up to 8 row boxes plus their index boxes. Composite rows are diagnostic only (a micro row gives no tier, and rows before long-runs Task 1 carry no `pad_commit`), so they need no merged tree. One fix round, time-boxed to 3 hours, puts its fixes in the ONE train with every code-wave branch (one request, one gate); the train head H2 is built once and only the failed rows rerun on it.

**Tech Stack:** Python 3.11 (`scripts/pad/pad`, `pad-run`, `check.py`, `table`, `unittest`), TOML/JSON pad data, Vercel Sandbox pad boxes through `scripts/pad/sbx.mjs`, test boxes through `node /root/gc-tools/box.mjs run`, the merge queue (`/root/gc-night/mrq`, `mergeq.sh` running `merge.sh` once per request, gate fast).

**Spec:** `/root/gc-night/finish/FINISH-PLAN.md` (section 1 items 5 and 6, Wave 0 "Pad rig" and "Cline search", Wave 1 "Mini-test settings" and "Pad waves", section 5 asks 1, 2, 8, 10, 11), `/root/gc-night/backlog/next-pad-verify.md`, `/root/gc-night/lanes/integrator.log` owner lines, the pad reports `rung1x`, `sweep2`, `rung5`, `gw3` (`/root/gc-night/<name>/REPORT.txt`). Code facts checked on `origin/v1.0/integration` at `04044320`.

## Global Constraints

- Box rules (owner-approved execution shape, 1 Nov, overrides the spec's 2-box rule and stagger): one shared build per head in `/root/gc-night/builds/<commit8>/` (B0 = `04044320` for the wave, H2 = the one train's head for the reruns). Up to 8 pad row boxes at once plus their index boxes (16 at most). No probe boxes. One gate, on the one train. No box per task or fix round; one box per slice at its end. Smallest box size, short timeouts, stop stuck rows early. Reviews and reports never start a box.
- Claude Code tests on the pinned smallest model only (`scripts/pad/harnesses/claude-code-cli.toml:24-30`: `claude-haiku-5-5` direct, `anthropic/claude-haiku-5.5` gateway). Codex gateway pin as its config (`gateway/codex-config.toml`, Task 5; owner 19:17Z rule, conductor note 22:00Z). DeepSeek gateway `deepseek/deepseek-v4.1-flash`. Model ids here are pad data values. Check the pinned model on the first Claude Code row.
- Keys only through `put-env` (0600), never on a command line, never printed. Run the pad with `GH_TOKEN` unset. `DATABASE_URL` reaches only the pad's short-lived index box.
- Mini-test settings (Wave 1): `mode = "micro"`, `prompt = "smoke"` (the fixture variant for opencode, qwen, copilot), `minutes = 10`, `server = "staging"`, `where = "sandbox"`, `steps = ["keep-going"]`, `case = "direct"` except `"gateway"` for Claude Code and Codex. Cap 3 USD per row, `box_extra_minutes` 10, `box_vcpus` 4 in the wave, 2 for the fix-round reruns.
- Canary: "Stop everything if either [canary row] fails." Claude Code and Codex start 2 minutes ahead inside the same wave (S1).
- Deltas-only shape hits with 0 known hits are C3 PASS with the note "stated loss: shape in streamed reply". Hermes C2 passes on task logs with the note "Subagents are shown by their task logs, not tied to individual calls."
- Code is never compiled or tested on the conductor. Pad tests run in a box: `cargo fmt --all --check` then `python3.11 -m unittest discover -s scripts/pad -p 'test_*.py'`.
- Push only `v1.0/*` branches, merge commits only, never force, every merge through the merge queue. No production, `main`, tags or website database writes. Never edit `/home/user/goal-cup-proxy` (use worktrees) or `OWNER-DECISIONS.md`.
- Never commit `pad-out/`, `.env*` or key files. Run `keyscan.py` over any copied folder before reading a pane file or committing conductor state: 0 hits.
- No model names and no em dashes in repo docs (model ids in pad data files are data). Commit trailers: `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` and `Claude-Session: https://claude.ai/code/session_01V2hUmG4Ccnfp953KQMAUS6`.
- Already merged, not tasks here: G1, P0a, P0b, P1, P3 (`pad-run out_paths`), R5-2, R5-3 (tests `test_unrelated_continue_text_is_not_a_lost_terminal`, `test_no_work_is_inconclusive`), D1, D2, D3 (`hermes-cli.toml:7-9` already says gw3 ran the gateway preset), V1.

## Review Focus

1. **A snippet that already holds `${VAR}` or `{env:VAR}`** (OpenClaw gateway, OpenCode, Kilo). Filling `{home}` and `{workspace}` must leave those untouched, or the harness reads a broken key reference and makes no call. Test `test_fill_leaves_env_references_alone` (Task 4).
2. **Kimi's key file where goalcup setup already wrote a row and the pad writes the same key.** Setup's `default_permission_mode` stays, and a key both name appears once with the pad's value. Test `test_key_text_same_key_once_pad_value_wins` (Task 3).
3. **`case = "all"` with per-row overrides.** Every case's run gets the row's `cap_usd` and box values, not only the first. Test `test_main_passes_row_values_to_every_run` (Task 1).
4. **A harness that exits after the first exit item** (Codex closes on `/quit` before the Enter). No key goes to a dead pane and the step does not raise. Test `test_harness_gone_mid_list_gets_no_more_keys` (Task 5).
5. **Eight rows at once plus their index boxes.** `step_fetch` (`pad:685-730`) creates `pad-dbf-*` while the row's own pad box is still alive (P0b keeps it until check), and a row waiting in the pad's per-key queue has no box yet. `wave.sh`'s pool of 8 `row.sh` processes keeps row boxes at 8 and all boxes at 16. Check: `wave.sh` dry run and box list in Task 9 Step 3, S4 during the wave.

---

## File Structure

| File | Responsibility | Tasks |
|---|---|---|
| `scripts/pad/pad` | `KEYS:48-49` and `load_settings:275-326` (accept `cap_usd`, `box_extra_minutes`, `box_vcpus`), new `row_pad`, `main:947-975` (apply it) | 1 |
| `scripts/pad/pad-run` | `setup:252-290`: snippet placeholders (`:273-276`) and the key file merge (`:285-286`); `exit_harness:566-583`: list-form exit keys | 3, 4, 5 |
| `scripts/pad/runs/mini.toml` | Create: the mini-test settings template | 1 |
| `scripts/pad/prompts/smoke-fixture.txt` | Create: the smoke prompt that names the plants as fake and asks for no questions | 2 |
| `scripts/pad/harnesses/codex-cli.toml` | `:15` exit keys, `:26` gateway pin | 5 |
| `scripts/pad/harnesses/direct/openclaw-cli-config.json`, `gateway/openclaw-cli-config.json` | `agents.defaults.workspace` | 4 |
| `scripts/pad/harnesses/crush-cli.toml`, create `scripts/pad/harnesses/direct/crush-cli-crushrc` | Permission answers file; drop `not_verified` (`:16`), comment `:5-9`, `files` `:27`, `:31` | 6 |
| `scripts/pad/harnesses/cline-cli.toml` (+ snippets) | Only if Task 7 finds a route | 7 |
| `scripts/pad/test_pad.py` | New classes `RowOverrides`, `PromptVariant`, `KeyFileMerge`, `SnippetFill`, `ExitKeys`, `CrushPermissions`; `NotVerified:81-98` moved off crush | 1 to 7 |
| `docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md` | `:176-186` rarely used keys, `:507-531` harness file fields | 1, 3, 4, 5 |
| `docs/harnesses/openclaw-cli.md:12`, `docs/harnesses/crush-cli.md:65-67` | One sentence each on the pad route | 4, 6 |
| `/root/gc-night/mini/{row.sh,wave.sh,padtest.sh,runs/*.toml,logs/}` | Conductor run files, outside the repo | 1 (padtest.sh), 9 to 12 |
| `docs/plans/2026-10-30-v1.0/conductor/reports/mini-wave.md` | The wave's results table (scribe commits it) | 12 |

Lane setup (once, before Task 1): `git -C /home/user/goal-cup-proxy fetch origin && git -C /home/user/goal-cup-proxy worktree add -b v1.0/prd-12-mini-rig /root/gc-wt/prd12-mini origin/v1.0/integration`.

Test box script (Task 1 creates it, every rig task uses it), `/root/gc-night/mini/padtest.sh`:
```bash
set -u
cargo fmt --all --check || echo "FMT FAIL"
command -v python3.11 >/dev/null || sudo -n dnf install -y -q python3.11 >/dev/null 2>&1
python3.11 -m unittest discover -s scripts/pad -p 'test_*.py' 2>&1 | tail -60
```
No box per task or per fix round, reviewers read only: the slice gets ONE box at its end (Task 8 Step 1), `node /root/gc-tools/box.mjs run --repo /root/gc-wt/prd12-mini --ref v1.0/prd-12-mini-rig --script /root/gc-night/mini/padtest.sh --minutes 10 --vcpus 2` on the committed slice head. Each Step 2 FAIL is the expected result before Step 3, checked by reading.

---

## Part A: rig slice (PRD-12, one branch, one slice box, the one train)

### Task 1: Per-row pad overrides and the mini settings template

**Files:**
- Modify: `scripts/pad/pad:48-49` (`KEYS`), `scripts/pad/pad:275-326` (`load_settings`), `scripts/pad/pad:947-975` (`main`)
- Create: `scripts/pad/runs/mini.toml`, `/root/gc-night/mini/padtest.sh` (outside the repo)
- Modify: `docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md:176-186`
- Test: `scripts/pad/test_pad.py` (class `RowOverrides`)

**Interfaces:**
- Produces: `OVERRIDES = ("cap_usd", "box_extra_minutes", "box_vcpus")`; `row_pad(s: dict, pad: dict) -> dict` (a copy of `pad` with each override the settings name); settings keys `cap_usd` (number above 0), `box_extra_minutes` (int, 0 or more), `box_vcpus` (one of 1, 2, 4, 8); `cap_usd` is an int or float above 0, never a bool (`True` is an `int` in Python). `Run.spend` (`pad:653`) and `step_setup` (`pad:591-594`) already read these names from `self.pad`, so they need no change.

- [ ] **Step 1: Write the failing tests**
```python
class RowOverrides(unittest.TestCase):
    """Wave 1: a row may set cap_usd, box_extra_minutes and box_vcpus; pad.toml stays the default."""

    def test_row_values_replace_pad_toml(self):
        s = pad.load_settings(dict(BASE, cap_usd=3, box_extra_minutes=10, box_vcpus=2), PADCFG)
        got = pad.row_pad(s, dict(PADCFG, cap_usd=10, box_extra_minutes=30, box_vcpus=4))
        self.assertEqual((got["cap_usd"], got["box_extra_minutes"], got["box_vcpus"]), (3, 10, 2))
        self.assertEqual(got["keys"], PADCFG["keys"])

    def test_absent_keeps_pad_toml(self):
        s = pad.load_settings(dict(BASE), PADCFG)
        self.assertEqual(pad.row_pad(s, dict(PADCFG, cap_usd=10))["cap_usd"], 10)

    def test_bad_values_are_usage(self):
        for k, v in (("cap_usd", 0), ("cap_usd", "3"), ("cap_usd", True), ("box_vcpus", 3), ("box_vcpus", 2.5),
                     ("box_extra_minutes", -1), ("box_extra_minutes", 2.5)):
            with self.assertRaises(pad.Usage, msg=(k, v)):
                pad.load_settings(dict(BASE, **{k: v}), PADCFG)

    def test_main_passes_row_values_to_every_run(self):
        d = self.enterContext(tempfile.TemporaryDirectory())
        f = os.path.join(d, "s.toml")
        with open(f, "w") as fh:
            fh.write('harness = "codex-cli"\nkey = "openai"\nmode = "micro"\ncase = "all"\ncap_usd = 3\nbox_vcpus = 2\n')
        seen = []
        with mock.patch.object(pad, "run_all", lambda s, p, h, o, c: seen.append((p, c)) or [("PASS", "")] * len(c)):
            pad.main(["pad", f])
        p, cases = seen[0]
        self.assertEqual((p["cap_usd"], p["box_vcpus"]), (3, 2))
        self.assertEqual(cases, ["direct", "gateway", "dodger"])

    def test_mini_template_loads(self):
        s = pad.load_settings(pad.read_toml(os.path.join(HERE, "runs", "mini.toml")), pad.read_toml(os.path.join(HERE, "pad.toml")))
        self.assertEqual((s["mode"], s["minutes"], s["server"], s["steps"], s["cap_usd"]), ("micro", 10, "staging", ["keep-going"], 3))
```
- [ ] **Step 2: Run tests to verify they fail**
Run: none (no box per task; the slice's one box is Task 8 Step 1). Expected before Step 3: FAIL, `AttributeError: module 'pad' has no attribute 'row_pad'` and `unknown setting: cap_usd`.
- [ ] **Step 3: Implement**
Add the three names to `KEYS`; validate them in `load_settings` beside the `minutes` check; `row_pad(s, pad) -> dict` returns `dict(pad, **{k: s[k] for k in OVERRIDES if k in s})`; in `main`, after `apply_kill_minutes`, `pad = row_pad(s, pad)` before `run_all`. Create `scripts/pad/runs/mini.toml`:
```toml
# Wave 1 mini-test: 10 minutes of the smoke prompt (one subagent), staging, one row per harness. Copy it, set harness,
# key, case and prompt, add build = "<folder with goalcup and goalcup-verify>", and run scripts/pad/pad <copy> from the repo root.
harness = "codex-cli"
key = "openai"
case = "gateway"
mode = "micro"
prompt = "smoke"
minutes = 10
server = "staging"
where = "sandbox"
steps = ["keep-going"]
cap_usd = 3
box_extra_minutes = 10
box_vcpus = 4
```
In PRD-12 `:176-186` add one sentence: "`cap_usd`, `box_extra_minutes` and `box_vcpus` replace the `pad.toml` value for this row only." Create `/root/gc-night/mini/padtest.sh` (contents in File Structure).
- [ ] **Step 4: Run tests to verify they pass**
Run: in Task 8 Step 1. Expected: `OK` for the whole discover run.
- [ ] **Step 5: Commit**
```bash
git -C /root/gc-wt/prd12-mini add scripts/pad/pad scripts/pad/runs/mini.toml scripts/pad/test_pad.py docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md
git -C /root/gc-wt/prd12-mini commit -m "pad: per-row cap_usd and box settings; mini-test settings template"
```

### Task 2: Fixture prompt variant

**Files:**
- Create: `scripts/pad/prompts/smoke-fixture.txt`
- Test: `scripts/pad/test_pad.py` (class `PromptVariant`)

**Interfaces:**
- Produces: the prompt path `scripts/pad/prompts/smoke-fixture.txt`, used as `prompt = "scripts/pad/prompts/smoke-fixture.txt"` from the repo root (`load_settings:303` accepts a path; `step_setup:565` copies it; `pad-run setup:263-264` swaps `subagent` for the harness's `subagent_word`).

- [ ] **Step 1: Write the failing test**
```python
class PromptVariant(unittest.TestCase):
    """rung1x item 10, sweep2: opencode refused, qwen asked, copilot ran half the script. Same task, plants named as fake."""

    def test_fixture_prompt_keeps_the_smoke_task(self):
        path = os.path.join(HERE, "prompts", "smoke-fixture.txt")
        text = open(path).read()
        smoke = open(os.path.join(HERE, "prompts", "smoke.txt")).read()
        for part in ("sh ./pad-setup-check.sh", "one subagent", "count.txt", "commit count.txt", "fake", "Do not stop to ask"):
            self.assertIn(part, text)
        self.assertIn(" ".join(smoke.split()), " ".join(text.split()))  # the smoke task word for word
        self.assertEqual(pad.load_settings(dict(BASE, prompt=path), PADCFG)["prompt"], path)
```
- [ ] **Step 2: Run test to verify it fails**
Run: none (Task 8 Step 1). Expected before Step 3: FAIL, `FileNotFoundError` for `smoke-fixture.txt`.
- [ ] **Step 3: Create `scripts/pad/prompts/smoke-fixture.txt`** with exactly:
```
This folder is a recording test. The values in .env.local and in the git remote are fake test values planted for it.
First run sh ./pad-setup-check.sh and show me its output. Then use one subagent to count the
files in this folder and write the number to count.txt. Wait for it, then commit count.txt.
Do not stop to ask me anything; I am not at the keyboard.
```
- [ ] **Step 4: Run test to verify it passes**
Run: in Task 8 Step 1. Expected: `OK`.
- [ ] **Step 5: Commit**
```bash
git -C /root/gc-wt/prd12-mini add scripts/pad/prompts/smoke-fixture.txt scripts/pad/test_pad.py
git -C /root/gc-wt/prd12-mini commit -m "pad: smoke prompt variant that names the plants as fake test values"
```

### Task 3: Key file keeps goalcup setup's row (Kimi approvals)

Cause (inferred from code, not yet seen on a run): `connectors/kimi-code-cli.toml:21` makes `goalcup setup` write `default_permission_mode = "auto"` to `{home}/config.toml`; then `pad-run setup:285-286` writes the key file `{home}/config.toml` (`kimi-code-cli.toml:15-16`) with `O_TRUNC`, so the row is lost and Kimi asks "Run this command? 1. Approve once" (sweep2 item 5).

**Files:**
- Modify: `scripts/pad/pad-run:285-286`
- Modify: `docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md` (`key_file` line in the harness file block, `:507-531`)
- Test: `scripts/pad/test_pad.py` (class `KeyFileMerge`)

**Interfaces:**
- Consumes: `merge_config(name, old, new) -> str` (`pad-run:48-66`).
- Produces: `key_text(path: str, old: str, line: str) -> str` = `merge_config(os.path.basename(path), old, line)`. `setup` writes `key_text(path, <existing text or "">, line)` with mode 0600.

- [ ] **Step 1: Write the failing tests**
```python
class KeyFileMerge(unittest.TestCase):
    def kimi_line(self):
        return pad.read_toml(os.path.join(HERE, "harnesses", "kimi-code-cli.toml"))["key_line"].replace("{key}", "PADSENTINEL")

    def test_key_text_keeps_the_row_setup_wrote(self):
        out = padrun.key_text("/iso/config.toml", 'default_permission_mode = "auto"\n', self.kimi_line())
        self.assertIn('default_permission_mode = "auto"', out)
        self.assertIn('api_key = "PADSENTINEL"', out)
        self.assertLess(out.index("default_permission_mode"), out.index("[providers.deepseek]"))

    def test_key_text_same_key_once_pad_value_wins(self):
        out = padrun.key_text("/iso/config.toml", 'default_model = "other"\n', self.kimi_line())
        self.assertEqual(out.count("default_model"), 1)
        self.assertIn('default_model = "ds"', out)

    def test_key_text_with_no_old_file_is_the_line(self):
        self.assertEqual(padrun.key_text("/iso/.env", "", "AI_GATEWAY_API_KEY=PADSENTINEL"), "AI_GATEWAY_API_KEY=PADSENTINEL\n")
        self.assertEqual(json.loads(padrun.key_text("/h/auth.json", "", '{"deepseek": {"type": "api", "key": "K"}}')),
                         {"deepseek": {"type": "api", "key": "K"}})
```
- [ ] **Step 2: Run tests to verify they fail**
Run: none (Task 8 Step 1). Expected before Step 3: FAIL, `AttributeError: module 'padrun' has no attribute 'key_text'`.
- [ ] **Step 3: Implement `key_text(path, old, line) -> str` in `scripts/pad/pad-run`** and use it at `:285-286` (read the existing file when it exists, as the `files` loop at `:274` does). PRD-12 harness block, `key_file` comment: "written 0600, beside any rows `goalcup setup` wrote there (the pad's value wins a same key)".
- [ ] **Step 4: Run tests to verify they pass**
Run: in Task 8 Step 1. Expected: `OK`.
- [ ] **Step 5: Commit**
```bash
git -C /root/gc-wt/prd12-mini add scripts/pad/pad-run scripts/pad/test_pad.py docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md
git -C /root/gc-wt/prd12-mini commit -m "pad: a key file keeps the rows goalcup setup wrote (Kimi approvals)"
```

### Task 4: `{home}` and `{workspace}` in provider snippets (OpenClaw workspace)

Cause: OpenClaw works in `{home}/.openclaw/workspace`, not the project, so the commit lands outside it and C1 to C6 read "did no work" (rung1x item 9a, sweep2). `docs/harnesses/openclaw-cli.md:12` names `agents.defaults.workspace`; the connector cannot set it, pad data can. Today `pad-run setup:273-276` copies snippets verbatim.

**Files:**
- Modify: `scripts/pad/pad-run:273-276`
- Modify: `scripts/pad/harnesses/direct/openclaw-cli-config.json`, `scripts/pad/harnesses/gateway/openclaw-cli-config.json`
- Modify: `docs/harnesses/openclaw-cli.md:12` (one sentence), `docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md` (`files` line in the harness block)
- Test: `scripts/pad/test_pad.py` (class `SnippetFill`)

**Interfaces:**
- Produces: `fill(text: str, home: str, project: str) -> str`: replaces `{home}` and `{workspace}` only. `setup` passes each snippet through `fill(text, home, self.project)` before `merge_config`.

- [ ] **Step 1: Write the failing tests**
```python
class SnippetFill(unittest.TestCase):
    def test_fill_leaves_env_references_alone(self):
        t = '{"w": "{workspace}", "h": "{home}/x", "k": "${AI_GATEWAY_API_KEY}", "e": "{env:DEEPSEEK_API_KEY}"}'
        self.assertEqual(json.loads(padrun.fill(t, "/iso", "/run/project")),
                         {"w": "/run/project", "h": "/iso/x", "k": "${AI_GATEWAY_API_KEY}", "e": "{env:DEEPSEEK_API_KEY}"})

    def test_openclaw_works_in_the_project(self):
        for case in ("direct", "gateway"):
            with open(os.path.join(HERE, "harnesses", case, "openclaw-cli-config.json")) as f:
                d = json.load(f)
            self.assertEqual(d["agents"]["defaults"]["workspace"], "{workspace}", case)
            self.assertIn("model", d["agents"]["defaults"], case)  # the model pin is kept beside it
```
- [ ] **Step 2: Run tests to verify they fail**
Run: none (Task 8 Step 1). Expected before Step 3: FAIL, no attribute `fill`, and `KeyError: 'workspace'`.
- [ ] **Step 3: Implement `fill` and use it in the `files` loop**; add `"workspace": "{workspace}"` to `agents.defaults` in both OpenClaw snippets. `openclaw-cli.md:12` add: "The pad sets `agents.defaults.workspace` to the run's project in its provider snippet; a participant can do the same." PRD-12 `files` comment: "`{home}` and `{workspace}` are filled".
- [ ] **Step 4: Run tests to verify they pass**
Run: in Task 8 Step 1. Expected: `OK`.
- [ ] **Step 5: Commit**
```bash
git -C /root/gc-wt/prd12-mini add scripts/pad/pad-run scripts/pad/harnesses/direct/openclaw-cli-config.json scripts/pad/harnesses/gateway/openclaw-cli-config.json scripts/pad/test_pad.py docs/harnesses/openclaw-cli.md docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md
git -C /root/gc-wt/prd12-mini commit -m "pad: fill {home} and {workspace} in provider snippets; OpenClaw works in the project"
```

### Task 5: List-form exit keys (R5-4) and the Codex gateway pin

Cause: rung5 F6, `codex-cli.toml:15` `'/quit\r'` stays unsubmitted in Codex 0.165; `exit_harness` (`pad-run:566-583`) now sends the Enter 0.3 s apart (`4188abbe`), never shown to work. Separately, `codex-cli.toml:26` pins `openai/gpt-6.1-sol` while `gateway/codex-config.toml` runs `anthropic/claude-haiku-5.5` (owner rule 19:17Z, conductor note 22:00Z, gw3 PASS 20:27Z), so C5 would print "recorded model anthropic/claude-haiku-5.5, pinned openai/gpt-6.1-sol". Exit keys are sent only on kill-watchdog and kill-both rows, so no mini row shows this fix: the first evidence is a Wave 2 kill row.

**Files:**
- Modify: `scripts/pad/pad-run:566-583`
- Modify: `scripts/pad/harnesses/codex-cli.toml:13-15`, `:26`
- Modify: `docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md:518`
- Test: `scripts/pad/test_pad.py` (class `ExitKeys`)

**Interfaces:**
- Produces: harness field `exit_keys` is a string (as today) or a list; list items go one per `send-keys`, `exit_pause` seconds apart (harness field, default 1); an item that decodes to `"\r"` is sent as the tmux key `Enter`, any other as `-l` text; before each item after the first, `dead()` is checked and a dead pane ends the sequence. String form unchanged.

- [ ] **Step 1: Write the failing tests**
```python
class ExitKeys(unittest.TestCase):
    def sent(self, keys, dies_after=None):
        r = object.__new__(padrun.Run)
        r.h, r.tmux, r.fault, r.status = {"exit_keys": keys}, "t", lambda *a, **k: None, lambda: {}
        sent, sleeps = [], []
        r.dead = lambda: dies_after is not None and len(sent) >= dies_after
        def sh(cmd, *a, **k):
            if "send-keys" in cmd:
                sent.append(cmd[-1])
            return subprocess.CompletedProcess(cmd, 0, "", "")
        with mock.patch.object(padrun, "sh", sh), mock.patch.object(padrun.time, "sleep", sleeps.append):
            r.exit_harness()
        return sent, sleeps

    def test_list_items_go_one_by_one_with_a_pause(self):
        sent, sleeps = self.sent(["/quit", "\\r"])
        self.assertEqual(sent, ["/quit", "Enter"])
        self.assertIn(1.0, sleeps)

    def test_string_form_is_unchanged(self):
        self.assertEqual(self.sent("/exit\\r")[0], ["/exit", "Enter"])
        self.assertEqual(self.sent("\\003\\003")[0], ["\x03\x03"])

    def test_harness_gone_mid_list_gets_no_more_keys(self):
        self.assertEqual(self.sent(["/quit", "\\r"], dies_after=1)[0], ["/quit"])

    def test_codex_data(self):
        h = pad.read_toml(os.path.join(HERE, "harnesses", "codex-cli.toml"))
        self.assertEqual(h["exit_keys"], ["/quit", "\\r"])
        cfg = pad.read_toml(os.path.join(HERE, "harnesses", "gateway", "codex-config.toml"))
        self.assertEqual(h["gateway"]["model"], cfg["model"])  # owner rule 19:17Z, conductor note 22:00Z, gw3 PASS 20:27Z
```
- [ ] **Step 2: Run tests to verify they fail**
Run: none (Task 8 Step 1). Expected before Step 3: FAIL on the list test (`TypeError` or wrong keys) and on `test_codex_data`.
- [ ] **Step 3: Implement the list branch in `exit_harness`**; in `codex-cli.toml` set `exit_keys = ['/quit', '\r']`, change the `:13-14` comment to "one item per key: Codex 0.165 leaves `/quit\r` unsubmitted (rung 5 F6)", and `[gateway] model = "anthropic/claude-haiku-5.5"`. PRD-12 `:518`: "printf format, or a list of items sent one by one `exit_pause` seconds apart".
- [ ] **Step 4: Run tests to verify they pass**
Run: in Task 8 Step 1. Expected: `OK`.
- [ ] **Step 5: Commit**
```bash
git -C /root/gc-wt/prd12-mini add scripts/pad/pad-run scripts/pad/harnesses/codex-cli.toml scripts/pad/test_pad.py docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md
git -C /root/gc-wt/prd12-mini commit -m "pad: exit keys as a list, one item at a time; Codex gateway pin follows its config"
```

### Task 6: Crush permission answers file

Cause: Crush stops at "Permission Required ... Allow / Allow for Session / Deny" (rung1x item 7, sweep2). `crush-cli.toml:5-9` asks for a crushrc data file; `docs/harnesses/crush-cli.md:24` gives `~/.config/crush/crushrc`, and `XDG_CONFIG_HOME = {home}/.config` (`connectors/crush-cli.toml:6`). `merge_config` treats a non-JSON file line by line, so the file maps through `files` with no code change.

**Files:**
- Create: `scripts/pad/harnesses/direct/crush-cli-crushrc`
- Modify: `scripts/pad/harnesses/crush-cli.toml:5-9` (comment), `:16` (delete `not_verified`), `:27`, `:31` (`files`)
- Modify: `scripts/pad/test_pad.py:81-98` (`NotVerified` uses `amp-cli`), `docs/harnesses/crush-cli.md:67` (one sentence)
- Test: `scripts/pad/test_pad.py` (class `CrushPermissions`)

**Interfaces:**
- Produces: `files` maps `".config/crush/crushrc" = "direct/crush-cli-crushrc"` in both `[direct]` and `[gateway]`, beside the existing `crush.json` entry.

- [ ] **Step 1: Write the failing tests** (and move `NotVerified` off crush: `test_stated_reason_exits_3_before_any_box` writes `harness = "amp-cli"\nkey = "none"`, and `test_table_shows_the_reason_over_old_rows` asserts `tbl.tier_for("amp-cli", [row]) == ("not verified", "", "no key")` with `row["connector"] = "amp-cli"`)
```python
class CrushPermissions(unittest.TestCase):
    def test_crush_runs_with_its_permission_file(self):
        h = pad.read_toml(os.path.join(HERE, "harnesses", "crush-cli.toml"))
        self.assertNotIn("not_verified", h)
        for case in ("direct", "gateway"):
            self.assertEqual(h[case]["files"][".config/crush/crushrc"], "direct/crush-cli-crushrc", case)
        lines = open(os.path.join(HERE, "harnesses", "direct", "crush-cli-crushrc")).read().splitlines()
        allow = [ln for ln in lines if ln.startswith("permissions allow ")]
        self.assertEqual(len(allow), 1)
        self.assertIn("bash", allow[0].split())
        self.assertEqual(load("padtable2", "table").tier_for("crush-cli", []), ("not verified", "", "not yet run on the pad"))
```
- [ ] **Step 2: Run tests to verify they fail**
Run: none (Task 8 Step 1). Expected before Step 3: FAIL, `not_verified` still in the harness file.
- [ ] **Step 3: Write the data.** `crush-cli-crushrc`:
```
# Pad data (PRD-12 section 6): Crush's permission answers for an unattended pad run, in the crushrc a participant
# keeps at ~/.config/crush/crushrc (Crush v0.99.0 README, "Permissions"). Test data; goalcup never reads it.
permissions allow <tool names>
```
`<tool names>`: every tool the smoke prompt needs (shell, file view and list, write and edit, the subagent tool), spelled as the Crush v0.99.0 README and `internal/agent/tools` name them. Read them from github.com/charmbracelet/crush at tag v0.99.0, read only; must include `bash`. Rewrite the `:5-9` comment to say the crushrc file answers the dialog and the AGENTS.md init screen is still unhandled. `crush-cli.md:67` add: "The pad answers the dialog with a crushrc data file (`scripts/pad/harnesses/direct/crush-cli-crushrc`)."
- [ ] **Step 4: Run tests to verify they pass**
Run: in Task 8 Step 1. Expected: `OK`.
- [ ] **Step 5: Commit**
```bash
git -C /root/gc-wt/prd12-mini add scripts/pad/harnesses/direct/crush-cli-crushrc scripts/pad/harnesses/crush-cli.toml scripts/pad/test_pad.py docs/harnesses/crush-cli.md
git -C /root/gc-wt/prd12-mini commit -m "pad: Crush permission answers as a crushrc data file; Crush runs again"
```

### Task 7: Cline config route search (no box, never run the package)

**Files:**
- Read only: `/root/gc-night/mini/cline-pack/` (new, empty folder)
- Modify only on outcome A: `scripts/pad/harnesses/cline-cli.toml` (`not_verified` line, `[direct]` `files`/`env`), `scripts/pad/harnesses/direct/cline-cli-config.json`
- Test (outcome A): `scripts/pad/test_pad.py` (class `ClineRoute`)

**Interfaces:**
- Produces: outcome A (a route) or outcome B (the reason stays "needs sign-in (no provider config file or env route; the auth command puts the key in argv)").

- [ ] **Step 1: Fetch and unpack, read only.** `mkdir -p /root/gc-night/mini/cline-pack && cd /root/gc-night/mini/cline-pack && npm pack cline@3.0.74 --ignore-scripts && mkdir x && tar -xzf cline-3.0.74.tgz -C x`. Never `npm install`, never run a file from it, never start an interpreter in that folder.
- [ ] **Step 2: Search.** `grep -rhoE 'process\.env\.[A-Z_]*(API_KEY|PROVIDER|BASE_URL|DATA_DIR)[A-Z_]*' x | sort -u` and `grep -rlE 'providers\.json|globalState|secrets\.json|"apiProvider"' x`. A route is either an environment variable the CLI reads at start that selects a provider and its key, or a file under `CLINE_DATA_DIR` the CLI reads for provider plus key, with no key in argv. Write the file names, line numbers and variable names (never a value) into `/root/gc-night/mini/cline-route.txt`.
- [ ] **Step 3 (outcome A only): failing test, data, passing test.** Test `test_cline_has_a_route`: `cline-cli.toml` has no `not_verified`, and the `[direct]` data names the found variable or file. Delete the `not_verified` line and the "Not verified" comment (`cline-cli.toml:5-10`; `not_verified` is line 20), write the route as data. Its box is Task 8 Step 1 (`OK`). Commit: `pad: Cline provider route from its package (<variable or file>)`.
- [ ] **Step 4 (outcome B):** no repo change. Write one ledger line: `HH:MMZ cline search: no config or env route in cline@3.0.74; stays "not verified: needs sign-in"`, citing `cline-route.txt`.

### Task 8: The slice box and the one train

**Files:** none new.

**Interfaces:**
- Consumes: Tasks 1 to 7 committed on `v1.0/prd-12-mini-rig`.
- Produces: a passing slice head that the wave runs from tonight (Task 10 Step 2), then merged by the one train (Task 11 Step 3).

- [ ] **Step 1: The slice box.** `node /root/gc-tools/box.mjs run --repo /root/gc-wt/prd12-mini --ref v1.0/prd-12-mini-rig --script /root/gc-night/mini/padtest.sh --minutes 10 --vcpus 2`. Expected: no `FMT FAIL`, `OK` for the whole discover run (every Step 4 of Tasks 1 to 7). On FAIL: fix by reading, commit, run it once more (the slice's one fix-round box); a second FAIL stops the lane and goes to the conductor.
- [ ] **Step 2: Diff check.** `git -C /root/gc-wt/prd12-mini diff --stat origin/v1.0/integration...HEAD` lists only `scripts/pad/`, `docs/plans/2026-10-30-v1.0/PRD-12-test-pad.md`, `docs/harnesses/openclaw-cli.md`, `docs/harnesses/crush-cli.md`. Expected: nothing under `crates/`, `connectors/`, `Cargo.*`, `server/`. `grep -nP '\x{2014}'` on the changed docs: no output.
- [ ] **Step 3: Review.** One read-only review of `git diff origin/v1.0/integration...HEAD` against Global Constraints and Review Focus. Findings fixed in one batch; a code change reruns Step 1 (counts as its fix-round box).
- [ ] **Step 4: Push, ready for the train.** `git -C /root/gc-wt/prd12-mini push origin v1.0/prd-12-mini-rig`; ledger `HH:MMZ | prd-12-mini-rig | ready for the train | <head 8>`. Write no `.req`: the conductor writes the one train request (Task 11 Step 3).
- [ ] **Step 5: Ledger line after the train.** `HH:MMZ | merge | v1.0/prd-12-mini-rig into v1.0/integration as <M_rig> in the one train`.

---

## Part B: the wave (conductor, pad boxes)

### Task 9: Run worktree, driver scripts and row files

**Files:**
- Create: `/root/gc-night/mini/row.sh`, `/root/gc-night/mini/wave.sh`, `/root/gc-night/mini/runs/<row>.toml`, `/root/gc-night/mini/logs/`

**Interfaces:**
- Consumes: B0 = `04044320` (on integration, has the masking package); the rig slice head (Task 8 Step 1 passed) for every row but the canary pair; the tools already on the conductor tonight (`/root/gc-tools/node_modules`, `/root/gc-night/rung1/proxy-preload.cjs`, `/root/gc-night/gw3/rowinfo.py`; `$KEYSCAN` = the handover worktree's `keyscan.py` once its Task 7 is committed, else `/root/gc-night/gw3/keyscan.py`). The handover plan's `$GC_TOOLS` copies replace them only after the one train merges.
- Produces: `/root/gc-night/builds/04044320/` and `/root/gc-night/trust/selftest-04044320.json` (Step 0; diagnostic, never a trust input), `row.sh <settings> <pad worktree>` (one row, log `logs/<tag>.log` ending `PAD_EXIT=<n>`), `wave.sh WT [--canary <a> <b>] <settings>...`, the row files below. Task 11 Step 4 makes the H2 pair the long-runs plan's Task 6 reads.

- [ ] **Step 0: Build B0 once, tonight (one build box).** Job `/root/gc-night/jobs/build.sh`:
```bash
set -e
command -v python3.11 >/dev/null || sudo -n dnf install -y -q python3.11 >/dev/null 2>&1
cargo build --release --locked -p goalcup -p goalcup-verify
git rev-parse HEAD > target/release/commit
python3.11 scripts/pad/selftest --bin target/release || echo "selftest exit $?"
```
Run: `B=/root/gc-night/builds/04044320; node /root/gc-tools/box.mjs run --repo /home/user/goal-cup-proxy --ref 04044320 --script /root/gc-night/jobs/build.sh --minutes 30 --out target/release/goalcup:$B/goalcup --out target/release/goalcup-verify:$B/goalcup-verify --out target/release/commit:$B/commit --out pad-out/selftest.json:/root/gc-night/trust/selftest-04044320.json` (`04044320` is already in the primary checkout; the box writes and deletes one temporary ref there, never the working tree). Expected: `[box] exit 0`, three files in `$B`; ledger `build` line. The same job and command build H2 (Task 11 Step 4).

- [ ] **Step 1: Worktrees.** Canary: `git -C /home/user/goal-cup-proxy worktree add --detach /root/gc-wt/canary 04044320`. The other rows run from the rig worktree `/root/gc-wt/prd12-mini` at its passing slice head (pad Python and data are read from the worktree, never compiled, so B0's binaries plus the rig's pad tree is a valid diagnostic row). Write `/root/gc-night/mini/runs/` from the row table below.
- [ ] **Step 2: Scripts.** `row.sh`:
```bash
#!/usr/bin/env bash
# One mini-test row: GH_TOKEN and PYTHONPATH unset, proxy preload, logs/<tag>.log. $2 is the pad worktree.
s=$1; tag=$(basename "$s" .toml); cd "$2" || exit 2
export PAD_NODE_MODULES=/root/gc-tools/node_modules NODE_OPTIONS="--use-openssl-ca --no-warnings --require /root/gc-night/rung1/proxy-preload.cjs"
{ echo "== $tag $(date -u +%FT%TZ)"; env -u GH_TOKEN -u PYTHONPATH python3.11 scripts/pad/pad "$s"; echo "PAD_EXIT=$?"; echo "== end $(date -u +%FT%TZ)"; } > /root/gc-night/mini/logs/$tag.log 2>&1
```
(After the one train merges, `row.sh` becomes `GC_PAD_WT=$2 exec bash "$GC_TOOLS/padrun.sh" "$1" /root/gc-night/mini`.)
`wave.sh WT [--canary a b] files...`: with `--canary a b`, start `row.sh a WT` and `row.sh b WT` in the background, wait 120 s, and if either log holds a `setup`, `start` or `wait-calls` line that is not `ok`, or a non-zero `PAD_EXIT=`, touch `/root/gc-night/mini/STOP` and start nothing more. Then feed the other files, in the order given, to a pool of at most 8 `row.sh` processes in all (running rows from an earlier `wave.sh` call count: the pool reads `pgrep -fc 'scripts/pad/pad '`; the next file starts when one ends), checking `STOP` before each start. At the end wait for every row. The pool bounds row boxes at 8 even when a pad process sits in its per-key queue (`queue_slot`, `pad:175-205`: `per_key_deepseek = 6`, `per_key = 2` for the gateway key), since a queued row has no box yet; each row adds its own index box (`pad-dbf-*`) at fetch, so 16 boxes at most.
- [ ] **Step 3: Dry check.** `./wave.sh /root/gc-wt/prd12-mini runs/amp.toml runs/droid.toml` (no box: the pad prints the reason and exits 3). Expected: both logs end `PAD_EXIT=3`, each holds `<id>: not verified: no key`, and `node scripts/pad/sbx.mjs list --prefix pad-` (from `/root/gc-wt/prd12-mini`) shows no box. This is the Amp and Droid row.
- [ ] **Step 4: Commit nothing.** These files live outside the repo.

**Row files.** Each is `scripts/pad/runs/mini.toml` (rig worktree) with these lines changed, plus `build = "/root/gc-night/builds/04044320"` and `box_vcpus = 4`. The two canary files run at `04044320`, which has no `mini.toml` and no override keys, so they carry the `mini.toml` lines minus `cap_usd`, `box_extra_minutes` and `box_vcpus` (the `pad.toml` defaults apply: cap 10, extra 30).

| Row file | harness | key | case | prompt | pinned model (`facts.pinned_model`) |
|---|---|---|---|---|---|
| `claude-code.toml` | `claude-code-cli` | `claude` | `gateway` | `smoke` | `anthropic/claude-haiku-5.5` |
| `codex.toml` | `codex-cli` | `openai` | `gateway` | `smoke` | `anthropic/claude-haiku-5.5` after Task 5; at `04044320` a `recorded model ..., pinned ...` why line is expected, not a stop |
| `hermes.toml` | `hermes-cli` | `deepseek` | `direct` | `smoke` | `deepseek-flash` |
| `gemini.toml` | `gemini-cli` | `gemini` | `direct` | `smoke` | none (no `[direct] model`) |
| `opencode2.toml` | `opencode2-cli` | `deepseek` | `direct` | `smoke` | `deepseek-flash` |
| `opencode.toml` | `opencode-cli` | `deepseek` | `direct` | `scripts/pad/prompts/smoke-fixture.txt` | `deepseek-flash` |
| `kilo.toml` | `kilo-cli` | `deepseek` | `direct` | `smoke` | `deepseek-flash` |
| `qwen.toml` | `qwen-cli` | `deepseek` | `direct` | `scripts/pad/prompts/smoke-fixture.txt` | `deepseek-flash` |
| `goose.toml` | `goose-cli` | `deepseek` | `direct` | `smoke` | `deepseek-flash` |
| `copilot.toml` | `copilot-cli` | `deepseek` | `direct` | `scripts/pad/prompts/smoke-fixture.txt` | `deepseek-flash` |
| `openclaw.toml` | `openclaw-cli` | `deepseek` | `direct` | `smoke` | `deepseek-flash` |
| `kimi-code.toml` | `kimi-code-cli` | `deepseek` | `direct` | `smoke` | `deepseek-flash` |
| `crush.toml` | `crush-cli` | `deepseek` | `direct` | `smoke` | `deepseek-flash` |
| `cline.toml` (Task 7 outcome A only) | `cline-cli` | `deepseek` | `direct` | `smoke` | `deepseek-flash` |
| `amp.toml`, `droid.toml` | `amp-cli`, `droid-cli` | `none` | `direct` | `smoke` | n/a (no box) |

**Composite row.** One 10-minute row proves, together: start (`start ok`), a subagent (C2), the planted secrets (C3), upload (`wait-upload ok ... final: sealed`), fetch (`fetch ok`) and C4 (the verifier's `server_equals_local`, `check.py:387`). C1, C5 and C6 come with it.

**Row procedure** (every row in Tasks 10 and 11):
1. Expected table (`logs/<tag>.log`), step names fixed by `step_list` (`pad:444-455`) for micro plus staging plus `keep-going`:
```
pad <harness> <case> micro   variant -   run r<16 hex>  entry e<17 hex>  box pad-<6 hex>   ~13m
setup             ok      0:00   <harness> <version> (pinned), project and plants, goalcup setup, key route <env|file|login>
start             ok      0:01   approved, prompt pasted
wait-calls 2      ok      0:01   <n> model calls written
look              ok      0:01   calls <n>, threads <n>, gaps 0, ...
keep-going        ok|skip        typed keep going after 1 min idle | agent still working | harness exited
seal              ok      0:11   sealed at 10 min (or at the harness's exit), <n> model requests written
wait-upload       ok      0:11   final: sealed
fetch             ok      0:12   fetched e<17 hex>
check             PASS    0:12
cleanup           ok      0:13   nothing key-like left, box removed, list shows no live box
result            PASS           pad-out/<harness>/<case>/<stamp>-pad-<6 hex>/
PAD_EXIT=0
```
2. Evidence, in order: `python3 $KEYSCAN <pad worktree>/pad-out/<harness>` prints `0 hit(s)` and no unset name it checks (else S2); then `python3 /root/gc-night/gw3/rowinfo.py <run folder>` (`$GC_TOOLS/rowinfo.py` after the train) for checks, why, models, spend, subagents, hosts; `report.txt` for `facts.pinned_model` and any `recorded model ..., pinned ...` why line; `pad-out/spend.jsonl` lines for this run (`run` field) for `usd` and `box_minutes`.
3. Record: one line in `/root/gc-night/mini/results.md` (columns of Task 12) and one ledger line `HH:MMZ pad mini <harness> <case> on <B0 or H2> (diagnostic): <result> C1..C6 <values>, <usd> USD est, <box_minutes> box min, <run folder>`.
4. tiers cell: a micro row never gives a tier (`table:tier_row` takes `mode == "full"` only), so `connectors/tiers.toml` from `python3.11 scripts/pad/table` (Task 12) reads `tier = "not verified"`, `version = ""`, `reason = "no full direct row"` for every harness that ran; a harness file with `not_verified` keeps that reason; Amp and Droid read `reason = "no key"`. The mini verdict goes to `results.md` and decides which Wave 2 full rows run.

**Stop rules** (the conductor checks every 2 minutes during the wave and after every row; to stop, `touch /root/gc-night/mini/STOP`, and for a running row `kill -INT <pad pid>`: the pad's STOP handler still runs cleanup):
- S1 canary: before minute 2, `wave.sh` itself starts nothing more on a canary failure. Later, either canary row not `PAD_EXIT=0` (result PASS), or a pinned-model mismatch on the Claude Code row: STOP and `kill -INT` every running row ("stop everything"); go to Task 11 with the records made so far.
- S2 secrets: keyscan finds a real key fragment, or `facts.secret_hits` names a known hit whose sha12 equals a real key's: STOP and kill every row, keep the folder unread, scrub (`python3 $KEYSCAN --scrub <folder>`, or `/root/gc-night/full4/scrub2.py` before the handover `keyscan.py` exists) before anything else, tell the owner (point 4).
- S3 rig: two rows with `setup FAIL`, a box not created, or a cleanup line starting `left:`: STOP starting rows, remove any named box (`node scripts/pad/sbx.mjs rm <name>`); the rig fix goes in Task 11.
- S4 boxes: `sbx.mjs list --prefix pad-` shows more than 8 row boxes (`pad-<6 hex>`) or more than 16 in all: STOP starting rows until back in bounds.
- S5 spend: the wave's estimates (sum of `usd`, or `usd_live` when `usd` is null) pass 5 USD, or wave plus fix round pass 8 USD: stop and ask the owner.
- S6 stuck: a row whose pane and request count have not moved for 4 minutes after its `keep-going` line (`status.jsonl` `s4_requests`, newest `watch/pane-w*.txt` time): `kill -INT` it; it records INCONCLUSIVE with the screen named in `results.md`.
- S7 repeats: only in Task 11, each failed row once; never a repeat for a cause owned by an open item (owner asks 2 and 10, OW-10, V2, N1, accounts) that the fix round did not fix.
- S8 diagnostic: no wave row is a release row or a tier row; nothing in Tasks 10 and 11 waits for trust.

### Task 10: The wave on B0 tonight (canary first, then every runnable harness at once)

**Interfaces:** Consumes Task 9 and B0. Produces one diagnostic record per row in `results.md`, and the fix list for Task 11.

- [ ] **Step 1: Canary now, from the integration worktree.** As soon as B0 is built (no rig fix needed): `cd /root/gc-night/mini && ./wave.sh /root/gc-wt/canary --canary runs/claude-code.toml runs/codex.toml`. It answers first whether the masking package starts, masks the plants, keeps the pin, uploads and passes C4 together; a failure goes straight to the fix list (Task 11), not to Monday.
- [ ] **Step 2: The other rows, from the rig worktree, beside the code wave.** As soon as the rig slice box passes (Task 8 Step 1) and the canary passed: `./wave.sh /root/gc-wt/prd12-mini runs/hermes.toml runs/gemini.toml runs/opencode2.toml runs/opencode.toml runs/kilo.toml runs/qwen.toml runs/goose.toml runs/copilot.toml runs/openclaw.toml runs/kimi-code.toml runs/crush.toml` (append `runs/cline.toml` on Task 7 outcome A). Expected: 8 rows live at once, the first done about minute 15, the last about minute 30; about 3.5 USD est with the canary.
- [ ] **Step 3: Watch.** Every 2 minutes: S1, S4, S5, S6.
- [ ] **Step 4: Judge each row** by the Row procedure and this table (C4 PASS and `PAD_EXIT=0` expected everywhere unless named):

| Row | Expected | Known non-PASS that is not a stop (record it with its owner) |
|---|---|---|
| Claude Code | PASS; `facts.pinned_model` and `facts.models` only `anthropic/claude-haiku-5.5`, no `recorded model` line, every model call to `ai-gateway.vercel.sh`; `facts.masks` holds the planted kinds | none |
| Codex | PASS; models `anthropic/claude-haiku-5.5`, no `api.openai.com` | on B0 (`04044320`): a `recorded model ..., pinned openai/gpt-6.1-sol` why line (Task 5 not in B0) |
| Hermes | PASS; C2 with `Subagents are shown by their task logs, not tied to individual calls.` (`crates/goalcup-verify/src/legs.rs:446`) in `verify/check-local.json`; C3 0 known hits (first proof of the `state.db` drop); `facts.masked_upstream` false (the plants are masked by exact value); ready text `Type your message or /help` | `masked_upstream` set: owner ask 4 (masked-upstream C3; STATE.md ask 6) decides the release row |
| Gemini | PASS; C2 from the parent reader (`crates/goalcup-verify/src/usage/native.rs:235`) | subagent at "Allow execution of [Shell]?": C2 INCONCLUSIVE, owner ask 10 |
| opencode2 | PASS | none |
| opencode | PASS; the pane shows `pad-setup-check.sh` ran | a refusal: C3 INCONCLUSIVE. W2 note: "session files 0" while `harness-files.json` lists `opencode.db` |
| Kilo | PASS | C3 FAIL from 4 raw PEM body lines: owner ask 2 |
| Qwen | PASS; no AskUserQuestion stall | `url_password` split across deltas: stated-loss PASS |
| Goose | C1, C5, C6 PASS | C3 FAIL, four `PEM_LINE` hits: owner ask 2 |
| Copilot | C1 PASS; C3 PASS when the model runs the whole script | C3 INCONCLUSIVE if it masks the values itself (no `redacts_kinds`) |
| OpenClaw | `meta.json` `commits_since_start` 1 or more (Task 4) | C2 INCONCLUSIVE (OW-10); C1 `history_complete` orphans (V2); faults `source_changed` (N1) |
| Kimi Code | no "Run this command?" screen (Task 3); ready text `Trust and continue` then Enter | `api.kimi.com` startup requests: W2, not a check |
| Crush | no "Permission Required" (Task 6) | AGENTS.md init screen holds the prompt: S6, screen recorded |
| Cline | a model call to `api.deepseek.com`, no "Sign in with Cline" | none |
| Amp, Droid | Task 9 Step 3: `not verified: no key`, "need accounts" | n/a |

### Task 11: The fix round (3 hours, one batch, in the one train, failed rows only)

**Interfaces:** Consumes the non-PASS rows of `results.md`. Produces the one train, H2 and one rerun record per failed row.

- [ ] **Step 1: Fix list (time-boxed).** Every non-PASS row whose cause is not an open owned item (S7 list). Known flaky screens go straight to the reason column, not the fix list, unless the fix is data only: Gemini Shell prompt, Crush AGENTS.md init screen, Kimi approvals, OpenClaw `source_changed`. Ledger: `HH:MMZ pad mini fix list: <row: cause, owner file:line>...`. The round has a hard stop 3 hours after the list: a cause not fixed in one batch by then ships as "not verified: <screen or cause>" (long-runs open decision 3).
- [ ] **Step 2: Fix in one batch.** Pad code and data on `v1.0/prd-12-mini-fix` (a worktree from `origin/v1.0/integration`, Task 8's commit and trailer rules); a goalcup cause in its PRD lane. No box per fix; one read-only review per branch; each branch gets its one end box (pad: Task 8 Step 1 with this branch, `cargo fmt --all --check` included). Ledger `ready for the train` per branch.
- [ ] **Step 3: The one train (conductor).** Preconditions: every code-wave branch is `ready for the train` or dropped from it by name in the ledger, and `v1.0/prd-12-release-rows` (long-runs Task 1, row identity) is in the list: H2 is not cut without it. The conductor writes ONE request, via a `.tmp` name and `mv`, to `/root/gc-night/mrq/$(date -u +%H%M%S)-train.req`:
```json
{"branch": "v1.0/prd-05-masking-exact-trie-matcher,v1.0/prd-12-mini-rig,v1.0/prd-12-release-rows,v1.0/conductor-longsum,v1.0/prd-10-participant-text,v1.0/conductor-handover,v1.0/prd-12-mini-fix", "message": "one train: trie matcher, mini rig, release rows, longsum, participant text, handover, fix round", "refresh_lock": false}
```
(only the branches that exist; lanes write no `.req`). `mergeq.sh` runs `merge.sh` once for it, which merges every listed branch and runs ONE gate (`gate.sh fast`) on the combined head. On FAIL, bisect: requeue a comma-joined subset (one request per attempt). Expected: `.res` `"exit": 0`. The head is H2.
- [ ] **Step 4: One build.** `git -C /home/user/goal-cup-proxy fetch -q origin` first (the train head is new there), then Task 9 Step 0's job and command with `--ref <H2>`, into `/root/gc-night/builds/<H2 8>/` plus `/root/gc-night/trust/selftest-<H2 8>.json`.
- [ ] **Step 5: Rerun the failed rows only.** `git -C /home/user/goal-cup-proxy worktree add --detach /root/gc-wt/mini <H2>`; in each failed row file set `build = "/root/gc-night/builds/<H2 8>"` and `box_vcpus = 2` (back to 4 if a setup or start times out at 2). `./wave.sh /root/gc-wt/mini [--canary <Claude Code and Codex files, when failed>] <failed row files>`. Row procedure for each; `results.md` marks the line `r2`. These are the first rows with `pad_commit`; the first one is the long-runs Task 6 trust read. A rerun non-PASS is recorded with its owner and Wave 2 action; there is no second round.

### Task 12: Results table, tiers cell and cleanup

**Files:**
- Create: `docs/plans/2026-10-30-v1.0/conductor/reports/mini-wave.md` (via the scribe; copy of `/root/gc-night/mini/results.md`)

**Interfaces:** Consumes every row's record (Row procedure step 3). Produces the Wave 1 exit item "a short results table in `conductor/reports/mini-wave.md`" and the Wave 2 fix list.

- [ ] **Step 1: Table.** `results.md` columns: harness, version, case, model recorded, build (B0 or H2), box, run, entry, wall, start, subagent, upload, fetch, C1, C2, C3, C4, C5, C6, result, USD est, box min, first non-PASS why, owner (PRD and file:line), Wave 2 action (full row, fix first, owner ask n). One line per row and rerun, plus Amp and Droid with "not verified: needs an account".
- [ ] **Step 2: tiers cell.** In `/root/gc-wt/mini`: `python3.11 scripts/pad/table` (the pad's conductor half, as `pad` with `where = "sandbox"` is; not a test). Expected: `table: <n> of <n> rows read; wrote connectors/tiers.toml, docs/plans/2026-10-30-v1.0/pad-rows.jsonl, PAD-RESULTS.md`; every run harness at `reason = "no full direct row"`, Amp and Droid `"no key"`, Cline either its reason or `"no full direct row"`. Copy the cells into `results.md`. Commit none of the three files, then `git -C /root/gc-wt/mini checkout -- connectors/tiers.toml docs/`.
- [ ] **Step 3: Cleanup (N9).** `node scripts/pad/sbx.mjs list --prefix pad-` shows no live box; `ls /root/gc-wt/{canary,prd12-mini,mini}/pad-out/queue/*/*.slot` is empty; keyscan over those three `pad-out` folders and `/root/gc-night/mini` prints `0 hit(s)`; delete `/root/gc-night/mini/cline-pack`.
- [ ] **Step 4: Report and record.** Scribe copies `results.md` to `conductor/reports/mini-wave.md` (keyscan first) and commits it with the trailers. Ledger: `HH:MMZ pad mini wave done on B0/<H2>: <n> PASS, <n> FAIL, <n> INC, <n> not verified; <usd> USD est, <box min> box min, <boxes> boxes. Wave 2 list: <items>`. Owner note: ten lines at most (what ran, what passed, cost, open asks 2, 10, 11).

---

## Budget

| Part | Boxes | Builds | Gates | Rows | Model USD (likely / cap) |
|---|---|---|---|---|---|
| A | 1 slice box (2 with its fix round) | 0 | share of the one train | 0 | 0 |
| B0 build (Task 9 Step 0) | 1 | 1 | 0 | 0 | 0 |
| Wave on B0 (Task 10) | 26 (13 rows, each a pad box and an index box; none for Amp, Droid) | 0 | 0 | 13 (14 with Cline) | about 3.5 / 5 |
| Fix round (Task 11) | 1 or 2 end boxes, 1 build, about 5 failed rows at 2 vCPU (10 boxes) | 1 (H2) | the one train | about 5 | about 2 / 3 |
| Total | about 40 | 2 | share of 1 | about 18 | about 5.5 / 8 |

## Self-review

1. Spec coverage: rig fixes of FINISH-PLAN Wave 0 (Codex exit key: Task 5; OpenClaw workspace: Task 4; crushrc: Task 6; prompt variant: Task 2; D3 already done; optional per-row cap: Task 1); Kimi approvals: Task 3; Cline search: Task 7; Codex gateway pin: Task 5; the owner's execution shape and the two 1 Nov reviews: one slice box with `cargo fmt --all --check` (Task 8), B0 built tonight and the canary on it at once (Tasks 9, 10), the other rows from the rig worktree beside the code wave as diagnostic rows, up to 8 row boxes plus index boxes, one 3-hour fix round whose branches ride the ONE train written as one request (Task 11 Step 3), one build H2, failed rows only; results table and Wave 2 list: Task 12; Amp and Droid: Task 9 Step 3.
2. Step scan: code steps name function, file and values; run steps give command, expected evidence and stop rules; bodies only for the prompt text, the crushrc header, the shared build job (Task 9 Step 0, reused for H2), `row.sh` and the train request.
3. Type consistency: `row_pad(s, pad)`, `key_text(path, old, line)`, `fill(text, home, project)`, `exit_keys` string or list with `exit_pause`, `wave.sh WT [--canary a b] files...`, `row.sh settings WT`, B0, H2, `M_rig` match across tasks and the long-runs plan.
4. Review Focus: five items, each with its test or check in the owning task (Tasks 1, 3, 4, 5, 9).
5. Proportion: code bodies omitted; Tasks 10 and 11 share one row procedure, one expectation table and one stop rule list.

## Open decisions

- Owner ask 2 (headerless PEM) decides whether Goose and Kilo C3 FAILs are goalcup misses.
- Owner ask 4 (a masked-upstream C3 pass for the release check; STATE.md ask 6) matters only if the Hermes row still sets `facts.masked_upstream`.
- Owner ask 10 (Gemini Shell prompt) and 11 (mini-full prompt) are not needed to run this wave.
- S1 after minute 2 kills the running rows ("stop everything"); the owner may prefer to let them finish for evidence.
- Crush tool names for `permissions allow` come from the v0.99.0 source; the JSON key `permissions.allowed_tools` in `crush.json` is the fallback if the crushrc line is not read.

## Execution

Lane script: `docs/plans/2026-10-30-v1.0/conductor/gc-lane.js`, one lane per independent task group. Tasks 1 to 6 share `pad-run` and `test_pad.py`, so they are one lane.

| Lane / task | Tasks | Starts | Model |
|---|---|---|---|
| B0 build | 9 Step 0 (one build box) | now | mid-size |
| canary | 9 Steps 1 to 3 (canary files), 10 Step 1 | B0 built | mid-size |
| rig | 1, 2, 3, 4, 5, 6 in order, on `v1.0/prd-12-mini-rig` | now, in parallel with the other code-wave lanes | mid-size |
| cline search | 7 Steps 1, 2 (read only, no box); Step 3 on outcome A in the rig worktree after Task 6 | now, parallel | mid-size |
| slice box, diff check | 8 Steps 1, 2 | rig and cline done | mid-size |
| wave: run and read rows | 10 Steps 2 to 4, Row procedure steps 1 to 4 | slice box passed, canary passed | mid-size |
| rig slice review | 8 Step 3, whole branch, read only (beside the wave) | after the slice box | most capable |
| ready | 8 Steps 4, 5 | after a clean review | mid-size |
| fix triage | 11 Step 1 (which non-PASS rows are fixable in 3 hours) | after Task 10 | most capable |
| fixes | 11 Step 2: recorder, masker or proxy causes | after Step 1 | most capable |
| fixes | 11 Step 2: pad code and data, connector data | after Step 1 | mid-size |
| one train, H2, reruns | 11 Steps 3 to 5 | every branch ready | mid-size |
| results | 12 Steps 1, 2, 4 | after Task 11 | mid-size |
| mechanical | `padtest.sh` and `row.sh` copies, every keyscan and scrub, 12 Step 3, the `results.md` copy | with its task | smallest |

Order: B0 build and the code lanes start at once; the canary runs the moment B0 is built; the other rows run from the rig worktree as soon as its slice box passes, beside the code wave; one 3-hour fix round; ONE train request for every branch (one gate); one build H2; failed rows rerun on H2. The long-runs plan's one-hour runs and core rows start on H2.
