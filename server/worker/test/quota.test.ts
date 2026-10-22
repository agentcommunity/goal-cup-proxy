// The per-run size limit (RUN_MAX_COMPRESSED_BYTES, WP-D), in this process over the fake bucket.
// The chunk upload needs the Workers runtime's FixedLengthStream; a pass-through stands in for it
// here (the runtime one also checks the length, which test/integration.test.ts covers).

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { GENESIS, type TestChunk, credentialFor, makeChunk, openBody } from "./helpers/client.ts";
import { type InProcess, inProcess } from "./helpers/inproc.ts";

const g = globalThis as unknown as { FixedLengthStream?: unknown };
g.FixedLengthStream ??= class extends TransformStream<Uint8Array, Uint8Array> {
  constructor(_length: number) {
    super();
  }
};

let rig: InProcess | undefined;
afterEach(() => rig?.stop());

const T0 = "2026-11-06T10:00:00Z";

/** Open a competition run for `member` and give back what the chunk calls need. */
async function openRun(r: InProcess, member: string) {
  const token = await credentialFor(r.api, member);
  const res = await r.api.call("POST", "/v1/runs", { token, json: openBody({ session_id: `${member}-1` }) });
  assert.equal(res.status, 201, JSON.stringify(res.body));
  const { run_id, entry_id } = res.body as { run_id: string; entry_id: string };
  const put = (c: TestChunk, offset: number) =>
    r.api.call("PUT", `/v1/runs/${run_id}/chunks/${offset}`, { token, body: c.body, headers: { ...c.headers, "content-length": String(c.body.length) } });
  const totals = async () => {
    const run = (await r.db().getRun(member, run_id, null))!.run;
    return { stored_bytes: run.stored_bytes, chunk_count: run.chunk_count };
  };
  return { token, run_id, entry_id, put, totals };
}

const traceKeys = (r: InProcess) => [...r.bucket.map.keys()].filter((k) => k.startsWith("traces/"));

/** Run `fn` with console.error and console.warn captured. */
async function logged<T>(fn: () => Promise<T>): Promise<{ value: T; lines: string[] }> {
  const lines: string[] = [];
  const [error, warn] = [console.error, console.warn];
  console.error = console.warn = (...args: unknown[]) => void lines.push(args.map(String).join(" "));
  try {
    return { value: await fn(), lines };
  } finally {
    [console.error, console.warn] = [error, warn];
  }
}

test("a chunk that would pass the limit is 413 quota_exceeded, stores nothing, and the upload resumes once the limit is raised", async () => {
  // The chunk bodies do not depend on the entry id, so their sizes are known before the run exists.
  const size1 = makeChunk("e0000000000000000", 0, 5, GENESIS, "a").body.length;
  const size2 = makeChunk("e0000000000000000", 5, 5, "0".repeat(64), "b").body.length;
  rig = inProcess(T0, { RUN_MAX_COMPRESSED_BYTES: String(size1 + size2 - 1) });
  const run = await openRun(rig, "alice");
  const c1 = makeChunk(run.entry_id, 0, 5, GENESIS, "a");
  const c2 = makeChunk(run.entry_id, 5, 5, c1.head, "b");

  const first = await run.put(c1, 0);
  assert.equal(first.status, 200, JSON.stringify(first.body));
  assert.deepEqual(await run.totals(), { stored_bytes: size1, chunk_count: 1 });
  assert.equal(traceKeys(rig).length, 1);

  // One byte over: refused before the body is read, nothing in R2, the position unchanged.
  const { value: over, lines } = await logged(() => run.put(c2, c1.length));
  assert.equal(over.status, 413, JSON.stringify(over.body));
  assert.equal(over.body.error, "quota_exceeded");
  assert.match(over.body.message, /limit/);
  assert.equal(traceKeys(rig).length, 1);
  assert.deepEqual(await run.totals(), { stored_bytes: size1, chunk_count: 1 });
  assert.equal((await rig.api.call("GET", `/v1/runs/${run.run_id}`, { token: run.token })).body.offset, c1.length);
  assert.ok(lines.some((l) => l.startsWith(`quota-exceeded ${run.run_id} `)), lines.join("\n"));

  // A retry of the chunk the server already holds is still answered with its ack.
  const retry = await run.put(c1, 0);
  assert.equal(retry.status, 200);
  assert.deepEqual(retry.body.ack, first.body.ack);

  // The organizers raise the limit (a deploy with a new value): the same chunk now goes through.
  rig.env.RUN_MAX_COMPRESSED_BYTES = String(size1 + size2);
  const resumed = await run.put(c2, c1.length);
  assert.equal(resumed.status, 200, JSON.stringify(resumed.body));
  assert.deepEqual(await run.totals(), { stored_bytes: size1 + size2, chunk_count: 2 });
  assert.equal(traceKeys(rig).length, 2);
});

test("the limit is per run: another run of another member is not held back by it", async () => {
  const size = makeChunk("e0000000000000000", 0, 5, GENESIS, "a").body.length;
  rig = inProcess(T0, { RUN_MAX_COMPRESSED_BYTES: String(size) });
  const a = await openRun(rig, "alice");
  const b = await openRun(rig, "bob");
  const ca = makeChunk(a.entry_id, 0, 5, GENESIS, "a");
  assert.equal((await a.put(ca, 0)).status, 200); // exactly at the limit is allowed
  const { value: full } = await logged(() => a.put(makeChunk(a.entry_id, 5, 1, ca.head, "x"), ca.length));
  assert.equal(full.status, 413);
  assert.equal((await b.put(makeChunk(b.entry_id, 0, 5, GENESIS, "a"), 0)).status, 200);
});

test("the operators get a log line when a run reaches half its limit and when it reaches all of it", async () => {
  const size = makeChunk("e0000000000000000", 0, 5, GENESIS, "a").body.length;
  rig = inProcess(T0, { RUN_MAX_COMPRESSED_BYTES: String(2 * size) });
  const run = await openRun(rig, "alice");
  const c1 = makeChunk(run.entry_id, 0, 5, GENESIS, "a");
  const { value: half, lines: l1 } = await logged(() => run.put(c1, 0));
  assert.equal(half.status, 200);
  assert.deepEqual(l1, [`quota-half ${run.run_id} stored=${size} max=${2 * size}`]);
  // A chunk of the same compressed size: the run is then exactly full.
  const c2 = makeChunk(run.entry_id, 5, 5, c1.head, "a");
  assert.equal(c2.body.length, size);
  const { value: full, lines: l2 } = await logged(() => run.put(c2, c1.length));
  assert.equal(full.status, 200);
  assert.deepEqual(l2, [`quota-full ${run.run_id} stored=${2 * size} max=${2 * size}`]);
});
