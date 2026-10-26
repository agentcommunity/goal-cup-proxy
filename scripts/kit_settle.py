#!/usr/bin/env python3
"""What the test kit settles in a goalcup configuration directory, for a Docker cell (called by
scripts/linux-matrix.sh inside the container) and a host cell (called by scripts/campaign.py) alike.

    kit_settle.py settle CFG WORK
    kit_settle.py home-config CFG CONNECTOR [EFFORT]
    kit_settle.py model-check CFG CONNECTOR MARKER [EFFORT]

settle       prep.json gets `work_root` (kept when set, else WORK); its other keys are kept. 0600,
             written atomically. goalcup keeps no GitHub key (D69): nothing writes github.json. A
             test run's repository is named at `goalcup start --repo` (scripts/matrix.sh); its
             git login is the machine's own.
home-config  the owner's cheapest-model rules and the kit's no-permission-prompt setting, in the
             connector's tracker home (never ~/.codex or ~/.claude): codex-cli's config.toml gets
             `model = "gpt-5.6-luna"`, `model_reasoning_effort` = EFFORT (low, the default, or
             minimal: only when the installed codex lists minimal for that model),
             `approval_policy = "never"` and `sandbox_mode = "danger-full-access"` (the container
             or the owner's test machine is the sandbox); claude-code-cli's settings.json gets
             `"model": "haiku"`, `"alwaysThinkingEnabled": false`,
             `permissions.defaultMode = "bypassPermissions"` (other permission keys kept) and
             `"skipDangerousModePermissionPrompt": true` (the one-time acceptance of the bypass
             disclaimer, which Claude Code 2.1.283 and 2.1.299 read from the user settings; without
             it the mode falls back to default), and its .claude.json
             `"fullscreenUpsellSeenCount": 3` (what answering "Not now" to "Try the new fullscreen
             renderer?" stores; the renderer is not changed) and `"hasSeenAutoDefaultNudge": true`
             (what answering "No, keep bypass permissions" to "Make auto mode your default
             permission mode?" stores; the mode is not changed). amp-cli's run home gets
             `"agentMode": "low"` and `"neoWelcomeDismissed": true` in
             .local/share/amp/session.json (its login files are never read). Other keys kept.
model-check  after the run: every Codex turn (rollouts' `turn_context`) of the sessions written
             since MARKER must be gpt-5.6-luna at EFFORT; every Claude assistant message of the
             transcripts under projects/ (subagents too) must name a haiku model (`<synthetic>`
             is no model call, counted apart). Finding none fails too. Exit 1 on a failure.

Exit 0 when done, 1 when refused (one line saying why), 2 on a bad command line.
"""

from collections import Counter
import json
import os
import re
import subprocess
import sys
import tempfile

LUNA = "gpt-5.6-luna"
CLAUDE_MODEL = "haiku"
# Claude Code (2.1.299) shows "Try the new fullscreen renderer?" until this count reaches 3;
# "Not now" sets it to 3 and changes nothing else.
CLAUDE_UPSELL_KEY, CLAUDE_UPSELL_SEEN = "fullscreenUpsellSeenCount", 3
# Claude Code (2.1.283, 2.1.299) offers "Make auto mode your default permission mode?" when the
# user settings name another defaultMode, until this key is true; "No, keep bypass permissions"
# sets it to true and changes nothing else (fix plan v7, S1).
CLAUDE_AUTO_NUDGE_SEEN = "hasSeenAutoDefaultNudge"
# A test cell runs without permission prompts (nobody may answer one): a test-kit setting, like
# the cheapest model. Claude Code (2.1.283, 2.1.299) accepts default, acceptEdits,
# bypassPermissions, plan, dontAsk and auto as permissions.defaultMode; bypassPermissions is
# downgraded to default unless the user settings hold the disclaimer's acceptance.
CLAUDE_MODE = "bypassPermissions"
CLAUDE_BYPASS_ACCEPTED = "skipDangerousModePermissionPrompt"
CODEX_NO_PROMPTS = (("approval_policy", "never"), ("sandbox_mode", "danger-full-access"))
AMP_SESSION = {"agentMode": "low", "neoWelcomeDismissed": True}
EFFORTS = ("low", "minimal")
TAG = "kit:"


class Refused(Exception):
    pass


def say(line):
    print(f"{TAG} {line}", flush=True)


