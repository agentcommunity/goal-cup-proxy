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
import { APPROVAL_KEY, Client, GENESIS, credentialFor, makeChunk, openBody, sha256, verifyAck, verifyReceipt } from "./helpers/client.ts";
import { TEST_SEED, startWorker, type RunningWorker } from "./helpers/worker.ts";

// Run credentials, minted through the device flow in each describe's `before`.
let ALICE = "";
let BOB = "";
let CAROL = "";
let DAVE = "";
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
    worker = await startWorker({ EVENT_ID: "practice", TEST_APPROVAL_KEY: APPROVAL_KEY });
    api = new Client(worker.url);
    [ALICE, BOB, CAROL] = [await credentialFor(api, "alice"), await credentialFor(api, "bob"), await credentialFor(api, "carol")];
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
      assert.deepEqual(Object.keys(r.body).sort(), ["deadline_at", "entry_id", "official_start", "run_id", "seconds_left"]);
      assert.match(r.body.run_id, ID_RUN);
      assert.match(r.body.entry_id, ID_ENTRY);
      assert.match(r.body.official_start, TIME);
      assert.match(r.body.deadline_at, TIME);
      const start = Date.parse(r.body.official_start);
      assert.ok(Math.abs(start - before) < 30_000, "official_start is the server's clock");
      assert.equal(Date.parse(r.body.deadline_at) - start, 168 * 3600 * 1000);
      // The time left, on the server's clock: the whole run, less the second or two this call took.
      assert.ok(Number.isSafeInteger(r.body.seconds_left), JSON.stringify(r.body));
      assert.ok(r.body.seconds_left <= 168 * 3600 && r.body.seconds_left > 168 * 3600 - 30, JSON.stringify(r.body));
      runId = r.body.run_id;
      entryId = r.body.entry_id;
    });

    test("a repeat of the same session_id is 200 with the same answer", async () => {
      const first = await api.call("GET", `/v1/runs/${runId}`, { token: ALICE });
      const r = await api.call("POST", "/v1/runs", { token: ALICE, json: openBody() });
      assert.equal(r.status, 200, JSON.stringify(r.body));
      const { seconds_left, ...rest } = r.body;
      assert.deepEqual(rest, { run_id: runId, entry_id: entryId, official_start: first.body.official_start, deadline_at: first.body.deadline_at });
      assert.ok(seconds_left > 0 && seconds_left <= 168 * 3600, JSON.stringify(r.body));
    });

    test("a different session_id on a credential that opened a run is 409", async () => {
      const r = await api.call("POST", "/v1/runs", { token: ALICE, json: openBody({ session_id: "another-session" }) });
      assertError(r, 409, "conflict");
    });

    test("another member is independent, and cannot see this run (404)", async () => {
      const r = await api.call("POST", "/v1/runs", { token: BOB, json: openBody({ session_id: "bobs-session" }) });
      assert.equal(r.status, 201);
      assert.notEqual(r.body.run_id, runId);
      assertError(await api.call("GET", `/v1/runs/${runId}`, { token: BOB }), 404, "not_found");
      assertError(await api.call("POST", `/v1/runs/${runId}/abandon`, { token: BOB, json: { reason: "x" } }), 404, "not_found");
      assertError(await api.call("PUT", `/v1/runs/${runId}/chunks/0`, { token: BOB, body: new Uint8Array([1]), headers: makeChunk(entryId, 0, 1, GENESIS, "x").headers }), 404, "not_found");
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

    test("once a receipt exists, repeating the open call is still 200 with the same run", async () => {
      const r = await api.call("POST", "/v1/runs", { token: ALICE, json: openBody() });
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.equal(r.body.run_id, runId);
    });
  });

  describe("position", () => {
    test("a fresh run: offset 0, next_seq 0, genesis head, no ack", async () => {
      const r = await api.call("GET", `/v1/runs/${runId}`, { token: ALICE });
      assert.equal(r.status, 200);
      assert.deepEqual(r.body, {
        run_id: runId, entry_id: entryId, segment: 0, status: "open",
        official_start: r.body.official_start, deadline_at: r.body.deadline_at, ends_at: r.body.deadline_at,
        offset: 0, next_seq: 0, head: GENESIS, last_ack: null, stop: false, final: null,
      });
    });
  });

  describe("heartbeat", () => {
    const beat = (extra: Record<string, unknown> = {}, id = runId, entry = entryId, token = ALICE) =>
      api.call("POST", `/v1/runs/${id}/heartbeat`, {
        token,
        json: { entry_id: entry, seq: 0, head: GENESIS, offset_local: 0, final: false, repo_head: null, ...extra },
      });

    test("an ordinary heartbeat is 200 with stop false and the deadline", async () => {
      const r = await beat();
      assert.equal(r.status, 200, JSON.stringify(r.body));
      const pos = await api.call("GET", `/v1/runs/${runId}`, { token: ALICE });
      const { seconds_left, ...rest } = r.body;
      assert.deepEqual(rest, { stop: false, deadline_at: pos.body.deadline_at, reason: null });
      assert.ok(seconds_left > 168 * 3600 - 300 && seconds_left <= 168 * 3600, JSON.stringify(r.body));
    });

    test("bad fields are 400, a foreign entry 404, another member's run 404, no token 401", async () => {
      for (const bad of [{ seq: -1 }, { seq: 1.5 }, { seq: "1" }, { head: "xyz" }, { offset_local: null }, { final: "no" }, { repo_head: "abc" }]) {
        assertError(await beat(bad), 400, "malformed");
      }
      assertError(await beat({}, runId, "e" + "5".repeat(16)), 404, "not_found");
      assertError(await beat({}, runId, entryId, BOB), 404, "not_found");
      assertError(await api.call("POST", `/v1/runs/${runId}/heartbeat`, { json: {} }), 401, "unauthorized");
      assertError(await api.call("GET", `/v1/runs/${runId}/heartbeat`, { token: ALICE }), 405, "method_not_allowed");
    });
  });

  describe("chunks", () => {
    const chunk1 = () => makeChunk(entryId, 0, 5, GENESIS, "one");
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
      assert.equal(p.next_seq, 5);
      assert.equal(p.head, c1.head);
      assert.deepEqual(p.last_ack, ack1);
    });

    test("a retry of the same chunk is 200 with the very same stored ack", async () => {
      await new Promise((r) => setTimeout(r, 1100)); // a re-signed ack would carry a later acked_at
      const r = await put(c1, 0);
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(r.body.ack, ack1);
      assert.equal(r.body.offset, c1.length);
      assert.equal(r.body.next_seq, 5);
      assert.equal(r.body.head, c1.head);
      assert.equal((await position()).offset, c1.length);
    });

    test("the same offset with a different length or head is 409", async () => {
      assertError(await put({ body: c1.body, headers: { ...c1.headers, "goalcup-length": String(c1.length + 1) } }, 0), 409, "conflict");
      assertError(await put({ body: c1.body, headers: { ...c1.headers, "goalcup-head": "f".repeat(64) } }, 0), 409, "conflict");
      assertError(await put(makeChunk(entryId, 0, 5, GENESIS, "other"), 0), 409, "conflict");
    });

    test("wrong offset, prev or first_seq is 409 and nothing moves", async () => {
      const good = makeChunk(entryId, 5, 3, c1.head, "two");
      assertError(await put(good, c1.length + 10), 409, "conflict"); // a gap
      assertError(await put({ body: good.body, headers: { ...good.headers, "goalcup-prev": "9".repeat(64) } }, c1.length), 409, "conflict");
      assertError(await put({ body: good.body, headers: { ...good.headers, "goalcup-first-seq": "6", "goalcup-last-seq": "8" } }, c1.length), 409, "conflict");
      const p = await position();
      assert.equal(p.offset, c1.length);
      assert.equal(p.next_seq, 5);
      assert.equal(p.head, c1.head);
    });

    test("a body that does not match Goalcup-Sha256 is 422, nothing is committed, and the right body then succeeds", async () => {
      const good = makeChunk(entryId, 5, 3, c1.head, "two");
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
      assert.equal(p.next_seq, 5);
      assert.equal(p.head, c1.head);
      assert.deepEqual(p.last_ack, ack1);
      // A shorter body than Content-Length cannot happen over HTTP, but a *different* body of a
      // different size with the old hash is a mismatch too.
      assertError(await put({ body: Buffer.concat([good.body, Buffer.from([0])]), headers: good.headers }, c1.length), 422, "digest_mismatch");

      const ok = await put(good, c1.length);
      assert.equal(ok.status, 200, JSON.stringify(ok.body));
      assert.equal(ok.body.offset, c1.length + good.length);
      assert.equal(ok.body.next_seq, 8);
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
        "goalcup-first-seq": "0",
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

      // The credential stays with the run it opened; a new approval opens the next one.
      const next = await api.call("POST", "/v1/runs", { token: await credentialFor(api, "carol"), json: openBody({ session_id: "carols-session" }) });
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

    test("a seal needs the final heartbeat first, and the chain held up to it", async () => {
      const p = (await api.call("GET", `/v1/runs/${runId}`, { token: ALICE })).body;
      const noFinal = await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: entryId, ...docs } });
      assertError(noFinal, 409, "conflict");
      assert.match(noFinal.body.message, /final heartbeat/);
      // An end the server does not hold the chain up to yet.
      const beat = (seq: number, head: string) =>
        api.call("POST", `/v1/runs/${runId}/heartbeat`, { token: ALICE, json: { entry_id: entryId, seq, head, offset_local: 1, final: true, repo_head: null } });
      assert.equal((await beat(p.next_seq + 3, "d".repeat(64))).status, 200);
      assertError(await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: entryId, ...docs } }), 409, "conflict");
    });

    test("200, a repeat with the same documents is 200, different documents are 409", async () => {
      // A fresh run whose end is committed where the server holds the chain.
      const token = await credentialFor(api, "dora");
      const run = (await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: "doras-session" }) })).body;
      const c = makeChunk(run.entry_id, 0, 3, GENESIS, "dora");
      assert.equal((await api.call("PUT", `/v1/runs/${run.run_id}/chunks/0`, { token, body: c.body, headers: c.headers })).status, 200);
      const final = await api.call("POST", `/v1/runs/${run.run_id}/heartbeat`, { token, json: { entry_id: run.entry_id, seq: c.lastSeq, head: c.head, offset_local: 1, final: true, repo_head: null } });
      assert.equal(final.status, 200, JSON.stringify(final.body));
      const seal = (body: Record<string, unknown>) => api.call("POST", `/v1/runs/${run.run_id}/seal`, { token, json: { entry_id: run.entry_id, ...body } });

      const r = await seal(docs);
      assert.equal(r.status, 200, JSON.stringify(r.body));
      // Before the run's end, a sealed entry leaves the run open.
      assert.deepEqual(r.body, { status: "sealed", run_status: "open" });
      const again = await seal(docs);
      assert.equal(again.status, 200);
      assert.deepEqual(again.body, { status: "sealed", run_status: "open" });
      assertError(await seal({ seal_json: '{"sealed":false}', baseline_json: docs.baseline_json }), 409, "conflict");
      assertError(await seal({ seal_json: docs.seal_json, baseline_json: "{}" }), 409, "conflict");

      // The sealed entry takes no more, and the run (open until its end) is still readable.
      const p = await api.call("GET", `/v1/runs/${run.run_id}`, { token });
      assert.equal(p.body.status, "open");
      const late = makeChunk(run.entry_id, p.body.next_seq, 1, p.body.head, "after");
      assertError(await api.call("PUT", `/v1/runs/${run.run_id}/chunks/${p.body.offset}`, { token, body: late.body, headers: late.headers }), 410, "closed");
      assertError(await api.call("POST", `/v1/runs/${run.run_id}/abandon`, { token, json: { reason: "x" } }), 409, "conflict"); // it holds chunks
    });

    test("a later start by the same account is a new run", async () => {
      const r = await api.call("POST", "/v1/runs", { token: await credentialFor(api, "alice"), json: openBody({ session_id: "second-run" }) });
      assert.equal(r.status, 201, JSON.stringify(r.body));
      assert.notEqual(r.body.run_id, runId);
    });
  });
});

