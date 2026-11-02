import os, subprocess, sys, tempfile, unittest

HERE = os.path.dirname(os.path.abspath(__file__))
KEYSCAN = os.path.join(os.path.dirname(HERE), "keyscan.py")
K = "sk-test-" + "0123456789abcdef"


def w(d, name, text):
    p = os.path.join(d, name)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w") as f:
        f.write(text)


def cli(args, env):
    e = {"PATH": os.environ.get("PATH", "/usr/bin:/bin")}
    e.update(env)
    return subprocess.run([sys.executable, KEYSCAN] + list(args), env=e,
                          stdout=subprocess.PIPE, stderr=subprocess.PIPE, universal_newlines=True)


class KeyscanTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.d = self.tmp.name

    def tearDown(self):
        self.tmp.cleanup()

    def test_dotfile_and_py_scanned(self):
        d = self.d
        w(d, ".env.local", "X=" + K)
        w(d, "a.py", "k = '%s'" % K[3:19])
        r = cli([d], {"CLAUDE_API_KEY": K})
        self.assertEqual(r.returncode, 1)
        hits = sorted(l.split()[2].rsplit("/", 1)[1] for l in r.stdout.splitlines() if l.startswith("HIT"))
        self.assertEqual(hits, [".env.local", "a.py"])

    def test_no_values_exit_3(self):
        w(self.d, "x", "nothing")
        r = cli([self.d], {})
        self.assertEqual(r.returncode, 3)
        self.assertIn("nothing proven", r.stdout)

    def test_clean_exit_0_names_unset(self):
        w(self.d, "x", "nothing")
        r = cli([self.d], {"OPENAI_API_KEY": K})
        self.assertEqual(r.returncode, 0)
        self.assertIn("unset: VERCEL_TOKEN", r.stdout)

    def test_database_url_password_not_scheme(self):
        d = self.d
        u = "postgresql://reader:pw0123456789abc@db.example.internal:5432/x"
        w(d, "doc.md", "connect with postgresql://reader@host")
        self.assertEqual(cli([d], {"DATABASE_URL": u}).returncode, 0)
        w(d, "leak.txt", "pw0123456789abc")
        self.assertEqual(cli([d], {"DATABASE_URL": u}).returncode, 1)

    def test_host_matched_whole_only(self):
        d = self.d
        u = "postgresql://reader:pw0123456789abc@db.long-project-name.example.internal:5432/x"
        w(d, "doc.md", "see example.internal docs and ect-name.example.internal")
        self.assertEqual(cli([d], {"DATABASE_URL": u}).returncode, 0)
        w(d, "leak.txt", "host db.long-project-name.example.internal here")
        self.assertEqual(cli([d], {"DATABASE_URL": u}).returncode, 1)

    def test_missing_path_exit_2(self):
        r = cli([os.path.join(self.d, "nope")], {"CLAUDE_API_KEY": K})
        self.assertEqual(r.returncode, 2)

    def test_scrub_then_clean(self):
        d = self.d
        w(d, "log.txt", "a " + K + " b")
        cli(["--scrub", d], {"CLAUDE_API_KEY": K})
        with open(d + "/log.txt") as f:
            self.assertIn("[KEY-FRAGMENT-REDACTED]", f.read())
        self.assertEqual(cli([d], {"CLAUDE_API_KEY": K}).returncode, 0)

    def test_never_prints_value(self):
        w(self.d, "x", K)
        r = cli([self.d], {"CLAUDE_API_KEY": K})
        self.assertNotIn(K[:12], r.stdout + r.stderr)

    def test_git_dir_skipped(self):
        w(self.d, ".git/objects/x", K)
        self.assertEqual(cli([self.d], {"CLAUDE_API_KEY": K}).returncode, 0)


if __name__ == "__main__":
    unittest.main()
