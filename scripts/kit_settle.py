#!/usr/bin/env python3
"""What the test kit settles in a goalcup configuration directory, for a Docker cell (called by
scripts/linux-matrix.sh inside the container) and a host cell (called by scripts/campaign.py) alike.

    kit_settle.py settle CFG WORK [REPO_URL]

settle       prep.json gets `repo_url` (REPO_URL, default the empty practice repository) and
             `work_root` (kept when set, else WORK); its other keys are kept. github.json gets
             that `repo_url` and the token from the file named in MATRIX_GITHUB_TOKEN_FILE. Both
             0600, each written atomically. A competition or test start asks GitHub, with that
             token, whether the repository is empty, so a test run needs a real token; a practice
             start does not, and without the file an existing github.json for the repository is
             kept, else a format-valid placeholder (never sent: a practice clone is anonymous) is
             written. A prep.json that names a prepared full-flow repository (another repository,
             with a github.json for it) is kept as it is, with its github.json.

The token is a FILE whose path is only in the environment; its value is never in an argument,
a log or this script's output. Exit 0 when done, 1 when refused (one line saying why), 2 on a bad
command line.
"""

import json
import os
import sys
import tempfile

PRACTICE_REPO = "https://github.com/agentcommunity/goalcup-matrix-empty"
TOKEN_ENV = "MATRIX_GITHUB_TOKEN_FILE"
# Format-valid and fake, for a practice start only (scripts/matrix.sh writes the same one).
PLACEHOLDER_TOKEN = "ghp_matrixfaketoken00000000000000000000"
# What the CLI accepts as a token (crates/goalcup-cli/src/account/github.rs, checked_token).
TOKEN_MIN, TOKEN_MAX = 20, 255
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


def main(argv):
    usage = __doc__.split("\n\n")[1]
    if len(argv) < 3 or argv[0] != "settle":
        print(usage, file=sys.stderr)
        return 2
    cmd, cfg = argv[0], argv[1]
    try:
        if cmd == "settle" and len(argv) in (3, 4):
            settle(cfg, argv[2], argv[3] if len(argv) == 4 else PRACTICE_REPO)
            return 0
    except Refused as e:
        say(f"refused: {e}")
        return 1
    print(usage, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
