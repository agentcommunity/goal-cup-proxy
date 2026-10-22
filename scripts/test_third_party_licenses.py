"""Tests for scripts/third-party-licenses.py: the SPDX allowlist and the standard-text fallback.
No cargo: packages are made by hand in a temporary directory."""

import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("tpl", HERE / "third-party-licenses.py")
tpl = importlib.util.module_from_spec(spec)
spec.loader.exec_module(tpl)


class Allowlist(unittest.TestCase):
    def test_allowed_expressions(self):
        for expr in ("MIT", "MIT OR Apache-2.0", "Apache-2.0 OR MIT", "MIT/Apache-2.0",
                     "Unlicense OR MIT", "0BSD OR MIT OR Apache-2.0", "MIT AND BSD-3-Clause",
                     "(MIT OR Apache-2.0) AND Unicode-3.0", "Apache-2.0 AND ISC",
                     "CDLA-Permissive-2.0", "MIT OR Zlib OR Apache-2.0", "BSD-2-Clause",
                     "MPL-2.0", "Apache-2.0 WITH LLVM-exception"):
            with self.subTest(expr=expr):
                self.assertTrue(tpl.licence_allowed(expr))

    def test_refused_expressions(self):
        for expr in ("GPL-3.0", "GPL-3.0 OR LGPL-2.1", "MIT AND GPL-2.0", "Unlicense",
                     "(MIT OR GPL-3.0) AND AGPL-3.0"):
            with self.subTest(expr=expr):
                self.assertFalse(tpl.licence_allowed(expr))

    def test_malformed_or_missing_is_an_error(self):
        for expr in (None, "", "MIT OR", "(MIT", "MIT)", "AND MIT"):
            with self.subTest(expr=expr), self.assertRaises(tpl.LicenceError):
                tpl.licence_allowed(expr)


class Packages(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)

    def package(self, name, licence, files=(), authors=("Jane Doe <jane@example.org>",)):
        d = self.root / name
        d.mkdir()
        (d / "Cargo.toml").write_text("[package]\n")
        for f in files:
            (d / f).write_text(f"the {f} text of {name}\n")
        return {"name": name, "version": "1.0.0", "license": licence, "authors": list(authors),
                "manifest_path": str(d / "Cargo.toml"), "source": "registry"}

    def test_a_package_with_a_disallowed_licence_is_refused(self):
        good = self.package("good", "MIT", ["LICENSE"])
        bad = self.package("bad", "GPL-3.0-only", ["COPYING"])
        out = tpl.refusals([good, bad])
        self.assertEqual(len(out), 1)
        self.assertIn("bad 1.0.0", out[0])

    def test_a_package_without_a_licence_file_gets_the_standard_text(self):
        bare = self.package("bare", "MIT/Apache-2.0")
        self.assertEqual(tpl.refusals([bare]), [])
        text = tpl.render([bare], "x86_64-unknown-linux-gnu")
        self.assertIn("Copyright (c) Jane Doe", text)
        self.assertNotIn("jane@example.org", text)
        self.assertIn("Permission is hereby granted", text)
        self.assertIn("Apache License", text)

    def test_a_package_without_a_file_or_a_known_text_is_refused(self):
        bare = self.package("isc-bare", "ISC")
        self.assertEqual(len(tpl.refusals([bare])), 1)

    def test_shipped_files_are_carried(self):
        p = self.package("files", "MIT OR Apache-2.0", ["LICENSE-MIT", "LICENSE-APACHE"])
        text = tpl.render([p], "t")
        self.assertIn("the LICENSE-MIT text of files", text)
        self.assertIn("the LICENSE-APACHE text of files", text)
        self.assertNotIn("standard text", text)

    def test_only_normal_dependencies_of_the_shipped_binaries_are_followed(self):
        meta = {"packages": [
            {"id": "cli", "name": "goalcup-cli", "source": None},
            {"id": "a", "name": "a", "version": "1", "source": "registry"},
            {"id": "b", "name": "b", "version": "1", "source": "registry"}],
            "resolve": {"nodes": [
                {"id": "cli", "deps": [{"pkg": "a", "dep_kinds": [{"kind": None}]},
                                       {"pkg": "b", "dep_kinds": [{"kind": "dev"}]}]},
                {"id": "a", "deps": []}, {"id": "b", "deps": []}]}}
        self.assertEqual([p["name"] for p in tpl.linked_packages(json.loads(json.dumps(meta)))],
                         ["a"])


if __name__ == "__main__":
    unittest.main()
