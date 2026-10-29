#!/usr/bin/env python3
"""Behavioral tests for scripts/fetch-run.sh, against a fake bucket and a fixture chunk index.

    python3 -m unittest discover -s scripts -p 'test_fetch_run.py'

No database and no R2: a directory stands in for the bucket (`--store`), a JSON file for the chunk
index (`--index`), and, for the credentialed paths, a fake `psql` and a fake `curl` on PATH. The
trace that gets chunked is a real corpus case of the verifier, and the rebuilt directory goes
through the real `goalcup-verify`: built by `setUpModule` (GOALCUP_VERIFY names another binary), and
a failure to have one fails the run. The acks of a multi-chunk case are signed in pure Python.
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

SCRIPT = Path(__file__).with_name("fetch-run.sh")
REPO = Path(__file__).resolve().parent.parent
CORPUS = REPO / "crates/goalcup-verifier/tests/fixtures/v5-corpus"
# The strongest test runs the real verifier. It is built here (a no-op when current) unless
# GOALCUP_VERIFY names one, and a missing verifier fails the run; it never skips.
VERIFY = Path(os.environ.get("GOALCUP_VERIFY") or Path(os.environ.get("CARGO_TARGET_DIR") or REPO / "target") / "debug/goalcup-verify")
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
# What psql would connect with: the service file and its mode, which only psql may read.
cp "$PGSERVICEFILE" "$FAKE_LOG.psql.service"
find "$PGSERVICEFILE" -perm 600 | grep -q . && echo private >"$FAKE_LOG.psql.mode"
echo "$PGSERVICE" >"$FAKE_LOG.psql.name"
cat "$FAKE_INDEX"
"""

FAKE_CURL = """#!/usr/bin/env python3
# Reads curl's configuration from stdin, as the script gives it, and serves objects from FAKE_STORE.
# As curl does: with `fail` a missing object (or a FAKE_FORBID one, refused as 403) is exit 22; without
# it the error body is written and the status is what `write-out` names.
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
status = 200
if key == os.environ.get("FAKE_FORBID"):
    status = 403
elif not os.path.isfile(src):
    status = 404
if status != 200 and "fail" in cfg:
    sys.exit(22)
with open(cfg["output"][0], "wb") as o:
    o.write(open(src, "rb").read() if status == 200 else b'{"error":"not here"}')
if "write-out" in cfg:
    sys.stdout.write(cfg["write-out"][0].replace("%{http_code}", str(status)))
"""


# Ed25519 (RFC 8032), so the test signs acks with the corpus's server key and needs no package.
_P = 2**255 - 19
_Q = 2**252 + 27742317777372353535851937790883648493
_D = -121665 * pow(121666, -1, _P) % _P


def _add(a, b):
    A, B = (a[1] - a[0]) * (b[1] - b[0]) % _P, (a[1] + a[0]) * (b[1] + b[0]) % _P
    C, D = 2 * a[3] * b[3] * _D % _P, 2 * a[2] * b[2] % _P
    E, F, G, H = B - A, D - C, D + C, B + A
    return (E * F % _P, G * H % _P, F * G % _P, E * H % _P)


def _mul(n, point):
    result, addend = (0, 1, 1, 0), point
    while n:
        if n & 1:
            result = _add(result, addend)
        addend = _add(addend, addend)
        n >>= 1
    return result


def _compress(point) -> bytes:
    zinv = pow(point[2], _P - 2, _P)
    x, y = point[0] * zinv % _P, point[1] * zinv % _P
    return (y | ((x & 1) << 255)).to_bytes(32, "little")


