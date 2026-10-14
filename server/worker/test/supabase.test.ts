// SupabaseDb against a stubbed fetch: every method's RPC function name, argument names and
// result mapping. There is no Supabase here; sql/proposed-goalcup-functions.sql is the other half
// of this contract, and these names must stay equal to its function and parameter names.

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import type { ChunkRecord, RunRow } from "../src/db.ts";
import { SupabaseDb } from "../src/db-supabase.ts";
import { ApiError } from "../src/errors.ts";
import type { StartReceipt } from "../src/sign.ts";

const realFetch = globalThis.fetch;
afterEach(() => {
  globalThis.fetch = realFetch;
});

interface Call {
  fn: string;
  args: Record<string, unknown>;
}

/** Answer each call with the next canned JSON value; record function name and arguments. */
function stub(...replies: unknown[]): Call[] {
  const calls: Call[] = [];
  let i = 0;
  globalThis.fetch = (async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    const prefix = "https://p.example/rest/v1/rpc/";
    assert.ok(url.startsWith(prefix), url);
    calls.push({ fn: url.slice(prefix.length), args: JSON.parse(init?.body as string) as Record<string, unknown> });
    return new Response(JSON.stringify(replies[i++]), { status: 200, headers: { "content-type": "application/json" } });
  }) as typeof fetch;
  return calls;
}

const db = () => new SupabaseDb("https://p.example", "key");
const H = (c: string) => c.repeat(64);

const run = { run_id: "r0000000000000001", entry_id: "e0000000000000001", status: "open" } as unknown as RunRow;
const receipt = { claims: { chain_head: H("b") }, public_key: "p", signature: "s" } as unknown as StartReceipt;
const chunk = {
  entry_id: "e0000000000000001", offset: 0, length: 10, sha256: H("1"), first_seq: 0, last_seq: 2, prev: H("0"), head: H("2"),
  r2_key: "traces/x", received_at: "2026-11-06T09:00:00Z",
  ack: { entry_id: "e0000000000000001", seq: 2, hash: H("2"), acked_at: "2026-11-06T09:00:00Z", server_signature: "sig" },
} as ChunkRecord;

test("credential -> upload_credential(p_credential_hash, p_now)", async () => {
  const cred = { member_id: "member-1", run_id: null, mode: "practice", expires_at: "2026-11-06T10:05:00Z" };
  const calls = stub(cred, null);
  assert.deepEqual(await db().credential(H("a"), "2026-11-06T09:06:00Z"), cred);
  assert.equal(await db().credential(H("b"), "2026-11-06T09:06:00Z"), null);
  assert.deepEqual(calls[0], { fn: "upload_credential", args: { p_credential_hash: H("a"), p_now: "2026-11-06T09:06:00Z" } });
});

test("bindCredential -> upload_bind_credential", async () => {
  const calls = stub("bound");
  await db().bindCredential(H("a"), "r0000000000000001", "2026-11-14T09:00:00Z");
  assert.deepEqual(calls[0], {
    fn: "upload_bind_credential",
    args: { p_credential_hash: H("a"), p_run_id: "r0000000000000001", p_expires_at: "2026-11-14T09:00:00Z" },
  });
});

test("createDevice -> upload_create_device", async () => {
  const calls = stub("created");
  await db().createDevice({
    device_sha256: H("d"), user_sha256: H("u"), purpose: "run", mode: "competition", connector_id: "c", tracer_version: "t",
    created_at: "2026-11-06T09:00:00Z", expires_at: "2026-11-06T09:15:00Z",
  });
  assert.deepEqual(calls[0], {
    fn: "upload_create_device",
    args: {
      p_device_hash: H("d"), p_user_hash: H("u"), p_purpose: "run", p_mode: "competition", p_connector_id: "c", p_tracer_version: "t",
      p_created_at: "2026-11-06T09:00:00Z", p_expires_at: "2026-11-06T09:15:00Z",
    },
  });
});

