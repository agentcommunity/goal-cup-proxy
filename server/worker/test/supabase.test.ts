// SupabaseDb against a stubbed fetch: every method's RPC function name, argument names and
// result mapping. There is no Supabase here; sql/proposed-goalcup-functions.sql is the other half
// of this contract, and these names must stay equal to its function and parameter names.

import assert from "node:assert/strict";
import { afterEach, test } from "node:test";
import type { ChunkRecord, EntryRow, OpenRunArgs, RunRow } from "../src/db.ts";
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

const T = "2026-11-06T09:00:00Z";
const run: RunRow = {
  run_id: "r0000000000000001", event_id: "cup", member_id: "m", mode: "competition", status: "open", official_start: T,
  deadline_at: "2026-11-13T09:00:00Z", replaced_at: null, repo_url: "https://github.com/a/b", first_entry_id: "e0000000000000001", current_entry_id: "e0000000000000001", current_sealed_at: null,
};
const entry: EntryRow = {
  entry_id: "e0000000000000001", run_id: "r0000000000000001", segment: 0, connector_id: "c", manifest_sha256: H("a"), connector_dir_sha256: H("c"),
  workspace: "/w", session_id: "s", offset: 0, next_seq: 0, head: H("0"), last_ack: null, final: null, receipt: null, seal_sha256: null,
  baseline_sha256: null, sealed_at: null,
};
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

const openArgs: OpenRunArgs = {
  member_id: "m", event_id: "cup", mode: "competition", run_id: "r1", entry_id: "e1", official_start: "s", deadline_at: "d", connector_id: "c",
  manifest_sha256: "ms", connector_dir_sha256: "cd", workspace: "w", session_id: "sid", harness_version: "h", tracer_version: "t", repo_url: "u",
  machine: { os: "linux", cpus: 4 }, client_ip: "203.0.113.9", client_country: "DE", credential: H("c"), credential_expires_at: "x",
};

test("openRun -> upload_open_run with every p_ argument, binding the credential in the same call", async () => {
  const calls = stub({ result: "created", run, entry }, { result: "conflict" }, { result: "forbidden", message: "no" }, { result: "existing", run, entry });
  assert.deepEqual(await db().openRun(openArgs), { kind: "created", run, entry });
  assert.deepEqual(await db().openRun(openArgs), { kind: "conflict" });
  assert.deepEqual(await db().openRun(openArgs), { kind: "forbidden", message: "no" });
  assert.deepEqual(await db().openRun(openArgs), { kind: "existing", run, entry }); // a retry that raced the original
  assert.equal(calls[0]!.fn, "upload_open_run");
  assert.deepEqual(calls[0]!.args, {
    p_member_id: "m", p_event_id: "cup", p_mode: "competition", p_run_id: "r1", p_entry_id: "e1", p_official_start: "s", p_deadline_at: "d",
    p_connector_id: "c", p_manifest_sha256: "ms", p_connector_dir_sha256: "cd", p_workspace: "w", p_session_id: "sid",
    p_harness_version: "h", p_tracer_version: "t", p_repo_url: "u", p_machine: { os: "linux", cpus: 4 },
    p_client_ip: "203.0.113.9", p_client_country: "DE", p_credential_hash: H("c"), p_credential_expires_at: "x",
  });
});

test("continueRun -> upload_continue_run", async () => {
  const second = { ...entry, entry_id: "e0000000000000002", segment: 1 };
  const calls = stub(
    { result: "created", run, entry: second }, { result: "existing", run, entry: second },
    { result: "conflict" }, { result: "closed" }, { result: "not_found" },
  );
  const args = {
    member_id: "m", run_id: "r1", previous_entry_id: "e1", entry_id: "e2", connector_id: "c", manifest_sha256: "ms", connector_dir_sha256: "cd",
    workspace: "w", session_id: "sid", harness_version: "h", tracer_version: "t", client_ip: null, client_country: null, at: "t",
  };
  assert.deepEqual(await db().continueRun(args), { kind: "created", run, entry: second });
  assert.deepEqual(await db().continueRun(args), { kind: "existing", run, entry: second });
  for (const kind of ["conflict", "closed", "not_found"]) assert.deepEqual(await db().continueRun(args), { kind });
  assert.deepEqual(calls[0], {
    fn: "upload_continue_run",
    args: {
      p_member_id: "m", p_run_id: "r1", p_previous_entry_id: "e1", p_entry_id: "e2", p_connector_id: "c", p_manifest_sha256: "ms",
      p_connector_dir_sha256: "cd", p_workspace: "w", p_session_id: "sid", p_harness_version: "h", p_tracer_version: "t",
      p_client_ip: null, p_client_country: null, p_at: "t",
    },
  });
});

