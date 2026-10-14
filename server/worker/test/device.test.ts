// The device flow and the run-bound credential, end to end over HTTP against `wrangler dev`
// (docs/UPLOAD-API.md, "Sign in and the run credential").

import assert from "node:assert/strict";
import { after, before, describe, test } from "node:test";
import { APPROVAL_KEY, Client, GENESIS, approve, credentialFor, deviceBody, makeChunk, openBody } from "./helpers/client.ts";
import { startWorker, type RunningWorker } from "./helpers/worker.ts";

const USER_CODE = /^[BCDFGHJKLMNPQRSTVWXZ]{4}-[BCDFGHJKLMNPQRSTVWXZ]{4}$/;

function assertError(r: { status: number; body: { error?: string; message?: string } }, status: number, code: string) {
  assert.equal(r.status, status, JSON.stringify(r.body));
  assert.equal(r.body.error, code);
  assert.equal(typeof r.body.message, "string");
  assert.deepEqual(Object.keys(r.body).sort(), ["error", "message"]);
}

describe("device flow against wrangler dev", { concurrency: false }, () => {
  let worker: RunningWorker;
  let api: Client;
  const deviceCodes: string[] = [];

  before(async () => {
    worker = await startWorker({ EVENT_ID: "practice", TEST_APPROVAL_KEY: APPROVAL_KEY });
    api = new Client(worker.url);
  });
  after(async () => {
    await worker?.stop();
  });

  const start = async (over: Record<string, unknown> = {}) => {
    const d = await api.call("POST", "/v1/device", { json: deviceBody(over) });
    assert.equal(d.status, 200, JSON.stringify(d.body));
    deviceCodes.push(d.body.device_code);
    return d.body as { device_code: string; user_code: string; verification_uri: string; verification_uri_complete: string };
  };
  const poll = (device_code: string) => api.call("POST", "/v1/device/token", { json: { device_code } });

  test("pending, approve, pickup once, then 410", async () => {
    const before = Date.now();
    const d = await api.call("POST", "/v1/device", { json: deviceBody() });
    assert.equal(d.status, 200, JSON.stringify(d.body));
    deviceCodes.push(d.body.device_code);
    assert.deepEqual(Object.keys(d.body), ["device_code", "user_code", "verification_uri", "verification_uri_complete", "interval", "expires_in"]);
    assert.match(d.body.device_code, /^[0-9a-f]{64}$/);
    assert.match(d.body.user_code, USER_CODE);
    assert.equal(d.body.verification_uri_complete, `${d.body.verification_uri}?code=${d.body.user_code}`);
    assert.equal(d.body.interval, 5);
    assert.equal(d.body.expires_in, 900);

    for (let i = 0; i < 2; i++) {
      const p = await poll(d.body.device_code);
      assert.equal(p.status, 202, JSON.stringify(p.body));
      assert.deepEqual(p.body, { status: "pending" });
    }

    // The approval takes the code in lower case and without the dash, too.
    const a = await approve(api, d.body.user_code.replace("-", "").toLowerCase(), "alice");
    assert.equal(a.status, 200, JSON.stringify(a.body));
    assert.deepEqual(a.body, { status: "approved" });

    const t = await poll(d.body.device_code);
    assert.equal(t.status, 200, JSON.stringify(t.body));
    assert.deepEqual(Object.keys(t.body), ["status", "purpose", "credential", "expires_at"]);
    assert.equal(t.body.status, "approved");
    assert.equal(t.body.purpose, "run");
    assert.match(t.body.credential, /^gcr_[0-9a-f]{64}$/);
    assert.match(t.body.expires_at, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
    const hours = (Date.parse(t.body.expires_at) - before) / 3_600_000;
    assert.ok(hours > 0.99 && hours < 1.01, `unbound, the credential lasts an hour from the approval: ${t.body.expires_at}`);

    // Once: every later poll is gone.
    assertError(await poll(d.body.device_code), 410, "closed");
    assertError(await poll(d.body.device_code), 410, "closed");

    // The credential works.
    const run = await api.call("POST", "/v1/runs", { token: t.body.credential, json: openBody() });
    assert.equal(run.status, 201, JSON.stringify(run.body));
  });

  test("a login approval carries the account handle once and mints no credential", async () => {
    const d = await start({ purpose: "login" });
    assert.deepEqual((await poll(d.device_code)).body, { status: "pending" });
    assert.equal((await approve(api, d.user_code, "alice")).status, 200);
    const t = await poll(d.device_code);
    assert.equal(t.status, 200, JSON.stringify(t.body));
    assert.deepEqual(t.body, { status: "approved", purpose: "login", account: { handle: "alice" } });
    assertError(await poll(d.device_code), 410, "closed");
  });

  test("a denied device is 403, on every poll", async () => {
    const d = await start();
    assert.equal((await approve(api, d.user_code, "alice", false)).status, 200);
    assertError(await poll(d.device_code), 403, "forbidden");
    assertError(await poll(d.device_code), 403, "forbidden");
    // A decision is final.
    assertError(await approve(api, d.user_code, "alice", true), 409, "conflict");
    assertError(await poll(d.device_code), 403, "forbidden");
  });

  test("bad requests", async () => {
    assertError(await api.call("POST", "/v1/device", { json: deviceBody({ purpose: "root" }) }), 400, "malformed");
    assertError(await api.call("POST", "/v1/device", { json: deviceBody({ mode: "dry_run" }) }), 400, "malformed");
    assertError(await api.call("POST", "/v1/device", { json: deviceBody({ connector_id: 5 }) }), 400, "malformed");
    assertError(await api.call("POST", "/v1/device", { json: [] }), 400, "malformed");
    assertError(await api.call("GET", "/v1/device", {}), 405, "method_not_allowed");
    assertError(await poll("not-hex"), 400, "malformed");
    assertError(await poll("a".repeat(64)), 410, "closed"); // never issued
    assertError(await api.call("GET", "/v1/device/token", {}), 405, "method_not_allowed");
  });

  test("the test approval needs the key and a known code", async () => {
    const d = await start();
    assertError(await approve(api, d.user_code, "alice", true, null), 401, "unauthorized");
    assertError(await approve(api, d.user_code, "alice", true, "wrong-key"), 401, "unauthorized");
    assertError(await approve(api, "BBBB-BBBB", "alice"), 404, "not_found");
    assertError(await approve(api, "not a code", "alice"), 400, "malformed");
    assertError(await api.call("POST", "/v1/device/approve", { token: APPROVAL_KEY, json: { user_code: d.user_code, member_id: "alice" } }), 400, "malformed");
    assert.deepEqual((await poll(d.device_code)).body, { status: "pending" }); // still undecided
  });

  test("a credential opens one run and is 404 on another", async () => {
    const [a, b, unbound] = [await credentialFor(api, "gina"), await credentialFor(api, "hank"), await credentialFor(api, "iris")];
    const runA = (await api.call("POST", "/v1/runs", { token: a, json: openBody({ session_id: "sa" }) })).body;
    const runB = (await api.call("POST", "/v1/runs", { token: b, json: openBody({ session_id: "sb" }) })).body;
    assert.match(runA.run_id, /^r[0-9a-f]{16}$/);

    assert.equal((await api.call("GET", `/v1/runs/${runA.run_id}`, { token: a })).status, 200);
    const c = makeChunk(runA.entry_id, 0, 1, GENESIS, "x");
    const beat = { entry_id: runA.entry_id, seq: 0, head: GENESIS, offset_local: 0, final: false, repo_head: null };
    for (const token of [b, unbound]) {
      assertError(await api.call("GET", `/v1/runs/${runA.run_id}`, { token }), 404, "not_found");
      assertError(await api.call("POST", `/v1/runs/${runA.run_id}/heartbeat`, { token, json: beat }), 404, "not_found");
      assertError(await api.call("PUT", `/v1/runs/${runA.run_id}/chunks/0`, { token, body: c.body, headers: c.headers }), 404, "not_found");
    }
    assertError(await api.call("GET", `/v1/runs/${runB.run_id}`, { token: a }), 404, "not_found");
    // Not even a run that does not exist: an unbound credential sees nothing at all.
    assertError(await api.call("GET", "/v1/runs/r0123456789abcdef", { token: unbound }), 404, "not_found");
    // Unknown or malformed: 401.
    assertError(await api.call("GET", `/v1/runs/${runA.run_id}`, { token: "gcr_" + "0".repeat(64) }), 401, "unauthorized");
    assertError(await api.call("GET", `/v1/runs/${runA.run_id}`, { token: "nope" }), 401, "unauthorized");
    assertError(await api.call("GET", `/v1/runs/${runA.run_id}`, {}), 401, "unauthorized");
  });

  test("repeat open with the same session_id returns the same run; another session_id is 409", async () => {
    const token = await credentialFor(api, "dave");
    const first = await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: "same" }) });
    assert.equal(first.status, 201, JSON.stringify(first.body));
    const again = await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: "same" }) });
    assert.equal(again.status, 200, JSON.stringify(again.body));
    assert.deepEqual(again.body, first.body);

    const other = await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: "different" }) });
    assertError(other, 409, "conflict");
    assert.match(other.body.message, /goalcup continue/);

    // After the receipt, still the same run.
    const r = await api.call("POST", `/v1/runs/${first.body.run_id}/receipt`, { token, json: { entry_id: first.body.entry_id, chain_head: "b".repeat(64) } });
    assert.equal(r.status, 200);
    assert.deepEqual((await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: "same" }) })).body, first.body);
  });

  test("a member's second credential opens a second run, which replaces the first", async () => {
    const one = await credentialFor(api, "erin");
    const first = await api.call("POST", "/v1/runs", { token: one, json: openBody({ session_id: "e1" }) });
    assert.equal(first.status, 201);
    const two = await credentialFor(api, "erin");
    const second = await api.call("POST", "/v1/runs", { token: two, json: openBody({ session_id: "e2" }) });
    assert.equal(second.status, 201, JSON.stringify(second.body)); // no more "already has an open run"
    const beat = (token: string, run: { run_id: string; entry_id: string }) =>
      api.call("POST", `/v1/runs/${run.run_id}/heartbeat`, {
        token,
        json: { entry_id: run.entry_id, seq: 0, head: GENESIS, offset_local: 0, final: false, repo_head: null },
      });
    assert.equal((await beat(one, first.body)).body.reason, "replaced");
    assert.equal((await beat(two, second.body)).body.reason, null);
  });

  test("the heartbeat takes recording and model_requests, and answers reason null", async () => {
    const token = await credentialFor(api, "frank");
    const run = (await api.call("POST", "/v1/runs", { token, json: openBody({ session_id: "f1" }) })).body;
    const beat = (extra: Record<string, unknown>) =>
      api.call("POST", `/v1/runs/${run.run_id}/heartbeat`, {
        token,
        json: { entry_id: run.entry_id, seq: 0, head: GENESIS, offset_local: 0, final: false, repo_head: null, ...extra },
      });
    for (const recording of ["ok", "gaps", "failed", "unknown"]) {
      const r = await beat({ recording, model_requests: 12 });
      assert.equal(r.status, 200, JSON.stringify(r.body));
      assert.deepEqual(r.body, { stop: false, deadline_at: run.deadline_at, reason: null });
    }
    assert.equal((await beat({})).status, 200);
    // null is absent, as for repo_head.
    const nulls = await beat({ recording: null, model_requests: null });
    assert.equal(nulls.status, 200, JSON.stringify(nulls.body));
    assert.deepEqual(nulls.body, { stop: false, deadline_at: run.deadline_at, reason: null });
    assertError(await beat({ model_requests: 2 ** 53 }), 400, "malformed");
    assertError(await beat({ recording: "fine" }), 400, "malformed");
    assertError(await beat({ model_requests: -1 }), 400, "malformed");
    assertError(await beat({ model_requests: "7" }), 400, "malformed");
  });

  test("the device code never appears in a URL", async () => {
    const d = await start();
    // A code in the query string is not read.
    const viaQuery = await api.call("POST", `/v1/device/token?device_code=${d.device_code}`, { json: {} });
    assertError(viaQuery, 400, "malformed");
    assertError(await api.call("GET", `/v1/device/token?device_code=${d.device_code}`, {}), 405, "method_not_allowed");
    assert.ok(!d.verification_uri.includes(d.device_code) && !d.verification_uri_complete.includes(d.device_code));
    assert.equal((await poll(d.device_code)).status, 202);
    // And the Worker's own log, which lists every request line, never holds one.
    assert.ok(worker.logs().includes("/v1/device/token"), "the log lists the requests (else this check proves nothing)");
    for (const code of deviceCodes) assert.ok(!worker.logs().includes(code), "a device code reached the log");
  });
});

