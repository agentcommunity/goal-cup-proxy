"""Tests of row identity (plan Task 1): the goalcup commit a row ran, its OS, and the commit the pad was trusted at.
No network; git runs on a temporary repository."""
import importlib.machinery
import importlib.util
import json
import os
import subprocess
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))


def load(name, file):
    loader = importlib.machinery.SourceFileLoader(name, os.path.join(HERE, file))
    mod = importlib.util.module_from_spec(importlib.util.spec_from_loader(name, loader))
    loader.exec_module(mod)
    return mod


pad, check = load("pad", "pad"), load("padcheck", "check.py")
GIT_ENV = dict(os.environ, GIT_AUTHOR_NAME="t", GIT_AUTHOR_EMAIL="t@example.com",
               GIT_COMMITTER_NAME="t", GIT_COMMITTER_EMAIL="t@example.com")


def git(repo, *args):
    return subprocess.run(["git", "-C", repo, *args], check=True, capture_output=True, text=True, env=GIT_ENV).stdout.strip()


def git_repo(tmp):
    repo = tempfile.mkdtemp(dir=tmp)
    git(repo, "init", "-q")
    for rel, text in (("Cargo.toml", '[workspace.package]\nversion = "1.0.0"\n'), ("scripts/pad/x.py", "x\n"),
                      ("crates/goalcup-verify/y.rs", "y\n"), ("README.md", "r\n"),
                      ("connectors/a-cli.toml", "a\n"), ("crates/goalcup-format/z.rs", "z\n")):
        os.makedirs(os.path.dirname(os.path.join(repo, rel)), exist_ok=True)
        with open(os.path.join(repo, rel), "w") as f:
            f.write(text)
    git(repo, "add", "-A")
    git(repo, "commit", "-qm", "one")
    return repo, git(repo, "rev-parse", "HEAD")


def commit_change(repo, rel):
    with open(os.path.join(repo, rel), "a") as f:
        f.write("more\n")
    git(repo, "commit", "-qam", "change " + rel)
    return git(repo, "rev-parse", "HEAD")


def write_trust(repo, commit, exit, all_ran, read, trust_commit=None):
    os.makedirs(os.path.join(repo, "pad-out"), exist_ok=True)
    with open(os.path.join(repo, "pad-out", "selftest.json"), "w") as f:
        json.dump({"commit": commit, "exit": exit, "all_ran": all_ran}, f)
    with open(os.path.join(repo, "pad-out", "trust.json"), "w") as f:
        json.dump({"commit": trust_commit or commit, "read": read}, f)


def evaluate(meta):
    with tempfile.TemporaryDirectory() as run:
        with open(os.path.join(run, "meta.json"), "w") as f:
            json.dump(meta, f)
        return check.evaluate(run)


class Identity(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.tmp = self._tmp.name
        self.addCleanup(self._tmp.cleanup)

    def test_build_commit_must_match_folder_name(self):
        c = "ab" * 20
        for name, want in (("abababab", c), ("cdcdcdcd", None), ("build", c)):
            d = os.path.join(self.tmp, name)
            os.makedirs(d)
            with open(os.path.join(d, "commit"), "w") as f:
                f.write(c + "\n")
            self.assertEqual(pad.build_commit(d), want)
        self.assertIsNone(pad.build_commit(os.path.join(self.tmp, "missing")))
        bad = os.path.join(self.tmp, "bad")
        os.makedirs(bad)
        with open(os.path.join(bad, "commit"), "w") as f:
            f.write("main\n")
        self.assertIsNone(pad.build_commit(bad))

    def test_goalcup_id_reads_workspace_version(self):
        repo, sha = git_repo(self.tmp)
        self.assertEqual(pad.goalcup_id(repo, sha), "1.0.0+" + sha)
        self.assertIsNone(pad.goalcup_id(repo, None))
        self.assertIsNone(pad.goalcup_id(repo, "0" * 40))

    def test_os_id(self):
        self.assertEqual(pad.os_id("sandbox"), "linux-x86_64")

    def test_trusted_at_same_and_unrelated_change(self):
        repo, c1 = git_repo(self.tmp)
        write_trust(repo, c1, exit=0, all_ran=True, read="12:00Z first-output read done")
        c2 = commit_change(repo, "README.md")
        self.assertEqual(check.trusted_at(repo, c1), c1)
        self.assertEqual(check.trusted_at(repo, c2), c1)

    def test_trusted_at_refuses(self):
        repo, c1 = git_repo(self.tmp)
        c3 = commit_change(repo, "scripts/pad/x.py")
        for kw in (dict(exit=3, all_ran=False, read="r"), dict(exit=0, all_ran=True, read=""),
                   dict(exit=0, all_ran=True, read="r", trust_commit="f" * 40)):
            write_trust(repo, c1, **kw)
            self.assertIsNone(check.trusted_at(repo, c1))
        write_trust(repo, c1, exit=0, all_ran=True, read="r")
        self.assertIsNone(check.trusted_at(repo, c3))
        self.assertIsNone(check.trusted_at(repo, None))
        for rel in ("connectors/a-cli.toml", "crates/goalcup-format/z.rs"):  # the verifier reads both
            base = git(repo, "rev-parse", "HEAD")
            write_trust(repo, base, exit=0, all_ran=True, read="r")
            self.assertEqual(check.trusted_at(repo, commit_change(repo, "README.md")), base)
            self.assertIsNone(check.trusted_at(repo, commit_change(repo, rel)))

    def test_row_takes_identity_from_meta(self):
        row = evaluate({"connector": "codex-cli", "case": "direct", "mode": "full",
                        "goalcup": "1.0.0+" + "ab" * 20, "os": "linux-x86_64"})
        self.assertEqual((row["goalcup"], row["os"], row["pad_trusted_at"]), ("1.0.0+" + "ab" * 20, "linux-x86_64", None))


if __name__ == "__main__":
    unittest.main()