test("getRun -> upload_run_position (the entry named, or null for the current one); getChunk -> upload_get_chunk", async () => {
  const calls = stub({ run, entry }, { run, entry: null }, null, chunk, null);
  assert.deepEqual(await db().getRun("m", "r1", null), { run, entry });
  assert.deepEqual(await db().getRun("m", "r1", "e9"), { run, entry: null });
  assert.equal(await db().getRun("m", "r2", null), null);
  assert.deepEqual(await db().getChunk("m", "r1", "e1", 0), chunk);
  assert.equal(await db().getChunk("m", "r1", "e1", 99), null);
  assert.deepEqual(calls[0], { fn: "upload_run_position", args: { p_member_id: "m", p_run_id: "r1", p_entry_id: null } });
  assert.deepEqual(calls[1]!.args, { p_member_id: "m", p_run_id: "r1", p_entry_id: "e9" });
  assert.deepEqual(calls[3], { fn: "upload_get_chunk", args: { p_member_id: "m", p_run_id: "r1", p_entry_id: "e1", p_offset: 0 } });
});

test("a row that does not look like one is a 502 that names the fields, not the values", async () => {
  const lines: string[] = [];
  const quiet = console.error;
  console.error = (...args: unknown[]) => void lines.push(args.map(String).join(" "));
  try {
    const bad502 = (e: unknown) => e instanceof ApiError && e.status === 502 && e.code === "upstream_error";
    const secret = "do-not-log-this-workspace";
    for (const [reply, run_, entry_] of [
      [{ run: { ...run, status: "flagged" }, entry }, "run", "status"], // a word the Worker does not know
      [{ run: { ...run, replaced_at: "yesterday" }, entry }, "run", "replaced_at"],
      [{ run, entry: { ...entry, offset: 1.5 } }, "entry", "offset"], // not a whole number
      [{ run, entry: { ...entry, next_seq: 2 ** 53 } }, "entry", "next_seq"], // not a safe integer
      [{ run, entry: { ...entry, head: "ABC", workspace: secret } }, "entry", "head"],
      [{ run, entry: { ...entry, final: { seq: 1 } } }, "entry", "final"],
      [{ run, entry: { ...entry, sealed_at: 5 } }, "entry", "sealed_at"],
      [{ run: { ...run, current_entry_id: "nope" }, entry }, "run", "current_entry_id"],
    ] as const) {
      stub(reply);
      await assert.rejects(db().getRun("m", "r1", null), bad502);
      assert.match(lines.at(-1) ?? "", new RegExp(`the ${run_} has fields.*${entry_}`));
      assert.ok(!lines.at(-1)!.includes(secret), "a value reached the log");
    }
    stub({ result: "created", run, entry: { ...entry, segment: -1 } });
    await assert.rejects(db().openRun(openArgs), bad502);
    stub({ result: "existing", run: { ...run, mode: "dry_run" }, entry });
    await assert.rejects(db().continueRun({ member_id: "m", run_id: "r1", previous_entry_id: "e1", entry_id: "e2", connector_id: "c", manifest_sha256: "m", connector_dir_sha256: "d", workspace: "w", session_id: "s", harness_version: "h", tracer_version: "t", client_ip: null, client_country: null, at: "t" }), bad502);
    stub({ ...chunk, length: "ten" });
    await assert.rejects(db().getChunk("m", "r1", "e1", 0), bad502);
    stub({ ...chunk, head: "short" });
    await assert.rejects(db().getChunk("m", "r1", "e1", 0), bad502);
    stub(["not", "an", "object"]);
    await assert.rejects(db().getRun("m", "r1", null), bad502);
    stub("incomplete-ish");
    await assert.rejects(db().closeRun("m", "r1", "t"), bad502);
  } finally {
    console.error = quiet;
  }
});