def _base():
    y = 4 * pow(5, -1, _P) % _P
    x2 = (y * y - 1) * pow(_D * y * y + 1, -1, _P) % _P
    x = pow(x2, (_P + 3) // 8, _P)
    if (x * x - x2) % _P:
        x = x * pow(2, (_P - 1) // 4, _P) % _P
    if x & 1:
        x = _P - x
    return (x, y, 1, x * y % _P)


def ed25519_sign(seed: bytes, message: bytes) -> bytes:
    h = hashlib.sha512(seed).digest()
    a = (int.from_bytes(h[:32], "little") & ((1 << 254) - 8)) | (1 << 254)
    public = _compress(_mul(a, _base()))
    r = int.from_bytes(hashlib.sha512(h[32:] + message).digest(), "little") % _Q
    big_r = _compress(_mul(r, _base()))
    k = int.from_bytes(hashlib.sha512(big_r + public + message).digest(), "little") % _Q
    return big_r + ((r + k * a) % _Q).to_bytes(32, "little")


def setUpModule() -> None:
    if os.environ.get("GOALCUP_VERIFY"):
        return
    built = subprocess.run(
        ["cargo", "build", "--offline", "-p", "goalcup-verifier", "--bin", "goalcup-verify"],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    if built.returncode != 0:
        raise RuntimeError("cannot build goalcup-verify, which these tests run:\n" + built.stderr[-2000:])


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

    def coherent_signed_case(self) -> Path:
        """A valid receipt-anchored trace: a sealed corpus case plus a signed start receipt over
        its Launch and a signed ack at its local finish. The committed corpus is not changed.

        The signed-receipt corpus cases end in the signed cutoff and finalization that left the
        protocol (D54 amendment), so a valid signed trace is derived from a local finish."""
        case = self.root / "coherent-signed"
        shutil.copytree(CORPUS / "sealed-websocket", case)
        lines = [json.loads(line) for line in (case / "chain.jsonl").read_text().splitlines()]
        position, launch = next(
            (n, line["body"]["content"]["record"]) for n, line in enumerate(lines)
            if line["body"].get("content", {}).get("record", {}).get("kind") == "launch"
        )
        receipt = json.loads((CORPUS / "signed-receipt-key-agrees" / "receipt.json").read_text())
        receipt["claims"]["entry_id"] = lines[0]["entry_id"]
        receipt["claims"]["connector_id"] = launch["profile_id"]
        receipt["claims"]["manifest_sha256"] = launch["profile_digest"]
        receipt["claims"]["chain_head"] = lines[position]["hash"]
        del receipt["claims"]["run_public_key"]
        compact = lambda value: json.dumps(value, separators=(",", ":")).encode()  # noqa: E731
        receipt["signature"] = ed25519_sign(
            DEV_SEED, b"goalcup-receipt-v1\0" + compact(receipt["claims"])
        ).hex()
        (case / "receipt.json").write_bytes(compact(receipt))
        ack = {
            "entry_id": lines[-1]["entry_id"],
            "seq": lines[-1]["seq"],
            "hash": lines[-1]["hash"],
            "acked_at": "2026-10-02T12:01:00Z",
            "server_signature": "",
        }
        ack["server_signature"] = ed25519_sign(DEV_SEED, ack_signing_bytes(ack)).hex()
        (case / "acks.jsonl").write_bytes(compact(ack) + b"\n")
        return case

    def put(self, key: str, data: bytes) -> None:
        path = self.store / key
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def entry_index(self, case: str | Path, entry: str, chunks: int, sign: bool = False) -> dict:
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
                ack["server_signature"] = ed25519_sign(DEV_SEED, ack_signing_bytes(ack)).hex()
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
            "upload_offset": offset,
            "final_seq": rows[-1]["last_seq"],
            "final_head": rows[-1]["head"],
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
        self.assertTrue(VERIFY.exists(), f"no goalcup-verify at {VERIFY}")
        return subprocess.run(
            [str(VERIFY), "--capture-integrity", str(directory), *args], capture_output=True, text=True, timeout=60
        )

    def assert_same_json_lines(self, got: Path, want: Path) -> None:
        read = lambda p: [json.loads(line) for line in p.read_text().splitlines()]  # noqa: E731
        self.assertEqual(read(got), read(want))

    # The trace comes back whole, and the verifier takes it.

    def test_one_chunk_rebuilds_a_trace_the_verifier_accepts(self) -> None:
        case = self.coherent_signed_case()
        self.write_index(self.entry_index(case, ENTRY, 1))
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = self.out / ENTRY
        self.assertEqual(result.stdout.splitlines(), [str(trace)])
        self.assertEqual((trace / "chain.jsonl").read_bytes(), (case / "chain.jsonl").read_bytes())
        self.assert_same_json_lines(trace / "acks.jsonl", case / "acks.jsonl")
        self.assertEqual(json.loads((trace / "receipt.json").read_text()), json.loads((case / "receipt.json").read_text()))
        self.assertEqual(
            sorted(p.name for p in trace.iterdir()), ["acks.jsonl", "baseline.json", "chain.jsonl", "receipt.json", "seal.json"]
        )
        for name in ("baseline.json", "seal.json"):
            self.assertEqual((trace / name).read_bytes(), (case / name).read_bytes(), name)
        self.assertEqual(sorted(p.name for p in self.out.iterdir()), [ENTRY], "no temporary leftovers")

        key = json.loads((case / "receipt.json").read_text())["public_key"]
        report = self.verify(trace, "--dev-key", key)
        self.assertEqual(report.returncode, 0, report.stderr)
        self.assertEqual(json.loads(report.stdout)["integrity"], "valid")

    def test_several_chunks_join_in_offset_order_and_still_verify(self) -> None:
        case = self.coherent_signed_case()
        self.write_index(self.entry_index(case, ENTRY, 3, sign=True))
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = self.out / ENTRY
        self.assertEqual((trace / "chain.jsonl").read_bytes(), (case / "chain.jsonl").read_bytes())
        self.assertEqual(len((trace / "acks.jsonl").read_text().splitlines()), 3, "one ack per chunk")
        key = json.loads((case / "receipt.json").read_text())["public_key"]
        report = self.verify(trace, "--dev-key", key)
        said = json.loads(report.stdout)
        records = len((case / "chain.jsonl").read_text().splitlines())
        self.assertEqual((said["integrity"], said["validated_records"]), ("valid", records), report.stdout)
        self.assertIsNone(said["first_failure"])

    def test_contradictory_historical_evidence_is_rebuilt_without_repair(self) -> None:
        case = CORPUS / "signed-receipt-key-agrees"
        self.write_index(self.entry_index("signed-receipt-key-agrees", ENTRY, 1))
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = self.out / ENTRY
        for name in ("chain.jsonl", "baseline.json"):
            self.assertEqual((trace / name).read_bytes(), (case / name).read_bytes(), name)
        self.assertEqual(json.loads((trace / "receipt.json").read_text()), json.loads((case / "receipt.json").read_text()))
        self.assert_same_json_lines(trace / "acks.jsonl", case / "acks.jsonl")
        key = json.loads((case / "receipt.json").read_text())["public_key"]
        report = self.verify(trace, "--dev-key", key)
        self.assertEqual(report.returncode, 2, report.stderr)
        self.assertEqual(json.loads(report.stdout)["first_failure"]["code"], "receipt_launch_mismatch")

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

    def test_the_sealed_connection_list_comes_back_beside_its_seal(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 1)
        prefix = f"traces/goalcup-cup-2026/{RUN}/{ENTRY}"
        listing = (
            json.dumps({"format": "goalcup-tunnels-v1", "entry_id": ENTRY}) + "\n"
            + json.dumps({"host": "github.example.test", "port": 443, "reason": "sign_in"}) + "\n"
            + json.dumps({"dropped": 0}) + "\n"
        ).encode()
        self.put(f"{prefix}/tunnels.jsonl", listing)
        self.write_index(index)
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        trace = self.out / ENTRY
        self.assertEqual((trace / "tunnels.jsonl").read_bytes(), listing)
        self.assertEqual(
            sorted(p.name for p in trace.iterdir()),
            ["acks.jsonl", "baseline.json", "chain.jsonl", "seal.json", "tunnels.jsonl"],
        )

    def test_a_list_that_cannot_be_copied_fails_the_fetch(self) -> None:
        # A copy that fails must fail the fetch, not pass as an absent list. A fake `cp` on PATH
        # fails only the copy of the list and leaves a partial file behind, which must be removed.
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        self.put(f"traces/goalcup-cup-2026/{RUN}/{ENTRY}/tunnels.jsonl", b'{"format":"goalcup-tunnels-v1"}\n')
        fake = self.root / "fakebin"
        fake.mkdir()
        real_cp = shutil.which("cp")
        self.assertIsNotNone(real_cp)
        (fake / "cp").write_text(
            "#!/bin/sh\n"
            'case "$*" in *tunnels.jsonl*) echo partial >"$2" 2>/dev/null; exit 1 ;; esac\n'
            f'exec {real_cp} "$@"\n'
        )
        (fake / "cp").chmod(0o755)
        result = self.run_script(
            "--index", str(self.index_path), "--store", str(self.store), ENTRY, str(self.out), path_first=fake
        )
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn("could not copy", result.stderr)
        self.assertFalse((self.out / ENTRY).exists())

    def test_an_entry_whose_seal_carried_no_list_gets_none(self) -> None:
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertFalse((self.out / ENTRY / "tunnels.jsonl").exists())

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
        self.refused(self.fetch(), "the server holds")  # the stated end no longer matches
        index["upload_offset"] = None  # without it, the chunk itself still shows the lie
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

    def test_a_chunk_row_listed_twice_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 2)
        index["chunks"].insert(1, dict(index["chunks"][0]))
        self.write_index(index)
        self.refused(self.fetch(), "the next chunk starts at byte")

    def test_an_index_cut_short_of_what_the_server_holds_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 3)
        del index["chunks"][-1]  # contiguous from 0, but short of upload_offset
        self.write_index(index)
        self.refused(self.fetch(), "the server holds")
        index = self.entry_index("sealed-websocket", ENTRY, 3)
        index["upload_offset"] = None  # an older saved index: the final heartbeat still checks the tail
        del index["chunks"][-1]
        self.write_index(index)
        self.refused(self.fetch(), "stop before the end")

    def test_a_chunk_that_skips_sequence_numbers_is_refused(self) -> None:
        index = self.entry_index("sealed-websocket", ENTRY, 3)
        index["chunks"][1]["first_seq"] += 1
        self.write_index(index)
        self.refused(self.fetch(), "starts at seq")

    def test_an_outdir_with_a_quote_in_it_is_refused(self) -> None:
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        result = self.run_script("--index", str(self.index_path), "--store", str(self.store), ENTRY, str(self.root / 'o"ut'))
        self.assertEqual(result.returncode, 2)

    def test_a_term_signal_leaves_no_temporary_directory(self) -> None:
        import signal
        import time

        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        (self.bin / "zstd").write_text("#!/bin/sh\nsleep 30\n")
        (self.bin / "zstd").chmod(0o755)
        full = {**os.environ, "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}"}
        proc = subprocess.Popen(
            ["sh", str(SCRIPT), "--index", str(self.index_path), "--store", str(self.store), ENTRY, str(self.out)],
            env=full, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True,
        )
        for _ in range(100):  # wait until the temporary directory exists
            if self.out.exists() and list(self.out.glob(".fetch-run.*")):
                break
            time.sleep(0.05)
        time.sleep(0.5)
        os.killpg(proc.pid, signal.SIGTERM)
        proc.wait(timeout=20)
        self.assertEqual(list(self.out.iterdir()), [])

    def test_the_list_of_each_entry_is_its_own_prefix_and_two_lists_stay_apart(self) -> None:
        first = self.entry_index("sealed-websocket", ENTRY, 1)
        second = self.entry_index("sealed-websocket", ENTRY_2, 1)
        second["segment"] = 1
        for entry in (ENTRY, ENTRY_2):
            self.put(
                f"traces/goalcup-cup-2026/{RUN}/{entry}/tunnels.jsonl",
                json.dumps({"format": "goalcup-tunnels-v1", "entry_id": entry}).encode() + b"\n",
            )
        self.write_index(first, second)
        result = self.fetch(RUN)
        self.assertEqual(result.returncode, 0, result.stderr)
        for entry in (ENTRY, ENTRY_2):
            head = json.loads((self.out / entry / "tunnels.jsonl").read_text().splitlines()[0])
            self.assertEqual(head["entry_id"], entry)

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
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("DATABASE_URL", result.stderr)
        result = self.run_script("--index", str(self.index_path), ENTRY, str(self.out))
        self.assertNotEqual(result.returncode, 0)
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
        self.assertNotIn("db-password", args, "the password is not on psql's command line")
        self.assertNotIn("db.invalid", args)
        # The connection reached psql in a private service file instead.
        service = (self.root / "fake.psql.service").read_text()
        self.assertEqual(
            service.splitlines(),
            [f"[{(self.root / 'fake.psql.name').read_text().strip()}]", "host=db.invalid", "user=reader",
             "password=db-password-must-never-appear", "dbname=postgres"],
        )
        self.assertTrue((self.root / "fake.psql.mode").exists(), "the service file is mode 600")
        self.assertFalse(list(self.out.glob(".fetch-run.*")), "and it is gone afterwards")
        self.assertTrue((self.out / ENTRY / "chain.jsonl").is_file())

    def test_tracing_the_script_shows_no_secret(self) -> None:
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        result = subprocess.run(
            ["sh", "-x", str(SCRIPT), RUN, str(self.out)],
            env={**os.environ, **self.fakes(), "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}"},
            capture_output=True,
            text=True,
            timeout=120,
        )
        self.assertEqual(result.returncode, 0, result.stderr[-2000:])
        self.assertIn("+ ", result.stderr, "it really was traced")
        for secret in (TOKEN, "db-password", "reader:"):
            self.assertNotIn(secret, result.stdout + result.stderr)

    def test_a_missing_credential_fails_without_echoing_the_others(self) -> None:
        env = {**self.fakes()}
        del env["CLOUDFLARE_API_TOKEN"]
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        result = subprocess.run(
            ["sh", "-x", str(SCRIPT), RUN, str(self.out)],
            env={**os.environ, **env, "PATH": f"{self.bin}{os.pathsep}{os.environ['PATH']}"},
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("CLOUDFLARE_API_TOKEN", result.stderr)
        for secret in (TOKEN, "db-password", "reader:"):
            self.assertNotIn(secret, result.stdout + result.stderr)

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
        cfg = (self.root / "fake.curl.cfg").read_text()
        self.assertIn("'fail'", cfg)
        self.assertIn("'max-time': ['600']", cfg)
        self.assertIn("'connect-timeout': ['15']", cfg)

    def test_a_refused_list_read_is_not_an_absence(self) -> None:
        # Only a 404 means the seal carried no list. Any other refusal fails the fetch, as it does
        # for the seal, and leaves nothing behind.
        self.put(f"traces/goalcup-cup-2026/{RUN}/{ENTRY}/tunnels.jsonl", b"{}\n")
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        env = {**self.fakes(), "FAKE_FORBID": f"traces/goalcup-cup-2026/{RUN}/{ENTRY}/tunnels.jsonl"}
        result = self.run_script("--index", str(self.index_path), ENTRY, str(self.out), env=env, path_first=self.bin)
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("HTTP 403", result.stderr)
        self.assertEqual(list(self.out.iterdir()), [], "a failed fetch leaves no directory")

    def test_the_rebuilt_files_are_private_to_the_organizer(self) -> None:
        self.write_index(self.entry_index("sealed-websocket", ENTRY, 1))
        self.assertEqual(self.fetch().returncode, 0)
        trace = self.out / ENTRY
        for path in (trace, *trace.iterdir()):
            self.assertEqual(path.stat().st_mode & 0o077, 0, path)

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
