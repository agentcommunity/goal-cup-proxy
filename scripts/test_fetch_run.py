#!/usr/bin/env python3
"""Behavioral tests for scripts/fetch-run.sh, against a fake bucket and a fixture index.

    python3 scripts/test_fetch_run.py

No database, no R2, no zstd and no verifier: a directory stands in for the bucket (`--store`), a JSON
file for the index (`--index`), and, for the credentialed paths, a fake `psql` and a fake `curl` on
PATH. Chunk bodies are synthetic bytes: fetch-run never decodes them.
"""

from __future__ import annotations

import hashlib
import json
import os
import random
import re
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("fetch-run.sh")
RUN = "r0123456789abcdef"
ENTRY = "e0123456789abcdef"
ENTRY_2 = "efedcba9876543210"
BUCKET = "goalcup-traces"
GENESIS = "0" * 64
TOKEN = "cf-token-must-never-appear"
DB_URL = "postgres://reader:db-password-must-never-appear@db.invalid/postgres"
SECRETS = (TOKEN, "db-password", "reader:")

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
# Reads curl's configuration from stdin, as the script gives it, and serves objects from FAKE_STORE:
# the body and 200, or an error body and 404 (403 for FAKE_FORBID), the status as `write-out` names it.
import os, sys, urllib.parse
with open(os.environ["FAKE_LOG"] + ".curl.args", "a") as f:
    f.write(" ".join(sys.argv[1:]) + "\\n")
cfg = {}
for line in sys.stdin.read().splitlines():
    name, _, value = line.partition(" = ")
    cfg.setdefault(name.strip(), []).append(value.strip().strip('"'))
prefix = ("https://api.cloudflare.com/client/v4/accounts/" + os.environ["EXPECT_ACCOUNT"] + "/r2/buckets/"
          + os.environ["EXPECT_BUCKET"] + "/objects/")
url = cfg["url"][0]
status = 200
if cfg.get("header") != ["Authorization: Bearer " + os.environ["EXPECT_TOKEN"]] or not url.startswith(prefix):
    status = 403
key = urllib.parse.unquote(url[len(prefix):])
src = os.path.join(os.environ["FAKE_STORE"], key)
if status == 200 and key == os.environ.get("FAKE_FORBID"):
    status = 403
elif status == 200 and not os.path.isfile(src):
    status = 404
with open(cfg["output"][0], "wb") as o:
    o.write(open(src, "rb").read() if status == 200 else b'{"error":"no"}')
