#!/usr/bin/env python3
"""Behavioral tests for scripts/fetch-run.sh, against a fake bucket and a fixture chunk index.

    python3 -m unittest discover -s scripts -p 'test_fetch_run.py'

No database and no R2: a directory stands in for the bucket (`--store`), a JSON file for the chunk
index (`--index`), and, for the credentialed paths, a fake `psql` and a fake `curl` on PATH. The
trace that gets chunked is a real corpus case of the verifier, so the rebuilt directory can go
through the real `goalcup-verify` when a debug build exists (GOALCUP_VERIFY names another binary).
"""

from __future__ import annotations

import hashlib
import json
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

try:
    from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
except ImportError:  # pragma: no cover
    Ed25519PrivateKey = None

SCRIPT = Path(__file__).with_name("fetch-run.sh")
REPO = Path(__file__).resolve().parent.parent
CORPUS = REPO / "crates/goalcup-verifier/tests/fixtures/v5-corpus"
VERIFY = Path(os.environ.get("GOALCUP_VERIFY") or REPO / "target/debug/goalcup-verify")
RUN = "r0123456789abcdef"
ENTRY = "e0123456789abcdef"
ENTRY_2 = "efedcba9876543210"
BUCKET = "goalcup-traces"
# The server key the signed corpus cases were made under: seed 0x42 repeated 32 times.
DEV_SEED = bytes([0x42]) * 32
TOKEN = "cf-token-must-never-appear"
DB_URL = "postgres://reader:db-password-must-never-appear@db.invalid/postgres"

FAKE_PSQL = """#!/bin/sh
# Records its arguments and the SQL it was fed, then answers with the index.
echo "$*" >>"$FAKE_LOG.psql.args"
cat >"$FAKE_LOG.psql.sql"
cat "$FAKE_INDEX"
"""

FAKE_CURL = """#!/usr/bin/env python3
# Reads curl's configuration from stdin, as the script gives it, and serves objects from FAKE_STORE.
import os, sys, urllib.parse
with open(os.environ["FAKE_LOG"] + ".curl.args", "a") as f:
    f.write(" ".join(sys.argv[1:]) + "\\n")
cfg = {}
for line in sys.stdin.read().splitlines():
    name, _, value = line.partition(" = ")
    cfg.setdefault(name.strip(), []).append(value.strip().strip('"'))
with open(os.environ["FAKE_LOG"] + ".curl.cfg", "a") as f:
    f.write(repr(cfg) + "\\n")
if cfg.get("header") != ["Authorization: Bearer " + os.environ["EXPECT_TOKEN"]]:
    sys.exit(22)
url = cfg["url"][0]
prefix = "https://api.cloudflare.com/client/v4/accounts/" + os.environ["EXPECT_ACCOUNT"] + "/r2/buckets/" + os.environ["EXPECT_BUCKET"] + "/objects/"
if not url.startswith(prefix):
    sys.exit(22)
key = urllib.parse.unquote(url[len(prefix):])
src = os.path.join(os.environ["FAKE_STORE"], key)
if not os.path.isfile(src):
    sys.exit(22)
with open(src, "rb") as s, open(cfg["output"][0], "wb") as o:
    o.write(s.read())
"""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def zstd(data: bytes) -> bytes:
    return subprocess.run(["zstd", "-q", "-c"], input=data, capture_output=True, check=True).stdout


def ack_signing_bytes(ack: dict) -> bytes:
    body = {k: ack[k] for k in ("entry_id", "seq", "hash", "acked_at")}
    return b"goalcup-ack-v1\0" + json.dumps(body, separators=(",", ":")).encode()


