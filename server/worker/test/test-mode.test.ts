// Organizer test runs (mode "test"): their own event, and refused where the server has none.
// The allowlist of operators is enforced by the database (website migration 20261015160000);
// the local backend used here does not model it.

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import { credentialFor, openBody } from "./helpers/client.ts";
import { type InProcess, inProcess } from "./helpers/inproc.ts";

let rig: InProcess | undefined;
afterEach(() => rig?.stop());

test("a test credential opens its run in the test event, and nowhere else", async () => {
  rig = inProcess("2026-11-06T10:00:00Z", { TEST_EVENT_ID: "test" });
  const cred = await credentialFor(rig.api, "op-1", "test");
  const res = await rig.api.call("POST", "/v1/runs", { token: cred, json: openBody({ session_id: "s1" }) });
  assert.equal(res.status, 201, JSON.stringify(res.body));
  const run = (await rig.db().getRun("op-1", (res.body as { run_id: string }).run_id, null))!.run;
  assert.equal(run.event_id, "test");
  assert.equal(run.mode, "test");
});

test("without TEST_EVENT_ID a test credential cannot open a run", async () => {
  rig = inProcess("2026-11-06T10:00:00Z");
  const cred = await credentialFor(rig.api, "op-1", "test");
  const res = await rig.api.call("POST", "/v1/runs", { token: cred, json: openBody({ session_id: "s1" }) });
  assert.equal(res.status, 403);
});

test("a test run never replaces the account's competition run", async () => {
  rig = inProcess("2026-11-06T10:00:00Z", { TEST_EVENT_ID: "test" });
  const comp = await rig.api.call("POST", "/v1/runs", { token: await credentialFor(rig.api, "op-1"), json: openBody({ session_id: "c1" }) });
  assert.equal(comp.status, 201);
  const t = await rig.api.call("POST", "/v1/runs", { token: await credentialFor(rig.api, "op-1", "test"), json: openBody({ session_id: "t1" }) });
  assert.equal(t.status, 201);
  const held = (await rig.db().getRun("op-1", (comp.body as { run_id: string }).run_id, null))!.run;
  assert.equal(held.replaced_at, null);
});