describe("the test approval is switched off or refused where it must be", () => {
  test("without TEST_APPROVAL_KEY the route does not exist", async () => {
    const worker = await startWorker({ EVENT_ID: "practice" });
    try {
      assertError(await approve(new Client(worker.url), "BBBB-BBBB", "alice"), 404, "not_found");
    } finally {
      await worker.stop();
    }
  });

  test("TEST_APPROVAL_KEY outside local or STAGING is misconfigured, on every call", async () => {
    const other = { EVENT_ID: "practice", DB_BACKEND: "supabase", TEST_APPROVAL_KEY: APPROVAL_KEY, SUPABASE_URL: "https://unused.invalid", SUPABASE_KEY: "unused" };
    const worker = await startWorker(other);
    try {
      const api = new Client(worker.url);
      assertError(await approve(api, "BBBB-BBBB", "alice"), 500, "misconfigured");
      assertError(await api.call("POST", "/v1/device", { json: deviceBody() }), 500, "misconfigured");
      assertError(await api.call("POST", "/v1/device/token", { json: { device_code: "a".repeat(64) } }), 500, "misconfigured");
      assertError(await api.call("POST", "/v1/runs", { token: "gcr_" + "0".repeat(64), json: openBody() }), 500, "misconfigured");
      assert.ok(!worker.logs().includes(APPROVAL_KEY), "the key reached the log");
    } finally {
      await worker.stop();
    }
    // With STAGING=on the same settings are fine (a wrong key is refused before any database call).
    const staging = await startWorker({ ...other, STAGING: "on" });
    try {
      assertError(await approve(new Client(staging.url), "BBBB-BBBB", "alice", true, "wrong-key"), 401, "unauthorized");
    } finally {
      await staging.stop();
    }
  });
});