test("pollDevice -> upload_poll_device; the credential comes back once", async () => {
  const calls = stub(
    { result: "pending" },
    { result: "approved", purpose: "run", credential: "gcr_" + H("1"), expires_at: "2026-11-06T10:05:00Z" },
    { result: "approved", purpose: "login", handle: "alice" },
    { result: "denied" },
    { result: "expired" },
  );
  assert.deepEqual(await db().pollDevice(H("d"), "t"), { kind: "pending" });
  assert.deepEqual(await db().pollDevice(H("d"), "t"), { kind: "approved", purpose: "run", credential_plain: "gcr_" + H("1"), expires_at: "2026-11-06T10:05:00Z" });
  assert.deepEqual(await db().pollDevice(H("d"), "t"), { kind: "approved", purpose: "login", handle: "alice" });
  assert.deepEqual(await db().pollDevice(H("d"), "t"), { kind: "denied" });
  assert.deepEqual(await db().pollDevice(H("d"), "t"), { kind: "expired" });
  assert.deepEqual(calls[0], { fn: "upload_poll_device", args: { p_device_hash: H("d"), p_now: "t" } });
});

test("approveDevice -> upload_approve_device(p_user_code, ...)", async () => {
  const calls = stub("ok", "not_found", "expired", "already_decided");
  for (const want of ["ok", "not_found", "expired", "already_decided"]) {
    assert.equal(await db().approveDevice("BCDFGHJK", "m", true, "t"), want);
  }
  assert.deepEqual(calls[0], { fn: "upload_approve_device", args: { p_user_code: "BCDFGHJK", p_member_id: "m", p_approve: true, p_now: "t" } });
});

test("openRun -> upload_open_run with every p_ argument", async () => {
  const calls = stub({ result: "created", run }, { result: "existing", run }, { result: "conflict" }, { result: "forbidden", message: "no" });
  const args = {
    member_id: "m", event_id: "practice", run_id: "r1", entry_id: "e1", official_start: "s", deadline_at: "d", connector_id: "c",
    manifest_sha256: "ms", connector_dir_sha256: "cd", workspace: "w", session_id: "sid", harness_version: "h", tracer_version: "t", repo_url: "u",
  };
  assert.deepEqual(await db().openRun(args), { kind: "created", run });
  assert.deepEqual(await db().openRun(args), { kind: "existing", run });
  assert.deepEqual(await db().openRun(args), { kind: "conflict" });
  assert.deepEqual(await db().openRun(args), { kind: "forbidden", message: "no" });
  assert.equal(calls[0]!.fn, "upload_open_run");
  assert.deepEqual(calls[0]!.args, {
    p_member_id: "m", p_event_id: "practice", p_run_id: "r1", p_entry_id: "e1", p_official_start: "s", p_deadline_at: "d",
    p_connector_id: "c", p_manifest_sha256: "ms", p_connector_dir_sha256: "cd", p_workspace: "w", p_session_id: "sid",
    p_harness_version: "h", p_tracer_version: "t", p_repo_url: "u",
  });
});

test("getRun -> upload_run_position; getChunk -> upload_get_chunk", async () => {
  const calls = stub(run, null, chunk, null);
  assert.deepEqual(await db().getRun("m", "r1"), run);
  assert.equal(await db().getRun("m", "r2"), null);
  assert.deepEqual(await db().getChunk("m", "r1", 0), chunk);
  assert.equal(await db().getChunk("m", "r1", 99), null);
  assert.deepEqual(calls[0], { fn: "upload_run_position", args: { p_member_id: "m", p_run_id: "r1" } });
  assert.deepEqual(calls[2], { fn: "upload_get_chunk", args: { p_member_id: "m", p_run_id: "r1", p_offset: 0 } });
});

test("storeReceipt -> upload_issue_receipt", async () => {
  const calls = stub({ result: "stored", receipt }, { result: "existing", receipt }, { result: "conflict" }, { result: "closed" });
  assert.deepEqual(await db().storeReceipt("m", "r1", H("b"), receipt), { kind: "stored", receipt });
  assert.deepEqual(await db().storeReceipt("m", "r1", H("b"), receipt), { kind: "existing", receipt });
  assert.deepEqual(await db().storeReceipt("m", "r1", H("d"), receipt), { kind: "conflict" });
  assert.deepEqual(await db().storeReceipt("m", "r1", H("b"), receipt), { kind: "closed" });
  assert.deepEqual(calls[0], { fn: "upload_issue_receipt", args: { p_member_id: "m", p_run_id: "r1", p_chain_head: H("b"), p_receipt: receipt } });
});

