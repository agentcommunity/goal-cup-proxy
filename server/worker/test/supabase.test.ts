// SupabaseDb against a stubbed fetch: every method's RPC function name, argument names and
// result mapping. There is no Supabase here; sql/goal-cup-tracer-upload.sql (the website's migration)
// is the other half of this contract, and these names (after the goal_cup_ prefix) must stay equal
// to its function and parameter names.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { afterEach, test } from "node:test";
import { type ChunkRecord, type EntryRow, NOTICE_KINDS, type NoticeArgs, type OpenRunArgs, type RunRow, type TickRun } from "../src/db.ts";
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
    const prefix = "https://p.example/rest/v1/rpc/goal_cup_";
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
  stored_bytes: 0, chunk_count: 0,
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
  const calls = stub("ok", "not_found", "expired", "already_decided", "forbidden");
  for (const want of ["ok", "not_found", "expired", "already_decided", "forbidden"]) {
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

test("commitChunk -> upload_commit_chunk(p_chunk, p_stored_bytes)", async () => {
  const calls = stub("committed", "conflict", "beyond_final", "closed", "not_found");
  for (const want of ["committed", "conflict", "beyond_final", "closed", "not_found"]) {
    assert.equal(await db().commitChunk({ member_id: "m", run_id: "r1", chunk, stored_bytes: 4321 }), want);
  }
  assert.deepEqual(calls[0], { fn: "upload_commit_chunk", args: { p_member_id: "m", p_run_id: "r1", p_chunk: chunk, p_stored_bytes: 4321 } });
});

test("a run's stored_bytes and chunk_count come through, are checked, and must be there", async () => {
  const counted = { ...run, stored_bytes: 68_719_476_736, chunk_count: 2048 };
  stub({ run: counted, entry });
  assert.deepEqual(await db().getRun("m", "r1", null), { run: counted, entry });
  const { stored_bytes: _s, ...uncounted } = run;
  const quiet = console.error;
  console.error = () => {};
  try {
    for (const bad of [{ stored_bytes: -1 }, { stored_bytes: "12" }, { chunk_count: 1.5 }, { stored_bytes: null }]) {
      stub({ run: { ...run, ...bad }, entry });
      await assert.rejects(db().getRun("m", "r1", null), (e) => e instanceof ApiError && e.status === 502, JSON.stringify(bad));
    }
    stub({ run: uncounted, entry }); // a database without the migration this Worker needs
    await assert.rejects(db().getRun("m", "r1", null), (e) => e instanceof ApiError && e.status === 502);
  } finally {
    console.error = quiet;
  }
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
  const a = {
    member_id: "m", run_id: "r1", entry_id: "e1", seq: 9, head: H("9"), offset_local: 1234, final: true, repo_head: "f".repeat(40),
    recording: "gaps" as const, model_requests: 77, vitals: null, client_ip: "203.0.113.9", client_country: "DE", at: "t",
  };
  for (const want of ["recorded", "final_committed", "final_existing", "final_conflict", "closed", "not_found"]) {
    assert.equal(await db().recordHeartbeat(a), want);
  }
  assert.deepEqual(calls[0], {
    fn: "upload_heartbeat",
    args: {
      p_member_id: "m", p_run_id: "r1", p_entry_id: "e1", p_seq: 9, p_head: H("9"), p_offset_local: 1234,
      p_final: true, p_repo_head: "f".repeat(40), p_recording: "gaps", p_model_requests: 77,
      p_client_ip: "203.0.113.9", p_client_country: "DE", p_at: "t",
    },
  });
  // Fields that were not sent go as null.
  const calls2 = stub("recorded");
  await db().recordHeartbeat({ ...a, recording: null, model_requests: null, client_ip: null, client_country: null });
  assert.deepEqual([calls2[0]!.args.p_recording, calls2[0]!.args.p_model_requests, calls2[0]!.args.p_client_ip, calls2[0]!.args.p_client_country], [null, null, null, null]);
});

test("recordHeartbeat sends p_vitals only when the heartbeat carried vitals (the 13-argument form answers otherwise)", async () => {
  const a = {
    member_id: "m", run_id: "r1", entry_id: "e1", seq: 9, head: H("9"), offset_local: 1234, final: false, repo_head: null,
    recording: null, model_requests: null, client_ip: null, client_country: null, at: "t",
  };
  const vitals = { disk_free_bytes: 2 ** 53 - 1, disk_short: true, agent_paused: "disk" as const, rss_bytes: 1, cpu_ms: 2, open_files: 3 };
  const calls = stub("recorded", "recorded");
  await db().recordHeartbeat({ ...a, vitals });
  await db().recordHeartbeat({ ...a, vitals: null });
  assert.deepEqual(calls[0]!.args.p_vitals, vitals);
  assert.ok(!("p_vitals" in calls[1]!.args), JSON.stringify(calls[1]!.args));
  assert.equal(Object.keys(calls[0]!.args).length, Object.keys(calls[1]!.args).length + 1);
});

test("a tick run's disk_notice_at comes through, is checked, and must be there", async () => {
  const owed = { ...tickRun, disk_notice_at: "2026-11-06T10:00:00Z" };
  stub([owed, tickRun]);
  assert.deepEqual(await db().runsForTick(T), [owed, tickRun]);
  const quiet = console.error;
  console.error = () => {};
  try {
    for (const bad of ["yesterday", 5, true, undefined]) {
      stub([{ ...tickRun, disk_notice_at: bad }]);
      await assert.rejects(db().runsForTick(T), (e) => e instanceof ApiError && e.status === 502, JSON.stringify(bad));
    }
  } finally {
    console.error = quiet;
  }
});

test("a claimed disk_full notice is a notice the Worker knows", async () => {
  const n = { id: 7, kind: "disk_full", run_id: "r0000000000000001", to: null, subject: "s", text: "t" };
  stub([n]);
  assert.deepEqual(await db().claimNotices(20, T), [n]);
});

const tickRun: TickRun = {
  ...run, last_heartbeat_at: "2026-11-06T09:05:00Z", last_model_request_at: T, flagged_at: null, flag_reason: null, final_commit: null,
  disk_notice_at: null,
};

test("runsForTick -> upload_runs_for_tick(p_at), each row checked", async () => {
  const flagged = { ...tickRun, run_id: "r0000000000000002", flagged_at: T, flag_reason: "gaps", final_commit: "none", last_heartbeat_at: null, last_model_request_at: null };
  const calls = stub([tickRun, flagged], [], null);
  assert.deepEqual(await db().runsForTick(T), [tickRun, flagged]);
  assert.deepEqual(await db().runsForTick(T), []);
  assert.deepEqual(calls[0], { fn: "upload_runs_for_tick", args: { p_at: T } });
  const quiet = console.error;
  const lines: string[] = [];
  console.error = (...args: unknown[]) => void lines.push(args.map(String).join(" "));
  try {
    await assert.rejects(db().runsForTick(T), (e) => e instanceof ApiError && e.status === 502); // not a list
    for (const bad of [{ last_model_request_at: "yesterday" }, { flag_reason: "dunno" }, { final_commit: "abc" }, { last_heartbeat_at: 5 }, { status: "finished" }]) {
      stub([{ ...tickRun, ...bad }]);
      await assert.rejects(db().runsForTick(T), (e) => e instanceof ApiError && e.status === 502, JSON.stringify(bad));
    }
  } finally {
    console.error = quiet;
  }
  assert.ok(lines.some((l) => l.includes("last_model_request_at") && !l.includes("yesterday")), lines.join("\n"));
});

test("queueNotices -> one upload_queue_notices call with the whole batch, the count back", async () => {
  const n: NoticeArgs = { run_id: "r1", member_id: "m", kind: "silence", dedupe_key: T, payload: { last_heartbeat_at: T }, at: "t" };
  const batch = [n, { ...n, run_id: "r2", kind: "started" as const, dedupe_key: "" }];
  const calls = stub(2, 0, "two", -1, 1.5);
  assert.equal(await db().queueNotices(batch), 2);
  assert.equal(await db().queueNotices(batch), 0);
  assert.equal(calls.length, 2);
  assert.deepEqual(calls[0], { fn: "upload_queue_notices", args: { p_notices: batch } });
  const quiet = console.error;
  console.error = () => {};
  try {
    for (let i = 0; i < 3; i++) await assert.rejects(db().queueNotices(batch), (e) => e instanceof ApiError && e.status === 502);
  } finally {
    console.error = quiet;
  }
  assert.equal(await db().queueNotices([]), 0);
  assert.equal(calls.length, 5, "an empty batch is not sent");
});

test("recordFinalCommit -> upload_record_final_commit", async () => {
  const calls = stub(true, false, true);
  await db().recordFinalCommit("m", "r1", "a".repeat(40), "t");
  await db().recordFinalCommit("m", "r1", "none", "t");
  await db().recordFinalCommit("m", "r1", "b".repeat(40), "t", { pushed_at: T, branch: "main", late_push: true, force_push: false, head_fallback: false });
  const none = { p_pushed_at: null, p_branch: null, p_late_push: null, p_force_push: null, p_head_fallback: false };
  assert.deepEqual(calls[0], { fn: "upload_record_final_commit", args: { p_member_id: "m", p_run_id: "r1", p_commit: "a".repeat(40), p_at: "t", ...none } });
  assert.deepEqual(calls[2], {
    fn: "upload_record_final_commit",
    args: { p_member_id: "m", p_run_id: "r1", p_commit: "b".repeat(40), p_at: "t", p_pushed_at: T, p_branch: "main", p_late_push: true, p_force_push: false, p_head_fallback: false },
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

// ---- SQL parity: the names the Worker sends must exist in the migration ----------------------------
// Production runs these functions; the behaviour suites run the local DB. Nothing else ties the two,
// so read both source files and compare function names and parameter names.

const SRC = join(dirname(fileURLToPath(import.meta.url)), "..");

/** The `p_` keys written in a piece of an argument object. */
const pKeys = (text: string) => [...text.matchAll(/\b(p_[a-z0-9_]+)\s*[:,}\n]/g)].map((k) => k[1] as string);

/**
 * Every `this.#rpc("<name>", { p_…: … })` call in db-supabase.ts, once per argument set it can send:
 * a conditional spread `...(cond ? {} : { p_x: … })` makes two variants, without and with its keys.
 */
function workerCalls(): { name: string; args: Set<string> }[] {
  const src = readFileSync(join(SRC, "src", "db-supabase.ts"), "utf8");
  const calls: { name: string; args: Set<string> }[] = [];
  for (const m of src.matchAll(/#rpc\("([a-z_]+)",\s*\{/g)) {
    let depth = 1;
    let i = (m.index ?? 0) + m[0].length;
    const start = i;
    for (; i < src.length && depth > 0; i++) {
      if (src[i] === "{") depth++;
      else if (src[i] === "}") depth--;
    }
    const body = src.slice(start, i - 1);
    const spread = /\.\.\.\([^?]*\?\s*\{\s*\}\s*:\s*\{([^}]*)\}\s*\)/g;
    const optional = [...body.matchAll(spread)].map((s) => pKeys(s[1] as string));
    let variants = [pKeys(body.replace(spread, ""))];
    for (const keys of optional) variants = variants.flatMap((v) => [v, [...v, ...keys]]);
    for (const v of variants) calls.push({ name: m[1] as string, args: new Set(v) });
  }
  return calls;
}

interface SqlFn {
  params: Set<string>;
  required: Set<string>;
}

/**
 * Every overload of each `public.goal_cup_<name>` in the mirror. A later `CREATE OR REPLACE` with the
 * same parameters replaces an earlier one (the last wins); different parameters are another overload.
 */
function sqlFunctions(): Map<string, SqlFn[]> {
  const sql = readFileSync(join(SRC, "sql", "goal-cup-tracer-upload.sql"), "utf8");
  const fns = new Map<string, Map<string, SqlFn>>();
  for (const m of sql.matchAll(/CREATE (?:OR REPLACE )?FUNCTION public\.goal_cup_([a-z_]+)\(/g)) {
    let depth = 1;
    let i = (m.index ?? 0) + m[0].length;
    const start = i;
    for (; i < sql.length && depth > 0; i++) {
      if (sql[i] === "(") depth++;
      else if (sql[i] === ")") depth--;
    }
    const params = new Set<string>();
    const required = new Set<string>();
    let level = 0;
    let piece = "";
    const pieces: string[] = [];
    for (const ch of sql.slice(start, i - 1)) {
      if (ch === "(") level++;
      if (ch === ")") level--;
      if (ch === "," && level === 0) {
        pieces.push(piece);
        piece = "";
      } else piece += ch;
    }
    pieces.push(piece);
    for (const p of pieces) {
      const name = /^\s*(p_[a-z0-9_]+)\s/.exec(p)?.[1];
      if (!name) continue;
      params.add(name);
      if (!/\bDEFAULT\b/i.test(p)) required.add(name);
    }
    const name = m[1] as string;
    if (!fns.has(name)) fns.set(name, new Map());
    fns.get(name)!.set([...params].join(","), { params, required });
  }
  return new Map([...fns].map(([name, overloads]) => [name, [...overloads.values()]]));
}

/** An overload answers a named call when it has every argument sent and gets every one it requires. */
const answers = (fn: SqlFn, args: Set<string>) => [...args].every((a) => fn.params.has(a)) && [...fn.required].every((r) => args.has(r));

test("every RPC the Worker calls exists in the SQL, with the same argument names, and each call matches exactly one overload", () => {
  const calls = workerCalls();
  const fns = sqlFunctions();
  // The scan must see every call site, or a call could hide from the check.
  const sites = readFileSync(join(SRC, "src", "db-supabase.ts"), "utf8").split('#rpc("').length - 1;
  assert.equal(new Set(calls.map((c) => c.name)).size, sites, "the scan missed a #rpc call");
  assert.ok(fns.size >= sites, `found only ${fns.size} SQL functions: the scan is broken`);
  // The heartbeat is sent with and without p_vitals: both forms must be checked.
  assert.deepEqual(calls.filter((c) => c.name === "upload_heartbeat").map((c) => c.args.size).sort(), [13, 14], "the scan missed a call variant");
  const problems: string[] = [];
  for (const { name, args } of calls) {
    const overloads = fns.get(name);
    if (!overloads) {
      problems.push(`${name}: no CREATE FUNCTION public.goal_cup_${name}( in the SQL`);
      continue;
    }
    if (args.size === 0) problems.push(`${name}: no p_ argument was read from the Worker call: the scan is broken`);
    const matched = overloads.filter((fn) => answers(fn, args));
    if (matched.length !== 1) {
      problems.push(`${name}(${[...args].join(", ")}): ${matched.length} overloads answer it, not exactly one`);
    }
  }
  assert.deepEqual(problems, []);
});

/** The text of the last `CREATE [OR REPLACE] FUNCTION <name>(` in the mirror, up to its closing `$$;`. */
function lastFunctionBody(name: string): string {
  const sql = readFileSync(join(SRC, "sql", "goal-cup-tracer-upload.sql"), "utf8");
  const at = [...sql.matchAll(new RegExp(`CREATE (?:OR REPLACE )?FUNCTION ${name.replace(".", "\\.")}\\(`, "g"))].at(-1)?.index;
  assert.ok(at !== undefined, `no ${name} in the SQL`);
  const open = sql.indexOf("$$", at);
  return sql.slice(at, sql.indexOf("$$", open + 2));
}

test("every notice kind the Worker queues is allowed by the notices CHECK constraint and has an email in notice_text", () => {
  const sql = readFileSync(join(SRC, "sql", "goal-cup-tracer-upload.sql"), "utf8");
  // The constraint as the last migration that touches it leaves it (the table's own, or a later ADD CONSTRAINT).
  const checks = [...sql.matchAll(/(?:ADD CONSTRAINT notices_kind_check\s+CHECK|kind\s+text NOT NULL CHECK) \(kind IN \(([^)]*)\)\)/g)];
  assert.ok(checks.length >= 2, "the scan found no widened notices_kind_check");
  const allowed = [...checks.at(-1)![1]!.matchAll(/'([a-z0-9_]+)'/g)].map((m) => m[1]);
  assert.deepEqual([...allowed].sort(), [...NOTICE_KINDS].sort());
  const text = lastFunctionBody("goal_cup_tracer.notice_text");
  for (const kind of NOTICE_KINDS) assert.equal(text.split(`WHEN '${kind}'`).length - 1, 2, `notice_text: ${kind} needs a subject and a body`);
});
