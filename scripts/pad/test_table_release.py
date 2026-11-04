"""The release check and the tier rules, on throwaway git repos. No network. Run: python3.11 -m unittest discover -s scripts/pad -p 'test_table_release.py'"""
import importlib.machinery
import importlib.util
import io
from contextlib import redirect_stdout
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest import mock

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    loader = importlib.machinery.SourceFileLoader(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader))
    loader.exec_module(mod)
    return mod


table = load("padtable_release", "table")


def git(root, *a):
    return subprocess.run(["git", "-C", str(root), "-c", "user.name=t", "-c", "user.email=t@t", *a],
                          capture_output=True, text=True, check=True).stdout.strip()


def commit(root, files, msg):
    for rel, text in files.items():
        (root / rel).parent.mkdir(parents=True, exist_ok=True)
        (root / rel).write_text(text, encoding="utf-8")
    git(root, "add", "-A")
    git(root, "commit", "-q", "-m", msg)
    return git(root, "rev-parse", "HEAD")


def mkrepo(tmp, rows, tiers_text=None, harness=None):
    """Repo in tmp: a base commit (a guarded file), then a second commit with the rows at ROWS_REL, tiers_text at
    connectors/tiers.toml and harness {cid: text} under scripts/pad/harnesses (Repo.repo also patches table.ROOT).
    A row goalcup ending "+BASE" (green_rows' default) names the base commit. Returns the sha."""
    root = Path(tmp)
    git(root, "init", "-q")
    base = commit(root, {"crates/x/lib.rs": "// base\n"}, "base")
    files = {}
    if rows is not None:
        files[table.ROWS_REL] = "".join(json.dumps({k: v.replace("+BASE", "+" + base) if k == "goalcup" else v for k, v in r.items()}) + "\n"
                                        for r in rows)
    if tiers_text is not None:
        files["connectors/tiers.toml"] = tiers_text
    for cid, text in (harness or {}).items():
        files["scripts/pad/harnesses/%s.toml" % cid] = text
    return commit(root, files or {"README": "x\n"}, "rows")


def green_rows(base_sha="BASE"):
    """Rows that pass the release check: 8 core tier rows and the codex kill-recorder row."""
    v = {"claude-code-cli": "2.1.312", "codex-cli": "0.165.0", "hermes-cli": "0.22.5"}
    rows, n = [], 0
    for cid in table.CORE:
        for case in table.CASES:
            if (cid, case) == ("claude-code-cli", "gateway"):
                continue
            n += 1
            rows.append({"connector": cid, "case": case, "mode": "full", "variant": [], "result": "PASS",
                         "checks": {k: "PASS" for k in table.CHECKS}, "goalcup": "1.0.0+" + base_sha,
                         "os": "darwin-arm64", "harness_version": v[cid], "started": "2026-11-03T19:%02d:00Z" % n})
    rows.append(dict(rows[2], checks=dict(rows[2]["checks"]), variant=["kill-recorder@3"], started="2026-11-03T20:00:00Z",
                     exec={"h2-parallel.sh": "PASS", "long-tunnel.sh": "PASS"}))
    assert rows[2]["connector"] == "codex-cli" and rows[2]["case"] == "direct"
    return rows


class Repo(unittest.TestCase):
    def repo(self, rows, **kw):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        p = mock.patch.object(table, "ROOT", Path(tmp.name))
        p.start()
        self.addCleanup(p.stop)
        return mkrepo(tmp.name, rows, **kw)

    def check(self, sha):
        with redirect_stdout(io.StringIO()) as out:
            code = table.release_check(sha)
        return code, out.getvalue()


class PathAndPass(Repo):

    def test_green_rows_pass_at_the_stable_path(self):
        sha = self.repo(green_rows())
        self.assertEqual(table.ROWS_REL, "scripts/pad/rows/pad-rows.jsonl")
        with redirect_stdout(io.StringIO()) as out:
            self.assertEqual(table.release_check(sha), 0, out.getvalue())

    def test_missing_rows_file(self):
        sha = self.repo(None)
        with redirect_stdout(io.StringIO()) as out:
            self.assertEqual(table.release_check(sha), 1)
        self.assertEqual(out.getvalue().strip(), "no committed scripts/pad/rows/pad-rows.jsonl at %s" % sha)


if __name__ == "__main__":
    unittest.main()
