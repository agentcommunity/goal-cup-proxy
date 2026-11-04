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
    """Repo in tmp: a base commit (a guarded file), then a commit with the rows at ROWS_REL and harness {cid: text}
    under scripts/pad/harnesses, then connectors/tiers.toml: tiers_text, or None for the file those rows render,
    or False for none. A row goalcup ending "+BASE" (green_rows' default) names the base commit. Returns the sha."""
    root = Path(tmp)
    git(root, "init", "-q")
    base = commit(root, {"crates/x/lib.rs": "// base\n"}, "base")
    files = {}
    if rows is not None:
        files[table.ROWS_REL] = "".join(json.dumps({k: v.replace("+BASE", "+" + base) if k == "goalcup" else v for k, v in r.items()}) + "\n"
                                        for r in rows)
    for cid, text in dict({c: "[gateway]\n" for c in table.CORE}, **(harness or {})).items():  # a CORE file is required
        if text is not None:
            files["scripts/pad/harnesses/%s.toml" % cid] = text
    sha = commit(root, files or {"README": "x\n"}, "rows")
    if tiers_text is None and rows is not None:
        with mock.patch.object(table, "ROOT", root):
            lines = lambda c: table.harness_lines(c, sha)
            got = [json.loads(ln) for ln in (root / table.ROWS_REL).read_text(encoding="utf-8").splitlines()]
            tiers_text = table.render_all(got, lines)[1]
    if tiers_text:
        sha = commit(root, {"connectors/tiers.toml": tiers_text}, "tiers")
    return sha


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


def hermes_rows(**over):
    return [dict(r, **over) for r in green_rows() if r["connector"] == "hermes-cli"]


class Agreement(Repo):
    """tier_for and release_check use one rule for what counts."""

    def tier(self, cid, rows):
        with mock.patch.object(table, "harness_lines", return_value=["[gateway]"]):
            return table.tier_for(cid, rows)

    def test_claude_gateway_fail_does_not_block_claude(self):
        rows = green_rows() + [dict(green_rows()[0], case="gateway", result="FAIL", started="2026-11-03T21:00:00Z")]
        sha = self.repo(rows)
        self.assertEqual(self.tier("claude-code-cli", rows)[0], "verified")
        code, out = self.check(sha)
        self.assertEqual(code, 0, out)

    def test_hermes_c3_not_exercised_counts(self):
        rows = [dict(r, result="INCONCLUSIVE", checks=dict(r["checks"], C3="INCONCLUSIVE"),
                     why=["C3 INCONCLUSIVE: exercise not done"]) for r in hermes_rows()]
        self.assertEqual(self.tier("hermes-cli", rows)[0], "verified")

    def test_masked_upstream_blocks_both(self):
        rows = green_rows()
        i = next(i for i, r in enumerate(rows) if r["connector"] == "hermes-cli")
        rows[i] = dict(rows[i], facts={"masked_upstream": True})
        sha = self.repo(rows)
        tier = self.tier("hermes-cli", rows)
        self.assertEqual(tier[0], "not verified")
        self.assertIn("masked upstream", tier[3])
        self.assertIn("C3 passed as masked upstream", self.check(sha)[1])

    def test_exec_verdict_missing(self):
        rows = green_rows()
        rows[-1] = {k: v for k, v in rows[-1].items() if k != "exec"}
        self.assertIn("exec h2-parallel.sh verdict PASS: missing", self.check(self.repo(rows))[1])


class PadOnlyRows(Repo):
    def tier(self, rows):
        with mock.patch.object(table, "harness_lines", return_value=["[gateway]"]):
            return table.tier_for("hermes-cli", rows)

    def test_pad_only_row_never_counts(self):
        rows = green_rows()
        i = next(i for i, r in enumerate(rows) if r["connector"] == "hermes-cli" and r["case"] == "direct")
        rows[i] = dict(rows[i], pad_only=["x", "y"])
        tier = self.tier(rows)
        self.assertEqual(tier[0], "not verified")
        self.assertEqual(tier[3], "direct pad-only config: x; y")
        self.assertIn("hermes-cli direct full: result PASS, pad-only config", self.check(self.repo(rows))[1])

    def test_legacy_row_without_key_counts(self):
        rows = hermes_rows()
        self.assertTrue(all("pad_only" not in r for r in rows))
        self.assertEqual(self.tier(rows)[0], "verified")


