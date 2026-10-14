// LocalDb on an in-memory bucket: the rules that need a clock the HTTP tests cannot move.

import assert from "node:assert/strict";
import { test } from "node:test";
import { isAbandonable } from "../src/db.ts";
import type { OpenRunArgs } from "../src/db.ts";
import { LocalDb } from "../src/db-local.ts";
import { FakeBucket, asR2 } from "./helpers/fake-r2.ts";

const open = (id: number, session: string, startedAt: string, deadlineAt: string): OpenRunArgs => ({
  member_id: "m1",
  event_id: "practice",
  run_id: `r${String(id).padStart(16, "0")}`,
  entry_id: `e${String(id).padStart(16, "0")}`,
  official_start: startedAt,
  deadline_at: deadlineAt,
  connector_id: "codex-cli",
  manifest_sha256: "a".repeat(64),
  connector_dir_sha256: "c".repeat(64),
  workspace: "/w",
  session_id: session,
  harness_version: "1",
  tracer_version: "1",
  repo_url: "https://github.com/a/b",
});

test("isAbandonable: strictly more than 24 hours after the deadline", () => {
  const deadline = "2026-11-13T09:00:00Z";
  assert.equal(isAbandonable(deadline, new Date("2026-11-14T09:00:00Z")), false);
  assert.equal(isAbandonable(deadline, new Date("2026-11-14T09:00:01Z")), true);
  assert.equal(isAbandonable(deadline, new Date("2026-11-06T09:00:00Z")), false);
});

test("opening a run abandons the account's unsealed run that is 24 hours past its deadline", async () => {
  const db = await LocalDb.create(asR2(new FakeBucket()), "t1:m1");
  const first = await db.openRun(open(1, "s1", "2026-11-06T09:00:00Z", "2026-11-13T09:00:00Z"));
  assert.equal(first.kind, "created");

  // Exactly 24 hours past the deadline: still the account's open run.
  assert.equal((await db.openRun(open(2, "s2", "2026-11-14T09:00:00Z", "2026-11-21T09:00:00Z"))).kind, "conflict");

  // One second later it is abandoned and the slot is free.
  const next = await db.openRun(open(3, "s2", "2026-11-14T09:00:01Z", "2026-11-21T09:00:01Z"));
  assert.equal(next.kind, "created");
  const old = await db.getRun("m1", "r0000000000000001");
  assert.equal(old?.status, "abandoned");
  assert.equal((await db.getRun("m1", "r0000000000000003"))?.status, "open");
});

const mk = async () => {
  const db = await LocalDb.create(asR2(new FakeBucket()), "t1:m1");
  await db.openRun(open(1, "s1", "2026-11-06T09:00:00Z", "2026-11-13T09:00:00Z"));
  return db;
};
const hb = (seq: number, head: string, final: boolean) => ({
  member_id: "m1", run_id: "r0000000000000001", entry_id: "e0000000000000001", seq, head, offset_local: 10, final, repo_head: null, at: "2026-11-13T09:00:00Z",
});
const chunk = (first: number, last: number, prev: string, head: string, offset: number) => ({
  member_id: "m1",
  run_id: "r0000000000000001",
  chunk: {
    entry_id: "e0000000000000001", offset, length: 10, sha256: "1".repeat(64), first_seq: first, last_seq: last, prev, head, r2_key: "k",
    received_at: "2026-11-06T09:30:00Z", ack: { entry_id: "e0000000000000001", seq: last, hash: head, acked_at: "t", server_signature: "s" },
  },
});
const G = "0".repeat(64);

test("the final heartbeat commits once; same is existing, different is a conflict", async () => {
  const db = await mk();
  assert.equal(await db.recordHeartbeat(hb(3, "3".repeat(64), false)), "recorded");
  assert.equal((await db.getRun("m1", "r0000000000000001"))?.final, null);
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true)), "final_committed");
  assert.deepEqual((await db.getRun("m1", "r0000000000000001"))?.final, { seq: 5, head: "5".repeat(64) });
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true)), "final_existing");
  assert.equal(await db.recordHeartbeat(hb(6, "6".repeat(64), true)), "final_conflict");
  assert.equal(await db.recordHeartbeat(hb(5, "7".repeat(64), true)), "final_conflict");
});

test("a final that contradicts the chain already stored is a conflict", async () => {
  const db = await mk();
  assert.equal(await db.commitChunk(chunk(0, 4, G, "4".repeat(64), 0)), "committed");
  assert.equal(await db.recordHeartbeat(hb(3, "3".repeat(64), true)), "final_conflict"); // behind what is stored
  assert.equal(await db.recordHeartbeat(hb(4, "9".repeat(64), true)), "final_conflict"); // same seq, other head
  assert.equal(await db.recordHeartbeat(hb(4, "4".repeat(64), true)), "final_committed");
});

test("commit refuses a chunk past the committed end, and takes one that ends exactly on it", async () => {
  const db = await mk();
  assert.equal(await db.recordHeartbeat(hb(6, "6".repeat(64), true)), "final_committed");
  assert.equal(await db.commitChunk(chunk(0, 7, G, "7".repeat(64), 0)), "beyond_final");
  assert.equal(await db.commitChunk(chunk(0, 6, G, "8".repeat(64), 0)), "beyond_final"); // other head at the final seq
  assert.equal(await db.commitChunk(chunk(0, 6, G, "6".repeat(64), 0)), "committed");
});

test("expireRun: only an open, unsealed run past 24 hours, and then heartbeats are closed", async () => {
  const db = await mk();
  assert.equal(await db.expireRun("m1", "r0000000000000001", "2026-11-14T09:00:00Z"), false);
  assert.equal(await db.expireRun("m1", "r0000000000000001", "2026-11-14T09:00:01Z"), true);
  assert.equal((await db.getRun("m1", "r0000000000000001"))?.status, "abandoned");
  assert.equal(await db.expireRun("m1", "r0000000000000001", "2026-11-15T09:00:00Z"), false);
  assert.equal(await db.recordHeartbeat(hb(5, "5".repeat(64), true)), "closed");
  assert.equal(await db.commitChunk(chunk(0, 1, G, "1".repeat(64), 0)), "closed");
});

test("a final heartbeat is still taken on a sealed run, an ordinary one is a no-op", async () => {
  const db = await mk();
  await db.sealRun({ member_id: "m1", run_id: "r0000000000000001", entry_id: "e0000000000000001", seal_sha256: "1", baseline_sha256: "2", seal_key: "k", baseline_key: "k2", sealed_at: "t" });
  assert.equal(await db.recordHeartbeat(hb(0, G, false)), "recorded");
  assert.equal(await db.recordHeartbeat(hb(-1 + 1, G, true)), "final_committed");
});

test("a sealed run never blocks, and an old run of a different member is untouched", async () => {
  const db = await LocalDb.create(asR2(new FakeBucket()), "t1:m1,t2:m2");
  const a = await db.openRun(open(1, "s1", "2026-11-06T09:00:00Z", "2026-11-13T09:00:00Z"));
  assert.equal(a.kind, "created");
  const sealed = await db.sealRun({
    member_id: "m1", run_id: "r0000000000000001", entry_id: "e0000000000000001", seal_sha256: "1", baseline_sha256: "2",
    seal_key: "k", baseline_key: "k2", sealed_at: "2026-11-13T09:00:00Z",
  });
  assert.equal(sealed, "sealed");
  assert.equal((await db.openRun(open(2, "s2", "2026-11-13T10:00:00Z", "2026-11-20T10:00:00Z"))).kind, "created");
  assert.equal((await db.getRun("m1", "r0000000000000001"))?.status, "sealed");
});