def write_private(path, text):
    """`text` into `path`, 0600, atomically (a temporary file beside it, then a rename)."""
    fd, tmp = tempfile.mkstemp(prefix=os.path.basename(path) + ".", dir=os.path.dirname(path))
    try:
        with os.fdopen(fd, "w") as f:
            f.write(text)
        os.chmod(tmp, 0o600)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def write_json(path, doc):
    write_private(path, json.dumps(doc, indent=2) + "\n")


def read_object(path, missing):
    """The JSON object in `path`; `missing` when there is no file. Anything else is refused and
    the file is left as it is."""
    try:
        with open(path) as f:
            doc = json.load(f)
    except FileNotFoundError:
        return missing
    except (OSError, ValueError):
        raise Refused(f"{path} is not JSON; not overwritten")
    if not isinstance(doc, dict):
        raise Refused(f"{path} is not a JSON object; not overwritten")
    return doc


# --------------------------------------------------------------------------- settle

def settle(cfg, work):
    prep_path = os.path.join(cfg, "prep.json")
    prep = read_object(prep_path, {})
    if not prep.get("work_root"):
        os.makedirs(work, mode=0o700, exist_ok=True)
        prep["work_root"] = work
    write_json(prep_path, prep)
    say(f"prep.json: work folder {prep['work_root']}")


# --------------------------------------------------------------------------- home-config

def codex_lists_minimal():
    """Whether the installed codex lists `minimal` for Luna: `codex debug models --bundled`, read
    as JSON; the object that names the model (slug, id, model or name) must hold "minimal"
    somewhere. Anything else (no codex, no such command, another format) is a no."""
    try:
        out = subprocess.run(["codex", "debug", "models", "--bundled"], capture_output=True,
                             timeout=60, check=False)
    except (OSError, subprocess.TimeoutExpired):
        return False
    if out.returncode != 0:
        return False
    try:
        doc = json.loads(out.stdout)
    except ValueError:
        return False

    def objects(o):
        if isinstance(o, dict):
            yield o
            for v in o.values():
                yield from objects(v)
        elif isinstance(o, list):
            for v in o:
                yield from objects(v)

    def strings(o):
        if isinstance(o, str):
            yield o
        elif isinstance(o, dict):
            for k, v in o.items():
                yield k
                yield from strings(v)
        elif isinstance(o, list):
            for v in o:
                yield from strings(v)

    named = [o for o in objects(doc) if any(o.get(k) == LUNA for k in ("slug", "id", "model", "name"))]
    return any("minimal" in set(strings(o)) for o in named)


def home(cfg, connector):
    homes = os.path.join(cfg, "capture-homes")
    path = os.path.join(homes, connector)
    os.makedirs(path, exist_ok=True)
    os.chmod(homes, 0o700)
    os.chmod(path, 0o700)
    return path


def home_config(cfg, connector, effort):
    if connector == "codex-cli":
        if effort not in EFFORTS:
            raise Refused(f"the Codex effort must be one of {', '.join(EFFORTS)}")
        if effort == "minimal" and not codex_lists_minimal():
            raise Refused(f"effort minimal: the installed codex does not list minimal for {LUNA} "
                          "(codex debug models --bundled); use low")
        path = os.path.join(home(cfg, connector), "config.toml")
        try:
            with open(path) as f:
                lines = f.read().splitlines()
        except FileNotFoundError:
            lines = []
        # Top-level keys come before the first table; drop ours there and put them first.
        kept, top = [], True
        for line in lines:
            if line.lstrip().startswith("["):
                top = False
            if top and re.match(r"\s*(model|model_reasoning_effort|approval_policy|sandbox_mode)\s*=",
                                line):
                continue
            kept.append(line)
        ours = [f'model = "{LUNA}"', f'model_reasoning_effort = "{effort}"']
        ours += [f'{k} = "{v}"' for k, v in CODEX_NO_PROMPTS]
        write_private(path, "\n".join(ours + kept) + "\n")
        say(f"codex home config: model {LUNA}, reasoning effort {effort}, no approval prompts, "
            "no sandbox (the test machine or container is the sandbox)")
    elif connector == "claude-code-cli":
        path = os.path.join(home(cfg, connector), "settings.json")
        doc = read_object(path, {})
        perms = doc.get("permissions")
        if perms is not None and not isinstance(perms, dict):
            raise Refused(f"{path}: permissions is not a JSON object; not overwritten")
        doc["model"] = CLAUDE_MODEL
        doc["alwaysThinkingEnabled"] = False
        doc["permissions"] = {**(perms or {}), "defaultMode": CLAUDE_MODE}
        doc[CLAUDE_BYPASS_ACCEPTED] = True
        state_path = os.path.join(home(cfg, connector), ".claude.json")
        state = read_object(state_path, {})
        write_json(path, doc)
        seen = state.get(CLAUDE_UPSELL_KEY)
        if not (isinstance(seen, int) and seen >= CLAUDE_UPSELL_SEEN):
            state[CLAUDE_UPSELL_KEY] = CLAUDE_UPSELL_SEEN
        state[CLAUDE_AUTO_NUDGE_SEEN] = True
        write_json(state_path, state)
        say(f"claude home settings: model {CLAUDE_MODEL}, thinking off, permission mode "
            f"{CLAUDE_MODE} (disclaimer accepted), the fullscreen renderer offer answered as "
            "\"Not now\", the auto-mode offer as \"No, keep bypass permissions\"")
    elif connector == "amp-cli":
        # The run HOME (run_home = "shared"): <cfg>/capture-homes/amp-cli.run-home.
        run_home = home(cfg, connector + ".run-home")
        d = run_home
        for part in (".local", "share", "amp"):
            d = os.path.join(d, part)
            os.makedirs(d, mode=0o700, exist_ok=True)
        path = os.path.join(d, "session.json")
        doc = read_object(path, {})
        doc.update(AMP_SESSION)
        write_json(path, doc)
        say("amp session: agent mode low, the welcome dismissed")