class TiersDiff(Repo):
    DIFFERS = "connectors/tiers.toml differs from the rows; run scripts/pad/table and commit it"
    NOTE = '[gateway]\ntier_note = "%s"\n'

    def test_matching_tiers_pass(self):
        code, out = self.check(self.repo(green_rows()))
        self.assertEqual(code, 0, out)

    def test_seed_tiers_fail(self):
        seed = table.render_tiers({c: table.tier_for(c, []) for c in table.CONNECTOR_IDS})
        code, out = self.check(self.repo(green_rows(), tiers_text=seed))
        self.assertEqual(code, 1)
        self.assertIn(self.DIFFERS, out)

    def test_missing_tiers_file(self):
        sha = self.repo(green_rows(), tiers_text=False)
        code, out = self.check(sha)
        self.assertEqual(code, 1)
        self.assertIn("connectors/tiers.toml missing at %s" % sha, out)

    def test_harness_files_read_at_the_commit(self):
        sha = self.repo(green_rows(), harness={"hermes-cli": self.NOTE % "n1"})
        (table.ROOT / "scripts/pad/harnesses/hermes-cli.toml").write_text(self.NOTE % "n2", encoding="utf-8")
        self.assertIn('reason = "n1"', git(table.ROOT, "show", sha + ":connectors/tiers.toml"))
        code, out = self.check(sha)
        self.assertEqual(code, 0, out)

    def test_main_renders_from_committed_rows(self):
        self.repo(green_rows(), tiers_text=False)
        with mock.patch.object(table, "PAD_OUT", table.ROOT / "pad-out"), \
                mock.patch.object(table, "TIERS", table.ROOT / "connectors" / "tiers.toml"), \
                mock.patch.object(table, "RESULTS", table.ROOT / "PAD-RESULTS.md"), redirect_stdout(io.StringIO()):
            self.assertEqual(table.main([]), 0)
        text = (table.ROOT / "connectors" / "tiers.toml").read_text(encoding="utf-8")
        self.assertIn('[codex-cli]\ntier = "verified"', text)
        self.assertFalse((table.ROOT / "PAD-RESULTS.md").exists())


class TierOs(unittest.TestCase):
    def tier(self, rows):
        with mock.patch.object(table, "harness_lines", return_value=["[gateway]"]):
            return table.tier_for("hermes-cli", rows)

    def test_verified_names_the_os(self):
        self.assertEqual(self.tier(hermes_rows()), ("verified", "0.22.5", "darwin-arm64", ""))

    def test_two_systems_do_not_merge(self):
        rows = hermes_rows()
        rows[0] = dict(rows[0], os="linux-x86_64")
        self.assertEqual(self.tier(rows), ("not verified", "", "", "case rows on different versions or systems"))

    def test_row_without_os(self):
        rows = [{k: v for k, v in r.items() if k != "os"} for r in hermes_rows()]
        self.assertEqual(self.tier(rows), ("not verified", "", "", "no system on a case row"))

    def test_render_has_os_line(self):
        self.assertIn('os = "darwin-arm64"\nreason', table.render_tiers({"hermes-cli": self.tier(hermes_rows())}))