test("abandonRun -> upload_abandon_run", async () => {
  const calls = stub("abandoned", "has_chunks", "closed", "not_found");
  assert.equal(await db().abandonRun("m", "r1", "why", "t"), "abandoned");
  assert.equal(await db().abandonRun("m", "r1", "why", "t"), "has_chunks");
  assert.equal(await db().abandonRun("m", "r1", "why", "t"), "closed");
  assert.equal(await db().abandonRun("m", "r1", "why", "t"), "not_found");
  assert.deepEqual(calls[0], { fn: "upload_abandon_run", args: { p_member_id: "m", p_run_id: "r1", p_reason: "why", p_at: "t" } });
});

test("commitChunk -> upload_commit_chunk(p_chunk)", async () => {
  const calls = stub("committed", "conflict", "beyond_final", "closed", "not_found");
  for (const want of ["committed", "conflict", "beyond_final", "closed", "not_found"]) {
    assert.equal(await db().commitChunk({ member_id: "m", run_id: "r1", chunk }), want);
  }
  assert.deepEqual(calls[0], { fn: "upload_commit_chunk", args: { p_member_id: "m", p_run_id: "r1", p_chunk: chunk } });
});

test("sealRun -> upload_seal_run", async () => {
  const calls = stub("sealed", "already_sealed", "conflict", "closed", "not_found");
  const a = {
    member_id: "m", run_id: "r1", entry_id: "e1", seal_sha256: "s", baseline_sha256: "b", seal_key: "sk", baseline_key: "bk", sealed_at: "t",
  };
  for (const want of ["sealed", "already_sealed", "conflict", "closed", "not_found"]) assert.equal(await db().sealRun(a), want);
  assert.deepEqual(calls[0], {
    fn: "upload_seal_run",
    args: {
      p_member_id: "m", p_run_id: "r1", p_entry_id: "e1", p_seal_sha256: "s", p_baseline_sha256: "b",
      p_seal_key: "sk", p_baseline_key: "bk", p_sealed_at: "t",
    },
  });
});

test("recordHeartbeat -> upload_heartbeat", async () => {
  const calls = stub("recorded", "final_committed", "final_existing", "final_conflict", "closed", "not_found");
  const a = { member_id: "m", run_id: "r1", entry_id: "e1", seq: 9, head: H("9"), offset_local: 1234, final: true, repo_head: "f".repeat(40), at: "t" };
  for (const want of ["recorded", "final_committed", "final_existing", "final_conflict", "closed", "not_found"]) {
    assert.equal(await db().recordHeartbeat(a), want);
  }
  assert.deepEqual(calls[0], {
    fn: "upload_heartbeat",
    args: {
      p_member_id: "m", p_run_id: "r1", p_entry_id: "e1", p_seq: 9, p_head: H("9"), p_offset_local: 1234,
      p_final: true, p_repo_head: "f".repeat(40), p_at: "t",
    },
  });
});

test("expireRun -> upload_expire_run", async () => {
  const calls = stub("expired", "not_expired");
  assert.equal(await db().expireRun("m", "r1", "t"), true);
  assert.equal(await db().expireRun("m", "r1", "t"), false);
  assert.deepEqual(calls[0], { fn: "upload_expire_run", args: { p_member_id: "m", p_run_id: "r1", p_at: "t" } });
});

test("an answer outside the agreed words is a 502, not a guess", async () => {
  const quiet = console.error;
  console.error = () => {}; // the Db logs what it did not understand
  try {
    stub("maybe");
    await assert.rejects(
      db().sealRun({ member_id: "m", run_id: "r", entry_id: "e", seal_sha256: "s", baseline_sha256: "b", seal_key: "k", baseline_key: "k", sealed_at: "t" }),
      (e) => e instanceof ApiError && e.status === 502,
    );
    stub({ result: "created" }); // created, but no run in the answer
    await assert.rejects(
      db().openRun({ member_id: "m", event_id: "e", run_id: "r", entry_id: "e", official_start: "s", deadline_at: "d", connector_id: "c", manifest_sha256: "m", connector_dir_sha256: "c", workspace: "w", session_id: "s", harness_version: "h", tracer_version: "t", repo_url: "u" }),
      (e) => e instanceof ApiError && e.status === 502,
    );
  } finally {
    console.error = quiet;
  }
});
