"""Behavioral tests for the offline research replay comparator."""

from __future__ import annotations

import json
import os
import shutil
import stat
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[1]
SCRIPT = REPO / "scripts/research-replay.py"
VERIFY = Path(os.environ.get("GOALCUP_VERIFY", REPO / "target/debug/goalcup-verify"))
CORPUS = REPO / "crates/goalcup-verifier/tests/fixtures/v5-corpus"


def copy_trace(name: str, parent: Path, target_name: str | None = None) -> Path:
    target = parent / (target_name or name)
    shutil.copytree(CORPUS / name, target)
    return target


@unittest.skipUnless(VERIFY.is_file(), "build goalcup-verify or set GOALCUP_VERIFY")
class ResearchReplayTests(unittest.TestCase):
    def invoke(self, local: Path, rebuilt: Path, output: Path, verifier: Path = VERIFY,
               extra: list[str] | None = None) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [sys.executable, str(SCRIPT), "--local", str(local), "--rebuilt", str(rebuilt),
             "--output", str(output), "--verifier", str(verifier), *(extra or [])],
            capture_output=True, text=True, timeout=60,
        )

    def run_pair(self, left_name: str, right_name: str | None = None):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace(left_name, root, "local-trace")
        right = copy_trace(right_name or left_name, root, "rebuilt-trace")
        output = root / "private-result"
        result = self.invoke(left, right, output)
        report = json.loads((output / "comparison.json").read_text())
        return result, report, output, left, right

    def test_matching_valid_trace_has_equal_chain_observations_and_report(self):
        # This corpus entry contains an explicit unknown observation.
        result, report, output, _, _ = self.run_pair("sealed-end-interrupted")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertTrue(report["chain"]["equal"])
        self.assertTrue(report["observations"]["equal"])
        self.assertTrue(report["full_report"]["equal"])
        self.assertEqual(report["observations"]["local_count"], 1)
        self.assertTrue(report["observations"]["includes_unknown_observations"])
        observation = json.loads((output / "local.research.ndjson").read_text().splitlines()[0])
        self.assertEqual(observation["observation"]["kind"], "unknown")
        self.assertEqual(report["aggregate_exit"], 0)
        self.assertEqual(report["legs"]["local"]["integrity_exit"], 0)
        self.assertEqual(report["legs"]["rebuilt"]["research_exit"], 0)
        for name in ("local.integrity.stdout", "local.integrity.stderr", "local.research.stderr",
                     "local.research.ndjson", "rebuilt.integrity.stdout", "rebuilt.research.ndjson"):
            self.assertTrue((output / name).is_file(), name)
        self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o700)
        for path in output.iterdir():
            self.assertEqual(stat.S_IMODE(path.stat().st_mode) & 0o077, 0, path.name)

    def test_matching_incomplete_trace_remains_degraded(self):
        result, report, _, _, _ = self.run_pair("unsealed-vertical")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertTrue(report["chain"]["equal"])
        self.assertEqual(report["legs"]["local"]["integrity"]["integrity"], "incomplete")
        self.assertEqual(report["aggregate_exit"], 1)

    def test_different_valid_chains_do_not_compare_as_equal(self):
        result, report, _, _, _ = self.run_pair("sealed-with-files", "sealed-old-launch")
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertFalse(report["chain"]["equal"])
        self.assertEqual(report["legs"]["local"]["integrity"]["integrity"], "valid")
        self.assertEqual(report["legs"]["rebuilt"]["integrity"]["integrity"], "valid")

    def test_matching_invalid_evidence_keeps_invalid_exit(self):
        result, report, _, _, _ = self.run_pair("torn-tail")
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertEqual(report["aggregate_exit"], 2)
        self.assertEqual(report["legs"]["local"]["integrity"]["integrity"], "invalid")

    def test_supplied_public_key_file_uses_real_verifier_and_stays_private(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-end-interrupted", root, "local-trace")
        right = copy_trace("sealed-end-interrupted", root, "rebuilt-trace")
        key_path = root / "public-key.txt"
        public_key = "ab" * 32
        key_path.write_text(public_key.upper() + "\n")
        output = root / "private-result"
        result = self.invoke(left, right, output, extra=["--dev-key-file", str(key_path)])
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads((output / "comparison.json").read_text())
        self.assertEqual(report["key_provenance"]["source"], "supplied_file")
        self.assertNotIn(public_key, (output / "comparison.json").read_text())

    def write_stub_verifier(self, path: Path, mode: str) -> Path:
        integrity_head = "3614294651c35e4f89e30d07e0b457fd53bff868552e0982ffb6d61848bd43a4"
        report_head = ("5141fc4084328f31a1fc927e8082d4d733f1ec845ab0a16b05a7d8f447763cab"
                       if mode == "wrong-binding" else integrity_head)
        if mode in ("metadata", "exit-mismatch", "wrong-binding"):
            summary_exit = 0 if mode == "metadata" else 1
            if mode == "wrong-binding":
                summary_exit = 0
            delivery_source = (' delivery={"state":"local_only" if leg == "local-trace" else "reconstructed"}'
                               if mode == "metadata" else ' delivery={"state":"local_only"}')
            source = f'''#!{sys.executable}\nimport json, pathlib, sys\nargs=sys.argv[1:]\nif "--capture-integrity" in args:\n print(json.dumps({{"schema_version":3,"integrity":"valid","validated_records":7,"validated_bytes":4111,"validated_head":"{integrity_head}"}}))\nelse:\n out=pathlib.Path(args[args.index("--output")+1])\n leg=pathlib.Path(args[args.index("--research-json")+1]).name\n{delivery_source}\n lines=[{{"type":"observation","observation":{{"kind":"unknown"}}}},{{"type":"summary","analysis":{{"state":"available"}},"delivery":delivery,"capture":{{"integrity":"valid"}},"input":{{"validated_records":7,"validated_bytes":4111,"validated_head":"{report_head}"}},"options":{{}},"run":{{}},"schema_version":3,"exit_code":{summary_exit},"key_source":"none","verifier_version":"test"}}]\n out.write_text("\\n".join(json.dumps(x) for x in lines)+"\\n")\n'''
        else:
            source = "#!/bin/sh\nexit 11\n"
        path.write_text(source)
        path.chmod(0o700)
        return path

    def test_metadata_only_difference_keeps_chain_and_observation_equal(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        verifier = self.write_stub_verifier(root / "metadata-verifier", "metadata")
        output = root / "private-result"
        result = self.invoke(left, right, output, verifier)
        self.assertEqual(result.returncode, 0, result.stderr)
        report = json.loads((output / "comparison.json").read_text())
        self.assertTrue(report["chain"]["equal"])
        self.assertTrue(report["observations"]["equal"])
        self.assertFalse(report["full_report"]["equal"])
        self.assertTrue(any("delivery.state" in path for path in report["metadata_differences"]))

    def test_incomplete_summary_is_report_failure_for_both_legs(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        source = f'''#!{sys.executable}\nimport json, pathlib, sys\nargs=sys.argv[1:]\nif "--capture-integrity" in args:\n print(json.dumps({{"schema_version":3,"integrity":"valid","validated_records":7,"validated_bytes":4111,"validated_head":"3614294651c35e4f89e30d07e0b457fd53bff868552e0982ffb6d61848bd43a4"}}))\nelse:\n pathlib.Path(args[args.index("--output")+1]).write_text('{{"type":"summary","analysis":{{}},"delivery":{{}},"capture":{{"integrity":"valid"}},"input":{{"validated_records":7,"validated_bytes":4111,"validated_head":"3614294651c35e4f89e30d07e0b457fd53bff868552e0982ffb6d61848bd43a4"}}}}\\n')\n'''
        verifier = root / "incomplete-summary-verifier"
        verifier.write_text(source)
        verifier.chmod(0o700)
        output = root / "private-result"
        result = self.invoke(left, right, output, verifier)
        self.assertEqual(result.returncode, 3)
        report = json.loads((output / "comparison.json").read_text())
        self.assertTrue(report["chain"]["equal"])
        self.assertFalse(report["observations"]["equal"])
        self.assertFalse(report["full_report"]["equal"])
        self.assertTrue(report["legs"]["local"]["errors"])

    def test_summary_exit_must_match_verifier_process_exit(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        verifier = self.write_stub_verifier(root / "exit-mismatch-verifier", "exit-mismatch")
        output = root / "private-result"
        result = self.invoke(left, right, output, verifier)
        self.assertEqual(result.returncode, 3)
        report = json.loads((output / "comparison.json").read_text())
        self.assertTrue(report["chain"]["equal"])
        self.assertTrue(report["observations"]["equal"])
        self.assertTrue(report["full_report"]["equal"])
        self.assertEqual(report["legs"]["local"]["research_exit"], 0)
        self.assertEqual(report["legs"]["local"]["research_summary"]["exit_code"], 1)
        self.assertTrue(report["legs"]["local"]["errors"])

    def test_same_wrong_summary_identity_for_both_legs_is_rejected(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        verifier = self.write_stub_verifier(root / "wrong-binding-verifier", "wrong-binding")
        output = root / "private-result"
        result = self.invoke(left, right, output, verifier)
        self.assertEqual(result.returncode, 3)
        report = json.loads((output / "comparison.json").read_text())
        self.assertTrue(report["chain"]["equal"])
        self.assertTrue(report["observations"]["equal"])
        self.assertTrue(report["full_report"]["equal"])
        for name in ("local", "rebuilt"):
            self.assertEqual(report["legs"][name]["research_exit"], 0)
            self.assertEqual(report["legs"][name]["integrity"]["validated_head"],
                             "3614294651c35e4f89e30d07e0b457fd53bff868552e0982ffb6d61848bd43a4")
            self.assertNotEqual(report["legs"][name]["research_summary"]["input"]["validated_head"],
                                report["legs"][name]["integrity"]["validated_head"])
            self.assertTrue(report["legs"][name]["errors"])

    def test_unusable_integrity_json_is_report_failure(self):
        for integrity_json in ("[]", "{}"):
            with self.subTest(integrity_json=integrity_json):
                temp = tempfile.TemporaryDirectory()
                self.addCleanup(temp.cleanup)
                root = Path(temp.name)
                left = copy_trace("sealed-with-files", root, "local-trace")
                right = copy_trace("sealed-with-files", root, "rebuilt-trace")
                source = f'''#!{sys.executable}\nimport json, pathlib, sys\nargs=sys.argv[1:]\nif "--capture-integrity" in args:\n print({integrity_json!r})\nelse:\n out=pathlib.Path(args[args.index("--output")+1])\n summary={{"type":"summary","analysis":{{}},"delivery":{{}},"capture":{{"integrity":"valid"}},"input":{{"validated_records":7,"validated_bytes":4111,"validated_head":"3614294651c35e4f89e30d07e0b457fd53bff868552e0982ffb6d61848bd43a4"}},"schema_version":3,"exit_code":0,"verifier_version":"test"}}\n out.write_text(json.dumps(summary)+"\\n")\n'''
                verifier = root / "malformed-integrity-verifier"
                verifier.write_text(source)
                verifier.chmod(0o700)
                output = root / "private-result"
                result = self.invoke(left, right, output, verifier)
                self.assertEqual(result.returncode, 3)
                report = json.loads((output / "comparison.json").read_text())
                self.assertIsNone(report["legs"]["local"]["integrity"])
                self.assertFalse(report["chain"]["equal"])
                self.assertTrue(any("integrity report" in error
                                    for error in report["legs"]["local"]["errors"]))

    def test_zero_record_null_head_preserves_incomplete_verdict(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        source = f'''#!{sys.executable}\nimport json, pathlib, sys\nargs=sys.argv[1:]\nif "--capture-integrity" in args:\n print(json.dumps({{"schema_version":3,"integrity":"incomplete","validated_records":0,"validated_bytes":0,"validated_head":None}}))\n sys.exit(1)\nelse:\n out=pathlib.Path(args[args.index("--output")+1])\n summary={{"type":"summary","analysis":{{}},"delivery":{{}},"capture":{{"integrity":"incomplete"}},"input":{{"validated_records":0,"validated_bytes":0,"validated_head":None}},"schema_version":3,"exit_code":1,"verifier_version":"test"}}\n out.write_text(json.dumps(summary)+"\\n")\n sys.exit(1)\n'''
        verifier = root / "empty-incomplete-verifier"
        verifier.write_text(source)
        verifier.chmod(0o700)
        output = root / "private-result"
        result = self.invoke(left, right, output, verifier)
        self.assertEqual(result.returncode, 1)
        report = json.loads((output / "comparison.json").read_text())
        self.assertTrue(report["chain"]["equal"])
        self.assertEqual(report["chain"]["local"]["validated_records"], 0)
        self.assertIsNone(report["chain"]["local"]["validated_head"])
        self.assertEqual(report["aggregate_exit"], 1)
        self.assertEqual(report["legs"]["local"]["errors"], [])

    def test_process_failure_exit_is_preserved_and_cannot_be_equal(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        verifier = self.write_stub_verifier(root / "failure-verifier", "failure")
        output = root / "private-result"
        result = self.invoke(left, right, output, verifier)
        self.assertEqual(result.returncode, 3)
        report = json.loads((output / "comparison.json").read_text())
        self.assertEqual(report["legs"]["local"]["integrity_exit"], 11)
        self.assertEqual(report["legs"]["local"]["research_exit"], 11)
        self.assertFalse(report["chain"]["equal"])
        self.assertFalse(report["observations"]["equal"])

    def test_existing_output_is_refused_without_modification(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        output = root / "existing-output"
        output.mkdir()
        marker = output / "keep.txt"
        marker.write_text("preserved")
        result = self.invoke(left, right, output)
        self.assertEqual(result.returncode, 3)
        self.assertEqual(marker.read_text(), "preserved")

    def test_resolved_symlink_into_trace_is_refused(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        link = root / "output-link"
        link.symlink_to(left, target_is_directory=True)
        result = self.invoke(left, right, link)
        self.assertEqual(result.returncode, 3)
        self.assertFalse((left / "comparison.json").exists())

    def test_report_failure_cannot_create_equal_evidence(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        source = f'''#!{sys.executable}\nimport json, pathlib, sys\nargs=sys.argv[1:]\nif "--capture-integrity" in args:\n print(json.dumps({{"schema_version":3,"integrity":"valid","validated_records":7,"validated_bytes":4111,"validated_head":"3614294651c35e4f89e30d07e0b457fd53bff868552e0982ffb6d61848bd43a4"}}))\nelse:\n pathlib.Path(args[args.index("--output")+1]).write_text('{{"type":"summary","analysis":{{}},"delivery":{{}},"capture":{{}},"input":{{}}}}\\n{{"type":"observation"}}\\n')\n'''
        verifier = root / "bad-report-verifier"
        verifier.write_text(source)
        verifier.chmod(0o700)
        output = root / "private-result"
        result = self.invoke(left, right, output, verifier)
        self.assertEqual(result.returncode, 3)
        report = json.loads((output / "comparison.json").read_text())
        self.assertTrue(report["chain"]["equal"])
        self.assertFalse(report["observations"]["equal"])
        self.assertFalse(report["full_report"]["equal"])
        self.assertTrue(report["legs"]["local"]["errors"])

    def test_output_inside_trace_is_refused_without_writing(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        left = copy_trace("sealed-with-files", root, "local-trace")
        right = copy_trace("sealed-with-files", root, "rebuilt-trace")
        output = left / "nested-output"
        result = self.invoke(left, right, output)
        self.assertEqual(result.returncode, 3)
        self.assertFalse(output.exists())

    def test_output_overlapping_trace_tree_is_refused(self):
        temp = tempfile.TemporaryDirectory()
        self.addCleanup(temp.cleanup)
        root = Path(temp.name)
        input_parent = root / "inputs"
        input_parent.mkdir()
        left = copy_trace("sealed-with-files", input_parent, "local-trace")
        right = copy_trace("sealed-with-files", input_parent, "rebuilt-trace")
        result = self.invoke(left, right, input_parent)
        self.assertEqual(result.returncode, 3)
        self.assertTrue(input_parent.is_dir())


if __name__ == "__main__":
    unittest.main()
