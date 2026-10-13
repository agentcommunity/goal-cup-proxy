// End-to-end over HTTP against `wrangler dev` (local mode, DB_BACKEND=local, R2 simulated on disk).
// Every test in a `describe` shares one running Worker, and runs in order: the later ones continue
// the run the earlier ones built. One Worker per configuration (normal, maintenance, short-lived runs).

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { join } from "node:path";
import { after, before, describe, test } from "node:test";
import { Signer } from "../src/sign.ts";
import type { Ack, StartReceipt } from "../src/sign.ts";
import { Client, GENESIS, makeChunk, openBody, sha256, verifyAck, verifyReceipt } from "./helpers/client.ts";
import { TEST_SEED, startWorker, type RunningWorker } from "./helpers/worker.ts";

const ALICE = "tok-alice";
const BOB = "tok-bob";
const CAROL = "tok-carol";
const DEV_TOKENS = `${ALICE}:alice,${BOB}:bob,${CAROL}:carol`;
const ID_RUN = /^r[0-9a-f]{16}$/;
const ID_ENTRY = /^e[0-9a-f]{16}$/;
const TIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/;

const publicKey = (await Signer.fromSeedHex(TEST_SEED)).publicKeyHex;

/**
 * What miniflare has persisted in R2: key and sha256 of each committed object. (Reads the
 * simulator's own files, so it sees what a failed put left behind, which must be nothing.)
 */
function r2Objects(stateDir: string): { key: string; sha256: string }[] {
  const r2 = join(stateDir, "v3", "r2");
  const objects: { key: string; sha256: string }[] = [];
  const metaDir = join(r2, "miniflare-R2BucketObject");
  for (const f of readdirSync(metaDir)) {
    if (!f.endsWith(".sqlite") || f === "metadata.sqlite") continue; // one file per bucket
    const db = new DatabaseSync(join(metaDir, f), { readOnly: true });
    for (const row of db.prepare("select key, blob_id from _mf_objects").all() as { key: string; blob_id: string }[]) {
      for (const bucket of readdirSync(r2, { withFileTypes: true })) {
        if (!bucket.isDirectory() || bucket.name === "miniflare-R2BucketObject") continue;
        try {
          const bytes = readFileSync(join(r2, bucket.name, "blobs", row.blob_id));
          objects.push({ key: row.key, sha256: createHash("sha256").update(bytes).digest("hex") });
        } catch {
          // the blob is in another bucket directory
        }
      }
    }
    db.close();
  }
  return objects;
}

function assertError(r: { status: number; body: { error?: string; message?: string } }, status: number, code: string) {
  assert.equal(r.status, status, JSON.stringify(r.body));
  assert.equal(r.body.error, code);
  assert.equal(typeof r.body.message, "string");
  assert.deepEqual(Object.keys(r.body).sort(), ["error", "message"]);
}