describe("maintenance switch", () => {
  let worker: RunningWorker;
  before(async () => {
    worker = await startWorker({ EVENT_ID: "practice", MAINTENANCE: "on", MAINTENANCE_RETRY_AFTER: "120" });
  });
  after(async () => {
    await worker?.stop();
  });

  test("every call is 503 with Retry-After, before authentication and before the body", async () => {
    const api = new Client(worker.url);
    const c = makeChunk("e" + "1".repeat(16), 1, 1, GENESIS, "m");
    const calls = [
      api.call("POST", "/v1/device", { json: { purpose: "run", mode: "practice", connector_id: "x", tracer_version: "1" } }),
      api.call("POST", "/v1/device/token", { json: { device_code: "a".repeat(64) } }),
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


// ---- the deadline --------------------------------------------------------------------------

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
const sleepUntil = (iso: string) => sleep(Math.max(0, Date.parse(iso) + 400 - Date.now()));

describe("the deadline: stop, final heartbeat, no late window", { concurrency: false }, () => {
  let worker: RunningWorker;
  let api: Client;
  interface R {
    id: string;
    entry: string;
    deadline: string;
  }
  let alice: R, bob: R, carol: R, dave: R;

  const open = async (token: string): Promise<R> => {
    const r = await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: `s-${token}` }) });
    assert.equal(r.status, 201, JSON.stringify(r.body));
    return { id: r.body.run_id, entry: r.body.entry_id, deadline: r.body.deadline_at };
  };
  const put = (r: R, token: string, c: { body: Uint8Array; headers: Record<string, string> }, offset: number) =>
    api.call("PUT", `/v1/runs/${r.id}/chunks/${offset}`, { token, body: c.body, headers: c.headers });
  const beat = (r: R, token: string, seq: number, head: string, final: boolean, repo_head: string | null = null) =>
    api.call("POST", `/v1/runs/${r.id}/heartbeat`, { token, json: { entry_id: r.entry, seq, head, offset_local: 1, final, repo_head } });
  const get = (r: R, token: string) => api.call("GET", `/v1/runs/${r.id}`, { token });
  const seal = (r: R, token: string) =>
    api.call("POST", `/v1/runs/${r.id}/seal`, { token, json: { entry_id: r.entry, seal_json: '{"s":1}', baseline_json: '{"b":1}' } });

  // alice's chain: 0..4, then 5..7, then 8..9 (the committed end), plus two chunks that must be refused.
  let a1: ReturnType<typeof makeChunk>, a2: typeof a1, a3: typeof a1, aOver: typeof a1, aWrongHead: typeof a1;
  let carolC: typeof a1;

  before(async () => {
    // Every run lasts 6 seconds.
    worker = await startWorker({ EVENT_ID: "practice", TEST_APPROVAL_KEY: APPROVAL_KEY, RUN_SECONDS: "6" });
    api = new Client(worker.url);
    [ALICE, BOB, CAROL, DAVE] = [await credentialFor(api, "alice"), await credentialFor(api, "bob"), await credentialFor(api, "carol"), await credentialFor(api, "dave")];
    [alice, bob, carol, dave] = [await open(ALICE), await open(BOB), await open(CAROL), await open(DAVE)];
    assert.equal(Date.parse(alice.deadline) - Date.parse((await get(alice, ALICE)).body.official_start), 6000);
    a1 = makeChunk(alice.entry, 0, 5, GENESIS, "a1");
    a2 = makeChunk(alice.entry, 5, 3, a1.head, "a2");
    a3 = makeChunk(alice.entry, 8, 2, a2.head, "a3");
    aOver = makeChunk(alice.entry, 8, 3, a2.head, "a3-over");
    aWrongHead = makeChunk(alice.entry, 8, 2, a2.head, "a3-wrong");
    carolC = makeChunk(carol.entry, 0, 5, GENESIS, "c1");
  });
  after(async () => {
    await worker?.stop();
  });

  test("before the deadline: heartbeats say stop false, and GET agrees", async () => {
    assert.equal((await put(alice, ALICE, a1, 0)).status, 200);
    const r = await beat(alice, ALICE, a1.lastSeq, a1.head, false);
    assert.equal(r.status, 200);
    const { seconds_left, ...rest } = r.body;
    assert.deepEqual(rest, { stop: false, deadline_at: alice.deadline, reason: null });
    // The 6-second run has between 1 and 6 seconds left, on the server's clock.
    assert.ok(seconds_left >= 1 && seconds_left <= 6, JSON.stringify(r.body));
    const g = await get(alice, ALICE);
    assert.equal(g.body.stop, false);
    assert.equal(g.body.final, null);
  });

  test("an agent that ended early commits its end before the deadline, and nothing goes past it", async () => {
    assert.equal((await put(carol, CAROL, carolC, 0)).status, 200);
    const f = await beat(carol, CAROL, carolC.lastSeq, carolC.head, true, "f".repeat(40));
    assert.equal(f.status, 200, JSON.stringify(f.body));
    assert.equal(f.body.stop, false);
    assert.deepEqual((await get(carol, CAROL)).body.final, { seq: 4, head: carolC.head });
    // Still before the deadline, but past the committed end: closed.
    const more = makeChunk(carol.entry, 5, 2, carolC.head, "c2");
    assertError(await put(carol, CAROL, more, carolC.length), 410, "closed");
    // The seal before the deadline needs nothing more; a repeated final heartbeat is fine, also after the seal.
    assert.equal((await seal(carol, CAROL)).status, 200);
    assert.equal((await beat(carol, CAROL, 4, carolC.head, true)).status, 200);
  });

  test("dave uploads a chain the server will hold at the deadline", async () => {
    const d1 = makeChunk(dave.entry, 0, 5, GENESIS, "d1");
    assert.equal((await put(dave, DAVE, d1, 0)).status, 200);
  });

  test("past the deadline: stop is true on a heartbeat and on GET", async () => {
    await sleepUntil(alice.deadline);
    const r = await beat(alice, ALICE, a1.lastSeq, a1.head, false);
    assert.equal(r.status, 200);
    assert.deepEqual(r.body, { stop: true, deadline_at: alice.deadline, seconds_left: 0, reason: "deadline" });
    assert.equal((await get(alice, ALICE)).body.stop, true);
    assert.equal((await get(alice, ALICE)).body.status, "open");
  });

  test("a chunk past the deadline with no final heartbeat is 503 with Retry-After, and nothing is stored", async () => {
    const r = await put(alice, ALICE, a2, a1.length);
    assertError(r, 503, "awaiting_final");
    assert.equal(r.headers.get("retry-after"), "60");
    assert.equal((await get(alice, ALICE)).body.offset, a1.length);
  });

  test("the seal past the deadline with no final heartbeat is 503 with Retry-After", async () => {
    const r = await seal(bob, BOB);
    assertError(r, 503, "awaiting_final");
    assert.equal(r.headers.get("retry-after"), "60");
  });

  test("the final heartbeat commits the end; a repeat is 200, a different end is 409", async () => {
    const f = await beat(alice, ALICE, a3.lastSeq, a3.head, true, "a".repeat(40));
    assert.equal(f.status, 200, JSON.stringify(f.body));
    assert.deepEqual(f.body, { stop: true, deadline_at: alice.deadline, seconds_left: 0, reason: "deadline" });
    assert.deepEqual((await get(alice, ALICE)).body.final, { seq: 9, head: a3.head });

    assert.equal((await beat(alice, ALICE, 9, a3.head, true, "a".repeat(40))).status, 200); // same
    assert.equal((await beat(alice, ALICE, 9, a3.head, true, null)).status, 200); // repo_head is not part of "same"
    assertError(await beat(alice, ALICE, 10, a3.head, true), 409, "conflict"); // other seq
    assertError(await beat(alice, ALICE, 9, "e".repeat(64), true), 409, "conflict"); // other head
    assert.deepEqual((await get(alice, ALICE)).body.final, { seq: 9, head: a3.head }); // unchanged
  });

  test("with a committed end the wrap-up upload runs, with no time limit, up to it and no further", async () => {
    assert.equal((await put(alice, ALICE, a2, a1.length)).status, 200);
    // Past the committed seq: 410. At the committed seq with another head: 410.
    assertError(await put(alice, ALICE, aOver, a1.length + a2.length), 410, "closed");
    assertError(await put(alice, ALICE, aWrongHead, a1.length + a2.length), 410, "closed");
    assert.equal((await get(alice, ALICE)).body.next_seq, 8);
  });

  test("the seal past the deadline needs the stored chain to have reached the committed head", async () => {
    assertError(await seal(alice, ALICE), 409, "conflict"); // the server holds 0..7, the end is 9
    const last = await put(alice, ALICE, a3, a1.length + a2.length);
    assert.equal(last.status, 200, JSON.stringify(last.body));
    assert.equal(last.body.head, a3.head);
    const s = await seal(alice, ALICE);
    assert.equal(s.status, 200, JSON.stringify(s.body));
    assert.equal((await seal(alice, ALICE)).status, 200); // repeat
    assert.equal((await get(alice, ALICE)).body.status, "sealed");
  });

  test("a run with no chunks can commit a final and seal after the deadline once the chain arrives", async () => {
    assert.equal((await beat(bob, BOB, 2, makeChunk(bob.entry, 0, 3, GENESIS, "b1").head, true)).status, 200);
    assertError(await seal(bob, BOB), 409, "conflict"); // nothing stored yet
    const b1 = makeChunk(bob.entry, 0, 3, GENESIS, "b1");
    assert.equal((await put(bob, BOB, b1, 0)).status, 200);
    assert.equal((await seal(bob, BOB)).status, 200);
  });

  test("a final heartbeat that contradicts the chain the server already holds is 409", async () => {
    // dave's server copy ends at seq 4.
    const d1 = makeChunk(dave.entry, 0, 5, GENESIS, "d1");
    assertError(await beat(dave, DAVE, 3, "3".repeat(64), true), 409, "conflict"); // behind it
    assertError(await beat(dave, DAVE, 4, "9".repeat(64), true), 409, "conflict"); // same seq, other head
    assert.equal((await get(dave, DAVE)).body.final, null);
    assert.equal((await beat(dave, DAVE, 4, d1.head, true)).status, 200);
    assertError(await put(dave, DAVE, makeChunk(dave.entry, 5, 1, d1.head, "d2"), d1.length), 410, "closed");
  });
});

describe("a run still unsealed 24 hours after it ends is closed as incomplete", { concurrency: false }, () => {
  let worker: RunningWorker;
  let api: Client;

  before(async () => {
    // Test-only clock: a 2-second run, closed 2 seconds after its deadline.
    worker = await startWorker({ EVENT_ID: "practice", TEST_APPROVAL_KEY: APPROVAL_KEY, RUN_SECONDS: "2", CLOSE_AFTER_SECONDS: "2" });
    api = new Client(worker.url);
    [ALICE, BOB, CAROL] = [await credentialFor(api, "alice"), await credentialFor(api, "bob"), await credentialFor(api, "carol")];
  });
  after(async () => {
    await worker?.stop();
  });

  test("it answers 410 to everything, shows as incomplete; a sealed run stays sealed", async () => {
    const open = async (token: string, session: string) => {
      const r = await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: session }) });
      assert.equal(r.status, 201, JSON.stringify(r.body));
      return r.body as { run_id: string; entry_id: string; deadline_at: string };
    };
    const alice = await open(ALICE, "a1");
    const bob = await open(BOB, "b1");
    const carol = await open(CAROL, "c1");
    const c = makeChunk(alice.entry_id, 0, 3, GENESIS, "x");
    assert.equal((await api.call("PUT", `/v1/runs/${alice.run_id}/chunks/0`, { token: ALICE, body: c.body, headers: c.headers })).status, 200);
    // carol commits her end and seals in time; a sealed run is never closed as incomplete.
    const cc = makeChunk(carol.entry_id, 0, 2, GENESIS, "carol");
    assert.equal((await api.call("PUT", `/v1/runs/${carol.run_id}/chunks/0`, { token: CAROL, body: cc.body, headers: cc.headers })).status, 200);
    assert.equal(
      (await api.call("POST", `/v1/runs/${carol.run_id}/heartbeat`, { token: CAROL, json: { entry_id: carol.entry_id, seq: cc.lastSeq, head: cc.head, offset_local: 1, final: true, repo_head: null } })).status,
      200,
    );
    assert.equal(
      (await api.call("POST", `/v1/runs/${carol.run_id}/seal`, { token: CAROL, json: { entry_id: carol.entry_id, seal_json: "{}", baseline_json: "{}" } })).status,
      200,
    );

    // Still open between the deadline and deadline + 2 s.
    await sleepUntil(alice.deadline_at);
    assert.equal((await api.call("GET", `/v1/runs/${alice.run_id}`, { token: ALICE })).body.status, "open");

    await sleep(3300); // the server clock counts whole seconds, and "past" is strictly past
    const g = await api.call("GET", `/v1/runs/${alice.run_id}`, { token: ALICE });
    assert.equal(g.status, 200);
    assert.equal(g.body.status, "incomplete");
    const more = makeChunk(alice.entry_id, 3, 1, c.head, "y");
    assertError(await api.call("PUT", `/v1/runs/${alice.run_id}/chunks/${c.length}`, { token: ALICE, body: more.body, headers: more.headers }), 410, "closed");
    assertError(
      await api.call("POST", `/v1/runs/${alice.run_id}/heartbeat`, { token: ALICE, json: { entry_id: alice.entry_id, seq: 2, head: c.head, offset_local: 1, final: true, repo_head: null } }),
      410,
      "closed",
    );
    assertError(await api.call("POST", `/v1/runs/${alice.run_id}/seal`, { token: ALICE, json: { entry_id: alice.entry_id, seal_json: "{}", baseline_json: "{}" } }), 410, "closed");
    // bob never looked at his old run: it is incomplete the first time he does.
    assert.equal((await api.call("GET", `/v1/runs/${bob.run_id}`, { token: BOB })).body.status, "incomplete");
    // carol's sealed run stayed sealed.
    assert.equal((await api.call("GET", `/v1/runs/${carol.run_id}`, { token: CAROL })).body.status, "sealed");
  });
});

describe("test-only clocks are refused outside the local backend", () => {
  let worker: RunningWorker;
  before(async () => {
    worker = await startWorker({ EVENT_ID: "practice", DB_BACKEND: "supabase", RUN_SECONDS: "5", SUPABASE_URL: "https://unused.invalid", SUPABASE_KEY: "unused" });
  });
  after(async () => {
    await worker?.stop();
  });
  test("RUN_SECONDS with another backend is a 500 misconfigured, not silently honoured", async () => {
    const r = await new Client(worker.url).call("POST", "/v1/runs", { token: ALICE, json: openBody() });
    assertError(r, 500, "misconfigured");
  });
});
