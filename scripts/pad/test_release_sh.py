"""release.sh prepare: a failure leaves the tree as it found it. Local bare origin, fake cargo and gh."""
import os, shutil, subprocess, tempfile, unittest

SCRIPT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "release.sh")


def run(*a, cwd, env=None):
    return subprocess.run(a, cwd=cwd, env=env, capture_output=True, text=True)


class ReleasePrepare(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp()
        self.addCleanup(shutil.rmtree, self.tmp, True)
        origin, self.repo, self.bin = (os.path.join(self.tmp, d) for d in ("origin.git", "repo", "bin"))
        os.mkdir(self.bin)
        for name in ("cargo", "gh"):
            self.fake(name, 0)
        run("git", "init", "-q", "--bare", "-b", "main", origin, cwd=self.tmp)
        run("git", "init", "-q", "-b", "main", self.repo, cwd=self.tmp)
        os.mkdir(os.path.join(self.repo, "scripts"))
        shutil.copy(SCRIPT, os.path.join(self.repo, "scripts", "release.sh"))
        for f, text in (("Cargo.toml", '[workspace.package]\nversion = "1.0.0"\n'), ("Cargo.lock", "# lock\n")):
            with open(os.path.join(self.repo, f), "w") as fh:
                fh.write(text)
        git = ["git", "-c", "user.name=t", "-c", "user.email=t@t"]
        run("git", "remote", "add", "origin", origin, cwd=self.repo)
        run("git", "add", ".", cwd=self.repo)
        run(*git, "commit", "-q", "-m", "init", cwd=self.repo)
        run("git", "push", "-q", "origin", "main", cwd=self.repo)
        run("git", "config", "user.name", "t", cwd=self.repo)
        run("git", "config", "user.email", "t@t", cwd=self.repo)

    def fake(self, name, code):
        p = os.path.join(self.bin, name)
        with open(p, "w") as fh:
            fh.write(f"#!/bin/sh\nexit {code}\n")
        os.chmod(p, 0o755)

    def prepare(self):
        env = dict(os.environ, PATH=self.bin + os.pathsep + os.environ["PATH"])
        return run("sh", "scripts/release.sh", "prepare", "1.0.1", cwd=self.repo, env=env)

    def assertRestored(self, r):
        self.assertNotEqual(r.returncode, 0, r.stdout)
        self.assertEqual(run("git", "status", "--porcelain", cwd=self.repo).stdout, "")
        self.assertEqual(run("git", "symbolic-ref", "--short", "HEAD", cwd=self.repo).stdout.strip(), "main")
        self.assertEqual(run("git", "branch", "--list", "release/*", cwd=self.repo).stdout, "")

    def test_failing_commit_restores_the_tree(self):
        hook = os.path.join(self.repo, ".git", "hooks", "pre-commit")
        with open(hook, "w") as fh:
            fh.write("#!/bin/sh\nexit 1\n")
        os.chmod(hook, 0o755)
        self.assertRestored(self.prepare())

    def test_failing_cargo_restores(self):
        self.fake("cargo", 1)
        self.assertRestored(self.prepare())


if __name__ == "__main__":
    unittest.main()