describe("upload API against wrangler dev", { concurrency: false }, () => {
  let worker: RunningWorker;
  let api: Client;

  before(async () => {
    worker = await startWorker({ EVENT_ID: "practice", DEV_TOKENS });
    api = new Client(worker.url);
  });
  after(async () => {
    await worker?.stop();
  });

  // State carried from test to test (alice's run).
  let runId = "";
  let entryId = "";
  let receipt: StartReceipt;

  describe("authentication and routing", () => {
    test("no token and wrong token are 401", async () => {
      assertError(await api.call("POST", "/v1/runs", { json: openBody() }), 401, "unauthorized");
      assertError(await api.call("POST", "/v1/runs", { token: "nope", json: openBody() }), 401, "unauthorized");
      assertError(await api.call("GET", "/v1/runs/r0123456789abcdef", { token: "nope" }), 401, "unauthorized");
      assertError(await api.call("PUT", "/v1/runs/r0123456789abcdef/chunks/0", { token: "nope", body: new Uint8Array([1]) }), 401, "unauthorized");
    });

    test("unknown paths are 404 and wrong verbs are 405", async () => {
      assertError(await api.call("GET", "/v1/nothing", { token: ALICE }), 404, "not_found");
      assertError(await api.call("GET", "/v1/runs/not-an-id", { token: ALICE }), 404, "not_found");
      assertError(await api.call("GET", "/v1/runs", { token: ALICE }), 405, "method_not_allowed");
    });
  });

  describe("open a run", () => {
    test("rejects bad bodies with 400", async () => {
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: openBody({ repo_url: "https://gitlab.com/a/b" }) }), 400, "malformed");
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: openBody({ repo_url: "https://github.com/a/b/" }) }), 400, "malformed");
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: openBody({ manifest_sha256: "ABC" }) }), 400, "malformed");
      const { workspace: _w, ...missing } = openBody();
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: missing }), 400, "malformed");
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, body: new TextEncoder().encode("{not json") }), 400, "malformed");
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: [] }), 400, "malformed");
    });

    test("201 with server-minted ids and times", async () => {
      const before = Date.now();
      const r = await api.call("POST", "/v1/runs", { token: ALICE, json: openBody() });
      assert.equal(r.status, 201, JSON.stringify(r.body));
      assert.deepEqual(Object.keys(r.body).sort(), ["deadline_at", "entry_id", "official_start", "run_id"]);
      assert.match(r.body.run_id, ID_RUN);
      assert.match(r.body.entry_id, ID_ENTRY);
      assert.match(r.body.official_start, TIME);
      assert.match(r.body.deadline_at, TIME);
      const start = Date.parse(r.body.official_start);
      assert.ok(Math.abs(start - before) < 30_000, "official_start is the server's clock");
      assert.equal(Date.parse(r.body.deadline_at) - start, 168 * 3600 * 1000);
      runId = r.body.run_id;
      entryId = r.body.entry_id;
    });

    test("a repeat of the same session_id is 200 with the same answer", async () => {
      const first = await api.call("GET", `/v1/runs/${runId}`, { token: ALICE });
      const r = await api.call("POST", "/v1/runs", { token: ALICE, json: openBody() });
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(r.body, { run_id: runId, entry_id: entryId, official_start: first.body.official_start, deadline_at: first.body.deadline_at });
    });

    test("a different session_id while a run is open is 409", async () => {
      const r = await api.call("POST", "/v1/runs", { token: ALICE, json: openBody({ session_id: "another-session" }) });
      assertError(r, 409, "conflict");
    });

    test("another member is independent, and cannot see this run (404)", async () => {
      const r = await api.call("POST", "/v1/runs", { token: BOB, json: openBody({ session_id: "bobs-session" }) });
      assert.equal(r.status, 201);
      assert.notEqual(r.body.run_id, runId);
      assertError(await api.call("GET", `/v1/runs/${runId}`, { token: BOB }), 404, "not_found");
      assertError(await api.call("POST", `/v1/runs/${runId}/abandon`, { token: BOB, json: { reason: "x" } }), 404, "not_found");
      assertError(await api.call("PUT", `/v1/runs/${runId}/chunks/0`, { token: BOB, body: new Uint8Array([1]), headers: makeChunk(entryId, 1, 1, GENESIS, "x").headers }), 404, "not_found");
    });
  });

  describe("start receipt", () => {
    test("200 with a signed receipt in the verifier's format", async () => {
      const r = await api.call("POST", `/v1/runs/${runId}/receipt`, { token: ALICE, json: { entry_id: entryId, chain_head: "b".repeat(64) } });
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(Object.keys(r.body), ["start_receipt"]);
      receipt = r.body.start_receipt;
      assert.deepEqual(Object.keys(receipt), ["claims", "public_key", "signature"]);
      assert.deepEqual(Object.keys(receipt.claims), [
        "entry_id", "connector_id", "workspace", "session_id", "started_at", "deadline_at",
        "manifest_sha256", "chain_head", "connector_dir_sha256",
      ]);
      const pos = await api.call("GET", `/v1/runs/${runId}`, { token: ALICE });
      assert.equal(receipt.claims.entry_id, entryId);
      assert.equal(receipt.claims.started_at, pos.body.official_start);
      assert.equal(receipt.claims.deadline_at, pos.body.deadline_at);
      assert.equal(receipt.claims.chain_head, "b".repeat(64));
      assert.equal(receipt.claims.workspace, openBody().workspace);
      assert.equal(receipt.claims.session_id, openBody().session_id);
      assert.equal(receipt.claims.connector_dir_sha256, "c".repeat(64));
      assert.equal(receipt.public_key, publicKey);
      assert.equal(await verifyReceipt(receipt, publicKey), true);
    });

    test("a repeat with the same chain_head returns the stored receipt; another chain_head is 409", async () => {
      const again = await api.call("POST", `/v1/runs/${runId}/receipt`, { token: ALICE, json: { entry_id: entryId, chain_head: "b".repeat(64) } });
      assert.equal(again.status, 200);
      assert.deepEqual(again.body.start_receipt, receipt);
      assertError(await api.call("POST", `/v1/runs/${runId}/receipt`, { token: ALICE, json: { entry_id: entryId, chain_head: "d".repeat(64) } }), 409, "conflict");
    });

    test("wrong entry is 404, malformed fields are 400", async () => {
      assertError(await api.call("POST", `/v1/runs/${runId}/receipt`, { token: ALICE, json: { entry_id: "e" + "1".repeat(16), chain_head: "b".repeat(64) } }), 404, "not_found");
      assertError(await api.call("POST", `/v1/runs/${runId}/receipt`, { token: ALICE, json: { entry_id: entryId, chain_head: "xyz" } }), 400, "malformed");
      assertError(await api.call("POST", `/v1/runs/${runId}/receipt`, { token: ALICE, json: { entry_id: "nope", chain_head: "b".repeat(64) } }), 400, "malformed");
    });

    test("once a receipt exists, repeating the open call is a 409 (only a receipt-less run is returned again)", async () => {
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: openBody() }), 409, "conflict");
    });
  });

  describe("position", () => {
    test("a fresh run: offset 0, next_seq 1, genesis head, no ack", async () => {
      const r = await api.call("GET", `/v1/runs/${runId}`, { token: ALICE });
      assert.equal(r.status, 200);
      assert.deepEqual(r.body, {
        run_id: runId, entry_id: entryId, status: "open",
        official_start: r.body.official_start, deadline_at: r.body.deadline_at,
        offset: 0, next_seq: 1, head: GENESIS, last_ack: null,
      });
    });
  });

  describe("chunks", () => {
    const chunk1 = () => makeChunk(entryId, 1, 5, GENESIS, "one");
    let c1: ReturnType<typeof chunk1>;
    let ack1: Ack;

    const put = (c: { body: Uint8Array; headers: Record<string, string> }, offset: number, token = ALICE) =>
      api.call("PUT", `/v1/runs/${runId}/chunks/${offset}`, { token, body: c.body, headers: c.headers });
    const position = async () => (await api.call("GET", `/v1/runs/${runId}`, { token: ALICE })).body;

    test("refused before the body is read: bad headers, size, content type", async () => {
      const c = chunk1();
      const without = (h: string) => {
        const { [h]: _gone, ...rest } = c.headers;
        return { body: c.body, headers: rest };
      };
      for (const h of ["goalcup-entry-id", "goalcup-length", "goalcup-sha256", "goalcup-first-seq", "goalcup-last-seq", "goalcup-prev", "goalcup-head"]) {
        assertError(await put(without(h), 0), 400, "malformed");
      }
      assertError(await put({ body: c.body, headers: { ...c.headers, "goalcup-head": "XYZ" } }, 0), 400, "malformed");
      assertError(await put({ body: c.body, headers: { ...c.headers, "goalcup-first-seq": "5", "goalcup-last-seq": "4" } }, 0), 400, "malformed");
      assertError(await put({ body: c.body, headers: { ...c.headers, "content-type": "text/plain" } }, 0), 400, "malformed");
      assertError(await api.call("PUT", `/v1/runs/${runId}/chunks/abc`, { token: ALICE, body: c.body, headers: c.headers }), 400, "malformed");
      assertError(await api.call("PUT", `/v1/runs/${runId}/chunks/-1`, { token: ALICE, body: c.body, headers: c.headers }), 400, "malformed");
      // Entry that is not this run's: 404.
      assertError(await put({ body: c.body, headers: { ...c.headers, "goalcup-entry-id": "e" + "2".repeat(16) } }, 0), 404, "not_found");
      assert.deepEqual((await position()).offset, 0);
    });

    test("no Content-Length is 400; a body one byte over 32 MiB is 413 and is not stored", async () => {
      const c = chunk1();
      // Chunked transfer: the request carries no Content-Length.
      const chunked = await api.raw("PUT", `/v1/runs/${runId}/chunks/0`, { ...c.headers, authorization: `Bearer ${ALICE}`, "transfer-encoding": "chunked" }, [c.body]);
      assertError(chunked, 400, "malformed");
      // One byte over the cap. (The Worker answers before reading it; the client still sends it.)
      const over = Buffer.alloc(33_554_433, 7);
      const huge = await api.raw("PUT", `/v1/runs/${runId}/chunks/0`, { ...c.headers, authorization: `Bearer ${ALICE}`, "content-length": String(over.length) }, [over]);
      assertError(huge, 413, "too_large");
      assert.deepEqual((await position()).offset, 0);
    });

    test("first chunk: 200 with a signed ack and the new position", async () => {
      c1 = chunk1();
      const r = await put(c1, 0);
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(Object.keys(r.body), ["ack", "offset", "next_seq", "head"]);
      ack1 = r.body.ack;
      assert.deepEqual(Object.keys(ack1), ["entry_id", "seq", "hash", "acked_at", "server_signature"]);
      assert.equal(ack1.entry_id, entryId);
      assert.equal(ack1.seq, c1.lastSeq);
      assert.equal(ack1.hash, c1.head);
      assert.match(ack1.acked_at, TIME);
      assert.equal(await verifyAck(ack1, publicKey), true);
      assert.equal(r.body.offset, c1.length);
      assert.equal(r.body.next_seq, c1.lastSeq + 1);
      assert.equal(r.body.head, c1.head);
      const p = await position();
      assert.equal(p.offset, c1.length);
      assert.equal(p.next_seq, 6);
      assert.equal(p.head, c1.head);
      assert.deepEqual(p.last_ack, ack1);
    });

    test("a retry of the same chunk is 200 with the very same stored ack", async () => {
      await new Promise((r) => setTimeout(r, 1100)); // a re-signed ack would carry a later acked_at
      const r = await put(c1, 0);
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(r.body.ack, ack1);
      assert.equal(r.body.offset, c1.length);
      assert.equal(r.body.next_seq, 6);
      assert.equal(r.body.head, c1.head);
      assert.equal((await position()).offset, c1.length);
    });

    test("the same offset with a different length or head is 409", async () => {
      assertError(await put({ body: c1.body, headers: { ...c1.headers, "goalcup-length": String(c1.length + 1) } }, 0), 409, "conflict");
      assertError(await put({ body: c1.body, headers: { ...c1.headers, "goalcup-head": "f".repeat(64) } }, 0), 409, "conflict");
      assertError(await put(makeChunk(entryId, 1, 5, GENESIS, "other"), 0), 409, "conflict");
    });

    test("wrong offset, prev or first_seq is 409 and nothing moves", async () => {
      const good = makeChunk(entryId, 6, 3, c1.head, "two");
      assertError(await put(good, c1.length + 10), 409, "conflict"); // a gap
      assertError(await put({ body: good.body, headers: { ...good.headers, "goalcup-prev": "9".repeat(64) } }, c1.length), 409, "conflict");
      assertError(await put({ body: good.body, headers: { ...good.headers, "goalcup-first-seq": "7", "goalcup-last-seq": "9" } }, c1.length), 409, "conflict");
      const p = await position();
      assert.equal(p.offset, c1.length);
      assert.equal(p.next_seq, 6);
      assert.equal(p.head, c1.head);
    });

    test("a body that does not match Goalcup-Sha256 is 422, nothing is committed, and the right body then succeeds", async () => {
      const good = makeChunk(entryId, 6, 3, c1.head, "two");
      const tampered = Buffer.from(good.body);
      tampered[tampered.length - 1] = tampered[tampered.length - 1]! ^ 0xff; // same length, different bytes
      assert.equal(tampered.length, good.body.length);
      const bad = await put({ body: tampered, headers: good.headers }, c1.length);
      assertError(bad, 422, "digest_mismatch");
      // Nothing of the tampered upload reached storage, so the name it claimed is still free.
      const traceKeys = () => r2Objects(worker.stateDir).filter((o) => o.key.startsWith("traces/"));
      assert.ok(!traceKeys().some((o) => o.key.includes(good.headers["goalcup-sha256"]!)), "the claimed name exists in storage");
      assert.ok(!traceKeys().some((o) => o.sha256 === sha256(tampered)), "tampered bytes were stored");
      let p = await position();
      assert.equal(p.offset, c1.length);
      assert.equal(p.next_seq, 6);
      assert.equal(p.head, c1.head);
      assert.deepEqual(p.last_ack, ack1);
      // A shorter body than Content-Length cannot happen over HTTP, but a *different* body of a
      // different size with the old hash is a mismatch too.
      assertError(await put({ body: Buffer.concat([good.body, Buffer.from([0])]), headers: good.headers }, c1.length), 422, "digest_mismatch");

      const ok = await put(good, c1.length);
      assert.equal(ok.status, 200, JSON.stringify(ok.body));
      assert.equal(ok.body.offset, c1.length + good.length);
      assert.equal(ok.body.next_seq, 9);
      p = await position();
      assert.equal(p.head, good.head);
      assert.equal(await verifyAck(ok.body.ack, publicKey), true);
      // The stored object is exactly the verified body, once.
      const goodKey = `traces/practice/${runId}/${entryId}/${String(c1.length).padStart(16, "0")}-${good.headers["goalcup-sha256"]}.zst`;
      assert.deepEqual(traceKeys().filter((o) => o.key === goodKey).map((o) => o.sha256), [good.headers["goalcup-sha256"]]);
    });

    test("two identical uploads at once: both 200 with one ack, and the run advanced once", async () => {
      const before = await position();
      const next = makeChunk(entryId, before.next_seq, 4, before.head, "three");
      const [a, b] = await Promise.all([put(next, before.offset), put(next, before.offset)]);
      assert.equal(a.status, 200, JSON.stringify(a.body));
      assert.equal(b.status, 200, JSON.stringify(b.body));
      assert.deepEqual(a.body.ack, b.body.ack);
      const p = await position();
      assert.equal(p.offset, before.offset + next.length);
      assert.equal(p.next_seq, before.next_seq + 4);
    });

    test("an old chunk can still be retried after the run moved on", async () => {
      const r = await put(c1, 0);
      assert.equal(r.status, 200);
      assert.deepEqual(r.body.ack, ack1);
    });

    test("abandon is refused once a chunk is stored", async () => {
      assertError(await api.call("POST", `/v1/runs/${runId}/abandon`, { token: ALICE, json: { reason: "too late" } }), 409, "conflict");
      assert.equal((await position()).status, "open");
    });
  });

  describe("a chunk of exactly 32 MiB", () => {
    test("is streamed into storage and accepted (bob's run)", async () => {
      const bobRun = (await api.call("POST", "/v1/runs", { token: BOB, json: openBody({ session_id: "bobs-session" }) })).body;
      assert.equal(bobRun.entry_id.length, 17);
      // Not a real zstd frame: the Worker checks headers and the hash, not the contents.
      const body = Buffer.alloc(33_554_432);
      for (let i = 0; i < body.length; i += 4096) body.writeUInt32LE(i / 4096, i);
      const headers = {
        "content-type": "application/zstd",
        "goalcup-entry-id": bobRun.entry_id,
        "goalcup-length": "100000000",
        "goalcup-sha256": sha256(body),
        "goalcup-first-seq": "1",
        "goalcup-last-seq": "2000",
        "goalcup-prev": GENESIS,
        "goalcup-head": "a".repeat(64),
      };
      const r = await api.call("PUT", `/v1/runs/${bobRun.run_id}/chunks/0`, { token: BOB, body, headers });
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.equal(r.body.offset, 100_000_000);
      assert.equal(r.body.next_seq, 2001);
      const p = await api.call("GET", `/v1/runs/${bobRun.run_id}`, { token: BOB });
      assert.equal(p.body.offset, 100_000_000);
    });
  });

  describe("abandon", () => {
    test("before any chunk: 200, repeatable, then the run is closed and the account is free again", async () => {
      const open = await api.call("POST", "/v1/runs", { token: CAROL, json: openBody({ session_id: "carols-session" }) });
      assert.equal(open.status, 201);
      const id = open.body.run_id;
      assertError(await api.call("POST", `/v1/runs/${id}/abandon`, { token: CAROL, json: { reason: "x".repeat(501) } }), 400, "malformed");
      assertError(await api.call("POST", `/v1/runs/${id}/abandon`, { token: CAROL, json: {} }), 400, "malformed");
      const r = await api.call("POST", `/v1/runs/${id}/abandon`, { token: CAROL, json: { reason: "the daemon refused to start" } });
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(r.body, { status: "abandoned" });
      assert.equal((await api.call("POST", `/v1/runs/${id}/abandon`, { token: CAROL, json: { reason: "again" } })).status, 200);
      assert.equal((await api.call("GET", `/v1/runs/${id}`, { token: CAROL })).body.status, "abandoned");

      const c = makeChunk(open.body.entry_id, 1, 2, GENESIS, "late");
      assertError(await api.call("PUT", `/v1/runs/${id}/chunks/0`, { token: CAROL, body: c.body, headers: c.headers }), 410, "closed");
      assertError(await api.call("POST", `/v1/runs/${id}/receipt`, { token: CAROL, json: { entry_id: open.body.entry_id, chain_head: "b".repeat(64) } }), 410, "closed");

      const next = await api.call("POST", "/v1/runs", { token: CAROL, json: openBody({ session_id: "carols-session" }) });
      assert.equal(next.status, 201, JSON.stringify(next.body));
      assert.notEqual(next.body.run_id, id);
    });
  });

  describe("seal", () => {
    const docs = { seal_json: '{"sealed":true,"n":1}', baseline_json: '{"baseline":"ünï"}' };

    test("malformed bodies are 400; the wrong entry is 404", async () => {
      assertError(await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: entryId, seal_json: "", baseline_json: "{}" } }), 400, "malformed");
      assertError(await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: entryId, seal_json: 5, baseline_json: "{}" } }), 400, "malformed");
      assertError(await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: "bad", ...docs } }), 400, "malformed");
      assertError(await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: "e" + "3".repeat(16), ...docs } }), 404, "not_found");
    });

    test("200, a repeat with the same documents is 200, different documents are 409", async () => {
      const r = await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: entryId, ...docs } });
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(r.body, { status: "sealed" });
      const again = await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: entryId, ...docs } });
      assert.equal(again.status, 200);
      assert.deepEqual(again.body, { status: "sealed" });
      assertError(await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: entryId, seal_json: '{"sealed":false}', baseline_json: docs.baseline_json } }), 409, "conflict");
      assertError(await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: entryId, seal_json: docs.seal_json, baseline_json: "{}" } }), 409, "conflict");
    });

    test("after the seal the run is closed (410) and still readable", async () => {
      const p = await api.call("GET", `/v1/runs/${runId}`, { token: ALICE });
      assert.equal(p.body.status, "sealed");
      const c = makeChunk(entryId, p.body.next_seq, 1, p.body.head, "after");
      assertError(await api.call("PUT", `/v1/runs/${runId}/chunks/${p.body.offset}`, { token: ALICE, body: c.body, headers: c.headers }), 410, "closed");
      assertError(await api.call("POST", `/v1/runs/${runId}/abandon`, { token: ALICE, json: { reason: "x" } }), 410, "closed");
    });

    test("the account is free again after the seal", async () => {
      const r = await api.call("POST", "/v1/runs", { token: ALICE, json: openBody({ session_id: "second-run" }) });
      assert.equal(r.status, 201, JSON.stringify(r.body));
      assert.notEqual(r.body.run_id, runId);
    });
  });
});