sys.stdout.write(cfg["write-out"][0].replace("%{http_code}", str(status)))
"""


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def link(tag: str, seq: int) -> str:
    return sha256(f"{tag}:{seq}".encode())


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

    def entry(self, entry: str = ENTRY, chunks: int = 3, sealed: bool = True, held: int | None = None) -> dict:
        """An index entry of `chunks` chunks of synthetic bytes, each put into the store. `held` chunks
        of them are on the server (the rest never arrived); a sealed entry has its final heartbeat."""
        rng = random.Random(entry + str(chunks))
        held = chunks if held is None else held
        rows, bodies = [], []
        offset, seq, head = 0, 0, GENESIS
        for n in range(chunks):
            body = bytes(rng.randrange(256) for _ in range(300 + 17 * n))
            count = 4 + n
            last = seq + count - 1
            key = f"traces/goalcup-cup-2026/{RUN}/{entry}/{offset:016d}-{sha256(body)}.zst"
            row = {
                "offset": offset, "length": 1000 + n, "sha256": sha256(body),
                "first_seq": seq, "last_seq": last, "prev": head, "head": link(entry, last), "r2_key": key,
                "received_at": f"2026-11-14T00:0{n}:00Z",
            }
            rows.append(row)
            bodies.append(body)
            (self.store / key).parent.mkdir(parents=True, exist_ok=True)
            (self.store / key).write_bytes(body)
            offset, seq, head = offset + row["length"], last + 1, row["head"]
        kept = rows[:held]
        end = kept[-1] if kept else None
        heartbeats = [
            {"received_at": f"2026-11-14T00:{m:02d}:00Z", "seq": m, "offset_local": 100 * m, "final": False,
             "recording": ["ok", "gaps", "gaps", "ok", "gaps", "failed"][m], "model_requests": m,
             "counts": {"v": 1, "off_ms": 10 * m, "mode": "recording"}, "counts_dropped": None}
            for m in range(6)
        ]
        heartbeats[2]["counts"], heartbeats[2]["counts_dropped"] = None, 4096
        return {
            "run_id": RUN, "entry_id": entry, "segment": 0,
            "upload_offset": end["offset"] + end["length"] if end else 0,
            "next_seq": end["last_seq"] + 1 if end else 0, "head": end["head"] if end else GENESIS,
            "model_requests": 905, "final_seq": rows[-1]["last_seq"] if sealed else None,
            "final_head": rows[-1]["head"] if sealed else None, "final_at": "2026-11-14T00:09:00Z" if sealed else None,
            "sealed_at": "2026-11-14T00:09:09Z" if sealed and held == chunks else None,
            "stored_bytes": sum(len(b) for b in bodies[:held]), "chunk_count": held,
            "heartbeats": heartbeats, "chunks": kept, "_bodies": bodies,
        }

    def write_index(self, *entries: dict) -> None:
        self.index_path.write_text(json.dumps([{k: v for k, v in e.items() if k != "_bodies"} for e in entries]))

    def run_script(self, *args: str, env: dict | None = None, trace: bool = False):
        full = {k: v for k, v in os.environ.items() if k not in ("DATABASE_URL", "CLOUDFLARE_API_TOKEN", "CLOUDFLARE_ACCOUNT_ID")}
        full["PATH"] = f"{self.bin}{os.pathsep}{full.get('PATH', '/usr/bin:/bin')}"
        full.update(env or {})
        cmd = ["sh", "-x", str(SCRIPT)] if trace else ["sh", str(SCRIPT)]
        return subprocess.run([*cmd, *args], env=full, capture_output=True, text=True, timeout=120)

    def fetch(self, target: str = ENTRY):
        return self.run_script("--index", str(self.index_path), "--store", str(self.store), target, str(self.out))

    def fetched(self, entry: str = ENTRY) -> tuple[bytes, dict]:
        d = self.out / entry
        return (d / "rec.gcr.zst").read_bytes(), json.loads((d / "fetch.json").read_text())

    # The server copy, whole or partial.

    def test_a_whole_sealed_entry_is_complete_and_is_the_chunks_joined(self) -> None:
        e = self.entry()
        self.write_index(e)
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.split(), [str(self.out / ENTRY)])
        rec, doc = self.fetched()
        self.assertEqual(rec, b"".join(e["_bodies"]), "the objects joined in offset order, not decoded")
        self.assertEqual(doc["v"], 1)
        self.assertTrue(doc["complete"])
        self.assertIsNone(doc["missing"])
        self.assertEqual(doc["final"], {"seq": e["final_seq"], "head": e["final_head"], "at": e["final_at"]})
        self.assertEqual((doc["run_id"], doc["entry_id"], doc["upload_offset"], doc["next_seq"], doc["model_requests"]),
                         (RUN, ENTRY, e["upload_offset"], e["next_seq"], 905))
        self.assertEqual((doc["stored_bytes"], doc["chunk_count"]), (len(rec), 3))
        self.assertEqual([c["bytes"] for c in doc["chunks"]], [len(b) for b in e["_bodies"]])
        self.assertEqual([c["r2_key"] for c in doc["chunks"]], [c["r2_key"] for c in e["chunks"]])

    def test_a_partial_entry_is_written_as_far_as_it_goes_and_names_what_is_missing(self) -> None:
        # The final heartbeat committed the end, but only two of three chunks reached the server.
        e = self.entry(held=2)
        self.write_index(e)
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        rec, doc = self.fetched()
        self.assertEqual(rec, b"".join(e["_bodies"][:2]))
        self.assertFalse(doc["complete"])
        self.assertEqual(doc["missing"], {"from_offset": e["upload_offset"], "from_seq": e["next_seq"], "to_seq": e["final_seq"]})

    def test_an_entry_without_a_final_heartbeat_is_written_with_an_open_end(self) -> None:
        e = self.entry(sealed=False)
        self.write_index(e)
        result = self.fetch()
        self.assertEqual(result.returncode, 0, result.stderr)
        rec, doc = self.fetched()
        self.assertEqual(rec, b"".join(e["_bodies"]))
        self.assertFalse(doc["complete"])
        self.assertIsNone(doc["final"])
        self.assertEqual(doc["missing"], {"from_offset": e["upload_offset"], "from_seq": e["next_seq"], "to_seq": None})

    def test_a_chunk_that_does_not_follow_the_previous_head_stops_the_entry(self) -> None:
        e = self.entry()
        e["chunks"][2]["prev"] = link("elsewhere", 1)
        self.write_index(e)
        result = self.fetch()
        self.assertEqual(result.returncode, 1, result.stderr)
        rec, doc = self.fetched()
        self.assertEqual(rec, b"".join(e["_bodies"][:2]), "nothing from the chunk that breaks the chain")
        self.assertFalse(doc["complete"])
        self.assertEqual(doc["missing"]["from_offset"], e["chunks"][2]["offset"])
        self.assertEqual(doc["missing"]["from_seq"], e["chunks"][2]["first_seq"])

    def test_a_missing_object_ends_the_entry_there(self) -> None:
        e = self.entry()
        (self.store / e["chunks"][1]["r2_key"]).unlink()
        self.write_index(e)
        result = self.fetch()
        self.assertEqual(result.returncode, 1, result.stderr)
        rec, doc = self.fetched()
        self.assertEqual(rec, e["_bodies"][0])
        self.assertEqual(len(doc["chunks"]), 1)
        self.assertFalse(doc["complete"])
        self.assertEqual(doc["missing"]["why"], "object_bad")
        self.assertEqual(doc["missing"]["r2_key"], e["chunks"][1]["r2_key"])
        self.assertEqual(doc["missing"]["from_offset"], e["chunks"][1]["offset"])

    def test_an_object_failing_its_sha256_ends_the_entry_there(self) -> None:
        e = self.entry()
        path = self.store / e["chunks"][2]["r2_key"]
        path.write_bytes(path.read_bytes()[:-1] + b"\x00")
        self.write_index(e)
        result = self.fetch()
        self.assertEqual(result.returncode, 1, result.stderr)
        rec, doc = self.fetched()
        self.assertEqual(rec, b"".join(e["_bodies"][:2]))
        self.assertEqual(doc["missing"]["why"], "object_bad")
        self.assertEqual(doc["missing"]["r2_key"], e["chunks"][2]["r2_key"])

    def test_a_hole_in_the_index_writes_nothing_for_that_entry(self) -> None:
        e = self.entry()
        del e["chunks"][1]
        self.write_index(e)
        result = self.fetch()
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertFalse((self.out / ENTRY).exists())

    def test_fetch_json_holds_the_last_heartbeat_and_every_heartbeat(self) -> None:
        e = self.entry()
        rows = list(e["heartbeats"])
        self.write_index(e)
        self.assertEqual(self.fetch().returncode, 0)
        _, doc = self.fetched()
        self.assertEqual(doc["heartbeats"], rows, "every row, in the index's received_at order")
        self.assertEqual(doc["last_heartbeat"], rows[-1])
        self.assertEqual(doc["last_heartbeat"]["recording"], "failed")
        self.assertEqual(doc["last_heartbeat"]["counts"], {"v": 1, "off_ms": 50, "mode": "recording"})
        self.assertEqual(doc["heartbeats"][2]["counts_dropped"], 4096)

    def test_a_run_id_fetches_every_entry_of_the_run(self) -> None:
        a, b = self.entry(ENTRY), self.entry(ENTRY_2, chunks=2)
        b["segment"] = 1
        self.write_index(a, b)
        result = self.fetch(RUN)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(result.stdout.split(), [str(self.out / ENTRY), str(self.out / ENTRY_2)])
        self.assertEqual(self.fetched(ENTRY_2)[0], b"".join(b["_bodies"]))

    def test_an_entry_that_is_already_there_is_never_overwritten(self) -> None:
        self.write_index(self.entry())
        (self.out / ENTRY).mkdir(parents=True)
        (self.out / ENTRY / "rec.gcr.zst").write_bytes(b"kept")
        result = self.fetch()
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual((self.out / ENTRY / "rec.gcr.zst").read_bytes(), b"kept")

    def test_the_fetched_files_are_private_to_the_organizer(self) -> None:
        self.write_index(self.entry())
        self.assertEqual(self.fetch().returncode, 0)
        for path in (self.out / ENTRY, *(self.out / ENTRY).iterdir()):
            self.assertEqual(path.stat().st_mode & 0o077, 0, path)
        self.assertFalse(list(self.out.glob(".fetch-run.*")), "no temporary directory is left")

    # The credentialed paths, with a fake psql and curl on PATH.

    def fakes(self) -> dict:
        (self.bin / "psql").write_text(FAKE_PSQL)
        (self.bin / "curl").write_text(FAKE_CURL)
        for name in ("psql", "curl"):
            (self.bin / name).chmod(0o755)
        return {
            "FAKE_LOG": str(self.root / "fake"), "FAKE_INDEX": str(self.index_path), "FAKE_STORE": str(self.store),
            "EXPECT_TOKEN": TOKEN, "EXPECT_ACCOUNT": "acct123", "EXPECT_BUCKET": BUCKET,
            "DATABASE_URL": DB_URL, "CLOUDFLARE_ACCOUNT_ID": "acct123", "CLOUDFLARE_API_TOKEN": TOKEN,
        }

    def test_the_database_is_only_read_and_the_id_is_a_variable(self) -> None:
        self.write_index(self.entry())
        result = self.run_script("--store", str(self.store), RUN, str(self.out), env=self.fakes())
        self.assertEqual(result.returncode, 0, result.stderr)
        sql = (self.root / "fake.psql.sql").read_text()
        self.assertTrue(sql.lstrip().upper().startswith("BEGIN READ ONLY"), sql)
        self.assertTrue(sql.rstrip().upper().endswith("ROLLBACK;"), sql)
        for word in ("INSERT", "UPDATE", "DELETE", "DROP", "ALTER", "TRUNCATE", "COMMIT", "GRANT"):
            self.assertIsNone(re.search(rf"\b{word}\b", sql.upper()), word)
        self.assertNotIn(RUN, sql, "the id is a psql variable, never spliced into the SQL")
        args = (self.root / "fake.psql.args").read_text()
        self.assertIn(f"id={RUN}", args)
        self.assertIn("staging=false", args, "the production bucket reads only production runs")
        for secret in ("db-password", "db.invalid"):
            self.assertNotIn(secret, args, "the connection is not on psql's command line")
        service = (self.root / "fake.psql.service").read_text()
        self.assertIn("password=db-password-must-never-appear", service.splitlines())
        self.assertTrue((self.root / "fake.psql.mode").exists(), "the service file is mode 600")
        self.assertFalse(list(self.out.glob(".fetch-run.*")), "and it is gone afterwards")
        self.assertTrue((self.out / ENTRY / "rec.gcr.zst").is_file())

    def test_tracing_the_script_shows_no_secret(self) -> None:
        self.write_index(self.entry())
        result = self.run_script(RUN, str(self.out), env=self.fakes(), trace=True)
        self.assertEqual(result.returncode, 0, result.stderr[-2000:])
        self.assertIn("+ ", result.stderr, "it really was traced")
        for secret in SECRETS:
            self.assertNotIn(secret, result.stdout + result.stderr)

    def test_a_missing_credential_fails_without_echoing_the_others(self) -> None:
        env = self.fakes()
        del env["CLOUDFLARE_API_TOKEN"]
        self.write_index(self.entry())
        result = self.run_script(RUN, str(self.out), env=env, trace=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("CLOUDFLARE_API_TOKEN", result.stderr)
        for secret in SECRETS:
            self.assertNotIn(secret, result.stdout + result.stderr)

    def test_r2_is_read_with_the_token_out_of_the_command_line_and_the_output(self) -> None:
        e = self.entry()
        self.write_index(e)
        result = self.run_script("--index", str(self.index_path), ENTRY, str(self.out), env=self.fakes())
        self.assertEqual(result.returncode, 0, result.stderr)
        args = (self.root / "fake.curl.args").read_text()
        self.assertEqual(args.split(), ["-K", "-"] * 3, "curl is told nothing but its configuration, once per chunk")
        for text in (result.stdout, result.stderr, args):
            for secret in SECRETS:
                self.assertNotIn(secret, text)
        self.assertEqual(self.fetched()[0], b"".join(e["_bodies"]))

    def test_an_object_r2_does_not_have_is_object_bad_but_a_refusal_writes_nothing(self) -> None:
        e = self.entry()
        self.write_index(e)
        env = {**self.fakes(), "FAKE_FORBID": e["chunks"][1]["r2_key"]}
        refused = self.run_script("--index", str(self.index_path), ENTRY, str(self.out), env=env)
        self.assertEqual(refused.returncode, 1, refused.stderr)
        self.assertFalse((self.out / ENTRY).exists(), "a refused read is not taken for a missing object")
        (self.store / e["chunks"][1]["r2_key"]).unlink()
        absent = self.run_script("--index", str(self.index_path), ENTRY, str(self.out), env=self.fakes())
        self.assertEqual(absent.returncode, 1, absent.stderr)
        self.assertEqual(self.fetched()[1]["missing"]["why"], "object_bad")

    def test_a_run_on_the_other_side_fails_and_writes_nothing(self) -> None:
        # A production run looked up with the staging bucket: the query keeps to staging, finds none.
        self.index_path.write_text("[]")
        self.entry()
        result = self.run_script("--store", str(self.store), "--bucket", "goalcup-traces-staging", RUN, str(self.out),
                                 env=self.fakes())
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("staging=true", (self.root / "fake.psql.args").read_text())
        self.assertFalse(list(self.out.rglob("rec.gcr.zst")), "nothing is written")
        self.assertFalse(list(self.out.glob(".fetch-run.*")), "no temporary directory is left")

    def test_a_staging_bucket_is_named_in_the_url(self) -> None:
        self.write_index(self.entry())
        env = {**self.fakes(), "EXPECT_BUCKET": "goalcup-traces-staging"}
        wrong = self.run_script("--index", str(self.index_path), ENTRY, str(self.out), env=env)
        self.assertEqual(wrong.returncode, 1, "the default bucket is the production one")
        right = self.run_script("--index", str(self.index_path), "--bucket", "goalcup-traces-staging", ENTRY, str(self.out), env=env)
        self.assertEqual(right.returncode, 0, right.stderr)


if __name__ == "__main__":
    unittest.main()