@unittest.skipUnless(shutil.which("zstd"), "zstd is needed")
class FetchRunTest(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        self.store = self.root / "bucket"
        self.out = self.root / "out"
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.index_path = self.root / "index.json"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    # Fixtures.

    def put(self, key: str, data: bytes) -> None:
        path = self.store / key
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def entry_index(self, case: str, entry: str, chunks: int, sign: bool = False) -> dict:
        """One index entry for a corpus case, its chain cut into `chunks` pieces at line ends."""
        src = CORPUS / case
        lines = (src / "chain.jsonl").read_bytes().splitlines(keepends=True)
        cuts = [round(i * len(lines) / chunks) for i in range(chunks + 1)]
        original = []
        acks_file = src / "acks.jsonl"
        if acks_file.exists() and acks_file.read_text().strip():
            original = [json.loads(line) for line in acks_file.read_text().splitlines()]
        rows, offset = [], 0
        for n in range(chunks):
            part = lines[cuts[n] : cuts[n + 1]]
            data = b"".join(part)
            last = json.loads(part[-1])
            body = zstd(data)
            digest = sha256(body)
            key = f"traces/goalcup-cup-2026/{RUN}/{entry}/{offset:016d}-{digest}.zst"
            self.put(key, body)
            ack = {
                "entry_id": last["entry_id"],
                "seq": last["seq"],
                "hash": last["hash"],
                "acked_at": f"2026-10-02T12:00:{10 + n:02d}Z",
                "server_signature": "00" * 64,
            }
            if n == chunks - 1 and original:
                ack = original[-1]  # the real one, at the chain's end
            elif sign:
                ack["server_signature"] = Ed25519PrivateKey.from_private_bytes(DEV_SEED).sign(ack_signing_bytes(ack)).hex()
            rows.append(
                {
                    "offset": offset,
                    "length": len(data),
                    "sha256": digest,
                    "first_seq": json.loads(part[0])["seq"],
                    "last_seq": last["seq"],
                    "head": last["hash"],
                    "r2_key": key,
                    "ack": ack,
                }
            )
            offset += len(data)
        receipt = src / "receipt.json"
        doc = {"seal_key": None, "seal_sha256": None, "baseline_key": None, "baseline_sha256": None}
        for name in ("seal", "baseline"):
            if (src / f"{name}.json").exists() and (src / f"{name}.json").stat().st_size:
                data = (src / f"{name}.json").read_bytes()
                digest = sha256(data)
                key = f"traces/goalcup-cup-2026/{RUN}/{entry}/{name}-{digest}.json"
                self.put(key, data)
                doc[f"{name}_key"], doc[f"{name}_sha256"] = key, digest
        return {
            "run_id": RUN,
            "entry_id": entry,
            "segment": 0,
            "receipt": json.loads(receipt.read_text()) if receipt.exists() else None,
            **doc,
            "chunks": rows,
        }

    def write_index(self, *entries: dict) -> None:
        self.index_path.write_text(json.dumps(list(entries)))

    def run_script(self, *args: str, env: dict | None = None, path_first: Path | None = None):
        full = {k: v for k, v in os.environ.items() if k not in ("DATABASE_URL", "CLOUDFLARE_API_TOKEN", "CLOUDFLARE_ACCOUNT_ID")}
        if path_first is not None:
            full["PATH"] = f"{path_first}{os.pathsep}{full.get('PATH', '/usr/bin:/bin')}"
        full.update(env or {})
        return subprocess.run(["sh", str(SCRIPT), *args], env=full, capture_output=True, text=True, timeout=120)

    def fetch(self, target: str = ENTRY, *extra: str):
        return self.run_script("--index", str(self.index_path), "--store", str(self.store), *extra, target, str(self.out))

    def verify(self, directory: Path, *args: str) -> subprocess.CompletedProcess:
        if not VERIFY.exists():
            self.skipTest("no goalcup-verify build (cargo build -p goalcup-verifier)")
        return subprocess.run(
            [str(VERIFY), "--capture-integrity", str(directory), *args], capture_output=True, text=True, timeout=60
        )

    def assert_same_json_lines(self, got: Path, want: Path) -> None:
        read = lambda p: [json.loads(line) for line in p.read_text().splitlines()]  # noqa: E731
        self.assertEqual(read(got), read(want))

    # The trace comes back whole, and the verifier takes it.

    def test_one_chunk_rebuilds_a_trace_the_verifier_accepts(self) -> None:
        case = CORPUS / "signed-receipt-key-agrees"
        self.write_index(self.entry_index("signed-receipt-key-agrees", ENTRY, 1))
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = self.out / ENTRY
        self.assertEqual(result.stdout.splitlines(), [str(trace)])
        self.assertEqual((trace / "chain.jsonl").read_bytes(), (case / "chain.jsonl").read_bytes())
        self.assert_same_json_lines(trace / "acks.jsonl", case / "acks.jsonl")
        self.assertEqual(json.loads((trace / "receipt.json").read_text()), json.loads((case / "receipt.json").read_text()))
        self.assertEqual(sorted(p.name for p in trace.iterdir()), ["acks.jsonl", "baseline.json", "chain.jsonl", "receipt.json"])
        self.assertEqual((trace / "baseline.json").read_bytes(), (case / "baseline.json").read_bytes())
        self.assertEqual(sorted(p.name for p in self.out.iterdir()), [ENTRY], "no temporary leftovers")
        # This case is a signed finalization: it has a baseline and no seal.
        self.assertIn("no seal.json", result.stderr)

        key = json.loads((case / "receipt.json").read_text())["public_key"]
        report = self.verify(trace, "--dev-key", key)
        self.assertEqual(report.returncode, 0, report.stderr)
        self.assertEqual(json.loads(report.stdout)["integrity"], "valid")

    @unittest.skipUnless(Ed25519PrivateKey, "the cryptography package signs the acks")
    def test_several_chunks_join_in_offset_order_and_still_verify(self) -> None:
        case = CORPUS / "signed-receipt-key-agrees"
        self.write_index(self.entry_index("signed-receipt-key-agrees", ENTRY, 3, sign=True))
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = self.out / ENTRY
        self.assertEqual((trace / "chain.jsonl").read_bytes(), (case / "chain.jsonl").read_bytes())
        self.assertEqual(len((trace / "acks.jsonl").read_text().splitlines()), 3, "one ack per chunk")
        key = json.loads((case / "receipt.json").read_text())["public_key"]
        report = self.verify(trace, "--dev-key", key)
        said = json.loads(report.stdout)
        self.assertEqual((said["integrity"], said["validated_records"]), ("valid", 21), report.stdout)
        self.assertIsNone(said["first_failure"])

    def test_a_sealed_entry_brings_back_its_seal_and_baseline_documents(self) -> None:
        case = CORPUS / "sealed-websocket"
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 2))
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = self.out / ENTRY
        for name in ("chain.jsonl", "seal.json", "baseline.json"):
            self.assertEqual((trace / name).read_bytes(), (case / name).read_bytes(), name)
        self.assertFalse((trace / "receipt.json").exists())
        self.assertEqual(result.stderr.count("it was not sealed"), 0)

    def test_a_run_id_builds_every_entry_of_the_run(self) -> None:
        first = self.entry_index("sealed-websocket", ENTRY, 1)
        second = self.entry_index("sealed-websocket", ENTRY_2, 1)
        second["segment"] = 1
        self.write_index(first, second)
        result = self.fetch(RUN)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.splitlines(), [str(self.out / ENTRY), str(self.out / ENTRY_2)])
        for entry in (ENTRY, ENTRY_2):
            self.assertTrue((self.out / entry / "chain.jsonl").is_file())

    # What it refuses, and what it leaves behind when it does.

    def refused(self, result, text: str) -> None:
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn(text, result.stderr)
        self.assertEqual(result.stdout, "")
        self.assertEqual(list(self.out.iterdir()), [], "a failed fetch leaves no directory")

    def test_a_chunk_that_is_not_the_indexed_bytes_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 2)
        self.write_index(index)
        victim = self.store / index["chunks"][1]["r2_key"]
        victim.write_bytes(zstd(b"something else entirely\n"))
        self.refused(self.fetch(), "does not match the index's SHA-256")

    def test_a_chunk_of_the_wrong_length_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 2)
        index["chunks"][0]["length"] += 1
        # The next chunk now starts one byte early in the index, which is a gap or overlap too.
        self.write_index(index)
        self.assertEqual(self.fetch().returncode, 1)
        index = self.entry_index("sealed-websocket", ENTRY, 1)
        index["chunks"][0]["length"] -= 1
        self.write_index(index)
        self.refused(self.fetch(), "the chunk holds")

    def test_a_gap_in_the_offsets_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 3)
        del index["chunks"][1]
        self.write_index(index)
        self.refused(self.fetch(), "the next chunk starts at byte")

    def test_an_index_that_starts_after_byte_zero_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 2)
        del index["chunks"][0]
        self.write_index(index)
        self.refused(self.fetch(), "the next chunk starts at byte")

    def test_an_object_missing_from_the_bucket_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 2)
        self.write_index(index)
        (self.store / index["chunks"][1]["r2_key"]).unlink()
        self.refused(self.fetch(), "is not in")

    def test_an_entry_without_chunks_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 1)
        index["chunks"] = []
        self.write_index(index)
        self.refused(self.fetch(), "has no chunks")

    def test_a_key_that_climbs_out_of_the_bucket_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 1)
        index["chunks"][0]["r2_key"] = "traces/../../etc/passwd"
        self.write_index(index)
        self.refused(self.fetch(), "malformed chunk row")

    def test_an_entry_that_is_already_there_is_never_overwritten(self) -> None:
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        (self.out / ENTRY).mkdir(parents=True)
        (self.out / ENTRY / "mine.txt").write_text("keep")
        result = self.fetch()
        self.assertEqual(result.returncode, 1)
        self.assertIn("exists", result.stderr)
        self.assertEqual((self.out / ENTRY / "mine.txt").read_text(), "keep")
        self.assertEqual(sorted(p.name for p in self.out.iterdir()), [ENTRY])

    def test_a_bad_id_or_missing_arguments_are_usage_errors(self) -> None:
        for args in ([], ["r123", str(self.out)], [ENTRY], ["--nope", ENTRY, str(self.out)]):
            with self.subTest(args=args):
                result = self.run_script("--index", str(self.index_path), "--store", str(self.store), *args)
                self.assertEqual(result.returncode, 2, result.stderr)

    def test_without_credentials_or_options_it_says_what_to_set(self) -> None:
        result = self.run_script(ENTRY, str(self.out))
        self.assertEqual(result.returncode, 1)
        self.assertIn("DATABASE_URL", result.stderr)
        result = self.run_script("--index", str(self.index_path), ENTRY, str(self.out))
        self.assertEqual(result.returncode, 1)
        self.assertIn("CLOUDFLARE_ACCOUNT_ID", result.stderr)

    # The credentialed paths, with a fake psql and curl on PATH.

    def fakes(self) -> dict:
        (self.bin / "psql").write_text(FAKE_PSQL)
        (self.bin / "curl").write_text(FAKE_CURL)
        for name in ("psql", "curl"):
            (self.bin / name).chmod(0o755)
        return {
            "FAKE_LOG": str(self.root / "fake"),
            "FAKE_INDEX": str(self.index_path),
            "FAKE_STORE": str(self.store),
            "EXPECT_TOKEN": TOKEN,
            "EXPECT_ACCOUNT": "acct123",
            "EXPECT_BUCKET": BUCKET,
            "DATABASE_URL": DB_URL,
            "CLOUDFLARE_ACCOUNT_ID": "acct123",
            "CLOUDFLARE_API_TOKEN": TOKEN,
        }

    def test_the_database_is_only_read_and_the_run_id_is_a_variable(self) -> None:
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        result = self.run_script("--store", str(self.store), RUN, str(self.out), env=self.fakes(), path_first=self.bin)
        self.assertEqual(result.returncode, 0, result.stderr)
        sql = (self.root / "fake.psql.sql").read_text()
        self.assertTrue(sql.lstrip().upper().startswith("BEGIN READ ONLY"), sql)
        self.assertIn("ROLLBACK", sql)
        for word in ("INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "TRUNCATE", "COMMIT"):
            self.assertNotIn(word, sql.upper().replace("UPDATED", ""), word)
        self.assertNotIn(RUN, sql, "the id is passed as a psql variable, never spliced into the SQL")
        args = (self.root / "fake.psql.args").read_text()
        self.assertIn(f"id={RUN}", args)
        self.assertTrue((self.out / ENTRY / "chain.jsonl").is_file())

    def test_r2_is_read_with_the_token_out_of_the_command_line_and_the_output(self) -> None:
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 2))
        env = self.fakes()
        result = self.run_script("--index", str(self.index_path), ENTRY, str(self.out), env=env, path_first=self.bin)
        self.assertEqual(result.returncode, 0, result.stderr)
        args = (self.root / "fake.curl.args").read_text()
        self.assertEqual(args.split(), ["-K", "-"] * (len(args.splitlines())), "curl is told nothing but its config")
        self.assertGreaterEqual(len(args.splitlines()), 4, "two chunks, the seal and the baseline")
        for text in (result.stdout, result.stderr, args):
            self.assertNotIn(TOKEN, text)
            self.assertNotIn("db-password", text)
        self.assertEqual((self.out / ENTRY / "seal.json").read_bytes(), (CORPUS / "sealed-websocket/seal.json").read_bytes())

    def test_a_staging_bucket_is_named_in_the_url(self) -> None:
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        env = {**self.fakes(), "EXPECT_BUCKET": "goalcup-traces-staging"}
        wrong = self.run_script("--index", str(self.index_path), ENTRY, str(self.out), env=env, path_first=self.bin)
        self.assertEqual(wrong.returncode, 1, "the default bucket is the production one")
        right = self.run_script(
            "--index", str(self.index_path), "--bucket", "goalcup-traces-staging", ENTRY, str(self.out), env=env, path_first=self.bin
        )
        self.assertEqual(right.returncode, 0, right.stderr)


if __name__ == "__main__":
    unittest.main()