test("storeReceipt -> upload_issue_receipt", async () => {
  const calls = stub({ result: "stored", receipt }, { result: "existing", receipt }, { result: "conflict" }, { result: "closed" }, { result: "not_found" });
  assert.deepEqual(await db().storeReceipt("m", "r1", "e1", H("b"), receipt), { kind: "stored", receipt });
  assert.deepEqual(await db().storeReceipt("m", "r1", "e1", H("b"), receipt), { kind: "existing", receipt });
  assert.deepEqual(await db().storeReceipt("m", "r1", "e1", H("d"), receipt), { kind: "conflict" });
  assert.deepEqual(await db().storeReceipt("m", "r1", "e1", H("b"), receipt), { kind: "closed" });
  assert.deepEqual(await db().storeReceipt("m", "r1", "e1", H("b"), receipt), { kind: "not_found" });
  assert.deepEqual(calls[0], { fn: "upload_issue_receipt", args: { p_member_id: "m", p_run_id: "r1", p_entry_id: "e1", p_chain_head: H("b"), p_receipt: receipt } });
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

test("sealRun -> upload_seal_run, with the run's status after it", async () => {
  const calls = stub(
    { result: "sealed", run_status: "sealed" }, { result: "already_sealed", run_status: "open" },
    { result: "conflict" }, { result: "no_final" }, { result: "chain_behind" }, { result: "closed" }, { result: "not_found" },
    { result: "sealed", run_status: "finished" },
  );
  const a = {
    member_id: "m", run_id: "r1", entry_id: "e1", seal_sha256: "s", baseline_sha256: "b", seal_key: "sk", baseline_key: "bk", sealed_at: "t",
  };
  assert.deepEqual(await db().sealRun(a), { kind: "sealed", run_status: "sealed" });
  assert.deepEqual(await db().sealRun(a), { kind: "already_sealed", run_status: "open" });
  for (const kind of ["conflict", "no_final", "chain_behind", "closed", "not_found"]) assert.deepEqual(await db().sealRun(a), { kind });
  const quiet = console.error;
  console.error = () => {};
  try {
    await assert.rejects(db().sealRun(a), (e) => e instanceof ApiError && e.status === 502); // a status word the Worker does not know
  } finally {
    console.error = quiet;
  }
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

test("closeRun -> upload_close_run", async () => {
  const calls = stub("open", "sealed", "incomplete", null);
  assert.equal(await db().closeRun("m", "r1", "t"), "open");
  assert.equal(await db().closeRun("m", "r1", "t"), "sealed");
  assert.equal(await db().closeRun("m", "r1", "t"), "incomplete");
  assert.equal(await db().closeRun("m", "r2", "t"), null);
  assert.deepEqual(calls[0], { fn: "upload_close_run", args: { p_member_id: "m", p_run_id: "r1", p_at: "t" } });
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
    await assert.rejects(db().openRun(openArgs), (e) => e instanceof ApiError && e.status === 502);
  } finally {
    console.error = quiet;
  }
});

test("a refused request logs PostgREST's code and message, not its details or hint", async () => {
  const lines: string[] = [];
  const quiet = console.error;
  console.error = (...args: unknown[]) => void lines.push(args.map(String).join(" "));
  try {
    globalThis.fetch = (async () =>
      new Response(
        JSON.stringify({ code: "23505", message: "duplicate key", details: "Key (client_ip)=(203.0.113.9) exists.", hint: 'machine={"os":"macOS"}' }),
        { status: 409 },
      )) as typeof fetch;
    await assert.rejects(db().closeRun("m", "r1", "t"), (e) => e instanceof ApiError && e.status === 502);
    globalThis.fetch = (async () => new Response("<html>not json 203.0.113.9</html>", { status: 500 })) as typeof fetch;
    await assert.rejects(db().closeRun("m", "r1", "t"), (e) => e instanceof ApiError && e.status === 502);
  } finally {
    console.error = quiet;
  }
  assert.match(lines[0] ?? "", /HTTP 409: 23505 duplicate key$/);
  assert.match(lines[1] ?? "", /HTTP 500: \(no readable error\)$/);
  assert.ok(!lines.join("\n").includes("203.0.113.9") && !lines.join("\n").includes("macOS"), lines.join("\n"));
});
