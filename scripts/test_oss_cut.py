#!/usr/bin/env python3
"""Regression tests for the fail-closed OSS cut link-check authority."""

from __future__ import annotations

import importlib.util
import io
import json
import errno
import os
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path
from unittest import mock


ROOT = Path(__file__).resolve().parents[1]
OSS_CUT = ROOT / "scripts" / "oss-cut.py"
CANONICAL_CHECKER = ROOT / "scripts" / "check-links.py"
REQUIRED = (
    ".gitattributes",
    "LICENSE",
    "NOTICE",
    "README.md",
    "SECURITY.md",
    "CONTRIBUTING.md",
    "CODE_OF_CONDUCT.md",
    "AGENTS.md",
    ".github/CODEOWNERS",
    ".github/PULL_REQUEST_TEMPLATE.md",
    ".github/workflows/ci.yml",
    "Cargo.toml",
    "Cargo.lock",
    "docs/CONTRACTS.md",
    "scripts/check-links.py",
)


def load_oss_cut():
    spec = importlib.util.spec_from_file_location("oss_cut_under_test", OSS_CUT)
    assert spec is not None and spec.loader is not None
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class OssCutTests(unittest.TestCase):
    def make_source(
        self,
        base: Path,
        *,
        checker: str | None = None,
        readme: str = "# Fixture\n",
        missing: set[str] | None = None,
        commit: bool = True,
    ) -> Path:
        source = base / "source"
        source.mkdir(parents=True)
        omitted = missing or set()
        for relative in REQUIRED:
            if relative in omitted:
                continue
            path = source / relative
            path.parent.mkdir(parents=True, exist_ok=True)
            if relative == "scripts/check-links.py":
                path.write_text(
                    CANONICAL_CHECKER.read_text(encoding="utf-8") if checker is None else checker,
                    encoding="utf-8",
                )
            elif relative == "README.md":
                path.write_text(readme, encoding="utf-8")
            else:
                path.write_text("", encoding="utf-8")
        subprocess.run(["git", "init", "-q", str(source)], check=True)
        subprocess.run(
            ["git", "-C", str(source), "config", "user.name", "OSS Cut Test"], check=True
        )
        subprocess.run(
            ["git", "-C", str(source), "config", "user.email", "oss-cut@example.com"],
            check=True,
        )
        subprocess.run(["git", "-C", str(source), "add", "."], check=True)
        if commit:
            self.commit(source)
        return source

    def commit(self, source: Path) -> None:
        subprocess.run(
            ["git", "-C", str(source), "commit", "-qm", "fixture"], check=True
        )

    def run_cut(
        self,
        source: Path,
        out: Path,
        *extra: str,
        env: dict[str, str] | None = None,
    ) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            [
                sys.executable,
                str(OSS_CUT),
                "--source",
                str(source),
                "--out",
                str(out),
                *extra,
            ],
            text=True,
            capture_output=True,
            check=False,
            env=env,
        )

    def tree_snapshot(self, root: Path) -> dict[str, tuple[str, int, bytes | None]]:
        snapshot: dict[str, tuple[str, int, bytes | None]] = {}
        for path in [root, *sorted(root.rglob("*"))]:
            relative = "." if path == root else path.relative_to(root).as_posix()
            mode = os.lstat(path).st_mode
            if path.is_dir():
                snapshot[relative] = ("directory", mode & 0o777, None)
            else:
                snapshot[relative] = ("file", mode & 0o777, path.read_bytes())
        return snapshot

    def test_clean_cut_passes_the_canonical_checker(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            result = self.run_cut(source, base / "cut")

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("check-links: 0 broken link(s)", result.stdout)
        self.assertIn("link checker: PASS", result.stdout)

    def test_gitattributes_is_required_and_copied(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base / "present")
            out = base / "present-cut"
            present_result = self.run_cut(source, out)
            attributes_copied = (out / ".gitattributes").is_file()

            missing_source = self.make_source(
                base / "missing", missing={".gitattributes"}
            )
            missing_result = self.run_cut(missing_source, base / "missing-cut")

        self.assertEqual(
            present_result.returncode,
            0,
            present_result.stdout + present_result.stderr,
        )
        self.assertTrue(attributes_copied)
        self.assertEqual(
            missing_result.returncode,
            3,
            missing_result.stdout + missing_result.stderr,
        )

    def test_gitattributes_home_path_is_strict(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            (source / ".gitattributes").write_text(
                "/" + "Users/alice/private text eol=lf\n", encoding="utf-8"
            )
            subprocess.run(
                ["git", "-C", str(source), "add", ".gitattributes"], check=True
            )
            self.commit(source)
            result = self.run_cut(source, base / "cut")

        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn(".gitattributes", result.stdout)

    def test_broken_file_and_anchor_make_cut_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            (source / "docs" / "present.md").write_text("# Present\n", encoding="utf-8")
            (source / "docs" / "source.md").write_text(
                "[x](present.md#missing)\n[absent](absent.md)\n", encoding="utf-8"
            )
            subprocess.run(
                ["git", "-C", str(source), "add", "docs/present.md", "docs/source.md"],
                check=True,
            )
            self.commit(source)
            result = self.run_cut(source, base / "cut")

        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("present.md#missing", result.stdout)
        self.assertIn("absent.md", result.stdout)
        self.assertIn("link checker: FAIL", result.stdout)

    def test_checker_execution_failure_is_not_success(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            module = load_oss_cut()
            argv = [
                str(OSS_CUT),
                "--source",
                str(source),
                "--out",
                str(base / "cut"),
            ]
            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module, "run_link_checker", return_value=-15
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()

        self.assertEqual(status, 2, output.getvalue())
        self.assertIn("link checker: FAIL", output.getvalue())

        module = load_oss_cut()
        out = Path("/tmp") / "oss-cut-test-output"
        for returncode in (7, -15):
            with self.subTest(returncode=returncode), mock.patch.object(
                module.subprocess,
                "run",
                return_value=subprocess.CompletedProcess([], returncode),
            ) as run:
                self.assertNotEqual(module.run_link_checker(out), 0)
                run.assert_called_once_with(
                    [
                        sys.executable,
                        "-I",
                        "-S",
                        str(out / "scripts/check-links.py"),
                        "--root",
                        str(out),
                    ],
                    cwd=out,
                )
        with mock.patch.object(module.subprocess, "run", side_effect=OSError("exec failed")):
            self.assertNotEqual(module.run_link_checker(out), 0)

    def test_checker_execution_is_isolated_from_committed_import_shadow(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base, readme="[broken](missing.md)\n")
            (source / "scripts" / "html.py").write_text(
                "raise SystemExit(0)\n", encoding="utf-8"
            )
            subprocess.run(
                ["git", "-C", str(source), "add", "scripts/html.py"], check=True
            )
            self.commit(source)
            result = self.run_cut(source, base / "cut")

        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("missing.md", result.stdout)
        self.assertIn("link checker: FAIL", result.stdout)

    def test_oss_cut_has_one_link_authority(self) -> None:
        source = OSS_CUT.read_text(encoding="utf-8")

        self.assertIn('"scripts/check-links.py"', source)
        self.assertIn("def run_link_checker(out: Path) -> int:", source)
        self.assertNotIn("LINK = re.compile", source)
        self.assertNotIn("FENCE = re.compile", source)
        self.assertNotIn("def check_links(", source)
        self.assertEqual(source.count("run_link_checker(out)"), 1)

    def test_repository_test_fixture_does_not_trigger_strict_scan(self) -> None:
        module = load_oss_cut()
        result = module.scan(ROOT, ["scripts/test_oss_cut.py"])

        self.assertEqual(result["strict_hits"], [])

    def test_dirty_and_unborn_sources_fail_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            dirty = self.make_source(base / "dirty")
            (dirty / "README.md").write_text("# changed after commit\n", encoding="utf-8")
            dirty_result = self.run_cut(dirty, base / "dirty-cut")

            unborn = self.make_source(base / "unborn", commit=False)
            unborn_result = self.run_cut(unborn, base / "unborn-cut")

        self.assertEqual(dirty_result.returncode, 1, dirty_result.stdout + dirty_result.stderr)
        self.assertEqual(unborn_result.returncode, 1, unborn_result.stdout + unborn_result.stderr)

    def test_ambient_git_dir_cannot_redirect_the_source_repository(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base / "intended")
            foreign = self.make_source(base / "foreign")
            subprocess.run(
                [
                    "git", "-C", str(foreign), "commit", "--allow-empty", "-qm",
                    "foreign head",
                ],
                check=True,
            )
            intended_head = subprocess.run(
                ["git", "-C", str(source), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            foreign_head = subprocess.run(
                ["git", "-C", str(foreign), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            self.assertNotEqual(intended_head, foreign_head)
            hostile_env = os.environ.copy()
            hostile_env["GIT_DIR"] = str(foreign / ".git")
            hostile_env["GIT_WORK_TREE"] = str(source)
            out = base / "cut"

            result = self.run_cut(source, out, env=hostile_env)
            marker = json.loads((out / ".oss-cut-marker").read_text(encoding="utf-8"))

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(marker["source_head"], intended_head)

    def test_ambient_git_reference_backend_cannot_redirect_source_head(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            intended_head = subprocess.run(
                ["git", "-C", str(source), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            head_ref = subprocess.run(
                ["git", "-C", str(source), "symbolic-ref", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            subprocess.run(
                ["git", "-C", str(source), "commit", "--allow-empty", "-qm", "foreign head"],
                check=True,
            )
            foreign_head = subprocess.run(
                ["git", "-C", str(source), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            subprocess.run(
                ["git", "-C", str(source), "reset", "--hard", "-q", intended_head],
                check=True,
            )
            reference_backend = base / "foreign-refs"
            foreign_ref = reference_backend / head_ref
            foreign_ref.parent.mkdir(parents=True)
            (reference_backend / "HEAD").write_text(
                f"ref: {head_ref}\n", encoding="utf-8"
            )
            foreign_ref.write_text(f"{foreign_head}\n", encoding="utf-8")
            hostile_env = os.environ.copy()
            hostile_env["GIT_REFERENCE_BACKEND"] = f"files://{reference_backend}"
            out = base / "cut"

            result = self.run_cut(source, out, env=hostile_env)
            marker = json.loads((out / ".oss-cut-marker").read_text(encoding="utf-8"))

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(marker["source_head"], intended_head)

    def test_git_subprocess_environment_drops_repository_routing_controls(self) -> None:
        routing_controls = {
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_CEILING_DIRECTORIES",
            "GIT_COMMON_DIR",
            "GIT_CONFIG",
            "GIT_CONFIG_COUNT",
            "GIT_CONFIG_GLOBAL",
            "GIT_CONFIG_KEY_0",
            "GIT_CONFIG_NOSYSTEM",
            "GIT_CONFIG_PARAMETERS",
            "GIT_CONFIG_SYSTEM",
            "GIT_CONFIG_VALUE_0",
            "GIT_DIR",
            "GIT_DISCOVERY_ACROSS_FILESYSTEM",
            "GIT_EXEC_PATH",
            "GIT_GRAFT_FILE",
            "GIT_IMPLICIT_WORK_TREE",
            "GIT_INDEX_FILE",
            "GIT_NAMESPACE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_PREFIX",
            "GIT_QUARANTINE_PATH",
            "GIT_REFERENCE_BACKEND",
            "GIT_REPLACE_REF_BASE",
            "GIT_SHALLOW_FILE",
            "GIT_WORK_TREE",
        }
        hostile = {name: "attacker-controlled" for name in routing_controls}

        with mock.patch.dict(os.environ, hostile, clear=False):
            module = load_oss_cut()

        self.assertTrue(routing_controls.isdisjoint(module.GIT_ENV))
        self.assertEqual(module.GIT_ENV["GIT_NO_REPLACE_OBJECTS"], "1")

    def test_git_subprocess_environment_drops_unknown_git_controls(self) -> None:
        with mock.patch.dict(
            os.environ,
            {
                "GIT_FUTURE_REFERENCE_ROUTER": "attacker-controlled",
                "GIT_TRACE": "attacker-controlled",
            },
            clear=False,
        ):
            module = load_oss_cut()

        retained_git_variables = {
            name for name in module.GIT_ENV if name.startswith("GIT_")
        }
        self.assertEqual(retained_git_variables, {"GIT_NO_REPLACE_OBJECTS"})

    def test_replace_object_cannot_diverge_recorded_head_and_exported_bytes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            original_head = subprocess.run(
                ["git", "-C", str(source), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            (source / "README.md").write_text("# replacement tree\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(source), "add", "README.md"], check=True)
            self.commit(source)
            replacement_head = subprocess.run(
                ["git", "-C", str(source), "rev-parse", "HEAD"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            subprocess.run(
                ["git", "-C", str(source), "reset", "--hard", "-q", original_head],
                check=True,
            )
            subprocess.run(
                ["git", "-C", str(source), "replace", original_head, replacement_head],
                check=True,
            )
            subprocess.run(
                ["git", "-C", str(source), "reset", "--hard", "-q", "HEAD"],
                check=True,
            )
            self.assertEqual(
                subprocess.run(
                    ["git", "-C", str(source), "rev-parse", "HEAD"],
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout.strip(),
                original_head,
            )
            self.assertEqual(
                subprocess.run(
                    ["git", "-C", str(source), "status", "--porcelain=v1"],
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout,
                "",
            )

            out = base / "cut"
            result = self.run_cut(source, out)

            marker_exists = (out / ".oss-cut-marker").exists()
            exported_readme = (out / "README.md").read_bytes() if marker_exists else None

            no_replace_env = os.environ.copy()
            no_replace_env["GIT_NO_REPLACE_OBJECTS"] = "1"
            subprocess.run(
                ["git", "-C", str(source), "reset", "--hard", "-q", original_head],
                check=True,
                env=no_replace_env,
            )
            canonical_out = base / "canonical-cut"
            canonical_result = self.run_cut(source, canonical_out)
            canonical_marker = json.loads(
                (canonical_out / ".oss-cut-marker").read_text(encoding="utf-8")
            )
            canonical_readme = (canonical_out / "README.md").read_bytes()

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(marker_exists)
        self.assertNotEqual(exported_readme, b"# replacement tree\n")
        self.assertEqual(
            canonical_result.returncode,
            0,
            canonical_result.stdout + canonical_result.stderr,
        )
        self.assertEqual(canonical_marker["source_head"], original_head)
        self.assertEqual(canonical_readme, b"# Fixture\n")

    def test_external_markdown_symlink_is_rejected(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            external = base / "external.md"
            external.write_text("# External\n", encoding="utf-8")
            link = source / "docs" / "external.md"
            link.symlink_to(external)
            subprocess.run(
                ["git", "-C", str(source), "add", "docs/external.md"], check=True
            )
            self.commit(source)
            out = base / "cut"
            result = self.run_cut(source, out)

            copied_link = out / "docs" / "external.md"
            copied = os.path.lexists(copied_link)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(copied, "external symlink must not be copied into a cut")

    def test_known_skill_symlink_alias_is_excluded(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            skill = source / "skills" / "adding-a-capture-harness" / "SKILL.md"
            skill.parent.mkdir(parents=True)
            skill.write_text("# Skill\n", encoding="utf-8")
            alias = source / ".claude" / "skills" / "adding-a-capture-harness"
            alias.parent.mkdir(parents=True)
            alias.symlink_to("../../skills/adding-a-capture-harness")
            subprocess.run(
                [
                    "git", "-C", str(source), "add",
                    "skills/adding-a-capture-harness/SKILL.md",
                    ".claude/skills/adding-a-capture-harness",
                ],
                check=True,
            )
            self.commit(source)
            out = base / "cut"
            result = self.run_cut(source, out)
            alias_copied = os.path.lexists(
                out / ".claude" / "skills" / "adding-a-capture-harness"
            )
            skill_copied = (out / "skills" / "adding-a-capture-harness" / "SKILL.md").is_file()

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertFalse(alias_copied)
        self.assertTrue(skill_copied)

    def test_checker_source_must_be_regular_and_authoritative(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)

            linked = self.make_source(base / "linked")
            external = base / "exit-zero.py"
            external.write_text("raise SystemExit(0)\n", encoding="utf-8")
            checker = linked / "scripts" / "check-links.py"
            checker.unlink()
            checker.symlink_to(external)
            subprocess.run(
                ["git", "-C", str(linked), "add", "scripts/check-links.py"], check=True
            )
            self.commit(linked)
            linked_result = self.run_cut(linked, base / "linked-cut")

            substituted = self.make_source(
                base / "substituted", checker="raise SystemExit(0)\n"
            )
            substituted_result = self.run_cut(substituted, base / "substituted-cut")

        self.assertEqual(
            linked_result.returncode, 1, linked_result.stdout + linked_result.stderr
        )
        self.assertEqual(
            substituted_result.returncode,
            2,
            substituted_result.stdout + substituted_result.stderr,
        )
        self.assertNotIn("check-links:", substituted_result.stdout)

    def test_replace_rejects_a_forged_marker_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)

            legitimate = base / "legitimate-cut"
            first = self.run_cut(source, legitimate)
            replaced = self.run_cut(source, legitimate, "--replace")

            forged = base / "forged-cut"
            forged.mkdir()
            (forged / ".oss-cut-marker").write_text(
                "created by scripts/oss-cut.py\n", encoding="utf-8"
            )
            victim = forged / "do-not-delete.txt"
            victim.write_text("owned by someone else\n", encoding="utf-8")
            forged_result = self.run_cut(source, forged, "--replace")
            victim_exists = victim.exists()
            victim_contents = victim.read_text(encoding="utf-8") if victim_exists else None

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(replaced.returncode, 0, replaced.stdout + replaced.stderr)
        self.assertEqual(
            forged_result.returncode, 1, forged_result.stdout + forged_result.stderr
        )
        self.assertTrue(victim_exists, "forged marker must not authorize deletion")
        self.assertEqual(victim_contents, "owned by someone else\n")

    def test_replace_rejects_chmod_only_output_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            readme = out / "README.md"
            readme.chmod(0o755)

            replaced = self.run_cut(source, out, "--replace")
            retained_mode = readme.stat().st_mode & 0o777

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(replaced.returncode, 1, replaced.stdout + replaced.stderr)
        self.assertEqual(retained_mode, 0o755)

    def test_replace_rejects_tree_oid_marker(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            tree_oid = subprocess.run(
                ["git", "-C", str(source), "rev-parse", "HEAD^{tree}"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            marker_path = out / ".oss-cut-marker"
            marker = json.loads(marker_path.read_text(encoding="utf-8"))
            marker["source_head"] = tree_oid
            marker_path.write_text(
                json.dumps(marker, sort_keys=True) + "\n", encoding="utf-8"
            )

            replaced = self.run_cut(source, out, "--replace")
            retained_marker = json.loads(marker_path.read_text(encoding="utf-8"))
            retained_readme = (out / "README.md").is_file()

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(replaced.returncode, 1, replaced.stdout + replaced.stderr)
        self.assertEqual(retained_marker["source_head"], tree_oid)
        self.assertTrue(retained_readme)

    def test_marker_is_source_neutral(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            result = self.run_cut(source, out)
            marker_text = (out / ".oss-cut-marker").read_text(encoding="utf-8")
            marker = json.loads(marker_text)

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn(str(source), marker_text)
        self.assertNotIn("source", marker)
        self.assertIn("source_head", marker)

    def test_replace_rejects_semantically_equal_but_byte_changed_marker(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            marker_path = out / ".oss-cut-marker"
            canonical = marker_path.read_bytes()
            marker = json.loads(canonical)
            reformatted = (json.dumps(marker, indent=2) + "\n").encode("utf-8")
            marker_path.write_bytes(reformatted)

            replaced = self.run_cut(source, out, "--replace")
            retained = marker_path.read_bytes()

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertNotEqual(reformatted, canonical)
        self.assertEqual(json.loads(reformatted), json.loads(canonical))
        self.assertEqual(replaced.returncode, 1, replaced.stdout + replaced.stderr)
        self.assertEqual(retained, reformatted)

    def test_blob_digest_closes_pipe_and_reaps_process(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            source = self.make_source(Path(temporary))
            object_id = subprocess.run(
                ["git", "-C", str(source), "rev-parse", "HEAD:README.md"],
                check=True,
                capture_output=True,
                text=True,
            ).stdout.strip()
            module = load_oss_cut()
            real_popen = subprocess.Popen
            processes = []

            def capture_process(*args, **kwargs):
                process = real_popen(*args, **kwargs)
                processes.append(process)
                return process

            with mock.patch.object(
                module.subprocess, "Popen", side_effect=capture_process
            ):
                digest = module.blob_digest(source, object_id)

        self.assertRegex(digest or "", r"^[0-9a-f]{64}$")
        self.assertEqual(len(processes), 1)
        self.assertIsNotNone(processes[0].stdout)
        self.assertTrue(processes[0].stdout.closed)
        self.assertIsNotNone(processes[0].returncode)

    def test_unlisted_candidate_is_not_private_but_listed_names_remain(self) -> None:
        module = load_oss_cut()
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "README.md").write_text(
                "NeutralCandidate work" + "trunk coworker_" + "agent\n", encoding="utf-8"
            )
            result = module.scan(root, ["README.md"])

        self.assertEqual(result["home"]["private names"]["(root)"], 2)

    def test_existing_non_link_status_precedence_is_preserved(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)

            missing_source = self.make_source(
                base / "missing-case", missing={"LICENSE"}
            )
            module = load_oss_cut()
            missing_argv = [
                str(OSS_CUT), "--source", str(missing_source),
                "--out", str(base / "missing-cut"),
            ]
            with mock.patch.object(sys, "argv", missing_argv), mock.patch.object(
                module, "run_link_checker", return_value=9
            ), redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                missing_status = module.main()
            self.assertEqual(
                missing_status,
                3,
            )

            strict_source = self.make_source(
                base / "strict-case",
                readme="# Fixture\n/" + "Users/alice/private\n",
            )
            strict_argv = [
                str(OSS_CUT), "--source", str(strict_source),
                "--out", str(base / "strict-cut"),
            ]
            with mock.patch.object(sys, "argv", strict_argv), mock.patch.object(
                module, "run_link_checker", return_value=9
            ), redirect_stdout(io.StringIO()), redirect_stderr(io.StringIO()):
                strict_status = module.main()
            self.assertEqual(
                strict_status,
                2,
            )

            refused = self.run_cut(strict_source, strict_source / "nested-cut")
            self.assertEqual(refused.returncode, 1, refused.stdout + refused.stderr)

    def test_missing_required_status_survives_gate_failure(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base, missing={"LICENSE"})
            module = load_oss_cut()
            argv = [
                str(OSS_CUT),
                "--source",
                str(source),
                "--out",
                str(base / "cut"),
                "--gate",
                "smoke",
            ]
            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module, "run_link_checker", return_value=0
            ), mock.patch.object(module, "run_gate", return_value=7) as gate, \
                    redirect_stdout(output), redirect_stderr(output):
                status = module.main()

        self.assertEqual(status, 3, output.getvalue())
        gate.assert_called_once()

    def test_full_gate_runs_on_an_isolated_copy_and_publishes_the_untouched_cut(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            platform_placeholder = source / "crates/goalcup-platform/README.md"
            platform_placeholder.parent.mkdir(parents=True)
            platform_placeholder.write_text("# platform fixture\n", encoding="utf-8")
            subprocess.run(
                ["git", "-C", str(source), "add", str(platform_placeholder.relative_to(source))],
                check=True,
            )
            self.commit(source)
            fake_bin = base / "bin"
            fake_bin.mkdir()
            cargo_cwds = base / "cargo-cwds.txt"
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                "pwd >> \"$FAKE_CARGO_CWDS\"\n"
                "if [ \"${1-}\" = test ]; then\n"
                "  mkdir -p crates/goalcup-platform/target/readcap-unix-tests\n"
                "  mkdir -p crates/goalcup-platform/target/tracecap-unit-tests\n"
                "  mkdir -p crates/goalcup-platform/target/unix-tracecap-native-tests\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            env["FAKE_CARGO_CWDS"] = str(cargo_cwds)
            out = base / "cut"

            result = self.run_cut(source, out, "--gate", "full", env=env)
            observed_cwds = cargo_cwds.read_text(encoding="utf-8").splitlines()
            gate_residue = list(base.glob(".cut.oss-cut-gate-*"))

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertEqual(len(observed_cwds), 4)
        self.assertTrue(all(Path(cwd) != out for cwd in observed_cwds))
        self.assertFalse((out / "crates/goalcup-platform/target").exists())
        self.assertEqual(gate_residue, [])
        self.assertIn("gate (full): PASS", result.stdout)

    def test_gate_drops_ambient_git_controls_and_blocks_ancestor_discovery(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            subprocess.run(["git", "init", "-q", str(base)], check=True)
            source = self.make_source(base / "nested")
            foreign = self.make_source(base / "foreign")
            fake_bin = base / "bin"
            fake_bin.mkdir()
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                "env | grep '^GIT_' > \"$FAKE_GIT_ENV\" || true\n"
                "if grep -Ev '^GIT_(NO_REPLACE_OBJECTS|CEILING_DIRECTORIES)=' "
                "\"$FAKE_GIT_ENV\" | grep -q .; then exit 41; fi\n"
                "if git rev-parse --show-toplevel >/dev/null 2>&1; then exit 42; fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            observed_git_env = base / "cargo-git-env.txt"
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            env["FAKE_GIT_ENV"] = str(observed_git_env)
            env["GIT_DIR"] = str(foreign / ".git")
            env["GIT_WORK_TREE"] = str(source)
            env["GIT_CONFIG_COUNT"] = "1"
            env["GIT_CONFIG_KEY_0"] = "core.bare"
            env["GIT_CONFIG_VALUE_0"] = "true"
            out = base / "cut"

            result = self.run_cut(source, out, "--gate", "full", env=env)
            observed = observed_git_env.read_text(encoding="utf-8")

        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertNotIn("GIT_DIR=", observed)
        self.assertNotIn("GIT_WORK_TREE=", observed)
        self.assertNotIn("GIT_CONFIG_COUNT=", observed)
        self.assertIn("GIT_NO_REPLACE_OBJECTS=1", observed)
        self.assertIn("GIT_CEILING_DIRECTORIES=", observed)

    def test_gate_copy_root_swap_is_refused_without_touching_the_swapped_tree(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            platform_placeholder = source / "crates/goalcup-platform/README.md"
            platform_placeholder.parent.mkdir(parents=True)
            platform_placeholder.write_text("# platform fixture\n", encoding="utf-8")
            subprocess.run(
                ["git", "-C", str(source), "add", str(platform_placeholder.relative_to(source))],
                check=True,
            )
            self.commit(source)
            out = base / "cut"
            first = self.run_cut(source, out)
            module = load_oss_cut()
            real_copytree = module.shutil.copytree
            swapped_tree: Path | None = None

            def swap_after_copy(src, dst, *args, **kwargs):
                nonlocal swapped_tree
                with mock.patch.object(module.shutil, "copytree", real_copytree):
                    result = real_copytree(src, dst, *args, **kwargs)
                destination = Path.cwd() if Path(dst) == Path(".") else Path(dst)
                displaced = destination.with_name("displaced-tree")
                destination.rename(displaced)
                with mock.patch.object(module.shutil, "copytree", real_copytree):
                    real_copytree(src, destination)
                (destination / "victim-sentinel").write_text(
                    "preserve replacement\n", encoding="utf-8"
                )
                target = destination / "crates/goalcup-platform/target"
                for child in (
                    "readcap-unix-tests",
                    "tracecap-unit-tests",
                    "unix-tracecap-native-tests",
                ):
                    (target / child).mkdir(parents=True, mode=0o700)
                target.chmod(0o700)
                swapped_tree = destination
                return result

            output = io.StringIO()
            with mock.patch.object(
                module.shutil, "copytree", side_effect=swap_after_copy
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.run_gate(out, "full", [], source)
            self.assertIsNotNone(swapped_tree)
            assert swapped_tree is not None
            swapped_survived = swapped_tree.is_dir()
            swapped_residue_survived = (
                swapped_tree / "crates/goalcup-platform/target/readcap-unix-tests"
            ).is_dir()
            swapped_sentinel = (swapped_tree / "victim-sentinel").read_text(
                encoding="utf-8"
            )

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(status, 1, output.getvalue())
        self.assertTrue(swapped_survived, output.getvalue())
        self.assertTrue(swapped_residue_survived, output.getvalue())
        self.assertEqual(swapped_sentinel, "preserve replacement\n", output.getvalue())
        self.assertIn("gate copy identity changed during copy", output.getvalue())

    @unittest.skipUnless(hasattr(os, "symlink"), "symlink support required")
    def test_gate_copy_root_swap_cannot_redirect_writes_to_a_symlink_victim(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            module = load_oss_cut()
            real_copytree = module.shutil.copytree
            victim = base / "external-victim"
            victim.mkdir()
            (victim / "sentinel").write_text("preserve me\n", encoding="utf-8")
            victim_before = self.tree_snapshot(victim)

            def swap_before_copy(src, dst, *args, **kwargs):
                destination = Path(dst)
                root = Path.cwd() if destination == Path(".") else destination
                root.rename(root.with_name("displaced-tree"))
                root.symlink_to(victim, target_is_directory=True)
                with mock.patch.object(module.shutil, "copytree", real_copytree):
                    return real_copytree(src, dst, *args, **kwargs)

            output = io.StringIO()
            with mock.patch.object(
                module.shutil, "copytree", side_effect=swap_before_copy
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.run_gate(out, "full", [], source)
            victim_after = self.tree_snapshot(victim)

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(victim_after, victim_before, output.getvalue())

    @unittest.skipUnless(hasattr(os, "symlink"), "symlink support required")
    def test_gate_runtime_root_swap_cannot_redirect_later_cargo_steps(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            fake_bin = base / "bin"
            fake_bin.mkdir()
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                "count=$(cat \"$FAKE_STATE\" 2>/dev/null || printf 0)\n"
                "count=$((count + 1))\n"
                "printf '%s\\n' \"$count\" > \"$FAKE_STATE\"\n"
                "pwd -P >> \"$FAKE_CWDS\"\n"
                "if [ \"$count\" -eq 1 ]; then\n"
                "  root=$(pwd -P)\n"
                "  mv \"$root\" \"$root.displaced\"\n"
                "  ln -s \"$FAKE_VICTIM\" \"$root\"\n"
                "else\n"
                "  printf 'touched by later cargo step\\n' > touched-by-later-step\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            state = base / "cargo-count.txt"
            cargo_cwds = base / "cargo-cwds.txt"
            victim = base / "external-victim"
            victim.mkdir()
            (victim / "sentinel").write_text("preserve me\n", encoding="utf-8")
            victim_before = self.tree_snapshot(victim)
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            env["FAKE_STATE"] = str(state)
            env["FAKE_CWDS"] = str(cargo_cwds)
            env["FAKE_VICTIM"] = str(victim)
            out = base / "cut"

            result = self.run_cut(source, out, "--gate", "full", env=env)
            observed_cwds = cargo_cwds.read_text(encoding="utf-8").splitlines()
            victim_after = self.tree_snapshot(victim)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertEqual(len(observed_cwds), 4)
        self.assertEqual(victim_after, victim_before, result.stdout + result.stderr)
        self.assertEqual(len(set(observed_cwds[1:])), 1)
        self.assertTrue(observed_cwds[1].endswith(".displaced"))

    def test_gate_runtime_root_swap_is_refused_without_touching_the_swapped_tree(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            platform_placeholder = source / "crates/goalcup-platform/README.md"
            platform_placeholder.parent.mkdir(parents=True)
            platform_placeholder.write_text("# platform fixture\n", encoding="utf-8")
            subprocess.run(
                ["git", "-C", str(source), "add", str(platform_placeholder.relative_to(source))],
                check=True,
            )
            self.commit(source)
            out = base / "cut"
            first = self.run_cut(source, out)
            module = load_oss_cut()
            real_run = module.subprocess.run
            real_copytree = module.shutil.copytree
            swapped_tree: Path | None = None

            def swap_on_test(args, *positional, **keywords):
                nonlocal swapped_tree
                if args[0] == "cargo":
                    if args[1] == "test":
                        destination = Path(keywords.get("cwd") or Path.cwd())
                        destination.rename(destination.with_name("displaced-tree"))
                        real_copytree(out, destination)
                        (destination / "victim-sentinel").write_text(
                            "preserve replacement\n", encoding="utf-8"
                        )
                        target = destination / "crates/goalcup-platform/target"
                        for child in (
                            "readcap-unix-tests",
                            "tracecap-unit-tests",
                            "unix-tracecap-native-tests",
                        ):
                            (target / child).mkdir(parents=True, mode=0o700)
                        target.chmod(0o700)
                        swapped_tree = destination
                    return subprocess.CompletedProcess(args, 0)
                return real_run(args, *positional, **keywords)

            output = io.StringIO()
            with mock.patch.object(
                module.subprocess, "run", side_effect=swap_on_test
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.run_gate(out, "full", [], source)
            self.assertIsNotNone(swapped_tree)
            assert swapped_tree is not None
            swapped_residue_survived = (
                swapped_tree / "crates/goalcup-platform/target/readcap-unix-tests"
            ).is_dir()
            swapped_sentinel = (swapped_tree / "victim-sentinel").read_text(
                encoding="utf-8"
            )

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(status, 1, output.getvalue())
        self.assertTrue(swapped_residue_survived, output.getvalue())
        self.assertEqual(swapped_sentinel, "preserve replacement\n", output.getvalue())
        self.assertIn("gate copy identity changed after gate", output.getvalue())

    def test_gate_side_effect_on_the_candidate_is_preserved_and_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            module = load_oss_cut()
            real_run = module.subprocess.run
            mutation_reached = False

            def cargo_mutates_candidate(args, *positional, **keywords):
                nonlocal mutation_reached
                if args[0] == "cargo":
                    if args[1] == "test":
                        mutation_reached = True
                        (out / "README.md").write_text(
                            "# changed sideways during gate\n", encoding="utf-8"
                        )
                    return subprocess.CompletedProcess(args, 0)
                return real_run(args, *positional, **keywords)

            output = io.StringIO()
            with mock.patch.object(
                module.subprocess, "run", side_effect=cargo_mutates_candidate
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.run_gate(out, "full", [], source)
            retained = (out / "README.md").read_text(encoding="utf-8")
            gate_residue = list(base.glob(".cut.oss-cut-gate-*"))

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertTrue(mutation_reached)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(retained, "# changed sideways during gate\n")
        self.assertEqual(gate_residue, [])

    def test_gate_copy_tracked_mutation_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            fake_bin = base / "bin"
            fake_bin.mkdir()
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                "if [ \"${1-}\" = test ]; then\n"
                "  printf '# changed by test\\n' > README.md\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            out = base / "cut"

            result = self.run_cut(source, out, "--gate", "full", env=env)
            gate_residue = list(
                base.glob(".cut.oss-cut-stage-*/.candidate.oss-cut-gate-*")
            )
            retained_gate_readmes = [
                (workspace / "tree/README.md").read_text(encoding="utf-8")
                for workspace in gate_residue
            ]

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(out.exists())
        self.assertEqual(len(gate_residue), 1)
        self.assertEqual(retained_gate_readmes, ["# changed by test\n"])
        self.assertIn("gate copy changed or became unverifiable after gate", result.stderr)

    def test_failed_gate_copy_verification_preserves_unexpected_content(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            module = load_oss_cut()
            real_run = module.subprocess.run

            def cargo_adds_unexpected_content(args, *positional, **keywords):
                if args[0] == "cargo":
                    if args[1] == "test":
                        sentinel = Path.cwd() / "unexpected/nested/sentinel"
                        sentinel.parent.mkdir(parents=True)
                        sentinel.write_text("preserve me\n", encoding="utf-8")
                    return subprocess.CompletedProcess(args, 0)
                return real_run(args, *positional, **keywords)

            output = io.StringIO()
            with mock.patch.object(
                module.subprocess, "run", side_effect=cargo_adds_unexpected_content
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.run_gate(out, "full", [], source)
            gate_workspaces = list(base.glob(".cut.oss-cut-gate-*"))
            retained = [
                path.read_text(encoding="utf-8")
                for workspace in gate_workspaces
                for path in workspace.glob("tree/unexpected/nested/sentinel")
            ]

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(len(gate_workspaces), 1, output.getvalue())
        self.assertEqual(retained, ["preserve me\n"], output.getvalue())
        self.assertIn("gate workspace preserved", output.getvalue())

    def test_gate_copy_unexpected_target_file_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            platform_placeholder = source / "crates/goalcup-platform/README.md"
            platform_placeholder.parent.mkdir(parents=True)
            platform_placeholder.write_text("# platform fixture\n", encoding="utf-8")
            subprocess.run(
                ["git", "-C", str(source), "add", str(platform_placeholder.relative_to(source))],
                check=True,
            )
            self.commit(source)
            fake_bin = base / "bin"
            fake_bin.mkdir()
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                "if [ \"${1-}\" = test ]; then\n"
                "  mkdir -p crates/goalcup-platform/target/readcap-unix-tests\n"
                "  mkdir -p crates/goalcup-platform/target/tracecap-unit-tests\n"
                "  mkdir -p crates/goalcup-platform/target/unix-tracecap-native-tests\n"
                "  printf 'unexpected\\n' > crates/goalcup-platform/target/unexpected.txt\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            out = base / "cut"

            result = self.run_cut(source, out, "--gate", "full", env=env)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(out.exists())
        self.assertIn("gate test residue is not exact", result.stderr)

    @unittest.skipUnless(hasattr(os, "symlink"), "symlink support required")
    def test_gate_copy_unexpected_target_symlink_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            platform_placeholder = source / "crates/goalcup-platform/README.md"
            platform_placeholder.parent.mkdir(parents=True)
            platform_placeholder.write_text("# platform fixture\n", encoding="utf-8")
            subprocess.run(
                ["git", "-C", str(source), "add", str(platform_placeholder.relative_to(source))],
                check=True,
            )
            self.commit(source)
            fake_bin = base / "bin"
            fake_bin.mkdir()
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                "if [ \"${1-}\" = test ]; then\n"
                "  mkdir -p crates/goalcup-platform/target/readcap-unix-tests\n"
                "  mkdir -p crates/goalcup-platform/target/tracecap-unit-tests\n"
                "  mkdir -p crates/goalcup-platform/target/unix-tracecap-native-tests\n"
                "  ln -s /tmp crates/goalcup-platform/target/unexpected-link\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            out = base / "cut"

            result = self.run_cut(source, out, "--gate", "full", env=env)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(out.exists())
        self.assertIn("gate test residue is not exact", result.stderr)

    def test_gate_copy_expected_target_with_wrong_mode_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            platform_placeholder = source / "crates/goalcup-platform/README.md"
            platform_placeholder.parent.mkdir(parents=True)
            platform_placeholder.write_text("# platform fixture\n", encoding="utf-8")
            subprocess.run(
                ["git", "-C", str(source), "add", str(platform_placeholder.relative_to(source))],
                check=True,
            )
            self.commit(source)
            fake_bin = base / "bin"
            fake_bin.mkdir()
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                "if [ \"${1-}\" = test ]; then\n"
                "  mkdir -p crates/goalcup-platform/target/readcap-unix-tests\n"
                "  mkdir -p crates/goalcup-platform/target/tracecap-unit-tests\n"
                "  mkdir -p crates/goalcup-platform/target/unix-tracecap-native-tests\n"
                "  chmod 0755 crates/goalcup-platform/target/readcap-unix-tests\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            out = base / "cut"

            result = self.run_cut(source, out, "--gate", "full", env=env)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(out.exists())
        self.assertIn("gate test residue is not exact", result.stderr)

    def test_gate_copy_partial_expected_target_residue_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            platform_placeholder = source / "crates/goalcup-platform/README.md"
            platform_placeholder.parent.mkdir(parents=True)
            platform_placeholder.write_text("# platform fixture\n", encoding="utf-8")
            subprocess.run(
                ["git", "-C", str(source), "add", str(platform_placeholder.relative_to(source))],
                check=True,
            )
            self.commit(source)
            fake_bin = base / "bin"
            fake_bin.mkdir()
            fake_cargo = fake_bin / "cargo"
            fake_cargo.write_text(
                "#!/bin/sh\n"
                "if [ \"${1-}\" = test ]; then\n"
                "  mkdir -p crates/goalcup-platform/target/readcap-unix-tests\n"
                "  mkdir -p crates/goalcup-platform/target/tracecap-unit-tests\n"
                "fi\n"
                "exit 0\n",
                encoding="utf-8",
            )
            fake_cargo.chmod(0o755)
            env = os.environ.copy()
            env["PATH"] = f"{fake_bin}{os.pathsep}{env['PATH']}"
            out = base / "cut"

            result = self.run_cut(source, out, "--gate", "full", env=env)

        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertFalse(out.exists())
        self.assertIn("gate test residue is not exact", result.stderr)

    def test_gate_copy_failure_is_closed_and_preserves_the_private_workspace(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            before = self.tree_snapshot(out)
            module = load_oss_cut()
            output = io.StringIO()
            partial_written = False

            def partial_copy_then_fail(src, dst, *args, **kwargs):
                nonlocal partial_written
                (Path(dst) / "partial-copy").write_text("partial\n", encoding="utf-8")
                partial_written = True
                raise OSError(errno.ENOSPC, "injected")

            with mock.patch.object(
                module.shutil, "copytree", side_effect=partial_copy_then_fail
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.run_gate(out, "full", [], source)
            after = self.tree_snapshot(out)
            gate_residue = list(base.glob(".cut.oss-cut-gate-*"))
            partial_retained = [
                (workspace / "tree/partial-copy").read_text(encoding="utf-8")
                for workspace in gate_residue
            ]

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertTrue(partial_written)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(after, before)
        self.assertEqual(len(gate_residue), 1, output.getvalue())
        self.assertEqual(partial_retained, ["partial\n"])
        self.assertIn("gate workspace preserved", output.getvalue())

    def test_replace_rejects_directory_mode_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            scripts = out / "scripts"
            scripts.chmod(0o700)

            replaced = self.run_cut(source, out, "--replace")
            retained_mode = scripts.stat().st_mode & 0o777

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(replaced.returncode, 1, replaced.stdout + replaced.stderr)
        self.assertEqual(retained_mode, 0o700)

    def test_replace_rejects_extra_empty_directory(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            extra = out / "extra-empty"
            extra.mkdir()

            replaced = self.run_cut(source, out, "--replace")
            extra_survived = extra.is_dir()

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(replaced.returncode, 1, replaced.stdout + replaced.stderr)
        self.assertTrue(extra_survived)

    def test_replace_rejects_unreadable_extra_directory_without_partial_deletion(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            readme_before = (out / "README.md").read_bytes()
            extra = out / "extra-unreadable"
            extra.mkdir()
            sentinel = extra / "sentinel"
            sentinel.write_text("preserve me\n", encoding="utf-8")
            extra.chmod(0o000)
            try:
                replaced = self.run_cut(source, out, "--replace")
                readme_survived = (
                    (out / "README.md").is_file()
                    and (out / "README.md").read_bytes() == readme_before
                )
                marker_survived = (out / ".oss-cut-marker").is_file()
            finally:
                if extra.exists():
                    extra.chmod(0o700)
            sentinel_survived = (
                sentinel.is_file()
                and sentinel.read_text(encoding="utf-8") == "preserve me\n"
            )

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(replaced.returncode, 1, replaced.stdout + replaced.stderr)
        self.assertTrue(readme_survived)
        self.assertTrue(marker_survived)
        self.assertTrue(sentinel_survived)

    def test_replace_rejects_marker_mode_mutation(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            marker = out / ".oss-cut-marker"
            marker.chmod(0o777)

            replaced = self.run_cut(source, out, "--replace")
            retained_mode = marker.stat().st_mode & 0o777

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(replaced.returncode, 1, replaced.stdout + replaced.stderr)
        self.assertEqual(retained_mode, 0o777)

    def test_replace_rejects_directory_mode_even_when_marker_is_edited(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            scripts = out / "scripts"
            scripts.chmod(0o700)
            marker_path = out / ".oss-cut-marker"
            marker = json.loads(marker_path.read_text(encoding="utf-8"))
            marker["inventory"]["scripts"]["mode"] = 0o700
            marker_path.write_text(json.dumps(marker, sort_keys=True) + "\n", encoding="utf-8")

            replaced = self.run_cut(source, out, "--replace")
            retained_mode = scripts.stat().st_mode & 0o777

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(replaced.returncode, 1, replaced.stdout + replaced.stderr)
        self.assertEqual(retained_mode, 0o700)

    def test_replace_copy_failure_preserves_exact_old_cut(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            before = self.tree_snapshot(out)
            module = load_oss_cut()
            argv = [str(OSS_CUT), "--source", str(source), "--out", str(out), "--replace"]

            def fail_copy(_source, candidate, _kept, _entries):
                (candidate / "partial-copy").write_bytes(b"partial")
                raise OSError(errno.ENOSPC, "injected copy exhaustion")

            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module, "copy_tree", side_effect=fail_copy
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()
            after = self.tree_snapshot(out)

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(after, before)

    def test_replace_partial_swap_failure_restores_exact_old_cut(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            before = self.tree_snapshot(out)
            (source / "README.md").write_text("# replacement\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(source), "add", "README.md"], check=True)
            self.commit(source)
            module = load_oss_cut()
            argv = [str(OSS_CUT), "--source", str(source), "--out", str(out), "--replace"]
            real_rename = module.os.rename
            rename_calls = 0

            def fail_candidate_publish(source_path, destination_path):
                nonlocal rename_calls
                rename_calls += 1
                if rename_calls == 2:
                    raise OSError(errno.EIO, "injected candidate publication failure")
                return real_rename(source_path, destination_path)

            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module.os, "rename", side_effect=fail_candidate_publish
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()
            after = self.tree_snapshot(out)

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(status, 1, output.getvalue())
        self.assertGreaterEqual(rename_calls, 3)
        self.assertEqual(after, before)

    def test_replace_backup_removal_failure_restores_exact_old_cut(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            (source / "README.md").write_text("# replacement one\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(source), "add", "README.md"], check=True)
            self.commit(source)
            second = self.run_cut(source, out, "--replace")
            before = self.tree_snapshot(out)
            (source / "README.md").write_text("# replacement two\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(source), "add", "README.md"], check=True)
            self.commit(source)
            module = load_oss_cut()
            argv = [str(OSS_CUT), "--source", str(source), "--out", str(out), "--replace"]
            real_remove_contents = module.remove_directory_contents
            removal_failure_reached = False

            def fail_backup_removal(descriptor):
                nonlocal removal_failure_reached
                if not removal_failure_reached:
                    removal_failure_reached = True
                    with os.scandir(descriptor) as entries:
                        victim = next(item for item in entries if item.is_file(follow_symlinks=False))
                    os.unlink(victim.name, dir_fd=descriptor)
                    raise OSError(errno.EIO, "injected partial backup removal failure")
                return real_remove_contents(descriptor)

            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module, "remove_directory_contents", side_effect=fail_backup_removal
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()
            after = self.tree_snapshot(out)

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(second.returncode, 0, second.stdout + second.stderr)
        self.assertTrue(removal_failure_reached)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(after, before)

    def test_replace_invalid_candidate_preserves_exact_old_cut(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            before = self.tree_snapshot(out)
            (source / "README.md").write_text(
                "# private /" + "Users/alice/work\n", encoding="utf-8"
            )
            subprocess.run(["git", "-C", str(source), "add", "README.md"], check=True)
            self.commit(source)

            replaced = self.run_cut(source, out, "--replace")
            after = self.tree_snapshot(out)

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(replaced.returncode, 2, replaced.stdout + replaced.stderr)
        self.assertEqual(after, before)

    def test_failed_candidate_cleanup_never_deletes_path_swapped_victim(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            before = self.tree_snapshot(out)
            victim = base / "victim"
            victim.mkdir()
            sentinel = victim / "sentinel"
            sentinel.write_text("must survive\n", encoding="utf-8")
            module = load_oss_cut()
            argv = [str(OSS_CUT), "--source", str(source), "--out", str(out), "--replace"]
            staged_path = None

            def swap_then_fail(_source, candidate, _kept, _entries):
                nonlocal staged_path
                staged_path = candidate
                os.rename(candidate, candidate.with_name("original-candidate"))
                os.rename(victim, candidate)
                raise OSError(errno.ENOSPC, "injected copy exhaustion after path swap")

            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module, "copy_tree", side_effect=swap_then_fail
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()

            self.assertIsNotNone(staged_path)
            assert staged_path is not None
            victim_sentinel = staged_path / "sentinel"
            victim_survived = (
                victim_sentinel.read_text(encoding="utf-8")
                if victim_sentinel.is_file()
                else None
            )
            staging_parent_mode = staged_path.parent.stat().st_mode & 0o777
            staging_parent_is_private_child = staged_path.parent.parent == base.resolve()
            after = self.tree_snapshot(out)

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(victim_survived, "must survive\n")
        self.assertEqual(staging_parent_mode, 0o700)
        self.assertTrue(staging_parent_is_private_child)
        self.assertEqual(after, before)

    def test_cleanup_path_swap_never_deletes_victim_sentinel(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            target = base / "target"
            target.mkdir()
            (target / "disposable").write_text("discard me\n", encoding="utf-8")
            original = base / "original-target"
            victim = base / "victim"
            victim.mkdir()
            sentinel = victim / "sentinel"
            sentinel.write_text("must survive\n", encoding="utf-8")
            module = load_oss_cut()
            identity = module.directory_identity(target)
            self.assertIsNotNone(identity)
            real_directory_identity = module.directory_identity
            swap_injected = False

            def swap_after_identity_check(path):
                nonlocal swap_injected
                observed = real_directory_identity(path)
                if not swap_injected and Path(path) == target:
                    swap_injected = True
                    os.rename(target, original)
                    os.rename(victim, target)
                return observed

            output = io.StringIO()
            with mock.patch.object(
                module, "directory_identity", side_effect=swap_after_identity_check
            ), redirect_stdout(output), redirect_stderr(output):
                removed = module.remove_tree_if_identity(target, identity, "test target")
            sentinel_survived = target.is_dir() and (target / "sentinel").is_file()
            sentinel_contents = (
                (target / "sentinel").read_text(encoding="utf-8")
                if sentinel_survived else None
            )

        self.assertTrue(swap_injected)
        self.assertFalse(removed, output.getvalue())
        self.assertEqual(sentinel_contents, "must survive\n")

    def test_post_validation_candidate_mutation_is_not_published(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            before = self.tree_snapshot(out)
            module = load_oss_cut()
            argv = [str(OSS_CUT), "--source", str(source), "--out", str(out), "--replace"]
            real_validate = module.validate_candidate

            def mutate_after_validation(candidate, *args, **kwargs):
                status = real_validate(candidate, *args, **kwargs)
                self.assertEqual(status, 0)
                (candidate / "README.md").write_text(
                    "# invalid post-validation replacement\n", encoding="utf-8"
                )
                return status

            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module, "validate_candidate", side_effect=mutate_after_validation
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()
            after = self.tree_snapshot(out)

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(after, before)

    def test_live_cut_mutation_during_candidate_validation_is_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            (source / "README.md").write_text("# valid replacement\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(source), "add", "README.md"], check=True)
            self.commit(source)
            module = load_oss_cut()
            argv = [str(OSS_CUT), "--source", str(source), "--out", str(out), "--replace"]
            real_validate = module.validate_candidate

            def mutate_live_during_validation(candidate, *args, **kwargs):
                status = real_validate(candidate, *args, **kwargs)
                self.assertEqual(status, 0)
                (out / "README.md").write_text(
                    "# changed live cut during validation\n", encoding="utf-8"
                )
                return status

            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module, "validate_candidate", side_effect=mutate_live_during_validation
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()
            retained = (out / "README.md").read_text(encoding="utf-8")

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(retained, "# changed live cut during validation\n")

    def test_live_cut_mutation_during_backup_rename_is_preserved_and_refused(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            backup = base / ".cut.oss-cut-backup"
            first = self.run_cut(source, out)
            (source / "README.md").write_text("# valid replacement\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(source), "add", "README.md"], check=True)
            self.commit(source)
            module = load_oss_cut()
            argv = [str(OSS_CUT), "--source", str(source), "--out", str(out), "--replace"]
            real_rename = module.os.rename
            mutation_injected = False

            def mutate_during_backup_rename(source_path, destination_path):
                nonlocal mutation_injected
                if Path(source_path) == out.resolve() and Path(destination_path) == backup.resolve():
                    mutation_injected = True
                    (Path(source_path) / "README.md").write_text(
                        "# changed live cut during backup handoff\n", encoding="utf-8"
                    )
                return real_rename(source_path, destination_path)

            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module.os, "rename", side_effect=mutate_during_backup_rename
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()
            recoverable = (backup / "README.md").read_text(encoding="utf-8")

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertTrue(mutation_injected)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(recoverable, "# changed live cut during backup handoff\n")

    def test_mutation_inside_final_rename_is_rolled_back(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            first = self.run_cut(source, out)
            before = self.tree_snapshot(out)
            (source / "README.md").write_text("# valid replacement\n", encoding="utf-8")
            subprocess.run(["git", "-C", str(source), "add", "README.md"], check=True)
            self.commit(source)
            module = load_oss_cut()
            argv = [str(OSS_CUT), "--source", str(source), "--out", str(out), "--replace"]
            real_rename = module.os.rename
            mutation_injected = False

            def mutate_after_candidate_rename(source_path, destination_path):
                nonlocal mutation_injected
                result = real_rename(source_path, destination_path)
                if Path(source_path).name == "candidate" and Path(destination_path) == out.resolve():
                    mutation_injected = True
                    (Path(destination_path) / "README.md").write_text(
                        "# invalid race winner\n", encoding="utf-8"
                    )
                return result

            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module.os, "rename", side_effect=mutate_after_candidate_rename
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()
            after = self.tree_snapshot(out)

        self.assertEqual(first.returncode, 0, first.stdout + first.stderr)
        self.assertTrue(mutation_injected)
        self.assertEqual(status, 1, output.getvalue())
        self.assertEqual(after, before)

    def test_mutation_inside_first_publication_rename_is_not_left_live(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            out = base / "cut"
            module = load_oss_cut()
            argv = [str(OSS_CUT), "--source", str(source), "--out", str(out)]
            real_rename = module.os.rename
            mutation_injected = False

            def mutate_after_candidate_rename(source_path, destination_path):
                nonlocal mutation_injected
                result = real_rename(source_path, destination_path)
                if Path(source_path).name == "candidate" and Path(destination_path) == out.resolve():
                    mutation_injected = True
                    (Path(destination_path) / "README.md").write_text(
                        "# invalid race winner\n", encoding="utf-8"
                    )
                return result

            output = io.StringIO()
            with mock.patch.object(sys, "argv", argv), mock.patch.object(
                module.os, "rename", side_effect=mutate_after_candidate_rename
            ), redirect_stdout(output), redirect_stderr(output):
                status = module.main()

            live_output_survived = out.exists()
            preserved_invalid = list(base.glob(".cut.oss-cut-stage-*/invalid-published"))

        self.assertTrue(mutation_injected)
        self.assertEqual(status, 1, output.getvalue())
        self.assertFalse(live_output_survived)
        self.assertEqual(len(preserved_invalid), 1)

    def test_private_journey_dogfood_is_excluded(self) -> None:
        module = load_oss_cut()
        private = "docs/qualification/2026-09-17-journey-dogfood.md"
        public = "docs/qualification/public-evidence.md"

        kept, _ = module.select([private, public])

        self.assertNotIn(private, kept)
        self.assertIn(public, kept)

    def test_codex_app_fixture_generator_and_public_inputs_are_selected(self) -> None:
        module = load_oss_cut()
        generator = "spikes/codex-app/make_fixtures.py"
        redactor = "spikes/redact.py"
        fixture = "spikes/codex-app/fixtures/rollout-a9-foreign-01a0665e-excerpt.jsonl"
        sidecar = fixture + ".redactions.jsonl"
        private = {
            "spikes/codex-app/REPORT.md",
            "spikes/codex-app/out/raw-rollout.jsonl",
            "docs/superpowers/plans/private-cut-plan.md",
        }

        kept, _ = module.select([generator, redactor, fixture, sidecar, *private])

        self.assertEqual(set(kept), {generator, redactor, fixture, sidecar})

    def test_failure_campaign_workload_helper_is_selected_without_the_private_spike(self) -> None:
        module = load_oss_cut()
        helper = "spikes/m2-first-release/e02/early-load.py"
        unrelated_private_spike = "spikes/m2-first-release/e02/early-crosscheck.py"

        kept, _ = module.select([helper, unrelated_private_spike])

        self.assertEqual(kept, [helper])

    def test_binary_strict_file_home_path_fails_closed(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            (source / "scripts" / "leak.bin").write_bytes(
                b"\0/" + b"Users/alice/private\n"
            )
            subprocess.run(
                ["git", "-C", str(source), "add", "scripts/leak.bin"], check=True
            )
            self.commit(source)
            result = self.run_cut(source, base / "cut")

        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("scripts/leak.bin", result.stdout)

    def test_oss_cut_self_scan_detects_non_pattern_home_path(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            base = Path(temporary)
            source = self.make_source(base)
            (source / "scripts" / "oss-cut.py").write_text(
                "# leaked /" + "Users/alice/private\n", encoding="utf-8"
            )
            subprocess.run(
                ["git", "-C", str(source), "add", "scripts/oss-cut.py"], check=True
            )
            self.commit(source)
            result = self.run_cut(source, base / "cut")

        self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
        self.assertIn("scripts/oss-cut.py", result.stdout)


if __name__ == "__main__":
    unittest.main()
