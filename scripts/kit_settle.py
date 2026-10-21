#!/usr/bin/env python3
"""What the test kit settles in a goalcup configuration directory, for a Docker cell (called by
scripts/linux-matrix.sh inside the container) and a host cell (called by scripts/campaign.py) alike.

    kit_settle.py settle CFG WORK [REPO_URL]
    kit_settle.py home-config CFG CONNECTOR [EFFORT]
    kit_settle.py model-check CFG CONNECTOR MARKER [EFFORT]

settle       prep.json gets `repo_url` (REPO_URL, default the empty practice repository) and
             `work_root` (kept when set, else WORK); its other keys are kept. github.json gets
             that `repo_url` and the token from the file named in MATRIX_GITHUB_TOKEN_FILE. Both
             0600, each written atomically. A competition or test start asks GitHub, with that
             token, whether the repository is empty, so a test run needs a real token; a practice
             start does not, and without the file an existing github.json for the repository is
             kept, else a format-valid placeholder (never sent: a practice clone is anonymous) is
             written. A prep.json that names a prepared full-flow repository (another repository,
             with a github.json for it) is kept as it is, with its github.json.
home-config  the owner's cheapest-model rules, in the connector's tracker home (never ~/.codex or
             ~/.claude): codex-cli's config.toml gets `model = "gpt-5.6-luna"` and
             `model_reasoning_effort` = EFFORT (low, the default, or minimal: only when the
             installed codex lists minimal for that model); claude-code-cli's settings.json gets
             `"model": "haiku"` and `"alwaysThinkingEnabled": false`, other keys kept.
model-check  after the run: every Codex turn (rollouts' `turn_context`) of the sessions written
             since MARKER must be gpt-5.6-luna at EFFORT; every Claude assistant message of the
             transcripts under projects/ (subagents too) must name a haiku model (`<synthetic>`
             is no model call, counted apart). Finding none fails too. Exit 1 on a failure.

The token is a FILE whose path is only in the environment; its value is never in an argument,
a log or this script's output. Exit 0 when done, 1 when refused (one line saying why), 2 on a bad
command line.
"""

from collections import Counter
import json
import os
import re
import subprocess
import sys
import tempfile

PRACTICE_REPO = "https://github.com/agentcommunity/goalcup-matrix-empty"
TOKEN_ENV = "MATRIX_GITHUB_TOKEN_FILE"
# Format-valid and fake, for a practice start only (scripts/matrix.sh writes the same one).
PLACEHOLDER_TOKEN = "ghp_matrixfaketoken00000000000000000000"
# What the CLI accepts as a token (crates/goalcup-cli/src/account/github.rs, checked_token).
TOKEN_MIN, TOKEN_MAX = 20, 255
LUNA = "gpt-5.6-luna"
CLAUDE_MODEL = "haiku"
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


def same_repo(a, b):
    def norm(u):
        u = (u or "").strip().rstrip("/")
        return (u[:-4] if u.endswith(".git") else u).lower()
    return bool(norm(a)) and norm(a) == norm(b)


def read_token(path):
    """The token in the file, checked as the CLI checks it. The refusal never repeats it."""
    if not os.path.isabs(path):
        raise Refused(f"{TOKEN_ENV} is given as an absolute path")
    try:
        with open(path, "rb") as f:
            raw = f.read(TOKEN_MAX + 64)
    except OSError as e:
        raise Refused(f"the GitHub token file cannot be read: {e.strerror}")
    token = raw.decode("ascii", "replace").strip()
    if not (TOKEN_MIN <= len(token) <= TOKEN_MAX
            and all(0x21 <= ord(c) <= 0x7e for c in token)):
        raise Refused(f"the GitHub token file holds no token ({TOKEN_MIN} to {TOKEN_MAX} "
                      "printable characters on one line)")
    return token


# --------------------------------------------------------------------------- settle

def settle(cfg, work, repo):
    token_file = os.environ.get(TOKEN_ENV) or ""
    token = read_token(token_file) if token_file else None
    prep_path = os.path.join(cfg, "prep.json")
    gh_path = os.path.join(cfg, "github.json")
    prep = read_object(prep_path, {})
    gh = read_object(gh_path, None)
    have = prep.get("repo_url")
    if have and not same_repo(have, repo) and gh and same_repo(gh.get("repo_url"), have):
        say(f"prep.json names the prepared repository {have}: kept, with its github.json")
        return
    prep["repo_url"] = repo
    if not prep.get("work_root"):
        os.makedirs(work, mode=0o700, exist_ok=True)
        prep["work_root"] = work
    write_json(prep_path, prep)
    if token is not None:
        write_json(gh_path, {"repo_url": repo, "token": token})
        say(f"github.json: {repo}, the token from {TOKEN_ENV}")
    elif gh and same_repo(gh.get("repo_url"), repo):
        say(f"github.json: {repo}, kept (no {TOKEN_ENV}; a test run needs one)")
    else:
        write_json(gh_path, {"repo_url": repo, "token": PLACEHOLDER_TOKEN})
        say(f"github.json: {repo}, a placeholder token (practice only; a test run needs "
            f"{TOKEN_ENV})")


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
            if top and re.match(r"\s*(model|model_reasoning_effort)\s*=", line):
                continue
            kept.append(line)
        write_private(path, "\n".join([f'model = "{LUNA}"',
                                       f'model_reasoning_effort = "{effort}"'] + kept) + "\n")
        say(f"codex home config: model {LUNA}, reasoning effort {effort}")
    elif connector == "claude-code-cli":
        path = os.path.join(home(cfg, connector), "settings.json")
        doc = read_object(path, {})
        doc["model"] = CLAUDE_MODEL
        doc["alwaysThinkingEnabled"] = False
        write_json(path, doc)
        say(f"claude home settings: model {CLAUDE_MODEL}, thinking off")


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
        if cmd == "settle" and len(argv) in (3, 4):
            settle(cfg, argv[2], argv[3] if len(argv) == 4 else PRACTICE_REPO)
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
