// End-to-end over HTTP against `wrangler dev` (local mode, DB_BACKEND=local, R2 simulated on disk).
// Every test in a `describe` shares one running Worker, and runs in order: the later ones continue
// the run the earlier ones built. One Worker per configuration (normal, maintenance, short-lived runs).

import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFileSync, readdirSync } from "node:fs";
import { DatabaseSync } from "node:sqlite";
import { join } from "node:path";
import { after, before, describe, mock, test } from "node:test";
import { APPROVAL_KEY, Client, GENESIS, credentialFor, makeChunk, openBody, sha256 } from "./helpers/client.ts";
import { lifecycle } from "./helpers/rig.ts";
import { startWorker, type RunningWorker } from "./helpers/worker.ts";

// Run credentials, minted through the device flow in each describe's `before`.
let ALICE = "";
let BOB = "";
let CAROL = "";
let DAVE = "";
const ID_RUN = /^r[0-9a-f]{16}$/;
const ID_ENTRY = /^e[0-9a-f]{16}$/;
const TIME = /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/;

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
    [ALICE, BOB] = [await credentialFor(api, "alice"), await credentialFor(api, "bob")];
  });
  after(async () => {
    await worker?.stop();
  });

  // State carried from test to test (alice's run).
  let runId = "";
  let entryId = "";

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
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: openBody({ connector_sha256: "ABC" }) }), 400, "malformed");
      const { workspace: _w, ...missing } = openBody();
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: missing }), 400, "malformed");
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, body: new TextEncoder().encode("{not json") }), 400, "malformed");
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: [] }), 400, "malformed");
      // The version under its old name only: refused, the old name is gone.
      assertError(await api.call("POST", "/v1/runs", { token: ALICE, json: openBody({ proxy_version: undefined, tracer_version: "1.0.0" }) }), 400, "malformed");
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
      assertError(await api.call("PUT", `/v1/runs/${runId}/chunks/0`, { token: BOB, body: new Uint8Array([1]), headers: makeChunk(entryId, 0, 1, GENESIS, "x").headers }), 404, "not_found");
    });
  });

  describe("routes gone since 0.7", () => {
    test("receipt and abandon are 404", async () => {
      for (const route of ["receipt", "abandon"]) {
        const r = await api.call("POST", `/v1/runs/${runId}/${route}`, { token: ALICE, json: { entry_id: entryId, chain_head: "b".repeat(64), reason: "x" } });
        assertError(r, 404, "not_found");
      }
    });
  });

  describe("position", () => {
    test("a fresh run: offset 0, next_seq 0, genesis head, not sealed", async () => {
      const r = await api.call("GET", `/v1/runs/${runId}`, { token: ALICE });
      assert.equal(r.status, 200);
      assert.deepEqual(r.body, {
        run_id: runId, entry_id: entryId, segment: 0, status: "open",
        official_start: r.body.official_start, deadline_at: r.body.deadline_at, ends_at: r.body.deadline_at,
        offset: 0, next_seq: 0, head: GENESIS, stop: false, final: null, sealed: false,
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
      assert.deepEqual(rest, { stop: false, deadline_at: pos.body.deadline_at, reason: null, sealed: false });
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
    // The first chunk's answer, which every later retry of that chunk must repeat.
    let answer1: Record<string, unknown>;

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

    test("first chunk: 200 with the new position and sealed, and no ack", async () => {
      c1 = chunk1();
      const r = await put(c1, 0);
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(Object.keys(r.body).sort(), ["head", "next_seq", "offset", "received_at", "sealed"]);
      answer1 = r.body;
      assert.equal(r.body.sealed, false);
      assert.match(r.body.received_at, TIME);
      assert.equal(r.body.offset, c1.length);
      assert.equal(r.body.next_seq, c1.lastSeq + 1);
      assert.equal(r.body.head, c1.head);
      const p = await position();
      assert.equal(p.offset, c1.length);
      assert.equal(p.next_seq, 5);
      assert.equal(p.head, c1.head);
      assert.equal(p.sealed, false);
      assert.ok(!("last_ack" in p), JSON.stringify(p));
    });

    test("a retry of the same chunk is 200 with the very same stored answer, body unread", async () => {
      await new Promise((r) => setTimeout(r, 1100)); // a fresh answer would carry a later received_at
      // Other bytes under the same headers: had the body been read, its hash would not match (422).
      const other = Buffer.from(c1.body);
      other[other.length - 1] = other[other.length - 1]! ^ 0xff;
      const r = await put({ body: other, headers: c1.headers }, 0);
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(r.body, answer1);
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
      // A shorter body than Content-Length cannot happen over HTTP, but a *different* body of a
      // different size with the old hash is a mismatch too.
      assertError(await put({ body: Buffer.concat([good.body, Buffer.from([0])]), headers: good.headers }, c1.length), 422, "digest_mismatch");

      const ok = await put(good, c1.length);
      assert.equal(ok.status, 200, JSON.stringify(ok.body));
      assert.equal(ok.body.offset, c1.length + good.length);
      assert.equal(ok.body.next_seq, 8);
      p = await position();
      assert.equal(p.head, good.head);
      // The stored object is exactly the verified body, once.
      const goodKey = `traces/practice/${runId}/${entryId}/${String(c1.length).padStart(16, "0")}-${good.headers["goalcup-sha256"]}.zst`;
      assert.deepEqual(traceKeys().filter((o) => o.key === goodKey).map((o) => o.sha256), [good.headers["goalcup-sha256"]]);
    });

    test("two identical uploads at once: both 200 with one answer, and the run advanced once", async () => {
      const before = await position();
      const next = makeChunk(entryId, before.next_seq, 4, before.head, "three");
      const [a, b] = await Promise.all([put(next, before.offset), put(next, before.offset)]);
      assert.equal(a.status, 200, JSON.stringify(a.body));
      assert.equal(b.status, 200, JSON.stringify(b.body));
      assert.deepEqual(a.body, b.body);
      const p = await position();
      assert.equal(p.offset, before.offset + next.length);
      assert.equal(p.next_seq, before.next_seq + 4);
    });

    test("an old chunk can still be retried after the run moved on", async () => {
      const r = await put(c1, 0);
      assert.equal(r.status, 200);
      assert.deepEqual(r.body, answer1);
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

  describe("the server seals when it holds the whole chain", () => {
    const fresh = async (member: string) => {
      const token = await credentialFor(api, member);
      const run = (await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: `${member}-session` }) })).body as { run_id: string; entry_id: string };
      const put = (c: { body: Uint8Array; headers: Record<string, string> }, offset: number) =>
        api.call("PUT", `/v1/runs/${run.run_id}/chunks/${offset}`, { token, body: c.body, headers: c.headers });
      const final = (seq: number, head: string) =>
        api.call("POST", `/v1/runs/${run.run_id}/heartbeat`, { token, json: { entry_id: run.entry_id, seq, head, offset_local: 1, final: true, repo_head: null } });
      const position = async () => (await api.call("GET", `/v1/runs/${run.run_id}`, { token })).body;
      return { token, run, put, final, position };
    };

    test("chunks, then a final heartbeat: the final heartbeat seals the entry", async () => {
      const d = await fresh("dora");
      const c = makeChunk(d.run.entry_id, 0, 3, GENESIS, "dora");
      const put = await d.put(c, 0);
      assert.equal(put.status, 200, JSON.stringify(put.body));
      assert.equal(put.body.sealed, false); // no end committed yet
      const f = await d.final(c.lastSeq, c.head);
      assert.equal(f.status, 200, JSON.stringify(f.body));
      assert.equal(f.body.sealed, true);
      const p = await d.position();
      assert.equal(p.sealed, true);
      assert.equal(p.status, "open"); // before the run's end, a sealed entry leaves the run open
      // The sealed entry takes no more chain.
      const late = makeChunk(d.run.entry_id, p.next_seq, 1, p.head, "after");
      assertError(await d.put(late, p.offset), 410, "closed");
    });

    test("a final heartbeat first, then chunks: the last chunk's answer says sealed, with no repeat", async () => {
      const e = await fresh("erin");
      const first = makeChunk(e.run.entry_id, 0, 3, GENESIS, "erin-1");
      const last = makeChunk(e.run.entry_id, 3, 2, first.head, "erin-2");
      const f = await e.final(last.lastSeq, last.head);
      assert.equal(f.status, 200, JSON.stringify(f.body));
      assert.equal(f.body.sealed, false); // the server holds none of the chain yet
      const one = await e.put(first, 0);
      assert.equal(one.status, 200, JSON.stringify(one.body));
      assert.equal(one.body.sealed, false);
      const two = await e.put(last, first.length);
      assert.equal(two.status, 200, JSON.stringify(two.body));
      assert.equal(two.body.sealed, true);
      assert.equal((await e.position()).sealed, true);
      // A resent chunk is answered from the stored row, with the entry's seal.
      const again = await e.put(last, first.length);
      assert.equal(again.status, 200, JSON.stringify(again.body));
      assert.equal(again.body.sealed, true);

      // A repeat of the final heartbeat says sealed (that it adds no row is guarded in local-db.test.ts).
      const repeat = await e.final(last.lastSeq, last.head);
      assert.equal(repeat.status, 200, JSON.stringify(repeat.body));
      assert.equal(repeat.body.sealed, true);
    });

    test("the seal route is gone: 404", async () => {
      const r = await api.call("POST", `/v1/runs/${runId}/seal`, { token: ALICE, json: { entry_id: entryId, seal_json: "{}", baseline_json: "{}" } });
      assertError(r, 404, "not_found");
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
      api.call("POST", "/v1/device", { json: { purpose: "run", mode: "practice", connector_id: "x", proxy_version: "1" } }),
      api.call("POST", "/v1/device/token", { json: { device_code: "a".repeat(64) } }),
      api.call("POST", "/v1/runs", { token: ALICE, json: openBody() }),
      api.call("POST", "/v1/runs", { json: openBody() }),
      api.call("GET", "/v1/runs/r0123456789abcdef", { token: ALICE }),
      api.call("PUT", "/v1/runs/r0123456789abcdef/chunks/0", { token: ALICE, body: c.body, headers: c.headers }),
      api.call("POST", "/v1/runs/r0123456789abcdef/heartbeat", { token: ALICE, json: {} }),
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
    assert.deepEqual(rest, { stop: false, deadline_at: alice.deadline, reason: null, sealed: false });
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
    // The server held the chain up to that end, so the final heartbeat sealed it; a repeat is fine.
    assert.equal(f.body.sealed, true);
    const again = await beat(carol, CAROL, 4, carolC.head, true);
    assert.equal(again.status, 200, JSON.stringify(again.body));
    assert.equal(again.body.sealed, true);
  });

  test("dave uploads a chain the server will hold at the deadline", async () => {
    const d1 = makeChunk(dave.entry, 0, 5, GENESIS, "d1");
    assert.equal((await put(dave, DAVE, d1, 0)).status, 200);
  });

  test("past the deadline: stop is true on a heartbeat and on GET", async () => {
    // The four opens can straddle a second on the server's clock, so bob's or dave's run can end a
    // second after alice's. The tests below need every one of them past its deadline.
    await sleepUntil(new Date(Math.max(...[alice, bob, dave].map((r) => Date.parse(r.deadline)))).toISOString());
    const r = await beat(alice, ALICE, a1.lastSeq, a1.head, false);
    assert.equal(r.status, 200);
    assert.deepEqual(r.body, { stop: true, deadline_at: alice.deadline, seconds_left: 0, reason: "deadline", sealed: false });
    assert.equal((await get(alice, ALICE)).body.stop, true);
    assert.equal((await get(alice, ALICE)).body.status, "open");
  });

  test("a chunk past the deadline with no final heartbeat is 503 with Retry-After, and nothing is stored", async () => {
    const r = await put(alice, ALICE, a2, a1.length);
    assertError(r, 503, "awaiting_final");
    assert.equal(r.headers.get("retry-after"), "60");
    assert.equal((await get(alice, ALICE)).body.offset, a1.length);
  });

  test("the final heartbeat commits the end; a repeat is 200, a different end is 409", async () => {
    const f = await beat(alice, ALICE, a3.lastSeq, a3.head, true, "a".repeat(40));
    assert.equal(f.status, 200, JSON.stringify(f.body));
    assert.deepEqual(f.body, { stop: true, deadline_at: alice.deadline, seconds_left: 0, reason: "deadline", sealed: false });
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

  test("past the deadline, the chunk that reaches the committed head seals the entry and the run", async () => {
    assert.equal((await get(alice, ALICE)).body.sealed, false); // the server holds 0..7, the end is 9
    const last = await put(alice, ALICE, a3, a1.length + a2.length);
    assert.equal(last.status, 200, JSON.stringify(last.body));
    assert.equal(last.body.head, a3.head);
    assert.equal(last.body.sealed, true);
    const g = (await get(alice, ALICE)).body;
    assert.deepEqual([g.status, g.sealed], ["sealed", true]);
    // A repeat of the final heartbeat says so too.
    assert.equal((await beat(alice, ALICE, 9, a3.head, true)).body.sealed, true);
  });

  test("a run with no chunks can commit a final after the deadline and is sealed once the chain arrives", async () => {
    const b1 = makeChunk(bob.entry, 0, 3, GENESIS, "b1");
    const f = await beat(bob, BOB, 2, b1.head, true);
    assert.equal(f.status, 200, JSON.stringify(f.body));
    assert.equal(f.body.sealed, false); // nothing stored yet
    const put1 = await put(bob, BOB, b1, 0);
    assert.equal(put1.status, 200, JSON.stringify(put1.body));
    assert.equal(put1.body.sealed, true);
    assert.equal((await get(bob, BOB)).body.status, "sealed");
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
    const sealedNow = await api.call("POST", `/v1/runs/${carol.run_id}/heartbeat`, {
      token: CAROL,
      json: { entry_id: carol.entry_id, seq: cc.lastSeq, head: cc.head, offset_local: 1, final: true, repo_head: null },
    });
    assert.equal(sealedNow.status, 200, JSON.stringify(sealedNow.body));
    assert.equal(sealedNow.body.sealed, true);

    // Still open between the deadline and deadline + 2 s.
    await sleepUntil(alice.deadline_at);
    assert.equal((await api.call("GET", `/v1/runs/${alice.run_id}`, { token: ALICE })).body.status, "open");

    // The three opens can straddle a second on the server's clock, so bob's or carol's run can end
    // a second after alice's. Wait until the last of them is past its close: the server clock
    // counts whole seconds, and "past" is strictly past.
    const lastDeadline = Math.max(...[alice, bob, carol].map((r) => Date.parse(r.deadline_at)));
    await sleep(Math.max(0, lastDeadline + 3700 - Date.now()));
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

// ---- the heartbeat's counts (in this process: the stored state is read back directly) -------

describe("heartbeat counts", () => {
  /** The stored run, as the local database keeps it: the run's latest vitals and each entry's last heartbeat. */
  type Stored = {
    last_vitals?: unknown;
    flag_reason?: string | null;
    entries: { last_heartbeat: { recording: string | null; vitals?: unknown } | null }[];
  };
  const counts = { v: 1, boot: 2, mode: null, off_ms: 2300, off_spans: 1, off_since: "2026-11-06T09:00:30Z", gaps: 3, lost_bytes: 1500000, lost_unknown: 0, waiting_bytes: 1048576 };
  const utf8 = (v: unknown) => Buffer.byteLength(JSON.stringify(v), "utf8");

  lifecycle("a heartbeat with recording failed and counts stores both, the counts nested on the row and the run", async (r) => {
    const token = await r.cred("alice");
    const run = await r.start(token, "s1");
    const res = await r.beat(token, run.run_id, run.entry_id, { recording: "failed", model_requests: 12, counts });
    assert.equal(res.status, 200, JSON.stringify(res.body));
    assert.equal(res.body.sealed, false);
    const s = r.storedRun(run.run_id) as unknown as Stored;
    assert.equal(s.entries[0]!.last_heartbeat!.recording, "failed");
    assert.deepEqual(s.entries[0]!.last_heartbeat!.vitals, { counts });
    assert.deepEqual(s.last_vitals, { counts });
    assert.equal(s.flag_reason, "failed");
  });

  lifecycle("counts over 2 KiB or not flat cost nothing: 200, the row says counts_dropped, and one log line", async (r) => {
    const token = await r.cred("alice");
    const run = await r.start(token, "s1");
    const big: Record<string, string> = {};
    for (let i = 0; i < 40; i++) big[`k${i}`] = "x".repeat(64);
    assert.ok(utf8(big) > 2048);
    const logged: string[] = [];
    const log = mock.method(console, "warn", (...args: unknown[]) => void logged.push(args.map(String).join(" ")));
    try {
      for (const bad of [big, { nested: { a: 1 } }, { list: [1] }, [1, 2], "counts", 5, { s: "y".repeat(65) }, { n: 1.5 }, { n: -1 }]) {
        logged.length = 0;
        const res = await r.beat(token, run.run_id, run.entry_id, { recording: "gaps", counts: bad });
        assert.equal(res.status, 200, JSON.stringify(res.body));
        const s = r.storedRun(run.run_id) as unknown as Stored;
        assert.deepEqual(s.entries[0]!.last_heartbeat!.vitals, { counts_dropped: utf8(bad) }, JSON.stringify(bad).slice(0, 80));
        assert.deepEqual(s.last_vitals, { counts_dropped: utf8(bad) });
        assert.equal(s.entries[0]!.last_heartbeat!.recording, "gaps"); // the rest of the heartbeat is kept
        // One visible line per drop, and no counts value in it.
        assert.equal(logged.length, 1);
        assert.match(logged[0]!, /^counts-dropped /);
        if (typeof bad !== "number") assert.ok(!logged[0]!.includes(JSON.stringify(bad)), logged[0]);
      }
    } finally {
      log.mock.restore();
    }
  });
});
