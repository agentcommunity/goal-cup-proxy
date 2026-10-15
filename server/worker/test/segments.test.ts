// The run lifecycle over real HTTP against `wrangler dev`, with real chunk bodies streamed into R2:
// what lifecycle.test.ts checks with a clock, here with the Workers runtime. Nothing here waits
// for a deadline; a replaced run is over at once.

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import { APPROVAL_KEY, Client, GENESIS, credentialFor, makeChunk, openBody } from "./helpers/client.ts";
import { startWorker, type RunningWorker } from "./helpers/worker.ts";

function assertError(r: { status: number; body: { error?: string; message?: string } }, status: number, code: string) {
  assert.equal(r.status, status, JSON.stringify(r.body));
  assert.equal(r.body.error, code);
  assert.equal(typeof r.body.message, "string");
  assert.deepEqual(Object.keys(r.body).sort(), ["error", "message"]);
}

describe("replace, practice and segments against wrangler dev", { concurrency: false }, () => {
  let worker: RunningWorker;
  let api: Client;

  before(async () => {
    worker = await startWorker({ EVENT_ID: "cup", TEST_APPROVAL_KEY: APPROVAL_KEY });
    api = new Client(worker.url);
  });
  after(async () => {
    await worker?.stop();
  });

  const open = async (token: string, session: string) => {
    const r = await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: session }) });
    assert.equal(r.status, 201, JSON.stringify(r.body));
    return r.body as { run_id: string; entry_id: string; official_start: string; deadline_at: string };
  };
  const put = (run: string, token: string, c: { body: Uint8Array; headers: Record<string, string> }, offset: number) =>
    api.call("PUT", `/v1/runs/${run}/chunks/${offset}`, { token, body: c.body, headers: c.headers });
  const beat = (run: string, token: string, entry: string, seq: number, head: string, final: boolean) =>
    api.call("POST", `/v1/runs/${run}/heartbeat`, { token, json: { entry_id: entry, seq, head, offset_local: 1, final, repo_head: null } });
  const seal = (run: string, token: string, entry: string) =>
    api.call("POST", `/v1/runs/${run}/seal`, { token, json: { entry_id: entry, seal_json: '{"s":1}', baseline_json: '{"b":1}' } });
  const get = (run: string, token: string, entry?: string) => api.call("GET", `/v1/runs/${run}${entry === undefined ? "" : `?entry_id=${entry}`}`, { token });
  const cont = (run: string, token: string, previous: string, session: string) =>
    api.call("POST", `/v1/runs/${run}/continue`, {
      token,
      json: {
        previous_entry_id: previous, connector_id: "codex-cli", manifest_sha256: "a".repeat(64), connector_dir_sha256: "c".repeat(64),
        workspace: "/w/two", session_id: session, harness_version: "0.153.0", tracer_version: "0.1.0",
      },
    });

  test("the replaced run still takes its wrap-up chunks", async () => {
    const [c1, c2] = [await credentialFor(api, "alice"), await credentialFor(api, "alice")];
    const a = await open(c1, "alice-1");
    const a1 = makeChunk(a.entry_id, 0, 5, GENESIS, "a1");
    const a2 = makeChunk(a.entry_id, 5, 3, a1.head, "a2");
    const a3 = makeChunk(a.entry_id, 8, 2, a2.head, "a3");
    const first = await put(a.run_id, c1, a1, 0);
    assert.equal(first.status, 200, JSON.stringify(first.body));

    const b = await open(c2, "alice-2"); // the start that replaces it
    assert.notEqual(b.run_id, a.run_id);

    // Over, so a chunk waits for the end to be committed...
    const wait = await put(a.run_id, c1, a2, a1.length);
    assertError(wait, 503, "awaiting_final");
    assert.equal(wait.headers.get("retry-after"), "60");
    // ...but a chunk the server already holds is answered with its stored ack, whatever the run's state.
    const retry = await put(a.run_id, c1, a1, 0);
    assert.equal(retry.status, 200, JSON.stringify(retry.body));
    assert.deepEqual(retry.body.ack, first.body.ack);

    const f = await beat(a.run_id, c1, a.entry_id, a2.lastSeq, a2.head, true);
    assert.equal(f.status, 200, JSON.stringify(f.body));
    const { seconds_left, ...rest } = f.body;
    assert.deepEqual(rest, { stop: true, deadline_at: a.deadline_at, reason: "replaced" });
    assert.ok(seconds_left > 0, "a replaced run's deadline_at has not come; stop ends it");
    assert.equal((await put(a.run_id, c1, a2, a1.length)).status, 200);
    assertError(await put(a.run_id, c1, a3, a1.length + a2.length), 410, "closed"); // past the committed end

    const s = await seal(a.run_id, c1, a.entry_id);
    assert.equal(s.status, 200, JSON.stringify(s.body));
    assert.deepEqual(s.body, { status: "sealed", run_status: "sealed" });
    // The second run was never touched.
    assert.equal((await beat(b.run_id, c2, b.entry_id, 0, GENESIS, false)).body.stop, false);
  });

  test("chunks and seal address an older segment by entry_id", async () => {
    const token = await credentialFor(api, "bob");
    const run = await open(token, "bob-1");
    const e0a = makeChunk(run.entry_id, 0, 3, GENESIS, "e0a");
    const e0b = makeChunk(run.entry_id, 3, 2, e0a.head, "e0b");
    assert.equal((await put(run.run_id, token, e0a, 0)).status, 200);

    const c = await cont(run.run_id, token, run.entry_id, "bob-2");
    assert.equal(c.status, 201, JSON.stringify(c.body));
    const e1 = c.body.entry_id as string;
    const e1a = makeChunk(e1, 0, 4, GENESIS, "e1a");
    assert.equal((await put(run.run_id, token, e1a, 0)).status, 200); // the new segment's own chain from zero

    // The older segment keeps uploading where it stood, by its entry id.
    const more = await put(run.run_id, token, e0b, e0a.length);
    assert.equal(more.status, 200, JSON.stringify(more.body));
    assert.equal(more.body.ack.entry_id, run.entry_id);
    assert.equal((await get(run.run_id, token, run.entry_id)).body.offset, e0a.length + e0b.length);
    assert.equal((await get(run.run_id, token)).body.offset, e1a.length);
    assert.equal((await get(run.run_id, token)).body.entry_id, e1);

    // An entry that is not this run's is 404, on a chunk and on a seal.
    const other = await credentialFor(api, "carol");
    const theirs = await open(other, "carol-1");
    const stray = makeChunk(theirs.entry_id, 0, 1, GENESIS, "stray");
    assertError(await put(run.run_id, token, stray, 0), 404, "not_found");
    assertError(await seal(run.run_id, token, theirs.entry_id), 404, "not_found");
  });

  test("a segment seal before the end leaves the run open", async () => {
    const token = await credentialFor(api, "dave");
    const run = await open(token, "dave-1");
    const a = makeChunk(run.entry_id, 0, 3, GENESIS, "d-a");
    assert.equal((await put(run.run_id, token, a, 0)).status, 200);
    assert.equal((await beat(run.run_id, token, run.entry_id, a.lastSeq, a.head, true)).status, 200);
    const s = await seal(run.run_id, token, run.entry_id);
    assert.equal(s.status, 200, JSON.stringify(s.body));
    assert.deepEqual(s.body, { status: "sealed", run_status: "open" });
    assert.equal((await get(run.run_id, token)).body.status, "open");

    const c = await cont(run.run_id, token, run.entry_id, "dave-2");
    assert.equal(c.status, 201, JSON.stringify(c.body));
    const next = makeChunk(c.body.entry_id, 0, 2, GENESIS, "d-b");
    assert.equal((await put(run.run_id, token, next, 0)).status, 200);
    // The sealed segment takes no more.
    assertError(await put(run.run_id, token, makeChunk(run.entry_id, 3, 1, a.head, "d-late"), a.length), 410, "closed");
  });

  test("a practice run lasts PRACTICE_RUN_MINUTES (10) and a second one does not stop the first", async () => {
    const [p1c, p2c] = [await credentialFor(api, "erin", "practice"), await credentialFor(api, "erin", "practice")];
    const p1 = await open(p1c, "erin-p1");
    const p2 = await open(p2c, "erin-p2");
    assert.equal(Date.parse(p1.deadline_at) - Date.parse(p1.official_start), 10 * 60 * 1000);
    assert.equal((await beat(p1.run_id, p1c, p1.entry_id, 0, GENESIS, false)).body.stop, false);
    assert.equal((await beat(p2.run_id, p2c, p2.entry_id, 0, GENESIS, false)).body.stop, false);
  });

  test("continue needs the run's own credential", async () => {
    const token = await credentialFor(api, "frank");
    const run = await open(token, "frank-1");
    assertError(await api.call("POST", `/v1/runs/${run.run_id}/continue`, { json: {} }), 401, "unauthorized");
    assertError(await cont(run.run_id, await credentialFor(api, "gina"), run.entry_id, "g2"), 404, "not_found");
    assertError(await cont(run.run_id, token, run.entry_id, "x".repeat(129)), 400, "malformed");
  });
});
