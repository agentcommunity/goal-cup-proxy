#!/usr/bin/env python3
"""Compare two already available Goal Cup traces with one offline verifier."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Any


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def is_inside(path: Path, parent: Path) -> bool:
    try:
        path.relative_to(parent)
        return True
    except ValueError:
        return False


def output_is_disjoint(output: Path, traces: list[Path]) -> bool:
    return all(not is_inside(output, trace) and not is_inside(trace, output) for trace in traces)


def write_private(path: Path, data: bytes) -> None:
    fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, "wb") as stream:
        stream.write(data)


def inspect_report(path: Path) -> tuple[dict[str, Any], int, str, str]:
    """Validate NDJSON while hashing it without retaining observation lines."""
    whole_hash = hashlib.sha256()
    observation_hash = hashlib.sha256()
    observation_count = 0
    summary: dict[str, Any] | None = None
    max_observation_line = 2 * 1024 * 1024
    with path.open("rb") as stream:
        while True:
            raw = stream.readline(max_observation_line + 1)
            if not raw:
                break
            if len(raw) > max_observation_line or not raw.endswith(b"\n"):
                raise ValueError("oversize or unterminated NDJSON line")
            whole_hash.update(raw)
            try:
                record = json.loads(raw)
            except (json.JSONDecodeError, UnicodeDecodeError) as exc:
                raise ValueError("malformed NDJSON line") from exc
            if not isinstance(record, dict):
                raise ValueError("NDJSON entries must be JSON objects")
            if record.get("type") == "summary":
                if summary is not None or len(raw) > 64 * 1024:
                    raise ValueError("duplicate or oversize summary")
                summary = record
                continue
            if summary is not None or record.get("type") != "observation":
                raise ValueError("report must contain observations followed by one terminal summary")
            observation_count += 1
            observation_hash.update(raw)
    if summary is None:
        raise ValueError("report has no terminal summary")
    schema_version = summary.get("schema_version")
    exit_code = summary.get("exit_code")
    if (not isinstance(schema_version, int) or isinstance(schema_version, bool)
            or schema_version < 1 or not isinstance(summary.get("verifier_version"), str)
            or not summary["verifier_version"]):
        raise ValueError("summary lacks a valid schema or verifier version")
    if not isinstance(exit_code, int) or isinstance(exit_code, bool) or exit_code not in (0, 1, 2, 3):
        raise ValueError("summary lacks a valid exit code")
    if not all(isinstance(summary.get(field), dict) for field in
               ("analysis", "delivery", "capture", "input")):
        raise ValueError("summary lacks analysis, delivery, capture, or input results")
    capture = summary["capture"]
    if capture.get("integrity") not in ("valid", "incomplete", "invalid"):
        raise ValueError("summary capture integrity is missing or invalid")
    input_summary = summary["input"]
    if not all(field in input_summary for field in
               ("validated_records", "validated_bytes", "validated_head")):
        raise ValueError("summary input identity is incomplete")
    records, byte_count, head = (input_summary.get("validated_records"),
                                 input_summary.get("validated_bytes"),
                                 input_summary.get("validated_head"))
    if (not isinstance(records, int) or isinstance(records, bool) or records < 0
            or not isinstance(byte_count, int) or isinstance(byte_count, bool) or byte_count < 0
            or (head is not None and (not isinstance(head, str)
                                      or not re.fullmatch(r"[0-9a-f]{64}", head)))):
        raise ValueError("summary input identity is incomplete or malformed")
    return summary, observation_count, observation_hash.hexdigest(), whole_hash.hexdigest()


def command_for(verifier: Path, mode: str, trace: Path, report: Path | None,
                dev_key: str | None) -> list[str]:
    args = [str(verifier)]
    if mode == "integrity":
        args += ["--capture-integrity", str(trace)]
    else:
        args += ["--research-json", str(trace), "--output", str(report)]
    if dev_key is not None:
        args += ["--dev-key", dev_key]
    return args


def validate_integrity_report(report: Any) -> dict[str, Any]:
    """Check the identity fields needed to compare an integrity result."""
    if not isinstance(report, dict):
        raise ValueError("integrity report is not a JSON object")
    schema_version = report.get("schema_version")
    records, byte_count, head = (report.get("validated_records"),
                                 report.get("validated_bytes"),
                                 report.get("validated_head"))
    if (not isinstance(schema_version, int) or isinstance(schema_version, bool)
            or schema_version < 1 or report.get("integrity") not in
            ("valid", "incomplete", "invalid")):
        raise ValueError("integrity report lacks a schema version or verdict")
    if (not isinstance(records, int) or isinstance(records, bool) or records < 0
            or not isinstance(byte_count, int) or isinstance(byte_count, bool) or byte_count < 0
            or (head is None and records != 0)
            or (head is not None and (not isinstance(head, str)
                                      or not re.fullmatch(r"[0-9a-f]{64}", head)))):
        raise ValueError("integrity report lacks a usable validated identity")
    return report


def run_leg(name: str, trace: Path, out: Path, verifier: Path,
            dev_key: str | None) -> dict[str, Any]:
    leg: dict[str, Any] = {"trace": str(trace), "integrity_exit": None,
                           "research_exit": None, "integrity": None,
                           "analysis": None, "delivery": None, "research_summary": None,
                           "observation_count": None, "observation_sha256": None,
                           "report_sha256": None, "errors": []}
    for mode in ("integrity", "research"):
        report_path = out / f"{name}.research.ndjson" if mode == "research" else None
        try:
            completed = subprocess.run(
                command_for(verifier, mode, trace, report_path, dev_key),
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=False,
            )
        except OSError as exc:
            leg["errors"].append(f"{mode}: {type(exc).__name__}")
            return leg
        stdout_path = out / f"{name}.{mode}.stdout"
        stderr_path = out / f"{name}.{mode}.stderr"
        write_private(stdout_path, completed.stdout)
        write_private(stderr_path, completed.stderr)
        leg[f"{mode}_exit"] = completed.returncode
        if mode == "integrity":
            try:
                leg["integrity"] = validate_integrity_report(json.loads(completed.stdout))
            except (json.JSONDecodeError, UnicodeDecodeError, ValueError):
                leg["errors"].append("integrity report is malformed or lacks a usable identity")
        else:
            if not report_path.is_file():
                leg["errors"].append("research report was not created")
                continue
            try:
                # Ensure the verifier's report is private even if its umask differs.
                report_path.chmod(0o600)
                summary, obs_count, obs_hash, report_hash = inspect_report(report_path)
                if summary["exit_code"] != completed.returncode:
                    leg["errors"].append("research summary exit code disagrees with process exit")
                leg["analysis"] = summary.get("analysis")
                leg["delivery"] = summary.get("delivery")
                leg["research_summary"] = summary
                leg["observation_count"] = obs_count
                leg["observation_sha256"] = obs_hash
                leg["report_sha256"] = report_hash
            except (OSError, ValueError, StopIteration) as exc:
                leg["errors"].append(f"research report could not be parsed: {type(exc).__name__}")
    summary = leg.get("research_summary")
    if summary is not None:
        integrity_identity = normalized_chain(leg)
        report_identity = normalized_chain({"integrity": summary.get("input")})
        if integrity_identity is None or report_identity != integrity_identity:
            leg["errors"].append("research summary input identity disagrees with integrity report")
    return leg


def differing_paths(left: Any, right: Any, prefix: str = "") -> list[str]:
    if isinstance(left, dict) and isinstance(right, dict):
        result: list[str] = []
        for key in sorted(set(left) | set(right)):
            path = f"{prefix}.{key}" if prefix else key
            if key not in left or key not in right:
                result.append(path)
            else:
                result.extend(differing_paths(left[key], right[key], path))
        return result
    if isinstance(left, list) and isinstance(right, list):
        if len(left) != len(right):
            return [prefix]
        result = []
        for index, (a, b) in enumerate(zip(left, right)):
            result.extend(differing_paths(a, b, f"{prefix}[{index}]"))
        return result
    return [] if left == right else [prefix]


def normalized_chain(leg: dict[str, Any]) -> dict[str, Any] | None:
    report = leg.get("integrity")
    if not isinstance(report, dict):
        return None
    records, size, head = (report.get("validated_records"), report.get("validated_bytes"),
                          report.get("validated_head"))
    if (not isinstance(records, int) or isinstance(records, bool) or records < 0
            or not isinstance(size, int) or isinstance(size, bool) or size < 0
            or (head is None and records != 0)
            or (head is not None and (not isinstance(head, str)
                                      or not re.fullmatch(r"[0-9a-f]{64}", head)))):
        return None
    return {"validated_records": records, "validated_bytes": size, "validated_head": head}


def aggregate_exit(legs: dict[str, dict[str, Any]], chain_equal: bool,
                   observations_equal: bool) -> int:
    if any(leg["errors"] or leg["integrity"] is None or leg["research_summary"] is None
           for leg in legs.values()):
        return 3
    exits = [leg[key] for leg in legs.values() for key in ("integrity_exit", "research_exit")]
    if any(code == 3 or code not in (0, 1, 2) for code in exits):
        return 3
    if 2 in exits:
        return 2
    if any(code != 0 for code in exits) or not chain_equal or not observations_equal:
        return 1
    return 0


def compare(args: argparse.Namespace) -> int:
    traces = [args.local.resolve(strict=True), args.rebuilt.resolve(strict=True)]
    if any(not path.is_dir() for path in traces):
        raise ValueError("each trace must be a directory")
    output = args.output.resolve(strict=False)
    if not output_is_disjoint(output, traces):
        raise ValueError("output directory must be outside both trace trees")
    if output.exists():
        raise ValueError("output directory already exists")
    if not output.parent.is_dir():
        raise ValueError("output parent directory must already exist")
    verifier = args.verifier.resolve(strict=True)
    if not verifier.is_file():
        raise ValueError("verifier must be a file")
    verifier_bytes = verifier.read_bytes()
    dev_key = None
    key_provenance: dict[str, Any] = {"source": "none"}
    if args.dev_key_file is not None:
        key_path = args.dev_key_file.resolve(strict=True)
        key_bytes = key_path.read_bytes().strip()
        try:
            dev_key = key_bytes.decode("ascii")
        except UnicodeDecodeError as exc:
            raise ValueError("public key file must contain 64 hexadecimal characters") from exc
        if not re.fullmatch(r"[0-9a-fA-F]{64}", dev_key):
            raise ValueError("public key file must contain 64 hexadecimal characters")
        key_provenance = {"source": "supplied_file", "file_name": key_path.name,
                          "sha256": sha256(key_bytes.lower())}

    os.mkdir(output, 0o700)
    output.chmod(0o700)
    legs = {
        "local": run_leg("local", traces[0], output, verifier, dev_key),
        "rebuilt": run_leg("rebuilt", traces[1], output, verifier, dev_key),
    }
    chain_left, chain_right = normalized_chain(legs["local"]), normalized_chain(legs["rebuilt"])
    chain_equal = chain_left is not None and chain_left == chain_right
    local_leg, rebuilt_leg = legs["local"], legs["rebuilt"]
    observation_equal = (
        local_leg.get("observation_count") == rebuilt_leg.get("observation_count")
        and local_leg.get("observation_sha256") == rebuilt_leg.get("observation_sha256")
        and isinstance(local_leg.get("observation_count"), int)
        and isinstance(rebuilt_leg.get("observation_count"), int)
        and isinstance(local_leg.get("observation_sha256"), str)
        and isinstance(rebuilt_leg.get("observation_sha256"), str)
    )
    report_equal = (local_leg.get("report_sha256") == rebuilt_leg.get("report_sha256")
                    and isinstance(local_leg.get("report_sha256"), str)
                    and isinstance(rebuilt_leg.get("report_sha256"), str))
    metadata_fields = ("schema_version", "input", "run", "options", "key_source",
                       "verifier_version", "delivery")
    metadata_left = local_leg.get("research_summary")
    metadata_right = rebuilt_leg.get("research_summary")
    if metadata_left is not None:
        metadata_left = {key: metadata_left.get(key) for key in metadata_fields}
    if metadata_right is not None:
        metadata_right = {key: metadata_right.get(key) for key in metadata_fields}
    metadata_differences = (differing_paths(metadata_left, metadata_right)
                            if metadata_left is not None and metadata_right is not None else ["unavailable"])
    exit_code = aggregate_exit(legs, chain_equal, observation_equal)
    comparison = {
        "schema_version": 1,
        "purpose": "offline artifact comparison; does not prove remote retrieval",
        "verifier": {"path": str(verifier), "sha256": sha256(verifier_bytes)},
        "key_provenance": key_provenance,
        "legs": legs,
        "chain": {"local": chain_left, "rebuilt": chain_right, "equal": chain_equal},
        "observations": {
            "includes_unknown_observations": True,
            "local_count": local_leg.get("observation_count"),
            "rebuilt_count": rebuilt_leg.get("observation_count"),
            "local_sha256": local_leg.get("observation_sha256"),
            "rebuilt_sha256": rebuilt_leg.get("observation_sha256"),
            "equal": observation_equal,
        },
        "full_report": {"local_sha256": local_leg.get("report_sha256"),
                        "rebuilt_sha256": rebuilt_leg.get("report_sha256"),
                        "equal": report_equal},
        "metadata_differences": metadata_differences,
        "remote_retrieval": "not established by this comparison",
        "aggregate_exit": exit_code,
    }
    payload = (json.dumps(comparison, sort_keys=True, indent=2, ensure_ascii=False) + "\n").encode()
    write_private(output / "comparison.json", payload)
    return exit_code


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--local", required=True, type=Path, help="local trace directory")
    parser.add_argument("--rebuilt", required=True, type=Path, help="rebuilt trace directory")
    parser.add_argument("--output", required=True, type=Path, help="new private output directory")
    parser.add_argument("--verifier", type=Path,
                        default=Path(__file__).resolve().parents[1] / "target/debug/goalcup-verify")
    parser.add_argument("--dev-key-file", type=Path,
                        help="optional file containing the verifier's 64-hex public key")
    args = parser.parse_args()
    try:
        return compare(args)
    except (OSError, ValueError) as exc:
        print(f"research-replay: {exc}", file=sys.stderr)
        return 3


if __name__ == "__main__":
    raise SystemExit(main())