describe("maintenance switch", () => {
  let worker: RunningWorker;
  before(async () => {
    worker = await startWorker({ EVENT_ID: "practice", DEV_TOKENS, MAINTENANCE: "on", MAINTENANCE_RETRY_AFTER: "120" });
  });
  after(async () => {
    await worker?.stop();
  });

  test("every call is 503 with Retry-After, before authentication and before the body", async () => {
    const api = new Client(worker.url);
    const c = makeChunk("e" + "1".repeat(16), 1, 1, GENESIS, "m");
    const calls = [
      api.call("POST", "/v1/runs", { token: ALICE, json: openBody() }),
      api.call("POST", "/v1/runs", { json: openBody() }),
      api.call("GET", "/v1/runs/r0123456789abcdef", { token: ALICE }),
      api.call("PUT", "/v1/runs/r0123456789abcdef/chunks/0", { token: ALICE, body: c.body, headers: c.headers }),
      api.call("POST", "/v1/runs/r0123456789abcdef/seal", { token: ALICE, json: {} }),
    ];
    for (const r of await Promise.all(calls)) {
      assertError(r, 503, "maintenance");
      assert.equal(r.headers.get("retry-after"), "120");
    }
  });
});

describe("a run past its deadline and grace period", () => {
  let worker: RunningWorker;
  before(async () => {
    // A 2-second run with no grace.
    worker = await startWorker({ EVENT_ID: "practice", DEV_TOKENS, RUN_HOURS: "0.0006", GRACE_HOURS: "0" });
  });
  after(async () => {
    await worker?.stop();
  });

  test("uploads, receipts and seals are 410 once it has expired", async () => {
    const api = new Client(worker.url);
    const open = await api.call("POST", "/v1/runs", { token: ALICE, json: openBody() });
    assert.equal(open.status, 201, JSON.stringify(open.body));
    const { run_id, entry_id } = open.body;
    assert.equal(Date.parse(open.body.deadline_at) - Date.parse(open.body.official_start), 2000);

    await new Promise((r) => setTimeout(r, 3200));
    const c = makeChunk(entry_id, 1, 2, GENESIS, "late");
    assertError(await api.call("PUT", `/v1/runs/${run_id}/chunks/0`, { token: ALICE, body: c.body, headers: c.headers }), 410, "closed");
    assertError(await api.call("POST", `/v1/runs/${run_id}/receipt`, { token: ALICE, json: { entry_id, chain_head: "b".repeat(64) } }), 410, "closed");
    assertError(await api.call("POST", `/v1/runs/${run_id}/seal`, { token: ALICE, json: { entry_id, seal_json: "{}", baseline_json: "{}" } }), 410, "closed");
    assert.equal((await api.call("GET", `/v1/runs/${run_id}`, { token: ALICE })).status, 200);
  });
});