# --------------------------------------------------------------------------- model-check

def new_files(root, since, keep):
    for d, _, files in os.walk(root):
        for f in files:
            path = os.path.join(d, f)
            if keep(f) and os.stat(path).st_mtime >= since:
                yield path


def json_lines(path):
    with open(path, errors="replace") as f:
        for line in f:
            try:
                o = json.loads(line)
            except ValueError:
                continue
            if isinstance(o, dict):
                yield o


def model_check(cfg, connector, marker, effort):
    """True when the run kept the rule (or the connector has none)."""
    since = os.stat(marker).st_mtime
    homes = os.path.join(cfg, "capture-homes")
    if connector == "codex-cli":
        turns = []
        for path in new_files(os.path.join(homes, "codex-cli", "sessions"), since,
                              lambda f: f.startswith("rollout-") and f.endswith(".jsonl")):
            for o in json_lines(path):
                if o.get("type") == "turn_context":
                    ctx = o.get("payload") or {}
                    turns.append((ctx.get("model"), ctx.get("effort")))
        seen = sorted({f"{m} at effort {e}" for m, e in turns})
        say("Codex turns of this run: " + (", ".join(seen) if seen else "none found"))
        if not turns:
            say("the model could not be confirmed (no Codex turn in the run's sessions)")
            return False
        return all(m == LUNA and e == effort for m, e in turns)
    if connector == "claude-code-cli":
        models, synthetic = Counter(), 0
        for path in new_files(os.path.join(homes, "claude-code-cli", "projects"), since,
                              lambda f: f.endswith(".jsonl")):
            for o in json_lines(path):
                if o.get("type") != "assistant":
                    continue
                msg = o.get("message")
                m = msg.get("model") if isinstance(msg, dict) else None
                if m == "<synthetic>":
                    synthetic += 1
                    continue
                models[m if isinstance(m, str) else repr(m)] += 1
        seen = ", ".join(f"{m} x{n}" for m, n in sorted(models.items()))
        say("Claude assistant messages of this run: " + (seen or "none found")
            + (f" (and {synthetic} <synthetic>)" if synthetic else ""))
        if not models:
            say("the model could not be confirmed (no Claude assistant message in the run's "
                "transcripts)")
            return False
        return all(CLAUDE_MODEL in m for m in models)
    return True


def main(argv):
    usage = __doc__.split("\n\n")[1]
    if len(argv) < 3 or argv[0] not in ("settle", "home-config", "model-check"):
        print(usage, file=sys.stderr)
        return 2
    cmd, cfg = argv[0], argv[1]
    try:
        if cmd == "settle" and len(argv) == 3:
            settle(cfg, argv[2])
            return 0
        if cmd == "home-config" and len(argv) in (3, 4):
            home_config(cfg, argv[2], argv[3] if len(argv) == 4 else "low")
            return 0
        if cmd == "model-check" and len(argv) in (4, 5):
            return 0 if model_check(cfg, argv[2], argv[3],
                                    argv[4] if len(argv) == 5 else "low") else 1
    except Refused as e:
        say(f"refused: {e}")
        return 1
    print(usage, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