class Builds(unittest.TestCase):
    def builds(self, rows):
        with mock.patch.object(table, "harness_lines", return_value=["[gateway]"]):
            return table.builds("hermes-cli", rows)

    def at(self, v, os_, hour=19):
        return hermes_rows(harness_version=v, os=os_, started="2026-11-03T%02d:00:00Z" % hour)

    def test_two_versions_two_systems(self):
        rows = self.at("0.22.5", "darwin-arm64") + self.at("0.22.6", "darwin-arm64", 20) + self.at("0.22.5", "linux-x86_64", 21)
        self.assertEqual(self.builds(rows), [("0.22.6", "darwin-arm64"), ("0.22.5", "darwin-arm64"), ("0.22.5", "linux-x86_64")])

    def test_incomplete_build_is_not_listed(self):
        rows = self.at("0.22.5", "darwin-arm64") + [r for r in self.at("0.22.6", "darwin-arm64", 20) if r["case"] != "dodger"]
        self.assertEqual(self.builds(rows), [("0.22.5", "darwin-arm64")])

    def test_last_five_per_os(self):
        rows = [r for n in range(2, 12) for r in self.at("0.%d.0" % n, "darwin-arm64")] + self.at("0.1.0", "linux-x86_64")
        self.assertEqual(self.builds(rows), [("0.%d.0" % n, "darwin-arm64") for n in (11, 10, 9, 8, 7)] + [("0.1.0", "linux-x86_64")])

    def test_mixed_version_latest_rows_still_verified(self):  # a matrix: direct's latest on 0.22.6, gateway's on 0.22.5
        rows = self.at("0.22.6", "darwin-arm64", 19) + self.at("0.22.5", "darwin-arm64", 20)
        rows = [dict(r, started="2026-11-03T21:00:00Z") if r["case"] == "direct" and r["harness_version"] == "0.22.6" else r for r in rows]
        with mock.patch.object(table, "harness_lines", return_value=["[gateway]"]):
            self.assertEqual(table.tier_for("hermes-cli", rows)[:3], ("verified", "0.22.6", "darwin-arm64"))

    def test_release_sorts_above_its_pre_release(self):
        self.assertGreater(table.vkey("0.22.6"), table.vkey("0.22.6-rc1"))
        self.assertGreater(table.vkey("0.22.6.1"), table.vkey("0.22.6"))

    def test_render_has_builds_tables(self):
        rows = self.at("0.22.5", "darwin-arm64") + self.at("0.22.6", "darwin-arm64", 20)
        with mock.patch.object(table, "harness_lines", return_value=["[gateway]"]):
            text = table.render_tiers({"hermes-cli": table.tier_for("hermes-cli", rows)}, {"hermes-cli": self.builds(rows)})
        self.assertIn('[[hermes-cli.builds]]\nversion = "0.22.6"\nos = "darwin-arm64"\n', text)


class ReleaseVersions(Repo):
    HERMES = 'release_versions = ["0.22.5", "0.22.6"]\n[gateway]\n'

    def test_listed_version_without_rows_fails(self):
        code, out = self.check(self.repo(green_rows(), harness={"hermes-cli": self.HERMES}))
        self.assertEqual(code, 1, out)
        self.assertIn("hermes-cli direct full 0.22.6 on darwin-arm64: missing", out)
        self.assertNotIn("0.22.5 on darwin-arm64", out)

    def test_every_listed_version_verified_passes(self):
        rows = green_rows()
        rows += [dict(r, harness_version="0.22.6", started=r["started"].replace("T19", "T21"))
                 for r in rows if r["connector"] == "hermes-cli"]
        code, out = self.check(self.repo(rows, harness={"hermes-cli": self.HERMES}))
        self.assertEqual(code, 0, out)

    def test_other_system_does_not_count(self):
        rows = green_rows()
        rows += [dict(r, harness_version="0.22.6", os="linux-x86_64", started=r["started"].replace("T19", "T21"))
                 for r in rows if r["connector"] == "hermes-cli"]
        code, out = self.check(self.repo(rows, harness={"hermes-cli": self.HERMES}))
        self.assertIn("hermes-cli gateway full 0.22.6 on darwin-arm64: missing", out)

    def test_unreadable_key_refuses(self):
        for text in ('release_versions = [\n"0.22.6",\n]\n', 'release_versions = []\n', 'release_versions = "0.22.6"\n'):
            code, out = self.check(self.repo(green_rows(), harness={"hermes-cli": text + "[gateway]\n"}))
            self.assertEqual(code, 1, text)
            self.assertIn("hermes-cli.toml release_versions unreadable", out)

    def test_trailing_comment_is_read(self):
        self.assertEqual(table.release_versions(['release_versions = ["0.22.5", "0.22.6"]  # owner']), ["0.22.5", "0.22.6"])

    def test_missing_core_harness_file_refuses(self):
        code, out = self.check(self.repo(green_rows(), harness={"hermes-cli": None}))
        self.assertIn("scripts/pad/harnesses/hermes-cli.toml missing at", out)

    def test_no_gateway_route_needs_no_gateway_row(self):  # the same cases builds() needs (G.2)
        rows = [r for r in green_rows() if (r["connector"], r["case"]) != ("codex-cli", "gateway")]
        code, out = self.check(self.repo(rows, harness={"codex-cli": "# no gateway route\n"}))
        self.assertEqual(code, 0, out)
        code, out = self.check(self.repo(rows))
        self.assertIn("codex-cli gateway full: missing", out)

    def test_the_hermes_file_requires_0_22_6(self):
        self.assertIn("0.22.6", table.release_versions(table.harness_lines("hermes-cli")))


if __name__ == "__main__":
    unittest.main()
